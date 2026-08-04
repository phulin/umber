use tex_out::dvi::DviPagePlan;
use tex_state::ContentHash;

/// Main-control progress counters.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ExecutionStats {
    /// Tokens accounted by execution, including batched text and memo-hit traces.
    pub delivered_tokens: usize,
    /// Tokens processed through full main-control dispatch rather than a text span.
    ///
    /// This counts actual scalar dispatch calls; batched text spans are separate.
    pub main_control_dispatches: usize,
    /// Ordinary macro-body characters delivered through the batched main path.
    pub macro_text_span_tokens: usize,
    /// Ordinary physical-source characters delivered through the batched path.
    pub source_text_span_tokens: usize,
    pub shipped_artifacts: Vec<ContentHash>,
    /// Precompiled DVI pages aligned with `shipped_artifacts`.
    pub dvi_pages: Vec<DviPagePlan>,
    pub(crate) prepared_dvi_pages: Vec<PreparedDviPage>,
    pub dumped_format: bool,
    pub format_dump_receipt: Option<crate::FormatDumpReceipt>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum DispatchAction {
    Continue,
    End,
    NotConsumed,
    Shipout(PreparedDviPage),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparedDviPage {
    pub(crate) hash: ContentHash,
    pub(crate) plan: DviPagePlan,
    pub(crate) committed_effects: Box<[tex_state::EffectRecord]>,
    pub(crate) publication: tex_state::ArtifactPublicationRecord,
    pub(crate) receipt: tex_state::PageOutputPublicationReceiptId,
}

/// One page publication whose artifact transaction has committed.
pub(crate) struct CommittedPagePublication {
    pub(crate) artifact: tex_state::PageOutputPublicationReceipt,
    pub(crate) dvi: Option<PreparedDviPage>,
    pub(crate) revision_candidate: Option<tex_state::OutputArtifactPublicationCandidate>,
    pub(crate) effects: std::ops::Range<usize>,
}

impl PreparedDviPage {
    #[doc(hidden)]
    #[must_use]
    pub const fn publication(&self) -> tex_state::ArtifactPublicationRecord {
        self.publication
    }

    #[doc(hidden)]
    #[must_use]
    pub const fn receipt(&self) -> tex_state::PageOutputPublicationReceiptId {
        self.receipt
    }
    /// Identity of the artifact published by the same committed shipout.
    #[must_use]
    pub const fn hash(&self) -> ContentHash {
        self.hash
    }

    /// Returns the detached page plan prepared before the artifact commit.
    #[must_use]
    pub fn into_plan(self) -> DviPagePlan {
        self.plan
    }
}
