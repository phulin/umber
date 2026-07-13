use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;
use std::sync::Arc;

use tex_lex::{InputStack, WorldInput};
use tex_state::{JobClock, Universe, World};

use crate::{
    EngineSession, MemoryOutputCollectionError, MemoryRunOutput,
    collect_final_memory_output_from_plans, prepare_run_stores,
};

mod hooks;
mod path;

use hooks::VirtualRunHooks;
pub use path::{VirtualPath, VirtualPathError};
use path::{normalize_request_name, user_path_for_key};

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
}

impl Default for SessionOptions {
    fn default() -> Self {
        Self {
            main_path: "/job/main.tex".to_owned(),
            job_name: None,
            format: None,
            clock: JobClock::DEFAULT,
            limits: SessionLimits::default(),
        }
    }
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
            Self::World(message) | Self::Output(message) => f.write_str(message),
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
        })
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
        let mut hooks =
            VirtualRunHooks::new(&self.user_files, &self.resolved_files, &self.job_name);
        let execution = EngineSession::new(&mut input, &mut stores, &mut hooks).execute();
        let (misses, fatal) = hooks.finish();

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
        let output = collect_final_memory_output_from_plans(
            &mut stores,
            &run.dvi_pages,
            self.limits.output_bytes,
        )
        .map_err(map_output_error)?;
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
