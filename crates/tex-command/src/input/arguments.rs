//! Final invocation-owned macro argument storage (TeX82 §§392–400).
//!
//! Every simultaneous invocation owns one reusable region vector. Parameter
//! rows borrow it; body retirement returns it to this allocator pool. Matching
//! writes the final words directly, independently of ordinary backup input.

use super::ArgumentSpan;
use crate::execution_scratch::{ArgumentRunAdmission, PlainRunSink, ScratchError};
use core::hash::{Hash, Hasher};
use core::num::NonZeroU32;
use tex_state::token::{TokenWord, TracedTokenWord};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct ArgumentRegionId(NonZeroU32);

impl ArgumentRegionId {
    fn index(self) -> usize {
        self.0.get() as usize - 1
    }
}

/// Move-only capture owner; final ranges already reside beside the words.
/// Neither this owner nor a matcher cursor enters a snapshot.
#[derive(Debug)]
pub(crate) struct MatchedArguments {
    region: ArgumentRegionId,
}

#[derive(Debug, Default)]
struct Region {
    words: Vec<TracedTokenWord>,
    spans: [ArgumentSpan; 9],
    count: u8,
    occupied: bool,
}

#[derive(Debug, Default)]
pub(crate) struct ArgumentArena {
    regions: Vec<Region>,
    free: Vec<ArgumentRegionId>,
    capture: Option<ArgumentRegionId>,
}

impl PartialEq for ArgumentArena {
    fn eq(&self, other: &Self) -> bool {
        self.capture == other.capture && self.live_regions().eq(other.live_regions())
    }
}
impl Eq for ArgumentArena {}
impl Hash for ArgumentArena {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.capture.hash(state);
        for (index, words, spans) in self.live_regions() {
            index.hash(state);
            words.hash(state);
            spans.hash(state);
        }
    }
}

pub(crate) type MacroWords<'a> = core::iter::Copied<core::slice::Iter<'a, TracedTokenWord>>;

impl ArgumentArena {
    fn live_regions(&self) -> impl Iterator<Item = (usize, &[TracedTokenWord], &[ArgumentSpan])> {
        self.regions
            .iter()
            .enumerate()
            .filter(|(_, region)| region.occupied)
            .map(|(index, region)| {
                (
                    index,
                    region.words.as_slice(),
                    &region.spans[..usize::from(region.count)],
                )
            })
    }

    pub(crate) fn begin_match(&mut self) -> Result<MatchedArguments, ScratchError> {
        debug_assert!(self.capture.is_none(), "raw matching never expands a macro");
        let region = if let Some(region) = self.free.pop() {
            region
        } else {
            let index = self.regions.len();
            let id = u32::try_from(index)
                .ok()
                .and_then(|index| index.checked_add(1))
                .and_then(NonZeroU32::new)
                .ok_or(ScratchError::CapacityOverflow)?;
            self.regions
                .try_reserve(1)
                .map_err(|_| ScratchError::AllocationFailed)?;
            // Reserve the return path before admitting this owner.
            self.free
                .try_reserve(index + 1 - self.free.len())
                .map_err(|_| ScratchError::AllocationFailed)?;
            self.regions.push(Region::default());
            ArgumentRegionId(id)
        };
        self.regions[region.index()].occupied = true;
        self.regions[region.index()].count = 0;
        self.capture = Some(region);
        Ok(MatchedArguments { region })
    }

    pub(crate) fn begin_argument(&self, matching: &MatchedArguments) -> ArgumentCursor {
        debug_assert!(self.regions[matching.region.index()].count < 9);
        let start = self.tail(matching.region);
        ArgumentCursor {
            region: matching.region,
            start,
            visible_end: start,
            cursor: crate::scanner_kernel::ScannerCursor::default(),
            end_trim: 0,
            holdback_start: start,
            holdback_len: 0,
        }
    }

    /// Transfers the final region's sole owner; words and ranges stay in place.
    pub(crate) fn publish(&mut self, matching: MatchedArguments) -> ArgumentRegionId {
        debug_assert_eq!(self.capture, Some(matching.region));
        self.capture = None;
        matching.region
    }

    /// Native capacity bookkeeping reads the count, without constructing a span view.
    #[inline(always)]
    pub(crate) fn argument_count(&self, id: ArgumentRegionId) -> usize {
        usize::from(self.regions[id.index()].count)
    }

    #[inline(always)]
    pub(crate) fn spans(&self, id: ArgumentRegionId) -> &[ArgumentSpan] {
        let region = &self.regions[id.index()];
        &region.spans[..usize::from(region.count)]
    }

    pub(crate) fn abort_capture(&mut self) {
        if let Some(region) = self.capture.take() {
            self.release(region);
        }
    }

    pub(crate) fn release(&mut self, id: ArgumentRegionId) {
        let region = &mut self.regions[id.index()];
        debug_assert!(region.occupied);
        region.words.clear();
        region.occupied = false;
        self.free.push(id);
    }

    /// Checkpoint install abandons all regions without copying the allocator pool.
    pub(crate) fn clear(&mut self) {
        self.capture = None;
        self.free.clear();
        for (index, region) in self.regions.iter_mut().enumerate() {
            region.words.clear();
            region.occupied = false;
            self.free.push(ArgumentRegionId(
                NonZeroU32::new(index as u32 + 1).expect("region index"),
            ));
        }
    }

    #[cfg(test)]
    pub(crate) fn pool_counts(&self) -> (usize, usize, usize) {
        (
            self.regions.len(),
            self.live_regions().count(),
            self.retained_bytes(),
        )
    }

    pub(crate) fn is_matching(&self) -> bool {
        self.capture.is_some()
    }
    pub(crate) fn captured_word_len(&self) -> usize {
        self.capture.map_or(0, |id| self.region(id).len())
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.live_regions().next().is_none()
    }
    pub(crate) fn region(&self, id: ArgumentRegionId) -> &[TracedTokenWord] {
        &self.regions[id.index()].words
    }
    fn region_mut(&mut self, id: ArgumentRegionId) -> &mut Vec<TracedTokenWord> {
        &mut self.regions[id.index()].words
    }
    fn tail(&self, id: ArgumentRegionId) -> u32 {
        self.region(id).len() as u32
    }
    pub(crate) fn retained_bytes(&self) -> usize {
        self.regions.capacity() * core::mem::size_of::<Region>()
            + self.free.capacity() * core::mem::size_of::<ArgumentRegionId>()
            + self
                .regions
                .iter()
                .map(|region| region.words.capacity() * core::mem::size_of::<TracedTokenWord>())
                .sum::<usize>()
    }
    /// Appends final spelling directly to the pending argument.
    #[inline(always)]
    pub(crate) fn append_argument_word(
        &mut self,
        writer: &mut ArgumentCursor,
        spelling: TracedTokenWord,
    ) -> Result<u32, ScratchError> {
        if writer.holdback_len != 0 {
            return Err(ScratchError::InvalidCoordinate);
        }
        self.region_mut(writer.region).push(spelling);
        writer.visible_end = self.tail(writer.region);
        Ok(writer.cursor.settle_argument_word(spelling.token_word()))
    }

    /// Appends an unpublished delimiter candidate after the visible argument
    /// prefix. Its scanner facts stay untouched until a mismatch reveals it.
    #[inline(always)]
    pub(crate) fn append_delimiter_word(
        &mut self,
        writer: &mut ArgumentCursor,
        spelling: TracedTokenWord,
    ) -> Result<(), ScratchError> {
        if writer.holdback_len == 0 {
            writer.holdback_start = writer.visible_end;
        }
        self.region_mut(writer.region).push(spelling);
        writer.holdback_len += 1;
        Ok(())
    }

    /// Commits the first `count` words of the unpublished delimiter suffix.
    /// The words are already in place; only the scanner cursor and the
    /// visible end advance.
    pub(crate) fn reveal_delimiter_words(
        &mut self,
        writer: &mut ArgumentCursor,
        count: u32,
    ) -> Result<(), ScratchError> {
        if count > writer.holdback_len {
            return Err(ScratchError::InvalidCoordinate);
        }
        let end = writer.holdback_start + count;
        let revealed = self
            .region(writer.region)
            .get(writer.visible_end as usize..end as usize)
            .ok_or(ScratchError::InvalidCoordinate)?;
        for spelling in revealed {
            // A delimiter prefix is deliberately not reclassified as a
            // paragraph after a mismatch. This preserves TeX82's historical
            // `paragraph delimiter prefix` fact: it contributes brace/group
            // state when revealed, but the non-long paragraph check applies
            // only to the newly ordinary token.
            writer.cursor.settle_argument_word(spelling.token_word());
        }
        writer.visible_end = end;
        writer.holdback_start = end;
        writer.holdback_len -= count;
        Ok(())
    }

    /// Drops a completed delimiter from the tail of the match words.
    pub(crate) fn truncate_delimiter_suffix(
        &mut self,
        writer: &mut ArgumentCursor,
    ) -> Result<(), ScratchError> {
        if writer.holdback_start + writer.holdback_len != self.tail(writer.region) {
            return Err(ScratchError::InvalidCoordinate);
        }
        self.region_mut(writer.region)
            .truncate(writer.holdback_start as usize);
        writer.visible_end = writer.holdback_start;
        writer.holdback_len = 0;
        Ok(())
    }

    pub(crate) fn match_words(
        &self,
        writer: &ArgumentCursor,
    ) -> Result<MacroWords<'_>, ScratchError> {
        let (start, end) = writer.range()?;
        self.region(writer.region)
            .get(start as usize..end as usize)
            .map(|words| words.iter().copied())
            .ok_or(ScratchError::InvalidCoordinate)
    }

    pub(crate) fn publish_argument(
        &mut self,
        matching: &mut MatchedArguments,
        writer: ArgumentCursor,
    ) -> Result<(), ScratchError> {
        if writer.region != matching.region
            || self.regions[matching.region.index()].count >= 9
            || writer.holdback_len != 0
            || writer.visible_end != self.tail(writer.region)
        {
            return Err(ScratchError::InvalidCoordinate);
        }
        let (start, end) = writer.range()?;
        let region = &mut self.regions[matching.region.index()];
        region.spans[usize::from(region.count)] = ArgumentSpan {
            start,
            len: end - start,
        };
        region.count += 1;
        Ok(())
    }

    pub(crate) fn delimiter_prefix_len(
        &self,
        writer: &ArgumentCursor,
    ) -> Result<usize, ScratchError> {
        Ok(writer.holdback_len as usize)
    }

    pub(crate) fn delimiter_prefix_word(
        &self,
        writer: &ArgumentCursor,
        index: usize,
    ) -> Result<TracedTokenWord, ScratchError> {
        self.delimiter_prefix(writer)?
            .get(index)
            .copied()
            .ok_or(ScratchError::InvalidCoordinate)
    }

    pub(crate) fn delimiter_prefix_words<'a>(
        &'a self,
        writer: &'a ArgumentCursor,
    ) -> Result<MacroWords<'a>, ScratchError> {
        self.delimiter_prefix(writer)
            .map(|words| words.iter().copied())
    }

    fn delimiter_prefix(
        &self,
        writer: &ArgumentCursor,
    ) -> Result<&[TracedTokenWord], ScratchError> {
        let start = writer.holdback_start as usize;
        self.region(writer.region)
            .get(start..start + writer.holdback_len as usize)
            .ok_or(ScratchError::InvalidCoordinate)
    }

    /// Source rows borrow a body-owned region; capture owns a distinct slot.
    #[inline(always)]
    fn consume_region<G>(
        &mut self,
        writer: &mut ArgumentCursor,
        source: ArgumentRegionId,
        range: core::ops::Range<usize>,
        admission: &mut ArgumentRunAdmission<'_, '_, G>,
    ) -> Result<usize, ScratchError> {
        debug_assert_ne!(source, writer.region);
        debug_assert_eq!(writer.holdback_len, 0);
        let destination = writer.region.index();
        let source_index = source.index();
        let (from, to) = if source_index < destination {
            let (lower, upper) = self.regions.split_at_mut(destination);
            (&lower[source_index].words, &mut upper[0].words)
        } else {
            let (lower, upper) = self.regions.split_at_mut(source_index);
            (&upper[0].words, &mut lower[destination].words)
        };
        let words = &from[range];
        let start = to.len();
        for &word in words {
            if !admission.admits(word.token_word(), writer) {
                break;
            }
            to.push(word);
        }
        writer.visible_end = to.len() as u32;
        Ok(to.len() - start)
    }
}

impl<G> PlainRunSink<G> for ArgumentCursor {
    #[inline(always)]
    fn brace_depth(&self) -> u32 {
        self.cursor.brace_depth()
    }

    #[inline(always)]
    fn settle_admitted_word(&mut self, brace_delta: i8) {
        self.cursor.settle_argument_delta(brace_delta);
    }
    #[inline(always)]
    fn consume_region_span(
        &mut self,
        arena: &mut ArgumentArena,
        source: ArgumentRegionId,
        range: core::ops::Range<usize>,
        admission: &mut ArgumentRunAdmission<'_, '_, G>,
    ) -> Result<usize, ScratchError> {
        arena.consume_region(self, source, range, admission)
    }
    #[inline(always)]
    fn consume_token_span(
        &mut self,
        arena: &mut ArgumentArena,
        words: &[TokenWord],
        admission: &mut ArgumentRunAdmission<'_, '_, G>,
    ) -> Result<usize, ScratchError> {
        if self.holdback_len != 0 {
            return Err(ScratchError::InvalidCoordinate);
        }
        let destination = arena.region_mut(self.region);
        let start = destination.len();
        for &word in words {
            if !admission.admits(word, self) {
                break;
            }
            destination.push(TracedTokenWord::from_parts(
                word,
                tex_state::token::OriginId::UNKNOWN,
            ));
        }
        self.visible_end = destination.len() as u32;
        Ok(destination.len() - start)
    }
}

#[cfg(test)]
mod tests;
/// Stack-local writer for one pending macro argument.
///
/// The argument occupies `start..visible_end` of the match words, less
/// `end_trim` trailing words. A delimiter candidate is already appended after
/// `visible_end` and stays unpublished until it is revealed or truncated.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct ArgumentCursor {
    region: ArgumentRegionId,
    start: u32,
    visible_end: u32,
    cursor: crate::scanner_kernel::ScannerCursor,
    end_trim: u8,
    holdback_start: u32,
    holdback_len: u32,
}

impl ArgumentCursor {
    pub(crate) const fn brace_depth(&self) -> u32 {
        self.cursor.brace_depth()
    }

    pub(crate) const fn removable_outer_group(&self) -> bool {
        self.cursor.removable_outer_group()
    }

    fn range(&self) -> Result<(u32, u32), ScratchError> {
        self.visible_end
            .checked_sub(u32::from(self.end_trim))
            .filter(|&end| end >= self.start)
            .map(|end| (self.start, end))
            .ok_or(ScratchError::InvalidCoordinate)
    }

    pub(crate) fn visible_len(&self) -> Result<usize, ScratchError> {
        self.range().map(|(start, end)| (end - start) as usize)
    }

    pub(crate) fn strip_outer_group(&mut self) -> Result<(), ScratchError> {
        if self.visible_len()? < 2 {
            return Err(ScratchError::InvalidCoordinate);
        }
        self.start += 1;
        self.end_trim = self
            .end_trim
            .checked_add(1)
            .ok_or(ScratchError::CapacityOverflow)?;
        Ok(())
    }

    #[cfg(test)]
    fn set_end_trim(&mut self, trim: u8) {
        self.end_trim = trim;
    }
}
