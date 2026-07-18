use std::env;
use std::error::Error;
use std::fmt;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

const OBJECTS: &str = "objects";
const MANIFESTS: &str = "manifests";
const MAX_MANIFEST_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Debug)]
pub struct CacheError {
    operation: &'static str,
    path: PathBuf,
    source: io::Error,
}

impl CacheError {
    fn new(operation: &'static str, path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self {
            operation,
            path: path.into(),
            source,
        }
    }
}

impl fmt::Display for CacheError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "failed to {} cache path {}: {}",
            self.operation,
            self.path.display(),
            self.source
        )
    }
}

impl Error for CacheError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.source)
    }
}

#[derive(Clone, Debug)]
pub struct ObjectCache {
    root: PathBuf,
}

impl ObjectCache {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Discovers the platform cache root without creating it.
    pub fn from_environment() -> Result<Self, CacheError> {
        let root = platform_cache_root().ok_or_else(|| {
            CacheError::new(
                "discover",
                "umber",
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "no platform cache directory is set",
                ),
            )
        })?;
        Ok(Self::new(root.join("umber")))
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn load_object(
        &self,
        digest: &str,
        expected_bytes: u64,
    ) -> Result<Option<Vec<u8>>, CacheError> {
        self.load_verified(OBJECTS, digest, Some(expected_bytes), expected_bytes)
    }

    pub fn store_object(
        &self,
        digest: &str,
        expected_bytes: u64,
        bytes: &[u8],
    ) -> Result<(), CacheError> {
        if !matches_blob(bytes, digest, Some(expected_bytes)) {
            return Err(CacheError::new(
                "verify object before storing",
                self.path(OBJECTS, digest),
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "object digest or length mismatch",
                ),
            ));
        }
        self.store_verified(OBJECTS, digest, bytes)
    }

    pub fn load_manifest(&self, digest: &str) -> Result<Option<Vec<u8>>, CacheError> {
        self.load_verified(MANIFESTS, digest, None, MAX_MANIFEST_BYTES)
    }

    pub fn store_manifest(&self, digest: &str, bytes: &[u8]) -> Result<(), CacheError> {
        if bytes.len() as u64 > MAX_MANIFEST_BYTES || !matches_blob(bytes, digest, None) {
            return Err(CacheError::new(
                "verify manifest before storing",
                self.path(MANIFESTS, digest),
                io::Error::new(io::ErrorKind::InvalidData, "manifest digest mismatch"),
            ));
        }
        self.store_verified(MANIFESTS, digest, bytes)
    }

    #[allow(
        clippy::disallowed_methods,
        reason = "this crate is the explicit native host cache I/O boundary"
    )]
    fn load_verified(
        &self,
        namespace: &str,
        digest: &str,
        expected_bytes: Option<u64>,
        max_bytes: u64,
    ) -> Result<Option<Vec<u8>>, CacheError> {
        validate_digest(digest).map_err(|source| {
            CacheError::new("validate digest for", self.path(namespace, digest), source)
        })?;
        let path = self.path(namespace, digest);
        let mut file = match fs::File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(CacheError::new("read", path, error)),
        };
        if file
            .metadata()
            .map_err(|error| CacheError::new("inspect", &path, error))?
            .len()
            > max_bytes
        {
            drop(file);
            return remove_invalid(&path);
        }
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .map_err(|error| CacheError::new("read", &path, error))?;
        if matches_blob(&bytes, digest, expected_bytes) {
            return Ok(Some(bytes));
        }
        remove_invalid(&path)
    }

    fn store_verified(
        &self,
        namespace: &str,
        digest: &str,
        bytes: &[u8],
    ) -> Result<(), CacheError> {
        validate_digest(digest).map_err(|source| {
            CacheError::new("validate digest for", self.path(namespace, digest), source)
        })?;
        let directory = self.root.join(namespace);
        fs::create_dir_all(&directory)
            .map_err(|error| CacheError::new("create", &directory, error))?;
        let destination = self.path(namespace, digest);
        let mut temporary = tempfile::NamedTempFile::new_in(&directory)
            .map_err(|error| CacheError::new("create temporary file in", &directory, error))?;
        temporary
            .write_all(bytes)
            .map_err(|error| CacheError::new("write temporary file in", &directory, error))?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|error| CacheError::new("sync temporary file in", &directory, error))?;
        match temporary.persist_noclobber(&destination) {
            Ok(_) => Ok(()),
            Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
            Err(error) => Err(CacheError::new("rename into", destination, error.error)),
        }
    }

    fn path(&self, namespace: &str, digest: &str) -> PathBuf {
        self.root.join(namespace).join(format!("sha256-{digest}"))
    }
}

fn remove_invalid(path: &Path) -> Result<Option<Vec<u8>>, CacheError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(None),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(CacheError::new("remove corrupt", path, error)),
    }
}

fn matches_blob(bytes: &[u8], digest: &str, expected_bytes: Option<u64>) -> bool {
    expected_bytes.is_none_or(|expected| bytes.len() as u64 == expected)
        && hex_digest(bytes) == digest
}

pub(crate) fn hex_digest(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(64);
    for byte in digest {
        use fmt::Write as _;
        write!(output, "{byte:02x}").expect("writing to a string cannot fail");
    }
    output
}

fn validate_digest(digest: &str) -> Result<(), io::Error> {
    if digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "digest must be 64 lowercase hexadecimal characters",
        ))
    }
}

fn nonempty_env(name: &str) -> Option<PathBuf> {
    env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

pub(crate) fn platform_cache_root() -> Option<PathBuf> {
    if let Some(path) = nonempty_env("XDG_CACHE_HOME") {
        return Some(path);
    }
    #[cfg(target_os = "macos")]
    {
        nonempty_env("HOME").map(|home| home.join("Library/Caches"))
    }
    #[cfg(target_os = "windows")]
    {
        nonempty_env("LOCALAPPDATA")
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        nonempty_env("HOME").map(|home| home.join(".cache"))
    }
}
