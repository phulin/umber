use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::conditionals::{ConditionFrame, ConditionalKind, IfLimit};
use crate::input::SharedTokenBuffer;
use crate::macro_call::{MacroActivation, MacroActivationId, MacroArgumentRange, MacroArguments};
use crate::processor::{
    AbsorbingContext, ActiveCellDelivery, AlignmentCellTemplates, AlignmentId, AlignmentIdentity,
    AlignmentScanContext, ArgumentBuilderId, ConditionId, DefinitionContext, MatchingContext,
    ScannerStatus, ScannerWarning, SkippingContext, SuspendedAlignment, TokenBuilderId,
};
use crate::profile::{CommandProfile, CommandProfileBoundary, CommandProfileFingerprint};
use crate::state::LiveTokenBuilder;
use crate::{CommandRuntime, CommandState};
use crate::{RegisteredSourceKind, SourceRegistration};
use tex_state::ids::TokenListId;
use tex_state::input::TracedTokenList;

use super::{CommandSummary, CommandSummaryError};

fn semantic_hash<T: Hash>(value: &T) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

fn templates() -> AlignmentCellTemplates {
    AlignmentCellTemplates {
        u_template: Some(TracedTokenList::synthetic(TokenListId::EMPTY)),
        v_template: TracedTokenList::synthetic(TokenListId::EMPTY),
    }
}

fn populated_quiescent_state() -> CommandState {
    let mut state = CommandState::new(CommandProfile::unicode_extended(
        crate::CommandDialect::Pdftex14029,
    ));
    let source = state
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Vec::from(&b"first  \r\nsecond"[..]),
        ))
        .expect("valid Unicode source");
    state
        .open_registered_source(source)
        .expect("registered source opens without host access");
    state
        .load_next_source_line(13)
        .expect("first physical line");
    state.next_source_character().expect("first scalar");
    state.input.next_level_identity = 11;
    state.input.next_source_identity = 13;
    state.parameters.activations.push(MacroActivation {
        identity: MacroActivationId(17),
        name: tex_state::interner::Symbol::testing_new(23),
        definition: tex_state::ids::MacroDefinitionId::testing_new(19),
        arguments: MacroArguments {
            buffer: SharedTokenBuffer::default(),
            ranges: [
                MacroArgumentRange::new(0, 0),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            ],
        },
        invocation: tex_state::token::OriginId::UNKNOWN,
    });
    state.conditions.frames.push(ConditionFrame {
        identity: ConditionId(23),
        kind: ConditionalKind::IfNum,
        limit: IfLimit::Else,
        source_line: 37,
        inverted: false,
    });
    state.conditions.next_identity = 41;
    state.alignment.align_state = 43;
    state.expansion.cumulative_expansions = 47;
    state.expansion.next_resource_resolution = 53;
    state.expansion.pending_diagnostics.push(59);
    state.expansion.observed_dependencies.push(61);
    state.expansion.semantic_barriers.push(67);
    state.transient.next_builder_identity = 71;
    state
}

#[test]
fn snapshot_roundtrip_preserves_nonquiescent_semantic_state() {
    let mut state = populated_quiescent_state();
    let status = ScannerStatus::Matching(MatchingContext {
        macro_name: tex_state::interner::Symbol::testing_new(73),
        builder: ArgumentBuilderId(79),
        warning: ScannerWarning(83),
    });
    let (expected, snapshot) = state.with_scanner_status(status, |state| {
        state.transient.builders.push(LiveTokenBuilder {
            identity: 79,
            tokens: Vec::new(),
        });
        state.transient.rollback_roots.push(89);
        state.transient.active_expansion_depth = 2;
        state.alignment.active_cell = Some(ActiveCellDelivery {
            alignment: AlignmentIdentity::new(97),
            templates: templates(),
            u_template_installed: false,
            u_level: None,
            v_level: None,
            delimiter: None,
            omit: false,
            omit_previous_align_state: None,
        });
        (state.clone(), state.snapshot())
    });

    state = CommandState::new(expected.profile());
    let runtime = CommandRuntime::default();
    state.rollback(snapshot).expect("matching snapshot profile");

    assert_eq!(state, expected);
    drop(runtime);
}

#[test]
fn snapshot_roundtrip_preserves_nested_alignment_delivery_lifecycle() {
    let mut state = CommandState::default();
    let outer = AlignmentIdentity::new(1);
    let inner = AlignmentIdentity::new(2);

    state.begin_alignment(outer);
    state
        .begin_alignment_cell(outer, templates())
        .expect("outer cell begins");
    state
        .suspend_alignment(outer)
        .expect("outer delivery suspends");
    state.begin_alignment(inner);
    state
        .begin_alignment_cell(inner, templates())
        .expect("inner cell begins");
    let expected = state.clone();
    let snapshot = state.snapshot();

    state
        .finish_alignment(inner)
        .expect("inner delivery can mutate after capture");
    state.rollback(snapshot).expect("matching snapshot profile");

    assert_eq!(state, expected);
    assert_eq!(
        state.publish_summary(),
        Err(CommandSummaryError::AlignmentTemplateActive),
        "an active inner template must keep the nested lifecycle nonquiescent"
    );
}

#[test]
fn quiescent_summary_roundtrip_is_exact_and_deterministic() {
    let expected = populated_quiescent_state();
    let summary = expected
        .publish_summary()
        .expect("the complete quiescent state must be publishable");
    let summary_clone = summary.clone();
    let original_hash = semantic_hash(&summary);

    let mut restored = CommandState::new(expected.profile());
    restored
        .restore_summary(summary)
        .expect("matching summary profile");
    let republished = restored
        .publish_summary()
        .expect("a restored summary must remain quiescent");

    assert_eq!(restored, expected);
    assert_eq!(republished, summary_clone);
    assert_eq!(semantic_hash(&republished), original_hash);
}

#[test]
fn force_eof_is_owned_by_snapshot_identity_and_rollback() {
    let mut state = populated_quiescent_state();
    let false_snapshot = state.snapshot();
    let false_hash = semantic_hash(&false_snapshot);
    state.input.force_eof = true;
    let true_snapshot = state.snapshot();

    assert_ne!(semantic_hash(&true_snapshot), false_hash);
    state.input.force_eof = false;
    state
        .rollback(true_snapshot)
        .expect("non-default input state rolls back");
    assert!(state.input.force_eof);
}

fn assert_rejected(mutate: impl FnOnce(&mut CommandState), expected: CommandSummaryError) {
    let mut state = populated_quiescent_state();
    mutate(&mut state);
    assert_eq!(state.publish_summary(), Err(expected));
}

#[test]
fn summary_rejects_each_scanner_episode() {
    let mut state = populated_quiescent_state();
    let cases = [
        (
            ScannerStatus::Skipping(SkippingContext {
                condition: ConditionId(1),
                warning: ScannerWarning(1),
                skip_line: 0,
                conditional: crate::conditionals::ConditionalKind::IfTrue,
            }),
            CommandSummaryError::ConditionalSkip,
        ),
        (
            ScannerStatus::Matching(MatchingContext {
                macro_name: tex_state::interner::Symbol::testing_new(1),
                builder: ArgumentBuilderId(2),
                warning: ScannerWarning(1),
            }),
            CommandSummaryError::MacroMatch,
        ),
        (
            ScannerStatus::Defining(DefinitionContext {
                target: Some(tex_state::interner::Symbol::testing_new(1)),
                builder: TokenBuilderId(2),
                warning: ScannerWarning(1),
            }),
            CommandSummaryError::DefinitionScan,
        ),
        (
            ScannerStatus::Aligning(AlignmentScanContext {
                alignment: AlignmentId(1),
                builder: TokenBuilderId(2),
                owner: None,
                warning: ScannerWarning(1),
            }),
            CommandSummaryError::AlignmentScan,
        ),
        (
            ScannerStatus::Absorbing(AbsorbingContext {
                owner: Some(tex_state::interner::Symbol::testing_new(1)),
                builder: TokenBuilderId(2),
                warning: ScannerWarning(1),
            }),
            CommandSummaryError::AbsorbingScan,
        ),
    ];
    for (status, expected) in cases {
        let actual = state.with_scanner_status(status, |state| state.publish_summary());
        assert_eq!(actual, Err(expected));
    }
}

#[test]
fn summary_rejects_expansion_alignment_and_live_transients() {
    assert_rejected(
        |state| state.transient.active_expansion_depth = 1,
        CommandSummaryError::ExpansionActive,
    );
    assert_rejected(
        |state| {
            state.alignment.active_cell = Some(ActiveCellDelivery {
                alignment: AlignmentIdentity::new(1),
                templates: templates(),
                u_template_installed: false,
                u_level: None,
                v_level: None,
                delimiter: None,
                omit: false,
                omit_previous_align_state: None,
            });
        },
        CommandSummaryError::AlignmentTemplateActive,
    );
    assert_rejected(
        |state| {
            state.alignment.suspended.push(SuspendedAlignment {
                alignment: AlignmentIdentity::new(1),
                active_cell: None,
            });
        },
        CommandSummaryError::SuspendedAlignment,
    );
    assert_rejected(
        |state| {
            state.transient.builders.push(LiveTokenBuilder {
                identity: 1,
                tokens: Vec::new(),
            });
        },
        CommandSummaryError::LiveTokenBuilder,
    );
    assert_rejected(
        |state| state.transient.rollback_roots.push(1),
        CommandSummaryError::LiveRollbackRoot,
    );
}

#[test]
fn snapshot_and_summary_are_owned_static_values() {
    fn assert_owned<T: Clone + Eq + Hash + Send + Sync + 'static>() {}

    assert_owned::<super::CommandStateSnapshot>();
    assert_owned::<CommandSummary>();
}

#[test]
fn paragraph_input_recording_rolls_back_with_command_snapshot() {
    let mut state = populated_quiescent_state();
    state.begin_paragraph_input_transaction();
    let snapshot = state.snapshot();
    state.record_paragraph_observation(&crate::CommandObservation::Input(crate::InputRecord {
        transition: crate::InputTransition::Backup,
        reason: crate::InputReason::Backup,
        source_name: None,
        source: None,
        level: 7,
        position: 11,
    }));
    state
        .next_source_character()
        .expect("recorded source advances");

    state
        .rollback(snapshot)
        .expect("paragraph recording is ordinary command state");
    let transaction = state
        .finish_paragraph_input_transaction()
        .expect("the restored recording remains active");
    assert_eq!(transaction.coverage().transitions().len(), 0);
    assert_eq!(
        transaction.coverage().root_start(),
        transaction.coverage().root_end()
    );
}

#[test]
fn paragraph_input_transaction_replays_only_from_exact_start() {
    let mut start = populated_quiescent_state();
    let mut recorded = start.clone();
    recorded.begin_paragraph_input_transaction();
    recorded.next_source_character().expect("source advances");
    let transaction = recorded
        .finish_paragraph_input_transaction()
        .expect("recording finishes");

    start
        .replay_paragraph_input_transaction(&transaction)
        .expect("exact starting input replays");
    assert_eq!(start.input, recorded.input);
    assert_eq!(
        start.replay_paragraph_input_transaction(&transaction),
        Err(crate::ParagraphInputReplayError::StartingInputMismatch)
    );
}

#[test]
fn summary_rejects_active_paragraph_input_recording() {
    assert_rejected(
        CommandState::begin_paragraph_input_transaction,
        CommandSummaryError::ActiveParagraphInputTransaction,
    );
}

#[test]
fn snapshot_and_summary_reject_profile_mismatch_without_mutation() {
    let foreign = populated_quiescent_state();
    let snapshot = foreign.snapshot();
    let summary = foreign
        .publish_summary()
        .expect("foreign state is quiescent");
    let mut state = CommandState::new(CommandProfile::TEX82);
    let expected = state.clone();

    let snapshot_error = state
        .rollback(snapshot)
        .expect_err("snapshot from another profile must be rejected");
    assert_eq!(snapshot_error.boundary(), CommandProfileBoundary::Snapshot);
    assert_eq!(state, expected);

    let summary_error = state
        .restore_summary(summary)
        .expect_err("summary from another profile must be rejected");
    assert_eq!(summary_error.boundary(), CommandProfileBoundary::Summary);
    assert_eq!(state, expected);
}

#[test]
fn retained_source_backing_and_partial_line_cursors_restore_without_host_access() {
    let profile = CommandProfile::unicode_extended(crate::CommandDialect::Tex82);
    let mut state = CommandState::new(profile);
    let source = state
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::World,
            Vec::from("é \r\nnext".as_bytes()),
        ))
        .expect("valid immutable Unicode backing");
    state
        .open_registered_source(source)
        .expect("registered source opens");
    let physical = state
        .load_next_source_line(13)
        .expect("first physical line");
    assert_eq!(physical.terminator(), crate::LineTerminator::CrLf);
    let first = state.next_source_character().expect("first scalar");
    assert_eq!(first.code(), crate::CharacterCode::from('é'));
    assert_eq!((first.range().start(), first.range().end()), (0, 2));

    let snapshot = state.snapshot();
    let summary = state
        .publish_summary()
        .expect("a partial source line has no active command episode");
    assert!(
        state
            .next_source_character()
            .expect("endline")
            .is_synthetic()
    );
    state.finish_source_line();
    state
        .load_next_source_line(-1)
        .expect("second physical line");

    state
        .rollback(snapshot)
        .expect("retained backing snapshot restores");
    let restored_endline = state.next_source_character().expect("restored endline");
    assert!(restored_endline.is_synthetic());
    assert_eq!(
        (
            restored_endline.range().start(),
            restored_endline.range().end()
        ),
        (2, 2)
    );

    let mut from_summary = CommandState::new(profile);
    from_summary
        .restore_summary(summary)
        .expect("retained backing summary restores");
    assert!(
        from_summary
            .next_source_character()
            .expect("summary-restored endline")
            .is_synthetic()
    );
}

#[test]
fn format_and_checkpoint_profile_components_reject_mismatch() {
    let profiles = [
        CommandProfile::TEX82,
        CommandProfile::ETEX26,
        CommandProfile::PDFTEX14029,
        CommandProfile::unicode_extended(crate::CommandDialect::Tex82),
        CommandProfile::unicode_extended(crate::CommandDialect::Etex26),
        CommandProfile::unicode_extended(crate::CommandDialect::Pdftex14029),
    ];

    for profile in profiles {
        let state = CommandState::new(profile);
        let matching = profile.fingerprint();
        assert_eq!(state.validate_format_profile(matching), Ok(()));
        assert_eq!(state.validate_checkpoint_profile(matching), Ok(()));
        assert_eq!(
            state.format_profile_fingerprint(),
            CommandProfileFingerprint::from_u64(matching.get())
        );
        assert_eq!(state.checkpoint_profile_fingerprint(), matching);

        for foreign in profiles
            .into_iter()
            .filter(|foreign| *foreign != profile)
            .map(CommandProfile::fingerprint)
        {
            let format_error = state
                .validate_format_profile(foreign)
                .expect_err("foreign format profile must be rejected");
            assert_eq!(format_error.boundary(), CommandProfileBoundary::Format);
            assert_eq!(format_error.expected(), matching);
            assert_eq!(format_error.found(), foreign);

            let checkpoint_error = state
                .validate_checkpoint_profile(foreign)
                .expect_err("foreign checkpoint profile must be rejected");
            assert_eq!(
                checkpoint_error.boundary(),
                CommandProfileBoundary::Checkpoint
            );
            assert_eq!(checkpoint_error.expected(), matching);
            assert_eq!(checkpoint_error.found(), foreign);
        }
    }
}

#[test]
fn nested_input_rollback_retains_the_resulting_typed_condition_stack() {
    let mut state = CommandState::default();
    state.conditions.push(ConditionalKind::IfCase, 419);
    let snapshot = state.snapshot();
    state.conditions.push(ConditionalKind::If, 442);

    state
        .rollback_nested_input_preserving_conditions(snapshot)
        .expect("nested input rollback preserves profile");

    assert_eq!(state.conditions.frames.len(), 2);
    assert_eq!(state.conditions.frames[0].kind, ConditionalKind::IfCase);
    assert_eq!(state.conditions.frames[0].source_line, 419);
    assert_eq!(state.conditions.frames[1].kind, ConditionalKind::If);
    assert_eq!(state.conditions.frames[1].source_line, 442);
}
