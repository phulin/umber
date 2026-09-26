//! Profiling-only attribution of built page roots that need a structural copy.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::page_node_arena::BuiltBoxOrigin;

const ORIGINS: usize = 4;
const SHAPES: usize = 8;
const CELLS: usize = ORIGINS * SHAPES;

static ENABLED: AtomicBool = AtomicBool::new(false);
static EVENTS: [AtomicU64; CELLS] = [const { AtomicU64::new(0) }; CELLS];
static COPIED_NODES: [AtomicU64; CELLS] = [const { AtomicU64::new(0) }; CELLS];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BoxFallbackShape {
    Inline,
    Glue,
    OtherAnnex,
    Math,
    Migration,
    Disc,
    NestedBox,
    Leader,
}

impl BoxFallbackShape {
    const fn index(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BoxFallbackCensus {
    pub events: [u64; CELLS],
    pub copied_nodes: [u64; CELLS],
}

impl BoxFallbackCensus {
    pub const ORIGIN_NAMES: [&'static str; ORIGINS] = ["setbox", "lastbox", "pdf_form", "other"];
    pub const SHAPE_NAMES: [&'static str; SHAPES] = [
        "inline",
        "glue",
        "other_annex",
        "math",
        "migration",
        "disc",
        "nested_box",
        "leader",
    ];

    #[must_use]
    pub fn saturating_sub(self, before: Self) -> Self {
        let mut result = Self::default();
        for index in 0..CELLS {
            result.events[index] = self.events[index].saturating_sub(before.events[index]);
            result.copied_nodes[index] =
                self.copied_nodes[index].saturating_sub(before.copied_nodes[index]);
        }
        result
    }
}

pub fn enable_box_fallback_census() {
    ENABLED.store(true, Ordering::Relaxed);
}

pub(crate) fn box_fallback_census_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

pub(crate) fn record_box_fallback(
    origin: BuiltBoxOrigin,
    shape: BoxFallbackShape,
    copied_nodes: u64,
) {
    let index = (origin as usize) * SHAPES + shape.index();
    EVENTS[index].fetch_add(1, Ordering::Relaxed);
    COPIED_NODES[index].fetch_add(copied_nodes, Ordering::Relaxed);
}

#[must_use]
pub fn box_fallback_census() -> BoxFallbackCensus {
    let mut census = BoxFallbackCensus::default();
    for index in 0..CELLS {
        census.events[index] = EVENTS[index].load(Ordering::Relaxed);
        census.copied_nodes[index] = COPIED_NODES[index].load(Ordering::Relaxed);
    }
    census
}
