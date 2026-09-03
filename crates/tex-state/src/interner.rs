//! Bounded session-local interning for control-sequence names and token spellings.
//!
//! An [`Interner`] owns one append-only session epoch. Canonical UTF-8 bytes
//! and packed reverse rows are reserved from the selected engine profile at
//! construction, and a fixed open-addressed index provides forward lookup
//! without storing a second copy of any spelling.

use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(test)]
use crate::state_hash::StateHasher;

/// Maximum number of interner slots representable in a packed token word.
pub const SYMBOL_CAPACITY: u32 = 1 << 30;

static NEXT_SESSION_EPOCH: AtomicU64 = AtomicU64::new(1);

/// The TeX82 control-sequence namespace containing an interned symbol.
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
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Symbol(u32);

/// A session-qualified control-sequence identity.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SymbolId {
    epoch: SessionEpochKey,
    symbol: Symbol,
}

/// A session-qualified spelling which is not a control sequence.
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
    /// token word.
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
    /// Occupied TeX82 multiletter hash entries.
    HashEntries,
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
    /// A profile-derived capacity does not fit the compact Symbol domain.
    ProfileCapacityOverflow,
}

/// Explicit limits for one session interning epoch.
///
/// The generic constructor remains available for focused unit fixtures. Engine
/// construction uses [`Self::from_profile`], so executable capacities are the
/// only production owner of interner limits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InternerBudget {
    control_sequence_names: u32,
    hash_entries: u32,
    slots: u32,
    bytes: u32,
}

impl InternerBudget {
    /// Validates and constructs a session budget for a focused fixture.
    pub const fn new(
        control_sequence_names: u32,
        slots: u32,
        bytes: u32,
    ) -> Result<Self, InternerBudgetError> {
        Self::with_hash_entries(control_sequence_names, control_sequence_names, slots, bytes)
    }

    const fn with_hash_entries(
        control_sequence_names: u32,
        hash_entries: u32,
        slots: u32,
        bytes: u32,
    ) -> Result<Self, InternerBudgetError> {
        if slots > SYMBOL_CAPACITY {
            return Err(InternerBudgetError::SlotCapacity {
                requested: slots,
                maximum: SYMBOL_CAPACITY,
            });
        }
        if control_sequence_names > slots || hash_entries > control_sequence_names {
            return Err(InternerBudgetError::NamesExceedSlots {
                names: control_sequence_names,
                slots,
            });
        }
        Ok(Self {
            control_sequence_names,
            hash_entries,
            slots,
            bytes,
        })
    }

    /// Derives all interner limits from one executable process profile.
    pub fn from_profile(
        profile: crate::EngineCapacityProfile,
    ) -> Result<Self, InternerBudgetError> {
        let capacities = profile.configuration();
        let hash_entries = u32::try_from(capacities.hash_entries())
            .map_err(|_| InternerBudgetError::ProfileCapacityOverflow)?;
        let control_sequence_names = u32::try_from(capacities.interner_control_sequence_capacity())
            .map_err(|_| InternerBudgetError::ProfileCapacityOverflow)?;
        let slots = u32::try_from(capacities.interner_slot_capacity())
            .map_err(|_| InternerBudgetError::ProfileCapacityOverflow)?;
        let bytes = u32::try_from(capacities.interner_byte_capacity())
            .map_err(|_| InternerBudgetError::ProfileCapacityOverflow)?;
        Self::with_hash_entries(control_sequence_names, hash_entries, slots, bytes)
    }

    #[must_use]
    pub const fn control_sequence_names(self) -> u32 {
        self.control_sequence_names
    }

    #[must_use]
    pub const fn hash_entries(self) -> u32 {
        self.hash_entries
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
    /// A spelling cannot be represented by the packed NameRecord length.
    NameTooLong { length: u64, maximum: u32 },
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EntryKind {
    ControlSequence(ControlSequenceKind),
    Spelling,
}

/// Exact inline key for short names with temporal locality in command
/// delivery. It owns no spelling bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ShortLookup {
    key: u64,
    slot: u32,
}

/// One packed reverse row stored once at its dense Symbol index.
///
/// The row contains only the byte-pool coordinates. Namespace and hash
/// occupancy metadata live in the fixed spelling index control byte, keeping
/// this row solely responsible for the persistent Symbol-to-name mapping.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct NameRecord {
    offset: u32,
    len: u32,
}

const _: () = assert!(core::mem::size_of::<NameRecord>() == core::mem::size_of::<u64>());

const NAME_RECORD_MAX_LENGTH: usize = u32::MAX as usize;

impl NameRecord {
    fn new(offset: usize, length: usize) -> Option<Self> {
        Some(Self {
            offset: u32::try_from(offset).ok()?,
            len: u32::try_from(length).ok()?,
        })
    }

    #[must_use]
    fn offset(self) -> usize {
        usize::try_from(self.offset).expect("u32 offset fits native usize")
    }

    #[must_use]
    fn len(self) -> usize {
        usize::try_from(self.len).expect("u32 length fits native usize")
    }
}

/// One profile-bounded contiguous UTF-8 byte owner.
#[derive(Debug, Default)]
struct BytePool {
    bytes: Vec<u8>,
}

impl BytePool {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(capacity),
        }
    }

    fn append(&mut self, value: &[u8]) -> Option<usize> {
        let offset = self.bytes.len();
        let end = offset.checked_add(value.len())?;
        if end > self.bytes.capacity() {
            return None;
        }
        self.bytes.extend_from_slice(value);
        Some(offset)
    }

    fn value(&self, record: NameRecord) -> Option<&[u8]> {
        let end = record.offset().checked_add(record.len())?;
        (end <= self.bytes.len()).then(|| &self.bytes[record.offset()..end])
    }

    #[cfg(test)]
    fn capacity(&self) -> usize {
        self.bytes.capacity()
    }
}

/// One fixed open-addressed name-index bucket.
///
/// The low 32 bits carry the Symbol and the high byte carries compact control
/// metadata: a four-bit spelling fingerprint, a three-bit namespace tag, and
/// one TeX82 hash-occupancy bit. A zero word is empty, so the entire index is
/// a single compact array with no duplicated key bytes or metadata sidecar.
#[derive(Clone, Copy, Debug, Default)]
struct IndexBucket(u64);

const INDEX_FINGERPRINT_MASK: u8 = 0x0f;
const INDEX_KIND_SHIFT: u8 = 4;
const INDEX_KIND_MASK: u8 = 0x07;
const INDEX_HASH_BIT: u8 = 0x80;
const INDEX_CONTROL_MASK: u8 = !INDEX_HASH_BIT;

impl IndexBucket {
    #[must_use]
    const fn empty(self) -> bool {
        self.0 == 0
    }

    #[must_use]
    const fn control(self) -> u8 {
        (self.0 >> 32) as u8
    }

    #[must_use]
    const fn symbol(self) -> u32 {
        self.0 as u32
    }

    #[must_use]
    const fn occupied(control: u8, symbol: u32) -> Self {
        Self(((control as u64) << 32) | (symbol as u64))
    }

    #[must_use]
    const fn kind(self) -> Option<EntryKind> {
        entry_kind_from_tag((self.control() >> INDEX_KIND_SHIFT) & INDEX_KIND_MASK)
    }

    #[must_use]
    const fn hash_entry(self) -> bool {
        self.control() & INDEX_HASH_BIT != 0
    }

    #[must_use]
    const fn without_hash_entry(self) -> u8 {
        self.control() & INDEX_CONTROL_MASK
    }
}

#[derive(Debug)]
struct FixedIndex {
    buckets: Vec<IndexBucket>,
    mask: usize,
    len: usize,
    usable: usize,
}

impl FixedIndex {
    fn with_capacity(slot_capacity: usize) -> Self {
        let bucket_count = index_bucket_count(slot_capacity);
        Self {
            buckets: vec![IndexBucket::default(); bucket_count],
            mask: bucket_count - 1,
            len: 0,
            usable: bucket_count - bucket_count / 4,
        }
    }

    fn empty() -> Self {
        Self {
            buckets: Vec::new(),
            mask: 0,
            len: 0,
            usable: 0,
        }
    }

    #[cfg(test)]
    fn capacity(&self) -> usize {
        self.buckets.capacity()
    }

    fn can_insert(&self, additional: usize) -> bool {
        self.len
            .checked_add(additional)
            .is_some_and(|required| required <= self.usable)
    }

    fn mark_hash_entry_at(&mut self, bucket: usize) -> bool {
        let bucket = &mut self.buckets[bucket];
        if bucket.hash_entry() {
            return false;
        }
        bucket.0 |= u64::from(INDEX_HASH_BIT) << 32;
        true
    }

    fn insert(&mut self, hash: u64, kind: EntryKind, symbol: u32, hash_entry: bool) {
        debug_assert!(self.can_insert(1));
        let control = index_control(hash, kind, hash_entry);
        let mut bucket = (hash as usize) & self.mask;
        loop {
            if self.buckets[bucket].empty() {
                self.buckets[bucket] = IndexBucket::occupied(control, symbol);
                self.len += 1;
                return;
            }
            bucket = (bucket + 1) & self.mask;
        }
    }
}

/// Append-only session interner with profile-bounded canonical storage.
#[derive(Debug)]
pub struct Interner {
    epoch: SessionEpochKey,
    budget: InternerBudget,
    physical_budget: InternerBudget,
    usage: InternerUsage,
    hash_entries: usize,
    retired: bool,
    /// Canonical UTF-8 storage; capacity is reserved once and never grows.
    arena: BytePool,
    /// One packed NameRecord per dense Symbol; capacity is reserved once.
    entries: Vec<NameRecord>,
    index: FixedIndex,
    short_lookup: Cell<Option<ShortLookup>>,
    profile: Option<crate::EngineCapacityProfile>,
}

impl Interner {
    pub(crate) const fn epoch_identity(&self) -> u64 {
        self.epoch.0
    }

    pub(crate) fn capture_format_names(&self) -> Vec<crate::format::schema::FormatName> {
        let mut names: Vec<_> = self
            .entries
            .iter()
            .copied()
            .map(|record| crate::format::schema::FormatName {
                kind: 5,
                hash_entry: false,
                text: self.entry_text(record).to_owned(),
            })
            .collect();
        // The fixed index is the only metadata owner besides the reverse
        // rows. Fill the already-required format output by dense Symbol slot,
        // preserving wire order without adding a persistent sidecar.
        for bucket in self.index.buckets.iter().copied() {
            if bucket.empty() {
                continue;
            }
            let slot = usize::try_from(bucket.symbol()).expect("Symbol fits native usize");
            let row = names
                .get_mut(slot)
                .expect("every live NameRecord has one index bucket");
            row.kind = format_kind(bucket.kind());
            row.hash_entry = bucket.hash_entry();
        }
        names
    }

    /// Checks a complete format name table before the first destination row is
    /// appended. The check includes rows, control names, hash occupancy, and
    /// UTF-8 bytes, so installation cannot encounter a capacity failure after
    /// mutating the canonical prefix.
    pub(crate) fn preflight_format_names(
        &self,
        rows: &[crate::format::schema::FormatName],
    ) -> Result<(), InternerError> {
        let mut names = u64::from(self.usage.control_sequence_names);
        let mut bytes = u64::from(self.usage.bytes);
        let mut hash_entries = self.hash_entries as u64;
        let mut additional = 0_usize;
        for row in rows {
            let kind = match row.kind {
                0 => EntryKind::ControlSequence(ControlSequenceKind::Null),
                1 => EntryKind::ControlSequence(ControlSequenceKind::SingleCharacter),
                2 => EntryKind::ControlSequence(ControlSequenceKind::Named),
                3 => EntryKind::ControlSequence(ControlSequenceKind::ActiveCharacter),
                4 => EntryKind::ControlSequence(ControlSequenceKind::Internal),
                5 => EntryKind::Spelling,
                _ => continue,
            };
            let bucket = self.lookup_bucket(kind, &row.text);
            let missing = bucket.is_none();
            if missing {
                additional = additional
                    .checked_add(1)
                    .ok_or(InternerError::BudgetExceeded {
                        resource: InternerResource::Slots,
                        limit: self.budget.slots,
                        attempted: u64::MAX,
                    })?;
                names = names
                    .checked_add(u64::from(matches!(kind, EntryKind::ControlSequence(_))))
                    .ok_or(InternerError::BudgetExceeded {
                        resource: InternerResource::ControlSequenceNames,
                        limit: self.budget.control_sequence_names,
                        attempted: u64::MAX,
                    })?;
                bytes = bytes
                    .checked_add(u64::try_from(row.text.len()).expect("usize fits in u64"))
                    .ok_or(InternerError::BudgetExceeded {
                        resource: InternerResource::Bytes,
                        limit: self.budget.bytes,
                        attempted: u64::MAX,
                    })?;
            }
            if row.hash_entry
                && bucket.is_none_or(|bucket| !self.index.buckets[bucket].hash_entry())
            {
                hash_entries =
                    hash_entries
                        .checked_add(1)
                        .ok_or(InternerError::BudgetExceeded {
                            resource: InternerResource::HashEntries,
                            limit: self.budget.hash_entries,
                            attempted: u64::MAX,
                        })?;
            }
        }
        let slots = u64::from(self.usage.slots)
            .checked_add(u64::try_from(additional).expect("usize fits in u64"))
            .ok_or(InternerError::BudgetExceeded {
                resource: InternerResource::Slots,
                limit: self.budget.slots,
                attempted: u64::MAX,
            })?;
        check_budget(
            InternerResource::ControlSequenceNames,
            self.budget.control_sequence_names,
            names,
        )?;
        check_budget(
            InternerResource::HashEntries,
            self.budget.hash_entries,
            hash_entries,
        )?;
        check_budget(InternerResource::Slots, self.budget.slots, slots)?;
        check_budget(InternerResource::Bytes, self.budget.bytes, bytes)?;
        if let Some(row) = rows
            .iter()
            .find(|row| row.text.len() > NAME_RECORD_MAX_LENGTH)
        {
            return Err(InternerError::NameTooLong {
                length: u64::try_from(row.text.len()).expect("usize fits in u64"),
                maximum: NAME_RECORD_MAX_LENGTH as u32,
            });
        }
        if !self.index.can_insert(additional) {
            return Err(InternerError::BudgetExceeded {
                resource: InternerResource::Slots,
                limit: self.budget.slots,
                attempted: slots,
            });
        }
        Ok(())
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
                let id = if row.hash_entry {
                    self.intern_hash_kind_with_status(kind, &row.text)
                        .map_err(|_| "format name exceeds destination budget")?
                        .0
                } else {
                    self.intern_control_sequence(kind, &row.text)
                        .map_err(|_| "format name exceeds destination budget")?
                };
                if row.hash_entry && id.raw() != expected_slot {
                    return Err("duplicate or noncanonical format control sequence");
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

    /// Creates a fresh, empty session epoch under explicit fixture limits.
    #[must_use]
    pub(crate) fn new(budget: InternerBudget) -> Self {
        Self::new_with_budgets(budget, budget, None)
    }

    /// Creates a fresh interner whose limits are owned by one executable
    /// profile. Storage is reserved once from that profile.
    pub(crate) fn new_for_profile(profile: crate::EngineCapacityProfile) -> Self {
        let budget = InternerBudget::from_profile(profile)
            .expect("pinned engine profile fits the compact interner domain");
        Self::new_with_budgets(budget, budget, Some(profile))
    }

    fn new_with_budgets(
        budget: InternerBudget,
        physical_budget: InternerBudget,
        profile: Option<crate::EngineCapacityProfile>,
    ) -> Self {
        let slot_capacity = usize::try_from(physical_budget.slots)
            .expect("profile Symbol capacity fits native usize");
        let byte_capacity = usize::try_from(physical_budget.bytes)
            .expect("profile byte capacity fits native usize");
        Self {
            epoch: SessionEpochKey::fresh(),
            budget,
            physical_budget,
            usage: InternerUsage::default(),
            hash_entries: 0,
            retired: false,
            arena: BytePool::with_capacity(byte_capacity),
            entries: Vec::with_capacity(slot_capacity),
            index: FixedIndex::with_capacity(slot_capacity),
            short_lookup: Cell::new(None),
            profile,
        }
    }

    /// Returns immutable limits enforced by this epoch.
    #[must_use]
    pub const fn budget(&self) -> InternerBudget {
        self.budget
    }

    /// Returns the executable profile currently enforcing interner limits.
    #[must_use]
    pub const fn capacity_profile(&self) -> Option<crate::EngineCapacityProfile> {
        self.profile
    }

    /// Selects a process profile without replacing canonical storage.
    pub(crate) fn select_capacity_profile(
        &mut self,
        profile: crate::EngineCapacityProfile,
    ) -> Result<(), InternerError> {
        if self.retired {
            return Err(InternerError::RetiredEpoch);
        }
        let requested = InternerBudget::from_profile(profile)
            .expect("pinned engine profile fits the compact interner domain");
        let budget = InternerBudget::with_hash_entries(
            self.physical_budget
                .control_sequence_names
                .min(requested.control_sequence_names),
            self.physical_budget
                .hash_entries
                .min(requested.hash_entries),
            self.physical_budget.slots.min(requested.slots),
            self.physical_budget.bytes.min(requested.bytes),
        )
        .expect("physical interner capacity dominates selected profile");
        check_budget(
            InternerResource::ControlSequenceNames,
            budget.control_sequence_names,
            u64::from(self.usage.control_sequence_names),
        )?;
        check_budget(
            InternerResource::HashEntries,
            budget.hash_entries,
            self.hash_entries as u64,
        )?;
        check_budget(
            InternerResource::Slots,
            budget.slots,
            u64::from(self.usage.slots),
        )?;
        check_budget(
            InternerResource::Bytes,
            budget.bytes,
            u64::from(self.usage.bytes),
        )?;
        self.budget = budget;
        self.profile = Some(profile);
        Ok(())
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
        self.intern_with_status(name).map(|(id, _)| id)
    }

    /// Interns an ordinary escaped control-sequence spelling and reports
    /// whether this call appended its stable identity.
    pub(crate) fn intern_with_status(
        &mut self,
        name: &str,
    ) -> Result<(SymbolId, bool), InternerError> {
        self.intern_control_sequence_with_status(named_kind(name), name)
    }

    /// Interns a name through TeX82 §259's hash-table path.
    #[cfg(test)]
    pub(crate) fn intern_hash(&mut self, name: &str) -> Result<SymbolId, InternerError> {
        self.intern_hash_with_status(name).map(|(id, _)| id)
    }

    /// Performs one TeX82 §259 lookup, appending only on a miss, and reports
    /// whether the stable identity was new.
    pub(crate) fn intern_hash_with_status(
        &mut self,
        name: &str,
    ) -> Result<(SymbolId, bool), InternerError> {
        self.intern_hash_kind_with_status(named_kind(name), name)
    }

    fn intern_hash_kind_with_status(
        &mut self,
        kind: ControlSequenceKind,
        name: &str,
    ) -> Result<(SymbolId, bool), InternerError> {
        if kind == ControlSequenceKind::Named {
            let entry_kind = EntryKind::ControlSequence(kind);
            if let Some(bucket) = self.lookup_bucket(entry_kind, name) {
                let slot = self.index.buckets[bucket].symbol();
                if !self.index.buckets[bucket].hash_entry() {
                    check_budget(
                        InternerResource::HashEntries,
                        self.budget.hash_entries,
                        self.hash_entries as u64 + 1,
                    )?;
                    assert!(
                        self.index.mark_hash_entry_at(bucket),
                        "hash observation addresses an unoccupied index bucket"
                    );
                    self.hash_entries += 1;
                }
                return Ok((SymbolId::new(self.epoch, slot), false));
            } else {
                check_budget(
                    InternerResource::HashEntries,
                    self.budget.hash_entries,
                    self.hash_entries as u64 + 1,
                )?;
                let slot = self.append(entry_kind, name, true)?;
                self.hash_entries += 1;
                return Ok((SymbolId::new(self.epoch, slot), true));
            }
        }
        self.intern_control_sequence_with_status(kind, name)
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
        let slot = self.append(kind, spelling, false)?;
        Ok(SpellingId::new(self.epoch, slot))
    }

    fn intern_control_sequence(
        &mut self,
        kind: ControlSequenceKind,
        name: &str,
    ) -> Result<SymbolId, InternerError> {
        self.intern_control_sequence_with_status(kind, name)
            .map(|(id, _)| id)
    }

    fn intern_control_sequence_with_status(
        &mut self,
        kind: ControlSequenceKind,
        name: &str,
    ) -> Result<(SymbolId, bool), InternerError> {
        if self.retired {
            return Err(InternerError::RetiredEpoch);
        }
        validate_character_kind(kind, name);
        let entry_kind = EntryKind::ControlSequence(kind);
        if let Some(slot) = self.lookup_slot(entry_kind, name) {
            return Ok((SymbolId::new(self.epoch, slot), false));
        }
        let slot = self.append(entry_kind, name, false)?;
        Ok((SymbolId::new(self.epoch, slot), true))
    }

    fn append(
        &mut self,
        kind: EntryKind,
        value: &str,
        hash_entry: bool,
    ) -> Result<u32, InternerError> {
        let names = u64::from(self.usage.control_sequence_names)
            + u64::from(matches!(kind, EntryKind::ControlSequence(_)));
        let slots = u64::from(self.usage.slots) + 1;
        let value_len = u64::try_from(value.len()).expect("usize fits in u64");
        let bytes = u64::from(self.usage.bytes).checked_add(value_len).ok_or(
            InternerError::BudgetExceeded {
                resource: InternerResource::Bytes,
                limit: self.budget.bytes,
                attempted: u64::MAX,
            },
        )?;
        check_budget(
            InternerResource::ControlSequenceNames,
            self.budget.control_sequence_names,
            names,
        )?;
        check_budget(InternerResource::Slots, self.budget.slots, slots)?;
        check_budget(InternerResource::Bytes, self.budget.bytes, bytes)?;
        if value.len() > NAME_RECORD_MAX_LENGTH {
            return Err(InternerError::NameTooLong {
                length: value_len,
                maximum: NAME_RECORD_MAX_LENGTH as u32,
            });
        }
        if self.entries.len() >= self.entries.capacity() || !self.index.can_insert(1) {
            return Err(InternerError::BudgetExceeded {
                resource: InternerResource::Slots,
                limit: self.budget.slots,
                attempted: slots,
            });
        }
        let offset = self
            .arena
            .append(value.as_bytes())
            .ok_or(InternerError::BudgetExceeded {
                resource: InternerResource::Bytes,
                limit: self.budget.bytes,
                attempted: bytes,
            })?;
        let record = NameRecord::new(offset, value.len()).ok_or(InternerError::NameTooLong {
            length: value_len,
            maximum: NAME_RECORD_MAX_LENGTH as u32,
        })?;
        let slot = self.usage.slots;
        self.entries.push(record);
        self.index
            .insert(lookup_hash(kind, value), kind, slot, hash_entry);
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

    fn lookup_bucket(&self, kind: EntryKind, value: &str) -> Option<usize> {
        if self.index.buckets.is_empty() {
            return None;
        }
        let hash = lookup_hash(kind, value);
        let fingerprint = index_control(hash, kind, false);
        let mut bucket = (hash as usize) & self.index.mask;
        loop {
            let candidate = self.index.buckets[bucket];
            if candidate.empty() {
                break None;
            }
            if candidate.without_hash_entry() == fingerprint {
                let candidate_slot = candidate.symbol();
                if self
                    .entries
                    .get(usize::try_from(candidate_slot).expect("Symbol fits native usize"))
                    .is_some_and(|record| self.entry_text(*record) == value)
                {
                    break Some(bucket);
                }
            }
            bucket = (bucket + 1) & self.index.mask;
        }
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
            .lookup_bucket(kind, value)
            .map(|bucket| self.index.buckets[bucket].symbol());
        if let (Some(key), Some(slot)) = (short_key, slot) {
            self.short_lookup.set(Some(ShortLookup { key, slot }));
        }
        slot
    }

    fn metadata_for_symbol(&self, symbol: u32) -> Option<(EntryKind, bool)> {
        let record = self
            .entries
            .get(usize::try_from(symbol).expect("Symbol fits native usize"))
            .copied()?;
        let value = self.entry_text(record);
        [
            EntryKind::ControlSequence(ControlSequenceKind::Null),
            EntryKind::ControlSequence(ControlSequenceKind::SingleCharacter),
            EntryKind::ControlSequence(ControlSequenceKind::Named),
            EntryKind::ControlSequence(ControlSequenceKind::ActiveCharacter),
            EntryKind::ControlSequence(ControlSequenceKind::Internal),
            EntryKind::Spelling,
        ]
        .into_iter()
        .find_map(|kind| {
            let bucket = self.lookup_bucket(kind, value)?;
            let candidate = self.index.buckets[bucket];
            (candidate.symbol() == symbol).then_some((kind, candidate.hash_entry()))
        })
    }

    /// Resolves a session-qualified control-sequence identity.
    pub fn resolve_id(&self, id: SymbolId) -> Result<&str, InternerAccessError> {
        let record = self.admit_symbol(id)?;
        Ok(self.entry_text(record))
    }

    /// Returns the namespace of a session-qualified control-sequence identity.
    pub fn kind_id(&self, id: SymbolId) -> Result<ControlSequenceKind, InternerAccessError> {
        self.admit_symbol(id)?;
        match self.metadata_for_symbol(id.raw()).map(|(kind, _)| kind) {
            Some(EntryKind::ControlSequence(kind)) => Ok(kind),
            Some(EntryKind::Spelling) => {
                unreachable!("symbol admission checked the entry kind")
            }
            None => Err(InternerAccessError::InvalidIdentity),
        }
    }

    /// Resolves a session-qualified non-control-sequence spelling.
    pub fn resolve_spelling(&self, id: SpellingId) -> Result<&str, InternerAccessError> {
        let record = self.admit_spelling(id)?;
        Ok(self.entry_text(record))
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

    fn admit_symbol(&self, id: SymbolId) -> Result<NameRecord, InternerAccessError> {
        self.admit_epoch(id.epoch)?;
        self.entries
            .get(usize::try_from(id.raw()).expect("Symbol fits native usize"))
            .copied()
            .filter(|_| {
                matches!(
                    self.metadata_for_symbol(id.raw()).map(|(kind, _)| kind),
                    Some(EntryKind::ControlSequence(_))
                )
            })
            .ok_or(InternerAccessError::InvalidIdentity)
    }

    fn admit_spelling(&self, id: SpellingId) -> Result<NameRecord, InternerAccessError> {
        self.admit_epoch(id.epoch)?;
        self.entries
            .get(usize::try_from(id.raw()).expect("Symbol fits native usize"))
            .copied()
            .filter(|_| {
                self.metadata_for_symbol(id.raw())
                    .is_some_and(|(kind, _)| kind == EntryKind::Spelling)
            })
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

    fn entry_text(&self, record: NameRecord) -> &str {
        std::str::from_utf8(
            self.arena
                .value(record)
                .expect("packed interner row addresses canonical UTF-8 bytes"),
        )
        .expect("interner byte pool contains valid UTF-8")
    }

    /// Admits a compact coordinate already owned by this session aggregate.
    pub(crate) fn resolve_local(&self, symbol: Symbol) -> Option<&str> {
        if self.retired {
            return None;
        }
        let record = self
            .entries
            .get(usize::try_from(symbol.raw()).expect("Symbol fits native usize"))
            .copied()?;
        Some(self.entry_text(record))
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
        Ok(self
            .metadata_for_symbol(id.raw())
            .is_some_and(|(_, hash_entry)| hash_entry))
    }

    /// Returns TeX82's occupied `hash` entries for the §1334 usage summary.
    #[must_use]
    pub(crate) fn multiletter_len(&self) -> usize {
        if self.retired { 0 } else { self.hash_entries }
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
        let record = self
            .entries
            .get(usize::try_from(symbol.raw()).expect("Symbol fits native usize"))
            .copied()?;
        let kind = self.metadata_for_symbol(symbol.raw())?.0;
        matches!(kind, EntryKind::ControlSequence(_))
            .then(|| semantic_atom_for_entry(kind, self.entry_text(record)))
    }

    #[cfg(test)]
    pub(crate) fn semantic_atom_identity(
        &self,
        symbol: Symbol,
    ) -> Option<(u64, crate::ContentHash)> {
        let record = self
            .entries
            .get(usize::try_from(symbol.raw()).expect("Symbol fits native usize"))
            .copied()?;
        let kind = self.metadata_for_symbol(symbol.raw())?.0;
        matches!(kind, EntryKind::ControlSequence(_)).then(|| {
            let text = self.entry_text(record);
            (
                semantic_atom_for_entry(kind, text),
                semantic_identity(kind, text),
            )
        })
    }

    /// Retires every name, lookup entry, and retained byte together.
    pub fn retire(&mut self) -> Result<InternerRetirement, InternerError> {
        if self.retired {
            return Err(InternerError::RetiredEpoch);
        }
        let usage = self.usage;
        self.arena = BytePool::default();
        self.entries = Vec::new();
        self.index = FixedIndex::empty();
        self.short_lookup.set(None);
        self.usage = InternerUsage::default();
        self.hash_entries = 0;
        self.retired = true;
        Ok(InternerRetirement { usage })
    }
}

const fn entry_kind_tag(kind: EntryKind) -> u8 {
    match kind {
        EntryKind::ControlSequence(ControlSequenceKind::Null) => 0,
        EntryKind::ControlSequence(ControlSequenceKind::SingleCharacter) => 1,
        EntryKind::ControlSequence(ControlSequenceKind::Named) => 2,
        EntryKind::ControlSequence(ControlSequenceKind::ActiveCharacter) => 3,
        EntryKind::ControlSequence(ControlSequenceKind::Internal) => 4,
        EntryKind::Spelling => 5,
    }
}

const fn entry_kind_from_tag(tag: u8) -> Option<EntryKind> {
    match tag {
        0 => Some(EntryKind::ControlSequence(ControlSequenceKind::Null)),
        1 => Some(EntryKind::ControlSequence(
            ControlSequenceKind::SingleCharacter,
        )),
        2 => Some(EntryKind::ControlSequence(ControlSequenceKind::Named)),
        3 => Some(EntryKind::ControlSequence(
            ControlSequenceKind::ActiveCharacter,
        )),
        4 => Some(EntryKind::ControlSequence(ControlSequenceKind::Internal)),
        5 => Some(EntryKind::Spelling),
        _ => None,
    }
}

const fn format_kind(kind: Option<EntryKind>) -> u8 {
    match kind {
        Some(EntryKind::ControlSequence(ControlSequenceKind::Null)) => 0,
        Some(EntryKind::ControlSequence(ControlSequenceKind::SingleCharacter)) => 1,
        Some(EntryKind::ControlSequence(ControlSequenceKind::Named)) => 2,
        Some(EntryKind::ControlSequence(ControlSequenceKind::ActiveCharacter)) => 3,
        Some(EntryKind::ControlSequence(ControlSequenceKind::Internal)) => 4,
        Some(EntryKind::Spelling) | None => 5,
    }
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
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    hash ^= u64::from(entry_kind_tag(kind));
    hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    for &byte in value.as_bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn index_control(hash: u64, kind: EntryKind, hash_entry: bool) -> u8 {
    let mut fingerprint = (hash >> 56) as u8 & INDEX_FINGERPRINT_MASK;
    if fingerprint == 0 {
        fingerprint = 1;
    }
    (entry_kind_tag(kind) << INDEX_KIND_SHIFT)
        | fingerprint
        | if hash_entry { INDEX_HASH_BIT } else { 0 }
}

fn index_bucket_count(slot_capacity: usize) -> usize {
    let required = slot_capacity
        .saturating_mul(4)
        .saturating_add(2)
        .checked_div(3)
        .unwrap_or(usize::MAX)
        .max(8);
    required.checked_next_power_of_two().unwrap_or(1 << 30)
}

fn short_lookup_key(kind: EntryKind, value: &str) -> Option<u64> {
    let bytes = value.as_bytes();
    if bytes.len() > 7 {
        return None;
    }
    let tag = u64::from(entry_kind_tag(kind));
    let mut key = tag | (bytes.len() as u64) << 3;
    for (index, &byte) in bytes.iter().enumerate() {
        key |= u64::from(byte) << (8 + index * 8);
    }
    Some(key)
}

#[cfg(test)]
fn entry_tag(kind: EntryKind) -> u8 {
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
fn semantic_identity(kind: EntryKind, value: &str) -> crate::ContentHash {
    let mut bytes = Vec::with_capacity(value.len() + 1);
    bytes.push(entry_tag(kind));
    bytes.extend_from_slice(value.as_bytes());
    crate::state_hash::semantic_identity_bytes(b"umber-session-interner-v1", &bytes)
}

#[cfg(test)]
#[path = "interner/tests.rs"]
mod tests;
