use super::*;
use crate::mode::{AlignColumn, AlignmentPackSpec};
use tex_state::glue::{GlueSpec, GlueSpecRef, Order};
use tex_state::ids::TokenListId;
use tex_state::node::{UnsetKind, UnsetNode, UnsetNodeFields};
use tex_state::node_arena::NodeListRef;
use tex_state::scaled::Scaled;

fn sp(raw: i32) -> Scaled {
    Scaled::from_raw(raw * Scaled::UNITY)
}

fn glue(stores: &mut Universe, width: i32) -> GlueSpecRef {
    stores.intern_glue(GlueSpec {
        width: sp(width),
        stretch: Scaled::from_raw(0),
        stretch_order: Order::Normal,
        shrink: Scaled::from_raw(0),
        shrink_order: Order::Normal,
    })
}

fn cell(empty: NodeListRef, width: i32, span_count: u16) -> Node {
    Node::Unset(UnsetNode::new(UnsetNodeFields {
        kind: UnsetKind::HBox,
        width: sp(width),
        height: sp(1),
        depth: Scaled::from_raw(0),
        span_count: span_count.saturating_sub(1),
        stretch: Scaled::from_raw(0),
        stretch_order: Order::Normal,
        shrink: Scaled::from_raw(0),
        shrink_order: Order::Normal,
        children: empty,
    }))
}

fn row(stores: &mut Universe, cells: &[Node]) -> Node {
    let children = stores.freeze_node_list(cells);
    Node::Unset(UnsetNode::new(UnsetNodeFields {
        kind: UnsetKind::HBox,
        width: Scaled::from_raw(0),
        height: sp(1),
        depth: Scaled::from_raw(0),
        span_count: 0,
        stretch: Scaled::from_raw(0),
        stretch_order: Order::Normal,
        shrink: Scaled::from_raw(0),
        shrink_order: Order::Normal,
        children,
    }))
}

fn state(columns: usize, tabskips: Vec<GlueSpecRef>) -> AlignState {
    let universe = Universe::new();
    let empty = universe.token_list_ref(TokenListId::EMPTY);
    AlignState::new(
        AlignmentKind::HAlign,
        AlignmentPackSpec::Natural,
        vec![
            AlignColumn {
                u_template: empty,
                v_template: empty,
            };
            columns
        ],
        tabskips,
        tex_state::glue::testing_zero_glue_ref(),
        None,
    )
}

#[test]
fn span_width_list_orders_counts_and_keeps_maximum() {
    let mut stores = Universe::new_with_plain_catcodes();
    let middle = glue(&mut stores, 1);
    let alignment = state(
        3,
        vec![
            tex_state::glue::testing_zero_glue_ref(),
            middle,
            middle,
            tex_state::glue::testing_zero_glue_ref(),
        ],
    );
    let empty = NodeListRef::empty();
    let rows = [
        row(
            &mut stores,
            &[
                cell(empty.clone(), 2, 1),
                cell(empty.clone(), 3, 1),
                cell(empty.clone(), 4, 1),
            ],
        ),
        row(&mut stores, &[cell(empty.clone(), 10, 2)]),
        row(&mut stores, &[cell(empty.clone(), 8, 2)]),
        row(&mut stores, &[cell(empty, 20, 3)]),
    ];

    let requirements = collect_width_requirements(AlignmentKind::HAlign, &rows)
        .expect("valid unset rows produce width requirements");
    assert_eq!(
        requirements
            .iter()
            .filter(|requirement| requirement.first_column == 0)
            .map(|requirement| (requirement.span, requirement.width))
            .collect::<Vec<_>>(),
        vec![(1, sp(2)), (2, sp(10)), (2, sp(8)), (3, sp(20))]
    );

    let resolved =
        resolve_widths(&alignment, &rows, &stores).expect("ordered span requirements resolve");
    assert_eq!(resolved.columns, vec![sp(2), sp(7), sp(9)]);
}

#[test]
fn resolve_alignment_widths_applies_tex82_recurrence() {
    let mut stores = Universe::new_with_plain_catcodes();
    let middle = glue(&mut stores, 1);
    let state = state(
        2,
        vec![
            tex_state::glue::testing_zero_glue_ref(),
            middle,
            tex_state::glue::testing_zero_glue_ref(),
        ],
    );
    let empty = NodeListRef::empty();
    let rows = [
        row(
            &mut stores,
            &[cell(empty.clone(), 4, 1), cell(empty.clone(), 3, 1)],
        ),
        row(&mut stores, &[cell(empty.clone(), 10, 2)]),
    ];

    let resolved = resolve_widths(&state, &rows, &stores).expect("span recurrence resolves");

    assert_eq!(resolved.columns, vec![sp(4), sp(5)]);
    assert_eq!(
        resolved.tabskips,
        vec![
            tex_state::glue::testing_zero_glue_ref(),
            middle,
            tex_state::glue::testing_zero_glue_ref()
        ]
    );
}

#[test]
fn resolve_alignment_widths_zeroes_null_column_tabskip() {
    let mut stores = Universe::new_with_plain_catcodes();
    let middle = glue(&mut stores, 1);
    let trailing = glue(&mut stores, 2);
    let state = state(
        2,
        vec![tex_state::glue::testing_zero_glue_ref(), middle, trailing],
    );
    let empty = NodeListRef::empty();
    let rows = [row(&mut stores, &[cell(empty, 4, 1)])];

    let resolved = resolve_widths(&state, &rows, &stores).expect("null columns resolve to zero");

    assert_eq!(resolved.columns, vec![sp(4), Scaled::from_raw(0)]);
    assert_eq!(resolved.tabskips[1], middle);
    assert_eq!(
        resolved.tabskips[2],
        tex_state::glue::testing_zero_glue_ref()
    );
}

#[test]
fn alignment_width_resolution_negative_zero_and_competing_span_matrix() {
    // TeX82 §§800--802 merge equal span counts by maximum width, process
    // shorter counts first, and retain negative residual requirements.
    let mut stores = Universe::new_with_plain_catcodes();
    let middle = glue(&mut stores, 2);
    let alignment = state(
        3,
        vec![
            tex_state::glue::testing_zero_glue_ref(),
            middle,
            middle,
            tex_state::glue::testing_zero_glue_ref(),
        ],
    );
    let empty = NodeListRef::empty();
    let rows = [
        row(&mut stores, &[cell(empty.clone(), 10, 1)]),
        row(&mut stores, &[cell(empty.clone(), 5, 2)]),
        row(&mut stores, &[cell(empty.clone(), 18, 2)]),
        row(&mut stores, &[cell(empty.clone(), 25, 3)]),
    ];

    let resolved = resolve_widths(&alignment, &rows, &stores).expect("§802 recurrence resolves");
    assert_eq!(resolved.columns, vec![sp(10), sp(6), sp(5)]);

    let empty_state = state(
        2,
        vec![
            tex_state::glue::testing_zero_glue_ref(),
            middle,
            tex_state::glue::testing_zero_glue_ref(),
        ],
    );
    let empty_rows = [row(&mut stores, &[cell(empty, -3, 2)])];
    let resolved = resolve_widths(&empty_state, &empty_rows, &stores)
        .expect("negative residual and null leading column resolve");
    assert_eq!(resolved.columns, vec![Scaled::from_raw(0), sp(-3)]);
    assert_eq!(
        resolved.tabskips[1],
        tex_state::glue::testing_zero_glue_ref()
    );
}
