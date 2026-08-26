//! Bounded session-local interning for control-sequence names and token spellings.
//!
//! An [`Interner`] is the storage owner for one engine session's interning
//! epoch. Entries are append-only until the whole epoch is retired. In
//! particular, TeX groups, failed commands, and incremental revision rollback
//! do not expose a cursor which can truncate this storage.

use ahash::{AHashMap, AHasher};
use std::cell::Cell;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};

/// Maximum number of interner slots representable in a packed token word.
pub const SYMBOL_CAPACITY: u32 = 1 << 30;

static NEXT_SESSION_EPOCH: AtomicU64 = AtomicU64::new(1);

/// The TeX82 control-sequence namespace containing an interned symbol.
///
/// Active characters and escaped names have distinct meanings even when
/// their printed spelling is the same (for example active `~` and `\~`).
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ControlSequenceKind {
    /// TeX82 §222's unique `null_cs` slot.
    Null,
    /// TeX82 §222's `single_base + c` slot for an escaped character.
    SingleCharacter,
    /// A name scanned after an escape character or manufactured by `\csname`.
    Named,
    /// A character whose current category code is active.
    ActiveCharacter,
    /// An inaccessible engine-owned fixed `eqtb` slot, outside §259's hash.
    Internal,
}

/// A compact slot coordinate stored in tokens and dense state.
///
/// This value deliberately contains no process-global identity. It is valid
/// only while carried by storage owned by the same session epoch. APIs which
/// admit values across an owner boundary use [`SymbolId`] instead.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Symbol(u32);

/// A session-qualified control-sequence identity.
///
/// The epoch component is private, so callers cannot forge admission into a
/// different session from a raw slot number.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SymbolId {
    epoch: SessionEpochKey,
    symbol: Symbol,
}

/// A session-qualified spelling which is not a control sequence.
///
/// Retained token spellings share the epoch's byte arena and total slot budget
/// with control-sequence names, but do not consume a control-sequence-name
/// slot or enter TeX82's `hash` namespace.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SpellingId {
    epoch: SessionEpochKey,
    slot: u32,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct SessionEpochKey(u64);

impl SessionEpochKey {
    fn fresh() -> Self {
        let raw = NEXT_SESSION_EPOCH
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
                next.checked_add(1)
            })
            .expect("session epoch identity space exhausted");
        Self(raw)
    }
}

impl Symbol {
    /// Reconstructs a compact coordinate from an already validated packed
    /// token word. Admission into a session still occurs at the aggregate
    /// state boundary.
    #[must_use]
    pub(crate) const fn from_packed_slot(slot: u32) -> Self {
        Self(slot)
    }

    /// Returns the compact slot for same-session packed storage.
    #[must_use]
    pub(crate) const fn raw(self) -> u32 {
        self.0
    }
}

impl SymbolId {
    const fn new(epoch: SessionEpochKey, slot: u32) -> Self {
        Self {
            epoch,
            symbol: Symbol(slot),
        }
    }

    /// Returns the compact same-session coordinate.
    #[must_use]
    pub const fn symbol(self) -> Symbol {
        self.symbol
    }

    #[must_use]
    pub(crate) const fn raw(self) -> u32 {
        self.symbol.0
    }
}

impl SpellingId {
    const fn new(epoch: SessionEpochKey, slot: u32) -> Self {
        Self { epoch, slot }
    }

    #[must_use]
    pub(crate) const fn raw(self) -> u32 {
        self.slot
    }
}

/// One independently enforced interning-budget dimension.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InternerResource {
    /// Distinct control-sequence entries, excluding retained spellings.
    ControlSequenceNames,
    /// All dense entries, including retained spellings.
    Slots,
    /// UTF-8 bytes retained by all entries.
    Bytes,
}

/// Invalid static configuration for a session interning epoch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InternerBudgetError {
    /// The configured slot count does not fit the packed token representation.
    SlotCapacity { requested: u32, maximum: u32 },
    /// A control-sequence-name limit cannot exceed the total slot limit.
    NamesExceedSlots { names: u32, slots: u32 },
}

/// Explicit limits for one session interning epoch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InternerBudget {
    control_sequence_names: u32,
    slots: u32,
    bytes: u32,
}

impl InternerBudget {
    /// Validates and constructs a session budget.
    pub const fn new(
        control_sequence_names: u32,
        slots: u32,
        bytes: u32,
    ) -> Result<Self, InternerBudgetError> {
        if slots > SYMBOL_CAPACITY {
            return Err(InternerBudgetError::SlotCapacity {
                requested: slots,
                maximum: SYMBOL_CAPACITY,
            });
        }
        if control_sequence_names > slots {
            return Err(InternerBudgetError::NamesExceedSlots {
                names: control_sequence_names,
                slots,
            });
        }
        Ok(Self {
            control_sequence_names,
            slots,
            bytes,
        })
    }

    #[must_use]
    pub const fn control_sequence_names(self) -> u32 {
        self.control_sequence_names
    }

    #[must_use]
    pub const fn slots(self) -> u32 {
        self.slots
    }

    #[must_use]
    pub const fn bytes(self) -> u32 {
        self.bytes
    }
}

/// Current or retired resource use for one complete epoch.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct InternerUsage {
    control_sequence_names: u32,
    slots: u32,
    bytes: u32,
}

impl InternerUsage {
    #[must_use]
    pub const fn control_sequence_names(self) -> u32 {
        self.control_sequence_names
    }

    #[must_use]
    pub const fn slots(self) -> u32 {
        self.slots
    }

    #[must_use]
    pub const fn bytes(self) -> u32 {
        self.bytes
    }
}

/// Failure to append to a session epoch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InternerError {
    /// The complete epoch was already retired and cannot be reused.
    RetiredEpoch,
    /// Appending the entry would exceed one explicit session limit.
    BudgetExceeded {
        resource: InternerResource,
        limit: u32,
        attempted: u64,
    },
}

/// Failure to admit a session-qualified identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InternerAccessError {
    /// The complete epoch was retired and owns no entries now.
    RetiredEpoch,
    /// The identity belongs to a different session epoch.
    ForeignEpoch,
    /// The identity does not address an entry of the requested kind.
    InvalidIdentity,
}

/// Evidence that all storage owned by one epoch was retired together.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InternerRetirement {
    usage: InternerUsage,
}

impl InternerRetirement {
    /// Returns the resources released by retirement.
    #[must_use]
    pub const fn usage(self) -> InternerUsage {
        self.usage
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum EntryKind {
    ControlSequence(ControlSequenceKind),
    Spelling,
}

/// Exact inline key for the short names with temporal locality in command
/// delivery. Seven bytes, their length, and the complete entry namespace fit
/// in one scalar, so a hit needs neither hashing nor a byte comparison.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ShortLookup {
    key: u64,
    slot: u32,
}

#[derive(Debug)]
struct Entry {
    start: u32,
    len: u32,
    kind: EntryKind,
    #[cfg(test)]
    semantic_atom: u64,
    #[cfg(test)]
    semantic_identity: ContentHash,
}

/// TeX82's observable §259 occupancy, independent of Rust lookup layout.
///
/// The dense bits follow interner slots only so format capture can associate
/// an occupied name with its portable row. The count changes solely when a
/// real multiletter lookup first observes a slot; reuse and rollback cannot
/// decrement it (§256).
#[derive(Debug, Default)]
struct ControlSequenceLedger {
    occupied: Vec<bool>,
    count: usize,
}

impl ControlSequenceLedger {
    fn append_slot(&mut self) {
        self.occupied.push(false);
    }

    fn observe_hash_lookup(&mut self, slot: u32) {
        let occupied = &mut self.occupied[slot as usize];
        if !*occupied {
            *occupied = true;
            self.count += 1;
        }
    }

    fn contains(&self, slot: u32) -> bool {
        self.occupied.get(slot as usize).copied().unwrap_or(false)
    }

    const fn len(&self) -> usize {
        self.count
    }
}

/// Append-only interned UTF-8 storage for one bounded engine session.
///
/// The type intentionally does not implement `Clone`: continuations retain
/// the same owner, while an independent job receives a fresh epoch key.
#[derive(Debug)]
pub struct Interner {
    epoch: SessionEpochKey,
    budget: InternerBudget,
    usage: InternerUsage,
    retired: bool,
    arena: String,
    entries: Vec<Entry>,
    index: AHashMap<u64, Vec<u32>>,
    short_lookup: Cell<Option<ShortLookup>>,
    control_sequences: ControlSequenceLedger,
}

impl Interner {
    pub(crate) fn capture_format_names(&self) -> Vec<crate::format::schema::FormatName> {
        self.entries
            .iter()
            .enumerate()
            .map(|(slot, entry)| {
                let kind = match entry.kind {
                    EntryKind::ControlSequence(ControlSequenceKind::Null) => 0,
                    EntryKind::ControlSequence(ControlSequenceKind::SingleCharacter) => 1,
                    EntryKind::ControlSequence(ControlSequenceKind::Named) => 2,
                    EntryKind::ControlSequence(ControlSequenceKind::ActiveCharacter) => 3,
                    EntryKind::ControlSequence(ControlSequenceKind::Internal) => 4,
                    EntryKind::Spelling => 5,
                };
                crate::format::schema::FormatName {
                    kind,
                    hash_entry: self.control_sequences.contains(slot as u32),
                    text: self.entry_text(entry).to_owned(),
                }
            })
            .collect()
    }

    pub(crate) fn install_format_name(
        &mut self,
        expected_slot: u32,
        row: &crate::format::schema::FormatName,
    ) -> Result<Option<Symbol>, &'static str> {
        let slot = match row.kind {
            0..=4 => {
                let kind = match row.kind {
                    0 => ControlSequenceKind::Null,
                    1 => ControlSequenceKind::SingleCharacter,
                    2 => ControlSequenceKind::Named,
                    3 => ControlSequenceKind::ActiveCharacter,
                    4 => ControlSequenceKind::Internal,
                    _ => unreachable!(),
                };
                if row.hash_entry && kind != ControlSequenceKind::Named {
                    return Err("only a multiletter format name can occupy the hash");
                }
                let id = self
                    .intern_control_sequence(kind, &row.text)
                    .map_err(|_| "format name exceeds destination budget")?;
                if row.hash_entry {
                    self.control_sequences.observe_hash_lookup(id.raw());
                }
                Some(id.symbol())
            }
            5 => {
                if row.hash_entry {
                    return Err("format spelling cannot be a hash entry");
                }
                let id = self
                    .intern_spelling(&row.text)
                    .map_err(|_| "format spelling exceeds destination budget")?;
                if id.raw() != expected_slot {
                    return Err("duplicate or noncanonical format spelling");
                }
                None
            }
            _ => return Err("unknown format name kind"),
        };
        if slot.is_some_and(|slot| slot.raw() != expected_slot) {
            return Err("duplicate or noncanonical format control sequence");
        }
        Ok(slot)
    }

    /// Creates a fresh, empty session epoch under explicit limits.
    #[must_use]
    pub(crate) fn new(budget: InternerBudget) -> Self {
        Self {
            epoch: SessionEpochKey::fresh(),
            budget,
            usage: InternerUsage::default(),
            retired: false,
            arena: String::new(),
            entries: Vec::new(),
            index: AHashMap::new(),
            short_lookup: Cell::new(None),
            control_sequences: ControlSequenceLedger::default(),
        }
    }

    /// Returns the immutable limits charged by this epoch.
    #[must_use]
    pub const fn budget(&self) -> InternerBudget {
        self.budget
    }

    /// Returns current resource use. A retired epoch reports zero live use.
    #[must_use]
    pub const fn usage(&self) -> InternerUsage {
        self.usage
    }

    /// Returns whether the complete session epoch has been retired.
    #[must_use]
    pub const fn is_retired(&self) -> bool {
        self.retired
    }

    /// Interns an ordinary escaped control-sequence spelling.
    pub(crate) fn intern(&mut self, name: &str) -> Result<SymbolId, InternerError> {
        self.intern_control_sequence(named_kind(name), name)
    }

    /// Interns a name through TeX82 §259's hash-table path.
    pub(crate) fn intern_hash(&mut self, name: &str) -> Result<SymbolId, InternerError> {
        let id = self.intern(name)?;
        if self.kind_id(id).expect("newly interned symbol is admitted")
            == ControlSequenceKind::Named
        {
            self.control_sequences.observe_hash_lookup(id.raw());
        }
        Ok(id)
    }

    /// Interns an active-character control sequence.
    pub(crate) fn intern_active(&mut self, ch: char) -> Result<SymbolId, InternerError> {
        let mut encoded = [0; 4];
        self.intern_control_sequence(
            ControlSequenceKind::ActiveCharacter,
            ch.encode_utf8(&mut encoded),
        )
    }

    /// Returns an already-interned active-character control sequence.
    pub(crate) fn active(&self, ch: char) -> Option<SymbolId> {
        let mut encoded = [0; 4];
        let name = ch.encode_utf8(&mut encoded);
        self.lookup_slot(
            EntryKind::ControlSequence(ControlSequenceKind::ActiveCharacter),
            name,
        )
        .map(|slot| SymbolId::new(self.epoch, slot))
    }

    pub(crate) fn known(&self, name: &str) -> Option<SymbolId> {
        let kind = named_kind(name);
        self.lookup_slot(EntryKind::ControlSequence(kind), name)
            .or_else(|| {
                self.lookup_slot(
                    EntryKind::ControlSequence(ControlSequenceKind::Internal),
                    name,
                )
            })
            .map(|slot| SymbolId::new(self.epoch, slot))
    }

    /// Interns an inaccessible engine-owned fixed control sequence.
    pub(crate) fn intern_internal(&mut self, name: &str) -> Result<SymbolId, InternerError> {
        self.intern_control_sequence(ControlSequenceKind::Internal, name)
    }

    /// Interns a non-control-sequence token spelling in the same epoch.
    pub(crate) fn intern_spelling(&mut self, spelling: &str) -> Result<SpellingId, InternerError> {
        if self.retired {
            return Err(InternerError::RetiredEpoch);
        }
        let kind = EntryKind::Spelling;
        if let Some(slot) = self.lookup_slot(kind, spelling) {
            return Ok(SpellingId::new(self.epoch, slot));
        }
        let slot = self.append(kind, spelling)?;
        Ok(SpellingId::new(self.epoch, slot))
    }

    fn intern_control_sequence(
        &mut self,
        kind: ControlSequenceKind,
        name: &str,
    ) -> Result<SymbolId, InternerError> {
        if self.retired {
            return Err(InternerError::RetiredEpoch);
        }
        validate_character_kind(kind, name);
        let entry_kind = EntryKind::ControlSequence(kind);
        if let Some(slot) = self.lookup_slot(entry_kind, name) {
            return Ok(SymbolId::new(self.epoch, slot));
        }
        let slot = self.append(entry_kind, name)?;
        Ok(SymbolId::new(self.epoch, slot))
    }

    fn append(&mut self, kind: EntryKind, value: &str) -> Result<u32, InternerError> {
        let names = u64::from(self.usage.control_sequence_names)
            + u64::from(matches!(kind, EntryKind::ControlSequence(_)));
        let slots = u64::from(self.usage.slots) + 1;
        let bytes = u64::from(self.usage.bytes) + value.len() as u64;
        check_budget(
            InternerResource::ControlSequenceNames,
            self.budget.control_sequence_names,
            names,
        )?;
        check_budget(InternerResource::Slots, self.budget.slots, slots)?;
        check_budget(InternerResource::Bytes, self.budget.bytes, bytes)?;

        let start = self.usage.bytes;
        let len = u32::try_from(value.len()).expect("byte budget bounds each spelling to u32");
        let slot = self.usage.slots;
        #[cfg(test)]
        let semantic_atom = semantic_atom_for_entry(kind, value);
        #[cfg(test)]
        let semantic_identity = semantic_identity(kind, value);
        let hash = lookup_hash(kind, value);

        self.arena.push_str(value);
        self.entries.push(Entry {
            start,
            len,
            kind,
            #[cfg(test)]
            semantic_atom,
            #[cfg(test)]
            semantic_identity,
        });
        self.control_sequences.append_slot();
        self.index.entry(hash).or_default().push(slot);
        self.usage = InternerUsage {
            control_sequence_names: names as u32,
            slots: slots as u32,
            bytes: bytes as u32,
        };
        Ok(slot)
    }

    /// Returns an ordinary escaped control sequence without mutation.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<SymbolId> {
        (!self.retired)
            .then(|| self.lookup_slot(EntryKind::ControlSequence(named_kind(name)), name))
            .flatten()
            .map(|slot| SymbolId::new(self.epoch, slot))
    }

    /// Returns an active-character control sequence without mutation.
    #[must_use]
    pub fn get_active(&self, ch: char) -> Option<SymbolId> {
        let mut encoded = [0; 4];
        let name = ch.encode_utf8(&mut encoded);
        (!self.retired)
            .then(|| {
                self.lookup_slot(
                    EntryKind::ControlSequence(ControlSequenceKind::ActiveCharacter),
                    name,
                )
            })
            .flatten()
            .map(|slot| SymbolId::new(self.epoch, slot))
    }

    /// Returns a retained token spelling without mutation.
    #[must_use]
    pub fn get_spelling(&self, spelling: &str) -> Option<SpellingId> {
        (!self.retired)
            .then(|| self.lookup_slot(EntryKind::Spelling, spelling))
            .flatten()
            .map(|slot| SpellingId::new(self.epoch, slot))
    }

    fn lookup_slot(&self, kind: EntryKind, value: &str) -> Option<u32> {
        let short_key = short_lookup_key(kind, value);
        if let Some(key) = short_key
            && let Some(cached) = self.short_lookup.get()
            && cached.key == key
        {
            return Some(cached.slot);
        }
        let slot = self
            .index
            .get(&lookup_hash(kind, value))?
            .iter()
            .copied()
            .find(|&slot| {
                self.entries
                    .get(slot as usize)
                    .is_some_and(|entry| entry.kind == kind && self.entry_text(entry) == value)
            });
        if let (Some(key), Some(slot)) = (short_key, slot) {
            self.short_lookup.set(Some(ShortLookup { key, slot }));
        }
        slot
    }

    /// Resolves a session-qualified control-sequence identity.
    pub fn resolve_id(&self, id: SymbolId) -> Result<&str, InternerAccessError> {
        let entry = self.admit_symbol(id)?;
        Ok(self.entry_text(entry))
    }

    /// Returns the namespace of a session-qualified control-sequence identity.
    pub fn kind_id(&self, id: SymbolId) -> Result<ControlSequenceKind, InternerAccessError> {
        match self.admit_symbol(id)?.kind {
            EntryKind::ControlSequence(kind) => Ok(kind),
            EntryKind::Spelling => unreachable!("symbol admission checked the entry kind"),
        }
    }

    /// Resolves a session-qualified non-control-sequence spelling.
    pub fn resolve_spelling(&self, id: SpellingId) -> Result<&str, InternerAccessError> {
        let entry = self.admit_spelling(id)?;
        Ok(self.entry_text(entry))
    }

    /// Returns whether this exact identity is admitted by the epoch.
    #[must_use]
    pub fn contains_id(&self, id: SymbolId) -> bool {
        self.admit_symbol(id).is_ok()
    }

    /// Returns whether this exact spelling identity is admitted by the epoch.
    #[must_use]
    pub fn contains_spelling(&self, id: SpellingId) -> bool {
        self.admit_spelling(id).is_ok()
    }

    fn admit_symbol(&self, id: SymbolId) -> Result<&Entry, InternerAccessError> {
        self.admit_epoch(id.epoch)?;
        self.entries
            .get(id.raw() as usize)
            .filter(|entry| matches!(entry.kind, EntryKind::ControlSequence(_)))
            .ok_or(InternerAccessError::InvalidIdentity)
    }

    fn admit_spelling(&self, id: SpellingId) -> Result<&Entry, InternerAccessError> {
        self.admit_epoch(id.epoch)?;
        self.entries
            .get(id.raw() as usize)
            .filter(|entry| entry.kind == EntryKind::Spelling)
            .ok_or(InternerAccessError::InvalidIdentity)
    }

    fn admit_epoch(&self, epoch: SessionEpochKey) -> Result<(), InternerAccessError> {
        if self.retired {
            Err(InternerAccessError::RetiredEpoch)
        } else if epoch != self.epoch {
            Err(InternerAccessError::ForeignEpoch)
        } else {
            Ok(())
        }
    }

    fn entry_text<'a>(&'a self, entry: &Entry) -> &'a str {
        let start = entry.start as usize;
        let end = start + entry.len as usize;
        &self.arena[start..end]
    }

    /// Admits a compact coordinate already owned by this session aggregate.
    pub(crate) fn resolve_local(&self, symbol: Symbol) -> Option<&str> {
        if self.retired {
            return None;
        }
        let entry = self.entries.get(symbol.raw() as usize)?;
        matches!(entry.kind, EntryKind::ControlSequence(_)).then(|| self.entry_text(entry))
    }

    /// Returns the session-qualified identity for a local compact coordinate.
    pub(crate) fn qualify_local(&self, symbol: Symbol) -> Option<SymbolId> {
        self.resolve_local(symbol)
            .map(|_| SymbolId::new(self.epoch, symbol.raw()))
    }

    /// Returns the control-sequence identity at a dense epoch slot.
    pub(crate) fn symbol_at_slot(&self, slot: u32) -> Option<SymbolId> {
        self.qualify_local(Symbol(slot))
    }

    /// Returns whether this identity owns a TeX82 §259 hash entry.
    #[cfg(test)]
    pub(crate) fn is_hash_entry(&self, id: SymbolId) -> Result<bool, InternerAccessError> {
        self.admit_symbol(id)?;
        Ok(self.control_sequences.contains(id.raw()))
    }

    /// Returns TeX82's occupied `hash` entries for the §1334 usage summary.
    #[must_use]
    pub(crate) fn multiletter_len(&self) -> usize {
        if self.retired {
            return 0;
        }
        self.control_sequences.len()
    }

    /// Returns the number of live slots, including retained spellings.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns whether the live epoch contains no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[cfg(test)]
    pub(crate) fn semantic_atom(&self, symbol: Symbol) -> Option<u64> {
        let entry = self.entries.get(symbol.raw() as usize)?;
        matches!(entry.kind, EntryKind::ControlSequence(_)).then_some(entry.semantic_atom)
    }

    #[cfg(test)]
    pub(crate) fn semantic_atom_identity(&self, symbol: Symbol) -> Option<(u64, ContentHash)> {
        let entry = self.entries.get(symbol.raw() as usize)?;
        matches!(entry.kind, EntryKind::ControlSequence(_))
            .then_some((entry.semantic_atom, entry.semantic_identity))
    }

    /// Retires every name, spelling, lookup entry, and retained byte together.
    ///
    /// Capacity is not retained because retirement is the daemon memory bound,
    /// not a rollback or scratch-pool operation.
    pub fn retire(&mut self) -> Result<InternerRetirement, InternerError> {
        if self.retired {
            return Err(InternerError::RetiredEpoch);
        }
        let usage = self.usage;
        self.arena = String::new();
        self.entries = Vec::new();
        self.index = AHashMap::new();
        self.short_lookup.set(None);
        self.control_sequences = ControlSequenceLedger::default();
        self.usage = InternerUsage::default();
        self.retired = true;
        Ok(InternerRetirement { usage })
    }
}

#[cfg(test)]
const fn entry_tag(kind: EntryKind) -> u8 {
    match kind {
        EntryKind::ControlSequence(ControlSequenceKind::Null)
        | EntryKind::ControlSequence(ControlSequenceKind::SingleCharacter)
        | EntryKind::ControlSequence(ControlSequenceKind::Named) => 0,
        EntryKind::ControlSequence(ControlSequenceKind::ActiveCharacter) => 1,
        EntryKind::ControlSequence(ControlSequenceKind::Internal) => 2,
        EntryKind::Spelling => 3,
    }
}

#[cfg(test)]
fn semantic_atom_for_entry(kind: EntryKind, value: &str) -> u64 {
    let mut hasher = StateHasher::new(0x6373_5f61_746f_6d31);
    hasher.u8(entry_tag(kind));
    hasher.str(value);
    hasher.finish()
}

#[cfg(test)]
fn semantic_identity(kind: EntryKind, value: &str) -> ContentHash {
    let mut bytes = Vec::with_capacity(value.len() + 1);
    bytes.push(entry_tag(kind));
    bytes.extend_from_slice(value.as_bytes());
    crate::state_hash::semantic_identity_bytes(b"umber-session-interner-v1", &bytes)
}

fn check_budget(
    resource: InternerResource,
    limit: u32,
    attempted: u64,
) -> Result<(), InternerError> {
    if attempted > u64::from(limit) {
        Err(InternerError::BudgetExceeded {
            resource,
            limit,
            attempted,
        })
    } else {
        Ok(())
    }
}

fn validate_character_kind(kind: ControlSequenceKind, name: &str) {
    if matches!(kind, ControlSequenceKind::ActiveCharacter) {
        let mut chars = name.chars();
        assert!(
            chars.next().is_some() && chars.next().is_none(),
            "active control sequence must contain exactly one character"
        );
    }
}

/// Selects TeX82 §222's fixed control-sequence namespace from its spelling.
#[must_use]
pub(crate) fn named_kind(name: &str) -> ControlSequenceKind {
    match name.chars().count() {
        0 => ControlSequenceKind::Null,
        1 => ControlSequenceKind::SingleCharacter,
        _ => ControlSequenceKind::Named,
    }
}

fn lookup_hash(kind: EntryKind, value: &str) -> u64 {
    let mut hasher = AHasher::default();
    kind.hash(&mut hasher);
    value.hash(&mut hasher);
    hasher.finish()
}

fn short_lookup_key(kind: EntryKind, value: &str) -> Option<u64> {
    let bytes = value.as_bytes();
    if bytes.len() > 7 {
        return None;
    }
    let tag = match kind {
        EntryKind::ControlSequence(ControlSequenceKind::Null) => 0,
        EntryKind::ControlSequence(ControlSequenceKind::SingleCharacter) => 1,
        EntryKind::ControlSequence(ControlSequenceKind::Named) => 2,
        EntryKind::ControlSequence(ControlSequenceKind::ActiveCharacter) => 3,
        EntryKind::ControlSequence(ControlSequenceKind::Internal) => 4,
        EntryKind::Spelling => 5,
    };
    let mut key = tag | (bytes.len() as u64) << 3;
    for (index, &byte) in bytes.iter().enumerate() {
        key |= u64::from(byte) << (8 + index * 8);
    }
    Some(key)
}

#[cfg(test)]
#[path = "interner/tests.rs"]
mod tests;
#[cfg(test)]
use crate::ContentHash;
#[cfg(test)]
use crate::state_hash::StateHasher;
