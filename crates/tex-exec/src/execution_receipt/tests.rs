use tex_command::{
    CommandObservation, DiagnosticRecord, EffectRecord, MutationRecord, MutationTarget,
    ObservationEffectKind, ObservationValue,
};
use tex_state::{ContentHash, EffectRecord as WorldEffectRecord, PrintSink};

use super::*;

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

fn outcome(
    kind: OperationKind,
    sink: &mut EvidenceSink,
) -> Result<ShadowOutcome, ShadowLimitExceeded> {
    let observations = [
        CommandObservation::Mutation(mutation()),
        CommandObservation::Effect(semantic_effect()),
        CommandObservation::Diagnostic(diagnostic()),
    ];
    let mut receipt = ExecutionReceipt::new(kind, OperationTermination::Continue);
    for observation in observations {
        receipt.capture_observation(&observation);
        if let Some(observer) = sink.as_observer() {
            observer.committed(observation);
        }
    }
    receipt.effects.world.push(WorldEffectRecord::StreamWrite {
        sink: PrintSink::TerminalAndLog,
        text: "receipt".into(),
    });
    receipt.artifacts.push(ContentHash::from_bytes(b"artifact"));
    Ok(ShadowOutcome {
        state: ExactStateEvidence::new(vec![1, 2, 3], LIMITS.max_state_bytes)?,
        receipt,
    })
}

#[test]
fn receipt_covers_every_commit_domain() {
    let mut sink = EvidenceSink::disabled();
    let mut result = outcome(OperationKind::Ordinary, &mut sink).expect("bounded receipt");
    result.receipt.resources.push(ResourceNeed::Input {
        name: "a.tex".into(),
        original_name: "a".into(),
    });
    result.receipt.termination = OperationTermination::Suspended;

    assert_eq!(result.receipt.mutations, [mutation()]);
    assert_eq!(result.receipt.resources.len(), 1);
    assert_eq!(result.receipt.effects.semantic, [semantic_effect()]);
    assert_eq!(result.receipt.effects.world.len(), 1);
    assert_eq!(result.receipt.artifacts.len(), 1);
    assert_eq!(result.receipt.diagnostics, [diagnostic()]);
    assert_eq!(result.receipt.termination, OperationTermination::Suspended);
}

#[test]
fn ordinary_observed_nested_and_alignment_paths_compare_exactly() {
    let mut harness = ShadowDifferentialHarness::new(LIMITS);
    for kind in [
        OperationKind::Ordinary,
        OperationKind::Observed,
        OperationKind::Nested,
        OperationKind::Alignment,
    ] {
        let compared = harness
            .compare(kind, |sink| outcome(kind, sink), |sink| outcome(kind, sink))
            .expect("identical shadows");
        assert_eq!(compared.receipt.operation, kind);
    }
}

#[test]
fn shadow_detects_exact_state_divergence() {
    let mut harness = ShadowDifferentialHarness::new(LIMITS);
    let error = harness
        .compare(
            OperationKind::Ordinary,
            |sink| outcome(OperationKind::Ordinary, sink),
            |sink| {
                let mut result = outcome(OperationKind::Ordinary, sink)?;
                result.state = ExactStateEvidence::new(vec![1, 2, 4], LIMITS.max_state_bytes)?;
                Ok(result)
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
            |sink| outcome(OperationKind::Observed, sink),
            |sink| {
                let result = outcome(OperationKind::Observed, sink)?;
                if let Some(observer) = sink.as_observer() {
                    observer.committed(CommandObservation::Diagnostic(diagnostic()));
                }
                Ok(result)
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
            |sink| outcome(OperationKind::Nested, sink),
            |sink| {
                let mut result = outcome(OperationKind::Nested, sink)?;
                result.receipt.termination = OperationTermination::End;
                Ok(result)
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
fn evidence_state_receipt_and_step_limits_are_hard() {
    let tiny = ShadowLimits {
        max_steps: 1,
        max_state_bytes: 2,
        max_evidence_records: 1,
        max_receipt_records: 1,
    };
    assert_eq!(
        ExactStateEvidence::new(vec![1, 2, 3], tiny.max_state_bytes),
        Err(ShadowLimitExceeded::StateBytes {
            limit: 2,
            actual: 3,
        })
    );

    let mut evidence_harness = ShadowDifferentialHarness::new(ShadowLimits {
        max_state_bytes: LIMITS.max_state_bytes,
        max_receipt_records: LIMITS.max_receipt_records,
        ..tiny
    });
    assert!(matches!(
        evidence_harness.compare(
            OperationKind::Alignment,
            |sink| outcome(OperationKind::Alignment, sink),
            |sink| outcome(OperationKind::Alignment, sink),
        ),
        Err(ShadowError::Limit {
            error: ShadowLimitExceeded::EvidenceRecords { limit: 1 },
            ..
        })
    ));

    let mut receipt_harness = ShadowDifferentialHarness::new(ShadowLimits {
        max_state_bytes: LIMITS.max_state_bytes,
        max_evidence_records: LIMITS.max_evidence_records,
        ..tiny
    });
    assert!(matches!(
        receipt_harness.compare(
            OperationKind::Alignment,
            |sink| outcome(OperationKind::Alignment, sink),
            |sink| outcome(OperationKind::Alignment, sink),
        ),
        Err(ShadowError::Limit {
            error: ShadowLimitExceeded::ReceiptRecords { limit: 1, .. },
            ..
        })
    ));

    let mut step_harness = ShadowDifferentialHarness::new(LIMITS);
    for _ in 0..LIMITS.max_steps {
        step_harness
            .compare(
                OperationKind::Ordinary,
                |sink| outcome(OperationKind::Ordinary, sink),
                |sink| outcome(OperationKind::Ordinary, sink),
            )
            .expect("inside step bound");
    }
    assert!(matches!(
        step_harness.compare(
            OperationKind::Ordinary,
            |sink| outcome(OperationKind::Ordinary, sink),
            |sink| outcome(OperationKind::Ordinary, sink),
        ),
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
