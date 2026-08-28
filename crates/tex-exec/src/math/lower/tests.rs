use tex_state::glue::{GlueSpec, Order};
use tex_state::math::MathListNode;
use tex_state::node::{BoxLr, BoxNode, BoxNodeFields, Direction, GlueKind, KernKind, Node, Sign};
use tex_state::scaled::{GlueSetRatio, Scaled};
use tex_state::token::OriginId;

use super::finish_math_lists_owned;

#[test]
fn math_lowering_retains_unchanged_ranges_without_source_copies() {
    crate::test_harness::with_nonstop_plain_universe(|universe| {
        let mut stores = universe.command_context().expect("test state is admitted");
        let leaf_children = stores.publish_page_nodes(vec![Node::Penalty(17)]);
        let nested_box = Node::HList(box_node(leaf_children, 13));
        let box_children = stores.publish_page_nodes(vec![nested_box]);
        let leaf_address = std::ptr::from_ref(
            stores
                .page_node_list(leaf_children)
                .expect("leaf list belongs to the page arena")
                .owned_node(0)
                .expect("leaf node exists"),
        );
        let nested_box_address = std::ptr::from_ref(
            stores
                .page_node_list(box_children)
                .expect("box-child list belongs to the page arena")
                .owned_node(0)
                .expect("nested box exists"),
        );
        let font = stores.current_font();
        let math_content = stores.publish_page_nodes(vec![
            Node::Penalty(19),
            Node::Kern {
                amount: Scaled::from_raw(23),
                kind: KernKind::Explicit,
            },
            Node::Glue {
                spec: GlueSpec::ZERO,
                kind: GlueKind::Normal,
                leader: None,
            },
            Node::Rule {
                width: Some(Scaled::from_raw(29)),
                height: Some(Scaled::from_raw(31)),
                depth: Some(Scaled::from_raw(37)),
            },
            Node::Char {
                font,
                ch: 'x',
                origin: OriginId::UNKNOWN,
            },
            Node::HList(box_node(box_children, 41)),
            Node::VList(box_node(box_children, 43)),
            Node::Direction(Direction::BeginL),
        ]);
        let math_source = stores
            .page_node_list(math_content)
            .expect("math content belongs to the page arena");
        let native_addresses = (0..math_source.len())
            .map(|index| {
                std::ptr::from_ref(
                    math_source
                        .owned_node(index)
                        .expect("native math node exists"),
                )
            })
            .collect::<Vec<_>>();
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
        assert_eq!(
            after.new_semantic_nodes - before.new_semantic_nodes,
            2,
            "only the inline math boundary nodes are genuinely new"
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
        assert!(matches!(lowered.owned_node(1), Some(Node::MathOn(_))));
        for (offset, (category, expected)) in [
            "penalty",
            "non-mu kern",
            "non-mu glue",
            "rule",
            "character",
            "hlist and its nested box",
            "vlist and its nested box",
            "opaque native node",
        ]
        .into_iter()
        .zip(native_addresses)
        .enumerate()
        {
            assert_eq!(
                std::ptr::from_ref(
                    lowered
                        .owned_node(offset + 2)
                        .unwrap_or_else(|| panic!("{category} survives lowering")),
                ),
                expected,
                "{category} must retain its exact arena address"
            );
        }
        assert!(matches!(lowered.owned_node(10), Some(Node::MathOff(_))));
        for index in [7, 8] {
            let children = match lowered.owned_node(index) {
                Some(Node::HList(boxed) | Node::VList(boxed)) => boxed.children,
                other => panic!("source box survives at {index}: {other:?}"),
            };
            assert_eq!(children, box_children);
            let nested = stores
                .page_node_list(children)
                .expect("retained nested list remains live");
            assert_eq!(
                std::ptr::from_ref(nested.owned_node(0).expect("nested box remains live")),
                nested_box_address
            );
            let nested_children = match nested.owned_node(0) {
                Some(Node::HList(boxed)) => boxed.children,
                other => panic!("nested hlist survives: {other:?}"),
            };
            assert_eq!(nested_children, leaf_children);
            assert_eq!(
                std::ptr::from_ref(
                    stores
                        .page_node_list(nested_children)
                        .expect("retained leaf list remains live")
                        .owned_node(0)
                        .expect("retained leaf remains live"),
                ),
                leaf_address
            );
        }
        assert_eq!(
            std::ptr::from_ref(lowered.owned_node(11).expect("trailing marker retained")),
            trailing_address
        );
    });
}

#[test]
fn math_lowering_appends_exactly_the_mu_rewrites() {
    crate::test_harness::with_nonstop_plain_universe(|universe| {
        let mut stores = universe.command_context().expect("test state is admitted");
        let math_content = stores.publish_page_nodes(vec![
            Node::Kern {
                amount: Scaled::from_raw(2 * Scaled::UNITY),
                kind: KernKind::Mu,
            },
            Node::Glue {
                spec: GlueSpec {
                    width: Scaled::from_raw(3 * Scaled::UNITY),
                    ..GlueSpec::ZERO
                },
                kind: GlueKind::MuSkip,
                leader: None,
            },
        ]);
        let source = stores
            .page_node_list(math_content)
            .expect("math content belongs to the page arena");
        let source_addresses = [0, 1].map(|index| {
            std::ptr::from_ref(source.owned_node(index).expect("mu source node exists"))
        });
        let wrapper = stores.publish_page_nodes(vec![Node::MathList(MathListNode {
            display: false,
            content: math_content,
        })]);
        let before = stores.page_node_arena_counters();

        let lowered = finish_math_lists_owned(
            &mut stores,
            &mut tex_state::diagnostic::DiagnosticEffects::new(),
            &mut crate::geometry::IgnorePackGeometry,
            wrapper,
            true,
        );

        let after = stores.page_node_arena_counters();
        assert_eq!(
            after.new_semantic_nodes - before.new_semantic_nodes,
            4,
            "two boundary nodes plus the two mu rewrites are appended"
        );
        assert_eq!(
            after.source_nodes_copied, before.source_nodes_copied,
            "rewrites are generated nodes, not source republishes"
        );
        let lowered = stores
            .page_node_list(lowered)
            .expect("lowered math belongs to the page arena");
        for (index, source) in source_addresses.into_iter().enumerate() {
            assert_ne!(
                std::ptr::from_ref(lowered.owned_node(index + 1).expect("rewrite exists")),
                source,
                "mu rewrite must have a fresh arena address"
            );
        }
        assert!(matches!(
            lowered.owned_node(1),
            Some(Node::Kern {
                kind: KernKind::Explicit,
                ..
            })
        ));
        assert!(matches!(
            lowered.owned_node(2),
            Some(Node::Glue {
                kind: GlueKind::Normal,
                ..
            })
        ));
    });
}

fn box_node(children: tex_state::node_arena::PageListId, width: i32) -> BoxNode {
    BoxNode::new(BoxNodeFields {
        width: Scaled::from_raw(width),
        height: Scaled::from_raw(5),
        depth: Scaled::from_raw(3),
        shift: Scaled::from_raw(0),
        box_lr: BoxLr::Normal,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children,
    })
}
