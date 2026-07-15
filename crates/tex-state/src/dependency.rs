//! Region-scoped semantic dependency tracking for incremental computations.
//!
//! Keys name observable facts rather than storage locations.  Observations are
//! detached from a [`crate::Universe`]: live ids may be used to locate a fact,
//! but the recorded value is always a scalar or canonical content identity.

use crate::world::ContentHash;
use std::collections::BTreeMap;

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
    changed: BTreeMap<DependencyKey, ChangedAt>,
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

    #[must_use]
    pub const fn tracker(&self) -> &DependencyTracker {
        &self.tracker
    }
}

impl DependencyTracker {
    #[must_use]
    pub fn changed_at(&self, key: DependencyKey) -> ChangedAt {
        self.changed.get(&key).copied().unwrap_or(ChangedAt::NEVER)
    }

    /// Marks a fact after its aggregate mutation barrier has run.
    pub fn mark_changed(&mut self, key: DependencyKey) -> ChangedAt {
        self.revision = self
            .revision
            .checked_add(1)
            .expect("dependency revision exhausted");
        let stamp = ChangedAt(self.revision);
        self.changed.insert(key, stamp);
        stamp
    }

    #[must_use]
    pub fn observe(&self, key: DependencyKey, value: DependencyValue) -> ObservedDependency {
        ObservedDependency {
            key,
            changed_at: self.changed_at(key),
            value,
        }
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
