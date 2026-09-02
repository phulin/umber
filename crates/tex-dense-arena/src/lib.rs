//! Safe physical block stores, pool-stable logical tables, and distinct
//! lifetime-policy wrappers above `tex-dense-prefix`.

#![forbid(unsafe_code)]

use core::fmt;
use core::sync::atomic::{AtomicU32, Ordering};

use tex_dense_prefix::{CapacityError, LayoutError};

mod generation;
mod logical;
mod metrics;
mod nonforking;
mod store;
mod transfer;

pub use generation::{
    GenerationArena, GenerationFork, GenerationForkFailure, GenerationSettlementFailure,
};
pub use logical::{
    AcceptedBlockTable, AcceptedBlockView, AcceptedCandidateTables, CandidateBlockView,
    LogicalBlockId, LogicalCursor, LogicalPosition, WholeBlockBoundary,
};
pub use metrics::{ArenaMetrics, ForkShape};
pub use nonforking::{
    ArenaCursor, AttemptMark, CheckpointJournal, CommittedOutput, CompletedScratch, DenseArena,
    GroupMark, GroupStorage, JournalMark, PageAttemptScratch, SpeculativeOutput,
};
pub use store::BlockStore;
pub use transfer::{
    BlockRangeDetachReceipt, BlockRangeOwner, BlockRangePrepareFailure, BlockRangeRollbackFailure,
    DetachedBlockRange, PreparedBlockRangeTransfer, prepare_block_range_transfer,
};

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests;
#[cfg(all(test, target_arch = "wasm32"))]
mod wasm_tests;

static NEXT_LOGICAL_SPACE: AtomicU32 = AtomicU32::new(1);

fn fresh_space_id() -> u32 {
    NEXT_LOGICAL_SPACE
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            value.checked_add(1)
        })
        .expect("logical block-space id domain exhausted")
}

#[derive(Debug)]
pub enum ArenaError {
    Layout(LayoutError),
    FullBlock,
    MetadataAllocationFailed,
    BlockIdDomainExhausted,
    LogicalOrdinalExhausted,
    IncarnationExhausted,
    LogicalLengthOverflow,
    BoundarySerialExhausted,
    InvalidCursor,
    InvalidBoundary,
    InvalidBlockRange,
    OpenBoundary,
    ForeignLogicalSpace,
    StaleLogicalBlock,
    StalePhysicalBlock,
    UninitializedLogicalOffset,
    OverlappingBlockRange,
    SourceFrontierChanged,
}

impl fmt::Display for ArenaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Layout(error) => error.fmt(formatter),
            Self::FullBlock => formatter.write_str("unexpected full superblock"),
            Self::MetadataAllocationFailed => {
                formatter.write_str("dense arena metadata allocation failed")
            }
            Self::BlockIdDomainExhausted => formatter.write_str("physical block id exhausted"),
            Self::LogicalOrdinalExhausted => formatter.write_str("logical block ordinal exhausted"),
            Self::IncarnationExhausted => formatter.write_str("block incarnation exhausted"),
            Self::LogicalLengthOverflow => formatter.write_str("logical length overflow"),
            Self::BoundarySerialExhausted => formatter.write_str("boundary serial exhausted"),
            Self::InvalidCursor => formatter.write_str("invalid or foreign logical cursor"),
            Self::InvalidBoundary => formatter.write_str("invalid whole-block boundary"),
            Self::InvalidBlockRange => formatter.write_str("invalid whole-block range"),
            Self::OpenBoundary => formatter.write_str("a whole-block boundary is already open"),
            Self::ForeignLogicalSpace => formatter.write_str("foreign logical block space"),
            Self::StaleLogicalBlock => formatter.write_str("stale logical block id"),
            Self::StalePhysicalBlock => formatter.write_str("stale physical block id"),
            Self::UninitializedLogicalOffset => {
                formatter.write_str("logical offset is outside the initialized prefix")
            }
            Self::OverlappingBlockRange => formatter.write_str("whole-block ranges overlap"),
            Self::SourceFrontierChanged => {
                formatter.write_str("source frontier changed after detachment")
            }
        }
    }
}

impl std::error::Error for ArenaError {}

impl From<LayoutError> for ArenaError {
    fn from(error: LayoutError) -> Self {
        Self::Layout(error)
    }
}

impl From<CapacityError> for ArenaError {
    fn from(_: CapacityError) -> Self {
        Self::FullBlock
    }
}

impl From<std::collections::TryReserveError> for ArenaError {
    fn from(_: std::collections::TryReserveError) -> Self {
        Self::MetadataAllocationFailed
    }
}
