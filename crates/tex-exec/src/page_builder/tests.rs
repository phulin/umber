use super::*;

use tex_command::FatalError;
use tex_state::env::banks::{DimenParam, GlueParam, IntParam};
use tex_state::glue::Order;
use tex_state::ids::{FontId, GlueId};
use tex_state::math::{
    FractionThickness, MathChoice, MathField, MathFraction, MathListNode, MathNoad, MathStyle,
    NoadClass, NoadKind,
};
use tex_state::node::{
    BoxNode, BoxNodeFields, Direction, DiscKind, KernKind, Sign, UnsetKind, UnsetNode,
    UnsetNodeFields,
};
use tex_state::page::{PageBreak, PageInteger};
use tex_state::scaled::GlueSetRatio;
use tex_state::token::OriginId;

fn s(raw: i32) -> Scaled {
    Scaled::from_raw(raw)
}

fn glue(
    stores: &mut Universe,
    width: i32,
    stretch: i32,
    stretch_order: Order,
    shrink: i32,
    shrink_order: Order,
) -> GlueId {
    stores.intern_glue(GlueSpec {
        width: s(width),
        stretch: s(stretch),
        stretch_order,
        shrink: s(shrink),
        shrink_order,
    })
}

fn params(stores: &mut Universe, goal: i32, max_depth: i32, top_skip: i32) {
    stores.set_dimen_param(DimenParam::V_SIZE, s(goal));
    stores.set_dimen_param(DimenParam::MAX_DEPTH, s(max_depth));
    let top = glue(stores, top_skip, 0, Order::Normal, 0, Order::Normal);
    stores.set_glue_param(GlueParam::TOP_SKIP, top);
}

fn rule(height: i32, depth: i32) -> Node {
    Node::Rule {
        width: Some(s(1)),
        height: Some(s(height)),
        depth: Some(s(depth)),
    }
}

fn boxed(stores: &mut Universe, height: i32, depth: i32, vertical: bool) -> Node {
    let children = stores.freeze_node_list(&[]);
    let payload = BoxNode::new(BoxNodeFields {
        width: s(1),
        height: s(height),
        depth: s(depth),
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

fn ins(
    stores: &mut Universe,
    class: u16,
    size: i32,
    floating_penalty: i32,
    nodes: &[Node],
) -> Node {
    let content = stores.freeze_node_list(nodes);
    Node::Ins {
        class,
        size: s(size),
        split_top_skip: stores.glue_param(GlueParam::SPLIT_TOP_SKIP),
        split_max_depth: Scaled::MAX_DIMEN,
        floating_penalty,
        content,
    }
}

fn ins_class(stores: &mut Universe, class: u16, count: i32, max: i32, skip: i32, stretch: i32) {
    stores.set_count(class, count);
    stores.set_dimen(class, s(max));
    let skip = glue(stores, skip, stretch, Order::Normal, 0, Order::Normal);
    stores.set_skip(class, skip);
}

fn effects(stores: &Universe) -> String {
    format!("{:?}", stores.world().effect_records())
}

#[test]
fn page_state_freezes_specs_and_tracks_sorted_insertion_records() {
    let mut stores = Universe::new();
    params(&mut stores, 1_000, 17, 0);
    stores.freeze_page_specs(PageContents::InsertsOnly);
    assert_eq!(stores.page_dimension(PageDimension::Goal), s(1_000));
    assert_eq!(stores.page_max_depth(), s(17));
    stores.set_dimen_param(DimenParam::V_SIZE, s(2_000));
    stores.set_dimen_param(DimenParam::MAX_DEPTH, s(29));
    assert_eq!(stores.page_dimension(PageDimension::Goal), s(1_000));
    assert_eq!(stores.page_max_depth(), s(17));
    for class in [9, 3] {
        ins_class(&mut stores, class, 1_000, 100, 0, 0);
        let node = ins(&mut stores, class, 0, 0, &[]);
        prepare_insertion(&mut stores, &node).expect("white-box operation succeeds");
    }
    assert_eq!(
        stores
            .page_insertions()
            .iter()
            .map(PageInsertion::class)
            .collect::<Vec<_>>(),
        [3, 9]
    );
}

#[test]
fn page_builder_output_active_boundary_preserves_pending_contributions() {
    let mut stores = Universe::new();
    // TeX.web §§980--990: the frozen page specifications and all accumulated
    // quantities survive calls made while §989's output boundary is pending.
    params(&mut stores, 1_000, 19, 0);
    stores.freeze_page_specs(PageContents::BoxThere);
    for (dimension, value) in [
        (PageDimension::Total, 101),
        (PageDimension::Stretch, 102),
        (PageDimension::FilStretch, 103),
        (PageDimension::FillStretch, 104),
        (PageDimension::FilllStretch, 105),
        (PageDimension::Shrink, 106),
        (PageDimension::Depth, 107),
    ] {
        stores.set_page_dimension(dimension, s(value));
    }
    stores.set_page_integer(PageInteger::InsertPenalties, 11);
    stores.record_best_page_break(0, s(23), 0);
    stores.record_page_fire_up(0);
    stores.set_output_routine_active(true);
    stores.append_page_contribution(Node::Penalty(17));
    stores.append_page_contribution(rule(5, 2));
    build_page(&mut stores).expect("white-box operation succeeds");
    assert_eq!(
        stores
            .page_contributions()
            .iter()
            .cloned()
            .collect::<Vec<_>>(),
        [Node::Penalty(17), rule(5, 2)]
    );
    assert_eq!(stores.page_contents(), PageContents::BoxThere);
    assert_eq!(stores.page_dimension(PageDimension::Goal), s(1_000));
    assert_eq!(stores.page_max_depth(), s(19));
    for (dimension, value) in [
        (PageDimension::Total, 101),
        (PageDimension::Stretch, 102),
        (PageDimension::FilStretch, 103),
        (PageDimension::FillStretch, 104),
        (PageDimension::FilllStretch, 105),
        (PageDimension::Shrink, 106),
        (PageDimension::Depth, 107),
    ] {
        assert_eq!(stores.page_dimension(dimension), s(value));
    }
    assert_eq!(stores.insert_penalties(), 11);
    assert!(stores.output_routine_is_active());
    assert_eq!(
        stores
            .page_fire_up()
            .expect("white-box operation succeeds")
            .best_size(),
        s(23)
    );
}

#[test]
fn new_current_page_resets_nodes_totals_depth_and_last_item_state() {
    let mut stores = Universe::new();
    params(&mut stores, 1_000, 9, 0);
    stores.freeze_page_specs(PageContents::BoxThere);
    stores.push_current_page_node(Node::Penalty(41));
    for d in [
        PageDimension::Total,
        PageDimension::Stretch,
        PageDimension::FilStretch,
        PageDimension::FillStretch,
        PageDimension::FilllStretch,
        PageDimension::Shrink,
        PageDimension::Depth,
    ] {
        stores.set_page_dimension(d, s(7));
    }
    stores.set_page_integer(PageInteger::InsertPenalties, 12);
    stores.update_page_last_from_node(&Node::Kern {
        amount: s(33),
        kind: KernKind::Explicit,
    });
    stores.record_best_page_break(1, s(77), 4);
    stores.record_page_fire_up(1);
    stores.start_new_page();
    assert_eq!(stores.page_contents(), PageContents::Empty);
    assert_eq!(stores.current_page_len(), 0);
    assert_eq!(stores.page_max_depth(), s(0));
    assert_eq!(stores.insert_penalties(), 0);
    assert_eq!(stores.best_page_break(), None);
    assert_eq!(stores.page_fire_up(), None);
    assert!(!stores.page_has_last_glue());
    assert_eq!(stores.page_last_penalty(), 0);
    assert_eq!(stores.page_last_kern(), s(0));
    assert_eq!(
        stores.page_dimension(PageDimension::Goal),
        Scaled::MAX_DIMEN
    );
    for d in [
        PageDimension::Total,
        PageDimension::Stretch,
        PageDimension::FilStretch,
        PageDimension::FillStretch,
        PageDimension::FilllStretch,
        PageDimension::Shrink,
        PageDimension::Depth,
    ] {
        assert_eq!(stores.page_dimension(d), s(0));
    }
}

#[test]
fn box_error_and_ensure_vbox_recover_only_invalid_live_boxes() {
    let mut stores = crate::test_harness::universe();
    assert_eq!(
        insertion_box_size(&mut stores, 4).expect("white-box operation succeeds"),
        s(0)
    );
    let node = boxed(&mut stores, 11, 3, true);
    let list = stores.freeze_node_list(&[node]);
    stores.set_box_reg(4, list);
    assert_eq!(
        insertion_box_size(&mut stores, 4).expect("white-box operation succeeds"),
        s(14)
    );
    assert!(stores.box_reg(4).is_some());
    let node = boxed(&mut stores, 9, 2, false);
    let list = stores.freeze_node_list(&[node]);
    stores.set_box_reg(5, list);
    assert_eq!(
        insertion_box_size(&mut stores, 5).expect("white-box operation succeeds"),
        s(0)
    );
    assert!(stores.box_reg(5).is_none());
    assert!(effects(&stores).contains("Insertions can only be added to a vbox"));
}

#[test]
fn outer_vertical_contribution_routes_every_node_kind_canonically() {
    let mut stores = Universe::new();
    params(&mut stores, 10_000, 10, 10);
    stores.set_int_param(IntParam::SAVING_V_DISCARDS, 1);
    let leading = glue(&mut stores, 2, 0, Order::Normal, 0, Order::Normal);
    let mark = stores.intern_token_list(&[]);
    stores.append_page_contribution(Node::Glue {
        spec: leading,
        kind: GlueKind::Normal,
        leader: None,
    });
    stores.append_page_contribution(Node::Kern {
        amount: s(3),
        kind: KernKind::Explicit,
    });
    stores.append_page_contribution(Node::Penalty(4));
    stores.append_page_contribution(Node::Mark {
        class: 2,
        tokens: mark,
    });
    stores.append_page_contribution(rule(5, 1));
    stores.append_page_contribution(Node::Penalty(INF_PENALTY));
    build_page(&mut stores).expect("white-box operation succeeds");
    assert_eq!(stores.page_discards().len(), 3);
    assert!(matches!(
        stores.current_page_nodes()[0],
        Node::Mark { class: 2, .. }
    ));
    assert!(matches!(
        stores.current_page_nodes()[1],
        Node::Glue {
            kind: GlueKind::TopSkip,
            ..
        }
    ));
    assert!(matches!(stores.current_page_nodes()[2], Node::Rule { .. }));
    assert_eq!(stores.current_page_nodes()[3], Node::Penalty(INF_PENALTY));
    assert!(stores.page_contributions().is_empty());
}

#[test]
fn page_builder_rejects_impossible_contribution_nodes_with_page_confusion() {
    let mut handles = Universe::new();
    let empty = handles.freeze_node_list(&[]);
    let impossible = [
        Node::Char {
            font: FontId::testing_new(0),
            ch: 'x',
            origin: OriginId::UNKNOWN,
        },
        Node::Lig {
            font: FontId::testing_new(0),
            ch: 'x',
            orig: vec!['x'],
            origins: vec![OriginId::UNKNOWN],
        },
        Node::Unset(UnsetNode::new(UnsetNodeFields {
            kind: UnsetKind::VBox,
            width: s(1),
            height: s(2),
            depth: s(3),
            span_count: 0,
            stretch: s(0),
            stretch_order: Order::Normal,
            shrink: s(0),
            shrink_order: Order::Normal,
            children: empty,
        })),
        Node::Disc {
            kind: DiscKind::Discretionary,
            pre: empty,
            post: empty,
            replace: empty,
        },
        Node::MathOn(s(1)),
        Node::MathOff(s(1)),
        Node::Direction(Direction::BeginR),
        Node::MathNoad(MathNoad::new(
            NoadKind::Normal(NoadClass::Ord),
            MathField::Empty,
        )),
        Node::FractionNoad(MathFraction {
            numerator: empty,
            denominator: empty,
            thickness: FractionThickness::Default,
            left_delimiter: None,
            right_delimiter: None,
        }),
        Node::MathStyle(MathStyle::Text),
        Node::MathChoice(MathChoice {
            display: empty,
            text: empty,
            script: empty,
            script_script: empty,
        }),
        Node::MathList(MathListNode {
            display: false,
            content: empty,
        }),
        Node::Nonscript,
        Node::Adjust(tex_state::node::AdjustNode {
            content: empty,
            pre: false,
        }),
    ];

    for node in impossible {
        let mut stores = Universe::new();
        params(&mut stores, 1_000, 17, 0);
        stores.set_page_contents(PageContents::BoxThere);
        stores.set_page_dimension(PageDimension::Total, s(23));
        stores.set_page_dimension(PageDimension::Depth, s(5));
        stores.push_current_page_node(Node::Penalty(41));
        stores.update_page_last_from_node(&Node::Kern {
            amount: s(33),
            kind: KernKind::Explicit,
        });
        stores.append_page_contribution(node.clone());

        let error = build_page(&mut stores).expect_err("impossible page node must be fatal");

        assert_eq!(error.as_fatal(), Some(FatalError::confusion("page")));
        assert_eq!(stores.page_contributions().len(), 1);
        assert_eq!(stores.page_contribution_front(), Some(&node));
        assert_eq!(stores.current_page_nodes(), [Node::Penalty(41)]);
        assert_eq!(stores.page_contents(), PageContents::BoxThere);
        assert_eq!(stores.page_dimension(PageDimension::Total), s(23));
        assert_eq!(stores.page_dimension(PageDimension::Depth), s(5));
        assert_eq!(stores.page_last_kern(), s(33));
    }
}

#[test]
fn page_topskip_totals_depth_and_terminal_kern_boundaries_match_tex82() {
    let mut stores = Universe::new();
    params(&mut stores, 10_000, 3, 10);
    stores.append_page_contribution(rule(5, 4));
    stores.append_page_contribution(Node::Kern {
        amount: s(2),
        kind: KernKind::Explicit,
    });
    build_page(&mut stores).expect("white-box operation succeeds");
    assert_eq!(stores.page_dimension(PageDimension::Total), s(11));
    assert_eq!(stores.page_dimension(PageDimension::Depth), s(3));
    assert!(matches!(
        stores.page_contribution_front(),
        Some(Node::Kern { .. })
    ));
    stores.append_page_contribution(Node::Penalty(INF_PENALTY));
    build_page(&mut stores).expect("white-box operation succeeds");
    assert_eq!(stores.page_dimension(PageDimension::Total), s(16));
    assert_eq!(stores.page_dimension(PageDimension::Depth), s(0));
    assert!(stores.page_contributions().is_empty());
}

#[test]
fn page_infinite_shrink_recovery_normalizes_only_the_offending_glue() {
    let mut stores = crate::test_harness::universe();
    params(&mut stores, 10_000, 10, 0);
    stores.append_page_contribution(rule(1, 0));
    let bad = glue(&mut stores, 2, 0, Order::Normal, 5, Order::Fil);
    let good = glue(&mut stores, 3, 0, Order::Normal, 7, Order::Normal);
    stores.append_page_contribution(Node::Glue {
        spec: bad,
        kind: GlueKind::Normal,
        leader: None,
    });
    stores.append_page_contribution(Node::Glue {
        spec: good,
        kind: GlueKind::Normal,
        leader: None,
    });
    build_page(&mut stores).expect("white-box operation succeeds");
    let specs = stores
        .current_page_nodes()
        .iter()
        .filter_map(|node| match node {
            Node::Glue {
                spec,
                kind: GlueKind::Normal,
                ..
            } => Some(stores.glue(*spec)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(specs.len(), 2);
    assert_eq!(
        (specs[0].shrink_order, specs[0].shrink),
        (Order::Normal, s(5))
    );
    assert_eq!(
        (specs[1].shrink_order, specs[1].shrink),
        (Order::Normal, s(7))
    );
    assert_eq!(stores.page_dimension(PageDimension::Shrink), s(12));
    assert!(effects(&stores).contains("Infinite glue shrinkage found on current page"));
}

#[test]
fn page_break_badness_cost_and_equal_champion_boundaries_match_tex82() {
    let mut stores = Universe::new();
    stores.set_page_contents(PageContents::BoxThere);
    stores.set_page_dimension(PageDimension::Goal, s(100));
    stores.set_page_dimension(PageDimension::Stretch, s(100));
    assert_eq!(
        page_badness(&stores).expect("white-box operation succeeds"),
        100
    );
    check_break(&mut stores, 0).expect("white-box operation succeeds");
    assert_eq!(stores.best_page_break(), Some(PageBreak::new(0)));
    assert_eq!(stores.least_page_cost(), 100);
    stores.push_current_page_node(Node::Penalty(1));
    check_break(&mut stores, 0).expect("white-box operation succeeds");
    assert_eq!(stores.best_page_break(), Some(PageBreak::new(1)));
    stores.set_page_dimension(PageDimension::FilStretch, s(1));
    assert_eq!(
        page_badness(&stores).expect("white-box operation succeeds"),
        0
    );
    stores.set_page_dimension(PageDimension::FilStretch, s(0));
    stores.set_page_dimension(PageDimension::Total, s(110));
    stores.set_page_dimension(PageDimension::Shrink, s(10));
    assert_eq!(
        page_badness(&stores).expect("white-box operation succeeds"),
        100
    );
}

#[test]
fn page_break_eject_and_awful_cost_paths_fire_the_selected_champion() {
    let mut forced = Universe::new();
    forced.set_page_contents(PageContents::BoxThere);
    forced.set_page_dimension(PageDimension::Goal, s(10));
    forced.set_page_dimension(PageDimension::Total, s(10));
    check_break(&mut forced, EJECT_PENALTY).expect("white-box operation succeeds");
    assert_eq!(forced.least_page_cost(), EJECT_PENALTY);
    assert_eq!(
        forced
            .page_fire_up()
            .expect("white-box operation succeeds")
            .best_break(),
        PageBreak::new(0)
    );
    let mut awful = Universe::new();
    awful.set_page_contents(PageContents::BoxThere);
    awful.set_page_dimension(PageDimension::Goal, s(10));
    awful.set_page_dimension(PageDimension::Total, s(11));
    check_break(&mut awful, 0).expect("white-box operation succeeds");
    assert_eq!(awful.least_page_cost(), AWFUL_BAD);
    assert_eq!(
        awful
            .page_fire_up()
            .expect("white-box operation succeeds")
            .best_break(),
        PageBreak::new(0)
    );

    let mut insertion_overflow = Universe::new();
    insertion_overflow.set_page_contents(PageContents::BoxThere);
    insertion_overflow.set_page_dimension(PageDimension::Goal, s(10));
    insertion_overflow.set_page_dimension(PageDimension::Total, s(10));
    insertion_overflow.set_page_integer(PageInteger::InsertPenalties, INF_PENALTY);
    check_break(&mut insertion_overflow, 0).expect("white-box operation succeeds");
    assert_eq!(insertion_overflow.least_page_cost(), AWFUL_BAD);
    assert!(insertion_overflow.page_fire_up().is_some());

    let mut prohibited = Universe::new();
    prohibited.set_page_contents(PageContents::BoxThere);
    prohibited.set_page_dimension(PageDimension::Goal, s(10));
    prohibited.set_page_dimension(PageDimension::Total, s(10));
    check_break(&mut prohibited, INF_PENALTY).expect("white-box operation succeeds");
    assert_eq!(prohibited.best_page_break(), None);
}

#[test]
fn page_insertion_class_order_scaling_skip_and_fit_match_tex82() {
    let mut stores = Universe::new();
    params(&mut stores, 100_000, 0, 0);
    stores.freeze_page_specs(PageContents::InsertsOnly);
    ins_class(&mut stores, 9, 500, 100_000, 10, 3);
    ins_class(&mut stores, 3, 1_000, 100_000, 4, 3);
    let nine = ins(&mut stores, 9, 20_000, 0, &[]);
    prepare_insertion(&mut stores, &nine).expect("white-box operation succeeds");
    let three = ins(&mut stores, 3, 8_000, 0, &[]);
    prepare_insertion(&mut stores, &three).expect("white-box operation succeeds");
    let records = stores.page_insertions();
    assert_eq!(
        records.iter().map(PageInsertion::class).collect::<Vec<_>>(),
        [3, 9]
    );
    assert_eq!(
        (records[0].height(), records[1].height()),
        (s(8_000), s(20_000))
    );
    assert_eq!(stores.page_dimension(PageDimension::Goal), s(81_986));
    assert_eq!(stores.page_dimension(PageDimension::Stretch), s(6));
}

#[test]
fn page_insertion_split_float_penalty_and_invalid_box_recovery_match_tex82() {
    let mut stores = crate::test_harness::universe();
    params(&mut stores, 1_000, 0, 0);
    stores.freeze_page_specs(PageContents::InsertsOnly);
    ins_class(&mut stores, 7, 1_000, 5, 0, 0);
    let split = ins(&mut stores, 7, 10, 17, &[rule(10, 0), Node::Penalty(51)]);
    prepare_insertion(&mut stores, &split).expect("white-box operation succeeds");
    assert!(matches!(
        stores
            .page_insertion(7)
            .expect("white-box operation succeeds")
            .status(),
        PageInsertionStatus::SplitUp { .. }
    ));
    let before = stores.insert_penalties();
    prepare_insertion(&mut stores, &split).expect("white-box operation succeeds");
    assert_eq!(stores.insert_penalties(), before + 17);
    ins_class(&mut stores, 8, 1_000, 100, 0, 0);
    let hbox = boxed(&mut stores, 4, 2, false);
    let list = stores.freeze_node_list(&[hbox]);
    stores.set_box_reg(8, list);
    let invalid = ins(&mut stores, 8, 0, 0, &[]);
    prepare_insertion(&mut stores, &invalid).expect("white-box operation succeeds");
    assert!(stores.box_reg(8).is_none());
    assert!(effects(&stores).contains("Insertions can only be added to a vbox"));
}
