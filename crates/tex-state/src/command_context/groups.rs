//! Native group operations on the actual admitted command context.

use super::*;

impl<'a, G> CommandContext<'a, G> {
    /// Borrows the exact open-group frames for command-layer coordination.
    ///
    /// The slice is valid only for this admitted command episode. Persistent
    /// command state copies the compact frames it needs and never retains a
    /// reference into the mutable state bank.
    #[must_use]
    pub fn group_frames(&self) -> crate::GroupFrames<'_, G> {
        self.admitted.state_ref().group_frames()
    }

    #[must_use]
    pub fn group_frames_from(&self, start: usize) -> Vec<(usize, &'static str, u32)> {
        self.admitted
            .state_ref()
            .group_frames()
            .iter()
            .copied()
            .enumerate()
            .skip(start)
            .map(|(index, frame)| (index + 1, frame.kind().group_text(), frame.entered_line()))
            .collect()
    }

    #[must_use]
    pub fn current_group_values(&self) -> (i32, i32) {
        let frames = self.admitted.state_ref().group_frames();
        let level =
            i32::try_from(self.admitted.state_ref().current_level() - 1).unwrap_or(i32::MAX);
        let kind = frames.last().map_or(0, |frame| frame.kind().etex_code());
        (level, kind)
    }

    #[must_use]
    pub fn execution_group_depth(&self) -> usize {
        self.admitted.state_ref().group_frames().len()
    }

    #[must_use]
    pub fn checked_save_stack_words(&self, source_lines: bool) -> usize {
        self.admitted
            .state_ref()
            .checked_save_stack_words(source_lines)
    }

    #[must_use]
    pub fn max_save_stack_words(&self, source_lines: bool) -> usize {
        self.admitted.state_ref().max_save_stack_words(source_lines)
    }

    pub fn add_box_spec_words(&mut self, words: usize) {
        self.admitted.state().add_box_spec_words(words);
    }

    pub fn remove_box_spec_words(&mut self, words: usize) {
        self.admitted.state().remove_box_spec_words(words);
    }

    #[must_use]
    pub fn innermost_group_kind(&self) -> Option<crate::GroupKind> {
        self.admitted
            .state_ref()
            .group_frames()
            .last()
            .map(|frame| frame.kind())
    }

    pub fn begin_group(
        &mut self,
        kind: crate::GroupKind,
        entered_line: u32,
    ) -> Result<(), StateError> {
        self.admitted.state().begin_group(kind, entered_line)
    }

    /// Streams each native restore while its saved owners are still live.
    /// Aftergroup words are admitted at their actual position in unsave.
    #[inline(always)]
    pub fn end_group_observed(
        &mut self,
        kind: crate::GroupKind,
        mut observe: impl FnMut(&mut Self, crate::GroupUnsaveEvent<'_, G>),
    ) -> Result<crate::GroupFrame, StateError> {
        let frame = self.admitted.state_ref().closing_group(kind)?;
        self.admitted.state().journal_mut().begin_unsave();
        if self.admitted.state_ref().group_has_saved_records(frame) {
            self.restore_group_records(frame, &mut observe)?;
        }
        self.admitted
            .state()
            .journal_mut()
            .record_group_exit_with_records(frame);
        self.resident
            .dependencies
            .poison(TrackedRegionBarrier::EnvironmentTimelineChange);
        Ok(frame)
    }

    pub fn end_group(&mut self, kind: crate::GroupKind) -> Result<crate::GroupFrame, StateError> {
        self.end_group_observed(kind, |_, _| {})
    }

    pub fn save_aftergroup(
        &mut self,
        word: crate::token::TracedTokenWord,
    ) -> Result<(), StateError> {
        if self.execution_group_depth() == 0 {
            return Err(StateError::CheckpointIneligible);
        }
        self.admitted.state().journal_mut().save_aftergroup(word);
        Ok(())
    }

    // The restoration cursor and sparse chain exist only when the actual
    // native stack contains saved records. Empty scopes never enter this frame.
    #[inline(never)]
    fn restore_group_records(
        &mut self,
        frame: crate::GroupFrame,
        observe: &mut impl FnMut(&mut Self, crate::GroupUnsaveEvent<'_, G>),
    ) -> Result<(), StateError> {
        use crate::journal::JournalEntry;
        let mut cursor = self.admitted.state_ref().journal().len();
        let mut sparse = None;
        while let Some(entry) =
            self.admitted
                .state()
                .next_group_restore(frame, &mut cursor, &mut sparse)
        {
            match entry {
                JournalEntry::Mutation(saved) => {
                    let mut saved = self.admitted.state_ref().own_saved_mutation(saved);
                    let entry = self
                        .admitted
                        .state()
                        .restore_group_mutation(&mut saved.saved)?;
                    observe(self, crate::GroupUnsaveEvent::Restoration(&entry));
                }
                JournalEntry::Box { mut saved, .. } => {
                    let value = self.durable_boxes.restore_group_record(&mut saved);
                    let entry = crate::GroupRestorationEntry {
                        cell: crate::GroupRestorationCell::BoxRegister(value.index),
                        saved: crate::GroupRestorationValue::NodeList(value.saved),
                        live: crate::GroupRestorationValue::NodeList(value.live),
                        outcome: value.outcome,
                        trace: self.admitted.state_ref().group_restoration_trace_state()?,
                    };
                    observe(self, crate::GroupUnsaveEvent::Restoration(&entry));
                    self.durable_boxes
                        .retire_group_record(&mut self.page_nodes, saved);
                }
                JournalEntry::AfterGroup(word) => {
                    observe(self, crate::GroupUnsaveEvent::AfterGroup(word))
                }
                _ => unreachable!("native unsave returns only typed saved entries"),
            }
        }
        Ok(())
    }
}
