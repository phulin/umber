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

/// Publication sidecar for one coarse episode over the canonical count bank
/// and journal.
///
/// This value deliberately owns no semantic state and does not borrow the
/// universe.  The command processor can therefore retain it while it lends
/// the same universe to canonical source delivery and expansion.  Mutations
/// still address the live canonical banks; the sidecar merely coalesces their
/// dependency and exact-identity publication until the episode commits.
#[doc(hidden)]
pub struct CountGroupEpisode {
    changed_counts: [u64; DENSE_COUNT_WORDS],
}

impl CountGroupEpisode {
    pub(crate) fn begin(universe: &mut Universe) -> Result<Self, CountGroupEpisodeBarrier> {
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
            changed_counts: [0; DENSE_COUNT_WORDS],
        })
    }

    /// Reads one live canonical count register without opening an observation.
    #[must_use]
    pub(crate) fn count(&self, universe: &Universe, index: u8) -> i32 {
        universe.stores.count(u16::from(index))
    }

    /// Applies one local or global write to the canonical count bank.
    pub(crate) fn set_count(
        &mut self,
        universe: &mut Universe,
        index: u8,
        value: i32,
        global: bool,
    ) {
        let index = u16::from(index);
        let receipt = if global {
            universe.stores.set_count_global(index, value)
        } else {
            universe.stores.set_count(index, value)
        };
        if receipt.changed() {
            let index = usize::from(index);
            self.changed_counts[index / COUNT_WORD_BITS] |= 1 << (index % COUNT_WORD_BITS);
        }
    }

    /// Returns the live canonical group depth.
    #[must_use]
    pub(crate) fn group_depth(&self, universe: &Universe) -> u32 {
        universe.stores.env_group_depth()
    }

    /// Returns the live canonical innermost group kind.
    #[must_use]
    pub(crate) fn innermost_group_kind(&self, universe: &Universe) -> Option<GroupKind> {
        universe.stores.innermost_group_kind()
    }

    /// Enters one group through the ordinary aggregate state boundary.
    pub(crate) fn enter_group(&mut self, universe: &mut Universe, kind: GroupKind) {
        self.flush_count_mutations(universe);
        universe.enter_group_with_kind(kind);
    }

    /// Leaves one group through the ordinary aggregate restoration boundary.
    pub(crate) fn leave_group(
        &mut self,
        universe: &mut Universe,
        expected: GroupKind,
    ) -> Result<(), GroupMismatch> {
        self.flush_count_mutations(universe);
        let aftergroup = universe.leave_group_with_kind(expected)?;
        debug_assert!(
            aftergroup.is_empty(),
            "native count/group episode does not admit aftergroup payloads"
        );
        Ok(())
    }

    /// Publishes the episode's last coalesced count mutation set.
    pub(crate) fn finish(mut self, universe: &mut Universe) {
        self.flush_count_mutations(universe);
    }

    fn flush_count_mutations(&mut self, universe: &mut Universe) {
        if self.changed_counts.iter().all(|word| *word == 0) {
            return;
        }
        universe.stores.synchronize_exact_env_identity();
        let dependencies = universe
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
