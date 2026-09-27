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
        if let Err(error) = preflight_page_interior_closure(
            self.pool,
            self.region,
            source_root,
            nodes.clone(),
            annex.clone(),
            &durable,
        ) {
            assert!(
                self.pool.retire_region(durable).is_ok(),
                "unpublished durable region retires"
            );
            successor
                .retire(self.pool)
                .expect("unpublished survivor copies retire");
            return Err(if error == ForkArenaError::InvalidRegion {
                OutputCarrierTakeError::UnsupportedGeometry
            } else {
                arena_error(error)
            });
        }

        let (taken, builder_swap) = builder.take_output_carrier_for_region_swap();
        debug_assert_eq!(taken, output);
        let permit = self
            .output_region_permit
            .take()
            .expect("page history permit was admitted");
        let (durable_root, chunks) = transfer_page_interior_closure(
            self.pool,
            self.region,
            source_root,
            nodes,
            annex,
            &mut durable,
        )
        .expect("paired whole-owner transfer was preflighted before consuming the output slot");
        let closure = durable
            .into_closure(self.pool, durable_root)
            .unwrap_or_else(|(error, _)| panic!("preflighted output closure: {error:?}"));
        let successor_region = successor.region.id();
        let old_region = std::mem::replace(self.region, successor.region);
        builder.publish_output_carrier_survivors(spans);
        // Consuming the non-Copy permit binds this one old owner to this one
        // output-slot removal. It is intentionally absent in the successor.
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
