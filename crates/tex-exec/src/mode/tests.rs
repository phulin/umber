use super::{
    AlignState, AlignmentKind, AlignmentPackSpec, DisplayEqNo, DisplayInterrupt, EqNoSide,
    ExecError, IncompleteFraction, Mode, ModeLevelSummary, ModeNest,
};
use std::sync::Arc;
use tex_command::{ConditionalMode, FatalError};
use tex_state::Universe;
use tex_state::ids::FontId;
use tex_state::math::FractionThickness;
use tex_state::node::{KernKind, Node};
use tex_state::scaled::Scaled;

fn kern(value: i32) -> Node {
    Node::Kern {
        amount: Scaled::from_raw(value),
        kind: KernKind::Explicit,
    }
}

#[test]
fn nest_usage_records_tex82_pre_push_depth_and_survives_pop() {
    let mut nest = ModeNest::new();
    nest.push(Mode::Horizontal).expect("horizontal push");
    nest.push(Mode::Math).expect("math push");
    assert_eq!(nest.maximum_saved_depth(), 1);

    nest.pop().expect("math pop");
    nest.pop().expect("horizontal pop");
    assert_eq!(nest.maximum_saved_depth(), 1);
}

#[test]
fn mode_summary_restores_an_independent_semantic_builder() {
    let mut nest = ModeNest::new();
    nest.push(Mode::Horizontal).expect("test mode push");
    nest.current_list_mutation().push(kern(1));
    let summary = nest.summary();

    let snapshot_nodes = summary
        .levels
        .last()
        .expect("horizontal level")
        .list
        .sequence
        .semantic()
        .to_vec();

    let mut restored = ModeNest::from_summary(summary.clone()).expect("restore mode nest");
    restored.current_list_mutation().push(kern(2));

    let restored_nodes = restored
        .levels
        .last()
        .expect("horizontal level")
        .list
        .sequence
        .semantic();
    assert_eq!(snapshot_nodes.len(), 1);
    assert_eq!(
        summary
            .levels
            .last()
            .expect("horizontal level")
            .list
            .nodes()
            .len(),
        1
    );
    assert_eq!(restored_nodes.len(), 2);
}

#[test]
fn episode_boundary_freezes_builder_sidecars_and_mutation_invalidates_them() {
    let mut stores = Universe::new();
    let mut nest = ModeNest::new();
    nest.current_list_mutation().push(kern(11));
    assert!(nest.levels[0].list.sequence.frozen_sidecars().is_none());

    nest.publish_node_sidecars(&mut stores);
    let (semantic, physical) = nest.levels[0]
        .list
        .sequence
        .frozen_sidecars()
        .expect("boundary materializes both projections");
    let nodes = |root| {
        stores
            .page_node_list(root)
            .expect("mode sidecar belongs to the page arena")
            .nodes()
            .to_vec()
    };
    assert_eq!(nodes(semantic), vec![kern(11)]);
    assert_eq!(nodes(physical), vec![kern(11)]);

    nest.current_list_mutation().push(kern(13));
    assert!(nest.levels[0].list.sequence.frozen_sidecars().is_none());
    nest.publish_node_sidecars(&mut stores);
    let semantic = nest.levels[0]
        .list
        .sequence
        .frozen_sidecars()
        .expect("next boundary refreezes the changed builder")
        .0;
    assert_eq!(
        stores
            .page_node_list(semantic)
            .expect("mode sidecar belongs to the page arena")
            .nodes(),
        [kern(11), kern(13)]
    );
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
fn pushing_a_mode_nest_preserves_the_prior_summary() {
    let mut nest = ModeNest::new();
    let summary = nest.summary();

    nest.push(Mode::Horizontal).expect("test mode push");

    assert_eq!(summary.levels.len(), 1);
    assert_eq!(nest.depth(), 2);
    assert_eq!(nest.current_mode(), Mode::Horizontal);
}

#[test]
fn mode_projection_is_canonical_and_content_sensitive() {
    let mut first = ModeNest::new();
    first.push(Mode::Horizontal).expect("test mode push");
    first.current_list_mutation().push(kern(11));
    let mut equal = ModeNest::new();
    equal.push(Mode::Horizontal).expect("test mode push");
    equal.current_list_mutation().push(kern(11));
    let mut changed = ModeNest::new();
    changed.push(Mode::Horizontal).expect("test mode push");
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
        (Mode::Vertical, ConditionalMode::Vertical, false, false),
        (
            Mode::InternalVertical,
            ConditionalMode::Vertical,
            true,
            false,
        ),
        (Mode::Horizontal, ConditionalMode::Horizontal, false, true),
        (
            Mode::RestrictedHorizontal,
            ConditionalMode::Horizontal,
            true,
            true,
        ),
        (Mode::Math, ConditionalMode::Math, true, false),
        (Mode::DisplayMath, ConditionalMode::Math, false, false),
    ] {
        let mut nest = ModeNest::new();
        nest.push(mode).expect("test mode push");
        let list = nest.current_list();
        let conditional_state = nest.conditional_state();

        assert_eq!(nest.current_mode(), mode);
        assert_eq!(conditional_state.mode(), family);
        assert_eq!(conditional_state.is_inner(), inner);
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
        nest.push(mode).expect("test mode push");
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

#[test]
fn semantic_nest_capacity_and_bottom_pop_recovery_match_tex82() {
    let mut nest = ModeNest::new();
    nest.current_list_mutation().set_prev_graf(7);
    nest.current_list_mutation().push(kern(11));

    for _ in 0..ModeNest::TEX82_NEST_SIZE {
        nest.push(Mode::Horizontal)
            .expect("TeX82 permits nest_size saved semantic levels");
    }
    nest.current_list_mutation().push(kern(29));
    let full = nest.summary();

    let error = nest.push(Mode::Math).expect_err("nest_size overflow");
    assert!(matches!(
        &error,
        ExecError::Fatal(FatalError::CapacityExceeded {
            resource: "semantic nest size",
            amount: 40
        })
    ));
    assert_eq!(
        error.as_fatal(),
        Some(FatalError::CapacityExceeded {
            resource: "semantic nest size",
            amount: 40,
        })
    );
    assert_eq!(nest.summary(), full);

    while nest.depth() > 1 {
        nest.pop().expect("saved semantic level pops");
    }
    assert_eq!(nest.current_mode(), Mode::Vertical);
    assert_eq!(nest.current_list().prev_graf(), 7);
    assert_eq!(nest.current_list().nodes(), &[kern(11)]);
    assert!(matches!(nest.pop(), Err(ExecError::CannotPopBaseMode)));

    nest.push(Mode::Math)
        .expect("bottom-pop rejection leaves the nest reusable");
    assert_eq!(nest.current_mode(), Mode::Math);
}

#[test]
fn semantic_nest_capacity_rejection_does_not_record_a_journal_push() {
    let mut nest = ModeNest::new();
    for _ in 0..ModeNest::TEX82_NEST_SIZE {
        nest.push(Mode::Math).expect("push within TeX82 limit");
    }
    let full = nest.summary();
    nest.reset_journal_for_test();
    let cursor = nest.begin_journal();

    let error = nest.push(Mode::Horizontal).expect_err("nest_size overflow");
    assert_eq!(
        error.as_fatal(),
        Some(FatalError::CapacityExceeded {
            resource: "semantic nest size",
            amount: 40,
        })
    );
    assert_eq!(nest.journal_inverse_len_for_test(), 0);
    nest.rollback_journal(cursor).expect("empty rollback");
    assert_eq!(nest.summary(), full);
}

#[test]
fn semantic_nest_summary_cannot_bypass_tex82_capacity() {
    let mut nest = ModeNest::new();
    for _ in 0..ModeNest::TEX82_NEST_SIZE {
        nest.push(Mode::Math).expect("push within TeX82 limit");
    }
    let full = nest.summary();
    assert_eq!(
        ModeNest::from_summary(full.clone())
            .expect("maximum TeX82 nest summary restores")
            .summary(),
        full
    );

    let mut oversized = full.clone();
    Arc::make_mut(&mut oversized.levels).push(ModeLevelSummary::new(Mode::Math));
    let error = ModeNest::from_summary(oversized).expect_err("oversized mode summary");
    assert!(matches!(
        &error,
        ExecError::Fatal(FatalError::CapacityExceeded {
            resource: "semantic nest size",
            amount: 40
        })
    ));
    assert_eq!(
        error.as_fatal(),
        Some(FatalError::CapacityExceeded {
            resource: "semantic nest size",
            amount: 40,
        })
    );
    assert_eq!(nest.summary(), full);
}

fn align_state() -> AlignState {
    AlignState::new(
        AlignmentKind::HAlign,
        AlignmentPackSpec::Natural,
        Vec::new(),
        vec![tex_state::glue::GlueSpec::ZERO],
        tex_state::glue::GlueSpec::ZERO,
        None,
    )
}

#[test]
fn journal_append_watermarks_restore_scalars_without_append_inverses() {
    let mut nest = ModeNest::new();
    nest.current_list_mutation().push(kern(1));
    let before = nest.summary();
    nest.reset_journal_for_test();
    let cursor = nest.begin_journal();

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
        list.begin_pending_hchars(
            FontId::testing_new(2),
            'x',
            tex_state::provenance::OriginRef::unknown(),
        );
        list.set_align_state(align_state());
        list.set_incomplete_fraction(IncompleteFraction {
            numerator: tex_state::node_arena::PageListId::empty(),
            thickness: FractionThickness::Explicit(Scaled::from_raw(4)),
            left_delimiter: Some(5),
            right_delimiter: Some(6),
        });
        list.set_display_interrupt(DisplayInterrupt {
            active_directions: Vec::new(),
            prototype: None,
        });
        list.set_display_eq_no(DisplayEqNo {
            side: EqNoSide::Right,
            display: tex_state::node_arena::PageListId::empty(),
        });
    }

    assert_eq!(nest.journal_inverse_len_for_test(), 0);
    nest.rollback_journal(cursor).expect("rollback");
    assert_eq!(nest.summary(), before);
}

#[test]
fn journal_destructive_node_reconstitution_alignment_and_transfers_restore() {
    let mut nest = ModeNest::new();
    nest.current_list_mutation()
        .append([kern(10), kern(20), kern(30)]);
    nest.current_list_mutation().set_align_state(align_state());
    let before = nest.summary();
    nest.reset_journal_for_test();
    let cursor = nest.begin_journal();

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

    assert_eq!(nest.journal_inverse_len_for_test(), 6);
    nest.rollback_journal(cursor).expect("rollback");
    assert_eq!(nest.summary(), before);
}

#[test]
fn alignment_template_coordinates_survive_destructive_journal_rollback() {
    let u_template = tex_state::node::NodeTokenList::new([tex_state::token::TokenWord::pack(
        tex_state::token::Token::Char {
            ch: 'u',
            cat: tex_state::token::Catcode::Other,
        },
    )]);
    let v_template = tex_state::node::NodeTokenList::new([tex_state::token::TokenWord::pack(
        tex_state::token::Token::Char {
            ch: 'v',
            cat: tex_state::token::Catcode::Other,
        },
    )]);
    let mut nest = ModeNest::new();
    nest.current_list_mutation()
        .set_align_state(AlignState::new(
            AlignmentKind::HAlign,
            AlignmentPackSpec::Natural,
            vec![super::AlignColumn {
                u_template: u_template.clone(),
                v_template: v_template.clone(),
            }],
            vec![tex_state::glue::GlueSpec::ZERO],
            tex_state::glue::GlueSpec::ZERO,
            None,
        ));
    nest.reset_journal_for_test();
    let cursor = nest.begin_journal();

    let _ = nest.current_list_mutation().take_align_state();
    nest.rollback_journal(cursor).expect("alignment rollback");

    let column = &nest
        .current_list()
        .align_state()
        .expect("alignment restored")
        .columns()[0];
    assert_eq!(column.u_template, u_template);
    assert_eq!(column.v_template, v_template);
}

#[test]
fn journal_math_and_display_ownership_transfers_restore() {
    let mut nest = ModeNest::new();
    {
        let mut list = nest.current_list_mutation();
        list.set_incomplete_fraction(IncompleteFraction {
            numerator: tex_state::node_arena::PageListId::empty(),
            thickness: FractionThickness::Default,
            left_delimiter: None,
            right_delimiter: Some(9),
        });
        list.set_display_interrupt(DisplayInterrupt {
            active_directions: vec![tex_state::node::Direction::BeginR],
            prototype: None,
        });
        list.set_display_eq_no(DisplayEqNo {
            side: EqNoSide::Left,
            display: tex_state::node_arena::PageListId::empty(),
        });
    }
    let before = nest.summary();
    nest.reset_journal_for_test();
    let cursor = nest.begin_journal();
    {
        let mut list = nest.current_list_mutation();
        assert!(list.take_incomplete_fraction().is_some());
        assert!(list.take_display_interrupt().is_some());
        assert!(list.take_display_eq_no().is_some());
    }
    nest.rollback_journal(cursor).expect("rollback");
    assert_eq!(nest.summary(), before);

    let mut display = ModeNest::new();
    display
        .current_list_mutation()
        .set_display_alignment(vec![kern(7), kern(8)], Some(Scaled::from_raw(9)));
    let before = display.summary();
    display.reset_journal_for_test();
    let cursor = display.begin_journal();
    assert_eq!(
        display
            .current_list_mutation()
            .take_display_alignment()
            .expect("display alignment")
            .0,
        vec![kern(7), kern(8)]
    );
    display.rollback_journal(cursor).expect("display rollback");
    assert_eq!(display.summary(), before);
}

#[test]
fn journal_nested_commit_and_rollback_compose() {
    let mut outer_rollback = ModeNest::new();
    outer_rollback.reset_journal_for_test();
    let outer = outer_rollback.begin_journal();
    outer_rollback.current_list_mutation().push(kern(1));
    let inner = outer_rollback.begin_journal();
    outer_rollback.current_list_mutation().push(kern(2));
    outer_rollback.commit_journal(inner).expect("inner commit");
    outer_rollback
        .rollback_journal(outer)
        .expect("outer rollback");
    assert!(outer_rollback.current_list().is_empty());

    let mut outer_commit = ModeNest::new();
    outer_commit.reset_journal_for_test();
    let outer = outer_commit.begin_journal();
    outer_commit.current_list_mutation().push(kern(1));
    let inner = outer_commit.begin_journal();
    outer_commit.current_list_mutation().push(kern(2));
    outer_commit
        .rollback_journal(inner)
        .expect("inner rollback");
    outer_commit.commit_journal(outer).expect("outer commit");
    assert_eq!(outer_commit.current_list().nodes(), &[kern(1)]);
}

#[test]
fn journal_level_identity_handles_push_pop_replacement_and_nested_edits() {
    let mut nest = ModeNest::new();
    nest.push(Mode::Horizontal).expect("test mode push");
    nest.current_list_mutation().push(kern(1));
    let before = nest.summary();
    nest.reset_journal_for_test();
    let outer = nest.begin_journal();

    let removed = nest.pop().expect("pop horizontal");
    nest.push(Mode::Math).expect("test mode push");
    nest.current_list_mutation().push(kern(2));
    let inner = nest.begin_journal();
    nest.push(Mode::InternalVertical).expect("test mode push");
    nest.current_list_mutation().push(kern(3));
    nest.rollback_journal(inner).expect("inner rollback");
    assert_eq!(nest.current_mode(), Mode::Math);
    drop(removed);
    nest.rollback_journal(outer).expect("outer rollback");
    assert_eq!(nest.summary(), before);
}

#[test]
fn mode_entry_lines_survive_summary_restore_and_journal_rollback() {
    let mut nest = ModeNest::new();
    nest.push_at_line(Mode::Horizontal, 7)
        .expect("horizontal mode push");
    let saved = nest.summary();

    let mut restored = ModeNest::from_summary(saved.clone()).expect("restore mode summary");
    assert_eq!(restored.summary().levels()[1].entry_line(), 7);

    restored.reset_journal_for_test();
    let cursor = restored.begin_journal();
    restored.pop().expect("pop restored horizontal mode");
    restored
        .push_at_line(Mode::Math, 11)
        .expect("replacement math mode push");
    restored.rollback_journal(cursor).expect("rollback");

    assert_eq!(restored.summary(), saved);
    assert_eq!(restored.summary().levels()[1].entry_line(), 7);
}

#[test]
fn journal_rejects_a_non_innermost_cursor_without_state_drift() {
    use super::journal::CursorError;

    let mut nest = ModeNest::new();
    nest.reset_journal_for_test();
    let outer = nest.begin_journal();
    let inner = nest.begin_journal();
    assert_eq!(nest.commit_journal(outer), Err(CursorError::NotInnermost));
    nest.rollback_journal(inner).expect("inner rollback");
    nest.commit_journal(outer).expect("outer commit");
    assert_eq!(nest.summary(), ModeNest::new().summary());
}

#[test]
fn journal_fatal_commit_model_and_operational_invisibility_hold() {
    let mut nest = ModeNest::new();
    nest.reset_journal_for_test();
    let cursor = nest.begin_journal();
    let semantic_before = nest.clone();
    let debug_before = format!("{nest:?}");
    nest.current_list_mutation().push(kern(42));
    nest.commit_journal(cursor)
        .expect("fatal path commits partial semantic state");

    assert_ne!(nest, semantic_before);
    assert_eq!(format!("{semantic_before:?}"), debug_before);
    assert_eq!(nest.current_list().nodes(), &[kern(42)]);
    assert_eq!(nest.journal_inverse_len_for_test(), 0);
}
