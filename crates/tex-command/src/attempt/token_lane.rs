//! Shared fixed-chunk storage for parent-owned scanner token destinations.
//!
//! One attempt owns the lane. Individual scans own only [`TokenSink`]
//! coordinates, so nested collectors can append independently without a
//! per-scan `Vec` or moving an older collector's words.

use tex_state::token::TracedTokenWord;

use super::AttemptError;

const WORDS_PER_CHUNK: usize = 64;
const NO_CHUNK: u32 = u32::MAX;

#[derive(Debug)]
struct TokenChunk {
    words: Vec<TracedTokenWord>,
    next: u32,
}

impl TokenChunk {
    fn try_new() -> Result<Self, AttemptError> {
        let mut words = Vec::new();
        words
            .try_reserve_exact(WORDS_PER_CHUNK)
            .map_err(|_| AttemptError::AllocationFailed)?;
        Ok(Self {
            words,
            next: NO_CHUNK,
        })
    }
}

/// One mutable branch in the attempt's shared token lane.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct TokenSink {
    head: u32,
    tail: u32,
    len: u32,
}

impl Default for TokenSink {
    fn default() -> Self {
        Self {
            head: NO_CHUNK,
            tail: NO_CHUNK,
            len: 0,
        }
    }
}

impl TokenSink {
    pub(super) const fn len(self) -> u32 {
        self.len
    }
}

/// Reusable physical storage shared by every scanner sink in one attempt.
#[derive(Debug)]
pub(super) struct TokenLane {
    chunks: Vec<TokenChunk>,
    free_head: u32,
    #[cfg(test)]
    counters: TokenLaneCounters,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct TokenLaneCounters {
    pub(super) chunk_allocations: u64,
    pub(super) chunk_reuses: u64,
    pub(super) words_appended: u64,
    pub(super) chunks_released: u64,
}

impl Default for TokenLane {
    fn default() -> Self {
        Self {
            chunks: Vec::new(),
            free_head: NO_CHUNK,
            #[cfg(test)]
            counters: TokenLaneCounters::default(),
        }
    }
}

impl TokenLane {
    pub(super) fn push(
        &mut self,
        sink: &mut TokenSink,
        word: TracedTokenWord,
    ) -> Result<(), AttemptError> {
        let next_len = sink
            .len
            .checked_add(1)
            .ok_or(AttemptError::CapacityOverflow)?;
        let needs_chunk =
            sink.tail == NO_CHUNK || self.chunks[sink.tail as usize].words.len() == WORDS_PER_CHUNK;
        if needs_chunk {
            let chunk = self.allocate_chunk()?;
            if sink.tail == NO_CHUNK {
                sink.head = chunk;
            } else {
                self.chunks[sink.tail as usize].next = chunk;
            }
            sink.tail = chunk;
        }
        self.chunks[sink.tail as usize].words.push(word);
        sink.len = next_len;
        #[cfg(test)]
        {
            self.counters.words_appended += 1;
        }
        Ok(())
    }

    /// Transfers a complete child sink chain to its parent sink in O(1).
    ///
    /// A completed nested scanner has no independent lifetime after a direct
    /// `\unexpanded` splice. Moving its chain preserves the one physical word
    /// owner; the emptied source coordinate cannot publish the words again.
    pub(super) fn append_sink(
        &mut self,
        destination: &mut TokenSink,
        source: &mut TokenSink,
    ) -> Result<(), AttemptError> {
        if source.len == 0 {
            if source.head != NO_CHUNK || source.tail != NO_CHUNK {
                return Err(AttemptError::InvalidCoordinate);
            }
            return Ok(());
        }
        if source.head == NO_CHUNK || source.tail == NO_CHUNK {
            return Err(AttemptError::InvalidCoordinate);
        }
        if self.chunks.get(source.head as usize).is_none()
            || self
                .chunks
                .get(source.tail as usize)
                .is_none_or(|tail| tail.next != NO_CHUNK)
        {
            return Err(AttemptError::InvalidCoordinate);
        }
        let next_len = destination
            .len
            .checked_add(source.len)
            .ok_or(AttemptError::CapacityOverflow)?;
        if destination.len == 0 {
            if destination.head != NO_CHUNK || destination.tail != NO_CHUNK {
                return Err(AttemptError::InvalidCoordinate);
            }
            *destination = *source;
        } else {
            if destination.head == NO_CHUNK || destination.tail == NO_CHUNK {
                return Err(AttemptError::InvalidCoordinate);
            }
            if self.chunks.get(destination.head as usize).is_none()
                || self
                    .chunks
                    .get(destination.tail as usize)
                    .is_none_or(|tail| tail.next != NO_CHUNK)
            {
                return Err(AttemptError::InvalidCoordinate);
            }
            self.chunks[destination.tail as usize].next = source.head;
            destination.tail = source.tail;
            destination.len = next_len;
        }
        *source = TokenSink::default();
        Ok(())
    }

    pub(super) fn view<'lane>(&'lane self, sink: &TokenSink) -> TokenLaneView<'lane> {
        TokenLaneView {
            lane: self,
            head: sink.head,
            len: sink.len,
        }
    }

    /// Returns a complete sink chain to the attempt's reusable high water.
    ///
    /// This visits chunks, not words. Clearing a `Vec<TracedTokenWord>` is a
    /// scalar length reset because the packed word is `Copy` and has no drop
    /// work.
    pub(super) fn release(&mut self, sink: TokenSink) -> Result<(), AttemptError> {
        if sink.head == NO_CHUNK {
            return if sink.tail == NO_CHUNK && sink.len == 0 {
                Ok(())
            } else {
                Err(AttemptError::InvalidCoordinate)
            };
        }
        let mut chunk = sink.head;
        loop {
            let current = self
                .chunks
                .get_mut(chunk as usize)
                .ok_or(AttemptError::InvalidCoordinate)?;
            current.words.clear();
            #[cfg(test)]
            {
                self.counters.chunks_released += 1;
            }
            if chunk == sink.tail {
                current.next = self.free_head;
                self.free_head = sink.head;
                return Ok(());
            }
            chunk = current.next;
            if chunk == NO_CHUNK {
                return Err(AttemptError::InvalidCoordinate);
            }
        }
    }

    fn allocate_chunk(&mut self) -> Result<u32, AttemptError> {
        if self.free_head != NO_CHUNK {
            let chunk = self.free_head;
            self.free_head = self.chunks[chunk as usize].next;
            self.chunks[chunk as usize].next = NO_CHUNK;
            #[cfg(test)]
            {
                self.counters.chunk_reuses += 1;
            }
            return Ok(chunk);
        }
        let chunk = u32::try_from(self.chunks.len()).map_err(|_| AttemptError::CapacityOverflow)?;
        self.chunks
            .try_reserve(1)
            .map_err(|_| AttemptError::AllocationFailed)?;
        self.chunks.push(TokenChunk::try_new()?);
        #[cfg(test)]
        {
            self.counters.chunk_allocations += 1;
        }
        Ok(chunk)
    }

    #[cfg(test)]
    pub(super) fn retained_chunks(&self) -> usize {
        self.chunks.len()
    }

    #[cfg(all(test, feature = "profiling"))]
    pub(super) const fn counters(&self) -> TokenLaneCounters {
        self.counters
    }
}

/// Borrowed immutable projection of one sink chain.
#[derive(Clone, Copy)]
pub(super) struct TokenLaneView<'lane> {
    lane: &'lane TokenLane,
    head: u32,
    len: u32,
}

impl<'lane> TokenLaneView<'lane> {
    pub(super) const fn len(self) -> usize {
        self.len as usize
    }

    pub(super) fn get(self, index: usize) -> Option<&'lane TracedTokenWord> {
        if index >= self.len() {
            return None;
        }
        let mut chunk = self.head;
        let mut remaining = index;
        loop {
            let current = self.lane.chunks.get(chunk as usize)?;
            if remaining < current.words.len() {
                return current.words.get(remaining);
            }
            remaining -= current.words.len();
            chunk = current.next;
            if chunk == NO_CHUNK {
                return None;
            }
        }
    }

    pub(super) fn iter(self) -> TokenLaneIter<'lane> {
        TokenLaneIter {
            lane: self.lane,
            chunk: self.head,
            offset: 0,
            remaining: self.len,
        }
    }
}

pub(super) struct TokenLaneIter<'lane> {
    lane: &'lane TokenLane,
    chunk: u32,
    offset: usize,
    remaining: u32,
}

impl<'lane> Iterator for TokenLaneIter<'lane> {
    type Item = &'lane TracedTokenWord;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        let current = self.lane.chunks.get(self.chunk as usize)?;
        let word = current.words.get(self.offset)?;
        self.remaining -= 1;
        self.offset += 1;
        if self.offset == current.words.len() && self.remaining != 0 {
            self.chunk = current.next;
            self.offset = 0;
        }
        Some(word)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.remaining as usize;
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for TokenLaneIter<'_> {}
