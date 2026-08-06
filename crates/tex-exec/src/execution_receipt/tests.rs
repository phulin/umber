use tex_command::{
    CommandObservation, DiagnosticRecord, EffectRecord, GeometryRecord, MutationRecord,
    MutationTarget, ObservationEffectKind, ObservationValue,
};
use tex_state::{ContentHash, EffectRecord as WorldEffectRecord, PrintSink};

use super::execution_receipt::*;
use crate::ResourceNeed;

const LIMITS: ShadowLimits = ShadowLimits {
    max_steps: 4,
    max_state_bytes: 32,
    max_evidence_records: 8,
    max_receipt_records: 16,
};

fn mutation() -> MutationRecord {
    MutationRecord {
        target: MutationTarget::Register,
        key: ObservationValue::Integer(7),
        value: ObservationValue::Integer(11),
        global: false,
    }
}

fn semantic_effect() -> EffectRecord {
    EffectRecord {
        kind: ObservationEffectKind::Write,
        channel: "term_and_log".into(),
        value: ObservationValue::Name("receipt".into()),
        source: None,
    }
}

fn diagnostic() -> DiagnosticRecord {
    DiagnosticRecord {
        severity: "error",
        diagnostic: "shadow_test",
        arguments: Vec::new(),
    }
}

// These are deliberately independent producers. The temporary differential
// gate must never prove itself by passing the same function to both sides.
fn legacy_path(kind: OperationKind, capture: &mut ShadowCapture) {
    capture.record_state_bytes(&[1]);
    capture.record_state_bytes(&[2, 3]);
    capture.record_observation(CommandObservation::Mutation(mutation()));
    capture.record_observation(CommandObservation::Effect(semantic_effect()));
    capture.record_observation(CommandObservation::Diagnostic(diagnostic()));
    capture.record_world_effect(WorldEffectRecord::StreamWrite {
        sink: PrintSink::TerminalAndLog,
        text: "receipt".into(),
    });
    capture.record_artifact(ContentHash::from_bytes(b"artifact"));
    if kind == OperationKind::Alignment {
        capture.set_termination(OperationTermination::Continue);
    }
}

fn replacement_path(kind: OperationKind, capture: &mut ShadowCapture) {
    capture.record_state_bytes(&[1, 2]);
    capture.record_state_bytes(&[3]);
    capture.record_observation(CommandObservation::Mutation(MutationRecord {
        target: MutationTarget::Register,
        key: ObservationValue::Integer(7),
        value: ObservationValue::Integer(11),
        global: false,
    }));
    capture.record_observation(CommandObservation::Effect(EffectRecord {
        kind: ObservationEffectKind::Write,
        channel: String::from("term_and_log"),
        value: ObservationValue::Name(String::from("receipt")),
        source: None,
    }));
    capture.record_observation(CommandObservation::Diagnostic(DiagnosticRecord {
        severity: "error",
        diagnostic: "shadow_test",
        arguments: vec![],
    }));
    capture.record_world_effect(WorldEffectRecord::StreamWrite {
        sink: PrintSink::TerminalAndLog,
        text: String::from("receipt"),
    });
    capture.record_artifact(ContentHash::from_bytes(&[
        97, 114, 116, 105, 102, 97, 99, 116,
    ]));
    match kind {
        OperationKind::Ordinary
        | OperationKind::Observed
        | OperationKind::Nested
        | OperationKind::Alignment => capture.set_termination(OperationTermination::Continue),
    }
}

#[test]
fn receipt_covers_every_commit_domain() {
    let mut capture = ShadowCapture::new(OperationKind::Ordinary, LIMITS);
    legacy_path(OperationKind::Ordinary, &mut capture);
    capture.record_resource(ResourceNeed::Input {
        name: "a.tex".into(),
        original_name: "a".into(),
    });
    capture.set_termination(OperationTermination::Suspended);
    let (result, _) = capture.finish().expect("bounded receipt");

    assert_eq!(result.receipt.mutations, [mutation()]);
    assert_eq!(result.receipt.resources.len(), 1);
    assert_eq!(result.receipt.effects.semantic, [semantic_effect()]);
    assert_eq!(result.receipt.effects.world.len(), 1);
    assert_eq!(result.receipt.artifacts.len(), 1);
    assert_eq!(result.receipt.diagnostics, [diagnostic()]);
    assert_eq!(result.receipt.termination, OperationTermination::Suspended);
}

#[test]
fn independent_ordinary_observed_nested_and_alignment_paths_compare_exactly() {
    let mut harness = ShadowDifferentialHarness::new(LIMITS);
    for kind in [
        OperationKind::Ordinary,
        OperationKind::Observed,
        OperationKind::Nested,
        OperationKind::Alignment,
    ] {
        let compared = harness
            .compare(
                kind,
                |capture| {
                    legacy_path(kind, capture);
                    Ok(())
                },
                |capture| {
                    replacement_path(kind, capture);
                    Ok(())
                },
            )
            .expect("independent equivalent shadows");
        assert_eq!(compared.receipt.operation, kind);
    }
}

#[test]
fn shadow_detects_exact_state_divergence() {
    let mut harness = ShadowDifferentialHarness::new(LIMITS);
    let error = harness
        .compare(
            OperationKind::Ordinary,
            |capture| {
                legacy_path(OperationKind::Ordinary, capture);
                Ok(())
            },
            |capture| {
                replacement_path(OperationKind::Ordinary, capture);
                capture.record_state_bytes(&[4]);
                Ok(())
            },
        )
        .expect_err("state mismatch");
    assert_eq!(
        error,
        ShadowError::Divergence {
            operation: OperationKind::Ordinary,
            component: ShadowComponent::State,
        }
    );
}

#[test]
fn shadow_detects_ordered_evidence_divergence() {
    let mut harness = ShadowDifferentialHarness::new(LIMITS);
    let error = harness
        .compare(
            OperationKind::Observed,
            |capture| {
                legacy_path(OperationKind::Observed, capture);
                Ok(())
            },
            |capture| {
                replacement_path(OperationKind::Observed, capture);
                capture.record_observation(CommandObservation::Geometry(GeometryRecord::Hpack {
                    width_sp: 1,
                    height_sp: 2,
                    depth_sp: 3,
                    line: 4,
                    source: None,
                }));
                Ok(())
            },
        )
        .expect_err("evidence mismatch");
    assert_eq!(
        error,
        ShadowError::Divergence {
            operation: OperationKind::Observed,
            component: ShadowComponent::Evidence,
        }
    );
}

#[test]
fn shadow_detects_receipt_divergence() {
    let mut harness = ShadowDifferentialHarness::new(LIMITS);
    let error = harness
        .compare(
            OperationKind::Nested,
            |capture| {
                legacy_path(OperationKind::Nested, capture);
                Ok(())
            },
            |capture| {
                replacement_path(OperationKind::Nested, capture);
                capture.set_termination(OperationTermination::End);
                Ok(())
            },
        )
        .expect_err("receipt mismatch");
    assert_eq!(
        error,
        ShadowError::Divergence {
            operation: OperationKind::Nested,
            component: ShadowComponent::Receipt,
        }
    );
}

#[test]
fn state_receipt_evidence_and_step_limits_are_append_time_hard() {
    let tiny = ShadowLimits {
        max_steps: 1,
        max_state_bytes: 2,
        max_evidence_records: 1,
        max_receipt_records: 1,
    };

    let mut state_capture = ShadowCapture::new(OperationKind::Ordinary, tiny);
    state_capture.record_state_bytes(&[1, 2, 3]);
    assert!(state_capture.retained_state_bytes() <= tiny.max_state_bytes);
    assert_eq!(
        state_capture.finish(),
        Err(ShadowLimitExceeded::StateBytes {
            limit: 2,
            actual: 3,
        })
    );

    let mut receipt_capture = ShadowCapture::new(OperationKind::Ordinary, tiny);
    for _ in 0..4_096 {
        receipt_capture.record_resource(ResourceNeed::Input {
            name: "a.tex".into(),
            original_name: "a".into(),
        });
    }
    assert!(receipt_capture.retained_receipt_records() <= tiny.max_receipt_records);
    assert!(matches!(
        receipt_capture.finish(),
        Err(ShadowLimitExceeded::ReceiptRecords { limit: 1, .. })
    ));

    let mut evidence_harness = ShadowDifferentialHarness::new(ShadowLimits {
        max_state_bytes: LIMITS.max_state_bytes,
        max_receipt_records: LIMITS.max_receipt_records,
        ..tiny
    });
    assert!(matches!(
        evidence_harness.compare(
            OperationKind::Alignment,
            |capture| {
                legacy_path(OperationKind::Alignment, capture);
                Ok(())
            },
            |capture| {
                replacement_path(OperationKind::Alignment, capture);
                Ok(())
            },
        ),
        Err(ShadowError::Limit {
            error: ShadowLimitExceeded::EvidenceRecords { limit: 1 },
            ..
        })
    ));

    let mut step_harness = ShadowDifferentialHarness::new(LIMITS);
    for _ in 0..LIMITS.max_steps {
        step_harness
            .compare(
                OperationKind::Ordinary,
                |capture| {
                    legacy_path(OperationKind::Ordinary, capture);
                    Ok(())
                },
                |capture| {
                    replacement_path(OperationKind::Ordinary, capture);
                    Ok(())
                },
            )
            .expect("inside step bound");
    }
    assert!(matches!(
        step_harness.compare(OperationKind::Ordinary, |_| Ok(()), |_| Ok(()),),
        Err(ShadowError::Limit {
            error: ShadowLimitExceeded::Steps { limit: 4 },
            ..
        })
    ));
}

#[test]
fn disabled_evidence_sink_is_allocation_free_and_empty() {
    let mut sink = EvidenceSink::disabled();
    assert!(sink.as_observer().is_none());
    assert_eq!(sink.into_evidence(), Ok(Vec::new()));
}
