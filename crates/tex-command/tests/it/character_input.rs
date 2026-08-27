use std::sync::Arc;

use tex_command::{
    CommandFuelLedger, CommandHostCapabilities, CommandHostContext, CommandProcessor, CommandState,
    RegisteredSourceKind, SourceRegistration,
};
use tex_state::interner::InternerBudget;
use tex_state::meaning::{Meaning, ResolvedMeaning};
use tex_state::token::Catcode;

fn budget() -> InternerBudget {
    InternerBudget::new(128, 128, 16 * 1024).expect("test budget")
}

#[test]
fn external_command_boundary_delivers_registered_source_characters() {
    tex_state::with_universe(budget(), |universe| {
        let mut command = CommandState::default();
        let source = command
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(&b"A"[..]),
            ))
            .expect("source registration");
        command.open_registered_source(source).expect("source open");
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = CommandProcessor::new(
            &mut command,
            &mut context,
            CommandHostContext::new(&mut capabilities),
            fuel.fuel_mut(),
            None,
            &mut diagnostic_effects,
        );

        assert_eq!(
            processor
                .get_next()
                .expect("delivery")
                .expect("command")
                .meaning(),
            ResolvedMeaning::Static(Meaning::CharToken {
                ch: 'A',
                cat: Catcode::Letter,
            })
        );
    })
    .expect("universe");
}
