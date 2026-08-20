//! Fresh brands and coarse owners for one revision generation.

use core::marker::PhantomData;

use crate::definition_arena::DefinitionArena;
use crate::durable_arena::{GlueArena, ProvenanceArena, TokenListArena};

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

/// Coarse owner of every immutable value arena in one revision generation.
///
/// Values carry only copyable typed ids. Keeping this bundle alive is the sole
/// runtime lifetime authority for those ids.
pub(crate) struct Generation<G> {
    definitions: DefinitionArena<G>,
    token_lists: TokenListArena<G>,
    glue: GlueArena<G>,
    provenance: ProvenanceArena<G>,
}

/// Logical rows released when one coarse generation owner retires.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct GenerationRetirement {
    pub(crate) definitions: usize,
    pub(crate) token_lists: usize,
    pub(crate) glue_values: usize,
    pub(crate) provenance_records: usize,
}

impl<G> Generation<G> {
    fn new() -> Self {
        Self {
            definitions: DefinitionArena::new(ArenaToken::new()),
            token_lists: TokenListArena::new(ArenaToken::new()),
            glue: GlueArena::new(ArenaToken::new()),
            provenance: ProvenanceArena::new(ArenaToken::new()),
        }
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
