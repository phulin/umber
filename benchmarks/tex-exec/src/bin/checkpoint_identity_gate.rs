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
    for mode_levels in [1, ACCUMULATED_MODE_LEVELS] {
        let ordinary = sample(mode_levels, false);
        let demanded = sample(mode_levels, true);
        assert_eq!(
            ordinary.allocations, demanded.allocations,
            "requesting an unavailable identity allocated for {mode_levels} mode levels"
        );
        assert_eq!(
            ordinary.bytes_allocated, demanded.bytes_allocated,
            "requesting an unavailable identity copied payload for {mode_levels} mode levels"
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

fn sample(mode_levels: usize, demand_identity: bool) -> Stats {
    with_universe(budget(), |universe| {
        let mut command = CommandState::new(CommandProfile::TEX82);
        let mut modes = ModeNest::new();
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
        assert_eq!(checkpoint.reachable_state_identity(), None);
        let stats = region.change();
        black_box(checkpoint);
        stats
    })
    .expect("checkpoint identity gate universe")
}

fn budget() -> InternerBudget {
    InternerBudget::new(4_096, 16_384, 2 * 1024 * 1024).expect("gate budget")
}
