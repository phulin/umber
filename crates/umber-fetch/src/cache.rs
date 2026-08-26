use std::env;
use std::error::Error;
use std::fmt;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use umber_hash::{AHash64, AHash64Hasher, HashDomain};

#[cfg(unix)]
#[path = "blob_store_unix.rs"]
mod native;
#[cfg(not(unix))]
#[path = "blob_store_unsupported.rs"]
mod native;

pub(crate) const BLOB_DIRECTORY: &str = "blobs-v2";
const BLOB_MAGIC: [u8; 8] = *b"UMBRBLOB";
const BLOB_SCHEMA: u32 = 2;
const HEADER_LEN: usize = 40;
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
    expected_ahash64: Option<String>,
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
            expected_ahash64: None,
            expected_bytes: None,
        };
        spec.validate()?;
        Ok(spec)
    }

    pub fn content_addressed(
        namespace: impl Into<String>,
        ahash64: impl Into<String>,
        bytes: u64,
        max_bytes: u64,
    ) -> Result<Self, CacheError> {
        let digest = ahash64.into();
        validate_digest(&digest)
            .map_err(|source| CacheError::new("validate digest for", &digest, source))?;
        if bytes > max_bytes {
            return Err(invalid_spec("declared blob length exceeds its limit"));
        }
        let mut spec = Self::new(namespace, digest.clone(), max_bytes)?;
        spec.expected_ahash64 = Some(digest);
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

/// Work performed by an explicitly requested complete cache audit.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CacheVerificationReport {
    pub blobs: u64,
    pub object_blobs: u64,
    pub manifest_blobs: u64,
    pub other_blobs: u64,
    pub payload_bytes: u64,
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
        self.resolve_entry(spec, false, &|_| Ok(()), || Ok::<_, CacheError>(None))
    }

    pub fn store(&self, spec: &VerifiedBlobSpec, bytes: &[u8]) -> Result<(), CacheError> {
        verify_payload(spec, bytes).map_err(|source| {
            CacheError::new(
                "verify blob before storing",
                self.root.join(BLOB_DIRECTORY),
                source,
            )
        })?;
        self.resolve_entry(spec, true, &|_| Ok(()), || {
            Ok::<_, CacheError>(Some(bytes.to_vec()))
        })?;
        Ok(())
    }

    /// Loads a structurally verified blob and applies caller-owned semantic validation.
    /// Invalid new entries are quarantined; valid compatibility entries are migrated.
    pub fn load_validated(
        &self,
        spec: &VerifiedBlobSpec,
        validate: impl Fn(&[u8]) -> Result<(), String>,
    ) -> Result<Option<Vec<u8>>, CacheError> {
        self.resolve_entry(spec, false, &validate, || Ok::<_, CacheError>(None))
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
        Ok(self
            .resolve_entry(spec, true, &validate, || construct().map(Some))?
            .expect("constructing entry resolution cannot remain absent"))
    }

    /// Runs the complete per-key entry transition while holding the sole key lock.
    ///
    /// Current entries, compatibility migration, semantic quarantine,
    /// construction, verified encoding, and no-clobber publication all pass
    /// through this state machine. Read-only misses avoid creating cache paths.
    fn resolve_entry<E>(
        &self,
        spec: &VerifiedBlobSpec,
        create: bool,
        validate: &impl Fn(&[u8]) -> Result<(), String>,
        construct: impl FnOnce() -> Result<Option<Vec<u8>>, E>,
    ) -> Result<Option<Vec<u8>>, E>
    where
        E: From<CacheError>,
    {
        spec.validate().map_err(E::from)?;
        let mut staged = None;
        let authority = match self.authority(create).map_err(E::from)? {
            Some(authority) => authority,
            None => {
                staged = self
                    .load_legacy(spec)
                    .map_err(E::from)?
                    .filter(|bytes| validate(bytes).is_ok());
                if staged.is_none() {
                    return Ok(None);
                }
                self.authority(true)
                    .map_err(E::from)?
                    .expect("creating the blob namespace returns an authority")
            }
        };
        let name = entry_name(spec);
        let _lock = authority.lock(&name).map_err(E::from)?;
        let mut construct = Some(construct);

        loop {
            if let Some(bytes) = self.load_locked(&authority, &name, spec).map_err(E::from)? {
                if validate(&bytes).is_ok() {
                    return Ok(Some(bytes));
                }
                authority.quarantine(&name).map_err(E::from)?;
            }

            let legacy = if staged.is_none() {
                self.load_legacy(spec)
                    .map_err(E::from)?
                    .filter(|bytes| validate(bytes).is_ok())
            } else {
                None
            };
            let candidate = match staged.take().or(legacy) {
                Some(bytes) => bytes,
                None if !create => return Ok(None),
                None => match construct
                    .take()
                    .expect("entry construction is attempted at most once")(
                )? {
                    Some(bytes) => bytes,
                    None => return Ok(None),
                },
            };
            verify_payload(spec, &candidate)
                .map_err(|source| {
                    CacheError::new("verify candidate blob for", authority.path(&name), source)
                })
                .map_err(E::from)?;
            validate(&candidate)
                .map_err(|message| {
                    CacheError::new(
                        "validate candidate blob for",
                        authority.path(&name),
                        io::Error::new(io::ErrorKind::InvalidData, message),
                    )
                })
                .map_err(E::from)?;
            let encoded = encode_entry(spec, &candidate);
            if authority.publish(&name, &encoded).map_err(E::from)? {
                return Ok(Some(candidate));
            }
            staged = Some(candidate);
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

    /// Authenticates every immutable entry in the current cache namespace.
    ///
    /// This is an explicit maintenance operation. Ordinary cache lookup calls
    /// [`Self::load`] for one requested key and never enumerate the namespace.
    pub fn verify_all(&self) -> Result<CacheVerificationReport, CacheError> {
        let Some(authority) = self.authority(false)? else {
            return Ok(CacheVerificationReport::default());
        };
        let mut report = CacheVerificationReport::default();
        for name in authority.entry_names()? {
            if name.starts_with('.') {
                continue;
            }
            let path = authority.path(&name);
            let Some(mut file) = authority.open_entry(&name)? else {
                return Err(CacheError::new(
                    "verify",
                    path,
                    io::Error::new(
                        io::ErrorKind::NotFound,
                        "cache entry disappeared during audit",
                    ),
                ));
            };
            let (namespace, payload_bytes) = verify_encoded_entry(&mut file, &name, &path)?;
            report.blobs = report.blobs.saturating_add(1);
            report.payload_bytes = report.payload_bytes.saturating_add(payload_bytes);
            match namespace.as_str() {
                "objects" => report.object_blobs = report.object_blobs.saturating_add(1),
                "manifests" => report.manifest_blobs = report.manifest_blobs.saturating_add(1),
                _ => report.other_blobs = report.other_blobs.saturating_add(1),
            }
        }
        Ok(report)
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
    let digest = AHash64::for_bytes(HashDomain::CacheEnvelope, bytes).to_le_bytes();
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

fn verify_encoded_entry(
    file: &mut fs::File,
    name: &str,
    path: &Path,
) -> Result<(String, u64), CacheError> {
    let encoded_bytes = file
        .metadata()
        .map_err(|error| CacheError::new("inspect", path, error))?
        .len();
    let mut header = [0_u8; HEADER_LEN];
    file.read_exact(&mut header)
        .map_err(|error| CacheError::new("read verification header from", path, error))?;
    if header[..8] != BLOB_MAGIC
        || u32::from_le_bytes(header[8..12].try_into().expect("fixed header slice")) != BLOB_SCHEMA
        || header[16..20] != [0; 4]
        || header[36..40] != [0; 4]
    {
        return Err(invalid_cache_entry(path, "invalid cache envelope header"));
    }
    let namespace_len =
        u16::from_le_bytes(header[12..14].try_into().expect("fixed header slice")) as usize;
    let key_len =
        u16::from_le_bytes(header[14..16].try_into().expect("fixed header slice")) as usize;
    let payload_len = u64::from_le_bytes(header[20..28].try_into().expect("fixed header slice"));
    let expected_encoded_bytes = (HEADER_LEN as u64)
        .checked_add(namespace_len as u64)
        .and_then(|bytes| bytes.checked_add(key_len as u64))
        .and_then(|bytes| bytes.checked_add(payload_len))
        .ok_or_else(|| invalid_cache_entry(path, "cache envelope length overflows"))?;
    if expected_encoded_bytes != encoded_bytes {
        return Err(invalid_cache_entry(
            path,
            "cache envelope length does not match the file",
        ));
    }
    let mut namespace = vec![0; namespace_len];
    let mut key = vec![0; key_len];
    file.read_exact(&mut namespace)
        .and_then(|()| file.read_exact(&mut key))
        .map_err(|error| CacheError::new("read verification identity from", path, error))?;
    let namespace = String::from_utf8(namespace)
        .map_err(|_| invalid_cache_entry(path, "cache namespace is not valid UTF-8"))?;
    let key = String::from_utf8(key)
        .map_err(|_| invalid_cache_entry(path, "cache key is not valid UTF-8"))?;
    let spec = VerifiedBlobSpec::new(namespace.clone(), key.clone(), payload_len)?;
    if entry_name(&spec) != name {
        return Err(invalid_cache_entry(
            path,
            "cache filename does not match its embedded identity",
        ));
    }
    let mut digest = AHash64Hasher::new(HashDomain::CacheEnvelope);
    let mut content = AHash64Hasher::new(HashDomain::DistributionContent);
    let mut remaining = payload_len;
    let mut buffer = [0_u8; 64 * 1024];
    while remaining != 0 {
        let length = usize::try_from(remaining.min(buffer.len() as u64))
            .expect("bounded verification read length");
        file.read_exact(&mut buffer[..length])
            .map_err(|error| CacheError::new("read verification payload from", path, error))?;
        digest.write(&buffer[..length]);
        content.write(&buffer[..length]);
        remaining -= length as u64;
    }
    let actual = digest.finish().to_le_bytes();
    if actual.as_slice() != &header[28..36] {
        return Err(invalid_cache_entry(
            path,
            "cache payload does not match its envelope digest",
        ));
    }
    if matches!(namespace.as_str(), "objects" | "manifests") {
        validate_digest(&key)
            .map_err(|_| invalid_cache_entry(path, "content-addressed cache key is invalid"))?;
        if content.finish().hex() != key {
            return Err(invalid_cache_entry(
                path,
                "cache payload does not match its content-addressed key",
            ));
        }
    }
    Ok((namespace, payload_len))
}

fn invalid_cache_entry(path: &Path, message: &'static str) -> CacheError {
    CacheError::new(
        "verify",
        path,
        io::Error::new(io::ErrorKind::InvalidData, message),
    )
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
    let digest = AHash64::for_bytes(HashDomain::CacheEnvelope, payload).to_le_bytes();
    if digest.as_slice() != &entry[28..36]
        || !payload_shape_matches(spec, payload)
        || spec
            .expected_ahash64
            .as_ref()
            .is_some_and(|expected| hex_digest(payload) != *expected)
    {
        return None;
    }
    Some(payload)
}

fn verify_payload(spec: &VerifiedBlobSpec, bytes: &[u8]) -> io::Result<()> {
    if !payload_shape_matches(spec, bytes)
        || spec
            .expected_ahash64
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

fn payload_shape_matches(spec: &VerifiedBlobSpec, bytes: &[u8]) -> bool {
    bytes.len() as u64 <= spec.max_bytes
        && spec
            .expected_bytes
            .is_none_or(|expected| expected == bytes.len() as u64)
}

fn entry_name(spec: &VerifiedBlobSpec) -> String {
    let mut digest = AHash64Hasher::new(HashDomain::CacheEnvelope);
    digest.write(b"umber.blob-store.key\0");
    digest.write(spec.namespace.as_bytes());
    digest.write([0]);
    digest.write(spec.key.as_bytes());
    format!("ahash64-v1-{}", digest.finish().hex())
}

fn legacy_path(root: &Path, spec: &VerifiedBlobSpec) -> Option<PathBuf> {
    match spec.namespace.as_str() {
        "objects" | "manifests" => Some(
            root.join(&spec.namespace)
                .join(format!("ahash64-v1-{}", spec.key)),
        ),
        "formats-v2" => Some(root.join("formats-v2").join(&spec.key)),
        _ => None,
    }
}

pub(crate) fn hex_digest(bytes: &[u8]) -> String {
    AHash64::for_bytes(HashDomain::DistributionContent, bytes).hex()
}

fn validate_digest(digest: &str) -> io::Result<()> {
    if digest.len() == 16
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "digest must be 16 lowercase hexadecimal characters",
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
