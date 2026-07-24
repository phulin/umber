use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::conditionals::ConditionFrame;
use crate::input::InputLevel;
use crate::macro_call::MacroActivation;
use crate::processor::{ActiveCellDelivery, ScannerStatus, SuspendedAlignment};
use crate::profile::{CharacterMode, CommandDialect};
use crate::state::LiveTokenBuilder;
use crate::{CommandRuntime, CommandState};

use super::{CommandSummary, CommandSummaryError};

fn semantic_hash<T: Hash>(value: &T) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

fn populated_quiescent_state() -> CommandState {
    let mut state = CommandState::default();
    state.input.levels.push(InputLevel { identity: 7 });
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
    state.expansion.profile.dialect = CommandDialect::Pdftex14027;
    state.expansion.profile.characters = CharacterMode::UnicodeExtended;
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

    state = CommandState::default();
    let runtime = CommandRuntime::default();
    state.rollback(snapshot);

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

    let mut restored = CommandState::default();
    restored.restore_summary(summary);
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
