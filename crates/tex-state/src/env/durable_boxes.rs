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
    /// Position of this save in the shared TeX group-save order. Checkpoint
    /// and operation inverses do not participate in group restoration.
    group_save_position: u32,
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

    fn restore(&mut self, id: DurableOwnerId, owner: DurableNodeClosure) {
        let slot = self.slot_mut(id);
        assert!(slot.owner.replace(owner).is_none());
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
    scalar_entries: usize,
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
    scalar_entries: usize,
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
    depth: usize,
    position: usize,
    action_position: usize,
    scalar_position: usize,
    group_position: usize,
    /// Existing saves survive operation rollback; only the newly created
    /// suffix is retired, preserving TeX82 §283 group restoration.
    group_entry_position: usize,
}

struct DurableBoxTransferLoan {
    mutation_position: usize,
    loan: crate::page_node_arena::DurableTransferLoan,
}

struct PageBoxTransferLoan {
    owner: DurableOwnerId,
    loan: crate::node_region::PageInteriorTransferLoan,
}

struct PageOutputCarrierLoan {
    owner: DurableOwnerId,
    loan: crate::page_node_arena::PageOutputRegionLoan,
}

pub(crate) struct OutputCarrierTarget {
    pub(crate) index: u16,
    pub(crate) scope: super::AssignmentScope,
    pub(crate) current_level: u32,
    pub(crate) group_save_position: u32,
}

struct RegisterTakeLoan {
    source: u16,
    destination: u16,
    owner: DurableOwnerId,
    serial: u64,
    destination_binding: Option<usize>,
}

/// One take from this state's live register binding. The owner stays in the
/// state-owned operation action; this opaque receipt carries no owner key.
pub struct UniqueBoxRegisterTake {
    state_id: u64,
    serial: u64,
    action_position: usize,
    operation_depth: usize,
}

/// The move-only closure and its exact page interval reversal travel together
/// across the assignment boundary.
pub(crate) struct PageBoxAssignment {
    pub(crate) closure: DurableNodeClosure,
    pub(crate) loan: crate::node_region::PageInteriorTransferLoan,
}

enum DurableOperationAction {
    Binding(usize),
    RegisterTake(RegisterTakeLoan),
    DurableToPage(DurableBoxTransferLoan),
    PageToDurable(PageBoxTransferLoan),
    OutputCarrier(Box<PageOutputCarrierLoan>),
    Dimension(DurableDimensionMutation),
    PageScalar(crate::page::PageOutputBoxDimensionInverse),
}

impl DurableBoxState {
    /// A live cross-region loan may still name the current page region.
    pub(crate) fn has_pending_page_region_loan(&self) -> bool {
        self.operation_actions.iter().any(|action| {
            matches!(
                action,
                DurableOperationAction::DurableToPage(_)
                    | DurableOperationAction::PageToDurable(_)
                    | DurableOperationAction::OutputCarrier(_)
            )
        })
    }

    #[cfg(test)]
    pub(crate) fn testing_fail_next_history_copy(&mut self) {
        self.fail_history_copy_after = Some(0);
    }
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

/// TeX's active box saves during one synchronous unsave. The original
/// mutation vector remains the only owner; two cursors visit its dense and
/// sparse subsequences without copying or allocating intermediate records.
pub(crate) struct BoxUnsave<'a, 'arena> {
    state: &'a mut DurableBoxState,
    arena: &'a mut PageMaterialArena<'arena>,
    group: DurableGroup,
    retained: Option<DurableGroup>,
    dense: Option<usize>,
    sparse: Option<usize>,
}

impl BoxUnsave<'_, '_> {
    pub(crate) fn len(&self) -> usize {
        self.group.entries.len()
    }

    fn key(&self, index: usize) -> u64 {
        (u64::from(self.group.entries[index].group_save_position) << 32)
            | u64::from(u32::try_from(index).expect("box saves fit u32"))
    }

    pub(crate) fn next_key(&self, sparse: bool) -> Option<u64> {
        (if sparse { self.sparse } else { self.dense }).map(|index| self.key(index))
    }

    pub(crate) fn sparse_boundary(&self) -> Option<u64> {
        self.group
            .entries
            .iter()
            .position(|entry| entry.index > 255)
            .map(|index| self.key(index))
    }

    pub(crate) fn restore(&mut self, sparse: bool) -> Result<DurableGroupRestoration, BankError> {
        let index =
            if sparse { self.sparse } else { self.dense }.expect("selected box save exists");
        let mutation = &mut self.group.entries[index];
        let saved = mutation
            .alternate
            .map(|owner| DurableNodeMetadata::from_closure(self.state.owners.owner(owner)));
        let outcome = if self.state.cell_mut(mutation.index).level == LEVEL_ONE {
            DurableBoxState::retire_value(
                &mut self.state.owners,
                self.arena,
                mutation.alternate.take(),
            );
            super::GroupRestorationOutcome::Retained
        } else {
            self.state.install_mutation(
                self.arena,
                mutation.index,
                mutation.alternate.take(),
                mutation.alternate_level,
                None,
                0,
            )?;
            super::GroupRestorationOutcome::Restored
        };
        let restored = DurableGroupRestoration {
            index: mutation.index,
            saved,
            live: self.state.metadata(mutation.index),
            outcome,
        };
        let next = self.group.entries[..index]
            .iter()
            .rposition(|entry| (entry.index > 255) == sparse);
        if sparse {
            self.sparse = next;
        } else {
            self.dense = next;
        }
        Ok(restored)
    }

    pub(crate) fn finish(mut self) {
        debug_assert!(self.dense.is_none() && self.sparse.is_none());
        if let Some(retained) = self.retained.take() {
            self.state.retained_groups.push(retained);
        }
    }
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
    scalar_stamps: HashMap<(u16, crate::command_context::BoxDimension), u64>,
    checkpoint_stamps: HashMap<u16, u64>,
    checkpoint_epoch: u64,
    checkpoint_anchored: bool,
    groups: Vec<DurableGroup>,
    retained_groups: Vec<DurableGroup>,
    retained_group_base: usize,
    next_group_id: u64,
    semantic_identity: Option<crate::state_hash::SemanticMapIdentity>,
    operation_entries: Vec<DurableMutation>,
    operation_actions: Vec<DurableOperationAction>,
    operation_depth: usize,
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
            scalar_stamps: HashMap::new(),
            checkpoint_stamps: HashMap::new(),
            checkpoint_epoch: 1,
            checkpoint_anchored: false,
            groups: Vec::new(),
            retained_groups: Vec::new(),
            retained_group_base: 0,
            next_group_id: 0,
            semantic_identity: None,
            operation_entries: Vec::new(),
            operation_actions: Vec::new(),
            operation_depth: 0,
        }
    }

    /// Reports whether this auxiliary state lane is at the level-zero
    /// checkpoint boundary. Durable-box group saves and operation transfers
    /// have the same admission rule as dense eqtb saves: a checkpoint never
    /// captures an open group or a live transactional suffix.
    pub(crate) fn checkpoint_eligible(&self) -> bool {
        self.groups.is_empty()
            && self.retained_groups.is_empty()
            && !self.operation_is_active()
            && self.operation_entries.is_empty()
            && self.operation_actions.is_empty()
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

    fn scalar_end(&self) -> Option<usize> {
        self.scalar_entry_base
            .checked_add(self.scalar_entries.len())
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
        let scalar_entries = cursor.scalar_entries.checked_sub(self.scalar_entry_base)?;
        let retained_groups = cursor
            .retained_groups
            .checked_sub(self.retained_group_base)?;
        if checkpoint_entries > self.checkpoint_entries.len()
            || scalar_entries > self.scalar_entries.len()
            || retained_groups > self.retained_groups.len()
        {
            return None;
        }
        Some(RebasedDurableBoxCursor {
            checkpoint_entries,
            scalar_entries,
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
        let Some((checkpoint_floor, scalar_floor, retained_group_floor)) = oldest_retained
            .map(|cursor| {
                self.validates_cursor_with_accepted(cursor, accepted)
                    .then_some((
                        cursor.checkpoint_entries,
                        cursor.scalar_entries,
                        cursor.retained_groups,
                    ))
            })
            .unwrap_or_else(|| {
                Some((
                    self.checkpoint_end()?,
                    self.scalar_end()?,
                    self.retained_group_end()?,
                ))
            })
        else {
            return false;
        };
        let checkpoint_valid = checkpoint_floor
            .checked_sub(self.checkpoint_entry_base)
            .is_some_and(|released| released <= self.checkpoint_entries.len());
        let scalar_valid = scalar_floor
            .checked_sub(self.scalar_entry_base)
            .is_some_and(|released| released <= self.scalar_entries.len());
        let groups_valid = retained_group_floor <= self.retained_group_base
            || retained_group_floor
                .checked_sub(self.retained_group_base)
                .is_some_and(|released| released <= self.retained_groups.len());
        checkpoint_valid
            && scalar_valid
            && groups_valid
            && accepted.is_none_or(|tail| tail.validates_retained_group_floor(retained_group_floor))
    }

    pub(crate) fn release_checkpoint_prefix(
        &mut self,
        arena: &mut PageMaterialArena,
        oldest_retained: Option<DurableBoxCursor>,
        mut accepted: Option<&mut AcceptedDurableBoxTail>,
    ) -> Result<DurableBoxPrefixReleaseReceipt, super::StateError> {
        let (checkpoint_floor, scalar_floor, retained_group_floor) =
            if let Some(cursor) = oldest_retained {
                if !self.validates_cursor_with_accepted(cursor, accepted.as_deref()) {
                    return Err(super::StateError::InvalidCursor);
                }
                (
                    cursor.checkpoint_entries,
                    cursor.scalar_entries,
                    cursor.retained_groups,
                )
            } else {
                (
                    self.checkpoint_end()
                        .ok_or(super::StateError::InvalidCursor)?,
                    self.scalar_end().ok_or(super::StateError::InvalidCursor)?,
                    self.retained_group_end()
                        .ok_or(super::StateError::InvalidCursor)?,
                )
            };

        let checkpoint_entries = self.release_checkpoint_entries(arena, checkpoint_floor)?;
        let released_scalars = scalar_floor
            .checked_sub(self.scalar_entry_base)
            .filter(|released| *released <= self.scalar_entries.len())
            .ok_or(super::StateError::InvalidCursor)?;
        self.scalar_entries.drain(..released_scalars);
        self.scalar_entry_base = scalar_floor;
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
            self.checkpoint_anchored = false;
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
                group_save_position: mutation.group_save_position,
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
        group_save_position: u32,
    ) -> Result<(), BankError> {
        let checkpoint_needed = self.checkpoint_anchored
            && self.checkpoint_stamps.get(&index).copied() != Some(self.checkpoint_epoch);
        let group_needed = saved_at.is_some();
        let operation_needed = self.operation_is_active();
        assert!(!group_needed || !self.groups.is_empty());
        let before = self
            .cell(index)
            .map_or((None, LEVEL_ONE), |cell| (cell.value, cell.level));
        // Historical owners are copied before the live binding changes. If a
        // later copy fails, the already staged copy retires and neither the
        // cell nor any inverse lane has been published.
        let checkpoint_copy = if checkpoint_needed && (group_needed || operation_needed) {
            self.check_history_copy_failure()?;
            Self::copy_value(&mut self.owners, arena, before.0)?
        } else {
            None
        };
        let group_copy = if group_needed && operation_needed {
            if let Err(error) = self.check_history_copy_failure() {
                Self::retire_value(&mut self.owners, arena, checkpoint_copy);
                return Err(error);
            }
            match Self::copy_value(&mut self.owners, arena, before.0) {
                Ok(copy) => copy,
                Err(error) => {
                    Self::retire_value(&mut self.owners, arena, checkpoint_copy);
                    return Err(error);
                }
            }
        } else {
            None
        };
        self.cell_mut(index).value = value;
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
                checkpoint_copy
            } else {
                owner.take().expect("checkpoint owner")
            };
            self.checkpoint_entries.push(DurableMutation {
                index,
                alternate,
                alternate_level: before.1,
                group_save_position: 0,
            });
            self.checkpoint_stamps.insert(index, self.checkpoint_epoch);
        }
        if group_needed {
            let alternate = if operation_needed {
                group_copy
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
                    group_save_position,
                });
        }
        if operation_needed {
            let position = self.operation_entries.len();
            self.operation_entries.push(DurableMutation {
                index,
                alternate: owner.take().expect("operation owner"),
                alternate_level: before.1,
                group_save_position: 0,
            });
            self.operation_actions
                .push(DurableOperationAction::Binding(position));
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

    #[cfg(test)]
    pub(crate) fn assign(
        &mut self,
        arena: &mut PageMaterialArena,
        index: u16,
        value: Option<DurableNodeClosure>,
        scope: super::AssignmentScope,
        current_level: u32,
    ) -> Result<(), BankError> {
        self.assign_with_group_position(arena, index, value, scope, current_level, 0)
    }

    pub(crate) fn assign_with_group_position(
        &mut self,
        arena: &mut PageMaterialArena,
        index: u16,
        value: Option<DurableNodeClosure>,
        scope: super::AssignmentScope,
        current_level: u32,
        group_save_position: u32,
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
        self.install_mutation(arena, index, value, level, saved_at, group_save_position)
    }

    pub(crate) fn assign_with_page_loan(
        &mut self,
        arena: &mut PageMaterialArena,
        index: u16,
        assignment: PageBoxAssignment,
        scope: super::AssignmentScope,
        current_level: u32,
        group_save_position: u32,
    ) -> Result<(), BankError> {
        let PageBoxAssignment { closure, loan } = assignment;
        let owner = self.owners.insert(closure);
        // The interval moved before the binding changed. Record that order so
        // rollback can reverse a later dimension edit, the binding swap, and
        // finally the page loan without leaving a live cell on an empty slot.
        if self.operation_is_active() {
            self.operation_actions
                .push(DurableOperationAction::PageToDurable(PageBoxTransferLoan {
                    owner,
                    loan,
                }));
        }
        let before_level = self.cell(index).map_or(LEVEL_ONE, |cell| cell.level);
        let level = match scope {
            super::AssignmentScope::Global => LEVEL_ONE,
            super::AssignmentScope::Local => current_level,
        };
        let saved_at = (scope == super::AssignmentScope::Local
            && current_level != LEVEL_ONE
            && before_level != current_level)
            .then_some(current_level);
        self.install_mutation(
            arena,
            index,
            Some(owner),
            level,
            saved_at,
            group_save_position,
        )
    }

    /// Publishes the consumed output carrier after its page-region swap.
    /// Historical destination copies remain fallible, so a rejected install
    /// immediately returns the complete old region and builder slots before
    /// the caller can attempt another assignment.
    pub(crate) fn assign_output_carrier(
        &mut self,
        arena: &mut PageMaterialArena,
        page: &mut crate::page::PageBuilderState,
        target: OutputCarrierTarget,
        assignment: crate::page_node_arena::PageOutputCarrierAssignment,
    ) -> Result<(), BankError> {
        assert!(
            self.operation_is_active(),
            "output carrier needs an operation"
        );
        let owner = self.owners.insert(assignment.closure);
        self.operation_actions
            .push(DurableOperationAction::OutputCarrier(Box::new(
                PageOutputCarrierLoan {
                    owner,
                    loan: assignment.loan,
                },
            )));
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
            target.group_save_position,
        ) {
            let Some(DurableOperationAction::OutputCarrier(loan)) = self.operation_actions.pop()
            else {
                unreachable!("rejected binding did not publish an inverse")
            };
            arena
                .rollback_output_carrier_loan(page, self.owners.owner_slot_mut(owner), loan.loan)
                .expect("rejected binding returns its exact output region");
            self.owners.retire(arena, owner);
            return Err(error);
        }
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
        self.install_mutation(arena, index, value, level, None, 0)
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
            && self.checkpoint_stamps.get(&index).copied() != Some(self.checkpoint_epoch);
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
                && self.scalar_stamps.get(&(index, dimension)).copied()
                    != Some(self.checkpoint_epoch)
            {
                self.scalar_entries.push(mutation);
                self.scalar_stamps
                    .insert((index, dimension), self.checkpoint_epoch);
            }
            if self.operation_is_active() {
                self.operation_actions
                    .push(DurableOperationAction::Dimension(mutation));
            }
        }
        Ok(true)
    }

    pub(crate) fn record_page_output_box_dimension(
        &mut self,
        inverse: crate::page::PageOutputBoxDimensionInverse,
    ) {
        if self.operation_is_active() {
            self.operation_actions
                .push(DurableOperationAction::PageScalar(inverse));
        }
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
            || self.checkpoint_stamps.get(&index).copied() == Some(self.checkpoint_epoch)
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

    /// Clears the semantic source first, as TeX's `make_box` does, then leaves
    /// its exclusive owner in the current operation until assignment. This
    /// also makes same-register tracing observe a void old destination.
    pub(crate) fn begin_unique_register_take(
        &mut self,
        source: u16,
        destination: u16,
    ) -> Option<UniqueBoxRegisterTake> {
        if !self.has_unique_current(source) {
            return None;
        }
        assert!(
            self.operation_is_active(),
            "register take needs a rollback operation"
        );
        let serial = self.next_take_serial.checked_add(1).expect("take serial");
        let owner = self.cell_mut(source).value.take().expect("admitted source");
        self.record_unique_take(source, owner);
        self.next_take_serial = serial;
        let action_position = self.operation_actions.len();
        self.operation_actions
            .push(DurableOperationAction::RegisterTake(RegisterTakeLoan {
                source,
                destination,
                owner,
                serial,
                destination_binding: None,
            }));
        Some(UniqueBoxRegisterTake {
            state_id: self.state_id,
            serial,
            action_position,
            operation_depth: self.operation_depth,
        })
    }

    /// Assigns the previously cleared source. Historical destination copies
    /// are staged before any binding or journal changes; failure returns the
    /// owner to its source and removes the pending operation action.
    pub(crate) fn finish_unique_register_take(
        &mut self,
        arena: &mut PageMaterialArena,
        take: UniqueBoxRegisterTake,
        destination: u16,
        scope: super::AssignmentScope,
        current_level: u32,
        group_save_position: u32,
    ) -> Result<(), BankError> {
        assert_eq!(take.state_id, self.state_id, "foreign box take");
        assert_eq!(
            take.operation_depth, self.operation_depth,
            "stale box operation"
        );
        assert_eq!(
            self.operation_actions.len(),
            take.action_position + 1,
            "box take must finish before another durable mutation"
        );
        let (source, owner) = match self.operation_actions.get(take.action_position) {
            Some(DurableOperationAction::RegisterTake(loan))
                if loan.serial == take.serial
                    && loan.destination == destination
                    && loan.destination_binding.is_none() =>
            {
                (loan.source, loan.owner)
            }
            _ => panic!("box take is not pending in its owner store"),
        };
        let destination_binding = self.operation_entries.len();
        let before_level = self.cell(destination).map_or(LEVEL_ONE, |cell| cell.level);
        let level = match scope {
            super::AssignmentScope::Global => LEVEL_ONE,
            super::AssignmentScope::Local => current_level,
        };
        let saved_at = (scope == super::AssignmentScope::Local
            && current_level != LEVEL_ONE
            && before_level != current_level)
            .then_some(current_level);
        if let Err(error) = self.install_mutation(
            arena,
            destination,
            Some(owner),
            level,
            saved_at,
            group_save_position,
        ) {
            self.operation_actions.pop();
            assert!(self.cell(source).and_then(|cell| cell.value).is_none());
            self.cell_mut(source).value = Some(owner);
            self.record_failed_unique_take(source, owner);
            return Err(error);
        }
        let Some(DurableOperationAction::RegisterTake(loan)) =
            self.operation_actions.get_mut(take.action_position)
        else {
            unreachable!()
        };
        loan.destination_binding = Some(destination_binding);
        Ok(())
    }

    #[cfg(test)]
    fn assign_unique_register_take(
        &mut self,
        arena: &mut PageMaterialArena,
        source: u16,
        destination: u16,
        scope: super::AssignmentScope,
        current_level: u32,
        group_save_position: u32,
    ) -> Result<bool, BankError> {
        let Some(take) = self.begin_unique_register_take(source, destination) else {
            return Ok(false);
        };
        self.finish_unique_register_take(
            arena,
            take,
            destination,
            scope,
            current_level,
            group_save_position,
        )?;
        Ok(true)
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
            if self.operation_is_active() {
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
                    group_save_position: 0,
                });
                self.operation_actions
                    .push(DurableOperationAction::Binding(mutation_position));
                self.operation_actions
                    .push(DurableOperationAction::DurableToPage(
                        DurableBoxTransferLoan {
                            mutation_position,
                            loan,
                        },
                    ));
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

    pub(crate) fn begin_unsave<'a, 'arena>(
        &'a mut self,
        arena: &'a mut PageMaterialArena<'arena>,
        level: u32,
    ) -> Result<BoxUnsave<'a, 'arena>, BankError> {
        let group = self.groups.pop().expect("durable group exists");
        assert_eq!(group.level, level);
        let retained = group
            .checkpoint_pinned
            .then(|| Self::copy_group(&mut self.owners, arena, &group))
            .transpose()?;
        let dense = group.entries.iter().rposition(|entry| entry.index <= 255);
        let sparse = group.entries.iter().rposition(|entry| entry.index > 255);
        Ok(BoxUnsave {
            state: self,
            arena,
            group,
            retained,
            dense,
            sparse,
        })
    }

    #[cfg(test)]
    pub(crate) fn end_group(
        &mut self,
        arena: &mut PageMaterialArena,
        level: u32,
    ) -> Result<Vec<DurableGroupRestoration>, BankError> {
        let mut unsave = self.begin_unsave(arena, level)?;
        let mut restored = Vec::new();
        while unsave.dense.is_some() {
            restored.push(unsave.restore(false)?);
        }
        while unsave.sparse.is_some() {
            restored.push(unsave.restore(true)?);
        }
        unsave.finish();
        Ok(restored)
    }

    pub(crate) fn checkpoint_cursor(&mut self) -> DurableBoxCursor {
        self.checkpoint_anchored = true;
        for group in &mut self.groups {
            group.checkpoint_pinned = true;
        }
        let cursor = DurableBoxCursor {
            checkpoint_entries: self
                .checkpoint_end()
                .expect("durable checkpoint position overflow"),
            scalar_entries: self.scalar_end().expect("durable scalar position overflow"),
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
        let Some(scalar_entries) = cursor.scalar_entries.checked_sub(self.scalar_entry_base) else {
            return false;
        };
        let current_contains_groups = cursor.retained_groups >= self.retained_group_base
            && self
                .retained_group_end()
                .is_some_and(|end| cursor.retained_groups <= end);
        if checkpoint_entries > self.checkpoint_entries.len()
            || scalar_entries > self.scalar_entries.len()
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
        self.operation_depth = self
            .operation_depth
            .checked_add(1)
            .expect("durable box operation depth exhausted");
        DurableBoxOperation {
            depth: self.operation_depth,
            position: self.operation_entries.len(),
            action_position: self.operation_actions.len(),
            scalar_position: self.scalar_entries.len(),
            group_position: self.groups.len(),
            group_entry_position: self.groups.last().map_or(0, |group| group.entries.len()),
        }
    }

    #[inline(always)]
    fn operation_is_active(&self) -> bool {
        self.operation_depth != 0
    }

    /// Settles an operation and reopens it in place. An operation that
    /// recorded nothing is already its own successor, so only one that
    /// touched a box register takes the full commit and begin.
    #[inline]
    pub(crate) fn roll_operation(
        &mut self,
        arena: &mut PageMaterialArena,
        operation: &mut DurableBoxOperation,
    ) {
        let unchanged = operation.depth == self.operation_depth
            && operation.position == self.operation_entries.len()
            && operation.action_position == self.operation_actions.len()
            && operation.scalar_position == self.scalar_entries.len()
            && operation.group_position == self.groups.len()
            && operation.group_entry_position
                == self.groups.last().map_or(0, |group| group.entries.len());
        if unchanged {
            return;
        }
        self.commit_operation(arena, *operation);
        *operation = self.begin_operation();
    }

    pub(crate) fn commit_operation(
        &mut self,
        arena: &mut PageMaterialArena,
        operation: DurableBoxOperation,
    ) {
        assert_eq!(self.operation_depth, operation.depth);
        assert!(
            !self.operation_actions[operation.action_position..]
                .iter()
                .any(|action| {
                    matches!(
                        action,
                        DurableOperationAction::RegisterTake(RegisterTakeLoan {
                            destination_binding: None,
                            ..
                        })
                    )
                }),
            "cannot commit an unfinished register take"
        );
        self.operation_depth -= 1;
        if !self.operation_is_active() {
            for action in self.operation_actions.drain(..) {
                match action {
                    DurableOperationAction::DurableToPage(loan) => {
                        arena.commit_durable_transfer_loan(loan.loan);
                    }
                    DurableOperationAction::OutputCarrier(loan) => {
                        arena
                            .commit_output_carrier_loan(loan.loan)
                            .expect("committed output loan retires its emptied old region");
                    }
                    DurableOperationAction::Binding(_)
                    | DurableOperationAction::RegisterTake(RegisterTakeLoan {
                        destination_binding: Some(_),
                        ..
                    })
                    | DurableOperationAction::PageToDurable(_)
                    | DurableOperationAction::Dimension(_)
                    | DurableOperationAction::PageScalar(_) => {}
                    DurableOperationAction::RegisterTake(RegisterTakeLoan {
                        destination_binding: None,
                        ..
                    }) => unreachable!(),
                }
            }
            for mutation in self.operation_entries.drain(..) {
                Self::retire_value(&mut self.owners, arena, mutation.alternate);
            }
        }
    }

    pub(crate) fn rollback_operation(
        &mut self,
        arena: &mut PageMaterialArena,
        page: &mut crate::page::PageBuilderState,
        operation: DurableBoxOperation,
    ) {
        assert_eq!(self.operation_depth, operation.depth);
        let actions = self.operation_actions.split_off(operation.action_position);
        for action in actions.into_iter().rev() {
            match action {
                DurableOperationAction::Binding(position) => {
                    let mut mutation = std::mem::replace(
                        self.operation_entries
                            .get_mut(position)
                            .expect("operation binding event names its inverse"),
                        DurableMutation {
                            index: 0,
                            alternate: None,
                            alternate_level: LEVEL_ONE,
                            group_save_position: 0,
                        },
                    );
                    self.swap_mutation(&mut mutation);
                    self.operation_entries[position] = mutation;
                }
                DurableOperationAction::RegisterTake(loan) => {
                    let displaced = if let Some(position) = loan.destination_binding {
                        self.operation_entries
                            .get_mut(position)
                            .expect("register take names its destination inverse")
                            .alternate
                            .take()
                            .expect("destination inverse holds the exclusive source owner")
                    } else {
                        loan.owner
                    };
                    assert_eq!(displaced, loan.owner);
                    assert!(self.cell(loan.source).and_then(|cell| cell.value).is_none());
                    self.cell_mut(loan.source).value = Some(displaced);
                    if self.semantic_identity.is_some() {
                        self.record_failed_unique_take(loan.source, displaced);
                    }
                }
                DurableOperationAction::DurableToPage(loan) => {
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
                DurableOperationAction::PageToDurable(loan) => {
                    arena
                        .rollback_interleaved_page_box(
                            self.owners.owner_slot_mut(loan.owner),
                            loan.loan,
                        )
                        .expect("page interval loan restores its original chunks");
                }
                DurableOperationAction::OutputCarrier(loan) => {
                    arena
                        .rollback_output_carrier_loan(
                            page,
                            self.owners.owner_slot_mut(loan.owner),
                            loan.loan,
                        )
                        .expect("page output loan restores its original region");
                }
                DurableOperationAction::Dimension(mutation) => {
                    self.apply_dimension_inverse(arena, mutation);
                }
                DurableOperationAction::PageScalar(inverse) => {
                    page.restore_output_box_dimension(arena, inverse)
                        .expect("page output scalar inverse retains its wrapper coordinate");
                }
            }
        }
        self.scalar_entries.truncate(operation.scalar_position);
        self.scalar_stamps.clear();
        let suffix = self.operation_entries.split_off(operation.position);
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
        if operation.group_position != 0 {
            let group = self
                .groups
                .last_mut()
                .expect("operation's existing group remains live");
            assert!(
                operation.group_entry_position <= group.entries.len(),
                "operation group save mark must remain within the live group"
            );
            while group.entries.len() > operation.group_entry_position {
                let mutation = group
                    .entries
                    .pop()
                    .expect("durable group save suffix is nonempty");
                Self::retire_value(&mut self.owners, arena, mutation.alternate);
            }
        }
        self.operation_depth -= 1;
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
        while self.scalar_entries.len() > rebased.scalar_entries {
            let mutation = self.scalar_entries.pop().expect("scalar suffix exists");
            self.apply_checkpoint_scalar_inverse(arena, mutation);
        }
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
        self.scalar_stamps.clear();
        self.checkpoint_epoch = self.checkpoint_epoch.checked_add(1).expect("box epoch");
    }

    pub(crate) fn begin_checkpoint_candidate(
        &mut self,
        arena: &mut PageMaterialArena,
        cursor: DurableBoxCursor,
    ) -> Result<AcceptedDurableBoxTail, BankError> {
        assert!(!self.operation_is_active());
        let rebased = self
            .rebase_cursor(cursor)
            .expect("validated durable candidate cursor rebases");
        let candidate_groups = self.checkpoint_groups(arena, cursor)?;
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
                    for group in candidate_groups {
                        Self::retire_group(&mut self.owners, arena, group);
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
        self.checkpoint_stamps.clear();
        self.scalar_stamps.clear();
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
            scalar_entries,
            forked_scalar_owners,
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
        self.scalar_stamps.clear();
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
        self.scalar_stamps.clear();
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
