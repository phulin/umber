//! Persistent ownership and borrow-scoped access for canonical command work.
//!
//! The command interpreter is a session object. A [`CommandProcessor`] is
//! its temporary exclusive borrow over the matching admitted command context.
//! Dropping the real processor releases scanner access and cleans actual
//! command scratch; no lifecycle wrapper or counters duplicate Rust's loan.

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
        }
    }

    pub(crate) fn from_state(state: CommandState<G>) -> Self {
        Self { state }
    }

    pub(crate) fn into_state(self) -> CommandState<G> {
        self.state
    }

    pub(crate) const fn state(&self) -> &CommandState<G> {
        &self.state
    }

    pub(crate) fn state_mut(&mut self) -> &mut CommandState<G> {
        &mut self.state
    }

    /// Exclusively borrows the persistent command state and admitted owners.
    pub(crate) fn processor<'episode, 'admission, Facts: tex_command::CommandHostFacts<G>>(
        &'episode mut self,
        state: &'episode mut CommandContext<'admission, G>,
        host: CommandHostContext<'episode, G, Facts>,
        fuel: &'episode mut CommandFuel,
        observer: Option<&'episode mut dyn CommandObserver>,
        diagnostic_effects: &'episode mut tex_state::diagnostic::DiagnosticEffects,
    ) -> CommandProcessor<'episode, 'admission, G, Facts> {
        #[cfg(feature = "profiling")]
        tex_state::measurement::record_hot_core_interpreter_operation_entry();
        #[cfg(feature = "profiling")]
        let _allocation_scope = tex_state::measurement::hot_core_allocation_scope(
            tex_state::measurement::HotCoreAllocationOwner::InterpreterBorrow,
        );
        CommandProcessor::new(
            &mut self.state,
            state,
            host,
            fuel,
            observer,
            diagnostic_effects,
        )
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

#[cfg(test)]
mod tests {
    use super::*;
    use tex_command::{CommandFuelLedger, CommandHostCapabilities};

    fn round_trip_context<'admission, G>(
        interpreter: &mut PersistentInterpreter<G>,
        context: &mut CommandContext<'admission, G>,
    ) {
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let _processor = interpreter.processor(
            context,
            CommandHostContext::new(&mut capabilities),
            fuel.fuel_mut(),
            None,
            &mut diagnostic_effects,
        );
    }

    #[test]
    fn admission_survives_the_shorter_processor_episode() {
        crate::test_harness::with_nonstop_universe(|universe| {
            let mut interpreter = PersistentInterpreter::default();
            let mut context = universe.command_context().expect("command admission");
            round_trip_context(&mut interpreter, &mut context);

            assert_eq!(context.execution_group_depth(), 0);
            round_trip_context(&mut interpreter, &mut context);
            assert_eq!(context.execution_group_depth(), 0);
        });
    }
}
