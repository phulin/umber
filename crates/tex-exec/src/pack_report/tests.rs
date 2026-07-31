//! TeX82 §§54/660/663 packed-box diagnostic routing regressions.

use super::*;
use tex_state::EffectRecord;
use tex_state::glue::Order;
use tex_state::node::{BoxNode, BoxNodeFields, Sign};
use tex_state::scaled::{GlueSetRatio, Scaled};

fn empty_hbox(stores: &mut Universe) -> Node {
    Node::HList(BoxNode::new(BoxNodeFields {
        width: Scaled::from_raw(0),
        height: Scaled::from_raw(0),
        depth: Scaled::from_raw(0),
        shift: Scaled::from_raw(0),
        display: false,
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
