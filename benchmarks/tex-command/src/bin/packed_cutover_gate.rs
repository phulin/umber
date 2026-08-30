use std::hint::black_box;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tex_command::{
    CommandFuelLedger, CommandHostCapabilities, CommandHostContext, CommandProcessor, CommandState,
    DeliveryStatus, RegisteredSourceKind, SourceRegistration,
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
    assert!(std::mem::size_of::<tex_state::DefinitionId<()>>() <= 8);
    assert!(std::mem::size_of::<tex_state::ResolvedMeaning<()>>() <= 24);
    assert!(std::mem::size_of::<tex_state::PrimitiveHandle<()>>() <= 16);
    assert_eq!(std::mem::size_of::<tex_command::CurrentCommand<()>>(), 112);
    assert!(std::mem::size_of::<DeliveryStatus>() <= 16);
    let only = std::env::args().nth(1);
    let only = only.as_deref().map(|row| {
        if row == "--mixed-stored-only" {
            "warmed_mixed_stored_cursor"
        } else {
            row.strip_prefix("--only=")
                .expect("benchmark selector must be --only=<row>")
        }
    });
    run_row(only, "ordinary_source_delivery", ordinary_source_delivery);
    run_row(only, "packed_backup_and_replay", packed_backup_and_replay);
    run_row(
        only,
        "warmed_backup_push_pop",
        warmed_backup_push_pop_throughput,
    );
    run_row(only, "stored_token_replay", stored_token_replay);
    run_row(
        only,
        "warmed_mixed_stored_cursor",
        warmed_mixed_stored_cursor,
    );
    run_row(
        only,
        "warmed_long_macro_argument_cursor",
        warmed_long_macro_argument_cursor,
    );
    run_row(only, "known_name_lookup", warmed_short_interner_lookup);
    run_row(only, "primitive_resolution", warmed_primitive_resolution);
    run_row(
        only,
        "source_known_creating_delivery",
        source_known_creating_delivery,
    );
    run_row(
        only,
        "source_known_probe_delivery",
        source_known_probe_delivery,
    );
    run_row(
        only,
        "source_new_creating_delivery",
        source_new_creating_delivery,
    );
    run_row(
        only,
        "source_unknown_probe_delivery",
        source_unknown_probe_delivery,
    );
    run_row(
        only,
        "stored_control_sequence_delivery",
        stored_control_sequence_delivery,
    );
    run_row(
        only,
        "borrowed_meaning_projection",
        borrowed_meaning_projection,
    );
    run_row(only, "macro_argument_matching", macro_argument_matching);
    run_row(
        only,
        "warmed_keyword_mismatch",
        warmed_keyword_mismatch_throughput,
    );
    run_row(
        only,
        "destination_directed_warm_delivery",
        destination_directed_warm_delivery,
    );
    run_row(
        only,
        "fused_raw_expanded_delivery",
        fused_raw_expanded_delivery,
    );
    run_row(
        only,
        "destination_owned_macro_expansion",
        destination_owned_macro_expansion,
    );
    run_row(
        only,
        "stationary_scan_toks_progress",
        stationary_scan_toks_progress,
    );
    if let Some(only) = only {
        assert!(
            BENCHMARK_ROWS.contains(&only),
            "unknown benchmark row {only}"
        );
    }
    println!("packed token/macro cutover gate: PASS");
}

const BENCHMARK_ROWS: &[&str] = &[
    "ordinary_source_delivery",
    "packed_backup_and_replay",
    "warmed_backup_push_pop",
    "stored_token_replay",
    "warmed_mixed_stored_cursor",
    "warmed_long_macro_argument_cursor",
    "known_name_lookup",
    "primitive_resolution",
    "source_known_creating_delivery",
    "source_known_probe_delivery",
    "source_new_creating_delivery",
    "source_unknown_probe_delivery",
    "stored_control_sequence_delivery",
    "borrowed_meaning_projection",
    "macro_argument_matching",
    "warmed_keyword_mismatch",
    "destination_directed_warm_delivery",
    "fused_raw_expanded_delivery",
    "destination_owned_macro_expansion",
    "stationary_scan_toks_progress",
];

fn run_row(only: Option<&str>, name: &str, row: fn()) {
    if only.is_none_or(|only| only == name) {
        row();
    }
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

fn warmed_long_macro_argument_cursor() {
    const CALLS: u32 = 5_000_000;
    let mut benchmark = tex_command::LongMacroArgumentCursorBenchmark::<()>::new();
    let _ = benchmark.run(64);
    let mut receipt = None;
    let mut elapsed = Duration::ZERO;
    measure_zero("warmed_long_macro_argument_cursor", || {
        let start = Instant::now();
        receipt = Some(black_box(benchmark.run(CALLS)));
        elapsed = start.elapsed();
    });
    let receipt = receipt.expect("long macro-argument cursor receipt");
    assert_eq!(receipt.calls, u64::from(CALLS));
    assert!(receipt.retirements > 0);
    assert_eq!(receipt.rollbacks, 1);
    println!(
        "warmed_long_macro_argument_cursor calls={} retirements={} rollbacks={} checksum={} elapsed_ns={} ns_per_call={:.2}",
        receipt.calls,
        receipt.retirements,
        receipt.rollbacks,
        receipt.checksum,
        elapsed.as_nanos(),
        elapsed.as_nanos() as f64 / receipt.calls as f64,
    );
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

fn source_known_creating_delivery() {
    const OPERATIONS: usize = 1_000_000;
    const NAME: &str = "deliveryidentity";
    with_universe(|universe| {
        let symbol = universe.intern(NAME).expect("control sequence name");
        let mut command = CommandState::default();
        open_source(&mut command, &format!(r"\{NAME} ").repeat(OPERATIONS));
        let mut capabilities = CommandHostCapabilities::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut fuel = CommandFuelLedger::default();
        let mut processor = processor(
            &mut context,
            &mut command,
            &mut capabilities,
            &mut fuel,
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
            "source_known_creating_delivery operations={OPERATIONS} throughput_ns_per_op={:.2}",
            elapsed.as_nanos() as f64 / OPERATIONS as f64
        );
    });
}

fn source_known_probe_delivery() {
    const OPERATIONS: usize = 1_000_000;
    const NAME: &str = "deliveryidentity";
    with_universe(|universe| {
        let symbol = universe.intern(NAME).expect("control sequence name");
        let mut command = CommandState::default();
        open_source(&mut command, &format!(r"\{NAME} ").repeat(OPERATIONS));
        let mut capabilities = CommandHostCapabilities::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut fuel = CommandFuelLedger::default();
        let mut processor = processor(
            &mut context,
            &mut command,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );
        let start = Instant::now();
        for _ in 0..OPERATIONS {
            let delivered = processor.get_next().unwrap().unwrap();
            assert_eq!(delivered.control_sequence(), Some(symbol.symbol()));
            black_box(delivered);
        }
        let elapsed = start.elapsed();
        println!(
            "source_known_probe_delivery operations={OPERATIONS} throughput_ns_per_op={:.2}",
            elapsed.as_nanos() as f64 / OPERATIONS as f64
        );
    });
}

fn source_new_creating_delivery() {
    const OPERATIONS: usize = 65_536;
    with_large_interner_universe(|universe| {
        let source = unique_control_sequence_source("newname", OPERATIONS);
        let mut command = CommandState::default();
        open_source(&mut command, &source);
        let mut capabilities = CommandHostCapabilities::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut fuel = CommandFuelLedger::default();
        let mut processor = processor(
            &mut context,
            &mut command,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );
        let start = Instant::now();
        for _ in 0..OPERATIONS {
            let delivered = processor.get_token().unwrap().unwrap();
            assert!(delivered.control_sequence().is_some());
            black_box(delivered);
        }
        let elapsed = start.elapsed();
        println!(
            "source_new_creating_delivery operations={OPERATIONS} throughput_ns_per_op={:.2}",
            elapsed.as_nanos() as f64 / OPERATIONS as f64
        );
    });
}

fn source_unknown_probe_delivery() {
    const OPERATIONS: usize = 65_536;
    with_large_interner_universe(|universe| {
        let source = unique_control_sequence_source("probename", OPERATIONS);
        let mut command = CommandState::default();
        open_source(&mut command, &source);
        let mut capabilities = CommandHostCapabilities::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut fuel = CommandFuelLedger::default();
        let mut processor = processor(
            &mut context,
            &mut command,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );
        let start = Instant::now();
        for _ in 0..OPERATIONS {
            let delivered = processor.get_next().unwrap().unwrap();
            assert!(
                delivered
                    .spelling()
                    .semantic_token()
                    .is_undefined_control_sequence()
            );
            black_box(delivered);
        }
        let elapsed = start.elapsed();
        println!(
            "source_unknown_probe_delivery operations={OPERATIONS} throughput_ns_per_op={:.2}",
            elapsed.as_nanos() as f64 / OPERATIONS as f64
        );
    });
}

fn stored_control_sequence_delivery() {
    const OPERATIONS: usize = 1_000_000;
    const NAME: &str = "deliveryidentity";

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
        let mut context = universe.command_context().expect("command context");
        let mut fuel = CommandFuelLedger::default();
        let mut processor = processor(
            &mut context,
            &mut command,
            &mut capabilities,
            &mut fuel,
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
            "stored_control_sequence_delivery operations={OPERATIONS} throughput_ns_per_op={:.2}",
            elapsed.as_nanos() as f64 / OPERATIONS as f64
        );
    });
}

fn borrowed_meaning_projection() {
    with_universe(|universe| {
        let mut benchmark = tex_command::MeaningProjectionBenchmark::new(universe);
        let context = universe.command_context().expect("command context");
        let _ = benchmark.run(&context, 64);
        let mut one = None;
        let mut four_k = None;
        measure_zero("borrowed_meaning_projection_1_4096", || {
            one = Some(black_box(benchmark.run(&context, 1)));
            four_k = Some(black_box(benchmark.run(&context, 4096)));
        });
        for (rounds, receipt) in [
            (1_u32, one.expect("one-round projection receipt")),
            (4096_u32, four_k.expect("4096-round projection receipt")),
        ] {
            assert_eq!(receipt.table_probes, receipt.resolved_meanings);
            assert_eq!(receipt.tag_decodes, receipt.resolved_meanings);
            assert_eq!(receipt.macro_owner_resolutions, receipt.macro_meanings);
            assert_eq!(receipt.duplicate_owner_resolutions, 0);
            assert_eq!(receipt.whole_meaning_copies, 0);
            assert_eq!(receipt.whole_command_copies, 0);
            println!(
                "borrowed_meaning_projection rounds={rounds} meanings={} table_probes={} tag_decodes={} macro_meanings={} owner_resolutions={} duplicate_owner_resolutions={} whole_meaning_copies={} whole_command_copies={} checksum={}",
                receipt.resolved_meanings,
                receipt.table_probes,
                receipt.tag_decodes,
                receipt.macro_meanings,
                receipt.macro_owner_resolutions,
                receipt.duplicate_owner_resolutions,
                receipt.whole_meaning_copies,
                receipt.whole_command_copies,
                receipt.checksum,
            );
        }
    });
}

fn unique_control_sequence_source(prefix: &str, operations: usize) -> String {
    let mut source = String::with_capacity(operations.saturating_mul(prefix.len() + 10));
    for index in 0..operations {
        source.push('\\');
        source.push_str(prefix);
        let mut value = index;
        let mut suffix = [b'a'; 8];
        for byte in suffix.iter_mut().rev() {
            *byte = b'a' + (value % 26) as u8;
            value /= 26;
        }
        source.push_str(std::str::from_utf8(&suffix).expect("alphabetic suffix"));
        source.push(' ');
    }
    source
}

fn warmed_backup_push_pop_throughput() {
    const OPERATIONS: usize = 1_000_000;
    with_universe(|universe| {
        let mut command = CommandState::default();
        open_source(&mut command, "b");
        let mut capabilities = CommandHostCapabilities::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut fuel = CommandFuelLedger::default();
        let mut processor = processor(
            &mut context,
            &mut command,
            &mut capabilities,
            &mut fuel,
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
        let mut context = universe.command_context().expect("command context");
        let mut fuel = CommandFuelLedger::default();
        let mut processor = processor(
            &mut context,
            &mut command,
            &mut capabilities,
            &mut fuel,
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
    const WARMUPS_PER_POLICY: usize = 64;
    const DELIVERIES_PER_POLICY: usize = 8_192;
    const POLICY_COUNT: usize = 3;
    with_universe(|universe| {
        let words = vec![
            TokenWord::pack(Token::Char {
                ch: 'd',
                cat: Catcode::Letter,
            });
            (WARMUPS_PER_POLICY + DELIVERIES_PER_POLICY) * POLICY_COUNT
        ];
        let stored = universe.allocate_token_list(&words).expect("stored tokens");
        let mut command = CommandState::default();
        {
            let context = universe.command_context().expect("command context");
            command.push_everyjob(&context, stored);
        }
        let mut capabilities = CommandHostCapabilities::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut fuel = CommandFuelLedger::default();
        let mut processor = processor(
            &mut context,
            &mut command,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );
        let mut destination = None;
        for _ in 0..WARMUPS_PER_POLICY {
            assert_eq!(
                processor.get_next_into(&mut destination).unwrap(),
                DeliveryStatus::Command
            );
            assert_char_ref(destination.as_ref().expect("direct raw command"), 'd');
            destination = None;
        }
        for _ in 0..WARMUPS_PER_POLICY {
            assert_eq!(
                processor.get_token_into(&mut destination).unwrap(),
                DeliveryStatus::Command
            );
            assert_char_ref(destination.as_ref().expect("direct token command"), 'd');
            destination = None;
        }
        for _ in 0..WARMUPS_PER_POLICY {
            assert_eq!(
                processor.get_x_token_into(&mut destination).unwrap(),
                DeliveryStatus::Command
            );
            assert_char_ref(destination.as_ref().expect("direct expanded command"), 'd');
            destination = None;
        }
        measure_zero("destination_directed_24576_delivery", || {
            for _ in 0..DELIVERIES_PER_POLICY {
                assert_eq!(
                    processor.get_next_into(&mut destination).unwrap(),
                    DeliveryStatus::Command
                );
                assert_char_ref(destination.as_ref().expect("direct raw command"), 'd');
                destination = None;
            }
            for _ in 0..DELIVERIES_PER_POLICY {
                assert_eq!(
                    processor.get_token_into(&mut destination).unwrap(),
                    DeliveryStatus::Command
                );
                assert_char_ref(destination.as_ref().expect("direct token command"), 'd');
                destination = None;
            }
            for _ in 0..DELIVERIES_PER_POLICY {
                assert_eq!(
                    processor.get_x_token_into(&mut destination).unwrap(),
                    DeliveryStatus::Command
                );
                assert_char_ref(destination.as_ref().expect("direct expanded command"), 'd');
                destination = None;
            }
        });
    });
}

fn fused_raw_expanded_delivery() {
    const WARMUPS: usize = 64;
    const DELIVERIES: usize = 1_000_000;
    with_universe(|universe| {
        let words = vec![
            TokenWord::pack(Token::Char {
                ch: 'f',
                cat: Catcode::Letter,
            });
            (WARMUPS + DELIVERIES) * 2
        ];
        let stored = universe.allocate_token_list(&words).expect("stored tokens");
        let mut command = CommandState::default();
        {
            let context = universe.command_context().expect("command context");
            command.push_everyjob(&context, stored);
        }
        let mut capabilities = CommandHostCapabilities::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut fuel = CommandFuelLedger::default();
        let mut delivery_processor = processor(
            &mut context,
            &mut command,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );
        let mut destination = None;
        for _ in 0..WARMUPS {
            assert_eq!(
                delivery_processor.get_next_into(&mut destination).unwrap(),
                DeliveryStatus::Command
            );
            assert_char_ref(destination.as_ref().expect("warm raw command"), 'f');
            destination = None;
        }
        for _ in 0..WARMUPS {
            assert_eq!(
                delivery_processor
                    .get_x_token_into(&mut destination)
                    .unwrap(),
                DeliveryStatus::Command
            );
            assert_char_ref(destination.as_ref().expect("warm expanded command"), 'f');
            destination = None;
        }

        drop(delivery_processor);
        let work_before = fuel.work();
        let mut delivery_processor = processor(
            &mut context,
            &mut command,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );
        let mut raw_elapsed = Duration::ZERO;
        let mut expanded_elapsed = Duration::ZERO;
        measure_zero("fused_raw_expanded_2000000_delivery", || {
            let start = Instant::now();
            for _ in 0..DELIVERIES {
                assert_eq!(
                    delivery_processor.get_next_into(&mut destination).unwrap(),
                    DeliveryStatus::Command
                );
                assert_char_ref(destination.as_ref().expect("fused raw command"), 'f');
                destination = None;
            }
            raw_elapsed = start.elapsed();
            let start = Instant::now();
            for _ in 0..DELIVERIES {
                assert_eq!(
                    delivery_processor
                        .get_x_token_into(&mut destination)
                        .unwrap(),
                    DeliveryStatus::Command
                );
                assert_char_ref(destination.as_ref().expect("fused expanded command"), 'f');
                destination = None;
            }
            expanded_elapsed = start.elapsed();
        });
        drop(delivery_processor);
        let work_after = fuel.work();
        assert_eq!(
            work_after.fuel_charges - work_before.fuel_charges,
            (DELIVERIES * 2) as u64
        );
        assert_eq!(
            work_after.token_frame_steps - work_before.token_frame_steps,
            (DELIVERIES * 2) as u64
        );
        assert_eq!(
            work_after.expanded_deliveries - work_before.expanded_deliveries,
            DELIVERIES as u64
        );
        assert_eq!(work_after.meaning_lookups - work_before.meaning_lookups, 0);
        println!(
            "fused_raw_expanded_delivery raw={} expanded={} raw_ns_per_delivery={:.2} expanded_ns_per_delivery={:.2}",
            DELIVERIES,
            DELIVERIES,
            raw_elapsed.as_nanos() as f64 / DELIVERIES as f64,
            expanded_elapsed.as_nanos() as f64 / DELIVERIES as f64,
        );
    });
}

fn destination_owned_macro_expansion() {
    const WARMUPS: usize = 64;
    const EXPANSIONS: usize = 1_000_000;
    with_universe(|universe| {
        let name = universe.intern("emptyexpansion").expect("macro name");
        let definition = universe
            .allocate_definition(&[], &[])
            .expect("empty macro definition");
        universe
            .assign_meaning(
                name,
                MeaningWord::macro_definition(MeaningFlags::EMPTY, definition),
                AssignmentScope::Global,
            )
            .expect("macro meaning");
        let macro_word = TokenWord::pack(Token::Cs(name.symbol()));
        let terminal = TokenWord::pack(Token::Char {
            ch: 'z',
            cat: Catcode::Letter,
        });
        let mut warm_words = vec![macro_word; WARMUPS];
        warm_words.push(terminal);
        let warm = universe
            .allocate_token_list(&warm_words)
            .expect("warm expansion input");
        let mut measured_words = vec![macro_word; EXPANSIONS];
        measured_words.push(terminal);
        let measured = universe
            .allocate_token_list(&measured_words)
            .expect("measured expansion input");

        let mut command = CommandState::default();
        {
            let context = universe.command_context().expect("command context");
            command.push_everyjob(&context, warm);
        }
        let mut capabilities = CommandHostCapabilities::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut fuel = CommandFuelLedger::default();
        let mut destination = None;
        {
            let mut delivery_processor = processor(
                &mut context,
                &mut command,
                &mut capabilities,
                &mut fuel,
                &mut diagnostic_effects,
            );
            assert_eq!(
                delivery_processor
                    .get_x_token_into(&mut destination)
                    .expect("warm macro expansion"),
                DeliveryStatus::Command
            );
            assert_char_ref(destination.as_ref().expect("warm terminal"), 'z');
        }
        destination = None;
        command.push_everyjob(&context, measured);
        let work_before = fuel.work();
        let mut delivery_processor = processor(
            &mut context,
            &mut command,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );
        let mut elapsed = Duration::ZERO;
        measure_zero("destination_owned_macro_expansion_1000000", || {
            let start = Instant::now();
            assert_eq!(
                delivery_processor
                    .get_x_token_into(&mut destination)
                    .expect("measured macro expansion"),
                DeliveryStatus::Command
            );
            elapsed = start.elapsed();
        });
        assert_char_ref(destination.as_ref().expect("measured terminal"), 'z');
        drop(delivery_processor);
        let work_after = fuel.work();
        assert_eq!(
            work_after.token_frame_steps - work_before.token_frame_steps,
            (EXPANSIONS + 1) as u64
        );
        assert_eq!(
            work_after.expanded_deliveries - work_before.expanded_deliveries,
            1
        );
        println!(
            "destination_owned_macro_expansion expansions={EXPANSIONS} ns_per_expansion={:.2}",
            elapsed.as_nanos() as f64 / EXPANSIONS as f64,
        );
    });
}

fn ordinary_source_delivery() {
    with_universe(|universe| {
        let mut command = CommandState::default();
        open_source(&mut command, "ssssssssssssssss");
        let mut capabilities = CommandHostCapabilities::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut fuel = CommandFuelLedger::default();
        let mut processor = processor(
            &mut context,
            &mut command,
            &mut capabilities,
            &mut fuel,
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
        let mut context = universe.command_context().expect("command context");
        let mut fuel = CommandFuelLedger::default();
        let mut processor = processor(
            &mut context,
            &mut command,
            &mut capabilities,
            &mut fuel,
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
        let mut context = universe.command_context().expect("command context");
        let mut fuel = CommandFuelLedger::default();
        let mut processor = processor(
            &mut context,
            &mut command,
            &mut capabilities,
            &mut fuel,
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

fn stationary_scan_toks_progress() {
    const OPERATIONS: usize = 1_000_000;
    const WARMUPS: usize = 64;
    const BODY: &str = "{}";

    with_universe(|universe| {
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
                    .expect("scan_toks benchmark category code");
            }
        }
        let source = BODY.repeat(OPERATIONS + WARMUPS);
        let mut command = CommandState::default();
        open_source(&mut command, &source);
        let mut capabilities = CommandHostCapabilities::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut fuel = CommandFuelLedger::default();
        let mut scan_one = || {
            let operation = command.begin_attempt_operation();
            {
                let mut context = universe.command_context().expect("command context");
                let mut processor = processor(
                    &mut context,
                    &mut command,
                    &mut capabilities,
                    &mut fuel,
                    &mut diagnostic_effects,
                );
                black_box(
                    processor
                        .scan_balanced_text(false)
                        .expect("balanced text scan"),
                );
            }
            command
                .commit_attempt_operation(operation)
                .expect("scan_toks operation commit");
        };
        for _ in 0..WARMUPS {
            scan_one();
        }
        let mut elapsed = Duration::ZERO;
        measure_zero("stationary_scan_toks_progress_1000000", || {
            let start = Instant::now();
            for _ in 0..OPERATIONS {
                scan_one();
            }
            elapsed = start.elapsed();
        });
        println!(
            "stationary_scan_toks_progress scans={OPERATIONS} ns_per_scan={:.2}",
            elapsed.as_nanos() as f64 / OPERATIONS as f64,
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
        let mut context = universe.command_context().expect("command context");
        let mut fuel = CommandFuelLedger::default();
        let mut processor = processor(
            &mut context,
            &mut command,
            &mut capabilities,
            &mut fuel,
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
        let match_word_reads = processor.macro_argument_match_word_reads();
        measure_zero("macro_matching_replay_expansion", || {
            assert_char(processor.get_x_token().unwrap().unwrap(), 'a');
        });
        assert_eq!(
            processor.macro_argument_match_word_reads(),
            match_word_reads,
            "macro paragraph and outer-group decisions reread matched words"
        );
    });
}

fn with_universe(test: impl for<'id> FnOnce(&mut Universe<tex_state::GenerationBrand<'id>>)) {
    let budget = InternerBudget::new(4_096, 4_096, 1 << 20).expect("benchmark interner budget");
    tex_state::with_universe(budget, test).expect("benchmark universe");
}

fn with_large_interner_universe(
    test: impl for<'id> FnOnce(&mut Universe<tex_state::GenerationBrand<'id>>),
) {
    let budget =
        InternerBudget::new(131_072, 131_072, 8 << 20).expect("large benchmark interner budget");
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

fn processor<'episode, 'admission, G>(
    context: &'episode mut tex_state::CommandContext<'admission, G>,
    command: &'episode mut CommandState<G>,
    capabilities: &'episode mut CommandHostCapabilities,
    fuel: &'episode mut CommandFuelLedger,
    diagnostic_effects: &'episode mut tex_state::diagnostic::DiagnosticEffects,
) -> CommandProcessor<'episode, 'admission, G> {
    CommandProcessor::new(
        command,
        context,
        CommandHostContext::new(capabilities),
        fuel.fuel_mut(),
        None,
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
