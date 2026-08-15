//! Compact epoch storage for immutable node lists.
//!
//! Raw words and sidecars stay private to this module family. Public consumers
//! receive only logical node-list views or the aggregate arena facade.

mod builder;
mod copy;
mod measurement;
mod mutation;
mod owned;
mod schema;
mod semantic;
mod storage;
mod tables;
mod view;

pub use builder::NodeListBuilder;
pub(crate) use copy::ChildPatch;
#[cfg(feature = "profiling")]
pub use measurement::{NodeMemoryColumn, NodeStorageObservation, peak_node_storage_measurement};
pub use owned::NodeListRef;
pub(crate) use owned::{
    NodeListPayload, NodeListWeakIndex, OwnedSemanticSpan, allocate_node_payload_root,
};
pub use schema::{
    FieldPolicy, NodeChildRole, NodeDescriptor, NodeField, NodeHandle, NodeHandleEvent,
    NodeHandleKind, NodeHandlePolicy, NodeHandleRole, NodeSchemaVisitor, NodeTag,
};
pub(crate) use semantic::{NodeSemanticId, NodeSemanticIdBuilder};
pub(crate) use storage::{NodeStorage, SidecarNeeds};
pub use view::{CharCodes, CharRun, NodeCursor, NodeIter, NodeList, NodeRef, PackedNode};

pub(super) fn checked_len(value: usize, message: &str) -> u32 {
    u32::try_from(value).unwrap_or_else(|_| panic!("{message}"))
}

pub(super) fn preflight_capacity(have: u32, add: u32, message: &str) -> u32 {
    have.checked_add(add).unwrap_or_else(|| panic!("{message}"))
}

#[cfg(test)]
mod tests;
