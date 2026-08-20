//! TeX82 §§54/660/663 packed-box diagnostic routing regressions.

use super::*;
use tex_fonts::{FontMetrics, LoadedFont};
use tex_state::EffectRecord;
use tex_state::env::banks::IntParam;
use tex_state::glue::{GlueSpec, Order};
use tex_state::node::{AdjustNode, BoxNode, BoxNodeFields, DiscKind, GlueKind, KernKind, Sign};
use tex_state::scaled::{GlueSetRatio, Scaled};

fn empty_hbox(_stores: &mut Universe) -> Node {
    Node::HList(BoxNode::new(BoxNodeFields {
        width: Scaled::from_raw(0),
        height: Scaled::from_raw(0),
        depth: Scaled::from_raw(0),
        shift: Scaled::from_raw(0),
        box_lr: tex_state::node::BoxLr::Normal,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children: tex_state::node_arena::NodeListRef::empty(),
    }))
}

fn sink_text(stores: &Universe, terminal: bool) -> String {
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
    let mut stores = Universe::new();
    let empty = tex_state::node_arena::NodeListRef::empty();
    let replacement = stores.freeze_node_list(&[Node::Kern {
        amount: Scaled::from_raw(Scaled::UNITY),
        kind: KernKind::Explicit,
    }]);
    let space = stores.intern_glue(GlueSpec {
        width: Scaled::from_raw(Scaled::UNITY),
        ..GlueSpec::ZERO
    });
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
            pre: empty.clone(),
            post: empty.clone(),
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

    let list = stores.freeze_node_list(&nodes);
    assert_eq!(
        ShortDisplayRenderer::new().render_list(&stores, list),
        "[][][] [] [] []|"
    );
}

#[test]
fn short_display_retains_rule_after_nonphysical_discretionary_replacement() {
    // TeX82 §174 uses the disc node's `replace_count`, not the length of its
    // replacement side list. This is the source shape produced by the TRIP
    // display's `\discretionary{...}{...}{...}` after math conversion: the
    // immutable replacement remains available, but no replacement node is
    // linked after the disc, and the following rule must print as `|`.
    let mut stores = Universe::new();
    let empty = tex_state::node_arena::NodeListRef::empty();
    let replacement = stores.freeze_node_list(&[Node::Kern {
        amount: Scaled::from_raw(Scaled::UNITY),
        kind: KernKind::Explicit,
    }]);
    let space = stores.intern_glue(GlueSpec {
        width: Scaled::from_raw(Scaled::UNITY),
        ..GlueSpec::ZERO
    });
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
            pre: empty.clone(),
            post: empty.clone(),
            replace: replacement,
            physical_replace_count: 0,
        },
        Node::Rule {
            width: None,
            height: None,
            depth: None,
        },
    ]);

    let list = stores.freeze_node_list(&nodes);
    assert_eq!(
        ShortDisplayRenderer::new().render_list(&stores, list),
        "[][][] [] [] []|"
    );
}

#[test]
fn short_display_physical_count_is_independent_of_empty_side_list() {
    // Counterexample in the other direction: physical replacement nodes are
    // skipped even when this frozen representation no longer needs to retain
    // their source side list.
    let mut stores = Universe::new();
    let empty = tex_state::node_arena::NodeListRef::empty();
    let nodes = [
        Node::Disc {
            kind: DiscKind::Discretionary,
            pre: empty.clone(),
            post: empty.clone(),
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

    let list = stores.freeze_node_list(&nodes);
    assert_eq!(ShortDisplayRenderer::new().render_list(&stores, list), "|");
}

#[test]
fn line_trace_projection_renders_detached_replacement_content() {
    // TRIP's line trace supplies a detached slice: the side list is empty
    // while its three replacement characters remain in the displayed
    // projection. Applying the frozen list's physical count would incorrectly
    // reduce TeX82's `B-BBB` to `B-B`.
    let mut stores = Universe::new();
    let font = tex_state::font::NULL_FONT;
    let chars = |text: &str| {
        text.chars()
            .map(|ch| Node::Char {
                font,
                ch,
                origin: tex_state::provenance::OriginRef::unknown(),
            })
            .collect::<Vec<_>>()
    };
    let pre = stores.freeze_node_list(&chars("B-"));
    let empty = tex_state::node_arena::NodeListRef::empty();
    let mut nodes = vec![Node::Disc {
        kind: DiscKind::Discretionary,
        pre,
        post: empty.clone(),
        replace: empty,
        physical_replace_count: 3,
    }];
    nodes.extend(chars("BBB"));

    assert_eq!(
        ShortDisplayRenderer::new().render_nodes(&stores, &nodes),
        format!("{} B-BBB", crate::node_dump::font_identifier(&stores, font))
    );
}

#[test]
fn frozen_line_diagnostic_renders_both_disc_branches_then_skips_replacement() {
    // TeX82 §174 renders pre_break and post_break before advancing over the
    // physically linked replacement. This is the remaining TRIP underfull
    // line shape: `BB`, then `-`/`B-`, one hidden replacement node, then
    // `BBB`, yielding exactly `BB-B-BBB`.
    let mut stores = Universe::new();
    let font = tex_state::font::NULL_FONT;
    let chars = |text: &str| {
        text.chars()
            .map(|ch| Node::Char {
                font,
                ch,
                origin: tex_state::provenance::OriginRef::unknown(),
            })
            .collect::<Vec<_>>()
    };
    let pre = stores.freeze_node_list(&chars("-"));
    let post = stores.freeze_node_list(&chars("B-"));
    let replacement = stores.freeze_node_list(&chars("X"));
    let mut nodes = chars("BB");
    nodes.push(Node::Disc {
        kind: DiscKind::AutomaticHyphen,
        pre,
        post,
        replace: replacement,
        physical_replace_count: 1,
    });
    nodes.extend(chars("XBBB"));
    let list = stores.freeze_node_list(&nodes);

    assert_eq!(
        ShortDisplayRenderer::new().render_list(&stores, list),
        format!(
            "{} BB-B-BBB",
            crate::node_dump::font_identifier(&stores, font)
        )
    );
}

#[test]
fn short_display_maps_all_node_classes() {
    let mut stores = Universe::new();
    let empty = tex_state::node_arena::NodeListRef::empty();
    let zero_glue = stores.intern_glue(GlueSpec::ZERO);
    let nonzero_glue = stores.intern_glue(GlueSpec {
        width: Scaled::from_raw(Scaled::UNITY),
        ..GlueSpec::ZERO
    });
    let mark_tokens = stores.intern_token_list(&[]);
    let mark_tokens = stores.token_list_ref(mark_tokens);
    let pre = stores.freeze_node_list(&[Node::Kern {
        amount: Scaled::from_raw(Scaled::UNITY),
        kind: KernKind::Explicit,
    }]);
    let post = stores.freeze_node_list(&[Node::Kern {
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
            content: empty.clone(),
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
}

#[test]
fn etex_direction_nodes_follow_short_display_subtypes_without_mutation() {
    let stores = Universe::new();
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
}

#[test]
fn short_display_renderer_retains_font_across_fragments_until_reset() {
    // TeX82 §§174/851: one line-breaking pass initializes
    // `font_in_short_display` once, then successive feasible-break fragments
    // omit an unchanged font identifier. The next pass resets it.
    let stores = Universe::new();
    let font = tex_state::font::NULL_FONT;
    let fragment = [Node::Char {
        font,
        ch: 'A',
        origin: tex_state::provenance::OriginRef::unknown(),
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
}

#[test]
fn short_display_uses_print_ascii_for_eight_bit_character_codes() {
    let stores = Universe::new();
    let font = tex_state::font::NULL_FONT;
    let nodes = [
        Node::Lig {
            font,
            ch: '\u{82}',
            orig: vec!['C', 'A'],
            origins: vec![tex_state::provenance::OriginRef::unknown(); 2],
            left_hit: false,
            right_hit: false,
        },
        Node::Char {
            font,
            ch: '\u{82}',
            origin: tex_state::provenance::OriginRef::unknown(),
        },
    ];

    assert_eq!(
        ShortDisplayRenderer::new().render_nodes(&stores, &nodes),
        format!(
            "{} CA^^82",
            crate::node_dump::font_identifier(&stores, font)
        )
    );
}

#[test]
fn short_display_honors_live_newline_character() {
    // TeX82 §§59/174: `short_display` prints a character node through the
    // one-character string path, where `\newlinechar` is recognized before
    // the `^^` spelling for an unprintable character.
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::NEWLINE_CHAR, 10);
    let font = tex_state::font::NULL_FONT;
    let nodes = [Node::Char {
        font,
        ch: '\n',
        origin: tex_state::provenance::OriginRef::unknown(),
    }];

    assert_eq!(
        ShortDisplayRenderer::new().render_nodes(&stores, &nodes),
        format!("{} \n", crate::node_dump::font_identifier(&stores, font))
    );

    stores.set_int_param(IntParam::NEWLINE_CHAR, -1);
    assert_eq!(
        ShortDisplayRenderer::new().render_nodes(&stores, &nodes),
        format!("{} ^^J", crate::node_dump::font_identifier(&stores, font))
    );
}

#[test]
fn short_display_renders_byte_zero_in_a_font_identifier_through_print() {
    // TeX82 §§58--60/174: `print_esc(font_id_text(f))` sends every character
    // in the control-sequence name through `print`. Byte zero is therefore a
    // line break when `\newlinechar=0`, and `^^@` when new lines are disabled;
    // it is never a raw NUL in the completed diagnostic.
    let size = Scaled::from_raw(10 * Scaled::UNITY);
    let loaded = LoadedFont::new(
        "fixture",
        "fixture.tfm",
        [0; 32],
        0,
        size,
        size,
        vec![Scaled::from_raw(0); 7],
        FontMetrics::new(Vec::new(), Vec::new(), None, None, Vec::new()),
    );
    let mut stores = Universe::new_with_plain_catcodes();
    let font = stores.intern_font(loaded);
    let identifier = stores.intern("bigtr\0p");
    stores.set_font_identifier_symbol(font, identifier);
    let nodes = [Node::Char {
        font,
        ch: '-',
        origin: tex_state::provenance::OriginRef::unknown(),
    }];

    stores.set_int_param(IntParam::NEWLINE_CHAR, 0);
    let rendered = ShortDisplayRenderer::new().render_nodes(&stores, &nodes);
    assert_eq!(rendered, "\\bigtr\np -");
    assert!(!rendered.as_bytes().contains(&0));

    stores.set_int_param(IntParam::NEWLINE_CHAR, -1);
    let rendered = ShortDisplayRenderer::new().render_nodes(&stores, &nodes);
    assert_eq!(rendered, "\\bigtr^^@p -");
    assert!(!rendered.as_bytes().contains(&0));
}

#[test]
fn short_display_compares_restored_fonts_by_tex_number() {
    // TeX82 §174 retains an integer `font_in_short_display`. Restoring an
    // immutable format can change Umber's owner namespace without changing
    // that dense TeX font number.
    let size = Scaled::from_raw(10 * Scaled::UNITY);
    let loaded = LoadedFont::new(
        "fixture",
        "fixture.tfm",
        [0; 32],
        0,
        size,
        size,
        vec![Scaled::from_raw(0); 7],
        FontMetrics::new(Vec::new(), Vec::new(), None, None, Vec::new()),
    );
    let mut stores = Universe::new_with_plain_catcodes();
    let font = stores.intern_font(loaded);
    let identifier = stores.intern("fixturefont");
    stores.set_font_identifier_symbol(font, identifier);
    let mut restored = Universe::new_with_plain_catcodes();
    let restored_font = restored.intern_font(stores.font(font).clone());
    assert_ne!(font, restored_font, "the owner namespace is the challenge");
    assert_eq!(font.raw(), restored_font.raw());
    let nodes = [
        Node::Char {
            font,
            ch: 'A',
            origin: tex_state::provenance::OriginRef::unknown(),
        },
        Node::Char {
            font: restored_font,
            ch: 'B',
            origin: tex_state::provenance::OriginRef::unknown(),
        },
    ];

    assert_eq!(
        ShortDisplayRenderer::new().render_nodes(&stores, &nodes),
        "\\fixturefont AB"
    );
}

#[test]
fn batch_mode_routes_pack_headline_and_box_dump_to_log_only() {
    let mut stores = Universe::new();
    stores.set_interaction_mode(tex_state::InteractionMode::Batch);
    let packed = empty_hbox(&mut stores);

    report_pack_diagnostics(
        &mut stores,
        PackedDirection::Horizontal,
        &[PackDiagnostic::Overfull {
            excess: Scaled::from_raw(2 * Scaled::UNITY),
        }],
        &packed,
        DiagnosticListLayout::FrozenList,
    );

    assert_eq!(sink_text(&stores, true), "");
    let log = sink_text(&stores, false);
    assert!(
        log.starts_with("\nOverfull \\hbox (2.0pt too wide)"),
        "{log:?}"
    );
    assert!(log.ends_with("\n\n"), "{log:?}");
}

#[test]
fn nonstop_mode_keeps_pack_headline_before_dump_on_both_channels() {
    let mut stores = Universe::new();
    stores.set_interaction_mode(tex_state::InteractionMode::Nonstop);
    stores.set_int_param(tex_state::env::banks::IntParam::TRACING_ONLINE, 1);
    let packed = empty_hbox(&mut stores);

    report_pack_diagnostics(
        &mut stores,
        PackedDirection::Horizontal,
        &[PackDiagnostic::Overfull {
            excess: Scaled::from_raw(2 * Scaled::UNITY),
        }],
        &packed,
        DiagnosticListLayout::FrozenList,
    );

    let terminal = sink_text(&stores, true);
    let log = sink_text(&stores, false);
    assert_eq!(terminal, log);
    assert!(
        terminal.starts_with("\nOverfull \\hbox (2.0pt too wide)"),
        "{terminal:?}"
    );
    assert!(terminal.ends_with("\n\n"), "{terminal:?}");
}

#[test]
fn output_active_vbox_dump_supplies_the_headline_newline() {
    // TeX82 §§182/675: the output-active vbox branch omits its own
    // `print_ln`; `show_box` supplies exactly one newline before the node.
    // Outside `\output`, §675's explicit newline remains in addition.
    for (output_active, separator) in [(true, "\n"), (false, "\n\n")] {
        let mut stores = Universe::new();
        stores.set_output_routine_active(output_active);
        let packed = empty_hbox(&mut stores);

        report_pack_diagnostics(
            &mut stores,
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
        assert_eq!(
            sink_text(&stores, false),
            format!("\nOverfull \\vbox (2.0pt too high{origin}{separator}\\hbox(0.0+0.0)x0.0\n\n")
        );
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
            let mut stores = Universe::new();
            stores.set_interaction_mode(mode);
            stores.set_current_input_line(29);
            stores.set_pack_begin_line(pack_begin_line);
            stores.set_output_routine_active(output_active);
            let packed = empty_hbox(&mut stores);

            report_pack_diagnostics(
                &mut stores,
                PackedDirection::Horizontal,
                &[PackDiagnostic::Underfull {
                    badness: 10_000,
                    excess: Scaled::from_raw(Scaled::UNITY),
                }],
                &packed,
                DiagnosticListLayout::FrozenList,
            );

            let expected_headline = format!("\nUnderfull \\hbox (badness 10000{origin}\n\n");
            assert_eq!(
                sink_text(&stores, false),
                format!("{expected_headline}\n\\hbox(0.0+0.0)x0.0\n\n")
            );
            assert_eq!(
                sink_text(&stores, true),
                if headline_on_terminal {
                    expected_headline
                } else {
                    String::new()
                }
            );
        }
    }
}
