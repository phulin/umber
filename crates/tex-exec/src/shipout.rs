//! Source-free page and PDF-form staging transaction.

use tex_state::node::Node;
use tex_state::token::TokenWord;
use tex_state::{InputSummary, PdfFormArtifact, PdfFormRecord, PrintSink, Universe};

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
    pub(crate) output_open_context: Option<String>,
    pub(crate) pending_end: usize,
    pub(crate) announce_openout: bool,
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

pub(crate) type WriteReplayHost<'a> =
    dyn FnMut(&mut Universe, PrintSink, &[TokenWord]) -> Result<ExpandedWrite, ExecError> + 'a;
pub(crate) type TextReplayHost<'a> = dyn FnMut(&mut Universe, ReplayTextKind, &[TokenWord]) -> Result<ExpandedReplayText, ExecError>
    + 'a;

/// Staging capabilities borrowed for one atomic page/form traversal.
pub(crate) struct ShipoutTransaction<'a> {
    write: &'a mut WriteReplayHost<'a>,
    replay: &'a mut TextReplayHost<'a>,
}

impl<'a> ShipoutTransaction<'a> {
    pub(crate) fn new(
        write: &'a mut WriteReplayHost<'a>,
        replay: &'a mut TextReplayHost<'a>,
    ) -> Self {
        Self { write, replay }
    }

    pub(crate) fn stage_page(
        &mut self,
        node: Node,
        input_summary: InputSummary,
        origin: ShipoutOrigin,
        stores: &mut Universe,
        emit_dvi: bool,
    ) -> Result<Option<CommittedPagePublication>, ExecError> {
        let prior_attempt = stores.world().active_effect_output_attempt();
        let output_attempt =
            prior_attempt.unwrap_or_else(|| stores.world_mut().allocate_effect_output_attempt());
        stores
            .world_mut()
            .set_active_effect_output_attempt(Some(output_attempt));
        let publication = transaction::stage_page(
            node,
            input_summary,
            origin,
            stores,
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
        form: PdfFormRecord,
        stores: &mut Universe,
    ) -> Result<PdfFormArtifact, ExecError> {
        transaction::stage_form(form, stores, self.write, self.replay)
    }
}
