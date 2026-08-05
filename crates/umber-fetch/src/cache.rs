use std::env;
use std::error::Error;
use std::fmt;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

#[cfg(unix)]
#[path = "blob_store_unix.rs"]
mod native;
#[cfg(not(unix))]
#[path = "blob_store_unsupported.rs"]
mod native;

pub(crate) const BLOB_DIRECTORY: &str = "blobs-v1";
const BLOB_MAGIC: [u8; 8] = *b"UMBRBLOB";
const BLOB_SCHEMA: u32 = 1;
const HEADER_LEN: usize = 64;
const MAX_MANIFEST_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Debug)]
pub struct CacheError {
    operation: &'static str,
    path: PathBuf,
    source: io::Error,
}

impl CacheError {
    pub(crate) fn new(
        operation: &'static str,
        path: impl Into<PathBuf>,
        source: io::Error,
    ) -> Self {
        Self {
            operation,
            path: path.into(),
            source,
        }
    }

    pub(crate) fn io(operation: &'static str, path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::new(operation, path, source)
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

/// Complete verification and placement contract for one persistent blob.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedBlobSpec {
    namespace: String,
    key: String,
    max_bytes: u64,
    expected_sha256: Option<String>,
    expected_bytes: Option<u64>,
}

impl VerifiedBlobSpec {
    pub fn new(
        namespace: impl Into<String>,
        key: impl Into<String>,
        max_bytes: u64,
    ) -> Result<Self, CacheError> {
        let spec = Self {
            namespace: namespace.into(),
            key: key.into(),
            max_bytes,
            expected_sha256: None,
            expected_bytes: None,
        };
        spec.validate()?;
        Ok(spec)
    }

    pub fn content_addressed(
        namespace: impl Into<String>,
        sha256: impl Into<String>,
        bytes: u64,
        max_bytes: u64,
    ) -> Result<Self, CacheError> {
        let digest = sha256.into();
        validate_digest(&digest)
            .map_err(|source| CacheError::new("validate digest for", &digest, source))?;
        if bytes > max_bytes {
            return Err(invalid_spec("declared blob length exceeds its limit"));
        }
        let mut spec = Self::new(namespace, digest.clone(), max_bytes)?;
        spec.expected_sha256 = Some(digest);
        spec.expected_bytes = Some(bytes);
        Ok(spec)
    }

    #[must_use]
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }

    fn validate(&self) -> Result<(), CacheError> {
        if self.namespace.is_empty()
            || self.namespace.len() > u16::MAX as usize
            || self.key.is_empty()
            || self.key.len() > u16::MAX as usize
            || self.namespace.bytes().any(|byte| byte == 0)
            || self.key.bytes().any(|byte| byte == 0)
        {
            return Err(invalid_spec(
                "blob namespace and key must be bounded nonempty strings",
            ));
        }
        Ok(())
    }
}

/// One bounded, verified, atomic native persistence substrate.
#[derive(Clone, Debug)]
pub struct BlobStore {
    root: PathBuf,
}

impl BlobStore {
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

    /// Returns the canonical native path for diagnostics and corruption tests.
    #[must_use]
    pub fn entry_path(&self, spec: &VerifiedBlobSpec) -> PathBuf {
        self.root.join(BLOB_DIRECTORY).join(entry_name(spec))
    }

    pub fn load(&self, spec: &VerifiedBlobSpec) -> Result<Option<Vec<u8>>, CacheError> {
        spec.validate()?;
        let Some(authority) = self.authority(false)? else {
            let legacy = self.load_legacy(spec)?;
            if let Some(bytes) = &legacy {
                self.store(spec, bytes)?;
            }
            return Ok(legacy);
        };
        let name = entry_name(spec);
        let _lock = authority.lock(&name)?;
        if let Some(bytes) = self.load_locked(&authority, &name, spec)? {
            return Ok(Some(bytes));
        }
        let legacy = self.load_legacy(spec)?;
        if let Some(bytes) = &legacy {
            let _ = authority.publish(&name, &encode_entry(spec, bytes))?;
        }
        Ok(legacy)
    }

    pub fn store(&self, spec: &VerifiedBlobSpec, bytes: &[u8]) -> Result<(), CacheError> {
        verify_payload(spec, bytes).map_err(|source| {
            CacheError::new(
                "verify blob before storing",
                self.root.join(BLOB_DIRECTORY),
                source,
            )
        })?;
        let authority = self
            .authority(true)?
            .expect("creating the blob namespace returns an authority");
        let name = entry_name(spec);
        let _lock = authority.lock(&name)?;
        if self.load_locked(&authority, &name, spec)?.is_some() {
            return Ok(());
        }
        let entry = encode_entry(spec, bytes);
        loop {
            if authority.publish(&name, &entry)? {
                return Ok(());
            }
            if self.load_locked(&authority, &name, spec)?.is_some() {
                return Ok(());
            }
        }
    }

    /// Loads a structurally verified blob and applies caller-owned semantic validation.
    /// Invalid new entries are quarantined; valid compatibility entries are migrated.
    pub fn load_validated(
        &self,
        spec: &VerifiedBlobSpec,
        validate: impl Fn(&[u8]) -> Result<(), String>,
    ) -> Result<Option<Vec<u8>>, CacheError> {
        spec.validate()?;
        let Some(authority) = self.authority(false)? else {
            let legacy = self
                .load_legacy(spec)?
                .filter(|bytes| validate(bytes).is_ok());
            if let Some(bytes) = &legacy {
                self.store(spec, bytes)?;
            }
            return Ok(legacy);
        };
        let name = entry_name(spec);
        let _lock = authority.lock(&name)?;
        if let Some(bytes) = self.load_locked(&authority, &name, spec)? {
            if validate(&bytes).is_ok() {
                return Ok(Some(bytes));
            }
            authority.quarantine(&name)?;
        }
        let Some(bytes) = self.load_legacy(spec)? else {
            return Ok(None);
        };
        if validate(&bytes).is_err() {
            return Ok(None);
        }
        let encoded = encode_entry(spec, &bytes);
        let _ = authority.publish(&name, &encoded)?;
        Ok(Some(bytes))
    }

    /// Returns a validated blob, constructing and publishing at most once per key.
    pub fn ensure_validated<E>(
        &self,
        spec: &VerifiedBlobSpec,
        validate: impl Fn(&[u8]) -> Result<(), String>,
        construct: impl FnOnce() -> Result<Vec<u8>, E>,
    ) -> Result<Vec<u8>, E>
    where
        E: From<CacheError>,
    {
        spec.validate().map_err(E::from)?;
        let authority = self
            .authority(true)
            .map_err(E::from)?
            .expect("creating the blob namespace returns an authority");
        let name = entry_name(spec);
        let _lock = authority.lock(&name).map_err(E::from)?;
        if let Some(bytes) = self.load_locked(&authority, &name, spec).map_err(E::from)? {
            if validate(&bytes).is_ok() {
                return Ok(bytes);
            }
            authority.quarantine(&name).map_err(E::from)?;
        }
        if let Some(bytes) = self.load_legacy(spec).map_err(E::from)?
            && validate(&bytes).is_ok()
        {
            let encoded = encode_entry(spec, &bytes);
            let _ = authority.publish(&name, &encoded).map_err(E::from)?;
            return Ok(bytes);
        }
        let bytes = construct()?;
        verify_payload(spec, &bytes)
            .map_err(|source| {
                CacheError::new("verify constructed blob for", authority.path(&name), source)
            })
            .map_err(E::from)?;
        validate(&bytes)
            .map_err(|message| {
                CacheError::new(
                    "validate constructed blob for",
                    authority.path(&name),
                    io::Error::new(io::ErrorKind::InvalidData, message),
                )
            })
            .map_err(E::from)?;
        let encoded = encode_entry(spec, &bytes);
        loop {
            if authority.publish(&name, &encoded).map_err(E::from)? {
                return Ok(bytes);
            }
            if let Some(winner) = self.load_locked(&authority, &name, spec).map_err(E::from)? {
                if validate(&winner).is_ok() {
                    return Ok(winner);
                }
                authority.quarantine(&name).map_err(E::from)?;
            }
        }
    }

    pub fn load_object(
        &self,
        digest: &str,
        expected_bytes: u64,
    ) -> Result<Option<Vec<u8>>, CacheError> {
        self.load(&VerifiedBlobSpec::content_addressed(
            "objects",
            digest,
            expected_bytes,
            expected_bytes,
        )?)
    }

    pub fn store_object(
        &self,
        digest: &str,
        expected_bytes: u64,
        bytes: &[u8],
    ) -> Result<(), CacheError> {
        self.store(
            &VerifiedBlobSpec::content_addressed(
                "objects",
                digest,
                expected_bytes,
                expected_bytes,
            )?,
            bytes,
        )
    }

    pub fn load_manifest(&self, digest: &str) -> Result<Option<Vec<u8>>, CacheError> {
        self.load(
            &VerifiedBlobSpec::content_addressed("manifests", digest, 0, MAX_MANIFEST_BYTES)?
                .without_expected_length(),
        )
    }

    pub fn store_manifest(&self, digest: &str, bytes: &[u8]) -> Result<(), CacheError> {
        self.store(
            &VerifiedBlobSpec::content_addressed(
                "manifests",
                digest,
                bytes.len() as u64,
                MAX_MANIFEST_BYTES,
            )?,
            bytes,
        )
    }

    fn authority(&self, create: bool) -> Result<Option<native::Authority>, CacheError> {
        match native::Authority::open(&self.root, create) {
            Ok(authority) => Ok(Some(authority)),
            Err(error) if !create && error.source.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn load_locked(
        &self,
        authority: &native::Authority,
        name: &str,
        spec: &VerifiedBlobSpec,
    ) -> Result<Option<Vec<u8>>, CacheError> {
        let path = authority.path(name);
        let Some(mut file) = authority.open_entry(name)? else {
            return Ok(None);
        };
        let length = file
            .metadata()
            .map_err(|error| CacheError::new("inspect", &path, error))?
            .len();
        if length
            > spec
                .max_bytes
                .saturating_add(HEADER_LEN as u64 + 2 * u16::MAX as u64)
        {
            authority.quarantine(name)?;
            return Ok(None);
        }
        let mut entry = Vec::with_capacity(length as usize);
        file.read_to_end(&mut entry)
            .map_err(|error| CacheError::new("read", &path, error))?;
        let Some(bytes) = decode_entry(spec, &entry) else {
            authority.quarantine(name)?;
            return Ok(None);
        };
        Ok(Some(bytes.to_vec()))
    }

    #[allow(
        clippy::disallowed_methods,
        reason = "compatibility readers preserve the previous cache layout"
    )]
    fn load_legacy(&self, spec: &VerifiedBlobSpec) -> Result<Option<Vec<u8>>, CacheError> {
        let Some(path) = legacy_path(&self.root, spec) else {
            return Ok(None);
        };
        let mut file = match open_legacy(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(CacheError::new("read legacy", path, error)),
        };
        if file
            .metadata()
            .map_err(|error| CacheError::new("inspect legacy", &path, error))?
            .len()
            > spec.max_bytes
        {
            return Ok(None);
        }
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .map_err(|error| CacheError::new("read legacy", &path, error))?;
        if verify_payload(spec, &bytes).is_err() {
            return Ok(None);
        }
        Ok(Some(bytes))
    }
}

#[cfg(unix)]
fn open_legacy(path: &Path) -> io::Result<fs::File> {
    use rustix::fs::{Mode, OFlags};

    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "legacy path has no parent"))?;
    let parent_type = fs::symlink_metadata(parent)?.file_type();
    if parent_type.is_symlink() || !parent_type.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "legacy cache namespace is not an owned directory",
        ));
    }
    rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
    )
    .map(fs::File::from)
    .map_err(io::Error::from)
}

#[cfg(not(unix))]
fn open_legacy(path: &Path) -> io::Result<fs::File> {
    fs::File::open(path)
}

/// Compatibility name retained while callers migrate to the general store.
pub type ObjectCache = BlobStore;

impl VerifiedBlobSpec {
    fn without_expected_length(mut self) -> Self {
        self.expected_bytes = None;
        self
    }
}

fn encode_entry(spec: &VerifiedBlobSpec, bytes: &[u8]) -> Vec<u8> {
    let namespace = spec.namespace.as_bytes();
    let key = spec.key.as_bytes();
    let digest: [u8; 32] = Sha256::digest(bytes).into();
    let mut entry = Vec::with_capacity(HEADER_LEN + namespace.len() + key.len() + bytes.len());
    entry.extend_from_slice(&BLOB_MAGIC);
    entry.extend_from_slice(&BLOB_SCHEMA.to_le_bytes());
    entry.extend_from_slice(&(namespace.len() as u16).to_le_bytes());
    entry.extend_from_slice(&(key.len() as u16).to_le_bytes());
    entry.extend_from_slice(&0_u32.to_le_bytes());
    entry.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
    entry.extend_from_slice(&digest);
    entry.extend_from_slice(&0_u32.to_le_bytes());
    entry.extend_from_slice(namespace);
    entry.extend_from_slice(key);
    entry.extend_from_slice(bytes);
    entry
}

fn decode_entry<'a>(spec: &VerifiedBlobSpec, entry: &'a [u8]) -> Option<&'a [u8]> {
    if entry.len() < HEADER_LEN
        || entry[..8] != BLOB_MAGIC
        || u32::from_le_bytes(entry[8..12].try_into().ok()?) != BLOB_SCHEMA
    {
        return None;
    }
    let namespace_len = u16::from_le_bytes(entry[12..14].try_into().ok()?) as usize;
    let key_len = u16::from_le_bytes(entry[14..16].try_into().ok()?) as usize;
    let payload_len = u64::from_le_bytes(entry[20..28].try_into().ok()?) as usize;
    let metadata_end = HEADER_LEN
        .checked_add(namespace_len)?
        .checked_add(key_len)?;
    let payload_end = metadata_end.checked_add(payload_len)?;
    if payload_end != entry.len()
        || &entry[HEADER_LEN..HEADER_LEN + namespace_len] != spec.namespace.as_bytes()
        || &entry[HEADER_LEN + namespace_len..metadata_end] != spec.key.as_bytes()
    {
        return None;
    }
    let payload = &entry[metadata_end..];
    let digest: [u8; 32] = Sha256::digest(payload).into();
    if digest.as_slice() != &entry[28..60] || verify_payload(spec, payload).is_err() {
        return None;
    }
    Some(payload)
}

fn verify_payload(spec: &VerifiedBlobSpec, bytes: &[u8]) -> io::Result<()> {
    if bytes.len() as u64 > spec.max_bytes
        || spec
            .expected_bytes
            .is_some_and(|expected| expected != bytes.len() as u64)
        || spec
            .expected_sha256
            .as_ref()
            .is_some_and(|expected| hex_digest(bytes) != *expected)
    {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "blob digest, length, or bound mismatch",
        ))
    } else {
        Ok(())
    }
}

fn entry_name(spec: &VerifiedBlobSpec) -> String {
    let mut digest = Sha256::new();
    digest.update(b"umber.blob-store.key\0");
    digest.update(spec.namespace.as_bytes());
    digest.update([0]);
    digest.update(spec.key.as_bytes());
    format!("sha256-{}", hex_bytes(&digest.finalize()))
}

fn legacy_path(root: &Path, spec: &VerifiedBlobSpec) -> Option<PathBuf> {
    match spec.namespace.as_str() {
        "objects" | "manifests" => Some(
            root.join(&spec.namespace)
                .join(format!("sha256-{}", spec.key)),
        ),
        "formats-v2" => Some(root.join("formats-v2").join(&spec.key)),
        _ => None,
    }
}

pub(crate) fn hex_digest(bytes: &[u8]) -> String {
    hex_bytes(&Sha256::digest(bytes))
}

fn hex_bytes(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use fmt::Write as _;
        write!(output, "{byte:02x}").expect("writing to a string cannot fail");
    }
    output
}

fn validate_digest(digest: &str) -> io::Result<()> {
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

fn invalid_spec(message: &'static str) -> CacheError {
    CacheError::new(
        "validate blob specification for",
        BLOB_DIRECTORY,
        io::Error::new(io::ErrorKind::InvalidInput, message),
    )
}

pub(crate) fn authority_error(path: &Path, message: &str) -> CacheError {
    CacheError::new(
        "validate authority for",
        path,
        io::Error::new(io::ErrorKind::InvalidData, message),
    )
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
