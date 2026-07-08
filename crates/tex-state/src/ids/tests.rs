use super::{FontId, GlueId, NodeListId, SnapshotId, TokenListId};

#[test]
fn placeholder_ids_preserve_raw_values_inside_the_crate() {
    assert_eq!(TokenListId::new(1).raw(), 1);
    assert_eq!(GlueId::new(2).raw(), 2);
    let nodes = NodeListId::new_epoch(3, 4);
    assert_eq!(nodes.start(), 3);
    assert_eq!(nodes.len(), 4);
    assert_eq!(FontId::new(4).raw(), 4);
    assert_eq!(SnapshotId::new(5).raw(), 5);
}
