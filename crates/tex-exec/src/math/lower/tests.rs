use tex_state::math::MathListNode;
use tex_state::node::{Direction, Node};

use super::finish_math_lists_owned;

#[test]
fn math_lowering_retains_unchanged_ranges_without_source_copies() {
    crate::test_harness::with_nonstop_plain_universe(|universe| {
        let mut stores = universe.command_context().expect("test state is admitted");
        let math_content = stores.publish_page_nodes(vec![Node::Direction(Direction::BeginL)]);
        let math_source = stores
            .page_node_list(math_content)
            .expect("math content belongs to the page arena");
        let math_native_address =
            std::ptr::from_ref(math_source.owned_node(0).expect("native math node exists"));
        let source = stores.publish_page_nodes(vec![
            Node::Penalty(111),
            Node::MathList(MathListNode {
                display: false,
                content: math_content,
            }),
            Node::Penalty(222),
        ]);
        let source_view = stores
            .page_node_list(source)
            .expect("math source belongs to the page arena");
        let leading_address =
            std::ptr::from_ref(source_view.owned_node(0).expect("leading marker exists"));
        let trailing_address =
            std::ptr::from_ref(source_view.owned_node(2).expect("trailing marker exists"));
        let before = stores.page_node_arena_counters();

        let lowered = finish_math_lists_owned(
            &mut stores,
            &mut tex_state::diagnostic::DiagnosticEffects::new(),
            &mut crate::geometry::IgnorePackGeometry,
            source,
            true,
        );

        let after = stores.page_node_arena_counters();
        assert!(
            after.new_semantic_nodes > before.new_semantic_nodes,
            "inline math must append its genuinely new boundary nodes"
        );
        assert_eq!(
            after.source_nodes_copied, before.source_nodes_copied,
            "math lowering must retain unchanged source ranges by coordinate"
        );
        let lowered = stores
            .page_node_list(lowered)
            .expect("lowered math belongs to the page arena");
        assert_eq!(
            std::ptr::from_ref(lowered.owned_node(0).expect("leading marker retained")),
            leading_address
        );
        assert_eq!(
            std::ptr::from_ref(lowered.owned_node(4).expect("trailing marker retained")),
            trailing_address
        );
        assert!(matches!(lowered.owned_node(1), Some(Node::MathOn(_))));
        assert_eq!(
            std::ptr::from_ref(lowered.owned_node(2).expect("native math node retained")),
            math_native_address
        );
        assert!(matches!(lowered.owned_node(3), Some(Node::MathOff(_))));
    });
}
