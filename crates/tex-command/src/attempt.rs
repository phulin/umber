//! Fixed-chunk packed-token scratch with consumer-owned sealed coordinates.
//!
//! Reading borrows a sealed root; its consumer releases the chunks and row.
//! Failed synchronous scanners discard their own serial-ordered suffix.

use core::marker::PhantomData;
use core::num::NonZeroU64;
use std::sync::atomic::{AtomicU64, Ordering};

use tex_state::token::TracedTokenWord;

mod token_lane;

use token_lane::{TokenLane, TokenLaneView, TokenSink};

#[cfg(test)]
#[path = "attempt/tests.rs"]
mod tests;

static NEXT_ATTEMPT_KEY: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AttemptKey(NonZeroU64);
impl AttemptKey {
    fn fresh() -> Self {
        loop {
            if let Some(key) = NonZeroU64::new(NEXT_ATTEMPT_KEY.fetch_add(1, Ordering::Relaxed)) {
                return Self(key);
            }
        }
    }
}
/// Move-only suffix boundary for a synchronous scanner child.
#[derive(Debug)]
#[must_use = "a child attempt scope must be closed"]
pub struct CommandAttemptChildScope {
    opening: AttemptMark,
}
macro_rules! attempt_id {
    ($name:ident) => {
        #[derive(Debug, Eq, Hash, PartialEq)]
        #[must_use = "scratch storage must be consumed or discarded by its owner"]
        pub struct $name {
            key: NonZeroU64,
            row: u32,
            serial: NonZeroU64,
        }
        impl $name {
            const fn index(&self) -> usize {
                self.row as usize
            }
        }
    };
}
attempt_id!(AttemptTokenListId);
attempt_id!(AttemptTokenBufferId);
#[cfg(test)]
impl AttemptTokenListId {
    pub(crate) fn test_alias(&self) -> Self {
        Self {
            key: self.key,
            row: self.row,
            serial: self.serial,
        }
    }
}
impl Copy for AttemptTokenBufferId {}
impl Clone for AttemptTokenBufferId {
    fn clone(&self) -> Self {
        *self
    }
}
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct AttemptMark {
    key: NonZeroU64,
    serial_frontier: u64,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttemptError {
    ForeignAttempt,
    InvalidCoordinate,
    CapacityOverflow,
    AllocationFailed,
}

/// Borrowed immutable words from the one scratch lane.
#[derive(Clone, Copy)]
pub struct AttemptTokenListView<'arena> {
    words: TokenLaneView<'arena>,
}
impl<'arena> AttemptTokenListView<'arena> {
    #[must_use]
    pub fn len(self) -> usize {
        self.words.len()
    }
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.len() == 0
    }
    #[must_use]
    pub fn get(self, index: usize) -> Option<&'arena TracedTokenWord> {
        self.words.get(index)
    }
    #[must_use]
    pub fn first(self) -> Option<&'arena TracedTokenWord> {
        self.get(0)
    }
    pub fn iter(self) -> AttemptTokenListIter<'arena> {
        AttemptTokenListIter {
            words: self.words.iter(),
        }
    }
    #[must_use]
    pub fn to_vec(self) -> Vec<TracedTokenWord> {
        self.iter().copied().collect()
    }
}
pub struct AttemptTokenListIter<'arena> {
    words: token_lane::TokenLaneIter<'arena>,
}
impl<'arena> Iterator for AttemptTokenListIter<'arena> {
    type Item = &'arena TracedTokenWord;
    fn next(&mut self) -> Option<Self::Item> {
        self.words.next()
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.words.size_hint()
    }
}
impl ExactSizeIterator for AttemptTokenListIter<'_> {}
impl<'arena> IntoIterator for AttemptTokenListView<'arena> {
    type Item = &'arena TracedTokenWord;
    type IntoIter = AttemptTokenListIter<'arena>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}
impl core::fmt::Debug for AttemptTokenListView<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.debug_list().entries(self.iter()).finish()
    }
}

impl PartialEq for AttemptTokenListView<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len() && self.iter().eq(other.iter())
    }
}

impl Eq for AttemptTokenListView<'_> {}

impl PartialEq<&[TracedTokenWord]> for AttemptTokenListView<'_> {
    fn eq(&self, other: &&[TracedTokenWord]) -> bool {
        self.len() == other.len() && self.iter().eq(other.iter())
    }
}

impl<const N: usize> PartialEq<&[TracedTokenWord; N]> for AttemptTokenListView<'_> {
    fn eq(&self, other: &&[TracedTokenWord; N]) -> bool {
        self.len() == N && self.iter().eq(other.iter())
    }
}

impl PartialEq<AttemptTokenListView<'_>> for &[TracedTokenWord] {
    fn eq(&self, other: &AttemptTokenListView<'_>) -> bool {
        other == self
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TokenPhase {
    Open,
    Finished,
    Consumed,
}
#[derive(Debug, Eq, PartialEq)]
struct TokenRow {
    serial: NonZeroU64,
    sink: TokenSink,
    phase: TokenPhase,
}
/// A single row owns each collector and its final list; sealing changes only phase.
pub(crate) struct AttemptArena<G> {
    key: AttemptKey,
    next_serial: u64,
    rows: Vec<TokenRow>,
    token_lane: TokenLane,
    _generation: PhantomData<fn(&G) -> &G>,
}
impl<G> core::fmt::Debug for AttemptArena<G> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("AttemptArena(..)")
    }
}
impl<G> Default for AttemptArena<G> {
    fn default() -> Self {
        Self {
            key: AttemptKey::fresh(),
            next_serial: 1,
            rows: Vec::new(),
            token_lane: TokenLane::default(),
            _generation: PhantomData,
        }
    }
}
impl<G> AttemptArena<G> {
    fn allocate_serial(&mut self) -> Result<NonZeroU64, AttemptError> {
        let serial = NonZeroU64::new(self.next_serial).ok_or(AttemptError::CapacityOverflow)?;
        self.next_serial = self
            .next_serial
            .checked_add(1)
            .ok_or(AttemptError::CapacityOverflow)?;
        Ok(serial)
    }
    #[must_use]
    pub(crate) fn mark(&self) -> AttemptMark {
        AttemptMark {
            key: self.key.0,
            serial_frontier: self.next_serial,
        }
    }
    pub(crate) fn validate_mark(&self, mark: AttemptMark) -> Result<(), AttemptError> {
        self.validate_key(mark.key)?;
        if mark.serial_frontier > self.next_serial {
            return Err(AttemptError::InvalidCoordinate);
        }
        Ok(())
    }
    pub(crate) fn clear(&mut self) {
        self.truncate(AttemptMark {
            key: self.key.0,
            serial_frontier: 1,
        })
        .expect("empty mark is a valid suffix");
    }
    #[inline(always)]
    pub(crate) fn is_clear(&self) -> bool {
        self.rows.is_empty()
    }
    #[inline(always)]
    pub(crate) fn begin_child_scope(&self) -> CommandAttemptChildScope {
        CommandAttemptChildScope {
            opening: self.mark(),
        }
    }
    pub(crate) fn close_child_scope(
        &mut self,
        scope: CommandAttemptChildScope,
    ) -> Result<(), AttemptError> {
        self.truncate(scope.opening)
    }
    pub(crate) fn truncate(&mut self, mark: AttemptMark) -> Result<(), AttemptError> {
        self.validate_mark(mark)?;
        while self
            .rows
            .last()
            .is_some_and(|row| row.serial.get() >= mark.serial_frontier)
        {
            let row = self.rows.pop().expect("nonempty scratch suffix");
            self.token_lane.release(row.sink)?;
        }
        self.retire_consumed_suffix();
        Ok(())
    }
    pub(crate) fn allocate_token_list(
        &mut self,
        words: impl IntoIterator<Item = TracedTokenWord>,
    ) -> Result<AttemptTokenListId, AttemptError> {
        let mark = self.mark();
        let result = (|| {
            let buffer = self.allocate_token_buffer()?;
            for word in words {
                self.push_buffer_token(&buffer, word)?;
            }
            self.finish_token_buffer(buffer)
        })();
        if result.is_err() {
            self.truncate(mark).expect("allocation-local mark is valid");
        }
        result
    }
    pub(crate) fn token_words(
        &self,
        id: &AttemptTokenListId,
    ) -> Result<AttemptTokenListView<'_>, AttemptError> {
        self.validate_key(id.key)?;
        let row = self
            .rows
            .get(id.index())
            .filter(|row| row.serial == id.serial && row.phase == TokenPhase::Finished)
            .ok_or(AttemptError::InvalidCoordinate)?;
        Ok(AttemptTokenListView {
            words: self.token_lane.view(&row.sink),
        })
    }
    pub(crate) fn allocate_token_buffer(&mut self) -> Result<AttemptTokenBufferId, AttemptError> {
        #[cfg(feature = "profiling")]
        let _allocation_scope = tex_state::measurement::hot_core_allocation_scope(
            tex_state::measurement::HotCoreAllocationOwner::AttemptScratch,
        );
        let serial = self.allocate_serial()?;
        let id = AttemptTokenBufferId {
            key: self.key.0,
            row: u32::try_from(self.rows.len()).map_err(|_| AttemptError::CapacityOverflow)?,
            serial,
        };
        self.rows
            .try_reserve(1)
            .map_err(|_| AttemptError::AllocationFailed)?;
        self.rows.push(TokenRow {
            serial,
            sink: TokenSink::default(),
            phase: TokenPhase::Open,
        });
        Ok(id)
    }
    pub(crate) fn discard_token_buffer(
        &mut self,
        id: AttemptTokenBufferId,
    ) -> Result<(), AttemptError> {
        self.validate_key(id.key)?;
        let row = self
            .rows
            .get_mut(id.index())
            .filter(|row| row.serial == id.serial && row.phase == TokenPhase::Open)
            .ok_or(AttemptError::InvalidCoordinate)?;
        let sink = core::mem::take(&mut row.sink);
        row.phase = TokenPhase::Consumed;
        self.token_lane.release(sink)?;
        self.retire_consumed_suffix();
        Ok(())
    }

    pub(crate) fn token_buffer(
        &self,
        id: &AttemptTokenBufferId,
    ) -> Result<AttemptTokenListView<'_>, AttemptError> {
        self.validate_key(id.key)?;
        let row = self
            .rows
            .get(id.index())
            .filter(|row| row.serial == id.serial && row.phase != TokenPhase::Consumed)
            .ok_or(AttemptError::InvalidCoordinate)?;
        Ok(AttemptTokenListView {
            words: self.token_lane.view(&row.sink),
        })
    }
    pub(crate) fn push_buffer_token(
        &mut self,
        id: &AttemptTokenBufferId,
        word: TracedTokenWord,
    ) -> Result<(), AttemptError> {
        #[cfg(feature = "profiling")]
        let _allocation_scope = tex_state::measurement::hot_core_allocation_scope(
            tex_state::measurement::HotCoreAllocationOwner::AttemptScratch,
        );
        self.validate_key(id.key)?;
        let row = self
            .rows
            .get_mut(id.index())
            .filter(|row| row.serial == id.serial && row.phase == TokenPhase::Open)
            .ok_or(AttemptError::InvalidCoordinate)?;
        self.token_lane.push(&mut row.sink, word)
    }
    /// Consumes a sealed child into an open parent, transferring whole chunks.
    pub(crate) fn consume_token_list_into_buffer(
        &mut self,
        source: AttemptTokenListId,
        destination: &AttemptTokenBufferId,
    ) -> Result<u32, AttemptError> {
        self.validate_key(source.key)?;
        self.validate_key(destination.key)?;
        if source.index() == destination.index() {
            return Err(AttemptError::InvalidCoordinate);
        }
        self.rows
            .get(source.index())
            .filter(|row| row.serial == source.serial && row.phase == TokenPhase::Finished)
            .ok_or(AttemptError::InvalidCoordinate)?;
        self.rows
            .get(destination.index())
            .filter(|row| row.serial == destination.serial && row.phase == TokenPhase::Open)
            .ok_or(AttemptError::InvalidCoordinate)?;
        let (source_row, destination_row) = if source.index() < destination.index() {
            let (before, after) = self.rows.split_at_mut(destination.index());
            (&mut before[source.index()], &mut after[0])
        } else {
            let (before, after) = self.rows.split_at_mut(source.index());
            (&mut after[0], &mut before[destination.index()])
        };
        let moved = source_row.sink.len();
        self.token_lane
            .append_sink(&mut destination_row.sink, &mut source_row.sink)?;
        source_row.phase = TokenPhase::Consumed;
        self.retire_consumed_suffix();
        Ok(moved)
    }
    pub(crate) fn finish_token_buffer(
        &mut self,
        id: AttemptTokenBufferId,
    ) -> Result<AttemptTokenListId, AttemptError> {
        self.validate_key(id.key)?;
        let row = self
            .rows
            .get_mut(id.index())
            .filter(|row| row.serial == id.serial && row.phase == TokenPhase::Open)
            .ok_or(AttemptError::InvalidCoordinate)?;
        row.phase = TokenPhase::Finished;
        Ok(AttemptTokenListId {
            key: id.key,
            row: id.row,
            serial: id.serial,
        })
    }
    /// Publication, insertion and rendering release their actual sealed owner.
    pub(crate) fn discard_token_list(
        &mut self,
        id: AttemptTokenListId,
    ) -> Result<(), AttemptError> {
        self.validate_key(id.key)?;
        let row = self
            .rows
            .get_mut(id.index())
            .filter(|row| row.serial == id.serial && row.phase == TokenPhase::Finished)
            .ok_or(AttemptError::InvalidCoordinate)?;
        let sink = std::mem::take(&mut row.sink);
        row.phase = TokenPhase::Consumed;
        self.token_lane.release(sink)?;
        self.retire_consumed_suffix();
        Ok(())
    }

    fn retire_consumed_suffix(&mut self) {
        while self
            .rows
            .last()
            .is_some_and(|row| row.phase == TokenPhase::Consumed)
        {
            self.rows.pop();
        }
    }
    fn validate_key(&self, key: NonZeroU64) -> Result<(), AttemptError> {
        if key == self.key.0 {
            Ok(())
        } else {
            Err(AttemptError::ForeignAttempt)
        }
    }
}
