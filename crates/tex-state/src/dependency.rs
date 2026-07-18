//! Region-scoped semantic dependency tracking for incremental computations.
//!
//! Keys name observable facts rather than storage locations.  Observations are
//! detached from a [`crate::Universe`]: live ids may be used to locate a fact,
//! but the recorded value is always a scalar or canonical content identity.

use crate::world::ContentHash;
use ahash::AHashMap;
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
    /// Aggregate identity of the mutable font-parameter vector.
    Parameters,
    HyphenChar,
    SkewChar,
    Metrics,
    PdfCode,
    /// Aggregate identity of unconditional PDF lig/kern shaping state.
    PdfShaping,
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
    invalidated_at: ChangedAt,
    changed: Arc<AHashMap<DependencyKey, ChangedAt>>,
}

/// O(1) rollback root for changed-at metadata.
#[derive(Clone, Debug, Default)]
pub(crate) struct DependencyTrackerSnapshot {
    invalidated_at: ChangedAt,
    changed: Arc<AHashMap<DependencyKey, ChangedAt>>,
}

/// Optional recording state installed around one interpreter computation.
///
/// Ordinary execution keeps `active` empty.  Recording therefore adds one
/// predictable branch and does not allocate, lock, or touch an atomic.
#[derive(Clone, Debug, Default)]
pub struct DependencyRuntime {
    tracker: DependencyTracker,
    active: Option<DependencyRegion>,
    tracking_enabled: bool,
}

impl DependencyRuntime {
    /// Starts a region. Nested computations should instead record a bounded
    /// [`DependencyKey::Query`] in their parent.
    pub fn begin_region(&mut self) {
        assert!(self.active.is_none(), "dependency region already active");
        self.tracking_enabled = true;
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
        if !self.tracking_enabled {
            return ChangedAt::NEVER;
        }
        self.tracker.mark_changed(key)
    }

    /// Registers a key for changed-at tracking without opening a region.
    pub fn track(&mut self, key: DependencyKey) -> ChangedAt {
        self.tracking_enabled = true;
        self.tracker.track(key)
    }

    pub fn invalidate_all(&mut self) {
        if self.tracking_enabled {
            self.tracker.invalidate_all();
        }
    }

    pub(crate) fn snapshot_tracker(&self) -> DependencyTrackerSnapshot {
        DependencyTrackerSnapshot {
            invalidated_at: self.tracker.invalidated_at,
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
        self.changed_at(key)
    }

    #[must_use]
    pub fn changed_at(&self, key: DependencyKey) -> ChangedAt {
        // Scalar code-table reads share one mutation clock per table. This
        // keeps the stamp table bounded while validation still compares the
        // exact scalar value recorded by the reader.
        let key = match key {
            DependencyKey::Code { table, .. } => DependencyKey::CodeGeneration(table),
            key => key,
        };
        self.changed
            .get(&key)
            .copied()
            .unwrap_or(ChangedAt::NEVER)
            .max(self.invalidated_at)
    }

    /// Marks a fact after its aggregate mutation barrier has run.
    pub fn mark_changed(&mut self, key: DependencyKey) -> ChangedAt {
        let key = match key {
            DependencyKey::Code { table, .. } => DependencyKey::CodeGeneration(table),
            key => key,
        };
        self.revision = self
            .revision
            .checked_add(1)
            .expect("dependency revision exhausted");
        let stamp = ChangedAt(self.revision);
        Arc::make_mut(&mut self.changed).insert(key, stamp);
        stamp
    }

    /// Advances every fact observed by this runtime after an aggregate restore.
    pub fn invalidate_all(&mut self) {
        self.revision = self
            .revision
            .checked_add(1)
            .expect("dependency revision exhausted");
        self.invalidated_at = ChangedAt(self.revision);
    }

    #[must_use]
    pub fn observe(&mut self, key: DependencyKey, value: DependencyValue) -> ObservedDependency {
        ObservedDependency {
            key,
            changed_at: self.changed_at(key),
            value,
        }
    }

    fn restore(&mut self, snapshot: &DependencyTrackerSnapshot) {
        if Arc::ptr_eq(&self.changed, &snapshot.changed)
            && self.invalidated_at == snapshot.invalidated_at
        {
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
        let global_changed = self.invalidated_at != snapshot.invalidated_at;
        let mut restored_invalidated_at = snapshot.invalidated_at;
        if global_changed || !changed_keys.is_empty() {
            self.revision = self
                .revision
                .checked_add(1)
                .expect("dependency revision exhausted");
            let stamp = ChangedAt(self.revision);
            if global_changed {
                restored_invalidated_at = stamp;
            }
            for key in changed_keys {
                restored.insert(key, stamp);
            }
        }
        self.invalidated_at = restored_invalidated_at;
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
        read_current: impl FnMut(DependencyKey) -> DependencyValue,
    ) -> bool {
        self.validate_region_failure(observations, read_current)
            .is_none()
    }

    /// Validates a deterministic region and returns its first red key.
    pub fn validate_region_failure(
        &self,
        observations: &mut [ObservedDependency],
        mut read_current: impl FnMut(DependencyKey) -> DependencyValue,
    ) -> Option<DependencyKey> {
        observations.iter_mut().find_map(|observation| {
            (self.validate(observation, &mut read_current) == DependencyValidation::Changed)
                .then_some(observation.key)
        })
    }

    /// Validates a deterministic region without backdating its stamps.
    ///
    /// This is useful for immutable shared memo payloads that are consumed at
    /// most once per execution. Their next accepted incarnation may retain the
    /// old stamps and take the semantic path again without cloning the payload.
    pub fn validate_region_failure_readonly(
        &self,
        observations: &[ObservedDependency],
        mut read_current: impl FnMut(DependencyKey) -> DependencyValue,
    ) -> Option<DependencyKey> {
        observations.iter().find_map(|observation| {
            let stamp_changed = self.changed_at(observation.key) != observation.changed_at;
            (stamp_changed && read_current(observation.key) != observation.value)
                .then_some(observation.key)
        })
    }
}

#[cfg(test)]
mod tests;
