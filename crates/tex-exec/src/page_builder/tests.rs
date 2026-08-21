use super::*;

use tex_command::FatalError;
use tex_state::env::banks::{DimenParam, GlueParam, IntParam};
use tex_state::glue::{GlueSpec, Order};
use tex_state::ids::FontId;
use tex_state::math::{
    FractionThickness, MathChoice, MathField, MathFraction, MathListNode, MathNoad, MathStyle,
    NoadClass, NoadKind,
};
use tex_state::node::{
    BoxNode, BoxNodeFields, Direction, DiscKind, KernKind, Sign, UnsetKind, UnsetNode,
    UnsetNodeFields, Whatsit,
};
use tex_state::page::{PageBreak, PageInteger};
use tex_state::scaled::GlueSetRatio;

fn s(raw: i32) -> Scaled {
    Scaled::from_raw(raw)
}

fn glue<G>(
    _stores: &mut CommandContext<'_, G>,
    width: i32,
    stretch: i32,
    stretch_order: Order,
    shrink: i32,
    shrink_order: Order,
) -> GlueSpec {
    GlueSpec {
        width: s(width),
        stretch: s(stretch),
        stretch_order,
        shrink: s(shrink),
        shrink_order,
    }
}

fn params<G>(stores: &mut CommandContext<'_, G>, goal: i32, max_depth: i32, top_skip: i32) {
    stores
        .assign_dimen_param(
            DimenParam::V_SIZE,
            s(goal),
            tex_state::AssignmentScope::Global,
        )
        .expect("parameter");
    stores
        .assign_dimen_param(
            DimenParam::MAX_DEPTH,
            s(max_depth),
            tex_state::AssignmentScope::Global,
        )
        .expect("parameter");
    let top = glue(stores, top_skip, 0, Order::Normal, 0, Order::Normal);
    let top = stores.allocate_glue(top).expect("glue");
    stores
        .assign_glue_parameter(
            GlueParam::TOP_SKIP,
            Some(top),
            tex_state::AssignmentScope::Global,
        )
        .expect("parameter");
}

fn freeze_page_specs<G>(stores: &mut CommandContext<'_, G>, contents: PageContents) {
    let goal = stores.dimen_param(DimenParam::V_SIZE);
    let max_depth = stores.dimen_param(DimenParam::MAX_DEPTH);
    stores.freeze_page_specs(contents, goal, max_depth);
}

fn rule(height: i32, depth: i32) -> Node {
    Node::Rule {
        width: Some(s(1)),
        height: Some(s(height)),
        depth: Some(s(depth)),
    }
}

fn boxed<G>(stores: &mut CommandContext<'_, G>, height: i32, depth: i32, vertical: bool) -> Node {
    let children = stores.publish_page_nodes(Vec::new());
    let payload = BoxNode::new(BoxNodeFields {
        width: s(1),
        height: s(height),
        depth: s(depth),
        shift: s(0),
        box_lr: tex_state::node::BoxLr::Normal,
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

fn ins<G>(
    stores: &mut CommandContext<'_, G>,
    class: u16,
    size: i32,
    floating_penalty: i32,
    nodes: &[Node],
) -> Node {
    let content = stores.publish_page_nodes(nodes.to_vec());
    Node::Ins {
        class,
        size: s(size),
        split_top_skip: stores
            .glue_param(GlueParam::SPLIT_TOP_SKIP)
            .map_or(GlueSpec::ZERO, |id| stores.glue(id)),
        split_max_depth: Scaled::MAX_DIMEN,
        floating_penalty,
        content,
    }
}

fn ins_class<G>(
    stores: &mut CommandContext<'_, G>,
    class: u16,
    count: i32,
    max: i32,
    skip: i32,
    stretch: i32,
) {
    stores
        .assign_count(class, count, tex_state::AssignmentScope::Global)
        .expect("count");
    stores
        .assign_dimension(class, s(max), tex_state::AssignmentScope::Global)
        .expect("dimension");
    let skip = glue(stores, skip, stretch, Order::Normal, 0, Order::Normal);
    let skip = stores.allocate_glue(skip).expect("glue");
    stores
        .assign_glue_register(class, Some(skip), tex_state::AssignmentScope::Global)
        .expect("register");
}

fn effects<G>(stores: &tex_state::Universe<G>) -> String {
    format!("{:?}", stores.world().effect_records())
}

#[test]
fn pdftex_page_top_discards_snapy_but_preserves_other_whatsits() {
    // pdftex.web §§1378-1379 extends the page builder's discardable top
    // material with `pdf_snapy_node` only. `\pdfsnaprefpoint` is the subtype
    // negative control, and saving_vdiscards must retain the discarded node.
    crate::test_harness::with_universe(|universe| {
        let mut stores = universe.command_context().expect("test state is admitted");
        params(&mut stores, 1_000, 0, 0);
        stores
            .assign_int_param(
                IntParam::SAVING_V_DISCARDS,
                1,
                tex_state::AssignmentScope::Global,
            )
            .expect("parameter");
        let snap_glue = GlueSpec {
            width: s(7),
            ..GlueSpec::ZERO
        };
        let snap = Node::Whatsit(Whatsit::PdfSnapY { glue: snap_glue });
        let reference = Node::Whatsit(Whatsit::PdfSnapRefPoint);
        let first_box = boxed(&mut stores, 4, 0, false);
        stores.append_page_contribution(snap.clone());
        stores.append_page_contribution(reference.clone());
        stores.append_page_contribution(first_box.clone());

        build_page(&mut stores).expect("page-top snapping classification");

        assert_eq!(stores.take_page_discards(), [snap]);
        let current_page = stores.current_page_nodes().cloned().collect::<Vec<_>>();
        assert_eq!(
            current_page,
            [
                reference,
                Node::Glue {
                    spec: stores.glue(stores.glue_param(GlueParam::TOP_SKIP).expect("top skip")),
                    kind: GlueKind::TopSkip,
                    leader: None,
                },
                first_box,
            ]
        );
    });
}

#[test]
fn page_state_freezes_specs_and_tracks_sorted_insertion_records() {
    crate::test_harness::with_universe(|universe| {
        let mut stores = universe.command_context().expect("test state is admitted");
        params(&mut stores, 1_000, 17, 0);
        freeze_page_specs(&mut stores, PageContents::InsertsOnly);
        assert_eq!(stores.page_dimension(PageDimension::Goal), s(1_000));
        assert_eq!(stores.page_max_depth(), s(17));
        stores
            .assign_dimen_param(
                DimenParam::V_SIZE,
                s(2_000),
                tex_state::AssignmentScope::Global,
            )
            .expect("parameter");
        stores
            .assign_dimen_param(
                DimenParam::MAX_DEPTH,
                s(29),
                tex_state::AssignmentScope::Global,
            )
            .expect("parameter");
        assert_eq!(stores.page_dimension(PageDimension::Goal), s(1_000));
        assert_eq!(stores.page_max_depth(), s(17));
        for class in [9, 3] {
            ins_class(&mut stores, class, 1_000, 100, 0, 0);
            let node = ins(&mut stores, class, 0, 0, &[]);
            prepare_insertion(
                &mut stores,
                &node,
                &crate::diagnostics::ExecutionDiagnosticContext::default(),
            )
            .expect("white-box operation succeeds");
        }
        assert_eq!(
            stores
                .page_insertions()
                .iter()
                .map(PageInsertion::class)
                .collect::<Vec<_>>(),
            [3, 9]
        );
    });
}

#[test]
fn page_builder_output_active_boundary_preserves_pending_contributions() {
    crate::test_harness::with_universe(|universe| {
        let mut stores = universe.command_context().expect("test state is admitted");
        // TeX.web §§980--990: the frozen page specifications and all accumulated
        // quantities survive calls made while §989's output boundary is pending.
        params(&mut stores, 1_000, 19, 0);
        freeze_page_specs(&mut stores, PageContents::BoxThere);
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
        assert_eq!(
            stores
                .page_fire_up()
                .expect("white-box operation succeeds")
                .best_size(),
            s(23)
        );
    });
}

#[test]
fn new_current_page_resets_nodes_totals_depth_and_last_item_state() {
    crate::test_harness::with_universe(|universe| {
        let mut stores = universe.command_context().expect("test state is admitted");
        params(&mut stores, 1_000, 9, 0);
        freeze_page_specs(&mut stores, PageContents::BoxThere);
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
        stores.start_page_after_output();
        stores.freeze_page_specs(PageContents::Empty, Scaled::MAX_DIMEN, s(0));
        assert_eq!(stores.page_contents(), PageContents::Empty);
        assert_eq!(stores.current_page_len(), 0);
        assert_eq!(stores.page_max_depth(), s(0));
        assert_eq!(stores.insert_penalties(), 0);
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
    });
}

#[test]
fn output_page_reset_retains_totals_until_the_next_page_starts() {
    crate::test_harness::with_universe(|universe| {
        let mut stores = universe.command_context().expect("test state is admitted");
        params(&mut stores, 1_000, 9, 0);
        freeze_page_specs(&mut stores, PageContents::BoxThere);
        stores.push_current_page_node(Node::Penalty(41));
        stores.set_page_dimension(PageDimension::Total, s(52));
        stores.set_page_dimension(PageDimension::Shrink, s(51));

        stores.start_page_after_output();

        assert_eq!(stores.page_contents(), PageContents::Empty);
        assert_eq!(stores.current_page_len(), 0);
        assert_eq!(stores.page_dimension(PageDimension::Total), s(52));
        assert_eq!(stores.page_dimension(PageDimension::Shrink), s(51));

        freeze_page_specs(&mut stores, PageContents::BoxThere);
        assert_eq!(stores.page_dimension(PageDimension::Total), s(0));
        assert_eq!(stores.page_dimension(PageDimension::Shrink), s(0));
    });
}

#[test]
fn box_error_and_ensure_vbox_recover_only_invalid_live_boxes() {
    crate::test_harness::with_universe(|universe| {
        let mut stores = universe.command_context().expect("test state is admitted");
        assert_eq!(
            insertion_box_size(
                &mut stores,
                4,
                &crate::diagnostics::ExecutionDiagnosticContext::default(),
            )
            .expect("white-box operation succeeds"),
            s(0)
        );
        let node = boxed(&mut stores, 11, 3, true);
        let list = stores.publish_page_nodes(vec![node]);
        stores
            .assign_page_box(4, Some(list), tex_state::AssignmentScope::Local)
            .expect("box");
        assert_eq!(
            insertion_box_size(
                &mut stores,
                4,
                &crate::diagnostics::ExecutionDiagnosticContext::default(),
            )
            .expect("white-box operation succeeds"),
            s(14)
        );
        assert!(stores.copy_box_to_page(4).is_some());
        let node = boxed(&mut stores, 9, 2, false);
        let list = stores.publish_page_nodes(vec![node]);
        stores
            .assign_page_box(5, Some(list), tex_state::AssignmentScope::Local)
            .expect("box");
        assert_eq!(
            insertion_box_size(
                &mut stores,
                5,
                &crate::diagnostics::ExecutionDiagnosticContext::default(),
            )
            .expect("white-box operation succeeds"),
            s(0)
        );
        assert!(stores.copy_box_to_page(5).is_none());
        drop(stores);
        assert!(effects(universe).contains("Insertions can only be added to a vbox"));
    });
}

#[test]
fn box_error_voids_the_register_without_creating_local_assignment_history() {
    fn install_box<G>(stores: &mut CommandContext<'_, G>, register: u16, vertical: bool) {
        let node = boxed(stores, 9, 2, vertical);
        let list = stores.publish_page_nodes(vec![node]);
        stores.assign_page_box_global(register, list).expect("box");
    }

    fn log_text<G>(stores: &tex_state::Universe<G>) -> String {
        stores
            .world()
            .effect_records()
            .iter()
            .filter_map(|effect| match effect {
                tex_state::EffectRecord::StreamWrite {
                    sink: tex_state::PrintSink::Log | tex_state::PrintSink::TerminalAndLog,
                    text,
                } => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    let register = 5;
    let recovered_effects = crate::test_harness::with_universe(|universe| {
        let mut stores = universe.command_context().expect("test state is admitted");
        stores
            .assign_int_param(
                IntParam::TRACING_RESTORES,
                1,
                tex_state::AssignmentScope::Global,
            )
            .expect("parameter");
        install_box(&mut stores, register, false);
        stores
            .begin_group(tex_state::GroupKind::Simple, 0)
            .expect("group");
        assert_eq!(
            insertion_box_size(
                &mut stores,
                register,
                &crate::diagnostics::ExecutionDiagnosticContext::default(),
            )
            .expect("section 993 recovery is nonfatal"),
            s(0)
        );
        install_box(&mut stores, register, true);
        stores
            .end_group(tex_state::GroupKind::Simple)
            .expect("group");
        assert!(stores.copy_box_to_page(register).is_some());
        drop(stores);
        log_text(universe)
    });
    assert!(
        !recovered_effects.contains("retaining \\box5="),
        "section 993 direct mutation must not create a restore record: {recovered_effects}"
    );

    // Negative control: the ordinary local assignment barrier must still
    // save and report the retained global value under TeX82 §§275/283.
    let assigned_effects = crate::test_harness::with_universe(|universe| {
        let mut stores = universe.command_context().expect("test state is admitted");
        stores
            .assign_int_param(
                IntParam::TRACING_RESTORES,
                1,
                tex_state::AssignmentScope::Global,
            )
            .expect("parameter");
        install_box(&mut stores, register, false);
        stores
            .begin_group(tex_state::GroupKind::Simple, 0)
            .expect("group");
        stores
            .assign_page_box(register, None, tex_state::AssignmentScope::Local)
            .expect("box");
        install_box(&mut stores, register, true);
        stores
            .end_group(tex_state::GroupKind::Simple)
            .expect("group");
        drop(stores);
        log_text(universe)
    });
    assert!(
        assigned_effects.contains("retaining \\box5="),
        "ordinary assignments remain save-stack visible: {assigned_effects}"
    );
}

#[test]
fn outer_vertical_contribution_routes_every_node_kind_canonically() {
    crate::test_harness::with_universe(|universe| {
        let mut stores = universe.command_context().expect("test state is admitted");
        params(&mut stores, 10_000, 10, 10);
        stores
            .assign_int_param(
                IntParam::SAVING_V_DISCARDS,
                1,
                tex_state::AssignmentScope::Global,
            )
            .expect("parameter");
        let leading = glue(&mut stores, 2, 0, Order::Normal, 0, Order::Normal);
        let mark = tex_state::node::NodeTokenList::default();
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
        assert_eq!(stores.take_page_discards().len(), 3);
        let current_page = stores.current_page_nodes().cloned().collect::<Vec<_>>();
        assert!(matches!(current_page[0], Node::Mark { class: 2, .. }));
        assert!(matches!(
            current_page[1],
            Node::Glue {
                kind: GlueKind::TopSkip,
                ..
            }
        ));
        assert!(matches!(current_page[2], Node::Rule { .. }));
        assert_eq!(current_page[3], Node::Penalty(INF_PENALTY));
        assert!(stores.page_contributions().is_empty());
    });
}

#[test]
fn page_builder_rejects_impossible_contribution_nodes_with_page_confusion() {
    let empty = tex_state::node_arena::PageListId::empty();
    let impossible = [
        Node::Char {
            font: FontId::testing_new(0),
            ch: 'x',
            origin: tex_state::token::OriginId::UNKNOWN,
        },
        Node::Lig {
            font: FontId::testing_new(0),
            ch: 'x',
            orig: vec!['x'],
            origins: vec![tex_state::token::OriginId::UNKNOWN],
            left_hit: false,
            right_hit: false,
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
            children: empty.clone(),
        })),
        Node::Disc {
            kind: DiscKind::Discretionary,
            pre: empty.clone(),
            post: empty.clone(),
            replace: empty.clone(),
            physical_replace_count: 0,
        },
        Node::MathOn(s(1)),
        Node::MathOff(s(1)),
        Node::Direction(Direction::BeginR),
        Node::MathNoad(MathNoad::new(
            NoadKind::Normal(NoadClass::Ord),
            MathField::Empty,
        )),
        Node::FractionNoad(MathFraction {
            numerator: empty.clone(),
            denominator: empty.clone(),
            thickness: FractionThickness::Default,
            left_delimiter: None,
            right_delimiter: None,
        }),
        Node::MathStyle(MathStyle::Text),
        Node::MathChoice(MathChoice {
            display: empty.clone(),
            text: empty.clone(),
            script: empty.clone(),
            script_script: empty.clone(),
        }),
        Node::MathList(MathListNode {
            display: false,
            content: empty.clone(),
        }),
        Node::Nonscript,
        Node::Adjust(tex_state::node::AdjustNode {
            content: empty,
            pre: false,
        }),
    ];

    for node in impossible {
        crate::test_harness::with_universe(|universe| {
            let mut stores = universe.command_context().expect("test state is admitted");
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
            assert_eq!(
                stores.current_page_nodes().cloned().collect::<Vec<_>>(),
                [Node::Penalty(41)]
            );
            assert_eq!(stores.page_contents(), PageContents::BoxThere);
            assert_eq!(stores.page_dimension(PageDimension::Total), s(23));
            assert_eq!(stores.page_dimension(PageDimension::Depth), s(5));
            assert_eq!(stores.page_last_kern(), s(33));
        });
    }
}

#[test]
fn page_topskip_totals_depth_and_terminal_kern_boundaries_match_tex82() {
    crate::test_harness::with_universe(|universe| {
        let mut stores = universe.command_context().expect("test state is admitted");
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
    });
}

#[test]
fn page_contribution_last_items_and_max_depth_matrix() {
    // TeX.web §§994--1004: each contribution refreshes the last-item
    // enquiries, while §1004 corrects the preceding depth immediately before
    // the next node is linked to the current page.
    crate::test_harness::with_universe(|universe| {
        let mut stores = universe.command_context().expect("test state is admitted");
        params(&mut stores, 10_000, 3, 0);

        stores.append_page_contribution(rule(5, 7));
        build_page(&mut stores).expect("box contribution succeeds");
        assert!(!stores.page_has_last_glue());
        assert_eq!(stores.page_last_penalty(), 0);
        assert_eq!(stores.page_last_kern(), s(0));
        assert_eq!(stores.page_dimension(PageDimension::Total), s(9));
        assert_eq!(stores.page_dimension(PageDimension::Depth), s(3));

        let zero_glue = glue(&mut stores, 0, 0, Order::Normal, 0, Order::Normal);
        stores.append_page_contribution(Node::Glue {
            spec: zero_glue,
            kind: GlueKind::Normal,
            leader: None,
        });
        build_page(&mut stores).expect("glue contribution succeeds");
        assert!(stores.page_has_last_glue());
        assert_eq!(stores.page_last_skip(), Some(GlueSpec::ZERO));
        assert_eq!(stores.page_dimension(PageDimension::Total), s(12));
        assert_eq!(stores.page_dimension(PageDimension::Depth), s(0));

        stores.append_page_contribution(Node::Kern {
            amount: s(11),
            kind: KernKind::Explicit,
        });
        build_page(&mut stores).expect("terminal kern remains pending");
        assert_eq!(stores.page_last_kern(), s(11));
        assert_eq!(stores.current_page_len(), 3);
        assert!(matches!(
            stores.page_contribution_front(),
            Some(Node::Kern { .. })
        ));

        stores.append_page_contribution(Node::Penalty(23));
        build_page(&mut stores).expect("kern and penalty contributions succeed");
        assert!(!stores.page_has_last_glue());
        assert_eq!(stores.page_last_penalty(), 23);
        assert_eq!(stores.page_last_kern(), s(0));
        assert_eq!(stores.page_dimension(PageDimension::Total), s(23));

        let mark = tex_state::node::NodeTokenList::default();
        stores.append_page_contribution(Node::Mark {
            class: 4,
            tokens: mark,
        });
        build_page(&mut stores).expect("mark contribution succeeds");
        assert!(!stores.page_has_last_glue());
        assert_eq!(stores.page_last_penalty(), 0);
        assert_eq!(stores.page_last_kern(), s(0));
    });
}

#[test]
fn page_infinite_shrink_recovery_normalizes_only_the_offending_glue() {
    crate::test_harness::with_universe(|universe| {
        let mut stores = universe.command_context().expect("test state is admitted");
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
        build_page_with_error_context(&mut stores, "l.27 published page continuation")
            .expect("white-box operation succeeds");
        let specs = stores
            .current_page_nodes()
            .filter_map(|node| match node {
                Node::Glue {
                    spec,
                    kind: GlueKind::Normal,
                    ..
                } => Some(spec),
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
        drop(stores);
        let output = effects(universe);
        assert!(output.contains("Infinite glue shrinkage found on current page"));
        assert!(output.contains("l.27 published "), "{output}");
        assert!(output.contains("continuation"), "{output}");
    });
}

#[test]
fn page_break_badness_cost_and_equal_champion_boundaries_match_tex82() {
    crate::test_harness::with_universe(|universe| {
        let mut stores = universe.command_context().expect("test state is admitted");
        stores.set_page_contents(PageContents::BoxThere);
        stores.set_page_dimension(PageDimension::Goal, s(100));
        stores.set_page_dimension(PageDimension::Stretch, s(100));
        assert_eq!(
            page_badness(&stores).expect("white-box operation succeeds"),
            100
        );
        check_break(&mut stores, 0).expect("white-box operation succeeds");
        assert_eq!(stores.least_page_cost(), 110);
        assert_eq!(stores.least_page_cost(), 100);
        stores.push_current_page_node(Node::Penalty(1));
        check_break(&mut stores, 0).expect("white-box operation succeeds");
        assert_eq!(stores.least_page_cost(), 110);
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
    });
}

#[test]
fn page_break_eject_and_awful_cost_paths_fire_the_selected_champion() {
    crate::test_harness::with_universe(|universe| {
        let mut forced = universe.command_context().expect("test state is admitted");
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
    });
    crate::test_harness::with_universe(|universe| {
        let mut awful = universe.command_context().expect("test state is admitted");
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
    });

    crate::test_harness::with_universe(|universe| {
        let mut insertion_overflow = universe.command_context().expect("test state is admitted");
        insertion_overflow.set_page_contents(PageContents::BoxThere);
        insertion_overflow.set_page_dimension(PageDimension::Goal, s(10));
        insertion_overflow.set_page_dimension(PageDimension::Total, s(10));
        insertion_overflow.set_page_integer(PageInteger::InsertPenalties, INF_PENALTY);
        check_break(&mut insertion_overflow, 0).expect("white-box operation succeeds");
        assert_eq!(insertion_overflow.least_page_cost(), AWFUL_BAD);
        assert!(insertion_overflow.page_fire_up().is_some());
    });

    crate::test_harness::with_universe(|universe| {
        let mut prohibited = universe.command_context().expect("test state is admitted");
        prohibited.set_page_contents(PageContents::BoxThere);
        prohibited.set_page_dimension(PageDimension::Goal, s(10));
        prohibited.set_page_dimension(PageDimension::Total, s(10));
        check_break(&mut prohibited, INF_PENALTY).expect("white-box operation succeeds");
        assert_eq!(prohibited.page_fire_up(), None);
    });
}

#[test]
fn page_insertion_class_order_scaling_skip_and_fit_match_tex82() {
    crate::test_harness::with_universe(|universe| {
        let mut stores = universe.command_context().expect("test state is admitted");
        params(&mut stores, 100_000, 0, 0);
        freeze_page_specs(&mut stores, PageContents::InsertsOnly);
        ins_class(&mut stores, 9, 500, 100_000, 10, 3);
        ins_class(&mut stores, 3, 1_000, 100_000, 4, 3);
        let nine = ins(&mut stores, 9, 20_000, 0, &[]);
        prepare_insertion(
            &mut stores,
            &nine,
            &crate::diagnostics::ExecutionDiagnosticContext::default(),
        )
        .expect("white-box operation succeeds");
        let three = ins(&mut stores, 3, 8_000, 0, &[]);
        prepare_insertion(
            &mut stores,
            &three,
            &crate::diagnostics::ExecutionDiagnosticContext::default(),
        )
        .expect("white-box operation succeeds");
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
    });
}

#[test]
fn page_insertion_split_float_penalty_and_invalid_box_recovery_match_tex82() {
    crate::test_harness::with_universe(|universe| {
        let mut stores = universe.command_context().expect("test state is admitted");
        params(&mut stores, 1_000, 0, 0);
        freeze_page_specs(&mut stores, PageContents::InsertsOnly);
        ins_class(&mut stores, 7, 1_000, 5, 0, 0);
        let split = ins(&mut stores, 7, 10, 17, &[rule(10, 0), Node::Penalty(51)]);
        prepare_insertion(
            &mut stores,
            &split,
            &crate::diagnostics::ExecutionDiagnosticContext::default(),
        )
        .expect("white-box operation succeeds");
        assert!(matches!(
            stores
                .page_insertion(7)
                .expect("white-box operation succeeds")
                .status(),
            PageInsertionStatus::SplitUp { .. }
        ));
        let before = stores.insert_penalties();
        prepare_insertion(
            &mut stores,
            &split,
            &crate::diagnostics::ExecutionDiagnosticContext::default(),
        )
        .expect("white-box operation succeeds");
        assert_eq!(stores.insert_penalties(), before + 17);
        ins_class(&mut stores, 8, 1_000, 100, 0, 0);
        let hbox = boxed(&mut stores, 4, 2, false);
        let list = stores.publish_page_nodes(vec![hbox]);
        stores
            .assign_page_box(8, Some(list), tex_state::AssignmentScope::Local)
            .expect("box");
        let invalid = ins(&mut stores, 8, 0, 0, &[]);
        prepare_insertion(
            &mut stores,
            &invalid,
            &crate::diagnostics::ExecutionDiagnosticContext::default(),
        )
        .expect("white-box operation succeeds");
        assert!(stores.copy_box_to_page(8).is_none());
        drop(stores);
        assert!(effects(universe).contains("Insertions can only be added to a vbox"));
    });
}

#[test]
fn page_insertion_split_tracing_reports_class_height_and_penalty() {
    crate::test_harness::with_universe(|universe| {
        let mut stores = universe.command_context().expect("test state is admitted");
        params(&mut stores, 5, 0, 0);
        stores
            .assign_int_param(
                IntParam::TRACING_PAGES,
                1,
                tex_state::AssignmentScope::Global,
            )
            .expect("parameter");
        freeze_page_specs(&mut stores, PageContents::InsertsOnly);
        ins_class(&mut stores, 7, 1_000, 20, 0, 0);
        let split = ins(&mut stores, 7, 10, 0, &[rule(10, 0), Node::Penalty(51)]);

        prepare_insertion(
            &mut stores,
            &split,
            &crate::diagnostics::ExecutionDiagnosticContext::default(),
        )
        .expect("traced insertion split succeeds");

        drop(stores);
        let trace = effects(universe);
        for operand in ["% split", "7", "0.00008", "0.00015", " p=", "51"] {
            assert!(trace.contains(operand), "missing {operand:?} from {trace}");
        }
    });
}

#[test]
fn page_insertion_count_capacity_and_null_split_matrix() {
    // TeX.web §§1008--1011: a class's correction glue is charged once, every
    // repeated insertion is count-scaled, equality fits both the page and the
    // class capacity, and a null split contributes the eject penalty.
    crate::test_harness::with_universe(|universe| {
        let mut repeated = universe.command_context().expect("test state is admitted");
        params(&mut repeated, 100_000, 0, 0);
        freeze_page_specs(&mut repeated, PageContents::InsertsOnly);
        ins_class(&mut repeated, 2, 500, 40_000, 7_000, 3_000);
        let first = ins(&mut repeated, 2, 10_000, 0, &[]);
        prepare_insertion(
            &mut repeated,
            &first,
            &crate::diagnostics::ExecutionDiagnosticContext::default(),
        )
        .expect("first insertion fits");
        let second = ins(&mut repeated, 2, 20_000, 0, &[]);
        prepare_insertion(
            &mut repeated,
            &second,
            &crate::diagnostics::ExecutionDiagnosticContext::default(),
        )
        .expect("repeated insertion fits");
        let record = repeated.page_insertion(2).expect("class record is present");
        assert_eq!(record.height(), s(30_000));
        assert_eq!(record.last_ins_index(), Some(0));
        assert_eq!(repeated.page_dimension(PageDimension::Goal), s(78_000));
        assert_eq!(repeated.page_dimension(PageDimension::Stretch), s(3_000));
    });

    crate::test_harness::with_universe(|universe| {
        let mut exact = universe.command_context().expect("test state is admitted");
        params(&mut exact, 10, 0, 0);
        freeze_page_specs(&mut exact, PageContents::InsertsOnly);
        ins_class(&mut exact, 3, 1_000, 10, 0, 0);
        let at_capacity = ins(&mut exact, 3, 10, 0, &[]);
        prepare_insertion(
            &mut exact,
            &at_capacity,
            &crate::diagnostics::ExecutionDiagnosticContext::default(),
        )
        .expect("capacity equality fits");
        assert_eq!(exact.page_dimension(PageDimension::Goal), s(0));
        assert_eq!(
            exact.page_insertion(3).map(|record| record.height()),
            Some(s(10))
        );
        assert_eq!(
            exact.page_insertion(3).expect("class record").status(),
            PageInsertionStatus::Inserting
        );
    });

    crate::test_harness::with_universe(|universe| {
        let mut null_split = universe.command_context().expect("test state is admitted");
        params(&mut null_split, 0, 0, 0);
        freeze_page_specs(&mut null_split, PageContents::InsertsOnly);
        ins_class(&mut null_split, 5, 1_000, 20, 0, 0);
        let unsplittable = ins(&mut null_split, 5, 9, 37, &[rule(9, 0)]);
        prepare_insertion(
            &mut null_split,
            &unsplittable,
            &crate::diagnostics::ExecutionDiagnosticContext::default(),
        )
        .expect("null split is recorded");
        assert_eq!(
            null_split.page_insertion(5).map(|record| record.height()),
            Some(s(9))
        );
        assert_eq!(null_split.insert_penalties(), EJECT_PENALTY);
        assert!(matches!(
            null_split.page_insertion(5).expect("class record").status(),
            PageInsertionStatus::SplitUp {
                broken_at: None,
                ..
            }
        ));

        prepare_insertion(
            &mut null_split,
            &unsplittable,
            &crate::diagnostics::ExecutionDiagnosticContext::default(),
        )
        .expect("later insertion in split class is held over");
        assert_eq!(null_split.insert_penalties(), EJECT_PENALTY + 37);
    });
}
