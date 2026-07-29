#![allow(
    clippy::disallowed_methods,
    reason = "committed fixture loading is host-only correctness tooling, not engine I/O"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    CanonicalValue, EngineDialect, Event, Manifest, ObservationError, ObservationStream,
    OracleToken, SourceLocation,
};

/// Current repository contract for committed semantic fixtures.
pub const FIXTURE_CONTRACT_VERSION: u32 = 2;
pub const FIXTURE_MANIFEST_NAME: &str = "manifest.json";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FixtureProfile {
    pub invocation: String,
    pub characters: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ToolIdentity {
    pub version: String,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalCitation {
    /// Canonical WEB source logical name, never a host path.
    pub source: String,
    /// Narrow section or named procedure in the cited canonical source.
    pub section: String,
    pub description: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FixtureArtifact {
    /// Path relative to the fixture directory.
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

/// Complete repository manifest for one committed canonical semantic fixture.
///
/// `oracle` is the immutable schema-version semantic manifest whose identity is
/// written in the event-stream header. This outer contract adds repository
/// selection, tool, profile, citation, and committed-file identities without
/// changing the schema-version manifest preimage.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FixtureManifest {
    pub contract: u32,
    pub name: String,
    pub profile: FixtureProfile,
    pub oracle: Manifest,
    pub tools: BTreeMap<String, ToolIdentity>,
    pub citations: Vec<CanonicalCitation>,
    /// Stable logical source name to committed source file.
    pub sources: BTreeMap<String, FixtureArtifact>,
    /// The `sources` key TeX was invoked on: the job's root file, as
    /// tex.web §537's `start_input` opens it, distinct from every other
    /// declared source, which the root reaches only through `\input`.
    /// `oracle.inputs` lists every input a run touched with no entry-point
    /// distinction, so this cannot be recovered from it or from any other
    /// existing field.
    pub root_source: String,
    pub events: FixtureArtifact,
    /// Ordinary-output channel to committed artifact observation.
    pub outputs: BTreeMap<String, FixtureArtifact>,
}

impl FixtureManifest {
    pub fn from_canonical_json(bytes: &[u8]) -> Result<Self, FixtureError> {
        let value: Self = serde_json::from_slice(bytes)
            .map_err(crate::EncodingError::from)
            .map_err(FixtureError::Encoding)?;
        value.validate()?;
        if value.to_canonical_json()? != bytes {
            return Err(FixtureError::Invalid(
                "fixture manifest is not canonical compact JSON followed by LF".into(),
            ));
        }
        Ok(value)
    }

    pub fn to_canonical_json(&self) -> Result<Vec<u8>, FixtureError> {
        self.validate()?;
        let mut bytes = serde_json::to_vec(self)
            .map_err(crate::EncodingError::from)
            .map_err(FixtureError::Encoding)?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    pub fn validate(&self) -> Result<(), FixtureError> {
        if self.contract != FIXTURE_CONTRACT_VERSION {
            return Err(FixtureError::Invalid(format!(
                "unsupported fixture contract {}; expected {FIXTURE_CONTRACT_VERSION}",
                self.contract
            )));
        }
        validate_logical_name("fixture", &self.name)?;
        validate_profile(self)?;
        self.oracle
            .to_canonical_json()
            .map_err(FixtureError::Encoding)?;
        if self.oracle.engine.banner.is_empty() {
            return Err(FixtureError::Invalid(
                "fixture engine banner must not be empty".into(),
            ));
        }
        if self.oracle.engine.upstream_change_sha256.is_empty() {
            return Err(FixtureError::Invalid(
                "fixture must identify at least one ordered upstream change".into(),
            ));
        }
        if self.oracle.environment.is_empty()
            || self.oracle.clock.is_empty()
            || self.oracle.ordinary_output_sha256.is_empty()
        {
            return Err(FixtureError::Invalid(
                "fixture environment, clock, and ordinary outputs are mandatory".into(),
            ));
        }
        if self.tools.is_empty() {
            return Err(FixtureError::Invalid(
                "fixture must identify its canonical generation tools".into(),
            ));
        }
        for (name, tool) in &self.tools {
            validate_logical_name("tool", name)?;
            validate_nonempty("tool version", &tool.version)?;
            validate_hash("tool", &tool.sha256)?;
        }
        if self.citations.is_empty() {
            return Err(FixtureError::Invalid(
                "fixture must contain at least one canonical source citation".into(),
            ));
        }
        for citation in &self.citations {
            validate_logical_name("citation source", &citation.source)?;
            validate_nonempty("citation section", &citation.section)?;
            validate_nonempty("citation description", &citation.description)?;
        }
        if self.sources.is_empty() {
            return Err(FixtureError::Invalid(
                "fixture must contain at least one focused INITEX source".into(),
            ));
        }
        if !self.sources.contains_key(&self.root_source) {
            return Err(FixtureError::Invalid(format!(
                "root source {} is not a declared fixture source",
                self.root_source
            )));
        }
        if self.outputs.is_empty() {
            return Err(FixtureError::Invalid(
                "fixture must contain ordinary artifact observations".into(),
            ));
        }

        let mut paths = BTreeSet::new();
        for (logical_name, artifact) in &self.sources {
            validate_logical_name("source", logical_name)?;
            validate_artifact("source", artifact, &mut paths)?;
            let Some(input) = self.oracle.inputs.get(logical_name) else {
                return Err(FixtureError::Invalid(format!(
                    "source {logical_name} is absent from the schema-version oracle manifest"
                )));
            };
            if input.bytes != artifact.bytes || input.sha256 != artifact.sha256 {
                return Err(FixtureError::Invalid(format!(
                    "source {logical_name} identity disagrees with the schema-version oracle manifest"
                )));
            }
        }
        if self.oracle.inputs.len() != self.sources.len() {
            return Err(FixtureError::Invalid(
                "schema-version oracle inputs and committed fixture sources differ".into(),
            ));
        }

        validate_artifact("event stream", &self.events, &mut paths)?;
        for (channel, artifact) in &self.outputs {
            validate_logical_name("ordinary output channel", channel)?;
            validate_artifact("ordinary output", artifact, &mut paths)?;
            if self.oracle.ordinary_output_sha256.get(channel) != Some(&artifact.sha256) {
                return Err(FixtureError::Invalid(format!(
                    "ordinary output {channel} identity disagrees with the schema-version oracle manifest"
                )));
            }
        }
        if self.oracle.ordinary_output_sha256.len() != self.outputs.len() {
            return Err(FixtureError::Invalid(
                "schema-version ordinary outputs and committed artifact observations differ".into(),
            ));
        }
        Ok(())
    }
}

/// A fully validated committed fixture loaded without invoking a live engine.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommittedFixture {
    pub manifest: FixtureManifest,
    pub stream: ObservationStream,
}

impl CommittedFixture {
    pub fn load(directory: impl AsRef<Path>) -> Result<Self, FixtureError> {
        let directory = directory.as_ref();
        let manifest_path = directory.join(FIXTURE_MANIFEST_NAME);
        let manifest_bytes =
            fs::read(&manifest_path).map_err(|error| FixtureError::io(&manifest_path, error))?;
        let manifest = FixtureManifest::from_canonical_json(&manifest_bytes)?;

        for artifact in manifest.sources.values() {
            validate_committed_file(directory, artifact)?;
        }

        let event_path = directory.join(&manifest.events.path);
        let event_bytes =
            fs::read(&event_path).map_err(|error| FixtureError::io(&event_path, error))?;
        validate_committed_bytes(&manifest.events, &event_bytes)?;
        for artifact in manifest.outputs.values() {
            validate_committed_file(directory, artifact)?;
        }
        let stream = ObservationStream::from_canonical_json_lines(&event_bytes)
            .map_err(FixtureError::Observation)?;
        let expected_manifest = manifest
            .oracle
            .identity()
            .map_err(FixtureError::Encoding)?
            .hex();
        if stream.header.manifest != expected_manifest {
            return Err(FixtureError::Invalid(
                "event stream is not bound to the fixture's schema-version oracle manifest".into(),
            ));
        }
        validate_stream_sources(&stream, &manifest.sources)?;
        Ok(Self { manifest, stream })
    }
}

#[derive(Debug)]
pub enum FixtureError {
    Encoding(crate::EncodingError),
    Observation(ObservationError),
    Invalid(String),
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

impl FixtureError {
    fn io(path: &Path, source: std::io::Error) -> Self {
        Self::Io {
            path: path.to_owned(),
            source,
        }
    }
}

impl fmt::Display for FixtureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Encoding(error) => error.fmt(formatter),
            Self::Observation(error) => error.fmt(formatter),
            Self::Invalid(error) => formatter.write_str(error),
            Self::Io { path, source } => {
                write!(formatter, "failed to read {}: {source}", path.display())
            }
        }
    }
}

impl Error for FixtureError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Encoding(error) => Some(error),
            Self::Observation(error) => Some(error),
            Self::Io { source, .. } => Some(source),
            Self::Invalid(_) => None,
        }
    }
}

fn validate_profile(manifest: &FixtureManifest) -> Result<(), FixtureError> {
    let expected = match manifest.oracle.engine.dialect {
        EngineDialect::Tex82 => ("initex", "eight_bit_exact"),
        EngineDialect::Etex26 => ("initex_compatibility_or_extended", "eight_bit_exact"),
        EngineDialect::Pdftex14027 => ("initex_with_etex_extensions", "eight_bit_exact"),
    };
    if (
        manifest.profile.invocation.as_str(),
        manifest.profile.characters.as_str(),
    ) != expected
    {
        return Err(FixtureError::Invalid(format!(
            "fixture profile {:?}/{:?} is not canonical for {:?}",
            manifest.profile.invocation,
            manifest.profile.characters,
            manifest.oracle.engine.dialect
        )));
    }
    Ok(())
}

fn validate_artifact(
    kind: &str,
    artifact: &FixtureArtifact,
    paths: &mut BTreeSet<String>,
) -> Result<(), FixtureError> {
    validate_logical_name(kind, &artifact.path)?;
    validate_hash(kind, &artifact.sha256)?;
    if !paths.insert(artifact.path.clone()) {
        return Err(FixtureError::Invalid(format!(
            "fixture artifact path {} is declared more than once",
            artifact.path
        )));
    }
    Ok(())
}

fn validate_committed_file(
    directory: &Path,
    artifact: &FixtureArtifact,
) -> Result<(), FixtureError> {
    let path = directory.join(&artifact.path);
    let bytes = fs::read(&path).map_err(|error| FixtureError::io(&path, error))?;
    validate_committed_bytes(artifact, &bytes)
}

fn validate_committed_bytes(artifact: &FixtureArtifact, bytes: &[u8]) -> Result<(), FixtureError> {
    if bytes.len() as u64 != artifact.bytes {
        return Err(FixtureError::Invalid(format!(
            "{} byte length drifted: expected {}, observed {}",
            artifact.path,
            artifact.bytes,
            bytes.len()
        )));
    }
    let observed = hex_hash(bytes);
    if observed != artifact.sha256 {
        return Err(FixtureError::Invalid(format!(
            "{} SHA-256 drifted: expected {}, observed {observed}",
            artifact.path, artifact.sha256
        )));
    }
    Ok(())
}

fn validate_stream_sources(
    stream: &ObservationStream,
    sources: &BTreeMap<String, FixtureArtifact>,
) -> Result<(), FixtureError> {
    for event in &stream.events {
        visit_event_locations(&event.semantic, &mut |location| {
            if !sources.contains_key(&location.source) {
                return Err(FixtureError::Invalid(format!(
                    "event {} cites undeclared source {}",
                    event.sequence, location.source
                )));
            }
            if location.line == 0 {
                return Err(FixtureError::Invalid(format!(
                    "event {} contains a zero source line",
                    event.sequence
                )));
            }
            Ok(())
        })?;
    }
    Ok(())
}

fn visit_event_locations(
    event: &Event,
    visitor: &mut impl FnMut(&SourceLocation) -> Result<(), FixtureError>,
) -> Result<(), FixtureError> {
    match event {
        Event::Command(event) => {
            if let Some(location) = &event.command.location {
                visitor(location)?;
            }
            visit_value_locations(&event.command.operand, visitor)
        }
        Event::Recovery(event) => visit_token_locations(&event.tokens, visitor),
        Event::Macro(crate::MacroEvent::Argument { tokens, .. })
        | Event::TokenList(crate::TokenListEvent { tokens, .. }) => {
            visit_token_locations(tokens, visitor)
        }
        Event::Scanner(event) => visit_value_locations(&event.result, visitor),
        Event::Mutation(event) => {
            visit_value_locations(&event.key, visitor)?;
            visit_value_locations(&event.value, visitor)
        }
        Event::Diagnostic(event) => {
            for value in &event.arguments {
                visit_value_locations(value, visitor)?;
            }
            Ok(())
        }
        Event::Effect(event) => visit_value_locations(&event.value, visitor),
        Event::Geometry(_) => Ok(()),
        Event::Input(_)
        | Event::ScannerStatus(_)
        | Event::Macro(crate::MacroEvent::Activation { .. })
        | Event::Condition(_)
        | Event::Alignment(_) => Ok(()),
    }
}

fn visit_value_locations(
    value: &CanonicalValue,
    visitor: &mut impl FnMut(&SourceLocation) -> Result<(), FixtureError>,
) -> Result<(), FixtureError> {
    match value {
        CanonicalValue::Token(token) => visit_token_location(token, visitor),
        CanonicalValue::Tokens(tokens) => visit_token_locations(tokens, visitor),
        _ => Ok(()),
    }
}

fn visit_token_locations(
    tokens: &[OracleToken],
    visitor: &mut impl FnMut(&SourceLocation) -> Result<(), FixtureError>,
) -> Result<(), FixtureError> {
    for token in tokens {
        visit_token_location(token, visitor)?;
    }
    Ok(())
}

fn visit_token_location(
    token: &OracleToken,
    visitor: &mut impl FnMut(&SourceLocation) -> Result<(), FixtureError>,
) -> Result<(), FixtureError> {
    if let Some(location) = &token.location {
        visitor(location)?;
    }
    Ok(())
}

fn validate_nonempty(kind: &str, value: &str) -> Result<(), FixtureError> {
    if value.is_empty() {
        Err(FixtureError::Invalid(format!("{kind} must not be empty")))
    } else {
        Ok(())
    }
}

fn validate_hash(kind: &str, hash: &str) -> Result<(), FixtureError> {
    if hash.len() == 64
        && hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(FixtureError::Invalid(format!(
            "{kind} identity must be 64 lowercase hexadecimal characters"
        )))
    }
}

fn validate_logical_name(kind: &str, name: &str) -> Result<(), FixtureError> {
    if name.is_empty()
        || name.starts_with('/')
        || name.contains('\\')
        || name
            .split('/')
            .any(|component| component.is_empty() || component == "..")
    {
        Err(FixtureError::Invalid(format!(
            "{kind} name must be a non-empty logical name, not a host path"
        )))
    } else {
        Ok(())
    }
}

fn hex_hash(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}
