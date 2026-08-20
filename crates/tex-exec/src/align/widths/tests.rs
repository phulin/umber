use super::*;
use crate::mode::AlignColumn;
use tex_state::glue::{GlueSpec, GlueSpecRef, Order};
use tex_state::ids::TokenListId;
use tex_state::node::{UnsetKind, UnsetNodeFields};
use tex_state::node_arena::NodeListRef;

fn sp(raw: i32) -> Scaled {
    Scaled::from_raw(raw * Scaled::UNITY)
}

fn columns(count: usize) -> Vec<AlignColumn> {
    let universe = Universe::new();
    let empty = universe.token_list_ref(TokenListId::EMPTY);
    vec![
        AlignColumn {
            u_template: empty,
            v_template: empty,
        };
        count
    ]
}

fn state(kind: AlignmentKind, spec: AlignmentPackSpec, tabskips: Vec<GlueSpecRef>) -> AlignState {
    AlignState::new(
        kind,
        spec,
        columns(tabskips.len() - 1),
        tabskips,
        tex_state::glue::testing_zero_glue_ref(),
        None,
    )
}

fn unset(kind: UnsetKind, natural: i32, span_count: u16) -> Node {
    let empty = NodeListRef::empty();
    let (width, height) = match kind {
        UnsetKind::HBox => (sp(natural), sp(1)),
        UnsetKind::VBox => (sp(1), sp(natural)),
    };
    Node::Unset(UnsetNode::new(UnsetNodeFields {
        kind,
        width,
        height,
        depth: Scaled::from_raw(0),
        span_count: span_count.saturating_sub(1),
        stretch: Scaled::from_raw(0),
        stretch_order: tex_state::glue::Order::Normal,
        shrink: Scaled::from_raw(0),
        shrink_order: tex_state::glue::Order::Normal,
        children: empty,
    }))
}

#[test]
fn pack_alignment_prototype_applies_spec_in_both_modes() {
    for kind in [AlignmentKind::HAlign, AlignmentKind::VAlign] {
        let mut stores = Universe::new_with_plain_catcodes();
        let flexible = stores.intern_glue(GlueSpec {
            width: sp(1),
            stretch: sp(1),
            stretch_order: Order::Fil,
            shrink: Scaled::from_raw(0),
            shrink_order: Order::Normal,
        });
        let resolved = ResolvedWidths {
            columns: vec![sp(4), sp(5)],
            tabskips: vec![flexible, tex_state::glue::testing_zero_glue_ref(), flexible],
        };
        let empty = NodeListRef::empty();

        let exact = pack_prototype(
            &state(
                kind,
                AlignmentPackSpec::Exactly(sp(20)),
                resolved.tabskips.clone(),
            ),
            &resolved,
            &empty,
            &mut stores,
        );
        let exact_extent = match kind {
            AlignmentKind::HAlign => exact.box_node.width,
            AlignmentKind::VAlign => exact.box_node.height + exact.box_node.depth,
        };
        assert_eq!(exact_extent, sp(20));
        assert_eq!(exact.box_node.glue_sign, Sign::Stretching);
        assert_eq!(exact.box_node.glue_order, Order::Fil);
        assert_eq!(
            exact.box_node.glue_set,
            GlueSetRatio::from_ratio_parts(9, 2)
        );

        let spread = pack_prototype(
            &state(
                kind,
                AlignmentPackSpec::Spread(sp(3)),
                resolved.tabskips.clone(),
            ),
            &resolved,
            &empty,
            &mut stores,
        );
        let spread_extent = match kind {
            AlignmentKind::HAlign => spread.box_node.width,
            AlignmentKind::VAlign => spread.box_node.height + spread.box_node.depth,
        };
        assert_eq!(spread_extent, sp(14));
        assert_eq!(spread.box_node.glue_sign, Sign::Stretching);
        assert_eq!(spread.box_node.glue_order, Order::Fil);
        assert_eq!(
            spread.box_node.glue_set,
            GlueSetRatio::from_ratio_parts(3, 2)
        );
    }
}

/// TeX82 §805 converts preamble column records to `unset_node` before the
/// prototype pack.  §663's box diagnostic traverses that packed list, so the
/// retained node identity must survive packing instead of being projected as
/// an ordinary hbox/vbox.
#[test]
fn alignment_prototype_diagnostic_retains_unset_columns() {
    for (kind, expected) in [
        (AlignmentKind::HAlign, "\\unsetbox(0.0+0.0)x4.0"),
        (AlignmentKind::VAlign, "\\unsetbox(4.0+0.0)x0.0"),
    ] {
        let mut stores = Universe::new_with_plain_catcodes();
        let resolved = ResolvedWidths {
            columns: vec![sp(4)],
            tabskips: vec![
                tex_state::glue::testing_zero_glue_ref(),
                tex_state::glue::testing_zero_glue_ref(),
            ],
        };
        let empty = NodeListRef::empty();
        let prototype = pack_prototype(
            &state(kind, AlignmentPackSpec::Natural, resolved.tabskips.clone()),
            &resolved,
            &empty,
            &mut stores,
        );

        let dump = crate::node_dump::dump_node_slice(
            &stores,
            &prototype.box_node.children.to_vec(),
            crate::node_dump::DumpConfig {
                breadth: 10,
                depth: 10,
                profile: tex_command::CommandProfile::TEX82,
            },
        );
        assert!(dump.contains(expected), "prototype dump: {dump}");
    }
}

#[test]
fn fin_align_orders_groups_packing_pop_and_insertion() {
    let mut stores = Universe::new_with_plain_catcodes();
    let first = unset(UnsetKind::HBox, 4, 1);
    let second = unset(UnsetKind::HBox, 6, 1);
    let row_children = stores.freeze_node_list(&[
        tabskip_node(tex_state::glue::testing_zero_glue_ref()),
        first,
        tabskip_node(tex_state::glue::testing_zero_glue_ref()),
        second,
        tabskip_node(tex_state::glue::testing_zero_glue_ref()),
    ]);
    let row = Node::Unset(UnsetNode::new(UnsetNodeFields {
        kind: UnsetKind::HBox,
        width: sp(10),
        height: sp(2),
        depth: sp(1),
        span_count: 0,
        stretch: Scaled::from_raw(0),
        stretch_order: Order::Normal,
        shrink: Scaled::from_raw(0),
        shrink_order: Order::Normal,
        children: row_children,
    }));
    let state = state(
        AlignmentKind::HAlign,
        AlignmentPackSpec::Exactly(sp(12)),
        vec![
            tex_state::glue::testing_zero_glue_ref(),
            tex_state::glue::testing_zero_glue_ref(),
            tex_state::glue::testing_zero_glue_ref(),
        ],
    );

    let finished = finish_alignment(&state, &[row], Scaled::from_raw(0), &mut stores)
        .expect("the complete width, prototype, and setting pipeline succeeds");

    let [Node::HList(row)] = finished.as_slice() else {
        panic!("fin_align must convert the unset row to an hlist");
    };
    assert_eq!(row.width, sp(12));
    assert!(
        row.children
            .to_vec()
            .iter()
            .all(|node| !matches!(node, Node::Unset(_)))
    );
}
