//! Capture extends a retained source or materializes its actual final owner.
use super::*;
use tex_state::ResidentMacroBody;

impl<G> ArgumentArena<G> {
    #[inline(always)]
    pub(crate) fn words(&self, id: ArgumentRegionId, slot: u8) -> ArgumentWords<'_> {
        match &self.regions[id.index()].values[usize::from(slot)] {
            ArgumentValue::Inline(word) => ArgumentWords::Traced(core::slice::from_ref(word)),
            ArgumentValue::Owned { start, len } => ArgumentWords::Traced(
                &self.regions[id.index()].words[*start as usize..(*start + *len) as usize],
            ),
            ArgumentValue::Definition { body, start, len } => {
                ArgumentWords::Definition(&body.replacement_from(*start)[..*len as usize])
            }
            ArgumentValue::Region {
                region,
                slot,
                start,
                len,
            } => {
                let ArgumentValue::Owned { start: base, .. } =
                    self.regions[region.index()].values[usize::from(*slot)]
                else {
                    unreachable!("region aliases resolve to owned spelling")
                };
                ArgumentWords::Traced(
                    &self.regions[region.index()].words
                        [(base + *start) as usize..(base + *start + *len) as usize],
                )
            }
        }
    }

    #[inline(never)]
    fn materialize(&mut self, id: ArgumentRegionId) -> Result<(), ScratchError> {
        let slot = usize::from(self.pending_slot(id));
        if matches!(
            self.regions[id.index()].values[slot],
            ArgumentValue::Owned { .. }
        ) {
            return Ok(());
        }
        let len = self.regions[id.index()].values[slot].len();
        self.regions[id.index()]
            .words
            .try_reserve(len as usize)
            .map_err(|_| ScratchError::AllocationFailed)?;
        let old = core::mem::take(&mut self.regions[id.index()].values[slot]);
        let start = self.regions[id.index()].words.len() as u32;
        match &old {
            ArgumentValue::Inline(word) => self.regions[id.index()].words.push(*word),
            ArgumentValue::Definition { body, start, len } => {
                self.regions[id.index()].words.extend(
                    body.replacement_from(*start)[..*len as usize]
                        .iter()
                        .map(|&word| {
                            TracedTokenWord::from_parts(word, tex_state::token::OriginId::UNKNOWN)
                        }),
                );
            }
            ArgumentValue::Region {
                region,
                slot,
                start,
                len,
            } => {
                let source = region.index();
                let destination = id.index();
                debug_assert_ne!(source, destination);
                let (from, to) = if source < destination {
                    let (a, b) = self.regions.split_at_mut(destination);
                    (&a[source], &mut b[0])
                } else {
                    let (a, b) = self.regions.split_at_mut(source);
                    (&b[0], &mut a[destination])
                };
                let ArgumentValue::Owned { start: base, .. } = from.values[usize::from(*slot)]
                else {
                    unreachable!("alias owns an original pooled range")
                };
                to.words.extend_from_slice(
                    &from.words[(base + *start) as usize..(base + *start + *len) as usize],
                );
            }
            ArgumentValue::Owned { .. } => unreachable!(),
        }
        self.regions[id.index()].values[slot] = ArgumentValue::Owned { start, len };
        if let ArgumentValue::Region { region, .. } = old {
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
        if self.regions[id.index()].values[slot].len() == 0 {
            let old = core::mem::replace(
                &mut self.regions[id.index()].values[slot],
                ArgumentValue::Inline(word),
            );
            if let ArgumentValue::Region { region, .. } = old {
                self.release(region);
            }
            return Ok(());
        }
        if !matches!(
            self.regions[id.index()].values[slot],
            ArgumentValue::Owned { .. }
        ) {
            self.materialize(id)?;
        }
        self.regions[id.index()].words.push(word);
        self.regions[id.index()].values[slot].extend(1);
        Ok(())
    }
    pub(super) fn truncate_pending(&mut self, id: ArgumentRegionId, len: u32) {
        let slot = usize::from(self.pending_slot(id));
        if let ArgumentValue::Owned { start, .. } = self.regions[id.index()].values[slot] {
            self.regions[id.index()]
                .words
                .truncate((start + len) as usize);
        }
        self.regions[id.index()].values[slot].trim(0, len);
    }
    #[inline(always)]
    pub(super) fn extend_definition(
        &mut self,
        id: ArgumentRegionId,
        body: &ResidentMacroBody<G>,
        start: u32,
        len: u32,
    ) -> Result<(), ScratchError> {
        let slot = usize::from(self.pending_slot(id));
        let value = &self.regions[id.index()].values[slot];
        if matches!(value, ArgumentValue::Definition { body: previous, start: begin, len: size } if previous == body && *begin + *size == start)
        {
            self.regions[id.index()].values[slot].extend(len);
            return Ok(());
        }
        if len == 1 {
            return self.append_owned(
                id,
                TracedTokenWord::from_parts(
                    body.word(start as usize).expect("admitted definition word"),
                    tex_state::token::OriginId::UNKNOWN,
                ),
            );
        }
        if value.len() == 0 {
            // A zero-length existing lease still has an owner to release.
            let old = core::mem::replace(
                &mut self.regions[id.index()].values[slot],
                ArgumentValue::Definition {
                    body: body.clone(),
                    start,
                    len,
                },
            );
            if let ArgumentValue::Region { region, .. } = old {
                self.release(region);
            }
        } else {
            self.materialize(id)?;
            self.regions[id.index()].words.extend(
                body.replacement_from(start)[..len as usize]
                    .iter()
                    .map(|&word| {
                        TracedTokenWord::from_parts(word, tex_state::token::OriginId::UNKNOWN)
                    }),
            );
            self.regions[id.index()].values[slot].extend(len);
        }
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
        let slot = usize::from(self.pending_slot(id));
        if len == 1 {
            let word = self
                .words(source, source_slot)
                .get(start as usize)
                .ok_or(ScratchError::InvalidCoordinate)?;
            return self.append_owned(id, word);
        }
        // Flatten to the original immutable definition or pooled owner once.
        if let ArgumentValue::Definition {
            body, start: base, ..
        } = &self.regions[source.index()].values[usize::from(source_slot)]
        {
            let at = *base + start;
            let value = &self.regions[id.index()].values[slot];
            if matches!(value, ArgumentValue::Definition { body: previous, start: begin, len: size } if previous == body && *begin + *size == at)
            {
                self.regions[id.index()].values[slot].extend(len);
                return Ok(());
            }
            let body = body.clone();
            return self.extend_definition(id, &body, at, len);
        }
        let (owner, owner_slot, at) =
            match &self.regions[source.index()].values[usize::from(source_slot)] {
                ArgumentValue::Owned { .. } => (source, source_slot, start),
                ArgumentValue::Region {
                    region,
                    slot,
                    start: base,
                    ..
                } => (*region, *slot, *base + start),
                ArgumentValue::Definition { .. } => unreachable!(),
                ArgumentValue::Inline(_) => unreachable!(),
            };
        let value = &self.regions[id.index()].values[slot];
        if value.len() == 0 {
            self.regions[owner.index()].roots = self.regions[owner.index()]
                .roots
                .checked_add(1)
                .ok_or(ScratchError::CapacityOverflow)?;
            let old = core::mem::replace(
                &mut self.regions[id.index()].values[slot],
                ArgumentValue::Region {
                    region: owner,
                    slot: owner_slot,
                    start: at,
                    len,
                },
            );
            if let ArgumentValue::Region { region, .. } = old {
                self.release(region);
            }
        } else if matches!(value, ArgumentValue::Region { region, slot: previous_slot, start: begin, len: size } if *region == owner && *previous_slot == owner_slot && *begin + *size == at)
        {
            self.regions[id.index()].values[slot].extend(len);
        } else {
            self.materialize(id)?;
            let destination = id.index();
            let source = owner.index();
            let (from, to) = if source < destination {
                let (a, b) = self.regions.split_at_mut(destination);
                (&a[source], &mut b[0])
            } else {
                let (a, b) = self.regions.split_at_mut(source);
                (&b[0], &mut a[destination])
            };
            let ArgumentValue::Owned { start: base, .. } = from.values[usize::from(owner_slot)]
            else {
                unreachable!("alias resolves to owned range")
            };
            to.words
                .extend_from_slice(&from.words[(base + at) as usize..(base + at + len) as usize]);
            to.values[slot].extend(len);
        }
        Ok(())
    }

    /// Scalar matching shares the resident source proof used by bulk capture.
    pub(crate) fn append_from_input(
        &mut self,
        writer: &mut ArgumentCursor,
        word: TracedTokenWord,
        row: Option<&super::super::ResidentTokenRow<G>>,
        delimiter: bool,
    ) -> Result<u32, ScratchError> {
        if !delimiter && writer.holdback_len != 0 {
            return Err(ScratchError::InvalidCoordinate);
        }
        if matches!(
            self.regions[writer.region.index()].values
                [usize::from(self.pending_slot(writer.region))],
            ArgumentValue::Owned { .. } | ArgumentValue::Inline(_)
        ) {
            return if delimiter {
                self.append_delimiter_word(writer, word)?;
                Ok(writer.brace_depth())
            } else {
                self.append_argument_word(writer, word)
            };
        }
        let mut retained = false;
        if let Some(row) = row
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
            writer.visible_end = self.pending_len(writer.region);
            Ok(writer.cursor.settle_argument_word(word.token_word()))
        }
    }
}
