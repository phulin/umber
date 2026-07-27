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
fn pushing_a_shared_mode_nest_preserves_the_snapshot_root() {
    let mut nest = ModeNest::new();
    let summary = nest.summary();

    nest.push(Mode::Horizontal);

    assert!(!Arc::ptr_eq(&nest.levels, &summary.levels));
    assert_eq!(summary.levels.len(), 1);
    assert_eq!(nest.depth(), 2);
    assert_eq!(nest.current_mode(), Mode::Horizontal);
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

#[test]
fn semantic_nest_six_modes_and_fields_initialize_canonically() {
    for (mode, family, inner, horizontal_space_factor) in [
        (
            Mode::Vertical,
            tex_expand::EngineMode::Vertical,
            false,
            false,
        ),
        (
            Mode::InternalVertical,
            tex_expand::EngineMode::Vertical,
            true,
            false,
        ),
        (
            Mode::Horizontal,
            tex_expand::EngineMode::Horizontal,
            false,
            true,
        ),
        (
            Mode::RestrictedHorizontal,
            tex_expand::EngineMode::Horizontal,
            true,
            true,
        ),
        (Mode::Math, tex_expand::EngineMode::Math, true, false),
        (
            Mode::DisplayMath,
            tex_expand::EngineMode::Math,
            false,
            false,
        ),
    ] {
        let mut nest = ModeNest::new();
        nest.push(mode);
        let list = nest.current_list();

        assert_eq!(nest.current_mode(), mode);
        assert_eq!(mode.engine_mode(), family);
        assert_eq!(mode.is_inner(), inner);
        assert!(list.is_empty());
        assert_eq!(
            list.raw_space_factor(),
            if horizontal_space_factor { 1000 } else { 0 }
        );
        assert_eq!(list.prev_depth(), None);
        assert_eq!(list.prev_graf(), 0);
        assert!(!list.no_boundary());
        assert_eq!(list.hyphen_language(), 0);
        assert!(list.align_state().is_none());
        assert!(list.incomplete_fraction().is_none());
        assert!(list.display_interrupt().is_none());
        assert!(list.display_eq_no().is_none());
    }
}

#[test]
fn semantic_nest_push_and_pop_preserve_fields_and_start_empty_list() {
    let mut nest = ModeNest::new();
    nest.current_list_mut().set_prev_graf(7);
    nest.current_list_mut().push(kern(11));

    for mode in [Mode::Horizontal, Mode::Math, Mode::InternalVertical] {
        nest.push(mode);
        assert_eq!(nest.current_mode(), mode);
        assert!(nest.current_list().is_empty());
    }
    nest.current_list_mut().set_prev_depth(Scaled::from_raw(23));
    nest.current_list_mut().push(kern(29));

    let inner = nest.pop().expect("nested mode pops");
    assert_eq!(inner.mode(), Mode::InternalVertical);
    assert_eq!(inner.list().prev_depth(), Some(Scaled::from_raw(23)));
    assert_eq!(inner.list().nodes(), &[kern(29)]);
    assert_eq!(nest.current_mode(), Mode::Math);
    assert!(nest.current_list().is_empty());

    nest.pop().expect("math mode pops");
    nest.pop().expect("horizontal mode pops");
    assert_eq!(nest.current_mode(), Mode::Vertical);
    assert_eq!(nest.current_list().prev_graf(), 7);
    assert_eq!(nest.current_list().nodes(), &[kern(11)]);
    assert!(nest.pop().is_err());
}
