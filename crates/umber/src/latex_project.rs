use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::PathBuf;

use bib_engine::{BibAttempt, BibFailure, BibJob, BibResult, BibSession, BibSessionOptions};
use tex_fonts::{FontRequestKey, ResolvedFont};
use tex_state::ContentHash;
use umber_vfs::{
    BuildId, BuildPlan, FileProvisioner, FileRequestBatch, ProducerId, ResolvedFile, VirtualPath,
};

use crate::{
    CompileAttemptResult, CompileError, MemoryOutputFile, MemoryRunOutput, NeedResources,
    RenderedSourceResult, ResourceRequest, ResourceResponse, SessionOptions, SourcePatch,
    VirtualCompileSession,
};

const PROJECT_PRODUCER: ProducerId = ProducerId::new(3);

mod support;
use support::{
    CandidateStop, accepted_generated, add_candidate_inputs, candidate_snapshot, file_needs,
    generated_signature, merge_tex_files, project_vfs_limits,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LatexProjectLimits {
    pub attempts: u32,
    pub passes: u32,
}

impl Default for LatexProjectLimits {
    fn default() -> Self {
        Self {
            attempts: 32,
            passes: 8,
        }
    }
}

#[derive(Clone, Debug)]
pub struct LatexProjectOptions {
    pub tex: SessionOptions,
    pub bibliography: BibJob,
    pub bib_session: BibSessionOptions,
    pub limits: LatexProjectLimits,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LatexProjectOutput {
    pub revision: tex_incr::RevisionId,
    pub content_hash: ContentHash,
    pub passes: u32,
    pub tex: MemoryRunOutput,
    pub bibliography: Option<BibResult>,
    pub generated_files: Vec<MemoryOutputFile>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LatexProjectAttempt {
    NeedResources(NeedResources),
    Complete(LatexProjectOutput),
    Error(LatexProjectError),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LatexProjectError {
    Compile(CompileError),
    Bibliography(BibFailure),
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

pub struct LatexProjectSession {
    options: LatexProjectOptions,
    files: FileProvisioner,
    bibliography: BibSession,
    file_responses: BTreeMap<umber_vfs::FileRequestKey, ResolvedFile>,
    font_responses: BTreeMap<FontRequestKey, ResolvedFont>,
    unavailable_fonts: BTreeSet<FontRequestKey>,
    awaiting: BTreeSet<ProjectRequestKey>,
    attempts: u32,
    accepted_revision: Option<tex_incr::RevisionId>,
    accepted_root: Option<Vec<u8>>,
    pending_root: Option<(tex_incr::RevisionId, Vec<u8>)>,
    accepted_tex: Option<VirtualCompileSession>,
    accepted_output: Option<LatexProjectOutput>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum ProjectRequestKey {
    File(umber_vfs::FileRequestKey),
    Font(FontRequestKey),
}

impl LatexProjectSession {
    pub fn new(options: LatexProjectOptions) -> Result<Self, LatexProjectError> {
        for (name, value, hard) in [
            ("attempt", options.limits.attempts, 128),
            ("pass", options.limits.passes, 64),
        ] {
            if value == 0 || value > hard {
                return Err(LatexProjectError::InvalidLimit { name, value });
            }
        }
        let files = FileProvisioner::new(project_vfs_limits(&options.tex))
            .map_err(|error| LatexProjectError::Transaction(error.to_string()))?;
        let bibliography = BibSession::new(options.bib_session)
            .map_err(|error| LatexProjectError::Transaction(error.to_string()))?;
        Ok(Self {
            options,
            files,
            bibliography,
            file_responses: BTreeMap::new(),
            font_responses: BTreeMap::new(),
            unavailable_fonts: BTreeSet::new(),
            awaiting: BTreeSet::new(),
            attempts: 0,
            accepted_revision: None,
            accepted_root: None,
            pending_root: None,
            accepted_tex: None,
            accepted_output: None,
        })
    }

    pub fn add_user_file(&mut self, path: &str, bytes: Vec<u8>) -> Result<(), LatexProjectError> {
        if self.accepted_output.is_some() {
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
            .map_err(|error| LatexProjectError::Transaction(error.to_string()))?;
        Ok(())
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
        self.awaiting.clear();
        Ok(())
    }

    pub fn provide_resources(
        &mut self,
        responses: Vec<ResourceResponse>,
    ) -> Result<(), LatexProjectError> {
        let mut files = self.files.clone();
        let mut file_responses = self.file_responses.clone();
        let mut font_responses = self.font_responses.clone();
        let mut unavailable_fonts = self.unavailable_fonts.clone();
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
                        .map_err(|error| LatexProjectError::Transaction(error.to_string()))?;
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
                        .map_err(|error| LatexProjectError::Transaction(error.to_string()))?;
                }
                ResourceResponse::Font(font) => {
                    let key = ProjectRequestKey::Font(font.request.clone());
                    if !self.awaiting.contains(&key) {
                        return Err(LatexProjectError::UnexpectedResource(
                            font.request.logical_name().to_owned(),
                        ));
                    }
                    if unavailable_fonts.contains(&font.request) {
                        return Err(LatexProjectError::ConflictingResource(
                            font.request.logical_name().to_owned(),
                        ));
                    }
                    if let Some(existing) = font_responses.get(&font.request)
                        && existing != &font
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
            }
        }
        self.files = files;
        self.file_responses = file_responses;
        self.font_responses = font_responses;
        self.unavailable_fonts = unavailable_fonts;
        Ok(())
    }

    pub fn compile_attempt(&mut self) -> LatexProjectAttempt {
        if self.pending_root.is_none()
            && let Some(output) = &self.accepted_output
        {
            return LatexProjectAttempt::Complete(output.clone());
        }
        if self.attempts >= self.options.limits.attempts {
            self.reject_pending();
            return LatexProjectAttempt::Error(LatexProjectError::Compile(
                CompileError::AttemptLimit {
                    limit: self.options.limits.attempts,
                },
            ));
        }
        if !self.awaiting.is_empty()
            && !self.awaiting.iter().any(|key| match key {
                ProjectRequestKey::File(key) => {
                    self.file_responses.contains_key(key)
                        || self.files.unavailable_keys().any(|missing| missing == key)
                }
                ProjectRequestKey::Font(key) => {
                    self.font_responses.contains_key(key) || self.unavailable_fonts.contains(key)
                }
            })
        {
            self.reject_pending();
            return LatexProjectAttempt::Error(LatexProjectError::Compile(
                CompileError::NoProgress,
            ));
        }
        self.attempts += 1;
        match self.run_candidate() {
            Ok(output) => LatexProjectAttempt::Complete(output),
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
            .map(|bytes| ContentHash::from_bytes(bytes))
    }

    #[must_use]
    pub fn accepted_output(&self) -> Option<&LatexProjectOutput> {
        self.accepted_output.as_ref()
    }

    pub fn rendered_source_location(
        &self,
        page: u32,
        event: u32,
        unit: Option<u32>,
        output_id: tex_incr::RenderedOutputId,
        revision: tex_incr::RevisionId,
    ) -> Result<Option<RenderedSourceResult>, LatexProjectError> {
        if self.pending_root.is_some() {
            return Ok(None);
        }
        self.accepted_tex
            .as_ref()
            .map_or(Ok(None), |tex| {
                tex.rendered_source_location(page, event, unit, output_id, revision)
            })
            .map_err(LatexProjectError::Compile)
    }

    fn run_candidate(&mut self) -> Result<LatexProjectOutput, CandidateStop> {
        let (revision, root) = self.candidate_root()?;
        let mut generated = accepted_generated(&self.files)?;
        let mut seen = BTreeMap::new();
        seen.insert(generated_signature(&generated), 0u32);
        let bib_paths = self
            .options
            .bibliography
            .options()
            .outputs()
            .map(|request| request.path().clone())
            .collect::<BTreeSet<_>>();

        for pass in 1..=self.options.limits.passes {
            let before = generated_signature(&generated);
            let (tex_output, tex_session) = self.run_tex_pass(revision, &root, &generated)?;
            generated.retain(|path, _| bib_paths.contains(path));
            merge_tex_files(&mut generated, &tex_output.files)?;

            let mut bib_result = None;
            if generated.contains_key(self.options.bibliography.control_path()) {
                let snapshot = candidate_snapshot(
                    &self.files,
                    &self.options.tex.main_path,
                    &root,
                    &generated,
                )?;
                match self
                    .bibliography
                    .process(&self.options.bibliography, &snapshot)
                {
                    BibAttempt::Complete(result) => {
                        for path in &bib_paths {
                            generated.remove(path);
                        }
                        for file in result.files() {
                            generated.insert(file.path().clone(), file.bytes().to_vec());
                        }
                        bib_result = Some(result);
                    }
                    BibAttempt::NeedResources(batch) => {
                        return Err(CandidateStop::Need(file_needs(batch)));
                    }
                    BibAttempt::Failed(error) => {
                        return Err(CandidateStop::Failed(LatexProjectError::Bibliography(
                            error,
                        )));
                    }
                }
            } else {
                for path in &bib_paths {
                    generated.remove(path);
                }
            }

            let after = generated_signature(&generated);
            if after == before {
                return self.accept_candidate(
                    revision,
                    root,
                    pass,
                    tex_output,
                    bib_result,
                    generated,
                    tex_session,
                );
            }
            if let Some(first_pass) = seen.insert(after, pass) {
                return Err(CandidateStop::Failed(LatexProjectError::Oscillation {
                    first_pass,
                    repeated_pass: pass,
                }));
            }
        }
        Err(CandidateStop::Failed(LatexProjectError::PassLimit {
            limit: self.options.limits.passes,
        }))
    }

    fn run_tex_pass(
        &self,
        revision: tex_incr::RevisionId,
        root: &[u8],
        generated: &BTreeMap<VirtualPath, Vec<u8>>,
    ) -> Result<(MemoryRunOutput, VirtualCompileSession), CandidateStop> {
        let mut session =
            VirtualCompileSession::new_at_revision(self.options.tex.clone(), revision)
                .map_err(LatexProjectError::Compile)?;
        add_candidate_inputs(
            &mut session,
            &self.files,
            &self.options.tex.main_path,
            root,
            generated,
        )?;
        for response in self.file_responses.values() {
            session
                .provide_resolved_file(
                    response.request.clone(),
                    &response.virtual_path,
                    response.bytes.clone(),
                )
                .map_err(LatexProjectError::Compile)?;
        }
        loop {
            match session.compile_attempt() {
                CompileAttemptResult::Complete(output) => return Ok((output, session)),
                CompileAttemptResult::Error(error) => {
                    return Err(CandidateStop::Failed(LatexProjectError::Compile(error)));
                }
                CompileAttemptResult::NeedResources(needs) => {
                    let mut supplied = Vec::new();
                    let mut missing = Vec::new();
                    for request in needs.required {
                        match &request {
                            ResourceRequest::File(file) => {
                                if let Some(response) = self.file_responses.get(file.key()) {
                                    supplied.push(ResourceResponse::File(response.clone()));
                                } else if self
                                    .files
                                    .unavailable_keys()
                                    .any(|missing| missing == file.key())
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
                        }
                    }
                    if !missing.is_empty() {
                        return Err(CandidateStop::Need(NeedResources {
                            required: missing,
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
        bibliography: Option<BibResult>,
        generated: BTreeMap<VirtualPath, Vec<u8>>,
        tex_session: VirtualCompileSession,
    ) -> Result<LatexProjectOutput, CandidateStop> {
        let mut pending = self.files.clone();
        let main = VirtualPath::user(&self.options.tex.main_path)
            .map_err(|error| LatexProjectError::Transaction(error.to_string()))?;
        pending
            .register_user(main, root.clone())
            .map_err(|error| LatexProjectError::Transaction(error.to_string()))?;
        let mut build = pending.begin_build(BuildPlan::new(BuildId::new(u64::from(self.attempts))));
        let mut stage = build
            .begin_stage(PROJECT_PRODUCER)
            .map_err(|error| LatexProjectError::Transaction(error.to_string()))?;
        for (path, bytes) in &generated {
            stage
                .write(path.clone(), bytes.clone())
                .map_err(|error| LatexProjectError::Transaction(error.to_string()))?;
        }
        stage
            .finish()
            .map_err(|error| LatexProjectError::Transaction(error.to_string()))?;
        build
            .accept()
            .map_err(|error| LatexProjectError::Transaction(error.to_string()))?;
        let generated_files = generated
            .into_iter()
            .map(|(path, bytes)| MemoryOutputFile {
                path: PathBuf::from(path.as_str()),
                bytes,
            })
            .collect();
        let output = LatexProjectOutput {
            revision,
            content_hash: ContentHash::from_bytes(&root),
            passes,
            tex,
            bibliography,
            generated_files,
        };
        self.files = pending;
        self.accepted_revision = Some(revision);
        self.accepted_root = Some(root);
        self.pending_root = None;
        self.awaiting.clear();
        self.accepted_tex = Some(tex_session);
        self.accepted_output = Some(output.clone());
        Ok(output)
    }

    fn candidate_root(&self) -> Result<(tex_incr::RevisionId, Vec<u8>), CandidateStop> {
        if let Some((revision, root)) = &self.pending_root {
            return Ok((*revision, root.clone()));
        }
        let main = VirtualPath::user(&self.options.tex.main_path)
            .map_err(|error| LatexProjectError::Transaction(error.to_string()))?;
        let snapshot = self.files.snapshot();
        let file = snapshot
            .get(&main)
            .map_err(|error| LatexProjectError::Transaction(error.to_string()))?
            .ok_or_else(|| {
                LatexProjectError::Compile(CompileError::MissingMainFile(main.to_string()))
            })?;
        Ok((tex_incr::RevisionId::new(1), file.bytes().to_vec()))
    }

    fn remember_needs(&mut self, needs: &NeedResources) {
        self.awaiting = needs
            .required
            .iter()
            .map(|request| match request {
                ResourceRequest::File(file) => ProjectRequestKey::File(file.key().clone()),
                ResourceRequest::Font(font) => ProjectRequestKey::Font(font.key.clone()),
            })
            .collect();
        let file_batch = FileRequestBatch::new(
            needs.required.iter().filter_map(|request| match request {
                ResourceRequest::File(file) => Some(file.clone()),
                ResourceRequest::Font(_) => None,
            }),
            [],
        );
        self.files.expect(&file_batch);
    }

    fn reject_pending(&mut self) {
        self.pending_root = None;
        self.awaiting.clear();
    }
}

#[cfg(test)]
mod tests;
