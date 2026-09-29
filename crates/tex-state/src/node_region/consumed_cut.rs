//! Bounded shallow projection of one consumed direct-record cut chunk.

use core::ops::Range;

use super::*;
use crate::node_record::NodeAnnexCopyReader;

/// Copies only the selected direct records from one source logical chunk.
/// Every child key is mapped to an already destination-owned closure before
/// its parent record is published. The source remains readable, and failure
/// restores both destination lanes to their exact operation marks.
pub(crate) fn copy_consumed_direct_cut_into<Source, Destination>(
    pool: &mut NodePool,
    source: &NodeRegion<Source>,
    source_chunk: usize,
    local_records: Range<usize>,
    destination: &mut NodeRegion<Destination>,
    semantic_identity_enabled: bool,
    mut map_child: impl FnMut(PageListId) -> Option<PageListId>,
) -> Result<PageListId, ForkArenaError> {
    pool.validate_region(source)?;
    pool.validate_region(destination)?;
    let records =
        source
            .pub_arena
            .read_consumed_cut_values(&pool.chunks, source_chunk, local_records)?;
    let node_mark = destination.pub_arena.operation_mark(&pool.chunks);
    let annex_mark = destination.annex_arena.operation_mark(&pool.annex_chunks);
    let mut annex_reader = NodeAnnexCopyReader::new(&source.annex_arena);
    let copied = (|| {
        let mut rewritten = Vec::with_capacity(records.len());
        let mut dependency_floor = usize::MAX;
        let mut paired_floor = usize::MAX;
        let mut identity = semantic_identity_enabled.then(SemanticSequenceIdentity::empty);
        for record in records {
            if let Some(identity) = &mut identity {
                identity.push_back(record.semantic_identity(NodeAnnexView::new(
                    &pool.annex_chunks,
                    &source.annex_arena,
                )));
            }
            let mut relocated = None;
            let (child_floor, child_annex_floor) = destination
                .pub_arena
                .dependency_floors_for_region_lists(&pool.chunks, |visit| {
                    relocated = record.reencode_between_regions(
                        &mut pool.annex_chunks,
                        &mut annex_reader,
                        &mut destination.annex_arena,
                        |child| {
                            let mapped = map_child(child)?;
                            visit(mapped.coordinate());
                            Some(mapped)
                        },
                    );
                    relocated.is_some().then_some(())
                })?;
            let (record, annex_floor) = relocated.ok_or(ForkArenaError::InvalidRange)?;
            dependency_floor = dependency_floor.min(child_floor.unwrap_or(usize::MAX));
            paired_floor = paired_floor
                .min(annex_floor.unwrap_or(usize::MAX))
                .min(child_annex_floor.unwrap_or(usize::MAX));
            rewritten.push(record);
        }
        let mut root = crate::fork_arena::ArenaListId::empty();
        let reservation = destination.pub_arena.reserve_constructed_list_run(
            &mut pool.chunks,
            &mut root,
            rewritten.len(),
        )?;
        reservation.publish(
            &rewritten,
            (dependency_floor != usize::MAX).then_some(dependency_floor),
            (paired_floor != usize::MAX).then_some(paired_floor),
        )?;
        destination
            .pub_arena
            .finish_constructed_list(&mut pool.chunks, root)?;
        Ok(PageListId::from_parts(root, identity))
    })();
    match copied {
        Ok(root) => {
            // Mapped children may keep borrowed coordinates.
            pool.inherit_borrows(source, destination);
            Ok(root)
        }
        Err(error) => {
            destination
                .pub_arena
                .restore_operation(&mut pool.chunks, node_mark)
                .expect("failed shallow destination restores its node mark");
            destination
                .annex_arena
                .restore_operation(&mut pool.annex_chunks, annex_mark)
                .expect("failed shallow destination restores its annex mark");
            Err(error)
        }
    }
}
