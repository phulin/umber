use tex_state::glue::GlueSpec;
use tex_state::glue::Order;
use tex_state::node::{BoxLr, BoxNode, BoxNodeFields, Direction, GlueKind, KernKind, Node, Sign};
use tex_state::node_arena::PageListId;
use tex_state::scaled::{GlueSetRatio, Scaled};

use super::{display_line_prototype, package_directed_display_line};

fn box_node(width: i32, height: i32, depth: i32, shift: i32, box_lr: BoxLr) -> BoxNode {
    let children = PageListId::empty();
    BoxNode::new(BoxNodeFields {
        width: Scaled::from_raw(width),
        height: Scaled::from_raw(height),
        depth: Scaled::from_raw(depth),
        shift: Scaled::from_raw(shift),
        box_lr,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children,
    })
}

#[test]
fn etex_display_prototype_replaces_its_list_without_repacking() {
    // Merged e-TeX §§1475 and 1478--1480: a display after a nonempty
    // paragraph copies the saved last-line prototype and replaces its list.
    // Only the no-prototype control calls hpack to create a new line box.
    crate::test_harness::with_nonstop_plain_universe(|universe| {
        let mut stores = universe.command_context().expect("test state is admitted");
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut geometry = crate::geometry::IgnorePackGeometry;
        let diagnostic_context = crate::pack_report::ExecutionDiagnosticContext::source_free("");
        let last_line = box_node(100, 7, 3, 5, BoxLr::Normal);
        let prototype = display_line_prototype(&mut stores, last_line);
        let display = box_node(10, 8, 2, 0, BoxLr::DList);

        let reused = package_directed_display_line(
            &mut stores,
            &mut diagnostic_effects,
            &mut geometry,
            &diagnostic_context,
            display,
            Some(prototype),
            Scaled::from_raw(20),
            Scaled::from_raw(10),
            Scaled::from_raw(100),
            1,
        );

        assert_eq!(
            (reused.width.raw(), reused.height.raw(), reused.depth.raw()),
            (100, 8, 2)
        );
        assert_eq!(reused.shift.raw(), 5);
        let nodes = stores
            .page_node_list(reused.children)
            .expect("display children belong to the page arena")
            .nodes()
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        assert!(matches!(
            nodes.as_slice(),
            [
                Node::Direction(Direction::BeginM),
                Node::Kern { amount: left, kind: KernKind::Font },
                Node::HList(_),
                Node::Kern { amount: right, kind: KernKind::Font },
                Node::Direction(Direction::EndM),
            ] if left.raw() == 25 && right.raw() == 65
        ));

        let display = box_node(10, 8, 2, 0, BoxLr::DList);
        let packed = package_directed_display_line(
            &mut stores,
            &mut diagnostic_effects,
            &mut geometry,
            &diagnostic_context,
            display,
            None,
            Scaled::from_raw(20),
            Scaled::from_raw(10),
            Scaled::from_raw(100),
            1,
        );
        assert_eq!(packed.shift.raw(), 10);
    });
}

#[test]
fn directed_display_retains_prototype_glue_ranges() {
    crate::test_harness::with_nonstop_plain_universe(|universe| {
        let mut stores = universe.command_context().expect("test state is admitted");
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut geometry = crate::geometry::IgnorePackGeometry;
        let diagnostic_context = crate::pack_report::ExecutionDiagnosticContext::source_free("");
        let skip = GlueSpec {
            width: Scaled::from_raw(3),
            stretch: Scaled::from_raw(1),
            stretch_order: Order::Normal,
            shrink: Scaled::from_raw(1),
            shrink_order: Order::Normal,
        };
        let children = stores.publish_page_nodes(vec![
            Node::Glue {
                spec: skip,
                kind: GlueKind::LeftSkip,
                leader: None,
            },
            Node::Glue {
                spec: skip,
                kind: GlueKind::RightSkip,
                leader: None,
            },
        ]);
        let prototype = BoxNode::new(BoxNodeFields {
            width: Scaled::from_raw(100),
            height: Scaled::from_raw(0),
            depth: Scaled::from_raw(0),
            shift: Scaled::from_raw(5),
            box_lr: BoxLr::Normal,
            glue_set: GlueSetRatio::ZERO,
            glue_sign: Sign::Normal,
            glue_order: Order::Normal,
            children,
        });
        let prototype_view = stores
            .page_node_list(children)
            .expect("prototype children belong to the page arena");
        let left_address =
            std::ptr::from_ref(prototype_view.owned_node(0).expect("left skip exists"));
        let right_address =
            std::ptr::from_ref(prototype_view.owned_node(1).expect("right skip exists"));
        let before = stores.page_node_arena_counters();

        let reused = package_directed_display_line(
            &mut stores,
            &mut diagnostic_effects,
            &mut geometry,
            &diagnostic_context,
            box_node(10, 8, 2, 0, BoxLr::DList),
            Some(prototype),
            Scaled::from_raw(20),
            Scaled::from_raw(10),
            Scaled::from_raw(100),
            1,
        );

        let after = stores.page_node_arena_counters();
        assert!(after.new_semantic_nodes > before.new_semantic_nodes);
        assert_eq!(after.source_nodes_copied, before.source_nodes_copied);
        let reused = stores
            .page_node_list(reused.children)
            .expect("directed display belongs to the page arena");
        assert_eq!(
            std::ptr::from_ref(reused.owned_node(0).expect("left skip retained")),
            left_address
        );
        assert_eq!(
            std::ptr::from_ref(reused.owned_node(6).expect("right skip retained")),
            right_address
        );
    });
}
