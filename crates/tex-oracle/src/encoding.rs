use std::error::Error;
use std::fmt;

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::{Manifest, SchemaVersion};

const MANIFEST_DOMAIN: &[u8] = b"umber-command-oracle-manifest\0";
const STREAM_DOMAIN: &[u8] = b"umber-command-oracle-stream\0";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EncodingError(String);

impl fmt::Display for EncodingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for EncodingError {}

impl From<serde_json::Error> for EncodingError {
    fn from(error: serde_json::Error) -> Self {
        Self(error.to_string())
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ManifestIdentity([u8; 32]);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct StreamIdentity([u8; 32]);

macro_rules! identity_impl {
    ($name:ident) => {
        impl $name {
            #[must_use]
            pub const fn from_bytes(bytes: [u8; 32]) -> Self {
                Self(bytes)
            }

            #[must_use]
            pub const fn bytes(self) -> [u8; 32] {
                self.0
            }

            #[must_use]
            pub fn hex(self) -> String {
                self.0.iter().map(|byte| format!("{byte:02x}")).collect()
            }
        }
    };
}

identity_impl!(ManifestIdentity);
identity_impl!(StreamIdentity);

impl Manifest {
    /// Canonical compact JSON. Struct field order is schema order and all maps
    /// are `BTreeMap`, so equal manifests always produce equal bytes.
    pub fn to_canonical_json(&self) -> Result<Vec<u8>, EncodingError> {
        validate_manifest(self)?;
        Ok(serde_json::to_vec(self)?)
    }

    pub fn identity(&self) -> Result<ManifestIdentity, EncodingError> {
        let bytes = self.to_canonical_json()?;
        let schema = SchemaVersion::try_from(self.schema).map_err(EncodingError)?;
        Ok(ManifestIdentity(domain_hash(
            MANIFEST_DOMAIN,
            schema,
            &bytes,
        )))
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self, EncodingError> {
        let manifest: Self = serde_json::from_slice(bytes)?;
        validate_manifest(&manifest)?;
        Ok(manifest)
    }
}

pub(crate) fn encode_line<T: Serialize>(value: &T) -> Result<Vec<u8>, EncodingError> {
    let mut bytes = serde_json::to_vec(value)?;
    bytes.push(b'\n');
    Ok(bytes)
}

pub(crate) struct StreamHasher(Sha256);

impl StreamHasher {
    pub(crate) fn new(schema: SchemaVersion) -> Self {
        let mut digest = Sha256::new();
        digest.update(STREAM_DOMAIN);
        digest.update(schema.number().to_le_bytes());
        Self(digest)
    }

    pub(crate) fn update(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }

    pub(crate) fn finish(self) -> StreamIdentity {
        StreamIdentity(self.0.finalize().into())
    }
}

fn domain_hash(domain: &[u8], schema: SchemaVersion, bytes: &[u8]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(domain);
    digest.update(schema.number().to_le_bytes());
    digest.update(bytes);
    digest.finalize().into()
}

fn validate_manifest(manifest: &Manifest) -> Result<(), EncodingError> {
    let _ = SchemaVersion::try_from(manifest.schema).map_err(EncodingError)?;
    validate_hash("web source", &manifest.engine.web_source_sha256)?;
    for hash in &manifest.engine.upstream_change_sha256 {
        validate_hash("upstream change", hash)?;
    }
    validate_hash(
        "instrumentation change",
        &manifest.engine.instrumentation_change_sha256,
    )?;
    validate_hash("distribution", &manifest.distribution_sha256)?;
    for (name, input) in &manifest.inputs {
        validate_logical_name("input", name)?;
        validate_hash("input", &input.sha256)?;
    }
    for (channel, hash) in &manifest.ordinary_output_sha256 {
        validate_logical_name("ordinary output channel", channel)?;
        validate_hash("ordinary output", hash)?;
    }
    Ok(())
}

fn validate_hash(kind: &str, hash: &str) -> Result<(), EncodingError> {
    if hash.len() == 64
        && hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(EncodingError(format!(
            "{kind} identity must be 64 lowercase hexadecimal characters"
        )))
    }
}

fn validate_logical_name(kind: &str, name: &str) -> Result<(), EncodingError> {
    if name.is_empty()
        || name.starts_with('/')
        || name.contains('\\')
        || name.split('/').any(|component| component == "..")
    {
        Err(EncodingError(format!(
            "{kind} name must be a non-empty logical name, not a host path"
        )))
    } else {
        Ok(())
    }
}
