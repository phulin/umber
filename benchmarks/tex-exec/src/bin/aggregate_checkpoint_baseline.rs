use std::alloc::System;
use std::hint::black_box;
use std::sync::Arc;
use std::time::{Duration, Instant};

use stats_alloc::{INSTRUMENTED_SYSTEM, Region, Stats, StatsAlloc};
use tex_command::{CommandProfile, CommandState, RegisteredSourceKind, SourceRegistration};
use tex_exec::{EngineBoundary, EngineCheckpoint, ExecutionBudgetCounters, Mode, ModeNest};
use tex_state::diagnostic::DiagnosticEffects;
use tex_state::hyphenation::{ExceptionSpec, PatternSpec};
use tex_state::interner::InternerBudget;
use tex_state::node::{Node, NodeTokenList};
use tex_state::page::{PageInsertion, PageMark};
use tex_state::scaled::Scaled;
use tex_state::token::{Catcode, Token, TokenWord};
use tex_state::world::{PrintSink, StreamSlot};
use tex_state::{Universe, with_universe};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

const MANY_UNITS: usize = 64;
const MANY_BOUNDARIES: usize = 32;

#[derive(Clone, Copy)]
struct RetainedBytes {
    command: usize,
    modes: usize,
    page: usize,
    hyphenation: usize,
    world: usize,
    pdf: usize,
    dependencies: usize,
    sources_fonts: usize,
    core: usize,
    counters: usize,
}

impl From<tex_exec::CheckpointRetention> for RetainedBytes {
    fn from(retention: tex_exec::CheckpointRetention) -> Self {
        Self {
            command: retention.command_bytes(),
            modes: retention.mode_bytes(),
            page: retention.page_bytes(),
            hyphenation: retention.hyphenation_bytes(),
            world: retention.world_bytes(),
            pdf: retention.pdf_bytes(),
            dependencies: retention.dependency_bytes(),
            sources_fonts: retention.source_font_bytes(),
            core: retention.core_bytes(),
            counters: retention.execution_counter_bytes(),
        }
    }
}

struct Measurement {
    elapsed: Duration,
    stats: Stats,
    checksum: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AllocationCount {
    allocations: usize,
    requested_bytes: usize,
}

impl From<&Measurement> for AllocationCount {
    fn from(measurement: &Measurement) -> Self {
        Self {
            allocations: measurement.stats.allocations,
            requested_bytes: measurement.stats.bytes_allocated,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FlatCaptureCosts {
    capture: AllocationCount,
    mark_copy: AllocationCount,
    fork_reject: AllocationCount,
    restore: AllocationCount,
}

fn main() {
    run_early_suffix_gate();
    let mut minimal_costs = [None; 2];
    for &(shape, units) in &[("minimal", 1), ("accumulated", MANY_UNITS)] {
        for (boundary_index, boundaries) in [1, MANY_BOUNDARIES].into_iter().enumerate() {
            let costs = with_universe(budget(), |universe| {
                let (command, mut modes) = fixture(universe, units);
                let mut command = Some(command);
                let mut output = tex_exec::OutputLedger::new();
                let capture = measure(|| {
                    let mut checkpoints = Vec::with_capacity(boundaries);
                    let mut checksum = 0_u64;
                    for ordinal in 0..boundaries {
                        let mut checkpoint = EngineCheckpoint::profile_capture_checkpoint(
                            EngineBoundary::OuterParagraphEnd,
                            command.as_mut().expect("accepted command owner"),
                            &mut modes,
                            universe,
                            ExecutionBudgetCounters {
                                committed_steps: ordinal as u64,
                                cumulative_fuel: (ordinal * 17) as u64,
                            },
                        )
                        .expect("fixture is quiescent");
                        checkpoint.profile_attach_output_ledger(&mut output);
                        checksum ^= (checkpoint.root_anchor() as u64)
                            .rotate_left((ordinal % 63) as u32);
                        checkpoints.push(checkpoint);
                    }
                    (checkpoints, checksum)
                });
                let capture_checksum = capture.1.checksum ^ capture.0.len() as u64;
                let checkpoints = capture.0;
                let retained = checkpoints[0].retention().into();

                let mark_copy = measure(|| {
                    let mut checksum = 0_u64;
                    for ordinal in 0..boundaries {
                        let checkpoint = black_box(&checkpoints[ordinal % checkpoints.len()]);
                        checksum ^= (checkpoint.retention().checkpoint_metadata_bytes() as u64)
                            .rotate_left((ordinal % 63) as u32)
                            ^ checkpoint.budget_counters().cumulative_fuel;
                    }
                    ((), checksum)
                });
                let mark_copy_checksum = mark_copy.1.checksum;

                let fork = measure(|| {
                    let mut checksum = 0_u64;
                    for _ in 0..boundaries {
                        checksum ^= checkpoints[0]
                            .profile_fork_and_reject(universe, &mut command, &mut output)
                            .expect("aggregate checkpoint fork and rejection");
                    }
                    ((), checksum)
                });

                let restore = measure(|| {
                    let mut checksum = 0_u64;
                    let checkpoint = checkpoints.last().expect("retained checkpoint");
                    for _ in 0..boundaries {
                        checkpoint
                            .restore_state(
                                command.as_mut().expect("accepted command owner"),
                                &mut modes,
                                universe,
                            )
                            .expect("same-generation restore");
                        checksum ^= modes.depth() as u64 ^ checkpoint.root_anchor() as u64;
                    }
                    ((), checksum)
                });

                let semantic_checksum = capture_checksum
                    ^ mark_copy_checksum.rotate_left(7)
                    ^ restore.1.checksum.rotate_left(13)
                    ^ fork.1.checksum.rotate_left(29)
                    ^ fixture_checksum(universe, &modes);
                assert_mode_page_flat_gate(
                    shape,
                    boundaries,
                    &capture.1,
                    &mark_copy.1,
                    &fork.1,
                    &restore.1,
                );
                let costs = FlatCaptureCosts {
                    capture: (&capture.1).into(),
                    mark_copy: (&mark_copy.1).into(),
                    fork_reject: (&fork.1).into(),
                    restore: (&restore.1).into(),
                };
                print_row(
                    shape,
                    units,
                    boundaries,
                    retained,
                    capture.1,
                    mark_copy.1,
                    fork.1,
                    restore.1,
                    semantic_checksum,
                );
                black_box(checkpoints);
                costs
            })
            .expect("aggregate benchmark universe");
            if shape == "minimal" {
                minimal_costs[boundary_index] = Some(costs);
            } else {
                assert_eq!(
                    costs,
                    minimal_costs[boundary_index].expect("minimal cost row"),
                    "aggregate capture costs grew with accumulated payload at {boundaries} boundaries"
                );
            }
        }
    }
}

fn run_early_suffix_gate() {
    let mut observations = Vec::new();
    for units in [1, 4_096] {
        with_universe(budget(), |universe| {
            let mut command = CommandState::new(CommandProfile::TEX82);
            let mut modes = ModeNest::new();
            let mark = NodeTokenList::new([TokenWord::pack(Token::Char {
                ch: 'm',
                cat: Catcode::Other,
            })]);
            {
                let mut context = universe.command_context().expect("root context");
                context.append_page_contribution(Node::Penalty(-1));
                context.push_current_page_node(Node::Penalty(-1));
                context.push_page_discard(Node::Penalty(-1));
                let split = context.publish_page_nodes(vec![Node::Penalty(-1)]);
                context.set_split_discards(split);
                context.upsert_page_insertion(PageInsertion::new(7, Scaled::from_raw(-1)));
                context.set_page_mark_class(PageMark::Bot, 7, mark.clone());
            }
            let checkpoint = EngineCheckpoint::profile_capture_checkpoint(
                EngineBoundary::JobStart,
                &mut command,
                &mut modes,
                universe,
                ExecutionBudgetCounters::default(),
            )
            .expect("early rooted checkpoint");
            for index in 0..units {
                let mut context = universe.command_context().expect("suffix context");
                modes.push_current_node(&mut context, Node::Penalty(index as i32));
                context.append_page_contribution(Node::Penalty(index as i32));
                context.push_current_page_node(Node::Penalty(index as i32));
                context.push_page_discard(Node::Penalty(index as i32));
                context.upsert_page_insertion(PageInsertion::new(
                    (index % 256) as u16,
                    Scaled::from_raw(index as i32),
                ));
                context.set_page_mark_class(PageMark::Bot, index as u16, mark.clone());
            }
            let (_, measurement) = measure(|| {
                let work = checkpoint
                    .profile_mode_page_owner_cycle(universe)
                    .expect("rooted owner cycle");
                assert_eq!(&work[..4], &[0, 1, 1, 1]);
                assert_eq!(&work[5..], &[1, 1, 2, 2]);
                ((), work[4])
            });
            observations.push((
                units,
                measurement.stats.allocations,
                measurement.stats.bytes_allocated,
                measurement.elapsed,
                measurement.checksum,
            ));
        })
        .expect("early suffix universe");
    }
    let small = observations[0];
    let large = observations[1];
    println!(
        "MODE_PAGE_EARLY_SUFFIX_GATE small_units={} large_units={} small_allocations={} large_allocations={} small_requested_bytes={} large_requested_bytes={} small_ns={} large_ns={} mode_replay_work=0 small_page_replay_work={} large_page_replay_work={} mode_replace=1 mode_private_pop=1 mode_root_pop=1 contribution_pop=1 current_pop=1 discard_clears=2 insertion_mark_updates=2",
        small.0,
        large.0,
        small.1,
        large.1,
        small.2,
        large.2,
        small.3.as_nanos(),
        large.3.as_nanos(),
        small.4,
        large.4,
    );
}

fn assert_mode_page_flat_gate(
    _shape: &str,
    _boundaries: usize,
    _capture: &Measurement,
    mark_copy: &Measurement,
    _fork: &Measurement,
    _restore: &Measurement,
) {
    assert!(
        mark_copy.stats.allocations == 0 && mark_copy.stats.bytes_allocated == 0,
        "bounded checkpoint metadata copy allocated: allocations={} requested_bytes={}",
        mark_copy.stats.allocations,
        mark_copy.stats.bytes_allocated,
    );
}

fn fixture<G>(
    universe: &mut Universe<G>,
    units: usize,
) -> (CommandState<G>, ModeNest) {
    let mut command = CommandState::new(CommandProfile::TEX82);
    let mut modes = ModeNest::new();
    let token_words = vec![
        TokenWord::pack(Token::Char {
            ch: 'x',
            cat: Catcode::Other,
        });
        units * 8
    ];
    let token_root = universe
        .command_context()
        .expect("fixture context")
        .allocate_token_list(&token_words)
        .expect("fixture token list");
    for index in 0..units {
        let bytes: Arc<[u8]> = format!("source-{index:04}-payload").into_bytes().into();
        let source = command
            .register_source(
                SourceRegistration::new(RegisteredSourceKind::Generated, Arc::clone(&bytes))
                    .with_name(format!("fixture-{index}.tex")),
            )
            .expect("register fixture source");
        command
            .open_registered_source(source)
            .expect("open fixture source");
        let context = universe.command_context().expect("command context");
        command.push_everypar(&context, token_root.clone());
    }
    let mut diagnostic_effects = DiagnosticEffects::new();
    let _ = command.publish_named_token_list_pushes(
        &mut universe.command_context().expect("command context"),
        &mut diagnostic_effects,
    );

    for level in 0..units.min(32) {
        modes
            .push(if level % 2 == 0 {
                Mode::Horizontal
            } else {
                Mode::InternalVertical
            })
            .expect("nested mode");
        modes.push_current_node(
            &mut universe.command_context().expect("mode fixture context"),
            Node::Penalty(level as i32),
        );
    }
    for _ in 0..units.min(32) {
        let _ = modes.pop().expect("close nested fixture mode");
    }

    {
        let mut context = universe.command_context().expect("state fixture context");
        for index in 0..units {
            context
                .assign_count(
                    index as u16,
                    index as i32,
                    tex_state::AssignmentScope::Global,
                )
                .expect("dependency-tracked count");
            context.append_page_contribution(Node::Penalty(index as i32));
            context.push_current_page_node(Node::Penalty(index as i32 + 1));
            context.push_page_discard(Node::Penalty(-(index as i32)));
            context.upsert_page_insertion(PageInsertion::new(
                index as u16,
                Scaled::from_raw(index as i32 + 2),
            ));
            context.set_page_mark_class(
                PageMark::Bot,
                index as u16,
                NodeTokenList::new(token_words.clone().into_boxed_slice()),
            );
            context
                .add_hyphenation_pattern_for_language(
                    (index % 8) as u8,
                    PatternSpec {
                        letters: format!("p{index:04}").chars().collect(),
                        values: vec![0, 1, 0, 0, 0, 0],
                    },
                )
                .expect("hyphenation pattern");
            context.add_hyphenation_exception_for_language(
                (index % 8) as u8,
                ExceptionSpec {
                    word: format!("exception{index:04}"),
                    positions: vec![3, 6],
                },
            );
        }
        context.close_hyphenation_patterns();
        context.save_hyphenation_codes(0, [('a', 'a'), ('b', 'b'), ('c', 'c')]);
    }

    let world = universe.world_mut();
    world.open_out(StreamSlot::new(0), "aggregate-checkpoint.out");
    for index in 0..units {
        world.write_text(
            PrintSink::Stream(StreamSlot::new(0)),
            &format!("effect-{index}"),
        );
        world.record_special("aggregate-checkpoint", vec![index as u8; 16]);
    }
    world.profile_publish_artifact(vec![0xa5; units * 64]);

    (command, modes)
}

fn fixture_checksum<G>(universe: &Universe<G>, modes: &ModeNest) -> u64 {
    universe.world().effect_records().len() as u64
        ^ (universe.world().committed_artifacts().len() as u64).rotate_left(7)
        ^ (modes.depth() as u64).rotate_left(13)
}

fn measure<T>(operation: impl FnOnce() -> (T, u64)) -> (T, Measurement) {
    let region = Region::new(GLOBAL);
    let start = Instant::now();
    let (value, checksum) = operation();
    let elapsed = start.elapsed();
    let stats = region.change();
    black_box(&value);
    (
        value,
        Measurement {
            elapsed,
            stats,
            checksum,
        },
    )
}

#[allow(clippy::too_many_arguments)]
fn print_row(
    shape: &str,
    units: usize,
    boundaries: usize,
    retained: RetainedBytes,
    capture: Measurement,
    mark_copy: Measurement,
    fork: Measurement,
    restore: Measurement,
    checksum: u64,
) {
    println!(
        "AGGREGATE_CHECKPOINT_GATE shape={shape} units={units} boundaries={boundaries} capture_ns={} capture_allocations={} capture_requested_bytes={} clone_api=deleted accounting_copy_ns={} accounting_copy_allocations={} accounting_copy_requested_bytes={} fork_reject_ns={} fork_reject_allocations={} fork_reject_requested_bytes={} restore_ns={} restore_allocations={} restore_requested_bytes={} retained_command_bytes={} retained_mode_bytes={} retained_page_bytes={} retained_hyphenation_bytes={} retained_world_bytes={} retained_pdf_bytes={} retained_dependencies_bytes={} retained_sources_fonts_bytes={} retained_core_bytes={} retained_execution_counters_bytes={} semantic_checksum={checksum}",
        capture.elapsed.as_nanos(),
        capture.stats.allocations,
        capture.stats.bytes_allocated,
        mark_copy.elapsed.as_nanos(),
        mark_copy.stats.allocations,
        mark_copy.stats.bytes_allocated,
        fork.elapsed.as_nanos(),
        fork.stats.allocations,
        fork.stats.bytes_allocated,
        restore.elapsed.as_nanos(),
        restore.stats.allocations,
        restore.stats.bytes_allocated,
        retained.command,
        retained.modes,
        retained.page,
        retained.hyphenation,
        retained.world,
        retained.pdf,
        retained.dependencies,
        retained.sources_fonts,
        retained.core,
        retained.counters,
    );
}

fn budget() -> InternerBudget {
    InternerBudget::new(65_536, 131_072, 16 * 1024 * 1024).expect("benchmark budget")
}
