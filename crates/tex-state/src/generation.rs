//! Fresh brands and coarse owners for one revision generation.

use core::marker::PhantomData;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::definition_arena::DefinitionArena;
use crate::durable_arena::{GlueArena, ProvenanceArena, TokenListArena};
use crate::memory_accounting::MemoryAccounting;

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
/// Macro definitions and durable token lists leave their publisher through
/// generation-branded shared owners. Glue and provenance remain compact
/// direct-index values owned by this bundle.
pub(crate) struct Generation<G> {
    accounting: MemoryAccounting,
    definitions: DefinitionArena<G>,
    token_lists: TokenListArena<G>,
    glue: GlueArena<G>,
    provenance: ProvenanceArena<G>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct GenerationCursor {
    definitions: u32,
    token_lists: u32,
    glue: usize,
    provenance: usize,
}

/// Cloneable lifetime authority for one complete immutable generation.
///
/// Cloning is deliberately available only at this coarse boundary. The
/// backing publishers remain private. Ordinary admitted reads borrow this
/// bundle; macro and token-list carriers retain their own non-atomic owner.
pub struct GenerationOwner<G> {
    generation: Arc<RwLock<Generation<G>>>,
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

    /// Creates the publisher bundle for an ordinary retained-generation
    /// fork. Published definition and token-list payloads remain owned by
    /// their exact semantic carriers; only destination-local publisher
    /// cursors and compact direct-row stores are duplicated.
    pub(crate) fn fork(&self) -> Self {
        let accounting = self.accounting.clone();
        Self {
            accounting: accounting.clone(),
            definitions: self.definitions.fork(accounting.clone()),
            token_lists: self.token_lists.fork(accounting),
            glue: self.glue.clone(),
            provenance: self.provenance.clone(),
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
        cursor.definitions <= self.definitions.cursor()
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
