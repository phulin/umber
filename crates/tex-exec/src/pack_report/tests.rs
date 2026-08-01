//! TeX82 §§54/660/663 packed-box diagnostic routing regressions.

use super::*;
use tex_state::EffectRecord;
use tex_state::glue::{GlueSpec, Order};
use tex_state::node::{AdjustNode, BoxNode, BoxNodeFields, DiscKind, GlueKind, KernKind, Sign};
use tex_state::scaled::{GlueSetRatio, Scaled};

fn empty_hbox(stores: &mut Universe) -> Node {
    Node::HList(BoxNode::new(BoxNodeFields {
        width: Scaled::from_raw(0),
        height: Scaled::from_raw(0),
        depth: Scaled::from_raw(0),
        shift: Scaled::from_raw(0),
        box_lr: tex_state::node::BoxLr::Normal,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children: stores.freeze_node_list(&[]),
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
fn short_display_does_not_skip_following_nodes_for_side_stored_replacement() {
    let mut stores = Universe::new();
    let empty = stores.freeze_node_list(&[]);
    let replacement = stores.freeze_node_list(&[Node::Kern {
        amount: Scaled::from_raw(Scaled::UNITY),
        kind: KernKind::Explicit,
    }]);
    let nodes = [
        Node::Disc {
            kind: DiscKind::Discretionary,
            pre: empty,
            post: empty,
            replace: replacement,
        },
        Node::Rule {
            width: None,
            height: None,
            depth: None,
        },
    ];

    assert_eq!(short_display_nodes(&stores, &nodes), "|");
}

#[test]
fn short_display_maps_all_node_classes() {
    let mut stores = Universe::new();
    let empty = stores.freeze_node_list(&[]);
    let zero_glue = stores.intern_glue(GlueSpec::ZERO);
    let nonzero_glue = stores.intern_glue(GlueSpec {
        width: Scaled::from_raw(Scaled::UNITY),
        ..GlueSpec::ZERO
    });
    let mark_tokens = stores.intern_token_list(&[]);
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
        },
        Node::Penalty(100),
        Node::Kern {
            amount: Scaled::from_raw(3 * Scaled::UNITY),
            kind: KernKind::Explicit,
        },
    ];

    assert_eq!(short_display_nodes(&stores, &nodes), "[]| $[][]");
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
