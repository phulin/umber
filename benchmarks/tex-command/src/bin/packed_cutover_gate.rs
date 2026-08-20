use std::alloc::System;
use std::hint::black_box;
use std::sync::Arc;

use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};
use tex_command::{
    CommandHostCapabilities, CommandHostContext, CommandProcessor, CommandState,
    RegisteredSourceKind, SourceRegistration,
};
use tex_state::Universe;
use tex_state::macro_store::MacroMeaning;
use tex_state::meaning::{Meaning, MeaningFlags};
use tex_state::measurement::{HotCoreCensus, hot_core_census};
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

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
    let (mut universe, mut command, mut capabilities) = source_case("ssssssssssssssss");
    let mut processor = processor(&mut universe, &mut command, &mut capabilities);
    for _ in 0..3 {
        assert_char(processor.get_next().unwrap().unwrap(), 's');
    }
    measure_zero("ordinary_source_delivery", || {
        assert_char(processor.get_next().unwrap().unwrap(), 's');
    });
}

fn packed_backup_and_replay() {
    let (mut universe, mut command, mut capabilities) = source_case("bbbbbbbbbbbbbbbb");
    let mut processor = processor(&mut universe, &mut command, &mut capabilities);
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
}

fn stored_token_replay() {
    let mut universe = Universe::new_with_plain_catcodes();
    let words = (0..16)
        .map(|_| {
            TracedTokenWord::pack(
                Token::Char {
                    ch: 't',
                    cat: Catcode::Letter,
                },
                OriginId::UNKNOWN,
            )
        })
        .collect::<Vec<_>>();
    let stored = universe.finish_traced_token_list(&words);
    let mut command = CommandState::default();
    command.push_everyjob(&universe.command_context(), stored);
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut universe, &mut command, &mut capabilities);
    for _ in 0..3 {
        assert_char(processor.get_token().unwrap().unwrap(), 't');
    }
    measure_zero("stored_token_replay", || {
        assert_char(processor.get_token().unwrap().unwrap(), 't');
    });
}

fn macro_argument_matching() {
    let mut universe = Universe::new_with_plain_catcodes();
    install_macro(&mut universe);
    let mut command = CommandState::default();
    open_source(
        &mut command,
        r"\m{abcdefghijklmnop}\m{abcdefghijklmnop}\m{abcdefghijklmnop}",
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut universe, &mut command, &mut capabilities);
    for _ in 0..32 {
        black_box(processor.get_x_token().unwrap().unwrap());
    }
    let pending = processor.get_next().unwrap().unwrap();
    processor.back_input(pending).unwrap();
    measure_zero("macro_matching_replay_expansion", || {
        assert_char(processor.get_x_token().unwrap().unwrap(), 'a');
    });
}

fn source_case(source: &str) -> (Universe, CommandState, CommandHostCapabilities) {
    let universe = Universe::new_with_plain_catcodes();
    let mut command = CommandState::default();
    open_source(&mut command, source);
    (universe, command, CommandHostCapabilities::default())
}

fn open_source(command: &mut CommandState, source: &str) {
    let registered = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(source.as_bytes()),
        ))
        .unwrap();
    command.open_registered_source(registered).unwrap();
}

fn processor<'a>(
    universe: &'a mut Universe,
    command: &'a mut CommandState,
    capabilities: &'a mut CommandHostCapabilities,
) -> CommandProcessor<'a> {
    CommandProcessor::new(
        command,
        universe.command_context(),
        CommandHostContext::new(capabilities),
    )
}

fn install_macro(universe: &mut Universe) {
    let name = universe.intern("m").symbol();
    let parameters = universe.intern_token_list_ref(&[Token::param(1)]);
    let replacement = universe.intern_token_list_ref(&[Token::param(1)]);
    let definition = universe.intern_macro(MacroMeaning::new(
        MeaningFlags::EMPTY,
        parameters.id(),
        replacement.id(),
    ));
    universe.set_meaning(
        name,
        Meaning::Macro {
            flags: MeaningFlags::EMPTY,
            definition: definition.id(),
        },
    );
}

fn assert_char(command: tex_command::CurrentCommand, expected: char) {
    assert_eq!(
        command.spelling().semantic_token(),
        Token::Char {
            ch: expected,
            cat: Catcode::Letter,
        }
    );
}

fn measure_zero(name: &str, operation: impl FnOnce()) {
    let before = hot_core_census();
    let region = Region::new(GLOBAL);
    operation();
    let stats = region.change();
    let delta = hot_core_census().saturating_sub(before);
    assert_eq!(stats.allocations, 0, "{name}: allocation calls");
    assert_eq!(stats.bytes_allocated, 0, "{name}: requested bytes");
    assert_zero_ownership(name, delta);
    println!(
        "{name} allocations=0 requested_bytes=0 arc_retains=0 weak_retains=0 weak_upgrades=0 weak_index_calls=0 content_hash_calls=0"
    );
}

fn assert_zero_ownership(name: &str, delta: HotCoreCensus) {
    assert_eq!(delta.weak_graph.arc_retains, 0, "{name}: Arc retains");
    assert_eq!(delta.weak_graph.weak_retains, 0, "{name}: weak retains");
    assert_eq!(
        delta.weak_graph.weak_upgrade_calls, 0,
        "{name}: weak upgrades"
    );
    assert_eq!(delta.weak_index.calls, 0, "{name}: weak-index calls");
    assert_eq!(
        delta.weak_index.candidate_entries, 0,
        "{name}: weak-index candidates"
    );
    assert_eq!(
        delta.weak_index.exact_comparisons, 0,
        "{name}: weak-index comparisons"
    );
    assert_eq!(
        delta.weak_index.content_hash_calls, 0,
        "{name}: content hashes"
    );
}
