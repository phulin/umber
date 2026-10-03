//! Final pooled spelling and sparse leases on actual immutable sources.
use super::*;
use tex_state::ResidentMacroBody;

#[derive(Debug)]
pub(super) enum RetainedSource<G> {
    Definition {
        body: ResidentMacroBody<G>,
        start: u32,
    },
    Region {
        region: ArgumentRegionId,
        start: u32,
    },
}
impl<G> RetainedSource<G> {
    pub(super) fn advance(&mut self, delta: u32) {
        match self {
            Self::Definition { start, .. } | Self::Region { start, .. } => *start += delta,
        }
    }
}
impl<G> ArgumentArena<G> {
    #[inline(always)]
    pub(crate) fn words(&self, id: ArgumentRegionId, slot: u8) -> ArgumentWords<'_> {
        let region = &self.regions[id.index()];
        let value = region.values[usize::from(slot)];
        if region.source_slots & (1 << slot) == 0 {
            return ArgumentWords::Traced(
                &region.words[value.start as usize..(value.start + value.len) as usize],
            );
        }
        match region.sources[value.start as usize]
            .as_ref()
            .expect("retained argument source")
        {
            RetainedSource::Definition { body, start } => {
                ArgumentWords::Definition(&body.replacement_from(*start)[..value.len as usize])
            }
            RetainedSource::Region { region, start } => ArgumentWords::Traced(
                &self.regions[region.index()].words[*start as usize..(*start + value.len) as usize],
            ),
        }
    }
    #[inline(never)]
    fn materialize(&mut self, id: ArgumentRegionId) -> Result<(), ScratchError> {
        let slot = usize::from(self.pending_slot(id));
        let value = self.regions[id.index()].values[slot];
        self.regions[id.index()]
            .words
            .try_reserve(value.len as usize)
            .map_err(|_| ScratchError::AllocationFailed)?;
        let source = self.regions[id.index()].sources[value.start as usize]
            .take()
            .expect("retained source");
        let start = self.regions[id.index()].words.len() as u32;
        match &source {
            RetainedSource::Definition { body, start } => self.regions[id.index()].words.extend(
                body.replacement_from(*start)[..value.len as usize]
                    .iter()
                    .map(|&word| {
                        TracedTokenWord::from_parts(word, tex_state::token::OriginId::UNKNOWN)
                    }),
            ),
            RetainedSource::Region { region, start } => {
                let original = region.index();
                let destination = id.index();
                debug_assert_ne!(original, destination);
                let (from, to) = if original < destination {
                    let (a, b) = self.regions.split_at_mut(destination);
                    (&a[original], &mut b[0])
                } else {
                    let (a, b) = self.regions.split_at_mut(original);
                    (&b[0], &mut a[destination])
                };
                to.words
                    .extend_from_slice(&from.words[*start as usize..(*start + value.len) as usize]);
            }
        }
        self.regions[id.index()].source_slots &= !(1 << slot);
        self.regions[id.index()].values[slot].start = start;
        while self.regions[id.index()]
            .sources
            .last()
            .is_some_and(Option::is_none)
        {
            self.regions[id.index()].sources.pop();
        }
        if let RetainedSource::Region { region, .. } = source {
            self.release(region);
        }
        Ok(())
    }
    #[inline(always)]
    pub(super) fn append_owned(
        &mut self,
        id: ArgumentRegionId,
        word: TracedTokenWord,
    ) -> Result<(), ScratchError> {
        let slot = usize::from(self.pending_slot(id));
        if self.regions[id.index()].source_slots & (1 << slot) != 0 {
            self.materialize(id)?;
        }
        let region = &mut self.regions[id.index()];
        region.words.push(word);
        region.values[slot].len += 1;
        Ok(())
    }
    pub(super) fn truncate_pending(&mut self, id: ArgumentRegionId, len: u32) {
        let slot = usize::from(self.pending_slot(id));
        let region = &mut self.regions[id.index()];
        if region.source_slots & (1 << slot) == 0 {
            region
                .words
                .truncate((region.values[slot].start + len) as usize);
        }
        region.values[slot].len = len;
    }
    fn install_source(
        &mut self,
        id: ArgumentRegionId,
        source: RetainedSource<G>,
        len: u32,
    ) -> Result<(), ScratchError> {
        let slot = usize::from(self.pending_slot(id));
        let borrowed = self.regions[id.index()].source_slots & (1 << slot) != 0;
        let new_root = if let RetainedSource::Region { region, .. } = &source {
            Some((
                *region,
                self.regions[region.index()]
                    .roots
                    .checked_add(1)
                    .ok_or(ScratchError::CapacityOverflow)?,
            ))
        } else {
            None
        };
        if !borrowed {
            self.regions[id.index()]
                .sources
                .try_reserve(1)
                .map_err(|_| ScratchError::AllocationFailed)?;
        }
        if let Some((region, roots)) = new_root {
            self.regions[region.index()].roots = roots;
        }
        let region = &mut self.regions[id.index()];
        let old = if borrowed {
            region.sources[region.values[slot].start as usize].replace(source)
        } else {
            region.values[slot].start = region.sources.len() as u32;
            region.sources.push(Some(source));
            region.source_slots |= 1 << slot;
            None
        };
        region.values[slot].len = len;
        if let Some(RetainedSource::Region { region, .. }) = old {
            self.release(region);
        }
        Ok(())
    }
    #[inline(always)]
    pub(super) fn extend_definition(
        &mut self,
        id: ArgumentRegionId,
        body: &ResidentMacroBody<G>,
        start: u32,
        len: u32,
    ) -> Result<(), ScratchError> {
        let slot = self.pending_slot(id);
        if len == 1 && self.regions[id.index()].source_slots & (1 << slot) == 0 {
            return self.append_owned(
                id,
                TracedTokenWord::from_parts(
                    body.word(start as usize).expect("admitted word"),
                    tex_state::token::OriginId::UNKNOWN,
                ),
            );
        }
        let region = &mut self.regions[id.index()];
        if len <= 4 && region.source_slots & (1 << slot) == 0 {
            region.words.extend(
                body.replacement_from(start)[..len as usize]
                    .iter()
                    .map(|&word| {
                        TracedTokenWord::from_parts(word, tex_state::token::OriginId::UNKNOWN)
                    }),
            );
            region.values[usize::from(slot)].len += len;
            return Ok(());
        }
        self.retain_definition_span(id, body, start, len)
    }
    #[inline(never)]
    fn retain_definition_span(
        &mut self,
        id: ArgumentRegionId,
        body: &ResidentMacroBody<G>,
        start: u32,
        len: u32,
    ) -> Result<(), ScratchError> {
        let slot = usize::from(self.pending_slot(id));
        let region = &self.regions[id.index()];
        let value = region.values[slot];
        if region.source_slots & (1 << slot) != 0
            && matches!(region.sources[value.start as usize].as_ref(),
            Some(RetainedSource::Definition { body: previous, start: begin }) if previous == body && *begin + value.len == start)
        {
            self.regions[id.index()].values[slot].len += len;
            return Ok(());
        }
        if value.len == 0 {
            return self.install_source(
                id,
                RetainedSource::Definition {
                    body: body.clone(),
                    start,
                },
                len,
            );
        }
        if region.source_slots & (1 << slot) != 0 {
            self.materialize(id)?;
        }
        let region = &mut self.regions[id.index()];
        region.words.extend(
            body.replacement_from(start)[..len as usize]
                .iter()
                .map(|&word| {
                    TracedTokenWord::from_parts(word, tex_state::token::OriginId::UNKNOWN)
                }),
        );
        region.values[slot].len += len;
        Ok(())
    }
    #[inline(always)]
    pub(super) fn extend_region(
        &mut self,
        id: ArgumentRegionId,
        source: ArgumentRegionId,
        source_slot: u8,
        start: u32,
        len: u32,
    ) -> Result<(), ScratchError> {
        if id == source {
            return Err(ScratchError::InvalidCoordinate);
        }
        if len == 1 {
            let word = self
                .words(source, source_slot)
                .get(start as usize)
                .ok_or(ScratchError::InvalidCoordinate)?;
            return self.append_owned(id, word);
        }
        let source_region = &self.regions[source.index()];
        let source_value = source_region.values[usize::from(source_slot)];
        let (owner, at) = if source_region.source_slots & (1 << source_slot) == 0 {
            (source, source_value.start + start)
        } else {
            match source_region.sources[source_value.start as usize]
                .as_ref()
                .expect("retained source")
            {
                RetainedSource::Definition { body, start: base } => {
                    let at = *base + start;
                    let slot = usize::from(self.pending_slot(id));
                    let destination = &self.regions[id.index()];
                    let value = destination.values[slot];
                    if destination.source_slots & (1 << slot) != 0
                        && matches!(destination.sources[value.start as usize].as_ref(),
                        Some(RetainedSource::Definition { body: previous, start: begin }) if previous == body && *begin + value.len == at)
                    {
                        self.regions[id.index()].values[slot].len += len;
                        return Ok(());
                    }
                    let body = body.clone();
                    if value.len == 0 {
                        return self.install_source(
                            id,
                            RetainedSource::Definition { body, start: at },
                            len,
                        );
                    }
                    return self.retain_definition_span(id, &body, at, len);
                }
                RetainedSource::Region {
                    region,
                    start: base,
                } => (*region, *base + start),
            }
        };
        let slot = usize::from(self.pending_slot(id));
        let destination = &self.regions[id.index()];
        let value = destination.values[slot];
        if value.len == 0 {
            return self.install_source(
                id,
                RetainedSource::Region {
                    region: owner,
                    start: at,
                },
                len,
            );
        }
        if destination.source_slots & (1 << slot) != 0
            && matches!(destination.sources[value.start as usize].as_ref(),
            Some(RetainedSource::Region { region, start: begin }) if *region == owner && *begin + value.len == at)
        {
            self.regions[id.index()].values[slot].len += len;
            return Ok(());
        }
        if destination.source_slots & (1 << slot) != 0 {
            self.materialize(id)?;
        }
        let original = owner.index();
        let destination = id.index();
        let (from, to) = if original < destination {
            let (a, b) = self.regions.split_at_mut(destination);
            (&a[original], &mut b[0])
        } else {
            let (a, b) = self.regions.split_at_mut(original);
            (&b[0], &mut a[destination])
        };
        to.words
            .extend_from_slice(&from.words[at as usize..(at + len) as usize]);
        to.values[slot].len += len;
        Ok(())
    }
    /// Scalar matching shares the resident source proof used by bulk capture.
    pub(crate) fn append_from_input(
        &mut self,
        writer: &mut ArgumentCursor,
        word: TracedTokenWord,
        rows: &[super::super::InputLevel<G>],
        delimiter: bool,
    ) -> Result<u32, ScratchError> {
        if !delimiter && writer.holdback_len != 0 {
            return Err(ScratchError::InvalidCoordinate);
        }
        if self.regions[writer.region.index()].source_slots
            & (1 << self.pending_slot(writer.region))
            == 0
        {
            return if delimiter {
                self.append_delimiter_word(writer, word)?;
                Ok(writer.brace_depth())
            } else {
                self.append_argument_word(writer, word)
            };
        }
        let mut retained = false;
        if let Some(super::super::InputLevel::Resident(row)) = rows.last()
            && row.header.frame.position() != 0
        {
            let at = row.header.frame.position() - 1;
            match &row.storage {
                super::super::ResidentTokenStorage::MacroBody(source)
                    if word.origin() == tex_state::token::OriginId::UNKNOWN
                        && source.body.word(at as usize) == Some(word.token_word()) =>
                {
                    self.extend_definition(writer.region, &source.body, at, 1)?;
                    retained = true;
                }
                super::super::ResidentTokenStorage::MacroArgument { region, slot }
                    if self.words(*region, *slot - 1).get(at as usize) == Some(word) =>
                {
                    self.extend_region(writer.region, *region, *slot - 1, at, 1)?;
                    retained = true;
                }
                _ => {}
            }
        }
        if !retained {
            return if delimiter {
                self.append_delimiter_word(writer, word)?;
                Ok(writer.brace_depth())
            } else {
                self.append_argument_word(writer, word)
            };
        }
        if delimiter {
            if writer.holdback_len == 0 {
                writer.holdback_start = writer.visible_end;
            }
            writer.holdback_len += 1;
            Ok(writer.brace_depth())
        } else {
            writer.visible_end += 1;
            Ok(writer.cursor.settle_argument_word(word.token_word()))
        }
    }
}
