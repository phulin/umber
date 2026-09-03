use super::OpenTypeShapingScratch;

#[test]
fn shaping_scratch_clear_preserves_warm_capacity_without_logical_contents() {
    let mut scratch = OpenTypeShapingScratch::default();
    scratch.text.push_str("mapped text");
    scratch.byte_starts.extend([0, 2, 5]);
    scratch.break_bytes.extend([2, 5]);
    scratch.cluster_advances.extend([(0, 10), (1, 20)]);
    scratch
        .adjustments
        .extend([tex_state::scaled::Scaled::from_raw(1); 3]);
    let capacities = (
        scratch.text.capacity(),
        scratch.byte_starts.capacity(),
        scratch.break_bytes.capacity(),
        scratch.cluster_advances.capacity(),
        scratch.adjustments.capacity(),
    );

    scratch.clear();

    assert!(scratch.text.is_empty());
    assert!(scratch.byte_starts.is_empty());
    assert!(scratch.break_bytes.is_empty());
    assert!(scratch.cluster_advances.is_empty());
    assert!(scratch.adjustments.is_empty());
    assert_eq!(
        capacities,
        (
            scratch.text.capacity(),
            scratch.byte_starts.capacity(),
            scratch.break_bytes.capacity(),
            scratch.cluster_advances.capacity(),
            scratch.adjustments.capacity(),
        )
    );
}
