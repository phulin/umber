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
    pub(super) value: Option<DurableNodeClosure>,
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
    alternate: Option<DurableNodeClosure>,
    alternate_level: u32,
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
    checkpoint_entries: usize,
    group_id: u64,
    group_entry_position: usize,
    group_depth: usize,
    next_group_id: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DurableBoxOperation {
    position: usize,
}

pub(crate) struct AcceptedDurableBoxTail {
    entries: Vec<DurableMutation>,
    groups: AcceptedDurableGroupTail,
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

pub(crate) struct DurableGroupRestoration {
    pub(crate) index: u16,
    pub(crate) saved: Option<DurableNodeMetadata>,
    pub(crate) live: Option<DurableNodeMetadata>,
    pub(crate) outcome: super::GroupRestorationOutcome,
}

pub(crate) struct DurableBoxState {
    dense: Box<[DurableBoxCell]>,
    overflow: HashMap<u16, DurableBoxCell>,
    checkpoint_entries: Vec<DurableMutation>,
    checkpoint_stamps: HashMap<u16, u64>,
    checkpoint_epoch: u64,
    groups: Vec<DurableGroup>,
    retained_groups: Vec<DurableGroup>,
    next_group_id: u64,
    semantic_identity: Option<crate::state_hash::SemanticMapIdentity>,
    operation_entries: Vec<DurableMutation>,
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
            DurableBoxState::retire_value(arena, Some(entry.owner));
        }
    }

    pub(crate) fn accept_candidate(&mut self, arena: &mut PageMaterialArena) {
        let base_len = self
            .base_len
            .take()
            .expect("PDF form transaction is active");
        for entry in self.accepted.drain(base_len..) {
            DurableBoxState::retire_value(arena, Some(entry.owner));
        }
        self.accepted.append(&mut self.delta);
    }

    pub(crate) fn truncate(&mut self, arena: &mut PageMaterialArena, len: usize) {
        assert!(
            self.base_len.is_none(),
            "PDF form candidate is not truncated directly"
        );
        for entry in self.accepted.drain(len..) {
            DurableBoxState::retire_value(arena, Some(entry.owner));
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
            DurableBoxState::retire_value(arena, Some(entry.owner));
        }
        for entry in self.delta.drain(..) {
            DurableBoxState::retire_value(arena, Some(entry.owner));
        }
    }
}

impl DurableBoxState {
    pub(crate) fn new() -> Self {
        Self {
            dense: (0..=u8::MAX)
                .map(|_| DurableBoxCell::default())
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            overflow: HashMap::new(),
            checkpoint_entries: Vec::new(),
            checkpoint_stamps: HashMap::new(),
            checkpoint_epoch: 1,
            groups: Vec::new(),
            retained_groups: Vec::new(),
            next_group_id: 0,
            semantic_identity: None,
            operation_entries: Vec::new(),
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
            .and_then(|cell| cell.value.as_ref())
            .map(DurableNodeMetadata::from_closure)
    }

    pub(crate) fn value(&self, index: u16) -> Option<&DurableNodeClosure> {
        self.cell(index).and_then(|cell| cell.value.as_ref())
    }

    pub(crate) fn visit_current(&self, mut visit: impl FnMut(u16, &DurableNodeClosure)) {
        for (index, cell) in self.dense.iter().enumerate() {
            if let Some(owner) = &cell.value {
                visit(index as u16, owner);
            }
        }
        let mut overflow = self.overflow.iter().collect::<Vec<_>>();
        overflow.sort_unstable_by_key(|(index, _)| **index);
        for (&index, cell) in overflow {
            if let Some(owner) = &cell.value {
                visit(index, owner);
            }
        }
    }

    pub(crate) fn enable_semantic_identity(&mut self) -> bool {
        if self.semantic_identity.is_some() {
            return true;
        }
        let mut identity = crate::state_hash::SemanticMapIdentity::empty(0x626f_785f_726f_6f74);
        for (index, cell) in self.dense.iter().enumerate() {
            if let Some(owner) = &cell.value {
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
            if let Some(owner) = &cell.value {
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

    fn value_identity(value: &Option<DurableNodeClosure>) -> Option<u64> {
        value.as_ref().map(|owner| {
            owner
                .root()
                .list()
                .semantic_identity()
                .expect("identity demand precedes durable box publication")
        })
    }

    fn swap_mutation(&mut self, mutation: &mut DurableMutation) {
        let identities = self.semantic_identity.is_some().then(|| {
            (
                Self::value_identity(&self.cell_mut(mutation.index).value),
                Self::value_identity(&mutation.alternate),
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
        arena: &mut PageMaterialArena,
        value: &Option<DurableNodeClosure>,
    ) -> Result<Option<DurableNodeClosure>, BankError> {
        value
            .as_ref()
            .map(|value| arena.copy_durable_owner(value))
            .transpose()
            .map_err(|_| BankError::AllocationFailed)
    }

    fn retire_value(arena: &mut PageMaterialArena, value: Option<DurableNodeClosure>) {
        if let Some(value) = value {
            arena
                .retire_durable(value)
                .expect("durable journal owns a live closure");
        }
    }

    fn retire_group(arena: &mut PageMaterialArena, group: DurableGroup) {
        for mutation in group.entries {
            Self::retire_value(arena, mutation.alternate);
        }
    }

    fn copy_group(
        arena: &mut PageMaterialArena,
        group: &DurableGroup,
    ) -> Result<DurableGroup, BankError> {
        let mut entries = Vec::with_capacity(group.entries.len());
        for mutation in &group.entries {
            entries.push(DurableMutation {
                index: mutation.index,
                alternate: Self::copy_value(arena, &mutation.alternate)?,
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
        value: Option<DurableNodeClosure>,
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
                Self::value_identity(&before.0),
                Self::value_identity(&self.cell(index).expect("durable box cell exists").value),
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
            Self::retire_value(arena, before.0);
            return Ok(());
        }

        let mut owner = Some(before.0);
        if checkpoint_needed {
            let alternate = if group_needed || operation_needed {
                Self::copy_value(arena, owner.as_ref().expect("checkpoint source"))?
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
                Self::copy_value(arena, owner.as_ref().expect("group source"))?
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
        let level = self.cell(index).map_or(LEVEL_ONE, |cell| cell.level);
        self.install_mutation(arena, index, value, level, None)
    }

    fn can_take_unique(&self, index: u16) -> bool {
        self.checkpoint_stamps.get(&index).copied() == Some(self.checkpoint_epoch)
            && self.active_operations.is_empty()
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
            return arena
                .move_durable_to_page(owner)
                .map(Some)
                .map_err(|(_, owner)| {
                    self.cell_mut(index).value = Some(owner);
                    BankError::AllocationFailed
                });
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
            .then(|| Self::copy_group(arena, &group))
            .transpose()?;
        let mut restorations = Vec::with_capacity(group.entries.len());
        for mut mutation in group.entries.into_iter().rev() {
            let saved = mutation
                .alternate
                .as_ref()
                .map(DurableNodeMetadata::from_closure);
            if self.cell_mut(mutation.index).level == LEVEL_ONE {
                let live = self
                    .cell(mutation.index)
                    .expect("durable box cell")
                    .value
                    .as_ref()
                    .map(DurableNodeMetadata::from_closure);
                restorations.push(DurableGroupRestoration {
                    index: mutation.index,
                    saved,
                    live,
                    outcome: super::GroupRestorationOutcome::Retained,
                });
                Self::retire_value(arena, mutation.alternate);
            } else {
                let restored = mutation.alternate.take();
                self.install_mutation(
                    arena,
                    mutation.index,
                    restored,
                    mutation.alternate_level,
                    None,
                )?;
                let live = self
                    .cell(mutation.index)
                    .expect("durable box cell")
                    .value
                    .as_ref()
                    .map(DurableNodeMetadata::from_closure);
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
            checkpoint_entries: self.checkpoint_entries.len(),
            group_id: self.groups.last().map_or(0, |group| group.id),
            group_entry_position: self.groups.last().map_or(0, |group| group.entries.len()),
            group_depth: self.groups.len(),
            next_group_id: self.next_group_id,
        };
        self.checkpoint_epoch = self.checkpoint_epoch.checked_add(1).expect("box epoch");
        cursor
    }

    pub(crate) fn validates_cursor(&self, cursor: DurableBoxCursor) -> bool {
        if cursor.checkpoint_entries > self.checkpoint_entries.len() {
            return false;
        }
        if cursor.group_depth == 0 {
            return cursor.group_id == 0 && cursor.group_entry_position == 0;
        }
        let groups = self.groups.iter().chain(&self.retained_groups);
        let Some(inner) = groups.clone().find(|group| group.id == cursor.group_id) else {
            return false;
        };
        if cursor.group_entry_position > inner.entries.len() {
            return false;
        }
        let mut depth = 0;
        let mut id = cursor.group_id;
        while id != 0 {
            depth += 1;
            let Some(group) = self
                .groups
                .iter()
                .chain(&self.retained_groups)
                .find(|group| group.id == id)
            else {
                return false;
            };
            id = group.parent;
        }
        depth == cursor.group_depth
    }

    fn checkpoint_groups(
        &self,
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
            result.push(Self::copy_group(arena, source)?);
        }
        let inner = result
            .last_mut()
            .expect("non-root cursor has an inner group");
        for mutation in inner.entries.drain(cursor.group_entry_position..) {
            Self::retire_value(arena, mutation.alternate);
        }
        Ok(result)
    }

    pub(crate) fn begin_operation(&mut self) -> DurableBoxOperation {
        let operation = DurableBoxOperation {
            position: self.operation_entries.len(),
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
            for mutation in self.operation_entries.drain(..) {
                Self::retire_value(arena, mutation.alternate);
            }
        }
    }

    pub(crate) fn rollback_operation(
        &mut self,
        arena: &mut PageMaterialArena,
        operation: DurableBoxOperation,
    ) {
        assert_eq!(self.active_operations.last(), Some(&operation.position));
        let mut suffix = self.operation_entries.split_off(operation.position);
        for mutation in suffix.iter_mut().rev() {
            self.swap_mutation(mutation);
        }
        for mutation in suffix {
            Self::retire_value(arena, mutation.alternate);
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
        let restored_groups = self
            .checkpoint_groups(arena, cursor)
            .expect("checkpoint group preservation copy must succeed");
        self.swap_checkpoint_suffix(cursor.checkpoint_entries);
        for mutation in self.checkpoint_entries.drain(cursor.checkpoint_entries..) {
            Self::retire_value(arena, mutation.alternate);
        }
        for group in std::mem::take(&mut self.groups) {
            Self::retire_group(arena, group);
        }
        for group in std::mem::take(&mut self.retained_groups) {
            Self::retire_group(arena, group);
        }
        self.groups = restored_groups;
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
        let candidate_groups = self.checkpoint_groups(arena, cursor)?;
        self.swap_checkpoint_suffix(cursor.checkpoint_entries);
        let entries = self.checkpoint_entries.split_off(cursor.checkpoint_entries);
        self.checkpoint_stamps.clear();
        self.checkpoint_epoch = self.checkpoint_epoch.checked_add(1).expect("box epoch");
        let accepted_groups = std::mem::replace(&mut self.groups, candidate_groups);
        let accepted_retained_groups = std::mem::take(&mut self.retained_groups);
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
        Ok(AcceptedDurableBoxTail { entries, groups })
    }

    pub(crate) fn reject_checkpoint_candidate(
        &mut self,
        arena: &mut PageMaterialArena,
        cursor: DurableBoxCursor,
        mut accepted: AcceptedDurableBoxTail,
    ) {
        self.swap_checkpoint_suffix(cursor.checkpoint_entries);
        for mutation in self.checkpoint_entries.drain(cursor.checkpoint_entries..) {
            Self::retire_value(arena, mutation.alternate);
        }
        for mutation in &mut accepted.entries {
            self.swap_mutation(mutation);
        }
        self.checkpoint_entries.append(&mut accepted.entries);
        for group in std::mem::take(&mut self.groups) {
            Self::retire_group(arena, group);
        }
        for group in std::mem::take(&mut self.retained_groups) {
            Self::retire_group(arena, group);
        }
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
                self.retained_groups = accepted_retained_groups;
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
            Self::retire_value(arena, mutation.alternate);
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
                    Self::retire_group(arena, group);
                }
                for group in accepted_retained_groups {
                    Self::retire_group(arena, group);
                }
            }
        }
        self.checkpoint_stamps.clear();
        self.checkpoint_epoch = self.checkpoint_epoch.checked_add(1).expect("box epoch");
    }

    pub(crate) fn retire_all(mut self, arena: &mut PageMaterialArena) {
        for cell in &mut self.dense {
            Self::retire_value(arena, cell.value.take());
        }
        for (_, mut cell) in self.overflow.drain() {
            Self::retire_value(arena, cell.value.take());
        }
        for mutation in self.checkpoint_entries.drain(..) {
            Self::retire_value(arena, mutation.alternate);
        }
        for group in self.groups.drain(..) {
            for mutation in group.entries {
                Self::retire_value(arena, mutation.alternate);
            }
        }
        for group in self.retained_groups.drain(..) {
            for mutation in group.entries {
                Self::retire_value(arena, mutation.alternate);
            }
        }
        for mutation in self.operation_entries.drain(..) {
            Self::retire_value(arena, mutation.alternate);
        }
    }
}
