//! Already-admitted, interpretation-neutral command-state borrow.

use crate::definition_arena::{DefinitionId, DefinitionView};
use crate::durable_arena::{GlueId, TokenListId};
use crate::env::{CodeTableKind, DenseState, StateError};
use crate::glue::GlueSpec;
use crate::interner::{Interner, InternerAccessError, SymbolId};
use crate::meaning::ResolvedMeaning;
use crate::scaled::Scaled;
use crate::stores::AdmittedState;
use crate::token::TokenWord;

/// One command episode's borrowed session and generation.
///
/// Admission validates coarse owners once. Meaning reads and definition
/// resolution then index the dense bank and definition arena directly.
pub struct CommandContext<'a, G> {
    interner: &'a Interner,
    admitted: AdmittedState<'a, G>,
}

impl<'a, G> CommandContext<'a, G> {
    pub(super) const fn new(interner: &'a Interner, admitted: AdmittedState<'a, G>) -> Self {
        Self { interner, admitted }
    }

    pub fn resolve_symbol(&self, symbol: SymbolId) -> Result<&'a str, InternerAccessError> {
        self.interner.resolve_id(symbol)
    }

    #[inline(always)]
    pub fn meaning(&self, symbol: SymbolId) -> Result<ResolvedMeaning<G>, StateError> {
        // `resolve_id` is the admission check. The compact slot is then a
        // direct index for the lifetime of this context.
        self.interner
            .resolve_id(symbol)
            .map_err(|_| StateError::ForeignSession)?;
        self.admitted.state().meaning(symbol.symbol())
    }

    #[inline(always)]
    pub fn definition(&self, id: DefinitionId<G>) -> DefinitionView<'a, G> {
        self.admitted.definition(id)
    }

    #[inline(always)]
    pub fn token_list(&self, id: TokenListId<G>) -> &'a [TokenWord] {
        self.admitted.token_list(id)
    }

    #[inline(always)]
    pub fn glue(&self, id: GlueId<G>) -> GlueSpec {
        self.admitted.glue(id)
    }

    #[inline(always)]
    pub fn count(&self, index: u16) -> Result<i32, StateError> {
        self.admitted.state().count(index)
    }

    #[inline(always)]
    pub fn dimension(&self, index: u16) -> Result<Scaled, StateError> {
        self.admitted.state().dimension(index)
    }

    #[inline(always)]
    pub fn token_register(&self, index: u16) -> Result<Option<TokenListId<G>>, StateError> {
        self.admitted.state().token_register(index)
    }

    #[inline(always)]
    pub fn glue_register(&self, index: u16) -> Result<Option<GlueId<G>>, StateError> {
        self.admitted.state().glue_register(index)
    }

    #[inline(always)]
    pub fn code(&self, kind: CodeTableKind, scalar: char) -> Result<i64, StateError> {
        self.admitted.state().code(kind, scalar)
    }

    #[must_use]
    pub(crate) const fn state(&self) -> &'a DenseState<G> {
        self.admitted.state()
    }
}
