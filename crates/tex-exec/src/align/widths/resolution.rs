use tex_state::Universe;
use tex_state::ids::GlueId;
use tex_state::node::Node;
use tex_state::scaled::Scaled;

use crate::ExecError;
use crate::mode::{AlignState, AlignmentKind};

use super::{ResolvedWidths, add_scaled, sub_scaled, unset_axis_size};

#[derive(Clone, Debug, Default)]
struct ColumnRecord {
    width: Option<Scaled>,
    spans: Vec<SpanRecord>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SpanRecord {
    count: usize,
    width: Scaled,
}

pub(super) fn resolve_widths(
    state: &AlignState,
    rows: &[Node],
    stores: &Universe,
) -> Result<ResolvedWidths, ExecError> {
    let mut records = vec![ColumnRecord::default(); state.columns().len()];
    let mut tabskips = initial_tabskips(state, records.len());
    collect_width_requirements(
        state.kind(),
        rows,
        stores,
        state,
        &mut records,
        &mut tabskips,
    );

    let mut index = 0;
    while index < records.len() {
        if records[index].width.is_none() {
            records[index].width = Some(Scaled::from_raw(0));
            ensure_layout_len(&mut records, &mut tabskips, state, index + 1);
            tabskips[index + 1] = GlueId::ZERO;
        }

        let spans = std::mem::take(&mut records[index].spans);
        if !spans.is_empty() {
            ensure_layout_len(&mut records, &mut tabskips, state, index + 2);
            let reduction = add_scaled(
                records[index].width.expect("column width was resolved"),
                stores.glue(tabskips[index + 1]).width,
            )?;
            for span in spans {
                let reduced = sub_scaled(span.width, reduction)?;
                merge_width(&mut records[index + 1], span.count - 1, reduced);
            }
        }
        index += 1;
    }

    Ok(ResolvedWidths {
        columns: records
            .into_iter()
            .map(|record| record.width.expect("all columns are resolved"))
            .collect(),
        tabskips,
    })
}

fn initial_tabskips(state: &AlignState, columns: usize) -> Vec<GlueId> {
    (0..=columns)
        .map(|boundary| state.tabskip_for_boundary(boundary))
        .collect()
}

fn collect_width_requirements(
    kind: AlignmentKind,
    rows: &[Node],
    stores: &Universe,
    state: &AlignState,
    records: &mut Vec<ColumnRecord>,
    tabskips: &mut Vec<GlueId>,
) {
    for node in rows {
        let Node::Unset(row) = node else {
            continue;
        };
        let mut column = 0usize;
        for child in stores.nodes(row.children) {
            let Node::Unset(cell) = child else {
                continue;
            };
            let span = usize::from(cell.span_count.max(1));
            ensure_layout_len(records, tabskips, state, column + span);
            merge_width(&mut records[column], span, unset_axis_size(kind, cell));
            column += span;
        }
    }
}

fn ensure_layout_len(
    records: &mut Vec<ColumnRecord>,
    tabskips: &mut Vec<GlueId>,
    state: &AlignState,
    columns: usize,
) {
    while records.len() < columns {
        records.push(ColumnRecord::default());
    }
    while tabskips.len() <= records.len() {
        let boundary = tabskips.len();
        tabskips.push(state.tabskip_for_boundary(boundary));
    }
}

fn merge_width(record: &mut ColumnRecord, count: usize, width: Scaled) {
    if count <= 1 {
        if record.width.is_none_or(|old| width > old) {
            record.width = Some(width);
        }
        return;
    }

    match record.spans.binary_search_by_key(&count, |span| span.count) {
        Ok(index) => {
            if width > record.spans[index].width {
                record.spans[index].width = width;
            }
        }
        Err(index) => record.spans.insert(index, SpanRecord { count, width }),
    }
}
