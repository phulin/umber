use super::*;

fn sp(raw: i32) -> Scaled {
    Scaled::from_raw(raw)
}

#[test]
fn resolves_single_columns_independently() {
    let plan = plan_alignment_widths(
        2,
        &[sp(0), sp(0), sp(0)],
        [
            AlignmentWidthRequirement {
                first_column: 0,
                span: 1,
                width: sp(8),
            },
            AlignmentWidthRequirement {
                first_column: 1,
                span: 1,
                width: sp(13),
            },
        ],
    )
    .expect("single-column plan");
    assert_eq!(plan.columns, vec![sp(8), sp(13)]);
}

#[test]
fn span_excess_lands_in_last_spanned_column() {
    let plan = plan_alignment_widths(
        2,
        &[sp(0), sp(3), sp(0)],
        [
            AlignmentWidthRequirement {
                first_column: 0,
                span: 1,
                width: sp(7),
            },
            AlignmentWidthRequirement {
                first_column: 0,
                span: 2,
                width: sp(25),
            },
        ],
    )
    .expect("two-column span plan");
    assert_eq!(plan.columns, vec![sp(7), sp(15)]);
}

#[test]
fn tabskip_width_is_subtracted_at_each_span_boundary() {
    let plan = plan_alignment_widths(
        3,
        &[sp(0), sp(2), sp(3), sp(0)],
        [
            AlignmentWidthRequirement {
                first_column: 0,
                span: 1,
                width: sp(5),
            },
            AlignmentWidthRequirement {
                first_column: 1,
                span: 1,
                width: sp(7),
            },
            AlignmentWidthRequirement {
                first_column: 0,
                span: 3,
                width: sp(30),
            },
        ],
    )
    .expect("three-column tabskip plan");
    assert_eq!(plan.columns, vec![sp(5), sp(7), sp(13)]);
}

#[test]
fn empty_column_forces_following_tabskip_to_zero() {
    let plan = plan_alignment_widths(
        2,
        &[sp(0), sp(9), sp(0)],
        [AlignmentWidthRequirement {
            first_column: 0,
            span: 2,
            width: sp(12),
        }],
    )
    .expect("empty-column plan");
    assert_eq!(plan.columns, vec![sp(0), sp(12)]);
    assert_eq!(plan.zero_tabskip_boundaries, vec![1]);
}
