//! Region-scoped semantic dependency tracking for incremental computations.
//!
//! Keys name observable facts rather than storage locations.  Observations are
//! detached from a [`crate::Universe`]: live ids may be used to locate a fact,
//! but the recorded value is always a scalar or canonical content identity.

use crate::world::ContentHash;
use std::collections::BTreeMap;
use std::sync::Arc;

/// A monotonically increasing revision at which an observable fact changed.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ChangedAt(u64);

impl ChangedAt {
    /// The stamp used for facts that have not been explicitly mutated.
    pub const NEVER: Self = Self(0);

    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

/// Environment bank addressed by a [`DependencyKey::Cell`].
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum DependencyBank {
    Count,
    Dimen,
    Skip,
    Muskip,
    Toks,
    Box,
    IntParam,
    DimenParam,
    GlueParam,
    TokParam,
    CurrentFont,
    MathFamilyFont,
    LastBadness,
    Magnification,
}

/// One mutable TeX code table.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum DependencyCodeTable {
    Catcode,
    Lccode,
    Uccode,
    Sfcode,
    Mathcode,
    Delcode,
}

/// One observable field of a loaded font.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum DependencyFontField {
    Identifier,
    Name,
    Parameter,
    ParameterCount,
    HyphenChar,
    SkewChar,
    Metrics,
    PdfCode,
}

/// Executor-owned state that is not stored in an environment cell.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum DependencyEngineField {
    Mode,
    InnerMode,
    GroupLevel,
    GroupType,
    ConditionLevel,
    ConditionType,
    ConditionBranch,
    ConditionStack,
    LastNodeType,
    ParShape,
    PenaltyArrays,
    InteractionMode,
    PdfTimer,
    PdfRandom,
    PdfShellEscape,
    PageInsertions,
    PdfExternalImages,
    PdfObjects,
    PdfPositions,
    PdfForms,
    PdfPages,
}

/// Page-builder aggregates whose values are observed as canonical roots.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum DependencyPageField {
    Contents,
    Contributions,
    CurrentPage,
    Insertions,
    Discards,
    SplitDiscards,
    BreakState,
    FireUp,
}

/// Host-facing state kept behind [`crate::World`].
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum DependencyWorldField {
    InputResource,
    OutputStream,
    InputStream,
    TerminalInputCursor,
    EffectPolicy,
    ShellEscapePolicy,
    JobClock,
    Rng,
    LoadedResources,
    MaterializationBarrier,
}

/// Complete state-layer vocabulary for memoized reads.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum DependencyKey {
    Meaning(u32),
    Cell {
        bank: DependencyBank,
        index: u32,
    },
    Code {
        table: DependencyCodeTable,
        scalar: u32,
    },
    CodeGeneration(DependencyCodeTable),
    Font {
        field: DependencyFontField,
        font: u32,
        index: u32,
    },
    HyphenationPatterns(u8),
    HyphenationExceptions(u8),
    HyphenationCodes(u8),
    InputRecord(ContentHash),
    PhysicalLine {
        content: ContentHash,
        terminator: u8,
    },
    InputLine,
    InputStream(u8),
    InputStack,
    Engine(DependencyEngineField),
    PageDimension(u8),
    PageInteger(u8),
    PageMark(u8),
    PageMarkClass {
        mark: u8,
        class: u16,
    },
    Page(DependencyPageField),
    World {
        field: DependencyWorldField,
        index: u64,
    },
    /// A bounded parent dependency on a memoized child result.
    Query {
        domain: u32,
        identity: u64,
    },
}

/// A detached semantic value suitable for comparison across Universes.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum DependencyValue {
    Absent,
    Bool(bool),
    Integer(i64),
    Unsigned(u64),
    Content(ContentHash),
    /// A versioned canonical projection for structured values.
    Projection {
        schema: u32,
        fingerprint: u64,
    },
}

/// One dependency captured while a computation region executes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservedDependency {
    pub key: DependencyKey,
    pub changed_at: ChangedAt,
    pub value: DependencyValue,
}

/// Opaque validation identity for a memoized interpreter episode.
///
/// The universe nonce is process-local and never serialized. The state hash is
/// canonical and permits a detached entry to be reconsidered in an
/// allocation-distinct universe with equal semantic state.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MemoValidationStamp {
    universe_nonce: u64,
    state_hash: u64,
}

impl MemoValidationStamp {
    pub(crate) const fn new(universe_nonce: u64, state_hash: u64) -> Self {
        Self {
            universe_nonce,
            state_hash,
        }
    }

    /// Builds an owner-only probe for the fast path; its hash is not observed.
    #[must_use]
    pub const fn new_for_owner(universe_nonce: u64) -> Self {
        Self {
            universe_nonce,
            state_hash: 0,
        }
    }

    #[must_use]
    pub const fn same_universe(self, other: Self) -> bool {
        self.universe_nonce == other.universe_nonce
    }

    #[must_use]
    pub const fn state_hash(self) -> u64 {
        self.state_hash
    }
}

/// Deterministic, deduplicating recorder for one active computation region.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DependencyRegion {
    observations: BTreeMap<DependencyKey, ObservedDependency>,
}

impl DependencyRegion {
    pub fn record(&mut self, observation: ObservedDependency) {
        self.observations
            .entry(observation.key)
            .or_insert(observation);
    }

    #[must_use]
    pub fn observations(&self) -> impl ExactSizeIterator<Item = &ObservedDependency> {
        self.observations.values()
    }

    #[must_use]
    pub fn into_observations(self) -> Vec<ObservedDependency> {
        self.observations.into_values().collect()
    }
}

/// Result of validating one previously observed dependency.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DependencyValidation {
    /// Its changed-at stamp still matches; no semantic read was needed.
    Unchanged,
    /// Its stamp advanced, but its semantic value is equal and was backdated.
    Backdated,
    /// Its semantic value changed.
    Changed,
}

/// Session-local changed-at clock used by aggregate state facades.
#[derive(Clone, Debug, Default)]
pub struct DependencyTracker {
    revision: u64,
    changed: Arc<BTreeMap<DependencyKey, ChangedAt>>,
}

/// O(1) rollback root for changed-at metadata.
#[derive(Clone, Debug, Default)]
pub(crate) struct DependencyTrackerSnapshot {
    changed: Arc<BTreeMap<DependencyKey, ChangedAt>>,
}

/// Optional recording state installed around one interpreter computation.
///
/// Ordinary execution keeps `active` empty.  Recording therefore adds one
/// predictable branch and does not allocate, lock, or touch an atomic.
#[derive(Clone, Debug, Default)]
pub struct DependencyRuntime {
    tracker: DependencyTracker,
    active: Option<DependencyRegion>,
}

impl DependencyRuntime {
    /// Starts a region. Nested computations should instead record a bounded
    /// [`DependencyKey::Query`] in their parent.
    pub fn begin_region(&mut self) {
        assert!(self.active.is_none(), "dependency region already active");
        self.active = Some(DependencyRegion::default());
    }

    #[must_use]
    pub const fn is_recording(&self) -> bool {
        self.active.is_some()
    }

    /// Records a semantic read only when a region is active.
    #[inline(always)]
    pub fn record(&mut self, key: DependencyKey, value: DependencyValue) {
        if let Some(region) = &mut self.active {
            region.record(self.tracker.observe(key, value));
        }
    }

    /// Finishes the active region in canonical key order.
    pub fn finish_region(&mut self) -> Vec<ObservedDependency> {
        self.active
            .take()
            .expect("no dependency region is active")
            .into_observations()
    }

    pub fn mark_changed(&mut self, key: DependencyKey) -> ChangedAt {
        self.tracker.mark_changed(key)
    }

    /// Registers a key for changed-at tracking without opening a region.
    pub fn track(&mut self, key: DependencyKey) -> ChangedAt {
        self.tracker.track(key)
    }

    pub fn invalidate_all(&mut self) {
        self.tracker.invalidate_all();
    }

    pub(crate) fn snapshot_tracker(&self) -> DependencyTrackerSnapshot {
        DependencyTrackerSnapshot {
            changed: Arc::clone(&self.tracker.changed),
        }
    }

    pub(crate) fn restore_tracker(&mut self, snapshot: &DependencyTrackerSnapshot) {
        self.tracker.restore(snapshot);
    }

    #[must_use]
    pub const fn tracker(&self) -> &DependencyTracker {
        &self.tracker
    }
}

impl DependencyTracker {
    pub fn track(&mut self, key: DependencyKey) -> ChangedAt {
        *Arc::make_mut(&mut self.changed)
            .entry(key)
            .or_insert(ChangedAt::NEVER)
    }

    #[must_use]
    pub fn changed_at(&self, key: DependencyKey) -> ChangedAt {
        self.changed.get(&key).copied().unwrap_or(ChangedAt::NEVER)
    }

    /// Marks a fact after its aggregate mutation barrier has run.
    pub fn mark_changed(&mut self, key: DependencyKey) -> ChangedAt {
        let Some(changed_at) = Arc::make_mut(&mut self.changed).get_mut(&key) else {
            return ChangedAt::NEVER;
        };
        self.revision = self
            .revision
            .checked_add(1)
            .expect("dependency revision exhausted");
        let stamp = ChangedAt(self.revision);
        *changed_at = stamp;
        stamp
    }

    /// Advances every fact observed by this runtime after an aggregate restore.
    pub fn invalidate_all(&mut self) {
        if self.changed.is_empty() {
            return;
        }
        self.revision = self
            .revision
            .checked_add(1)
            .expect("dependency revision exhausted");
        let stamp = ChangedAt(self.revision);
        for changed_at in Arc::make_mut(&mut self.changed).values_mut() {
            *changed_at = stamp;
        }
    }

    #[must_use]
    pub fn observe(&mut self, key: DependencyKey, value: DependencyValue) -> ObservedDependency {
        Arc::make_mut(&mut self.changed)
            .entry(key)
            .or_insert(ChangedAt::NEVER);
        ObservedDependency {
            key,
            changed_at: self.changed_at(key),
            value,
        }
    }

    fn restore(&mut self, snapshot: &DependencyTrackerSnapshot) {
        if Arc::ptr_eq(&self.changed, &snapshot.changed) {
            return;
        }
        let mut restored = (*snapshot.changed).clone();
        let changed_keys = self
            .changed
            .iter()
            .filter_map(|(&key, &stamp)| {
                (snapshot.changed.get(&key).copied() != Some(stamp)).then_some(key)
            })
            .chain(
                snapshot
                    .changed
                    .keys()
                    .copied()
                    .filter(|key| !self.changed.contains_key(key)),
            )
            .collect::<Vec<_>>();
        if !changed_keys.is_empty() {
            self.revision = self
                .revision
                .checked_add(1)
                .expect("dependency revision exhausted");
            let stamp = ChangedAt(self.revision);
            for key in changed_keys {
                restored.insert(key, stamp);
            }
        }
        self.changed = Arc::new(restored);
    }

    /// Validates one observation, reading the current value only on a stamp miss.
    pub fn validate(
        &self,
        observation: &mut ObservedDependency,
        read_current: impl FnOnce(DependencyKey) -> DependencyValue,
    ) -> DependencyValidation {
        let current_stamp = self.changed_at(observation.key);
        if current_stamp == observation.changed_at {
            return DependencyValidation::Unchanged;
        }
        if read_current(observation.key) != observation.value {
            return DependencyValidation::Changed;
        }
        observation.changed_at = current_stamp;
        DependencyValidation::Backdated
    }

    /// Validates a deterministic region and stops at its first red dependency.
    pub fn validate_region(
        &self,
        observations: &mut [ObservedDependency],
        mut read_current: impl FnMut(DependencyKey) -> DependencyValue,
    ) -> bool {
        observations.iter_mut().all(|observation| {
            self.validate(observation, &mut read_current) != DependencyValidation::Changed
        })
    }
}

#[cfg(test)]
mod tests;
