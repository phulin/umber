//! Runtime page-material ownership above the generic coarse fork arena.
//!
//! The generic arena remains coordinate-only. This facade is the semantic
//! boundary that pairs one canonical physical list coordinate with the
//! optional demand-maintained identity used by state hashing.

use core::hash::{Hash, Hasher};
use std::ops::Range;

use crate::fork_arena::{
    ArenaListId, ArenaListView, ArenaRange, CheckpointMark, ChunkPool, ForkArena,
    ForkArenaCounters, ForkArenaError, OperationMark, PageMaterialLane, SealedBoundary,
};
use crate::node::Node;
use crate::node_sequence::{SemanticSequenceIdentity, semantic_node_identity};

/// Canonical runtime coordinate plus its demand-maintained semantic scalar.
pub struct PageMaterialListId {
    coordinate: ArenaListId<PageMaterialLane>,
    semantic_identity: u64,
}

impl PageMaterialListId {
    fn from_parts(
        coordinate: ArenaListId<PageMaterialLane>,
        identity: Option<SemanticSequenceIdentity>,
    ) -> Self {
        assert_eq!(
            identity.map(SemanticSequenceIdentity::len),
            identity.map(|_| coordinate.len()),
            "page-list semantic identity length matches its coordinate"
        );
        let semantic_identity = identity.map_or(0, |identity| match identity.raw() {
            0 if !identity.is_empty() => u64::MAX,
            raw => raw,
        });
        Self {
            coordinate,
            semantic_identity,
        }
    }

    #[must_use]
    pub const fn len(self) -> usize {
        self.coordinate.len()
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.coordinate.is_empty()
    }

    #[must_use]
    pub(crate) const fn coordinate(self) -> ArenaListId<PageMaterialLane> {
        self.coordinate
    }

    #[must_use]
    pub const fn semantic_identity(self) -> Option<u64> {
        if self.is_empty() {
            Some(0)
        } else if self.semantic_identity == 0 {
            None
        } else {
            Some(self.semantic_identity)
        }
    }
}

impl Clone for PageMaterialListId {
    fn clone(&self) -> Self {
        *self
    }
}

impl Copy for PageMaterialListId {}

impl core::fmt::Debug for PageMaterialListId {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("PageMaterialListId")
            .field("len", &self.len())
            .finish_non_exhaustive()
    }
}

impl PartialEq for PageMaterialListId {
    fn eq(&self, other: &Self) -> bool {
        self.coordinate == other.coordinate
    }
}

impl Eq for PageMaterialListId {}

impl Hash for PageMaterialListId {
    fn hash<H: Hasher>(&self, state: &mut H) {
        if self.semantic_identity == 0 {
            self.coordinate.hash(state);
        } else {
            self.semantic_identity.hash(state);
        }
    }
}

/// Runtime page payload owner. Every `Node` is appended exactly once.
pub struct PageMaterialArena {
    pool: ChunkPool<Node>,
    arena: ForkArena<Node, PageMaterialLane>,
    range_scratch: Vec<ArenaRange<PageMaterialLane>>,
    coordinate_scratch: Vec<ArenaListId<PageMaterialLane>>,
    semantic_identity_enabled: bool,
    semantic_hash_work: u64,
}

impl Default for PageMaterialArena {
    fn default() -> Self {
        Self::new()
    }
}

impl PageMaterialArena {
    #[must_use]
    pub fn new() -> Self {
        Self::with_chunk_bytes(4 * 1024)
    }

    #[must_use]
    pub fn with_chunk_bytes(chunk_bytes: usize) -> Self {
        Self {
            pool: ChunkPool::with_chunk_bytes(chunk_bytes),
            arena: ForkArena::new(),
            range_scratch: Vec::new(),
            coordinate_scratch: Vec::new(),
            semantic_identity_enabled: false,
            semantic_hash_work: 0,
        }
    }

    pub fn enable_semantic_identity(&mut self) {
        assert!(
            self.arena.counters().new_semantic_nodes == 0 || self.semantic_identity_enabled,
            "semantic identity demand starts before page-node publication"
        );
        self.semantic_identity_enabled = true;
    }

    #[must_use]
    pub const fn semantic_hash_work(&self) -> u64 {
        self.semantic_hash_work
    }

    #[must_use]
    pub const fn counters(&self) -> ForkArenaCounters {
        self.arena.counters()
    }

    pub fn publish_owned(
        &mut self,
        nodes: impl IntoIterator<Item = Node>,
    ) -> Result<PageMaterialListId, ForkArenaError> {
        let mut identity = self
            .semantic_identity_enabled
            .then(SemanticSequenceIdentity::empty);
        let mut builder = self.arena.begin_builder(&mut self.pool)?;
        for node in nodes {
            if let Some(identity) = &mut identity {
                identity.push_back(semantic_node_identity(&node));
                self.semantic_hash_work += 1;
            }
            builder.push(node)?;
        }
        let coordinate = builder.seal()?;
        Ok(PageMaterialListId::from_parts(coordinate, identity))
    }

    pub fn slice_with_identity(
        &mut self,
        list: PageMaterialListId,
        selected: Range<usize>,
        identity: Option<SemanticSequenceIdentity>,
    ) -> Result<PageMaterialListId, ForkArenaError> {
        assert_eq!(self.semantic_identity_enabled, identity.is_some());
        let coordinate = self.arena.slice_list(
            &mut self.pool,
            list.coordinate(),
            selected,
            &mut self.range_scratch,
        )?;
        Ok(PageMaterialListId::from_parts(coordinate, identity))
    }

    pub fn compose_with_identity(
        &mut self,
        lists: &[PageMaterialListId],
        identity: Option<SemanticSequenceIdentity>,
    ) -> Result<PageMaterialListId, ForkArenaError> {
        assert_eq!(self.semantic_identity_enabled, identity.is_some());
        self.coordinate_scratch.clear();
        self.coordinate_scratch
            .extend(lists.iter().map(|list| list.coordinate()));
        let coordinate = self.arena.compose_lists(
            &mut self.pool,
            &self.coordinate_scratch,
            &mut self.range_scratch,
        )?;
        Ok(PageMaterialListId::from_parts(coordinate, identity))
    }

    pub fn list(
        &self,
        list: PageMaterialListId,
    ) -> Result<ArenaListView<'_, Node, PageMaterialLane>, ForkArenaError> {
        self.arena.list(&self.pool, list.coordinate())
    }

    #[must_use]
    pub fn operation_mark(&self) -> OperationMark<PageMaterialLane> {
        self.arena.operation_mark(&self.pool)
    }

    pub fn restore_operation(
        &mut self,
        mark: OperationMark<PageMaterialLane>,
    ) -> Result<(), ForkArenaError> {
        self.arena.restore_operation(&mut self.pool, mark)
    }

    pub fn seal_boundary(&mut self) -> Result<SealedBoundary<PageMaterialLane>, ForkArenaError> {
        self.arena.seal_boundary(&mut self.pool)
    }

    pub fn checkpoint_mark(
        &self,
        boundary: SealedBoundary<PageMaterialLane>,
    ) -> Result<CheckpointMark<PageMaterialLane>, ForkArenaError> {
        self.arena.checkpoint_mark(boundary)
    }

    pub fn begin_checkpoint_candidate(
        &mut self,
        mark: CheckpointMark<PageMaterialLane>,
    ) -> Result<(), ForkArenaError> {
        self.arena.begin_checkpoint_candidate(mark)
    }

    pub fn reject_checkpoint_candidate(
        &mut self,
        boundary: SealedBoundary<PageMaterialLane>,
    ) -> Result<(), ForkArenaError> {
        self.arena
            .reject_checkpoint_candidate(&mut self.pool, boundary)
    }

    pub fn accept_checkpoint_candidate(
        &mut self,
        boundary: SealedBoundary<PageMaterialLane>,
    ) -> Result<(), ForkArenaError> {
        self.arena
            .accept_checkpoint_candidate(&mut self.pool, boundary)
    }
}

#[cfg(test)]
#[path = "page_node_arena/tests.rs"]
mod tests;
