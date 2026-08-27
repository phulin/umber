use tex_state::meaning::Meaning;
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use crate::{
    AlignmentIdentity, CommandDeliveryBoundary, CommandHostCapabilities, CommandObservation,
    CommandObserver, CommandState, DeliveryStatus,
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
            (
                delivered.delivery_stamp(),
                delivered.source_range().expect("source range"),
                delivered.source_location().expect("source location"),
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
        assert_eq!(raw.provenance.delivery_sequence, stamp.sequence());
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
