//! Pure width planning for TeX alignments.
//!
//! Execution extracts detached cell requirements and tabskip widths before
//! entering this module. The planner consequently has no access to live state
//! handles and performs no materialization into an engine arena.

use std::collections::BTreeMap;

use tex_state::scaled::Scaled;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AlignmentWidthRequirement {
    pub first_column: usize,
    pub span: usize,
    pub width: Scaled,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AlignmentWidthPlan {
    pub columns: Vec<Scaled>,
    /// Boundaries TeX forces to zero after discovering an empty column.
    pub zero_tabskip_boundaries: Vec<usize>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AlignmentPlanError {
    ArithmeticOverflow,
    MissingTabskipBoundary(usize),
}

#[derive(Clone, Debug, Default)]
struct ColumnRecord {
    width: Option<Scaled>,
    // TeX's span chains are keyed by their remaining span count. A map avoids
    // the old sorted-Vec insertion and shifting cost for adversarial spans.
    spans: BTreeMap<usize, Scaled>,
}

pub fn plan_alignment_widths(
    declared_columns: usize,
    tabskip_widths: &[Scaled],
    requirements: impl IntoIterator<Item = AlignmentWidthRequirement>,
) -> Result<AlignmentWidthPlan, AlignmentPlanError> {
    let mut records = vec![ColumnRecord::default(); declared_columns];
    let mut zero_tabskip_boundaries = Vec::new();
    for requirement in requirements {
        let span = requirement.span.max(1);
        ensure_columns(&mut records, requirement.first_column + span);
        merge_width(
            &mut records[requirement.first_column],
            span,
            requirement.width,
        );
    }

    let mut index = 0;
    while index < records.len() {
        if records[index].width.is_none() {
            records[index].width = Some(Scaled::from_raw(0));
            zero_tabskip_boundaries.push(index + 1);
        }

        let spans = std::mem::take(&mut records[index].spans);
        if !spans.is_empty() {
            ensure_columns(&mut records, index + 2);
            let tabskip = if zero_tabskip_boundaries.last() == Some(&(index + 1)) {
                Scaled::from_raw(0)
            } else {
                *tabskip_widths
                    .get(index + 1)
                    .ok_or(AlignmentPlanError::MissingTabskipBoundary(index + 1))?
            };
            let reduction = records[index]
                .width
                .expect("column width was resolved")
                .checked_add(tabskip)
                .ok_or(AlignmentPlanError::ArithmeticOverflow)?;
            for (span, width) in spans {
                let reduced = width
                    .checked_sub(reduction)
                    .ok_or(AlignmentPlanError::ArithmeticOverflow)?;
                merge_width(&mut records[index + 1], span - 1, reduced);
            }
        }
        index += 1;
    }

    Ok(AlignmentWidthPlan {
        columns: records
            .into_iter()
            .map(|record| record.width.expect("all columns are resolved"))
            .collect(),
        zero_tabskip_boundaries,
    })
}

fn ensure_columns(records: &mut Vec<ColumnRecord>, columns: usize) {
    if records.len() < columns {
        records.resize_with(columns, ColumnRecord::default);
    }
}

fn merge_width(record: &mut ColumnRecord, span: usize, width: Scaled) {
    if span <= 1 {
        if record.width.is_none_or(|old| width > old) {
            record.width = Some(width);
        }
        return;
    }
    record
        .spans
        .entry(span)
        .and_modify(|old| *old = (*old).max(width))
        .or_insert(width);
}

#[cfg(test)]
mod tests;
