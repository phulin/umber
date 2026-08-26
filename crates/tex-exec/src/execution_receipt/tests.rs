use super::{ExecutionReceipt, OperationTermination};

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
