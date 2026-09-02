#![allow(
    clippy::result_large_err,
    reason = "transaction failures must return every move-only owner and the exact unchanged loan"
)]

use core::marker::PhantomData;

use crate::{ArenaError, ArenaMetrics, LogicalBlockId};

type DetachResult<T> = Result<
    (
        BlockRangeOwner<T>,
        DetachedBlockRange<T>,
        BlockRangeDetachReceipt<T>,
    ),
    (ArenaError, BlockRangeOwner<T>),
>;

/// Semantic-neutral exclusive ownership metadata for whole logical blocks.
///
/// The logical-to-physical table remains untouched when this owner is moved.
pub struct BlockRangeOwner<T> {
    space: u32,
    blocks: Vec<LogicalBlockId>,
    frontier: u64,
    next_detach_serial: u64,
    metrics: ArenaMetrics,
    _payload: PhantomData<fn() -> T>,
}

impl<T> BlockRangeOwner<T> {
    pub(crate) fn new(space: u32, blocks: Vec<LogicalBlockId>) -> Self {
        Self {
            space,
            blocks,
            frontier: 0,
            next_detach_serial: 1,
            metrics: ArenaMetrics::default(),
            _payload: PhantomData,
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    #[must_use]
    pub const fn frontier(&self) -> u64 {
        self.frontier
    }

    #[must_use]
    pub const fn metrics(&self) -> ArenaMetrics {
        self.metrics
    }

    #[must_use]
    pub fn logical_blocks(&self) -> &[LogicalBlockId] {
        &self.blocks
    }

    /// Detaches the whole-block suffix beginning at `at`.
    ///
    /// Metadata allocation is completed before the source owner changes.
    pub fn detach_suffix(mut self, at: usize) -> DetachResult<T> {
        if at > self.blocks.len() {
            return Err((ArenaError::InvalidBlockRange, self));
        }
        let detached_len = self.blocks.len() - at;
        let serial = self.next_detach_serial;
        let Some(next_serial) = serial.checked_add(1) else {
            return Err((ArenaError::BoundarySerialExhausted, self));
        };
        let Some(next_frontier) = self.frontier.checked_add(1) else {
            return Err((ArenaError::BoundarySerialExhausted, self));
        };
        if next_frontier.checked_add(1).is_none() {
            return Err((ArenaError::BoundarySerialExhausted, self));
        }
        let mut detached = Vec::new();
        if let Err(error) = detached.try_reserve_exact(detached_len) {
            return Err((ArenaError::from(error), self));
        }
        detached.extend_from_slice(&self.blocks[at..]);
        self.blocks.truncate(at);
        self.next_detach_serial = next_serial;
        self.frontier = next_frontier;
        self.metrics.block_ranges_detached += 1;
        let loan = DetachedBlockRange {
            space: self.space,
            detach_serial: serial,
            blocks: detached,
            _payload: PhantomData,
        };
        let receipt = BlockRangeDetachReceipt {
            space: self.space,
            detach_serial: serial,
            expected_frontier: next_frontier,
            insertion_frontier: at,
            detached_len,
            _payload: PhantomData,
        };
        Ok((self, loan, receipt))
    }
}

/// Move-only unchanged loan returned by whole-block detachment.
pub struct DetachedBlockRange<T> {
    space: u32,
    detach_serial: u64,
    blocks: Vec<LogicalBlockId>,
    _payload: PhantomData<fn() -> T>,
}

impl<T> DetachedBlockRange<T> {
    #[must_use]
    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    #[must_use]
    pub fn logical_blocks(&self) -> &[LogicalBlockId] {
        &self.blocks
    }
}

/// Exact source insertion-frontier authority for a detached suffix.
pub struct BlockRangeDetachReceipt<T> {
    space: u32,
    detach_serial: u64,
    expected_frontier: u64,
    insertion_frontier: usize,
    detached_len: usize,
    _payload: PhantomData<fn() -> T>,
}

impl<T> BlockRangeDetachReceipt<T> {
    /// Restores an unchanged loan only when the source is still at the exact
    /// post-detach frontier. A mismatch returns every move-only authority.
    pub fn rollback(
        self,
        mut source: BlockRangeOwner<T>,
        mut loan: DetachedBlockRange<T>,
    ) -> Result<BlockRangeOwner<T>, BlockRangeRollbackFailure<T>> {
        let valid = source.space == self.space
            && loan.space == self.space
            && loan.detach_serial == self.detach_serial
            && source.frontier == self.expected_frontier
            && source.blocks.len() == self.insertion_frontier
            && loan.blocks.len() == self.detached_len
            && source.blocks.capacity() - source.blocks.len() >= loan.blocks.len();
        if !valid {
            return Err(BlockRangeRollbackFailure {
                error: ArenaError::SourceFrontierChanged,
                source,
                loan,
                receipt: self,
            });
        }
        source.blocks.append(&mut loan.blocks);
        source.frontier += 1;
        source.metrics.block_ranges_rolled_back += 1;
        Ok(source)
    }
}

pub struct BlockRangeRollbackFailure<T> {
    error: ArenaError,
    source: BlockRangeOwner<T>,
    loan: DetachedBlockRange<T>,
    receipt: BlockRangeDetachReceipt<T>,
}

impl<T> BlockRangeRollbackFailure<T> {
    #[must_use]
    pub const fn error(&self) -> &ArenaError {
        &self.error
    }

    #[must_use]
    pub fn into_parts(
        self,
    ) -> (
        ArenaError,
        BlockRangeOwner<T>,
        DetachedBlockRange<T>,
        BlockRangeDetachReceipt<T>,
    ) {
        (self.error, self.source, self.loan, self.receipt)
    }
}

pub struct BlockRangePrepareFailure<T> {
    error: ArenaError,
    destination: BlockRangeOwner<T>,
    loan: DetachedBlockRange<T>,
}

impl<T> BlockRangePrepareFailure<T> {
    #[must_use]
    pub const fn error(&self) -> &ArenaError {
        &self.error
    }

    #[must_use]
    pub fn into_parts(self) -> (ArenaError, BlockRangeOwner<T>, DetachedBlockRange<T>) {
        (self.error, self.destination, self.loan)
    }
}

/// Prepared metadata-only transfer. `commit` cannot allocate or fail.
pub struct PreparedBlockRangeTransfer<T> {
    destination: BlockRangeOwner<T>,
    loan: DetachedBlockRange<T>,
}

impl<T> PreparedBlockRangeTransfer<T> {
    #[must_use]
    pub fn commit(mut self) -> BlockRangeOwner<T> {
        self.destination.blocks.append(&mut self.loan.blocks);
        self.destination.frontier += 1;
        self.destination.metrics.block_ranges_transferred += 1;
        self.destination
    }

    #[must_use]
    pub fn cancel(self) -> (BlockRangeOwner<T>, DetachedBlockRange<T>) {
        (self.destination, self.loan)
    }
}

/// Validates and reserves all destination metadata before returning an
/// infallible transfer authority. Failure returns the exact loan unchanged.
pub fn prepare_block_range_transfer<T>(
    mut destination: BlockRangeOwner<T>,
    loan: DetachedBlockRange<T>,
) -> Result<PreparedBlockRangeTransfer<T>, BlockRangePrepareFailure<T>> {
    if destination.space != loan.space {
        return Err(BlockRangePrepareFailure {
            error: ArenaError::ForeignLogicalSpace,
            destination,
            loan,
        });
    }
    if loan
        .blocks
        .iter()
        .any(|block| destination.blocks.contains(block))
    {
        return Err(BlockRangePrepareFailure {
            error: ArenaError::OverlappingBlockRange,
            destination,
            loan,
        });
    }
    if destination.frontier.checked_add(1).is_none() {
        return Err(BlockRangePrepareFailure {
            error: ArenaError::BoundarySerialExhausted,
            destination,
            loan,
        });
    }
    if let Err(error) = destination.blocks.try_reserve(loan.blocks.len()) {
        return Err(BlockRangePrepareFailure {
            error: ArenaError::from(error),
            destination,
            loan,
        });
    }
    destination.metrics.block_ranges_prepared += 1;
    Ok(PreparedBlockRangeTransfer { destination, loan })
}
