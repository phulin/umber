//! Registered source and source-cursor ownership.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use tex_state::source_map::SourceDescriptor;
use tex_state::world::FileContent;
use tex_state::{InputRecordId, SourceId};

pub use tex_state::packed_input::SourceRole;

use crate::profile::{CharacterMode, CommandProfile};

use super::lines::SourceLineState;

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

/// Ownership policy for transcript framing of a named source level.
///
/// Source identity and diagnostic provenance remain attached to the
/// registration in both modes. This policy controls only whether the command
/// machine emits the canonical §537/§362 open and close events.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum SourceFramingPolicy {
    /// The command machine owns the source's ordinary open and close events.
    #[default]
    Canonical,
    /// A surrounding host frame already represents this source in the
    /// transcript, so the command machine must not duplicate that frame.
    ExternallyOwned,
}

/// tex.web §303's classification of a source input level's `name`.
///
/// TeX82 §303 states its original partition: `name` "is zero if we are reading from
/// the terminal; it is `n+1` if we are reading from input stream `n`, where
/// `0<=n<=16`", and otherwise it is the string number of a text file. Only
/// three assignments in tex.web set a source level's `name`, and each lands
/// in one arm below: §328's `begin_file_reading` (`name:=0`,
/// which §331 repeats for the base terminal level), §483's
/// `begin_file_reading; name:=m+1` inside `read_toks`, and §537's
/// `start_input` (`name:=a_make_name_string(cur_file)`). e-TeX merged §53a
/// adds the fourth assignment, numeric name 18 or 19 for `\scantokens`, and
/// merged §22 raises the real-file close and error-context bottom-line test
/// to `name>19`. §313 renders `<*>`/`<insert>` for `name=0`, `<read n>` for
/// `1<=name<=17`, and `l.<line>` otherwise.
///
/// This is deliberately _not_ [`InputReason`](crate::InputReason). That enum
/// is a one-to-one model of §307's sixteen `token_type` codes plus one
/// `Source` arm for §303's `state<>token_list`; `token_type` is what §307
/// stores in place of `index` on a token-list level and has no meaning for a
/// source level at all. Nor is it [`RegisteredSourceKind`], which classifies
/// how Umber _acquired_ immutable bytes rather than which of TeX's four
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
    /// e-TeX 2.6 merged §53a's generated `\scantokens` pseudo-file. Its
    /// numeric name is 18 when `\tracingscantokens<=0` and 19 otherwise.
    /// Both names use §313's line-number rendering, but merged §22's
    /// `name>19` test deliberately keeps traversing to the enclosing input
    /// level when an error context is shown.
    Scantokens(u8),
    /// e-TeX merged §22 `name>19`: a text file, whose name §537's
    /// `start_input` records with `a_make_name_string`. This is the only arm
    /// §329's `end_file_reading` closes a handle for.
    File,
}

/// Host-neutral input used to register one complete immutable source.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct SourceRegistration {
    kind: RegisteredSourceKind,
    /// Explicit host/VFS classification. `None` means inherit the enclosing
    /// external source when this registration is opened as nested input.
    role: Option<SourceRole>,
    bytes: Arc<[u8]>,
    world_record: Option<InputRecordId>,
    modification_date: Option<tex_state::FileModificationDate>,
    name: Option<Arc<str>>,
    framing_name: Option<Arc<str>>,
    framing: SourceFramingPolicy,
}

impl SourceRegistration {
    /// Constructs a registration from already acquired bytes.
    #[must_use]
    pub fn new(kind: RegisteredSourceKind, bytes: impl Into<Arc<[u8]>>) -> Self {
        Self {
            kind,
            role: None,
            bytes: bytes.into(),
            world_record: None,
            modification_date: None,
            name: None,
            framing_name: None,
            framing: SourceFramingPolicy::Canonical,
        }
    }

    /// Constructs a registration retaining the exact successful World read.
    /// This preserves the input-record identity used by source provenance.
    #[must_use]
    pub fn world(content: FileContent) -> Self {
        Self {
            kind: RegisteredSourceKind::World,
            role: None,
            bytes: content.shared_bytes(),
            world_record: Some(content.record()),
            modification_date: content.modification_date(),
            name: None,
            framing_name: None,
            framing: SourceFramingPolicy::Canonical,
        }
    }

    /// Attaches tex.web §537's `a_make_name_string` name.
    ///
    /// §537's `start_input` is the only place tex.web ever names a source:
    /// `name:=a_make_name_string(cur_file)` records the opened file's name so
    /// §303's `name>17` case -- and §313's transcript rendering of it -- has
    /// something to print. A registration with no name is exactly §331's
    /// terminal or §483's `\read` pseudo-file, neither of which §537 ever
    /// touches, so leaving this unset is the correct default rather than a
    /// gap to fill in later.
    #[must_use]
    pub fn with_name(mut self, name: impl Into<Arc<str>>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Selects a selector-visible §537 name distinct from source provenance.
    #[must_use]
    pub fn with_framing_name(mut self, name: impl Into<Arc<str>>) -> Self {
        self.framing_name = Some(name.into());
        self
    }

    /// Returns immutable modification metadata captured with a World read.
    #[must_use]
    pub const fn modification_date(&self) -> Option<tex_state::FileModificationDate> {
        self.modification_date
    }

    /// Selects who owns transcript framing for this source.
    #[must_use]
    pub fn with_framing(mut self, framing: SourceFramingPolicy) -> Self {
        self.framing = framing;
        self
    }

    /// Supplies the semantic source role selected by the host or VFS.
    ///
    /// Registrations without an override inherit the enclosing external
    /// source deterministically when opened. Root registration supplies
    /// [`SourceRole::RootDocument`] when the host did not select a more
    /// specific role such as [`SourceRole::FormatInitialization`].
    #[must_use]
    pub fn with_role(mut self, role: SourceRole) -> Self {
        self.role = Some(role);
        self
    }

    /// Returns the host/VFS override, or `None` when nested opening inherits.
    #[must_use]
    pub const fn role(&self) -> Option<SourceRole> {
        self.role
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

    /// Returns shared ownership of the exact immutable physical backing.
    #[must_use]
    pub fn shared_bytes(&self) -> Arc<[u8]> {
        Arc::clone(&self.bytes)
    }

    /// Returns the §537 name attached by [`Self::with_name`], if any.
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Returns the transcript-framing ownership policy.
    #[must_use]
    pub const fn framing(&self) -> SourceFramingPolicy {
        self.framing
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
    pub(crate) usage: &'a mut crate::state::CommandStackUsage,
    pub(crate) buffer_start: usize,
    pub(crate) name_class: Option<SourceNameClass>,
}

impl LineBackingRegistry<'_> {
    pub(crate) fn record_line_usage(&mut self, cursor: &SourceCursor) {
        if let Some(positions) =
            source_line_buffer_high_water(cursor, self.name_class, self.buffer_start)
        {
            self.usage.record_buffer_usage(positions);
        }
    }

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

/// Test-only accounting for source-buffer projection work.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct SourceBufferSlotMeasurement {
    pub(crate) calls: usize,
    pub(crate) unicode_scalars_counted: usize,
}

#[cfg(test)]
thread_local! {
    static SOURCE_BUFFER_SLOT_MEASUREMENT: core::cell::Cell<SourceBufferSlotMeasurement> =
        const { core::cell::Cell::new(SourceBufferSlotMeasurement {
            calls: 0,
            unicode_scalars_counted: 0,
        }) };
}

#[cfg(test)]
pub(crate) fn reset_source_buffer_slot_measurement() {
    SOURCE_BUFFER_SLOT_MEASUREMENT
        .with(|measurement| measurement.set(SourceBufferSlotMeasurement::default()));
}

#[cfg(test)]
pub(crate) fn source_buffer_slot_measurement() -> SourceBufferSlotMeasurement {
    SOURCE_BUFFER_SLOT_MEASUREMENT.with(core::cell::Cell::get)
}

/// Number of TeX `buffer` slots occupied by one currently loaded source line.
///
/// TeX keeps every live source line in one contiguous buffer. The occupied
/// range includes its retained characters, the optional normalized
/// `endlinechar`, and the sentinel position after the line.
pub(crate) fn occupied_source_buffer_slots(cursor: &SourceCursor) -> usize {
    #[cfg(test)]
    SOURCE_BUFFER_SLOT_MEASUREMENT.with(|measurement| {
        let mut current = measurement.get();
        current.calls = current.calls.saturating_add(1);
        measurement.set(current);
    });
    let Some(line) = cursor.line.as_ref() else {
        return 0;
    };
    let start = line.physical.content_range().start();
    let retained = line.retained_end.saturating_sub(start);
    let len = match cursor.current_backing().mode {
        CharacterMode::EightBitExact => usize::try_from(retained).unwrap_or(usize::MAX),
        CharacterMode::UnicodeExtended => {
            let start = usize::try_from(start).unwrap_or(usize::MAX);
            let end = usize::try_from(line.retained_end).unwrap_or(usize::MAX);
            let count = cursor
                .current_backing()
                .bytes
                .get(start..end)
                .and_then(|bytes| std::str::from_utf8(bytes).ok())
                .map_or(usize::MAX, |text| text.chars().count());
            #[cfg(test)]
            SOURCE_BUFFER_SLOT_MEASUREMENT.with(|measurement| {
                let mut current = measurement.get();
                current.unicode_scalars_counted =
                    current.unicode_scalars_counted.saturating_add(count);
                measurement.set(current);
            });
            count
        }
    };
    len.saturating_add(usize::from(line.endline.is_some()))
        .saturating_add(1)
}

/// Returns the §1334 buffer-stack projection reached while acquiring one line.
///
/// Ordinary tex.web §31 `input_ln` records every physical character before
/// stripping trailing spaces. e-TeX §53a's `pseudo_input` instead copies each
/// `\scantokens` line in padded four-byte blocks and updates `max_buf_stack`
/// before removing that padding. A §363 terminal replacement has ordinary
/// `input_ln` storage even when it replaces a pseudo-file line.
pub(crate) fn source_line_buffer_high_water(
    cursor: &SourceCursor,
    name_class: Option<SourceNameClass>,
    buffer_start: usize,
) -> Option<usize> {
    let line = cursor.line.as_ref()?;
    let backing = cursor.current_backing();
    let start = usize::try_from(line.physical.content_range().start()).unwrap_or(usize::MAX);
    let end = usize::try_from(line.physical.content_range().end()).unwrap_or(usize::MAX);
    let len = match backing.mode {
        CharacterMode::EightBitExact => end.saturating_sub(start),
        CharacterMode::UnicodeExtended => backing
            .bytes
            .get(start..end)
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
            .map_or(usize::MAX, |text| text.chars().count()),
    };
    let pseudo_input =
        cursor.line_backing.is_none() && matches!(name_class, Some(SourceNameClass::Scantokens(_)));
    if pseudo_input {
        let occupied = len.max(1);
        let padded = occupied.saturating_add((4 - occupied % 4) % 4);
        // e-TeX §53a sets max_buf_stack to `last+1`; §1334 prints one
        // further position. This differs deliberately from §31's update.
        Some(buffer_start.saturating_add(padded).saturating_add(2))
    } else if len > 0 {
        // §31 updates max_buf_stack for each stored character; §1334
        // prints that greatest index plus one.
        Some(buffer_start.saturating_add(len).saturating_add(1))
    } else {
        None
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
    /// The new editor buffer differs before a checkpoint's conservative
    /// physical-line anchor.
    RestartPrefixMismatch { anchor: u64 },
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
            Self::RestartPrefixMismatch { anchor } => write!(
                formatter,
                "editor source changed before restart anchor {anchor}"
            ),
        }
    }
}

impl std::error::Error for SourceRegistrationError {}

/// Complete immutable source backing owned by a pending open or live level.
#[derive(Clone)]
pub(crate) struct RegisteredSource {
    pub(crate) id: SourceId,
    pub(crate) kind: RegisteredSourceKind,
    pub(crate) role: Option<SourceRole>,
    pub(crate) mode: CharacterMode,
    pub(crate) bytes: Arc<[u8]>,
    /// tex.web §537's `a_make_name_string` name, carried from the
    /// [`SourceRegistration`] that produced this backing. See
    /// [`SourceRegistration::with_name`].
    pub(crate) name: Option<Arc<str>>,
    pub(crate) framing_name: Option<Arc<str>>,
    pub(crate) framing: SourceFramingPolicy,
    descriptor: Arc<SourceDescriptor>,
}

impl RegisteredSource {
    pub(crate) fn is_editor_backing(&self, accepted: &[u8]) -> bool {
        self.kind == RegisteredSourceKind::Generated
            && self.name.is_some()
            && self.bytes.as_ref() == accepted
    }

    /// Returns the §537 name when this source owns canonical file framing.
    pub(crate) fn canonical_framing_name(&self) -> Option<Arc<str>> {
        (self.framing == SourceFramingPolicy::Canonical)
            .then(|| self.framing_name.clone().or_else(|| self.name.clone()))
            .flatten()
    }

    pub(crate) fn rebind_generated(
        &self,
        id: SourceId,
        bytes: Arc<[u8]>,
    ) -> Result<Self, SourceRegistrationError> {
        u64::try_from(bytes.len()).map_err(|_| SourceRegistrationError::BackingTooLarge)?;
        if self.mode == CharacterMode::UnicodeExtended
            && let Err(error) = std::str::from_utf8(&bytes)
        {
            let start = u64::try_from(error.valid_up_to())
                .map_err(|_| SourceRegistrationError::BackingTooLarge)?;
            let remaining = bytes.len() - error.valid_up_to();
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
        let descriptor = Arc::new(SourceDescriptor::editor_revision(
            self.name.as_deref(),
            Arc::clone(&bytes),
        ));
        Ok(Self {
            id,
            kind: RegisteredSourceKind::Generated,
            role: self.role,
            mode: self.mode,
            bytes,
            name: self.name.clone(),
            framing_name: self.framing_name.clone(),
            framing: self.framing,
            descriptor,
        })
    }

    pub(crate) fn rehome_generated(
        &self,
        bytes: Arc<[u8]>,
    ) -> Result<Self, SourceRegistrationError> {
        self.rebind_generated(self.id, bytes)
    }

    /// Returns the immutable backing descriptor used to register source
    /// coordinates with the aggregate source map.
    pub(crate) fn source_descriptor(&self) -> SourceDescriptor {
        (*self.descriptor).clone()
    }

    fn world_record(&self) -> Option<InputRecordId> {
        match self.descriptor.as_ref() {
            SourceDescriptor::World { input_record, .. } => Some(*input_record),
            SourceDescriptor::Generated(_) => None,
        }
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
        let descriptor = registration.world_record.map_or_else(
            || {
                registration.name.as_ref().map_or_else(
                    || SourceDescriptor::generated(Arc::clone(&registration.bytes)),
                    |name| {
                        SourceDescriptor::named_generated(
                            name.to_string(),
                            Arc::clone(&registration.bytes),
                        )
                    },
                )
            },
            |record| {
                SourceDescriptor::world(
                    record,
                    u64::try_from(registration.bytes.len())
                        .expect("registered source length was validated"),
                )
            },
        );
        Ok(Self {
            id,
            kind: registration.kind,
            role: registration.role,
            mode,
            bytes: registration.bytes,
            name: registration.name,
            framing_name: registration.framing_name,
            framing: registration.framing,
            descriptor: Arc::new(descriptor),
        })
    }
}

impl fmt::Debug for RegisteredSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RegisteredSource")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("role", &self.role)
            .field("mode", &self.mode)
            .field("bytes", &self.bytes)
            .field("name", &self.name)
            .field("framing_name", &self.framing_name)
            .field("world_record", &self.world_record())
            .finish()
    }
}

impl PartialEq for RegisteredSource {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.kind == other.kind
            && self.mode == other.mode
            && self.bytes == other.bytes
            && self.name == other.name
            && self.framing_name == other.framing_name
            && self.world_record() == other.world_record()
    }
}

impl Eq for RegisteredSource {}

impl Hash for RegisteredSource {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
        self.kind.hash(state);
        self.mode.hash(state);
        self.bytes.hash(state);
        self.name.hash(state);
        self.framing_name.hash(state);
        self.world_record().hash(state);
    }
}

/// Future-relevant physical cursor into registered immutable backing.
#[derive(Debug, Eq, Hash, PartialEq)]
pub(crate) struct SourceCursor {
    pub(crate) backing: RegisteredSource,
    /// Whether the aggregate source map already owns this backing's cold
    /// registration payload. This avoids retaining and releasing the
    /// Arc-backed descriptor for every token after the first one.
    pub(crate) backing_registered: bool,
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
    /// Whether the current replacement-line backing has been registered.
    pub(crate) line_backing_registered: bool,
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
    pub(crate) end_after_line: bool,
}

/// Reversible cold execution state for one authoritative source slot.
///
/// The immutable physical backing stays in the slot. Current-line and
/// replacement-backing owners move into this value exactly once; rollback and
/// candidate redo swap them back without cloning an `Arc`, `Vec`, or `Box`.
#[derive(Debug)]
pub(crate) struct SourceCursorExecutionState {
    backing_registered: bool,
    line_backing: Option<RegisteredSource>,
    line_backing_registered: bool,
    pending_acquired_line: bool,
    next_physical_offset: u64,
    next_line_number: u64,
    line: Option<SourceLineState>,
    end_after_line: bool,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct SourceOffsetMap {
    old_start: u64,
    old_end: u64,
    new_end: u64,
}

impl SourceOffsetMap {
    pub(crate) fn new(old_start: usize, old_end: usize, new_end: usize) -> Self {
        Self {
            old_start: u64::try_from(old_start).unwrap_or(u64::MAX),
            old_end: u64::try_from(old_end).unwrap_or(u64::MAX),
            new_end: u64::try_from(new_end).unwrap_or(u64::MAX),
        }
    }

    pub(crate) const fn map(self, old: u64) -> u64 {
        if old <= self.old_start {
            old
        } else if old >= self.old_end {
            self.new_end
                .saturating_add(old.saturating_sub(self.old_end))
        } else {
            // Only divergent journal rows can point inside the replaced span;
            // they are discarded with the divergent boundary interval. Keep
            // their coordinates valid until that bounded release completes.
            self.new_end
        }
    }
}

impl SourceCursor {
    pub(crate) fn new(backing: RegisteredSource) -> Self {
        Self {
            backing,
            backing_registered: false,
            line_backing: None,
            line_backing_registered: false,
            pending_acquired_line: false,
            next_physical_offset: 0,
            next_line_number: 1,
            line: None,
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

    pub(crate) fn take_execution_state(&mut self) -> SourceCursorExecutionState {
        SourceCursorExecutionState {
            backing_registered: self.backing_registered,
            line_backing: self.line_backing.take(),
            line_backing_registered: self.line_backing_registered,
            pending_acquired_line: self.pending_acquired_line,
            next_physical_offset: self.next_physical_offset,
            next_line_number: self.next_line_number,
            line: self.line.take(),
            end_after_line: self.end_after_line,
        }
    }

    pub(crate) fn swap_execution_state(&mut self, state: &mut SourceCursorExecutionState) {
        std::mem::swap(&mut self.backing_registered, &mut state.backing_registered);
        std::mem::swap(&mut self.line_backing, &mut state.line_backing);
        std::mem::swap(
            &mut self.line_backing_registered,
            &mut state.line_backing_registered,
        );
        std::mem::swap(
            &mut self.pending_acquired_line,
            &mut state.pending_acquired_line,
        );
        std::mem::swap(
            &mut self.next_physical_offset,
            &mut state.next_physical_offset,
        );
        std::mem::swap(&mut self.next_line_number, &mut state.next_line_number);
        std::mem::swap(&mut self.line, &mut state.line);
        std::mem::swap(&mut self.end_after_line, &mut state.end_after_line);
    }

    pub(crate) fn rehome_offsets(&mut self, map: SourceOffsetMap) {
        self.next_physical_offset = map.map(self.next_physical_offset);
        if self.line_backing.is_none()
            && let Some(line) = self.line.as_mut()
        {
            line.rehome_offsets(map);
        }
    }
}

impl SourceCursorExecutionState {
    pub(crate) fn rehome_offsets(&mut self, map: SourceOffsetMap) {
        self.next_physical_offset = map.map(self.next_physical_offset);
        if self.line_backing.is_none()
            && let Some(line) = self.line.as_mut()
        {
            line.rehome_offsets(map);
        }
    }
}

#[cfg(test)]
mod tests;
