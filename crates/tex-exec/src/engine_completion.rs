//! Terminal engine-output detachment and outer publication.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};

use tex_out::dvi::DviPagePlan;
use tex_state::{
    CommittedArtifact, ContentDomain, ContentHash, DetachedPdfCompletion, EffectRecord,
    EffectRetrySafety, PdfCompletionError, StreamSlot, World, WorldError,
};

use crate::dispatch::PreparedDviPage;

/// Explicit terminal capture demand. PDF collection is never selected by a
/// `Default` implementation because it walks every cold PDF family.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EngineCompletionDemand {
    pdf: bool,
}

impl EngineCompletionDemand {
    #[must_use]
    pub const fn new(pdf: bool) -> Self {
        Self { pdf }
    }

    #[must_use]
    pub const fn with_pdf() -> Self {
        Self { pdf: true }
    }

    #[must_use]
    pub const fn without_pdf() -> Self {
        Self { pdf: false }
    }

    #[must_use]
    pub const fn pdf(self) -> bool {
        self.pdf
    }
}

/// One aligned page row in terminal engine completion.
#[derive(Clone, Debug)]
pub struct DetachedPreparedPage {
    artifact: CommittedArtifact,
    dvi: Option<DviPagePlan>,
}

impl DetachedPreparedPage {
    #[must_use]
    pub const fn artifact(&self) -> &CommittedArtifact {
        &self.artifact
    }

    #[must_use]
    pub const fn dvi(&self) -> Option<&DviPagePlan> {
        self.dvi.as_ref()
    }

    #[doc(hidden)]
    pub fn into_parts(self) -> (CommittedArtifact, Option<DviPagePlan>) {
        (self.artifact, self.dvi)
    }
}

/// Handle-free output of one admitted terminal engine episode.
#[derive(Clone, Debug)]
pub struct DetachedEngineCompletion {
    effect_base: u64,
    effects: Vec<EffectRecord>,
    stream_open_contexts: Vec<Option<String>>,
    pages: Vec<DetachedPreparedPage>,
    pdf: Option<DetachedPdfCompletion>,
}

impl DetachedEngineCompletion {
    /// Replaces the post-convergence output suffix with rows detached from
    /// the accepted revision. Live engine graphs never enter this operation;
    /// it joins only already detached effects, artifacts, and DVI plans.
    #[doc(hidden)]
    pub fn splice_retained_suffix(
        &mut self,
        retained: &Self,
        new_effect_prefix: usize,
        old_effect_prefix: usize,
        new_artifact_prefix: usize,
        old_artifact_prefix: usize,
    ) -> Result<(), EngineCompletionError> {
        let new_effect_prefix = new_effect_prefix
            .saturating_sub(usize::try_from(self.effect_base).unwrap_or(usize::MAX))
            .min(self.effects.len());
        let old_effect_prefix = old_effect_prefix
            .saturating_sub(usize::try_from(retained.effect_base).unwrap_or(usize::MAX))
            .min(retained.effects.len());
        self.effects.truncate(new_effect_prefix);
        self.stream_open_contexts.truncate(new_effect_prefix);
        self.effects
            .extend(retained.effects[old_effect_prefix..].iter().cloned());
        self.stream_open_contexts.extend(
            retained.stream_open_contexts[old_effect_prefix..]
                .iter()
                .cloned(),
        );

        let new_artifact_prefix = new_artifact_prefix.min(self.pages.len());
        let old_artifact_prefix = old_artifact_prefix.min(retained.pages.len());
        self.pages.truncate(new_artifact_prefix);
        for page in &retained.pages[old_artifact_prefix..] {
            let mut page = page.clone();
            page.artifact
                .rebase_open_out_suffix(old_effect_prefix, new_effect_prefix)
                .map_err(|error| EngineCompletionError::OutputSplice(error.to_string()))?;
            self.pages.push(page);
        }
        if self.pdf.is_none() {
            self.pdf.clone_from(&retained.pdf);
        }
        validate_stream_open_contexts(&self.effects, &self.stream_open_contexts)?;
        if let Some(pdf) = &self.pdf {
            validate_pdf(pdf, self.pages.iter().map(DetachedPreparedPage::artifact))?;
        }
        Ok(())
    }

    /// Captures the terminal page projection directly from borrowed canonical
    /// output-ledger rows. The only page vector created is the final detached
    /// completion; no accumulated `PreparedDviPage` prefix is materialized.
    #[allow(clippy::too_many_arguments)] // Terminal detachment joins independent output-owner roots.
    pub(crate) fn capture_borrowed_pages(
        effect_base: u64,
        effects: Vec<EffectRecord>,
        stream_open_contexts: Vec<Option<String>>,
        artifacts: Vec<CommittedArtifact>,
        artifact_publications: &[tex_state::ArtifactPublicationRecord],
        prepared_page_count: usize,
        visit_prepared: impl FnOnce(&mut dyn FnMut(&PreparedDviPage)),
        pdf: Option<DetachedPdfCompletion>,
    ) -> Result<Self, EngineCompletionError> {
        validate_stream_open_contexts(&effects, &stream_open_contexts)?;
        if artifacts.len() != artifact_publications.len() {
            return Err(EngineCompletionError::ArtifactPublicationCount);
        }
        if prepared_page_count != 0 && prepared_page_count != artifacts.len() {
            return Err(EngineCompletionError::DviPageCount);
        }
        for (index, artifact) in artifacts.iter().enumerate() {
            validate_artifact(index, artifact, effect_base, &effects)?;
        }

        let mut pages = artifacts
            .into_iter()
            .map(|artifact| DetachedPreparedPage {
                artifact,
                dvi: None,
            })
            .collect::<Vec<_>>();
        let mut prepared_index = 0_usize;
        let mut validation = Ok(());
        visit_prepared(&mut |prepared| {
            if validation.is_err() {
                return;
            }
            let Some((page, publication)) = pages
                .get_mut(prepared_index)
                .zip(artifact_publications.get(prepared_index))
            else {
                validation = Err(EngineCompletionError::DviPageCount);
                return;
            };
            if prepared.publication != *publication {
                validation = Err(EngineCompletionError::DviPublicationMismatch);
                return;
            }
            if prepared.hash != page.artifact.hash() {
                validation = Err(EngineCompletionError::DviArtifactMismatch);
                return;
            }
            page.dvi = Some(prepared.plan.clone());
            prepared_index += 1;
        });
        validation?;
        if prepared_index != prepared_page_count {
            return Err(EngineCompletionError::DviPageCount);
        }
        if let Some(pdf) = &pdf {
            let artifacts = pages.iter().map(DetachedPreparedPage::artifact);
            validate_pdf(pdf, artifacts)?;
        }
        Ok(Self {
            effect_base,
            effects,
            stream_open_contexts,
            pages,
            pdf,
        })
    }

    #[must_use]
    pub fn effects(&self) -> &[EffectRecord] {
        &self.effects
    }

    #[must_use]
    pub fn pages(&self) -> &[DetachedPreparedPage] {
        &self.pages
    }

    #[must_use]
    pub const fn pdf(&self) -> Option<&DetachedPdfCompletion> {
        self.pdf.as_ref()
    }

    #[doc(hidden)]
    pub fn into_parts(
        self,
    ) -> (
        Vec<EffectRecord>,
        Vec<Option<String>>,
        Vec<DetachedPreparedPage>,
        Option<DetachedPdfCompletion>,
    ) {
        (
            self.effects,
            self.stream_open_contexts,
            self.pages,
            self.pdf,
        )
    }

    /// Performs all structural validation before a destination World can be
    /// mutated and consumes the detached value into a non-Clone transaction.
    pub fn into_publication(self) -> Result<PreparedEnginePublication, EnginePublicationError> {
        if self.effect_base != 0 {
            return Err(EnginePublicationError::MaterializedEffectBase);
        }
        validate_prepared(&self.effects, &self.pages, self.pdf.as_ref())?;
        Ok(PreparedEnginePublication {
            effects: self.effects,
            stream_open_contexts: self.stream_open_contexts,
            cursor: 0,
            pages: self.pages,
            pdf: self.pdf,
            retry_attempt: 0,
        })
    }
}

#[derive(Debug)]
pub enum EngineCompletionError {
    TerminalRevisionUnavailable,
    Admission(tex_state::UniverseError),
    Pdf(PdfCompletionError),
    MaterializedEffectBase,
    EffectContextCount,
    InvalidEffectContext { ordinal: usize },
    ArtifactPublicationCount,
    DviPageCount,
    DviPublicationMismatch,
    DviArtifactMismatch,
    InvalidArtifact { page: usize, message: String },
    InvalidArtifactIdentity { page: usize },
    InvalidEffectOrdinal { page: usize, ordinal: u32 },
    InvalidEffectOccurrence { page: usize, ordinal: u32 },
    OutputSplice(String),
    PdfPageCount,
    PdfPageArtifact { page: usize },
    PdfObjectIdentity(u32),
    PdfResourceIdentity(u32),
    PdfAllocationCursor { object: u32, next: u32 },
}

impl From<PdfCompletionError> for EngineCompletionError {
    fn from(error: PdfCompletionError) -> Self {
        Self::Pdf(error)
    }
}

impl fmt::Display for EngineCompletionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid terminal engine completion: {self:?}")
    }
}

impl std::error::Error for EngineCompletionError {}

/// One-based effect ordinal local to a detached completion.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct CompletionEffectOrdinal(u32);

impl CompletionEffectOrdinal {
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Detached publication failure. A safe stream-open failure can be paired
/// only with the `Retry` plan returned in the same outcome.
#[derive(Debug)]
pub struct CompletionPublicationFailure {
    ordinal: Option<CompletionEffectOrdinal>,
    slot: Option<StreamSlot>,
    path: Option<PathBuf>,
    committed_prefix: usize,
    retry_safety: EffectRetrySafety,
    message: String,
    attempt: u64,
}

impl CompletionPublicationFailure {
    #[must_use]
    pub const fn ordinal(&self) -> Option<CompletionEffectOrdinal> {
        self.ordinal
    }
    #[must_use]
    pub const fn slot(&self) -> Option<StreamSlot> {
        self.slot
    }
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }
    #[must_use]
    pub const fn committed_prefix(&self) -> usize {
        self.committed_prefix
    }
    #[must_use]
    pub const fn retry_safety(&self) -> EffectRetrySafety {
        self.retry_safety
    }
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

#[derive(Debug)]
pub enum EnginePublicationError {
    MaterializedEffectBase,
    InvalidArtifact { page: usize, message: String },
    InvalidArtifactIdentity { page: usize },
    InvalidEffectOrdinal { page: usize, ordinal: u32 },
    InvalidEffectOccurrence { page: usize, ordinal: u32 },
    PdfPageCount,
    PdfPageArtifact { page: usize },
    Destination(WorldError),
    StaleRetarget,
    Irreversible(CompletionPublicationFailure),
}

impl fmt::Display for EnginePublicationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "engine completion publication failed: {self:?}")
    }
}

impl std::error::Error for EnginePublicationError {}

/// Non-Clone publication transaction. Its cursor is the exact first effect
/// not yet committed to the destination.
#[derive(Debug)]
pub struct PreparedEnginePublication {
    effects: Vec<EffectRecord>,
    stream_open_contexts: Vec<Option<String>>,
    cursor: usize,
    pages: Vec<DetachedPreparedPage>,
    pdf: Option<DetachedPdfCompletion>,
    retry_attempt: u64,
}

impl PreparedEnginePublication {
    #[must_use]
    pub fn remaining_effects(&self) -> &[EffectRecord] {
        &self.effects[self.cursor..]
    }

    #[must_use]
    pub fn pages(&self) -> &[DetachedPreparedPage] {
        &self.pages
    }

    #[must_use]
    pub const fn pdf(&self) -> Option<&DetachedPdfCompletion> {
        self.pdf.as_ref()
    }

    /// Retargets the exact failed head and every artifact occurrence carrying
    /// its DTO-local ordinal. Validation and page reserialization finish
    /// before this plan is changed.
    pub fn retarget(
        &mut self,
        failure: &CompletionPublicationFailure,
        replacement: PathBuf,
    ) -> Result<(), EnginePublicationError> {
        let Some(ordinal) = failure.ordinal else {
            return Err(stale_retarget());
        };
        let Some(slot) = failure.slot else {
            return Err(stale_retarget());
        };
        let Some(path) = failure.path.as_ref() else {
            return Err(stale_retarget());
        };
        if failure.attempt != self.retry_attempt
            || ordinal.get() as usize != self.cursor + 1
            || !matches!(
                self.effects.get(self.cursor),
                Some(EffectRecord::StreamOpen { slot: candidate, target })
                    if *candidate == slot && target.path() == path
            )
        {
            return Err(stale_retarget());
        }

        let failed_text = path.to_string_lossy();
        let replacement_text = replacement.to_string_lossy();
        let mut rewritten = Vec::with_capacity(self.pages.len());
        for (page_index, page) in self.pages.iter().enumerate() {
            let mut artifact = page.artifact.clone();
            let old_hash = artifact.hash();
            let occurrences: Vec<_> = artifact
                .open_out_occurrences()
                .iter()
                .filter(|(_, candidate)| candidate.index() == ordinal.get())
                .map(|(index, _)| *index)
                .collect();
            if !occurrences.is_empty() {
                let mut model =
                    tex_out::PageArtifact::from_bytes(artifact.bytes()).map_err(|error| {
                        EnginePublicationError::InvalidArtifact {
                            page: page_index,
                            message: error.to_string(),
                        }
                    })?;
                for occurrence in occurrences {
                    if !model.retarget_open_out_at(
                        occurrence,
                        slot.raw(),
                        &failed_text,
                        &replacement_text,
                    ) {
                        return Err(EnginePublicationError::InvalidEffectOccurrence {
                            page: page_index,
                            ordinal: ordinal.get(),
                        });
                    }
                }
                let bytes =
                    model
                        .to_bytes()
                        .map_err(|error| EnginePublicationError::InvalidArtifact {
                            page: page_index,
                            message: error.to_string(),
                        })?;
                artifact = artifact.with_prepared_bytes(bytes);
            }
            rewritten.push((old_hash, artifact.hash(), artifact.bytes().to_vec()));
        }

        let retargeted =
            self.effects[self.cursor].retarget_detached_stream_open(slot, path, replacement);
        debug_assert!(retargeted);
        for (page, (_, _, bytes)) in self.pages.iter_mut().zip(&rewritten) {
            page.artifact = page.artifact.clone().with_prepared_bytes(bytes.clone());
        }
        if let Some(pdf) = &mut self.pdf {
            for (old, new, bytes) in &rewritten {
                pdf.retarget_page_artifact(*old, *new, bytes);
            }
        }
        self.retry_attempt = self.retry_attempt.saturating_add(1);
        Ok(())
    }

    pub fn publish(
        mut self,
        world: &mut World,
    ) -> Result<CompletionPublication, EnginePublicationError> {
        if self.cursor == 0 {
            world
                .preflight_detached_publication()
                .map_err(EnginePublicationError::Destination)?;
        } else {
            world
                .preflight_detached_retry(self.cursor)
                .map_err(EnginePublicationError::Destination)?;
        }
        match world.publish_detached_effect_records_with_contexts(
            &self.effects[self.cursor..],
            &self.stream_open_contexts[self.cursor..],
        ) {
            Ok(()) => {
                self.cursor = self.effects.len();
            }
            Err(error) => {
                let old_cursor = self.cursor;
                let committed = error.committed();
                self.cursor += committed;
                let ordinal = error
                    .failed_ordinal()
                    .and_then(|relative| u32::try_from(old_cursor).ok()?.checked_add(relative))
                    .map(CompletionEffectOrdinal);
                let failure = CompletionPublicationFailure {
                    ordinal,
                    slot: error.slot(),
                    path: error.path().map(Path::to_owned),
                    committed_prefix: self.cursor,
                    retry_safety: error.retry_safety(),
                    message: error.world_error().to_string(),
                    attempt: self.retry_attempt,
                };
                if failure.retry_safety == EffectRetrySafety::Safe && failure.ordinal.is_some() {
                    return Ok(CompletionPublication::Retry {
                        plan: self,
                        failure,
                    });
                }
                return Err(EnginePublicationError::Irreversible(failure));
            }
        }
        let artifacts = self
            .pages
            .iter()
            .map(|page| page.artifact.clone())
            .collect();
        if let Err(error) = world.publish_detached_artifacts(artifacts) {
            return Err(EnginePublicationError::Irreversible(
                CompletionPublicationFailure {
                    ordinal: None,
                    slot: None,
                    path: None,
                    committed_prefix: self.cursor,
                    retry_safety: EffectRetrySafety::NotAnEffectCommit,
                    message: error.to_string(),
                    attempt: self.retry_attempt,
                },
            ));
        }
        let pages = self
            .pages
            .into_iter()
            .map(|page| PublishedEnginePage {
                artifact: page.artifact.hash(),
                dvi: page.dvi,
            })
            .collect();
        Ok(CompletionPublication::Committed(
            CommittedEnginePublication {
                effect_count: self.effects.len(),
                pages,
                pdf: self.pdf,
            },
        ))
    }
}

#[derive(Debug)]
pub enum CompletionPublication {
    Committed(CommittedEnginePublication),
    Retry {
        plan: PreparedEnginePublication,
        failure: CompletionPublicationFailure,
    },
}

#[derive(Debug)]
pub struct PublishedEnginePage {
    artifact: ContentHash,
    dvi: Option<DviPagePlan>,
}

impl PublishedEnginePage {
    #[must_use]
    pub const fn artifact(&self) -> ContentHash {
        self.artifact
    }
    #[must_use]
    pub const fn dvi(&self) -> Option<&DviPagePlan> {
        self.dvi.as_ref()
    }
}

#[derive(Debug)]
pub struct CommittedEnginePublication {
    effect_count: usize,
    pages: Vec<PublishedEnginePage>,
    pdf: Option<DetachedPdfCompletion>,
}

impl CommittedEnginePublication {
    #[must_use]
    pub const fn effect_count(&self) -> usize {
        self.effect_count
    }
    #[must_use]
    pub fn pages(&self) -> &[PublishedEnginePage] {
        &self.pages
    }
    #[must_use]
    pub const fn pdf(&self) -> Option<&DetachedPdfCompletion> {
        self.pdf.as_ref()
    }
    pub fn into_pdf(self) -> Option<DetachedPdfCompletion> {
        self.pdf
    }
}

fn validate_stream_open_contexts(
    effects: &[EffectRecord],
    contexts: &[Option<String>],
) -> Result<(), EngineCompletionError> {
    if effects.len() != contexts.len() {
        return Err(EngineCompletionError::EffectContextCount);
    }
    for (index, (effect, context)) in effects.iter().zip(contexts).enumerate() {
        if context.is_some() && !matches!(effect, EffectRecord::StreamOpen { .. }) {
            return Err(EngineCompletionError::InvalidEffectContext { ordinal: index + 1 });
        }
    }
    Ok(())
}

fn validate_prepared(
    effects: &[EffectRecord],
    pages: &[DetachedPreparedPage],
    pdf: Option<&DetachedPdfCompletion>,
) -> Result<(), EnginePublicationError> {
    for (index, page) in pages.iter().enumerate() {
        validate_artifact(index, &page.artifact, 0, effects)
            .map_err(publication_validation_error)?;
    }
    if let Some(pdf) = pdf {
        let artifacts: Vec<_> = pages.iter().map(|page| page.artifact.clone()).collect();
        validate_pdf(pdf, artifacts.iter()).map_err(publication_validation_error)?;
    }
    Ok(())
}

fn validate_artifact(
    page: usize,
    artifact: &CommittedArtifact,
    effect_base: u64,
    effects: &[EffectRecord],
) -> Result<(), EngineCompletionError> {
    let expected = ContentHash::for_domain(ContentDomain::Artifact, artifact.bytes());
    if expected != artifact.hash() {
        return Err(EngineCompletionError::InvalidArtifactIdentity { page });
    }
    let model = tex_out::PageArtifact::from_bytes(artifact.bytes()).map_err(|error| {
        EngineCompletionError::InvalidArtifact {
            page,
            message: error.to_string(),
        }
    })?;
    for (occurrence, ordinal) in artifact.open_out_occurrences() {
        let raw = ordinal.index();
        if u64::from(raw) <= effect_base {
            continue;
        }
        let Some(index) = u64::from(raw)
            .checked_sub(effect_base.saturating_add(1))
            .and_then(|value| usize::try_from(value).ok())
        else {
            return Err(EngineCompletionError::InvalidEffectOrdinal { page, ordinal: raw });
        };
        let Some(EffectRecord::StreamOpen { slot, target }) = effects.get(index) else {
            return Err(EngineCompletionError::InvalidEffectOrdinal { page, ordinal: raw });
        };
        let Some(tex_out::PageEffect::OpenOut { stream, path }) = model.effects.get(*occurrence)
        else {
            return Err(EngineCompletionError::InvalidEffectOccurrence { page, ordinal: raw });
        };
        if *stream != slot.raw() || Path::new(path) != target.path() {
            return Err(EngineCompletionError::InvalidEffectOccurrence { page, ordinal: raw });
        }
    }
    Ok(())
}

fn validate_pdf<'a>(
    pdf: &DetachedPdfCompletion,
    artifacts: impl ExactSizeIterator<Item = &'a CommittedArtifact>,
) -> Result<(), EngineCompletionError> {
    if !pdf.pages().is_empty() {
        if pdf.pages().len() != artifacts.len() {
            return Err(EngineCompletionError::PdfPageCount);
        }
        for (index, (page, artifact)) in pdf.pages().iter().zip(artifacts).enumerate() {
            if page.artifact != artifact.hash() || page.artifact_bytes != artifact.bytes() {
                return Err(EngineCompletionError::PdfPageArtifact { page: index });
            }
        }
    }
    let mut objects = BTreeSet::new();
    for page in pdf.pages() {
        for object in [
            page.resources_object,
            page.contents_object,
            page.page_object,
        ] {
            if object == 0 || !objects.insert(object) {
                return Err(EngineCompletionError::PdfObjectIdentity(object));
            }
        }
    }
    for form in pdf.forms() {
        if form.object == 0 || !objects.insert(form.object) {
            return Err(EngineCompletionError::PdfObjectIdentity(form.object));
        }
    }
    let mut form_resources = BTreeSet::new();
    for form in pdf.forms() {
        if form.resource == 0 || !form_resources.insert(form.resource) {
            return Err(EngineCompletionError::PdfResourceIdentity(form.resource));
        }
    }
    let mut font_resources = BTreeMap::new();
    let mut font_identities = BTreeSet::new();
    for font in pdf.fonts() {
        if font.object_number == 0 || font.resource_number == 0 {
            return Err(EngineCompletionError::PdfObjectIdentity(font.object_number));
        }
        if !font_identities.insert(font.recipe.semantic_identity) {
            return Err(EngineCompletionError::PdfResourceIdentity(
                font.resource_number,
            ));
        }
        if let Some(object) = font_resources.get(&font.resource_number) {
            // Multiple realized TeX font recipes may alias the same PDF
            // resource (notably a base font and a scaled copy). They must
            // agree on the ledger's object identity, while their distinct
            // semantic recipes remain available to artifact lookup.
            if *object != font.object_number {
                return Err(EngineCompletionError::PdfResourceIdentity(
                    font.resource_number,
                ));
            }
        } else {
            font_resources.insert(font.resource_number, font.object_number);
            if !objects.insert(font.object_number) {
                return Err(EngineCompletionError::PdfObjectIdentity(font.object_number));
            }
        }
    }
    for image in pdf.images() {
        let object = image.id().raw();
        if object == 0 || !objects.insert(object) {
            return Err(EngineCompletionError::PdfObjectIdentity(object));
        }
    }
    for raw in pdf.raw_objects() {
        if raw.object == 0 || !objects.insert(raw.object) {
            return Err(EngineCompletionError::PdfObjectIdentity(raw.object));
        }
    }
    for annotation in pdf.annotations() {
        insert_pdf_object(&mut objects, annotation.object)?;
    }
    for link in pdf.links() {
        insert_pdf_object(&mut objects, link.object)?;
    }
    for destination in pdf
        .destinations()
        .iter()
        .chain(pdf.structure_destinations())
    {
        insert_pdf_object(&mut objects, destination.object())?;
    }
    for outline in pdf.outlines() {
        for object in [
            outline.action_object,
            outline.item_object,
            outline.title_object,
        ] {
            insert_pdf_object(&mut objects, object)?;
        }
    }
    for thread in pdf.threads() {
        insert_pdf_object(&mut objects, thread.object())?;
        for bead in thread.beads() {
            insert_pdf_object(&mut objects, bead.bead_object())?;
            insert_pdf_object(&mut objects, bead.rectangle_object())?;
        }
    }
    let document = pdf.document();
    for object in [
        document.objects.pages(),
        document.objects.names(),
        document.objects.catalog(),
        document.objects.info(),
    ]
    .into_iter()
    .flatten()
    {
        insert_pdf_object(&mut objects, object)?;
    }
    if let Some(action) = &document.open_action {
        insert_pdf_object(&mut objects, action.id)?;
    }
    if let Some(object) = objects
        .iter()
        .next_back()
        .copied()
        .filter(|object| *object >= pdf.next_object())
    {
        return Err(EngineCompletionError::PdfAllocationCursor {
            object,
            next: pdf.next_object(),
        });
    }
    Ok(())
}

fn insert_pdf_object(
    objects: &mut BTreeSet<u32>,
    object: u32,
) -> Result<(), EngineCompletionError> {
    if object == 0 || !objects.insert(object) {
        return Err(EngineCompletionError::PdfObjectIdentity(object));
    }
    Ok(())
}

fn publication_validation_error(error: EngineCompletionError) -> EnginePublicationError {
    match error {
        EngineCompletionError::InvalidArtifact { page, message } => {
            EnginePublicationError::InvalidArtifact { page, message }
        }
        EngineCompletionError::InvalidArtifactIdentity { page } => {
            EnginePublicationError::InvalidArtifactIdentity { page }
        }
        EngineCompletionError::InvalidEffectOrdinal { page, ordinal } => {
            EnginePublicationError::InvalidEffectOrdinal { page, ordinal }
        }
        EngineCompletionError::InvalidEffectOccurrence { page, ordinal } => {
            EnginePublicationError::InvalidEffectOccurrence { page, ordinal }
        }
        EngineCompletionError::PdfPageCount => EnginePublicationError::PdfPageCount,
        EngineCompletionError::PdfPageArtifact { page } => {
            EnginePublicationError::PdfPageArtifact { page }
        }
        other => EnginePublicationError::InvalidArtifact {
            page: 0,
            message: other.to_string(),
        },
    }
}

fn stale_retarget() -> EnginePublicationError {
    EnginePublicationError::StaleRetarget
}

#[cfg(test)]
mod tests;
