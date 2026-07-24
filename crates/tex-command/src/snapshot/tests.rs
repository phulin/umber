use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::conditionals::ConditionFrame;
use crate::macro_call::MacroActivation;
use crate::processor::{ActiveCellDelivery, ScannerStatus, SuspendedAlignment};
use crate::profile::{CommandProfile, CommandProfileBoundary, CommandProfileFingerprint};
use crate::state::LiveTokenBuilder;
use crate::{CommandRuntime, CommandState};
use crate::{RegisteredSourceKind, SourceRegistration};

use super::{CommandSummary, CommandSummaryError};

fn semantic_hash<T: Hash>(value: &T) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

fn populated_quiescent_state() -> CommandState {
    let mut state = CommandState::new(CommandProfile::unicode_extended(
        crate::CommandDialect::Pdftex14027,
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
        definition: 17,
        invocation: 19,
    });
    state.conditions.frames.push(ConditionFrame {
        identity: 23,
        kind: 29,
        limit: 31,
        source_line: 37,
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
    state.scanner.status = ScannerStatus::Matching {
        macro_name: 73,
        builder: 79,
    };
    state.scanner.warning_identity = Some(83);
    state.transient.builders.push(LiveTokenBuilder {
        identity: 79,
        tokens: Vec::new(),
    });
    state.transient.rollback_roots.push(89);
    state.transient.active_expansion_depth = 2;
    state.alignment.active_cell = Some(ActiveCellDelivery {
        alignment: 97,
        u_template: 101,
        v_template: 103,
    });
    let expected = state.clone();
    let snapshot = state.snapshot();

    state = CommandState::new(expected.profile());
    let runtime = CommandRuntime::default();
    state.rollback(snapshot).expect("matching snapshot profile");

    assert_eq!(state, expected);
    drop(runtime);
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

fn assert_rejected(mutate: impl FnOnce(&mut CommandState), expected: CommandSummaryError) {
    let mut state = populated_quiescent_state();
    mutate(&mut state);
    assert_eq!(state.publish_summary(), Err(expected));
}

#[test]
fn summary_rejects_each_scanner_episode() {
    assert_rejected(
        |state| {
            state.scanner.status = ScannerStatus::Skipping { condition: 1 };
        },
        CommandSummaryError::ConditionalSkip,
    );
    assert_rejected(
        |state| {
            state.scanner.status = ScannerStatus::Matching {
                macro_name: 1,
                builder: 2,
            };
        },
        CommandSummaryError::MacroMatch,
    );
    assert_rejected(
        |state| {
            state.scanner.status = ScannerStatus::Defining {
                target: Some(1),
                builder: 2,
            };
        },
        CommandSummaryError::DefinitionScan,
    );
    assert_rejected(
        |state| {
            state.scanner.status = ScannerStatus::Aligning {
                alignment: 1,
                builder: 2,
            };
        },
        CommandSummaryError::AlignmentScan,
    );
    assert_rejected(
        |state| {
            state.scanner.status = ScannerStatus::Absorbing {
                owner: Some(1),
                builder: 2,
            };
        },
        CommandSummaryError::AbsorbingScan,
    );
    assert_rejected(
        |state| {
            state.scanner.warning_identity = Some(1);
        },
        CommandSummaryError::ScannerWarningContext,
    );
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
                alignment: 1,
                u_template: 2,
                v_template: 3,
            });
        },
        CommandSummaryError::AlignmentTemplateActive,
    );
    assert_rejected(
        |state| {
            state.alignment.suspended.push(SuspendedAlignment {
                alignment: 1,
                align_state: 2,
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
    let state = CommandState::new(CommandProfile::PDFTEX14027);
    let matching = CommandProfile::PDFTEX14027.fingerprint();
    let foreign = CommandProfile::TEX82.fingerprint();

    assert_eq!(state.validate_format_profile(matching), Ok(()));
    let format_error = state
        .validate_format_profile(foreign)
        .expect_err("foreign format profile must be rejected");
    assert_eq!(format_error.boundary(), CommandProfileBoundary::Format);

    assert_eq!(state.validate_checkpoint_profile(matching), Ok(()));
    let checkpoint_error = state
        .validate_checkpoint_profile(foreign)
        .expect_err("foreign checkpoint profile must be rejected");
    assert_eq!(
        checkpoint_error.boundary(),
        CommandProfileBoundary::Checkpoint
    );

    assert_eq!(
        state.format_profile_fingerprint(),
        CommandProfileFingerprint::from_u64(matching.get())
    );
    assert_eq!(state.checkpoint_profile_fingerprint(), matching);
}
