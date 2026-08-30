use std::alloc::System;
use std::sync::Arc;
use std::time::{Duration, Instant};

use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};
use tex_command::{CommandProfile, CommandState, RegisteredSourceKind, SourceRegistration};
use tex_state::GroupKind;
use tex_state::interner::InternerBudget;
use tex_state::token::{Catcode, OriginId, Token, TokenWord, TracedTokenWord};

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SourceHistoryCounts {
    lex_first_touch: Counts,
    cold_owner_swap: Counts,
    lex_ordered_row_reuse: Counts,
    cold_ordered_row_reuse: Counts,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SourceDepthCounts {
    mutations: usize,
    allocations: Counts,
    records: u64,
    record_bytes: u64,
    stored_state_captures: u64,
    coalesced_mutations: u64,
    owner_swaps: u64,
    full_frame_history_clones: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CursorStructureCounts {
    summary_bytes: usize,
    cursor_bytes: usize,
    capture: Counts,
    restore: Counts,
    capture_records: u64,
    restore_records: u64,
    capture_full_payload_clones: u64,
    restore_full_payload_clones: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SettlementWork {
    selected_rewind_records: u64,
    candidate_reject_records: u64,
    accepted_redo_records: u64,
    candidate_chunks_released: u64,
    accepted_chunks_released: u64,
    frame_chain_transfers: u64,
    frame_reuse_link_visits: u64,
    frame_reuse_visits: u64,
    frame_reuse_incarnations: u64,
    settlement_allocations: Counts,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FrameDiscardWork {
    chain_transfers: u64,
    settlement_visits: u64,
    settlement_reuse_visits: u64,
    settlement_reuse_incarnations: u64,
    settlement_allocations: Counts,
    lazy_reuse_visits: u64,
    lazy_reuse_incarnations: u64,
}

fn main() {
    let shallow = run_fixture(1);
    let accumulated = run_fixture(64);
    let source_history = run_source_history_fixture();
    let source_depth_one = run_source_depth_fixture(1);
    let source_depth_many = run_source_depth_fixture(4_096);
    let cursor_shallow = run_cursor_structure_fixture(1, 1);
    let cursor_deep = run_cursor_structure_fixture(4_096, 1);
    let cursor_aftergroups = run_cursor_structure_fixture(1, 65_536);
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
    assert_eq!(source_history.lex_first_touch, Counts::ZERO);
    assert_eq!(source_history.cold_owner_swap, Counts::ZERO);
    assert_eq!(source_history.lex_ordered_row_reuse, Counts::ZERO);
    assert_eq!(source_history.cold_ordered_row_reuse, Counts::ZERO);
    assert_eq!(source_depth_one, source_depth_many);
    assert_eq!(source_depth_one.allocations, Counts::ZERO);
    assert_eq!(source_depth_one.records, 1);
    assert!(source_depth_one.record_bytes <= 48);
    assert_eq!(source_depth_one.stored_state_captures, 1);
    assert_eq!(source_depth_one.coalesced_mutations, 4_095);
    assert_eq!(source_depth_one.owner_swaps, 0);
    assert_eq!(source_depth_one.full_frame_history_clones, 0);
    assert_eq!(cursor_shallow, cursor_deep);
    assert_eq!(cursor_shallow, cursor_aftergroups);
    assert_eq!(cursor_shallow.cursor_bytes, std::mem::size_of::<u32>());
    assert_eq!(cursor_shallow.capture, Counts::ZERO);
    assert_eq!(cursor_shallow.restore, Counts::ZERO);
    assert_eq!(cursor_shallow.capture_records, 0);
    assert_eq!(cursor_shallow.restore_records, 0);
    assert_eq!(cursor_shallow.capture_full_payload_clones, 0);
    assert_eq!(cursor_shallow.restore_full_payload_clones, 0);
    let rejected = run_settlement_work(73, 5, false);
    assert_eq!(rejected.selected_rewind_records, 73);
    assert_eq!(rejected.candidate_reject_records, 5);
    assert_eq!(rejected.accepted_redo_records, 73);
    assert_eq!(rejected.candidate_chunks_released, 5);
    assert_eq!(rejected.accepted_chunks_released, 0);
    assert_eq!(rejected.frame_chain_transfers, 1);
    assert_eq!(rejected.frame_reuse_link_visits, 0);
    assert_eq!(rejected.frame_reuse_visits, 0);
    assert_eq!(rejected.frame_reuse_incarnations, 0);
    assert_eq!(rejected.settlement_allocations, Counts::ZERO);
    let accepted = run_settlement_work(73, 5, true);
    assert_eq!(accepted.selected_rewind_records, 73);
    assert_eq!(accepted.candidate_reject_records, 0);
    assert_eq!(accepted.accepted_redo_records, 0);
    assert_eq!(accepted.candidate_chunks_released, 0);
    assert_eq!(accepted.accepted_chunks_released, 73);
    assert_eq!(accepted.frame_chain_transfers, 1);
    assert_eq!(accepted.frame_reuse_link_visits, 0);
    assert_eq!(accepted.frame_reuse_visits, 0);
    assert_eq!(accepted.frame_reuse_incarnations, 0);
    assert_eq!(accepted.settlement_allocations, Counts::ZERO);
    let rejected_one = run_frame_discard_work(1, false);
    let rejected_many = run_frame_discard_work(4_096, false);
    assert_eq!(rejected_one, rejected_many);
    let accepted_one = run_frame_discard_work(1, true);
    let accepted_many = run_frame_discard_work(4_096, true);
    assert_eq!(accepted_one, accepted_many);
    for work in [rejected_one, accepted_one] {
        assert_eq!(work.chain_transfers, 1);
        assert_eq!(work.settlement_visits, 0);
        assert_eq!(work.settlement_reuse_visits, 0);
        assert_eq!(work.settlement_reuse_incarnations, 0);
        assert_eq!(work.settlement_allocations, Counts::ZERO);
        assert_eq!(work.lazy_reuse_visits, 1);
        assert_eq!(work.lazy_reuse_incarnations, 1);
    }
    let prefix_plateau = run_prefix_plateau(10_000_000);
    assert_eq!(prefix_plateau.live_frames, 1);
    assert_eq!(prefix_plateau.frame_capacity, 128);
    assert_eq!(
        prefix_plateau.journal_chunks_released,
        prefix_plateau.boundaries
    );
    println!(
        "COMMAND_CHECKPOINT_GATE capture={:?} clone={:?} restore={:?} first_mutation={:?} fork={:?} fork_first_mutation={:?} release={:?} repeated_scalar_mutations={:?} repeated_input_frame_mutations={:?} repeated_input_level_reuse={:?} logical_history={:?} source_history={:?} source_depth={:?} cursor_structure={:?} rejected_settlement={:?} accepted_settlement={:?} rejected_frame_discard={:?} accepted_frame_discard={:?} prefix_plateau={:?}",
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
        source_history,
        source_depth_one,
        cursor_shallow,
        rejected,
        accepted,
        rejected_one,
        accepted_one,
        prefix_plateau,
    );
}

fn run_cursor_structure_fixture(
    group_depth: usize,
    aftergroup_payloads: usize,
) -> CursorStructureCounts {
    tex_state::with_universe(budget(), |universe| {
        let mut command = CommandState::new(CommandProfile::TEX82);
        {
            let mut state = universe.command_context().expect("cursor fixture context");
            for _ in 0..group_depth {
                command
                    .begin_group(&mut state, GroupKind::Simple, 0)
                    .expect("cursor fixture group opens");
            }
            let spelling = TracedTokenWord::pack(
                Token::Char {
                    ch: 'x',
                    cat: Catcode::Other,
                },
                OriginId::UNKNOWN,
            );
            for _ in 0..aftergroup_payloads {
                command
                    .save_aftergroup(&state, spelling)
                    .expect("cursor fixture aftergroup saves");
            }
        }
        assert_eq!(
            command.aftergroup_save_stack_projection().0,
            aftergroup_payloads
        );

        let warm = command
            .publish_summary(universe)
            .expect("cursor fixture warms checkpoint storage");
        drop(warm);
        let before_capture = command.profile_timeline_counters();
        let (summary, capture) = measure(|| {
            command
                .publish_summary(universe)
                .expect("cursor fixture captures")
        });
        let after_capture = command.profile_timeline_counters();
        let summary_bytes = std::mem::size_of_val(&summary);
        let cursor_bytes = std::mem::size_of_val(&summary.cursor());
        let (_, restore) = measure(|| {
            command
                .restore_summary(&summary, universe)
                .expect("cursor fixture restores")
        });
        let after_restore = command.profile_timeline_counters();

        CursorStructureCounts {
            summary_bytes,
            cursor_bytes,
            capture,
            restore,
            capture_records: after_capture
                .logical_records
                .saturating_sub(before_capture.logical_records),
            restore_records: after_restore
                .logical_records
                .saturating_sub(after_capture.logical_records),
            capture_full_payload_clones: after_capture
                .full_frame_history_clones
                .saturating_sub(before_capture.full_frame_history_clones),
            restore_full_payload_clones: after_restore
                .full_frame_history_clones
                .saturating_sub(after_capture.full_frame_history_clones),
        }
    })
    .expect("cursor structure gate universe")
}

fn run_source_depth_fixture(depth: usize) -> SourceDepthCounts {
    tex_state::with_universe(budget(), |universe| {
        let bytes = Arc::<[u8]>::from(&b"a"[..]);
        let mut command = CommandState::new(CommandProfile::TEX82);
        for _ in 0..depth {
            let source = command
                .register_source(SourceRegistration::new(
                    RegisteredSourceKind::Generated,
                    Arc::clone(&bytes),
                ))
                .expect("source-depth fixture source");
            command
                .open_registered_source(source)
                .expect("source-depth fixture opens");
        }
        command.profile_prepare_source_line(13);
        let summary = command
            .publish_summary(universe)
            .expect("source-depth checkpoint");

        command.profile_repeated_source_lex_mutations(1);
        command
            .restore_summary(&summary, universe)
            .expect("source-depth warmup restores");
        let before = command.profile_timeline_counters();
        let (_, allocations) = measure(|| command.profile_repeated_source_lex_mutations(4_096));
        let after = command.profile_timeline_counters();

        SourceDepthCounts {
            mutations: 4_096,
            allocations,
            records: after.logical_records.saturating_sub(before.logical_records),
            record_bytes: after
                .logical_record_bytes
                .saturating_sub(before.logical_record_bytes),
            stored_state_captures: after
                .logical_stored_state_captures
                .saturating_sub(before.logical_stored_state_captures),
            coalesced_mutations: after
                .logical_coalesced_mutations
                .saturating_sub(before.logical_coalesced_mutations),
            owner_swaps: after
                .logical_owner_swaps
                .saturating_sub(before.logical_owner_swaps),
            full_frame_history_clones: after
                .full_frame_history_clones
                .saturating_sub(before.full_frame_history_clones),
        }
    })
    .expect("source-depth gate universe")
}

fn run_settlement_work(
    accepted_intervals: usize,
    candidate_intervals: usize,
    accept: bool,
) -> SettlementWork {
    tex_state::with_universe(budget(), |universe| {
        let mut command = CommandState::new(CommandProfile::TEX82);
        let selected = command
            .publish_summary(universe)
            .expect("selected settlement summary publishes");
        let mut accepted_marks = Vec::with_capacity(accepted_intervals);
        for _ in 0..accepted_intervals {
            command.profile_repeated_timeline_mutations(1);
            accepted_marks.push(
                command
                    .publish_summary(universe)
                    .expect("accepted settlement mark publishes"),
            );
        }
        let before = command.profile_timeline_counters();
        let mut candidate = CommandState::profile_fork_summary(command, &selected, universe)
            .expect("settlement candidate forks");
        let mut candidate_marks = Vec::with_capacity(candidate_intervals);
        for _ in 0..candidate_intervals {
            candidate.profile_repeated_timeline_mutations(1);
            candidate_marks.push(
                candidate
                    .publish_summary(universe)
                    .expect("candidate settlement mark publishes"),
            );
        }
        let before_settlement = candidate.profile_timeline_counters();
        let (_, settlement_allocations) = measure(|| {
            if accept {
                candidate.accept_checkpoint_candidate();
            } else {
                candidate.reject_checkpoint_candidate();
            }
        });
        let after = candidate.profile_timeline_counters();
        drop(candidate_marks);
        drop(accepted_marks);
        SettlementWork {
            selected_rewind_records: after
                .selected_rewind_records
                .saturating_sub(before.selected_rewind_records),
            candidate_reject_records: after
                .candidate_reject_records
                .saturating_sub(before.candidate_reject_records),
            accepted_redo_records: after
                .accepted_redo_records
                .saturating_sub(before.accepted_redo_records),
            candidate_chunks_released: after
                .candidate_chunks_released
                .saturating_sub(before.candidate_chunks_released),
            accepted_chunks_released: after
                .accepted_chunks_released
                .saturating_sub(before.accepted_chunks_released),
            frame_chain_transfers: after
                .frame_chain_transfers
                .saturating_sub(before_settlement.frame_chain_transfers),
            frame_reuse_link_visits: after
                .frame_reuse_link_visits
                .saturating_sub(before_settlement.frame_reuse_link_visits),
            frame_reuse_visits: after
                .frame_reuse_visits
                .saturating_sub(before_settlement.frame_reuse_visits),
            frame_reuse_incarnations: after
                .frame_reuse_incarnations
                .saturating_sub(before_settlement.frame_reuse_incarnations),
            settlement_allocations,
        }
    })
    .expect("settlement gate universe")
}

fn run_frame_discard_work(discarded_frames: usize, accept: bool) -> FrameDiscardWork {
    tex_state::with_universe(budget(), |universe| {
        let mut command = CommandState::new(CommandProfile::TEX82);
        let selected = command
            .publish_summary(universe)
            .expect("frame-discard selected summary publishes");
        let mut accepted_marks = Vec::with_capacity(discarded_frames);
        for _ in 0..discarded_frames {
            accepted_marks.push(
                command
                    .publish_summary(universe)
                    .expect("frame-discard accepted summary publishes"),
            );
        }
        let mut candidate = CommandState::profile_fork_summary(command, &selected, universe)
            .expect("frame-discard selected prefix forks");
        let candidate_frames = if accept { 1 } else { discarded_frames };
        let mut candidate_marks = Vec::with_capacity(candidate_frames);
        for _ in 0..candidate_frames {
            candidate_marks.push(
                candidate
                    .publish_summary(universe)
                    .expect("frame-discard candidate summary publishes"),
            );
        }

        let before = candidate.profile_timeline_counters();
        let (_, settlement_allocations) = measure(|| {
            if accept {
                candidate.accept_checkpoint_candidate();
            } else {
                candidate.reject_checkpoint_candidate();
            }
        });
        let settled = candidate.profile_timeline_counters();
        let before_reuse = settled;
        let _reused = candidate
            .publish_summary(universe)
            .expect("frame-discard row reuses lazily");
        let after_reuse = candidate.profile_timeline_counters();
        drop(candidate_marks);
        drop(accepted_marks);

        FrameDiscardWork {
            chain_transfers: settled
                .frame_chain_transfers
                .saturating_sub(before.frame_chain_transfers),
            settlement_visits: settled
                .frame_reuse_link_visits
                .saturating_sub(before.frame_reuse_link_visits),
            settlement_reuse_visits: settled
                .frame_reuse_visits
                .saturating_sub(before.frame_reuse_visits),
            settlement_reuse_incarnations: settled
                .frame_reuse_incarnations
                .saturating_sub(before.frame_reuse_incarnations),
            settlement_allocations,
            lazy_reuse_visits: after_reuse
                .frame_reuse_visits
                .saturating_sub(before_reuse.frame_reuse_visits),
            lazy_reuse_incarnations: after_reuse
                .frame_reuse_incarnations
                .saturating_sub(before_reuse.frame_reuse_incarnations),
        }
    })
    .expect("frame-discard gate universe")
}

fn run_source_history_fixture() -> SourceHistoryCounts {
    tex_state::with_universe(budget(), |universe| {
        let mut command = CommandState::new(CommandProfile::TEX82);
        let source = command
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(&b"a\nb"[..]),
            ))
            .expect("source-history fixture source");
        command
            .open_registered_source(source)
            .expect("source-history fixture opens");
        command.profile_prepare_source_line(13);
        let token_words = [TokenWord::pack(Token::Char {
            ch: 'x',
            cat: Catcode::Other,
        })];
        let tokens = universe
            .command_context()
            .expect("source-history token context")
            .allocate_token_list(&token_words)
            .expect("source-history token list");
        let summary = command
            .publish_summary(universe)
            .expect("source-history checkpoint");

        let before = command.profile_timeline_counters();
        let (_, lex_first_touch) = measure(|| command.profile_repeated_source_lex_mutations(8_192));
        let after = command.profile_timeline_counters();
        assert_eq!(after.full_frame_history_clones, 0);
        assert_eq!(
            after.logical_stored_state_captures - before.logical_stored_state_captures,
            1
        );
        assert_eq!(after.logical_owner_swaps, before.logical_owner_swaps);
        assert_eq!(
            after.logical_coalesced_mutations - before.logical_coalesced_mutations,
            8_191
        );
        command
            .restore_summary(&summary, universe)
            .expect("source lexer cleanup restores");

        let before = command.profile_timeline_counters();
        let (_, cold_owner_swap) = measure(|| command.profile_advance_source_line(13));
        let after = command.profile_timeline_counters();
        assert_eq!(after.full_frame_history_clones, 0);
        assert_eq!(after.logical_owner_swaps - before.logical_owner_swaps, 1);
        assert_eq!(
            after.logical_stored_state_captures,
            before.logical_stored_state_captures
        );
        command
            .restore_summary(&summary, universe)
            .expect("source owner cleanup restores");

        {
            let stores = universe
                .command_context()
                .expect("source-history warm reuse context");
            command.profile_source_lex_then_token_row_reuse(&stores, tokens.clone());
        }
        command
            .restore_summary(&summary, universe)
            .expect("source lexer reuse warmup restores");
        let before = command.profile_timeline_counters();
        let (_, lex_ordered_row_reuse) = measure(|| {
            let stores = universe
                .command_context()
                .expect("source-history measured reuse context");
            command.profile_source_lex_then_token_row_reuse(&stores, tokens.clone());
        });
        let after = command.profile_timeline_counters();
        assert_eq!(after.full_frame_history_clones, 0);
        assert_eq!(after.logical_records - before.logical_records, 2);
        assert_eq!(
            after.logical_stored_state_captures - before.logical_stored_state_captures,
            1
        );
        assert_eq!(after.logical_owner_swaps, before.logical_owner_swaps);
        command
            .restore_summary(&summary, universe)
            .expect("source lexer ordered reuse restores");

        {
            let stores = universe
                .command_context()
                .expect("source-history warm owner reuse context");
            command.profile_source_owner_then_token_row_reuse(&stores, tokens.clone(), 13);
        }
        command
            .restore_summary(&summary, universe)
            .expect("source owner reuse warmup restores");
        let before = command.profile_timeline_counters();
        let (_, cold_ordered_row_reuse) = measure(|| {
            let stores = universe
                .command_context()
                .expect("source-history measured owner reuse context");
            command.profile_source_owner_then_token_row_reuse(&stores, tokens.clone(), 13);
        });
        let after = command.profile_timeline_counters();
        assert_eq!(after.full_frame_history_clones, 0);
        assert_eq!(after.logical_records - before.logical_records, 2);
        assert_eq!(after.logical_owner_swaps - before.logical_owner_swaps, 1);
        command
            .restore_summary(&summary, universe)
            .expect("source owner ordered reuse restores");

        SourceHistoryCounts {
            lex_first_touch,
            cold_owner_swap,
            lex_ordered_row_reuse,
            cold_ordered_row_reuse,
        }
    })
    .expect("source-history universe")
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
