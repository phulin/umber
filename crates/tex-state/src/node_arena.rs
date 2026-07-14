//! Compact epoch storage for immutable node lists.
//!
//! Raw words and sidecars stay private to this module family. Public consumers
//! receive only logical node-list views or the aggregate arena facade.

mod arena;
mod copy;
#[cfg(feature = "profiling-stats")]
mod measurement;
mod mutation;
mod semantic;
mod storage;
mod tables;
mod view;

pub use arena::{NodeArena, NodeListBuilder};
pub(crate) use copy::ChildPatch;
#[cfg(feature = "profiling-stats")]
pub use measurement::{NodeMemoryColumn, NodeStorageObservation, peak_node_storage_measurement};
pub(crate) use semantic::{NodeSemanticId, NodeSemanticIdBuilder};
pub(crate) use storage::{NodeArenaMark, NodeStorage};
pub use view::{CharCodes, CharRun, NodeIter, NodeList, NodeRef};

pub(super) fn checked_len(value: usize, message: &str) -> u32 {
    u32::try_from(value).unwrap_or_else(|_| panic!("{message}"))
}

pub(super) fn preflight_capacity(have: u32, add: u32, message: &str) -> u32 {
    have.checked_add(add).unwrap_or_else(|| panic!("{message}"))
}

#[cfg(test)]
mod tests;
