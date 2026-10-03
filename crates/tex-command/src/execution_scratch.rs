//! Reusable bounded expression scratch and plain resident-run admission.
use tex_state::token::{Catcode, TokenWord};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ScratchError {
    InvalidCoordinate,
    CapacityOverflow,
    AllocationFailed,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct ExecutionScratch<G> {
    expression_frames: Vec<crate::scanners::ExpressionFrame<G>>,
}
impl<G> Default for ExecutionScratch<G> {
    fn default() -> Self {
        Self {
            expression_frames: Vec::new(),
        }
    }
}
impl<G> ExecutionScratch<G> {
    /// Discards the bounded expression stack after a resource miss. The
    /// enclosing host checkpoint owns retry; no scanner frame, scalar phase,
    /// or expansion control is allowed to outlive this processor episode.
    pub(crate) fn unwind_resource_failure(&mut self) -> Result<(), ScratchError> {
        self.expression_frames.clear();
        Ok(())
    }

    pub(crate) fn expression_stack_len(&self) -> usize {
        self.expression_frames.len()
    }

    pub(crate) fn push_expression_frame(
        &mut self,
        frame: crate::scanners::ExpressionFrame<G>,
    ) -> Result<(), ScratchError> {
        self.expression_frames
            .try_reserve(1)
            .map_err(|_| ScratchError::AllocationFailed)?;
        self.expression_frames.push(frame);
        Ok(())
    }

    pub(crate) fn pop_expression_frame(
        &mut self,
        mark: usize,
    ) -> Result<Option<crate::scanners::ExpressionFrame<G>>, ScratchError> {
        if self.expression_frames.len() < mark {
            return Err(ScratchError::InvalidCoordinate);
        }
        Ok((self.expression_frames.len() > mark)
            .then(|| self.expression_frames.pop())
            .flatten())
    }

    pub(crate) fn truncate_expression_stack(&mut self, mark: usize) -> Result<(), ScratchError> {
        if self.expression_frames.len() < mark {
            return Err(ScratchError::InvalidCoordinate);
        }
        self.expression_frames.truncate(mark);
        Ok(())
    }

    pub(crate) fn is_quiescent(&self) -> bool {
        self.expression_frames.is_empty()
    }
}

/// Decides which raw words a batched macro-argument run may consume without
/// a delivery, tracking the brace depth the run reaches.
///
/// TeX82 §§392--399 only inspect each matched token's spelling, except that
/// §336 validates an outer control sequence, §395 counts braces, and §396
/// rejects `\par` in a non-long argument. A word that raises none of those
/// cases settles exactly like a plain character: its spelling is appended and
/// the scanner cursor advances. Interior braces only move the depth (and
/// §347's `align_state`, which the caller settles once from the run's net
/// brace change). Parameters, frozen tokens, the paragraph token, an outer
/// meaning, a brace that would close the argument, and the delimiter's first
/// token at depth zero return the run to scalar delivery. Without a command
/// context only literal non-brace, non-active characters are admitted.
/// Destination of one batched run of plain resident words.
pub(crate) trait PlainRunSink<G> {
    fn brace_depth(&self) -> u32;
    fn settle_admitted_word(&mut self, brace_delta: i8);
    fn consume_region_span(
        &mut self,
        arena: &mut crate::input::ArgumentArena<G>,
        source: crate::input::ArgumentRegionId,
        slot: u8,
        range: core::ops::Range<usize>,
        admission: &mut ArgumentRunAdmission<'_, '_, G>,
    ) -> Result<usize, ScratchError>;
    fn consume_token_span(
        &mut self,
        arena: &mut crate::input::ArgumentArena<G>,
        body: &tex_state::ResidentMacroBody<G>,
        range: core::ops::Range<usize>,
        admission: &mut ArgumentRunAdmission<'_, '_, G>,
    ) -> Result<usize, ScratchError>;
}

/// Writes a plain run into an open definition's replacement text and tracks
/// the net brace depth the run's interior groups leave behind.
pub(crate) struct DefinitionReplacementSink<'w, G> {
    pub(crate) writer: &'w mut tex_state::DefinitionBuildWriter<G>,
    pub(crate) cursor: &'w mut crate::scanner_kernel::ScannerCursor,
}

impl<G> PlainRunSink<G> for DefinitionReplacementSink<'_, G> {
    #[inline(always)]
    fn brace_depth(&self) -> u32 {
        self.cursor.brace_depth()
    }

    #[inline(always)]
    fn settle_admitted_word(&mut self, brace_delta: i8) {
        if brace_delta != 0 {
            self.cursor.advance_balanced_depth(i32::from(brace_delta));
        }
    }
    fn consume_region_span(
        &mut self,
        arena: &mut crate::input::ArgumentArena<G>,
        source: crate::input::ArgumentRegionId,
        slot: u8,
        range: core::ops::Range<usize>,
        admission: &mut ArgumentRunAdmission<'_, '_, G>,
    ) -> Result<usize, ScratchError> {
        let words = arena
            .words(source, slot)
            .slice(range)
            .ok_or(ScratchError::InvalidCoordinate)?;
        let mut count = 0;
        for word in words.iter() {
            if !admission.admits(word.token_word(), self) {
                break;
            }
            self.writer
                .push_replacement(word.token_word())
                .map_err(|_| ScratchError::InvalidCoordinate)?;
            count += 1;
        }
        Ok(count)
    }

    fn consume_token_span(
        &mut self,
        _: &mut crate::input::ArgumentArena<G>,
        body: &tex_state::ResidentMacroBody<G>,
        range: core::ops::Range<usize>,
        admission: &mut ArgumentRunAdmission<'_, '_, G>,
    ) -> Result<usize, ScratchError> {
        let mut count = 0;
        for &word in &body.replacement_from(range.start as u32)[..range.len()] {
            if !admission.admits(word, self) {
                break;
            }
            self.writer
                .push_replacement(word)
                .map_err(|_| ScratchError::InvalidCoordinate)?;
            count += 1;
        }
        Ok(count)
    }
}

/// Appends a plain run of literal characters to a `\\csname` name.
pub(crate) struct CsnameNameSink<'n> {
    pub(crate) name: &'n mut String,
}

impl CsnameNameSink<'_> {
    #[inline(always)]
    fn push(&mut self, word: TokenWord) -> Result<(), ScratchError> {
        self.name
            .push(word.literal_char().ok_or(ScratchError::InvalidCoordinate)?);
        Ok(())
    }
}

impl<G> PlainRunSink<G> for CsnameNameSink<'_> {
    fn brace_depth(&self) -> u32 {
        0
    }

    fn settle_admitted_word(&mut self, _: i8) {}
    fn consume_region_span(
        &mut self,
        arena: &mut crate::input::ArgumentArena<G>,
        source: crate::input::ArgumentRegionId,
        slot: u8,
        range: core::ops::Range<usize>,
        admission: &mut ArgumentRunAdmission<'_, '_, G>,
    ) -> Result<usize, ScratchError> {
        let words = arena
            .words(source, slot)
            .slice(range)
            .ok_or(ScratchError::InvalidCoordinate)?;
        let mut count = 0;
        for word in words.iter() {
            if !admission.admits(word.token_word(), self) {
                break;
            }
            self.push(word.token_word())?;
            count += 1;
        }
        Ok(count)
    }

    fn consume_token_span(
        &mut self,
        _: &mut crate::input::ArgumentArena<G>,
        body: &tex_state::ResidentMacroBody<G>,
        range: core::ops::Range<usize>,
        admission: &mut ArgumentRunAdmission<'_, '_, G>,
    ) -> Result<usize, ScratchError> {
        let mut count = 0;
        for &word in &body.replacement_from(range.start as u32)[..range.len()] {
            if !admission.admits(word, self) {
                break;
            }
            self.push(word)?;
            count += 1;
        }
        Ok(count)
    }
}

#[derive(Clone, Copy)]
pub(crate) struct ArgumentRunAdmission<'a, 'admission, G> {
    state: Option<&'a tex_state::CommandContext<'admission, G>>,
    paragraph_token: Option<TokenWord>,
    stop_word: Option<TokenWord>,
    /// The lowest depth a closing brace may leave: 1 inside an undelimited
    /// argument's group, whose final brace ends the argument, and 0 for a
    /// delimited argument.
    close_floor: u32,
    brace_delta: i32,
    /// A macro definition's replacement text gives §479 meaning to parameter
    /// characters, so its runs stop before them.
    stop_at_parameter: bool,
    /// An undelimited argument's run may admit the word that completes the
    /// argument: its sole token at depth zero, or the brace closing its
    /// group. `closed` then refuses every later word.
    completes_undelimited: bool,
    closed: bool,
}

impl<'a, 'admission, G> ArgumentRunAdmission<'a, 'admission, G> {
    pub(crate) const fn with_commands(
        state: &'a tex_state::CommandContext<'admission, G>,
        paragraph_token: Option<TokenWord>,
        stop_word: Option<TokenWord>,
        close_floor: u32,
    ) -> Self {
        Self {
            state: Some(state),
            paragraph_token,
            stop_word,
            close_floor,
            brace_delta: 0,
            stop_at_parameter: false,
            completes_undelimited: false,
            closed: false,
        }
    }

    /// Literal characters only, outside any group: every brace, alignment
    /// tab, active character, and control sequence stops the run.
    pub(crate) const fn characters() -> Self {
        Self {
            state: None,
            paragraph_token: None,
            stop_word: None,
            close_floor: 0,
            brace_delta: 0,
            stop_at_parameter: false,
            completes_undelimited: false,
            closed: false,
        }
    }

    /// An undelimited argument's admission, which also admits the word that
    /// completes it (TeX82 §392-§394). A space before the argument, a stray
    /// closing brace, and every word the ordinary admission refuses stay with
    /// the scalar matcher.
    pub(crate) const fn completing_undelimited(self) -> Self {
        Self {
            completes_undelimited: true,
            ..self
        }
    }

    /// The same admission, also stopping before parameter characters.
    pub(crate) const fn stopping_at_parameters(self, stop: bool) -> Self {
        Self {
            stop_at_parameter: stop,
            ..self
        }
    }

    /// The same admission restricted to literal characters, for input frames
    /// whose control sequences need ordinary delivery settlement.
    pub(crate) const fn characters_only(self) -> Self {
        Self {
            state: None,
            ..self
        }
    }

    /// Net `align_state` change of the braces admitted so far.
    pub(crate) const fn brace_delta(&self) -> i32 {
        self.brace_delta
    }

    #[inline(always)]
    pub(crate) fn admits(&mut self, word: TokenWord, sink: &mut impl PlainRunSink<G>) -> bool {
        let depth = sink.brace_depth();
        if self.closed || (depth == 0 && Some(word) == self.stop_word) {
            return false;
        }
        let mut delta = 0;
        let admitted = match word.literal_catcode() {
            Some(Catcode::BeginGroup) if self.state.is_some() => {
                delta = 1;
                true
            }
            Some(Catcode::EndGroup)
                if self.state.is_some()
                    && (depth > self.close_floor || (self.completes_undelimited && depth == 1)) =>
            {
                delta = -1;
                true
            }
            Some(Catcode::Space) if self.completes_undelimited && depth == 0 => false,
            Some(Catcode::BeginGroup | Catcode::EndGroup | Catcode::AlignmentTab) => false,
            Some(Catcode::Parameter) => !self.stop_at_parameter,
            Some(Catcode::Active) => self.admits_command(word),
            Some(_) => true,
            None => {
                word.is_control_sequence()
                    && Some(word) != self.paragraph_token
                    && self.admits_command(word)
            }
        };
        if admitted {
            sink.settle_admitted_word(delta);
            self.brace_delta += i32::from(delta);
            self.closed = self.completes_undelimited && sink.brace_depth() == 0;
        }
        admitted
    }

    #[inline(always)]
    fn admits_command(&self, word: TokenWord) -> bool {
        let Some(state) = self.state else {
            return false;
        };
        let mut probe = OuterMeaningProbe { outer: false };
        state.write_packed_token_command_into(word, &mut probe);
        !probe.outer
    }
}

/// Records only whether a resolved meaning is §336's outer class.
struct OuterMeaningProbe {
    outer: bool,
}

impl<G> tex_state::token::PackedCommandTarget<G> for OuterMeaningProbe {
    #[inline(always)]
    fn write_static_meaning_word(&mut self, word: u64) {
        self.outer = word == tex_state::meaning::Meaning::END_TEMPLATE_WORD;
    }

    #[inline(always)]
    fn write_font_meaning(&mut self, _: tex_state::ids::FontId) {
        self.outer = false;
    }

    #[inline(always)]
    fn write_macro_meaning(
        &mut self,
        flags: tex_state::meaning::MeaningFlags,
        _: tex_state::DefinitionRef<G>,
    ) {
        self.outer = flags.contains(tex_state::meaning::MeaningFlags::OUTER);
    }
}

#[cfg(test)]
mod tests;
