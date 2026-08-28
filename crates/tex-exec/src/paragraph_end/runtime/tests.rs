use super::*;

fn node_addresses<G>(
    stores: &CommandContext<'_, G>,
    list: tex_state::node_arena::PageListId,
) -> Vec<*const Node> {
    stores
        .page_node_list(list)
        .expect("test list remains live")
        .nodes()
        .iter()
        .map(core::ptr::from_ref)
        .collect()
}

#[test]
fn production_post_line_retains_source_addresses_and_counts_no_copies() {
    crate::test_harness::with_nonstop_plain_universe(|universe| {
        let mut stores = universe.command_context().expect("test state is admitted");
        let source = stores.publish_page_nodes(vec![
            Node::Penalty(101),
            Node::Kern {
                amount: Scaled::from_raw(23),
                kind: KernKind::Explicit,
            },
            Node::Penalty(303),
        ]);
        let source_addresses = node_addresses(&stores, source);
        let before = stores.page_material_counters();
        let line_params = LineBreakParams {
            pretolerance: 10_000,
            tolerance: 10_000,
            line_penalty: 0,
            hyphen_penalty: 0,
            ex_hyphen_penalty: 0,
            adj_demerits: 0,
            double_hyphen_demerits: 0,
            final_hyphen_demerits: 0,
            emergency_stretch: Scaled::from_raw(0),
            looseness: 0,
            last_line_fit: 0,
            pdf_adjust_spacing: 0,
            expansion_steps: None,
            pdf_protrude_chars: 0,
            left_skip: GlueSpec::ZERO,
            right_skip: GlueSpec::ZERO,
            par_fill_skip: GlueSpec::ZERO,
            shape: LineShape::natural(Scaled::from_raw(1_000)),
        };
        let tape = ParagraphTape::analyze_arena_id(
            &crate::typeset_context::TypesetContext::new(&stores),
            source,
            &line_params,
        );
        let mut materializer = ArenaPostLineMaterializer::new(
            tape,
            vec![tex_typeset::linebreak::BreakDecision {
                position: source.len(),
                penalty: -10_000,
                hyphenated: false,
            }],
            PostLineBreakParams {
                empty_list: tex_state::node_arena::PageListId::empty(),
                left_skip: GlueSpec::ZERO,
                right_skip: GlueSpec::ZERO,
                interline_penalty: 0,
                club_penalty: 0,
                widow_penalties: tex_typeset::linebreak::WidowPenalties {
                    selector: tex_typeset::linebreak::WidowPenaltySelector::Ordinary,
                    ordinary: tex_typeset::linebreak::PenaltySequence {
                        fallback: 0,
                        values: Vec::new(),
                    },
                    display: tex_typeset::linebreak::PenaltySequence {
                        fallback: 0,
                        values: Vec::new(),
                    },
                },
                broken_penalty: 0,
                prev_graf: 0,
                interline_penalties: Vec::new(),
                club_penalties: Vec::new(),
                shape: LineShape::natural(Scaled::from_raw(1_000)),
            },
        );

        let line = materializer
            .materialize_next(&mut stores)
            .expect("the sole line materializes");
        assert!(line.diagnostic_nodes.is_none());
        assert_eq!(
            &node_addresses(&stores, line.nodes)[..source.len()],
            source_addresses.as_slice(),
            "unchanged post-line material keeps its published payload addresses"
        );
        assert_eq!(node_addresses(&stores, source), source_addresses);
        let after = stores.page_material_counters();
        assert_eq!(after.source_nodes_copied, 0);
        assert_eq!(
            after.new_semantic_nodes - before.new_semantic_nodes,
            1,
            "only the generated right skip is appended"
        );
        assert!(materializer.materialize_next(&mut stores).is_none());
    });
}
