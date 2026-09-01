use super::*;

use tex_state::page::{EJECT_PENALTY, PageBreak, PageInsertion};

fn fire_up(best_break: usize, trigger: usize) -> PageFireUp {
    PageFireUp::new(
        PageBreak::new(best_break),
        Scaled::from_raw(0),
        PageBreak::new(trigger),
    )
}

fn rule(height: i32) -> Node {
    Node::Rule {
        width: Some(Scaled::from_raw(1)),
        height: Some(Scaled::from_raw(height)),
        depth: Some(Scaled::from_raw(0)),
    }
}

fn insertion<G>(stores: &mut CommandContext<'_, G>, class: u16, height: i32) -> Node {
    let content = stores.publish_page_nodes(vec![rule(height)]);
    Node::Ins {
        class,
        size: Scaled::from_raw(height),
        split_top_skip: GlueSpec::ZERO,
        split_max_depth: Scaled::MAX_DIMEN,
        floating_penalty: 0,
        content,
    }
}

#[test]
fn fire_up_recovers_hbox_insertion_register_before_distribution() {
    crate::test_harness::with_nonstop_universe(|universe| {
        universe.set_interaction_mode(tex_state::InteractionMode::Nonstop);
        let mut stores = universe.command_context().expect("test state is admitted");
        let diagnostic_context = ExecutionDiagnosticContext::source_free("fire-up context");
        let class = 7;
        let first = insertion(&mut stores, class, 11);
        let unrelated = insertion(&mut stores, class + 1, 12);
        let last = insertion(&mut stores, class, 13);
        let later = insertion(&mut stores, class, 14);
        let page_nodes = stores.publish_page_nodes(vec![
            first,
            unrelated.clone(),
            Node::Penalty(29),
            last,
            later.clone(),
        ]);

        let mut record = PageInsertion::new(class, Scaled::from_raw(0));
        record.set_last_ins_index(Some(3));
        stores.upsert_page_insertion(record);
        stores.record_best_page_break(page_nodes.len(), Scaled::from_raw(0), 0);

        let children = stores.publish_page_nodes(vec![rule(99)]);
        let hbox = BoxNode::new(BoxNodeFields {
            width: Scaled::from_raw(1),
            height: Scaled::from_raw(99),
            depth: Scaled::from_raw(0),
            shift: Scaled::from_raw(0),
            box_lr: tex_state::node::BoxLr::Normal,
            glue_set: GlueSetRatio::ZERO,
            glue_sign: Sign::Normal,
            glue_order: Order::Normal,
            children,
        });
        let register = stores.publish_page_nodes(vec![Node::HList(hbox)]);
        stores
            .assign_page_box(class, Some(register), tex_state::AssignmentScope::Local)
            .expect("box assignment");

        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut geometry = crate::geometry::IgnorePackGeometry;
        let distributed = distribute_insertions(
            &mut stores,
            &mut diagnostic_effects,
            &mut geometry,
            &diagnostic_context,
            page_nodes,
        )
        .expect("hbox recovery is nonfatal");

        assert!(matches!(
            stores
                .page_node_list(distributed.page_nodes)
                .expect("distributed page list remains live")
                .nodes()
                .first(),
            Some(tex_state::NodeView::Penalty(29))
        ));
        let heldover = stores
            .page_node_list(distributed.heldover)
            .expect("held-over list remains live")
            .nodes()
            .iter()
            .map(|node| node.to_owned_with(std::convert::identity))
            .collect::<Vec<_>>();
        assert_eq!(heldover, [unrelated, later]);
        assert_eq!(distributed.heldover_count, 2);
        let register = stores
            .copy_box_to_page(class)
            .expect("accepted inserts are repackaged");
        let tex_state::NodeView::VList(box_node) = stores
            .page_node_list(register)
            .expect("register belongs to the page arena")
            .nodes()
            .first()
            .expect("register contains its vbox")
        else {
            panic!("insertion register must become a vbox");
        };
        let register_children = stores
            .page_node_list(box_node.children)
            .expect("register children belong to the page arena")
            .nodes()
            .iter()
            .map(|node| node.to_owned_with(std::convert::identity))
            .collect::<Vec<_>>();
        assert_eq!(register_children, [rule(11), rule(13)]);
        drop(stores);
        universe
            .world_mut()
            .publish_diagnostic_effects(diagnostic_effects);
        let effects = format!("{:?}", universe.world().effect_records());
        assert!(effects.contains("Insertions can only be added to a vbox"));
        assert!(effects.contains("The following box has been deleted:"));
        assert!(effects.contains("fire-up context"));
    });
}

#[test]
fn input_free_box255_recovery_uses_explicit_context() {
    crate::test_harness::with_nonstop_universe(|universe| {
        let mut stores = universe.command_context().expect("test state is admitted");
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let deleted = stores.publish_page_nodes(vec![rule(7)]);

        report_box255_not_void(
            &mut stores,
            &mut diagnostic_effects,
            deleted,
            Some("l.31 published output continuation"),
        )
        .expect("recovery is nonfatal");

        drop(stores);
        universe
            .world_mut()
            .publish_diagnostic_effects(diagnostic_effects);
        let output = universe
            .world()
            .effect_records()
            .iter()
            .filter_map(|record| match record {
                tex_state::EffectRecord::StreamWrite { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<String>();
        assert!(output.contains("box255 is not void"), "{output}");
        assert!(output.contains("l.31 published "), "{output}");
        assert!(output.contains("output continuation"), "{output}");
    });
}

#[test]
fn fire_up_preserves_void_and_vbox_insertion_queues() {
    crate::test_harness::with_nonstop_universe(|universe| {
        let mut stores = universe.command_context().expect("test state is admitted");
        assert!(
            insertion_box_nodes(
                &mut stores,
                &mut tex_state::diagnostic::DiagnosticEffects::new(),
                2,
                &crate::diagnostics::ExecutionDiagnosticContext::default(),
            )
            .expect("void box is valid")
            .is_empty()
        );
        assert!(stores.copy_box_to_page(2).is_none());

        let children = stores.publish_page_nodes(vec![rule(17), Node::Penalty(23)]);
        let vbox = BoxNode::new(BoxNodeFields {
            width: Scaled::from_raw(1),
            height: Scaled::from_raw(17),
            depth: Scaled::from_raw(0),
            shift: Scaled::from_raw(0),
            box_lr: tex_state::node::BoxLr::Normal,
            glue_set: GlueSetRatio::ZERO,
            glue_sign: Sign::Normal,
            glue_order: Order::Normal,
            children,
        });
        let register = stores.publish_page_nodes(vec![Node::VList(vbox)]);
        stores
            .assign_page_box(2, Some(register), tex_state::AssignmentScope::Local)
            .expect("box assignment");

        let insertion_nodes = insertion_box_nodes(
            &mut stores,
            &mut tex_state::diagnostic::DiagnosticEffects::new(),
            2,
            &crate::diagnostics::ExecutionDiagnosticContext::default(),
        )
        .expect("vbox is valid");
        assert_eq!(
            stores
                .page_node_list(insertion_nodes)
                .expect("insertion children remain live")
                .nodes()
                .iter()
                .cloned()
                .collect::<Vec<_>>(),
            [rule(17), Node::Penalty(23)]
        );
        let retained = stores
            .copy_box_to_page(2)
            .expect("vbox register remains populated");
        assert!(matches!(
            stores
                .page_node_list(retained)
                .expect("retained register belongs to the page arena")
                .nodes()
                .first(),
            Some(tex_state::NodeView::VList(_))
        ));
    });
}

#[test]
fn earlier_break_preserves_unrelated_pending_penalty() {
    crate::test_harness::with_nonstop_universe(|universe| {
        let mut stores = universe.command_context().expect("test state is admitted");
        stores.append_page_contribution(Node::Penalty(EJECT_PENALTY));
        let glue = GlueSpec {
            width: Scaled::from_raw(0),
            stretch: Scaled::from_raw(0),
            stretch_order: Order::Normal,
            shrink: Scaled::from_raw(0),
            shrink_order: Order::Normal,
        };
        let chosen_break = Node::Glue {
            spec: glue,
            kind: GlueKind::Normal,
            leader: None,
        };
        let after_break = stores.publish_page_nodes(vec![chosen_break.clone()]);

        let (penalty, after_break) =
            output_penalty_and_rewrite_break(&mut stores, after_break, fire_up(1, 2));

        assert_eq!(penalty, INF_PENALTY);
        assert_eq!(
            stores
                .page_node_list(after_break)
                .expect("unchosen break remains live")
                .nodes()
                .first()
                .map(|node| node.to_owned_with(std::convert::identity)),
            Some(chosen_break)
        );
        assert_eq!(stores.page_contributions().len(), 1);
        assert_eq!(
            stores.page_contributions().front(),
            Some(tex_state::NodeView::Penalty(EJECT_PENALTY))
        );
    });
}

#[test]
fn chosen_pending_penalty_is_rewritten() {
    crate::test_harness::with_nonstop_universe(|universe| {
        let mut stores = universe.command_context().expect("test state is admitted");
        stores.append_page_contribution(Node::Penalty(EJECT_PENALTY));
        let after_break = tex_state::node_arena::PageListId::empty();

        let (penalty, after_break) =
            output_penalty_and_rewrite_break(&mut stores, after_break, fire_up(1, 1));

        assert_eq!(penalty, EJECT_PENALTY);
        assert!(matches!(
            stores
                .page_node_list(after_break)
                .expect("rewritten break remains live")
                .nodes()
                .first(),
            Some(tex_state::NodeView::Penalty(INF_PENALTY))
        ));
        assert!(stores.page_contributions().is_empty());
    });
}

#[test]
fn end_cleanup_uses_tex_its_all_over_penalty() {
    crate::test_harness::with_nonstop_universe(|universe| {
        let mut stores = universe.command_context().expect("test state is admitted");

        append_end_job_contributions(&mut stores);

        assert_eq!(
            stores.page_contributions().back(),
            Some(tex_state::NodeView::Penalty(-1_073_741_824))
        );
    });
}

#[test]
fn job_is_all_over_only_when_page_and_contributions_are_empty() {
    // TeX82 §1054: `(page_head=page_tail) and (head=tail) and
    // (dead_cycles=0)`. A residual contribution alone keeps `\end` from
    // ending the job, which is what makes the end-job trio reachable.
    crate::test_harness::with_nonstop_universe(|universe| {
        let mut stores = universe.command_context().expect("test state is admitted");
        assert!(job_is_all_over(&stores));

        let residual = Node::HList(BoxNode::new(BoxNodeFields {
            width: Scaled::from_raw(17),
            height: Scaled::from_raw(11),
            depth: Scaled::from_raw(3),
            shift: Scaled::from_raw(0),
            box_lr: tex_state::node::BoxLr::Normal,
            glue_set: GlueSetRatio::ZERO,
            glue_sign: Sign::Normal,
            glue_order: Order::Normal,
            children: tex_state::node_arena::PageListId::empty(),
        }));
        stores.append_page_contribution(residual);
        assert!(!job_is_all_over(&stores));

        drop(stores);
        let mut stores = universe.command_context().expect("test state is admitted");
        stores.set_page_integer(PageInteger::DeadCycles, 1);
        assert!(!job_is_all_over(&stores));
    });
}
