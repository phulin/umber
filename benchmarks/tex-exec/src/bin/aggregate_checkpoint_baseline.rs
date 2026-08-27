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
    counters: usize,
}

struct Measurement {
    elapsed: Duration,
    stats: Stats,
    checksum: u64,
}

fn main() {
    for &(shape, units) in &[("minimal", 1), ("accumulated", MANY_UNITS)] {
        for boundaries in [1, MANY_BOUNDARIES] {
            with_universe(budget(), |universe| {
                let (mut command, mut modes, retained) = fixture(universe, units);
                let capture = measure(|| {
                    let mut checkpoints = Vec::with_capacity(boundaries);
                    let mut checksum = 0_u64;
                    for ordinal in 0..boundaries {
                        let checkpoint = EngineCheckpoint::capture_checkpoint(
                            EngineBoundary::OuterParagraphEnd,
                            &mut command,
                            &mut modes,
                            universe,
                            ExecutionBudgetCounters {
                                committed_steps: ordinal as u64,
                                cumulative_fuel: (ordinal * 17) as u64,
                            },
                        )
                        .expect("fixture is quiescent");
                        checksum ^= checkpoint.mode_hash().rotate_left((ordinal % 63) as u32);
                        checkpoints.push(checkpoint);
                    }
                    (checkpoints, checksum)
                });
                let capture_checksum = capture.1.checksum ^ capture.0.len() as u64;
                let checkpoints = capture.0;

                let clone = measure(|| {
                    let clones = (0..boundaries)
                        .map(|_| checkpoints[0].clone())
                        .collect::<Vec<_>>();
                    (clones, checkpoints[0].mode_hash())
                });
                let clones = clone.0;
                let clone_checksum = clone.1.checksum ^ clones.len() as u64;

                let restore = measure(|| {
                    let mut checksum = 0_u64;
                    for checkpoint in &clones {
                        checkpoint
                            .restore_state(&mut command, &mut modes, universe)
                            .expect("same-generation restore");
                        checksum ^= modes.depth() as u64 ^ checkpoint.mode_hash();
                    }
                    ((), checksum)
                });
                drop(command);

                let fork = measure(|| {
                    let mut checksum = 0_u64;
                    for checkpoint in &clones {
                        let (mut candidate, control) = checkpoint
                            .profile_fork_state(universe)
                            .expect("aggregate checkpoint fork");
                        checksum ^= control.command_profile().fingerprint().get();
                        universe.return_rejected_pdf_from(&mut candidate);
                        drop(control);
                        drop(candidate);
                    }
                    ((), checksum)
                });

                let semantic_checksum = capture_checksum
                    ^ clone_checksum.rotate_left(7)
                    ^ restore.1.checksum.rotate_left(13)
                    ^ fork.1.checksum.rotate_left(29)
                    ^ fixture_checksum(universe, &modes);
                assert_mode_page_flat_gate(
                    shape, boundaries, &capture.1, &clone.1, &fork.1, &restore.1,
                );
                print_row(
                    shape,
                    units,
                    boundaries,
                    retained,
                    capture.1,
                    clone.1,
                    fork.1,
                    restore.1,
                    semantic_checksum,
                );
                black_box((checkpoints, clones));
            })
            .expect("aggregate benchmark universe");
        }
    }
    run_early_suffix_gate();
}

fn run_early_suffix_gate() {
    let mut observations = Vec::new();
    for units in [1, 4_096] {
        with_universe(budget(), |universe| {
            let mut command = CommandState::new(CommandProfile::TEX82);
            let mut modes = ModeNest::new();
            modes.push_current_node(Node::Penalty(-1));
            {
                let mut context = universe.command_context().expect("root context");
                context.append_page_contribution(Node::Penalty(-1));
                context.push_current_page_node(Node::Penalty(-1));
            }
            let checkpoint = EngineCheckpoint::capture_checkpoint(
                EngineBoundary::JobStart,
                &mut command,
                &mut modes,
                universe,
                ExecutionBudgetCounters::default(),
            )
            .expect("early rooted checkpoint");
            let mark = NodeTokenList::new([TokenWord::pack(Token::Char {
                ch: 'm',
                cat: Catcode::Other,
            })]);
            for index in 0..units {
                modes.push_current_node(Node::Penalty(index as i32));
                let mut context = universe.command_context().expect("suffix context");
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
                ((), work.0 ^ work.1.rotate_left(17))
            });
            assert_eq!(measurement.checksum, 0, "owner cycle replayed accepted history");
            observations.push((
                units,
                measurement.stats.allocations,
                measurement.stats.bytes_allocated,
                measurement.elapsed,
            ));
        })
        .expect("early suffix universe");
    }
    let small = observations[0];
    let large = observations[1];
    assert_eq!(small.1, large.1, "owner-cycle allocations depend on suffix depth");
    assert_eq!(small.2, large.2, "owner-cycle bytes depend on suffix depth");
    println!(
        "MODE_PAGE_EARLY_SUFFIX_GATE small_units={} large_units={} allocations={} requested_bytes={} small_ns={} large_ns={} replay_work=0",
        small.0,
        large.0,
        large.1,
        large.2,
        small.3.as_nanos(),
        large.3.as_nanos(),
    );
}

fn assert_mode_page_flat_gate(
    shape: &str,
    boundaries: usize,
    capture: &Measurement,
    clone: &Measurement,
    fork: &Measurement,
    restore: &Measurement,
) {
    if shape != "accumulated" {
        return;
    }
    let per_boundary = |value: usize| value / boundaries;
    assert!(
        per_boundary(capture.stats.allocations) <= 400
            && per_boundary(capture.stats.bytes_allocated) <= 60_000,
        "mode/page capture allocation gate regressed"
    );
    assert!(
        per_boundary(clone.stats.allocations) <= 320
            && per_boundary(clone.stats.bytes_allocated) <= 30_000,
        "mode/page checkpoint-clone allocation gate regressed"
    );
    assert!(
        per_boundary(fork.stats.allocations) <= 1_260
            && per_boundary(fork.stats.bytes_allocated) <= 1_600_000,
        "mode/page fork allocation gate regressed"
    );
    assert!(
        per_boundary(restore.stats.allocations) <= 380
            && per_boundary(restore.stats.bytes_allocated) <= 60_000,
        "mode/page restore allocation gate regressed"
    );
}

fn fixture<G>(
    universe: &mut Universe<G>,
    units: usize,
) -> (CommandState<G>, ModeNest, RetainedBytes) {
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
        modes.push_current_node(Node::Penalty(level as i32));
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

    let retained = RetainedBytes {
        command: units * (32 + token_words.len() * size_of::<TokenWord>()),
        modes: modes.depth() * size_of::<tex_exec::ModeLevelSummary>()
            + units.min(32) * size_of::<Node>(),
        page: units
            * (size_of::<Node>() * 3
                + size_of::<PageInsertion>()
                + token_words.len() * size_of::<TokenWord>()),
        hyphenation: units * (size_of::<PatternSpec>() + 64),
        world: units * 32 + units * 16 + units * 64,
        pdf: 0,
        dependencies: units * size_of::<usize>() * 2,
        sources_fonts: units * 32,
        counters: size_of::<ExecutionBudgetCounters>(),
    };
    (command, modes, retained)
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
    clone: Measurement,
    fork: Measurement,
    restore: Measurement,
    checksum: u64,
) {
    println!(
        "AGGREGATE_CHECKPOINT_BASELINE shape={shape} units={units} boundaries={boundaries} capture_ns={} capture_allocations={} capture_requested_bytes={} clone_ns={} clone_allocations={} clone_requested_bytes={} fork_ns={} fork_allocations={} fork_requested_bytes={} restore_ns={} restore_allocations={} restore_requested_bytes={} retained_command_bytes={} retained_mode_bytes={} retained_page_bytes={} retained_hyphenation_bytes={} retained_world_bytes={} retained_pdf_bytes={} retained_dependencies_bytes={} retained_sources_fonts_bytes={} retained_execution_counters_bytes={} semantic_checksum={checksum}",
        capture.elapsed.as_nanos(),
        capture.stats.allocations,
        capture.stats.bytes_allocated,
        clone.elapsed.as_nanos(),
        clone.stats.allocations,
        clone.stats.bytes_allocated,
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
        retained.counters,
    );
}

fn budget() -> InternerBudget {
    InternerBudget::new(65_536, 131_072, 16 * 1024 * 1024).expect("benchmark budget")
}
