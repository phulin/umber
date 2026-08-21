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
#[derive(Debug)]
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
}

/// Handle-free output of one admitted terminal engine episode.
#[derive(Debug)]
pub struct DetachedEngineCompletion {
    effects: Vec<EffectRecord>,
    pages: Vec<DetachedPreparedPage>,
    pdf: Option<DetachedPdfCompletion>,
}

impl DetachedEngineCompletion {
    pub(crate) fn capture(
        effects: Vec<EffectRecord>,
        artifacts: Vec<CommittedArtifact>,
        artifact_publications: &[tex_state::ArtifactPublicationRecord],
        dvi_pages: Vec<PreparedDviPage>,
        pdf: Option<DetachedPdfCompletion>,
    ) -> Result<Self, EngineCompletionError> {
        if artifacts.len() != artifact_publications.len() {
            return Err(EngineCompletionError::ArtifactPublicationCount);
        }
        if !dvi_pages.is_empty() && dvi_pages.len() != artifacts.len() {
            return Err(EngineCompletionError::DviPageCount);
        }
        for (index, artifact) in artifacts.iter().enumerate() {
            validate_artifact(index, artifact, &effects)?;
        }
        for (prepared, (artifact, publication)) in dvi_pages
            .iter()
            .zip(artifacts.iter().zip(artifact_publications))
        {
            if prepared.publication != *publication {
                return Err(EngineCompletionError::DviPublicationMismatch);
            }
            if prepared.hash != artifact.hash() {
                return Err(EngineCompletionError::DviArtifactMismatch);
            }
        }
        if let Some(pdf) = &pdf {
            validate_pdf(pdf, &artifacts)?;
        }
        let dvi = if dvi_pages.is_empty() {
            vec![None; artifacts.len()]
        } else {
            dvi_pages.into_iter().map(|page| Some(page.plan)).collect()
        };
        let pages = artifacts
            .into_iter()
            .zip(dvi)
            .map(|(artifact, dvi)| DetachedPreparedPage { artifact, dvi })
            .collect();
        Ok(Self {
            effects,
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

    /// Performs all structural validation before a destination World can be
    /// mutated and consumes the detached value into a non-Clone transaction.
    pub fn into_publication(self) -> Result<PreparedEnginePublication, EnginePublicationError> {
        validate_prepared(&self.effects, &self.pages, self.pdf.as_ref())?;
        Ok(PreparedEnginePublication {
            effects: self.effects,
            cursor: 0,
            pages: self.pages,
            pdf: self.pdf,
            retry_attempt: 0,
        })
    }
}

#[derive(Debug)]
pub enum EngineCompletionError {
    Admission(tex_state::UniverseError),
    Pdf(PdfCompletionError),
    MaterializedEffectBase,
    ArtifactPublicationCount,
    DviPageCount,
    DviPublicationMismatch,
    DviArtifactMismatch,
    InvalidArtifact { page: usize, message: String },
    InvalidArtifactIdentity { page: usize },
    InvalidEffectOrdinal { page: usize, ordinal: u32 },
    InvalidEffectOccurrence { page: usize, ordinal: u32 },
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
        match world.publish_detached_effect_records(&self.effects[self.cursor..]) {
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

fn validate_prepared(
    effects: &[EffectRecord],
    pages: &[DetachedPreparedPage],
    pdf: Option<&DetachedPdfCompletion>,
) -> Result<(), EnginePublicationError> {
    for (index, page) in pages.iter().enumerate() {
        validate_artifact(index, &page.artifact, effects).map_err(publication_validation_error)?;
    }
    if let Some(pdf) = pdf {
        let artifacts: Vec<_> = pages.iter().map(|page| page.artifact.clone()).collect();
        validate_pdf(pdf, &artifacts).map_err(publication_validation_error)?;
    }
    Ok(())
}

fn validate_artifact(
    page: usize,
    artifact: &CommittedArtifact,
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
        let Some(index) = raw
            .checked_sub(1)
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

fn validate_pdf(
    pdf: &DetachedPdfCompletion,
    artifacts: &[CommittedArtifact],
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
    for font in pdf.fonts() {
        if font.object_number == 0 || font.resource_number == 0 {
            return Err(EngineCompletionError::PdfObjectIdentity(font.object_number));
        }
        if let Some((object, recipe)) = font_resources.get(&font.resource_number) {
            if *object != font.object_number || *recipe != &font.recipe {
                return Err(EngineCompletionError::PdfResourceIdentity(
                    font.resource_number,
                ));
            }
        } else {
            font_resources.insert(font.resource_number, (font.object_number, &font.recipe));
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
