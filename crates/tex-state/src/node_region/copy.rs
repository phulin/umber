//! Chunk-batched explicit copies between independently owned node regions.

use super::*;
use smallvec::SmallVec;

pub(super) struct CopyContext<'a> {
    pool: &'a mut ChunkPool<RegionNode>,
    annex_pool: &'a mut ChunkPool<u32>,
    source: &'a ForkArena<RegionNode, PageMaterialLane>,
    source_annex: &'a ForkArena<u32, NodeAnnexLane>,
    destination: &'a mut ForkArena<RegionNode, PageMaterialLane>,
    destination_annex: &'a mut ForkArena<u32, NodeAnnexLane>,
    stack: Vec<PageListId>,
    semantic_identity_enabled: bool,
}

impl<'a> CopyContext<'a> {
    pub(super) fn new(
        pool: &'a mut ChunkPool<RegionNode>,
        annex_pool: &'a mut ChunkPool<u32>,
        source: &'a ForkArena<RegionNode, PageMaterialLane>,
        source_annex: &'a ForkArena<u32, NodeAnnexLane>,
        destination: &'a mut ForkArena<RegionNode, PageMaterialLane>,
        destination_annex: &'a mut ForkArena<u32, NodeAnnexLane>,
        semantic_identity_enabled: bool,
    ) -> Self {
        Self {
            pool,
            annex_pool,
            source,
            source_annex,
            destination,
            destination_annex,
            stack: Vec::new(),
            semantic_identity_enabled,
        }
    }

    pub(super) fn copy_list(
        &mut self,
        list: PageListId,
    ) -> Result<(PageListId, usize), ForkArenaError> {
        if list.is_empty() {
            return Ok((PageListId::empty(), 0));
        }
        if self.stack.contains(&list) {
            return Err(ForkArenaError::InvalidRegion);
        }
        self.stack.push(list);
        let result = self.copy_nonempty_list(list);
        self.stack.pop();
        result
    }

    fn copy_nonempty_list(
        &mut self,
        list: PageListId,
    ) -> Result<(PageListId, usize), ForkArenaError> {
        let admitted = self.source.admit_owned_root(self.pool, list.coordinate())?;
        let mut cursors = SmallVec::<[AdmittedListChunkCursor<PageMaterialLane>; 8]>::new();
        let mut cursor =
            self.source
                .admitted_tail_chunk_from_root(self.pool, list.coordinate(), admitted)?;
        while let Some(current) = cursor {
            cursor = self.source.admitted_previous_chunk(self.pool, &current)?;
            cursors.push(current);
        }
        let mut root = crate::fork_arena::ArenaListId::empty();
        let mut count = list.len();
        let mut computed_identity = (self.semantic_identity_enabled
            && list.semantic_identity().is_none())
        .then(SemanticSequenceIdentity::empty);
        let mut records = SmallVec::<[RegionNode; 16]>::new();
        for mut cursor in cursors.into_iter().rev() {
            records.clear();
            if let Some((_, source)) = self.source.admitted_remaining_chunk(self.pool, &mut cursor)
            {
                source.for_each(|record| records.push(*record));
            }
            let mut dependency_floor = usize::MAX;
            let mut paired_floor = usize::MAX;
            for record in records.iter_mut().filter(|record| !record.is_inline_leaf()) {
                let mut children = SmallVec::<[PageListId; 4]>::new();
                record
                    .visit_node_lists(
                        NodeAnnexView::new(self.annex_pool, self.source_annex),
                        |child| {
                            children.push(child);
                        },
                    )
                    .ok_or(ForkArenaError::InvalidRange)?;
                for child in &mut children {
                    let (copied, child_count) = self.copy_list(*child)?;
                    *child = copied;
                    count = count.saturating_add(child_count);
                }
                let mut children = children.into_iter();
                let mut reencoded = None;
                let (child_floor, child_annex_floor) = self
                    .destination
                    .dependency_floors_for_region_lists(self.pool, |visit| {
                        reencoded = record.reencode_between_regions(
                            self.annex_pool,
                            self.source_annex,
                            self.destination_annex,
                            |_| {
                                let child = children.next()?;
                                visit(child.coordinate());
                                Some(child)
                            },
                        );
                        reencoded.as_ref().map(|_| ())
                    })?;
                if children.next().is_some() {
                    return Err(ForkArenaError::InvalidRegion);
                }
                let (relocated, annex_floor) = reencoded.ok_or(ForkArenaError::InvalidRange)?;
                *record = relocated;
                dependency_floor = dependency_floor.min(child_floor.unwrap_or(usize::MAX));
                paired_floor = paired_floor
                    .min(child_annex_floor.unwrap_or(usize::MAX))
                    .min(annex_floor.unwrap_or(usize::MAX));
            }
            if let Some(identity) = &mut computed_identity {
                let annex = NodeAnnexView::new(self.annex_pool, self.destination_annex);
                for record in &records {
                    identity.push_back(record.semantic_identity(annex));
                }
            }
            self.destination.append_constructed_list_run(
                self.pool,
                &mut root,
                &records,
                (dependency_floor != usize::MAX).then_some(dependency_floor),
                (paired_floor != usize::MAX).then_some(paired_floor),
            )?;
        }
        self.destination.finish_constructed_list(self.pool, root)?;
        let identity = if self.semantic_identity_enabled {
            list.semantic_identity()
                .map(|hash| SemanticSequenceIdentity::from_raw(hash, list.len()))
                .or(computed_identity)
        } else {
            None
        };
        Ok((PageListId::from_parts(root, identity), count))
    }
}
