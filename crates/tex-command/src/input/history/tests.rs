use tex_state::token::{Catcode, OriginId, Token, TokenWord, TracedTokenWord};

#[cfg(feature = "profiling")]
use tex_state::measurement::HotCoreAllocationOwner;

use crate::input::{
    InputLevel, InputLevelId, MacroArgumentCursor, PackedTokenSpanHandle, PackedTokenSpanSource,
    ReplayTrace, RetirementBehavior, TokenBehavior, TokenCursor, packed_token_frame,
};

fn word(ch: char) -> TracedTokenWord {
    TracedTokenWord::from_parts(
        TokenWord::pack(Token::Char {
            ch,
            cat: Catcode::Other,
        }),
        OriginId::UNKNOWN,
    )
}

fn assert_exact_direct_transition<G>(
    state: &mut crate::CommandState<G>,
    expected_branches: (u64, u64, u64),
    mut mutate: impl FnMut(&mut crate::CommandState<G>),
) {
    let checkpoint = state.input.levels.mark().expect("input checkpoint");
    state.input.levels.reset_cursor_mutation_counters();
    let opening_revision = state.input.levels.context_revision();
    let before_history = state.input.levels.counters();
    #[cfg(feature = "profiling")]
    let owner = HotCoreAllocationOwner::DeliveryAndScan;
    #[cfg(feature = "profiling")]
    let before_allocations = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
    {
        #[cfg(feature = "profiling")]
        let _scope = tex_state::measurement::hot_core_allocation_scope(owner);
        mutate(state);
        mutate(state);
    }
    #[cfg(feature = "profiling")]
    let after_allocations = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
    let after_history = state.input.levels.counters();

    assert_eq!(
        state.input.levels.cursor_mutation_counters(),
        super::InputCursorMutationCounters {
            typed_top_accesses: 2,
            source_branch_entries: expected_branches.0,
            stored_token_branch_entries: expected_branches.1,
            macro_argument_branch_entries: expected_branches.2,
            first_touch_transitions: 1,
            closure_dispatches: 0,
        }
    );
    assert_eq!(after_history.undo_records - before_history.undo_records, 1);
    assert_eq!(
        state.input.levels.context_revision(),
        opening_revision.wrapping_add(2).max(1)
    );
    assert_eq!(
        state.input.levels.as_slice().last().map(cursor_position),
        Some(2)
    );

    state.input.levels.begin_checkpoint_candidate(checkpoint);
    assert_eq!(
        state.input.levels.as_slice().last().map(cursor_position),
        Some(0)
    );
    state.input.levels.reject_checkpoint_candidate();
    assert_eq!(
        state.input.levels.as_slice().last().map(cursor_position),
        Some(2)
    );
    #[cfg(feature = "profiling")]
    {
        assert_eq!(after_allocations.calls - before_allocations.calls, 0);
        assert_eq!(
            after_allocations.requested_bytes - before_allocations.requested_bytes,
            0
        );
    }
}

fn cursor_position<G>(level: &InputLevel<G>) -> usize {
    match level {
        InputLevel::Tokens(cursor) => cursor.position(),
        InputLevel::MacroArgument(cursor) => cursor.position(),
        InputLevel::Source(_) => panic!("fixture admits a stored-token cursor"),
    }
}

#[test]
fn token_cursor_mutation_is_one_typed_access_and_one_coalesced_journal_transition() {
    crate::test_harness::with_universe(|universe| {
        let mut context = universe.command_context().expect("command context");
        let behavior = TokenBehavior::Ordinary;
        let retirement = RetirementBehavior::Pop;
        let trace = ReplayTrace::Inserted;
        let mut state = crate::CommandState::default();
        let span = PackedTokenSpanHandle::transient([word('a'), word('b')])
            .admit(&mut state.input.replay)
            .expect("token span admits");
        let mut fuel = crate::CommandFuelLedger::default();
        state.input.levels.push(InputLevel::Tokens(TokenCursor {
            span,
            frame: packed_token_frame(InputLevelId(1), 2, &behavior, retirement, &trace),
            behavior,
            retirement,
            trace,
        }));

        assert_exact_direct_transition(&mut state, (0, 2, 0), |state| {
            let mut command = crate::command::CurrentCommand::empty();
            let delivery = state
                .advance_resident_command_into(
                    &mut context,
                    fuel.fuel_mut(),
                    true,
                    command.empty_for_raw_delivery(),
                    (&mut None, &mut None),
                )
                .expect("token delivery succeeds");
            assert!(matches!(
                delivery,
                crate::input::ResidentCommandTransition::Delivered { .. }
            ));
        });
    });
}

#[test]
fn macro_argument_mutation_uses_the_same_direct_transition() {
    crate::test_harness::with_universe(|universe| {
        let mut context = universe.command_context().expect("command context");
        let mut state = crate::CommandState::default();
        let matching = state.scratch.begin_macro_match().expect("macro match");
        let mut buffer = state
            .scratch
            .begin_argument_collector(&matching)
            .expect("match writer");
        for spelling in [word('a'), word('b')] {
            state
                .scratch
                .settle_argument_token(
                    &mut buffer,
                    crate::token_collector::ClassifiedToken::from_word(spelling, None),
                    true,
                )
                .expect("argument word");
        }
        state
            .scratch
            .finish_argument_collector(buffer)
            .expect("argument range");
        let macro_frame = state
            .scratch
            .commit_macro_match(matching)
            .expect("sealed macro frame");
        let range = state
            .scratch
            .argument_range(macro_frame, 1)
            .expect("live macro frame")
            .expect("first argument");
        let behavior = TokenBehavior::Parameter;
        let retirement = RetirementBehavior::Pop;
        let trace = ReplayTrace::MacroParameter { slot: 1 };
        state
            .input
            .levels
            .push(InputLevel::MacroArgument(MacroArgumentCursor {
                range,
                slot: 1,
                frame: packed_token_frame(InputLevelId(2), 2, &behavior, retirement, &trace),
            }));
        let mut fuel = crate::CommandFuelLedger::default();

        assert_exact_direct_transition(&mut state, (0, 0, 2), |state| {
            let mut command = crate::command::CurrentCommand::empty();
            let delivery = state
                .advance_resident_command_into(
                    &mut context,
                    fuel.fuel_mut(),
                    true,
                    command.empty_for_raw_delivery(),
                    (&mut None, &mut None),
                )
                .expect("macro-argument delivery succeeds");
            assert!(matches!(
                delivery,
                crate::input::ResidentCommandTransition::Delivered { .. }
            ));
        });
    });
}

#[test]
#[cfg(feature = "profiling")]
fn resident_source_delivery_skips_owner_validation_at_one_and_4096_operations() {
    fn run(operations: usize) -> ((u64, u64, u64, u64), u64, u64, u64) {
        crate::test_harness::with_universe(|universe| {
            let mut state = crate::CommandState::default();
            let source = state
                .register_source(crate::SourceRegistration::new(
                    crate::RegisteredSourceKind::Generated,
                    std::sync::Arc::<[u8]>::from(vec![b'x'; operations + 1]),
                ))
                .expect("source delivery fixture registers");
            state
                .open_registered_source(source)
                .expect("source delivery fixture opens");
            state.profile_prepare_source_line(1);

            let mut context = universe.command_context().expect("command context");
            let mut capabilities = crate::CommandHostCapabilities::default();
            let mut fuel = crate::CommandFuelLedger::new(
                u64::try_from(operations).expect("operation count fits u64") + 16,
            )
            .expect("source delivery fixture fuel limit");
            let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
            let mut command_slot = None;

            state.profile_reset_input_source_context_counters();
            let copies_before = state.profile_timeline_counters().full_frame_history_clones;
            let owner = HotCoreAllocationOwner::DeliveryAndScan;
            let allocations_before =
                tex_state::measurement::hot_core_thread_allocation_measurement(owner);
            {
                let _scope = tex_state::measurement::hot_core_allocation_scope(owner);
                let mut processor = crate::test_harness::processor(
                    &mut state,
                    &mut context,
                    &mut capabilities,
                    &mut fuel,
                    &mut diagnostic_effects,
                );
                for _ in 0..operations {
                    assert!(matches!(
                        processor.get_next_into(&mut command_slot),
                        Ok(crate::DeliveryStatus::Command)
                    ));
                    command_slot = None;
                }
            }
            let allocations_after =
                tex_state::measurement::hot_core_thread_allocation_measurement(owner);
            let copies_after = state.profile_timeline_counters().full_frame_history_clones;
            (
                state.profile_input_source_context_counters(),
                allocations_after.calls - allocations_before.calls,
                allocations_after.requested_bytes - allocations_before.requested_bytes,
                copies_after - copies_before,
            )
        })
    }

    let one = run(1);
    let four_k = run(4_096);
    assert_eq!(one, ((0, 0, 0, 1), 0, 0, 0));
    assert_eq!(four_k, ((0, 0, 0, 4_096), 0, 0, 0));
}

#[test]
#[cfg(feature = "profiling")]
fn one_and_4096_typed_resident_branches_select_and_write_exactly_once() {
    #[derive(Clone, Copy, Debug)]
    enum Branch {
        Source,
        StoredToken,
        MacroArgument,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct Evidence {
        branches: (u64, u64, u64, u64),
        paths: (u64, u64, u64, u64),
        transitions: (u64, u64, u64),
        allocation_calls: u64,
        requested_bytes: u64,
        whole_row_copies: u64,
        whole_input_frame_copies: u64,
        whole_command_copies: u64,
        final_command_writes: u64,
    }

    fn run(branch: Branch, operations: usize) -> Evidence {
        crate::test_harness::with_universe(|universe| {
            let mut state = crate::CommandState::default();
            match branch {
                Branch::Source => {
                    let source = state
                        .register_source(crate::SourceRegistration::new(
                            crate::RegisteredSourceKind::Generated,
                            std::sync::Arc::<[u8]>::from(vec![b'x'; operations]),
                        ))
                        .expect("source fixture registers");
                    state
                        .open_registered_source(source)
                        .expect("source fixture opens");
                    state.profile_prepare_source_line(1);
                }
                Branch::StoredToken => {
                    let span = PackedTokenSpanHandle::transient(std::iter::repeat_n(
                        word('x'),
                        operations,
                    ))
                    .admit(&mut state.input.replay)
                    .expect("stored fixture admits");
                    let behavior = TokenBehavior::Ordinary;
                    let retirement = RetirementBehavior::Pop;
                    let trace = ReplayTrace::Inserted;
                    state.input.levels.push(InputLevel::Tokens(TokenCursor {
                        span,
                        frame: packed_token_frame(
                            InputLevelId(1),
                            operations,
                            &behavior,
                            retirement,
                            &trace,
                        ),
                        behavior,
                        retirement,
                        trace,
                    }));
                }
                Branch::MacroArgument => {
                    let matching = state.scratch.begin_macro_match().expect("macro match");
                    let mut writer = state
                        .scratch
                        .begin_argument_collector(&matching)
                        .expect("macro writer");
                    for _ in 0..operations {
                        state
                            .scratch
                            .settle_argument_token(
                                &mut writer,
                                crate::token_collector::ClassifiedToken::from_word(word('x'), None),
                                true,
                            )
                            .expect("macro argument word");
                    }
                    state
                        .scratch
                        .finish_argument_collector(writer)
                        .expect("macro argument range");
                    let macro_frame = state
                        .scratch
                        .commit_macro_match(matching)
                        .expect("macro frame");
                    let range = state
                        .scratch
                        .argument_range(macro_frame, 1)
                        .expect("live macro frame")
                        .expect("macro argument");
                    let behavior = TokenBehavior::Parameter;
                    let retirement = RetirementBehavior::Pop;
                    let trace = ReplayTrace::MacroParameter { slot: 1 };
                    state
                        .input
                        .levels
                        .push(InputLevel::MacroArgument(MacroArgumentCursor {
                            range,
                            slot: 1,
                            frame: packed_token_frame(
                                InputLevelId(1),
                                operations,
                                &behavior,
                                retirement,
                                &trace,
                            ),
                        }));
                }
            }

            let mut context = universe.command_context().expect("resident context");
            let mut fuel = crate::CommandFuelLedger::default();
            state.profile_reset_input_cursor_mutation_counters();
            state.profile_reset_raw_delivery_path_counters();
            let timeline_before = state.profile_timeline_counters();
            let commands_before = crate::command::command_ownership_counters();
            let owner = HotCoreAllocationOwner::DeliveryAndScan;
            let allocations_before =
                tex_state::measurement::hot_core_thread_allocation_measurement(owner);
            {
                let _scope = tex_state::measurement::hot_core_allocation_scope(owner);
                for _ in 0..operations {
                    let mut command = crate::command::CurrentCommand::empty();
                    let transition = state
                        .advance_resident_command_into(
                            &mut context,
                            fuel.fuel_mut(),
                            true,
                            command.empty_for_raw_delivery(),
                            (&mut None, &mut None),
                        )
                        .expect("resident delivery succeeds");
                    assert!(matches!(
                        transition,
                        crate::input::ResidentCommandTransition::Delivered { .. }
                    ));
                }
            }
            let allocations_after =
                tex_state::measurement::hot_core_thread_allocation_measurement(owner);
            let commands_after = crate::command::command_ownership_counters();
            let timeline_after = state.profile_timeline_counters();
            Evidence {
                branches: state.profile_resident_input_branch_counters(),
                paths: state.profile_raw_delivery_path_counters(),
                transitions: state.profile_resident_delivery_transition_counters(),
                allocation_calls: allocations_after.calls - allocations_before.calls,
                requested_bytes: allocations_after.requested_bytes
                    - allocations_before.requested_bytes,
                whole_row_copies: timeline_after.full_frame_history_clones
                    - timeline_before.full_frame_history_clones,
                whole_input_frame_copies: state.profile_resident_delivery_transition_counters().2,
                whole_command_copies: commands_after
                    .clones
                    .saturating_sub(commands_before.clones)
                    .saturating_add(
                        commands_after
                            .backup_copies
                            .saturating_sub(commands_before.backup_copies),
                    ),
                final_command_writes: commands_after.resolved_writes
                    - commands_before.resolved_writes,
            }
        })
    }

    for operations in [1_usize, 4_096] {
        let operations = u64::try_from(operations).expect("operation count fits u64");
        for (branch, expected_branches, expected_paths) in [
            (
                Branch::Source,
                (operations, operations, 0, 0),
                (operations, 0, 0, 0),
            ),
            (
                Branch::StoredToken,
                (operations, 0, operations, 0),
                (0, operations, 0, 0),
            ),
            (
                Branch::MacroArgument,
                (operations, 0, 0, operations),
                (0, 0, operations, 0),
            ),
        ] {
            let evidence = run(
                branch,
                usize::try_from(operations).expect("operation count fits usize"),
            );
            assert_eq!(evidence.branches, expected_branches);
            assert_eq!(evidence.paths, expected_paths);
            assert_eq!(evidence.transitions, (operations, 0, 0));
            assert_eq!(evidence.allocation_calls, 0);
            assert_eq!(evidence.requested_bytes, 0);
            assert_eq!(evidence.whole_row_copies, 0);
            assert_eq!(evidence.whole_input_frame_copies, 0);
            assert_eq!(evidence.whole_command_copies, 0);
            assert_eq!(evidence.final_command_writes, operations);
        }
    }
}
