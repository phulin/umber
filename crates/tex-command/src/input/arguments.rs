//! Final invocation-owned macro argument storage (TeX82 §§392–400).
//!
//! Every simultaneous invocation owns one reusable region vector. Parameter
//! rows borrow it; body retirement returns it to this allocator pool. Matching
//! writes the final words directly, independently of ordinary backup input.

use super::ArgumentSpan;
use crate::command::HotCommand;
use crate::execution_scratch::{PlainRunSink, ScratchError};
#[cfg(test)]
use crate::token_collector::ClassifiedToken;
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

/// Call-local ranges. Neither this value nor a matcher cursor enters a snapshot.
#[derive(Debug)]
pub(crate) struct MatchedArguments {
    region: ArgumentRegionId,
    spans: [ArgumentSpan; 9],
    count: u8,
}

#[derive(Debug, Default)]
struct Region {
    words: Vec<TracedTokenWord>,
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
        for (index, words) in self.live_regions() {
            index.hash(state);
            words.hash(state);
        }
    }
}

pub(crate) type MacroWords<'a> = core::iter::Copied<core::slice::Iter<'a, TracedTokenWord>>;

impl ArgumentArena {
    fn live_regions(&self) -> impl Iterator<Item = (usize, &[TracedTokenWord])> {
        self.regions
            .iter()
            .enumerate()
            .filter(|(_, region)| region.occupied)
            .map(|(index, region)| (index, region.words.as_slice()))
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
        self.capture = Some(region);
        Ok(MatchedArguments {
            region,
            spans: [ArgumentSpan::default(); 9],
            count: 0,
        })
    }

    pub(crate) fn begin_argument(&self, matching: &MatchedArguments) -> ArgumentCursor {
        debug_assert!(matching.count < 9);
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

    /// Publishes coordinates only; the body assumes the region's sole ownership.
    pub(crate) fn publish(
        &mut self,
        matching: MatchedArguments,
        parameters: &mut Vec<ArgumentSpan>,
    ) -> Result<ArgumentRegionId, ScratchError> {
        let count = usize::from(matching.count);
        parameters
            .try_reserve(count)
            .map_err(|_| ScratchError::AllocationFailed)?;
        parameters.extend_from_slice(&matching.spans[..count]);
        debug_assert_eq!(self.capture, Some(matching.region));
        self.capture = None;
        Ok(matching.region)
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
    /// Settles one classified token: the append and the paragraph,
    /// brace-depth, and removable-outer-group facts from one classification.
    #[cfg(test)]
    pub(crate) fn append_argument_token(
        &mut self,
        writer: &mut ArgumentCursor,
        token: ClassifiedToken,
        paragraph_checked: bool,
    ) -> Result<u32, ScratchError> {
        if writer.holdback_len != 0 {
            return Err(ScratchError::InvalidCoordinate);
        }
        self.region_mut(writer.region).push(token.word());
        writer.visible_end = self.tail(writer.region);
        Ok(writer.cursor.settle_argument(token, paragraph_checked))
    }

    /// Appends one compact raw delivery directly to the pending argument.
    #[inline(always)]
    pub(crate) fn append_hot_delivery(
        &mut self,
        writer: &mut ArgumentCursor,
        delivery: &HotCommand<impl Sized>,
        paragraph_checked: bool,
        paragraph_token: Option<TokenWord>,
    ) -> Result<u32, ScratchError> {
        if writer.holdback_len != 0 {
            return Err(ScratchError::InvalidCoordinate);
        }
        let spelling = delivery.spelling();
        debug_assert_eq!(spelling.origin(), delivery.origin());
        self.region_mut(writer.region).push(spelling);
        writer.visible_end = self.tail(writer.region);
        Ok(writer.cursor.settle_argument_word(
            delivery.spelling_word(),
            paragraph_checked && delivery.is_paragraph_spelling(paragraph_token),
        ))
    }

    pub(crate) fn append_plain_argument_tokens(
        &mut self,
        writer: &mut ArgumentCursor,
        words: &[TokenWord],
    ) -> Result<(), ScratchError> {
        if writer.holdback_len != 0 {
            return Err(ScratchError::InvalidCoordinate);
        }
        self.region_mut(writer.region).extend(
            words.iter().map(|&word| {
                TracedTokenWord::from_parts(word, tex_state::token::OriginId::UNKNOWN)
            }),
        );
        for &word in words {
            writer.cursor.settle_argument_word(word, false);
        }
        writer.visible_end = self.tail(writer.region);
        Ok(())
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
            writer
                .cursor
                .settle_argument_word(spelling.token_word(), false);
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
        &self,
        matching: &mut MatchedArguments,
        writer: ArgumentCursor,
    ) -> Result<(), ScratchError> {
        if writer.region != matching.region
            || matching.count >= 9
            || writer.holdback_len != 0
            || writer.visible_end != self.tail(writer.region)
        {
            return Err(ScratchError::InvalidCoordinate);
        }
        let (start, end) = writer.range()?;
        matching.spans[usize::from(matching.count)] = ArgumentSpan {
            start,
            len: end - start,
        };
        matching.count += 1;
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
    fn append_region(
        &mut self,
        writer: &mut ArgumentCursor,
        source: ArgumentRegionId,
        range: core::ops::Range<usize>,
    ) -> Result<(), ScratchError> {
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
        to.extend_from_slice(words);
        for word in words {
            writer.cursor.settle_argument_word(word.token_word(), false);
        }
        writer.visible_end = to.len() as u32;
        Ok(())
    }
}

impl<G> PlainRunSink<G> for ArgumentCursor {
    fn append_region_span(
        &mut self,
        arena: &mut ArgumentArena,
        source: ArgumentRegionId,
        range: core::ops::Range<usize>,
    ) -> Result<(), ScratchError> {
        arena.append_region(self, source, range)
    }
    fn append_token_span(
        &mut self,
        arena: &mut ArgumentArena,
        words: &[TokenWord],
    ) -> Result<(), ScratchError> {
        arena.append_plain_argument_tokens(self, words)
    }
}

#[cfg(test)]
mod tests;
/// Exact TeX82 §394 facts established while one argument is collected.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct MacroArgumentFacts {
    rejects_non_long_paragraph: bool,
    removable_outer_group: bool,
}

impl MacroArgumentFacts {
    #[cfg(test)]
    pub(crate) const fn rejects_non_long_paragraph(self) -> bool {
        self.rejects_non_long_paragraph
    }

    pub(crate) const fn removable_outer_group(self) -> bool {
        self.removable_outer_group
    }
}

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

    pub(crate) const fn facts(&self) -> MacroArgumentFacts {
        MacroArgumentFacts {
            rejects_non_long_paragraph: self.cursor.rejects_non_long_paragraph(),
            removable_outer_group: self.cursor.removable_outer_group(),
        }
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
