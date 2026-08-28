//! Source-free page and PDF-form staging transaction.

use tex_state::{PdfFormArtifact, PdfFormRecord, PrintSink, Universe};

use crate::ExecError;
use crate::dispatch::CommittedPagePublication;

pub(crate) mod direct;
mod transaction;

pub use transaction::retry_unavailable_stream_open;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReplayTextKind {
    Special,
    PdfLiteral,
}

/// Detached job state captured before staging begins.
pub(crate) struct ShipoutOrigin {
    pub(crate) output_open_context: String,
    pub(crate) announce_openout: bool,
}

/// Handle-free geometry emitted only after a page publication commits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ShipoutGeometry {
    pub(crate) page_width_sp: i64,
    pub(crate) page_height_sp: i64,
    pub(crate) counts: [i32; 10],
}

/// Publication callback owned by the surrounding execution boundary.
///
/// Main control may attach its live line/source attribution while forwarding
/// this DTO to an in-process observer. Shipout never stores those runtime
/// coordinates in the committed output.
pub(crate) trait ShipoutGeometrySink {
    fn committed_shipout_geometry(&mut self, geometry: ShipoutGeometry);
}

pub(crate) struct ExpandedWrite {
    pub(crate) text: String,
    pub(crate) publication: WritePublication,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WritePublication {
    Transactional,
}

impl ExpandedWrite {
    pub(crate) fn transactional(text: String) -> Self {
        Self {
            text,
            publication: WritePublication::Transactional,
        }
    }
}

pub(crate) struct ExpandedReplayText(pub(crate) Vec<u8>);

pub(crate) type WriteReplayHost<'a, G> = dyn FnMut(
        &mut Universe<G>,
        &mut tex_state::diagnostic::DiagnosticEffects,
        PrintSink,
        tex_state::ShipoutTokenSource<G>,
    ) -> Result<ExpandedWrite, ExecError>
    + 'a;
pub(crate) type TextReplayHost<'a, G> = dyn FnMut(
        &mut Universe<G>,
        &mut tex_state::diagnostic::DiagnosticEffects,
        ReplayTextKind,
        tex_state::ShipoutTokenSource<G>,
    ) -> Result<ExpandedReplayText, ExecError>
    + 'a;

/// Staging capabilities borrowed for one atomic page/form traversal.
pub(crate) struct ShipoutTransaction<'a, G> {
    write: &'a mut WriteReplayHost<'a, G>,
    replay: &'a mut TextReplayHost<'a, G>,
    source_resolver: &'a dyn crate::output_provenance::ArtifactSourceResolver,
    provenance_demand: tex_state::ProvenanceDemand,
    provenance_budget_bytes: usize,
    geometry_sink: &'a mut dyn ShipoutGeometrySink,
    diagnostic_effects: tex_state::diagnostic::DiagnosticEffects,
}

impl<'a, G> ShipoutTransaction<'a, G> {
    pub(crate) fn new(
        write: &'a mut WriteReplayHost<'a, G>,
        replay: &'a mut TextReplayHost<'a, G>,
        source_resolver: &'a dyn crate::output_provenance::ArtifactSourceResolver,
        provenance_demand: tex_state::ProvenanceDemand,
        provenance_budget_bytes: usize,
        geometry_sink: &'a mut dyn ShipoutGeometrySink,
    ) -> Self {
        Self {
            write,
            replay,
            source_resolver,
            provenance_demand,
            provenance_budget_bytes,
            geometry_sink,
            diagnostic_effects: tex_state::diagnostic::DiagnosticEffects::new(),
        }
    }

    pub(crate) fn take_diagnostic_effects(&mut self) -> tex_state::diagnostic::DiagnosticEffects {
        std::mem::take(&mut self.diagnostic_effects)
    }

    pub(crate) fn stage_page(
        &mut self,
        source: direct::ShipoutRoot,
        region: Option<
            tex_state::fork_arena::OperationMark<tex_state::fork_arena::PageMaterialLane>,
        >,
        origin: ShipoutOrigin,
        pending_effect_end: usize,
        stores: &mut Universe<G>,
        emit_dvi: bool,
    ) -> Result<Option<CommittedPagePublication>, ExecError> {
        let prior_attempt = stores.world().active_effect_output_attempt();
        let output_attempt =
            prior_attempt.unwrap_or_else(|| stores.world_mut().allocate_effect_output_attempt());
        stores
            .world_mut()
            .set_active_effect_output_attempt(Some(output_attempt));
        let publication = transaction::stage_page(
            source,
            region,
            origin,
            pending_effect_end,
            stores,
            &mut self.diagnostic_effects,
            self.source_resolver,
            self.provenance_demand,
            self.provenance_budget_bytes,
            self.geometry_sink,
            emit_dvi,
            self.write,
            self.replay,
        );
        stores
            .world_mut()
            .set_active_effect_output_attempt(prior_attempt);
        let mut publication = publication?;
        if let Some(publication) = publication.as_mut() {
            publication.effect_output_attempt = Some(output_attempt);
        }
        if let Some(publication) = publication.as_ref() {
            let live = publication.artifact.effect();
            stores
                .world_mut()
                .commit_effect_publication_winner(None, live, output_attempt, None);
        }
        Ok(publication)
    }

    pub(crate) fn stage_form(
        &mut self,
        form: PdfFormRecord<G>,
        stores: &mut Universe<G>,
    ) -> Result<PdfFormArtifact, ExecError> {
        transaction::stage_form(
            form,
            stores,
            &mut self.diagnostic_effects,
            self.write,
            self.replay,
        )
    }
}
