use std::error::Error;
use std::fmt;
use std::io::{self, Write};

use serde::{Deserialize, Serialize};

use crate::encoding::{StreamHasher, encode_line};
use crate::{Event, ManifestIdentity, NormalizedEvent, Normalizer, SchemaVersion, StreamIdentity};

/// The first JSON line of every semantic event stream.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationHeader {
    pub schema: u32,
    pub manifest: String,
}

impl ObservationHeader {
    #[must_use]
    pub fn new(manifest: ManifestIdentity) -> Self {
        Self::for_schema(SchemaVersion::V1, manifest)
    }

    #[must_use]
    pub fn for_schema(schema: SchemaVersion, manifest: ManifestIdentity) -> Self {
        Self {
            schema: schema.number(),
            manifest: manifest.hex(),
        }
    }
}

/// Decoded canonical stream used by fixture comparison harnesses.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservationStream {
    pub header: ObservationHeader,
    pub events: Vec<NormalizedEvent>,
}

impl ObservationStream {
    /// Parses canonical JSON Lines and rejects alternate whitespace, missing
    /// final newlines, unsupported schemas, or discontinuous sequence numbers.
    pub fn from_canonical_json_lines(bytes: &[u8]) -> Result<Self, ObservationError> {
        if !bytes.ends_with(b"\n") {
            return Err(ObservationError::InvalidStream(
                "canonical event stream must end with LF".into(),
            ));
        }
        let mut lines = bytes.split_inclusive(|byte| *byte == b'\n');
        let header_line = lines.next().ok_or_else(|| {
            ObservationError::InvalidStream("event stream is missing its header".into())
        })?;
        let header: ObservationHeader = decode_canonical_line(header_line)?;
        let schema =
            SchemaVersion::try_from(header.schema).map_err(ObservationError::InvalidStream)?;
        validate_identity("manifest", &header.manifest)?;

        // Large document traces contain millions of events. Count their
        // already-required line delimiters once so decoding does not
        // repeatedly grow and copy the event vector.
        let event_count = memchr::memchr_iter(b'\n', bytes).count() - 1;
        let mut events = Vec::with_capacity(event_count);
        for (expected, line) in lines.enumerate() {
            let event: NormalizedEvent = decode_canonical_line(line)?;
            if event.sequence != expected as u64 {
                return Err(ObservationError::InvalidStream(format!(
                    "oracle event sequence {} is not expected sequence {expected}",
                    event.sequence
                )));
            }
            if matches!(event.semantic, Event::Geometry(_)) && schema == SchemaVersion::V1 {
                return Err(ObservationError::InvalidStream(
                    "schema v1 does not permit geometry events".into(),
                ));
            }
            if matches!(event.semantic, Event::DiagnosticLifecycle(_))
                && schema < SchemaVersion::V4
            {
                return Err(ObservationError::InvalidStream(
                    "schema versions before v4 do not permit diagnostic lifecycle events".into(),
                ));
            }
            if schema == SchemaVersion::V4
                && matches!(
                    event.semantic,
                    Event::DiagnosticLifecycle(crate::DiagnosticLifecycleEvent::Report {
                        location: None,
                        ..
                    })
                )
            {
                return Err(ObservationError::InvalidStream(
                    "schema v4 diagnostic reports require source provenance".into(),
                ));
            }
            if event.semantic.view().class() == crate::EventClass::Geometry
                && schema >= SchemaVersion::V3
                && !has_geometry_location(&event.semantic)
            {
                return Err(ObservationError::InvalidStream(
                    "schema v3 geometry events require source provenance".into(),
                ));
            }
            events.push(event);
        }
        Ok(Self { header, events })
    }

    #[must_use]
    pub fn identity(bytes: &[u8]) -> StreamIdentity {
        let schema = bytes
            .split_inclusive(|byte| *byte == b'\n')
            .next()
            .and_then(|line| serde_json::from_slice::<ObservationHeader>(line).ok())
            .and_then(|header| SchemaVersion::try_from(header.schema).ok())
            .unwrap_or(SchemaVersion::V1);
        let mut hasher = StreamHasher::new(schema);
        hasher.update(bytes);
        hasher.finish()
    }
}

fn has_geometry_location(event: &Event) -> bool {
    let mut found = false;
    event.view().visit_locations(&mut |location| {
        found |= matches!(location, crate::EventLocation::Geometry(_));
    });
    found
}

#[derive(Debug)]
pub enum ObservationError {
    Encoding(crate::EncodingError),
    InvalidStream(String),
    Io(io::Error),
}

impl fmt::Display for ObservationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Encoding(error) => error.fmt(formatter),
            Self::InvalidStream(error) => formatter.write_str(error),
            Self::Io(error) => error.fmt(formatter),
        }
    }
}

impl Error for ObservationError {}

impl From<crate::EncodingError> for ObservationError {
    fn from(error: crate::EncodingError) -> Self {
        Self::Encoding(error)
    }
}

impl From<io::Error> for ObservationError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Narrow engine-facing boundary. Callers must construct an owned event only
/// after its semantic transition commits.
pub trait EventObserver {
    fn committed(&mut self, event: Event) -> Result<(), ObservationError>;
}

/// Compile-away observation boundary for ordinary builds.
#[derive(Clone, Copy, Debug, Default)]
pub struct DisabledObserver;

impl EventObserver for DisabledObserver {
    #[inline(always)]
    fn committed(&mut self, _event: Event) -> Result<(), ObservationError> {
        Ok(())
    }
}

/// Dedicated JSON-lines instrumentation transport.
///
/// The writer is supplied independently from stdout, transcript, selector, and
/// other ordinary engine outputs. This type never reads semantic engine state.
pub struct JsonLinesObserver<W> {
    writer: W,
    normalizer: Normalizer,
    identity: StreamHasher,
}

impl<W: Write> JsonLinesObserver<W> {
    pub fn new(writer: W, manifest: ManifestIdentity) -> Result<Self, ObservationError> {
        Self::new_for_schema(writer, SchemaVersion::V1, manifest)
    }

    pub fn new_for_schema(
        mut writer: W,
        schema: SchemaVersion,
        manifest: ManifestIdentity,
    ) -> Result<Self, ObservationError> {
        let header = encode_line(&ObservationHeader::for_schema(schema, manifest))?;
        writer.write_all(&header)?;
        let mut identity = StreamHasher::new(schema);
        identity.update(&header);
        Ok(Self {
            writer,
            normalizer: Normalizer::new(),
            identity,
        })
    }

    pub fn finish(mut self) -> Result<(W, StreamIdentity), ObservationError> {
        self.writer.flush()?;
        let identity = self.identity.finish();
        Ok((self.writer, identity))
    }
}

impl<W: Write> EventObserver for JsonLinesObserver<W> {
    fn committed(&mut self, event: Event) -> Result<(), ObservationError> {
        let normalized = self.normalizer.normalize(event);
        let line = encode_line(&normalized)?;
        self.writer.write_all(&line)?;
        self.identity.update(&line);
        Ok(())
    }
}

fn decode_canonical_line<T>(line: &[u8]) -> Result<T, ObservationError>
where
    T: for<'de> Deserialize<'de> + Serialize,
{
    let value = serde_json::from_slice(line).map_err(crate::EncodingError::from)?;
    if encode_line(&value)? != line {
        return Err(ObservationError::InvalidStream(
            "event stream line is not in canonical encoding".into(),
        ));
    }
    Ok(value)
}

fn validate_identity(kind: &str, identity: &str) -> Result<(), ObservationError> {
    if identity.len() == 64
        && identity
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(ObservationError::InvalidStream(format!(
            "{kind} identity must be 64 lowercase hexadecimal characters"
        )))
    }
}
