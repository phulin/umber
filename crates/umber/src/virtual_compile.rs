use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;
use std::sync::Arc;

use tex_lex::{InputStack, WorldInput};
use tex_out::html::{HtmlFontKey, HtmlFontResolver, WebFont};
use tex_state::{JobClock, Universe, World};

use crate::{
    EngineSession, MemoryOutputCollectionError, MemoryRunOutput,
    collect_final_memory_output_from_plans, prepare_run_stores,
};

mod path;
mod resolvers;

pub use path::{VirtualPath, VirtualPathError};
use path::{normalize_request_name, user_path_for_key};
use resolvers::VirtualRunResolvers;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum FileKind {
    TexInput,
    Tfm,
}

impl FileKind {
    const fn extension(self) -> &'static str {
        match self {
            Self::TexInput => "tex",
            Self::Tfm => "tfm",
        }
    }
}

impl fmt::Display for FileKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TexInput => f.write_str("TeX input"),
            Self::Tfm => f.write_str("TFM"),
        }
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct FileRequestKey {
    kind: FileKind,
    normalized_name: String,
}

impl FileRequestKey {
    pub fn new(kind: FileKind, name: &str) -> Result<Self, VirtualPathError> {
        Ok(Self::from_normalized(
            kind,
            normalize_request_name(kind, name)?,
        ))
    }

    fn from_normalized(kind: FileKind, normalized_name: String) -> Self {
        Self {
            kind,
            normalized_name,
        }
    }

    #[must_use]
    pub const fn kind(&self) -> FileKind {
        self.kind
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.normalized_name
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileRequest {
    key: FileRequestKey,
    original_name: String,
}

impl FileRequest {
    #[must_use]
    pub const fn key(&self) -> &FileRequestKey {
        &self.key
    }

    #[must_use]
    pub fn original_name(&self) -> &str {
        &self.original_name
    }
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
        user_files: 4096,
        resolved_files: 4096,
        one_file_bytes: 64 * 1024 * 1024,
        cached_file_bytes: 256 * 1024 * 1024,
        user_source_bytes: 64 * 1024 * 1024,
        output_bytes: 256 * 1024 * 1024,
    };

    fn validate(self) -> Result<Self, CompileError> {
        for (resource, attempted, hard) in [
            (
                "compile attempts",
                self.attempts as usize,
                Self::HARD_MAX.attempts as usize,
            ),
            ("user files", self.user_files, Self::HARD_MAX.user_files),
            (
                "resolved files",
                self.resolved_files,
                Self::HARD_MAX.resolved_files,
            ),
            (
                "one file bytes",
                self.one_file_bytes,
                Self::HARD_MAX.one_file_bytes,
            ),
            (
                "cached file bytes",
                self.cached_file_bytes,
                Self::HARD_MAX.cached_file_bytes,
            ),
            (
                "user source bytes",
                self.user_source_bytes,
                Self::HARD_MAX.user_source_bytes,
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
}

impl Default for SessionLimits {
    fn default() -> Self {
        Self {
            attempts: 32,
            user_files: 512,
            resolved_files: 512,
            one_file_bytes: 16 * 1024 * 1024,
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompileDiagnostic {
    pub message: String,
    pub file: Option<String>,
    pub line: Option<usize>,
    pub column: Option<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CompileAttemptResult {
    NeedFiles(Vec<FileRequest>),
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
    DistributionPathCollision(String),
    Format(String),
    Diagnostic(CompileDiagnostic),
    World(String),
    Output(String),
    Html(String),
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
            Self::DistributionPathCollision(path) => {
                write!(
                    f,
                    "distribution path {path} is already bound to another request"
                )
            }
            Self::Format(message) => write!(f, "format image rejected: {message}"),
            Self::Diagnostic(diagnostic) => f.write_str(&diagnostic.message),
            Self::World(message) | Self::Output(message) | Self::Html(message) => {
                f.write_str(message)
            }
        }
    }
}

impl std::error::Error for CompileError {}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ResolvedFile {
    virtual_path: VirtualPath,
    bytes: Arc<[u8]>,
}

pub struct VirtualCompileSession {
    main_path: VirtualPath,
    job_name: String,
    format: Option<Vec<u8>>,
    clock: JobClock,
    limits: SessionLimits,
    user_files: BTreeMap<VirtualPath, Vec<u8>>,
    user_bytes: usize,
    resolved_files: BTreeMap<FileRequestKey, ResolvedFile>,
    resolved_paths: BTreeMap<VirtualPath, Arc<[u8]>>,
    cached_bytes: usize,
    attempts: u32,
    awaiting: Option<BTreeSet<FileRequestKey>>,
    html: bool,
    html_fonts: BTreeMap<(String, String), SessionWebFont>,
    html_font_bytes: usize,
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
            check_limit("format image bytes", format.len(), limits.one_file_bytes)?;
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
            resolved_files: BTreeMap::new(),
            resolved_paths: BTreeMap::new(),
            cached_bytes: 0,
            attempts: 0,
            awaiting: None,
            html: options.html,
            html_fonts: BTreeMap::new(),
            html_font_bytes: 0,
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
        let path = VirtualPath::user(path).map_err(|error| CompileError::InvalidVirtualPath {
            path: path.to_owned(),
            message: error.to_string(),
        })?;
        check_limit(
            "one user file bytes",
            bytes.len(),
            self.limits.one_file_bytes,
        )?;
        let replaced = self.user_files.get(&path);
        let file_count = self
            .user_files
            .len()
            .saturating_add(usize::from(replaced.is_none()));
        check_limit("user files", file_count, self.limits.user_files)?;
        let replaced = replaced.map_or(0, Vec::len);
        let attempted = self
            .user_bytes
            .checked_sub(replaced)
            .and_then(|total| total.checked_add(bytes.len()))
            .ok_or(CompileError::LimitExceeded {
                resource: "user source bytes",
                limit: self.limits.user_source_bytes,
                attempted: usize::MAX,
            })?;
        check_limit(
            "user source bytes",
            attempted,
            self.limits.user_source_bytes,
        )?;
        self.user_files.insert(path, bytes);
        self.user_bytes = attempted;
        Ok(())
    }

    pub fn provide_resolved_file(
        &mut self,
        request: FileRequestKey,
        virtual_path: &str,
        bytes: Vec<u8>,
    ) -> Result<(), CompileError> {
        let virtual_path = VirtualPath::distribution(virtual_path).map_err(|error| {
            CompileError::InvalidVirtualPath {
                path: virtual_path.to_owned(),
                message: error.to_string(),
            }
        })?;
        check_limit(
            "one resolved file bytes",
            bytes.len(),
            self.limits.one_file_bytes,
        )?;

        if let Some(existing) = self.resolved_files.get(&request) {
            if existing.virtual_path == virtual_path && existing.bytes.as_ref() == bytes {
                return Ok(());
            }
            return Err(CompileError::ConflictingResolvedBinding(
                request.name().to_owned(),
            ));
        }
        let shared_bytes = if let Some(existing) = self.resolved_paths.get(&virtual_path) {
            if existing.as_ref() != bytes {
                return Err(CompileError::DistributionPathCollision(
                    virtual_path.to_string(),
                ));
            }
            Arc::clone(existing)
        } else {
            Arc::from(bytes)
        };

        check_limit(
            "resolved files",
            self.resolved_files.len().saturating_add(1),
            self.limits.resolved_files,
        )?;
        let added_bytes = if self.resolved_paths.contains_key(&virtual_path) {
            0
        } else {
            shared_bytes.len()
        };
        let attempted =
            self.cached_bytes
                .checked_add(added_bytes)
                .ok_or(CompileError::LimitExceeded {
                    resource: "cached file bytes",
                    limit: self.limits.cached_file_bytes,
                    attempted: usize::MAX,
                })?;
        check_limit(
            "cached file bytes",
            attempted,
            self.limits.cached_file_bytes,
        )?;

        self.resolved_paths
            .insert(virtual_path.clone(), Arc::clone(&shared_bytes));
        self.resolved_files.insert(
            request,
            ResolvedFile {
                virtual_path,
                bytes: shared_bytes,
            },
        );
        self.cached_bytes = attempted;
        Ok(())
    }

    pub fn compile_attempt(&mut self) -> CompileAttemptResult {
        if self.attempts >= self.limits.attempts {
            return CompileAttemptResult::Error(CompileError::AttemptLimit {
                limit: self.limits.attempts,
            });
        }
        if let Some(awaiting) = &self.awaiting {
            let progressed = awaiting.iter().any(|key| {
                self.resolved_files.contains_key(key)
                    || self.user_files.contains_key(&user_path_for_key(key))
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
        let mut world = World::memory_with_clock(self.clock);
        for (path, bytes) in &self.user_files {
            world
                .set_memory_file(path.as_path(), bytes.clone())
                .map_err(|error| CompileError::World(error.to_string()))?;
        }
        for resolved in self.resolved_files.values() {
            world
                .set_memory_file(resolved.virtual_path.as_path(), resolved.bytes.to_vec())
                .map_err(|error| CompileError::World(error.to_string()))?;
        }

        let mut stores = if let Some(format) = &self.format {
            Universe::from_format(world, format)
                .map_err(|error| CompileError::Format(error.to_string()))?
        } else {
            let mut stores = Universe::with_world(world);
            prepare_run_stores(&mut stores);
            stores
        };
        let main = stores
            .world_mut()
            .read_file(self.main_path.as_path())
            .map_err(|_| CompileError::MissingMainFile(self.main_path.to_string()))?;
        let mut input = InputStack::new(WorldInput::from_content(main));
        let mut resolvers = VirtualRunResolvers::new(&self.user_files, &self.resolved_files);
        let execution =
            EngineSession::new(&mut input, &mut stores, resolvers.context(&self.job_name))
                .execute();
        let (misses, fatal) = resolvers.finish();

        if !misses.is_empty() {
            self.awaiting = Some(misses.iter().map(|request| request.key.clone()).collect());
            return Ok(CompileAttemptResult::NeedFiles(misses));
        }
        if let Some(fatal) = fatal {
            return Err(fatal);
        }
        let run = execution.map_err(|error| {
            CompileError::Diagnostic(CompileDiagnostic {
                message: error.format_with_provenance(&stores),
                file: None,
                line: None,
                column: None,
            })
        })?;
        let mut output = collect_final_memory_output_from_plans(
            &mut stores,
            &run.dvi_pages,
            self.limits.output_bytes,
        )
        .map_err(map_output_error)?;
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
            let mut resolver = SessionFontResolver {
                fonts: &self.html_fonts,
            };
            let html_options = tex_out::html::HtmlOptions {
                max_html_bytes: remaining,
                max_total_asset_bytes: remaining,
                max_asset_bytes: remaining,
                ..tex_out::html::HtmlOptions::default()
            };
            Some(
                crate::html_from_committed_artifacts(
                    &run.committed_artifacts,
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
        Ok(CompileAttemptResult::Complete(output))
    }

    pub fn clear_distribution_cache(&mut self) {
        self.resolved_files.clear();
        self.resolved_paths.clear();
        self.cached_bytes = 0;
        self.awaiting = None;
    }

    #[must_use]
    pub const fn attempts(&self) -> u32 {
        self.attempts
    }

    #[must_use]
    pub fn resolved_file_count(&self) -> usize {
        self.resolved_files.len()
    }

    #[must_use]
    pub const fn cached_file_bytes(&self) -> usize {
        self.cached_bytes
    }
}

struct SessionFontResolver<'a> {
    fonts: &'a BTreeMap<(String, String), SessionWebFont>,
}

impl HtmlFontResolver for SessionFontResolver<'_> {
    fn resolve(&mut self, font: &tex_out::FontResource) -> Result<WebFont, String> {
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

fn map_output_error(error: MemoryOutputCollectionError) -> CompileError {
    match error {
        MemoryOutputCollectionError::OutputLimitExceeded {
            limit,
            required_at_least,
        } => CompileError::LimitExceeded {
            resource: "returned output bytes",
            limit,
            attempted: required_at_least,
        },
        error => CompileError::Output(error.to_string()),
    }
}

#[cfg(test)]
mod tests;
