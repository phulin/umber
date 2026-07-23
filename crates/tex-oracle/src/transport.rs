use std::error::Error;
use std::fmt;
use std::io::{self, Write};

use serde::{Deserialize, Serialize};

use crate::encoding::{encode_line, stream_hash};
use crate::{Event, ManifestIdentity, Normalizer, SCHEMA_VERSION, StreamIdentity};

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
        Self {
            schema: SCHEMA_VERSION,
            manifest: manifest.hex(),
        }
    }
}

#[derive(Debug)]
pub enum ObservationError {
    Encoding(crate::EncodingError),
    Io(io::Error),
}

impl fmt::Display for ObservationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Encoding(error) => error.fmt(formatter),
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
    encoded: Vec<u8>,
}

impl<W: Write> JsonLinesObserver<W> {
    pub fn new(mut writer: W, manifest: ManifestIdentity) -> Result<Self, ObservationError> {
        let header = encode_line(&ObservationHeader::new(manifest))?;
        writer.write_all(&header)?;
        Ok(Self {
            writer,
            normalizer: Normalizer::new(),
            encoded: header,
        })
    }

    pub fn finish(mut self) -> Result<(W, StreamIdentity), ObservationError> {
        self.writer.flush()?;
        let identity = stream_hash(&self.encoded);
        Ok((self.writer, identity))
    }
}

impl<W: Write> EventObserver for JsonLinesObserver<W> {
    fn committed(&mut self, event: Event) -> Result<(), ObservationError> {
        let normalized = self.normalizer.normalize(event);
        let line = encode_line(&normalized)?;
        self.writer.write_all(&line)?;
        self.encoded.extend_from_slice(&line);
        Ok(())
    }
}
