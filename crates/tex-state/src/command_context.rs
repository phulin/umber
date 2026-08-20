//! Already-admitted, interpretation-neutral command-state borrow.

use crate::definition_arena::{DefinitionId, DefinitionView};
use crate::durable_arena::{GlueId, ProvenanceId, TokenListId};
use crate::env::banks::IntParam;
use crate::env::{CodeTableKind, DenseState, StateError};
use crate::glue::GlueSpec;
use crate::interner::{ControlSequenceKind, Interner, InternerAccessError, Symbol, SymbolId};
use crate::meaning::{Meaning, ResolvedMeaning};
use crate::node_arena::{DurableListId, NodeArenaError, NodeList};
use crate::provenance::OriginRecord;
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
    primitive_names: &'a [String],
    primitive_meanings: &'a [Meaning],
}

impl<'a, G> CommandContext<'a, G> {
    pub(super) const fn new(
        interner: &'a Interner,
        admitted: AdmittedState<'a, G>,
        primitive_names: &'a [String],
        primitive_meanings: &'a [Meaning],
    ) -> Self {
        Self {
            interner,
            admitted,
            primitive_names,
            primitive_meanings,
        }
    }

    pub fn resolve_symbol(&self, symbol: SymbolId) -> Result<&'a str, InternerAccessError> {
        self.interner.resolve_id(symbol)
    }

    #[inline(always)]
    pub fn meaning(&self, symbol: Symbol) -> Result<ResolvedMeaning<G>, StateError> {
        self.interner
            .resolve_local(symbol)
            .ok_or(StateError::ForeignSession)?;
        self.admitted.state().meaning(symbol)
    }

    #[inline(always)]
    pub fn meaning_id(&self, symbol: SymbolId) -> Result<ResolvedMeaning<G>, StateError> {
        // `resolve_id` is the admission check. The compact slot is then a
        // direct index for the lifetime of this context.
        self.interner
            .resolve_id(symbol)
            .map_err(|_| StateError::ForeignSession)?;
        self.admitted.state().meaning(symbol.symbol())
    }

    pub fn resolve(&self, symbol: Symbol) -> Option<&'a str> {
        self.interner.resolve_local(symbol)
    }

    pub fn control_sequence_kind(&self, symbol: Symbol) -> Option<ControlSequenceKind> {
        self.interner
            .qualify_local(symbol)
            .and_then(|id| self.interner.kind_id(id).ok())
    }

    #[must_use]
    pub fn active_character_symbol(&self, ch: char) -> Option<Symbol> {
        self.interner.active(ch).map(SymbolId::symbol)
    }

    #[must_use]
    pub fn frozen_primitive_meaning(&self, token: crate::token::Token) -> Option<Meaning> {
        let crate::token::Token::Frozen(frozen) = token else {
            return None;
        };
        self.primitive_meanings
            .get(usize::from(frozen.primitive_index()?))
            .copied()
    }

    #[must_use]
    pub fn primitive_name(&self, meaning: Meaning) -> Option<&'a str> {
        self.primitive_meanings
            .iter()
            .position(|&candidate| candidate == meaning)
            .map(|index| self.primitive_names[index].as_str())
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
    pub fn provenance(&self, id: ProvenanceId<G>) -> OriginRecord {
        self.admitted.provenance(id)
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
    pub fn int_param(&self, parameter: IntParam) -> Result<i32, StateError> {
        self.admitted.state().integer_parameter(parameter)
    }

    #[inline(always)]
    pub fn token_register(&self, index: u16) -> Result<Option<TokenListId<G>>, StateError> {
        self.admitted.state().token_register(index)
    }

    #[inline(always)]
    pub fn token_parameter(
        &self,
        parameter: crate::env::banks::TokParam,
    ) -> Result<Option<TokenListId<G>>, StateError> {
        self.admitted.state().token_parameter(parameter)
    }

    #[inline(always)]
    pub fn glue_register(&self, index: u16) -> Result<Option<GlueId<G>>, StateError> {
        self.admitted.state().glue_register(index)
    }

    #[inline(always)]
    pub fn box_register(&self, index: u16) -> Result<Option<DurableListId<G>>, StateError> {
        self.admitted.state().box_register(index)
    }

    #[inline(always)]
    pub fn node_list(
        &self,
        id: DurableListId<G>,
    ) -> Result<NodeList<'a, G, GlueId<G>, TokenListId<G>>, NodeArenaError> {
        self.admitted.node_list(id)
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
