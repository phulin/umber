//! Reusable node storage visible only while one shipout attempt is active.
//!
//! Rows retain page-list children because normalization may derive a small
//! replacement from a checkpointable page without copying that page graph.
//! The row coordinate itself has a distinct type, so it cannot enter page
//! state, a mode summary, or an engine checkpoint.

use core::marker::PhantomData;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::glue::GlueSpec;
use crate::node::{Node, NodeTokenList};
use crate::node_arena::{DurableListId, PageListId};

#[cfg(test)]
mod tests;

static NEXT_SHIPOUT_SCRATCH_OWNER: AtomicU64 = AtomicU64::new(1);
static NEXT_SHIPOUT_SCRATCH_SERIAL: AtomicU64 = AtomicU64::new(1);

/// Copy-only coordinate into the current generation's shipout scratch lane.
///
/// Constructors are private. Semantic page carriers accept `PageListId`
/// instead, making a scratch-coordinate escape a Rust type error.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ShipoutScratchListId {
    owner: u64,
    row: u32,
    serial: u64,
}

/// Borrow-only coordinate used by one active shipout traversal.
///
/// No semantic page, mode, journal, format, memo, or checkpoint carrier
/// accepts this type.
pub enum ShipoutListId<G> {
    Page(PageListId),
    Durable(DurableListId<G>),
    Scratch(ShipoutScratchListId),
}

impl<G> Clone for ShipoutListId<G> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<G> Copy for ShipoutListId<G> {}

impl<G> core::fmt::Debug for ShipoutListId<G> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Page(_) => formatter.write_str("ShipoutListId::Page(..)"),
            Self::Durable(_) => formatter.write_str("ShipoutListId::Durable(..)"),
            Self::Scratch(_) => formatter.write_str("ShipoutListId::Scratch(..)"),
        }
    }
}

impl<G> PartialEq for ShipoutListId<G> {
    fn eq(&self, other: &Self) -> bool {
        match (*self, *other) {
            (Self::Page(left), Self::Page(right)) => left == right,
            (Self::Durable(left), Self::Durable(right)) => left == right,
            (Self::Scratch(left), Self::Scratch(right)) => left == right,
            _ => false,
        }
    }
}

impl<G> Eq for ShipoutListId<G> {}

/// Node representation stored only in the reusable shipout lane.
pub type ShipoutScratchNode<G> = Node<ShipoutListId<G>, GlueSpec, NodeTokenList>;

/// Token payload selected from one immutable shipout source node.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShipoutTokenField {
    DeferredWrite,
    DeferredSpecial,
    DeferredPdfLiteral,
    PdfDestinationIdentifier,
    PdfThreadIdentifier,
    PdfThreadAttributes,
}

/// Borrow-only coordinate for a shipout source node whose non-token payload
/// must be interpreted in place.
pub struct ShipoutNodeSource<G> {
    pub(crate) list: ShipoutListId<G>,
    pub(crate) index: usize,
}

impl<G> Clone for ShipoutNodeSource<G> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<G> Copy for ShipoutNodeSource<G> {}

impl<G> core::fmt::Debug for ShipoutNodeSource<G> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("ShipoutNodeSource(..)")
    }
}

impl<G> ShipoutNodeSource<G> {
    #[must_use]
    pub fn new(list: ShipoutListId<G>, index: usize) -> Self {
        Self { list, index }
    }
}

/// Generation-branded token input retained across deferred replay.
pub struct ShipoutTokenSource<G> {
    pub(crate) list: ShipoutListId<G>,
    pub(crate) index: usize,
    pub(crate) field: ShipoutTokenField,
}

impl<G> Clone for ShipoutTokenSource<G> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<G> Copy for ShipoutTokenSource<G> {}

impl<G> core::fmt::Debug for ShipoutTokenSource<G> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("ShipoutTokenSource(..)")
    }
}

impl<G> ShipoutTokenSource<G> {
    #[must_use]
    pub fn new(list: ShipoutListId<G>, index: usize, field: ShipoutTokenField) -> Self {
        Self { list, index, field }
    }
}

/// Nested scratch suffix opened by one aggregate shipout transaction.
#[derive(Clone, Copy)]
pub(crate) struct ShipoutScratchMark {
    owner: u64,
    active_rows: u32,
}

struct ShipoutScratchRow<G> {
    serial: u64,
    nodes: Vec<ShipoutScratchNode<G>>,
}

/// One generation-owned lane whose physical rows remain at warmed high water.
pub(crate) struct ShipoutScratchArena<G> {
    owner: u64,
    active_rows: usize,
    rows: Vec<ShipoutScratchRow<G>>,
    _generation: PhantomData<fn(&G) -> &G>,
}

impl<G> Default for ShipoutScratchArena<G> {
    fn default() -> Self {
        let owner = NEXT_SHIPOUT_SCRATCH_OWNER.fetch_add(1, Ordering::Relaxed);
        assert_ne!(owner, 0, "shipout scratch owner identity exhausted");
        Self {
            owner,
            active_rows: 0,
            rows: Vec::new(),
            _generation: PhantomData,
        }
    }
}

impl<G> ShipoutScratchArena<G> {
    pub(crate) fn mark(&self) -> ShipoutScratchMark {
        ShipoutScratchMark {
            owner: self.owner,
            active_rows: u32::try_from(self.active_rows).expect("shipout scratch exceeds u32 rows"),
        }
    }

    pub(crate) fn reset(&mut self, mark: ShipoutScratchMark) {
        assert_eq!(mark.owner, self.owner, "foreign shipout scratch mark");
        let active_rows = mark.active_rows as usize;
        assert!(
            active_rows <= self.active_rows,
            "shipout scratch mark is beyond the active suffix"
        );
        self.active_rows = active_rows;
    }

    /// Opens one empty final row. Nested rows may be opened before this one is
    /// filled because each stable row owns its warmed node buffer.
    pub(crate) fn begin_list(&mut self) -> ShipoutScratchListId {
        let index = self.active_rows;
        if index == self.rows.len() {
            self.rows.push(ShipoutScratchRow {
                serial: 0,
                nodes: Vec::new(),
            });
        }
        let row = &mut self.rows[index];
        row.serial = NEXT_SHIPOUT_SCRATCH_SERIAL.fetch_add(1, Ordering::Relaxed);
        assert_ne!(row.serial, 0, "shipout scratch serial exhausted");
        row.nodes.clear();
        self.active_rows += 1;
        ShipoutScratchListId {
            owner: self.owner,
            row: u32::try_from(index + 1).expect("shipout scratch exceeds u32 rows"),
            serial: row.serial,
        }
    }

    pub(crate) fn get(&self, id: ShipoutScratchListId) -> Option<&[ShipoutScratchNode<G>]> {
        let index = id.row.checked_sub(1)? as usize;
        (id.owner == self.owner && index < self.active_rows)
            .then(|| self.rows.get(index))
            .flatten()
            .filter(|row| row.serial == id.serial)
            .map(|row| row.nodes.as_slice())
    }

    /// Appends one node directly to its final scratch row.
    pub(crate) fn push(&mut self, id: ShipoutScratchListId, node: ShipoutScratchNode<G>) {
        let index = id
            .row
            .checked_sub(1)
            .expect("shipout scratch empty coordinate") as usize;
        assert!(index < self.active_rows, "shipout scratch row is inactive");
        let row = self
            .rows
            .get_mut(index)
            .filter(|row| id.owner == self.owner && row.serial == id.serial)
            .expect("shipout scratch builder belongs to the active transaction");
        row.nodes.push(node);
    }

    #[cfg(test)]
    pub(crate) fn high_water(&self) -> (usize, Vec<usize>) {
        (
            self.rows.len(),
            self.rows.iter().map(|row| row.nodes.capacity()).collect(),
        )
    }
}
