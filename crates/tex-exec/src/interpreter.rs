//! Persistent ownership and borrow-scoped access for canonical command work.
//!
//! The command interpreter is a session object. A [`CommandProcessor`] is
//! only its temporary borrow facade over the matching [`Universe`] command
//! context; semantic apply drops that facade so the stomach can borrow the
//! rest of `Universe`. Keeping that distinction explicit prevents a facade
//! construction from becoming a second command-state owner.

use std::ops::{Deref, DerefMut};

use tex_command::{
    CommandFuel, CommandHostContext, CommandObserver, CommandProcessor, CommandProfile,
    CommandState,
};
use tex_state::CommandContext;

/// The sole session-lived owner of canonical command state.
#[derive(Debug)]
pub(crate) struct PersistentInterpreter<G> {
    state: CommandState<G>,
    lifecycle: InterpreterLifecycle,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct InterpreterLifecycle {
    processor_entries: u64,
    processor_completions: u64,
    live_processors: u8,
    maximum_live_processors: u8,
}

/// Assertion-bearing lifecycle evidence for focused architecture controls.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct InterpreterLifecycleStats {
    pub processor_entries: u64,
    pub processor_completions: u64,
    pub live_processors: u8,
    pub maximum_live_processors: u8,
}

impl<G> Default for PersistentInterpreter<G> {
    fn default() -> Self {
        Self::new(CommandProfile::default())
    }
}

impl<G> PersistentInterpreter<G> {
    /// Creates the one canonical interpreter owned by an engine session.
    pub(crate) fn new(profile: CommandProfile) -> Self {
        #[cfg(feature = "profiling")]
        tex_state::measurement::record_hot_core_interpreter_construction();
        #[cfg(feature = "profiling")]
        let _allocation_scope = tex_state::measurement::hot_core_allocation_scope(
            tex_state::measurement::HotCoreAllocationOwner::InterpreterConstruction,
        );
        Self {
            state: CommandState::new(profile),
            lifecycle: InterpreterLifecycle::default(),
        }
    }

    pub(crate) const fn state(&self) -> &CommandState<G> {
        &self.state
    }

    pub(crate) fn state_mut(&mut self) -> &mut CommandState<G> {
        &mut self.state
    }

    /// Borrows the persistent interpreter for one uninterrupted command
    /// delivery/scanner scope.
    ///
    /// Rust prevents overlapping mutable borrows. The explicit lifecycle
    /// assertion additionally makes that invariant measurable and ensures
    /// every facade is retired before a semantic, rollback, or host barrier.
    pub(crate) fn processor<'a>(
        &'a mut self,
        state: CommandContext<'a, G>,
        host: CommandHostContext<'a>,
        fuel: &'a mut CommandFuel,
        observer: Option<&'a mut dyn CommandObserver>,
    ) -> InterpreterProcessor<'a, G> {
        #[cfg(feature = "profiling")]
        tex_state::measurement::record_hot_core_interpreter_operation_entry();
        #[cfg(feature = "profiling")]
        let _allocation_scope = tex_state::measurement::hot_core_allocation_scope(
            tex_state::measurement::HotCoreAllocationOwner::InterpreterBorrow,
        );
        let Self {
            state: command,
            lifecycle,
        } = self;
        assert_eq!(
            lifecycle.live_processors, 0,
            "canonical interpreter processor borrows cannot overlap"
        );
        lifecycle.processor_entries = lifecycle.processor_entries.saturating_add(1);
        lifecycle.live_processors = 1;
        lifecycle.maximum_live_processors = lifecycle.maximum_live_processors.max(1);

        let processor = CommandProcessor::borrowed(command, state, host, fuel, observer);
        InterpreterProcessor {
            processor,
            lifecycle,
        }
    }

    #[cfg(test)]
    pub(crate) fn lifecycle_stats(&self) -> InterpreterLifecycleStats {
        InterpreterLifecycleStats {
            processor_entries: self.lifecycle.processor_entries,
            processor_completions: self.lifecycle.processor_completions,
            live_processors: self.lifecycle.live_processors,
            maximum_live_processors: self.lifecycle.maximum_live_processors,
        }
    }
}

impl<G> Deref for PersistentInterpreter<G> {
    type Target = CommandState<G>;

    fn deref(&self) -> &Self::Target {
        &self.state
    }
}

impl<G> DerefMut for PersistentInterpreter<G> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.state
    }
}

/// One borrow-scoped facade over the persistent canonical interpreter.
pub(crate) struct InterpreterProcessor<'a, G> {
    processor: CommandProcessor<'a, G>,
    lifecycle: &'a mut InterpreterLifecycle,
}

impl<'a, G> Deref for InterpreterProcessor<'a, G> {
    type Target = CommandProcessor<'a, G>;

    fn deref(&self) -> &Self::Target {
        &self.processor
    }
}

impl<G> DerefMut for InterpreterProcessor<'_, G> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.processor
    }
}

impl<G> Drop for InterpreterProcessor<'_, G> {
    fn drop(&mut self) {
        assert_eq!(
            self.lifecycle.live_processors, 1,
            "canonical interpreter processor lifecycle is unbalanced"
        );
        self.lifecycle.live_processors = 0;
        self.lifecycle.processor_completions =
            self.lifecycle.processor_completions.saturating_add(1);
    }
}
