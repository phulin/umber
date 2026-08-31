//! Fresh brands and coarse owners for one revision generation.

use core::marker::PhantomData;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::definition_arena::{
    AcceptedDefinitionTail, DefinitionArena, DefinitionArenaCursor, DefinitionCheckpointLease,
};
use crate::durable_arena::{GlueArena, ProvenanceArena, TokenListArena};
use crate::memory_accounting::MemoryAccounting;
use crate::provenance::OriginRecord;

#[cfg(test)]
#[path = "generation/tests.rs"]
mod tests;

/// An invariant brand which exists only inside one admitted generation scope.
///
/// The constructor is private. `with_generation` introduces a fresh late-bound
/// lifetime on every call, so ids from independently admitted generations
/// cannot have the same Rust type.
pub struct GenerationBrand<'id> {
    _invariant: PhantomData<fn(&'id ()) -> &'id ()>,
}

/// A sealed, single-use capability for constructing one typed arena.
///
/// Arena constructors consume these tokens. Consequently, even code which can
/// name a generation brand cannot construct a second arena with that brand and
/// accidentally resolve an id against the wrong backing vector.
pub(super) struct ArenaToken<G, Namespace> {
    _brand: PhantomData<fn(&G) -> &G>,
    _namespace: PhantomData<fn(Namespace) -> Namespace>,
}

impl<G, Namespace> ArenaToken<G, Namespace> {
    fn new() -> Self {
        Self {
            _brand: PhantomData,
            _namespace: PhantomData,
        }
    }
}

/// Coarse owner of the publishers and inline arenas in one revision generation.
///
/// Macro definitions remain in structurally owned regions behind compact
/// generation-branded keys. Durable token lists leave their publisher through
/// generation-branded shared owners. Glue and provenance remain compact direct
/// values owned by this bundle.
pub(crate) struct Generation<G> {
    accounting: MemoryAccounting,
    definitions: DefinitionArena<G>,
    token_lists: TokenListArena<G>,
    glue: GlueArena<G>,
    provenance: ProvenanceArena<G>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct GenerationCursor {
    definitions: DefinitionArenaCursor,
    token_lists: u32,
    glue: usize,
    provenance: usize,
}

pub(crate) struct AcceptedGenerationTail<G> {
    head: GenerationCursor,
    definitions: AcceptedDefinitionTail<G>,
    glue: Vec<crate::glue::GlueSpec>,
    provenance: Vec<OriginRecord>,
}

/// Cloneable lifetime authority for one complete immutable generation.
///
/// Cloning is deliberately available only at this coarse boundary. The
/// backing publishers remain private. Ordinary admitted reads borrow this
/// bundle; local macro input rows may retain a coarse definition-region lease,
/// while token-list carriers retain their exact non-atomic owner.
pub struct GenerationOwner<G> {
    generation: Arc<RwLock<Generation<G>>>,
}

/// Coarse generation owner plus the local definition regions reachable from
/// one state checkpoint.
#[doc(hidden)]
pub struct CheckpointGenerationOwner<G> {
    generation: GenerationOwner<G>,
    _definitions: DefinitionCheckpointLease<G>,
}

impl<G> Clone for CheckpointGenerationOwner<G> {
    fn clone(&self) -> Self {
        Self {
            generation: self.generation.clone(),
            _definitions: self._definitions.clone(),
        }
    }
}

impl<G> CheckpointGenerationOwner<G> {
    #[must_use]
    pub(crate) const fn generation(&self) -> &GenerationOwner<G> {
        &self.generation
    }

    pub(crate) fn into_generation(self) -> GenerationOwner<G> {
        self.generation
    }

    #[must_use]
    pub(crate) fn checkpoint_owner_id(&self) -> CheckpointOwnerId {
        self.generation.checkpoint_owner_id()
    }
}

/// Process-local identity of one coarse retained-generation owner.
///
/// This value exists solely for checkpoint retention accounting. It is never
/// semantic identity, never serialized, and never used to admit a runtime
/// handle. Keeping construction beside the authoritative owner prevents
/// downstream accounting from inventing ids from checkpoint cursors.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CheckpointOwnerId(usize);

impl<G> Clone for GenerationOwner<G> {
    fn clone(&self) -> Self {
        Self {
            generation: Arc::clone(&self.generation),
        }
    }
}

impl<G> core::fmt::Debug for GenerationOwner<G> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("GenerationOwner(..)")
    }
}

impl<G> GenerationOwner<G> {
    pub(crate) fn new(generation: Generation<G>) -> Self {
        Self {
            generation: Arc::new(RwLock::new(generation)),
        }
    }

    #[must_use]
    pub fn same_generation(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.generation, &other.generation)
    }

    /// Returns the opaque process-local accounting identity of this owner.
    #[must_use]
    pub fn checkpoint_owner_id(&self) -> CheckpointOwnerId {
        CheckpointOwnerId(Arc::as_ptr(&self.generation) as usize)
    }

    pub(crate) fn generation(&self) -> RwLockReadGuard<'_, Generation<G>> {
        self.generation
            .read()
            .expect("generation lock poisoned by a failed admitted episode")
    }

    pub(crate) fn generation_mut(&self) -> RwLockWriteGuard<'_, Generation<G>> {
        self.generation
            .write()
            .expect("generation lock poisoned by a failed admitted episode")
    }

    pub(crate) fn checkpoint_owner(&self) -> CheckpointGenerationOwner<G> {
        let definitions = self.generation().definitions().checkpoint_lease();
        CheckpointGenerationOwner {
            generation: self.clone(),
            _definitions: definitions,
        }
    }

    /// Returns whether this is the only coarse owner of the generation.
    ///
    /// This is a cold lifecycle check, never a per-value access path.
    #[must_use]
    pub(crate) fn is_unique(&self) -> bool {
        Arc::strong_count(&self.generation) == 1
    }

    #[must_use]
    pub(crate) fn is_owned_only_by(&self, other: &Self) -> bool {
        self.same_generation(other) && Arc::strong_count(&self.generation) == 2
    }

    pub(crate) fn retire(self) -> Result<GenerationRetirement, Self> {
        match Arc::try_unwrap(self.generation) {
            Ok(generation) => Ok(generation
                .into_inner()
                .expect("generation lock poisoned by a failed admitted episode")
                .retire()),
            Err(generation) => Err(Self { generation }),
        }
    }
}

impl CheckpointOwnerId {
    pub(crate) fn from_owner<T>(owner: &T) -> Self {
        Self(std::ptr::from_ref(owner).cast::<()>() as usize)
    }
}

/// Published coordinates and inline rows observed at coarse retirement.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct GenerationRetirement {
    pub(crate) definitions: usize,
    pub(crate) token_lists: usize,
    pub(crate) glue_values: usize,
    pub(crate) provenance_records: usize,
}

impl<G> Generation<G> {
    pub(crate) fn new() -> Self {
        let accounting = MemoryAccounting::default();
        Self {
            definitions: DefinitionArena::new(ArenaToken::new(), accounting.clone()),
            token_lists: TokenListArena::new(ArenaToken::new(), accounting.clone()),
            glue: GlueArena::new(ArenaToken::new()),
            provenance: ProvenanceArena::new(ArenaToken::new()),
            accounting,
        }
    }

    pub(crate) fn cursor(&self) -> GenerationCursor {
        GenerationCursor {
            definitions: self.definitions.cursor(),
            token_lists: self.token_lists.cursor(),
            glue: self.glue.len(),
            provenance: self.provenance.len(),
        }
    }

    pub(crate) fn validates_cursor(&self, cursor: GenerationCursor) -> bool {
        self.definitions.validates_cursor(cursor.definitions)
            && cursor.token_lists <= self.token_lists.cursor()
            && cursor.glue <= self.glue.len()
            && cursor.provenance <= self.provenance.len()
    }

    pub(crate) fn restore_cursor(&mut self, cursor: GenerationCursor) {
        assert!(self.validates_cursor(cursor));
        self.definitions.restore_cursor(cursor.definitions);
        self.token_lists.restore_cursor(cursor.token_lists);
        self.glue.truncate(cursor.glue);
        self.provenance.truncate(cursor.provenance);
    }

    /// Enables canonical immutable-value roots before this generation
    /// publishes semantic payload. A late request fails closed instead of
    /// walking already-published values or hashing their coordinates.
    pub(crate) fn enable_semantic_identity(&mut self) -> bool {
        let definitions = self.definitions.enable_semantic_identity();
        let token_lists = self.token_lists.enable_semantic_identity();
        let glue = self.glue.enable_semantic_identity();
        definitions && token_lists && glue
    }

    pub(crate) fn begin_checkpoint_candidate(
        &mut self,
        cursor: GenerationCursor,
    ) -> AcceptedGenerationTail<G> {
        assert!(self.validates_cursor(cursor));
        let head = self.cursor();
        let definitions = self
            .definitions
            .begin_checkpoint_candidate(cursor.definitions);
        let glue = self.glue.split_off(cursor.glue);
        let provenance = self.provenance.split_off(cursor.provenance);
        self.token_lists.restore_cursor(cursor.token_lists);
        AcceptedGenerationTail {
            head,
            definitions,
            glue,
            provenance,
        }
    }

    pub(crate) fn reject_checkpoint_candidate(
        &mut self,
        cursor: GenerationCursor,
        mut tail: AcceptedGenerationTail<G>,
    ) {
        // Candidate-local publisher coordinates are disposable. They can be
        // lower than the rooted coordinate when initialization abandoned an
        // unpublished row, so rejection must not model this as an ordinary
        // monotonic rewind. Inline arenas retain the rooted prefix in place.
        assert!(cursor.glue <= self.glue.len());
        assert!(cursor.provenance <= self.provenance.len());
        self.glue.truncate(cursor.glue);
        self.provenance.truncate(cursor.provenance);
        self.glue.append_rows(&mut tail.glue);
        self.provenance.append_rows(&mut tail.provenance);
        self.definitions
            .reject_checkpoint_candidate(cursor.definitions, tail.definitions);
        self.token_lists
            .restore_accepted_cursor(tail.head.token_lists);
    }

    pub(crate) fn accept_checkpoint_candidate(&mut self, tail: AcceptedGenerationTail<G>) {
        self.definitions
            .accept_checkpoint_candidate(tail.definitions);
    }

    #[must_use]
    pub(crate) fn memory_accounting(&self) -> MemoryAccounting {
        self.accounting.clone()
    }

    #[must_use]
    pub(crate) const fn definitions(&self) -> &DefinitionArena<G> {
        &self.definitions
    }

    pub(crate) const fn definitions_mut(&mut self) -> &mut DefinitionArena<G> {
        &mut self.definitions
    }

    #[must_use]
    pub(crate) const fn token_lists(&self) -> &TokenListArena<G> {
        &self.token_lists
    }

    pub(crate) const fn token_lists_mut(&mut self) -> &mut TokenListArena<G> {
        &mut self.token_lists
    }

    #[must_use]
    pub(crate) const fn glue(&self) -> &GlueArena<G> {
        &self.glue
    }

    pub(crate) const fn glue_mut(&mut self) -> &mut GlueArena<G> {
        &mut self.glue
    }

    #[must_use]
    pub(crate) const fn provenance(&self) -> &ProvenanceArena<G> {
        &self.provenance
    }

    pub(crate) const fn provenance_mut(&mut self) -> &mut ProvenanceArena<G> {
        &mut self.provenance
    }

    /// Retires every immutable arena in this generation together.
    #[must_use]
    pub(crate) fn retire(self) -> GenerationRetirement {
        GenerationRetirement {
            definitions: self.definitions.len(),
            token_lists: self.token_lists.len(),
            glue_values: self.glue.len(),
            provenance_records: self.provenance.len(),
        }
    }
}

/// Admits one fresh generation for the duration of `use_generation`.
///
/// The higher-ranked closure prevents the brand, the generation owner, and
/// every id derived from it from escaping the admission boundary. Longer-lived
/// engine owners store a type-erased generation and use this boundary when
/// admitting a matching episode; raw ids are never exposed by this module.
pub(crate) fn with_generation<R>(
    use_generation: impl for<'id> FnOnce(Generation<GenerationBrand<'id>>) -> R,
) -> R {
    use_generation(Generation::new())
}
