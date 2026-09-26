//! Authoritative owner-relative chunk slots with compact vacancy ranges.

use super::{LogicalChunkId, VACANT_LOGICAL_CHUNK};
use std::ops::Range;

#[derive(Clone, Copy, Debug)]
struct Gap {
    start: usize,
    end: usize,
    vacant_through: usize,
}

/// Logical positions remain stable after an interior move. Live keys stay in
/// order, while disjoint vacant ranges carry only scalar lengths. The common
/// dense owner has no gaps and indexes its live vector directly.
#[derive(Clone, Default)]
pub(super) struct SparseChunks {
    live: Vec<LogicalChunkId>,
    gaps: Vec<Gap>,
    logical_len: usize,
}

impl SparseChunks {
    fn push_gap(gaps: &mut Vec<Gap>, start: usize, end: usize) {
        if start == end {
            return;
        }
        let vacant_through = gaps.last().map_or(0, |gap| gap.vacant_through) + end - start;
        gaps.push(Gap {
            start,
            end,
            vacant_through,
        });
    }

    pub(super) fn len(&self) -> usize {
        self.logical_len
    }

    #[cfg(test)]
    pub(super) fn live_len(&self) -> usize {
        self.live.len()
    }

    #[cfg(test)]
    pub(super) fn gap_count(&self) -> usize {
        self.gaps.len()
    }

    fn gap_after_or_at(&self, position: usize) -> usize {
        self.gaps.partition_point(|gap| gap.end <= position)
    }

    fn vacant_before(&self, position: usize) -> usize {
        let next = self.gap_after_or_at(position);
        let mut count = if next == 0 {
            0
        } else {
            self.gaps[next - 1].vacant_through
        };
        if let Some(gap) = self.gaps.get(next)
            && gap.start < position
        {
            count += position.min(gap.end) - gap.start;
        }
        count
    }

    pub(super) fn get(&self, position: usize) -> Option<&LogicalChunkId> {
        if position >= self.logical_len {
            return None;
        }
        if self.gaps.is_empty() {
            return self.live.get(position);
        }
        let next = self.gap_after_or_at(position);
        if self.gaps.get(next).is_some_and(|gap| gap.start <= position) {
            return Some(&VACANT_LOGICAL_CHUNK);
        }
        let before = if next == 0 {
            0
        } else {
            self.gaps[next - 1].vacant_through
        };
        self.live.get(position - before)
    }

    pub(super) fn last(&self) -> Option<&LogicalChunkId> {
        self.logical_len
            .checked_sub(1)
            .and_then(|position| self.get(position))
    }

    pub(super) fn last_live_position(&self) -> Option<usize> {
        self.live.last()?;
        let trailing_gap = self.gaps.last().filter(|gap| gap.end == self.logical_len);
        Some(trailing_gap.map_or(self.logical_len - 1, |gap| gap.start - 1))
    }

    #[cfg(test)]
    pub(super) fn iter(&self) -> SparseChunksIter<'_> {
        SparseChunksIter {
            chunks: self,
            position: 0,
        }
    }

    pub(super) fn iter_live_with_positions(&self) -> SparseChunksLiveIter<'_> {
        self.iter_live_from(0)
    }

    pub(super) fn iter_live_from(&self, start: usize) -> SparseChunksLiveIter<'_> {
        let mut position = start.min(self.logical_len);
        let mut gap_index = self.gap_after_or_at(position);
        if let Some(gap) = self.gaps.get(gap_index)
            && gap.start <= position
        {
            position = gap.end;
            gap_index += 1;
        }
        SparseChunksLiveIter {
            chunks: self,
            live_index: position - self.vacant_before(position),
            gap_index,
            position,
        }
    }

    fn rebuild_gap_prefixes_from(&mut self, first: usize) {
        let mut count = if first == 0 {
            0
        } else {
            self.gaps[first - 1].vacant_through
        };
        for gap in &mut self.gaps[first..] {
            count += gap.end - gap.start;
            gap.vacant_through = count;
        }
        debug_assert_eq!(
            self.live.len() + self.gaps.last().map_or(0, |gap| gap.vacant_through),
            self.logical_len
        );
    }

    fn rebuild_gap_prefixes(&mut self) {
        self.rebuild_gap_prefixes_from(0);
    }

    fn insert_gap(&mut self, start: usize, end: usize) {
        if start == end {
            return;
        }
        let first = self.gaps.partition_point(|gap| gap.end < start);
        let mut merged_start = start;
        let mut merged_end = end;
        while self
            .gaps
            .get(first)
            .is_some_and(|gap| gap.start <= merged_end)
        {
            let gap = self.gaps.remove(first);
            assert!(
                gap.end <= merged_start || gap.start >= merged_end,
                "vacant ranges cannot overlap"
            );
            merged_start = merged_start.min(gap.start);
            merged_end = merged_end.max(gap.end);
        }
        self.gaps.insert(
            first,
            Gap {
                start: merged_start,
                end: merged_end,
                vacant_through: 0,
            },
        );
        self.rebuild_gap_prefixes_from(first);
    }

    /// Removes an entirely live logical interval and returns its keys in
    /// unchanged coordinate order. The interval becomes one merged gap.
    pub(super) fn take_live_range(&mut self, start: usize, end: usize) -> Vec<LogicalChunkId> {
        assert!(start <= end && end <= self.logical_len);
        if start == end {
            return Vec::new();
        }
        assert_eq!(self.vacant_before(end) - self.vacant_before(start), 0);
        let physical_start = start - self.vacant_before(start);
        let keys = self
            .live
            .drain(physical_start..physical_start + (end - start))
            .collect();
        self.insert_gap(start, end);
        keys
    }

    /// Partitions every selected interval in one ordered pass. The returned
    /// envelope retains the coordinates between intervals as vacancies, and
    /// existing vacancies stay vacant in both owners.
    pub(super) fn take_selected_ranges(&mut self, ranges: &[Range<usize>]) -> (Self, Vec<usize>) {
        if ranges.is_empty() {
            return (Self::default(), Vec::new());
        }
        let base = ranges[0].start;
        let end = ranges.last().expect("nonempty ranges").end;
        debug_assert!(base < end && end <= self.logical_len);
        debug_assert!(ranges.windows(2).all(|pair| pair[0].end <= pair[1].start));
        let original = std::mem::take(self);
        let mut retained_live = Vec::with_capacity(original.live.len());
        let mut selected_live = Vec::new();
        let mut retained_gaps = Vec::new();
        let mut selected_gaps = Vec::new();
        let mut retained_cursor = 0;
        let mut selected_cursor = 0;
        let mut range_index = 0;
        let mut live_counts = vec![0; ranges.len()];
        for (position, key) in original.iter_live_with_positions() {
            while ranges
                .get(range_index)
                .is_some_and(|range| range.end <= position)
            {
                range_index += 1;
            }
            if ranges
                .get(range_index)
                .is_some_and(|range| range.start <= position)
            {
                let relative = position - base;
                Self::push_gap(&mut selected_gaps, selected_cursor, relative);
                selected_live.push(key);
                selected_cursor = relative + 1;
                live_counts[range_index] += 1;
            } else {
                Self::push_gap(&mut retained_gaps, retained_cursor, position);
                retained_live.push(key);
                retained_cursor = position + 1;
            }
        }
        Self::push_gap(&mut retained_gaps, retained_cursor, original.logical_len);
        Self::push_gap(&mut selected_gaps, selected_cursor, end - base);
        *self = Self {
            live: retained_live,
            gaps: retained_gaps,
            logical_len: original.logical_len,
        };
        let selected = Self {
            live: selected_live,
            gaps: selected_gaps,
            logical_len: end - base,
        };
        debug_assert_eq!(
            self.live.len() + self.gaps.last().map_or(0, |gap| gap.vacant_through),
            self.logical_len
        );
        debug_assert_eq!(
            selected.live.len() + selected.gaps.last().map_or(0, |gap| gap.vacant_through),
            selected.logical_len
        );
        (selected, live_counts)
    }

    /// Recombines a loan with its page owner in one ordered pass. Logical
    /// vacancies that belonged to either owner remain vacant.
    pub(super) fn restore_sparse_overlay(&mut self, base: usize, selected: Self) {
        debug_assert!(base + selected.logical_len <= self.logical_len);
        let retained = std::mem::take(self);
        let mut retained_live = retained.iter_live_with_positions().peekable();
        let mut selected_live = selected.iter_live_with_positions().peekable();
        let mut live = Vec::with_capacity(retained.live.len() + selected.live.len());
        let mut gaps = Vec::new();
        let mut cursor = 0;
        loop {
            let from_selected = match (retained_live.peek(), selected_live.peek()) {
                (Some((left, _)), Some((right, _))) => {
                    debug_assert_ne!(*left, base + *right);
                    *left > base + *right
                }
                (None, Some(_)) => true,
                (Some(_), None) => false,
                (None, None) => break,
            };
            let (position, key) = if from_selected {
                let (relative, key) = selected_live.next().expect("selected position remains");
                (base + relative, key)
            } else {
                retained_live.next().expect("retained position remains")
            };
            Self::push_gap(&mut gaps, cursor, position);
            live.push(key);
            cursor = position + 1;
        }
        Self::push_gap(&mut gaps, cursor, retained.logical_len);
        *self = Self {
            live,
            gaps,
            logical_len: retained.logical_len,
        };
        debug_assert_eq!(
            self.live.len() + self.gaps.last().map_or(0, |gap| gap.vacant_through),
            self.logical_len
        );
    }

    /// Fills the exact vacant interval named by a reverse ownership loan.
    pub(super) fn restore_range(&mut self, start: usize, keys: Vec<LogicalChunkId>) {
        let end = start + keys.len();
        if start == end {
            return;
        }
        let gap_index = self.gap_after_or_at(start);
        let gap = *self
            .gaps
            .get(gap_index)
            .expect("reverse loan names an existing vacant range");
        assert!(gap.start <= start && end <= gap.end);
        let physical_start = start - self.vacant_before(start);
        self.live.splice(physical_start..physical_start, keys);
        self.gaps.remove(gap_index);
        if gap.start < start {
            self.gaps.insert(
                gap_index,
                Gap {
                    start: gap.start,
                    end: start,
                    vacant_through: 0,
                },
            );
        }
        if end < gap.end {
            self.gaps.insert(
                self.gap_after_or_at(end),
                Gap {
                    start: end,
                    end: gap.end,
                    vacant_through: 0,
                },
            );
        }
        self.rebuild_gap_prefixes();
    }

    pub(super) fn push(&mut self, key: LogicalChunkId) {
        self.logical_len += 1;
        if key == VACANT_LOGICAL_CHUNK {
            self.insert_gap(self.logical_len - 1, self.logical_len);
        } else {
            self.live.push(key);
        }
    }

    pub(super) fn extend(&mut self, keys: impl IntoIterator<Item = LogicalChunkId>) {
        for key in keys {
            self.push(key);
        }
    }

    pub(super) fn append(&mut self, mut suffix: Self) {
        let offset = self.logical_len;
        self.logical_len += suffix.logical_len;
        self.live.append(&mut suffix.live);
        for mut gap in suffix.gaps {
            gap.start += offset;
            gap.end += offset;
            if let Some(last) = self.gaps.last_mut()
                && last.end == gap.start
            {
                last.end = gap.end;
            } else {
                self.gaps.push(gap);
            }
        }
        self.rebuild_gap_prefixes();
    }

    pub(super) fn pop(&mut self) -> Option<LogicalChunkId> {
        let position = self.logical_len.checked_sub(1)?;
        self.logical_len -= 1;
        if self.gaps.last().is_some_and(|gap| gap.end == position + 1) {
            let gap = self.gaps.last_mut().expect("checked final gap");
            gap.end -= 1;
            if gap.start == gap.end {
                self.gaps.pop();
            }
            self.rebuild_gap_prefixes();
            Some(VACANT_LOGICAL_CHUNK)
        } else {
            self.live.pop()
        }
    }

    pub(super) fn split_off(&mut self, at: usize) -> Self {
        assert!(at <= self.logical_len);
        let old_len = self.logical_len;
        let physical_at = at - self.vacant_before(at);
        let right_live = self.live.split_off(physical_at);
        let mut left_gaps = Vec::new();
        let mut right_gaps = Vec::new();
        for gap in self.gaps.drain(..) {
            if gap.start < at {
                left_gaps.push(Gap {
                    end: gap.end.min(at),
                    ..gap
                });
            }
            if gap.end > at {
                right_gaps.push(Gap {
                    start: gap.start.max(at) - at,
                    end: gap.end - at,
                    vacant_through: 0,
                });
            }
        }
        self.gaps = left_gaps;
        self.logical_len = at;
        self.rebuild_gap_prefixes();
        let mut right = Self {
            live: right_live,
            gaps: right_gaps,
            logical_len: old_len - at,
        };
        right.rebuild_gap_prefixes();
        right
    }

    pub(super) fn drain_prefix(&mut self, count: usize) -> Vec<LogicalChunkId> {
        let suffix = self.split_off(count);
        let prefix = core::mem::replace(self, suffix);
        prefix.into_live_keys()
    }

    pub(super) fn slice_clone(&self, start: usize, end: usize) -> Self {
        assert!(start <= end && end <= self.logical_len);
        let physical_start = start - self.vacant_before(start);
        let physical_end = end - self.vacant_before(end);
        let gaps = self
            .gaps
            .iter()
            .filter_map(|gap| {
                let clipped_start = gap.start.max(start);
                let clipped_end = gap.end.min(end);
                (clipped_start < clipped_end).then_some(Gap {
                    start: clipped_start - start,
                    end: clipped_end - start,
                    vacant_through: 0,
                })
            })
            .collect();
        let mut selected = Self {
            live: self.live[physical_start..physical_end].to_vec(),
            gaps,
            logical_len: end - start,
        };
        selected.rebuild_gap_prefixes();
        selected
    }

    pub(super) fn into_live_keys(self) -> Vec<LogicalChunkId> {
        self.live
    }
}

pub(super) struct SparseChunksLiveIter<'a> {
    chunks: &'a SparseChunks,
    live_index: usize,
    gap_index: usize,
    position: usize,
}

impl Iterator for SparseChunksLiveIter<'_> {
    type Item = (usize, LogicalChunkId);

    fn next(&mut self) -> Option<Self::Item> {
        let key = *self.chunks.live.get(self.live_index)?;
        if let Some(gap) = self.chunks.gaps.get(self.gap_index)
            && gap.start == self.position
        {
            self.position = gap.end;
            self.gap_index += 1;
        }
        let position = self.position;
        self.position += 1;
        self.live_index += 1;
        Some((position, key))
    }
}

#[cfg(test)]
pub(super) struct SparseChunksIter<'a> {
    chunks: &'a SparseChunks,
    position: usize,
}

#[cfg(test)]
impl<'a> Iterator for SparseChunksIter<'a> {
    type Item = &'a LogicalChunkId;

    fn next(&mut self) -> Option<Self::Item> {
        let value = self.chunks.get(self.position)?;
        self.position += 1;
        Some(value)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let left = self.chunks.logical_len - self.position;
        (left, Some(left))
    }
}

#[cfg(test)]
impl ExactSizeIterator for SparseChunksIter<'_> {}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(ordinal: u32) -> LogicalChunkId {
        LogicalChunkId {
            ordinal,
            incarnation: 1,
        }
    }

    #[test]
    fn append_and_split_preserve_multiple_compact_gaps() {
        let mut prefix = SparseChunks::default();
        prefix.extend([key(0), VACANT_LOGICAL_CHUNK, key(2)]);
        let mut suffix = SparseChunks::default();
        suffix.extend([
            key(3),
            VACANT_LOGICAL_CHUNK,
            key(5),
            VACANT_LOGICAL_CHUNK,
            key(7),
        ]);
        prefix.append(suffix);
        assert_eq!(prefix.len(), 8);
        assert_eq!(prefix.gap_count(), 3);
        assert_eq!(prefix.live_len(), 5);
        assert_eq!(prefix.get(5), Some(&key(5)));
        assert_eq!(prefix.last_live_position(), Some(7));
        assert_eq!(
            prefix.iter_live_from(4).collect::<Vec<_>>(),
            vec![(5, key(5)), (7, key(7))]
        );
        let right = prefix.split_off(5);
        assert_eq!(
            prefix.iter().copied().collect::<Vec<_>>(),
            vec![
                key(0),
                VACANT_LOGICAL_CHUNK,
                key(2),
                key(3),
                VACANT_LOGICAL_CHUNK,
            ]
        );
        assert_eq!(
            right.iter().copied().collect::<Vec<_>>(),
            vec![key(5), VACANT_LOGICAL_CHUNK, key(7),]
        );
        prefix.append(right);
        prefix.restore_range(1, vec![key(1)]);
        prefix.restore_range(4, vec![key(4)]);
        prefix.restore_range(6, vec![key(6)]);
        assert_eq!(prefix.gap_count(), 0);
        assert_eq!(
            prefix.iter().copied().collect::<Vec<_>>(),
            (0..8).map(key).collect::<Vec<_>>()
        );
        prefix.take_live_range(6, 8);
        assert_eq!(prefix.last_live_position(), Some(5));
    }

    #[test]
    fn batch_partition_and_restore_preserve_exclusions_and_nested_vacancies() {
        let mut page = SparseChunks::default();
        let original = [
            key(0),
            key(1),
            VACANT_LOGICAL_CHUNK,
            key(3),
            key(4),
            VACANT_LOGICAL_CHUNK,
            key(6),
            key(7),
            key(8),
            VACANT_LOGICAL_CHUNK,
            key(10),
        ];
        page.extend(original);
        let ranges = [1..4, 6..8];
        let (selected, counts) = page.take_selected_ranges(&ranges);
        assert_eq!(counts, vec![2, 2]);
        assert_eq!(page.live_len(), 4);
        assert_eq!(page.gap_count(), 3);
        assert_eq!(selected.len(), 7);
        assert_eq!(selected.live_len(), 4);
        assert_eq!(selected.gap_count(), 2);
        assert_eq!(page.get(4), Some(&key(4)));
        assert_eq!(selected.get(0), Some(&key(1)));
        assert_eq!(selected.get(2), Some(&key(3)));
        assert_eq!(selected.get(5), Some(&key(6)));
        page.restore_sparse_overlay(1, selected);
        assert_eq!(page.iter().copied().collect::<Vec<_>>(), original);
        assert_eq!(page.gap_count(), 3);

        let (vacant, counts) = page.take_selected_ranges(std::slice::from_ref(&(2..3)));
        assert_eq!(counts, vec![0]);
        assert_eq!(vacant.len(), 1);
        assert_eq!(vacant.live_len(), 0);
        page.restore_sparse_overlay(2, vacant);
        assert_eq!(page.iter().copied().collect::<Vec<_>>(), original);
    }
}
