//! Whole-owner transfer for the consumed page-builder output carrier.
//!
//! The page builder, not a copied list coordinate, owns this path. Before it
//! releases box 255, all four surviving builder roots are copied into a fresh
//! page region. The old complete node and annex envelopes can then move as
//! one paired loan, even when the output box shares cut chunks with held-over
//! material. The loan retains the exact old region for command rollback.

use super::*;
use crate::page::{ModeListRegionPreflight, PageBuilderState, PageOutputBuilderSwap};

pub(crate) struct PageOutputCarrierAssignment {
    pub(crate) closure: DurableNodeClosure,
    pub(crate) loan: PageOutputRegionLoan,
}

pub(crate) struct PageOutputRegionLoan {
    old_region: Box<NodeRegion<PageRole>>,
    successor_region: crate::node_region::NodeRegionId,
    permit: crate::page::PageOutputRegionPermit,
    builder: PageOutputBuilderSwap,
    chunks: crate::node_region::PageInteriorTransferLoan,
}

impl PageMaterialArena<'_> {
    #[cfg(test)]
    pub(crate) fn testing_reject_output_after_seal(&mut self) {
        self.output_fail_after_seal = true;
    }

    pub(crate) fn can_take_output_carrier(&self) -> bool {
        self.output_region_permit
            .as_ref()
            .is_some_and(|permit| permit.region() == self.region.id())
    }

    pub(crate) fn take_output_carrier_to_durable(
        &mut self,
        builder: &mut PageBuilderState,
        modes: ModeListRegionPreflight,
    ) -> Result<PageOutputCarrierAssignment, OutputCarrierTakeError> {
        let old_id = self.region.id();
        if modes.region != old_id
            || !self
                .output_region_permit
                .as_ref()
                .is_some_and(|permit| permit.region() == old_id)
        {
            return Err(OutputCarrierTakeError::Promotion(
                crate::NodePromotionError::Nodes(ForkArenaError::InvalidRegion),
            ));
        }
        let output = builder.output_box();
        if output.is_empty() {
            return Err(OutputCarrierTakeError::Promotion(
                crate::NodePromotionError::Nodes(ForkArenaError::InvalidRange),
            ));
        }
        let source_root = self.region.root(self.pool, output).map_err(arena_error)?;

        // No builder slot or source chunk changes during survivor preparation.
        // The source roots come from the actual PageBuilderState owner, while
        // copied spans are admitted against their new region immediately.
        let survivors = builder.output_carrier_survivor_roots();
        let mut successor = PageMaterialRegion::new(self.pool);
        successor.semantic_identity_enabled = *self.semantic_identity_enabled;
        let copied = (|| {
            let mut spans = [PageListSpan::empty(); 4];
            for (index, root) in survivors.into_iter().enumerate() {
                let source = self.region.root(self.pool, root)?;
                let copied = copy_region_root_into(
                    self.pool,
                    self.region,
                    source,
                    &mut successor.region,
                    *self.semantic_identity_enabled,
                )?
                .page_list();
                spans[index] =
                    PageMaterialArena::new(self.pool, &mut successor).admit_span(copied)?;
            }
            Ok::<_, ForkArenaError>(spans)
        })();
        let spans = match copied {
            Ok(spans) => spans,
            Err(error) => {
                successor
                    .retire(self.pool)
                    .expect("unpublished survivor copies retire");
                return Err(arena_error(error));
            }
        };

        // The construction mark armed by prepare_box255 is not a pending
        // sealed batch. Its suffix has been copied above and remains in the
        // old region until its exact loan either commits or rolls back.
        if let Err(error) = self.region.seal_checkpoint_boundary(self.pool) {
            successor
                .retire(self.pool)
                .expect("unpublished survivor copies retire");
            return Err(arena_error(error));
        }
        #[cfg(test)]
        if std::mem::take(&mut self.output_fail_after_seal) {
            successor
                .retire(self.pool)
                .expect("rejected survivor projection retires");
            return Err(OutputCarrierTakeError::UnsupportedGeometry);
        }
        let mut durable = match self.pool.start_region::<DurableRole>() {
            Ok(durable) => durable,
            Err(error) => {
                successor
                    .retire(self.pool)
                    .expect("unpublished survivor copies retire");
                return Err(arena_error(error));
            }
        };
        let nodes = self.region.pub_arena.live_payload_interval();
        let annex = self.region.annex_arena.live_payload_interval();
        #[cfg(feature = "profiling")]
        let measurement = self
            .measure_output_carrier_transfer(output, survivors, &nodes, &annex)
            .unwrap_or(crate::measurement::OutputCarrierTransferCensus {
                observation_failures: 1,
                ..crate::measurement::OutputCarrierTransferCensus::default()
            });
        #[cfg(feature = "profiling")]
        let survivor_nodes_copied = successor.region.pub_arena.counters().source_nodes_copied;

        // One typed closure pass proves and moves the whole envelope before
        // the output slot is consumed; a decline leaves both regions intact.
        let (durable_root, chunks) = match transfer_page_interior_closure_staged(
            self.pool,
            self.region,
            source_root,
            nodes,
            annex,
            &mut durable,
        ) {
            Ok(moved) => moved,
            Err(error) => {
                assert!(
                    self.pool.retire_region(durable).is_ok(),
                    "unpublished durable region retires"
                );
                successor
                    .retire(self.pool)
                    .expect("unpublished survivor copies retire");
                return Err(match error {
                    PageInteriorClosurePreflightError::Envelope(
                        ForkArenaError::InvalidRegion | ForkArenaError::InvalidChunk,
                    ) => OutputCarrierTakeError::UnsupportedGeometry,
                    PageInteriorClosurePreflightError::Root(error)
                    | PageInteriorClosurePreflightError::Envelope(error) => arena_error(error),
                });
            }
        };
        let (taken, builder_swap) = builder.take_output_carrier_for_region_swap();
        debug_assert_eq!(taken, output);
        let permit = self
            .output_region_permit
            .take()
            .expect("page history permit was admitted");
        let closure = durable
            .into_closure(self.pool, durable_root)
            .unwrap_or_else(|(error, _)| panic!("preflighted output closure: {error:?}"));
        let successor_region = successor.region.id();
        let old_region = std::mem::replace(self.region, successor.region);
        builder.publish_output_carrier_survivors(spans);
        // Consuming the non-Copy permit binds this one old owner to this one
        // output-slot removal. It is intentionally absent in the successor.
        #[cfg(feature = "profiling")]
        crate::measurement::record_output_carrier_transfer(
            crate::measurement::OutputCarrierTransferCensus {
                survivor_nodes_copied,
                ..measurement
            },
        );
        Ok(PageOutputCarrierAssignment {
            closure,
            loan: PageOutputRegionLoan {
                old_region: Box::new(old_region),
                successor_region,
                permit,
                builder: builder_swap,
                chunks,
            },
        })
    }

    #[cfg(feature = "profiling")]
    fn measure_output_carrier_transfer(
        &self,
        output: PageListId,
        survivors: [PageListId; 4],
        nodes: &std::ops::Range<usize>,
        annex: &std::ops::Range<usize>,
    ) -> Result<crate::measurement::OutputCarrierTransferCensus, ForkArenaError> {
        use std::collections::HashSet;

        fn semantic_nodes(
            arena: &PageMaterialArena<'_>,
            roots: impl IntoIterator<Item = PageListId>,
        ) -> Result<u64, ForkArenaError> {
            let mut seen = HashSet::new();
            let mut pending = roots.into_iter().collect::<Vec<_>>();
            let mut count = 0_u64;
            while let Some(list) = pending.pop() {
                if list.is_empty() || !seen.insert(list) {
                    continue;
                }
                let cursor = arena.node_cursor(list)?;
                let expected = cursor.len();
                let mut observed = 0_usize;
                for node in cursor {
                    observed = observed.saturating_add(1);
                    node.visit_semantic_node_lists(|child| pending.push(*child));
                }
                if observed != expected {
                    return Err(ForkArenaError::InvalidChunk);
                }
                count = count.saturating_add(u64::try_from(observed).unwrap_or(u64::MAX));
            }
            Ok(count)
        }

        let output_semantic_nodes = semantic_nodes(self, [output])?;
        let all_roots_semantic_nodes = semantic_nodes(self, [output].into_iter().chain(survivors))?;
        let mut moved_envelope_nodes = 0_u64;
        self.region
            .pub_arena
            .visit_interval_values(&self.pool.chunks, &[nodes.clone()], |_| {
                moved_envelope_nodes = moved_envelope_nodes.saturating_add(1);
                Ok(())
            })?;
        let mut moved_envelope_annex_words = 0_u64;
        self.region.annex_arena.visit_interval_values(
            &self.pool.annex_chunks,
            &[annex.clone()],
            |_| {
                moved_envelope_annex_words = moved_envelope_annex_words.saturating_add(1);
                Ok(())
            },
        )?;
        Ok(crate::measurement::OutputCarrierTransferCensus {
            output_semantic_nodes,
            all_roots_semantic_nodes,
            moved_envelope_nodes,
            moved_envelope_annex_words,
            ..crate::measurement::OutputCarrierTransferCensus::default()
        })
    }

    pub(crate) fn rollback_output_carrier_loan(
        &mut self,
        builder: &mut PageBuilderState,
        owner: &mut Option<DurableNodeClosure>,
        mut loan: PageOutputRegionLoan,
    ) -> Result<(), ForkArenaError> {
        if self.region.id() != loan.successor_region {
            return Err(ForkArenaError::InvalidRegion);
        }
        let closure = owner.as_mut().ok_or(ForkArenaError::InvalidRegion)?;
        rollback_page_interior_closure(
            self.pool,
            &mut loan.old_region,
            closure.region_mut(),
            loan.chunks,
        )?;
        self.retire_durable_in_place(owner)?;
        let successor = std::mem::replace(self.region, *loan.old_region);
        builder.restore_output_carrier_region(loan.builder);
        self.output_region_permit = Some(loan.permit);
        self.pool
            .retire_region(successor)
            .map_err(|(error, _)| error)
    }

    pub(crate) fn commit_output_carrier_loan(
        &mut self,
        loan: PageOutputRegionLoan,
    ) -> Result<(), ForkArenaError> {
        self.pool
            .retire_region(*loan.old_region)
            .map_err(|(error, _)| error)
    }
}

fn arena_error(error: ForkArenaError) -> OutputCarrierTakeError {
    OutputCarrierTakeError::Promotion(crate::NodePromotionError::Nodes(error))
}
