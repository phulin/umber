use std::alloc::System;
use std::sync::Arc;
use std::time::{Duration, Instant};

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
    release: Counts,
    repeated_scalar_mutations: Counts,
    repeated_input_frame_mutations: Counts,
    repeated_input_level_reuse: Counts,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PrefixPlateauCounts {
    boundaries: usize,
    live_frames: usize,
    frame_capacity: usize,
    journal_chunks_released: usize,
    elapsed: Duration,
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
        ("release", shallow.release),
        (
            "repeated_scalar_mutations",
            shallow.repeated_scalar_mutations,
        ),
        (
            "repeated_input_frame_mutations",
            shallow.repeated_input_frame_mutations,
        ),
        (
            "repeated_input_level_reuse",
            shallow.repeated_input_level_reuse,
        ),
    ] {
        assert_eq!(counts, Counts::ZERO, "{name} must remain allocation-free");
    }
    let prefix_plateau = run_prefix_plateau(10_000_000);
    assert_eq!(prefix_plateau.live_frames, 1);
    assert_eq!(prefix_plateau.frame_capacity, 128);
    assert_eq!(
        prefix_plateau.journal_chunks_released,
        prefix_plateau.boundaries
    );
    println!(
        "COMMAND_CHECKPOINT_GATE capture={:?} clone={:?} restore={:?} first_mutation={:?} fork={:?} fork_first_mutation={:?} release={:?} repeated_scalar_mutations={:?} repeated_input_frame_mutations={:?} repeated_input_level_reuse={:?} logical_history={:?} prefix_plateau={:?}",
        shallow.capture,
        shallow.clone,
        shallow.restore,
        shallow.first_mutation,
        shallow.fork,
        shallow.fork_first_mutation,
        shallow.release,
        shallow.repeated_scalar_mutations,
        shallow.repeated_input_frame_mutations,
        shallow.repeated_input_level_reuse,
        shallow.logical_history,
        prefix_plateau,
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
        command.profile_repeated_input_level_reuse(
            &universe.command_context().expect("reuse warmup context"),
            tokens.clone(),
            1,
        );
        let reuse_before = command.profile_timeline_counters();
        let (_, repeated_input_level_reuse) = measure(|| {
            command.profile_repeated_input_level_reuse(
                &universe
                    .command_context()
                    .expect("reuse measurement context"),
                tokens.clone(),
                8_192,
            );
        });
        let reuse_after = command.profile_timeline_counters();
        assert_eq!(
            reuse_after.logical_records, reuse_before.logical_records,
            "unobserved input-frame reuse appends no rollback history"
        );
        assert_eq!(
            reuse_after.displaced_payloads, reuse_before.displaced_payloads,
            "unobserved input-frame reuse retains no displaced payload"
        );
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
        let released = command
            .publish_summary(universe)
            .expect("obsolete summary publishes");
        let survivor = command
            .publish_summary(universe)
            .expect("surviving summary publishes");
        let (receipt, release) = measure(|| {
            command
                .release_checkpoint_summary(&released, Some(&survivor))
                .expect("obsolete command frame releases")
        });
        assert!(receipt.timeline_frames_live() < receipt.timeline_frame_capacity());
        assert_ne!(receipt.timeline_frames_released(), 0);
        let isolated = CommandState::profile_fork_summary(command, &survivor, universe)
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
            release,
            repeated_scalar_mutations,
            repeated_input_frame_mutations,
            repeated_input_level_reuse,
            logical_history,
        }
    })
    .expect("checkpoint gate universe")
}

fn run_prefix_plateau(boundaries: usize) -> PrefixPlateauCounts {
    tex_state::with_universe(budget(), |universe| {
        let mut command = CommandState::new(CommandProfile::TEX82);
        let job_start = command
            .publish_summary(universe)
            .expect("JobStart summary publishes");
        command
            .release_checkpoint_summary(&job_start, None)
            .expect("frozen JobStart releases its live frame");
        let mut prior = command
            .publish_summary(universe)
            .expect("first ordinary summary publishes");
        let mut journal_chunks_released = 0usize;
        let started = Instant::now();
        let mut final_receipt = None;
        for _ in 0..boundaries {
            command.profile_repeated_timeline_mutations(1);
            let next = command
                .publish_summary(universe)
                .expect("next ordinary summary publishes");
            let receipt = command
                .release_checkpoint_summary(&prior, Some(&next))
                .expect("obsolete prefix releases");
            journal_chunks_released =
                journal_chunks_released.saturating_add(receipt.command_journal_chunks_released());
            final_receipt = Some(receipt);
            prior = next;
        }
        let elapsed = started.elapsed();
        let final_receipt = final_receipt.expect("positive boundary measurement");
        PrefixPlateauCounts {
            boundaries,
            live_frames: final_receipt.timeline_frames_live(),
            frame_capacity: final_receipt.timeline_frame_capacity(),
            journal_chunks_released,
            elapsed,
        }
    })
    .expect("prefix plateau universe")
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
