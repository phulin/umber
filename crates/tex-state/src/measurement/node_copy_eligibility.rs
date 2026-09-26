//! Observational source classes for complete-owner copy experiments.
//!
//! This census never grants copy authority. The marker it observes records
//! only that a source owner was born from an empty destination by the existing
//! successful recursive copier and has not been lent for topology mutation.

use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy, Debug)]
pub(crate) enum DurableSourceCopyKind {
    ExplicitToPage,
    HistoryToPage,
    DurableOwner,
}

impl DurableSourceCopyKind {
    const fn index(self) -> usize {
        match self {
            Self::ExplicitToPage => 0,
            Self::HistoryToPage => 1,
            Self::DurableOwner => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NodeCopyEligibilityLane {
    pub calls: u64,
    pub nodes: u64,
    pub marked_calls: u64,
    pub marked_nodes: u64,
}

impl NodeCopyEligibilityLane {
    fn saturating_sub(self, baseline: Self) -> Self {
        Self {
            calls: self.calls.saturating_sub(baseline.calls),
            nodes: self.nodes.saturating_sub(baseline.nodes),
            marked_calls: self.marked_calls.saturating_sub(baseline.marked_calls),
            marked_nodes: self.marked_nodes.saturating_sub(baseline.marked_nodes),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NodeCopyEligibilityCensus {
    /// Successful top-level recursive region copies of every source class.
    pub all_region_calls: u64,
    /// Recursive node volume charged once per successful top-level copy.
    pub all_region_nodes: u64,
    pub explicit_to_page: NodeCopyEligibilityLane,
    pub history_to_page: NodeCopyEligibilityLane,
    pub durable_owner: NodeCopyEligibilityLane,
    /// Successful materializations at the destructive `\vsplit` source read.
    /// These calls and nodes are also included in `explicit_to_page`.
    pub vsplit_source_calls: u64,
    pub vsplit_source_nodes: u64,
}

impl NodeCopyEligibilityCensus {
    #[must_use]
    pub fn saturating_sub(self, baseline: Self) -> Self {
        Self {
            all_region_calls: self
                .all_region_calls
                .saturating_sub(baseline.all_region_calls),
            all_region_nodes: self
                .all_region_nodes
                .saturating_sub(baseline.all_region_nodes),
            explicit_to_page: self
                .explicit_to_page
                .saturating_sub(baseline.explicit_to_page),
            history_to_page: self
                .history_to_page
                .saturating_sub(baseline.history_to_page),
            durable_owner: self.durable_owner.saturating_sub(baseline.durable_owner),
            vsplit_source_calls: self
                .vsplit_source_calls
                .saturating_sub(baseline.vsplit_source_calls),
            vsplit_source_nodes: self
                .vsplit_source_nodes
                .saturating_sub(baseline.vsplit_source_nodes),
        }
    }
}

static CALLS: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];
static NODES: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];
static MARKED_CALLS: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];
static MARKED_NODES: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];
static ALL_REGION_CALLS: AtomicU64 = AtomicU64::new(0);
static ALL_REGION_NODES: AtomicU64 = AtomicU64::new(0);
static VSPLIT_SOURCE_CALLS: AtomicU64 = AtomicU64::new(0);
static VSPLIT_SOURCE_NODES: AtomicU64 = AtomicU64::new(0);

/// Records one completed region-copy entrypoint. Recursive child visits are
/// already included in `copied_nodes` and never call this function separately.
pub(crate) fn record_region_copy(copied_nodes: usize) {
    ALL_REGION_CALLS.fetch_add(1, Ordering::Relaxed);
    ALL_REGION_NODES.fetch_add(copied_nodes as u64, Ordering::Relaxed);
}

/// Charges the copy performed before `\vsplit` examines or mutates its
/// source. Later TeX error or recovery does not erase that completed work.
pub fn record_vsplit_source_copy(copied_nodes: u64) {
    VSPLIT_SOURCE_CALLS.fetch_add(1, Ordering::Relaxed);
    VSPLIT_SOURCE_NODES.fetch_add(copied_nodes, Ordering::Relaxed);
}

pub(crate) fn record_durable_source_copy(
    kind: DurableSourceCopyKind,
    marked: bool,
    copied_nodes: u64,
) {
    let index = kind.index();
    CALLS[index].fetch_add(1, Ordering::Relaxed);
    NODES[index].fetch_add(copied_nodes, Ordering::Relaxed);
    if marked {
        MARKED_CALLS[index].fetch_add(1, Ordering::Relaxed);
        MARKED_NODES[index].fetch_add(copied_nodes, Ordering::Relaxed);
    }
}

#[must_use]
pub fn node_copy_eligibility_census() -> NodeCopyEligibilityCensus {
    fn lane(index: usize) -> NodeCopyEligibilityLane {
        NodeCopyEligibilityLane {
            calls: CALLS[index].load(Ordering::Relaxed),
            nodes: NODES[index].load(Ordering::Relaxed),
            marked_calls: MARKED_CALLS[index].load(Ordering::Relaxed),
            marked_nodes: MARKED_NODES[index].load(Ordering::Relaxed),
        }
    }
    NodeCopyEligibilityCensus {
        all_region_calls: ALL_REGION_CALLS.load(Ordering::Relaxed),
        all_region_nodes: ALL_REGION_NODES.load(Ordering::Relaxed),
        explicit_to_page: lane(0),
        history_to_page: lane(1),
        durable_owner: lane(2),
        vsplit_source_calls: VSPLIT_SOURCE_CALLS.load(Ordering::Relaxed),
        vsplit_source_nodes: VSPLIT_SOURCE_NODES.load(Ordering::Relaxed),
    }
}
