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

/// tex.web §303's classification of a source input level's `name`.
///
/// §303 states the whole partition: `name` "is zero if we are reading from
/// the terminal; it is `n+1` if we are reading from input stream `n`, where
/// `0<=n<=16`", and otherwise it is the string number of a text file. Only
/// three assignments in tex.web ever set a source level's `name`, and each
/// lands in exactly one arm below: §328's `begin_file_reading` (`name:=0`,
/// which §331 repeats for the base terminal level), §483's
/// `begin_file_reading; name:=m+1` inside `read_toks`, and §537's
/// `start_input` (`name:=a_make_name_string(cur_file)`). §329's
/// `end_file_reading` tests `name>17` to decide whether a real file handle
/// must be closed, and §313 renders `<*>`/`<insert>` for `name=0`, `<read n>`
/// for `1<=name<=17`, and `l.<line>` otherwise.
///
/// This is deliberately _not_ [`InputReason`](crate::InputReason). That enum
/// is a one-to-one model of §307's sixteen `token_type` codes plus one
/// `Source` arm for §303's `state<>token_list`; `token_type` is what §307
/// stores in place of `index` on a token-list level and has no meaning for a
/// source level at all. Nor is it [`RegisteredSourceKind`], which classifies
/// how Umber _acquired_ immutable bytes rather than which of TeX's three
/// input channels a level reads them as.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SourceNameClass {
    /// §303 `name=0`: the terminal, for which §304 defines the abbreviation
    /// `terminal_input==(name=0)`. §328's `begin_file_reading` leaves every
    /// new level here until its opener says otherwise, and §331 initializes
    /// the base level this way.
    Terminal,
    /// §303 `name=n+1` for `0<=n<=16`: input stream `n`, established by
    /// §483's `name:=m+1`. Stream 16 is §303's invalid stream number, whose
    /// input actually comes from the terminal under `read_toks` control and
    /// which §313 prints as `<read *>`.
    ReadStream(u8),
    /// §303 `name>17`: a text file, whose name §537's `start_input` records
    /// with `a_make_name_string`. This is the only arm §329's
    /// `end_file_reading` closes a handle for.
    File,
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

/// Allocates backing identity for a line acquired while tokenizing.
///
/// TeX82 writes an acquired line straight into `buffer`; Umber's lines are
/// ranges into immutable registered backing, so a line that did not come from
/// the file needs a source identity of its own before it can be read or
/// reported. Only the identity counter is borrowed: a replacement line is
/// held by the cursor reading it and is never added to the registry
/// [`crate::CommandState::open_registered_source`] searches, because no
/// input level may be opened on it.
pub(crate) struct LineBackingRegistry<'a> {
    pub(crate) profile: CommandProfile,
    pub(crate) next_identity: &'a mut u64,
}

impl LineBackingRegistry<'_> {
    /// Registers acquired line bytes, or refuses when no identity remains.
    pub(crate) fn register(
        &mut self,
        registration: SourceRegistration,
    ) -> Option<RegisteredSource> {
        let raw = u32::try_from(*self.next_identity).ok()?;
        let source =
            RegisteredSource::register(tex_state::SourceId::new(raw), self.profile, registration)
                .ok()?;
        *self.next_identity += 1;
        Some(source)
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
    /// Backing for the loaded line alone, when it is not the physical one.
    ///
    /// TeX82 §363's `firm_up_the_line` moves a line typed at the terminal
    /// down into `buffer` over the line the file supplied, so the level, its
    /// line number, and its `\endinput` accounting all survive while the
    /// characters do not. Umber's lines are ranges into immutable backing
    /// rather than a mutable buffer, so the replacement is registered as its
    /// own source and recorded here: every read of the _current line's_
    /// bytes goes through [`Self::current_backing`], while
    /// [`Self::load_next_line`] keeps advancing through the physical file.
    pub(crate) line_backing: Option<RegisteredSource>,
    /// This level's backing is one line tex.web §483 already acquired.
    ///
    /// §483 loads that line whether or not it has any characters: `limit:=
    /// last` with `last=first` is an empty line, `buffer[limit]:=
    /// end_line_char` still applies to it, and §486 depends on exactly that
    /// -- its appended empty line is what §351 turns into `\par`. An empty
    /// *file* is a different thing: §31's `input_ln` returns false before any
    /// line exists, so no line is loaded at all.
    pub(crate) pending_acquired_line: bool,
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
            line_backing: None,
            pending_acquired_line: false,
            next_physical_offset: 0,
            next_line_number: 1,
            line: None,
            lexer_state: LexerState::NewLine,
            end_after_line: false,
        }
    }

    /// The backing the loaded line's characters are read from.
    pub(crate) const fn current_backing(&self) -> &RegisteredSource {
        match self.line_backing.as_ref() {
            Some(replacement) => replacement,
            None => &self.backing,
        }
    }
}

#[cfg(test)]
mod tests;
