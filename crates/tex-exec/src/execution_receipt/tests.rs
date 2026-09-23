use super::{ExecutionReceipt, OperationTermination};
use tex_command::{
    DiagnosticRecord, EffectRecord, MutationRecord, MutationTarget, ObservationEffectKind,
    ObservationValue, ResourceNeed,
};
use tex_state::{ContentHash, PrintSink};

#[test]
fn every_receipt_category_is_bounded_and_consumed() {
    let mutation = MutationRecord {
        target: MutationTarget::Register,
        key: ObservationValue::Name("count:0".into()),
        value: ObservationValue::Integer(7),
        global: false,
    };
    let diagnostic = DiagnosticRecord {
        severity: "error",
        diagnostic: "probe",
        arguments: Vec::new(),
    };
    let effect = EffectRecord {
        kind: ObservationEffectKind::Message,
        channel: "terminal".into(),
        value: ObservationValue::Name("probe".into()),
        source: None,
    };
    let resource = ResourceNeed::Input {
        name: "probe.tex".into(),
        original_name: "probe".into(),
    };
    let world_effect = tex_state::EffectRecord::StreamWrite {
        sink: PrintSink::Terminal,
        text: "probe".into(),
    };
    let artifact = ContentHash::from_bytes(b"probe");
    let mut receipt = ExecutionReceipt {
        limit: 7,
        ..ExecutionReceipt::default()
    };

    assert!(receipt.push_mutation(mutation.clone()));
    assert!(receipt.push_diagnostic(diagnostic.clone()));
    assert!(receipt.push_semantic_effect(effect.clone()));
    assert!(receipt.record_resource(resource.clone()));
    assert!(receipt.record_world_effect(world_effect.clone()));
    assert!(receipt.record_artifact(artifact));
    receipt.set_termination(OperationTermination::End);
    assert_eq!(receipt.record_count(), receipt.limit());

    assert!(!receipt.push_mutation(mutation));
    assert!(!receipt.push_diagnostic(diagnostic));
    assert!(!receipt.push_semantic_effect(effect));
    assert!(!receipt.record_resource(resource));
    assert!(!receipt.record_world_effect(world_effect));
    assert!(!receipt.record_artifact(artifact));
    assert_eq!(receipt.mutations.len(), 1);
    assert_eq!(receipt.resources.len(), 1);
    assert_eq!(receipt.effects.semantic.len(), 1);
    assert_eq!(receipt.effects.world.len(), 1);
    assert_eq!(receipt.artifacts.len(), 1);
    assert_eq!(receipt.diagnostics.len(), 1);

    let terminal = receipt.clone().consume();
    let reusable = receipt.reset_for_next_operation();
    assert_eq!(terminal, reusable);
    assert_eq!(terminal.records, 7);
    assert_eq!(terminal.termination, OperationTermination::End);
    assert_eq!(receipt.record_count(), 1);
    assert_eq!(receipt.termination, OperationTermination::Continue);
}

#[test]
fn operation_reset_preserves_warmed_category_capacities() {
    let mut receipt = ExecutionReceipt::default();
    receipt.mutations.reserve(3);
    receipt.resources.reserve(5);
    receipt.effects.semantic.reserve(7);
    receipt.effects.world.reserve(11);
    receipt.artifacts.reserve(13);
    receipt.diagnostics.reserve(17);
    receipt.termination = OperationTermination::Failed;
    let capacities = (
        receipt.mutations.capacity(),
        receipt.resources.capacity(),
        receipt.effects.semantic.capacity(),
        receipt.effects.world.capacity(),
        receipt.artifacts.capacity(),
        receipt.diagnostics.capacity(),
    );

    let consumed = receipt.reset_for_next_operation();

    assert_eq!(consumed.records, 1);
    assert_eq!(consumed.termination, OperationTermination::Failed);
    assert_eq!(receipt.record_count(), 1);
    assert_eq!(receipt.termination, OperationTermination::Continue);
    assert_eq!(
        (
            receipt.mutations.capacity(),
            receipt.resources.capacity(),
            receipt.effects.semantic.capacity(),
            receipt.effects.world.capacity(),
            receipt.artifacts.capacity(),
            receipt.diagnostics.capacity(),
        ),
        capacities
    );
}
