use std::sync::Arc;

use tex_command::{
    AlignmentCellTemplates, AlignmentRequest, CommandDeliveryBoundary, CommandObservation,
    CommandObserver, InputReason, InputTransition, ObservedToken, RegisteredSourceKind,
    SourceRegistration, TracedTokenList,
};
use tex_state::Universe;
use tex_state::env::banks::{GlueParam, IntParam};
use tex_state::meaning::{ExpandablePrimitive, Meaning};
use tex_state::scaled::Scaled;

use super::*;

fn install_input(universe: &mut Universe) {
    let input = universe.intern("input").symbol();
    universe.set_meaning(
        input,
        Meaning::ExpandablePrimitive(ExpandablePrimitive::Input),
    );
}

fn register_source(control: &mut CommandReplayControl, bytes: &[u8]) {
    let source = control
        .command_mut()
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(bytes),
        ))
        .expect("source registers");
    control
        .command_mut()
        .open_registered_source(source)
        .expect("source opens");
}

#[test]
fn replay_uses_typed_scanners_for_definitions_assignments_and_termination() {
    let mut universe = Universe::new();
    crate::install_unexpandable_primitives(&mut universe);
    let mut control = CommandReplayControl::default();
    register_source(
        &mut control,
        br"\def\id#1{#1}\count12=\id{7}\global\def\g{z}\end",
    );

    assert_eq!(
        control.step(&mut universe).expect("definition"),
        ReplayStep::Continue
    );
    assert_eq!(
        control.step(&mut universe).expect("assignment"),
        ReplayStep::Continue
    );
    assert_eq!(universe.count(12), 7);
    assert_eq!(
        control.step(&mut universe).expect("global definition"),
        ReplayStep::Continue
    );
    let id = universe.intern("id").symbol();
    let g = universe.intern("g").symbol();
    assert!(universe.macro_meaning(id).is_some());
    assert!(universe.macro_meaning(g).is_some());
    assert_eq!(control.step(&mut universe).expect("end"), ReplayStep::End);
    assert_eq!(
        control.step(&mut universe).expect("eof"),
        ReplayStep::EndOfInput
    );
}

#[test]
fn canonical_initex_replay_scans_and_applies_integer_parameters() {
    let mut universe = Universe::new();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\year=2026\month=7\end");
    let mut observations = ObservationRecorder::default();

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("year assignment"),
        ReplayStep::Continue
    );
    assert_eq!(universe.int_param(IntParam::YEAR), 2026);
    assert_eq!(
        control.step(&mut universe).expect("month assignment"),
        ReplayStep::Continue
    );
    assert_eq!(universe.int_param(IntParam::MONTH), 7);
    assert_eq!(control.step(&mut universe).expect("end"), ReplayStep::End);

    assert!(matches!(
        observations.0.as_slice(),
        [..,
            CommandObservation::Scanner(scanner),
            CommandObservation::Mutation(mutation)]
            if scanner.kind == "integer"
                && scanner.value == "2026"
                && mutation.target == "parameter"
                && mutation.value == "integer_parameter:23=2026"
    ));
}

#[test]
fn canonical_initex_replay_scans_tabskip_before_alignment_preamble() {
    let mut universe = Universe::new();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\tabskip = 2pt\halign&\end");
    let mut observations = ObservationRecorder::default();

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("tabskip assignment"),
        ReplayStep::Continue
    );
    let tabskip = universe.glue(universe.glue_param(GlueParam::TAB_SKIP));
    assert_eq!(tabskip.width, Scaled::from_raw(2 * Scaled::UNITY));
    assert!(matches!(
        observations.0.as_slice(),
        [.., CommandObservation::Scanner(scanner), CommandObservation::Mutation(mutation)]
            if scanner.kind == "glue"
                && mutation.target == "parameter"
                && mutation.key.as_deref() == Some("glue_parameter:11")
                && mutation.value.starts_with("glue:width=131072")
    ));
    observations.0.clear();

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("alignment"),
        ReplayStep::Continue
    );
    assert!(matches!(
        observations.0.as_slice(),
        [..,
            CommandObservation::Command(raw),
            CommandObservation::Command(expanded),
            CommandObservation::Alignment(alignment)]
            if raw.command == "halign"
                && expanded.command == "halign"
                && alignment.transition == "begin"
                && alignment.align_state == -1_000_000
    ));
    let alignment = control
        .active_alignment()
        .expect("alignment begins after tabskip");
    control
        .apply_alignment_request(AlignmentRequest::Preamble(alignment))
        .expect("preamble lifecycle remains available");
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("scanner-backed alignment token retires before the next delivery"),
        ReplayStep::Continue
    );
    let alignment_begin = observations
        .0
        .iter()
        .position(|event| {
            matches!(event, CommandObservation::Alignment(alignment) if alignment.transition == "begin")
        })
        .expect("replayed hAlign publishes its typed begin transition");
    let backup_retirement = observations
        .0
        .iter()
        .position(|event| {
            matches!(event, CommandObservation::Input(input)
                if input.transition == InputTransition::Retire && input.reason == InputReason::Backup)
        })
        .expect("exhausted hAlign backup retires on the following delivery");
    assert!(alignment_begin < backup_retirement);
}

#[test]
fn omit_cell_sets_body_state_without_backing_up_or_installing_a_u_template() {
    let mut universe = Universe::new();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\halign{u#v\cr {a}\cr \omit b\cr}\end");
    let mut observations = ObservationRecorder::default();

    for _ in 0..20 {
        if control
            .step_with_observer(&mut universe, &mut observations)
            .is_err()
        {
            break;
        }
        if observations.0.windows(3).any(|events| {
            matches!(
                events,
                [
                    CommandObservation::Command(raw),
                    CommandObservation::Command(expanded),
                    CommandObservation::Alignment(state_change),
                ] if raw.command == "omit"
                    && expanded.command == "omit"
                    && state_change.transition == "state_change"
                    && state_change.align_state == 0
                    && state_change.previous_align_state == Some(1_000_000)
            )
        }) {
            break;
        }
    }

    assert!(
        observations.0.windows(3).any(|events| {
            matches!(
                events,
                [
                    CommandObservation::Command(raw),
                    CommandObservation::Command(expanded),
                    CommandObservation::Alignment(state_change),
                ] if raw.command == "omit"
                    && expanded.command == "omit"
                    && state_change.transition == "state_change"
                    && state_change.align_state == 0
                    && state_change.previous_align_state == Some(1_000_000)
            )
        }),
        "omit must transition directly from the lookahead sentinel to the cell body: {:?}",
        observations.0
    );
    assert!(
        !observations.0.windows(3).any(|events| {
            matches!(
                events,
                [
                    CommandObservation::Command(command),
                    CommandObservation::Input(input),
                    ..
                ] if command.command == "omit"
                    && input.transition == InputTransition::Backup
            )
        }),
        "TeX82 init_col never backs up its omit lookahead: {:?}",
        observations.0
    );

    for _ in 0..20 {
        if control
            .step_with_observer(&mut universe, &mut observations)
            .is_err()
        {
            break;
        }
    }
    assert!(
        observations.0.iter().any(|event| {
            matches!(event, CommandObservation::Alignment(template)
                if template.transition == "omit_template_push")
        }),
        "omit must install TeX82's omit_template, not the selected v-template: {:?}",
        observations.0
    );
}

#[test]
fn noalign_uses_command_owned_brace_scan_without_a_generic_backup() {
    let mut universe = Universe::new();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\halign{#\cr {a}\cr\noalign{\relax}{b}\cr}\end",
    );
    let mut observations = ObservationRecorder::default();

    for _ in 0..40 {
        if control
            .step_with_observer(&mut universe, &mut observations)
            .is_err()
        {
            break;
        }
        if observations.0.windows(4).any(|events| {
            matches!(
                events,
                [
                    CommandObservation::Command(raw),
                    CommandObservation::Command(expanded),
                    CommandObservation::Alignment(state_change),
                    CommandObservation::Command(brace),
                ] if raw.command == "no_align"
                    && raw.command_operand == Some(0)
                    && expanded.command == "no_align"
                    && expanded.command_operand == Some(0)
                    && state_change.transition == "begin_group"
                    && state_change.previous_align_state == Some(1_000_000)
                    && state_change.align_state == 1_000_001
                    && brace.spelling == ObservedToken::Character {
                        character: '{',
                        catcode: Catcode::BeginGroup,
                    }
            )
        }) {
            break;
        }
    }

    let noalign = observations
        .0
        .iter()
        .position(|event| {
            matches!(event, CommandObservation::Command(command)
            if command.boundary == CommandDeliveryBoundary::Raw && command.command == "no_align")
        })
        .expect("raw TeX82 no_align delivery");
    let brace = observations
        .0
        .iter()
        .skip(noalign + 1)
        .position(|event| {
            matches!(event, CommandObservation::Command(command)
            if command.boundary == CommandDeliveryBoundary::Raw
                && command.spelling == ObservedToken::Character {
                    character: '{', catcode: Catcode::BeginGroup
                })
        })
        .map(|offset| noalign + 1 + offset)
        .expect("command-owned noalign opening brace");
    assert!(
        observations.0[noalign..=brace]
            .iter()
            .any(|event| matches!(event,
                CommandObservation::Alignment(state_change)
                    if state_change.transition == "begin_group"
                        && state_change.previous_align_state == Some(1_000_000)
                        && state_change.align_state == 1_000_001
            ))
    );
    assert!(
        !observations.0[noalign..=brace]
            .iter()
            .any(|event| matches!(event,
                CommandObservation::Input(input) if input.transition == InputTransition::Backup
            ))
    );
}

#[test]
fn alignment_preamble_opener_uses_command_owned_backup_before_source_resumes() {
    let mut universe = Universe::new();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\halign{ U#\cr{\end");

    assert_eq!(
        control.step(&mut universe).expect("alignment begins"),
        ReplayStep::Continue
    );
    let mut observations = ObservationRecorder::default();
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("preamble opening is backed up"),
        ReplayStep::Continue
    );
    assert!(
        matches!(
            observations.0.as_slice(),
            [..,
                CommandObservation::Alignment(state_change),
                CommandObservation::Command(raw),
                CommandObservation::Command(expanded),
                CommandObservation::Input(backup),
                CommandObservation::Recovery(recovery),
                CommandObservation::Alignment(correction),
            ]
                if state_change.transition == "begin_group"
                    && state_change.align_state == -999_999
                    && state_change.previous_align_state == Some(-1_000_000)
                    && matches!(raw.spelling, ObservedToken::Character { character: '{', .. })
                    && matches!(expanded.spelling, ObservedToken::Character { character: '{', .. })
                    && backup.transition == InputTransition::Backup
                    && backup.reason == InputReason::Backup
                    && recovery.backup
                    && correction.transition == "backup_correction"
                    && correction.align_state == -1_000_000
                    && correction.previous_align_state == Some(-999_999)
        ),
        "unexpected observations: {:?}",
        observations.0
    );

    observations.0.clear();
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("replayed preamble opener is backed up again"),
        ReplayStep::Continue
    );
    assert!(
        matches!(
            observations.0.as_slice(),
            [
                CommandObservation::Alignment(state_change),
                CommandObservation::Command(raw),
                CommandObservation::Command(expanded),
                CommandObservation::Input(retirement),
                CommandObservation::Input(backup),
                CommandObservation::Recovery(recovery),
                CommandObservation::Alignment(correction),
            ]
                if state_change.transition == "begin_group"
                    && state_change.align_state == -999_999
                    && state_change.previous_align_state == Some(-1_000_000)
                    && matches!(raw.spelling, ObservedToken::Character { character: '{', .. })
                    && matches!(expanded.spelling, ObservedToken::Character { character: '{', .. })
                    && retirement.transition == InputTransition::Retire
                    && retirement.reason == InputReason::Backup
                    && backup.transition == InputTransition::Backup
                    && backup.reason == InputReason::Backup
                    && recovery.backup
                    && correction.transition == "backup_correction"
                    && correction.align_state == -1_000_000
                    && correction.previous_align_state == Some(-999_999)
        ),
        "unexpected observations: {:?}",
        observations.0
    );

    observations.0.clear();
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("replayed brace enters the live preamble scanner"),
        ReplayStep::Continue
    );
    assert!(
        matches!(
            observations.0.as_slice(),
            [
                CommandObservation::Alignment(state_change),
                CommandObservation::Command(raw),
                CommandObservation::Command(expanded),
                CommandObservation::ScannerStatus(status),
                CommandObservation::Alignment(preamble_start),
                CommandObservation::Input(retirement),
                CommandObservation::Command(space),
                CommandObservation::Command(template),
                CommandObservation::Command(parameter),
                CommandObservation::Command(terminator),
                CommandObservation::Alignment(preamble_finish),
                CommandObservation::ScannerStatus(finished),
                CommandObservation::Alignment(cell),
            ]
                if state_change.transition == "begin_group"
                    && state_change.align_state == -999_999
                    && state_change.previous_align_state == Some(-1_000_000)
                    && matches!(raw.spelling, ObservedToken::Character { character: '{', .. })
                    && matches!(expanded.spelling, ObservedToken::Character { character: '{', .. })
                    && status.entering
                    && status.status.starts_with("Aligning")
                    && preamble_start.transition == "preamble_start"
                    && preamble_start.align_state == -1_000_000
                    && retirement.transition == InputTransition::Retire
                    && retirement.reason == InputReason::Backup
                    && matches!(space.spelling, ObservedToken::Character { character: ' ', .. })
                    && matches!(template.spelling, ObservedToken::Character { character: 'U', .. })
                    && matches!(parameter.spelling, ObservedToken::Character { character: '#', .. })
                    && matches!(terminator.spelling, ObservedToken::ControlSequence(ref name) if name == "cr")
                    && !finished.entering
                    && finished.status.starts_with("Aligning")
                    && preamble_finish.transition == "preamble_finish"
                    && cell.transition == "state_change"
                    && cell.align_state == 1_000_000
                    && cell.previous_align_state.is_none()
        ),
        "unexpected observations: {:?}",
        observations.0
    );

    observations.0.clear();
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("first cell opener is backed up before the u-template"),
        ReplayStep::Continue
    );
    assert!(
        matches!(
            observations.0.as_slice(),
            [
                CommandObservation::Alignment(state_change),
                CommandObservation::Command(raw),
                CommandObservation::Command(expanded),
                CommandObservation::Input(backup),
                CommandObservation::Recovery(recovery),
                CommandObservation::Alignment(correction),
                CommandObservation::Input(template),
                CommandObservation::Alignment(template_alignment),
            ]
                if state_change.transition == "begin_group"
                    && state_change.align_state == 1_000_001
                    && state_change.previous_align_state == Some(1_000_000)
                    && matches!(raw.spelling, ObservedToken::Character { character: '{', .. })
                    && matches!(expanded.spelling, ObservedToken::Character { character: '{', .. })
                    && backup.transition == InputTransition::Backup
                    && backup.reason == InputReason::Backup
                    && recovery.backup
                    && correction.transition == "backup_correction"
                    && correction.align_state == 1_000_000
                    && correction.previous_align_state == Some(1_000_001)
                    && template.transition == InputTransition::Push
                    && template.reason == InputReason::AlignmentUTemplate
                    && template_alignment.transition == "u_template_push"
                    && template_alignment.align_state == 1_000_000
        ),
        "unexpected observations: {:?}",
        observations.0
    );

    observations.0.clear();
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("u-template delivers its final token"),
        ReplayStep::Continue
    );
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("backed-up u-template token replays"),
        ReplayStep::Continue
    );
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("u-template retires before the cell body resumes"),
        ReplayStep::End
    );
    assert!(
        observations.0.windows(3).any(|events| {
            matches!(
                events,
                [
                    CommandObservation::Input(input),
                    CommandObservation::Alignment(retirement),
                    CommandObservation::Alignment(body),
                ] if input.transition == InputTransition::Retire
                    && input.reason == InputReason::AlignmentUTemplate
                    && retirement.transition == "u_template_retire"
                    && retirement.align_state == 1_000_000
                    && body.transition == "state_change"
                    && body.align_state == 0
                    && body.previous_align_state == Some(1_000_000)
            )
        }),
        "unexpected observations: {:?}",
        observations.0
    );
}

#[test]
fn empty_ordinary_u_template_pushes_and_retires_before_the_cell_opener_replays() {
    let mut universe = Universe::new();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    // The empty list before `#` is an ordinary u-template, not `\omit`.
    // `init_col` backs up any ordinary first-cell command, not just `{`.
    // This mirrors the nested `\halign{#\cr\vrule...}` trace case.
    register_source(&mut control, br"\halign{#\cr\vrule\end");
    let mut observations = ObservationRecorder::default();

    for phase in [
        "alignment begin",
        "preamble opener backup",
        "preamble opener replay",
        "preamble scan and cell setup",
        "cell opener backup and empty template installation",
    ] {
        assert_eq!(
            control
                .step_with_observer(&mut universe, &mut observations)
                .expect(phase),
            ReplayStep::Continue
        );
    }

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("empty template retires before the backed-up opener"),
        ReplayStep::Continue
    );
    assert!(
        observations.0.windows(5).any(|events| {
            matches!(
                events,
                [
                    CommandObservation::Input(push),
                    CommandObservation::Alignment(template_push),
                    CommandObservation::Input(retire),
                    CommandObservation::Alignment(template_retire),
                    CommandObservation::Alignment(body),
                ] if push.transition == InputTransition::Push
                    && push.reason == InputReason::AlignmentUTemplate
                    && template_push.transition == "u_template_push"
                    && template_push.align_state == 1_000_000
                    && retire.transition == InputTransition::Retire
                    && retire.reason == InputReason::AlignmentUTemplate
                    && template_retire.transition == "u_template_retire"
                    && template_retire.align_state == 1_000_000
                    && body.transition == "state_change"
                    && body.align_state == 0
                    && body.previous_align_state == Some(1_000_000)
            )
        }),
        "empty ordinary u-template must retain the TeX82 lifecycle: {:?}",
        observations.0
    );
}

#[test]
fn periodic_preamble_replays_its_u_template_before_retirement() {
    let mut universe = Universe::new();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    // TeX82 §760 treats `&&` as the start of the periodic preamble suffix,
    // not as an empty second column. The following cell must therefore see
    // `\hskip` from that u-template before `end_token_list` retires it.
    register_source(&mut control, br"\halign{#&&\hskip1pt#\cr\relax&\relax\end");
    let mut observations = ObservationRecorder::default();

    for _ in 0..16 {
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("periodic preamble replay");
        if observations.0.windows(6).any(|events| {
            matches!(
                events,
                [
                    CommandObservation::Input(push),
                    CommandObservation::Alignment(template_push),
                    CommandObservation::Command(raw_hskip),
                    CommandObservation::Command(expanded_hskip),
                    CommandObservation::Command(raw_numeric),
                    CommandObservation::Command(expanded_numeric),
                ] if push.transition == InputTransition::Push
                    && push.reason == InputReason::AlignmentUTemplate
                    && template_push.transition == "u_template_push"
                    && raw_hskip.boundary == CommandDeliveryBoundary::Raw
                    && raw_hskip.command == "hskip"
                    && expanded_hskip.boundary == CommandDeliveryBoundary::Expanded
                    && expanded_hskip.command == "hskip"
                    && matches!(raw_numeric.spelling, ObservedToken::Character { character: '1', .. })
                    && matches!(expanded_numeric.spelling, ObservedToken::Character { character: '1', .. })
            )
        }) && observations.0.windows(6).any(|events| {
            matches!(
                events,
                [
                    CommandObservation::Command(raw_hskip),
                    CommandObservation::Command(expanded_hskip),
                    CommandObservation::Command(raw_numeric),
                    CommandObservation::Command(expanded_numeric),
                    CommandObservation::Input(backup),
                    CommandObservation::Recovery(recovery),
                ] if raw_hskip.boundary == CommandDeliveryBoundary::Raw
                    && raw_hskip.command == "hskip"
                    && expanded_hskip.boundary == CommandDeliveryBoundary::Expanded
                    && expanded_hskip.command == "hskip"
                    && matches!(raw_numeric.spelling, ObservedToken::Character { character: '1', .. })
                    && matches!(expanded_numeric.spelling, ObservedToken::Character { character: '1', .. })
                    && backup.transition == InputTransition::Backup
                    && backup.reason == InputReason::Backup
                    && recovery.backup
                    && matches!(recovery.tokens.as_slice(), [ObservedToken::Character { character: '1', .. }])
            )
        }) {
            return;
        }
    }
    panic!(
        "periodic u-template must deliver hskip before retirement: {:?}",
        observations.0
    );
}

#[test]
fn completed_rule_spec_restarts_active_cell_through_typed_delimiter_delivery() {
    let mut universe = Universe::new();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\halign{#\cr{\vrule width1pt}&\end");
    let mut observations = ObservationRecorder::default();

    for phase in [
        "alignment begin",
        "preamble opener backup",
        "preamble opener replay",
        "preamble scan and first cell",
        "cell opener and template installation",
        "replayed cell opener",
    ] {
        assert_eq!(
            control
                .step_with_observer(&mut universe, &mut observations)
                .expect(phase),
            ReplayStep::Continue
        );
        observations.0.clear();
    }

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("rule specification"),
        ReplayStep::Continue
    );
    assert!(
        observations.0.iter().any(|observation| {
            matches!(
                observation,
                CommandObservation::Scanner(scanner)
                    if scanner.kind == "dimension" && scanner.value == "65536"
            )
        }),
        "unexpected rule observations: {:?}",
        observations.0
    );
    observations.0.clear();

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("cell-body closing brace"),
        ReplayStep::Continue
    );
    observations.0.clear();

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("backed-up tab reaches the alignment delivery boundary"),
        ReplayStep::Continue
    );
    assert!(
        observations.0.windows(4).any(|events| {
            matches!(
                events,
                [
                    CommandObservation::Alignment(delimiter),
                    CommandObservation::Input(template_input),
                    CommandObservation::Alignment(template),
                    CommandObservation::Alignment(state_change),
                ] if delimiter.transition == "delimiter"
                    && delimiter.align_state == 0
                    && delimiter.delimiter == Some("tab")
                    && template_input.transition == InputTransition::Push
                    && template_input.reason == InputReason::AlignmentVTemplate
                    && template.transition == "v_template_push"
                    && state_change.transition == "state_change"
                    && state_change.align_state == 1_000_000
            )
        }),
        "unexpected observations: {:?}",
        observations.0
    );
}

#[test]
fn canonical_initex_replay_scans_token_register_assignments_through_command_core() {
    let mut universe = Universe::new();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\toks0={TOKEN LIST}\end");
    let mut observations = ObservationRecorder::default();

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("token-register assignment"),
        ReplayStep::Continue
    );
    assert_eq!(replay_text(universe.tokens(universe.toks(0))), "TOKEN LIST");
    assert!(observations.0.windows(2).any(|pair| {
        matches!(
            &pair[0],
            CommandObservation::Input(input)
                if input.transition == InputTransition::Backup && input.reason == InputReason::Backup
        ) && matches!(
            &pair[1],
            CommandObservation::Recovery(recovery) if recovery.backup
        )
    }));
    assert!(matches!(
        observations.0.as_slice(),
        [..,
            CommandObservation::ScannerStatus(status),
            CommandObservation::TokenList(tokens),
            CommandObservation::Mutation(mutation)]
            if !status.entering
                && tokens.transition == "complete"
                && tokens.purpose == "scan_toks"
                && mutation.target == "register"
                && mutation.key.as_deref() == Some("toks:0")
                && mutation.value == "tokens"
                && !mutation.global
    ));
    assert_eq!(control.step(&mut universe).expect("end"), ReplayStep::End);
}

#[test]
fn canonical_initex_replay_scans_setbox_then_hands_vbox_to_executor() {
    let mut universe = Universe::new();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\setbox10=\vbox{}\end");
    let mut observations = ObservationRecorder::default();

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("setbox prefix"),
        ReplayStep::Continue
    );
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("vbox handoff"),
        ReplayStep::Continue
    );
    assert!(matches!(
        universe.group_kinds().next_back(),
        Some(tex_state::GroupKind::VBox)
    ));
    assert_eq!(
        control.step(&mut universe).expect("replay opener"),
        ReplayStep::Continue
    );
    assert_eq!(
        control.step(&mut universe).expect("enter body"),
        ReplayStep::Continue
    );
    assert_eq!(
        control.step(&mut universe).expect("package vbox"),
        ReplayStep::Continue
    );
    assert!(
        universe.box_reg(10).is_some(),
        "vbox is assigned at group exit"
    );

    assert!(observations.0.windows(2).any(|pair| {
        matches!(
            &pair,
            [CommandObservation::Input(input), CommandObservation::Recovery(recovery)]
                if input.transition == InputTransition::Backup
                    && input.reason == InputReason::Backup
                    && recovery.backup
        )
    }));
    assert!(observations.0.iter().any(|event| {
        matches!(event, CommandObservation::Command(command)
            if command.command == "make_box" && command.command_operand == Some(5))
    }));
}

#[test]
fn canonical_initex_replay_observes_committed_message_effects() {
    let mut universe = Universe::new();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\message{READY}\end");
    let mut observations = ObservationRecorder::default();

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("message"),
        ReplayStep::Continue
    );
    assert!(matches!(
        observations.0.last(),
        Some(CommandObservation::Effect(effect))
            if effect.kind == "message" && effect.detail == "READY"
    ));
    assert_eq!(control.step(&mut universe).expect("end"), ReplayStep::End);
}

#[test]
fn canonical_initex_replay_scans_and_applies_code_table_assignments() {
    let mut universe = Universe::new();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\catcode`@=11 \lccode`Z=`z \end");

    assert_eq!(
        control.step(&mut universe).expect("catcode assignment"),
        ReplayStep::Continue
    );
    assert_eq!(universe.catcode('@'), tex_state::token::Catcode::Letter);
    assert_eq!(
        control.step(&mut universe).expect("lccode assignment"),
        ReplayStep::Continue
    );
    assert_eq!(universe.lccode('Z'), u32::from('z'));
    assert_eq!(control.step(&mut universe).expect("end"), ReplayStep::End);
}

#[test]
fn canonical_initex_replay_scans_raw_let_operands_and_commits_the_meaning() {
    let mut universe = Universe::new();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\let\alias = \begingroup\end");
    let mut observations = ObservationRecorder::default();

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("let assignment"),
        ReplayStep::Continue
    );
    let alias = universe.symbol("alias").expect("let target is interned");
    assert_eq!(
        universe.meaning(alias),
        Meaning::UnexpandablePrimitive(tex_state::meaning::UnexpandablePrimitive::BeginGroup)
    );
    assert!(matches!(
        observations.0.last(),
        Some(CommandObservation::Mutation(mutation))
            if mutation.target == "meaning"
                && mutation.key.as_deref() == Some("alias")
                && mutation.value == "begin_group"
    ));
    assert_eq!(control.step(&mut universe).expect("end"), ReplayStep::End);
}

#[test]
fn canonical_initex_replay_futurelet_preserves_lookahead_order_after_assignment() {
    let mut universe = Universe::new();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\futurelet\next\first x\end");
    let mut observations = ObservationRecorder::default();

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("futurelet assignment"),
        ReplayStep::Continue
    );
    let next = universe
        .symbol("next")
        .expect("futurelet target is interned");
    assert_eq!(
        universe.meaning(next),
        Meaning::CharToken {
            ch: 'x',
            cat: tex_state::token::Catcode::Letter,
        }
    );
    assert!(matches!(
        observations.0.last(),
        Some(CommandObservation::Mutation(mutation))
            if mutation.target == "meaning" && mutation.key.as_deref() == Some("next")
    ));

    observations.0.clear();
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("first lookahead replay"),
        ReplayStep::Continue
    );
    assert!(matches!(
        observations.0.as_slice(),
        [.., CommandObservation::Command(delivery)]
            if matches!(delivery.spelling, ObservedToken::ControlSequence(ref name) if name == "first")
    ));
    observations.0.clear();
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("second lookahead replay"),
        ReplayStep::Continue
    );
    assert!(observations.0.iter().any(|observation| {
        matches!(
            observation,
            CommandObservation::Command(delivery)
                if matches!(delivery.spelling, ObservedToken::Character { character: 'x', .. })
        )
    }));
    assert_eq!(control.current_mode(), crate::Mode::Horizontal);
    assert_eq!(
        control
            .step(&mut universe)
            .expect("paragraph character replay"),
        ReplayStep::Continue
    );
    assert_eq!(control.step(&mut universe).expect("end"), ReplayStep::End);
}

#[test]
fn canonical_initex_replay_scans_and_applies_dimension_and_glue_registers() {
    let mut universe = Universe::new();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\dimen0=1.5pt\skip0=2pt plus 3fil minus 4pt\end",
    );
    let mut observations = ObservationRecorder::default();

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("dimension assignment"),
        ReplayStep::Continue
    );
    assert_eq!(universe.dimen(0).raw(), 98_304);
    assert!(matches!(
        observations.0.as_slice(),
        [..,
            CommandObservation::Scanner(scanner),
            CommandObservation::Mutation(mutation)]
            if scanner.kind == "dimension"
                && scanner.value == "98304"
                && mutation.target == "register"
                && mutation.key.as_deref() == Some("dimen:0")
                && mutation.value == "scaled:98304"
    ));

    observations.0.clear();
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("glue assignment"),
        ReplayStep::Continue
    );
    let glue = universe.glue(universe.skip(0));
    assert_eq!(glue.width.raw(), 131_072);
    assert_eq!(glue.stretch.raw(), 196_608);
    assert_eq!(glue.stretch_order, tex_state::glue::Order::Fil);
    assert_eq!(glue.shrink.raw(), 262_144);
    assert!(matches!(
        observations.0.as_slice(),
        [..,
            CommandObservation::Scanner(scanner),
            CommandObservation::Mutation(mutation)]
            if scanner.kind == "glue"
                && mutation.target == "register"
                && mutation.key.as_deref() == Some("skip:0")
                && mutation.value.starts_with("glue:width=131072;")
    ));
    assert_eq!(control.step(&mut universe).expect("end"), ReplayStep::End);
}

#[test]
fn canonical_initex_replay_scans_complete_rule_specs_through_command_control() {
    let mut universe = Universe::new();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\vrule width1pt height2pt depth0pt\hrule width3pt height4pt depth1pt\end",
    );
    let mut observations = ObservationRecorder::default();

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("vertical rule"),
        ReplayStep::Continue
    );
    assert!(observations.0.iter().any(|observation| {
        matches!(
            observation,
            CommandObservation::Scanner(scanner)
                if scanner.kind == "dimension" && scanner.value == "65536"
        )
    }));
    assert!(observations.0.iter().any(|observation| {
        matches!(
            observation,
            CommandObservation::Command(delivery)
                if matches!(delivery.spelling, ObservedToken::Character { character: 'w', .. })
        )
    }));

    observations.0.clear();
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("horizontal rule"),
        ReplayStep::Continue
    );
    assert!(observations.0.iter().any(|observation| {
        matches!(
            observation,
            CommandObservation::Scanner(scanner)
                if scanner.kind == "dimension" && scanner.value == "196608"
        )
    }));
    assert_eq!(control.step(&mut universe).expect("end"), ReplayStep::End);
}

#[derive(Default)]
struct ObservationRecorder(Vec<CommandObservation>);

impl CommandObserver for ObservationRecorder {
    fn committed(&mut self, observation: CommandObservation) {
        self.0.push(observation);
    }
}

#[test]
fn replay_expands_registered_input_without_executor_source_consumption() {
    let mut universe = Universe::new();
    crate::install_unexpandable_primitives(&mut universe);
    install_input(&mut universe);
    let mut control = CommandReplayControl::default();
    control.capabilities_mut().register_input(
        "child",
        SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(&b"\\count3=9"[..]),
        ),
    );
    register_source(&mut control, br"\input child\count4=8\end");

    assert_eq!(
        control.step(&mut universe).expect("nested assignment"),
        ReplayStep::Continue
    );
    assert_eq!(universe.count(3), 9);
    assert_eq!(
        control.step(&mut universe).expect("parent assignment"),
        ReplayStep::Continue
    );
    assert_eq!(universe.count(4), 8);
    assert_eq!(control.step(&mut universe).expect("end"), ReplayStep::End);
}

#[test]
fn replay_command_snapshot_restores_typed_scanner_input_deterministically() {
    let mut universe = Universe::new();
    crate::install_unexpandable_primitives(&mut universe);
    let mut control = CommandReplayControl::default();
    register_source(&mut control, br"\count12=7\end");
    let snapshot = control.command_mut().snapshot();

    assert_eq!(
        control.step(&mut universe).expect("first assignment"),
        ReplayStep::Continue
    );
    assert_eq!(universe.count(12), 7);

    control
        .command_mut()
        .rollback(snapshot)
        .expect("command snapshot restores scanner-owned input");
    let mut replayed_universe = Universe::new();
    crate::install_unexpandable_primitives(&mut replayed_universe);
    assert_eq!(
        control
            .step(&mut replayed_universe)
            .expect("replayed assignment"),
        ReplayStep::Continue
    );
    assert_eq!(replayed_universe.count(12), 7);
    assert_eq!(
        control.step(&mut replayed_universe).expect("end"),
        ReplayStep::End
    );
}

#[test]
fn replay_dispatches_modes_effects_and_typed_alignment_lifecycle() {
    let mut universe = Universe::new();
    crate::install_unexpandable_primitives(&mut universe);
    let mut control = CommandReplayControl::default();
    register_source(&mut control, br"a$ $\par\message{ok}\halign&\end");

    assert_eq!(
        control.step(&mut universe).expect("character"),
        ReplayStep::Continue
    );
    assert_eq!(control.current_mode(), crate::Mode::Horizontal);
    assert_eq!(
        control.step(&mut universe).expect("backed-up character"),
        ReplayStep::Continue
    );
    assert_eq!(control.current_mode(), crate::Mode::Horizontal);
    assert_eq!(
        control.step(&mut universe).expect("math start"),
        ReplayStep::Continue
    );
    assert_eq!(control.current_mode(), crate::Mode::Math);
    assert_eq!(
        control.step(&mut universe).expect("math space"),
        ReplayStep::Continue
    );
    assert_eq!(
        control.step(&mut universe).expect("math end"),
        ReplayStep::Continue
    );
    assert_eq!(control.current_mode(), crate::Mode::Horizontal);
    assert_eq!(
        control.step(&mut universe).expect("paragraph"),
        ReplayStep::Continue
    );
    assert_eq!(control.current_mode(), crate::Mode::Vertical);
    assert_eq!(
        control.step(&mut universe).expect("message"),
        ReplayStep::Continue
    );
    assert!(matches!(
        universe.world().effect_records(),
        [tex_state::EffectRecord::StreamWrite { text, .. }] if text == "ok"
    ));
    assert_eq!(
        control.step(&mut universe).expect("alignment"),
        ReplayStep::Continue
    );
    let alignment = control
        .active_alignment()
        .expect("typed alignment identity");
    control
        .apply_alignment_request(AlignmentRequest::Preamble(alignment))
        .expect("preamble lifecycle");
    control
        .apply_alignment_request(AlignmentRequest::BeginCell {
            alignment,
            templates: AlignmentCellTemplates {
                u_template: None,
                v_template: TracedTokenList::synthetic(universe.intern_token_list(&[])),
            },
        })
        .expect("cell lifecycle");
    control
        .apply_alignment_request(AlignmentRequest::InstallCellTemplate(alignment))
        .expect("cell template lifecycle");
    assert_eq!(
        control
            .alignment_step(alignment, &mut universe)
            .expect("command processor intercepts the cell delimiter"),
        ReplayStep::Continue
    );
    assert_eq!(
        control
            .step(&mut universe)
            .expect("expanded end-v finishes the cell through typed command control"),
        ReplayStep::Continue
    );
    control
        .apply_alignment_request(AlignmentRequest::Finish(alignment))
        .expect("alignment lifecycle finishes through command core");
    assert_eq!(control.active_alignment(), None);
    assert_eq!(
        control
            .step(&mut universe)
            .expect("saved delimiter does not re-enter ordinary delivery"),
        ReplayStep::End
    );
    assert_eq!(
        control
            .step(&mut universe)
            .expect("input exhausted after end"),
        ReplayStep::EndOfInput
    );
}

#[test]
fn command_owned_endv_finishes_cell_and_publishes_retirement_in_canonical_order() {
    let mut universe = Universe::new();
    crate::install_unexpandable_primitives(&mut universe);
    let mut control = CommandReplayControl::default();
    register_source(&mut control, br"\halign&\end");

    assert_eq!(
        control.step(&mut universe).expect("alignment"),
        ReplayStep::Continue
    );
    let alignment = control.active_alignment().expect("active alignment");
    for request in [
        AlignmentRequest::Preamble(alignment),
        AlignmentRequest::BeginCell {
            alignment,
            templates: AlignmentCellTemplates {
                u_template: None,
                v_template: TracedTokenList::synthetic(universe.intern_token_list(&[])),
            },
        },
        AlignmentRequest::InstallCellTemplate(alignment),
    ] {
        control
            .apply_alignment_request(request)
            .expect("cell lifecycle setup");
    }
    assert_eq!(
        control.step(&mut universe).expect("intercepted delimiter"),
        ReplayStep::Continue
    );

    let mut observations = ObservationRecorder::default();
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("command-owned end-v"),
        ReplayStep::Continue
    );
    assert!(
        observations.0.windows(5).any(|events| {
            matches!(
                events,
                [
                    CommandObservation::Command(raw),
                    CommandObservation::Command(expanded),
                    CommandObservation::Alignment(state_change),
                    CommandObservation::Input(retirement),
                    CommandObservation::Alignment(template_retire),
                ] if raw.command == "end_template"
                    && expanded.command == "endv"
                    && state_change.transition == "state_change"
                    && state_change.align_state == 1_000_000
                    && retirement.transition == InputTransition::Retire
                    && retirement.reason == InputReason::AlignmentVTemplate
                    && template_retire.transition == "v_template_retire"
                    && template_retire.align_state == 1_000_000
            )
        }),
        "unexpected observations: {:?}",
        observations.0
    );

    control
        .apply_alignment_request(AlignmentRequest::Finish(alignment))
        .expect("alignment lifecycle finishes through command core");
    assert_eq!(
        control
            .step(&mut universe)
            .expect("saved delimiter does not re-enter ordinary delivery"),
        ReplayStep::End
    );
}

#[test]
fn nested_alignment_begin_suspends_the_outer_replay_context() {
    let mut universe = Universe::new();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    let outer = AlignmentIdentity::new(1);
    control
        .command
        .apply_alignment_request(AlignmentRequest::Begin(outer))
        .expect("outer alignment begins");
    control.active_alignment = Some(ActiveReplayAlignment {
        identity: outer,
        columns: Vec::new(),
        repeat_start: None,
        column: 0,
        preamble_opening_pending: false,
        preamble_opening_replay_pending: false,
        preamble_start_pending: false,
        cell_opening_pending: false,
        next_cell_opening_pending: false,
        align_peek_pending: false,
        align_peek_after_noalign: false,
        noalign_depth: None,
    });
    control.next_alignment_identity = 2;

    apply_scanned_step(
        ScannedStep::BeginAlignment { vertical: false },
        &mut universe,
        &mut control.modes,
        &mut control.next_alignment_identity,
        &mut control.active_alignment,
        &mut control.command,
        &mut control.boxes,
    )
    .expect("nested alignment begins through typed suspension");

    assert_eq!(control.boxes.suspended_alignments.len(), 1);
    let inner = control
        .active_alignment()
        .expect("inner alignment is active");
    assert_ne!(inner, outer);
    apply_scanned_step(
        ScannedStep::AlignmentFinish { alignment: inner },
        &mut universe,
        &mut control.modes,
        &mut control.next_alignment_identity,
        &mut control.active_alignment,
        &mut control.command,
        &mut control.boxes,
    )
    .expect("right-brace align_peek finish resumes the outer context");
    assert_eq!(control.active_alignment(), Some(outer));
    assert_eq!(control.boxes.suspended_alignments.len(), 0);
}

#[test]
fn scanner_backed_endv_retires_before_an_omit_template() {
    let mut universe = Universe::new();
    crate::install_unexpandable_primitives(&mut universe);
    let mut control = CommandReplayControl::default();
    // `scan_rule_spec` reads the alignment delimiter while scanning the
    // omitted cell's rule. Its scalar retry backs up the effective `endv`,
    // reproducing TeX82 §772's exhausted backup above `omit_template`.
    register_source(
        &mut control,
        br"\halign{#&#\cr\omit\vrule width3pt&\relax\cr}\end",
    );
    let mut observations = ObservationRecorder::default();

    for _ in 0..48 {
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("scanner-backed end-v completion");
        if observations.0.windows(6).any(|events| {
            matches!(
                events,
                [
                    CommandObservation::Command(raw),
                    CommandObservation::Command(expanded),
                    CommandObservation::Alignment(state_change),
                    CommandObservation::Input(backup_retirement),
                    CommandObservation::Input(template_retirement),
                    CommandObservation::Alignment(template),
                ] if raw.command == "endv"
                    && expanded.command == "endv"
                    && state_change.transition == "state_change"
                    && backup_retirement.transition == InputTransition::Retire
                    && backup_retirement.reason == InputReason::Backup
                    && template_retirement.transition == InputTransition::Retire
                    && template_retirement.reason == InputReason::AlignmentVTemplate
                    && template.transition == "omit_template_retire"
            )
        }) {
            return;
        }
    }
    panic!(
        "scanner-backed end-v must retire backup then omit-template: {:?}",
        observations.0
    );
}

#[test]
fn paragraph_start_backs_up_the_triggering_macro_parameter_before_replay() {
    let mut universe = Universe::new();
    crate::install_unexpandable_primitives(&mut universe);
    let mut control = CommandReplayControl::default();
    register_source(&mut control, br"\def\pair#1#2{#2#1}\pair AB\end");

    assert_eq!(
        control.step(&mut universe).expect("definition"),
        ReplayStep::Continue
    );

    let mut observations = ObservationRecorder::default();
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("paragraph start"),
        ReplayStep::Continue
    );
    assert_eq!(control.current_mode(), crate::Mode::Horizontal);
    assert!(observations.0.windows(2).any(|pair| {
        matches!(
            &pair[0],
            CommandObservation::Input(input)
                if input.transition == InputTransition::Backup && input.reason == InputReason::Backup
        ) && matches!(
            &pair[1],
            CommandObservation::Recovery(recovery)
                if recovery.backup
                    && matches!(
                        recovery.tokens.as_slice(),
                        [ObservedToken::Character { character: 'B', .. }]
                    )
        )
    }));

    observations.0.clear();
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("backed-up character replay"),
        ReplayStep::Continue
    );
    observations.0.clear();
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("following macro parameter"),
        ReplayStep::Continue
    );
    assert!(observations.0.iter().any(|observation| {
        matches!(
            observation,
            CommandObservation::Input(input)
                if input.transition == InputTransition::Retire && input.reason == InputReason::Backup
        )
    }));
    assert!(observations.0.iter().any(|observation| {
        matches!(
            observation,
            CommandObservation::Command(delivery)
                if matches!(
                    delivery.spelling,
                    ObservedToken::Character { character: 'A', .. }
                )
        )
    }));
}
