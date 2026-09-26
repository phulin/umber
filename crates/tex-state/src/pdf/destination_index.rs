//! Derived identity-to-row lookup for the two checkpointed destination ledgers.

use super::destination::PdfDestinationIdentity;
use std::collections::HashMap;

#[derive(Debug, Default)]
pub(super) struct DestinationIndex {
    accepted: HashMap<PdfDestinationIdentity, usize>,
    candidate: HashMap<PdfDestinationIdentity, usize>,
}

impl DestinationIndex {
    /// Estimates reserved bucket and cloned-name bytes. `HashMap` control
    /// bytes and allocator rounding are implementation details and omitted.
    pub(super) fn estimated_allocated_bytes(&self) -> usize {
        [&self.accepted, &self.candidate]
            .into_iter()
            .map(|map| {
                map.capacity() * std::mem::size_of::<(PdfDestinationIdentity, usize)>()
                    + map
                        .keys()
                        .map(|identity| match identity {
                            PdfDestinationIdentity::Name(name) => name.len(),
                            PdfDestinationIdentity::Number(_) => 0,
                        })
                        .sum::<usize>()
            })
            .sum()
    }

    pub(super) fn get(
        &self,
        identity: &PdfDestinationIdentity,
        base_len: Option<usize>,
    ) -> Option<usize> {
        if let Some(&row) = self.candidate.get(identity) {
            return Some(row);
        }
        self.accepted
            .get(identity)
            .copied()
            .filter(|&row| base_len.is_none_or(|base| row < base))
    }

    pub(super) fn insert(&mut self, identity: PdfDestinationIdentity, row: usize, candidate: bool) {
        let map = if candidate {
            &mut self.candidate
        } else {
            &mut self.accepted
        };
        let old = map.insert(identity, row);
        debug_assert!(old.is_none(), "destination identity is reserved once");
    }

    pub(super) fn truncate(&mut self, len: usize, candidate: bool) {
        let map = if candidate {
            &mut self.candidate
        } else {
            &mut self.accepted
        };
        map.retain(|_, row| *row < len);
    }

    pub(super) fn reject_candidate(&mut self) {
        self.candidate.clear();
    }

    pub(super) fn accept_candidate(&mut self, base_len: usize) {
        self.accepted.retain(|_, row| *row < base_len);
        self.accepted.extend(self.candidate.drain());
    }
}
