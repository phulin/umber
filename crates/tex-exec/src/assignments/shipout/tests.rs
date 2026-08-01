use super::*;

use tex_state::EffectRecord;
use tex_state::env::banks::DimenParam;
use tex_state::node::{BoxNode, BoxNodeFields, Sign};
use tex_state::page::PageInteger;
use tex_state::scaled::{GlueSetRatio, Scaled};

fn empty_box(stores: &mut Universe, width: Scaled, height: Scaled, depth: Scaled) -> Node {
    let children = stores.freeze_node_list(&[]);
    Node::HList(BoxNode::new(BoxNodeFields {
        width,
        height,
        depth,
        shift: Scaled::from_raw(0),
        box_lr: tex_state::node::BoxLr::Normal,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: tex_state::glue::Order::Normal,
        children,
    }))
}

#[test]
fn ship_out_rejects_each_huge_page_dimension_boundary() {
    let zero = Scaled::from_raw(0);
    let one = Scaled::from_raw(1);
    let max = Scaled::MAX_DIMEN;
    let over = Scaled::from_raw(max.raw() + 1);
    let cases = [
        (zero, over, zero, zero, zero),
        (zero, zero, over, zero, zero),
        (zero, max, one, zero, zero),
        (max, zero, zero, one, zero),
    ];

    for (width, height, depth, h_offset, v_offset) in cases {
        let mut stores = crate::test_harness::universe();
        stores.set_dimen_param(DimenParam::H_OFFSET, h_offset);
        stores.set_dimen_param(DimenParam::V_OFFSET, v_offset);
        stores.set_page_integer(PageInteger::DeadCycles, 3);
        let node = empty_box(&mut stores, width, height, depth);

        let receipt = crate::canonical_main_control::test_shipout_replay_box(node, &mut stores)
            .expect("huge shipout recovers");

        assert!(receipt.is_none());
        assert!(stores.world().committed_artifacts().is_empty());
        assert_eq!(stores.page_integer(PageInteger::DeadCycles), 0);
        // §638's `[n]` marker prints unconditionally around the (rejected)
        // shipout and is committed immediately after (see
        // `canonical_main_control::print_ship_out_marker`'s doc), so the
        // huge-page warning text committed alongside it no longer sits in
        // the live effect suffix; read it back from the materialized
        // terminal output instead.
        let terminal = stores
            .world()
            .memory_terminal_output()
            .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
            .unwrap_or_default();
        assert!(
            terminal.contains("Huge page cannot be shipped out"),
            "{terminal}"
        );
        assert!(!stores.world().effect_records().iter().any(|effect| {
            matches!(
                effect,
                EffectRecord::StreamWrite { text, .. }
                    if text.contains("Huge page cannot be shipped out")
            )
        }));
    }

    let mut stores = crate::test_harness::universe();
    stores.set_page_integer(PageInteger::DeadCycles, 3);
    let node = empty_box(&mut stores, max, max, zero);
    let receipt = crate::canonical_main_control::test_shipout_replay_box(node, &mut stores)
        .expect("maximum legal page ships");
    assert!(receipt.is_some());
    assert_eq!(stores.world().committed_artifacts().len(), 1);
    assert_eq!(stores.page_integer(PageInteger::DeadCycles), 0);
}
