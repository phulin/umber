//! Compact TeX §§126--127 variable-size allocator projection.
//!
//! The projection retains block boundaries and free-ring order, not TeX's
//! word contents. That is the complete state consulted by `get_node` and
//! `free_node`: allocation scans from the rover, coalesces physically adjacent
//! successors only when it visits them, and takes words from a block's high
//! end. A release is inserted immediately before the rover without eager
//! coalescing.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Allocation {
    start: usize,
    size: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FreeBlock {
    start: usize,
    size: usize,
}

impl FreeBlock {
    fn end(self) -> usize {
        self.start.saturating_add(self.size)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct LowMemoryArena {
    extent: usize,
    growth: usize,
    free_ring: Vec<FreeBlock>,
    growths: usize,
}

impl LowMemoryArena {
    #[cfg(test)]
    pub(super) fn contiguous(extent: usize, growth: usize) -> Self {
        assert!(extent > 1, "a low-memory arena needs a free-node header");
        assert!(
            growth > 1,
            "a low-memory growth block needs a free-node header"
        );
        Self {
            extent,
            growth,
            free_ring: vec![FreeBlock {
                start: 0,
                size: extent,
            }],
            growths: 0,
        }
    }

    pub(super) fn from_live_and_fragments(
        extent: usize,
        growth: usize,
        live_words: usize,
        fragments: &[usize],
    ) -> Self {
        let free_words = extent.saturating_sub(live_words);
        let fragment_words = fragments.iter().sum::<usize>();
        if fragment_words > free_words || fragments.iter().any(|size| *size < 2) {
            return Self::with_free_ring(extent, growth, vec![(0, free_words)]);
        }
        let primary = free_words - fragment_words;
        let mut ring = Vec::new();
        if primary >= 2 {
            ring.push((0, primary));
        }
        let mut cursor = primary.saturating_add(live_words.saturating_sub(fragments.len()));
        for &size in fragments {
            cursor = cursor.saturating_add(1);
            ring.push((cursor, size));
            cursor = cursor.saturating_add(size);
        }
        Self::with_free_ring(extent, growth, ring)
    }

    fn with_free_ring(extent: usize, growth: usize, ring: Vec<(usize, usize)>) -> Self {
        assert!(
            growth > 1,
            "a low-memory growth block needs a free-node header"
        );
        Self {
            extent,
            growth,
            free_ring: ring
                .into_iter()
                .filter(|(_, size)| *size >= 2)
                .map(|(start, size)| FreeBlock { start, size })
                .collect(),
            growths: 0,
        }
    }

    pub(super) fn allocate(&mut self, size: usize) -> Allocation {
        assert!(size > 1, "get_node is only for variable-size nodes");
        loop {
            if let Some(allocation) = self.try_allocate(size) {
                return allocation;
            }
            self.grow();
        }
    }

    pub(super) fn free(&mut self, allocation: Allocation) {
        assert!(
            allocation.start.saturating_add(allocation.size) <= self.extent,
            "released allocation belongs to this arena"
        );
        assert!(
            !self.free_ring.iter().any(|block| {
                allocation.start < block.end()
                    && block.start < allocation.start.saturating_add(allocation.size)
            }),
            "released allocation overlaps a free block"
        );
        // Index zero is the rover. Appending therefore inserts immediately
        // before it in the circular list, exactly as §127 does.
        self.free_ring.push(FreeBlock {
            start: allocation.start,
            size: allocation.size,
        });
    }

    pub(super) const fn extent(&self) -> usize {
        self.extent
    }

    #[cfg(test)]
    pub(super) const fn growths(&self) -> usize {
        self.growths
    }

    pub(super) fn detached_free_sizes(&mut self) -> Vec<usize> {
        let mut index = 0;
        while index < self.free_ring.len() {
            let start = self.free_ring[index].start;
            let _ = self.coalesce_successors(index);
            index = self
                .free_ring
                .iter()
                .position(|block| block.start == start)
                .map_or(self.free_ring.len(), |found| found + 1);
        }
        self.free_ring
            .iter()
            .filter(|block| block.start != 0)
            .map(|block| block.size)
            .collect()
    }

    #[cfg(test)]
    fn free_words(&self) -> usize {
        self.free_ring.iter().map(|block| block.size).sum()
    }

    #[cfg(test)]
    fn free_sizes_from_rover(&self) -> Vec<usize> {
        self.free_ring.iter().map(|block| block.size).collect()
    }

    fn try_allocate(&mut self, size: usize) -> Option<Allocation> {
        let start_rover = self.free_ring.first()?.start;
        let mut index = 0;
        loop {
            index = self.coalesce_successors(index);
            let block = self.free_ring[index];
            if let Some(remaining) = block.size.checked_sub(size) {
                if remaining > 1 {
                    self.free_ring[index].size = remaining;
                    self.rotate_to(index);
                    return Some(Allocation {
                        start: block.start.saturating_add(remaining),
                        size,
                    });
                }
                if remaining == 0 && self.free_ring.len() > 1 {
                    let allocation = Allocation {
                        start: block.start,
                        size,
                    };
                    self.free_ring.remove(index);
                    if index == self.free_ring.len() {
                        index = 0;
                    }
                    self.rotate_to(index);
                    return Some(allocation);
                }
            }

            index = (index + 1) % self.free_ring.len();
            if self.free_ring[index].start == start_rover {
                return None;
            }
        }
    }

    fn coalesce_successors(&mut self, mut index: usize) -> usize {
        let start = self.free_ring[index].start;
        loop {
            let end = self.free_ring[index].end();
            let Some(successor) = self.free_ring.iter().position(|block| block.start == end) else {
                return index;
            };
            if successor == index {
                return index;
            }
            let successor_size = self.free_ring[successor].size;
            self.free_ring[index].size = self.free_ring[index].size.saturating_add(successor_size);
            self.free_ring.remove(successor);
            index = self
                .free_ring
                .iter()
                .position(|block| block.start == start)
                .expect("coalescing retains the visited free block");
        }
    }

    fn rotate_to(&mut self, index: usize) {
        self.free_ring.rotate_left(index);
    }

    fn grow(&mut self) {
        let start = self.extent;
        self.extent = self.extent.saturating_add(self.growth);
        self.growths = self.growths.saturating_add(1);
        // TeX makes the newly grown block the rover before retrying.
        self.free_ring.insert(
            0,
            FreeBlock {
                start,
                size: self.growth,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::LowMemoryArena;

    fn release_terminal_break_pair(arena: &mut LowMemoryArena) {
        let initial = arena.allocate(3);
        arena.free(initial);
        let passive = arena.allocate(2);
        let active = arena.allocate(3);
        // Post-line-break material separates the winning pair from the
        // remaining primary block before §816 releases active then passive.
        let _separator = arena.allocate(4);
        let _line_box = arena.allocate(9);
        arena.free(active);
        arena.free(passive);
    }

    fn release_pair_before_post_line_break(arena: &mut LowMemoryArena) {
        let initial = arena.allocate(3);
        arena.free(initial);
        // This has the same allocation sizes, releases, and final live-word
        // total as `release_terminal_break_pair`, but moves §880's post-line
        // material before the winning pair. The released pair consequently
        // remains adjacent to the primary block instead of becoming a hole.
        let _separator = arena.allocate(4);
        let _line_box = arena.allocate(9);
        let passive = arena.allocate(2);
        let active = arena.allocate(3);
        arena.free(active);
        arena.free(passive);
    }

    #[test]
    fn equal_live_words_distinguish_fragmented_and_contiguous_free_space() {
        let mut fragmented = LowMemoryArena::contiguous(64, 43);
        release_terminal_break_pair(&mut fragmented);
        release_terminal_break_pair(&mut fragmented);
        let _retained = fragmented.allocate(26);
        assert_eq!(fragmented.free_words(), 12);
        assert_eq!(fragmented.free_sizes_from_rover(), vec![2, 3, 2, 3, 2]);

        let fragmented_live_words = fragmented.extent() - fragmented.free_words();
        let mut contiguous = LowMemoryArena::contiguous(64, 43);
        let _same_live_words = contiguous.allocate(fragmented_live_words);
        assert_eq!(contiguous.free_words(), fragmented.free_words());

        let _fragmented_request = fragmented.allocate(9);
        assert_eq!(fragmented.growths(), 1);
        assert_eq!(fragmented.extent(), 107);

        let _contiguous_request = contiguous.allocate(9);
        assert_eq!(contiguous.growths(), 0);
        assert_eq!(contiguous.extent(), 64);
    }

    #[test]
    fn post_line_break_event_order_controls_fragmentation_not_live_words() {
        let mut canonical = LowMemoryArena::contiguous(64, 43);
        let mut reordered = LowMemoryArena::contiguous(64, 43);
        for _ in 0..2 {
            release_terminal_break_pair(&mut canonical);
            release_pair_before_post_line_break(&mut reordered);
        }
        let _canonical_retained = canonical.allocate(26);
        let _reordered_retained = reordered.allocate(26);
        assert_eq!(canonical.free_words(), reordered.free_words());

        let _canonical_request = canonical.allocate(9);
        let _reordered_request = reordered.allocate(9);
        assert_eq!(canonical.growths(), 1);
        assert_eq!(reordered.growths(), 0);
    }
}
