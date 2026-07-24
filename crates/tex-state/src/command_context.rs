//! Aggregate state access reserved for the canonical command processor.
//!
//! This boundary is deliberately interpretation-neutral: it owns no command
//! state and decides no command behavior. Operations are added here only when
//! they represent typed reads or mutations of [`Universe`] state.

use crate::{ChangedAt, DependencyKey, DependencyValue, Universe};

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
}

impl Universe {
    /// Borrows the interpretation-neutral aggregate boundary used by the
    /// canonical command processor.
    pub fn command_context(&mut self) -> CommandContext<'_> {
        CommandContext { universe: self }
    }
}
