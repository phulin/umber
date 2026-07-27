use super::*;

use tex_state::env::banks::{GlueParam, TokParam};
use tex_state::node::KernKind;
use tex_state::page::{PageBreak, PageContents, PageInsertion};
use tex_state::token::{Catcode, Token};

fn s(raw: i32) -> Scaled {
    Scaled::from_raw(raw)
}

fn rule(height: i32) -> Node {
    Node::Rule {
        width: Some(s(1)),
        height: Some(s(height)),
        depth: Some(s(0)),
    }
}

fn fire(best: usize, trigger: usize, size: i32) -> PageFireUp {
    PageFireUp::new(PageBreak::new(best), s(size), PageBreak::new(trigger))
}

fn boxed(stores: &mut Universe, vertical: bool) -> Node {
    let children = stores.freeze_node_list(&[]);
    let payload = BoxNode::new(BoxNodeFields {
        width: s(1),
        height: s(1),
        depth: s(0),
        shift: s(0),
        display: false,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children,
    });
    if vertical {
        Node::VList(payload)
    } else {
        Node::HList(payload)
    }
}

fn insertion(stores: &mut Universe, class: u16, nodes: &[Node]) -> Node {
    let content = stores.freeze_node_list(nodes);
    Node::Ins {
        class,
        size: s(2),
        split_top_skip: stores.glue_param(GlueParam::SPLIT_TOP_SKIP),
        split_max_depth: Scaled::MAX_DIMEN,
        floating_penalty: 13,
        content,
    }
}

fn nonempty_tokens(stores: &mut Universe) -> tex_state::ids::TokenListId {
    stores.intern_token_list(&[Token::Char {
        ch: 'x',
        cat: Catcode::Letter,
    }])
}

fn effects(stores: &Universe) -> String {
    format!("{:?}", stores.world().effect_records())
}

fn push_simple_page(stores: &mut Universe) -> PageFireUp {
    stores.push_current_page_node(rule(5));
    fire(1, 1, 5)
}

#[test]
fn fire_up_records_penalty_marks_boundary_and_packages_box255() {
    let mut stores = Universe::new();
    let old_bot = nonempty_tokens(&mut stores);
    let first = stores.intern_token_list(&[Token::Char {
        ch: 'a',
        cat: Catcode::Letter,
    }]);
    let last = stores.intern_token_list(&[Token::Char {
        ch: 'b',
        cat: Catcode::Letter,
    }]);
    stores.set_page_mark_class(PageMark::Bot, 0, old_bot);
    stores.push_current_page_node(Node::Mark {
        class: 0,
        tokens: first,
    });
    stores.push_current_page_node(rule(5));
    stores.push_current_page_node(Node::Mark {
        class: 0,
        tokens: last,
    });
    stores.push_current_page_node(Node::Penalty(-51));

    prepare_box255(&mut stores, fire(3, 3, 5)).expect("white-box operation succeeds");

    assert_eq!(stores.int_param(IntParam::OUTPUT_PENALTY), -51);
    assert_eq!(stores.page_mark_class(PageMark::Top, 0), old_bot);
    assert_eq!(stores.page_mark_class(PageMark::First, 0), first);
    assert_eq!(stores.page_mark_class(PageMark::Bot, 0), last);
    assert!(matches!(
        stores
            .nodes(stores.box_reg(255).expect("white-box operation succeeds"))
            .first(),
        Some(tex_state::node_arena::NodeRef::VList(_))
    ));
    assert_eq!(
        stores.page_contribution_front(),
        Some(&Node::Penalty(INF_PENALTY))
    );
}

#[test]
fn fire_up_nonvoid_box255_recovery_discards_before_packaging() {
    let mut stores = Universe::new();
    let old = boxed(&mut stores, false);
    let old = stores.freeze_node_list(&[old]);
    stores.set_box_reg(255, old);
    let fire = push_simple_page(&mut stores);

    prepare_box255(&mut stores, fire).expect("white-box operation succeeds");

    assert!(matches!(
        stores
            .nodes(stores.box_reg(255).expect("white-box operation succeeds"))
            .first(),
        Some(tex_state::node_arena::NodeRef::VList(_))
    ));
    assert!(effects(&stores).contains("box255 is not void"));
}

#[test]
fn fire_up_distributes_selected_insertions_into_class_boxes() {
    let mut stores = Universe::new();
    let node = insertion(&mut stores, 3, &[rule(2)]);
    let mut record = PageInsertion::new(3, s(0));
    record.set_last_ins_index(Some(0));
    stores.upsert_page_insertion(record);
    stores.record_best_page_break(1, s(2), 0);

    let distributed =
        distribute_insertions(&mut stores, vec![node]).expect("white-box operation succeeds");

    assert!(distributed.page_nodes.is_empty());
    assert!(distributed.heldover.is_empty());
    let class_box = stores.box_reg(3).expect("class box packaged");
    assert!(matches!(
        stores.nodes(class_box).first(),
        Some(tex_state::node_arena::NodeRef::VList(_))
    ));
}

#[test]
fn fire_up_split_remainder_heldover_and_holding_inserts_boundaries_match_tex82() {
    let mut stores = Universe::new();
    let node = insertion(&mut stores, 4, &[rule(2), rule(3)]);
    let mut record = PageInsertion::new(4, s(0));
    record.set_status(PageInsertionStatus::SplitUp {
        broken_ins_index: 0,
        broken_at: Some(1),
    });
    record.set_last_ins_index(Some(0));
    stores.upsert_page_insertion(record);
    stores.record_best_page_break(1, s(2), 0);

    let distributed = distribute_insertions(&mut stores, vec![node.clone()])
        .expect("white-box operation succeeds");
    assert!(distributed.page_nodes.is_empty());
    assert_eq!(distributed.heldover_count, 1);
    assert!(matches!(
        distributed.heldover.as_slice(),
        [Node::Ins { class: 4, .. }]
    ));

    stores.set_int_param(IntParam::HOLDING_INSERTS, 1);
    let held =
        distribute_insertions(&mut stores, vec![node]).expect("white-box operation succeeds");
    assert!(matches!(
        held.page_nodes.as_slice(),
        [Node::Ins { class: 4, .. }]
    ));
    assert_eq!(held.heldover_count, 0);
}

#[test]
fn output_selection_enters_one_user_output_group_below_deadcycle_limit() {
    let mut stores = Universe::new();
    let output = nonempty_tokens(&mut stores);
    stores.set_tok_param(TokParam::OUTPUT, output);
    stores.set_int_param(IntParam::MAX_DEAD_CYCLES, 3);
    let fire = push_simple_page(&mut stores);

    assert!(matches!(
        select_pending_page_output(&mut stores, fire).expect("white-box operation succeeds"),
        SelectedPageOutput::UserRoutine
    ));
    assert_eq!(stores.page_integer(PageInteger::DeadCycles), 1);
    assert!(stores.box_reg(255).is_some());
}

#[test]
fn output_default_path_prepends_heldovers_ships_and_voids_box255() {
    let mut stores = Universe::new();
    let fire = push_simple_page(&mut stores);

    let selected =
        select_pending_page_output(&mut stores, fire).expect("white-box operation succeeds");

    assert!(matches!(
        selected,
        SelectedPageOutput::Default(Node::VList(_))
    ));
    assert!(stores.box_reg(255).is_none());
    assert_eq!(stores.page_contents(), PageContents::Empty);
    assert_eq!(stores.current_page_len(), 0);
}

#[test]
fn output_deadcycle_limit_reports_and_uses_default_path() {
    let mut stores = Universe::new();
    let output = nonempty_tokens(&mut stores);
    stores.set_tok_param(TokParam::OUTPUT, output);
    stores.set_int_param(IntParam::MAX_DEAD_CYCLES, 0);
    let fire = push_simple_page(&mut stores);

    assert!(matches!(
        select_pending_page_output(&mut stores, fire).expect("white-box operation succeeds"),
        SelectedPageOutput::Default(Node::VList(_))
    ));
    assert!(stores.box_reg(255).is_none());
    assert!(effects(&stores).contains("Output loop---0 consecutive dead cycles"));
}

#[test]
fn output_resume_tears_down_group_mode_and_page_state_canonically() {
    let mut stores = Universe::new();
    stores.freeze_page_specs(PageContents::BoxThere);
    stores.set_page_dimension(PageDimension::Total, s(91));
    stores.set_page_integer(PageInteger::InsertPenalties, 7);
    stores.push_current_page_node(Node::Kern {
        amount: s(1),
        kind: KernKind::Explicit,
    });

    prepend_output_heldover(&mut stores, vec![]);

    assert_eq!(stores.page_contents(), PageContents::Empty);
    assert_eq!(stores.current_page_len(), 0);
    assert_eq!(stores.page_dimension(PageDimension::Total), s(0));
    assert_eq!(stores.insert_penalties(), 0);
    assert!(
        matches!(stores.page_contribution_front(), Some(Node::Kern { amount, .. }) if *amount == s(1))
    );
}

#[test]
fn output_resume_orders_output_material_heldovers_and_contributions() {
    let mut stores = Universe::new();
    stores.push_current_page_node(Node::Penalty(1));
    stores.append_page_contribution(Node::Penalty(3));

    prepend_output_heldover(&mut stores, vec![Node::Penalty(2)]);

    assert_eq!(
        stores
            .page_contributions()
            .iter()
            .cloned()
            .collect::<Vec<_>>(),
        [Node::Penalty(1), Node::Penalty(2), Node::Penalty(3)]
    );
}

#[test]
fn output_resume_recovers_unbalanced_tokens_and_nonvoid_box255() {
    let mut stores = Universe::new();
    let box255 = boxed(&mut stores, true);
    let box255 = stores.freeze_node_list(&[box255]);
    stores.set_box_reg(255, box255);
    stores.push_page_discard(Node::Penalty(9));
    let tokens = nonempty_tokens(&mut stores);

    resume_page_builder_after_output(&mut stores, vec![Node::Mark { class: 12, tokens }])
        .expect("white-box operation succeeds");

    assert!(stores.box_reg(255).is_none());
    assert!(stores.page_discards().is_empty());
    assert!(stores.page_contributions().is_empty());
    assert!(matches!(
        stores.current_page_nodes().as_slice(),
        [Node::Mark { class: 12, .. }]
    ));
    assert!(effects(&stores).contains("Output routine didn't use all of"));
}
