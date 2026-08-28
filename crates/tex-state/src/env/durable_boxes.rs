//! Move-only durable box owners and their reversible TeX history.

use std::collections::HashMap;

use super::banks::{BankError, LEVEL_ONE};
use crate::node_region::NodeRegionId;
use crate::page_node_arena::{DurableNodeClosure, PageMaterialArena};

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct DurableNodeMetadata {
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

    pub(crate) const fn region(self) -> NodeRegionId {
        self.region
    }

    pub(crate) const fn len(self) -> usize {
        self.len
    }

    pub(crate) const fn semantic_identity(self) -> Option<u64> {
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
    level: u32,
    entries: Vec<DurableMutation>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DurableBoxCursor {
    checkpoint_entries: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DurableBoxOperation {
    position: usize,
}

pub(crate) struct AcceptedDurableBoxTail {
    entries: Vec<DurableMutation>,
}

pub(crate) struct DurableBoxState {
    dense: [DurableBoxCell; 256],
    overflow: HashMap<u16, DurableBoxCell>,
    checkpoint_entries: Vec<DurableMutation>,
    checkpoint_stamps: HashMap<u16, u64>,
    checkpoint_epoch: u64,
    groups: Vec<DurableGroup>,
    operation_entries: Vec<DurableMutation>,
    active_operations: Vec<usize>,
}

impl DurableBoxState {
    pub(crate) fn new() -> Self {
        Self {
            dense: std::array::from_fn(|_| DurableBoxCell::default()),
            overflow: HashMap::new(),
            checkpoint_entries: Vec::new(),
            checkpoint_stamps: HashMap::new(),
            checkpoint_epoch: 1,
            groups: Vec::new(),
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

    pub(crate) fn take_unique(&mut self, index: u16) -> Option<DurableNodeClosure> {
        self.cell_mut(index).value.take()
    }

    pub(crate) fn can_take_unique(&self, index: u16) -> bool {
        self.checkpoint_stamps.get(&index).copied() == Some(self.checkpoint_epoch)
            && self.active_operations.is_empty()
            && self
                .groups
                .last()
                .is_none_or(|group| group.entries.iter().any(|entry| entry.index == index))
    }

    pub(crate) fn begin_group(&mut self, level: u32) {
        self.groups.push(DurableGroup {
            level,
            entries: Vec::new(),
        });
    }

    pub(crate) fn end_group(&mut self, arena: &mut PageMaterialArena, level: u32) {
        let group = self.groups.pop().expect("durable group exists");
        assert_eq!(group.level, level);
        for mut mutation in group.entries.into_iter().rev() {
            let cell = self.cell_mut(mutation.index);
            if cell.level == LEVEL_ONE {
                Self::retire_value(arena, mutation.alternate);
            } else {
                std::mem::swap(&mut cell.value, &mut mutation.alternate);
                std::mem::swap(&mut cell.level, &mut mutation.alternate_level);
                Self::retire_value(arena, mutation.alternate);
            }
        }
    }

    pub(crate) fn checkpoint_cursor(&mut self) -> DurableBoxCursor {
        let cursor = DurableBoxCursor {
            checkpoint_entries: self.checkpoint_entries.len(),
        };
        self.checkpoint_epoch = self.checkpoint_epoch.checked_add(1).expect("box epoch");
        cursor
    }

    pub(crate) fn validates_cursor(&self, cursor: DurableBoxCursor) -> bool {
        cursor.checkpoint_entries <= self.checkpoint_entries.len()
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
            let cell = self.cell_mut(mutation.index);
            std::mem::swap(&mut cell.value, &mut mutation.alternate);
            std::mem::swap(&mut cell.level, &mut mutation.alternate_level);
        }
        for mutation in suffix {
            Self::retire_value(arena, mutation.alternate);
        }
        self.active_operations.pop();
    }

    fn swap_checkpoint_suffix(&mut self, start: usize) {
        let mut suffix = self.checkpoint_entries.split_off(start);
        for mutation in suffix.iter_mut().rev() {
            let cell = self.cell_mut(mutation.index);
            std::mem::swap(&mut cell.value, &mut mutation.alternate);
            std::mem::swap(&mut cell.level, &mut mutation.alternate_level);
        }
        self.checkpoint_entries.append(&mut suffix);
    }

    pub(crate) fn restore(&mut self, arena: &mut PageMaterialArena, cursor: DurableBoxCursor) {
        self.swap_checkpoint_suffix(cursor.checkpoint_entries);
        for mutation in self.checkpoint_entries.drain(cursor.checkpoint_entries..) {
            Self::retire_value(arena, mutation.alternate);
        }
        self.checkpoint_stamps.clear();
        self.checkpoint_epoch = self.checkpoint_epoch.checked_add(1).expect("box epoch");
    }

    pub(crate) fn begin_checkpoint_candidate(
        &mut self,
        cursor: DurableBoxCursor,
    ) -> AcceptedDurableBoxTail {
        assert!(self.groups.is_empty());
        assert!(self.active_operations.is_empty());
        self.swap_checkpoint_suffix(cursor.checkpoint_entries);
        let entries = self.checkpoint_entries.split_off(cursor.checkpoint_entries);
        self.checkpoint_stamps.clear();
        self.checkpoint_epoch = self.checkpoint_epoch.checked_add(1).expect("box epoch");
        AcceptedDurableBoxTail { entries }
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
            let cell = self.cell_mut(mutation.index);
            std::mem::swap(&mut cell.value, &mut mutation.alternate);
            std::mem::swap(&mut cell.level, &mut mutation.alternate_level);
        }
        self.checkpoint_entries.append(&mut accepted.entries);
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
        for mutation in self.operation_entries.drain(..) {
            Self::retire_value(arena, mutation.alternate);
        }
    }
}
