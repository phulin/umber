use super::*;
use crate::mode::{AlignColumn, AlignmentPackSpec};
use tex_state::glue::{GlueSpec, Order};
use tex_state::node::{NodeTokenList, UnsetKind, UnsetNode, UnsetNodeFields};
use tex_state::node_arena::PageListId;
use tex_state::scaled::Scaled;

fn sp(raw: i32) -> Scaled {
    Scaled::from_raw(raw * Scaled::UNITY)
}

fn glue<G>(_stores: &mut CommandContext<'_, G>, width: i32) -> GlueSpec {
    GlueSpec {
        width: sp(width),
        stretch: Scaled::from_raw(0),
        stretch_order: Order::Normal,
        shrink: Scaled::from_raw(0),
        shrink_order: Order::Normal,
    }
}

fn cell(empty: PageListId, width: i32, span_count: u16) -> Node {
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

fn row<G>(stores: &mut CommandContext<'_, G>, cells: &[Node]) -> Node {
    let children = stores.publish_page_nodes(cells.to_vec());
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

fn state(columns: usize, tabskips: Vec<GlueSpec>) -> AlignState {
    let empty = NodeTokenList::default();
    AlignState::new(
        AlignmentKind::HAlign,
        AlignmentPackSpec::Natural,
        vec![
            AlignColumn {
                u_template: empty.clone(),
                v_template: empty,
            };
            columns
        ],
        tabskips,
        tex_state::glue::GlueSpec::ZERO,
        None,
    )
}

#[test]
fn span_width_list_orders_counts_and_keeps_maximum() {
    crate::test_harness::with_nonstop_plain_universe(|universe| {
        let mut stores = universe.command_context().expect("test state is admitted");
        let middle = glue(&mut stores, 1);
        let alignment = state(
            3,
            vec![
                tex_state::glue::GlueSpec::ZERO,
                middle,
                middle,
                tex_state::glue::GlueSpec::ZERO,
            ],
        );
        let empty = PageListId::empty();
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

        let requirements = collect_width_requirements(AlignmentKind::HAlign, &rows, &stores)
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
    });
}

#[test]
fn resolve_alignment_widths_applies_tex82_recurrence() {
    crate::test_harness::with_nonstop_plain_universe(|universe| {
        let mut stores = universe.command_context().expect("test state is admitted");
        let middle = glue(&mut stores, 1);
        let state = state(
            2,
            vec![
                tex_state::glue::GlueSpec::ZERO,
                middle,
                tex_state::glue::GlueSpec::ZERO,
            ],
        );
        let empty = PageListId::empty();
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
                tex_state::glue::GlueSpec::ZERO,
                middle,
                tex_state::glue::GlueSpec::ZERO
            ]
        );
    });
}

#[test]
fn resolve_alignment_widths_zeroes_null_column_tabskip() {
    crate::test_harness::with_nonstop_plain_universe(|universe| {
        let mut stores = universe.command_context().expect("test state is admitted");
        let middle = glue(&mut stores, 1);
        let trailing = glue(&mut stores, 2);
        let state = state(2, vec![tex_state::glue::GlueSpec::ZERO, middle, trailing]);
        let empty = PageListId::empty();
        let rows = [row(&mut stores, &[cell(empty, 4, 1)])];

        let resolved =
            resolve_widths(&state, &rows, &stores).expect("null columns resolve to zero");

        assert_eq!(resolved.columns, vec![sp(4), Scaled::from_raw(0)]);
        assert_eq!(resolved.tabskips[1], middle);
        assert_eq!(resolved.tabskips[2], tex_state::glue::GlueSpec::ZERO);
    });
}

#[test]
fn alignment_width_resolution_negative_zero_and_competing_span_matrix() {
    // TeX82 §§800--802 merge equal span counts by maximum width, process
    // shorter counts first, and retain negative residual requirements.
    crate::test_harness::with_nonstop_plain_universe(|universe| {
        let mut stores = universe.command_context().expect("test state is admitted");
        let middle = glue(&mut stores, 2);
        let alignment = state(
            3,
            vec![
                tex_state::glue::GlueSpec::ZERO,
                middle,
                middle,
                tex_state::glue::GlueSpec::ZERO,
            ],
        );
        let empty = PageListId::empty();
        let rows = [
            row(&mut stores, &[cell(empty.clone(), 10, 1)]),
            row(&mut stores, &[cell(empty.clone(), 5, 2)]),
            row(&mut stores, &[cell(empty.clone(), 18, 2)]),
            row(&mut stores, &[cell(empty.clone(), 25, 3)]),
        ];

        let resolved =
            resolve_widths(&alignment, &rows, &stores).expect("§802 recurrence resolves");
        assert_eq!(resolved.columns, vec![sp(10), sp(6), sp(5)]);

        let empty_state = state(
            2,
            vec![
                tex_state::glue::GlueSpec::ZERO,
                middle,
                tex_state::glue::GlueSpec::ZERO,
            ],
        );
        let empty_rows = [row(&mut stores, &[cell(empty, -3, 2)])];
        let resolved = resolve_widths(&empty_state, &empty_rows, &stores)
            .expect("negative residual and null leading column resolve");
        assert_eq!(resolved.columns, vec![Scaled::from_raw(0), sp(-3)]);
        assert_eq!(resolved.tabskips[1], tex_state::glue::GlueSpec::ZERO);
    });
}
