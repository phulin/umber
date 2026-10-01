//! Move-only durable box owners and their reversible TeX history.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_BOX_STATE_ID: AtomicU64 = AtomicU64::new(1);

use super::banks::{BankError, LEVEL_ONE};
use crate::node_region::NodeRegionId;
use crate::page_node_arena::{DurableNodeClosure, PageMaterialArena};

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct DurableNodeMetadata {
    region: NodeRegionId,
    len: usize,
    semantic_identity: Option<u64>,
}

impl DurableNodeMetadata {
    pub(crate) fn from_closure(closure: &DurableNodeClosure) -> Self {
        let root = closure.root();
        Self {
            region: closure.region_id(),
            len: root.len(),
            semantic_identity: root.list().semantic_identity(),
        }
    }

    pub(crate) fn from_page_root(
        region: NodeRegionId,
        root: crate::page_node_arena::PageListId,
    ) -> Self {
        Self {
            region,
            len: root.len(),
            semantic_identity: root.semantic_identity(),
        }
    }

    pub const fn region(self) -> NodeRegionId {
        self.region
    }

    pub const fn len(self) -> usize {
        self.len
    }

    pub const fn is_empty(self) -> bool {
        self.len == 0
    }

    pub const fn semantic_identity(self) -> Option<u64> {
        self.semantic_identity
    }
}

fn dimension_index(dimension: crate::command_context::BoxDimension) -> usize {
    match dimension {
        crate::command_context::BoxDimension::Width => 0,
        crate::command_context::BoxDimension::Height => 1,
        crate::command_context::BoxDimension::Depth => 2,
    }
}

struct DurableBoxCell {
    value: Option<DurableOwnerId>,
    level: u32,
    host_epoch: u64,
    dimension_epochs: [u64; 3],
}

impl Default for DurableBoxCell {
    fn default() -> Self {
        Self {
            value: None,
            level: LEVEL_ONE,
            host_epoch: 0,
            dimension_epochs: [0; 3],
        }
    }
}

pub(crate) struct DurableMutation {
    pub(crate) index: u16,
    alternate: Option<DurableOwnerId>,
    alternate_level: u32,
}

/// Compact reference into the one durable-owner store. Cells and reversible
/// journals move only this coordinate; the exclusive region envelope never
/// leaves its authoritative slot merely because TeX changes which root names
/// it.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct DurableOwnerId {
    slot: u32,
    incarnation: u32,
}

struct DurableOwnerSlot {
    incarnation: u32,
    live: bool,
    lineage: u64,
    owner: Option<DurableNodeClosure>,
}

#[derive(Default)]
struct DurableOwnerStore {
    slots: Vec<DurableOwnerSlot>,
    free: Vec<u32>,
    next_lineage: u64,
    /// Owner read in place by the one staged `\shipout\copy`. Shipout
    /// expands only deferred token lists, which cannot assign registers, so
    /// the pinned closure stays live and unchanged below its root until the
    /// staging boundary releases the pin.
    shipout_source: Option<DurableOwnerId>,
}

impl DurableOwnerStore {
    fn insert(&mut self, owner: DurableNodeClosure) -> DurableOwnerId {
        self.next_lineage = self.next_lineage.checked_add(1).expect("box lineage id");
        self.insert_with_lineage(owner, self.next_lineage)
    }

    fn insert_historical_copy(
        &mut self,
        source: DurableOwnerId,
        owner: DurableNodeClosure,
    ) -> DurableOwnerId {
        let lineage = self.slot(source).lineage;
        self.insert_with_lineage(owner, lineage)
    }

    fn insert_with_lineage(&mut self, owner: DurableNodeClosure, lineage: u64) -> DurableOwnerId {
        if let Some(slot) = self.free.pop() {
            let entry = self
                .slots
                .get_mut(slot as usize)
                .expect("free durable owner slot exists");
            assert!(!entry.live && entry.owner.is_none());
            entry.live = true;
            entry.lineage = lineage;
            entry.owner = Some(owner);
            return DurableOwnerId {
                slot,
                incarnation: entry.incarnation,
            };
        }
        let slot = u32::try_from(self.slots.len()).expect("durable owner slots fit u32");
        self.slots.push(DurableOwnerSlot {
            incarnation: 1,
            live: true,
            lineage,
            owner: Some(owner),
        });
        DurableOwnerId {
            slot,
            incarnation: 1,
        }
    }

    fn slot(&self, id: DurableOwnerId) -> &DurableOwnerSlot {
        let entry = self
            .slots
            .get(id.slot as usize)
            .expect("durable owner id names a slot");
        assert!(entry.live && entry.incarnation == id.incarnation);
        entry
    }

    fn slot_mut(&mut self, id: DurableOwnerId) -> &mut DurableOwnerSlot {
        let entry = self
            .slots
            .get_mut(id.slot as usize)
            .expect("durable owner id names a slot");
        assert!(entry.live && entry.incarnation == id.incarnation);
        entry
    }

    fn owner(&self, id: DurableOwnerId) -> &DurableNodeClosure {
        self.slot(id)
            .owner
            .as_ref()
            .expect("live durable owner slot contains its region")
    }

    fn owner_slot_mut(&mut self, id: DurableOwnerId) -> &mut Option<DurableNodeClosure> {
        &mut self.slot_mut(id).owner
    }

    fn retire(&mut self, arena: &mut PageMaterialArena, id: DurableOwnerId) {
        assert_ne!(
            self.shipout_source,
            Some(id),
            "a staged shipout source cannot retire before its shipout ends"
        );
        let slot = self.slot_mut(id);
        arena
            .retire_durable_in_place(&mut slot.owner)
            .expect("durable journal owns a live or transferred closure");
        slot.live = false;
        slot.incarnation = slot
            .incarnation
            .checked_add(1)
            .expect("durable owner incarnation space");
        self.free.push(id.slot);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[doc(hidden)]
pub struct DurableBoxCursor {
    checkpoint_entries: usize,
    scalar_entries: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RebasedDurableBoxCursor {
    checkpoint_entries: usize,
    scalar_entries: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct DurableBoxPrefixReleaseReceipt {
    pub(crate) checkpoint_entries: usize,
}

pub(crate) struct OutputCarrierTarget {
    pub(crate) index: u16,
    pub(crate) scope: super::AssignmentScope,
    pub(crate) current_level: u32,
}

/// The one register take between `make_box` clearing its source and the
/// assignment that installs it. The cleared owner stays in the state's
/// owner store; the state, not the receipt, holds its key.
struct PendingRegisterTake {
    source: u16,
    destination: u16,
    owner: DurableOwnerId,
    serial: u64,
}

/// One take from this state's live register binding. This opaque receipt
/// carries no owner key.
pub struct UniqueBoxRegisterTake {
    state_id: u64,
    serial: u64,
}

#[derive(Clone, Copy)]
struct DurableDimensionMutation {
    index: u16,
    owner: DurableOwnerId,
    lineage: u64,
    dimension: crate::command_context::BoxDimension,
    previous: crate::scaled::Scaled,
}

#[derive(Clone, Copy)]
struct ForkedScalarOwner {
    index: u16,
    accepted: DurableOwnerId,
    candidate: DurableOwnerId,
    accepted_entry_position: Option<usize>,
}

pub(crate) struct AcceptedDurableBoxTail {
    entries: Vec<DurableMutation>,
    scalar_entries: Vec<DurableDimensionMutation>,
    forked_scalar_owners: Vec<ForkedScalarOwner>,
}

pub(crate) struct DurableGroupRestoration {
    pub(crate) index: u16,
    pub(crate) saved: Option<DurableNodeMetadata>,
    pub(crate) live: Option<DurableNodeMetadata>,
    pub(crate) outcome: super::GroupRestorationOutcome,
}

pub(crate) struct DurableBoxState {
    state_id: u64,
    next_take_serial: u64,
    #[cfg(test)]
    fail_history_copy_after: Option<usize>,
    owners: DurableOwnerStore,
    dense: Box<[DurableBoxCell]>,
    overflow: HashMap<u16, DurableBoxCell>,
    checkpoint_entries: Vec<DurableMutation>,
    checkpoint_entry_base: usize,
    scalar_entries: Vec<DurableDimensionMutation>,
    scalar_entry_base: usize,
    checkpoint_epoch: u64,
    checkpoint_anchored: bool,
    semantic_identity: Option<crate::state_hash::SemanticMapIdentity>,
    pending_take: Option<PendingRegisterTake>,
}

struct DurableFormEntry {
    object: u32,
    owner: DurableNodeClosure,
}

pub(crate) struct DurableFormState {
    accepted: Vec<DurableFormEntry>,
    base_len: Option<usize>,
    delta: Vec<DurableFormEntry>,
}

impl DurableFormState {
    pub(crate) fn new() -> Self {
        Self {
            accepted: Vec::new(),
            base_len: None,
            delta: Vec::new(),
        }
    }

    pub(crate) fn insert(&mut self, object: u32, owner: DurableNodeClosure) {
        let destination = if self.base_len.is_some() {
            &mut self.delta
        } else {
            &mut self.accepted
        };
        assert!(destination.iter().all(|entry| entry.object != object));
        destination.push(DurableFormEntry { object, owner });
    }

    pub(crate) fn owner(&self, object: u32) -> Option<&DurableNodeClosure> {
        self.accepted[..self.base_len.unwrap_or(self.accepted.len())]
            .iter()
            .chain(&self.delta)
            .find(|entry| entry.object == object)
            .map(|entry| &entry.owner)
    }

    pub(crate) fn begin_candidate(&mut self, base_len: usize) {
        assert!(self.base_len.is_none() && self.delta.is_empty());
        assert!(base_len <= self.accepted.len());
        self.base_len = Some(base_len);
    }

    pub(crate) fn is_candidate(&self) -> bool {
        self.base_len.is_some()
    }

    pub(crate) fn candidate_base_len(&self) -> Option<usize> {
        self.base_len
    }

    pub(crate) fn reject_candidate(&mut self, arena: &mut PageMaterialArena) {
        assert!(self.base_len.take().is_some());
        for entry in self.delta.drain(..) {
            arena
                .retire_durable(entry.owner)
                .expect("rejected PDF form owner remains live");
        }
    }

    /// Drops the current-candidate forms after `form_count` while retaining
    /// both the checkpoint prefix and the accepted suffix parked behind
    /// `base_len`. Resource replay rewinds an already forked generation in
    /// place; the parked suffix must remain available if the surrounding
    /// candidate is later rejected back into its source.
    pub(crate) fn rewind_candidate(&mut self, arena: &mut PageMaterialArena, form_count: usize) {
        let base_len = self.base_len.expect("PDF form transaction is active");
        assert!(base_len <= form_count);
        let candidate_len = form_count - base_len;
        assert!(candidate_len <= self.delta.len());
        for entry in self.delta.drain(candidate_len..) {
            arena
                .retire_durable(entry.owner)
                .expect("rewound PDF form owner remains live");
        }
    }

    pub(crate) fn accept_candidate(&mut self, arena: &mut PageMaterialArena) {
        let base_len = self
            .base_len
            .take()
            .expect("PDF form transaction is active");
        for entry in self.accepted.drain(base_len..) {
            arena
                .retire_durable(entry.owner)
                .expect("superseded PDF form owner remains live");
        }
        self.accepted.append(&mut self.delta);
    }

    pub(crate) fn truncate(&mut self, arena: &mut PageMaterialArena, len: usize) {
        assert!(
            self.base_len.is_none(),
            "PDF form candidate is not truncated directly"
        );
        for entry in self.accepted.drain(len..) {
            arena
                .retire_durable(entry.owner)
                .expect("truncated PDF form owner remains live");
        }
    }

    pub(crate) fn copy_to_page(
        &self,
        arena: &mut PageMaterialArena,
        object: u32,
    ) -> Result<Option<crate::page_node_arena::PageListId>, BankError> {
        self.owner(object)
            .map(|owner| arena.copy_durable_to_page(owner))
            .transpose()
            .map_err(|_| BankError::AllocationFailed)
    }

    pub(crate) fn retire_all(mut self, arena: &mut PageMaterialArena) {
        assert!(
            self.base_len.is_none(),
            "PDF form candidate settles before retirement"
        );
        for entry in self.accepted.drain(..) {
            arena
                .retire_durable(entry.owner)
                .expect("accepted PDF form owner remains live");
        }
        for entry in self.delta.drain(..) {
            arena
                .retire_durable(entry.owner)
                .expect("candidate PDF form owner remains live");
        }
    }
}

impl DurableBoxState {
    pub(crate) fn new() -> Self {
        Self {
            state_id: NEXT_BOX_STATE_ID
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
                .expect("durable box state ids exhausted"),
            next_take_serial: 0,
            #[cfg(test)]
            fail_history_copy_after: None,
            owners: DurableOwnerStore::default(),
            dense: (0..=u8::MAX)
                .map(|_| DurableBoxCell::default())
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            overflow: HashMap::new(),
            checkpoint_entries: Vec::new(),
            checkpoint_entry_base: 0,
            scalar_entries: Vec::new(),
            scalar_entry_base: 0,
            checkpoint_epoch: 1,
            checkpoint_anchored: false,
            semantic_identity: None,
            pending_take: None,
        }
    }

    /// Reports whether this auxiliary state lane is at the level-zero
    /// checkpoint boundary. Durable-box group saves have the same admission
    /// rule as dense eqtb saves: a checkpoint never captures an open group or
    /// a register take between its source and destination.
    pub(crate) fn checkpoint_eligible(&self) -> bool {
        self.pending_take.is_none()
    }

    fn cell(&self, index: u16) -> Option<&DurableBoxCell> {
        if index <= u8::MAX.into() {
            Some(&self.dense[index as usize])
        } else {
            self.overflow.get(&index)
        }
    }

    fn cell_mut(&mut self, index: u16) -> &mut DurableBoxCell {
        if index <= u8::MAX.into() {
            &mut self.dense[index as usize]
        } else {
            self.overflow.entry(index).or_default()
        }
    }

    pub(crate) fn metadata(&self, index: u16) -> Option<DurableNodeMetadata> {
        self.cell(index)
            .and_then(|cell| cell.value)
            .map(|id| self.owners.owner(id))
            .map(DurableNodeMetadata::from_closure)
    }

    pub(crate) fn value(&self, index: u16) -> Option<&DurableNodeClosure> {
        self.cell(index)
            .and_then(|cell| cell.value)
            .map(|id| self.owners.owner(id))
    }

    /// Pins register `index` as the in-place source of one `\shipout\copy`
    /// and returns its closure, or `None` for a void register.
    pub(crate) fn pin_shipout_source(&mut self, index: u16) -> Option<&DurableNodeClosure> {
        let id = self.cell(index)?.value?;
        debug_assert!(self.owners.shipout_source.is_none());
        self.owners.shipout_source = Some(id);
        Some(self.owners.owner(id))
    }

    /// The closure pinned by [`Self::pin_shipout_source`], while it remains
    /// in its owner slot.
    pub(crate) fn shipout_source(&self) -> Option<&DurableNodeClosure> {
        let id = self.owners.shipout_source?;
        self.owners
            .slots
            .get(id.slot as usize)
            .filter(|slot| slot.live && slot.incarnation == id.incarnation)
            .and_then(|slot| slot.owner.as_ref())
    }

    pub(crate) fn release_shipout_source(&mut self) {
        self.owners.shipout_source = None;
    }

    pub(crate) fn visit_current(&self, mut visit: impl FnMut(u16, &DurableNodeClosure)) {
        for (index, cell) in self.dense.iter().enumerate() {
            if let Some(owner) = cell.value {
                visit(index as u16, self.owners.owner(owner));
            }
        }
        let mut overflow = self.overflow.iter().collect::<Vec<_>>();
        overflow.sort_unstable_by_key(|(index, _)| **index);
        for (&index, cell) in overflow {
            if let Some(owner) = cell.value {
                visit(index, self.owners.owner(owner));
            }
        }
    }

    pub(crate) fn enable_semantic_identity(&mut self) -> bool {
        if self.semantic_identity.is_some() {
            return true;
        }
        let mut identity = crate::state_hash::SemanticMapIdentity::empty(0x626f_785f_726f_6f74);
        for (index, cell) in self.dense.iter().enumerate() {
            if let Some(owner) = cell.value {
                let owner = self.owners.owner(owner);
                identity.replace(
                    index as u64,
                    None,
                    Some(match owner.root().list().semantic_identity() {
                        Some(identity) => identity,
                        None => return false,
                    }),
                );
            }
        }
        let mut overflow = self.overflow.iter().collect::<Vec<_>>();
        overflow.sort_unstable_by_key(|(index, _)| **index);
        for (&index, cell) in overflow {
            if let Some(owner) = cell.value {
                let owner = self.owners.owner(owner);
                identity.replace(
                    u64::from(index),
                    None,
                    Some(match owner.root().list().semantic_identity() {
                        Some(identity) => identity,
                        None => return false,
                    }),
                );
            }
        }
        self.semantic_identity = Some(identity);
        true
    }

    pub(crate) fn semantic_identity_root(&self) -> Option<u64> {
        self.semantic_identity.map(|identity| identity.root())
    }

    fn value_identity(&self, value: Option<DurableOwnerId>) -> Option<u64> {
        value.map(|owner| {
            let owner = self.owners.owner(owner);
            owner
                .root()
                .list()
                .semantic_identity()
                .expect("identity demand precedes durable box publication")
        })
    }

    fn transferred_value_identity(&self, owner: DurableOwnerId) -> u64 {
        let owner = self.owners.owner(owner);
        owner
            .root()
            .list()
            .semantic_identity()
            .expect("identity demand precedes durable box transfer")
    }

    fn record_unique_take(&mut self, index: u16, owner: DurableOwnerId) {
        let owner_identity = self
            .semantic_identity
            .is_some()
            .then(|| self.transferred_value_identity(owner));
        if let (Some(identity), Some(owner_identity)) =
            (&mut self.semantic_identity, owner_identity)
        {
            identity.replace(u64::from(index), Some(owner_identity), None);
        }
    }

    fn record_failed_unique_take(&mut self, index: u16, owner: DurableOwnerId) {
        let owner_identity = self
            .semantic_identity
            .is_some()
            .then(|| self.transferred_value_identity(owner));
        if let (Some(identity), Some(owner_identity)) =
            (&mut self.semantic_identity, owner_identity)
        {
            identity.replace(u64::from(index), None, Some(owner_identity));
        }
    }

    fn swap_mutation(&mut self, mutation: &mut DurableMutation) {
        let identities = self.semantic_identity.is_some().then(|| {
            (
                self.value_identity(self.cell(mutation.index).and_then(|cell| cell.value)),
                self.value_identity(mutation.alternate),
            )
        });
        let epoch = self.checkpoint_epoch;
        let cell = self.cell_mut(mutation.index);
        cell.host_epoch = epoch;
        std::mem::swap(&mut cell.value, &mut mutation.alternate);
        std::mem::swap(&mut cell.level, &mut mutation.alternate_level);
        if let (Some(identity), Some((old, new))) = (&mut self.semantic_identity, identities) {
            identity.replace(u64::from(mutation.index), old, new);
        }
    }

    fn copy_value(
        owners: &mut DurableOwnerStore,
        arena: &mut PageMaterialArena,
        value: Option<DurableOwnerId>,
    ) -> Result<Option<DurableOwnerId>, BankError> {
        let Some(value) = value else {
            return Ok(None);
        };
        let copy = arena
            .copy_durable_owner(owners.owner(value))
            .map_err(|_| BankError::AllocationFailed)?;
        Ok(Some(owners.insert_historical_copy(value, copy)))
    }

    fn retire_value(
        owners: &mut DurableOwnerStore,
        arena: &mut PageMaterialArena,
        value: Option<DurableOwnerId>,
    ) {
        if let Some(value) = value {
            owners.retire(arena, value);
        }
    }

    fn checkpoint_end(&self) -> Option<usize> {
        self.checkpoint_entry_base
            .checked_add(self.checkpoint_entries.len())
    }

    fn scalar_end(&self) -> Option<usize> {
        self.scalar_entry_base
            .checked_add(self.scalar_entries.len())
    }

    /// Converts a stable monotonic cursor into positions relative to the
    /// prefixes which remain physically resident. This is the only cursor
    /// rebase seam; outer checkpoint owners never scan or rewrite their roots.
    fn rebase_cursor(&self, cursor: DurableBoxCursor) -> Option<RebasedDurableBoxCursor> {
        let checkpoint_entries = cursor
            .checkpoint_entries
            .checked_sub(self.checkpoint_entry_base)?;
        let scalar_entries = cursor.scalar_entries.checked_sub(self.scalar_entry_base)?;
        if checkpoint_entries > self.checkpoint_entries.len()
            || scalar_entries > self.scalar_entries.len()
        {
            return None;
        }
        Some(RebasedDurableBoxCursor {
            checkpoint_entries,
            scalar_entries,
        })
    }

    fn release_checkpoint_entries(
        &mut self,
        arena: &mut PageMaterialArena,
        floor: usize,
    ) -> Result<usize, super::StateError> {
        let released = floor
            .checked_sub(self.checkpoint_entry_base)
            .filter(|released| *released <= self.checkpoint_entries.len())
            .ok_or(super::StateError::InvalidCursor)?;
        for mutation in self.checkpoint_entries.drain(..released) {
            Self::retire_value(&mut self.owners, arena, mutation.alternate);
        }
        self.checkpoint_entry_base = floor;
        Ok(released)
    }

    /// Reclaims checkpoint-only alternates; native group saves are independent.
    pub(crate) fn validates_checkpoint_prefix_release(
        &self,
        oldest: Option<DurableBoxCursor>,
        _accepted: Option<&AcceptedDurableBoxTail>,
    ) -> bool {
        oldest.is_none_or(|cursor| self.validates_cursor(cursor))
    }

    pub(crate) fn release_checkpoint_prefix(
        &mut self,
        arena: &mut PageMaterialArena,
        oldest: Option<DurableBoxCursor>,
        _accepted: Option<&mut AcceptedDurableBoxTail>,
    ) -> Result<DurableBoxPrefixReleaseReceipt, super::StateError> {
        if !self.validates_checkpoint_prefix_release(oldest, None) {
            return Err(super::StateError::InvalidCursor);
        }
        let checkpoint_floor = oldest.map_or_else(
            || self.checkpoint_end().expect("box history fits usize"),
            |cursor| cursor.checkpoint_entries,
        );
        let scalar_floor = oldest.map_or_else(
            || self.scalar_end().expect("box scalar history fits usize"),
            |cursor| cursor.scalar_entries,
        );
        let checkpoint_entries = self.release_checkpoint_entries(arena, checkpoint_floor)?;
        let count = scalar_floor - self.scalar_entry_base;
        self.scalar_entries.drain(..count);
        self.scalar_entry_base = scalar_floor;
        if oldest.is_none() {
            self.checkpoint_anchored = false;
        }
        Ok(DurableBoxPrefixReleaseReceipt { checkpoint_entries })
    }

    fn install_mutation<G>(
        &mut self,
        arena: &mut PageMaterialArena,
        index: u16,
        value: Option<DurableOwnerId>,
        level: u32,
        saved_at: Option<u32>,
        journal: Option<&mut crate::journal::SaveJournal<G>>,
    ) -> Result<(), BankError> {
        let checkpoint_needed = self.checkpoint_anchored
            && self
                .cell(index)
                .is_none_or(|cell| cell.host_epoch != self.checkpoint_epoch);
        let group_needed = saved_at.is_some();
        assert!(!group_needed || journal.is_some());
        let before = self
            .cell(index)
            .map_or((None, LEVEL_ONE), |cell| (cell.value, cell.level));
        // A checkpoint and a group save each need their own historical
        // owner. The copy is staged before the live binding changes, so a
        // failed copy publishes nothing.
        let checkpoint_copy = if checkpoint_needed && group_needed {
            self.check_history_copy_failure()?;
            Self::copy_value(&mut self.owners, arena, before.0)?
        } else {
            None
        };
        self.cell_mut(index).value = value;
        self.cell_mut(index).level = level;
        self.cell_mut(index).host_epoch = self.checkpoint_epoch;
        let identities = self.semantic_identity.is_some().then(|| {
            (
                self.value_identity(before.0),
                self.value_identity(self.cell(index).expect("durable box cell exists").value),
            )
        });
        if let (Some(identity), Some((old_identity, new_identity))) =
            (&mut self.semantic_identity, identities)
        {
            identity.replace(u64::from(index), old_identity, new_identity);
        }

        if !checkpoint_needed && !group_needed {
            Self::retire_value(&mut self.owners, arena, before.0);
            return Ok(());
        }

        if checkpoint_needed {
            let alternate = if group_needed {
                checkpoint_copy
            } else {
                before.0
            };
            self.checkpoint_entries.push(DurableMutation {
                index,
                alternate,
                alternate_level: before.1,
            });
        }
        if group_needed {
            journal
                .expect("local box save has native journal")
                .record_box_save(DurableMutation {
                    index,
                    alternate: before.0,
                    alternate_level: before.1,
                });
        }
        Ok(())
    }

    #[inline]
    fn check_history_copy_failure(&mut self) -> Result<(), BankError> {
        #[cfg(test)]
        if let Some(remaining) = &mut self.fail_history_copy_after {
            if *remaining == 0 {
                self.fail_history_copy_after = None;
                return Err(BankError::AllocationFailed);
            }
            *remaining -= 1;
        }
        Ok(())
    }

    pub(crate) fn assign_grouped<G>(
        &mut self,
        arena: &mut PageMaterialArena,
        index: u16,
        value: Option<DurableNodeClosure>,
        scope: super::AssignmentScope,
        current_level: u32,
        journal: Option<&mut crate::journal::SaveJournal<G>>,
    ) -> Result<(), BankError> {
        let value = value.map(|owner| self.owners.insert(owner));
        let before_level = self.cell(index).map_or(LEVEL_ONE, |cell| cell.level);
        let level = match scope {
            super::AssignmentScope::Global => LEVEL_ONE,
            super::AssignmentScope::Local => current_level,
        };
        let saved_at = (scope == super::AssignmentScope::Local
            && current_level != LEVEL_ONE
            && before_level != current_level)
            .then_some(current_level);
        let result = self.install_mutation(arena, index, value, level, saved_at, journal);
        if result.is_err() {
            Self::retire_value(&mut self.owners, arena, value);
        }
        result
    }

    /// Publishes the consumed output carrier after its page-region swap.
    /// Historical destination copies remain fallible, so a rejected install
    /// immediately returns the complete old region and builder slots; an
    /// accepted install commits the loan and retires the emptied old region.
    pub(crate) fn assign_output_carrier<G>(
        &mut self,
        arena: &mut PageMaterialArena,
        page: &mut crate::page::PageBuilderState,
        target: OutputCarrierTarget,
        assignment: crate::page_node_arena::PageOutputCarrierAssignment,
        journal: &mut crate::journal::SaveJournal<G>,
    ) -> Result<(), BankError> {
        let owner = self.owners.insert(assignment.closure);
        let before_level = self.cell(target.index).map_or(LEVEL_ONE, |cell| cell.level);
        let level = match target.scope {
            super::AssignmentScope::Global => LEVEL_ONE,
            super::AssignmentScope::Local => target.current_level,
        };
        let saved_at = (target.scope == super::AssignmentScope::Local
            && target.current_level != LEVEL_ONE
            && before_level != target.current_level)
            .then_some(target.current_level);
        if let Err(error) = self.install_mutation(
            arena,
            target.index,
            Some(owner),
            level,
            saved_at,
            Some(journal),
        ) {
            arena
                .rollback_output_carrier_loan(
                    page,
                    self.owners.owner_slot_mut(owner),
                    assignment.loan,
                )
                .expect("rejected binding returns its exact output region");
            self.owners.retire(arena, owner);
            return Err(error);
        }
        arena
            .commit_output_carrier_loan(assignment.loan)
            .expect("committed output loan retires its emptied old region");
        Ok(())
    }

    pub(crate) fn replace(
        &mut self,
        arena: &mut PageMaterialArena,
        index: u16,
        value: Option<DurableNodeClosure>,
    ) -> Result<(), BankError> {
        let value = value.map(|owner| self.owners.insert(owner));
        let level = self.cell(index).map_or(LEVEL_ONE, |cell| cell.level);
        self.install_mutation::<()>(arena, index, value, level, None, None)
    }

    pub(crate) fn set_box_dimension(
        &mut self,
        arena: &mut PageMaterialArena,
        index: u16,
        dimension: crate::command_context::BoxDimension,
        value: crate::scaled::Scaled,
    ) -> Result<bool, BankError> {
        let Some(owner) = self.cell(index).and_then(|cell| cell.value) else {
            return Ok(false);
        };
        let checkpoint_needed = self.checkpoint_anchored
            && self
                .cell(index)
                .is_none_or(|cell| cell.host_epoch != self.checkpoint_epoch);
        let old_identity = self
            .semantic_identity
            .map(|_| self.value_identity(Some(owner)));
        let previous = arena
            .set_durable_root_box_dimension(
                self.owners
                    .owner_slot_mut(owner)
                    .as_mut()
                    .expect("live box"),
                dimension,
                value,
            )
            .map_err(|_| BankError::AllocationFailed)?;
        if let (Some(identity), Some(old_identity)) = (&mut self.semantic_identity, old_identity) {
            let new_identity = self
                .owners
                .owner(owner)
                .root()
                .list()
                .semantic_identity()
                .expect("maintained box identity");
            identity.replace(u64::from(index), old_identity, Some(new_identity));
        }
        if previous != value {
            let mutation = DurableDimensionMutation {
                index,
                owner,
                lineage: self.owners.slot(owner).lineage,
                dimension,
                previous,
            };
            if checkpoint_needed
                && self.cell(index).expect("box exists").dimension_epochs
                    [dimension_index(dimension)]
                    != self.checkpoint_epoch
            {
                self.scalar_entries.push(mutation);
                self.cell_mut(index).dimension_epochs[dimension_index(dimension)] =
                    self.checkpoint_epoch;
            }
        }
        Ok(true)
    }

    fn apply_dimension_inverse(
        &mut self,
        arena: &mut PageMaterialArena,
        mutation: DurableDimensionMutation,
    ) {
        let current = self.cell(mutation.index).and_then(|cell| cell.value) == Some(mutation.owner);
        let old_identity = (current && self.semantic_identity.is_some())
            .then(|| self.value_identity(Some(mutation.owner)));
        arena
            .set_durable_root_box_dimension(
                self.owners
                    .owner_slot_mut(mutation.owner)
                    .as_mut()
                    .expect("journaled box owner"),
                mutation.dimension,
                mutation.previous,
            )
            .expect("scalar history edits its original box root");
        let new_identity = old_identity.map(|_| self.value_identity(Some(mutation.owner)));
        if let (Some(identity), Some(old_identity), Some(new_identity)) =
            (&mut self.semantic_identity, old_identity, new_identity)
        {
            identity.replace(u64::from(mutation.index), old_identity, new_identity);
        }
    }

    fn apply_checkpoint_scalar_inverse(
        &mut self,
        arena: &mut PageMaterialArena,
        mut mutation: DurableDimensionMutation,
    ) {
        let Some(current) = self.cell(mutation.index).and_then(|cell| cell.value) else {
            // The selected checkpoint predates this box's assignment.
            return;
        };
        if self.owners.slot(current).lineage != mutation.lineage {
            return;
        }
        mutation.owner = current;
        self.apply_dimension_inverse(arena, mutation);
    }

    fn can_take_unique(&self, index: u16) -> bool {
        // A group save holds the previous binding, never the current box.
        // TeX82 §1079 clears the current register at its current level even
        // while groups are open. Only a checkpoint's pre-write current owner
        // still needs preservation before the destructive transfer.
        !self.checkpoint_anchored
            || self
                .cell(index)
                .is_some_and(|cell| cell.host_epoch == self.checkpoint_epoch)
    }

    /// Whether an assignment to `index` in `scope` at `current_level`
    /// replaces its binding without saving the previous value, as TeX82
    /// §277/§279's `eq_define` at the binding's own level and
    /// `geq_define` do.
    pub(crate) fn assignment_destroys_binding(
        &self,
        index: u16,
        scope: super::AssignmentScope,
        current_level: u32,
    ) -> bool {
        scope == super::AssignmentScope::Global
            || current_level == LEVEL_ONE
            || self.cell(index).map_or(LEVEL_ONE, |cell| cell.level) == current_level
    }

    pub(crate) fn has_unique_current(&self, index: u16) -> bool {
        self.cell(index).and_then(|cell| cell.value).is_some() && self.can_take_unique(index)
    }

    /// Clears the semantic source first, as TeX's `make_box` does, then parks
    /// its exclusive owner in the pending take until assignment. This also
    /// makes same-register tracing observe a void old destination.
    pub(crate) fn begin_unique_register_take(
        &mut self,
        source: u16,
        destination: u16,
    ) -> Option<UniqueBoxRegisterTake> {
        if !self.has_unique_current(source) {
            return None;
        }
        assert!(
            self.pending_take.is_none(),
            "box take must finish before another begins"
        );
        let serial = self.next_take_serial.checked_add(1).expect("take serial");
        let owner = self.cell_mut(source).value.take().expect("admitted source");
        self.record_unique_take(source, owner);
        self.next_take_serial = serial;
        self.pending_take = Some(PendingRegisterTake {
            source,
            destination,
            owner,
            serial,
        });
        Some(UniqueBoxRegisterTake {
            state_id: self.state_id,
            serial,
        })
    }

    /// Assigns the previously cleared source. Historical destination copies
    /// are staged before any binding or journal changes; failure returns the
    /// owner to its source.
    pub(crate) fn finish_unique_register_take<G>(
        &mut self,
        arena: &mut PageMaterialArena,
        take: UniqueBoxRegisterTake,
        destination: u16,
        scope: super::AssignmentScope,
        current_level: u32,
        journal: Option<&mut crate::journal::SaveJournal<G>>,
    ) -> Result<(), BankError> {
        assert_eq!(take.state_id, self.state_id, "foreign box take");
        let (source, owner) = match self.pending_take.take() {
            Some(pending)
                if pending.serial == take.serial && pending.destination == destination =>
            {
                (pending.source, pending.owner)
            }
            _ => panic!("box take is not pending in its owner store"),
        };
        let before_level = self.cell(destination).map_or(LEVEL_ONE, |cell| cell.level);
        let level = match scope {
            super::AssignmentScope::Global => LEVEL_ONE,
            super::AssignmentScope::Local => current_level,
        };
        let saved_at = (scope == super::AssignmentScope::Local
            && current_level != LEVEL_ONE
            && before_level != current_level)
            .then_some(current_level);
        if let Err(error) =
            self.install_mutation(arena, destination, Some(owner), level, saved_at, journal)
        {
            assert!(self.cell(source).and_then(|cell| cell.value).is_none());
            self.cell_mut(source).value = Some(owner);
            self.record_failed_unique_take(source, owner);
            return Err(error);
        }
        Ok(())
    }

    #[cfg(test)]
    fn assign_unique_register_take<G>(
        &mut self,
        arena: &mut PageMaterialArena,
        source: u16,
        destination: u16,
        scope: super::AssignmentScope,
        current_level: u32,
        journal: Option<&mut crate::journal::SaveJournal<G>>,
    ) -> Result<bool, BankError> {
        let Some(take) = self.begin_unique_register_take(source, destination) else {
            return Ok(false);
        };
        self.finish_unique_register_take(arena, take, destination, scope, current_level, journal)?;
        Ok(true)
    }

    pub(crate) fn copy_to_page(
        &mut self,
        arena: &mut PageMaterialArena,
        index: u16,
    ) -> Result<Option<crate::page_node_arena::PageListId>, BankError> {
        if let Some(id) = self.cell(index).and_then(|cell| cell.value)
            && self.owners.shipout_source != Some(id)
        {
            arena.share_durable_body(self.owners.owner_slot_mut(id));
        }
        self.value(index)
            .map(|owner| arena.copy_durable_to_page(owner))
            .transpose()
            .map_err(|_| BankError::AllocationFailed)
    }

    pub(crate) fn take_to_page(
        &mut self,
        arena: &mut PageMaterialArena,
        index: u16,
    ) -> Result<Option<crate::page_node_arena::PageListId>, BankError> {
        let Some(owner) = self.cell_mut(index).value.take() else {
            return Ok(None);
        };
        if self.can_take_unique(index) {
            self.record_unique_take(index, owner);
            let moved = arena.move_durable_to_page_in_place(self.owners.owner_slot_mut(owner));
            return match moved {
                Ok(root) => {
                    self.owners.retire(arena, owner);
                    Ok(Some(root))
                }
                Err(_) => {
                    self.record_failed_unique_take(index, owner);
                    self.cell_mut(index).value = Some(owner);
                    Err(BankError::AllocationFailed)
                }
            };
        }

        // The source owner must enter every retained history lane before the
        // consuming command can void the live cell. The page receives one
        // explicit semantic copy because moving that owner would invalidate
        // exact rollback.
        self.cell_mut(index).value = Some(owner);
        let copied = {
            let source = self.value(index).expect("restored source owner");
            arena
                .copy_history_preserved_to_page(source)
                .map_err(|_| BankError::AllocationFailed)?
        };
        self.replace(arena, index, None)?;
        Ok(Some(copied))
    }

    pub(crate) fn restore_group_record(
        &mut self,
        saved: &mut DurableMutation,
    ) -> DurableGroupRestoration {
        let metadata = saved
            .alternate
            .map(|owner| DurableNodeMetadata::from_closure(self.owners.owner(owner)));
        let outcome = if self
            .cell(saved.index)
            .is_none_or(|cell| cell.level == LEVEL_ONE)
        {
            super::GroupRestorationOutcome::Retained
        } else {
            self.swap_mutation(saved);
            super::GroupRestorationOutcome::Restored
        };
        DurableGroupRestoration {
            index: saved.index,
            saved: metadata,
            live: self.metadata(saved.index),
            outcome,
        }
    }

    pub(crate) fn retire_group_record(
        &mut self,
        arena: &mut PageMaterialArena,
        mut saved: DurableMutation,
    ) {
        Self::retire_value(&mut self.owners, arena, saved.alternate.take());
    }

    pub(crate) fn checkpoint_cursor(&mut self) -> DurableBoxCursor {
        self.checkpoint_anchored = true;
        let cursor = DurableBoxCursor {
            checkpoint_entries: self
                .checkpoint_end()
                .expect("durable checkpoint position overflow"),
            scalar_entries: self.scalar_end().expect("durable scalar position overflow"),
        };
        self.checkpoint_epoch = self.checkpoint_epoch.checked_add(1).expect("box epoch");
        cursor
    }

    pub(crate) fn validates_cursor(&self, cursor: DurableBoxCursor) -> bool {
        self.validates_cursor_with_accepted(cursor, None)
    }

    pub(crate) fn validates_cursor_for_release(
        &self,
        cursor: DurableBoxCursor,
        _accepted: Option<&AcceptedDurableBoxTail>,
    ) -> bool {
        self.validates_cursor_with_accepted(cursor, _accepted)
    }

    fn validates_cursor_with_accepted(
        &self,
        cursor: DurableBoxCursor,
        _accepted: Option<&AcceptedDurableBoxTail>,
    ) -> bool {
        let Some(checkpoint_entries) = cursor
            .checkpoint_entries
            .checked_sub(self.checkpoint_entry_base)
        else {
            return false;
        };
        let Some(scalar_entries) = cursor.scalar_entries.checked_sub(self.scalar_entry_base) else {
            return false;
        };
        checkpoint_entries <= self.checkpoint_entries.len()
            && scalar_entries <= self.scalar_entries.len()
    }

    fn swap_checkpoint_suffix(&mut self, start: usize) {
        let mut suffix = self.checkpoint_entries.split_off(start);
        for mutation in suffix.iter_mut().rev() {
            self.swap_mutation(mutation);
        }
        self.checkpoint_entries.append(&mut suffix);
    }

    fn abandon_pending_take(&mut self) {
        if let Some(pending) = self.pending_take.take() {
            assert!(
                self.cell(pending.source)
                    .and_then(|cell| cell.value)
                    .is_none()
            );
            self.cell_mut(pending.source).value = Some(pending.owner);
            self.record_failed_unique_take(pending.source, pending.owner);
        }
    }

    pub(crate) fn restore(&mut self, arena: &mut PageMaterialArena, cursor: DurableBoxCursor) {
        self.abandon_pending_take();
        let rebased = self
            .rebase_cursor(cursor)
            .expect("validated durable cursor rebases");
        self.swap_checkpoint_suffix(rebased.checkpoint_entries);
        while self.scalar_entries.len() > rebased.scalar_entries {
            let mutation = self.scalar_entries.pop().expect("scalar suffix exists");
            self.apply_checkpoint_scalar_inverse(arena, mutation);
        }
        for mutation in self.checkpoint_entries.drain(rebased.checkpoint_entries..) {
            Self::retire_value(&mut self.owners, arena, mutation.alternate);
        }
        self.checkpoint_epoch = self.checkpoint_epoch.checked_add(1).expect("box epoch");
    }

    pub(crate) fn begin_checkpoint_candidate(
        &mut self,
        arena: &mut PageMaterialArena,
        cursor: DurableBoxCursor,
    ) -> Result<AcceptedDurableBoxTail, BankError> {
        assert!(self.pending_take.is_none());
        let rebased = self
            .rebase_cursor(cursor)
            .expect("validated durable candidate cursor rebases");
        let mut selected_owners = HashMap::new();
        let mut edited_lineages = HashSet::new();
        for mutation in &self.scalar_entries[rebased.scalar_entries..] {
            selected_owners
                .entry(mutation.index)
                .or_insert_with(|| self.cell(mutation.index).and_then(|cell| cell.value));
            edited_lineages.insert((mutation.index, mutation.lineage));
        }
        let mut first_binding_entry = HashMap::new();
        for (position, mutation) in self.checkpoint_entries[rebased.checkpoint_entries..]
            .iter()
            .enumerate()
        {
            first_binding_entry
                .entry(mutation.index)
                .or_insert(position);
        }
        for mutation in self.checkpoint_entries[rebased.checkpoint_entries..]
            .iter()
            .rev()
        {
            if let Some(selected) = selected_owners.get_mut(&mutation.index) {
                *selected = mutation.alternate;
            }
        }
        // Allocate every independently live branch copy before swapping any
        // binding or scalar. An allocation failure leaves the accepted state
        // untouched and retires only unpublished candidate preparation.
        let mut prepared = Vec::new();
        for (index, selected) in selected_owners {
            let Some(accepted) = selected else { continue };
            if !edited_lineages.contains(&(index, self.owners.slot(accepted).lineage)) {
                continue;
            }
            match arena.copy_durable_owner(self.owners.owner(accepted)) {
                Ok(copy) => prepared.push((
                    index,
                    accepted,
                    first_binding_entry.get(&index).copied(),
                    copy,
                )),
                Err(_) => {
                    for (_, _, _, copy) in prepared {
                        arena
                            .retire_durable(copy)
                            .expect("unpublished scalar fork copy retires");
                    }
                    return Err(BankError::AllocationFailed);
                }
            }
        }
        self.swap_checkpoint_suffix(rebased.checkpoint_entries);
        let mut forked_scalar_owners = Vec::with_capacity(prepared.len());
        for (index, accepted, accepted_entry_position, copy) in prepared {
            let candidate = self.owners.insert_historical_copy(accepted, copy);
            assert_eq!(self.cell(index).and_then(|cell| cell.value), Some(accepted));
            self.cell_mut(index).value = Some(candidate);
            forked_scalar_owners.push(ForkedScalarOwner {
                index,
                accepted,
                candidate,
                accepted_entry_position,
            });
        }
        for position in (rebased.scalar_entries..self.scalar_entries.len()).rev() {
            self.apply_checkpoint_scalar_inverse(arena, self.scalar_entries[position]);
        }
        let scalar_entries = self.scalar_entries.split_off(rebased.scalar_entries);
        let entries = self
            .checkpoint_entries
            .split_off(rebased.checkpoint_entries);
        self.checkpoint_epoch = self.checkpoint_epoch.checked_add(1).expect("box epoch");
        Ok(AcceptedDurableBoxTail {
            entries,
            scalar_entries,
            forked_scalar_owners,
        })
    }

    pub(crate) fn reject_checkpoint_candidate(
        &mut self,
        arena: &mut PageMaterialArena,
        cursor: DurableBoxCursor,
        mut accepted: AcceptedDurableBoxTail,
    ) {
        let rebased = self
            .rebase_cursor(cursor)
            .expect("validated durable rejection cursor rebases");
        self.swap_checkpoint_suffix(rebased.checkpoint_entries);
        while self.scalar_entries.len() > rebased.scalar_entries {
            let mutation = self
                .scalar_entries
                .pop()
                .expect("candidate scalar suffix exists");
            self.apply_checkpoint_scalar_inverse(arena, mutation);
        }
        for mutation in self.checkpoint_entries.drain(rebased.checkpoint_entries..) {
            Self::retire_value(&mut self.owners, arena, mutation.alternate);
        }
        for mutation in &mut accepted.entries {
            self.swap_mutation(mutation);
        }
        for fork in accepted.forked_scalar_owners {
            if let Some(position) = fork.accepted_entry_position {
                let entry = accepted
                    .entries
                    .get_mut(position)
                    .expect("accepted binding entry");
                assert_eq!(entry.alternate, Some(fork.candidate));
                entry.alternate = Some(fork.accepted);
            } else {
                assert_eq!(
                    self.cell(fork.index).and_then(|cell| cell.value),
                    Some(fork.candidate)
                );
                let identities = self.semantic_identity.as_ref().map(|_| {
                    (
                        self.value_identity(Some(fork.candidate)),
                        self.value_identity(Some(fork.accepted)),
                    )
                });
                self.cell_mut(fork.index).value = Some(fork.accepted);
                if let (Some(identity), Some((old, new))) =
                    (&mut self.semantic_identity, identities)
                {
                    identity.replace(u64::from(fork.index), old, new);
                }
            }
            self.owners.retire(arena, fork.candidate);
        }
        self.checkpoint_entries.append(&mut accepted.entries);
        self.scalar_entries.append(&mut accepted.scalar_entries);
        self.checkpoint_epoch = self.checkpoint_epoch.checked_add(1).expect("box epoch");
    }

    pub(crate) fn accept_checkpoint_candidate(
        &mut self,
        arena: &mut PageMaterialArena,
        accepted: AcceptedDurableBoxTail,
    ) {
        for fork in accepted.forked_scalar_owners {
            self.owners.retire(arena, fork.accepted);
        }
        for mutation in accepted.entries {
            Self::retire_value(&mut self.owners, arena, mutation.alternate);
        }
        self.checkpoint_epoch = self.checkpoint_epoch.checked_add(1).expect("box epoch");
    }

    pub(crate) fn retire_all(mut self, arena: &mut PageMaterialArena) {
        self.abandon_pending_take();
        for cell in &mut self.dense {
            Self::retire_value(&mut self.owners, arena, cell.value.take());
        }
        for (_, mut cell) in self.overflow.drain() {
            Self::retire_value(&mut self.owners, arena, cell.value.take());
        }
        for mutation in self.checkpoint_entries.drain(..) {
            Self::retire_value(&mut self.owners, arena, mutation.alternate);
        }
        // Native group saves still name exclusive slots during whole-job
        // retirement; the physical owner store is their final authority.
        for index in 0..self.owners.slots.len() {
            let slot = &self.owners.slots[index];
            if slot.live {
                let id = DurableOwnerId {
                    slot: u32::try_from(index).expect("owner slots fit u32"),
                    incarnation: slot.incarnation,
                };
                self.owners.retire(arena, id);
            }
        }
    }
}
