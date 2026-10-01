//! Generation-owned reusable command execution scratch.
//!
//! The one pending macro match writes its arguments into a single reusable
//! word vector. Activation copies the finished arguments onto the input token
//! stack (TeX82 §400's `param_stack` pushes) and clears the vector, so no
//! argument outlives its match here and no invocation owns a heap buffer.

use core::marker::PhantomData;

use tex_state::token::{Catcode, TokenWord, TracedTokenWord};

use crate::command::HotCommand;
#[cfg(test)]
use crate::token_collector::ClassifiedToken;

/// Capability for the one synchronous, unpublished argument set.
///
/// It is consumed exactly once by transfer or discard and never enters input
/// state; its serial rejects a stale handle from an earlier match.
#[derive(Debug)]
pub(crate) struct PendingArgumentSet<G> {
    serial: u32,
    _generation: PhantomData<fn(&G) -> &G>,
}

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
pub(crate) struct MacroArgumentWriter<G> {
    slot: u8,
    start: u32,
    visible_end: u32,
    cursor: crate::scanner_kernel::ScannerCursor,
    end_trim: u8,
    holdback_start: u32,
    holdback_len: u32,
    _generation: PhantomData<fn(&G) -> &G>,
}

impl<G> MacroArgumentWriter<G> {
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ScratchError {
    InvalidCoordinate,
    CapacityOverflow,
    AllocationFailed,
}

/// Words of one matched argument or delimiter prefix, in order.
pub(crate) type MacroWords<'a> = core::iter::Copied<core::slice::Iter<'a, TracedTokenWord>>;

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct ExecutionScratch<G> {
    /// Argument and delimiter-candidate words of the pending match.
    macro_words: Vec<TracedTokenWord>,
    /// Published `(start, len)` ranges of the pending match's arguments.
    arguments: [(u32, u32); 9],
    argument_count: u8,
    writing_argument: bool,
    /// Serial of the pending match, if one is open.
    pending: Option<u32>,
    next_serial: u32,
    expression_frames: Vec<crate::scanners::ExpressionFrame<G>>,
}

impl<G> Default for ExecutionScratch<G> {
    fn default() -> Self {
        Self {
            macro_words: Vec::new(),
            arguments: [(0, 0); 9],
            argument_count: 0,
            writing_argument: false,
            pending: None,
            next_serial: 0,
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

    /// Opens the one pending match. Matching never nests: TeX82 §392 reads
    /// arguments with `get_token`, which never expands another macro.
    pub(crate) fn begin_macro_match(&mut self) -> Result<PendingArgumentSet<G>, ScratchError> {
        if self.pending.is_some() {
            return Err(ScratchError::InvalidCoordinate);
        }
        debug_assert!(self.macro_words.is_empty());
        let serial = self.next_serial;
        self.next_serial = serial.wrapping_add(1);
        self.pending = Some(serial);
        self.argument_count = 0;
        self.writing_argument = false;
        Ok(PendingArgumentSet {
            serial,
            _generation: PhantomData,
        })
    }

    #[inline]
    fn validate_pending(&self, matching: &PendingArgumentSet<G>) -> Result<(), ScratchError> {
        (self.pending == Some(matching.serial))
            .then_some(())
            .ok_or(ScratchError::InvalidCoordinate)
    }

    #[inline]
    fn tail(&self) -> u32 {
        self.macro_words.len() as u32
    }

    pub(crate) fn begin_argument_writer(
        &mut self,
        matching: &PendingArgumentSet<G>,
    ) -> Result<MacroArgumentWriter<G>, ScratchError> {
        self.validate_pending(matching)?;
        if self.writing_argument || self.argument_count >= 9 {
            return Err(ScratchError::InvalidCoordinate);
        }
        self.writing_argument = true;
        let start = self.tail();
        Ok(MacroArgumentWriter {
            slot: self.argument_count,
            start,
            visible_end: start,
            cursor: crate::scanner_kernel::ScannerCursor::default(),
            end_trim: 0,
            holdback_start: start,
            holdback_len: 0,
            _generation: PhantomData,
        })
    }

    /// Settles one classified token: the append and the paragraph,
    /// brace-depth, and removable-outer-group facts from one classification.
    #[cfg(test)]
    pub(crate) fn append_argument_token(
        &mut self,
        writer: &mut MacroArgumentWriter<G>,
        token: ClassifiedToken,
        paragraph_checked: bool,
    ) -> Result<u32, ScratchError> {
        if writer.holdback_len != 0 {
            return Err(ScratchError::InvalidCoordinate);
        }
        self.macro_words.push(token.word());
        writer.visible_end = self.tail();
        Ok(writer.cursor.settle_argument(token, paragraph_checked))
    }

    /// Appends one compact raw delivery directly to the pending argument.
    #[inline(always)]
    pub(crate) fn append_hot_delivery(
        &mut self,
        writer: &mut MacroArgumentWriter<G>,
        delivery: &HotCommand<G>,
        paragraph_checked: bool,
        paragraph_token: Option<TokenWord>,
    ) -> Result<u32, ScratchError> {
        if writer.holdback_len != 0 {
            return Err(ScratchError::InvalidCoordinate);
        }
        let spelling = delivery.spelling();
        debug_assert_eq!(spelling.origin(), delivery.origin());
        self.macro_words.push(spelling);
        writer.visible_end = self.tail();
        Ok(writer.cursor.settle_argument_word(
            delivery.spelling_word(),
            paragraph_checked && delivery.is_paragraph_spelling(paragraph_token),
        ))
    }

    /// Appends one already-classified ordinary span to the pending argument.
    ///
    /// Callers stop before argument-closing braces, delimiters, paragraph
    /// commands, outer control sequences, and input transitions, so each word
    /// settles as ordinary argument material.
    pub(crate) fn append_plain_argument_span(
        &mut self,
        writer: &mut MacroArgumentWriter<G>,
        words: &[TracedTokenWord],
    ) -> Result<(), ScratchError> {
        if writer.holdback_len != 0 {
            return Err(ScratchError::InvalidCoordinate);
        }
        self.macro_words.extend_from_slice(words);
        for word in words {
            writer.cursor.settle_argument_word(word.token_word(), false);
        }
        writer.visible_end = self.tail();
        Ok(())
    }

    /// Appends an unpublished delimiter candidate after the visible argument
    /// prefix. Its scanner facts stay untouched until a mismatch reveals it.
    #[inline(always)]
    pub(crate) fn append_delimiter_word(
        &mut self,
        writer: &mut MacroArgumentWriter<G>,
        spelling: TracedTokenWord,
    ) -> Result<(), ScratchError> {
        if writer.holdback_len == 0 {
            writer.holdback_start = writer.visible_end;
        }
        self.macro_words.push(spelling);
        writer.holdback_len += 1;
        Ok(())
    }

    /// Commits the first `count` words of the unpublished delimiter suffix.
    /// The words are already in place; only the scanner cursor and the
    /// visible end advance.
    pub(crate) fn reveal_delimiter_words(
        &mut self,
        writer: &mut MacroArgumentWriter<G>,
        count: u32,
    ) -> Result<(), ScratchError> {
        if count > writer.holdback_len {
            return Err(ScratchError::InvalidCoordinate);
        }
        let end = writer.holdback_start + count;
        let revealed = self
            .macro_words
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
        writer: &mut MacroArgumentWriter<G>,
    ) -> Result<(), ScratchError> {
        if writer.holdback_start + writer.holdback_len != self.tail() {
            return Err(ScratchError::InvalidCoordinate);
        }
        self.macro_words.truncate(writer.holdback_start as usize);
        writer.visible_end = writer.holdback_start;
        writer.holdback_len = 0;
        Ok(())
    }

    pub(crate) fn match_words(
        &self,
        writer: &MacroArgumentWriter<G>,
    ) -> Result<MacroWords<'_>, ScratchError> {
        let (start, end) = writer.range()?;
        self.macro_words
            .get(start as usize..end as usize)
            .map(|words| words.iter().copied())
            .ok_or(ScratchError::InvalidCoordinate)
    }

    pub(crate) fn publish_argument(
        &mut self,
        writer: MacroArgumentWriter<G>,
    ) -> Result<(), ScratchError> {
        if !self.writing_argument
            || self.argument_count != writer.slot
            || writer.holdback_len != 0
            || writer.visible_end != self.tail()
        {
            return Err(ScratchError::InvalidCoordinate);
        }
        let (start, end) = writer.range()?;
        self.arguments[usize::from(writer.slot)] = (start, end - start);
        self.argument_count += 1;
        self.writing_argument = false;
        Ok(())
    }

    pub(crate) fn delimiter_prefix_len(
        &self,
        writer: &MacroArgumentWriter<G>,
    ) -> Result<usize, ScratchError> {
        Ok(writer.holdback_len as usize)
    }

    pub(crate) fn delimiter_prefix_word(
        &self,
        writer: &MacroArgumentWriter<G>,
        index: usize,
    ) -> Result<TracedTokenWord, ScratchError> {
        self.delimiter_prefix(writer)?
            .get(index)
            .copied()
            .ok_or(ScratchError::InvalidCoordinate)
    }

    pub(crate) fn delimiter_prefix_words<'a>(
        &'a self,
        writer: &'a MacroArgumentWriter<G>,
    ) -> Result<MacroWords<'a>, ScratchError> {
        self.delimiter_prefix(writer)
            .map(|words| words.iter().copied())
    }

    fn delimiter_prefix(
        &self,
        writer: &MacroArgumentWriter<G>,
    ) -> Result<&[TracedTokenWord], ScratchError> {
        let start = writer.holdback_start as usize;
        self.macro_words
            .get(start..start + writer.holdback_len as usize)
            .ok_or(ScratchError::InvalidCoordinate)
    }

    pub(crate) fn discard_macro_match(
        &mut self,
        matching: PendingArgumentSet<G>,
    ) -> Result<(), ScratchError> {
        self.validate_pending(&matching)?;
        self.close_match();
        Ok(())
    }

    fn close_match(&mut self) {
        self.pending = None;
        self.writing_argument = false;
        self.argument_count = 0;
        self.macro_words.clear();
    }

    /// Copies a completed match's arguments onto the input token stack and
    /// pushes their spans: TeX82 §400's `param_stack` pushes. The match is
    /// then closed, so the scratch only ever holds the one argument set
    /// being matched.
    pub(crate) fn transfer_macro_match(
        &mut self,
        matching: PendingArgumentSet<G>,
        words: &mut Vec<TracedTokenWord>,
        parameters: &mut Vec<crate::input::ArgumentSpan>,
    ) -> Result<usize, ScratchError> {
        self.validate_pending(&matching)?;
        if self.writing_argument {
            return Err(ScratchError::InvalidCoordinate);
        }
        let count = usize::from(self.argument_count);
        for &(start, len) in &self.arguments[..count] {
            let span = crate::input::ArgumentSpan {
                start: u32::try_from(words.len()).map_err(|_| ScratchError::CapacityOverflow)?,
                len,
            };
            words.extend_from_slice(&self.macro_words[start as usize..(start + len) as usize]);
            parameters.push(span);
        }
        self.close_match();
        Ok(count)
    }

    pub(crate) fn argument_word_len(&self) -> usize {
        self.macro_words.len()
    }

    pub(crate) fn is_quiescent(&self) -> bool {
        self.pending.is_none() && self.macro_words.is_empty()
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
    fn append_span(
        &mut self,
        scratch: &mut ExecutionScratch<G>,
        words: &[TracedTokenWord],
    ) -> Result<(), ScratchError>;
}

impl<G> PlainRunSink<G> for MacroArgumentWriter<G> {
    fn append_span(
        &mut self,
        scratch: &mut ExecutionScratch<G>,
        words: &[TracedTokenWord],
    ) -> Result<(), ScratchError> {
        scratch.append_plain_argument_span(self, words)
    }
}

/// Writes a plain run into an open definition's replacement text and tracks
/// the net brace depth the run's interior groups leave behind.
pub(crate) struct DefinitionReplacementSink<'w, G> {
    pub(crate) writer: &'w mut tex_state::DefinitionBuildWriter<G>,
    pub(crate) depth_delta: i32,
}

impl<G> DefinitionReplacementSink<'_, G> {
    #[inline(always)]
    fn push(&mut self, word: TokenWord) -> Result<(), tex_state::DefinitionBuildError> {
        self.depth_delta += match word.literal_catcode() {
            Some(Catcode::BeginGroup) => 1,
            Some(Catcode::EndGroup) => -1,
            _ => 0,
        };
        self.writer.push_replacement(word)
    }
}

impl<G> PlainRunSink<G> for DefinitionReplacementSink<'_, G> {
    fn append_span(
        &mut self,
        _: &mut ExecutionScratch<G>,
        words: &[TracedTokenWord],
    ) -> Result<(), ScratchError> {
        words
            .iter()
            .try_for_each(|&word| self.push(word.token_word()))
            .map_err(|_| ScratchError::InvalidCoordinate)
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
    fn append_span(
        &mut self,
        _: &mut ExecutionScratch<G>,
        words: &[TracedTokenWord],
    ) -> Result<(), ScratchError> {
        words
            .iter()
            .try_for_each(|&word| self.push(word.token_word()))
    }
}

#[derive(Clone, Copy)]
pub(crate) struct ArgumentRunAdmission<'a, 'admission, G> {
    state: Option<&'a tex_state::CommandContext<'admission, G>>,
    paragraph_token: Option<TokenWord>,
    stop_word: Option<TokenWord>,
    depth: u32,
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
        depth: u32,
        close_floor: u32,
    ) -> Self {
        Self {
            state: Some(state),
            paragraph_token,
            stop_word,
            depth,
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
            depth: 0,
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
    pub(crate) fn admits(&mut self, word: TokenWord) -> bool {
        if self.closed || (self.depth == 0 && Some(word) == self.stop_word) {
            return false;
        }
        let admitted = match word.literal_catcode() {
            Some(Catcode::BeginGroup) if self.state.is_some() => {
                self.depth += 1;
                self.brace_delta += 1;
                return true;
            }
            Some(Catcode::EndGroup)
                if self.state.is_some()
                    && (self.depth > self.close_floor
                        || (self.completes_undelimited && self.depth == 1)) =>
            {
                self.depth -= 1;
                self.brace_delta -= 1;
                true
            }
            Some(Catcode::Space) if self.completes_undelimited && self.depth == 0 => false,
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
        self.closed = admitted && self.completes_undelimited && self.depth == 0;
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
    fn write_control_sequence(&mut self, _: Option<tex_state::interner::Symbol>) {}

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
