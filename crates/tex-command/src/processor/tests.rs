use tex_state::meaning::Meaning;
use tex_state::token::{Catcode, OriginId, Token, TokenWord, TracedTokenWord};

#[cfg(feature = "profiling")]
use crate::input::{InputLevel, MacroArgumentCursor};
use crate::input::{
    PackedTokenSpanHandle, ReplayTrace, RetirementBehavior, StoredReplayReason, TokenBehavior,
};
use crate::{
    AlignmentIdentity, CommandDeliveryBoundary, CommandHostCapabilities, CommandObservation,
    CommandObserver, CommandState, DeliveryStatus, InputReason, InputTransition,
};

#[derive(Default)]
struct RecordingObserver {
    observations: Vec<CommandObservation>,
}

impl CommandObserver for RecordingObserver {
    fn committed(&mut self, observation: CommandObservation) {
        self.observations.push(observation);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RetirementHandoffEvidence {
    top_source_checks: u64,
    slot_initializations: u64,
    resolved_writes: u64,
    command_clones: u64,
    backup_copies: u64,
    expansion_moves_in: u64,
    expansion_moves_out: u64,
}

fn retirement_handoff_evidence(empty_levels: usize) -> RetirementHandoffEvidence {
    crate::test_harness::with_universe(|universe| {
        let token = Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        };
        let mut command = CommandState::default();
        crate::test_harness::push(&mut command, [token]);
        for _ in 0..empty_levels {
            command.push_token_level(
                PackedTokenSpanHandle::transient([]),
                TokenBehavior::Ordinary,
                RetirementBehavior::Pop,
                ReplayTrace::Inserted,
            );
        }
        let before_checks = crate::state::retirement_top_source_checks();
        let before_ownership = crate::command::command_ownership_counters();
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );
        let mut destination = None;

        #[cfg(feature = "profiling")]
        let owner = tex_state::measurement::HotCoreAllocationOwner::DeliveryAndScan;
        #[cfg(feature = "profiling")]
        let before_allocations =
            tex_state::measurement::hot_core_thread_allocation_measurement(owner);
        let status = {
            #[cfg(feature = "profiling")]
            let _scope = tex_state::measurement::hot_core_allocation_scope(owner);
            processor
                .get_next_into(&mut destination)
                .expect("delivery through exhausted levels")
        };
        #[cfg(feature = "profiling")]
        let after_allocations =
            tex_state::measurement::hot_core_thread_allocation_measurement(owner);

        assert_eq!(status, DeliveryStatus::Command);
        assert_eq!(
            destination
                .as_ref()
                .expect("final caller slot")
                .spelling()
                .semantic_token(),
            token
        );
        #[cfg(feature = "profiling")]
        {
            assert_eq!(after_allocations.calls - before_allocations.calls, 0);
            assert_eq!(
                after_allocations.requested_bytes - before_allocations.requested_bytes,
                0
            );
        }

        let after_ownership = crate::command::command_ownership_counters();
        RetirementHandoffEvidence {
            top_source_checks: crate::state::retirement_top_source_checks() - before_checks,
            slot_initializations: after_ownership.slot_initializations
                - before_ownership.slot_initializations,
            resolved_writes: after_ownership.resolved_writes - before_ownership.resolved_writes,
            command_clones: after_ownership.clones - before_ownership.clones,
            backup_copies: after_ownership.backup_copies - before_ownership.backup_copies,
            expansion_moves_in: after_ownership.expansion_moves_in
                - before_ownership.expansion_moves_in,
            expansion_moves_out: after_ownership.expansion_moves_out
                - before_ownership.expansion_moves_out,
        }
    })
}

#[test]
fn one_and_4096_resident_retirements_skip_source_checks_and_reuse_one_command_slot() {
    let one = retirement_handoff_evidence(1);
    let many = retirement_handoff_evidence(4_096);

    assert_eq!(one.top_source_checks, 0);
    assert_eq!(many.top_source_checks, 0);
    for evidence in [one, many] {
        assert_eq!(evidence.slot_initializations, 1);
        assert_eq!(evidence.resolved_writes, 1);
        assert_eq!(evidence.command_clones, 0);
        assert_eq!(evidence.backup_copies, 0);
        assert_eq!(evidence.expansion_moves_in, 0);
        assert_eq!(evidence.expansion_moves_out, 0);
    }
}

#[test]
fn replay_completion_is_published_by_its_direct_retirement() {
    crate::test_harness::with_universe(|universe| {
        let token = Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        };
        let tokens = universe
            .allocate_token_list(&[TokenWord::pack(token)])
            .expect("replay token list");
        let mut command = CommandState::default();
        let episode = {
            let context = universe.command_context().expect("command context");
            command.push_discretionary_episode(&context, tokens)
        };
        command.profile_reset_raw_delivery_path_counters();
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );
        let mut destination = None;

        assert_eq!(
            processor
                .get_x_token_with_replay_completion_into(&mut destination)
                .expect("replay token"),
            DeliveryStatus::Command
        );
        assert_eq!(
            destination
                .take()
                .expect("replay command")
                .spelling()
                .semantic_token(),
            token
        );
        assert_eq!(
            processor
                .get_x_token_with_replay_completion_into(&mut destination)
                .expect("replay completion"),
            DeliveryStatus::ReplayCompleted(episode)
        );
        assert!(destination.is_none());
        assert_eq!(
            processor.command.profile_replay_completion_counters(),
            (1, 1, 0)
        );
    });
}

#[test]
#[cfg(feature = "profiling")]
fn ordinary_4096_resident_deliveries_perform_zero_replay_completion_checks() {
    crate::test_harness::with_universe(|universe| {
        let token = Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        };
        let mut command = CommandState::default();
        crate::test_harness::push(&mut command, std::iter::repeat_n(token, 4_096));
        command.profile_reset_raw_delivery_path_counters();
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );
        let mut destination = None;
        let owner = tex_state::measurement::HotCoreAllocationOwner::DeliveryAndScan;
        let before = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
        {
            let _scope = tex_state::measurement::hot_core_allocation_scope(owner);
            for _ in 0..4_096 {
                assert_eq!(
                    processor
                        .get_next_into(&mut destination)
                        .expect("ordinary resident delivery"),
                    DeliveryStatus::Command
                );
                assert_eq!(
                    destination
                        .take()
                        .expect("ordinary command")
                        .spelling()
                        .semantic_token(),
                    token
                );
            }
        }
        let after = tex_state::measurement::hot_core_thread_allocation_measurement(owner);

        assert_eq!(
            processor.command.profile_replay_completion_counters(),
            (0, 0, 0)
        );
        assert_eq!(after.calls - before.calls, 0);
        assert_eq!(after.requested_bytes - before.requested_bytes, 0);
    });
}

#[test]
fn stack_conservation_remains_an_explicit_counted_retirement_branch() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        for _ in 0..2 {
            command.push_token_level(
                PackedTokenSpanHandle::transient([]),
                TokenBehavior::Ordinary,
                RetirementBehavior::Pop,
                ReplayTrace::Inserted,
            );
        }
        command.profile_reset_raw_delivery_path_counters();
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );

        processor
            .conserve_input_stack()
            .expect("explicit conservation drains exhausted levels");

        assert_eq!(processor.command.input_level_count(), 0);
        assert_eq!(
            processor.command.profile_resident_retirement_counters(),
            (0, 0, 0, 0, 0, 0, 2)
        );
    });
}

#[test]
fn in_place_retirement_preserves_semantic_transition_order() {
    crate::test_harness::with_universe(|universe| {
        let token = Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        };
        let mut command = CommandState::default();
        crate::test_harness::push(&mut command, [token]);
        command.push_token_level(
            PackedTokenSpanHandle::transient([]),
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::Stored(StoredReplayReason::EveryPar),
        );
        command.push_token_level(
            PackedTokenSpanHandle::transient([]),
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::BackedUp,
        );
        command.push_token_level(
            PackedTokenSpanHandle::transient([]),
            TokenBehavior::Recovery,
            RetirementBehavior::Pop,
            ReplayTrace::Inserted,
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut observer = RecordingObserver::default();
        let mut context = universe.command_context().expect("command context");
        let mut destination = None;
        {
            let mut processor = crate::test_harness::processor(
                &mut command,
                &mut context,
                &mut capabilities,
                &mut fuel,
                &mut diagnostic_effects,
            )
            .with_observer(&mut observer);
            assert_eq!(
                processor
                    .get_next_into(&mut destination)
                    .expect("delivery after retirements"),
                DeliveryStatus::Command
            );
        }

        let reasons = observer
            .observations
            .iter()
            .filter_map(|observation| match observation {
                CommandObservation::Input(record)
                    if record.transition == InputTransition::Retire =>
                {
                    Some(record.reason)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            reasons,
            [
                InputReason::Recovery,
                InputReason::Backup,
                InputReason::EveryPar
            ]
        );
        assert_eq!(
            destination
                .expect("delivered command")
                .spelling()
                .semantic_token(),
            token
        );
    });
}

#[test]
fn awaiting_v_template_retires_in_resident_delivery_before_parent_token() {
    crate::test_harness::with_universe(|universe| {
        let token = Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        };
        let mut command = CommandState::default();
        crate::test_harness::push(&mut command, [token]);
        command.push_token_level(
            PackedTokenSpanHandle::transient([]),
            TokenBehavior::VTemplate,
            RetirementBehavior::AwaitingVTemplateRetirement,
            ReplayTrace::VTemplate,
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut observer = RecordingObserver::default();
        let mut context = universe.command_context().expect("command context");
        let mut destination = None;
        {
            let mut processor = crate::test_harness::processor(
                &mut command,
                &mut context,
                &mut capabilities,
                &mut fuel,
                &mut diagnostic_effects,
            )
            .with_observer(&mut observer);
            assert_eq!(
                processor
                    .get_next_into(&mut destination)
                    .expect("delivery after awaiting v-template"),
                DeliveryStatus::Command
            );
        }

        let retirement_reasons = observer
            .observations
            .iter()
            .filter_map(|observation| match observation {
                CommandObservation::Input(record)
                    if record.transition == InputTransition::Retire =>
                {
                    Some(record.reason)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(retirement_reasons, [InputReason::AlignmentVTemplate]);
        assert_eq!(
            destination
                .expect("parent command delivered after v-template retirement")
                .spelling()
                .semantic_token(),
            token
        );
    });
}

#[test]
fn processor_episode_borrows_generation_and_delivers_one_current_command() {
    crate::test_harness::with_universe(|universe| {
        let token = Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        };
        let mut command = CommandState::default();
        crate::test_harness::push(&mut command, [token]);
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );

        let delivered = processor
            .get_x_token()
            .expect("expanded delivery")
            .expect("one token");
        assert_eq!(delivered.spelling().semantic_token(), token);
        assert_eq!(
            delivered.meaning(),
            Meaning::CharToken {
                ch: 'x',
                cat: Catcode::Letter,
            }
        );
        assert!(processor.get_x_token().expect("end").is_none());
    });
}

#[test]
fn destination_raw_delivery_mints_fresh_stamps_and_reverses_backup_once() {
    crate::test_harness::with_universe(|universe| {
        let brace = Token::Char {
            ch: '{',
            cat: Catcode::BeginGroup,
        };
        let mut command = CommandState::default();
        crate::test_harness::push(&mut command, [brace]);
        let initial_align_state = command.alignment.align_state;
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );

        let mut first = None;
        assert_eq!(
            processor.get_next_into(&mut first).expect("first delivery"),
            DeliveryStatus::Command
        );
        let first = first.expect("first command");
        let first_stamp = first.delivery_stamp();
        let stale_copy = first.copy_for_backup();
        assert_eq!(
            processor.command.alignment.align_state,
            initial_align_state + 1
        );

        processor.back_input(first).expect("first backup");
        assert_eq!(processor.command.alignment.align_state, initial_align_state);
        assert_eq!(
            processor.back_input(stale_copy),
            Err(crate::CommandError::StaleDelivery)
        );
        assert_eq!(processor.command.alignment.align_state, initial_align_state);

        let mut replay = None;
        assert_eq!(
            processor
                .get_next_into(&mut replay)
                .expect("backup redelivery"),
            DeliveryStatus::Command
        );
        let replay = replay.expect("replayed command");
        assert_eq!(replay.spelling().semantic_token(), brace);
        assert_ne!(replay.delivery_stamp(), first_stamp);
        assert_eq!(
            processor.command.alignment.align_state,
            initial_align_state + 1
        );
    });
}

#[test]
fn saved_futurelet_and_post_next_deliveries_keep_exact_backup_freshness() {
    crate::test_harness::with_universe(|universe| {
        let first_token = Token::Char {
            ch: 'a',
            cat: Catcode::Letter,
        };
        let second_token = Token::Char {
            ch: 'b',
            cat: Catcode::Letter,
        };
        let mut command = CommandState::default();
        crate::test_harness::push(&mut command, [first_token, second_token]);
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );

        let first = processor
            .get_token()
            .expect("first raw delivery")
            .expect("first command");
        let saved_first = first.copy_for_backup();
        let second = processor
            .get_token()
            .expect("second raw delivery")
            .expect("second command");
        assert_eq!(
            processor.back_input(first),
            Err(crate::CommandError::StaleDelivery),
            "a later raw delivery invalidates ordinary backup"
        );

        processor
            .back_input(second)
            .expect("fresh second delivery backs up");
        processor
            .back_input_saved(saved_first)
            .expect("TeX82 §326 saved first token backs up without freshness");
        assert_eq!(
            processor
                .get_token()
                .expect("saved first replay")
                .expect("saved first command")
                .spelling()
                .semantic_token(),
            first_token
        );
        assert_eq!(
            processor
                .get_token()
                .expect("fresh second replay")
                .expect("fresh second command")
                .spelling()
                .semantic_token(),
            second_token
        );
    });
}

#[test]
fn cursor_resume_rejects_delivery_until_retained_command_is_readmitted() {
    crate::test_harness::with_universe(|universe| {
        let token = Token::Char {
            ch: 'r',
            cat: Catcode::Letter,
        };
        let mut command = CommandState::default();
        crate::test_harness::push(&mut command, [token]);
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");

        let (delivered, cursor) = {
            let mut processor = crate::test_harness::processor(
                &mut command,
                &mut context,
                &mut capabilities,
                &mut fuel,
                &mut diagnostic_effects,
            );
            let delivered = processor
                .get_token()
                .expect("raw delivery")
                .expect("resident command");
            let cursor = processor.delivery_cursor();
            (delivered, cursor)
        };
        let stale_copy = delivered.copy_for_backup();

        {
            let mut processor = crate::test_harness::processor(
                &mut command,
                &mut context,
                &mut capabilities,
                &mut fuel,
                &mut diagnostic_effects,
            );
            processor.resume_delivery_cursor(cursor);
            assert_eq!(
                processor.back_input(stale_copy),
                Err(crate::CommandError::StaleDelivery),
                "cursor-only resume admits no current command"
            );
        }

        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );
        processor.resume_delivery_cursor(cursor);
        processor.resume_current_command(&delivered);
        processor
            .back_input(delivered)
            .expect("retained current command is explicitly readmitted");
        assert!(processor.immediate_delivery_stamp.is_none());
    });
}

#[test]
fn resident_stopper_stamp_retires_exact_level_and_invalidates_freshness() {
    crate::test_harness::with_universe(|universe| {
        crate::install_tex82_unexpandable_primitives(universe);
        let endwrite = universe.primitive_token("endwrite").expect("write stopper");
        let mut command = CommandState::default();
        crate::test_harness::push(&mut command, [endwrite]);
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );

        let stopper = processor
            .get_token()
            .expect("stopper delivery")
            .expect("resident stopper");
        let stale = stopper.copy_for_backup();
        processor
            .retire_delivery_level(stopper.delivery_stamp())
            .expect("resident stamp retires its exact exhausted level");
        assert!(processor.immediate_delivery_stamp.is_none());
        assert_eq!(
            processor.back_input(stale),
            Err(crate::CommandError::StaleDelivery)
        );
    });
}

#[cfg(feature = "profiling")]
#[test]
fn one_and_4096_deliveries_have_one_compact_freshness_owner_and_zero_allocations() {
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct Evidence {
        freshness_coordinate_fields: usize,
        command_coordinate_writes: u64,
        resolved_command_writes: u64,
        work: crate::CommandWorkCounters,
        allocation_calls: u64,
        requested_bytes: u64,
    }

    fn census(deliveries: usize) -> Evidence {
        crate::test_harness::with_universe(|universe| {
            let token = Token::Char {
                ch: 'x',
                cat: Catcode::Letter,
            };
            let mut command = CommandState::default();
            crate::test_harness::push(&mut command, std::iter::repeat_n(token, deliveries));
            let mut capabilities = CommandHostCapabilities::default();
            let mut fuel = crate::CommandFuelLedger::default();
            let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
            let mut context = universe.command_context().expect("command context");
            let commands_before = crate::command::command_ownership_counters();
            let owner = tex_state::measurement::HotCoreAllocationOwner::DeliveryAndScan;
            let allocations_before =
                tex_state::measurement::hot_core_thread_allocation_measurement(owner);
            {
                let mut processor = crate::test_harness::processor(
                    &mut command,
                    &mut context,
                    &mut capabilities,
                    &mut fuel,
                    &mut diagnostic_effects,
                );
                let mut destination = None;
                {
                    let _scope = tex_state::measurement::hot_core_allocation_scope(owner);
                    for _ in 0..deliveries {
                        assert_eq!(
                            processor
                                .get_next_into(&mut destination)
                                .expect("resident raw delivery"),
                            DeliveryStatus::Command
                        );
                        assert_eq!(
                            destination
                                .take()
                                .expect("resident command")
                                .spelling()
                                .semantic_token(),
                            token
                        );
                    }
                }
            }
            let allocations_after =
                tex_state::measurement::hot_core_thread_allocation_measurement(owner);
            let commands_after = crate::command::command_ownership_counters();
            let freshness_coordinate_fields = include_str!("mod.rs")
                .matches("immediate_delivery_stamp: Option<crate::DeliveryStamp>")
                .count();
            Evidence {
                freshness_coordinate_fields,
                command_coordinate_writes: commands_after.delivery_stamp_writes
                    - commands_before.delivery_stamp_writes,
                resolved_command_writes: commands_after.resolved_writes
                    - commands_before.resolved_writes,
                work: fuel.work(),
                allocation_calls: allocations_after.calls - allocations_before.calls,
                requested_bytes: allocations_after.requested_bytes
                    - allocations_before.requested_bytes,
            }
        })
    }

    for deliveries in [1_usize, 4_096] {
        let count = deliveries as u64;
        assert_eq!(
            census(deliveries),
            Evidence {
                freshness_coordinate_fields: 1,
                command_coordinate_writes: count,
                resolved_command_writes: count,
                work: crate::CommandWorkCounters {
                    fuel_charges: count,
                    token_frame_steps: count,
                    expanded_deliveries: 0,
                    meaning_lookups: 0,
                    scanner_tokens: 0,
                    write_expansions: 0,
                    raw_delivery_kinds: [0, count, 0, 0],
                },
                allocation_calls: 0,
                requested_bytes: 0,
            }
        );
    }
}

#[test]
fn scalar_and_surface_alignment_handoffs_consume_coordinate_freshness() {
    crate::test_harness::with_universe(|universe| {
        let tab = Token::Char {
            ch: '&',
            cat: Catcode::AlignmentTab,
        };
        let empty_template = universe
            .command_context()
            .expect("template context")
            .allocate_token_list(&[])
            .expect("empty v-template");
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();

        let mut scalar_command = CommandState::default();
        let scalar_alignment = AlignmentIdentity::new(31);
        scalar_command.begin_alignment(scalar_alignment);
        scalar_command
            .begin_prepared_alignment_cell(
                scalar_alignment,
                crate::PreparedAlignmentCellTemplates {
                    u_template: None,
                    v_template: empty_template.clone(),
                },
            )
            .expect("scalar active cell");
        scalar_command
            .install_alignment_omit_cell_template(scalar_alignment)
            .expect("scalar cell base");
        crate::test_harness::push(&mut scalar_command, [tab]);
        {
            let mut context = universe.command_context().expect("scalar context");
            let mut processor = crate::test_harness::processor(
                &mut scalar_command,
                &mut context,
                &mut capabilities,
                &mut fuel,
                &mut diagnostic_effects,
            );
            let delimiter = match processor
                .get_next_with_replay_completion()
                .expect("scalar delimiter delivery")
                .expect("scalar delimiter command")
            {
                crate::CommandReplayDelivery::Command(delimiter) => delimiter,
                crate::CommandReplayDelivery::Completed(_) => {
                    panic!("scalar control expects a command")
                }
            };
            assert!(matches!(
                delimiter.alignment_adjustment(),
                super::AlignmentDeliveryAdjustment::Delimiter(_)
            ));
            processor
                .begin_scalar_alignment_v_template(&delimiter)
                .expect("fresh scalar delimiter handoff");
            assert!(processor.immediate_delivery_stamp.is_none());
            assert_eq!(
                processor.begin_scalar_alignment_v_template(&delimiter),
                Err(crate::CommandError::StaleDelivery)
            );
        }

        let mut surface_command = CommandState::default();
        let surface_alignment = AlignmentIdentity::new(32);
        surface_command.begin_alignment(surface_alignment);
        surface_command
            .begin_prepared_alignment_cell(
                surface_alignment,
                crate::PreparedAlignmentCellTemplates {
                    u_template: None,
                    v_template: empty_template,
                },
            )
            .expect("surface active cell");
        surface_command
            .install_alignment_omit_cell_template(surface_alignment)
            .expect("surface cell base");
        crate::test_harness::push(&mut surface_command, [tab]);
        let mut context = universe.command_context().expect("surface context");
        let mut processor = crate::test_harness::processor(
            &mut surface_command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );
        let event = match processor
            .get_x_alignment_delivery(false)
            .expect("surface delimiter delivery")
            .expect("surface delimiter event")
        {
            crate::AlignmentDelivery::Event(event) => event,
            crate::AlignmentDelivery::Command(_) | crate::AlignmentDelivery::Completed(_) => {
                panic!("surface control expects an alignment event")
            }
        };
        let stale_event = match &event {
            crate::AlignmentDeliveryEvent::EndTemplate(delimiter) => {
                crate::AlignmentDeliveryEvent::EndTemplate(delimiter.copy_for_backup())
            }
            crate::AlignmentDeliveryEvent::ClosingBrace(_) => {
                panic!("surface control expects end-template")
            }
        };
        processor
            .begin_alignment_v_template(surface_alignment, event)
            .expect("fresh surface delimiter handoff");
        assert!(processor.immediate_delivery_stamp.is_none());
        assert_eq!(
            processor.begin_alignment_v_template(surface_alignment, stale_event),
            Err(crate::CommandError::StaleDelivery)
        );
    });
}

#[cfg(feature = "profiling")]
#[test]
fn alignment_journal_attempts_follow_literal_braces_and_skip_delimiters() {
    crate::test_harness::with_universe(|universe| {
        let ordinary = Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        };
        let begin = Token::Char {
            ch: '{',
            cat: Catcode::BeginGroup,
        };
        let end = Token::Char {
            ch: '}',
            cat: Catcode::EndGroup,
        };
        let tab = Token::Char {
            ch: '&',
            cat: Catcode::AlignmentTab,
        };
        let mut command = CommandState::default();
        crate::test_harness::push(&mut command, [ordinary, begin, ordinary, end, tab]);
        let initial_align_state = command.alignment.align_state;
        let snapshot = command.snapshot(universe).expect("delivery checkpoint");
        let before = command.profile_timeline_counters();
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        {
            let mut context = universe.command_context().expect("command context");
            let mut processor = crate::test_harness::processor(
                &mut command,
                &mut context,
                &mut capabilities,
                &mut fuel,
                &mut diagnostic_effects,
            );
            for expected in [ordinary, begin, ordinary, end, tab] {
                let delivered = processor
                    .get_next()
                    .expect("raw delivery")
                    .expect("measured token");
                assert_eq!(delivered.spelling().semantic_token(), expected);
            }
        }
        let after = command.profile_timeline_counters();
        assert_eq!(
            after.alignment_delivery_journal_attempts - before.alignment_delivery_journal_attempts,
            2
        );
        assert_eq!(after.records - before.records, 1);
        assert_eq!(command.alignment.align_state, initial_align_state);

        command
            .rollback(&snapshot, universe)
            .expect("brace journal restores");
        assert_eq!(command.alignment.align_state, initial_align_state);

        let empty_template = universe
            .command_context()
            .expect("command context")
            .allocate_token_list(&[])
            .expect("empty v-template");
        let alignment = AlignmentIdentity::new(9);
        command.begin_alignment(alignment);
        command
            .begin_prepared_alignment_cell(
                alignment,
                crate::PreparedAlignmentCellTemplates {
                    u_template: None,
                    v_template: empty_template,
                },
            )
            .expect("active cell");
        command
            .install_alignment_omit_cell_template(alignment)
            .expect("omit cell template");
        let before_delimiter = command.profile_timeline_counters();
        {
            let context = universe.command_context().expect("command context");
            let mut delivered = crate::CurrentCommand::resolve(
                TracedTokenWord::pack(tab, OriginId::UNKNOWN),
                crate::command::DeliveryStamp::new(1, 0),
                None,
                false,
                None,
                &context,
            );
            command.classify_alignment_delivery(&mut delivered, Some(Catcode::AlignmentTab));
            assert!(matches!(
                delivered.alignment_adjustment(),
                super::AlignmentDeliveryAdjustment::Delimiter(_)
            ));
            assert_eq!(
                command.alignment.align_state,
                super::alignment::CELL_ALIGN_STATE
            );
        }
        let after_delimiter = command.profile_timeline_counters();
        assert_eq!(
            after_delimiter.alignment_delivery_journal_attempts,
            before_delimiter.alignment_delivery_journal_attempts
        );
        assert_eq!(after_delimiter.records, before_delimiter.records);

        let delimiter_snapshot = command.snapshot(universe).expect("active-cell checkpoint");
        crate::test_harness::push(&mut command, [tab]);
        let before_interception = command.profile_timeline_counters();
        {
            let mut context = universe.command_context().expect("command context");
            let mut processor = crate::test_harness::processor(
                &mut command,
                &mut context,
                &mut capabilities,
                &mut fuel,
                &mut diagnostic_effects,
            );
            let delivered = processor
                .get_next()
                .expect("intercepted delimiter delivery")
                .expect("retained end-template command");
            assert_ne!(delivered.spelling().semantic_token(), tab);
            assert_eq!(
                processor.command.alignment.align_state,
                super::alignment::TEMPLATE_ALIGN_STATE
            );
        }
        let after_interception = command.profile_timeline_counters();
        assert_eq!(
            after_interception.alignment_delivery_journal_attempts,
            before_interception.alignment_delivery_journal_attempts
        );
        command
            .rollback(&delimiter_snapshot, universe)
            .expect("alignment-owned delimiter lifecycle rolls back");
        assert_eq!(
            command.alignment.align_state,
            super::alignment::CELL_ALIGN_STATE
        );
    });
}

#[test]
fn failed_raw_delivery_clears_its_partially_written_final_slot() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        // A parameter reference without an active macro frame is malformed.
        // Raw delivery writes it before replay validation discovers that fact.
        crate::test_harness::push(&mut command, [Token::Param(1)]);
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );
        let mut destination = None;

        assert!(matches!(
            processor.get_next_into(&mut destination),
            Err(crate::CommandError::InputInvariant(_))
        ));
        assert!(destination.is_none());
        assert!(processor.immediate_delivery_stamp.is_none());
    });
}

#[test]
fn ordinary_raw_delivery_bypasses_out_parameter_interception() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        let source = command
            .register_source(crate::SourceRegistration::new(
                crate::RegisteredSourceKind::Generated,
                &b"ab"[..],
            ))
            .expect("source registration");
        command
            .open_registered_source(source)
            .expect("source opening");
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        {
            let mut processor = crate::test_harness::processor(
                &mut command,
                &mut context,
                &mut capabilities,
                &mut fuel,
                &mut diagnostic_effects,
            );
            assert_eq!(
                processor
                    .get_next()
                    .expect("first source delivery")
                    .expect("first source token")
                    .spelling()
                    .semantic_token(),
                Token::Char {
                    ch: 'a',
                    cat: Catcode::Letter,
                }
            );
            processor.command.profile_reset_raw_delivery_path_counters();
            processor
                .command
                .profile_reset_input_cursor_mutation_counters();
            processor
                .command
                .profile_reset_input_source_context_counters();
            assert_eq!(
                processor
                    .get_next()
                    .expect("resident source delivery")
                    .expect("resident source token")
                    .spelling()
                    .semantic_token(),
                Token::Char {
                    ch: 'b',
                    cat: Catcode::Letter,
                }
            );
            assert_eq!(
                processor.command.profile_raw_delivery_path_counters(),
                (1, 0, 0, 0)
            );
            assert_eq!(
                processor.command.profile_input_cursor_mutation_counters(),
                (1, 0, 0)
            );
            assert_eq!(
                processor.command.profile_input_source_context_counters(),
                (0, 0, 0, 1)
            );
        }

        let ordinary = Token::Char {
            ch: 't',
            cat: Catcode::Letter,
        };
        let mut command = CommandState::default();
        crate::test_harness::push(&mut command, [ordinary, Token::Param(1)]);
        command.profile_reset_raw_delivery_path_counters();
        command.profile_reset_input_cursor_mutation_counters();
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );
        assert_eq!(
            processor
                .get_next()
                .expect("stored delivery")
                .expect("stored token")
                .spelling()
                .semantic_token(),
            ordinary
        );
        assert!(processor.get_next().is_err());
        assert_eq!(
            processor.command.profile_raw_delivery_path_counters(),
            (0, 1, 0, 1)
        );
        assert_eq!(
            processor.command.profile_input_cursor_mutation_counters(),
            (2, 0, 0)
        );
    });
}

#[test]
#[cfg(feature = "profiling")]
fn mixed_resident_delivery_has_one_transition_and_no_result_redispatch() {
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct Evidence {
        path: (u64, u64, u64, u64),
        branches: (u64, u64, u64, u64),
        transitions: (u64, u64, u64),
        macro_kernel: (u64, u64, u64, u64, u64, u64, u64),
        delivery_loop: (u64, u64, u64),
        retirements: (u64, u64, u64, u64, u64, u64, u64),
        allocation_calls: u64,
        requested_bytes: u64,
        whole_frame_copies: u64,
        whole_command_copies: u64,
        resolved_writes: u64,
    }

    fn run(operations: usize, expanded: bool) -> Evidence {
        crate::test_harness::with_universe(|universe| {
            let token = Token::Char {
                ch: 'x',
                cat: Catcode::Letter,
            };
            let words = vec![tex_state::token::TokenWord::pack(token); operations];
            let traced = vec![TracedTokenWord::pack(token, OriginId::UNKNOWN); operations];
            let durable = universe
                .command_context()
                .expect("mixed delivery context")
                .allocate_token_list(&words)
                .expect("mixed delivery durable list");
            let definition = universe
                .allocate_definition(&[], &words)
                .expect("mixed delivery macro replacement");

            let mut command = CommandState::default();
            let source = command
                .register_source(
                    crate::SourceRegistration::new(
                        crate::RegisteredSourceKind::Generated,
                        std::sync::Arc::<[u8]>::from(vec![b'x'; operations]),
                    )
                    .with_role(crate::SourceRole::UserDocumentInclude),
                )
                .expect("mixed delivery source registration");
            command
                .open_registered_source(source)
                .expect("mixed delivery source opening");
            command.profile_prepare_source_line(1);
            {
                let context = universe.command_context().expect("mixed durable context");
                command.push_token_level(
                    PackedTokenSpanHandle::durable(context.token_list(durable)),
                    TokenBehavior::Ordinary,
                    RetirementBehavior::Pop,
                    ReplayTrace::Stored(StoredReplayReason::EveryPar),
                );
            }
            command.push_token_level(
                PackedTokenSpanHandle::transient(traced.iter().copied()),
                TokenBehavior::Ordinary,
                RetirementBehavior::Pop,
                ReplayTrace::Inserted,
            );
            let matching = command.scratch.begin_macro_match().expect("macro match");
            let mut writer = command
                .scratch
                .begin_argument_writer(&matching)
                .expect("macro writer");
            for spelling in &traced {
                command
                    .scratch
                    .append_argument_token(
                        &mut writer,
                        crate::token_collector::ClassifiedToken::from_word(*spelling, None),
                        true,
                    )
                    .expect("macro argument word");
            }
            command
                .scratch
                .publish_argument(writer)
                .expect("macro argument range");
            let argument_set = command
                .scratch
                .commit_macro_match(matching)
                .expect("macro frame");
            let macro_name = universe
                .intern("mixeddelivery")
                .expect("mixed delivery macro name")
                .symbol();
            let body = universe
                .command_context()
                .expect("mixed delivery definition context")
                .admit_macro_body(definition)
                .expect("resident mixed delivery definition")
                .2;
            command.push_macro_activation(macro_name, body, Some(argument_set), OriginId::UNKNOWN);
            let range = command
                .scratch
                .argument_range(argument_set, 1)
                .expect("live macro frame")
                .expect("macro argument");
            let identity = command.allocate_input_level_identity();
            let mut frame = crate::input::ResidentSpanCursor::new(identity, operations);
            frame.set_source_context(command.input.levels.current_source_context());
            let origin_run = command
                .scratch
                .admitted_argument_origin_run(range)
                .expect("argument provenance run");
            command.push_input_level(InputLevel::MacroArgument(MacroArgumentCursor {
                range,
                slot: 1,
                origin_run,
                frame,
            }));

            let mut capabilities = CommandHostCapabilities::default();
            let mut fuel = crate::CommandFuelLedger::default();
            let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
            let mut context = universe.command_context().expect("mixed command context");
            let mut destination = None;
            command.profile_reset_raw_delivery_path_counters();
            command.profile_reset_macro_kernel_counters();
            command.profile_reset_delivery_loop_counters();
            let timeline_before = command.profile_timeline_counters();
            let commands_before = crate::command::command_ownership_counters();
            let owner = tex_state::measurement::HotCoreAllocationOwner::DeliveryAndScan;
            let allocations_before =
                tex_state::measurement::hot_core_thread_allocation_measurement(owner);
            {
                let _scope = tex_state::measurement::hot_core_allocation_scope(owner);
                let mut processor = crate::test_harness::processor(
                    &mut command,
                    &mut context,
                    &mut capabilities,
                    &mut fuel,
                    &mut diagnostic_effects,
                );
                for _ in 0..operations.saturating_mul(5) {
                    let delivery = if expanded {
                        processor.get_x_token_into(&mut destination)
                    } else {
                        processor.get_next_into(&mut destination)
                    };
                    assert_eq!(
                        delivery.expect("mixed resident delivery"),
                        DeliveryStatus::Command
                    );
                    let delivered = destination.take().expect("mixed resident command");
                    assert_eq!(delivered.spelling().semantic_token(), token);
                    assert_eq!(
                        delivered.active_source_role(),
                        Some(crate::SourceRole::UserDocumentInclude)
                    );
                }
            }
            let allocations_after =
                tex_state::measurement::hot_core_thread_allocation_measurement(owner);
            {
                let mut processor = crate::test_harness::processor(
                    &mut command,
                    &mut context,
                    &mut capabilities,
                    &mut fuel,
                    &mut diagnostic_effects,
                );
                assert_eq!(
                    processor
                        .get_next_into(&mut destination)
                        .expect("normalized source terminator delivery"),
                    DeliveryStatus::Command
                );
                let terminator = destination.take().expect("normalized source terminator");
                assert_eq!(
                    terminator.active_source_role(),
                    Some(crate::SourceRole::UserDocumentInclude)
                );
                assert_eq!(
                    processor
                        .get_next_into(&mut destination)
                        .expect("mixed source exhaustion"),
                    DeliveryStatus::End
                );
            }
            let commands_after = crate::command::command_ownership_counters();
            let timeline_after = command.profile_timeline_counters();
            Evidence {
                path: command.profile_raw_delivery_path_counters(),
                branches: command.profile_resident_input_branch_counters(),
                transitions: command.profile_resident_delivery_transition_counters(),
                macro_kernel: command.profile_macro_kernel_counters(),
                delivery_loop: command.profile_delivery_loop_counters(),
                retirements: command.profile_resident_retirement_counters(),
                allocation_calls: allocations_after.calls - allocations_before.calls,
                requested_bytes: allocations_after.requested_bytes
                    - allocations_before.requested_bytes,
                whole_frame_copies: timeline_after.full_frame_history_clones
                    - timeline_before.full_frame_history_clones,
                whole_command_copies: commands_after
                    .clones
                    .saturating_sub(commands_before.clones)
                    .saturating_add(
                        commands_after
                            .backup_copies
                            .saturating_sub(commands_before.backup_copies),
                    ),
                resolved_writes: commands_after.resolved_writes - commands_before.resolved_writes,
            }
        })
    }

    let one = run(1, false);
    assert_eq!(
        one,
        Evidence {
            path: (2, 2, 1, 0),
            branches: (11, 3, 4, 2),
            transitions: (11, 0, 0),
            macro_kernel: (1, 1, 0, 1, 1, 1, 1),
            delivery_loop: (6, 2, 0),
            retirements: (3, 0, 0, 0, 0, 1, 0),
            allocation_calls: 0,
            requested_bytes: 0,
            whole_frame_copies: 0,
            whole_command_copies: 0,
            resolved_writes: 6,
        }
    );
    let four_k = run(4_096, false);
    assert_eq!(four_k.path, (4_097, 8_192, 4_096, 0));
    assert_eq!(four_k.branches, (20_486, 4_098, 8_194, 4_097));
    assert_eq!(four_k.transitions, (20_486, 0, 0));
    assert_eq!(four_k.delivery_loop, (20_481, 2, 0));
    assert_eq!(
        four_k.macro_kernel,
        (4_096, 4_096, 0, 4_096, 4_096, 4_096, 4_096)
    );
    assert_eq!(four_k.retirements, (3, 0, 0, 0, 0, 1, 0));
    assert_eq!(four_k.allocation_calls, 0);
    assert_eq!(four_k.requested_bytes, 0);
    assert_eq!(four_k.whole_frame_copies, 0);
    assert_eq!(four_k.whole_command_copies, 0);
    assert_eq!(four_k.resolved_writes, 20_481);

    let expanded_four_k = run(4_096, true);
    assert_eq!(expanded_four_k.delivery_loop, (20_481, 2, 0));
    assert_eq!(expanded_four_k.macro_kernel, four_k.macro_kernel);
    assert_eq!(expanded_four_k.allocation_calls, 0);
    assert_eq!(expanded_four_k.requested_bytes, 0);
    assert_eq!(expanded_four_k.whole_frame_copies, 0);
    assert_eq!(expanded_four_k.whole_command_copies, 0);
}

#[test]
fn raw_observation_follows_alignment_and_borrows_direct_source_provenance() {
    crate::test_harness::with_universe(|universe| {
        universe
            .assign_code(
                tex_state::env::CodeTableKind::Catcode,
                '{',
                i64::from(Catcode::BeginGroup as u8),
                tex_state::env::AssignmentScope::Global,
            )
            .expect("opening-brace catcode");
        let mut command = CommandState::default();
        command.begin_alignment(AlignmentIdentity::new(7));
        let source = command
            .register_source(
                crate::SourceRegistration::new(crate::RegisteredSourceKind::Generated, &b"{"[..])
                    .with_name("raw-order.tex"),
            )
            .expect("source registration");
        command
            .open_registered_source(source)
            .expect("source opening");
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut observer = RecordingObserver::default();
        let (stamp, source_range, source_location) = {
            let mut context = universe.command_context().expect("command context");
            let mut processor = crate::test_harness::processor(
                &mut command,
                &mut context,
                &mut capabilities,
                &mut fuel,
                &mut diagnostic_effects,
            )
            .with_observer(&mut observer);

            let mut destination = None;
            assert_eq!(
                processor
                    .get_next_into(&mut destination)
                    .expect("raw delivery"),
                DeliveryStatus::Command
            );
            let delivered = destination.as_ref().expect("delivered command");
            let provenance = processor
                .source_provenance(delivered)
                .expect("source provenance");
            (
                delivered.delivery_stamp(),
                provenance.range(),
                provenance.location(),
            )
        };

        let alignment_index = observer
            .observations
            .iter()
            .position(|observation| {
                matches!(
                    observation,
                    CommandObservation::Alignment(record)
                        if record.transition == "begin_group"
                )
            })
            .expect("alignment observation");
        let (raw_index, raw) = observer
            .observations
            .iter()
            .enumerate()
            .find_map(|(index, observation)| match observation {
                CommandObservation::Command(record)
                    if record.boundary == CommandDeliveryBoundary::Raw =>
                {
                    Some((index, record))
                }
                _ => None,
            })
            .expect("raw observation");
        assert!(alignment_index < raw_index);
        assert_eq!(raw.provenance.input_level, stamp.input_level());
        assert_eq!(raw.provenance.position, stamp.position());
        assert_eq!(raw.provenance.delivery_sequence, 0);
        assert_eq!(raw.provenance.source_range, Some(source_range));
        assert_eq!(raw.provenance.source_location, Some(source_location));
    });
}

#[test]
fn direct_source_command_captures_its_physical_line_before_retirement() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        let source = command
            .register_source(
                crate::SourceRegistration::new(crate::RegisteredSourceKind::Generated, &b"\nX"[..])
                    .with_name("two-lines.tex"),
            )
            .expect("source registration");
        command
            .open_registered_source(source)
            .expect("source opening");
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );

        loop {
            let delivered = processor
                .get_next()
                .expect("raw delivery")
                .expect("second-line character");
            if delivered.spelling().semantic_token()
                == (Token::Char {
                    ch: 'X',
                    cat: Catcode::Letter,
                })
            {
                assert_eq!(delivered.direct_source_line_number(), Some(2));
                break;
            }
        }
    });
}

#[test]
fn empty_direct_source_registers_provenance_before_observed_retirement() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        let source = command
            .register_source(
                crate::SourceRegistration::new(crate::RegisteredSourceKind::Generated, &b""[..])
                    .with_name("empty-root.tex"),
            )
            .expect("source registration");
        command
            .open_registered_source(source)
            .expect("source opening");
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut observer = RecordingObserver::default();
        let mut context = universe.command_context().expect("command context");
        {
            let mut processor = crate::test_harness::processor(
                &mut command,
                &mut context,
                &mut capabilities,
                &mut fuel,
                &mut diagnostic_effects,
            )
            .with_observer(&mut observer);
            assert!(
                processor
                    .get_next()
                    .expect("empty source retirement")
                    .is_none()
            );
        }

        let origin = context.source_range_origin(source, 0, 0);
        assert_ne!(origin, OriginId::UNKNOWN);
        let recipe = context
            .detach_artifact_source_recipe(origin)
            .expect("retired empty source remains registered for provenance");
        assert_eq!(recipe.logical_path, "empty-root.tex");
        assert_eq!((recipe.start, recipe.end), (0, 0));
        assert!(observer.observations.iter().any(|observation| {
            matches!(
                observation,
                CommandObservation::Input(record)
                    if record.transition == InputTransition::Retire
                        && record.reason == InputReason::Source
                        && record.source == Some(source)
            )
        }));
    });
}

#[test]
fn forced_eof_before_production_acquisition_registers_source_before_retirement() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        let source = command
            .register_source(
                crate::SourceRegistration::new(
                    crate::RegisteredSourceKind::Generated,
                    &b"unread"[..],
                )
                .with_name("forced-eof.tex"),
            )
            .expect("source registration");
        command
            .open_registered_source(source)
            .expect("source opening");
        command
            .input
            .levels
            .mutate_top_source(|source, slot| {
                let stored = crate::input::SourceLevelExecutionState::cursor(source, slot);
                let line = slot
                    .cursor
                    .load_next_line(13)
                    .expect("line is acquired outside production delivery");
                line.cursor.byte_cursor = line.retained_end;
                line.cursor.endline_delivered = true;
                (stored, ())
            })
            .expect("source is active");
        assert!(command.end_current_source_after_current_line());
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut observer = RecordingObserver::default();
        let mut context = universe.command_context().expect("command context");
        {
            let mut processor = crate::test_harness::processor(
                &mut command,
                &mut context,
                &mut capabilities,
                &mut fuel,
                &mut diagnostic_effects,
            )
            .with_observer(&mut observer);
            assert!(
                processor
                    .get_next()
                    .expect("forced source retirement")
                    .is_none()
            );
        }

        let origin = context.source_range_origin(source, 0, 6);
        let recipe = context
            .detach_artifact_source_recipe(origin)
            .expect("forced-EOF source is registered before retirement");
        assert_eq!(recipe.logical_path, "forced-eof.tex");
        assert_eq!((recipe.start, recipe.end), (0, 6));
        assert!(observer.observations.iter().any(|observation| {
            matches!(
                observation,
                CommandObservation::Input(record)
                    if record.transition == InputTransition::Retire
                        && record.reason == InputReason::Source
                        && record.source == Some(source)
            )
        }));
    });
}

#[test]
fn failed_source_map_registration_does_not_mark_cursor_registered() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        let source = command
            .register_source(crate::SourceRegistration::new(
                crate::RegisteredSourceKind::Generated,
                &b"x"[..],
            ))
            .expect("source registration");
        command
            .open_registered_source(source)
            .expect("source opening");
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        context
            .register_source(
                source,
                tex_state::source_map::SourceDescriptor::generated(std::sync::Arc::from(
                    &b"conflict"[..],
                )),
            )
            .expect("conflicting source id seed");
        let before = crate::input::source_registration_counters();
        {
            let mut processor = crate::test_harness::processor(
                &mut command,
                &mut context,
                &mut capabilities,
                &mut fuel,
                &mut diagnostic_effects,
            );
            assert_eq!(
                processor
                    .get_next()
                    .expect("delivery tolerates diagnostic registration failure")
                    .expect("source token")
                    .spelling()
                    .semantic_token(),
                Token::Char {
                    ch: 'x',
                    cat: Catcode::Letter,
                }
            );
        }
        let after_acquisition = crate::input::source_registration_counters();
        assert_eq!(after_acquisition.calls - before.calls, 2);
        let (_, source) = command
            .input
            .levels
            .top_source()
            .expect("source remains live after its first token");
        assert!(!source.cursor.backing_registered);

        {
            let mut processor = crate::test_harness::processor(
                &mut command,
                &mut context,
                &mut capabilities,
                &mut fuel,
                &mut diagnostic_effects,
            );
            assert!(
                processor
                    .get_next()
                    .expect("source retirement retries registration")
                    .is_none()
            );
        }
        let after_retirement = crate::input::source_registration_counters();
        assert_eq!(after_retirement.calls - after_acquisition.calls, 1);
    });
}

#[test]
fn warmed_source_token_transition_performs_no_registration_checks_or_calls() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        let source = command
            .register_source(crate::SourceRegistration::new(
                crate::RegisteredSourceKind::Generated,
                &b"ab"[..],
            ))
            .expect("source registration");
        command
            .open_registered_source(source)
            .expect("source opening");
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let before_acquisition = crate::input::source_registration_counters();
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );

        assert_eq!(
            processor
                .get_next()
                .expect("first source delivery")
                .expect("first source token")
                .spelling()
                .semantic_token(),
            Token::Char {
                ch: 'a',
                cat: Catcode::Letter,
            }
        );
        let after_acquisition = crate::input::source_registration_counters();
        assert_eq!(after_acquisition.calls - before_acquisition.calls, 1);

        assert_eq!(
            processor
                .get_next()
                .expect("warmed source delivery")
                .expect("second source token")
                .spelling()
                .semantic_token(),
            Token::Char {
                ch: 'b',
                cat: Catcode::Letter,
            }
        );
        let after_warmed_token = crate::input::source_registration_counters();
        assert_eq!(after_warmed_token, after_acquisition);

        while processor.get_next().expect("source retirement").is_some() {}
        let after_retirement = crate::input::source_registration_counters();
        assert_eq!(after_retirement.calls, after_acquisition.calls);
        assert!(after_retirement.checks > after_warmed_token.checks);
    });
}

#[test]
fn failed_replacement_registration_retries_at_next_physical_acquisition() {
    crate::test_harness::with_universe(|universe| {
        universe.set_interaction_mode(tex_state::InteractionMode::ErrorStop);
        universe
            .world_mut()
            .push_memory_terminal_line("ab")
            .expect("terminal replacement");
        universe
            .assign_int_param(
                tex_state::env::banks::IntParam::PAUSING,
                1,
                tex_state::env::AssignmentScope::Global,
            )
            .expect("enable pausing");
        let mut command = CommandState::default();
        let source = command
            .register_source(crate::SourceRegistration::new(
                crate::RegisteredSourceKind::Generated,
                &b"physical\nz"[..],
            ))
            .expect("source registration");
        command
            .open_registered_source(source)
            .expect("source opening");
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        context
            .register_source(
                tex_state::SourceId::new(1),
                tex_state::source_map::SourceDescriptor::generated(std::sync::Arc::from(
                    &b"conflict"[..],
                )),
            )
            .expect("replacement source id conflict");
        let before = crate::input::source_registration_counters();
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );

        for expected in ['a', 'b'] {
            assert_eq!(
                processor
                    .get_next()
                    .expect("replacement delivery")
                    .expect("replacement token")
                    .spelling()
                    .semantic_token(),
                Token::Char {
                    ch: expected,
                    cat: Catcode::Letter,
                }
            );
        }
        let after_replacement = crate::input::source_registration_counters();
        assert_eq!(after_replacement.calls - before.calls, 2);

        assert_eq!(
            processor
                .get_next()
                .expect("next physical line")
                .expect("next physical token")
                .spelling()
                .semantic_token(),
            Token::Char {
                ch: 'z',
                cat: Catcode::Letter,
            }
        );
        let after_next_line = crate::input::source_registration_counters();
        assert_eq!(after_next_line.calls - after_replacement.calls, 1);
    });
}

#[test]
fn input_top_transition_refills_only_at_line_boundary_and_backup_clears_direct_source() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        let source = command
            .register_source(crate::SourceRegistration::new(
                crate::RegisteredSourceKind::Generated,
                &b"ab\n"[..],
            ))
            .expect("source registration");
        command
            .open_registered_source(source)
            .expect("source opening");
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );

        let first = processor
            .get_next()
            .expect("first delivery")
            .expect("first character");
        assert_eq!(
            first.spelling().semantic_token(),
            Token::Char {
                ch: 'a',
                cat: Catcode::Letter,
            }
        );
        assert_eq!(first.direct_source_line_number(), Some(1));
        assert_eq!(processor.command.input.current_file_line_number(), 1);
        let first_line_number = match processor.command.input.levels.top_source() {
            Some((_, source)) => source
                .cursor
                .line
                .as_ref()
                .expect("the acquired line remains active")
                .physical
                .number(),
            _ => panic!("the source remains active"),
        };

        let second = processor
            .get_next()
            .expect("second delivery")
            .expect("second character");
        assert_eq!(
            second.spelling().semantic_token(),
            Token::Char {
                ch: 'b',
                cat: Catcode::Letter,
            }
        );
        let second_provenance = processor.source_provenance(&second);
        assert_eq!(second.direct_source_line_number(), Some(1));
        assert_eq!(processor.command.input.current_file_line_number(), 1);
        let second_line_number = match processor.command.input.levels.top_source() {
            Some((_, source)) => source
                .cursor
                .line
                .as_ref()
                .expect("the same physical line remains active")
                .physical
                .number(),
            _ => panic!("the source remains active"),
        };
        assert_eq!(second_line_number, first_line_number);

        processor.back_input(second).expect("backup");
        let replayed = processor
            .get_next()
            .expect("backup delivery")
            .expect("backed-up character");
        assert_eq!(processor.source_provenance(&replayed), second_provenance);
        assert_eq!(replayed.direct_source_line_number(), None);
        assert_eq!(processor.command.input.current_file_line_number(), 1);
    });
}

#[test]
fn direct_source_control_sequences_preserve_creation_policy_after_compact_delivery() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        let source = command
            .register_source(crate::SourceRegistration::new(
                crate::RegisteredSourceKind::Generated,
                &br"\previouslyunseen \previouslyunseen"[..],
            ))
            .expect("source registration");
        command
            .open_registered_source(source)
            .expect("source opening");
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );

        let forbidden = processor
            .get_next()
            .expect("forbidden-creation delivery")
            .expect("first control sequence");
        assert!(
            forbidden
                .spelling()
                .semantic_token()
                .is_undefined_control_sequence()
        );
        assert_eq!(forbidden.control_sequence(), None);

        let allowed = processor
            .get_token()
            .expect("allowed-creation delivery")
            .expect("second control sequence");
        assert!(matches!(allowed.spelling().semantic_token(), Token::Cs(_)));
        assert!(allowed.control_sequence().is_some());
        assert_eq!(allowed.meaning(), Meaning::Undefined);
    });
}

#[cfg(feature = "profiling")]
fn assert_warmed_single_character_control_sequence_is_allocation_free<G>(
    universe: &mut tex_state::Universe<G>,
    name: &str,
    profile: crate::CommandProfile,
    create_control_sequences: bool,
) {
    let expected = universe
        .command_context()
        .expect("command context")
        .intern_control_sequence(name);
    let source_text = format!(r"\{name}\{name}");
    let mut command = CommandState::new(profile);
    let source = command
        .register_source(crate::SourceRegistration::new(
            crate::RegisteredSourceKind::Generated,
            source_text.into_bytes(),
        ))
        .expect("source registration");
    command
        .open_registered_source(source)
        .expect("source opening");
    let mut capabilities = CommandHostCapabilities::default();
    let mut fuel = crate::CommandFuelLedger::default();
    let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
    let mut context = universe.command_context().expect("command context");
    let mut processor = crate::test_harness::processor(
        &mut command,
        &mut context,
        &mut capabilities,
        &mut fuel,
        &mut diagnostic_effects,
    );

    let first = if create_control_sequences {
        processor.get_token()
    } else {
        processor.get_next()
    }
    .expect("warm delivery")
    .expect("first control sequence");
    assert_eq!(first.spelling().semantic_token(), Token::Cs(expected));
    drop(first);

    let owner = tex_state::measurement::HotCoreAllocationOwner::DeliveryAndScan;
    let before = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
    let second = {
        let _scope = tex_state::measurement::hot_core_allocation_scope(owner);
        if create_control_sequences {
            processor.get_token()
        } else {
            processor.get_next()
        }
    }
    .expect("measured delivery")
    .expect("second control sequence");
    let after = tex_state::measurement::hot_core_thread_allocation_measurement(owner);

    assert_eq!(second.spelling().semantic_token(), Token::Cs(expected));
    assert_eq!(after.calls - before.calls, 0);
    assert_eq!(after.requested_bytes - before.requested_bytes, 0);
}

fn assert_superscript_control_word_identity(profile: crate::CommandProfile, source_text: &str) {
    crate::test_harness::with_universe(|universe| {
        universe
            .assign_code(
                tex_state::env::CodeTableKind::Catcode,
                '^',
                i64::from(Catcode::Superscript as u8),
                tex_state::env::AssignmentScope::Global,
            )
            .expect("superscript catcode");
        let mut command = CommandState::new(profile);
        let source = command
            .register_source(crate::SourceRegistration::new(
                crate::RegisteredSourceKind::Generated,
                std::sync::Arc::<[u8]>::from(source_text.as_bytes()),
            ))
            .expect("source registration");
        command
            .open_registered_source(source)
            .expect("source opening");
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );

        let transformed_mutable = processor
            .get_token()
            .expect("mutable transformed delivery")
            .expect("transformed control word")
            .spelling()
            .semantic_token();
        let literal_mutable = processor
            .get_token()
            .expect("mutable literal delivery")
            .expect("literal control word")
            .spelling()
            .semantic_token();
        let transformed_readonly = processor
            .get_next()
            .expect("readonly transformed delivery")
            .expect("transformed control word")
            .spelling()
            .semantic_token();
        let literal_readonly = processor
            .get_next()
            .expect("readonly literal delivery")
            .expect("literal control word")
            .spelling()
            .semantic_token();

        assert_eq!(transformed_mutable, literal_mutable);
        assert_eq!(transformed_readonly, transformed_mutable);
        assert_eq!(literal_readonly, transformed_mutable);
        assert!(matches!(transformed_mutable, Token::Cs(_)));
    });
}

#[test]
fn superscript_control_words_share_literal_identity_in_exact_and_unicode_paths() {
    assert_superscript_control_word_identity(
        crate::CommandProfile::TEX82,
        r"\^^61bc \abc \^^61bc \abc",
    );
    assert_superscript_control_word_identity(
        crate::CommandProfile::unicode_extended(crate::CommandDialect::Tex82),
        r"\^^^^0061bc \abc \^^^^0061bc \abc",
    );
}

#[cfg(feature = "profiling")]
fn assert_warmed_control_word_delivery_allocates_zero(create: bool) {
    crate::test_harness::with_universe(|universe| {
        const WARMUP_DELIVERIES: usize = 1_025;
        const MEASURED_DELIVERIES: usize = 257;
        let source_text = r"\warmedname "
            .repeat(WARMUP_DELIVERIES + MEASURED_DELIVERIES)
            .into_bytes()
            .into_boxed_slice();
        let mut command = CommandState::default();
        let source = command
            .register_source(crate::SourceRegistration::new(
                crate::RegisteredSourceKind::Generated,
                std::sync::Arc::<[u8]>::from(source_text),
            ))
            .expect("source registration");
        command
            .open_registered_source(source)
            .expect("source opening");
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let expected = Token::Cs(context.intern_hash_control_sequence("warmedname"));
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );

        let deliver = |processor: &mut crate::CommandProcessor<'_, '_, _>| {
            let delivered = if create {
                processor.get_token()
            } else {
                processor.get_next()
            }
            .expect("source delivery")
            .expect("control word");
            assert_eq!(delivered.spelling().semantic_token(), expected);
        };
        for _ in 0..WARMUP_DELIVERIES {
            deliver(&mut processor);
        }

        let owner = tex_state::measurement::HotCoreAllocationOwner::DeliveryAndScan;
        let before = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
        {
            let _scope = tex_state::measurement::hot_core_allocation_scope(owner);
            for _ in 0..MEASURED_DELIVERIES {
                deliver(&mut processor);
            }
        }
        let after = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
        assert_eq!(after.calls - before.calls, 0);
        assert_eq!(after.requested_bytes - before.requested_bytes, 0);
    });
}

#[cfg(feature = "profiling")]
#[test]
fn warmed_ascii_single_character_control_sequence_lookup_is_allocation_free() {
    for create_control_sequences in [false, true] {
        crate::test_harness::with_universe(|universe| {
            assert_warmed_single_character_control_sequence_is_allocation_free(
                universe,
                "!",
                crate::CommandProfile::TEX82,
                create_control_sequences,
            );
        });
    }
}

#[cfg(feature = "profiling")]
#[test]
fn warmed_mutable_multiletter_control_word_delivery_allocates_zero_heap() {
    assert_warmed_control_word_delivery_allocates_zero(true);
}

#[cfg(feature = "profiling")]
#[test]
fn warmed_unicode_single_character_control_sequence_lookup_is_allocation_free() {
    for create_control_sequences in [false, true] {
        crate::test_harness::with_universe(|universe| {
            assert_warmed_single_character_control_sequence_is_allocation_free(
                universe,
                "🦀",
                crate::CommandProfile::unicode_extended(crate::CommandDialect::Tex82),
                create_control_sequences,
            );
        });
    }
}

#[cfg(feature = "profiling")]
#[test]
fn warmed_readonly_multiletter_control_word_delivery_allocates_zero_heap() {
    assert_warmed_control_word_delivery_allocates_zero(false);
}

#[cfg(feature = "profiling")]
#[test]
fn warmed_stored_raw_delivery_allocates_zero_heap() {
    crate::test_harness::with_universe(|universe| {
        let token = Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        };
        let list = universe
            .command_context()
            .expect("command context")
            .allocate_token_list(&[
                tex_state::token::TokenWord::pack(token),
                tex_state::token::TokenWord::pack(token),
            ])
            .expect("stored list");
        let mut command = CommandState::default();
        {
            let context = universe.command_context().expect("command context");
            command.push_token_level(
                PackedTokenSpanHandle::durable(context.token_list(list)),
                TokenBehavior::Ordinary,
                RetirementBehavior::Pop,
                ReplayTrace::Stored(StoredReplayReason::EveryPar),
            );
        }
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );
        assert_eq!(
            processor
                .get_next()
                .expect("warm delivery")
                .expect("stored token")
                .spelling()
                .semantic_token(),
            token
        );

        let owner = tex_state::measurement::HotCoreAllocationOwner::DeliveryAndScan;
        let before = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
        let delivered = {
            let _scope = tex_state::measurement::hot_core_allocation_scope(owner);
            processor
                .get_next()
                .expect("measured delivery")
                .expect("stored token")
        };
        let after = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
        assert_eq!(delivered.spelling().semantic_token(), token);
        assert_eq!(after.calls - before.calls, 0);
        assert_eq!(after.requested_bytes - before.requested_bytes, 0);
    });
}

#[test]
fn frozen_macro_primitive_observation_retains_endwrite_identity() {
    crate::test_harness::with_universe(|universe| {
        crate::install_tex82_unexpandable_primitives(universe);
        let endwrite = universe.primitive_token("endwrite").expect("write stopper");
        let mut command = CommandState::default();
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );

        assert_eq!(
            processor.observed_token(TracedTokenWord::pack(endwrite, OriginId::UNKNOWN)),
            crate::observation::ObservedToken::FrozenPrimitive("endwrite".into())
        );
    });
}
