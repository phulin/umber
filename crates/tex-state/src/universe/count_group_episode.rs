//! Borrow-scoped direct access to canonical count and group state.
//!
//! A coarse command episode has no observation or checkpoint boundary inside
//! it, so changed-at and exact-identity publication may be coalesced until the
//! next group boundary. The live values and every restoration record still go
//! directly through [`Universe`]'s ordinary count banks and environment
//! journal.

use crate::cell::{BankTag, CellId};
use crate::dependency::{DependencyKey, TrackedRegionBarrier};
use crate::env::banks::IntParam;
use crate::{GroupKind, GroupMismatch, Universe};

const COUNT_WORD_BITS: usize = u64::BITS as usize;
const DENSE_COUNT_WORDS: usize = 256 / COUNT_WORD_BITS;

/// Why a coarse count/group episode cannot begin against the current state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CountGroupEpisodeBarrier {
    /// A tracked computation would require per-operation read publication.
    ActiveTrackedRegion,
    /// Group entry or restoration would emit an observable trace.
    ObservableGroupTracing,
}

/// A borrow-scoped coarse episode over the canonical count bank and journal.
///
/// This type deliberately owns no semantic state. Dropping it publishes any
/// pending count mutations, including on an error path before an enclosing
/// aggregate rollback restores the operation snapshot.
#[doc(hidden)]
pub struct CountGroupEpisode<'a> {
    universe: &'a mut Universe,
    changed_counts: [u64; DENSE_COUNT_WORDS],
}

impl<'a> CountGroupEpisode<'a> {
    pub(super) fn begin(universe: &'a mut Universe) -> Result<Self, CountGroupEpisodeBarrier> {
        if universe.dependency_region_is_active() {
            universe.poison_tracked_region(TrackedRegionBarrier::UnsupportedExecutionState);
            return Err(CountGroupEpisodeBarrier::ActiveTrackedRegion);
        }
        if universe.stores.int_param(IntParam::TRACING_GROUPS) != 0
            || universe.stores.int_param(IntParam::TRACING_RESTORES) != 0
        {
            return Err(CountGroupEpisodeBarrier::ObservableGroupTracing);
        }
        Ok(Self {
            universe,
            changed_counts: [0; DENSE_COUNT_WORDS],
        })
    }

    /// Reads one live canonical count register without opening an observation.
    #[must_use]
    pub fn count(&self, index: u8) -> i32 {
        self.universe.stores.count(u16::from(index))
    }

    /// Applies one local or global write to the canonical count bank.
    pub fn set_count(&mut self, index: u8, value: i32, global: bool) {
        let index = u16::from(index);
        let receipt = if global {
            self.universe.stores.set_count_global(index, value)
        } else {
            self.universe.stores.set_count(index, value)
        };
        if receipt.changed() {
            let index = usize::from(index);
            self.changed_counts[index / COUNT_WORD_BITS] |= 1 << (index % COUNT_WORD_BITS);
        }
    }

    /// Returns the live canonical group depth.
    #[must_use]
    pub fn group_depth(&self) -> u32 {
        self.universe.stores.env_group_depth()
    }

    /// Returns the live canonical innermost group kind.
    #[must_use]
    pub fn innermost_group_kind(&self) -> Option<GroupKind> {
        self.universe.stores.innermost_group_kind()
    }

    /// Enters one group through the ordinary aggregate state boundary.
    pub fn enter_group(&mut self, kind: GroupKind) {
        self.flush_count_mutations();
        self.universe.enter_group_with_kind(kind);
    }

    /// Leaves one group through the ordinary aggregate restoration boundary.
    pub fn leave_group(&mut self, expected: GroupKind) -> Result<(), GroupMismatch> {
        self.flush_count_mutations();
        let aftergroup = self.universe.leave_group_with_kind(expected)?;
        debug_assert!(
            aftergroup.is_empty(),
            "native count/group episode does not admit aftergroup payloads"
        );
        Ok(())
    }

    /// Publishes the episode's last coalesced count mutation set.
    pub fn finish(mut self) {
        self.flush_count_mutations();
    }

    fn flush_count_mutations(&mut self) {
        if self.changed_counts.iter().all(|word| *word == 0) {
            return;
        }
        self.universe.stores.synchronize_exact_env_identity();
        let dependencies = self
            .universe
            .dependencies
            .get_mut()
            .expect("dependency runtime mutex is not poisoned");
        for (word_index, word) in self.changed_counts.iter_mut().enumerate() {
            let mut pending = std::mem::take(word);
            while pending != 0 {
                let bit = pending.trailing_zeros() as usize;
                let index = word_index * COUNT_WORD_BITS + bit;
                dependencies.mark_changed(DependencyKey::Cell(CellId::new(
                    BankTag::Count,
                    index as u32,
                )));
                pending &= pending - 1;
            }
        }
    }
}

impl Drop for CountGroupEpisode<'_> {
    fn drop(&mut self) {
        self.flush_count_mutations();
    }
}
