//! Aggregate state access reserved for the canonical command processor.
//!
//! This boundary is deliberately interpretation-neutral: it owns no command
//! state and decides no command behavior. Operations are added here only when
//! they represent typed reads or mutations of [`Universe`] state.

use crate::{
    ChangedAt, DependencyKey, DependencyValue, TracedTokenList, Universe,
    env::banks::{IntParam, TokParam},
    ids::FontId,
    ids::{MacroDefinitionId, OriginListId, TokenListId},
    interner::Symbol,
    macro_store::{MacroDefinitionProvenance, MacroMeaning, MacroParameterPattern},
    meaning::Meaning,
    page::PageMark,
    provenance::SynthesizedOriginKind,
    token::{Catcode, OriginId, Token, TracedTokenWord},
};

/// Borrow-scoped aggregate access to live TeX state.
///
/// Construct this through [`Universe::command_context`]. The private
/// `Universe` borrow prevents consumers from retaining the context in a
/// snapshot or bypassing the aggregate mutation boundary.
#[derive(Debug)]
pub struct CommandContext<'a> {
    universe: &'a mut Universe,
}

impl CommandContext<'_> {
    /// Reads one catcode through the aggregate code-table boundary.
    #[must_use]
    pub fn catcode(&mut self, ch: char) -> Catcode {
        self.universe.catcode(ch)
    }

    /// Interns a control-sequence spelling without assigning it a meaning.
    #[must_use]
    pub fn intern_control_sequence(&mut self, name: &str) -> Symbol {
        self.universe.intern(name).symbol()
    }

    /// Interns a name built by `\\csname` and installs TeX's `\\relax`
    /// meaning only when that name was previously undefined.
    #[must_use]
    pub fn intern_relaxed_control_sequence(&mut self, name: &str) -> Symbol {
        self.universe.intern_relaxed_control_sequence(name).symbol()
    }

    /// Returns the immutable semantic words of one stored token list.
    #[must_use]
    pub fn tokens(&self, id: TokenListId) -> &[Token] {
        self.universe.tokens(id)
    }

    /// Freezes a scanner-owned traced token sequence through the aggregate
    /// content and provenance stores.  Command scanners may allocate their
    /// immutable result, but retain no wider store or host capability.
    #[must_use]
    pub fn finish_traced_token_list(&mut self, tokens: &[TracedTokenWord]) -> TracedTokenList {
        self.universe.finish_traced_token_list(tokens)
    }

    /// Reads the immutable spelling of a control sequence.
    #[must_use]
    pub fn resolve(&self, symbol: Symbol) -> &str {
        self.universe.resolve(symbol)
    }

    /// Reads one integer parameter for canonical expandable conversion.
    #[must_use]
    pub fn int_param(&self, param: IntParam) -> i32 {
        self.universe.int_param(param)
    }

    /// Reads one token parameter for direct `\\the` insertion.
    #[must_use]
    pub fn tok_param(&self, param: TokParam) -> TokenListId {
        self.universe.tok_param(param)
    }

    /// Reads one count register for canonical expandable conversion.
    #[must_use]
    pub fn count(&self, index: u16) -> i32 {
        self.universe.count(index)
    }

    /// Reads one token register for direct `\\the` insertion.
    #[must_use]
    pub fn toks(&self, index: u16) -> TokenListId {
        self.universe.toks(index)
    }

    /// Reads one TeX82 page-mark slot for expandable mark retrieval.
    #[must_use]
    pub fn page_mark(&self, mark: PageMark) -> TokenListId {
        self.universe.page_mark(mark)
    }

    /// Reads one e-TeX mark-class slot for expandable mark retrieval.
    #[must_use]
    pub fn page_mark_class(&self, mark: PageMark, class: u16) -> TokenListId {
        self.universe.page_mark_class(mark, class)
    }

    /// Reads a font's immutable external name for `\\fontname` and meaning.
    #[must_use]
    pub fn font_name(&self, font: FontId) -> String {
        self.universe.font_name(font)
    }

    /// Returns the parallel provenance words of one stored token list.
    #[must_use]
    pub fn origin_list(&self, id: OriginListId) -> &[OriginId] {
        self.universe.origin_list(id)
    }
    /// Returns the mutation stamp for a typed aggregate-state dependency.
    #[must_use]
    pub fn dependency_changed_at(&self, key: DependencyKey) -> ChangedAt {
        self.universe.dependency_changed_at(key)
    }

    /// Records a typed aggregate-state read.
    pub fn track_dependency(&mut self, key: DependencyKey) -> ChangedAt {
        self.universe.track_dependency(key)
    }

    /// Reads the detached semantic value for a typed aggregate-state
    /// dependency.
    #[must_use]
    pub fn semantic_dependency_value(&self, key: DependencyKey) -> Option<DependencyValue> {
        self.universe.semantic_dependency_value(key)
    }

    /// Resolves a control sequence's current meaning and records that semantic
    /// read for the active dependency region.
    #[must_use]
    pub fn meaning(&mut self, symbol: Symbol) -> Meaning {
        self.universe
            .track_dependency(DependencyKey::Meaning(symbol.raw()));
        self.universe.meaning(symbol)
    }

    /// Interns the distinct control sequence represented by an active
    /// character, if it has not already been interned.
    #[must_use]
    pub fn intern_active_character(&mut self, ch: char) -> Symbol {
        self.universe.intern_active_character(ch).symbol()
    }

    /// Resolves an engine-owned frozen token without consulting a mutable
    /// control-sequence meaning cell.
    #[must_use]
    pub fn frozen_primitive_meaning(&self, token: Token) -> Option<Meaning> {
        self.universe.frozen_primitive_meaning(token)
    }

    /// Returns the immutable replay token for a registered primitive.
    ///
    /// Command recovery uses this rather than a mutable control-sequence
    /// spelling for TeX's `frozen_cr`, `frozen_fi`, and `frozen_par` tokens.
    #[must_use]
    pub fn primitive_token(&self, name: &str) -> Option<Token> {
        self.universe.primitive_token(name)
    }

    /// Returns diagnostic provenance retained beside an immutable macro
    /// definition. This is deliberately separate from the definition's
    /// semantic token lists.
    #[must_use]
    pub fn macro_definition_provenance(
        &self,
        definition: MacroDefinitionId,
    ) -> MacroDefinitionProvenance {
        self.universe.macro_definition_provenance(definition)
    }

    /// Reads one immutable macro definition through the command-state boundary.
    #[must_use]
    pub fn macro_definition(&self, definition: MacroDefinitionId) -> MacroMeaning {
        self.universe.macro_definition(definition)
    }

    /// Reads the prevalidated parameter-marker layout for one macro definition.
    #[must_use]
    pub fn macro_definition_parameter_pattern(
        &self,
        definition: MacroDefinitionId,
    ) -> MacroParameterPattern {
        self.universe.macro_definition_parameter_pattern(definition)
    }

    /// Allocates one rollback-coupled macro invocation node.
    ///
    /// The command machine supplies the live parent invocation from its
    /// activation stack. The aggregate state owns arena allocation, so an
    /// invocation frame never stores an arena handle outside the usual origin
    /// representation.
    pub fn macro_invocation_origin(
        &mut self,
        definition: MacroDefinitionId,
        invocation: OriginId,
        definition_origin: OriginId,
        parent_invocation: OriginId,
    ) -> OriginId {
        self.universe.macro_invocation_origin(
            definition,
            invocation,
            definition_origin,
            parent_invocation,
        )
    }

    /// Allocates provenance for a token manufactured by canonical expansion.
    pub fn synthesized_origin(
        &mut self,
        kind: SynthesizedOriginKind,
        parent: OriginId,
    ) -> OriginId {
        self.universe.synthesized_origin(kind, parent)
    }
}

impl Universe {
    /// Borrows the interpretation-neutral aggregate boundary used by the
    /// canonical command processor.
    pub fn command_context(&mut self) -> CommandContext<'_> {
        CommandContext { universe: self }
    }
}
