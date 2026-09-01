use super::*;

fn node_addresses<G>(
    stores: &CommandContext<'_, G>,
    list: tex_state::node_arena::PageListId,
) -> Vec<*const Node> {
    let nodes = stores
        .page_node_list(list)
        .expect("test list remains live")
        .nodes();
    (0..nodes.len())
        .map(|index| {
            nodes
                .testing_node_address(index)
                .expect("test node remains live")
        })
        .collect()
}

fn test_line_break_params(width: i32) -> LineBreakParams {
    LineBreakParams {
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
        shape: LineShape::natural(Scaled::from_raw(width)),
    }
}

fn test_post_line_break_params(width: i32) -> PostLineBreakParams {
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
        shape: LineShape::natural(Scaled::from_raw(width)),
    }
}

fn traversal_delta(
    after: tex_state::node_arena::NodeTraversalCounters,
    before: tex_state::node_arena::NodeTraversalCounters,
) -> tex_state::node_arena::NodeTraversalCounters {
    tex_state::node_arena::NodeTraversalCounters {
        index_resolutions: after
            .index_resolutions
            .saturating_sub(before.index_resolutions),
        index_predecessor_steps: after
            .index_predecessor_steps
            .saturating_sub(before.index_predecessor_steps),
        forward_chunk_crossings: after
            .forward_chunk_crossings
            .saturating_sub(before.forward_chunk_crossings),
    }
}

fn normalize_test_paragraph<G>(
    stores: &mut CommandContext<'_, G>,
    source: tex_state::node_arena::PageListId,
) -> tex_state::node_arena::PageListId {
    let mut params = snapshot_paragraph_params(&ModeNest::new(), stores);
    let mut effects = DiagnosticEffects::new();
    normalize_paragraph_infinite_shrink(
        stores,
        &mut params,
        source,
        false,
        &crate::pack_report::ExecutionDiagnosticContext::source_free(""),
        &mut effects,
    )
    .expect("paragraph glue normalization succeeds")
}

fn normalize_test_paragraph_indexed_reference<G>(
    stores: &mut CommandContext<'_, G>,
    source: tex_state::node_arena::PageListId,
) -> tex_state::node_arena::PageListId {
    // Preserve the removed indexed loop only as an explicit perf-stat baseline;
    // production callers never select this path.
    let mut params = snapshot_paragraph_params(&ModeNest::new(), stores);
    let mut effects = DiagnosticEffects::new();
    let context = crate::pack_report::ExecutionDiagnosticContext::source_free("");
    let mut reported = false;
    normalize_paragraph_glue(
        stores,
        &mut params.left_skip,
        false,
        &context,
        &mut effects,
        &mut reported,
    )
    .expect("left skip normalization succeeds");
    normalize_paragraph_glue(
        stores,
        &mut params.right_skip,
        false,
        &context,
        &mut effects,
        &mut reported,
    )
    .expect("right skip normalization succeeds");
    let mut output = tex_state::page_node_arena::PageMaterialActiveListBuilder::default();
    stores.open_page_active_list(&mut output);
    for index in 0..source.len() {
        let replacement = match stores
            .page_node_list(source)
            .expect("reference paragraph remains live")
            .nodes()
            .get(index)
        {
            Some(tex_state::NodeView::Glue { spec, kind, leader })
                if spec.shrink.raw() != 0 && spec.shrink_order != Order::Normal =>
            {
                Some((spec, kind, leader))
            }
            _ => None,
        };
        if let Some((mut spec, kind, leader)) = replacement {
            normalize_paragraph_glue(
                stores,
                &mut spec,
                false,
                &context,
                &mut effects,
                &mut reported,
            )
            .expect("node glue normalization succeeds");
            stores.push_page_active_list(&mut output, Node::Glue { spec, kind, leader });
        } else {
            stores.append_page_active_list_range(&mut output, source, index..index + 1);
        }
    }
    stores.finalize_page_active_list(&mut output)
}

fn paragraph_normalization_perf_evidence(indexed_reference: bool) {
    const VALUES: usize = 4_096;
    const REPEATS: usize = 128;

    crate::test_harness::with_nonstop_plain_universe(|universe| {
        let mut stores = universe.command_context().expect("test state is admitted");
        let source = stores.publish_page_nodes(vec![Node::Penalty(17); VALUES]);
        let counters_before = stores.page_material_counters();
        let mut checksum = 0_usize;
        for _ in 0..REPEATS {
            let output = if indexed_reference {
                normalize_test_paragraph_indexed_reference(&mut stores, source)
            } else {
                normalize_test_paragraph(&mut stores, source)
            };
            checksum = checksum.wrapping_add(std::hint::black_box(output.len()));
        }
        let counters_after = stores.page_material_counters();
        let copied = counters_after.source_nodes_copied - counters_before.source_nodes_copied;
        assert_eq!(checksum, VALUES * REPEATS);
        assert_eq!(
            copied,
            if indexed_reference {
                (VALUES * REPEATS) as u64
            } else {
                0
            }
        );
        eprintln!(
            "PARAGRAPH_NORMALIZATION_PERF indexed_reference={indexed_reference} values={VALUES} repeats={REPEATS} source_nodes_copied={copied}"
        );
    });
}

#[test]
#[ignore = "focused perf-stat diagnostic"]
fn paragraph_normalization_direct_perf_evidence() {
    paragraph_normalization_perf_evidence(false);
}

#[test]
#[ignore = "focused perf-stat diagnostic"]
fn paragraph_normalization_indexed_reference_perf_evidence() {
    paragraph_normalization_perf_evidence(true);
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
        let line_params = test_line_break_params(1_000);
        let tape = ParagraphTape::analyze_arena_id(
            &crate::typeset_context::TypesetContext::new(&stores),
            source,
            &line_params,
        );
        let mut materializer = ArenaPostLineMaterializer::new(
            &stores,
            tape,
            vec![tex_typeset::linebreak::BreakDecision {
                position: source.len(),
                penalty: -10_000,
                hyphenated: false,
            }],
            test_post_line_break_params(1_000),
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

#[test]
fn production_post_line_discards_an_explicit_kern_chosen_as_a_break() {
    // TeX82 §§822/866 chooses the breakpoint before an explicit kern and
    // discards both that kern and the following glue during post-line breakup.
    crate::test_harness::with_nonstop_plain_universe(|universe| {
        let mut stores = universe.command_context().expect("test state is admitted");
        let rule = || Node::Rule {
            width: Some(Scaled::from_raw(10)),
            height: Some(Scaled::from_raw(1)),
            depth: Some(Scaled::from_raw(0)),
        };
        let source = stores.publish_page_nodes(vec![
            rule(),
            Node::Kern {
                amount: Scaled::from_raw(5),
                kind: KernKind::Explicit,
            },
            Node::Glue {
                spec: GlueSpec::ZERO,
                kind: GlueKind::Normal,
                leader: None,
            },
            rule(),
        ]);
        let line_params = test_line_break_params(10);
        let tape = ParagraphTape::analyze_arena_id(
            &crate::typeset_context::TypesetContext::new(&stores),
            source,
            &line_params,
        );
        let mut materializer = ArenaPostLineMaterializer::new(
            &stores,
            tape,
            vec![
                tex_typeset::linebreak::BreakDecision {
                    position: 2,
                    penalty: 0,
                    hyphenated: false,
                },
                tex_typeset::linebreak::BreakDecision {
                    position: source.len(),
                    penalty: -10_000,
                    hyphenated: false,
                },
            ],
            test_post_line_break_params(10),
        );

        let first = materializer
            .materialize_next(&mut stores)
            .expect("the first line materializes");
        let first_nodes = stores
            .page_node_list(first.nodes)
            .expect("the first line remains live")
            .nodes()
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        assert!(matches!(
            first_nodes.as_slice(),
            [
                Node::Rule { .. },
                Node::Glue {
                    kind: GlueKind::RightSkip,
                    ..
                }
            ]
        ));

        let second = materializer
            .materialize_next(&mut stores)
            .expect("the second line materializes");
        let second_nodes = stores
            .page_node_list(second.nodes)
            .expect("the second line remains live")
            .nodes()
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        assert!(matches!(
            second_nodes.as_slice(),
            [
                Node::Rule { .. },
                Node::Glue {
                    kind: GlueKind::RightSkip,
                    ..
                }
            ]
        ));
        assert!(materializer.materialize_next(&mut stores).is_none());
    });
}

#[test]
fn final_font_expansion_includes_the_preceding_margin_kern_width() {
    // pdftex.web §1061 inserts marginal kerns before §822's final
    // `hpack(..., cal_expand_ratio)`, so their negative width participates in
    // the ratio that §823 uses to select expanded fonts.
    crate::test_harness::with_nonstop_plain_universe(|universe| {
        let mut stores = universe.command_context().expect("test state is admitted");
        let mut characters = vec![None; 256];
        for code in [b'A', b'.'] {
            characters[usize::from(code)] = Some(tex_fonts::CharMetrics {
                width: Scaled::from_raw(1_000),
                height: Scaled::from_raw(0),
                depth: Scaled::from_raw(0),
                italic_correction: Scaled::from_raw(0),
                tag: tex_fonts::metrics::CharTag::None,
            });
        }
        let mut parameters = vec![Scaled::from_raw(0); 7];
        parameters[5] = Scaled::from_raw(1_000);
        let font = stores.intern_font(tex_fonts::LoadedFont::new(
            "post-line-expansion",
            "post-line-expansion.tfm",
            [17; 8],
            0,
            Scaled::from_raw(1_000),
            Scaled::from_raw(1_000),
            parameters,
            tex_fonts::FontMetrics::new(characters, Vec::new(), None, None, Vec::new()),
        ));
        stores
            .configure_font_expansion(
                font,
                tex_state::font::FontExpansion {
                    stretch: 20,
                    shrink: 20,
                    step: 1,
                    auto_expand: true,
                },
            )
            .expect("font expansion configuration is valid");
        for code in [b'A', b'.'] {
            stores.set_pdf_font_code(PdfFontCode::Ef, font, code, 1_000);
        }
        stores.set_pdf_font_code(PdfFontCode::Rp, font, b'.', 100);

        let mut source_nodes = vec![
            Node::Char {
                font,
                ch: 'A',
                origin: tex_state::token::OriginId::UNKNOWN,
            };
            99
        ];
        source_nodes.push(Node::Char {
            font,
            ch: '.',
            origin: tex_state::token::OriginId::UNKNOWN,
        });
        let source = stores.publish_page_nodes(source_nodes);
        let target = Scaled::from_raw(100_108);

        let without_protrusion =
            materialize_pdf_line_list(&mut stores, source, 0, target, true, false)
                .expect("ordinary expansion materializes");
        let without_font = match stores
            .page_node_list(without_protrusion)
            .expect("ordinary expanded line remains live")
            .nodes()
            .get(0)
            .expect("ordinary expanded line has a first glyph")
        {
            tex_state::NodeView::Char { font, .. } => font,
            node => panic!("expected a character, got {node:?}"),
        };
        assert!(matches!(
            stores.font_construction(without_font),
            tex_fonts::FontConstruction::Expanded { ratio: 1, .. }
        ));

        let protruded = materialize_pdf_line_list(&mut stores, source, 0, target, true, true)
            .expect("protruded expansion materializes");
        let protruded_nodes = stores
            .page_node_list(protruded)
            .expect("protruded expanded line remains live")
            .nodes()
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(protruded_nodes.len(), 101);
        for node in protruded_nodes {
            let expanded = match node {
                Node::Char { font, .. } | Node::MarginKern { font, .. } => font,
                node => panic!("expected character or margin kern, got {node:?}"),
            };
            assert!(matches!(
                stores.font_construction(expanded),
                tex_fonts::FontConstruction::Expanded { ratio: 2, .. }
            ));
        }
    });
}

#[test]
fn paragraph_glue_normalization_walks_large_unchanged_lists_by_direct_chunks() {
    for values in [1_usize, 4_096] {
        crate::test_harness::with_nonstop_plain_universe(|universe| {
            let mut stores = universe.command_context().expect("test state is admitted");
            let source = stores.publish_page_nodes(vec![Node::Penalty(17); values]);
            let source_addresses = node_addresses(&stores, source);
            let source_span = stores
                .admit_page_node_span(source)
                .expect("test paragraph source remains live");
            let nodes = stores
                .page_node_span(source_span)
                .expect("test paragraph span remains admitted");

            let reference_before = nodes.testing_traversal_counters();
            let mut reference_visits = 0;
            nodes.for_each_range(0..nodes.len(), |_, _| reference_visits += 1);
            let reference = traversal_delta(nodes.testing_traversal_counters(), reference_before);
            assert_eq!(reference_visits, values);

            let traversal_before = nodes.testing_traversal_counters();
            let counters_before = stores.page_material_counters();
            let _ = nodes;
            let output = normalize_test_paragraph(&mut stores, source);
            let traversal = traversal_delta(
                stores
                    .page_node_span(source_span)
                    .expect("source remains live after normalization")
                    .testing_traversal_counters(),
                traversal_before,
            );
            let counters_after = stores.page_material_counters();

            assert_eq!(traversal.index_resolutions, 0);
            assert_eq!(traversal.index_predecessor_steps, 0);
            assert_eq!(
                traversal.forward_chunk_crossings,
                reference.forward_chunk_crossings
            );
            assert_eq!(
                counters_after.source_nodes_copied,
                counters_before.source_nodes_copied
            );
            assert_eq!(
                counters_after.new_semantic_nodes,
                counters_before.new_semantic_nodes
            );
            assert_eq!(
                output, source,
                "an unchanged paragraph keeps its exact root"
            );
            assert_eq!(node_addresses(&stores, output), source_addresses);
            assert_eq!(node_addresses(&stores, source), source_addresses);
            eprintln!(
                "PARAGRAPH_NORMALIZATION_DIRECT_SCALE values={values} sequential_index_resolutions={} sequential_predecessor_steps={} forward_block_crossings={} source_nodes_copied=0 new_semantic_nodes=0",
                traversal.index_resolutions,
                traversal.index_predecessor_steps,
                traversal.forward_chunk_crossings,
            );
        });
    }
}

#[test]
fn paragraph_glue_normalization_preserves_an_appended_shared_prefix() {
    crate::test_harness::with_nonstop_plain_universe(|universe| {
        let mut stores = universe.command_context().expect("test state is admitted");
        let prefix = stores.publish_page_nodes(vec![Node::Penalty(17); 4_095]);
        let prefix_span = stores
            .admit_page_node_span(prefix)
            .expect("shared paragraph prefix remains live");
        let prefix_addresses = node_addresses(&stores, prefix);
        let suffix = stores.publish_unique_page_nodes(vec![Node::Penalty(23)]);
        let source = stores.append_unique_page_nodes(prefix_span, suffix);
        let counters_before = stores.page_material_counters();

        let output = normalize_test_paragraph(&mut stores, source.list());
        let counters_after = stores.page_material_counters();

        assert_eq!(output, source.list());
        assert_eq!(
            counters_after.source_nodes_copied,
            counters_before.source_nodes_copied
        );
        assert_eq!(
            counters_after.new_semantic_nodes,
            counters_before.new_semantic_nodes
        );
        assert_eq!(node_addresses(&stores, prefix), prefix_addresses);
        assert_eq!(
            &node_addresses(&stores, output)[..prefix.len()],
            prefix_addresses.as_slice(),
            "the appended paragraph keeps the sealed shared prefix address-stable"
        );
    });
}

#[test]
fn paragraph_glue_normalization_retains_source_across_interleaved_output_appends() {
    const VALUES: usize = 4_096;
    const OFFENDING_STEP: usize = 257;

    crate::test_harness::with_nonstop_plain_universe(|universe| {
        let mut stores = universe.command_context().expect("test state is admitted");
        let infinite = GlueSpec {
            shrink: Scaled::from_raw(1),
            shrink_order: Order::Fil,
            ..GlueSpec::ZERO
        };
        let source_nodes = (0..VALUES)
            .map(|index| {
                if index.is_multiple_of(OFFENDING_STEP) {
                    Node::Glue {
                        spec: infinite,
                        kind: GlueKind::Normal,
                        leader: None,
                    }
                } else {
                    Node::Penalty(index as i32)
                }
            })
            .collect::<Vec<_>>();
        let offending = source_nodes
            .iter()
            .filter(|node| matches!(node, Node::Glue { .. }))
            .count();
        let source = stores.publish_page_nodes(source_nodes.clone());
        let source_span = stores
            .admit_page_node_span(source)
            .expect("test paragraph source remains live");
        let traversal_before = stores
            .page_node_span(source_span)
            .expect("test paragraph span remains admitted")
            .testing_traversal_counters();
        let counters_before = stores.page_material_counters();

        let output = normalize_test_paragraph(&mut stores, source);
        let traversal = traversal_delta(
            stores
                .page_node_span(source_span)
                .expect("source remains live after normalization")
                .testing_traversal_counters(),
            traversal_before,
        );
        let counters_after = stores.page_material_counters();
        let normalized = stores
            .page_node_list(output)
            .expect("normalized paragraph remains live")
            .nodes()
            .iter()
            .cloned()
            .collect::<Vec<_>>();

        assert_eq!(normalized.len(), VALUES);
        assert_eq!(
            traversal.index_resolutions, 0,
            "direct shared copying adds no indexed source work"
        );
        assert_eq!(
            counters_after.new_semantic_nodes - counters_before.new_semantic_nodes,
            offending as u64
        );
        assert_eq!(
            counters_after.source_nodes_copied - counters_before.source_nodes_copied,
            (VALUES - offending) as u64,
            "only unchanged source material takes the explicit counted-copy path"
        );
        for (index, (before, after)) in source_nodes.iter().zip(&normalized).enumerate() {
            match (before, after) {
                (Node::Glue { spec: old, .. }, Node::Glue { spec: new, .. })
                    if index.is_multiple_of(OFFENDING_STEP) =>
                {
                    assert_eq!(old.shrink_order, Order::Fil);
                    assert_eq!(new.shrink_order, Order::Normal);
                    assert_eq!(new.shrink, old.shrink);
                }
                _ => assert_eq!(before, after),
            }
        }
        assert_eq!(
            stores
                .page_node_list(source)
                .expect("source remains live")
                .nodes()
                .iter()
                .cloned()
                .collect::<Vec<_>>(),
            source_nodes,
            "output appends never mutate the admitted source or its shared prefix"
        );
    });
}

#[cfg(feature = "profiling")]
#[test]
fn warmed_large_unchanged_paragraph_normalization_allocates_and_copies_nothing() {
    crate::test_harness::with_nonstop_plain_universe(|universe| {
        let mut stores = universe.command_context().expect("test state is admitted");
        let source = stores.publish_page_nodes(vec![Node::Penalty(17); 4_096]);
        let _warm = normalize_test_paragraph(&mut stores, source);
        let mut params = snapshot_paragraph_params(&ModeNest::new(), &mut stores);
        let mut effects = DiagnosticEffects::new();
        let context = crate::pack_report::ExecutionDiagnosticContext::source_free("");
        let owner = tex_state::measurement::HotCoreAllocationOwner::SemanticApply;
        let before = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
        let counters_before = stores.page_material_counters();
        let output = {
            let _scope = tex_state::measurement::hot_core_allocation_scope(owner);
            normalize_paragraph_infinite_shrink(
                &mut stores,
                &mut params,
                source,
                false,
                &context,
                &mut effects,
            )
            .expect("measured paragraph glue normalization succeeds")
        };
        let after = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
        let counters_after = stores.page_material_counters();

        assert_eq!(output.len(), source.len());
        assert_eq!(after.calls - before.calls, 0);
        assert_eq!(after.requested_bytes - before.requested_bytes, 0);
        assert_eq!(
            counters_after.source_nodes_copied,
            counters_before.source_nodes_copied
        );
        assert_eq!(
            counters_after.new_semantic_nodes,
            counters_before.new_semantic_nodes
        );
        eprintln!(
            "PARAGRAPH_NORMALIZATION_WARMED values={} allocation_calls={} allocation_bytes={} source_nodes_copied={} new_semantic_nodes={}",
            source.len(),
            after.calls - before.calls,
            after.requested_bytes - before.requested_bytes,
            counters_after.source_nodes_copied - counters_before.source_nodes_copied,
            counters_after.new_semantic_nodes - counters_before.new_semantic_nodes,
        );
    });
}
