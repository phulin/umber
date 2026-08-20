//! Copy-only stacks with inline storage and constant-size rollback marks.

use core::fmt;
use core::mem::size_of;
use smallvec::SmallVec;

const INLINE_ENTRIES: usize = 8;

/// A rejected compact-stack operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PodStackError {
    LengthCapacityExhausted,
    AllocationFailed,
    InvalidMark,
}

impl fmt::Display for PodStackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::LengthCapacityExhausted => "compact-stack length space is exhausted",
            Self::AllocationFailed => "compact-stack storage allocation failed",
            Self::InvalidMark => "compact-stack mark is not an ancestor of this stack",
        })
    }
}

impl std::error::Error for PodStackError {}

/// A constant-size watermark for one [`PodStack`].
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct PodStackMark(u32);

impl PodStackMark {
    pub(crate) const fn len(self) -> u32 {
        self.0
    }
}

/// Logical and retained storage owned by one compact stack.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct PodStackAccounting {
    pub(crate) logical_entries: usize,
    pub(crate) logical_bytes: usize,
    pub(crate) inline_capacity: usize,
    pub(crate) retained_heap_entries: usize,
    pub(crate) retained_heap_bytes: usize,
}

/// An append-only stack for copy-only hot-core records.
///
/// The first eight entries live in the stack value itself. Once a workload
/// spills, truncation retains the heap buffer for later bounded reuse.
pub(crate) struct PodStack<T: Copy> {
    entries: SmallVec<[T; INLINE_ENTRIES]>,
}

impl<T: Copy> Default for PodStack<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Copy> PodStack<T> {
    pub(crate) fn new() -> Self {
        Self {
            entries: SmallVec::new(),
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(crate) fn last(&self) -> Option<&T> {
        self.entries.last()
    }

    pub(crate) fn get(&self, index: usize) -> Option<&T> {
        self.entries.get(index)
    }

    pub(crate) fn mark(&self) -> Result<PodStackMark, PodStackError> {
        u32::try_from(self.entries.len())
            .map(PodStackMark)
            .map_err(|_| PodStackError::LengthCapacityExhausted)
    }

    pub(crate) fn push(&mut self, value: T) -> Result<(), PodStackError> {
        if self.entries.len() == u32::MAX as usize {
            return Err(PodStackError::LengthCapacityExhausted);
        }
        self.entries
            .try_reserve(1)
            .map_err(|_| PodStackError::AllocationFailed)?;
        self.entries.push(value);
        Ok(())
    }

    pub(crate) fn pop(&mut self) -> Option<T> {
        self.entries.pop()
    }

    pub(crate) fn truncate(&mut self, mark: PodStackMark) -> Result<(), PodStackError> {
        self.validate_mark(mark)?;
        self.entries.truncate(mark.0 as usize);
        Ok(())
    }

    /// Validates a stack watermark without changing the stack.
    pub(crate) fn validate_mark(&self, mark: PodStackMark) -> Result<(), PodStackError> {
        if mark.0 as usize > self.entries.len() {
            return Err(PodStackError::InvalidMark);
        }
        Ok(())
    }

    pub(crate) fn accounting(&self) -> PodStackAccounting {
        let retained_heap_entries = if self.entries.spilled() {
            self.entries.capacity()
        } else {
            0
        };
        PodStackAccounting {
            logical_entries: self.entries.len(),
            logical_bytes: self.entries.len().saturating_mul(size_of::<T>()),
            inline_capacity: INLINE_ENTRIES,
            retained_heap_entries,
            retained_heap_bytes: retained_heap_entries.saturating_mul(size_of::<T>()),
        }
    }
}
