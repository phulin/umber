//! Cold, owning dense undo tails. No node-region topology belongs to eqtb undo.

use std::collections::VecDeque;

use super::CheckpointDelta;
use crate::StateError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct CheckpointCut {
    identity: u64,
    position: usize,
}

struct AcceptedTail<G> {
    entries: VecDeque<CheckpointDelta<G>>,
    cuts: Vec<CheckpointCut>,
    start: usize,
    first_candidate_identity: u64,
}

pub(super) struct CheckpointUndoLog<G> {
    entries: VecDeque<CheckpointDelta<G>>,
    pub(super) roots: crate::definition_arena::DefinitionRoots,
    base: usize,
    cuts: Vec<CheckpointCut>,
    next_identity: u64,
    accepted: Option<AcceptedTail<G>>,
}

impl<G> CheckpointUndoLog<G> {
    pub(super) fn new(roots: crate::definition_arena::DefinitionRoots) -> Self {
        Self {
            entries: VecDeque::new(),
            roots,
            base: 0,
            cuts: Vec::new(),
            next_identity: 1,
            accepted: None,
        }
    }
}

impl<G> CheckpointUndoLog<G> {
    fn truncate_entries(&mut self, len: usize) {
        for delta in self.entries.drain(len..) {
            self.roots.release_word(&delta.alternate);
        }
    }

    pub(super) fn capture(&mut self) -> Result<CheckpointCut, StateError> {
        let identity = self.next_identity;
        let next = identity
            .checked_add(1)
            .ok_or(StateError::CheckpointEpochExhausted)?;
        self.cuts
            .try_reserve(1)
            .map_err(|_| StateError::Bank(crate::env::banks::BankError::AllocationFailed))?;
        let cut = CheckpointCut {
            identity,
            position: self.end(),
        };
        self.cuts.push(cut);
        self.next_identity = next;
        Ok(cut)
    }

    pub(super) fn retained(&self) -> bool {
        !self.cuts.is_empty()
    }

    pub(super) fn validates(&self, cut: CheckpointCut) -> bool {
        cut.position >= self.base
            && cut.position <= self.end()
            && self
                .cuts
                .binary_search_by_key(&cut.identity, |cut| cut.identity)
                .is_ok_and(|index| self.cuts[index] == cut)
    }

    pub(super) fn end(&self) -> usize {
        self.base + self.entries.len()
    }

    pub(super) fn len(&self) -> usize {
        self.entries.len() + self.accepted.as_ref().map_or(0, |tail| tail.entries.len())
    }

    pub(super) fn capacity(&self) -> usize {
        self.entries.capacity()
            + self
                .accepted
                .as_ref()
                .map_or(0, |tail| tail.entries.capacity())
    }

    pub(super) fn retained_bytes(&self) -> usize {
        self.capacity() * std::mem::size_of::<CheckpointDelta<G>>()
            + (self.cuts.capacity()
                + self
                    .accepted
                    .as_ref()
                    .map_or(0, |tail| tail.cuts.capacity()))
                * std::mem::size_of::<CheckpointCut>()
    }

    pub(super) fn push(&mut self, delta: CheckpointDelta<G>) {
        self.roots.acquire_word(&delta.alternate);
        self.entries.push_back(delta);
    }

    pub(super) fn visit_suffix(
        &self,
        cut: CheckpointCut,
        mut visit: impl FnMut(&CheckpointDelta<G>),
    ) {
        assert!(self.validates(cut));
        self.entries
            .iter()
            .skip(cut.position - self.base)
            .for_each(&mut visit);
    }

    #[cfg(test)]
    pub(super) fn visit_prefix(&self, cut: CheckpointCut, visit: impl FnMut(&CheckpointDelta<G>)) {
        assert!(self.validates(cut));
        self.entries
            .iter()
            .take(cut.position - self.base)
            .for_each(visit);
    }

    pub(super) fn reverse_suffix(
        &mut self,
        cut: CheckpointCut,
        visit: impl FnMut(&mut CheckpointDelta<G>),
    ) {
        assert!(self.validates(cut));
        self.entries
            .iter_mut()
            .skip(cut.position - self.base)
            .rev()
            .for_each(visit);
    }

    pub(super) fn restore(&mut self, cut: CheckpointCut) {
        assert!(self.validates(cut));
        self.truncate_entries(cut.position - self.base);
        self.cuts.truncate(
            self.cuts
                .binary_search_by_key(&cut.identity, |cut| cut.identity)
                .expect("validated checkpoint cut remains in its branch")
                + 1,
        );
    }

    pub(super) fn is_candidate(&self) -> bool {
        self.accepted.is_some()
    }

    pub(super) fn begin_candidate(&mut self, cut: CheckpointCut) -> Result<(), StateError> {
        if self.is_candidate() || !self.validates(cut) {
            return Err(StateError::InvalidCursor);
        }
        let cut_index = self
            .cuts
            .binary_search_by_key(&cut.identity, |cut| cut.identity)
            .expect("validated checkpoint cut remains in its branch");
        self.accepted = Some(AcceptedTail {
            entries: self.entries.split_off(cut.position - self.base),
            cuts: self.cuts.split_off(cut_index + 1),
            start: cut.position,
            first_candidate_identity: self.next_identity,
        });
        Ok(())
    }

    pub(super) fn forward_accepted(&mut self, visit: impl FnMut(&mut CheckpointDelta<G>)) {
        self.accepted
            .as_mut()
            .expect("one accepted tail during candidate execution")
            .entries
            .iter_mut()
            .for_each(visit);
    }

    pub(super) fn visit_accepted_prefix(
        &self,
        cut: CheckpointCut,
        mut visit: impl FnMut(&CheckpointDelta<G>),
    ) -> bool {
        let Some(tail) = &self.accepted else {
            return false;
        };
        let retained = self.cuts.iter().chain(&tail.cuts).any(|root| *root == cut);
        if !retained || cut.position < self.base || cut.position > tail.start + tail.entries.len() {
            return false;
        }
        self.entries
            .iter()
            .take(
                cut.position
                    .saturating_sub(self.base)
                    .min(tail.start - self.base),
            )
            .for_each(&mut visit);
        tail.entries
            .iter()
            .take(cut.position.saturating_sub(tail.start))
            .for_each(visit);
        true
    }

    pub(super) fn reject(&mut self) {
        let tail = self.accepted.take().expect("one accepted tail");
        self.truncate_entries(tail.start - self.base);
        self.cuts
            .retain(|cut| cut.identity < tail.first_candidate_identity);
        self.entries.extend(tail.entries);
        self.cuts.extend(tail.cuts);
    }

    pub(super) fn accept(&mut self) -> Result<(), StateError> {
        let mut tail = self.accepted.take().ok_or(StateError::InvalidCursor)?;
        for delta in tail.entries.drain(..) {
            self.roots.release_word(&delta.alternate);
        }
        Ok(())
    }

    pub(super) fn release_prefix(&mut self, cut: CheckpointCut) -> Result<usize, StateError> {
        if self.is_candidate() || !self.validates(cut) {
            return Err(StateError::InvalidCursor);
        }
        let released = cut.position - self.base;
        for delta in self.entries.drain(..released) {
            self.roots.release_word(&delta.alternate);
        }
        self.base = cut.position;
        self.cuts.retain(|root| root.identity >= cut.identity);
        self.entries.shrink_to_fit();
        self.cuts.shrink_to_fit();
        Ok(released)
    }

    pub(super) fn release_root(
        &mut self,
        released: CheckpointCut,
        oldest: Option<CheckpointCut>,
    ) -> Result<(), StateError> {
        if self.is_candidate()
            || !self.validates(released)
            || oldest.is_some_and(|cut| !self.validates(cut))
        {
            return Err(StateError::InvalidCursor);
        }
        if let Some(oldest) = oldest {
            self.release_prefix(oldest)?;
            self.cuts.retain(|cut| cut.identity != released.identity);
        } else {
            self.base = self.end();
            self.truncate_entries(0);
            self.cuts.clear();
            self.entries.shrink_to_fit();
            self.cuts.shrink_to_fit();
        }
        Ok(())
    }

    pub(super) fn release_capture(&mut self, released: CheckpointCut) {
        self.cuts.retain(|cut| *cut != released);
        if self.cuts.is_empty() && !self.is_candidate() {
            self.base = self.end();
            self.truncate_entries(0);
            self.entries.shrink_to_fit();
        }
    }
}

impl<G> Drop for CheckpointUndoLog<G> {
    fn drop(&mut self) {
        self.truncate_entries(0);
        if let Some(mut tail) = self.accepted.take() {
            for delta in tail.entries.drain(..) {
                self.roots.release_word(&delta.alternate);
            }
        }
    }
}
