use super::{
    AlignState, AlignmentKind, AlignmentPackSpec, DisplayEqNo, DisplayInterrupt, EqNoSide,
    IncompleteFraction, Mode, ModeNest,
};
use std::sync::Arc;
use tex_state::Universe;
use tex_state::ids::{FontId, GlueId, NodeListId};
use tex_state::math::FractionThickness;
use tex_state::node::{KernKind, Node};
use tex_state::scaled::Scaled;
use tex_state::token::OriginId;

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
    nest.current_list_mutation().push(kern(1));
    let summary = nest.summary();

    assert!(Arc::ptr_eq(&nest.levels, &summary.levels));
    let shared_nodes = Arc::clone(&summary.levels.last().expect("horizontal level").list.nodes);

    let mut restored = ModeNest::from_summary(summary.clone()).expect("restore mode nest");
    assert!(Arc::ptr_eq(&restored.levels, &summary.levels));
    restored.current_list_mutation().push(kern(2));

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
fn preexisting_node_write_barriers_apply_scoped_mutations() {
    let mut nest = ModeNest::new();
    nest.current_list_mutation().push(kern(11));
    nest.current_list_mutation()
        .with_node_mut(0, |node| {
            let Node::Kern { amount, .. } = node else {
                panic!("fixture node must be a kern");
            };
            *amount = Scaled::from_raw(17);
        })
        .expect("fixture node");
    nest.current_list_mutation()
        .with_last_node_mut(|node| {
            let Node::Kern { amount, .. } = node else {
                panic!("fixture tail must be a kern");
            };
            *amount = Scaled::from_raw(23);
        })
        .expect("fixture tail");

    let Node::Kern { amount, .. } = &nest.current_list().nodes()[0] else {
        panic!("fixture node must remain a kern");
    };
    assert_eq!(*amount, Scaled::from_raw(23));
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
    first.current_list_mutation().push(kern(11));
    let mut equal = ModeNest::new();
    equal.push(Mode::Horizontal);
    equal.current_list_mutation().push(kern(11));
    let mut changed = ModeNest::new();
    changed.push(Mode::Horizontal);
    changed.current_list_mutation().push(kern(12));

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
    nest.current_list_mutation().set_prev_graf(7);
    nest.current_list_mutation().push(kern(11));

    for mode in [Mode::Horizontal, Mode::Math, Mode::InternalVertical] {
        nest.push(mode);
        assert_eq!(nest.current_mode(), mode);
        assert!(nest.current_list().is_empty());
    }
    nest.current_list_mutation()
        .set_prev_depth(Scaled::from_raw(23));
    nest.current_list_mutation().push(kern(29));

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

fn align_state() -> AlignState {
    AlignState::new(
        AlignmentKind::HAlign,
        AlignmentPackSpec::Natural,
        Vec::new(),
        vec![GlueId::ZERO],
        GlueId::ZERO,
        None,
    )
}

#[test]
fn journal_append_watermarks_restore_scalars_without_append_inverses() {
    let mut nest = ModeNest::new();
    nest.current_list_mutation().push(kern(1));
    let before = nest.summary();
    nest.enable_journal_for_test();
    let cursor = nest.begin_journal_for_test();

    {
        let mut list = nest.current_list_mutation();
        list.push(kern(2));
        list.append([kern(3), kern(4)]);
        list.push_reconstituted(None, kern(5), Some(kern(6)), Some(kern(7)));
        list.set_space_factor(777);
        list.set_no_boundary(true);
        list.set_hyphen_language(9);
        list.set_prev_depth(Scaled::from_raw(11));
        list.set_prev_graf(12);
        list.begin_pending_hchars(FontId::testing_new(2), 'x', OriginId::UNKNOWN, true);
        list.set_align_state(align_state());
        list.set_incomplete_fraction(IncompleteFraction {
            numerator: NodeListId::testing_epoch(3, 1),
            thickness: FractionThickness::Explicit(Scaled::from_raw(4)),
            left_delimiter: Some(5),
            right_delimiter: Some(6),
        });
        list.set_display_interrupt(DisplayInterrupt {
            active_directions: Vec::new(),
        });
        list.set_display_eq_no(DisplayEqNo {
            side: EqNoSide::Right,
            display: NodeListId::testing_epoch(7, 1),
        });
    }

    assert_eq!(nest.journal_inverse_len_for_test(), 0);
    nest.rollback_journal_for_test(cursor).expect("rollback");
    assert_eq!(nest.summary(), before);
}

#[test]
fn journal_destructive_node_reconstitution_alignment_and_transfers_restore() {
    let mut nest = ModeNest::new();
    nest.current_list_mutation()
        .append([kern(10), kern(20), kern(30)]);
    nest.current_list_mutation().set_align_state(align_state());
    let before = nest.summary();
    nest.enable_journal_for_test();
    let cursor = nest.begin_journal_for_test();

    nest.current_list_mutation()
        .with_node_mut(0, |node| *node = kern(11));
    nest.current_list_mutation()
        .with_last_node_mut(|node| *node = kern(31));
    nest.current_list_mutation()
        .with_reconstitution_target(|nodes| {
            nodes.remove(1);
            nodes.insert(0, kern(99));
        });
    nest.current_list_mutation()
        .push_reconstituted(Some((1, kern(88))), kern(77), None, None);
    nest.current_list_mutation().with_align_state_mut(|state| {
        state.start_cell(4, 3);
        state.finish_row();
    });
    let _ = nest.current_list_mutation().take_align_state();
    let _ = nest.current_list_mutation().pop_last_node();
    let _ = nest.current_list_mutation().take_nodes();

    assert!(nest.journal_inverse_len_for_test() >= 8);
    nest.rollback_journal_for_test(cursor).expect("rollback");
    assert_eq!(nest.summary(), before);
}

#[test]
fn journal_math_and_display_ownership_transfers_restore() {
    let mut nest = ModeNest::new();
    {
        let mut list = nest.current_list_mutation();
        list.set_incomplete_fraction(IncompleteFraction {
            numerator: NodeListId::testing_epoch(1, 2),
            thickness: FractionThickness::Default,
            left_delimiter: None,
            right_delimiter: Some(9),
        });
        list.set_display_interrupt(DisplayInterrupt {
            active_directions: vec![tex_state::node::Direction::BeginR],
        });
        list.set_display_eq_no(DisplayEqNo {
            side: EqNoSide::Left,
            display: NodeListId::testing_epoch(4, 2),
        });
    }
    let before = nest.summary();
    nest.enable_journal_for_test();
    let cursor = nest.begin_journal_for_test();
    {
        let mut list = nest.current_list_mutation();
        assert!(list.take_incomplete_fraction().is_some());
        assert!(list.take_display_interrupt().is_some());
        assert!(list.take_display_eq_no().is_some());
    }
    nest.rollback_journal_for_test(cursor).expect("rollback");
    assert_eq!(nest.summary(), before);

    let mut display = ModeNest::new();
    display
        .current_list_mutation()
        .set_display_alignment(vec![kern(7), kern(8)], Some(Scaled::from_raw(9)));
    let before = display.summary();
    display.enable_journal_for_test();
    let cursor = display.begin_journal_for_test();
    assert_eq!(
        display
            .current_list_mutation()
            .take_display_alignment()
            .expect("display alignment")
            .0,
        vec![kern(7), kern(8)]
    );
    display
        .rollback_journal_for_test(cursor)
        .expect("display rollback");
    assert_eq!(display.summary(), before);
}

#[test]
fn journal_nested_commit_and_rollback_compose() {
    let mut outer_rollback = ModeNest::new();
    outer_rollback.enable_journal_for_test();
    let outer = outer_rollback.begin_journal_for_test();
    outer_rollback.current_list_mutation().push(kern(1));
    let inner = outer_rollback.begin_journal_for_test();
    outer_rollback.current_list_mutation().push(kern(2));
    outer_rollback
        .commit_journal_for_test(inner)
        .expect("inner commit");
    outer_rollback
        .rollback_journal_for_test(outer)
        .expect("outer rollback");
    assert!(outer_rollback.current_list().is_empty());

    let mut outer_commit = ModeNest::new();
    outer_commit.enable_journal_for_test();
    let outer = outer_commit.begin_journal_for_test();
    outer_commit.current_list_mutation().push(kern(1));
    let inner = outer_commit.begin_journal_for_test();
    outer_commit.current_list_mutation().push(kern(2));
    outer_commit
        .rollback_journal_for_test(inner)
        .expect("inner rollback");
    outer_commit
        .commit_journal_for_test(outer)
        .expect("outer commit");
    assert_eq!(outer_commit.current_list().nodes(), &[kern(1)]);
}

#[test]
fn journal_level_identity_handles_push_pop_replacement_and_nested_edits() {
    let mut nest = ModeNest::new();
    nest.push(Mode::Horizontal);
    nest.current_list_mutation().push(kern(1));
    let before = nest.summary();
    nest.enable_journal_for_test();
    let outer = nest.begin_journal_for_test();

    let removed = nest.pop().expect("pop horizontal");
    nest.push(Mode::Math);
    nest.current_list_mutation().push(kern(2));
    let inner = nest.begin_journal_for_test();
    nest.push(Mode::InternalVertical);
    nest.current_list_mutation().push(kern(3));
    nest.rollback_journal_for_test(inner)
        .expect("inner rollback");
    assert_eq!(nest.current_mode(), Mode::Math);
    drop(removed);
    nest.rollback_journal_for_test(outer)
        .expect("outer rollback");
    assert_eq!(nest.summary(), before);
}

#[test]
fn journal_rejects_non_innermost_and_stale_generation_cursors() {
    use super::journal::CursorError;

    let mut nest = ModeNest::new();
    nest.enable_journal_for_test();
    let outer = nest.begin_journal_for_test();
    let inner = nest.begin_journal_for_test();
    assert_eq!(
        nest.commit_journal_for_test(outer),
        Err(CursorError::NotInnermost)
    );
    nest.rollback_journal_for_test(inner)
        .expect("inner rollback");
    nest.commit_journal_for_test(outer).expect("outer commit");

    nest.enable_journal_for_test();
    let current = nest.begin_journal_for_test();
    assert_eq!(
        nest.rollback_journal_for_test(outer),
        Err(CursorError::WrongGeneration)
    );
    nest.rollback_journal_for_test(current)
        .expect("current generation rollback");
}

#[test]
fn journal_fatal_commit_model_and_operational_invisibility_hold() {
    let mut nest = ModeNest::new();
    nest.enable_journal_for_test();
    let cursor = nest.begin_journal_for_test();
    let semantic_before = nest.clone();
    let debug_before = format!("{nest:?}");
    nest.current_list_mutation().push(kern(42));
    nest.commit_journal_for_test(cursor)
        .expect("fatal path commits partial semantic state");

    assert_ne!(nest, semantic_before);
    assert_eq!(format!("{semantic_before:?}"), debug_before);
    assert_eq!(nest.current_list().nodes(), &[kern(42)]);
    assert_eq!(nest.journal_inverse_len_for_test(), 0);
}
