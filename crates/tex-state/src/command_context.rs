//! Aggregate state access reserved for the canonical command processor.
//!
//! This boundary is deliberately interpretation-neutral: it owns no command
//! state and decides no command behavior. Operations are added here only when
//! they represent typed reads or mutations of [`Universe`] state.

use crate::{
    ChangedAt, DependencyKey, DependencyValue, Universe,
    ids::{OriginListId, TokenListId},
    interner::Symbol,
    meaning::Meaning,
    token::{Catcode, OriginId, Token},
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

    /// Returns the immutable semantic words of one stored token list.
    #[must_use]
    pub fn tokens(&self, id: TokenListId) -> &[Token] {
        self.universe.tokens(id)
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
}

impl Universe {
    /// Borrows the interpretation-neutral aggregate boundary used by the
    /// canonical command processor.
    pub fn command_context(&mut self) -> CommandContext<'_> {
        CommandContext { universe: self }
    }
}
