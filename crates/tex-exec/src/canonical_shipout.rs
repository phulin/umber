//! Source-free canonical page and PDF-form staging transaction.

use tex_state::ids::TokenListId;
use tex_state::node::Node;
use tex_state::{InputSummary, PdfFormArtifact, PdfFormRecord, PrintSink, Universe};

use crate::ExecError;
use crate::dispatch::PreparedDviPage;

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

pub(crate) struct ExpandedWrite(pub(crate) String);

pub(crate) struct ExpandedReplayText(pub(crate) Vec<u8>);

pub(crate) type WriteReplayHost<'a> =
    dyn FnMut(&mut Universe, PrintSink, TokenListId) -> Result<ExpandedWrite, ExecError> + 'a;
pub(crate) type TextReplayHost<'a> = dyn FnMut(&mut Universe, ReplayTextKind, TokenListId) -> Result<ExpandedReplayText, ExecError>
    + 'a;

/// Canonical staging capabilities borrowed for one atomic page/form traversal.
pub(crate) struct CanonicalShipoutTransaction<'a> {
    write: &'a mut WriteReplayHost<'a>,
    replay: &'a mut TextReplayHost<'a>,
}

impl<'a> CanonicalShipoutTransaction<'a> {
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
    ) -> Result<Option<PreparedDviPage>, ExecError> {
        transaction::stage_canonical_page(
            node,
            input_summary,
            origin,
            stores,
            emit_dvi,
            self.write,
            self.replay,
        )
    }

    pub(crate) fn stage_form(
        &mut self,
        form: PdfFormRecord,
        stores: &mut Universe,
    ) -> Result<PdfFormArtifact, ExecError> {
        transaction::stage_canonical_form(form, stores, self.write, self.replay)
    }
}
