use std::alloc::System;
use std::sync::Arc;

use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};
use tex_command::{CommandProfile, CommandState, RegisteredSourceKind, SourceRegistration};
use tex_state::interner::InternerBudget;
use tex_state::token::{Catcode, Token, TokenWord};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Counts {
    allocations: usize,
    bytes: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct GateCounts {
    capture: Counts,
    clone: Counts,
    restore: Counts,
    first_mutation: Counts,
    fork: Counts,
    fork_first_mutation: Counts,
    repeated_scalar_mutations: Counts,
    repeated_input_frame_mutations: Counts,
    logical_history: LogicalHistoryCounts,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct LogicalHistoryCounts {
    payload_admissions_per_frame: u64,
    full_frame_history_clones: u64,
    records: u64,
    record_bytes: u64,
    coalesced_mutations: u64,
}

fn main() {
    let shallow = run_fixture(1);
    let accumulated = run_fixture(64);
    assert_eq!(
        shallow, accumulated,
        "command checkpoint costs must be independent of accumulated state"
    );
    for (name, counts) in [
        ("capture", shallow.capture),
        ("clone", shallow.clone),
        ("restore", shallow.restore),
        ("first_mutation", shallow.first_mutation),
        ("fork", shallow.fork),
        ("fork_first_mutation", shallow.fork_first_mutation),
        (
            "repeated_scalar_mutations",
            shallow.repeated_scalar_mutations,
        ),
        (
            "repeated_input_frame_mutations",
            shallow.repeated_input_frame_mutations,
        ),
    ] {
        assert_eq!(counts, Counts::ZERO, "{name} must remain allocation-free");
    }
    println!(
        "COMMAND_CHECKPOINT_GATE capture={:?} clone={:?} restore={:?} first_mutation={:?} fork={:?} fork_first_mutation={:?} repeated_scalar_mutations={:?} repeated_input_frame_mutations={:?} logical_history={:?}",
        shallow.capture,
        shallow.clone,
        shallow.restore,
        shallow.first_mutation,
        shallow.fork,
        shallow.fork_first_mutation,
        shallow.repeated_scalar_mutations,
        shallow.repeated_input_frame_mutations,
        shallow.logical_history,
    );
}

impl Counts {
    const ZERO: Self = Self {
        allocations: 0,
        bytes: 0,
    };
}

fn run_fixture(units: usize) -> GateCounts {
    tex_state::with_universe(budget(), |universe| {
        let mut command = CommandState::new(CommandProfile::TEX82);
        let words = [TokenWord::pack(Token::Char {
            ch: 'x',
            cat: Catcode::Other,
        })];
        let tokens = universe
            .command_context()
            .expect("fixture context")
            .allocate_token_list(&words)
            .expect("fixture token list");
        for index in 0..units {
            let source = command
                .register_source(
                    SourceRegistration::new(
                        RegisteredSourceKind::Generated,
                        Arc::<[u8]>::from(format!("source-{index:04}").into_bytes()),
                    )
                    .with_name(format!("checkpoint-{index}.tex")),
                )
                .expect("fixture source");
            command
                .open_registered_source(source)
                .expect("fixture source opens");
            command.push_everypar(
                &universe.command_context().expect("command context"),
                tokens.clone(),
            );
        }
        let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
        let _ = command.publish_named_token_list_pushes(
            &mut universe.command_context().expect("command context"),
            &mut effects,
        );

        let warm = command
            .publish_summary(universe)
            .expect("warm checkpoint captures");
        drop(warm);

        let (summary, capture) = measure(|| {
            command
                .publish_summary(universe)
                .expect("checkpoint captures")
        });
        let (clone, clone_counts) = measure(|| summary.clone());
        drop(clone);

        command.profile_first_timeline_mutation();
        let (_, restore) = measure(|| {
            command
                .restore_summary(&summary, universe)
                .expect("checkpoint restores");
        });
        assert!(!command.profile_name_in_progress());

        let (_, first_mutation) = measure(|| command.profile_first_timeline_mutation());
        command
            .restore_summary(&summary, universe)
            .expect("mutation cleanup restores");
        let journal_before = command.profile_timeline_counters();
        let (_, repeated_scalar_mutations) =
            measure(|| command.profile_repeated_timeline_mutations(8_192));
        let journal_after = command.profile_timeline_counters();
        assert_eq!(journal_after.records - journal_before.records, 1);
        assert_eq!(
            journal_after.coalesced_writes - journal_before.coalesced_writes,
            8_191
        );
        assert_eq!(journal_after.descriptor_publications, 0);
        assert!(journal_after.record_bytes - journal_before.record_bytes <= 32);
        command
            .restore_summary(&summary, universe)
            .expect("coalescing cleanup restores");
        let input_before = command.profile_timeline_counters();
        let logical_frames =
            u64::try_from(command.input_level_count()).expect("frame count fits u64");
        assert_eq!(
            input_before.logical_payload_admissions, logical_frames,
            "each logical frame has exactly one admitted payload"
        );
        let (_, repeated_input_frame_mutations) =
            measure(|| command.profile_repeated_input_frame_mutations(8_192));
        let input_after = command.profile_timeline_counters();
        assert_eq!(
            input_after.logical_payload_admissions,
            input_before.logical_payload_admissions
        );
        assert_eq!(input_after.full_frame_history_clones, 0);
        assert_eq!(
            input_after.logical_records - input_before.logical_records,
            1
        );
        assert_eq!(
            input_after.logical_coalesced_mutations - input_before.logical_coalesced_mutations,
            8_191
        );
        assert!(input_after.logical_record_bytes - input_before.logical_record_bytes <= 48);
        let logical_history = LogicalHistoryCounts {
            payload_admissions_per_frame: input_before.logical_payload_admissions / logical_frames,
            full_frame_history_clones: input_after.full_frame_history_clones,
            records: input_after.logical_records - input_before.logical_records,
            record_bytes: input_after.logical_record_bytes - input_before.logical_record_bytes,
            coalesced_mutations: input_after.logical_coalesced_mutations
                - input_before.logical_coalesced_mutations,
        };
        command
            .restore_summary(&summary, universe)
            .expect("input-frame coalescing cleanup restores");
        let (candidate, fork) = measure(|| {
            CommandState::profile_fork_summary(command, &summary, universe)
                .expect("checkpoint forks")
        });
        assert_eq!(candidate.input_level_count(), units * 2);
        let mut command = candidate;
        command.reject_checkpoint_candidate();
        let (candidate, fork_first_mutation) = measure(|| {
            let mut candidate = CommandState::profile_fork_summary(command, &summary, universe)
                .expect("checkpoint forks again");
            candidate.profile_first_timeline_mutation();
            assert!(candidate.profile_name_in_progress());
            candidate
        });
        let mut command = candidate;
        command.reject_checkpoint_candidate();
        let isolated = CommandState::profile_fork_summary(command, &summary, universe)
            .expect("fork restores exactly");
        assert!(!isolated.profile_name_in_progress());
        drop(isolated);

        GateCounts {
            capture,
            clone: clone_counts,
            restore,
            first_mutation,
            fork,
            fork_first_mutation,
            repeated_scalar_mutations,
            repeated_input_frame_mutations,
            logical_history,
        }
    })
    .expect("checkpoint gate universe")
}

fn measure<T>(operation: impl FnOnce() -> T) -> (T, Counts) {
    let region = Region::new(GLOBAL);
    let value = operation();
    let stats = region.change();
    (
        value,
        Counts {
            allocations: stats.allocations,
            bytes: stats.bytes_allocated,
        },
    )
}

fn budget() -> InternerBudget {
    InternerBudget::new(65_536, 131_072, 16 * 1024 * 1024).expect("benchmark budget")
}
