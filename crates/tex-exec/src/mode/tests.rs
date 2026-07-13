use super::{Mode, ModeNest};
use std::sync::Arc;
use tex_state::node::{KernKind, Node};
use tex_state::scaled::Scaled;

fn kern(value: i32) -> Node {
    Node::Kern {
        amount: Scaled::from_raw(value),
        kind: KernKind::Explicit,
    }
}

#[test]
fn mode_summary_shares_roots_and_restored_mutation_detaches() {
    let mut nest = ModeNest::new();
    nest.push(Mode::Horizontal);
    nest.current_list_mut().push(kern(1));
    let summary = nest.summary();

    assert!(Arc::ptr_eq(&nest.levels, &summary.levels));
    let shared_nodes = Arc::clone(&summary.levels.last().expect("horizontal level").list.nodes);

    let mut restored = ModeNest::from_summary(summary.clone()).expect("restore mode nest");
    assert!(Arc::ptr_eq(&restored.levels, &summary.levels));
    restored.current_list_mut().push(kern(2));

    assert!(!Arc::ptr_eq(&restored.levels, &summary.levels));
    let restored_nodes = &restored.levels.last().expect("horizontal level").list.nodes;
    assert!(!Arc::ptr_eq(restored_nodes, &shared_nodes));
    assert_eq!(
        summary
            .levels
            .last()
            .expect("horizontal level")
            .list
            .nodes
            .len(),
        1
    );
    assert_eq!(restored_nodes.len(), 2);
}
