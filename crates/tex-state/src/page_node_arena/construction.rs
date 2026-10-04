//! Direct final-record construction for live mode tails and real batch builders.
use super::*;

impl PageMaterialArena<'_> {
    /// Appends one node to the actual semantic mode-list tail.
    pub(crate) fn construct_mode_node(
        &mut self,
        slot: &mut ModePageListSlot,
        initialize: impl FnOnce(crate::NodeDestination<'_>),
    ) -> Result<ConstructedNodeMetadata, ForkArenaError> {
        if self.region.active_annex_operation.is_some() {
            return Err(ForkArenaError::ActiveBuilder);
        }
        self.region
            .pub_arena
            .admit_owned_root(&self.pool.chunks, slot.list().coordinate())?;
        let move_enabled = slot.move_enabled || slot.span.is_empty();
        let mut identity = (*self.semantic_identity_enabled).then(|| {
            slot.list()
                .sequence_identity()
                .expect("demand-enabled mode root carries identity")
        });
        let (item_identity, metadata) = self.construct_node_in_root(
            &mut slot.span.list.coordinate,
            identity.is_some(),
            initialize,
        )?;
        if let Some(identity) = &mut identity {
            identity
                .push_back(item_identity.expect("demand-enabled construction computes identity"));
            self.region
                .pub_arena
                .record_identity_work(crate::fork_arena::SequenceSummaryWork {
                    hashed_values: 1,
                    ..crate::fork_arena::SequenceSummaryWork::default()
                });
        }
        slot.span.list = PageListId::from_parts(slot.span.list.coordinate, identity);
        slot.move_enabled = move_enabled;
        Ok(metadata)
    }
    /// Constructs directly in the final slot of an actual multi-node producer.
    pub(crate) fn construct_active_list(
        &mut self,
        builder: &mut PageMaterialActiveListBuilder,
        initialize: impl FnOnce(crate::NodeDestination<'_>),
    ) -> Result<ConstructedNodeMetadata, ForkArenaError> {
        let root = self
            .region
            .pub_arena
            .active_list_root_mut(&mut builder.inner)?;
        let (item_identity, metadata) =
            self.construct_node_in_root(root, builder.identity.is_some(), initialize)?;
        if let Some(item_identity) = item_identity {
            if let Some(identity) = &mut builder.identity {
                identity.push_back(item_identity);
            }
            builder.identity_work.hashed_values =
                builder.identity_work.hashed_values.saturating_add(1);
        }
        Ok(metadata)
    }

    fn construct_node_in_root(
        &mut self,
        root: &mut ArenaListId<PageMaterialLane>,
        identity_enabled: bool,
        initialize: impl FnOnce(crate::NodeDestination<'_>),
    ) -> Result<(Option<u64>, ConstructedNodeMetadata), ForkArenaError> {
        let (constructed, annex_operation) = {
            let mut annex =
                NodeAnnexWriter::new(&mut self.pool.annex_chunks, &mut self.region.annex_arena);
            let constructed = self.region.pub_arena.construct_region_value(
                &mut self.pool.chunks,
                root,
                identity_enabled,
                &mut annex,
                |slot, annex| initialize(crate::NodeDestination::new_record(slot, annex)),
                |record, annex, arena, pool| {
                    let annex_dependency_floor = annex.dependency_floor();
                    let annex = annex.view();
                    let font = record.direct_font(annex);
                    let (dependency_floor, child_annex_floor) = arena
                        .dependency_floors_for_region_lists(pool, |visit| {
                            record
                                .visit_node_lists(annex, |child| visit(child.coordinate()))
                                .ok_or(ForkArenaError::InvalidRange)
                        })?;
                    let paired_dependency_floor = match (child_annex_floor, annex_dependency_floor)
                    {
                        (Some(child), Some(annex)) => Some(child.min(annex)),
                        (child, annex) => child.or(annex),
                    };
                    let metadata = ConstructedNodeMetadata {
                        tex82_words: record.tex_memory_words(annex, false),
                        etex_words: record.tex_memory_words(annex, true),
                        font,
                    };
                    let identity = if identity_enabled {
                        record.semantic_identity(annex)
                    } else {
                        0
                    };
                    Ok((
                        identity,
                        metadata,
                        dependency_floor,
                        paired_dependency_floor,
                    ))
                },
            );
            (constructed, annex.operation())
        };
        let (item_identity, metadata) = match constructed {
            Ok(constructed) => constructed,
            Err(error) => {
                if let Some(operation) = annex_operation {
                    self.region
                        .annex_arena
                        .restore_unsealed_copy_operation(&mut self.pool.annex_chunks, operation)
                        .expect("failed destination construction restores its annex suffix");
                }
                return Err(error);
            }
        };
        Ok((item_identity, metadata))
    }
}
