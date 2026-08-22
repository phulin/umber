use std::alloc::System;
use std::hint::black_box;
use std::sync::Arc;

use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};
use tex_command::{
    CommandHostCapabilities, CommandHostContext, CommandProcessor, CommandState,
    RegisteredSourceKind, SourceRegistration,
};
use tex_state::Universe;
use tex_state::env::AssignmentScope;
use tex_state::interner::InternerBudget;
use tex_state::meaning::{MeaningFlags, MeaningWord};
use tex_state::token::{Catcode, Token, TokenWord};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

fn main() {
    ordinary_source_delivery();
    packed_backup_and_replay();
    stored_token_replay();
    macro_argument_matching();
    println!("packed token/macro cutover gate: PASS");
}

fn ordinary_source_delivery() {
    with_universe(|universe| {
        let mut command = CommandState::default();
        open_source(&mut command, "ssssssssssssssss");
        let mut capabilities = CommandHostCapabilities::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut processor = processor(
            universe,
            &mut command,
            &mut capabilities,
            &mut diagnostic_effects,
        );
        for _ in 0..3 {
            assert_char(processor.get_next().unwrap().unwrap(), 's');
        }
        measure_zero("ordinary_source_delivery", || {
            assert_char(processor.get_next().unwrap().unwrap(), 's');
        });
    });
}

fn packed_backup_and_replay() {
    with_universe(|universe| {
        let mut command = CommandState::default();
        open_source(&mut command, "bbbbbbbbbbbbbbbb");
        let mut capabilities = CommandHostCapabilities::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut processor = processor(
            universe,
            &mut command,
            &mut capabilities,
            &mut diagnostic_effects,
        );
        for _ in 0..2 {
            let delivered = processor.get_next().unwrap().unwrap();
            processor.back_input(delivered).unwrap();
            assert_char(processor.get_next().unwrap().unwrap(), 'b');
        }
        let delivered = processor.get_next().unwrap().unwrap();
        measure_zero("packed_backup_and_replay", || {
            processor.back_input(delivered).unwrap();
            assert_char(processor.get_next().unwrap().unwrap(), 'b');
        });
    });
}

fn stored_token_replay() {
    with_universe(|universe| {
        let words = (0..16)
            .map(|_| {
                TokenWord::pack(Token::Char {
                    ch: 't',
                    cat: Catcode::Letter,
                })
            })
            .collect::<Vec<_>>();
        let stored = universe.allocate_token_list(&words).expect("stored tokens");
        let mut command = CommandState::default();
        {
            let context = universe.command_context().expect("command context");
            command.push_everyjob(&context, stored);
        }
        let mut capabilities = CommandHostCapabilities::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut processor = processor(
            universe,
            &mut command,
            &mut capabilities,
            &mut diagnostic_effects,
        );
        for _ in 0..3 {
            assert_char(processor.get_token().unwrap().unwrap(), 't');
        }
        measure_zero("stored_token_replay", || {
            assert_char(processor.get_token().unwrap().unwrap(), 't');
        });
    });
}

fn macro_argument_matching() {
    with_universe(|universe| {
        install_macro(universe);
        let mut command = CommandState::default();
        open_source(
            &mut command,
            r"\m{abcdefghijklmnop}\m{abcdefghijklmnop}\m{abcdefghijklmnop}",
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut processor = processor(
            universe,
            &mut command,
            &mut capabilities,
            &mut diagnostic_effects,
        );
        for _ in 0..32 {
            black_box(processor.get_x_token().unwrap().unwrap());
        }
        let pending = processor.get_next().unwrap().unwrap();
        processor.back_input(pending).unwrap();
        measure_zero("macro_matching_replay_expansion", || {
            assert_char(processor.get_x_token().unwrap().unwrap(), 'a');
        });
    });
}

fn with_universe(test: impl for<'id> FnOnce(&mut Universe<tex_state::GenerationBrand<'id>>)) {
    let budget = InternerBudget::new(4_096, 4_096, 1 << 20).expect("benchmark interner budget");
    tex_state::with_universe(budget, test).expect("benchmark universe");
}

fn open_source<G>(command: &mut CommandState<G>, source: &str) {
    let registered = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(source.as_bytes()),
        ))
        .unwrap();
    command.open_registered_source(registered).unwrap();
}

fn processor<'a, G>(
    universe: &'a mut Universe<G>,
    command: &'a mut CommandState<G>,
    capabilities: &'a mut CommandHostCapabilities,
    diagnostic_effects: &'a mut tex_state::diagnostic::DiagnosticEffects,
) -> CommandProcessor<'a, 'a, G> {
    CommandProcessor::new(
        command,
        universe.command_context().expect("command context"),
        CommandHostContext::new(capabilities),
        diagnostic_effects,
    )
}

fn install_macro<G>(universe: &mut Universe<G>) {
    let name = universe.intern("m").expect("macro name");
    let definition = universe
        .allocate_definition(
            &[TokenWord::pack(Token::param(1))],
            &[TokenWord::pack(Token::param(1))],
        )
        .expect("macro definition");
    universe
        .assign_meaning(
            name,
            MeaningWord::macro_definition(MeaningFlags::EMPTY, definition),
            AssignmentScope::Global,
        )
        .expect("macro meaning");
}

fn assert_char<G>(command: tex_command::CurrentCommand<G>, expected: char) {
    assert_eq!(
        command.spelling().semantic_token(),
        Token::Char {
            ch: expected,
            cat: Catcode::Letter,
        }
    );
}

fn measure_zero(name: &str, operation: impl FnOnce()) {
    let region = Region::new(GLOBAL);
    operation();
    let stats = region.change();
    assert_eq!(stats.allocations, 0, "{name}: allocation calls");
    assert_eq!(stats.bytes_allocated, 0, "{name}: requested bytes");
    println!("{name} allocations=0 requested_bytes=0");
}
