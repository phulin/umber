//! First-write inverse journals for generation-checked mutable targets.

use core::fmt;
use core::mem::size_of;
use core::num::NonZeroU32;
use smallvec::SmallVec;

const INLINE_INVERSES: usize = 32;
const INLINE_MARKS: usize = 8;

/// The private mutation surface required by [`FirstWriteJournal`].
///
/// A target owns its values and per-cell write epochs. The journal owns only
/// copyable inverse records and nested rollback frames.
pub(crate) trait JournalTarget {
    type Coordinate: Copy + Eq;
    type Value: Copy;
    type Owner: Copy + Eq;
    type Error;

    fn journal_owner(&self) -> Self::Owner;

    fn read_for_journal(
        &self,
        coordinate: Self::Coordinate,
    ) -> Result<(Self::Value, u32), Self::Error>;

    fn validate_for_journal(&self, coordinate: Self::Coordinate) -> Result<(), Self::Error>;

    fn write_validated(
        &mut self,
        coordinate: Self::Coordinate,
        value: Self::Value,
        write_epoch: u32,
    );
}

/// A rejected first-write journal operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FirstWriteJournalError<E> {
    AllocationFailed,
    EpochExhausted,
    NoActiveMark,
    InvalidMark,
    ForeignTarget,
    Target(E),
}

impl<E: fmt::Display> fmt::Display for FirstWriteJournalError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AllocationFailed => f.write_str("first-write journal allocation failed"),
            Self::EpochExhausted => f.write_str("first-write journal epoch space is exhausted"),
            Self::NoActiveMark => f.write_str("first-write journal has no active mark"),
            Self::InvalidMark => f.write_str("first-write journal mark is not the active mark"),
            Self::ForeignTarget => f.write_str("first-write journal belongs to another target"),
            Self::Target(error) => write!(f, "journal target rejected an operation: {error}"),
        }
    }
}

impl<E> std::error::Error for FirstWriteJournalError<E> where E: std::error::Error + 'static {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct InverseRecord<C, V> {
    coordinate: C,
    old_value: V,
    previous_epoch: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct JournalFrame {
    epoch: NonZeroU32,
    inverse_cursor: u32,
}

/// A fixed-size typed cursor into one journal's active nested mark stack.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FirstWriteMark<O> {
    owner: O,
    epoch: NonZeroU32,
    inverse_cursor: u32,
    depth: u32,
}

impl<O: Copy> FirstWriteMark<O> {
    pub(crate) const fn inverse_cursor(self) -> u32 {
        self.inverse_cursor
    }

    pub(crate) const fn depth(self) -> u32 {
        self.depth
    }
}

/// Logical and retained storage owned by a first-write journal.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct FirstWriteJournalAccounting {
    pub(crate) logical_inverses: usize,
    pub(crate) logical_inverse_bytes: usize,
    pub(crate) active_marks: usize,
    pub(crate) retained_inverse_heap_entries: usize,
    pub(crate) retained_mark_heap_entries: usize,
    pub(crate) retained_heap_bytes: usize,
}

/// Nested first-write inverse history for one generation-checked target.
pub(crate) struct FirstWriteJournal<T: JournalTarget> {
    owner: T::Owner,
    inverses: SmallVec<[InverseRecord<T::Coordinate, T::Value>; INLINE_INVERSES]>,
    marks: SmallVec<[JournalFrame; INLINE_MARKS]>,
    next_epoch: NonZeroU32,
}

impl<T: JournalTarget> FirstWriteJournal<T> {
    pub(crate) fn new(target: &T) -> Self {
        Self {
            owner: target.journal_owner(),
            inverses: SmallVec::new(),
            marks: SmallVec::new(),
            next_epoch: NonZeroU32::MIN,
        }
    }

    pub(crate) fn mark(
        &mut self,
        target: &T,
    ) -> Result<FirstWriteMark<T::Owner>, FirstWriteJournalError<T::Error>> {
        self.validate_owner(target)?;
        let inverse_cursor = u32::try_from(self.inverses.len())
            .map_err(|_| FirstWriteJournalError::AllocationFailed)?;
        let depth = u32::try_from(self.marks.len())
            .map_err(|_| FirstWriteJournalError::AllocationFailed)?;
        self.marks
            .try_reserve(1)
            .map_err(|_| FirstWriteJournalError::AllocationFailed)?;
        let epoch = self.next_epoch;
        self.next_epoch = epoch
            .get()
            .checked_add(1)
            .and_then(NonZeroU32::new)
            .ok_or(FirstWriteJournalError::EpochExhausted)?;
        self.marks.push(JournalFrame {
            epoch,
            inverse_cursor,
        });
        Ok(FirstWriteMark {
            owner: self.owner,
            epoch,
            inverse_cursor,
            depth,
        })
    }

    pub(crate) fn write(
        &mut self,
        target: &mut T,
        coordinate: T::Coordinate,
        value: T::Value,
    ) -> Result<(), FirstWriteJournalError<T::Error>> {
        self.validate_owner(target)?;
        let frame = *self
            .marks
            .last()
            .ok_or(FirstWriteJournalError::NoActiveMark)?;
        let (old_value, previous_epoch) = target
            .read_for_journal(coordinate)
            .map_err(FirstWriteJournalError::Target)?;
        if previous_epoch != frame.epoch.get() {
            self.inverses
                .try_reserve(1)
                .map_err(|_| FirstWriteJournalError::AllocationFailed)?;
            self.inverses.push(InverseRecord {
                coordinate,
                old_value,
                previous_epoch,
            });
        }
        target.write_validated(coordinate, value, frame.epoch.get());
        Ok(())
    }

    pub(crate) fn rollback(
        &mut self,
        target: &mut T,
        mark: FirstWriteMark<T::Owner>,
    ) -> Result<(), FirstWriteJournalError<T::Error>> {
        self.validate_active_mark(target, mark)?;
        self.validate_suffix(target, mark.inverse_cursor as usize)?;
        for inverse in self.inverses[mark.inverse_cursor as usize..]
            .iter()
            .rev()
            .copied()
        {
            target.write_validated(
                inverse.coordinate,
                inverse.old_value,
                inverse.previous_epoch,
            );
        }
        self.inverses.truncate(mark.inverse_cursor as usize);
        self.marks.pop();
        self.reset_epoch_if_idle();
        Ok(())
    }

    /// Validates an exact rollback without changing the journal or target.
    pub(crate) fn validate_rollback(
        &self,
        target: &T,
        mark: FirstWriteMark<T::Owner>,
    ) -> Result<(), FirstWriteJournalError<T::Error>> {
        self.validate_active_mark(target, mark)?;
        self.validate_suffix(target, mark.inverse_cursor as usize)
    }

    pub(crate) fn commit(
        &mut self,
        target: &mut T,
        mark: FirstWriteMark<T::Owner>,
    ) -> Result<(), FirstWriteJournalError<T::Error>> {
        self.validate_active_mark(target, mark)?;
        self.validate_suffix(target, mark.inverse_cursor as usize)?;
        let parent_epoch = self
            .marks
            .get(mark.depth.saturating_sub(1) as usize)
            .filter(|_| mark.depth != 0)
            .map_or(0, |frame| frame.epoch.get());
        if parent_epoch == 0 {
            for inverse in self.inverses[mark.inverse_cursor as usize..]
                .iter()
                .rev()
                .copied()
            {
                target.write_validated(
                    inverse.coordinate,
                    target
                        .read_for_journal(inverse.coordinate)
                        .map_err(FirstWriteJournalError::Target)?
                        .0,
                    inverse.previous_epoch,
                );
            }
            self.inverses.truncate(mark.inverse_cursor as usize);
        } else {
            for inverse in self.inverses[mark.inverse_cursor as usize..]
                .iter()
                .copied()
            {
                let value = target
                    .read_for_journal(inverse.coordinate)
                    .map_err(FirstWriteJournalError::Target)?
                    .0;
                target.write_validated(inverse.coordinate, value, parent_epoch);
            }
        }
        self.marks.pop();
        self.reset_epoch_if_idle();
        Ok(())
    }

    /// Returns whether no transaction mark owns journal history.
    pub(crate) fn is_idle(&self) -> bool {
        self.marks.is_empty()
    }

    pub(crate) fn accounting(&self) -> FirstWriteJournalAccounting {
        let retained_inverse_heap_entries = if self.inverses.spilled() {
            self.inverses.capacity()
        } else {
            0
        };
        let retained_mark_heap_entries = if self.marks.spilled() {
            self.marks.capacity()
        } else {
            0
        };
        FirstWriteJournalAccounting {
            logical_inverses: self.inverses.len(),
            logical_inverse_bytes: self
                .inverses
                .len()
                .saturating_mul(size_of::<InverseRecord<T::Coordinate, T::Value>>()),
            active_marks: self.marks.len(),
            retained_inverse_heap_entries,
            retained_mark_heap_entries,
            retained_heap_bytes: retained_inverse_heap_entries
                .saturating_mul(size_of::<InverseRecord<T::Coordinate, T::Value>>())
                .saturating_add(
                    retained_mark_heap_entries.saturating_mul(size_of::<JournalFrame>()),
                ),
        }
    }

    fn validate_owner(&self, target: &T) -> Result<(), FirstWriteJournalError<T::Error>> {
        if target.journal_owner() != self.owner {
            return Err(FirstWriteJournalError::ForeignTarget);
        }
        Ok(())
    }

    fn validate_active_mark(
        &self,
        target: &T,
        mark: FirstWriteMark<T::Owner>,
    ) -> Result<(), FirstWriteJournalError<T::Error>> {
        self.validate_owner(target)?;
        let Some(frame) = self.marks.last() else {
            return Err(FirstWriteJournalError::InvalidMark);
        };
        let depth = self.marks.len().saturating_sub(1);
        if mark.owner != self.owner
            || mark.depth as usize != depth
            || mark.epoch != frame.epoch
            || mark.inverse_cursor != frame.inverse_cursor
        {
            return Err(FirstWriteJournalError::InvalidMark);
        }
        Ok(())
    }

    fn validate_suffix(
        &self,
        target: &T,
        cursor: usize,
    ) -> Result<(), FirstWriteJournalError<T::Error>> {
        for inverse in &self.inverses[cursor..] {
            target
                .validate_for_journal(inverse.coordinate)
                .map_err(FirstWriteJournalError::Target)?;
        }
        Ok(())
    }

    fn reset_epoch_if_idle(&mut self) {
        if self.marks.is_empty() {
            debug_assert!(self.inverses.is_empty());
            self.next_epoch = NonZeroU32::MIN;
        }
    }
}
