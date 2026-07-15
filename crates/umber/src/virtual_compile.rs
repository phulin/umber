use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;

use tex_fonts::{
    AcceptedFontContainers, FontLimits, FontRequest, FontRequestKey, OpenTypeFont, ResolvedFont,
};
use tex_out::html::{HtmlFontKey, HtmlFontResolver, WebFont};
use tex_state::{ContentHash, JobClock, Universe, World};

use crate::{MemoryOutputFile, MemoryRunOutput, prepare_run_stores};

mod path;
mod resolvers;

use path::user_path_for_key;
use resolvers::VirtualRunResolvers;
pub use umber_vfs::{
    FileKind, FileRequest, FileRequestKey, RequestKeyError, ResolvedFile, ResourceDomain,
    VfsLimitError, VfsLimitKind, VfsLimits, VirtualPath, VirtualPathError,
};
use umber_vfs::{FileProvisioner, FileRequestBatch, ProvisionError, ProvisionOutcome};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResourceRequest {
    File(FileRequest),
    Font(FontRequest),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResourceResponse {
    File(ResolvedFile),
    Font(ResolvedFont),
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct NeedResources {
    pub required: Vec<ResourceRequest>,
    pub prefetch_hints: Vec<ResourceRequest>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SessionLimits {
    pub attempts: u32,
    pub user_files: usize,
    pub resolved_files: usize,
    pub one_file_bytes: usize,
    pub cached_file_bytes: usize,
    pub user_source_bytes: usize,
    pub output_bytes: usize,
}

impl SessionLimits {
    pub const HARD_MAX: Self = Self {
        attempts: 128,
        user_files: VfsLimits::HARD_MAX.user_files,
        resolved_files: VfsLimits::HARD_MAX.resolved_files,
        one_file_bytes: VfsLimits::HARD_MAX.one_file_bytes,
        cached_file_bytes: VfsLimits::HARD_MAX.resolved_bytes,
        user_source_bytes: VfsLimits::HARD_MAX.user_bytes,
        output_bytes: 256 * 1024 * 1024,
    };

    fn validate(self) -> Result<Self, CompileError> {
        self.vfs_limits().validate().map_err(map_vfs_limit)?;
        for (resource, attempted, hard) in [
            (
                "compile attempts",
                self.attempts as usize,
                Self::HARD_MAX.attempts as usize,
            ),
            (
                "returned output bytes",
                self.output_bytes,
                Self::HARD_MAX.output_bytes,
            ),
        ] {
            if attempted > hard {
                return Err(CompileError::HardLimitExceeded {
                    resource,
                    hard,
                    attempted,
                });
            }
        }
        Ok(self)
    }

    const fn vfs_limits(self) -> VfsLimits {
        VfsLimits {
            user_files: self.user_files,
            resolved_files: self.resolved_files,
            stage_files: VfsLimits::HARD_MAX.stage_files,
            generated_files: VfsLimits::HARD_MAX.generated_files,
            one_file_bytes: self.one_file_bytes,
            user_bytes: self.user_source_bytes,
            resolved_bytes: self.cached_file_bytes,
            stage_bytes: VfsLimits::HARD_MAX.stage_bytes,
            generated_bytes: VfsLimits::HARD_MAX.generated_bytes,
        }
    }
}

impl Default for SessionLimits {
    fn default() -> Self {
        Self {
            attempts: 32,
            user_files: 512,
            resolved_files: 512,
            one_file_bytes: 96 * 1024 * 1024,
            cached_file_bytes: 64 * 1024 * 1024,
            user_source_bytes: 16 * 1024 * 1024,
            output_bytes: 64 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionOptions {
    pub main_path: String,
    pub job_name: Option<String>,
    pub format: Option<Vec<u8>>,
    pub clock: JobClock,
    pub limits: SessionLimits,
    /// Request embedded standalone HTML in addition to DVI.
    pub html: bool,
    /// Font containers the host can provide. Browser sessions use WOFF2.
    pub accepted_font_containers: AcceptedFontContainers,
}

impl Default for SessionOptions {
    fn default() -> Self {
        Self {
            main_path: "/job/main.tex".to_owned(),
            job_name: None,
            format: None,
            clock: JobClock::DEFAULT,
            limits: SessionLimits::default(),
            html: false,
            accepted_font_containers: AcceptedFontContainers::WASM,
        }
    }
}

/// One explicitly provisioned web font for a host-neutral compile session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionWebFont {
    pub name: String,
    pub tfm_content_hash_hex: String,
    pub woff2: Vec<u8>,
    pub sha256: [u8; 32],
    pub encoding: Vec<Option<String>>,
    pub provenance: String,
    pub embeddable: bool,
}

/// One atomic root-buffer replacement for a persistent compile session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourcePatch {
    pub next_revision: tex_incr::RevisionId,
    pub base_revision: tex_incr::RevisionId,
    pub expected_hash: ContentHash,
    pub range: std::ops::Range<usize>,
    pub replacement: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompileDiagnostic {
    pub message: String,
    pub file: Option<String>,
    pub line: Option<usize>,
    pub column: Option<usize>,
}

/// One rendered text unit resolved against the accepted editor revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderedSourceLocation {
    pub revision: tex_incr::RevisionId,
    pub path: String,
    pub start: u64,
    pub end: u64,
    pub line: u32,
    pub column: u32,
}

/// Revision-checked result of a rendered-source query.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RenderedSourceResult {
    Current(RenderedSourceLocation),
    Deleted {
        minted_revision: u64,
    },
    StaleRevision {
        accepted: tex_incr::RevisionId,
    },
    OutputMismatch {
        accepted: tex_incr::RenderedOutputId,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CompileAttemptResult {
    NeedResources(NeedResources),
    Complete(MemoryRunOutput),
    Error(CompileError),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CompileError {
    InvalidVirtualPath {
        path: String,
        message: String,
    },
    InvalidRequestedPath {
        name: String,
        message: String,
    },
    UnavailableAbsoluteUserFile(String),
    MissingMainFile(String),
    HardLimitExceeded {
        resource: &'static str,
        hard: usize,
        attempted: usize,
    },
    LimitExceeded {
        resource: &'static str,
        limit: usize,
        attempted: usize,
    },
    AttemptLimit {
        limit: u32,
    },
    NoProgress,
    ConflictingResolvedBinding(String),
    UnexpectedResourceResponse(String),
    FileProvision(ProvisionError),
    Font(String),
    DistributionPathCollision(String),
    Format(String),
    Diagnostic(CompileDiagnostic),
    World(String),
    Output(String),
    Html(String),
    Incremental(String),
    SessionAlreadyStarted,
    PatchAlreadyPending,
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidVirtualPath { path, message } => {
                write!(f, "invalid virtual path {path:?}: {message}")
            }
            Self::InvalidRequestedPath { name, message } => {
                write!(f, "invalid requested path {name:?}: {message}")
            }
            Self::UnavailableAbsoluteUserFile(path) => {
                write!(f, "absolute user file {path} is unavailable")
            }
            Self::MissingMainFile(path) => write!(f, "main file {path} was not provided"),
            Self::HardLimitExceeded {
                resource,
                hard,
                attempted,
            } => write!(
                f,
                "{resource} setting {attempted} exceeds hard ceiling {hard}"
            ),
            Self::LimitExceeded {
                resource,
                limit,
                attempted,
            } => write!(
                f,
                "{resource} requires {attempted}, exceeding limit {limit}"
            ),
            Self::AttemptLimit { limit } => write!(f, "compile attempt limit {limit} reached"),
            Self::NoProgress => f.write_str("retry made no progress on requested files"),
            Self::ConflictingResolvedBinding(name) => {
                write!(
                    f,
                    "resolved request {name} was rebound to different content"
                )
            }
            Self::UnexpectedResourceResponse(name) => {
                write!(f, "resource response {name} was not requested")
            }
            Self::FileProvision(error) => error.fmt(f),
            Self::Font(message) => write!(f, "font resource rejected: {message}"),
            Self::DistributionPathCollision(path) => {
                write!(
                    f,
                    "distribution path {path} is already bound to another request"
                )
            }
            Self::Format(message) => write!(f, "format image rejected: {message}"),
            Self::Diagnostic(diagnostic) => f.write_str(&diagnostic.message),
            Self::World(message)
            | Self::Output(message)
            | Self::Html(message)
            | Self::Incremental(message) => f.write_str(message),
            Self::SessionAlreadyStarted => {
                f.write_str("user files cannot change after the first revision is accepted")
            }
            Self::PatchAlreadyPending => f.write_str("a source patch is already pending"),
        }
    }
}

impl std::error::Error for CompileError {}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FontResponseFingerprint {
    container: tex_fonts::FontContainer,
    object: tex_fonts::FontObjectIdentity,
    declared_object: Option<tex_fonts::FontObjectIdentity>,
    declared_program: Option<tex_fonts::FontProgramIdentity>,
    provenance: Option<String>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum ResourceRequestKey {
    File(FileRequestKey),
    Font(FontRequestKey),
}

pub struct VirtualCompileSession {
    main_path: VirtualPath,
    job_name: String,
    format: Option<Vec<u8>>,
    clock: JobClock,
    limits: SessionLimits,
    user_files: BTreeMap<VirtualPath, Vec<u8>>,
    user_bytes: usize,
    files: FileProvisioner,
    font_cached_bytes: usize,
    attempts: u32,
    awaiting: Option<BTreeSet<ResourceRequestKey>>,
    font_requests: BTreeMap<FontRequestKey, FontRequest>,
    resolved_fonts: BTreeMap<FontRequestKey, OpenTypeFont>,
    font_responses: BTreeMap<FontRequestKey, FontResponseFingerprint>,
    accepted_font_containers: AcceptedFontContainers,
    html: bool,
    html_fonts: BTreeMap<(String, String), SessionWebFont>,
    html_font_bytes: usize,
    incremental: Option<tex_incr::Session>,
    accepted_output: Option<MemoryRunOutput>,
    pending_patch: Option<(tex_incr::RevisionId, tex_incr::Edit)>,
    last_reuse: Option<tex_incr::ReuseMetrics>,
}

impl VirtualCompileSession {
    pub fn new(options: SessionOptions) -> Result<Self, CompileError> {
        let limits = options.limits.validate()?;
        let main_path = VirtualPath::user(&options.main_path).map_err(|error| {
            CompileError::InvalidVirtualPath {
                path: options.main_path.clone(),
                message: error.to_string(),
            }
        })?;
        if let Some(format) = &options.format {
            limits
                .vfs_limits()
                .check(VfsLimitKind::OneFileBytes, format.len())
                .map_err(map_vfs_limit)?;
        }
        let job_name = options.job_name.unwrap_or_else(|| {
            Path::new(main_path.as_str())
                .file_stem()
                .and_then(|name| name.to_str())
                .unwrap_or("texput")
                .to_owned()
        });
        Ok(Self {
            main_path,
            job_name,
            format: options.format,
            clock: options.clock,
            limits,
            user_files: BTreeMap::new(),
            user_bytes: 0,
            files: FileProvisioner::new(limits.vfs_limits()).map_err(map_vfs_limit)?,
            font_cached_bytes: 0,
            attempts: 0,
            awaiting: None,
            font_requests: BTreeMap::new(),
            resolved_fonts: BTreeMap::new(),
            font_responses: BTreeMap::new(),
            accepted_font_containers: options.accepted_font_containers,
            html: options.html,
            html_fonts: BTreeMap::new(),
            html_font_bytes: 0,
            incremental: None,
            accepted_output: None,
            pending_patch: None,
            last_reuse: None,
        })
    }

    pub fn add_html_font(&mut self, font: SessionWebFont) -> Result<(), CompileError> {
        check_limit(
            "one HTML font bytes",
            font.woff2.len(),
            self.limits.one_file_bytes,
        )?;
        if font.encoding.len() != 256 {
            return Err(CompileError::Html(format!(
                "HTML font {} encoding has {} entries, expected 256",
                font.name,
                font.encoding.len()
            )));
        }
        if font.tfm_content_hash_hex.len() != 64
            || !font
                .tfm_content_hash_hex
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(CompileError::Html(
                "HTML font TFM identity must be 64 lowercase hex digits".to_owned(),
            ));
        }
        let key = (font.name.clone(), font.tfm_content_hash_hex.clone());
        let replaced = self.html_fonts.get(&key).map_or(0, |font| font.woff2.len());
        let attempted = self
            .html_font_bytes
            .checked_sub(replaced)
            .and_then(|bytes| bytes.checked_add(font.woff2.len()))
            .ok_or(CompileError::LimitExceeded {
                resource: "cached HTML font bytes",
                limit: self.limits.cached_file_bytes,
                attempted: usize::MAX,
            })?;
        check_limit(
            "cached HTML font bytes",
            attempted,
            self.limits.cached_file_bytes,
        )?;
        self.html_fonts.insert(key, font);
        self.html_font_bytes = attempted;
        Ok(())
    }

    pub fn add_user_file(&mut self, path: &str, bytes: Vec<u8>) -> Result<(), CompileError> {
        if self.accepted_output.is_some() {
            return Err(CompileError::SessionAlreadyStarted);
        }
        let path = VirtualPath::user(path).map_err(|error| CompileError::InvalidVirtualPath {
            path: path.to_owned(),
            message: error.to_string(),
        })?;
        let vfs_limits = self.limits.vfs_limits();
        vfs_limits
            .check(VfsLimitKind::OneFileBytes, bytes.len())
            .map_err(map_vfs_limit)?;
        let replaced = self.user_files.get(&path);
        let file_count = self
            .user_files
            .len()
            .saturating_add(usize::from(replaced.is_none()));
        vfs_limits
            .check(VfsLimitKind::UserFiles, file_count)
            .map_err(map_vfs_limit)?;
        let replaced = replaced.map_or(0, Vec::len);
        let attempted = vfs_limits
            .checked_replacement_total(
                VfsLimitKind::UserBytes,
                self.user_bytes,
                replaced,
                bytes.len(),
            )
            .map_err(map_vfs_limit)?;
        self.user_files.insert(path.clone(), bytes.clone());
        self.user_bytes = attempted;
        if let Some(session) = &mut self.incremental {
            session
                .register_input_file(path.as_path(), bytes)
                .map_err(|error| CompileError::Incremental(error.to_string()))?;
        }
        Ok(())
    }

    pub fn apply_patch(&mut self, patch: SourcePatch) -> Result<(), CompileError> {
        if self.pending_patch.is_some() {
            return Err(CompileError::PatchAlreadyPending);
        }
        let session = self.incremental.as_ref().ok_or_else(|| {
            CompileError::Incremental("the initial revision has not been accepted".to_owned())
        })?;
        let edit = tex_incr::Edit {
            base_revision: patch.base_revision,
            expected_hash: patch.expected_hash,
            range: patch.range,
            replacement: patch.replacement,
        };
        session
            .validate_edit(patch.next_revision, &edit)
            .map_err(|error| CompileError::Incremental(error.to_string()))?;
        self.pending_patch = Some((patch.next_revision, edit));
        self.awaiting = None;
        self.accepted_output = None;
        Ok(())
    }

    #[must_use]
    pub fn revision(&self) -> Option<tex_incr::RevisionId> {
        self.incremental
            .as_ref()
            .filter(|_| self.accepted_output.is_some() || self.pending_patch.is_some())
            .map(tex_incr::Session::revision)
    }

    #[must_use]
    pub fn content_hash(&self) -> Option<ContentHash> {
        self.revision().and_then(|_| {
            self.incremental
                .as_ref()
                .map(tex_incr::Session::content_hash)
        })
    }

    #[must_use]
    pub fn rendered_output_id(&self) -> Option<tex_incr::RenderedOutputId> {
        self.revision()
            .and_then(|_| self.incremental.as_ref().map(tex_incr::Session::output_id))
    }

    /// Resolves one HTML page/event/unit against the currently accepted output.
    pub fn rendered_source_location(
        &self,
        page: u32,
        event: u32,
        unit: Option<u32>,
        output_id: tex_incr::RenderedOutputId,
        revision: tex_incr::RevisionId,
    ) -> Result<Option<RenderedSourceResult>, CompileError> {
        if self.accepted_output.is_none() {
            return Ok(None);
        }
        let Some(session) = self.incremental.as_ref() else {
            return Ok(None);
        };
        session
            .rendered_source_location(page, event, unit, output_id, revision)
            .map(|location| {
                location.map(|result| match result {
                    tex_incr::RenderedSourceResult::Current(location) => {
                        RenderedSourceResult::Current(RenderedSourceLocation {
                            revision,
                            path: location.path,
                            start: location.start,
                            end: location.end,
                            line: location.line,
                            column: location.column,
                        })
                    }
                    tex_incr::RenderedSourceResult::Deleted { minted_revision } => {
                        RenderedSourceResult::Deleted { minted_revision }
                    }
                    tex_incr::RenderedSourceResult::StaleRevision { accepted } => {
                        RenderedSourceResult::StaleRevision { accepted }
                    }
                    tex_incr::RenderedSourceResult::OutputMismatch { accepted } => {
                        RenderedSourceResult::OutputMismatch { accepted }
                    }
                })
            })
            .map_err(|error| CompileError::Incremental(error.to_string()))
    }

    #[must_use]
    pub const fn reuse_metrics(&self) -> Option<tex_incr::ReuseMetrics> {
        self.last_reuse
    }

    #[must_use]
    pub fn retention_metrics(&self) -> Option<tex_incr::RetentionMetrics> {
        self.incremental
            .as_ref()
            .and_then(tex_incr::Session::retention_metrics)
    }

    pub fn provide_resolved_file(
        &mut self,
        request: FileRequestKey,
        virtual_path: &str,
        bytes: Vec<u8>,
    ) -> Result<(), CompileError> {
        self.provide_file_inner(
            ResolvedFile {
                request,
                virtual_path: virtual_path.to_owned(),
                bytes,
                expected_digest: None,
            },
            false,
            true,
        )
    }

    pub fn provide_resources(
        &mut self,
        responses: Vec<ResourceResponse>,
    ) -> Result<(), CompileError> {
        let mut staged_files = self.files.clone();
        let mut staged_fonts = self.resolved_fonts.clone();
        let mut staged_font_responses = self.font_responses.clone();
        let original_files = std::mem::replace(&mut self.files, staged_files);
        let original_fonts = std::mem::replace(&mut self.resolved_fonts, staged_fonts);
        let original_font_responses =
            std::mem::replace(&mut self.font_responses, staged_font_responses);
        let original_font_cached_bytes = self.font_cached_bytes;
        let result = responses
            .into_iter()
            .try_for_each(|response| match response {
                ResourceResponse::File(file) => self.provide_file_inner(file, true, false),
                ResourceResponse::Font(font) => self.provide_resolved_font(font),
            });
        if result.is_err() {
            staged_files = std::mem::replace(&mut self.files, original_files);
            staged_fonts = std::mem::replace(&mut self.resolved_fonts, original_fonts);
            staged_font_responses =
                std::mem::replace(&mut self.font_responses, original_font_responses);
            drop((staged_files, staged_fonts, staged_font_responses));
            self.font_cached_bytes = original_font_cached_bytes;
        } else if let Some(session) = &mut self.incremental {
            for (request, file) in self.files.files() {
                if original_files.get(request).is_none() {
                    session
                        .register_input_file(file.path().as_path(), file.bytes().to_vec())
                        .map_err(|error| CompileError::Incremental(error.to_string()))?;
                }
            }
        }
        result
    }

    pub fn provide_resolved_font(&mut self, response: ResolvedFont) -> Result<(), CompileError> {
        let key = response.request.clone();
        let request = self.font_requests.get(&key).ok_or_else(|| {
            CompileError::UnexpectedResourceResponse(key.logical_name().to_owned())
        })?;
        let fingerprint = FontResponseFingerprint {
            container: response.container,
            object: tex_fonts::FontObjectIdentity::for_bytes(&response.bytes),
            declared_object: response.declared_object_sha256,
            declared_program: response.declared_program_identity,
            provenance: response.provenance.clone(),
        };
        if let Some(existing) = self.font_responses.get(&key) {
            if existing == &fingerprint {
                return Ok(());
            }
            return Err(CompileError::ConflictingResolvedBinding(
                key.logical_name().to_owned(),
            ));
        }
        let font = OpenTypeFont::parse(request, response, FontLimits::default())
            .map_err(|error| CompileError::Font(error.to_string()))?;
        let attempted = self
            .cached_file_bytes()
            .checked_add(font.transport_bytes.len())
            .ok_or(CompileError::LimitExceeded {
                resource: "cached resource bytes",
                limit: self.limits.cached_file_bytes,
                attempted: usize::MAX,
            })?;
        check_limit(
            "cached resource bytes",
            attempted,
            self.limits.cached_file_bytes,
        )?;
        let font_bytes = font.transport_bytes.len();
        self.resolved_fonts.insert(key.clone(), font);
        self.font_responses.insert(key.clone(), fingerprint);
        self.font_cached_bytes = self
            .font_cached_bytes
            .checked_add(font_bytes)
            .expect("combined cache limit checked overflow");
        Ok(())
    }

    fn provide_file_inner(
        &mut self,
        response: ResolvedFile,
        require_expected: bool,
        register_incremental: bool,
    ) -> Result<(), CompileError> {
        let request = response.request.clone();
        let mut staged = self.files.clone();
        let outcome = if require_expected {
            staged.provision(response)
        } else {
            staged.preload(response)
        }
        .map_err(map_provision)?;
        let attempted = staged
            .resolved_bytes()
            .checked_add(self.font_cached_bytes)
            .ok_or(CompileError::LimitExceeded {
                resource: "cached resource bytes",
                limit: self.limits.cached_file_bytes,
                attempted: usize::MAX,
            })?;
        check_limit(
            "cached resource bytes",
            attempted,
            self.limits.cached_file_bytes,
        )?;
        self.files = staged;
        if register_incremental
            && outcome == ProvisionOutcome::Inserted
            && let (Some(session), Some(file)) = (&mut self.incremental, self.files.get(&request))
        {
            session
                .register_input_file(file.path().as_path(), file.bytes().to_vec())
                .map_err(|error| CompileError::Incremental(error.to_string()))?;
        }
        Ok(())
    }

    pub fn compile_attempt(&mut self) -> CompileAttemptResult {
        if self.pending_patch.is_none()
            && let Some(output) = &self.accepted_output
        {
            return CompileAttemptResult::Complete(output.clone());
        }
        if self.attempts >= self.limits.attempts {
            return CompileAttemptResult::Error(CompileError::AttemptLimit {
                limit: self.limits.attempts,
            });
        }
        if let Some(awaiting) = &self.awaiting {
            let progressed = awaiting.iter().any(|key| match key {
                ResourceRequestKey::File(key) => {
                    self.files.get(key).is_some()
                        || user_path_for_key(key)
                            .is_ok_and(|path| self.user_files.contains_key(&path))
                }
                ResourceRequestKey::Font(key) => self.resolved_fonts.contains_key(key),
            });
            if !progressed {
                return CompileAttemptResult::Error(CompileError::NoProgress);
            }
        }
        self.awaiting = None;
        self.attempts += 1;

        match self.run_attempt() {
            Ok(result) => result,
            Err(error) => CompileAttemptResult::Error(error),
        }
    }

    fn run_attempt(&mut self) -> Result<CompileAttemptResult, CompileError> {
        if self.incremental.is_none() {
            let source = self
                .user_files
                .get(&self.main_path)
                .ok_or_else(|| CompileError::MissingMainFile(self.main_path.to_string()))?;
            let source = String::from_utf8(source.clone()).map_err(|_| {
                CompileError::Incremental("the editable main file must be valid UTF-8".to_owned())
            })?;
            let mut world = World::memory_with_clock(self.clock);
            for (path, bytes) in &self.user_files {
                world
                    .set_memory_file(path.as_path(), bytes.clone())
                    .map_err(|error| CompileError::World(error.to_string()))?;
            }
            for (_, resolved) in self.files.files() {
                world
                    .set_memory_file(resolved.path().as_path(), resolved.bytes().to_vec())
                    .map_err(|error| CompileError::World(error.to_string()))?;
            }
            let mut template = if let Some(format) = &self.format {
                Universe::from_format(world, format)
                    .map_err(|error| CompileError::Format(error.to_string()))?
            } else {
                let mut template = Universe::with_world(world);
                prepare_run_stores(&mut template);
                template
            };
            // The root is supplied through the editor input, not reopened as
            // an included file. Keeping the registered copy is harmless and
            // preserves absolute self-input behavior.
            template
                .world_mut()
                .set_memory_file(self.main_path.as_path(), source.as_bytes().to_vec())
                .map_err(|error| CompileError::World(error.to_string()))?;
            self.incremental = Some(
                tex_incr::Session::start_with_source_path(
                    template,
                    &self.job_name,
                    self.main_path.as_str(),
                    tex_incr::RevisionId::new(1),
                    source,
                    self.limits.cached_file_bytes,
                )
                .map_err(|error| CompileError::Incremental(error.to_string()))?,
            );
        }

        let mut resolvers = VirtualRunResolvers::new(
            &self.user_files,
            &self.files,
            &self.resolved_fonts,
            self.accepted_font_containers,
            self.html,
        );
        let (input_resolver, font_resolver) = resolvers.resolvers();
        let execution = if let Some((next_revision, edit)) = &self.pending_patch {
            self.incremental
                .as_mut()
                .expect("incremental session was initialized")
                .advance_with_resolvers(*next_revision, edit.clone(), input_resolver, font_resolver)
        } else {
            self.incremental
                .as_mut()
                .expect("incremental session was initialized")
                .cold_with_resolvers(input_resolver, font_resolver)
        };
        let (file_misses, font_misses, fatal) = resolvers.finish();

        if !file_misses.is_empty() || !font_misses.is_empty() {
            self.files
                .expect(&FileRequestBatch::new(file_misses.clone(), []));
            for request in &font_misses {
                self.font_requests
                    .entry(request.key.clone())
                    .or_insert_with(|| request.clone());
            }
            let mut required = file_misses
                .into_iter()
                .map(ResourceRequest::File)
                .chain(font_misses.into_iter().map(ResourceRequest::Font))
                .collect::<Vec<_>>();
            required.sort_by_key(resource_sort_key);
            required.dedup();
            self.awaiting = Some(
                required
                    .iter()
                    .map(|request| match request {
                        ResourceRequest::File(request) => {
                            ResourceRequestKey::File(request.key().clone())
                        }
                        ResourceRequest::Font(request) => {
                            ResourceRequestKey::Font(request.key.clone())
                        }
                    })
                    .collect(),
            );
            return Ok(CompileAttemptResult::NeedResources(NeedResources {
                required,
                prefetch_hints: Vec::new(),
            }));
        }
        if let Some(fatal) = fatal {
            return Err(fatal);
        }
        let accepted = execution.map_err(|error| {
            CompileError::Diagnostic(CompileDiagnostic {
                message: error.to_string(),
                file: None,
                line: None,
                column: None,
            })
        })?;
        let world = self
            .incremental
            .as_ref()
            .expect("accepted incremental session exists")
            .materialize_accepted_world()
            .map_err(|error| CompileError::Output(error.to_string()))?;
        let terminal = world
            .memory_terminal_output()
            .ok_or_else(|| CompileError::Output("accepted output is not memory-backed".to_owned()))?
            .to_vec();
        let log = world
            .memory_log_output()
            .ok_or_else(|| CompileError::Output("accepted output is not memory-backed".to_owned()))?
            .to_vec();
        let files = world
            .memory_outputs()
            .ok_or_else(|| CompileError::Output("accepted output is not memory-backed".to_owned()))?
            .map(|file| MemoryOutputFile {
                path: file.path().to_owned(),
                bytes: file.bytes().to_vec(),
            })
            .collect::<Vec<_>>();
        let dvi = if accepted.dvi_pages.is_empty() {
            Vec::new()
        } else {
            accepted
                .dvi_bytes()
                .map_err(|error| CompileError::Output(error.to_string()))?
        };
        let mut output = MemoryRunOutput {
            terminal,
            log,
            dvi,
            html: None,
            html_assets: Vec::new(),
            files,
        };
        let existing = output
            .terminal
            .len()
            .saturating_add(output.log.len())
            .saturating_add(output.dvi.len())
            .saturating_add(
                output
                    .files
                    .iter()
                    .map(|file| file.bytes.len())
                    .sum::<usize>(),
            );
        let remaining = self.limits.output_bytes.saturating_sub(existing);
        let html = if self.html {
            let output_id = self
                .incremental
                .as_ref()
                .expect("accepted incremental session exists")
                .output_id();
            let mut resolver = SessionFontResolver {
                fonts: &self.html_fonts,
                resolved: &self.resolved_fonts,
                responses: &self.font_responses,
            };
            let html_options = tex_out::html::HtmlOptions {
                revision: accepted.revision.raw(),
                output_id,
                max_html_bytes: remaining,
                max_total_asset_bytes: remaining,
                max_asset_bytes: remaining,
                ..tex_out::html::HtmlOptions::default()
            };
            Some(
                crate::html_from_committed_artifacts(
                    &accepted.artifacts,
                    &mut resolver,
                    &html_options,
                )
                .map_err(|error| CompileError::Html(error.to_string()))?,
            )
        } else {
            None
        };
        if let Some(html) = html {
            let attempted = existing.saturating_add(html.html.len()).saturating_add(
                html.assets
                    .iter()
                    .map(|asset| asset.bytes.len())
                    .sum::<usize>(),
            );
            check_limit("returned output bytes", attempted, self.limits.output_bytes)?;
            output.html = Some(html.html);
            output.html_assets = html
                .assets
                .into_iter()
                .map(|asset| crate::MemoryOutputFile {
                    path: asset.path.into(),
                    bytes: asset.bytes,
                })
                .collect();
        }
        check_limit("returned output bytes", existing, self.limits.output_bytes)?;
        self.pending_patch = None;
        self.last_reuse = Some(accepted.reuse);
        self.accepted_output = Some(output.clone());
        Ok(CompileAttemptResult::Complete(output))
    }

    pub fn clear_distribution_cache(&mut self) -> Result<(), CompileError> {
        if let Some(session) = &self.incremental {
            let latest = session.source().as_bytes().to_vec();
            let replaced = self
                .user_files
                .insert(self.main_path.clone(), latest.clone())
                .map_or(0, |bytes| bytes.len());
            self.user_bytes = self
                .user_bytes
                .saturating_sub(replaced)
                .saturating_add(latest.len());
        }
        self.files.clear();
        self.resolved_fonts.clear();
        self.font_responses.clear();
        self.font_requests.clear();
        self.font_cached_bytes = 0;
        self.awaiting = None;
        self.incremental = None;
        self.accepted_output = None;
        self.pending_patch = None;
        self.last_reuse = None;
        Ok(())
    }

    #[must_use]
    pub const fn attempts(&self) -> u32 {
        self.attempts
    }

    #[must_use]
    pub fn resolved_file_count(&self) -> usize {
        self.files.len()
    }

    #[must_use]
    pub fn cached_file_bytes(&self) -> usize {
        self.files
            .resolved_bytes()
            .saturating_add(self.font_cached_bytes)
    }
}

fn resource_sort_key(request: &ResourceRequest) -> (u8, String) {
    match request {
        ResourceRequest::File(request) => (
            0,
            format!("{:?}:{}", request.key().kind(), request.key().name()),
        ),
        ResourceRequest::Font(request) => (1, request.key.logical_name().to_owned()),
    }
}

struct SessionFontResolver<'a> {
    fonts: &'a BTreeMap<(String, String), SessionWebFont>,
    resolved: &'a BTreeMap<FontRequestKey, OpenTypeFont>,
    responses: &'a BTreeMap<FontRequestKey, FontResponseFingerprint>,
}

impl HtmlFontResolver for SessionFontResolver<'_> {
    fn resolve(&mut self, font: &tex_out::FontResource) -> Result<WebFont, String> {
        if let Some(binding) = &font.opentype {
            let (key, supplied) = self
                .resolved
                .iter()
                .find(|(key, supplied)| {
                    key.logical_name() == font.name
                        && supplied.identity == binding.program_identity
                        && supplied.object_identity == binding.object_identity
                })
                .ok_or_else(|| {
                    format!(
                        "retained OpenType resource for artifact font {} is unavailable or mismatched",
                        font.name
                    )
                })?;
            if binding.container != tex_fonts::FontContainer::Woff2
                || supplied.container != tex_fonts::FontContainer::Woff2
            {
                return Err(format!(
                    "HTML reuse for retained {:?} font {} is not supported",
                    supplied.container, font.name
                ));
            }
            let expected_instance = tex_fonts::FontInstanceIdentity::new(
                supplied.identity,
                key.face_index,
                font.at_size.raw(),
                &key.variation,
                &key.feature_policy,
                tex_fonts::WritingDirection::LeftToRight,
            );
            if binding.instance_identity != expected_instance {
                return Err(format!(
                    "artifact font instance identity for {} does not match the retained selection",
                    font.name
                ));
            }
            let response = self.responses.get(key).ok_or_else(|| {
                format!(
                    "retained response metadata for {} is unavailable",
                    font.name
                )
            })?;
            let provenance = response.provenance.clone().ok_or_else(|| {
                format!("retained font {} has no embedding provenance", font.name)
            })?;
            let mut encoding = vec![None; 256];
            for (code, mapped) in encoding.iter_mut().enumerate() {
                if let Some(scalar) = char::from_u32(code as u32)
                    && supplied.cmap.glyph(scalar).is_some()
                {
                    *mapped = Some(scalar.to_string());
                }
            }
            return Ok(WebFont {
                key: HtmlFontKey::from(font),
                woff2: supplied.transport_bytes.to_vec(),
                sha256: supplied.object_identity.bytes(),
                encoding,
                provenance,
                embeddable: true,
            });
        }
        let lookup = (font.name.clone(), font.tfm_content_hash.hex());
        let supplied = self.fonts.get(&lookup).ok_or_else(|| {
            format!(
                "no HTML font was supplied for {} with TFM identity {}",
                font.name, lookup.1
            )
        })?;
        Ok(WebFont {
            key: HtmlFontKey::from(font),
            woff2: supplied.woff2.clone(),
            sha256: supplied.sha256,
            encoding: supplied.encoding.clone(),
            provenance: supplied.provenance.clone(),
            embeddable: supplied.embeddable,
        })
    }
}

fn check_limit(resource: &'static str, attempted: usize, limit: usize) -> Result<(), CompileError> {
    if attempted > limit {
        return Err(CompileError::LimitExceeded {
            resource,
            limit,
            attempted,
        });
    }
    Ok(())
}

fn map_vfs_limit(error: VfsLimitError) -> CompileError {
    match error {
        VfsLimitError::HardLimitExceeded {
            kind,
            hard,
            attempted,
        } => CompileError::HardLimitExceeded {
            resource: kind.description(),
            hard,
            attempted,
        },
        VfsLimitError::LimitExceeded {
            kind,
            limit,
            attempted,
        } => CompileError::LimitExceeded {
            resource: kind.description(),
            limit,
            attempted,
        },
    }
}

fn map_provision(error: ProvisionError) -> CompileError {
    match error {
        ProvisionError::Limit(error) => map_vfs_limit(error),
        ProvisionError::Conflict { request, .. } => {
            CompileError::ConflictingResolvedBinding(request.name().to_owned())
        }
        ProvisionError::PathConflict { path, .. } => {
            CompileError::DistributionPathCollision(path.to_string())
        }
        ProvisionError::UnexpectedRequest(request) => {
            CompileError::UnexpectedResourceResponse(request.name().to_owned())
        }
        ProvisionError::InvalidPath { path, message, .. } => CompileError::InvalidVirtualPath {
            path,
            message: message.to_owned(),
        },
        error @ (ProvisionError::KindMismatch { .. } | ProvisionError::DigestMismatch { .. }) => {
            CompileError::FileProvision(error)
        }
    }
}

#[cfg(test)]
mod tests;
