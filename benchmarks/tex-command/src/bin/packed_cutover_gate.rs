use std::hint::black_box;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tex_command::{
    CommandHostCapabilities, CommandHostContext, CommandProcessor, CommandState, DeliveryStatus,
    RegisteredSourceKind, SourceRegistration,
};
use tex_state::Universe;
use tex_state::env::AssignmentScope;
use tex_state::interner::InternerBudget;
use tex_state::meaning::{MeaningFlags, MeaningWord};
use tex_state::measurement::{
    HotCoreAllocationOwner, HotCoreAllocator, hot_core_allocation_scope,
    hot_core_allocation_trace_cursor, hot_core_allocation_trace_entry, hot_core_census,
};
use tex_state::token::{Catcode, Token, TokenWord};

#[global_allocator]
static GLOBAL: HotCoreAllocator = HotCoreAllocator;

fn main() {
    if std::env::args().any(|argument| argument == "--mixed-stored-only") {
        warmed_mixed_stored_cursor();
        return;
    }
    assert!(std::mem::size_of::<tex_state::DefinitionId<()>>() <= 8);
    assert!(std::mem::size_of::<tex_state::ResolvedMeaning<()>>() <= 24);
    assert!(std::mem::size_of::<tex_state::PrimitiveHandle<()>>() <= 16);
    assert!(std::mem::size_of::<tex_command::CurrentCommand<()>>() <= 144);
    assert!(std::mem::size_of::<DeliveryStatus>() <= 16);
    ordinary_source_delivery();
    packed_backup_and_replay();
    warmed_backup_push_pop_throughput();
    stored_token_replay();
    warmed_mixed_stored_cursor();
    warmed_short_interner_lookup();
    warmed_primitive_resolution();
    warmed_control_sequence_delivery();
    macro_argument_matching();
    warmed_keyword_mismatch_throughput();
    destination_directed_warm_delivery();
    println!("packed token/macro cutover gate: PASS");
}

fn warmed_mixed_stored_cursor() {
    const ROUNDS: u32 = 1_000_000;
    with_universe(|universe| {
        let mut benchmark = tex_command::MixedPackedCursorBenchmark::new(universe);
        let _ = benchmark.run(64);
        let mut receipt = None;
        let mut elapsed = Duration::ZERO;
        measure_zero("warmed_mixed_stored_cursor", || {
            let start = Instant::now();
            receipt = Some(black_box(benchmark.run(ROUNDS)));
            elapsed = start.elapsed();
        });
        let receipt = receipt.expect("mixed cursor receipt");
        assert_eq!(receipt.calls, u64::from(ROUNDS) * 5);
        assert_eq!(receipt.retirements, u64::from(ROUNDS / 4) * 5);
        assert_eq!(receipt.rollbacks, 1);
        println!(
            "warmed_mixed_stored_cursor calls={} retirements={} rollbacks={} checksum={} elapsed_ns={} ns_per_call={:.2}",
            receipt.calls,
            receipt.retirements,
            receipt.rollbacks,
            receipt.checksum,
            elapsed.as_nanos(),
            elapsed.as_nanos() as f64 / receipt.calls as f64,
        );
    });
}

fn warmed_primitive_resolution() {
    const OPERATIONS: usize = 1_000_000;
    with_universe(|universe| {
        tex_command::install_tex82_expandable_primitives(universe);
        tex_command::install_tex82_unexpandable_primitives(universe);
        tex_command::install_etex_expandable_primitives(universe);
        tex_command::install_etex_unexpandable_primitives(universe);
        tex_command::install_pdftex_expandable_primitives(universe);
        tex_command::install_pdftex_unexpandable_primitives(universe);
        let handle = universe
            .primitive_handle("pdfignoreddimen")
            .expect("pdfTeX ignored-depth primitive handle");
        let context = universe.command_context().expect("command context");

        let mut named = Duration::ZERO;
        measure_zero("name_primitive_resolution_1m", || {
            let start = Instant::now();
            for _ in 0..OPERATIONS {
                black_box(context.primitive_resolved(black_box("pdfignoreddimen")))
                    .expect("name-based primitive resolution");
            }
            named = start.elapsed();
        });

        let mut packed = Duration::ZERO;
        measure_zero("packed_primitive_resolution_1m", || {
            let start = Instant::now();
            for _ in 0..OPERATIONS {
                black_box(context.resolve_primitive_handle(black_box(handle)))
                    .expect("packed primitive resolution");
            }
            packed = start.elapsed();
        });
        println!(
            "primitive_resolution name_ns_per_op={:.2} packed_ns_per_op={:.2}",
            named.as_nanos() as f64 / OPERATIONS as f64,
            packed.as_nanos() as f64 / OPERATIONS as f64,
        );
    });
}

fn warmed_control_sequence_delivery() {
    const OPERATIONS: usize = 1_000_000;
    const NAME: &str = "deliveryidentity";
    with_universe(|universe| {
        let symbol = universe.intern(NAME).expect("control sequence name");
        let mut command = CommandState::default();
        open_source(&mut command, &format!(r"\{NAME} ").repeat(OPERATIONS));
        let mut capabilities = CommandHostCapabilities::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut processor = processor(
            universe,
            &mut command,
            &mut capabilities,
            &mut diagnostic_effects,
        );
        let start = Instant::now();
        for _ in 0..OPERATIONS {
            let delivered = processor.get_token().unwrap().unwrap();
            assert_eq!(delivered.control_sequence(), Some(symbol.symbol()));
            black_box(delivered);
        }
        let elapsed = start.elapsed();
        println!(
            "source_control_sequence_delivery throughput_ns_per_op={:.2}",
            elapsed.as_nanos() as f64 / OPERATIONS as f64
        );
    });

    with_universe(|universe| {
        let symbol = universe.intern(NAME).expect("control sequence name");
        let words = vec![TokenWord::pack(Token::Cs(symbol.symbol())); OPERATIONS];
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
        let mut elapsed = Duration::ZERO;
        measure_zero("stored_control_sequence_delivery_1m", || {
            let start = Instant::now();
            for _ in 0..OPERATIONS {
                let delivered = processor.get_token().unwrap().unwrap();
                assert_eq!(delivered.control_sequence(), Some(symbol.symbol()));
                black_box(delivered);
            }
            elapsed = start.elapsed();
        });
        println!(
            "stored_control_sequence_delivery throughput_ns_per_op={:.2}",
            elapsed.as_nanos() as f64 / OPERATIONS as f64
        );
    });
}

fn warmed_backup_push_pop_throughput() {
    const OPERATIONS: usize = 1_000_000;
    with_universe(|universe| {
        let mut command = CommandState::default();
        open_source(&mut command, "b");
        let mut capabilities = CommandHostCapabilities::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut processor = processor(
            universe,
            &mut command,
            &mut capabilities,
            &mut diagnostic_effects,
        );
        let mut delivered = processor.get_next().unwrap().unwrap();
        for _ in 0..4_096 {
            processor.back_input(delivered).unwrap();
            delivered = processor.get_next().unwrap().unwrap();
        }
        let mut elapsed = Duration::ZERO;
        measure_zero("warmed_backup_push_pop_1m", || {
            let start = Instant::now();
            for _ in 0..OPERATIONS {
                processor.back_input(delivered).unwrap();
                delivered = processor.get_next().unwrap().unwrap();
            }
            elapsed = start.elapsed();
            black_box(&delivered);
        });
        println!(
            "warmed_backup_push_pop throughput_ns_per_op={:.2}",
            elapsed.as_nanos() as f64 / OPERATIONS as f64
        );
    });
}

fn warmed_keyword_mismatch_throughput() {
    const OPERATIONS: usize = 16_384;
    with_universe(|universe| {
        let mut command = CommandState::default();
        open_source(&mut command, &"dimensiox".repeat(OPERATIONS + 1));
        let mut capabilities = CommandHostCapabilities::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut processor = processor(
            universe,
            &mut command,
            &mut capabilities,
            &mut diagnostic_effects,
        );
        let run = |processor: &mut CommandProcessor<'_, '_, _>| {
            let tex_command::RetainedScalarScan::Complete(scanned) =
                processor.scan_keyword_retained("dimension")
            else {
                panic!("preloaded keyword scan must complete synchronously")
            };
            assert!(!scanned.value);
            for _ in 0..9 {
                black_box(processor.get_x_token().unwrap().unwrap());
            }
        };
        run(&mut processor);
        let mut elapsed = Duration::ZERO;
        measure_zero("warmed_keyword_mismatch_16384", || {
            let start = Instant::now();
            for _ in 0..OPERATIONS {
                run(&mut processor);
            }
            elapsed = start.elapsed();
        });
        println!(
            "warmed_keyword_mismatch throughput_ns_per_op={:.2}",
            elapsed.as_nanos() as f64 / OPERATIONS as f64
        );
    });
}

fn destination_directed_warm_delivery() {
    with_universe(|universe| {
        let words = vec![
            TokenWord::pack(Token::Char {
                ch: 'd',
                cat: Catcode::Letter,
            });
            8_256
        ];
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
        let mut destination = None;
        for _ in 0..64 {
            assert_eq!(
                processor.get_token_into(&mut destination).unwrap(),
                DeliveryStatus::Command
            );
            assert_char_ref(destination.as_ref().expect("direct command"), 'd');
            destination = None;
        }
        measure_zero("destination_directed_8192_delivery", || {
            for _ in 0..8_192 {
                assert_eq!(
                    processor.get_token_into(&mut destination).unwrap(),
                    DeliveryStatus::Command
                );
                assert_char_ref(destination.as_ref().expect("direct command"), 'd');
                destination = None;
            }
        });
    });
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
    const DELIVERIES: usize = 1_000_000;
    with_universe(|universe| {
        let words = (0..DELIVERIES + 64)
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
        let mut destination = None;
        for _ in 0..64 {
            assert_eq!(
                processor.get_token_into(&mut destination).unwrap(),
                DeliveryStatus::Command
            );
            assert_char_ref(destination.as_ref().expect("stored command"), 't');
            destination = None;
        }
        let mut elapsed = Duration::ZERO;
        measure_zero("stored_token_replay", || {
            let start = Instant::now();
            for _ in 0..DELIVERIES {
                assert_eq!(
                    processor.get_token_into(&mut destination).unwrap(),
                    DeliveryStatus::Command
                );
                assert_char_ref(destination.as_ref().expect("stored command"), 't');
                destination = None;
            }
            elapsed = start.elapsed();
        });
        println!(
            "stored_token_replay throughput_ns_per_token={:.2}",
            elapsed.as_nanos() as f64 / DELIVERIES as f64
        );
    });
}

fn warmed_short_interner_lookup() {
    const LOOKUPS: usize = 1_000_000;
    with_universe(|universe| {
        let par = universe.intern("par").expect("paragraph symbol").symbol();
        let context = universe.command_context().expect("command context");
        for _ in 0..64 {
            assert_eq!(context.symbol("par"), Some(par));
        }
        let mut elapsed = Duration::ZERO;
        measure_zero("warmed_short_interner_lookup", || {
            let start = Instant::now();
            for _ in 0..LOOKUPS {
                assert_eq!(black_box(context.symbol(black_box("par"))), Some(par));
            }
            elapsed = start.elapsed();
        });
        println!(
            "warmed_short_interner_lookup throughput_ns_per_lookup={:.2}",
            elapsed.as_nanos() as f64 / LOOKUPS as f64
        );
    });
}

fn macro_argument_matching() {
    with_universe(|universe| {
        install_macro(universe);
        let mut command = CommandState::default();
        open_source(
            &mut command,
            r"\m{abcdefghijklmnop}\m{abcdefghijklmnop}\m{abcdefghijklmnop}\m{abcdefghijklmnop}\m{abcdefghijklmnop}\m{abcdefghijklmnop}",
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut processor = processor(
            universe,
            &mut command,
            &mut capabilities,
            &mut diagnostic_effects,
        );
        for _ in 0..48 {
            black_box(processor.get_x_token().unwrap().unwrap());
        }
        for _ in 0..2 {
            let replay_warmup = processor.get_next().unwrap().unwrap();
            processor.back_input(replay_warmup).unwrap();
            assert_char(processor.get_x_token().unwrap().unwrap(), 'a');
            for _ in 0..15 {
                black_box(processor.get_x_token().unwrap().unwrap());
            }
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
    {
        let mut context = universe.command_context().expect("command context");
        for (character, catcode) in [('{', Catcode::BeginGroup), ('}', Catcode::EndGroup)] {
            context
                .assign_code(
                    tex_state::CodeTableKind::Catcode,
                    character,
                    i64::from(catcode as u8),
                    AssignmentScope::Global,
                )
                .expect("macro benchmark category code");
        }
    }
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
    assert_char_ref(&command, expected);
}

fn assert_char_ref<G>(command: &tex_command::CurrentCommand<G>, expected: char) {
    assert_eq!(
        command.spelling().semantic_token(),
        Token::Char {
            ch: expected,
            cat: Catcode::Letter,
        }
    );
}

fn measure_zero(name: &str, operation: impl FnOnce()) {
    let baseline = hot_core_census();
    let trace_start = hot_core_allocation_trace_cursor();
    let scope = hot_core_allocation_scope(HotCoreAllocationOwner::DeliveryAndScan);
    operation();
    drop(scope);
    let trace_end = hot_core_allocation_trace_cursor();
    let census = hot_core_census().saturating_sub(baseline);
    let allocations = census
        .allocations
        .iter()
        .map(|measurement| measurement.calls)
        .sum::<u64>();
    let requested_bytes = census
        .allocations
        .iter()
        .map(|measurement| measurement.requested_bytes)
        .sum::<u64>();
    let attribution = HotCoreAllocationOwner::NAMES
        .iter()
        .zip(census.allocations)
        .filter(|(_, measurement)| measurement.calls != 0)
        .map(|(owner, measurement)| {
            format!(
                "{owner}={}/{}",
                measurement.calls, measurement.requested_bytes
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let trace = (trace_start..trace_end)
        .filter_map(hot_core_allocation_trace_entry)
        .collect::<Vec<_>>();
    assert_eq!(
        allocations, 0,
        "{name}: allocation calls (requested_bytes={requested_bytes}; attribution={attribution}; trace={trace:?})"
    );
    assert_eq!(requested_bytes, 0, "{name}: requested bytes");
    println!("{name} allocations=0 requested_bytes=0");
}
