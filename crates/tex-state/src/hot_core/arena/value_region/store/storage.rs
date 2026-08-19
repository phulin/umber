//! Atomic bundle reservation and admitted-slice resolution.

use super::*;

#[derive(Clone, Copy, Default)]
pub(super) struct BundleAdditions {
    pub(super) tokens: usize,
    pub(super) token_lists: usize,
    pub(super) macro_records: usize,
    pub(super) macro_roots: usize,
    pub(super) glue_specs: usize,
    pub(super) provenance_roots: usize,
}

impl BundleAdditions {
    fn total(self) -> Result<usize, RegionArenaError> {
        self.tokens
            .checked_add(self.token_lists)
            .and_then(|total| total.checked_add(self.macro_records))
            .and_then(|total| total.checked_add(self.macro_roots))
            .and_then(|total| total.checked_add(self.glue_specs))
            .and_then(|total| total.checked_add(self.provenance_roots))
            .ok_or(RegionArenaError::OffsetCapacityExhausted)
    }
}

pub(super) fn prepare_bundle(
    arena: &mut ConcreteArena,
    additions: BundleAdditions,
) -> Result<
    &mut super::super::MutableRuntimeValueRegion<
        Token,
        RuntimeTokenListRow,
        RuntimeMacroRecord,
        RuntimeMacroRootRow,
        GlueSpec,
        RuntimeOriginRoot,
    >,
    RegionArenaError,
> {
    let total = additions.total()?;
    let needs_new = arena.active.as_ref().is_none_or(|active| {
        !active.appendable
            || active
                .columns
                .logical_values()
                .checked_add(total)
                .is_none_or(|end| end > arena.region_capacity as usize)
    });
    if needs_new {
        arena.seal_active()?;
        arena.ensure_active()?;
    }
    let active = arena.active.as_mut().expect("active region was ensured");
    validate_column_end(active.columns.token_words.len(), additions.tokens)?;
    validate_column_end(active.columns.token_lists.len(), additions.token_lists)?;
    validate_column_end(active.columns.macro_records.len(), additions.macro_records)?;
    validate_column_end(active.columns.macro_roots.len(), additions.macro_roots)?;
    validate_column_end(active.columns.glue_specs.len(), additions.glue_specs)?;
    validate_column_end(
        active.columns.provenance_roots.len(),
        additions.provenance_roots,
    )?;
    reserve_additional(
        &mut active.columns.token_words,
        additions.tokens,
        &mut arena.storage_growth_events,
    )?;
    reserve_additional(
        &mut active.columns.token_lists,
        additions.token_lists,
        &mut arena.storage_growth_events,
    )?;
    reserve_additional(
        &mut active.columns.macro_records,
        additions.macro_records,
        &mut arena.storage_growth_events,
    )?;
    reserve_additional(
        &mut active.columns.macro_roots,
        additions.macro_roots,
        &mut arena.storage_growth_events,
    )?;
    reserve_additional(
        &mut active.columns.glue_specs,
        additions.glue_specs,
        &mut arena.storage_growth_events,
    )?;
    reserve_additional(
        &mut active.columns.provenance_roots,
        additions.provenance_roots,
        &mut arena.storage_growth_events,
    )?;
    Ok(active)
}

fn reserve_additional<T>(
    column: &mut Vec<T>,
    additional: usize,
    storage_growth_events: &mut usize,
) -> Result<(), RegionArenaError> {
    if additional == 0 || column.capacity().saturating_sub(column.len()) >= additional {
        return Ok(());
    }
    let old_capacity = column.capacity();
    column
        .try_reserve_exact(additional)
        .map_err(|_| RegionArenaError::AllocationFailed)?;
    if column.capacity() != old_capacity {
        *storage_growth_events = storage_growth_events.saturating_add(1);
    }
    Ok(())
}

fn validate_column_end(len: usize, additional: usize) -> Result<(), RegionArenaError> {
    len.checked_add(additional)
        .filter(|end| u32::try_from(*end).is_ok())
        .map(|_| ())
        .ok_or(RegionArenaError::OffsetCapacityExhausted)
}

pub(super) fn checked_len(len: usize) -> Result<u32, RegionArenaError> {
    u32::try_from(len).map_err(|_| RegionArenaError::OffsetCapacityExhausted)
}

pub(super) fn token_list_view<'a>(
    regions: &'a ConcreteRegions,
    coordinate: RuntimeTokenListCoordinate,
) -> Result<RuntimeTokenListView<'a>, RegionArenaError> {
    let admitted = regions.admit(coordinate.owner())?;
    token_list_view_in(&admitted, coordinate)
}

pub(super) fn token_list_view_from_admission<'a>(
    regions: &'a ConcreteRegions,
    admitted: &ConcreteAdmission<'a>,
    coordinate: RuntimeTokenListCoordinate,
) -> Result<RuntimeTokenListView<'a>, RegionArenaError> {
    if admitted.region.key == coordinate.owner() {
        token_list_view_in(admitted, coordinate)
    } else {
        token_list_view(regions, coordinate)
    }
}

fn token_list_view_in<'a>(
    admitted: &ConcreteAdmission<'a>,
    coordinate: RuntimeTokenListCoordinate,
) -> Result<RuntimeTokenListView<'a>, RegionArenaError> {
    let row = admitted.token_list(coordinate.row)?;
    validate_token_identity(coordinate, row)?;
    Ok(RuntimeTokenListView {
        coordinate,
        semantic_id: row.semantic_id,
        tokens: token_span(admitted, row.tokens)?,
        provenance_roots: provenance_span(admitted, row.provenance_roots)?,
    })
}

pub(super) fn validate_token_identity(
    coordinate: RuntimeTokenListCoordinate,
    row: &RuntimeTokenListRow,
) -> Result<(), RegionArenaError> {
    (row.id == coordinate.id)
        .then_some(())
        .ok_or(RegionArenaError::InvalidMark)
}

pub(super) fn validate_macro_identity(
    coordinate: RuntimeMacroCoordinate,
    record: &RuntimeMacroRecord,
    roots: &RuntimeMacroRootRow,
) -> Result<(), RegionArenaError> {
    (record.definition == coordinate.id && roots.definition == coordinate.id)
        .then_some(())
        .ok_or(RegionArenaError::InvalidMark)
}

fn token_span<'a>(
    admitted: &ConcreteAdmission<'a>,
    span: LocalSpan<Token>,
) -> Result<&'a [Token], RegionArenaError> {
    resolve_local_span(&admitted.region.columns.token_words, span)
}

pub(super) fn provenance_at<'a>(
    admitted: &ConcreteAdmission<'a>,
    offset: u32,
) -> Result<&'a RuntimeOriginRoot, RegionArenaError> {
    admitted
        .region
        .columns
        .provenance_roots
        .get(offset as usize)
        .ok_or(RegionArenaError::OffsetOutOfBounds)
}

pub(super) fn provenance_span<'a>(
    admitted: &ConcreteAdmission<'a>,
    span: LocalSpan<RuntimeOriginRoot>,
) -> Result<&'a [RuntimeOriginRoot], RegionArenaError> {
    resolve_local_span(&admitted.region.columns.provenance_roots, span)
}

pub(super) fn validate_sparse_origins(
    roots: &[RuntimeOriginRoot],
    token_len: usize,
) -> Result<(), RegionArenaError> {
    let token_len =
        u32::try_from(token_len).map_err(|_| RegionArenaError::OffsetCapacityExhausted)?;
    let mut previous = None;
    for root in roots {
        if root.token_offset >= token_len || previous.is_some_and(|old| old >= root.token_offset) {
            return Err(RegionArenaError::OffsetOutOfBounds);
        }
        previous = Some(root.token_offset);
    }
    Ok(())
}

fn resolve_local_span<T>(values: &[T], span: LocalSpan<T>) -> Result<&[T], RegionArenaError> {
    let start = span.start as usize;
    let end = start
        .checked_add(span.len as usize)
        .ok_or(RegionArenaError::OffsetOutOfBounds)?;
    values
        .get(start..end)
        .ok_or(RegionArenaError::OffsetOutOfBounds)
}
