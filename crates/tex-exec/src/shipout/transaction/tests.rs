use super::*;

use tex_state::env::banks::IntParam;
use tex_state::glue::Order;
use tex_state::node::{BoxLr, BoxNode, BoxNodeFields, Sign};
use tex_state::scaled::{GlueSetRatio, Scaled};

fn empty_vbox() -> Node {
    Node::VList(BoxNode::new(BoxNodeFields {
        width: Scaled::from_raw(0),
        height: Scaled::MAX_DIMEN,
        depth: Scaled::from_raw(0),
        shift: Scaled::from_raw(0),
        box_lr: BoxLr::Normal,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children: tex_state::node_arena::PageListId::empty(),
    }))
}

fn pending_text(stores: &Universe) -> String {
    stores
        .world()
        .effect_records()
        .iter()
        .filter_map(|record| match record {
            tex_state::EffectRecord::StreamWrite { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

#[test]
fn pre_staging_shipout_error_uses_live_command_context() {
    // TeX82 §§82 and 641: `ship_out` reports the huge-page error against the
    // current input stack. Successful artifact staging republishes its
    // detached summary only later, so the command-owned entry context wins.
    let stores = Universe::new();
    let stale = tex_state::InputSummary::default();
    let origin = ShipoutOrigin {
        output_open_context: Some("\n<recently read> }\n                  ".to_owned()),
        pending_end: 0,
        announce_openout: false,
    };

    assert_eq!(
        shipout_error_context(&stores, &stale, &origin),
        "\n<recently read> }\n                  "
    );
}

#[test]
fn huge_page_recovery_displays_deleted_box_only_when_not_already_traced() {
    // TeX82 §641: after the huge-page error, `ship_out` calls `show_box(p)`
    // only when §638 did not already do so under positive `\tracingoutput`.
    let mut untraced = Universe::new();
    let node = empty_vbox();
    report_huge_page_deleted_box(&mut untraced, &node, 0);
    let text = pending_text(&untraced);
    assert!(
        text.contains("The following box has been deleted:\n\\vbox(16383.99998+0.0)x0.0"),
        "{text:?}"
    );

    let mut traced = Universe::new();
    traced.set_int_param(IntParam::TRACING_OUTPUT, 1);
    let node = empty_vbox();
    let tracing_output = traced.int_param(IntParam::TRACING_OUTPUT);
    report_huge_page_deleted_box(&mut traced, &node, tracing_output);
    assert_eq!(pending_text(&traced), "");
}

#[test]
fn huge_page_deleted_box_display_uses_live_escape_character() {
    // TeX82 §§63/183/641: `ship_out` delegates the deleted box to
    // `show_box`, whose list-node name is printed through `print_esc`.
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::ESCAPE_CHAR, i32::from(b'|'));
    let node = empty_vbox();

    report_huge_page_deleted_box(&mut stores, &node, 0);

    let text = pending_text(&stores);
    assert!(
        text.contains("The following box has been deleted:\n|vbox(16383.99998+0.0)x0.0"),
        "{text:?}"
    );
}
