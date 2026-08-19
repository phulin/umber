//! Test-feature facade for durable HotCore substrate benchmarks.
//!
//! Runtime coordinates and snapshots remain crate-private. This facade exposes
//! only workload construction, scalar observations, and bounded operations.

use core::mem::size_of;
use core::num::NonZeroU32;

use crate::hot_core::snapshot::{
    AcceptedHotCore, HotArenaKind, HotCore, HotCoreAccounting, HotSnapshot, HotStackKind,
};

const ARENAS: [HotArenaKind; 6] = [
    HotArenaKind::TokenWord,
    HotArenaKind::TokenList,
    HotArenaKind::MacroRecord,
    HotArenaKind::MacroRoot,
    HotArenaKind::Glue,
    HotArenaKind::Provenance,
];
const STACKS: [HotStackKind; 6] = [
    HotStackKind::Input,
    HotStackKind::Parameter,
    HotStackKind::Condition,
    HotStackKind::Group,
    HotStackKind::Save,
    HotStackKind::Mode,
];

/// Handle-free scalar accounting returned to external benchmark crates.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TestingHotCoreAccounting {
    pub arena_logical_values: usize,
    pub stack_logical_entries: usize,
    pub dense_logical_cells: usize,
    pub journal_logical_inverses: usize,
    pub active_snapshots: usize,
    pub retained_bytes: usize,
}

/// Storage-only aggregate workload for allocation and latency controls.
pub struct TestingHotCore {
    core: HotCore,
}

impl TestingHotCore {
    /// Builds a candidate with the requested already-live arena and stack words.
    pub fn with_live_words(live_words: usize) -> Self {
        let base = AcceptedHotCore::new(NonZeroU32::new(8).expect("constant is nonzero"))
            .expect("benchmark hot-core identity space remains available");
        let mut core = base
            .candidate(40, 0)
            .expect("benchmark candidate storage fits");
        let snapshot = core.snapshot().expect("benchmark population opens");
        for value in 0..live_words {
            for kind in ARENAS {
                let _ = core
                    .append_arena_word(kind, value as u64)
                    .expect("benchmark arena population fits");
            }
            for kind in STACKS {
                core.push_stack(kind, value as u64)
                    .expect("benchmark stack population fits");
            }
        }
        core.commit(snapshot)
            .expect("benchmark population snapshot commits");
        Self { core }
    }

    /// Opens and accepts an empty aggregate mark.
    pub fn snapshot_commit(&mut self) -> usize {
        let snapshot = self.core.snapshot().expect("benchmark snapshot opens");
        let shallow = size_of_val(&snapshot);
        self.core
            .commit(snapshot)
            .expect("benchmark snapshot commits");
        shallow
    }

    /// Warms every spill path used by the bounded retry workload.
    pub fn warm_bounded_cycle(&mut self) {
        let _ = self.rollback_cycle(0);
    }

    /// Mutates every storage family and rejects the suffix.
    pub fn rollback_cycle(&mut self, seed: u64) -> u64 {
        let snapshot = self.core.snapshot().expect("benchmark snapshot opens");
        mutate_suffix(&mut self.core, seed);
        let checksum = self
            .core
            .state_value(39)
            .expect("benchmark dense value is readable")
            ^ self.core.stack_len(HotStackKind::Mode) as u64;
        self.core
            .rollback(snapshot)
            .expect("benchmark snapshot rolls back");
        checksum
    }

    /// Runs the durable accept/reject/retry plateau workload.
    pub fn mixed_cycles(&mut self, cycles: u32) -> u64 {
        let mut checksum = 0_u64;
        for cycle in 0..cycles {
            checksum ^= self.snapshot_commit() as u64;
            checksum ^= self.rollback_cycle(u64::from(cycle));
            checksum ^= self.rollback_cycle(u64::from(cycle) + 1);
        }
        checksum
    }

    #[must_use]
    pub fn accounting(&self) -> TestingHotCoreAccounting {
        self.core.accounting().into()
    }

    #[must_use]
    pub const fn snapshot_size() -> usize {
        size_of::<HotSnapshot>()
    }

    #[must_use]
    pub const fn snapshot_retained_bytes() -> usize {
        0
    }
}

impl From<HotCoreAccounting> for TestingHotCoreAccounting {
    fn from(accounting: HotCoreAccounting) -> Self {
        Self {
            arena_logical_values: accounting.arena_logical_values,
            stack_logical_entries: accounting.stack_logical_entries,
            dense_logical_cells: accounting.dense_logical_cells,
            journal_logical_inverses: accounting.journal_logical_inverses,
            active_snapshots: accounting.active_snapshots,
            retained_bytes: accounting.retained_bytes,
        }
    }
}

fn mutate_suffix(core: &mut HotCore, seed: u64) {
    for (offset, kind) in ARENAS.into_iter().enumerate() {
        let _ = core
            .append_arena_word(kind, seed + offset as u64)
            .expect("benchmark arena suffix fits");
    }
    for kind in STACKS {
        for offset in 0..16_u64 {
            core.push_stack(kind, seed + offset)
                .expect("benchmark stack suffix fits");
        }
    }
    for index in 0..40 {
        core.write_state(index, seed + index as u64)
            .expect("benchmark inverse suffix fits");
    }
    core.advance_external_journals(1)
        .expect("benchmark external cursor advances");
}
