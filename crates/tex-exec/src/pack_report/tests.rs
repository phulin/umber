//! TeX82 §§54/660/663 packed-box diagnostic routing regressions.

use super::*;
use tex_fonts::{FontMetrics, LoadedFont};
use tex_state::EffectRecord;
use tex_state::diagnostic::DiagnosticEffects;
use tex_state::env::banks::IntParam;
use tex_state::glue::{GlueSpec, Order};
use tex_state::node::{AdjustNode, BoxNode, BoxNodeFields, DiscKind, GlueKind, KernKind, Sign};
use tex_state::scaled::{GlueSetRatio, Scaled};

fn empty_hbox<G>(_stores: &mut CommandContext<'_, G>) -> Node {
    Node::HList(BoxNode::new(BoxNodeFields {
        width: Scaled::from_raw(0),
        height: Scaled::from_raw(0),
        depth: Scaled::from_raw(0),
        shift: Scaled::from_raw(0),
        box_lr: tex_state::node::BoxLr::Normal,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children: tex_state::node_arena::PageListId::empty(),
    }))
}

fn sink_text<G>(stores: &tex_state::Universe<G>, terminal: bool) -> String {
    stores
        .world()
        .effect_records()
        .iter()
        .filter_map(|effect| match effect {
            EffectRecord::StreamWrite { sink, text }
                if if terminal {
                    matches!(
                        sink,
                        tex_state::PrintSink::Terminal | tex_state::PrintSink::TerminalAndLog
                    )
                } else {
                    matches!(
                        sink,
                        tex_state::PrintSink::Log | tex_state::PrintSink::TerminalAndLog
                    )
                } =>
            {
                Some(text.as_str())
            }
            _ => None,
        })
        .collect()
}

#[test]
fn short_display_skips_the_physical_discretionary_replacement_count() {
    crate::test_harness::with_nonstop_universe(|universe| {
        let mut stores = universe.command_context().expect("test state is admitted");
        let empty = tex_state::node_arena::PageListId::empty();
        let replacement = stores.publish_page_nodes(vec![Node::Kern {
            amount: Scaled::from_raw(Scaled::UNITY),
            kind: KernKind::Explicit,
        }]);
        let space = GlueSpec {
            width: Scaled::from_raw(Scaled::UNITY),
            ..GlueSpec::ZERO
        };
        let mut nodes = vec![
            empty_hbox(&mut stores),
            empty_hbox(&mut stores),
            empty_hbox(&mut stores),
        ];
        for _ in 0..3 {
            nodes.push(Node::Glue {
                spec: space,
                kind: GlueKind::Normal,
                leader: None,
            });
            nodes.push(empty_hbox(&mut stores));
        }
        nodes.extend([
            Node::Disc {
                kind: DiscKind::Discretionary,
                pre: empty,
                post: empty,
                replace: replacement,
                physical_replace_count: 1,
            },
            Node::Rule {
                width: None,
                height: None,
                depth: None,
            },
            Node::Rule {
                width: None,
                height: None,
                depth: None,
            },
        ]);

        let list = stores.publish_page_nodes(nodes);
        assert_eq!(
            ShortDisplayRenderer::new().render_list(&stores, list),
            "[][][] [] [] []|"
        );
    });
}

#[test]
fn short_display_retains_rule_after_nonphysical_discretionary_replacement() {
    // TeX82 §174 uses the disc node's `replace_count`, not the length of its
    // replacement side list. This is the source shape produced by the TRIP
    // display's `\discretionary{...}{...}{...}` after math conversion: the
    // immutable replacement remains available, but no replacement node is
    // linked after the disc, and the following rule must print as `|`.
    crate::test_harness::with_nonstop_universe(|universe| {
        let mut stores = universe.command_context().expect("test state is admitted");
        let empty = tex_state::node_arena::PageListId::empty();
        let replacement = stores.publish_page_nodes(vec![Node::Kern {
            amount: Scaled::from_raw(Scaled::UNITY),
            kind: KernKind::Explicit,
        }]);
        let space = GlueSpec {
            width: Scaled::from_raw(Scaled::UNITY),
            ..GlueSpec::ZERO
        };
        let mut nodes = vec![
            empty_hbox(&mut stores),
            empty_hbox(&mut stores),
            empty_hbox(&mut stores),
        ];
        for _ in 0..3 {
            nodes.push(Node::Glue {
                spec: space,
                kind: GlueKind::Normal,
                leader: None,
            });
            nodes.push(empty_hbox(&mut stores));
        }
        nodes.extend([
            Node::Disc {
                kind: DiscKind::Discretionary,
                pre: empty,
                post: empty,
                replace: replacement,
                physical_replace_count: 0,
            },
            Node::Rule {
                width: None,
                height: None,
                depth: None,
            },
        ]);

        let list = stores.publish_page_nodes(nodes);
        assert_eq!(
            ShortDisplayRenderer::new().render_list(&stores, list),
            "[][][] [] [] []|"
        );
    });
}

#[test]
fn short_display_physical_count_is_independent_of_empty_side_list() {
    // Counterexample in the other direction: physical replacement nodes are
    // skipped even when this frozen representation no longer needs to retain
    // their source side list.
    crate::test_harness::with_nonstop_universe(|universe| {
        let mut stores = universe.command_context().expect("test state is admitted");
        let empty = tex_state::node_arena::PageListId::empty();
        let nodes = [
            Node::Disc {
                kind: DiscKind::Discretionary,
                pre: empty,
                post: empty,
                replace: empty,
                physical_replace_count: 1,
            },
            Node::Rule {
                width: None,
                height: None,
                depth: None,
            },
            Node::Rule {
                width: None,
                height: None,
                depth: None,
            },
        ];

        let list = stores.publish_page_nodes(nodes.into());
        assert_eq!(ShortDisplayRenderer::new().render_list(&stores, list), "|");
    });
}

#[test]
fn line_trace_projection_renders_detached_replacement_content() {
    // TRIP's line trace supplies a detached slice: the side list is empty
    // while its three replacement characters remain in the displayed
    // projection. Applying the frozen list's physical count would incorrectly
    // reduce TeX82's `B-BBB` to `B-B`.
    crate::test_harness::with_nonstop_universe(|universe| {
        let mut stores = universe.command_context().expect("test state is admitted");
        let font = tex_state::font::NULL_FONT;
        let chars = |text: &str| {
            text.chars()
                .map(|ch| Node::Char {
                    font,
                    ch,
                    origin: tex_state::token::OriginId::UNKNOWN,
                })
                .collect::<Vec<_>>()
        };
        let pre = stores.publish_page_nodes(chars("B-"));
        let empty = tex_state::node_arena::PageListId::empty();
        let mut nodes = vec![Node::Disc {
            kind: DiscKind::Discretionary,
            pre,
            post: empty,
            replace: empty,
            physical_replace_count: 3,
        }];
        nodes.extend(chars("BBB"));

        assert_eq!(
            ShortDisplayRenderer::new().render_nodes(&stores, &nodes),
            format!("{} B-BBB", crate::node_dump::font_identifier(&stores, font))
        );
    });
}

#[test]
fn frozen_line_diagnostic_renders_both_disc_branches_then_skips_replacement() {
    // TeX82 §174 renders pre_break and post_break before advancing over the
    // physically linked replacement. This is the remaining TRIP underfull
    // line shape: `BB`, then `-`/`B-`, one hidden replacement node, then
    // `BBB`, yielding exactly `BB-B-BBB`.
    crate::test_harness::with_nonstop_universe(|universe| {
        let mut stores = universe.command_context().expect("test state is admitted");
        let font = tex_state::font::NULL_FONT;
        let chars = |text: &str| {
            text.chars()
                .map(|ch| Node::Char {
                    font,
                    ch,
                    origin: tex_state::token::OriginId::UNKNOWN,
                })
                .collect::<Vec<_>>()
        };
        let pre = stores.publish_page_nodes(chars("-"));
        let post = stores.publish_page_nodes(chars("B-"));
        let replacement = stores.publish_page_nodes(chars("X"));
        let mut nodes = chars("BB");
        nodes.push(Node::Disc {
            kind: DiscKind::AutomaticHyphen,
            pre,
            post,
            replace: replacement,
            physical_replace_count: 1,
        });
        nodes.extend(chars("XBBB"));
        let list = stores.publish_page_nodes(nodes);

        assert_eq!(
            ShortDisplayRenderer::new().render_list(&stores, list),
            format!(
                "{} BB-B-BBB",
                crate::node_dump::font_identifier(&stores, font)
            )
        );
    });
}

#[test]
fn short_display_maps_all_node_classes() {
    crate::test_harness::with_nonstop_universe(|universe| {
        let mut stores = universe.command_context().expect("test state is admitted");
        let empty = tex_state::node_arena::PageListId::empty();
        let zero_glue = GlueSpec::ZERO;
        let nonzero_glue = GlueSpec {
            width: Scaled::from_raw(Scaled::UNITY),
            ..GlueSpec::ZERO
        };
        let mark_tokens = tex_state::node::NodeTokenList::default();
        let pre = stores.publish_page_nodes(vec![Node::Kern {
            amount: Scaled::from_raw(Scaled::UNITY),
            kind: KernKind::Explicit,
        }]);
        let post = stores.publish_page_nodes(vec![Node::Kern {
            amount: Scaled::from_raw(2 * Scaled::UNITY),
            kind: KernKind::Explicit,
        }]);
        let nodes = [
            empty_hbox(&mut stores),
            Node::Rule {
                width: None,
                height: None,
                depth: None,
            },
            Node::Glue {
                spec: zero_glue,
                kind: GlueKind::Normal,
                leader: None,
            },
            Node::Glue {
                spec: nonzero_glue,
                kind: GlueKind::Normal,
                leader: None,
            },
            Node::Direction(tex_state::node::Direction::BeginL),
            Node::MathOn(Scaled::from_raw(0)),
            Node::Mark {
                class: 0,
                tokens: mark_tokens,
            },
            Node::Adjust(AdjustNode {
                content: empty,
                pre: false,
            }),
            Node::Disc {
                kind: DiscKind::Discretionary,
                pre,
                post,
                replace: empty,
                physical_replace_count: 0,
            },
            Node::Penalty(100),
            Node::Kern {
                amount: Scaled::from_raw(3 * Scaled::UNITY),
                kind: KernKind::Explicit,
            },
        ];

        assert_eq!(
            ShortDisplayRenderer::new().render_nodes(&stores, &nodes),
            "[]| []$[][]"
        );
    });
}

#[test]
fn etex_direction_nodes_follow_short_display_subtypes_without_mutation() {
    crate::test_harness::with_nonstop_universe(|universe| {
        let stores = universe.command_context().expect("test state is admitted");
        let nodes = [
            Node::Direction(tex_state::node::Direction::BeginM),
            Node::Direction(tex_state::node::Direction::EndM),
            Node::Direction(tex_state::node::Direction::BeginL),
            Node::Direction(tex_state::node::Direction::EndL),
            Node::Direction(tex_state::node::Direction::BeginR),
            Node::Direction(tex_state::node::Direction::EndR),
        ];

        assert_eq!(
            ShortDisplayRenderer::new().render_nodes(&stores, &nodes),
            "$$[][][][]"
        );
        assert_eq!(
            nodes[2],
            Node::Direction(tex_state::node::Direction::BeginL)
        );
    });
}

#[test]
fn short_display_renderer_retains_font_across_fragments_until_reset() {
    // TeX82 §§174/851: one line-breaking pass initializes
    // `font_in_short_display` once, then successive feasible-break fragments
    // omit an unchanged font identifier. The next pass resets it.
    crate::test_harness::with_nonstop_universe(|universe| {
        let stores = universe.command_context().expect("test state is admitted");
        let font = tex_state::font::NULL_FONT;
        let fragment = [Node::Char {
            font,
            ch: 'A',
            origin: tex_state::token::OriginId::UNKNOWN,
        }];
        let identifier = crate::node_dump::font_identifier(&stores, font);
        let mut renderer = ShortDisplayRenderer::new();

        assert_eq!(
            renderer.render_nodes(&stores, &fragment),
            format!("{identifier} A")
        );
        assert_eq!(renderer.render_nodes(&stores, &fragment), "A");
        renderer.reset();
        assert_eq!(
            renderer.render_nodes(&stores, &fragment),
            format!("{identifier} A")
        );
    });
}

#[test]
fn short_display_uses_print_ascii_for_eight_bit_character_codes() {
    crate::test_harness::with_nonstop_universe(|universe| {
        let stores = universe.command_context().expect("test state is admitted");
        let font = tex_state::font::NULL_FONT;
        let nodes = [
            Node::Lig {
                font,
                ch: '\u{82}',
                orig: vec!['C', 'A'],
                origins: vec![tex_state::token::OriginId::UNKNOWN; 2],
                left_hit: false,
                right_hit: false,
            },
            Node::Char {
                font,
                ch: '\u{82}',
                origin: tex_state::token::OriginId::UNKNOWN,
            },
        ];

        assert_eq!(
            ShortDisplayRenderer::new().render_nodes(&stores, &nodes),
            format!(
                "{} CA^^82",
                crate::node_dump::font_identifier(&stores, font)
            )
        );
    });
}

#[test]
fn short_display_honors_live_newline_character() {
    // TeX82 §§59/174: `short_display` prints a character node through the
    // one-character string path, where `\newlinechar` is recognized before
    // the `^^` spelling for an unprintable character.
    crate::test_harness::with_nonstop_universe(|universe| {
        let mut stores = universe.command_context().expect("test state is admitted");
        stores
            .assign_int_param(
                IntParam::NEWLINE_CHAR,
                10,
                tex_state::AssignmentScope::Global,
            )
            .expect("parameter");
        let font = tex_state::font::NULL_FONT;
        let nodes = [Node::Char {
            font,
            ch: '\n',
            origin: tex_state::token::OriginId::UNKNOWN,
        }];

        assert_eq!(
            ShortDisplayRenderer::new().render_nodes(&stores, &nodes),
            format!("{} \n", crate::node_dump::font_identifier(&stores, font))
        );

        stores
            .assign_int_param(
                IntParam::NEWLINE_CHAR,
                -1,
                tex_state::AssignmentScope::Global,
            )
            .expect("parameter");
        assert_eq!(
            ShortDisplayRenderer::new().render_nodes(&stores, &nodes),
            format!("{} ^^J", crate::node_dump::font_identifier(&stores, font))
        );
    });
}

#[test]
fn short_display_renders_byte_zero_in_a_font_identifier_through_print() {
    // TeX82 §§58--60/174: `print_esc(font_id_text(f))` sends every character
    // in the control-sequence name through `print`. Byte zero is therefore a
    // line break when `\newlinechar=0`, and `^^@` when new lines are disabled;
    // it is never a raw NUL in the completed diagnostic.
    let size = Scaled::from_raw(10 * Scaled::UNITY);
    let loaded = LoadedFont::new(
        "bigtr\0p",
        "fixture.tfm",
        [0; 32],
        0,
        size,
        size,
        vec![Scaled::from_raw(0); 7],
        FontMetrics::new(Vec::new(), Vec::new(), None, None, Vec::new()),
    );
    crate::test_harness::with_nonstop_plain_universe(|universe| {
        let mut stores = universe.command_context().expect("test state is admitted");
        let font = stores.intern_font(loaded);
        let nodes = [Node::Char {
            font,
            ch: '-',
            origin: tex_state::token::OriginId::UNKNOWN,
        }];

        stores
            .assign_int_param(
                IntParam::NEWLINE_CHAR,
                0,
                tex_state::AssignmentScope::Global,
            )
            .expect("parameter");
        let rendered = ShortDisplayRenderer::new().render_nodes(&stores, &nodes);
        assert_eq!(rendered, "\\bigtr\np -");
        assert!(!rendered.as_bytes().contains(&0));

        stores
            .assign_int_param(
                IntParam::NEWLINE_CHAR,
                -1,
                tex_state::AssignmentScope::Global,
            )
            .expect("parameter");
        let rendered = ShortDisplayRenderer::new().render_nodes(&stores, &nodes);
        assert_eq!(rendered, "\\bigtr^^@p -");
        assert!(!rendered.as_bytes().contains(&0));
    });
}

#[test]
fn batch_mode_routes_pack_headline_and_box_dump_to_log_only() {
    crate::test_harness::with_nonstop_universe(|universe| {
        universe.set_interaction_mode(tex_state::InteractionMode::Batch);
        let mut stores = universe.command_context().expect("test state is admitted");
        let mut diagnostic_effects = DiagnosticEffects::new();
        let context = ExecutionDiagnosticContext::source_free("");
        let packed = empty_hbox(&mut stores);

        report_pack_diagnostics(
            &mut stores,
            &mut diagnostic_effects,
            &context,
            PackedDirection::Horizontal,
            &[PackDiagnostic::Overfull {
                excess: Scaled::from_raw(2 * Scaled::UNITY),
            }],
            &packed,
            DiagnosticListLayout::FrozenList,
        );

        drop(stores);
        universe
            .world_mut()
            .publish_diagnostic_effects(diagnostic_effects);
        assert_eq!(sink_text(universe, true), "");
        let log = sink_text(universe, false);
        assert!(
            log.starts_with("\nOverfull \\hbox (2.0pt too wide)"),
            "{log:?}"
        );
        assert!(log.ends_with("\n\n"), "{log:?}");
    });
}

#[test]
fn nonstop_mode_keeps_pack_headline_before_dump_on_both_channels() {
    crate::test_harness::with_nonstop_universe(|universe| {
        universe.set_interaction_mode(tex_state::InteractionMode::Nonstop);
        let mut stores = universe.command_context().expect("test state is admitted");
        let mut diagnostic_effects = DiagnosticEffects::new();
        stores
            .assign_int_param(
                tex_state::env::banks::IntParam::TRACING_ONLINE,
                1,
                tex_state::AssignmentScope::Global,
            )
            .expect("parameter");
        let context = ExecutionDiagnosticContext::source_free("");
        let packed = empty_hbox(&mut stores);

        report_pack_diagnostics(
            &mut stores,
            &mut diagnostic_effects,
            &context,
            PackedDirection::Horizontal,
            &[PackDiagnostic::Overfull {
                excess: Scaled::from_raw(2 * Scaled::UNITY),
            }],
            &packed,
            DiagnosticListLayout::FrozenList,
        );

        drop(stores);
        universe
            .world_mut()
            .publish_diagnostic_effects(diagnostic_effects);
        let terminal = sink_text(universe, true);
        let log = sink_text(universe, false);
        assert_eq!(terminal, log);
        assert!(
            terminal.starts_with("\nOverfull \\hbox (2.0pt too wide)"),
            "{terminal:?}"
        );
        assert!(terminal.ends_with("\n\n"), "{terminal:?}");
    });
}

#[test]
fn output_active_vbox_dump_supplies_the_headline_newline() {
    // TeX82 §§182/675: the output-active vbox branch omits its own
    // `print_ln`; `show_box` supplies exactly one newline before the node.
    // Outside `\output`, §675's explicit newline remains in addition.
    for (output_active, separator) in [(true, "\n"), (false, "\n\n")] {
        crate::test_harness::with_nonstop_tex82_universe(|universe| {
            let mut stores = universe.command_context().expect("test state is admitted");
            let mut diagnostic_effects = DiagnosticEffects::new();
            let context = ExecutionDiagnosticContext::new(0, 0, output_active, "");
            let packed = empty_hbox(&mut stores);

            report_pack_diagnostics(
                &mut stores,
                &mut diagnostic_effects,
                &context,
                PackedDirection::Vertical,
                &[PackDiagnostic::Overfull {
                    excess: Scaled::from_raw(2 * Scaled::UNITY),
                }],
                &packed,
                DiagnosticListLayout::FrozenList,
            );

            let origin = if output_active {
                ") has occurred while \\output is active"
            } else {
                ") detected at line 0"
            };
            drop(stores);
            universe
                .world_mut()
                .publish_diagnostic_effects(diagnostic_effects);
            assert_eq!(
                sink_text(universe, false),
                format!(
                    "\nOverfull \\vbox (2.0pt too high{origin}{separator}\\hbox(0.0+0.0)x0.0\n\n"
                )
            );
        });
    }
}

/// tex.web §§661--663: every `pack_begin_line` origin spelling is exact,
/// output-routine context takes precedence, and §§54/§245 selectors keep the
/// headline on the live selector while the box dump is transcript-only.
#[test]
fn pack_diagnostic_origin_contexts() {
    let origins = [
        (0, false, ") detected at line 29"),
        (11, false, ") in paragraph at lines 11--29"),
        (-17, false, ") in alignment at lines 17--29"),
        (11, true, ") has occurred while \\output is active"),
    ];
    let modes = [
        (tex_state::InteractionMode::Batch, false),
        (tex_state::InteractionMode::Nonstop, true),
        (tex_state::InteractionMode::Scroll, true),
        (tex_state::InteractionMode::ErrorStop, true),
    ];

    for (pack_begin_line, output_active, origin) in origins {
        for (mode, headline_on_terminal) in modes {
            crate::test_harness::with_nonstop_tex82_universe(|universe| {
                universe.set_interaction_mode(mode);
                let mut stores = universe.command_context().expect("test state is admitted");
                let mut diagnostic_effects = DiagnosticEffects::new();
                let context =
                    ExecutionDiagnosticContext::new(29, pack_begin_line, output_active, "");
                let packed = empty_hbox(&mut stores);

                report_pack_diagnostics(
                    &mut stores,
                    &mut diagnostic_effects,
                    &context,
                    PackedDirection::Horizontal,
                    &[PackDiagnostic::Underfull {
                        badness: 10_000,
                        excess: Scaled::from_raw(Scaled::UNITY),
                    }],
                    &packed,
                    DiagnosticListLayout::FrozenList,
                );

                let expected_headline = format!("\nUnderfull \\hbox (badness 10000{origin}\n\n");
                drop(stores);
                universe
                    .world_mut()
                    .publish_diagnostic_effects(diagnostic_effects);
                assert_eq!(
                    sink_text(universe, false),
                    format!("{expected_headline}\n\\hbox(0.0+0.0)x0.0\n\n")
                );
                assert_eq!(
                    sink_text(universe, true),
                    if headline_on_terminal {
                        expected_headline
                    } else {
                        String::new()
                    }
                );
            });
        }
    }
}
