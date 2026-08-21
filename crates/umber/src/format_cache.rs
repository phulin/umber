//! TeX-specific identity and validation policy for persistent generated formats.

use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use tex_state::{
    DetachedFormatImage, FORMAT_ABI_FINGERPRINT, FORMAT_LOOKUP_CONFIGURATION_FINGERPRINT,
    FORMAT_SCHEMA_VERSION,
};
use umber_fetch::{BlobStore, CacheError, VerifiedBlobSpec};

const DIRECTORY: &str = "formats-v2";
const KEY_DOMAIN: &[u8] = b"umber.format-cache.key\0";
const KEY_SCHEMA: u32 = 2;
const ENTRY_MAGIC: [u8; 8] = *b"UMBRFCHE";
const ENTRY_SCHEMA: u32 = 1;
const ENTRY_HEADER_LEN: usize = 56;
const MAX_FORMAT_BYTES: u64 = 256 * 1024 * 1024;
const COMPOUND_ENTRY_SCHEMA: u32 = 2;
const COMPOUND_HEADER_LEN: usize = 96;
const MAX_OPAQUE_EVIDENCE_BYTES: u64 = 64 * 1024 * 1024;

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
    entry_kind: FormatCacheEntryKind,
    engine_mode: FormatEngineMode,
    format_schema: u32,
    format_abi_fingerprint: u64,
    lookup_configuration_fingerprint: u64,
    distribution_snapshot: FormatFingerprint,
    format_closure: FormatFingerprint,
    source_lock: FormatFingerprint,
    build_configuration: FormatFingerprint,
    semantic_contract: FormatFingerprint,
    producer_contract: FormatFingerprint,
    resource_closure: FormatFingerprint,
    generation_guards: FormatFingerprint,
    job_clock: FormatCacheClock,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[repr(u8)]
enum FormatCacheEntryKind {
    ImageOnly = 1,
    CompoundEvidence = 2,
}

/// Complete generic-fixture inputs grouped for construction of a cache identity.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct FormatFixtureIdentity {
    pub engine_mode: FormatEngineMode,
    pub distribution_snapshot: FormatFingerprint,
    pub format_closure: FormatFingerprint,
    pub source_lock: FormatFingerprint,
    pub job_clock: FormatCacheClock,
    pub build_configuration: FormatFingerprint,
    pub semantic_contract: FormatFingerprint,
    pub producer_contract: FormatFingerprint,
    pub resource_closure: FormatFingerprint,
    pub generation_guards: FormatFingerprint,
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
            entry_kind: FormatCacheEntryKind::ImageOnly,
            engine_mode,
            format_schema: FORMAT_SCHEMA_VERSION,
            format_abi_fingerprint: FORMAT_ABI_FINGERPRINT,
            lookup_configuration_fingerprint: FORMAT_LOOKUP_CONFIGURATION_FINGERPRINT,
            distribution_snapshot,
            format_closure,
            source_lock,
            build_configuration,
            semantic_contract: FormatFingerprint::sha256(b"legacy-format-cache-cli-v1"),
            producer_contract: FormatFingerprint::sha256(b"legacy-external-producer-v1"),
            resource_closure: format_closure,
            generation_guards: FormatFingerprint::sha256(b"legacy-external-guards-v1"),
            job_clock,
        }
    }

    /// Creates an identity with the complete generic fixture producer contract.
    #[must_use]
    pub fn fixture(fixture: FormatFixtureIdentity) -> Self {
        Self {
            entry_kind: FormatCacheEntryKind::CompoundEvidence,
            engine_mode: fixture.engine_mode,
            format_schema: FORMAT_SCHEMA_VERSION,
            format_abi_fingerprint: FORMAT_ABI_FINGERPRINT,
            lookup_configuration_fingerprint: FORMAT_LOOKUP_CONFIGURATION_FINGERPRINT,
            distribution_snapshot: fixture.distribution_snapshot,
            format_closure: fixture.format_closure,
            source_lock: fixture.source_lock,
            build_configuration: fixture.build_configuration,
            semantic_contract: fixture.semantic_contract,
            producer_contract: fixture.producer_contract,
            resource_closure: fixture.resource_closure,
            generation_guards: fixture.generation_guards,
            job_clock: fixture.job_clock,
        }
    }

    /// Canonical, host-independent key preimage.
    #[must_use]
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(KEY_DOMAIN.len() + 320);
        bytes.extend_from_slice(KEY_DOMAIN);
        bytes.extend_from_slice(&KEY_SCHEMA.to_le_bytes());
        bytes.push(self.entry_kind as u8);
        bytes.push(self.engine_mode as u8);
        bytes.extend_from_slice(&[0; 2]);
        bytes.extend_from_slice(&self.format_schema.to_le_bytes());
        bytes.extend_from_slice(&self.format_abi_fingerprint.to_le_bytes());
        bytes.extend_from_slice(&self.lookup_configuration_fingerprint.to_le_bytes());
        for fingerprint in [
            self.distribution_snapshot,
            self.format_closure,
            self.source_lock,
            self.build_configuration,
            self.semantic_contract,
            self.producer_contract,
            self.resource_closure,
            self.generation_guards,
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

/// A reusable format image that passed the complete detached decoder.
#[derive(Debug)]
pub struct ValidatedFormatImage(DetachedFormatImage);

impl ValidatedFormatImage {
    fn try_from_bytes(bytes: Vec<u8>) -> Result<Self, FormatCacheError> {
        DetachedFormatImage::try_from_bytes(bytes)
            .map(Self)
            .map_err(|error| FormatCacheError::InvalidFormat(error.to_string()))
    }

    #[must_use]
    pub const fn detached(&self) -> &DetachedFormatImage {
        &self.0
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }

    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.0.into_bytes()
    }
}

impl Clone for ValidatedFormatImage {
    fn clone(&self) -> Self {
        Self::try_from_bytes(self.as_bytes().to_vec())
            .expect("a validated detached format remains valid when copied")
    }
}

impl PartialEq for ValidatedFormatImage {
    fn eq(&self, other: &Self) -> bool {
        self.as_bytes() == other.as_bytes()
    }
}

impl Eq for ValidatedFormatImage {}

/// One atomically cached image plus caller-validated opaque evidence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedFormatEntry {
    image: ValidatedFormatImage,
    evidence: Vec<u8>,
}

impl ValidatedFormatEntry {
    #[must_use]
    pub fn image(&self) -> &ValidatedFormatImage {
        &self.image
    }
    #[must_use]
    pub fn evidence(&self) -> &[u8] {
        &self.evidence
    }
}

#[derive(Debug)]
pub enum FormatCacheError {
    InvalidFormat(String),
    FormatTooLarge(u64),
    WrongEntryKind,
    Storage(CacheError),
}

impl fmt::Display for FormatCacheError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFormat(message) => write!(f, "invalid schema-11 format image: {message}"),
            Self::FormatTooLarge(bytes) => {
                write!(
                    f,
                    "format image is {bytes} bytes; limit is {MAX_FORMAT_BYTES}"
                )
            }
            Self::WrongEntryKind => {
                write!(f, "format cache API does not match identity entry kind")
            }
            Self::Storage(error) => write!(f, "format cache storage failed: {error}"),
        }
    }
}

impl Error for FormatCacheError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Storage(error) => Some(error),
            Self::InvalidFormat(_) | Self::FormatTooLarge(_) | Self::WrongEntryKind => None,
        }
    }
}

impl From<CacheError> for FormatCacheError {
    fn from(error: CacheError) -> Self {
        Self::Storage(error)
    }
}

/// Native, content-addressed store for generated format entries.
#[derive(Clone, Debug)]
pub struct FormatCacheStore {
    root: PathBuf,
    blobs: BlobStore,
}

impl FormatCacheStore {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            blobs: BlobStore::new(&root),
            root,
        }
    }

    /// Discovers the platform Umber cache root without creating it.
    pub fn from_environment() -> Result<Self, FormatCacheError> {
        let blobs = BlobStore::from_environment()?;
        Ok(Self::new(blobs.root()))
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Loads and revalidates metadata, payload identity, and the full format image.
    pub fn load(
        &self,
        identity: &FormatCacheIdentity,
    ) -> Result<Option<ValidatedFormatImage>, FormatCacheError> {
        if identity.entry_kind != FormatCacheEntryKind::ImageOnly {
            return Err(FormatCacheError::WrongEntryKind);
        }
        let spec = self.spec(identity, MAX_FORMAT_BYTES + 4096)?;
        let entry = self
            .blobs
            .load_validated(&spec, |entry| validate_image_entry(entry, identity))?;
        Ok(entry.and_then(|entry| {
            decode_entry(&entry, identity)
                .and_then(|bytes| ValidatedFormatImage::try_from_bytes(bytes.to_vec()).ok())
        }))
    }

    /// Validates and atomically publishes a complete entry without replacing a peer.
    pub fn store(
        &self,
        identity: &FormatCacheIdentity,
        format: &[u8],
    ) -> Result<(), FormatCacheError> {
        if identity.entry_kind != FormatCacheEntryKind::ImageOnly {
            return Err(FormatCacheError::WrongEntryKind);
        }
        if format.len() as u64 > MAX_FORMAT_BYTES {
            return Err(FormatCacheError::FormatTooLarge(format.len() as u64));
        }
        ValidatedFormatImage::try_from_bytes(format.to_vec())?;
        let entry = encode_entry(identity, format);
        self.blobs.ensure_validated::<FormatCacheError>(
            &self.spec(identity, MAX_FORMAT_BYTES + 4096)?,
            |entry| validate_image_entry(entry, identity),
            || Ok(entry),
        )?;
        Ok(())
    }

    /// Loads one compound entry while holding the per-key lock through opaque validation.
    pub fn load_entry(
        &self,
        identity: &FormatCacheIdentity,
        validate_evidence: impl Fn(&[u8]) -> Result<(), String>,
    ) -> Result<Option<ValidatedFormatEntry>, FormatCacheError> {
        if identity.entry_kind != FormatCacheEntryKind::CompoundEvidence {
            return Err(FormatCacheError::WrongEntryKind);
        }
        let spec = self.spec(identity, compound_limit())?;
        let bytes = self.blobs.load_validated(&spec, |bytes| {
            validate_encoded_compound(bytes, identity, &validate_evidence)
        })?;
        Ok(bytes.and_then(|bytes| decoded_compound(&bytes, identity)))
    }

    /// Validates and atomically publishes image and opaque evidence as one entry.
    pub fn store_entry(
        &self,
        identity: &FormatCacheIdentity,
        image: &[u8],
        evidence: &[u8],
        validate_evidence: impl Fn(&[u8]) -> Result<(), String>,
    ) -> Result<(), FormatCacheError> {
        if identity.entry_kind != FormatCacheEntryKind::CompoundEvidence {
            return Err(FormatCacheError::WrongEntryKind);
        }
        if image.len() as u64 > MAX_FORMAT_BYTES {
            return Err(FormatCacheError::FormatTooLarge(image.len() as u64));
        }
        if evidence.len() as u64 > MAX_OPAQUE_EVIDENCE_BYTES {
            return Err(FormatCacheError::InvalidFormat(
                "opaque evidence exceeds cache limit".into(),
            ));
        }
        ValidatedFormatImage::try_from_bytes(image.to_vec())?;
        validate_evidence(evidence).map_err(FormatCacheError::InvalidFormat)?;
        let entry = encode_compound_entry(identity, image, evidence);
        self.blobs.ensure_validated::<FormatCacheError>(
            &self.spec(identity, compound_limit())?,
            |entry| validate_encoded_compound(entry, identity, &validate_evidence),
            || Ok(entry),
        )?;
        Ok(())
    }

    /// Returns a validated compound entry, constructing at most once while the key is locked.
    ///
    /// Decoder-invalid evidence is quarantined and regenerated without releasing the lock, so
    /// another process cannot replace the inspected pathname between validation and retry.
    pub fn ensure_entry<E>(
        &self,
        identity: &FormatCacheIdentity,
        validate_evidence: impl Fn(&[u8]) -> Result<(), String>,
        construct: impl FnOnce() -> Result<(Vec<u8>, Vec<u8>), E>,
    ) -> Result<ValidatedFormatEntry, E>
    where
        E: From<FormatCacheError> + From<CacheError>,
    {
        if identity.entry_kind != FormatCacheEntryKind::CompoundEvidence {
            return Err(E::from(FormatCacheError::WrongEntryKind));
        }
        let spec = self.spec(identity, compound_limit()).map_err(E::from)?;
        let bytes = self.blobs.ensure_validated::<E>(
            &spec,
            |bytes| validate_encoded_compound(bytes, identity, &validate_evidence),
            || {
                let (image, evidence) = construct()?;
                validate_compound_payload(&image, &evidence, &validate_evidence)
                    .map_err(E::from)?;
                Ok(encode_compound_entry(identity, &image, &evidence))
            },
        )?;
        decoded_compound(&bytes, identity).ok_or_else(|| {
            E::from(FormatCacheError::InvalidFormat(
                "validated compound entry could not be decoded".into(),
            ))
        })
    }

    fn name(&self, identity: &FormatCacheIdentity) -> String {
        format!("sha256-{}", identity.key().hex())
    }

    #[cfg(test)]
    fn path(&self, identity: &FormatCacheIdentity) -> PathBuf {
        self.blobs.entry_path(
            &self
                .spec(identity, compound_limit())
                .expect("fixed format blob specification"),
        )
    }

    fn spec(
        &self,
        identity: &FormatCacheIdentity,
        max_bytes: u64,
    ) -> Result<VerifiedBlobSpec, FormatCacheError> {
        VerifiedBlobSpec::new(DIRECTORY, self.name(identity), max_bytes).map_err(Into::into)
    }
}

fn compound_limit() -> u64 {
    MAX_FORMAT_BYTES + MAX_OPAQUE_EVIDENCE_BYTES + 4096
}

fn validate_image_entry(entry: &[u8], identity: &FormatCacheIdentity) -> Result<(), String> {
    let payload = decode_entry(entry, identity).ok_or("invalid format cache envelope")?;
    DetachedFormatImage::try_from_bytes(payload.to_vec())
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn validate_encoded_compound(
    bytes: &[u8],
    identity: &FormatCacheIdentity,
    validate_evidence: &impl Fn(&[u8]) -> Result<(), String>,
) -> Result<(), String> {
    let (image, evidence) =
        decode_compound_entry(bytes, identity).ok_or("invalid compound cache envelope")?;
    DetachedFormatImage::try_from_bytes(image.to_vec()).map_err(|error| error.to_string())?;
    validate_evidence(evidence)
}

fn decoded_compound(bytes: &[u8], identity: &FormatCacheIdentity) -> Option<ValidatedFormatEntry> {
    let (image, evidence) = decode_compound_entry(bytes, identity)?;
    Some(ValidatedFormatEntry {
        image: ValidatedFormatImage::try_from_bytes(image.to_vec()).ok()?,
        evidence: evidence.to_vec(),
    })
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

fn encode_compound_entry(identity: &FormatCacheIdentity, image: &[u8], evidence: &[u8]) -> Vec<u8> {
    let metadata = identity.canonical_bytes();
    let mut entry =
        Vec::with_capacity(COMPOUND_HEADER_LEN + metadata.len() + image.len() + evidence.len());
    entry.extend_from_slice(&ENTRY_MAGIC);
    entry.extend_from_slice(&COMPOUND_ENTRY_SCHEMA.to_le_bytes());
    entry.extend_from_slice(&(metadata.len() as u32).to_le_bytes());
    entry.extend_from_slice(&(image.len() as u64).to_le_bytes());
    entry.extend_from_slice(&(evidence.len() as u64).to_le_bytes());
    entry.extend_from_slice(&Sha256::digest(image));
    entry.extend_from_slice(&Sha256::digest(evidence));
    entry.extend_from_slice(&metadata);
    entry.extend_from_slice(image);
    entry.extend_from_slice(evidence);
    entry
}

fn validate_compound_payload(
    image: &[u8],
    evidence: &[u8],
    validate_evidence: &impl Fn(&[u8]) -> Result<(), String>,
) -> Result<(), FormatCacheError> {
    if image.len() as u64 > MAX_FORMAT_BYTES {
        return Err(FormatCacheError::FormatTooLarge(image.len() as u64));
    }
    if evidence.len() as u64 > MAX_OPAQUE_EVIDENCE_BYTES {
        return Err(FormatCacheError::InvalidFormat(
            "opaque evidence exceeds cache limit".into(),
        ));
    }
    ValidatedFormatImage::try_from_bytes(image.to_vec())?;
    validate_evidence(evidence).map_err(FormatCacheError::InvalidFormat)
}

fn decode_compound_entry<'a>(
    entry: &'a [u8],
    identity: &FormatCacheIdentity,
) -> Option<(&'a [u8], &'a [u8])> {
    if entry.len() < COMPOUND_HEADER_LEN
        || entry[..8] != ENTRY_MAGIC
        || read_u32(entry, 8)? != COMPOUND_ENTRY_SCHEMA
    {
        return None;
    }
    let metadata_len = usize::try_from(read_u32(entry, 12)?).ok()?;
    let image_len = usize::try_from(read_u64(entry, 16)?).ok()?;
    let evidence_len = usize::try_from(read_u64(entry, 24)?).ok()?;
    if image_len as u64 > MAX_FORMAT_BYTES || evidence_len as u64 > MAX_OPAQUE_EVIDENCE_BYTES {
        return None;
    }
    let metadata_end = COMPOUND_HEADER_LEN.checked_add(metadata_len)?;
    let image_end = metadata_end.checked_add(image_len)?;
    let evidence_end = image_end.checked_add(evidence_len)?;
    if evidence_end != entry.len()
        || entry[COMPOUND_HEADER_LEN..metadata_end] != identity.canonical_bytes()
    {
        return None;
    }
    let image = &entry[metadata_end..image_end];
    let evidence = &entry[image_end..evidence_end];
    if Sha256::digest(image).as_slice() != &entry[32..64]
        || Sha256::digest(evidence).as_slice() != &entry[64..96]
    {
        return None;
    }
    Some((image, evidence))
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
