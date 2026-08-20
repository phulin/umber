//! Atomic bundle reservation and admitted-slice resolution.

use core::num::NonZeroUsize;
use std::sync::Arc;

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
        RuntimeOriginEntry,
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

pub(super) fn candidate_token_list_view<'a>(
    arena: &'a ConcreteArena,
    coordinate: RuntimeTokenListCoordinate,
) -> Result<RuntimeTokenListView<'a>, RegionArenaError> {
    let columns = arena.resolve_columns(coordinate.owner())?;
    let row = columns
        .token_lists
        .get(coordinate.row.offset() as usize)
        .ok_or(RegionArenaError::OffsetOutOfBounds)?;
    validate_token_identity(coordinate, row)?;
    Ok(RuntimeTokenListView {
        coordinate,
        semantic_id: row.semantic_id,
        tokens: resolve_local_span(&columns.token_words, row.tokens)?,
        provenance: resolve_local_span(&columns.provenance_roots, row.provenance)?,
    })
}

pub(super) fn candidate_macro_view<'a>(
    arena: &'a ConcreteArena,
    coordinate: RuntimeMacroCoordinate,
) -> Result<RuntimeMacroView<'a>, RegionArenaError> {
    let columns = arena.resolve_columns(coordinate.owner())?;
    let offset = coordinate.row.offset() as usize;
    let record = columns
        .macro_records
        .get(offset)
        .ok_or(RegionArenaError::OffsetOutOfBounds)?;
    let roots = columns
        .macro_roots
        .get(offset)
        .ok_or(RegionArenaError::OffsetOutOfBounds)?;
    validate_macro_identity(coordinate, record, roots)?;
    let parameter_text = candidate_token_list_view(arena, roots.parameter_text)?;
    let replacement_text = candidate_token_list_view(arena, roots.replacement_text)?;
    let definition_origin = roots.definition_origin;
    let parameter_origins = resolve_local_span(&columns.provenance_roots, roots.parameter_origins)?;
    let replacement_origins =
        resolve_local_span(&columns.provenance_roots, roots.replacement_origins)?;
    Ok(RuntimeMacroView {
        coordinate,
        record,
        parameter_text,
        replacement_text,
        definition_origin,
        parameter_origins,
        replacement_origins,
    })
}

pub(super) fn candidate_glue_view<'a>(
    arena: &'a ConcreteArena,
    coordinate: RuntimeGlueCoordinate,
) -> Result<RuntimeGlueView<'a>, RegionArenaError> {
    let columns = arena.resolve_columns(coordinate.owner())?;
    let spec = columns
        .glue_specs
        .get(coordinate.row.offset() as usize)
        .ok_or(RegionArenaError::OffsetOutOfBounds)?;
    Ok(RuntimeGlueView { coordinate, spec })
}

/// Seals the mutable tail and adds only previously absent region owners.
pub(super) fn publish_candidate_regions(
    arena: &mut ConcreteArena,
    destination: &mut ConcreteRegions,
) -> Result<(), RegionArenaError> {
    arena.seal_active()?;
    for root in &arena.base.regions {
        retain_root_if_absent(destination, root.owner.key, root.uses, &root.owner)?;
    }
    for owner in &arena.sealed_suffix {
        retain_root_if_absent(destination, owner.key, NonZeroUsize::MIN, owner)?;
    }
    Ok(())
}

/// Clones only the one owner per already-immutable region. The mutable active
/// region is deliberately excluded and is copied into a fresh child namespace
/// by the registry's cold fork path.
pub(super) fn clone_sealed_regions(arena: &ConcreteArena) -> ConcreteRegions {
    let mut regions = arena.base.clone();
    regions
        .regions
        .extend(
            arena
                .sealed_suffix
                .iter()
                .map(|owner| super::super::RuntimeValueRegionRoot {
                    owner: Arc::clone(owner),
                    uses: NonZeroUsize::MIN,
                }),
        );
    regions
}

fn retain_root_if_absent(
    destination: &mut ConcreteRegions,
    key: ChunkOwner,
    uses: NonZeroUsize,
    owner: &super::super::SealedOwner<
        Token,
        RuntimeTokenListRow,
        RuntimeMacroRecord,
        RuntimeMacroRootRow,
        GlueSpec,
        RuntimeOriginEntry,
    >,
) -> Result<(), RegionArenaError> {
    match destination.root_position(key) {
        Ok(_) => Ok(()),
        Err(index) => {
            if index != destination.regions.len() {
                return Err(RegionArenaError::InvalidMark);
            }
            destination
                .regions
                .try_reserve(1)
                .map_err(|_| RegionArenaError::AllocationFailed)?;
            destination
                .regions
                .push(super::super::RuntimeValueRegionRoot {
                    owner: Arc::clone(owner),
                    uses,
                });
            Ok(())
        }
    }
}

pub(super) fn validate_private_truncate(
    arena: &ConcreteArena,
    mark: RuntimeValueRegionMark,
) -> Result<(), RegionArenaError> {
    arena.validate_mark(mark)?;
    for owner in arena
        .sealed_suffix
        .iter()
        .skip(mark.sealed_regions as usize)
    {
        if Arc::strong_count(owner) != 1 {
            return Err(RegionArenaError::InvalidMark);
        }
    }
    Ok(())
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
        provenance: provenance_span(admitted, row.provenance)?,
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

pub(super) fn provenance_span<'a>(
    admitted: &ConcreteAdmission<'a>,
    span: LocalSpan<RuntimeOriginEntry>,
) -> Result<&'a [RuntimeOriginEntry], RegionArenaError> {
    resolve_local_span(&admitted.region.columns.provenance_roots, span)
}

pub(super) fn validate_sparse_origins(
    roots: &[RuntimeOriginEntry],
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

pub(super) fn resolve_local_span<T>(
    values: &[T],
    span: LocalSpan<T>,
) -> Result<&[T], RegionArenaError> {
    let start = span.start as usize;
    let end = start
        .checked_add(span.len as usize)
        .ok_or(RegionArenaError::OffsetOutOfBounds)?;
    values
        .get(start..end)
        .ok_or(RegionArenaError::OffsetOutOfBounds)
}
