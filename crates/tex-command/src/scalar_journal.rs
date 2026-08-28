//! Reusable fixed-chunk bidirectional journals for checkpoint-local state.
//!
//! A retained mark always lands between chunks. Mutation therefore appends to
//! one interval-local suffix without publishing a range descriptor. A fork
//! cuts the accepted linked chunk lane at that mark, leaving exactly the
//! selected prefix, detached prior suffix, and current candidate suffix.

const CHUNKS_PER_PAGE: usize = 16;

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) struct PackedJournalMark {
    tail: Option<ChunkKey>,
    chunks: u32,
    records: u64,
}

impl PackedJournalMark {
    pub(crate) const fn synthetic(records: u32) -> Self {
        Self {
            tail: None,
            chunks: records,
            records: records as u64,
        }
    }

    pub(crate) const fn record_count(self) -> u32 {
        self.records as u32
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct PackedJournalCounters {
    pub(crate) records: u64,
    pub(crate) record_bytes: u64,
    pub(crate) chunks_acquired: u64,
    pub(crate) chunks_reused: u64,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct ChunkKey {
    slot: u32,
    generation: u32,
}

struct JournalChunk<T, const RECORDS: usize> {
    generation: u32,
    live: bool,
    used: u16,
    depth: u32,
    records_before: u64,
    lane: ChunkLane,
    previous: Option<ChunkKey>,
    next: Option<ChunkKey>,
    free_next: Option<u32>,
    values: [Option<T>; RECORDS],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ChunkLane {
    Free,
    Current,
    Detached,
}

impl<T, const RECORDS: usize> JournalChunk<T, RECORDS> {
    fn vacant(free_next: Option<u32>) -> Self {
        Self {
            generation: 1,
            live: false,
            used: 0,
            depth: 0,
            records_before: 0,
            lane: ChunkLane::Free,
            previous: None,
            next: None,
            free_next,
            values: std::array::from_fn(|_| None),
        }
    }
}

struct JournalPage<T, const RECORDS: usize> {
    chunks: Box<[JournalChunk<T, RECORDS>]>,
}

#[derive(Clone, Copy)]
struct JournalFork {
    selected: PackedJournalMark,
    prior_tail: Option<ChunkKey>,
    prior_chunks: u32,
    prior_records: u64,
}

/// Append-only fixed-chunk journal with one accepted lineage and one fork.
pub(crate) struct PackedJournal<T, const RECORDS: usize> {
    pages: Vec<JournalPage<T, RECORDS>>,
    free_head: Option<u32>,
    head: Option<ChunkKey>,
    tail: Option<ChunkKey>,
    chunks: u32,
    records: u64,
    detached_head: Option<ChunkKey>,
    fork: Option<JournalFork>,
    interval_tail_open: bool,
    counters: PackedJournalCounters,
}

impl<T, const RECORDS: usize> Default for PackedJournal<T, RECORDS> {
    fn default() -> Self {
        assert!(RECORDS != 0 && RECORDS <= u16::MAX as usize);
        Self {
            pages: Vec::new(),
            free_head: None,
            head: None,
            tail: None,
            chunks: 0,
            records: 0,
            detached_head: None,
            fork: None,
            interval_tail_open: false,
            counters: PackedJournalCounters::default(),
        }
    }
}

impl<T, const RECORDS: usize> PackedJournal<T, RECORDS> {
    pub(crate) fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            .saturating_add(
                self.pages
                    .capacity()
                    .saturating_mul(std::mem::size_of::<JournalPage<T, RECORDS>>()),
            )
            .saturating_add(self.pages.iter().fold(0_usize, |bytes, page| {
                bytes.saturating_add(
                    page.chunks
                        .len()
                        .saturating_mul(std::mem::size_of::<JournalChunk<T, RECORDS>>()),
                )
            }))
    }

    #[cfg(any(test, feature = "profiling"))]
    pub(crate) const fn counters(&self) -> PackedJournalCounters {
        self.counters
    }

    /// Warms the first coarse page before the measured post-checkpoint path.
    pub(crate) fn warm_first_page(&mut self) {
        if self.pages.is_empty() {
            self.add_page();
        }
    }

    pub(crate) fn mark(&mut self) -> PackedJournalMark {
        self.interval_tail_open = false;
        PackedJournalMark {
            tail: self.tail,
            chunks: self.chunks,
            records: self.records,
        }
    }

    pub(crate) fn validates(&self, mark: PackedJournalMark) -> bool {
        if mark.chunks > self.chunks || mark.records > self.records {
            return false;
        }
        match mark.tail {
            None => mark.chunks == 0 && mark.records == 0,
            Some(key) => self.chunk(key).is_some_and(|chunk| {
                chunk.live
                    && chunk.lane == ChunkLane::Current
                    && chunk.depth == mark.chunks
                    && chunk.records_before.saturating_add(u64::from(chunk.used)) == mark.records
            }),
        }
    }

    pub(crate) fn append(&mut self, value: T) {
        let needs_chunk = !self.interval_tail_open
            || self
                .tail
                .and_then(|tail| self.chunk(tail))
                .is_none_or(|chunk| usize::from(chunk.used) == RECORDS);
        if needs_chunk {
            self.acquire_current_chunk();
        }
        let tail = self.tail.expect("journal append acquired a tail chunk");
        let record = {
            let chunk = self
                .chunk_mut(tail)
                .expect("live journal tail remains addressable");
            let record = usize::from(chunk.used);
            chunk.values[record] = Some(value);
            chunk.used += 1;
            record
        };
        debug_assert!(record < RECORDS);
        self.records = self.records.saturating_add(1);
        self.counters.records = self.counters.records.saturating_add(1);
        self.counters.record_bytes = self
            .counters
            .record_bytes
            .saturating_add(std::mem::size_of::<T>() as u64);
    }

    pub(crate) fn restore(
        &mut self,
        mark: PackedJournalMark,
        mut swap: impl FnMut(&mut T),
    ) -> bool {
        self.restore_with(
            mark,
            &mut (),
            |value, ()| swap(value),
            |value, ()| drop(value),
        )
    }

    pub(crate) fn restore_with<C>(
        &mut self,
        mark: PackedJournalMark,
        context: &mut C,
        mut swap: impl FnMut(&mut T, &mut C),
        mut release: impl FnMut(T, &mut C),
    ) -> bool {
        if self.fork.is_some() || !self.validates(mark) {
            return false;
        }
        self.visit_current_suffix_reverse(mark, &mut |value| swap(value, context));
        self.release_current_suffix(mark, &mut |value| release(value, context));
        self.interval_tail_open = false;
        true
    }

    pub(crate) fn begin_checkpoint_candidate(
        &mut self,
        mark: PackedJournalMark,
        mut swap: impl FnMut(&mut T),
    ) {
        assert!(self.fork.is_none() && self.validates(mark));
        debug_assert!(self.detached_head.is_none());
        self.visit_current_suffix_reverse(mark, &mut swap);
        let prior_tail = self.tail;
        let prior_chunks = self.chunks;
        let prior_records = self.records;
        let suffix_head = match mark.tail {
            Some(tail) => self.chunk(tail).and_then(|chunk| chunk.next),
            None => self.head,
        };
        self.detached_head = suffix_head;
        if let Some(head) = suffix_head {
            self.chunk_mut(head)
                .expect("detached suffix head remains live")
                .previous = None;
        }
        let mut detached = suffix_head;
        while let Some(key) = detached {
            let chunk = self
                .chunk_mut(key)
                .expect("detached journal suffix remains live");
            chunk.lane = ChunkLane::Detached;
            detached = chunk.next;
        }
        if let Some(tail) = mark.tail {
            self.chunk_mut(tail)
                .expect("selected journal tail remains live")
                .next = None;
        } else {
            self.head = None;
        }
        self.tail = mark.tail;
        self.chunks = mark.chunks;
        self.records = mark.records;
        self.fork = Some(JournalFork {
            selected: mark,
            prior_tail,
            prior_chunks,
            prior_records,
        });
        self.interval_tail_open = false;
    }

    pub(crate) fn reject_checkpoint_candidate(&mut self, mut swap: impl FnMut(&mut T)) {
        self.reject_checkpoint_candidate_with(
            &mut (),
            |value, ()| swap(value),
            |value, ()| drop(value),
        );
    }

    pub(crate) fn reject_checkpoint_candidate_with<C>(
        &mut self,
        context: &mut C,
        mut swap: impl FnMut(&mut T, &mut C),
        mut release: impl FnMut(T, &mut C),
    ) {
        let fork = self
            .fork
            .take()
            .expect("journal rejection requires a candidate fork");
        self.visit_current_suffix_reverse(fork.selected, &mut |value| swap(value, context));
        self.release_current_suffix(fork.selected, &mut |value| release(value, context));
        self.visit_detached_forward(&mut |value| swap(value, context));

        match (self.tail, self.detached_head) {
            (Some(prefix_tail), Some(detached_head)) => {
                self.chunk_mut(prefix_tail)
                    .expect("selected prefix tail remains live")
                    .next = Some(detached_head);
                self.chunk_mut(detached_head)
                    .expect("detached head remains live")
                    .previous = Some(prefix_tail);
            }
            (None, Some(detached_head)) => self.head = Some(detached_head),
            (_, None) => {}
        }
        self.tail = fork.prior_tail;
        self.chunks = fork.prior_chunks;
        self.records = fork.prior_records;
        self.detached_head = None;
        let mut current = self.head;
        while let Some(key) = current {
            let chunk = self
                .chunk_mut(key)
                .expect("reattached journal suffix remains live");
            chunk.lane = ChunkLane::Current;
            current = chunk.next;
        }
        self.interval_tail_open = false;
    }

    pub(crate) fn accept_checkpoint_candidate(&mut self) {
        self.accept_checkpoint_candidate_with(&mut (), |value, ()| drop(value));
    }

    pub(crate) fn accept_checkpoint_candidate_with<C>(
        &mut self,
        context: &mut C,
        mut release: impl FnMut(T, &mut C),
    ) {
        self.fork
            .take()
            .expect("journal acceptance requires a candidate fork");
        self.release_detached(&mut |value| release(value, context));
        self.interval_tail_open = false;
    }

    fn add_page(&mut self) {
        let start = self.pages.len().saturating_mul(CHUNKS_PER_PAGE);
        assert!(u32::try_from(start.saturating_add(CHUNKS_PER_PAGE)).is_ok());
        let mut free = self.free_head;
        let chunks = (0..CHUNKS_PER_PAGE)
            .map(|offset| {
                let slot = u32::try_from(start + offset).expect("journal page fits u32");
                let chunk = JournalChunk::vacant(free);
                free = Some(slot);
                chunk
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        self.pages.push(JournalPage { chunks });
        self.free_head = free;
    }

    fn acquire_current_chunk(&mut self) {
        if self.free_head.is_none() {
            self.add_page();
        }
        let slot = self.free_head.expect("journal page supplied a free chunk");
        let previous = self.tail;
        let depth = self.chunks.checked_add(1).expect("journal chunk capacity");
        let records_before = self.records;
        let reused = self.chunk_by_slot_mut(slot).generation != 1;
        let next_free = self.chunk_by_slot_mut(slot).free_next;
        self.free_head = next_free;
        let key = {
            let chunk = self.chunk_by_slot_mut(slot);
            chunk.free_next = None;
            chunk.live = true;
            chunk.used = 0;
            chunk.depth = depth;
            chunk.records_before = records_before;
            chunk.lane = ChunkLane::Current;
            chunk.previous = previous;
            chunk.next = None;
            ChunkKey {
                slot,
                generation: chunk.generation,
            }
        };
        if let Some(previous) = previous {
            self.chunk_mut(previous)
                .expect("journal previous tail remains live")
                .next = Some(key);
        } else {
            self.head = Some(key);
        }
        self.tail = Some(key);
        self.chunks = depth;
        self.interval_tail_open = true;
        self.counters.chunks_acquired = self.counters.chunks_acquired.saturating_add(1);
        if reused {
            self.counters.chunks_reused = self.counters.chunks_reused.saturating_add(1);
        }
    }

    fn visit_current_suffix_reverse(
        &mut self,
        mark: PackedJournalMark,
        swap: &mut impl FnMut(&mut T),
    ) {
        let mut cursor = self.tail;
        while cursor != mark.tail {
            let key = cursor.expect("validated journal suffix reaches its mark");
            let previous = self.chunk(key).expect("live journal suffix chunk").previous;
            let used = usize::from(self.chunk(key).expect("live journal suffix chunk").used);
            for index in (0..used).rev() {
                let value = self
                    .chunk_mut(key)
                    .and_then(|chunk| chunk.values[index].as_mut())
                    .expect("used journal record is initialized");
                swap(value);
            }
            cursor = previous;
        }
    }

    fn visit_detached_forward(&mut self, swap: &mut impl FnMut(&mut T)) {
        let mut cursor = self.detached_head;
        while let Some(key) = cursor {
            let next = self.chunk(key).expect("live detached chunk").next;
            let used = usize::from(self.chunk(key).expect("live detached chunk").used);
            for index in 0..used {
                let value = self
                    .chunk_mut(key)
                    .and_then(|chunk| chunk.values[index].as_mut())
                    .expect("used detached record is initialized");
                swap(value);
            }
            cursor = next;
        }
    }

    fn release_current_suffix(&mut self, mark: PackedJournalMark, release: &mut impl FnMut(T)) {
        let mut cursor = self.tail;
        while cursor != mark.tail {
            let key = cursor.expect("validated journal suffix reaches mark");
            cursor = self.chunk(key).expect("live journal suffix chunk").previous;
            self.release_chunk(key, release);
        }
        self.tail = mark.tail;
        if let Some(tail) = mark.tail {
            self.chunk_mut(tail)
                .expect("selected journal tail remains live")
                .next = None;
        } else {
            self.head = None;
        }
        self.chunks = mark.chunks;
        self.records = mark.records;
    }

    fn release_detached(&mut self, release: &mut impl FnMut(T)) {
        let mut cursor = self.detached_head;
        while let Some(key) = cursor {
            cursor = self.chunk(key).expect("live detached chunk").next;
            self.release_chunk(key, release);
        }
        self.detached_head = None;
    }

    fn release_chunk(&mut self, key: ChunkKey, release: &mut impl FnMut(T)) {
        let free_head = self.free_head;
        let chunk = self.chunk_mut(key).expect("released journal chunk is live");
        for value in &mut chunk.values[..usize::from(chunk.used)] {
            if let Some(value) = value.take() {
                release(value);
            }
        }
        chunk.live = false;
        chunk.used = 0;
        chunk.depth = 0;
        chunk.records_before = 0;
        chunk.lane = ChunkLane::Free;
        chunk.previous = None;
        chunk.next = None;
        chunk.generation = chunk.generation.wrapping_add(1).max(1);
        chunk.free_next = free_head;
        self.free_head = Some(key.slot);
    }

    fn chunk(&self, key: ChunkKey) -> Option<&JournalChunk<T, RECORDS>> {
        let slot = key.slot as usize;
        let chunk = self
            .pages
            .get(slot / CHUNKS_PER_PAGE)?
            .chunks
            .get(slot % CHUNKS_PER_PAGE)?;
        (chunk.generation == key.generation && chunk.live).then_some(chunk)
    }

    fn chunk_mut(&mut self, key: ChunkKey) -> Option<&mut JournalChunk<T, RECORDS>> {
        let slot = key.slot as usize;
        let chunk = self
            .pages
            .get_mut(slot / CHUNKS_PER_PAGE)?
            .chunks
            .get_mut(slot % CHUNKS_PER_PAGE)?;
        (chunk.generation == key.generation && chunk.live).then_some(chunk)
    }

    fn chunk_by_slot_mut(&mut self, slot: u32) -> &mut JournalChunk<T, RECORDS> {
        let slot = slot as usize;
        &mut self.pages[slot / CHUNKS_PER_PAGE].chunks[slot % CHUNKS_PER_PAGE]
    }
}

#[cfg(test)]
mod tests;
