//! Registered source and source-cursor ownership.

use std::fmt;
use std::sync::Arc;

use tex_state::source_map::SourceDescriptor;
use tex_state::world::FileContent;
use tex_state::{InputRecordId, SourceId};

use crate::profile::{CharacterMode, CommandProfile};

use super::lines::SourceLineState;
use super::tokenizer::LexerState;

/// The acquisition class of immutable bytes handed to the command machine.
///
/// This is descriptive state, not a host capability. File lookup, stream
/// reads, editor synchronization, and all other acquisition must finish
/// before a value of this type is registered.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RegisteredSourceKind {
    /// Bytes already retained by aggregate `World` input state.
    World,
    /// Immutable generated or in-memory bytes.
    Generated,
    /// An immutable editor fragment selected from a registered layout.
    EditorFragment,
    /// One completely acquired, explicitly typed `\readline` source.
    ReadLine,
}

/// Host-neutral input used to register one complete immutable source.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct SourceRegistration {
    kind: RegisteredSourceKind,
    bytes: Arc<[u8]>,
    world_record: Option<InputRecordId>,
}

impl SourceRegistration {
    /// Constructs a registration from already acquired bytes.
    #[must_use]
    pub fn new(kind: RegisteredSourceKind, bytes: impl Into<Arc<[u8]>>) -> Self {
        Self {
            kind,
            bytes: bytes.into(),
            world_record: None,
        }
    }

    /// Constructs a registration retaining the exact successful World read.
    /// This preserves the input-record identity used by source provenance.
    #[must_use]
    pub fn world(content: FileContent) -> Self {
        Self {
            kind: RegisteredSourceKind::World,
            bytes: content.shared_bytes(),
            world_record: Some(content.record()),
        }
    }

    /// Returns the acquisition class retained with the backing.
    #[must_use]
    pub const fn kind(&self) -> RegisteredSourceKind {
        self.kind
    }

    /// Returns the complete immutable physical backing.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// The exact byte range implicated by malformed Unicode registration.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MalformedUnicodeRange {
    start: u64,
    end: u64,
}

impl MalformedUnicodeRange {
    /// Inclusive start byte offset.
    #[must_use]
    pub const fn start(self) -> u64 {
        self.start
    }

    /// Exclusive end byte offset.
    #[must_use]
    pub const fn end(self) -> u64 {
        self.end
    }
}

/// A source could not be registered in the fixed job profile.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SourceRegistrationError {
    /// The Unicode profile rejected malformed UTF-8 before source allocation.
    MalformedUnicode(MalformedUnicodeRange),
    /// No further `tex_state::SourceId` can be represented.
    SourceIdentityExhausted,
    /// The backing cannot be represented in the command core's `u64` ranges.
    BackingTooLarge,
}

impl fmt::Display for SourceRegistrationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MalformedUnicode(range) => write!(
                formatter,
                "malformed UTF-8 in source byte range {}..{}",
                range.start, range.end
            ),
            Self::SourceIdentityExhausted => formatter.write_str("source identity space exhausted"),
            Self::BackingTooLarge => {
                formatter.write_str("source backing is too large for source ranges")
            }
        }
    }
}

impl std::error::Error for SourceRegistrationError {}

/// Complete immutable source backing retained by command state.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct RegisteredSource {
    pub(crate) id: SourceId,
    pub(crate) kind: RegisteredSourceKind,
    pub(crate) mode: CharacterMode,
    pub(crate) bytes: Arc<[u8]>,
    pub(crate) world_record: Option<InputRecordId>,
}

impl RegisteredSource {
    /// Returns the immutable backing descriptor used to register source
    /// coordinates with the aggregate source map.
    pub(crate) fn source_descriptor(&self) -> SourceDescriptor {
        self.world_record.map_or_else(
            || SourceDescriptor::generated(Arc::clone(&self.bytes)),
            |record| {
                SourceDescriptor::world(
                    record,
                    u64::try_from(self.bytes.len())
                        .expect("registered source length was validated"),
                )
            },
        )
    }

    pub(crate) fn register(
        id: SourceId,
        profile: CommandProfile,
        registration: SourceRegistration,
    ) -> Result<Self, SourceRegistrationError> {
        u64::try_from(registration.bytes.len())
            .map_err(|_| SourceRegistrationError::BackingTooLarge)?;
        let mode = profile.character_mode();
        if mode == CharacterMode::UnicodeExtended
            && let Err(error) = std::str::from_utf8(&registration.bytes)
        {
            let start = u64::try_from(error.valid_up_to())
                .map_err(|_| SourceRegistrationError::BackingTooLarge)?;
            let remaining = registration.bytes.len() - error.valid_up_to();
            let error_len = error.error_len().unwrap_or(remaining);
            let end = start
                .checked_add(
                    u64::try_from(error_len)
                        .map_err(|_| SourceRegistrationError::BackingTooLarge)?,
                )
                .ok_or(SourceRegistrationError::BackingTooLarge)?;
            return Err(SourceRegistrationError::MalformedUnicode(
                MalformedUnicodeRange { start, end },
            ));
        }
        Ok(Self {
            id,
            kind: registration.kind,
            mode,
            bytes: registration.bytes,
            world_record: registration.world_record,
        })
    }
}

/// Future-relevant physical cursor into registered immutable backing.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct SourceCursor {
    pub(crate) backing: RegisteredSource,
    pub(crate) next_physical_offset: u64,
    pub(crate) next_line_number: u64,
    pub(crate) line: Option<SourceLineState>,
    pub(crate) lexer_state: LexerState,
    pub(crate) end_after_line: bool,
}

impl SourceCursor {
    pub(crate) fn new(backing: RegisteredSource) -> Self {
        Self {
            backing,
            next_physical_offset: 0,
            next_line_number: 1,
            line: None,
            lexer_state: LexerState::NewLine,
            end_after_line: false,
        }
    }
}

#[cfg(test)]
mod tests;
