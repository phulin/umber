//! Final invocation-owned macro arguments: retained spans or pooled spelling.
use crate::execution_scratch::{ArgumentRunAdmission, PlainRunSink, ScratchError};
use core::hash::{Hash, Hasher};
use core::num::NonZeroU32;
use tex_state::token::TracedTokenWord;
mod backing;
mod words;
pub(crate) use words::{ArgumentValue, ArgumentWords, MacroWords};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct ArgumentRegionId(NonZeroU32);
impl ArgumentRegionId {
    fn index(self) -> usize {
        self.0.get() as usize - 1
    }
}
#[derive(Debug)]
pub(crate) struct MatchedArguments {
    region: ArgumentRegionId,
}
#[derive(Debug)]
struct Region<G> {
    words: Vec<TracedTokenWord>,
    values: [ArgumentValue; 9],
    sources: Vec<Option<backing::RetainedSource<G>>>,
    source_slots: u16,
    count: u8,
    roots: u32,
    published_raw_words: usize,
}
impl<G> Default for Region<G> {
    fn default() -> Self {
        Self {
            words: Vec::new(),
            values: [ArgumentValue::default(); 9],
            sources: Vec::new(),
            source_slots: 0,
            count: 0,
            roots: 0,
            published_raw_words: 0,
        }
    }
}
#[derive(Debug)]
pub(crate) struct ArgumentArena<G> {
    regions: Vec<Region<G>>,
    free: Vec<ArgumentRegionId>,
    capture: Option<ArgumentRegionId>,
}
impl<G> Default for ArgumentArena<G> {
    fn default() -> Self {
        Self {
            regions: Vec::new(),
            free: Vec::new(),
            capture: None,
        }
    }
}
impl<G> PartialEq for ArgumentArena<G> {
    fn eq(&self, other: &Self) -> bool {
        self.capture == other.capture
            && self
                .regions
                .iter()
                .enumerate()
                .filter(|(_, r)| r.roots != 0)
                .map(|(i, _)| i)
                .eq(other
                    .regions
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| r.roots != 0)
                    .map(|(i, _)| i))
            && self
                .regions
                .iter()
                .zip(&other.regions)
                .enumerate()
                .all(|(index, (a, b))| {
                    if (a.roots != 0) != (b.roots != 0) {
                        return false;
                    }
                    if a.roots == 0 {
                        return true;
                    }
                    let id = ArgumentRegionId(NonZeroU32::new(index as u32 + 1).expect("region"));
                    a.count == b.count
                        && a.published_raw_words == b.published_raw_words
                        && (0..a.count + u8::from(self.capture == Some(id) && a.count < 9)).all(
                            |slot| self.words(id, slot).iter().eq(other.words(id, slot).iter()),
                        )
                })
    }
}
impl<G> Eq for ArgumentArena<G> {}
impl<G> Hash for ArgumentArena<G> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.capture.hash(state);
        for (index, region) in self
            .regions
            .iter()
            .enumerate()
            .filter(|(_, r)| r.roots != 0)
        {
            index.hash(state);
            region.count.hash(state);
            region.published_raw_words.hash(state);
            let id = ArgumentRegionId(NonZeroU32::new(index as u32 + 1).expect("region"));
            for slot in 0..region.count + u8::from(self.capture == Some(id) && region.count < 9) {
                for word in self.words(id, slot).iter() {
                    word.hash(state);
                }
            }
        }
    }
}
impl<G> ArgumentArena<G> {
    pub(crate) fn begin_match(&mut self) -> Result<MatchedArguments, ScratchError> {
        debug_assert!(self.capture.is_none(), "raw matching never expands a macro");
        let region = if let Some(region) = self.free.pop() {
            region
        } else {
            let index = self.regions.len();
            let id = u32::try_from(index)
                .ok()
                .and_then(|i| i.checked_add(1))
                .and_then(NonZeroU32::new)
                .ok_or(ScratchError::CapacityOverflow)?;
            self.regions
                .try_reserve(1)
                .map_err(|_| ScratchError::AllocationFailed)?;
            self.free
                .try_reserve(index + 1 - self.free.len())
                .map_err(|_| ScratchError::AllocationFailed)?;
            self.regions.push(Region::default());
            ArgumentRegionId(id)
        };
        let value = &mut self.regions[region.index()];
        debug_assert_eq!(value.count, 0);
        debug_assert_eq!(value.published_raw_words, 0);
        value.roots = 1;
        value.values[0] = ArgumentValue::default();
        self.capture = Some(region);
        Ok(MatchedArguments { region })
    }
    pub(crate) fn begin_argument(&mut self, matching: &MatchedArguments) -> ArgumentCursor {
        let region = &mut self.regions[matching.region.index()];
        debug_assert!(region.count < 9);
        region.values[usize::from(region.count)] = ArgumentValue {
            start: region.words.len() as u32,
            len: 0,
        };
        ArgumentCursor {
            region: matching.region,
            start: 0,
            visible_end: 0,
            cursor: crate::scanner_kernel::ScannerCursor::default(),
            end_trim: 0,
            holdback_start: 0,
            holdback_len: 0,
        }
    }
    pub(crate) fn publish(&mut self, matching: MatchedArguments) -> ArgumentRegionId {
        debug_assert_eq!(self.capture, Some(matching.region));
        self.capture = None;
        matching.region
    }
    #[inline(always)]
    pub(crate) fn argument_count(&self, id: ArgumentRegionId) -> usize {
        usize::from(self.regions[id.index()].count)
    }
    pub(crate) fn values(&self, id: ArgumentRegionId) -> &[ArgumentValue] {
        let r = &self.regions[id.index()];
        &r.values[..usize::from(r.count)]
    }
    pub(crate) fn abort_capture(&mut self) {
        if let Some(region) = self.capture {
            self.release(region);
            self.capture = None;
        }
    }
    pub(crate) fn release(&mut self, id: ArgumentRegionId) {
        let region = &mut self.regions[id.index()];
        debug_assert!(region.roots != 0);
        region.roots -= 1;
        if region.roots != 0 {
            return;
        }
        let mut pending = self.free.len();
        self.free.push(id);
        while pending < self.free.len() {
            let released = self.free[pending];
            pending += 1;
            let index = released.index();
            let (before, rest) = self.regions.split_at_mut(index);
            let (region, after) = rest.split_first_mut().expect("released region");
            region.words.clear();
            region.count = 0;
            region.published_raw_words = 0;
            region.source_slots = 0;
            for value in &mut region.sources {
                if let Some(backing::RetainedSource::Region { region: owner, .. }) = value {
                    let owner = *owner;
                    debug_assert_ne!(owner, released);
                    let source = if owner.index() < index {
                        &mut before[owner.index()]
                    } else {
                        &mut after[owner.index() - index - 1]
                    };
                    debug_assert!(source.roots != 0);
                    source.roots -= 1;
                    if source.roots == 0 {
                        self.free.push(owner);
                    }
                }
                *value = None;
            }
            region.sources.clear();
        }
    }
    pub(crate) fn clear(&mut self) {
        self.capture = None;
        self.free.clear();
        for (index, region) in self.regions.iter_mut().enumerate() {
            region.words.clear();
            region.roots = 0;
            region.count = 0;
            region.published_raw_words = 0;
            region.source_slots = 0;
            region.sources.clear();
            self.free.push(ArgumentRegionId(
                NonZeroU32::new(index as u32 + 1).expect("region"),
            ));
        }
    }
    #[cfg(test)]
    pub(crate) fn pool_counts(&self) -> (usize, usize, usize) {
        (
            self.regions.len(),
            self.regions.iter().filter(|r| r.roots != 0).count(),
            self.retained_bytes(),
        )
    }
    pub(crate) fn is_matching(&self) -> bool {
        self.capture.is_some()
    }
    pub(crate) fn captured_word_len(&self) -> usize {
        self.capture.map_or(0, |id| {
            let r = &self.regions[id.index()];
            r.published_raw_words
                + if r.count < 9 {
                    r.values[usize::from(r.count)].len() as usize
                } else {
                    0
                }
        })
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.regions.iter().all(|r| r.roots == 0)
    }
    pub(crate) fn retained_bytes(&self) -> usize {
        self.regions.capacity() * core::mem::size_of::<Region<G>>()
            + self.free.capacity() * core::mem::size_of::<ArgumentRegionId>()
            + self
                .regions
                .iter()
                .map(|r| {
                    r.words.capacity() * core::mem::size_of::<TracedTokenWord>()
                        + r.sources.capacity()
                            * core::mem::size_of::<Option<backing::RetainedSource<G>>>()
                })
                .sum::<usize>()
    }
    fn pending_slot(&self, id: ArgumentRegionId) -> u8 {
        self.regions[id.index()].count
    }
    fn pending_len(&self, id: ArgumentRegionId) -> u32 {
        self.regions[id.index()].values[usize::from(self.pending_slot(id))].len()
    }
    pub(crate) fn append_argument_word(
        &mut self,
        writer: &mut ArgumentCursor,
        spelling: TracedTokenWord,
    ) -> Result<u32, ScratchError> {
        if writer.holdback_len != 0 {
            return Err(ScratchError::InvalidCoordinate);
        }
        self.append_owned(writer.region, spelling)?;
        writer.visible_end = self.pending_len(writer.region);
        Ok(writer.cursor.settle_argument_word(spelling.token_word()))
    }
    pub(crate) fn append_delimiter_word(
        &mut self,
        writer: &mut ArgumentCursor,
        spelling: TracedTokenWord,
    ) -> Result<(), ScratchError> {
        if writer.holdback_len == 0 {
            writer.holdback_start = writer.visible_end;
        }
        self.append_owned(writer.region, spelling)?;
        writer.holdback_len += 1;
        Ok(())
    }
    pub(crate) fn reveal_delimiter_words(
        &mut self,
        writer: &mut ArgumentCursor,
        count: u32,
    ) -> Result<(), ScratchError> {
        if count > writer.holdback_len {
            return Err(ScratchError::InvalidCoordinate);
        }
        let end = writer.holdback_start + count;
        let words = self
            .words(writer.region, self.pending_slot(writer.region))
            .slice(writer.visible_end as usize..end as usize)
            .ok_or(ScratchError::InvalidCoordinate)?;
        for word in words.iter() {
            writer.cursor.settle_argument_word(word.token_word());
        }
        writer.visible_end = end;
        writer.holdback_start = end;
        writer.holdback_len -= count;
        Ok(())
    }
    pub(crate) fn truncate_delimiter_suffix(
        &mut self,
        writer: &mut ArgumentCursor,
    ) -> Result<(), ScratchError> {
        if writer.holdback_start + writer.holdback_len != self.pending_len(writer.region) {
            return Err(ScratchError::InvalidCoordinate);
        }
        self.truncate_pending(writer.region, writer.holdback_start);
        writer.visible_end = writer.holdback_start;
        writer.holdback_len = 0;
        Ok(())
    }
    pub(crate) fn match_words(
        &self,
        writer: &ArgumentCursor,
    ) -> Result<MacroWords<'_>, ScratchError> {
        let (start, end) = writer.range()?;
        self.words(writer.region, self.pending_slot(writer.region))
            .slice(start as usize..end as usize)
            .map(ArgumentWords::iter)
            .ok_or(ScratchError::InvalidCoordinate)
    }
    pub(crate) fn publish_argument(
        &mut self,
        matching: &mut MatchedArguments,
        writer: ArgumentCursor,
    ) -> Result<(), ScratchError> {
        if writer.region != matching.region
            || self.pending_slot(matching.region) >= 9
            || writer.holdback_len != 0
            || writer.visible_end != self.pending_len(writer.region)
        {
            return Err(ScratchError::InvalidCoordinate);
        }
        let (start, end) = writer.range()?;
        let region = &mut self.regions[matching.region.index()];
        region.published_raw_words += writer.visible_end as usize;
        let slot = usize::from(region.count);
        let value = &mut region.values[slot];
        if region.source_slots & (1 << slot) == 0 {
            value.start += start;
        } else {
            region.sources[value.start as usize]
                .as_mut()
                .expect("retained argument")
                .advance(start);
        }
        value.len = end - start;
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
            .ok_or(ScratchError::InvalidCoordinate)
    }
    pub(crate) fn delimiter_prefix_words<'a>(
        &'a self,
        writer: &ArgumentCursor,
    ) -> Result<MacroWords<'a>, ScratchError> {
        self.delimiter_prefix(writer).map(ArgumentWords::iter)
    }
    fn delimiter_prefix(&self, writer: &ArgumentCursor) -> Result<ArgumentWords<'_>, ScratchError> {
        let start = writer.holdback_start as usize;
        self.words(writer.region, self.pending_slot(writer.region))
            .slice(start..start + writer.holdback_len as usize)
            .ok_or(ScratchError::InvalidCoordinate)
    }
}
impl<G> PlainRunSink<G> for ArgumentCursor {
    #[inline(always)]
    fn brace_depth(&self) -> u32 {
        self.cursor.brace_depth()
    }
    #[inline(always)]
    fn settle_admitted_word(&mut self, delta: i8) {
        self.cursor.settle_argument_delta(delta);
    }
    fn consume_region_span(
        &mut self,
        arena: &mut ArgumentArena<G>,
        source: ArgumentRegionId,
        slot: u8,
        range: core::ops::Range<usize>,
        admission: &mut ArgumentRunAdmission<'_, '_, G>,
    ) -> Result<usize, ScratchError> {
        let mut count = 0;
        for word in arena
            .words(source, slot)
            .slice(range.clone())
            .ok_or(ScratchError::InvalidCoordinate)?
            .iter()
        {
            if !admission.admits(word.token_word(), self) {
                break;
            }
            count += 1;
        }
        if count != 0 {
            arena.extend_region(self.region, source, slot, range.start as u32, count as u32)?;
            self.visible_end = arena.pending_len(self.region);
        }
        Ok(count)
    }
    fn consume_token_span(
        &mut self,
        arena: &mut ArgumentArena<G>,
        body: &tex_state::ResidentMacroBody<G>,
        range: core::ops::Range<usize>,
        admission: &mut ArgumentRunAdmission<'_, '_, G>,
    ) -> Result<usize, ScratchError> {
        if self.holdback_len != 0 {
            return Err(ScratchError::InvalidCoordinate);
        }
        let mut count = 0;
        for &word in &body.replacement_from(range.start as u32)[..range.len()] {
            if !admission.admits(word, self) {
                break;
            }
            count += 1;
        }
        if count != 0 {
            arena.extend_definition(self.region, body, range.start as u32, count as u32)?;
            self.visible_end = arena.pending_len(self.region);
        }
        Ok(count)
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
