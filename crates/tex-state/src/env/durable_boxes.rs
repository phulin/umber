//! Move-only durable box owners and their reversible TeX history.

use std::collections::HashMap;

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

struct DurableBoxCell {
    value: Option<DurableOwnerId>,
    level: u32,
}

impl Default for DurableBoxCell {
    fn default() -> Self {
        Self {
            value: None,
            level: LEVEL_ONE,
        }
    }
}

struct DurableMutation {
    index: u16,
    alternate: Option<DurableOwnerId>,
    alternate_level: u32,
}

/// Compact reference into the one durable-owner store. Cells and reversible
/// journals move only this coordinate; the exclusive region envelope never
/// leaves its authoritative slot merely because TeX changes which root names
/// it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DurableOwnerId {
    slot: u32,
    incarnation: u32,
}

struct DurableOwnerSlot {
    incarnation: u32,
    live: bool,
    owner: Option<DurableNodeClosure>,
}

#[derive(Default)]
struct DurableOwnerStore {
    slots: Vec<DurableOwnerSlot>,
    free: Vec<u32>,
}

impl DurableOwnerStore {
    fn insert(&mut self, owner: DurableNodeClosure) -> DurableOwnerId {
        if let Some(slot) = self.free.pop() {
            let entry = self
                .slots
                .get_mut(slot as usize)
                .expect("free durable owner slot exists");
            assert!(!entry.live && entry.owner.is_none());
            entry.live = true;
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

    fn restore(&mut self, id: DurableOwnerId, owner: DurableNodeClosure) {
        let slot = self.slot_mut(id);
        assert!(slot.owner.replace(owner).is_none());
    }

    fn retire(&mut self, arena: &mut PageMaterialArena, id: DurableOwnerId) {
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

struct DurableGroup {
    id: u64,
    parent: u64,
    level: u32,
    entries: Vec<DurableMutation>,
    checkpoint_pinned: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[doc(hidden)]
pub struct DurableBoxCursor {
    /// Monotonic position in the checkpoint journal. The live journal may
    /// release a physical prefix without rewriting retained cursors.
    checkpoint_entries: usize,
    /// Monotonic position in the completed-group journal at capture.
    retained_groups: usize,
    group_id: u64,
    group_entry_position: usize,
    group_depth: usize,
    next_group_id: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RebasedDurableBoxCursor {
    checkpoint_entries: usize,
    retained_groups: usize,
}

/// Exact durable-owner work performed by one retained-prefix release.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct DurableBoxPrefixReleaseReceipt {
    pub(crate) checkpoint_entries: usize,
    pub(crate) retained_groups: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DurableBoxOperation {
    position: usize,
    loan_position: usize,
    group_position: usize,
}

struct DurableBoxTransferLoan {
    mutation_position: usize,
    loan: crate::page_node_arena::DurableTransferLoan,
}

pub(crate) struct AcceptedDurableBoxTail {
    entries: Vec<DurableMutation>,
    groups: AcceptedDurableGroupTail,
    retained_group_base: usize,
}

enum AcceptedDurableGroupTail {
    Root {
        accepted_groups: Vec<DurableGroup>,
        accepted_retained_groups: Vec<DurableGroup>,
        next_group_id: u64,
    },
    Arbitrary {
        next_group_id: u64,
        accepted_groups: Vec<DurableGroup>,
        accepted_retained_groups: Vec<DurableGroup>,
    },
}

impl AcceptedDurableBoxTail {
    fn accepted_retained_groups(&self) -> &[DurableGroup] {
        match &self.groups {
            AcceptedDurableGroupTail::Root {
                accepted_retained_groups,
                ..
            }
            | AcceptedDurableGroupTail::Arbitrary {
                accepted_retained_groups,
                ..
            } => accepted_retained_groups,
        }
    }

    fn accepted_retained_groups_mut(&mut self) -> &mut Vec<DurableGroup> {
        match &mut self.groups {
            AcceptedDurableGroupTail::Root {
                accepted_retained_groups,
                ..
            }
            | AcceptedDurableGroupTail::Arbitrary {
                accepted_retained_groups,
                ..
            } => accepted_retained_groups,
        }
    }

    fn release_retained_group_prefix(
        &mut self,
        owners: &mut DurableOwnerStore,
        arena: &mut PageMaterialArena,
        floor: usize,
    ) -> Result<usize, super::StateError> {
        if floor <= self.retained_group_base {
            return Ok(0);
        }
        let released = floor
            .checked_sub(self.retained_group_base)
            .filter(|released| *released <= self.accepted_retained_groups_mut().len())
            .ok_or(super::StateError::InvalidCursor)?;
        for group in self.accepted_retained_groups_mut().drain(..released) {
            DurableBoxState::retire_group(owners, arena, group);
        }
        self.retained_group_base = floor;
        Ok(released)
    }

    fn validates_retained_group_floor(&self, floor: usize) -> bool {
        floor <= self.retained_group_base
            || floor
                .checked_sub(self.retained_group_base)
                .is_some_and(|released| released <= self.accepted_retained_groups().len())
    }

    fn contains_retained_group_position(&self, position: usize) -> bool {
        position >= self.retained_group_base
            && self
                .retained_group_base
                .checked_add(self.accepted_retained_groups().len())
                .is_some_and(|end| position <= end)
    }

    fn group(&self, id: u64) -> Option<&DurableGroup> {
        match &self.groups {
            AcceptedDurableGroupTail::Root {
                accepted_groups,
                accepted_retained_groups,
                ..
            }
            | AcceptedDurableGroupTail::Arbitrary {
                accepted_groups,
                accepted_retained_groups,
                ..
            } => accepted_groups
                .iter()
                .chain(accepted_retained_groups)
                .find(|group| group.id == id),
        }
    }

    fn clear_checkpoint_pins(&mut self) {
        match &mut self.groups {
            AcceptedDurableGroupTail::Root {
                accepted_groups, ..
            }
            | AcceptedDurableGroupTail::Arbitrary {
                accepted_groups, ..
            } => {
                for group in accepted_groups {
                    group.checkpoint_pinned = false;
                }
            }
        }
    }
}

pub(crate) struct DurableGroupRestoration {
    pub(crate) index: u16,
    pub(crate) saved: Option<DurableNodeMetadata>,
    pub(crate) live: Option<DurableNodeMetadata>,
    pub(crate) outcome: super::GroupRestorationOutcome,
}

pub(crate) struct DurableBoxState {
    owners: DurableOwnerStore,
    dense: Box<[DurableBoxCell]>,
    overflow: HashMap<u16, DurableBoxCell>,
    checkpoint_entries: Vec<DurableMutation>,
    checkpoint_entry_base: usize,
    checkpoint_stamps: HashMap<u16, u64>,
    checkpoint_epoch: u64,
    groups: Vec<DurableGroup>,
    retained_groups: Vec<DurableGroup>,
    retained_group_base: usize,
    next_group_id: u64,
    semantic_identity: Option<crate::state_hash::SemanticMapIdentity>,
    operation_entries: Vec<DurableMutation>,
    transfer_loans: Vec<DurableBoxTransferLoan>,
    active_operations: Vec<usize>,
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

    pub(crate) fn reject_candidate(&mut self, arena: &mut PageMaterialArena) {
        assert!(self.base_len.take().is_some());
        for entry in self.delta.drain(..) {
            arena
                .retire_durable(entry.owner)
                .expect("rejected PDF form owner remains live");
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
            owners: DurableOwnerStore::default(),
            dense: (0..=u8::MAX)
                .map(|_| DurableBoxCell::default())
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            overflow: HashMap::new(),
            checkpoint_entries: Vec::new(),
            checkpoint_entry_base: 0,
            checkpoint_stamps: HashMap::new(),
            checkpoint_epoch: 1,
            groups: Vec::new(),
            retained_groups: Vec::new(),
            retained_group_base: 0,
            next_group_id: 0,
            semantic_identity: None,
            operation_entries: Vec::new(),
            transfer_loans: Vec::new(),
            active_operations: Vec::new(),
        }
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
        let cell = self.cell_mut(mutation.index);
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
        Ok(Some(owners.insert(copy)))
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

    fn retire_group(
        owners: &mut DurableOwnerStore,
        arena: &mut PageMaterialArena,
        group: DurableGroup,
    ) {
        for mutation in group.entries {
            Self::retire_value(owners, arena, mutation.alternate);
        }
    }

    fn checkpoint_end(&self) -> Option<usize> {
        self.checkpoint_entry_base
            .checked_add(self.checkpoint_entries.len())
    }

    fn retained_group_end(&self) -> Option<usize> {
        self.retained_group_base
            .checked_add(self.retained_groups.len())
    }

    /// Converts a stable monotonic cursor into positions relative to the
    /// prefixes which remain physically resident. This is the only cursor
    /// rebase seam; outer checkpoint owners never scan or rewrite their roots.
    fn rebase_cursor(&self, cursor: DurableBoxCursor) -> Option<RebasedDurableBoxCursor> {
        let checkpoint_entries = cursor
            .checkpoint_entries
            .checked_sub(self.checkpoint_entry_base)?;
        let retained_groups = cursor
            .retained_groups
            .checked_sub(self.retained_group_base)?;
        if checkpoint_entries > self.checkpoint_entries.len()
            || retained_groups > self.retained_groups.len()
        {
            return None;
        }
        Some(RebasedDurableBoxCursor {
            checkpoint_entries,
            retained_groups,
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

    fn release_retained_groups(
        &mut self,
        arena: &mut PageMaterialArena,
        floor: usize,
    ) -> Result<usize, super::StateError> {
        // During a candidate fork the current lane begins at the selected
        // cursor while the older accepted group prefix lives in its tail.
        if floor <= self.retained_group_base {
            return Ok(0);
        }
        let released = floor
            .checked_sub(self.retained_group_base)
            .filter(|released| *released <= self.retained_groups.len())
            .ok_or(super::StateError::InvalidCursor)?;
        for group in self.retained_groups.drain(..released) {
            Self::retire_group(&mut self.owners, arena, group);
        }
        self.retained_group_base = floor;
        Ok(released)
    }

    /// Releases checkpoint-only durable alternates older than the earliest
    /// surviving ordinary checkpoint. Active cells, groups, and operations
    /// own disjoint alternates and are never visited.
    pub(crate) fn validates_checkpoint_prefix_release(
        &self,
        oldest_retained: Option<DurableBoxCursor>,
        accepted: Option<&AcceptedDurableBoxTail>,
    ) -> bool {
        let Some((checkpoint_floor, retained_group_floor)) = oldest_retained
            .map(|cursor| {
                self.validates_cursor_with_accepted(cursor, accepted)
                    .then_some((cursor.checkpoint_entries, cursor.retained_groups))
            })
            .unwrap_or_else(|| Some((self.checkpoint_end()?, self.retained_group_end()?)))
        else {
            return false;
        };
        let checkpoint_valid = checkpoint_floor
            .checked_sub(self.checkpoint_entry_base)
            .is_some_and(|released| released <= self.checkpoint_entries.len());
        let groups_valid = retained_group_floor <= self.retained_group_base
            || retained_group_floor
                .checked_sub(self.retained_group_base)
                .is_some_and(|released| released <= self.retained_groups.len());
        checkpoint_valid
            && groups_valid
            && accepted.is_none_or(|tail| tail.validates_retained_group_floor(retained_group_floor))
    }

    pub(crate) fn release_checkpoint_prefix(
        &mut self,
        arena: &mut PageMaterialArena,
        oldest_retained: Option<DurableBoxCursor>,
        mut accepted: Option<&mut AcceptedDurableBoxTail>,
    ) -> Result<DurableBoxPrefixReleaseReceipt, super::StateError> {
        let (checkpoint_floor, retained_group_floor) = if let Some(cursor) = oldest_retained {
            if !self.validates_cursor_with_accepted(cursor, accepted.as_deref()) {
                return Err(super::StateError::InvalidCursor);
            }
            (cursor.checkpoint_entries, cursor.retained_groups)
        } else {
            (
                self.checkpoint_end()
                    .ok_or(super::StateError::InvalidCursor)?,
                self.retained_group_end()
                    .ok_or(super::StateError::InvalidCursor)?,
            )
        };

        let checkpoint_entries = self.release_checkpoint_entries(arena, checkpoint_floor)?;
        let mut retained_groups = self.release_retained_groups(arena, retained_group_floor)?;
        if let Some(accepted) = accepted.as_mut() {
            retained_groups =
                retained_groups.saturating_add(accepted.release_retained_group_prefix(
                    &mut self.owners,
                    arena,
                    retained_group_floor,
                )?);
        }
        if oldest_retained.is_none() {
            for group in &mut self.groups {
                group.checkpoint_pinned = false;
            }
            if let Some(accepted) = accepted.as_mut() {
                accepted.clear_checkpoint_pins();
            }
        }
        Ok(DurableBoxPrefixReleaseReceipt {
            checkpoint_entries,
            retained_groups,
        })
    }

    fn copy_group(
        owners: &mut DurableOwnerStore,
        arena: &mut PageMaterialArena,
        group: &DurableGroup,
    ) -> Result<DurableGroup, BankError> {
        let mut entries = Vec::with_capacity(group.entries.len());
        for mutation in &group.entries {
            entries.push(DurableMutation {
                index: mutation.index,
                alternate: Self::copy_value(owners, arena, mutation.alternate)?,
                alternate_level: mutation.alternate_level,
            });
        }
        Ok(DurableGroup {
            id: group.id,
            parent: group.parent,
            level: group.level,
            entries,
            checkpoint_pinned: true,
        })
    }

    fn install_mutation(
        &mut self,
        arena: &mut PageMaterialArena,
        index: u16,
        value: Option<DurableOwnerId>,
        level: u32,
        saved_at: Option<u32>,
    ) -> Result<(), BankError> {
        let before = {
            let cell = self.cell_mut(index);
            (std::mem::replace(&mut cell.value, value), cell.level)
        };
        self.cell_mut(index).level = level;
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

        let checkpoint_needed =
            self.checkpoint_stamps.get(&index).copied() != Some(self.checkpoint_epoch);
        let group_needed = saved_at.is_some();
        let operation_needed = !self.active_operations.is_empty();
        let destinations = usize::from(checkpoint_needed)
            + usize::from(group_needed)
            + usize::from(operation_needed);
        if destinations == 0 {
            Self::retire_value(&mut self.owners, arena, before.0);
            return Ok(());
        }

        let mut owner = Some(before.0);
        if checkpoint_needed {
            let alternate = if group_needed || operation_needed {
                Self::copy_value(
                    &mut self.owners,
                    arena,
                    *owner.as_ref().expect("checkpoint source"),
                )?
            } else {
                owner.take().expect("checkpoint owner")
            };
            self.checkpoint_entries.push(DurableMutation {
                index,
                alternate,
                alternate_level: before.1,
            });
            self.checkpoint_stamps.insert(index, self.checkpoint_epoch);
        }
        if group_needed {
            let alternate = if operation_needed {
                Self::copy_value(
                    &mut self.owners,
                    arena,
                    *owner.as_ref().expect("group source"),
                )?
            } else {
                owner.take().expect("group owner")
            };
            self.groups
                .last_mut()
                .expect("local save has an active group")
                .entries
                .push(DurableMutation {
                    index,
                    alternate,
                    alternate_level: before.1,
                });
        }
        if operation_needed {
            self.operation_entries.push(DurableMutation {
                index,
                alternate: owner.take().expect("operation owner"),
                alternate_level: before.1,
            });
        }
        Ok(())
    }

    pub(crate) fn assign(
        &mut self,
        arena: &mut PageMaterialArena,
        index: u16,
        value: Option<DurableNodeClosure>,
        scope: super::AssignmentScope,
        current_level: u32,
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
        self.install_mutation(arena, index, value, level, saved_at)
    }

    pub(crate) fn replace(
        &mut self,
        arena: &mut PageMaterialArena,
        index: u16,
        value: Option<DurableNodeClosure>,
    ) -> Result<(), BankError> {
        let value = value.map(|owner| self.owners.insert(owner));
        let level = self.cell(index).map_or(LEVEL_ONE, |cell| cell.level);
        self.install_mutation(arena, index, value, level, None)
    }

    fn can_take_unique(&self, index: u16) -> bool {
        self.checkpoint_stamps.get(&index).copied() == Some(self.checkpoint_epoch)
            && self
                .groups
                .last()
                .is_none_or(|group| group.entries.iter().any(|entry| entry.index == index))
    }

    pub(crate) fn copy_to_page(
        &self,
        arena: &mut PageMaterialArena,
        index: u16,
    ) -> Result<Option<crate::page_node_arena::PageListId>, BankError> {
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
            if !self.active_operations.is_empty() {
                let level = self.cell(index).map_or(LEVEL_ONE, |cell| cell.level);
                let transfer =
                    arena.loan_durable_to_page_in_place(self.owners.owner_slot_mut(owner));
                let (root, loan) = transfer.map_err(|_| {
                    self.record_failed_unique_take(index, owner);
                    self.cell_mut(index).value = Some(owner);
                    BankError::AllocationFailed
                })?;
                let mutation_position = self.operation_entries.len();
                self.operation_entries.push(DurableMutation {
                    index,
                    alternate: Some(owner),
                    alternate_level: level,
                });
                self.transfer_loans.push(DurableBoxTransferLoan {
                    mutation_position,
                    loan,
                });
                return Ok(Some(root));
            }
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

    pub(crate) fn begin_group(&mut self, level: u32) {
        self.next_group_id = self.next_group_id.checked_add(1).expect("box group id");
        self.groups.push(DurableGroup {
            id: self.next_group_id,
            parent: self.groups.last().map_or(0, |group| group.id),
            level,
            entries: Vec::new(),
            checkpoint_pinned: false,
        });
    }

    pub(crate) fn end_group(
        &mut self,
        arena: &mut PageMaterialArena,
        level: u32,
    ) -> Result<Vec<DurableGroupRestoration>, BankError> {
        let group = self.groups.pop().expect("durable group exists");
        assert_eq!(group.level, level);
        let retained = group
            .checkpoint_pinned
            .then(|| Self::copy_group(&mut self.owners, arena, &group))
            .transpose()?;
        let mut restorations = Vec::with_capacity(group.entries.len());
        for mut mutation in group.entries.into_iter().rev() {
            let saved = mutation
                .alternate
                .map(|owner| DurableNodeMetadata::from_closure(self.owners.owner(owner)));
            if self.cell_mut(mutation.index).level == LEVEL_ONE {
                let live = self.metadata(mutation.index);
                restorations.push(DurableGroupRestoration {
                    index: mutation.index,
                    saved,
                    live,
                    outcome: super::GroupRestorationOutcome::Retained,
                });
                Self::retire_value(&mut self.owners, arena, mutation.alternate);
            } else {
                let restored = mutation.alternate.take();
                self.install_mutation(
                    arena,
                    mutation.index,
                    restored,
                    mutation.alternate_level,
                    None,
                )?;
                let live = self.metadata(mutation.index);
                restorations.push(DurableGroupRestoration {
                    index: mutation.index,
                    saved,
                    live,
                    outcome: super::GroupRestorationOutcome::Restored,
                });
            }
        }
        if let Some(retained) = retained {
            self.retained_groups.push(retained);
        }
        Ok(restorations)
    }

    pub(crate) fn checkpoint_cursor(&mut self) -> DurableBoxCursor {
        for group in &mut self.groups {
            group.checkpoint_pinned = true;
        }
        let cursor = DurableBoxCursor {
            checkpoint_entries: self
                .checkpoint_end()
                .expect("durable checkpoint position overflow"),
            retained_groups: self
                .retained_group_end()
                .expect("durable group position overflow"),
            group_id: self.groups.last().map_or(0, |group| group.id),
            group_entry_position: self.groups.last().map_or(0, |group| group.entries.len()),
            group_depth: self.groups.len(),
            next_group_id: self.next_group_id,
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
        accepted: Option<&AcceptedDurableBoxTail>,
    ) -> bool {
        self.validates_cursor_with_accepted(cursor, accepted)
    }

    fn validates_cursor_with_accepted(
        &self,
        cursor: DurableBoxCursor,
        accepted: Option<&AcceptedDurableBoxTail>,
    ) -> bool {
        let Some(checkpoint_entries) = cursor
            .checkpoint_entries
            .checked_sub(self.checkpoint_entry_base)
        else {
            return false;
        };
        let current_contains_groups = cursor.retained_groups >= self.retained_group_base
            && self
                .retained_group_end()
                .is_some_and(|end| cursor.retained_groups <= end);
        if checkpoint_entries > self.checkpoint_entries.len()
            || !(current_contains_groups
                || accepted.is_some_and(|tail| {
                    tail.contains_retained_group_position(cursor.retained_groups)
                }))
        {
            return false;
        }
        if cursor.group_depth == 0 {
            return cursor.group_id == 0 && cursor.group_entry_position == 0;
        }
        let find_group = |id| {
            self.groups
                .iter()
                .chain(&self.retained_groups)
                .find(|group| group.id == id)
                .or_else(|| accepted.and_then(|tail| tail.group(id)))
        };
        let Some(inner) = find_group(cursor.group_id) else {
            return false;
        };
        if cursor.group_entry_position > inner.entries.len() {
            return false;
        }
        let mut depth = 0;
        let mut id = cursor.group_id;
        while id != 0 {
            depth += 1;
            let Some(group) = find_group(id) else {
                return false;
            };
            id = group.parent;
        }
        depth == cursor.group_depth
    }

    fn checkpoint_groups(
        &mut self,
        arena: &mut PageMaterialArena,
        cursor: DurableBoxCursor,
    ) -> Result<Vec<DurableGroup>, BankError> {
        if cursor.group_depth == 0 {
            return Ok(Vec::new());
        }
        let mut ids = Vec::with_capacity(cursor.group_depth);
        let mut id = cursor.group_id;
        while id != 0 {
            ids.push(id);
            id = self
                .groups
                .iter()
                .chain(&self.retained_groups)
                .find(|group| group.id == id)
                .expect("validated durable group ancestry")
                .parent;
        }
        ids.reverse();
        let mut result = Vec::with_capacity(ids.len());
        for id in ids {
            let source = self
                .groups
                .iter()
                .chain(&self.retained_groups)
                .find(|group| group.id == id)
                .expect("validated durable group remains retained");
            result.push(Self::copy_group(&mut self.owners, arena, source)?);
        }
        let inner = result
            .last_mut()
            .expect("non-root cursor has an inner group");
        for mutation in inner.entries.drain(cursor.group_entry_position..) {
            Self::retire_value(&mut self.owners, arena, mutation.alternate);
        }
        Ok(result)
    }

    pub(crate) fn begin_operation(&mut self) -> DurableBoxOperation {
        let operation = DurableBoxOperation {
            position: self.operation_entries.len(),
            loan_position: self.transfer_loans.len(),
            group_position: self.groups.len(),
        };
        self.active_operations.push(operation.position);
        operation
    }

    pub(crate) fn commit_operation(
        &mut self,
        arena: &mut PageMaterialArena,
        operation: DurableBoxOperation,
    ) {
        assert_eq!(self.active_operations.pop(), Some(operation.position));
        if self.active_operations.is_empty() {
            for loan in self.transfer_loans.drain(..) {
                arena.commit_durable_transfer_loan(loan.loan);
            }
            for mutation in self.operation_entries.drain(..) {
                Self::retire_value(&mut self.owners, arena, mutation.alternate);
            }
        }
    }

    pub(crate) fn rollback_operation(
        &mut self,
        arena: &mut PageMaterialArena,
        operation: DurableBoxOperation,
    ) {
        assert_eq!(self.active_operations.last(), Some(&operation.position));
        let loans = self.transfer_loans.split_off(operation.loan_position);
        for loan in loans.into_iter().rev() {
            let owner = arena
                .rollback_durable_transfer_loan(loan.loan)
                .expect("rollbackable durable transfer returns its exact owner");
            let mutation = self
                .operation_entries
                .get_mut(loan.mutation_position)
                .expect("durable transfer loan names its operation mutation");
            let owner_slot = mutation
                .alternate
                .expect("durable transfer mutation retains its owner slot");
            self.owners.restore(owner_slot, owner);
        }
        let mut suffix = self.operation_entries.split_off(operation.position);
        for mutation in suffix.iter_mut().rev() {
            self.swap_mutation(mutation);
        }
        for mutation in suffix {
            Self::retire_value(&mut self.owners, arena, mutation.alternate);
        }
        while self.groups.len() > operation.group_position {
            let group = self
                .groups
                .pop()
                .expect("operation-created group remains live");
            Self::retire_group(&mut self.owners, arena, group);
        }
        self.active_operations.pop();
    }

    fn swap_checkpoint_suffix(&mut self, start: usize) {
        let mut suffix = self.checkpoint_entries.split_off(start);
        for mutation in suffix.iter_mut().rev() {
            self.swap_mutation(mutation);
        }
        self.checkpoint_entries.append(&mut suffix);
    }

    pub(crate) fn restore(&mut self, arena: &mut PageMaterialArena, cursor: DurableBoxCursor) {
        let rebased = self
            .rebase_cursor(cursor)
            .expect("validated durable cursor rebases");
        let restored_groups = self
            .checkpoint_groups(arena, cursor)
            .expect("checkpoint group preservation copy must succeed");
        self.swap_checkpoint_suffix(rebased.checkpoint_entries);
        for mutation in self.checkpoint_entries.drain(rebased.checkpoint_entries..) {
            Self::retire_value(&mut self.owners, arena, mutation.alternate);
        }
        for group in std::mem::take(&mut self.groups) {
            Self::retire_group(&mut self.owners, arena, group);
        }
        for group in std::mem::take(&mut self.retained_groups) {
            Self::retire_group(&mut self.owners, arena, group);
        }
        self.groups = restored_groups;
        self.retained_group_base = cursor.retained_groups;
        self.next_group_id = cursor.next_group_id;
        self.checkpoint_stamps.clear();
        self.checkpoint_epoch = self.checkpoint_epoch.checked_add(1).expect("box epoch");
    }

    pub(crate) fn begin_checkpoint_candidate(
        &mut self,
        arena: &mut PageMaterialArena,
        cursor: DurableBoxCursor,
    ) -> Result<AcceptedDurableBoxTail, BankError> {
        assert!(self.active_operations.is_empty());
        let rebased = self
            .rebase_cursor(cursor)
            .expect("validated durable candidate cursor rebases");
        let candidate_groups = self.checkpoint_groups(arena, cursor)?;
        self.swap_checkpoint_suffix(rebased.checkpoint_entries);
        let entries = self
            .checkpoint_entries
            .split_off(rebased.checkpoint_entries);
        self.checkpoint_stamps.clear();
        self.checkpoint_epoch = self.checkpoint_epoch.checked_add(1).expect("box epoch");
        let accepted_groups = std::mem::replace(&mut self.groups, candidate_groups);
        let accepted_retained_groups = self.retained_groups.split_off(rebased.retained_groups);
        let retained_group_base = cursor.retained_groups;
        let groups = if cursor.group_depth == 0 {
            AcceptedDurableGroupTail::Root {
                accepted_groups,
                accepted_retained_groups,
                next_group_id: self.next_group_id,
            }
        } else {
            AcceptedDurableGroupTail::Arbitrary {
                accepted_groups,
                accepted_retained_groups,
                next_group_id: self.next_group_id,
            }
        };
        self.next_group_id = cursor.next_group_id;
        Ok(AcceptedDurableBoxTail {
            entries,
            groups,
            retained_group_base,
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
        for mutation in self.checkpoint_entries.drain(rebased.checkpoint_entries..) {
            Self::retire_value(&mut self.owners, arena, mutation.alternate);
        }
        for mutation in &mut accepted.entries {
            self.swap_mutation(mutation);
        }
        self.checkpoint_entries.append(&mut accepted.entries);
        for group in std::mem::take(&mut self.groups) {
            Self::retire_group(&mut self.owners, arena, group);
        }
        let candidate_retained_groups = self.retained_groups.split_off(rebased.retained_groups);
        for group in candidate_retained_groups {
            Self::retire_group(&mut self.owners, arena, group);
        }
        debug_assert_eq!(
            self.retained_group_end(),
            Some(accepted.retained_group_base)
        );
        match accepted.groups {
            AcceptedDurableGroupTail::Root {
                accepted_groups,
                accepted_retained_groups,
                next_group_id,
            }
            | AcceptedDurableGroupTail::Arbitrary {
                accepted_groups,
                accepted_retained_groups,
                next_group_id,
            } => {
                self.groups = accepted_groups;
                self.retained_groups.extend(accepted_retained_groups);
                self.next_group_id = next_group_id;
            }
        }
        self.checkpoint_stamps.clear();
        self.checkpoint_epoch = self.checkpoint_epoch.checked_add(1).expect("box epoch");
    }

    pub(crate) fn accept_checkpoint_candidate(
        &mut self,
        arena: &mut PageMaterialArena,
        accepted: AcceptedDurableBoxTail,
    ) {
        for mutation in accepted.entries {
            Self::retire_value(&mut self.owners, arena, mutation.alternate);
        }
        match accepted.groups {
            AcceptedDurableGroupTail::Root {
                accepted_groups,
                accepted_retained_groups,
                ..
            }
            | AcceptedDurableGroupTail::Arbitrary {
                accepted_groups,
                accepted_retained_groups,
                ..
            } => {
                for group in accepted_groups {
                    Self::retire_group(&mut self.owners, arena, group);
                }
                for group in accepted_retained_groups {
                    Self::retire_group(&mut self.owners, arena, group);
                }
            }
        }
        self.checkpoint_stamps.clear();
        self.checkpoint_epoch = self.checkpoint_epoch.checked_add(1).expect("box epoch");
    }

    pub(crate) fn retire_all(mut self, arena: &mut PageMaterialArena) {
        for cell in &mut self.dense {
            Self::retire_value(&mut self.owners, arena, cell.value.take());
        }
        for (_, mut cell) in self.overflow.drain() {
            Self::retire_value(&mut self.owners, arena, cell.value.take());
        }
        for mutation in self.checkpoint_entries.drain(..) {
            Self::retire_value(&mut self.owners, arena, mutation.alternate);
        }
        for group in self.groups.drain(..) {
            for mutation in group.entries {
                Self::retire_value(&mut self.owners, arena, mutation.alternate);
            }
        }
        for group in self.retained_groups.drain(..) {
            for mutation in group.entries {
                Self::retire_value(&mut self.owners, arena, mutation.alternate);
            }
        }
        for mutation in self.operation_entries.drain(..) {
            Self::retire_value(&mut self.owners, arena, mutation.alternate);
        }
    }
}
