use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::PathBuf;

use bib_engine::{
    BibJob, BibOptions, BibSessionOptions, BibliographyAttempt, BibliographyBackend,
    BibliographyDetector, BibliographyDetectorOptions, BibliographyFailure, BibliographyJob,
    BibliographyMode, BibliographyResult, BibliographySession, ClassicBibJob, ClassicBibOptions,
};
use tex_fonts::{FontRequestKey, PdfPkFontRequest, ResolvedFont};
use tex_state::ContentHash;
use umber_vfs::{
    BuildId, BuildPlan, FileProvisioner, FileRequestBatch, ProducerId, ResolvedFile, VirtualPath,
};

use crate::fixed_point::{FixedPointCandidate, FixedPointCoordinator, FixedPointFailure};
use crate::{
    CompileAttemptResult, CompileError, MemoryOutputFile, MemoryRunOutput, NeedResources,
    ResolvedPkFont, ResourceRequest, ResourceResponse, SessionOptions, SourcePatch,
    VirtualCompileSession,
};

const PROJECT_PRODUCER: ProducerId = ProducerId::new(3);
type GeneratedSignature = Vec<(VirtualPath, ContentHash)>;
type ProjectConvergenceKey = (Option<BibliographyBackend>, GeneratedSignature);

mod support;
use support::{
    CandidateStop, accepted_generated, add_candidate_inputs, candidate_snapshot, file_needs,
    generated_signature, merge_tex_files, project_vfs_limits,
};

pub type LatexProjectLimits = crate::FixedPointLimits;

/// Backend-neutral bibliography policy for a project session.
#[derive(Clone, Debug)]
pub struct BibliographyProjectOptions {
    pub mode: BibliographyMode,
    pub biblatex: BibOptions,
    pub bib_session: BibSessionOptions,
    pub classic: ClassicBibOptions,
    pub detector: BibliographyDetectorOptions,
}

impl BibliographyProjectOptions {
    #[must_use]
    pub fn biblatex(control_path: VirtualPath, options: BibOptions) -> Self {
        Self {
            mode: BibliographyMode::Biblatex { control_path },
            biblatex: options,
            bib_session: BibSessionOptions::default(),
            classic: ClassicBibOptions::default(),
            detector: BibliographyDetectorOptions::default(),
        }
    }

    #[must_use]
    pub fn classic(aux_path: VirtualPath) -> Self {
        Self {
            mode: BibliographyMode::Classic { aux_path },
            biblatex: BibOptions::default(),
            bib_session: BibSessionOptions::default(),
            classic: ClassicBibOptions::default(),
            detector: BibliographyDetectorOptions::default(),
        }
    }

    #[must_use]
    pub fn auto(job_path: VirtualPath) -> Self {
        Self {
            mode: BibliographyMode::Auto { job_path },
            biblatex: BibOptions::default(),
            bib_session: BibSessionOptions::default(),
            classic: ClassicBibOptions::default(),
            detector: BibliographyDetectorOptions::default(),
        }
    }
}

/// Backend-neutral project configuration.
#[derive(Clone, Debug)]
pub struct LatexProjectOptions {
    pub tex: SessionOptions,
    pub bibliography: BibliographyProjectOptions,
    pub limits: LatexProjectLimits,
}

/// A stable convergence identity for a project generation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectConvergenceFingerprint {
    pub backend: Option<BibliographyBackend>,
    pub generated: Vec<(VirtualPath, ContentHash)>,
}

/// Accepted output from the backend-neutral project session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LatexProjectOutput {
    pub revision: tex_incr::RevisionId,
    pub content_hash: ContentHash,
    pub passes: u32,
    pub tex: MemoryRunOutput,
    pub bibliography: Option<BibliographyResult>,
    pub generated_files: Vec<MemoryOutputFile>,
    pub fingerprint: ProjectConvergenceFingerprint,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LatexProjectAttempt {
    NeedResources(NeedResources),
    Complete(Box<LatexProjectOutput>),
    Error(LatexProjectError),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LatexProjectError {
    Compile(CompileError),
    Bibliography(BibliographyFailure),
    BibliographyFatal { backend: BibliographyBackend },
    InvalidLimit { name: &'static str, value: u32 },
    PassLimit { limit: u32 },
    Oscillation { first_pass: u32, repeated_pass: u32 },
    Transaction(String),
    InvalidPatch(String),
    UnexpectedResource(String),
    ConflictingResource(String),
}

impl fmt::Display for LatexProjectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Compile(error) => error.fmt(formatter),
            Self::Bibliography(error) => write!(formatter, "bibliography failed: {error:?}"),
            Self::BibliographyFatal { backend } => {
                write!(
                    formatter,
                    "{backend:?} bibliography execution ended fatally"
                )
            }
            Self::InvalidLimit { name, value } => {
                write!(formatter, "invalid project {name} limit {value}")
            }
            Self::PassLimit { limit } => write!(formatter, "project pass limit {limit} reached"),
            Self::Oscillation {
                first_pass,
                repeated_pass,
            } => write!(
                formatter,
                "project output oscillated between passes {first_pass} and {repeated_pass}"
            ),
            Self::Transaction(message) | Self::InvalidPatch(message) => {
                formatter.write_str(message)
            }
            Self::UnexpectedResource(name) => {
                write!(formatter, "resource response {name} was not requested")
            }
            Self::ConflictingResource(name) => {
                write!(
                    formatter,
                    "resource response {name} conflicts with retained content"
                )
            }
        }
    }
}

impl std::error::Error for LatexProjectError {}

fn project_fixed_point_error(error: FixedPointFailure) -> LatexProjectError {
    match error {
        FixedPointFailure::InvalidLimit { name, value } => {
            LatexProjectError::InvalidLimit { name, value }
        }
        FixedPointFailure::AttemptLimit { limit } => {
            LatexProjectError::Compile(CompileError::AttemptLimit { limit })
        }
        FixedPointFailure::NoProgress => LatexProjectError::Compile(CompileError::NoProgress),
        FixedPointFailure::PassLimit { limit } => LatexProjectError::PassLimit { limit },
        FixedPointFailure::Oscillation {
            first_pass,
            repeated_pass,
        } => LatexProjectError::Oscillation {
            first_pass,
            repeated_pass,
        },
    }
}

fn project_fixed_point_stop(error: FixedPointFailure) -> CandidateStop {
    CandidateStop::Failed(project_fixed_point_error(error))
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum ProjectRequestKey {
    File(umber_vfs::FileRequestKey),
    Font(FontRequestKey),
    PkFont(PdfPkFontRequest),
}

/// Transactional TeX--bibliography--TeX project session with explicit or
/// automatic backend selection.
pub struct LatexProjectSession {
    options: LatexProjectOptions,
    bibliography_enabled: bool,
    files: FileProvisioner,
    detector: BibliographyDetector,
    bibliography: Option<BibliographySession>,
    bibliography_backend: Option<BibliographyBackend>,
    published_bibliography_paths: BTreeSet<VirtualPath>,
    file_responses: BTreeMap<umber_vfs::FileRequestKey, ResolvedFile>,
    font_responses: BTreeMap<FontRequestKey, ResolvedFont>,
    unavailable_fonts: BTreeSet<FontRequestKey>,
    pk_font_responses: BTreeMap<PdfPkFontRequest, ResolvedPkFont>,
    unavailable_pk_fonts: BTreeSet<PdfPkFontRequest>,
    awaiting: BTreeSet<ProjectRequestKey>,
    fixed_point: FixedPointCoordinator,
    accepted_revision: Option<tex_incr::RevisionId>,
    accepted_root: Option<Vec<u8>>,
    pending_root: Option<(tex_incr::RevisionId, Vec<u8>)>,
    candidate: Option<ProjectCandidate>,
    accepted_tex: Option<Box<VirtualCompileSession>>,
    accepted_output: Option<LatexProjectOutput>,
    accepted_observations: Option<crate::AcceptedInputObservationLedger>,
}

struct ProjectCandidate {
    revision: tex_incr::RevisionId,
    root: Vec<u8>,
    generated: BTreeMap<VirtualPath, Vec<u8>>,
    fixed_point: FixedPointCandidate<ProjectConvergenceKey>,
    tex: Option<Box<VirtualCompileSession>>,
    tex_awaiting: bool,
    observations: Vec<crate::AcceptedInputObservation>,
    tex_observed: bool,
    detection_observed: bool,
}

impl LatexProjectSession {
    pub fn new(options: LatexProjectOptions) -> Result<Self, LatexProjectError> {
        Self::new_inner(options, true)
    }

    pub(crate) fn new_tex_only(
        tex: SessionOptions,
        limits: crate::FixedPointLimits,
    ) -> Result<Self, LatexProjectError> {
        let disabled_path = VirtualPath::user("/job/__umber_tex_only_disabled")
            .expect("internal TeX-only detector path is valid");
        Self::new_inner(
            LatexProjectOptions {
                tex,
                bibliography: BibliographyProjectOptions::auto(disabled_path),
                limits,
            },
            false,
        )
    }

    fn new_inner(
        options: LatexProjectOptions,
        bibliography_enabled: bool,
    ) -> Result<Self, LatexProjectError> {
        let fixed_point =
            FixedPointCoordinator::new(options.limits).map_err(project_fixed_point_error)?;
        Ok(Self {
            files: FileProvisioner::new(project_vfs_limits(&options.tex))
                .map_err(|error| LatexProjectError::Transaction(error.to_string()))?,
            detector: BibliographyDetector::new(options.bibliography.detector),
            options,
            bibliography_enabled,
            bibliography: None,
            bibliography_backend: None,
            published_bibliography_paths: BTreeSet::new(),
            file_responses: BTreeMap::new(),
            font_responses: BTreeMap::new(),
            unavailable_fonts: BTreeSet::new(),
            pk_font_responses: BTreeMap::new(),
            unavailable_pk_fonts: BTreeSet::new(),
            awaiting: BTreeSet::new(),
            fixed_point,
            accepted_revision: None,
            accepted_root: None,
            pending_root: None,
            candidate: None,
            accepted_tex: None,
            accepted_output: None,
            accepted_observations: None,
        })
    }

    /// Switches the bibliography policy for the next project generation.
    /// Previously accepted artifacts remain visible until that generation is
    /// accepted, but its bibliography artifacts are never reused by another
    /// backend.
    pub fn set_bibliography(
        &mut self,
        bibliography: BibliographyProjectOptions,
    ) -> Result<(), LatexProjectError> {
        if self.pending_root.is_some() {
            return Err(LatexProjectError::InvalidPatch(
                "cannot switch bibliography while a project patch is pending".into(),
            ));
        }
        self.options.bibliography = bibliography;
        self.detector = BibliographyDetector::new(self.options.bibliography.detector);
        self.bibliography = None;
        self.bibliography_backend = None;
        self.candidate = None;
        self.awaiting.clear();
        Ok(())
    }

    pub fn add_user_file(&mut self, path: &str, bytes: Vec<u8>) -> Result<(), LatexProjectError> {
        if self.accepted_output.is_some() || self.candidate.is_some() {
            return Err(LatexProjectError::Compile(
                CompileError::SessionAlreadyStarted,
            ));
        }
        let path = VirtualPath::user(path).map_err(|error| {
            LatexProjectError::Compile(CompileError::InvalidVirtualPath {
                path: path.to_owned(),
                message: error.to_string(),
            })
        })?;
        self.files
            .register_user(path, bytes)
            .map(|_| ())
            .map_err(|error| LatexProjectError::Transaction(error.to_string()))
    }

    pub fn apply_patch(&mut self, patch: SourcePatch) -> Result<(), LatexProjectError> {
        if self.pending_root.is_some() {
            return Err(LatexProjectError::Compile(
                CompileError::PatchAlreadyPending,
            ));
        }
        let revision = self.accepted_revision.ok_or_else(|| {
            LatexProjectError::InvalidPatch("the initial project revision is not accepted".into())
        })?;
        let root = self
            .accepted_root
            .as_ref()
            .expect("accepted revision owns root");
        if patch.base_revision != revision || patch.next_revision.raw() <= revision.raw() {
            return Err(LatexProjectError::InvalidPatch(
                "project patch revision is stale or non-monotonic".into(),
            ));
        }
        if patch.expected_hash != ContentHash::from_bytes(root) {
            return Err(LatexProjectError::InvalidPatch(
                "project patch content hash does not match".into(),
            ));
        }
        let source = std::str::from_utf8(root)
            .map_err(|_| LatexProjectError::InvalidPatch("project root is not UTF-8".into()))?;
        if patch.range.start > patch.range.end
            || patch.range.end > source.len()
            || !source.is_char_boundary(patch.range.start)
            || !source.is_char_boundary(patch.range.end)
        {
            return Err(LatexProjectError::InvalidPatch(
                "project patch range is invalid".into(),
            ));
        }
        let mut next = source.to_owned();
        next.replace_range(patch.range, &patch.replacement);
        self.pending_root = Some((patch.next_revision, next.into_bytes()));
        self.candidate = None;
        self.awaiting.clear();
        Ok(())
    }

    pub fn provide_resources(
        &mut self,
        responses: Vec<ResourceResponse>,
    ) -> Result<(), LatexProjectError> {
        let tex_responses = responses.clone();
        let mut files = self.files.clone();
        let mut file_responses = self.file_responses.clone();
        let mut font_responses = self.font_responses.clone();
        let mut unavailable_fonts = self.unavailable_fonts.clone();
        let mut pk_font_responses = self.pk_font_responses.clone();
        let mut unavailable_pk_fonts = self.unavailable_pk_fonts.clone();
        for response in responses {
            match response {
                ResourceResponse::File(file) => {
                    let key = ProjectRequestKey::File(file.request.clone());
                    if !self.awaiting.contains(&key) {
                        return Err(LatexProjectError::UnexpectedResource(
                            file.request.name().to_owned(),
                        ));
                    }
                    files
                        .provision(file.clone())
                        .map_err(|e| LatexProjectError::Transaction(e.to_string()))?;
                    file_responses.insert(file.request.clone(), file);
                }
                ResourceResponse::FileUnavailable(request) => {
                    let key = ProjectRequestKey::File(request.clone());
                    if !self.awaiting.contains(&key) {
                        return Err(LatexProjectError::UnexpectedResource(
                            request.name().to_owned(),
                        ));
                    }
                    files
                        .provision_unavailable(request)
                        .map_err(|e| LatexProjectError::Transaction(e.to_string()))?;
                }
                ResourceResponse::Font(font) => {
                    let key = ProjectRequestKey::Font(font.request.clone());
                    if !self.awaiting.contains(&key) {
                        return Err(LatexProjectError::UnexpectedResource(
                            font.request.logical_name().to_owned(),
                        ));
                    }
                    if unavailable_fonts.contains(&font.request)
                        || font_responses
                            .get(&font.request)
                            .is_some_and(|old| old != &font)
                    {
                        return Err(LatexProjectError::ConflictingResource(
                            font.request.logical_name().to_owned(),
                        ));
                    }
                    font_responses.insert(font.request.clone(), font);
                }
                ResourceResponse::FontUnavailable(request) => {
                    let key = ProjectRequestKey::Font(request.clone());
                    if !self.awaiting.contains(&key) {
                        return Err(LatexProjectError::UnexpectedResource(
                            request.logical_name().to_owned(),
                        ));
                    }
                    if font_responses.contains_key(&request) {
                        return Err(LatexProjectError::ConflictingResource(
                            request.logical_name().to_owned(),
                        ));
                    }
                    unavailable_fonts.insert(request);
                }
                ResourceResponse::PkFont(font) => {
                    let key = ProjectRequestKey::PkFont(font.request.clone());
                    if !self.awaiting.contains(&key) {
                        return Err(LatexProjectError::UnexpectedResource(
                            String::from_utf8_lossy(&font.request.logical_name()).into_owned(),
                        ));
                    }
                    if unavailable_pk_fonts.contains(&font.request)
                        || pk_font_responses
                            .get(&font.request)
                            .is_some_and(|old| old != &font)
                    {
                        return Err(LatexProjectError::ConflictingResource(
                            String::from_utf8_lossy(&font.request.logical_name()).into_owned(),
                        ));
                    }
                    pk_font_responses.insert(font.request.clone(), font);
                }
                ResourceResponse::PkFontUnavailable(request) => {
                    let key = ProjectRequestKey::PkFont(request.clone());
                    if !self.awaiting.contains(&key) {
                        return Err(LatexProjectError::UnexpectedResource(
                            String::from_utf8_lossy(&request.logical_name()).into_owned(),
                        ));
                    }
                    if pk_font_responses.contains_key(&request) {
                        return Err(LatexProjectError::ConflictingResource(
                            String::from_utf8_lossy(&request.logical_name()).into_owned(),
                        ));
                    }
                    unavailable_pk_fonts.insert(request);
                }
            }
        }
        self.files = files;
        self.file_responses = file_responses;
        self.font_responses = font_responses;
        self.unavailable_fonts = unavailable_fonts;
        self.pk_font_responses = pk_font_responses;
        self.unavailable_pk_fonts = unavailable_pk_fonts;
        if let Some(candidate) = self.candidate.as_mut()
            && candidate.tex_awaiting
        {
            candidate
                .tex
                .as_mut()
                .expect("a TeX wait retains its session")
                .provide_resources(tex_responses)
                .map_err(LatexProjectError::Compile)?;
            candidate.tex_awaiting = false;
        }
        Ok(())
    }

    /// Cancels an unaccepted edited project generation and releases its
    /// suspended TeX pass while preserving the accepted project.
    pub fn cancel_pending_patch(&mut self) -> bool {
        let cancelled = self.pending_root.take().is_some();
        if cancelled {
            self.candidate = None;
            self.awaiting.clear();
            self.fixed_point.reset_attempts();
        }
        cancelled
    }

    pub fn compile_attempt(&mut self) -> LatexProjectAttempt {
        if self.pending_root.is_none()
            && let Some(output) = &self.accepted_output
        {
            return LatexProjectAttempt::Complete(Box::new(output.clone()));
        }
        let made_progress = self.awaiting.is_empty()
            || self.awaiting.iter().any(|key| match key {
                ProjectRequestKey::File(key) => {
                    self.file_responses.contains_key(key)
                        || self.files.unavailable_keys().any(|missing| missing == key)
                }
                ProjectRequestKey::Font(key) => {
                    self.font_responses.contains_key(key) || self.unavailable_fonts.contains(key)
                }
                ProjectRequestKey::PkFont(key) => {
                    self.pk_font_responses.contains_key(key)
                        || self.unavailable_pk_fonts.contains(key)
                }
            });
        if let Err(error) = self.fixed_point.start_attempt(made_progress) {
            self.reject_pending();
            return LatexProjectAttempt::Error(project_fixed_point_error(error));
        }
        match self.run_candidate() {
            Ok(output) => LatexProjectAttempt::Complete(Box::new(output)),
            Err(CandidateStop::Need(needs)) => {
                self.remember_needs(&needs);
                LatexProjectAttempt::NeedResources(needs)
            }
            Err(CandidateStop::Failed(error)) => {
                self.reject_pending();
                LatexProjectAttempt::Error(error)
            }
        }
    }

    #[must_use]
    pub const fn revision(&self) -> Option<tex_incr::RevisionId> {
        self.accepted_revision
    }
    #[must_use]
    pub fn content_hash(&self) -> Option<ContentHash> {
        self.accepted_root
            .as_ref()
            .map(|root| ContentHash::from_bytes(root))
    }
    #[must_use]
    pub fn accepted_output(&self) -> Option<&LatexProjectOutput> {
        self.accepted_output.as_ref()
    }
    #[must_use]
    pub const fn accepted_input_observations(
        &self,
    ) -> Option<&crate::AcceptedInputObservationLedger> {
        self.accepted_observations.as_ref()
    }

    fn run_candidate(&mut self) -> Result<LatexProjectOutput, CandidateStop> {
        let mut candidate = if let Some(candidate) = self.candidate.take() {
            candidate
        } else {
            let (revision, root) = self.candidate_root()?;
            let mut generated = accepted_generated(&self.files)?;
            for path in &self.published_bibliography_paths {
                generated.remove(path);
            }
            let fixed_point = self
                .fixed_point
                .begin((self.bibliography_backend, generated_signature(&generated)));
            ProjectCandidate {
                revision,
                root,
                generated,
                fixed_point,
                tex: None,
                tex_awaiting: false,
                observations: Vec::new(),
                tex_observed: false,
                detection_observed: false,
            }
        };
        loop {
            let pass = candidate.fixed_point.pass();
            let mut bibliography = None;
            let before = (
                self.bibliography_backend,
                generated_signature(&candidate.generated),
            );
            let mut tex_session = match candidate.tex.take() {
                Some(session) => session,
                None => {
                    self.start_tex_pass(candidate.revision, &candidate.root, &candidate.generated)?
                }
            };
            let tex_output = match self.advance_tex_pass(&mut tex_session)? {
                Ok(output) => output,
                Err(needs) => {
                    candidate.tex = Some(tex_session);
                    candidate.tex_awaiting = true;
                    self.candidate = Some(candidate);
                    return Err(CandidateStop::Need(needs));
                }
            };
            merge_tex_files(&mut candidate.generated, &tex_output.files)?;
            let snapshot = candidate_snapshot(
                &self.files,
                &self.options.tex.main_path,
                &candidate.root,
                &candidate.generated,
            )?;
            if !candidate.tex_observed {
                let dependencies = tex_session.accepted_input_dependency_values();
                let observations = crate::input_observation::tex_observations(
                    dependencies.into_iter(),
                    &snapshot,
                    candidate.revision,
                    Some(pass),
                );
                if candidate
                    .observations
                    .len()
                    .saturating_add(observations.len())
                    > crate::MAX_ACCEPTED_INPUT_OBSERVATIONS
                {
                    return Err(CandidateStop::Failed(LatexProjectError::Transaction(
                        format!(
                            "accepted input observation limit {} exceeded",
                            crate::MAX_ACCEPTED_INPUT_OBSERVATIONS
                        ),
                    )));
                }
                candidate.observations.extend(observations);
                candidate.tex_observed = true;
            }
            if !self.bibliography_enabled {
                let after = (None, generated_signature(&candidate.generated));
                if after == before {
                    return self.accept_candidate(
                        candidate.revision,
                        candidate.root,
                        pass,
                        tex_output,
                        None,
                        candidate.generated,
                        tex_session,
                        candidate.observations,
                    );
                }
                candidate
                    .fixed_point
                    .observe_changed(after, self.options.limits)
                    .map_err(project_fixed_point_stop)?;
                candidate.tex_observed = false;
                continue;
            }
            if !candidate.detection_observed {
                let detection_observations =
                    crate::input_observation::bibliography_detection_observations(
                        &self.options.bibliography.mode,
                        &snapshot,
                        candidate.revision,
                        pass,
                    );
                if candidate
                    .observations
                    .len()
                    .saturating_add(detection_observations.len())
                    > crate::MAX_ACCEPTED_INPUT_OBSERVATIONS
                {
                    return Err(CandidateStop::Failed(LatexProjectError::Transaction(
                        format!(
                            "accepted input observation limit {} exceeded",
                            crate::MAX_ACCEPTED_INPUT_OBSERVATIONS
                        ),
                    )));
                }
                candidate.observations.extend(detection_observations);
                candidate.detection_observed = true;
            }
            match self
                .detector
                .detect(&self.options.bibliography.mode, &snapshot)
            {
                bib_engine::BibliographyDetection::NoBibliography => {
                    self.bibliography = None;
                    self.bibliography_backend = None;
                }
                bib_engine::BibliographyDetection::NeedResources(batch) => {
                    candidate.tex = Some(tex_session);
                    candidate.tex_awaiting = false;
                    self.candidate = Some(candidate);
                    return Err(CandidateStop::Need(file_needs(batch)));
                }
                bib_engine::BibliographyDetection::Failed(error) => {
                    return Err(CandidateStop::Failed(LatexProjectError::Bibliography(
                        error,
                    )));
                }
                bib_engine::BibliographyDetection::Selected(selected) => {
                    let job = self.selected_job(selected);
                    self.ensure_backend(job.backend())?;
                    let attempt = self
                        .bibliography
                        .as_mut()
                        .expect("selected session")
                        .process(&job, &snapshot);
                    match attempt {
                        BibliographyAttempt::NeedResources(batch) => {
                            candidate.tex = Some(tex_session);
                            candidate.tex_awaiting = false;
                            self.candidate = Some(candidate);
                            return Err(CandidateStop::Need(file_needs(batch)));
                        }
                        BibliographyAttempt::Failed(error) => {
                            return Err(CandidateStop::Failed(LatexProjectError::Bibliography(
                                error,
                            )));
                        }
                        BibliographyAttempt::Finished(result) if !result.is_publishable() => {
                            return Err(CandidateStop::Failed(
                                LatexProjectError::BibliographyFatal {
                                    backend: result.backend(),
                                },
                            ));
                        }
                        BibliographyAttempt::Finished(result) => {
                            let owner = match result.backend() {
                                BibliographyBackend::Biblatex => {
                                    crate::InputObservationOwner::Biblatex
                                }
                                BibliographyBackend::Classic => {
                                    crate::InputObservationOwner::ClassicBibtex
                                }
                            };
                            let inputs = self
                                .bibliography
                                .as_ref()
                                .expect("finished bibliography retains its session")
                                .accepted_inputs();
                            let bibliography_observations =
                                crate::input_observation::bibliography_observations(
                                    inputs,
                                    &snapshot,
                                    candidate.revision,
                                    pass,
                                    owner,
                                );
                            if candidate
                                .observations
                                .len()
                                .saturating_add(bibliography_observations.len())
                                > crate::MAX_ACCEPTED_INPUT_OBSERVATIONS
                            {
                                return Err(CandidateStop::Failed(LatexProjectError::Transaction(
                                    format!(
                                        "accepted input observation limit {} exceeded",
                                        crate::MAX_ACCEPTED_INPUT_OBSERVATIONS
                                    ),
                                )));
                            }
                            candidate.observations.extend(bibliography_observations);
                            self.published_bibliography_paths =
                                result.files().map(|file| file.path().clone()).collect();
                            for path in &self.published_bibliography_paths {
                                candidate.generated.remove(path);
                            }
                            for file in result.files() {
                                candidate
                                    .generated
                                    .insert(file.path().clone(), file.bytes().to_vec());
                            }
                            bibliography = Some(result);
                        }
                    }
                }
            }
            let after = (
                self.bibliography_backend,
                generated_signature(&candidate.generated),
            );
            if after == before {
                return self.accept_candidate(
                    candidate.revision,
                    candidate.root,
                    pass,
                    tex_output,
                    bibliography,
                    candidate.generated,
                    tex_session,
                    candidate.observations,
                );
            }
            candidate
                .fixed_point
                .observe_changed(after, self.options.limits)
                .map_err(project_fixed_point_stop)?;
            candidate.tex_observed = false;
            candidate.detection_observed = false;
        }
    }

    fn selected_job(&self, selected: BibliographyJob) -> BibliographyJob {
        match selected {
            BibliographyJob::Biblatex(job) => BibliographyJob::Biblatex(BibJob::new(
                job.control_path().clone(),
                self.options.bibliography.biblatex.clone(),
            )),
            BibliographyJob::Classic(job) => BibliographyJob::Classic(ClassicBibJob::new(
                job.aux_path().clone(),
                self.options.bibliography.classic.clone(),
            )),
        }
    }

    fn ensure_backend(&mut self, backend: BibliographyBackend) -> Result<(), CandidateStop> {
        if self.bibliography_backend == Some(backend) {
            return Ok(());
        }
        self.bibliography = Some(match backend {
            BibliographyBackend::Biblatex => {
                BibliographySession::biblatex(self.options.bibliography.bib_session)
                    .map_err(|e| LatexProjectError::Transaction(e.to_string()))?
            }
            BibliographyBackend::Classic => BibliographySession::classic(),
        });
        self.bibliography_backend = Some(backend);
        Ok(())
    }

    fn start_tex_pass(
        &self,
        revision: tex_incr::RevisionId,
        root: &[u8],
        generated: &BTreeMap<VirtualPath, Vec<u8>>,
    ) -> Result<Box<VirtualCompileSession>, CandidateStop> {
        let mut session = Box::new(
            VirtualCompileSession::new_at_revision(self.options.tex.clone(), revision)
                .map_err(LatexProjectError::Compile)?,
        );
        add_candidate_inputs(
            &mut session,
            &self.files,
            &self.options.tex.main_path,
            root,
            generated,
        )?;
        for response in self.file_responses.values() {
            session
                .restore_cached_file(
                    response.request.clone(),
                    &response.virtual_path,
                    response.bytes.clone(),
                )
                .map_err(LatexProjectError::Compile)?;
        }
        Ok(session)
    }

    fn advance_tex_pass(
        &self,
        session: &mut VirtualCompileSession,
    ) -> Result<Result<MemoryRunOutput, NeedResources>, CandidateStop> {
        loop {
            match session.compile_attempt() {
                CompileAttemptResult::Complete(output) => return Ok(Ok(output)),
                CompileAttemptResult::Error(error) => {
                    return Err(CandidateStop::Failed(LatexProjectError::Compile(error)));
                }
                CompileAttemptResult::NeedResources(needs) => {
                    let mut supplied = Vec::new();
                    let mut missing = Vec::new();
                    let mut missing_probes = Vec::new();
                    for request in needs.required {
                        match &request {
                            ResourceRequest::File(file) => {
                                if let Some(response) = self.file_responses.get(file.key()) {
                                    supplied.push(ResourceResponse::File(response.clone()));
                                } else if self.files.unavailable_keys().any(|key| key == file.key())
                                {
                                    supplied.push(ResourceResponse::FileUnavailable(
                                        file.key().clone(),
                                    ));
                                } else {
                                    missing.push(request);
                                }
                            }
                            ResourceRequest::Font(font) => {
                                if let Some(response) = self.font_responses.get(&font.key) {
                                    supplied.push(ResourceResponse::Font(response.clone()));
                                } else if self.unavailable_fonts.contains(&font.key) {
                                    supplied
                                        .push(ResourceResponse::FontUnavailable(font.key.clone()));
                                } else {
                                    missing.push(request);
                                }
                            }
                            ResourceRequest::PkFont(pk) => {
                                if let Some(response) = self.pk_font_responses.get(pk) {
                                    supplied.push(ResourceResponse::PkFont(response.clone()));
                                } else if self.unavailable_pk_fonts.contains(pk) {
                                    supplied.push(ResourceResponse::PkFontUnavailable(pk.clone()));
                                } else {
                                    missing.push(request);
                                }
                            }
                        }
                    }
                    for request in needs.probes {
                        let ResourceRequest::File(file) = &request else {
                            missing.push(request);
                            continue;
                        };
                        if let Some(response) = self.file_responses.get(file.key()) {
                            supplied.push(ResourceResponse::File(response.clone()));
                        } else if self.files.unavailable_keys().any(|key| key == file.key()) {
                            supplied.push(ResourceResponse::FileUnavailable(file.key().clone()));
                        } else {
                            missing_probes.push(request);
                        }
                    }
                    if !missing.is_empty() || !missing_probes.is_empty() {
                        return Ok(Err(NeedResources {
                            required: missing,
                            probes: missing_probes,
                            prefetch_hints: needs.prefetch_hints,
                        }));
                    }
                    session
                        .provide_resources(supplied)
                        .map_err(LatexProjectError::Compile)?;
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn accept_candidate(
        &mut self,
        revision: tex_incr::RevisionId,
        root: Vec<u8>,
        passes: u32,
        tex: MemoryRunOutput,
        bibliography: Option<BibliographyResult>,
        generated: BTreeMap<VirtualPath, Vec<u8>>,
        tex_session: Box<VirtualCompileSession>,
        observations: Vec<crate::AcceptedInputObservation>,
    ) -> Result<LatexProjectOutput, CandidateStop> {
        let mut pending = self.files.clone();
        pending
            .register_user(
                VirtualPath::user(&self.options.tex.main_path)
                    .map_err(|e| LatexProjectError::Transaction(e.to_string()))?,
                root.clone(),
            )
            .map_err(|e| LatexProjectError::Transaction(e.to_string()))?;
        let mut build = pending.begin_build(BuildPlan::new(BuildId::new(u64::from(
            self.fixed_point.attempts(),
        ))));
        let mut stage = build
            .begin_stage(PROJECT_PRODUCER)
            .map_err(|e| LatexProjectError::Transaction(e.to_string()))?;
        for (path, bytes) in &generated {
            stage
                .write(path.clone(), bytes.clone())
                .map_err(|e| LatexProjectError::Transaction(e.to_string()))?;
        }
        stage
            .finish()
            .map_err(|e| LatexProjectError::Transaction(e.to_string()))?;
        build
            .accept()
            .map_err(|e| LatexProjectError::Transaction(e.to_string()))?;
        let fingerprint = ProjectConvergenceFingerprint {
            backend: self.bibliography_backend,
            generated: generated_signature(&generated),
        };
        let output = LatexProjectOutput {
            revision,
            content_hash: ContentHash::from_bytes(&root),
            passes,
            tex,
            bibliography,
            generated_files: generated
                .into_iter()
                .map(|(path, bytes)| MemoryOutputFile {
                    path: PathBuf::from(path.as_str()),
                    bytes,
                })
                .collect(),
            fingerprint,
        };
        self.files = pending;
        self.accepted_revision = Some(revision);
        self.accepted_root = Some(root);
        self.pending_root = None;
        self.candidate = None;
        self.awaiting.clear();
        self.accepted_tex = Some(tex_session);
        self.accepted_output = Some(output.clone());
        self.accepted_observations = Some(crate::AcceptedInputObservationLedger::new(
            revision,
            observations,
        ));
        Ok(output)
    }

    fn candidate_root(&self) -> Result<(tex_incr::RevisionId, Vec<u8>), CandidateStop> {
        if let Some((revision, root)) = &self.pending_root {
            return Ok((*revision, root.clone()));
        }
        let main = VirtualPath::user(&self.options.tex.main_path)
            .map_err(|e| LatexProjectError::Transaction(e.to_string()))?;
        let snapshot = self.files.snapshot();
        let file = snapshot
            .get(&main)
            .map_err(|e| LatexProjectError::Transaction(e.to_string()))?
            .ok_or_else(|| {
                LatexProjectError::Compile(CompileError::MissingMainFile(main.to_string()))
            })?;
        Ok((tex_incr::RevisionId::new(1), file.bytes().to_vec()))
    }

    fn remember_needs(&mut self, needs: &NeedResources) {
        self.awaiting = needs
            .required
            .iter()
            .chain(&needs.probes)
            .map(|request| match request {
                ResourceRequest::File(file) => ProjectRequestKey::File(file.key().clone()),
                ResourceRequest::Font(font) => ProjectRequestKey::Font(font.key.clone()),
                ResourceRequest::PkFont(pk) => ProjectRequestKey::PkFont(pk.clone()),
            })
            .collect();
        self.files.expect(&FileRequestBatch::with_probes(
            needs.required.iter().filter_map(|request| match request {
                ResourceRequest::File(file) => Some(file.clone()),
                ResourceRequest::Font(_) | ResourceRequest::PkFont(_) => None,
            }),
            needs.probes.iter().filter_map(|request| match request {
                ResourceRequest::File(file) => Some(file.clone()),
                ResourceRequest::Font(_) | ResourceRequest::PkFont(_) => None,
            }),
            [],
        ));
    }
    fn reject_pending(&mut self) {
        self.pending_root = None;
        self.candidate = None;
        self.awaiting.clear();
    }
}

#[cfg(test)]
mod tests;
