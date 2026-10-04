//! Ordered evidence publication after one direct typed semantic call.
use super::*;

impl<G> MainControl<G> {
    /// Publishes evidence only for the active observation owner.
    pub(super) fn publish_semantic_observations(
        &mut self,
        stores: &mut Universe<G>,
        output_start: OperationOutputStart,
        effect: Option<EffectRecord>,
        assignment_receipts: Option<Vec<tex_command::MutationRecord>>,
    ) {
        #[cfg(feature = "profiling")]
        tex_state::measurement::record_hot_core_phase(
            tex_state::measurement::HotCorePhase::EvidencePublication,
        );
        #[cfg(feature = "profiling")]
        let _evidence_allocation_scope = tex_state::measurement::hot_core_allocation_scope(
            tex_state::measurement::HotCoreAllocationOwner::EvidencePublication,
        );
        {
            // These records are produced by applying the step, after the
            // command-processor episode's own borrow has ended. Named token
            // pushes were already sent directly to this operation's optional
            // commit sink while their live command context was admitted.
            let mut records: Vec<CommandObservation> = Vec::new();
            let effects = committed_stream_effect_observations(
                output_start.effect_count,
                output_start.prepared_page_count,
                stores,
                &self.execution.prepared_dvi_pages,
            );
            records.extend(
                assignment_receipts
                    .into_iter()
                    .flatten()
                    .map(CommandObservation::Mutation),
            );
            if let Some(effect) = effect {
                records.push(CommandObservation::Effect(effect));
            }
            records.extend(effects.into_iter().map(CommandObservation::Effect));
            for shipout in committed_shipout_observations(output_start.artifact_count, stores) {
                records.push(CommandObservation::Effect(shipout));
            }
            self.observe_committed(records);
        }
    }
}
