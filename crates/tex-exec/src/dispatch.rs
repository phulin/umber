use tex_out::dvi::DviPagePlan;
use tex_state::ContentHash;

/// Validated artifact rows closed by the executor at a revision boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactLedger {
    artifacts: Vec<tex_state::CommittedArtifact>,
    publications: Vec<tex_state::ArtifactPublicationRecord>,
}

impl ArtifactLedger {
    pub(crate) fn new(
        artifacts: Vec<tex_state::CommittedArtifact>,
        publications: Vec<tex_state::ArtifactPublicationRecord>,
    ) -> Result<Self, RevisionOutputPatchError> {
        if artifacts.len() != publications.len() {
            return Err(RevisionOutputPatchError::ArtifactPublicationCount);
        }
        Ok(Self {
            artifacts,
            publications,
        })
    }

    #[must_use]
    pub fn artifacts(&self) -> &[tex_state::CommittedArtifact] {
        &self.artifacts
    }

    #[must_use]
    pub fn publications(&self) -> &[tex_state::ArtifactPublicationRecord] {
        &self.publications
    }

    pub fn into_parts(
        self,
    ) -> (
        Vec<tex_state::CommittedArtifact>,
        Vec<tex_state::ArtifactPublicationRecord>,
    ) {
        (self.artifacts, self.publications)
    }
}

/// Executor-closed output delta for one completed revision execution.
///
/// The constructor is private: callers receive only ledgers whose aligned
/// publication rows and optional DVI plan stream were validated together.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RevisionOutputPatch {
    effects: tex_state::EffectJournal,
    artifacts: ArtifactLedger,
    dvi_pages: Vec<DviPagePlan>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RevisionOutputPatchError {
    ArtifactPublicationCount,
    DviPageCount,
    DviPublicationMismatch,
    DviArtifactMismatch,
}

impl RevisionOutputPatch {
    #[must_use]
    pub const fn effects(&self) -> &tex_state::EffectJournal {
        &self.effects
    }

    #[must_use]
    pub const fn artifacts(&self) -> &ArtifactLedger {
        &self.artifacts
    }

    #[must_use]
    pub fn dvi_pages(&self) -> &[DviPagePlan] {
        &self.dvi_pages
    }

    #[must_use]
    pub fn dvi_publications(&self) -> &[tex_state::ArtifactPublicationRecord] {
        if self.dvi_pages.is_empty() {
            &[]
        } else {
            self.artifacts.publications()
        }
    }

    pub fn into_parts(self) -> (tex_state::EffectJournal, ArtifactLedger, Vec<DviPagePlan>) {
        (self.effects, self.artifacts, self.dvi_pages)
    }

    /// Re-closes a revision payload assembled from already validated prefix,
    /// live-patch, and suffix rows.
    ///
    /// Incremental execution may select rows from multiple executor-closed
    /// patches. This constructor keeps the positional validation at the
    /// executor boundary instead of making the incremental caller mirror the
    /// ledgers in parallel vectors.
    pub fn recompose(
        effects: tex_state::EffectJournal,
        artifacts: Vec<tex_state::CommittedArtifact>,
        publications: Vec<tex_state::ArtifactPublicationRecord>,
        dvi_pages: Vec<DviPagePlan>,
    ) -> Result<Self, RevisionOutputPatchError> {
        let artifacts = ArtifactLedger::new(artifacts, publications)?;
        if !dvi_pages.is_empty() && dvi_pages.len() != artifacts.artifacts.len() {
            return Err(RevisionOutputPatchError::DviPageCount);
        }
        Ok(Self {
            effects,
            artifacts,
            dvi_pages,
        })
    }
}

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
    pub(crate) effects: std::ops::Range<usize>,
    pub(crate) effect_output_attempt: Option<tex_state::EffectOutputAttemptId>,
}

impl PreparedDviPage {
    /// Borrows the page-local DVI plan retained by the canonical output
    /// ledger.
    #[doc(hidden)]
    #[must_use]
    pub const fn plan(&self) -> &DviPagePlan {
        &self.plan
    }

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
