//! Host-neutral readiness and accepted lookup history for resource prefetch.
//!
//! The distribution catalogue answers whether a logical request exists.  It
//! does not answer whether the corresponding payload has crossed the engine
//! boundary.  This module keeps those two facts separate and provides the
//! small, deterministic manifest used by native and browser hosts to seed a
//! later run.

use std::fmt;

#[cfg(test)]
mod tests;

pub const PREFETCH_MANIFEST_SCHEMA: u32 = 1;

/// Payload admission state for one canonical resource request.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Readiness {
    /// The bytes and any required typed metadata are visible to the engine.
    Ready,
    /// Immutable catalogue/VFS metadata proves the resource exists, but the
    /// payload or typed admission has not completed.
    ExistsNotReady,
    /// The current immutable or versioned namespace proves that the request
    /// is absent.
    Absent,
}

impl Readiness {
    /// Classifies a lookup when the catalogue may not have an answer yet.
    ///
    /// `None` is deliberately retained for transport/access failures: those
    /// failures must never be persisted as semantic absence.  Callers that
    /// have an authoritative negative should pass `Some(false)` and
    /// `authoritative_absent = true` (or use [`Readiness::Absent`]).
    #[must_use]
    pub const fn classify(
        exists: Option<bool>,
        payload_admitted: bool,
        authoritative_absent: bool,
    ) -> Option<Self> {
        if authoritative_absent {
            return Some(Self::Absent);
        }
        if payload_admitted {
            return Some(Self::Ready);
        }
        match exists {
            Some(true) => Some(Self::ExistsNotReady),
            Some(false) | None => None,
        }
    }

    #[must_use]
    pub const fn from_evidence(
        exists: bool,
        payload_admitted: bool,
        authoritative_absent: bool,
    ) -> Self {
        if authoritative_absent {
            Self::Absent
        } else if payload_admitted {
            Self::Ready
        } else if exists {
            Self::ExistsNotReady
        } else {
            // Preserve this infallible compatibility helper's historical
            // result for an unclassified request.  New callers should use
            // `classify`, which returns `None` instead of misclassifying a
            // transport or access error as semantic absence.
            Self::ExistsNotReady
        }
    }

    #[must_use]
    pub const fn wire_name(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::ExistsNotReady => "exists-not-ready",
            Self::Absent => "absent",
        }
    }
}

/// The strength of a lookup request in an accepted-run manifest.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum LookupRole {
    Hint,
    Probe,
    Required,
}

impl LookupRole {
    #[must_use]
    pub const fn wire_name(self) -> &'static str {
        match self {
            Self::Hint => "hint",
            Self::Probe => "probe",
            Self::Required => "required",
        }
    }

    fn from_wire_name(value: &str) -> Option<Self> {
        match value {
            "hint" => Some(Self::Hint),
            "probe" => Some(Self::Probe),
            "required" => Some(Self::Required),
            _ => None,
        }
    }
}

/// Why a negative lookup remains valid after an accepted run.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum NegativeScope {
    /// The pinned distribution root and its immutable shard graph.
    Distribution { root: String },
    /// Authored project absence, valid only for one frozen source revision.
    Project { revision: String },
    /// Generated-file absence, valid only for one output transaction state.
    Generated { transaction: String },
}

impl NegativeScope {
    fn encode(&self) -> String {
        match self {
            Self::Distribution { root } => format!("distribution:{}", escape(root)),
            Self::Project { revision } => format!("project:{}", escape(revision)),
            Self::Generated { transaction } => format!("generated:{}", escape(transaction)),
        }
    }

    fn decode(value: &str) -> Result<Self, ManifestError> {
        let (kind, value) = value
            .split_once(':')
            .ok_or_else(|| ManifestError::new("negative scope has no kind"))?;
        let value = unescape(value)?;
        if value.is_empty() || value.len() > 256 {
            return Err(ManifestError::new("negative scope identity is invalid"));
        }
        match kind {
            "distribution" => Ok(Self::Distribution { root: value }),
            "project" => Ok(Self::Project { revision: value }),
            "generated" => Ok(Self::Generated { transaction: value }),
            _ => Err(ManifestError::new("unknown negative scope")),
        }
    }
}

/// The pinned object identity selected by a successful lookup.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ResolvedIdentity {
    pub manifest_key: String,
    pub virtual_path: Option<String>,
    pub object: String,
    pub ahash64: String,
    pub bytes: u64,
}

impl ResolvedIdentity {
    pub fn new(
        manifest_key: impl Into<String>,
        virtual_path: Option<String>,
        object: impl Into<String>,
        ahash64: impl Into<String>,
        bytes: u64,
    ) -> Result<Self, ManifestError> {
        let identity = Self {
            manifest_key: manifest_key.into(),
            virtual_path,
            object: object.into(),
            ahash64: ahash64.into(),
            bytes,
        };
        identity.validate()?;
        Ok(identity)
    }

    fn validate(&self) -> Result<(), ManifestError> {
        for (name, value, limit) in [
            ("manifest key", self.manifest_key.as_str(), 4096),
            ("object", self.object.as_str(), 512),
            ("aHash64", self.ahash64.as_str(), 64),
        ] {
            validate_field(name, value, limit)?;
        }
        if let Some(path) = &self.virtual_path {
            validate_field("virtual path", path, 4096)?;
        }
        if self.ahash64.len() != 16
            || !self
                .ahash64
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        {
            return Err(ManifestError::new("resolved identity has invalid aHash64"));
        }
        Ok(())
    }
}

/// Outcome retained for one successful lookup.  Access, network, and
/// integrity failures intentionally have no variant and are never published
/// as semantic absence.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum LookupOutcome {
    Resolved(ResolvedIdentity),
    Absent(NegativeScope),
}

/// One attempted lookup, retaining the original spelling and search context
/// in addition to its canonical request identity.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct LookupRecord {
    pub original_spelling: String,
    pub request_key: String,
    pub kind: String,
    pub search_context: String,
    pub role: LookupRole,
    pub outcome: LookupOutcome,
}

impl LookupRecord {
    pub fn new(
        original_spelling: impl Into<String>,
        request_key: impl Into<String>,
        kind: impl Into<String>,
        search_context: impl Into<String>,
        role: LookupRole,
        outcome: LookupOutcome,
    ) -> Result<Self, ManifestError> {
        let record = Self {
            original_spelling: original_spelling.into(),
            request_key: request_key.into(),
            kind: kind.into(),
            search_context: search_context.into(),
            role,
            outcome,
        };
        record.validate()?;
        Ok(record)
    }

    fn validate(&self) -> Result<(), ManifestError> {
        validate_field("original spelling", &self.original_spelling, 4096)?;
        validate_field("request key", &self.request_key, 4096)?;
        validate_field("resource kind", &self.kind, 64)?;
        validate_field("search context", &self.search_context, 1024)?;
        Ok(())
    }

    fn same_lookup(&self, other: &Self) -> bool {
        self.original_spelling == other.original_spelling
            && self.request_key == other.request_key
            && self.kind == other.kind
            && self.search_context == other.search_context
            && match (&self.outcome, &other.outcome) {
                (LookupOutcome::Resolved(_), LookupOutcome::Resolved(_)) => true,
                (LookupOutcome::Absent(left), LookupOutcome::Absent(right)) => left == right,
                _ => false,
            }
    }
}

/// Source-independent identity of a predictive lookup history.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct PrefetchIdentity {
    pub engine: String,
    pub format: String,
    pub options: String,
    pub distribution: String,
    pub search_policy: String,
}

impl PrefetchIdentity {
    pub fn new(
        engine: impl Into<String>,
        format: impl Into<String>,
        options: impl Into<String>,
        distribution: impl Into<String>,
        search_policy: impl Into<String>,
    ) -> Result<Self, ManifestError> {
        let identity = Self {
            engine: engine.into(),
            format: format.into(),
            options: options.into(),
            distribution: distribution.into(),
            search_policy: search_policy.into(),
        };
        identity.validate()?;
        Ok(identity)
    }

    fn validate(&self) -> Result<(), ManifestError> {
        for (name, value, limit) in [
            ("engine", self.engine.as_str(), 128),
            ("format", self.format.as_str(), 512),
            ("options", self.options.as_str(), 4096),
            ("distribution", self.distribution.as_str(), 4096),
            ("search policy", self.search_policy.as_str(), 1024),
        ] {
            validate_field(name, value, limit)?;
        }
        Ok(())
    }

    /// Canonical key deliberately excludes source bytes and source revision.
    #[must_use]
    pub fn canonical_key(&self) -> String {
        [
            escape(&self.engine),
            escape(&self.format),
            escape(&self.options),
            escape(&self.distribution),
            escape(&self.search_policy),
        ]
        .join("|")
    }
}

/// Bounded accepted-run lookup history.  Failed attempts can be used by a
/// caller while scheduling, but only this value from an accepted run should
/// be persisted and reused as the next run's prediction.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LookupManifest {
    identity: PrefetchIdentity,
    records: Vec<LookupRecord>,
}

impl LookupManifest {
    pub fn new(identity: PrefetchIdentity) -> Self {
        Self {
            identity,
            records: Vec::new(),
        }
    }

    #[must_use]
    pub const fn identity(&self) -> &PrefetchIdentity {
        &self.identity
    }

    #[must_use]
    pub fn records(&self) -> &[LookupRecord] {
        &self.records
    }

    /// Adds an accepted lookup, promoting a duplicate role to the stronger
    /// role while retaining the first deterministic record order.
    pub fn record(&mut self, record: LookupRecord) -> Result<(), ManifestError> {
        record.validate()?;
        if let Some(existing) = self
            .records
            .iter_mut()
            .find(|item| item.same_lookup(&record))
        {
            if existing.outcome != record.outcome {
                return Err(ManifestError::new(
                    "accepted lookup manifest contains conflicting outcomes",
                ));
            }
            if record.role > existing.role {
                existing.role = record.role;
            }
            return Ok(());
        }
        self.records.push(record);
        Ok(())
    }

    /// Returns resolved identities in stable manifest order for startup
    /// scheduling.  Absence records remain available to a resolver as
    /// metadata probes, but never cause a payload fetch.
    #[must_use = "iterate the accepted resolved lookup records"]
    pub fn resolved_records(&self) -> impl Iterator<Item = &LookupRecord> {
        self.records
            .iter()
            .filter(|record| matches!(record.outcome, LookupOutcome::Resolved(_)))
    }

    /// Encodes the small versioned interchange format shared by native cache
    /// adapters and browser persistent stores.  Fields are UTF-8 hex escaped,
    /// so record boundaries and control characters remain unambiguous.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut records = self.records.clone();
        records.sort();
        let mut output = format!("umber-prefetch-manifest\t{PREFETCH_MANIFEST_SCHEMA}\n");
        output.push_str("i\t");
        output.push_str(
            &[
                escape(&self.identity.engine),
                escape(&self.identity.format),
                escape(&self.identity.options),
                escape(&self.identity.distribution),
                escape(&self.identity.search_policy),
            ]
            .join("\t"),
        );
        output.push('\n');
        for record in records {
            output.push_str("r\t");
            output.push_str(record.role.wire_name());
            output.push('\t');
            for field in [
                &record.original_spelling,
                &record.request_key,
                &record.kind,
                &record.search_context,
            ] {
                output.push_str(&escape(field));
                output.push('\t');
            }
            match record.outcome {
                LookupOutcome::Resolved(identity) => {
                    output.push_str("resolved\t");
                    output.push_str(&escape(&identity.manifest_key));
                    output.push('\t');
                    output.push_str(
                        &identity
                            .virtual_path
                            .as_deref()
                            .map_or_else(|| "-".to_owned(), escape),
                    );
                    output.push('\t');
                    output.push_str(&escape(&identity.object));
                    output.push('\t');
                    output.push_str(&identity.ahash64);
                    output.push('\t');
                    output.push_str(&identity.bytes.to_string());
                }
                LookupOutcome::Absent(scope) => {
                    output.push_str("absent\t");
                    output.push_str(&scope.encode());
                }
            }
            output.push('\n');
        }
        output.into_bytes()
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, ManifestError> {
        let text = std::str::from_utf8(bytes)
            .map_err(|_| ManifestError::new("prefetch manifest is not UTF-8"))?;
        let mut lines = text.lines();
        let header = lines
            .next()
            .ok_or_else(|| ManifestError::new("prefetch manifest is empty"))?;
        let (name, schema) = header
            .split_once('\t')
            .ok_or_else(|| ManifestError::new("prefetch manifest header is invalid"))?;
        if name != "umber-prefetch-manifest"
            || schema
                .parse::<u32>()
                .map_err(|_| ManifestError::new("prefetch manifest schema is invalid"))?
                != PREFETCH_MANIFEST_SCHEMA
        {
            return Err(ManifestError::new("unsupported prefetch manifest schema"));
        }
        let identity_line = lines
            .next()
            .ok_or_else(|| ManifestError::new("prefetch manifest has no identity"))?;
        let identity_fields = identity_line.split('\t').collect::<Vec<_>>();
        if identity_fields.len() != 6 || identity_fields[0] != "i" {
            return Err(ManifestError::new("prefetch manifest identity is invalid"));
        }
        let identity = PrefetchIdentity::new(
            unescape(identity_fields[1])?,
            unescape(identity_fields[2])?,
            unescape(identity_fields[3])?,
            unescape(identity_fields[4])?,
            unescape(identity_fields[5])?,
        )?;
        let mut manifest = Self::new(identity);
        for line in lines {
            if line.is_empty() {
                continue;
            }
            let fields = line.split('\t').collect::<Vec<_>>();
            if fields.len() < 8 || fields[0] != "r" {
                return Err(ManifestError::new("prefetch manifest record is invalid"));
            }
            let role = LookupRole::from_wire_name(fields[1])
                .ok_or_else(|| ManifestError::new("prefetch manifest role is invalid"))?;
            let original = unescape(fields[2])?;
            let key = unescape(fields[3])?;
            let kind = unescape(fields[4])?;
            let context = unescape(fields[5])?;
            let outcome = match fields[6] {
                "resolved" if fields.len() == 12 => LookupOutcome::Resolved(ResolvedIdentity::new(
                    unescape(fields[7])?,
                    (fields[8] != "-")
                        .then(|| unescape(fields[8]))
                        .transpose()?,
                    unescape(fields[9])?,
                    fields[10].to_owned(),
                    fields[11]
                        .parse()
                        .map_err(|_| ManifestError::new("resolved byte count is invalid"))?,
                )?),
                "absent" if fields.len() == 8 => {
                    LookupOutcome::Absent(NegativeScope::decode(fields[7])?)
                }
                _ => return Err(ManifestError::new("prefetch manifest outcome is invalid")),
            };
            manifest.record(LookupRecord::new(
                original, key, kind, context, role, outcome,
            )?)?;
        }
        Ok(manifest)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManifestError {
    message: String,
}

impl ManifestError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ManifestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ManifestError {}

fn validate_field(name: &str, value: &str, limit: usize) -> Result<(), ManifestError> {
    if value.is_empty() || value.len() > limit || value.chars().any(char::is_control) {
        return Err(ManifestError::new(format!("{name} is invalid")));
    }
    Ok(())
}

fn escape(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(value.len() * 2);
    for byte in value.as_bytes() {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 15) as usize] as char);
    }
    output
}

fn unescape(value: &str) -> Result<String, ManifestError> {
    if !value.len().is_multiple_of(2) {
        return Err(ManifestError::new("escaped field has odd length"));
    }
    let bytes = value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let digit = |byte| match byte {
                b'0'..=b'9' => Ok(byte - b'0'),
                b'a'..=b'f' => Ok(byte - b'a' + 10),
                _ => Err(ManifestError::new("escaped field is not hexadecimal")),
            };
            Ok((digit(pair[0])? << 4) | digit(pair[1])?)
        })
        .collect::<Result<Vec<_>, ManifestError>>()?;
    String::from_utf8(bytes).map_err(|_| ManifestError::new("escaped field is not UTF-8"))
}
