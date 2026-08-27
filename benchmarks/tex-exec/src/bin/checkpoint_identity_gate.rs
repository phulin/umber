use std::alloc::System;
use std::hint::black_box;

use stats_alloc::{INSTRUMENTED_SYSTEM, Region, Stats, StatsAlloc};
use tex_command::{CommandProfile, CommandState};
use tex_exec::{EngineBoundary, EngineCheckpoint, ExecutionBudgetCounters, Mode, ModeNest};
use tex_state::interner::InternerBudget;
use tex_state::node::Node;
use tex_state::with_universe;

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

const ACCUMULATED_MODE_LEVELS: usize = 32;

fn main() {
    early_root_read_gate();
    for mode_levels in [1, ACCUMULATED_MODE_LEVELS] {
        let ordinary = sample(mode_levels, false);
        let demanded = sample(mode_levels, true);
        assert_eq!(
            ordinary.allocations, demanded.allocations,
            "requesting the maintained identity allocated during checkpoint capture for {mode_levels} mode levels"
        );
        assert_eq!(
            ordinary.bytes_allocated, demanded.bytes_allocated,
            "requesting the maintained identity copied payload during checkpoint capture for {mode_levels} mode levels"
        );
        println!(
            "CHECKPOINT_IDENTITY_GATE mode_levels={mode_levels} ordinary_allocations={} demanded_allocations={} ordinary_requested_bytes={} demanded_requested_bytes={}",
            ordinary.allocations,
            demanded.allocations,
            ordinary.bytes_allocated,
            demanded.bytes_allocated,
        );
    }
}

fn early_root_read_gate() {
    for suffix in [1, 4_096] {
        with_universe(budget(), |universe| {
            let mut command = CommandState::new(CommandProfile::TEX82);
            let mut modes = ModeNest::new();
            assert!(command.enable_reachable_state_identity());
            modes.enable_reachable_state_identity();
            universe.enable_reachable_state_identity();
            modes.push_current_node(Node::Penalty(-1));
            universe
                .command_context()
                .expect("root context")
                .append_page_contribution(Node::Penalty(-1));
            let checkpoint = EngineCheckpoint::profile_capture_checkpoint_with_identity_demand(
                EngineBoundary::OuterParagraphEnd,
                &mut command,
                &mut modes,
                universe,
                ExecutionBudgetCounters::default(),
            )
            .expect("early identity checkpoint");
            let expected = checkpoint.profile_mode_page_identity_roots();
            assert!(expected.0.is_some() && expected.1.is_some());
            let expected_complete = checkpoint
                .reachable_state_identity()
                .expect("all authoritative roots are available");
            for index in 0..suffix {
                modes.push_current_node(Node::Penalty(index as i32));
                universe
                    .command_context()
                    .expect("suffix context")
                    .append_page_contribution(Node::Penalty(index as i32));
            }

            let region = Region::new(GLOBAL);
            let mut checksum = 0_u64;
            for ordinal in 0..4_096 {
                let roots = black_box(&checkpoint).profile_mode_page_identity_roots();
                assert_eq!(roots, expected);
                let complete = black_box(&checkpoint)
                    .reachable_state_identity()
                    .expect("complete root remains available");
                assert_eq!(complete, expected_complete);
                checksum ^= complete.fingerprint().rotate_left(ordinal % 59)
                    ^ roots.0.expect("mode root").rotate_left(ordinal % 63)
                    ^ roots.1.expect("page root").rotate_right(ordinal % 61);
            }
            let stats = region.change();
            assert_eq!(stats.allocations, 0, "identity reads allocated");
            assert_eq!(stats.bytes_allocated, 0, "identity reads requested bytes");
            println!(
                "REACHABLE_STATE_IDENTITY_READ_GATE suffix={suffix} reads=4096 allocations={} requested_bytes={} checksum={checksum:016x}",
                stats.allocations, stats.bytes_allocated,
            );
        })
        .expect("identity read gate universe");
    }
}

fn sample(mode_levels: usize, demand_identity: bool) -> Stats {
    with_universe(budget(), |universe| {
        let mut command = CommandState::new(CommandProfile::TEX82);
        let mut modes = ModeNest::new();
        if demand_identity {
            assert!(command.enable_reachable_state_identity());
            modes.enable_reachable_state_identity();
            universe.enable_reachable_state_identity();
        }
        for level in 0..mode_levels {
            modes
                .push(if level % 2 == 0 {
                    Mode::Horizontal
                } else {
                    Mode::InternalVertical
                })
                .expect("bounded mode nest");
            modes.push_current_node(Node::Penalty(level as i32));
        }

        // Warm the generation-owned checkpoint slots before measuring the
        // difference made solely by optional identity demand.
        black_box(
            EngineCheckpoint::capture_checkpoint(
                EngineBoundary::OuterParagraphEnd,
                &mut command,
                &mut modes,
                universe,
                ExecutionBudgetCounters::default(),
            )
            .expect("warm checkpoint"),
        );
        let region = Region::new(GLOBAL);
        let checkpoint = if demand_identity {
            EngineCheckpoint::profile_capture_checkpoint_with_identity_demand(
                EngineBoundary::OuterParagraphEnd,
                &mut command,
                &mut modes,
                universe,
                ExecutionBudgetCounters::default(),
            )
        } else {
            EngineCheckpoint::capture_checkpoint(
                EngineBoundary::OuterParagraphEnd,
                &mut command,
                &mut modes,
                universe,
                ExecutionBudgetCounters::default(),
            )
        }
        .expect("measured checkpoint");
        assert_eq!(
            checkpoint.reachable_state_identity().is_some(),
            demand_identity
        );
        let stats = region.change();
        black_box(checkpoint);
        stats
    })
    .expect("checkpoint identity gate universe")
}

fn budget() -> InternerBudget {
    InternerBudget::new(4_096, 16_384, 2 * 1024 * 1024).expect("gate budget")
}
