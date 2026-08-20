//! Structurally owned compact storage for immutable node lists.
//!
//! Raw words and sidecars stay private to this module family. Public consumers
//! receive only strong list owners and borrow-scoped logical views.

mod builder;
#[cfg(all(test, feature = "profiling"))]
mod copy;
#[cfg(all(test, feature = "profiling"))]
mod mutation;
mod schema;
mod semantic;
mod storage;
mod tables;
mod view;

pub(crate) use builder::CompactBuilderNode;
pub use builder::NodeListBuilder;
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
