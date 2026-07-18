//! Validated native storage for generated schema-10 format images.

use std::error::Error;
use std::fmt;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use tex_state::{Universe, World};

use crate::cache::platform_cache_root;

const DIRECTORY: &str = "formats-v1";
const KEY_DOMAIN: &[u8] = b"umber.format-cache.key\0";
const KEY_SCHEMA: u32 = 1;
const ENTRY_MAGIC: [u8; 8] = *b"UMBRFCHE";
const ENTRY_SCHEMA: u32 = 1;
const ENTRY_HEADER_LEN: usize = 56;
const MAX_FORMAT_BYTES: u64 = 256 * 1024 * 1024;

/// Driver mode whose initialized state is captured by a generated format.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[repr(u8)]
pub enum FormatEngineMode {
    Tex82 = 1,
    ETex = 2,
    PdfTex = 3,
    Latex = 4,
    PdfLatex = 5,
}

/// SHA-256 identity of an immutable cache-key input.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FormatFingerprint([u8; 32]);

impl FormatFingerprint {
    #[must_use]
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Hashes a canonical source lock, closure, or build-configuration encoding.
    #[must_use]
    pub fn sha256(bytes: &[u8]) -> Self {
        Self(Sha256::digest(bytes).into())
    }

    #[must_use]
    pub const fn bytes(self) -> [u8; 32] {
        self.0
    }

    #[must_use]
    pub fn hex(self) -> String {
        hex(&self.0)
    }
}

/// Pinned TeX job clock used while generating the image.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct FormatCacheClock {
    pub time: i32,
    pub second: i32,
    pub day: i32,
    pub month: i32,
    pub year: i32,
}

/// Complete semantic preimage for one generated format-cache entry.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct FormatCacheIdentity {
    engine_mode: FormatEngineMode,
    format_schema: u32,
    format_abi_fingerprint: u64,
    lookup_configuration_fingerprint: u64,
    distribution_snapshot: FormatFingerprint,
    format_closure: FormatFingerprint,
    source_lock: FormatFingerprint,
    build_configuration: FormatFingerprint,
    job_clock: FormatCacheClock,
}

impl FormatCacheIdentity {
    /// Creates an identity pinned to the compatibility contract of this build.
    #[must_use]
    pub fn current(
        engine_mode: FormatEngineMode,
        distribution_snapshot: FormatFingerprint,
        format_closure: FormatFingerprint,
        source_lock: FormatFingerprint,
        job_clock: FormatCacheClock,
        build_configuration: FormatFingerprint,
    ) -> Self {
        Self {
            engine_mode,
            format_schema: Universe::FORMAT_SCHEMA_VERSION,
            format_abi_fingerprint: Universe::FORMAT_ABI_FINGERPRINT,
            lookup_configuration_fingerprint: Universe::FORMAT_LOOKUP_CONFIGURATION_FINGERPRINT,
            distribution_snapshot,
            format_closure,
            source_lock,
            build_configuration,
            job_clock,
        }
    }

    /// Canonical, host-independent key preimage.
    #[must_use]
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(KEY_DOMAIN.len() + 180);
        bytes.extend_from_slice(KEY_DOMAIN);
        bytes.extend_from_slice(&KEY_SCHEMA.to_le_bytes());
        bytes.push(self.engine_mode as u8);
        bytes.extend_from_slice(&[0; 3]);
        bytes.extend_from_slice(&self.format_schema.to_le_bytes());
        bytes.extend_from_slice(&self.format_abi_fingerprint.to_le_bytes());
        bytes.extend_from_slice(&self.lookup_configuration_fingerprint.to_le_bytes());
        for fingerprint in [
            self.distribution_snapshot,
            self.format_closure,
            self.source_lock,
            self.build_configuration,
        ] {
            bytes.extend_from_slice(&fingerprint.bytes());
        }
        for value in [
            self.job_clock.time,
            self.job_clock.second,
            self.job_clock.day,
            self.job_clock.month,
            self.job_clock.year,
        ] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes
    }

    /// Content-addressed key used by native and browser cache implementations.
    #[must_use]
    pub fn key(&self) -> FormatFingerprint {
        FormatFingerprint::sha256(&self.canonical_bytes())
    }
}

/// Format bytes that passed the complete schema-10 `Universe` decoder.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedFormatImage(Vec<u8>);

impl ValidatedFormatImage {
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }
}

#[derive(Debug)]
pub enum FormatCacheError {
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    InvalidFormat(String),
    FormatTooLarge(u64),
}

impl FormatCacheError {
    fn io(operation: &'static str, path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Io {
            operation,
            path: path.into(),
            source,
        }
    }
}

impl fmt::Display for FormatCacheError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io {
                operation,
                path,
                source,
            } => write!(
                f,
                "failed to {operation} format cache path {}: {source}",
                path.display()
            ),
            Self::InvalidFormat(message) => write!(f, "invalid schema-10 format image: {message}"),
            Self::FormatTooLarge(bytes) => {
                write!(
                    f,
                    "format image is {bytes} bytes; limit is {MAX_FORMAT_BYTES}"
                )
            }
        }
    }
}

impl Error for FormatCacheError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::InvalidFormat(_) | Self::FormatTooLarge(_) => None,
        }
    }
}

/// Native, content-addressed store for generated format entries.
#[derive(Clone, Debug)]
pub struct FormatCacheStore {
    root: PathBuf,
}

impl FormatCacheStore {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Discovers the platform Umber cache root without creating it.
    pub fn from_environment() -> Result<Self, FormatCacheError> {
        let root = platform_cache_root().ok_or_else(|| {
            FormatCacheError::io(
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

    /// Loads and revalidates metadata, payload identity, and the full format image.
    #[allow(
        clippy::disallowed_methods,
        reason = "this crate is the explicit native host cache I/O boundary"
    )]
    pub fn load(
        &self,
        identity: &FormatCacheIdentity,
    ) -> Result<Option<ValidatedFormatImage>, FormatCacheError> {
        let path = self.path(identity);
        let mut file = match fs::File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(FormatCacheError::io("open", path, error)),
        };
        let length = file
            .metadata()
            .map_err(|error| FormatCacheError::io("inspect", &path, error))?
            .len();
        if length > MAX_FORMAT_BYTES + 4096 {
            drop(file);
            return self.remove_invalid(&path);
        }
        let mut entry = Vec::with_capacity(length as usize);
        file.read_to_end(&mut entry)
            .map_err(|error| FormatCacheError::io("read", &path, error))?;
        drop(file);

        let Some(payload) = decode_entry(&entry, identity) else {
            return self.remove_invalid(&path);
        };
        if Universe::from_format(World::memory(), payload).is_err() {
            return self.remove_invalid(&path);
        }
        Ok(Some(ValidatedFormatImage(payload.to_vec())))
    }

    /// Validates and atomically publishes a complete entry without replacing a peer.
    #[allow(
        clippy::disallowed_methods,
        reason = "this crate is the explicit native host cache I/O boundary"
    )]
    pub fn store(
        &self,
        identity: &FormatCacheIdentity,
        format: &[u8],
    ) -> Result<(), FormatCacheError> {
        if format.len() as u64 > MAX_FORMAT_BYTES {
            return Err(FormatCacheError::FormatTooLarge(format.len() as u64));
        }
        Universe::from_format(World::memory(), format)
            .map_err(|error| FormatCacheError::InvalidFormat(error.to_string()))?;
        if self.load(identity)?.is_some() {
            return Ok(());
        }

        let directory = self.root.join(DIRECTORY);
        fs::create_dir_all(&directory)
            .map_err(|error| FormatCacheError::io("create", &directory, error))?;
        let destination = self.path(identity);
        let entry = encode_entry(identity, format);
        let mut temporary = tempfile::NamedTempFile::new_in(&directory)
            .map_err(|error| FormatCacheError::io("create temporary file in", &directory, error))?;
        temporary
            .write_all(&entry)
            .map_err(|error| FormatCacheError::io("write temporary file in", &directory, error))?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|error| FormatCacheError::io("sync temporary file in", &directory, error))?;
        loop {
            match temporary.persist_noclobber(&destination) {
                Ok(_) => return Ok(()),
                Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => {
                    temporary = error.file;
                    if self.load(identity)?.is_some() {
                        return Ok(());
                    }
                }
                Err(error) => {
                    return Err(FormatCacheError::io(
                        "rename into",
                        destination,
                        error.error,
                    ));
                }
            }
        }
    }

    fn path(&self, identity: &FormatCacheIdentity) -> PathBuf {
        self.root
            .join(DIRECTORY)
            .join(format!("sha256-{}", identity.key().hex()))
    }

    #[allow(
        clippy::disallowed_methods,
        reason = "this crate is the explicit native host cache I/O boundary"
    )]
    fn remove_invalid(
        &self,
        path: &Path,
    ) -> Result<Option<ValidatedFormatImage>, FormatCacheError> {
        match fs::remove_file(path) {
            Ok(()) => Ok(None),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(FormatCacheError::io("remove corrupt", path, error)),
        }
    }
}

fn encode_entry(identity: &FormatCacheIdentity, format: &[u8]) -> Vec<u8> {
    let metadata = identity.canonical_bytes();
    let payload_digest = Sha256::digest(format);
    let mut entry = Vec::with_capacity(ENTRY_HEADER_LEN + metadata.len() + format.len());
    entry.extend_from_slice(&ENTRY_MAGIC);
    entry.extend_from_slice(&ENTRY_SCHEMA.to_le_bytes());
    entry.extend_from_slice(&(metadata.len() as u32).to_le_bytes());
    entry.extend_from_slice(&(format.len() as u64).to_le_bytes());
    entry.extend_from_slice(&payload_digest);
    entry.extend_from_slice(&metadata);
    entry.extend_from_slice(format);
    entry
}

fn decode_entry<'a>(entry: &'a [u8], identity: &FormatCacheIdentity) -> Option<&'a [u8]> {
    if entry.len() < ENTRY_HEADER_LEN
        || entry[..8] != ENTRY_MAGIC
        || read_u32(entry, 8)? != ENTRY_SCHEMA
    {
        return None;
    }
    let metadata_len = usize::try_from(read_u32(entry, 12)?).ok()?;
    let payload_len = usize::try_from(read_u64(entry, 16)?).ok()?;
    if payload_len as u64 > MAX_FORMAT_BYTES {
        return None;
    }
    let metadata_end = ENTRY_HEADER_LEN.checked_add(metadata_len)?;
    let payload_end = metadata_end.checked_add(payload_len)?;
    if payload_end != entry.len()
        || entry[ENTRY_HEADER_LEN..metadata_end] != identity.canonical_bytes()
    {
        return None;
    }
    let payload = &entry[metadata_end..payload_end];
    (Sha256::digest(payload).as_slice() == &entry[24..56]).then_some(payload)
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        bytes.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

fn read_u64(bytes: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_le_bytes(
        bytes.get(offset..offset + 8)?.try_into().ok()?,
    ))
}

fn hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use fmt::Write as _;
        write!(output, "{byte:02x}").expect("writing to a string cannot fail");
    }
    output
}

#[cfg(test)]
mod tests;
