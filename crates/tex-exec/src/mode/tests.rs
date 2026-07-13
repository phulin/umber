use super::{Mode, ModeNest};
use std::sync::Arc;
use tex_state::Universe;
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

#[test]
fn mode_projection_is_canonical_and_content_sensitive() {
    let mut first = ModeNest::new();
    first.push(Mode::Horizontal);
    first.current_list_mut().push(kern(11));
    let mut equal = ModeNest::new();
    equal.push(Mode::Horizontal);
    equal.current_list_mut().push(kern(11));
    let mut changed = ModeNest::new();
    changed.push(Mode::Horizontal);
    changed.current_list_mut().push(kern(12));

    let first_hash = first.summary().semantic_fingerprint(&Universe::new());
    assert_eq!(
        equal.summary().semantic_fingerprint(&Universe::new()),
        first_hash
    );
    assert_ne!(
        changed.summary().semantic_fingerprint(&Universe::new()),
        first_hash
    );
}
