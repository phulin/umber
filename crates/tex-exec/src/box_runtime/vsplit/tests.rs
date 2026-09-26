use super::*;
use tex_state::node::{GlueKind, GlueSpecOrigin};

#[test]
fn split_shrink_normalization_keeps_unaffected_nodes_and_candidate_glue() {
    crate::test_harness::with_nonstop_universe(|universe| {
        universe.set_interaction_mode(tex_state::InteractionMode::Nonstop);
        let mut stores = universe.command_context().expect("test state is admitted");
        let infinite = GlueSpec {
            shrink: Scaled::from_raw(1),
            shrink_order: Order::Fil,
            ..GlueSpec::ZERO
        };
        let finite = GlueSpec {
            shrink: Scaled::from_raw(2),
            ..GlueSpec::ZERO
        };
        let zero = GlueSpec {
            shrink_order: Order::Fill,
            ..GlueSpec::ZERO
        };
        let glue = |spec| Node::Glue {
            origin: GlueSpecOrigin::Owned,
            spec,
            kind: GlueKind::Normal,
            leader: None,
        };
        let nodes = vec![
            Node::Penalty(1),
            glue(infinite),
            Node::Penalty(2),
            glue(finite),
            Node::Penalty(3),
            glue(zero),
            Node::Penalty(4),
        ];
        let source = stores.publish_page_nodes(nodes.clone());
        let mut effects = DiagnosticEffects::new();
        let context = crate::diagnostics::ExecutionDiagnosticContext::default();

        let unchanged =
            normalize_split_infinite_shrink(&mut stores, source, &[], &context, &mut effects)
                .expect("no normalization needed");
        assert_eq!(
            unchanged, source,
            "no-op normalization keeps the source coordinate"
        );

        let normalized = normalize_split_infinite_shrink(
            &mut stores,
            source,
            &[1, 3, 5],
            &context,
            &mut effects,
        )
        .expect("infinite shrink can be normalized");
        let mut expected = nodes;
        expected[1] = glue(GlueSpec {
            shrink_order: Order::Normal,
            ..infinite
        });
        assert_eq!(
            stores
                .page_nodes(normalized)
                .expect("normalized source remains live")
                .iter()
                .cloned()
                .collect::<Vec<_>>(),
            expected,
            "only infinite nonzero shrink changes; candidate glue and both sides survive"
        );
    });
}
