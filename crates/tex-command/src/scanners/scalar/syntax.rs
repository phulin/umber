//! Small scalar grammars consumed inside the sole retained input reader.
use super::*;
use std::ops::ControlFlow;
use tex_state::meaning::MeaningWord;

/// Inline capacity for TeX/e-TeX/pdfTeX's current keyword vocabulary.
///
/// The longest production keyword is 13 characters. Longer or non-ASCII
/// spellings are rejected because they are outside the canonical vocabulary.
const KEYWORD_PREFIX_INLINE_CAPACITY: usize = 13;

#[derive(Default)]
struct OptionalEquals {
    equals: bool,
}

impl OptionalEquals {
    #[inline(always)]
    fn accept(&mut self, character: Option<(char, Catcode)>) -> ControlFlow<(), bool> {
        match character {
            Some((' ', Catcode::Space)) => ControlFlow::Continue(true),
            // §405 compares the actual other-category '=' token.
            Some(('=', Catcode::Other)) => {
                self.equals = true;
                ControlFlow::Break(())
            }
            _ => ControlFlow::Continue(false),
        }
    }
}

impl<'state, 'host, G, Facts: crate::CommandHostFacts<G>>
    crate::NativeWordConsumer<CommandProcessor<'state, 'host, G, Facts>, G, Facts>
    for OptionalEquals
{
    const RESIDENT_TERMINALS: bool = true;
    const RESIDENT_LITERALS: bool = true;

    #[inline(always)]
    fn resident_ready(&self) -> bool {
        // This grammar depends only on the current token, not on host entry facts.
        true
    }

    #[inline(always)]
    #[allow(clippy::too_many_arguments)] // Disjoint input-preserving execution owners and spelling.
    fn resident_terminal(
        &mut self,
        _context: &mut tex_state::CommandContext<'_, G>,
        _fuel: &mut crate::CommandFuel,
        _effects: &mut tex_state::diagnostic::DiagnosticEffects,
        _facts: &mut Facts,
        _entered_line: u32,
        _macro_origin: Option<tex_state::token::OriginId>,
        _spelling: TracedTokenWord,
        meaning: &MeaningWord<G>,
    ) -> ControlFlow<(), bool> {
        self.accept(native_character(meaning))
    }

    #[inline(always)]
    fn consume(
        &mut self,
        processor: &mut CommandProcessor<'state, 'host, G, Facts>,
        spelling: TracedTokenWord,
        meaning: &MeaningWord<G>,
        _code: u64,
        _control_sequence: Option<Symbol>,
        exceptional: Option<&CurrentCommand<G>>,
    ) -> Result<ControlFlow<()>, CommandError> {
        match self.accept(native_character(meaning)) {
            ControlFlow::Continue(true) => Ok(ControlFlow::Continue(())),
            ControlFlow::Break(()) => Ok(ControlFlow::Break(())),
            ControlFlow::Continue(false) => {
                processor.back_consumed_word(spelling, exceptional)?;
                Ok(ControlFlow::Break(()))
            }
        }
    }
}

struct Keyword<'a> {
    bytes: &'a [u8],
    // §407's link(backup_head): only spelling required by a failed match.
    matched: smallvec::SmallVec<[crate::input::BackedUpToken; KEYWORD_PREFIX_INLINE_CAPACITY]>,
}

impl Keyword<'_> {
    #[inline(always)]
    fn accept(
        &mut self,
        spelling: TracedTokenWord,
        character: Option<(char, Catcode)>,
        control_sequence: Option<Symbol>,
    ) -> ControlFlow<(), bool> {
        let letter = self.bytes[self.matched.len()];
        if control_sequence.is_none()
            && matches!(character, Some((ch, _)) if ch.eq_ignore_ascii_case(&char::from(letter)))
        {
            self.matched.push(crate::input::BackedUpToken { spelling });
            if self.matched.len() == self.bytes.len() {
                return ControlFlow::Break(());
            }
        } else if self.matched.is_empty() && matches!(character, Some((_, Catcode::Space))) {
            // Leading spaces are consumed without advancing the prefix.
        } else {
            return ControlFlow::Continue(false);
        }
        ControlFlow::Continue(true)
    }
}

impl<'state, 'host, G, Facts: crate::CommandHostFacts<G>>
    crate::NativeWordConsumer<CommandProcessor<'state, 'host, G, Facts>, G, Facts> for Keyword<'_>
{
    const RESIDENT_TERMINALS: bool = true;
    const RESIDENT_LITERALS: bool = true;

    #[inline(always)]
    fn resident_ready(&self) -> bool {
        // Expansion changes input, but cannot invalidate the matched prefix.
        true
    }

    #[inline(always)]
    #[allow(clippy::too_many_arguments)] // Disjoint input-preserving execution owners and spelling.
    fn resident_terminal(
        &mut self,
        _context: &mut tex_state::CommandContext<'_, G>,
        _fuel: &mut crate::CommandFuel,
        _effects: &mut tex_state::diagnostic::DiagnosticEffects,
        _facts: &mut Facts,
        _entered_line: u32,
        _macro_origin: Option<tex_state::token::OriginId>,
        spelling: TracedTokenWord,
        meaning: &MeaningWord<G>,
    ) -> ControlFlow<(), bool> {
        self.accept(
            spelling,
            native_character(meaning),
            spelling.token_word().control_sequence(),
        )
    }

    #[inline(always)]
    fn consume(
        &mut self,
        processor: &mut CommandProcessor<'state, 'host, G, Facts>,
        spelling: TracedTokenWord,
        meaning: &MeaningWord<G>,
        _code: u64,
        control_sequence: Option<Symbol>,
        exceptional: Option<&CurrentCommand<G>>,
    ) -> Result<ControlFlow<()>, CommandError> {
        match self.accept(spelling, native_character(meaning), control_sequence) {
            ControlFlow::Continue(true) => Ok(ControlFlow::Continue(())),
            ControlFlow::Break(()) => Ok(ControlFlow::Break(())),
            ControlFlow::Continue(false) => {
                processor.back_consumed_word(spelling, exceptional)?;
                Ok(ControlFlow::Break(()))
            }
        }
    }
}

impl<G, Facts: crate::CommandHostFacts<G>> CommandProcessor<'_, '_, G, Facts> {
    /// Consumes TeX82 §405's other-category optional equals sign, after spaces.
    pub fn scan_optional_equals(&mut self) -> Result<bool, CommandError> {
        let mut equals = OptionalEquals::default();
        let result = self.consume_expanded_words(&mut equals);
        result.map_err(|error| self.finish_scalar_error(error))?;
        Ok(equals.equals)
    }

    /// TeX82 §407's `scan_keyword`, which tex.web writes as
    ///
    /// ```text
    /// p:=backup_head; link(p):=null; k:=str_start[s];
    /// while k<str_start[s+1] do
    ///   begin get_x_token;
    ///   if (cur_cs=0)and((cur_chr=so(str_pool[k]))or
    ///                    (cur_chr=so(str_pool[k])-"a"+"A")) then
    ///     begin store_new_token(cur_tok); incr(k);
    ///     end
    ///   else if (cur_cmd<>spacer)or(p<>backup_head) then
    ///     begin back_input;
    ///     if p<>backup_head then back_list(link(backup_head));
    ///     scan_keyword:=false; return;
    ///     end;
    ///   end;
    /// flush_list(link(backup_head)); scan_keyword:=true;
    /// ```
    ///
    /// Three properties of that loop are load-bearing and none of them is a
    /// special case.
    ///
    /// - The failed match restores the input as **two** levels, not one:
    ///   §325's `back_input` for the offending token, then §323's `back_list`
    ///   for the matched prefix pushed on top of it. Both pushes are
    ///   observable, and only the first carries a recovery record.
    /// - A spacer read while nothing has matched yet (`p=backup_head`) takes
    ///   neither branch: it is consumed, discarded for good, and `k` does not
    ///   advance. `scan_keyword` therefore skips leading spaces rather than
    ///   restoring them, and a spacer *after* a partial match is an ordinary
    ///   mismatch.
    /// - `cur_cs=0` restricts a match to a character token. A control
    ///   sequence `\let` to a character (or an active character) has the same
    ///   `cur_cmd`/`cur_chr` and still cannot spell a keyword letter.
    ///
    /// The comparison itself is on `cur_chr` alone, so a keyword letter
    /// matches under any category code, and `cur_chr-"a"+"A"` accepts the
    /// uppercase form of tex.web's all-lowercase keywords.
    #[inline(always)]
    pub fn scan_keyword(&mut self, keyword: &str) -> Result<bool, CommandError> {
        if !keyword.is_ascii() || keyword.len() > KEYWORD_PREFIX_INLINE_CAPACITY {
            return Err(CommandError::input_invariant());
        }
        if keyword.is_empty() {
            return Ok(true);
        }
        let mut keyword = Keyword {
            bytes: keyword.as_bytes(),
            matched: smallvec::SmallVec::new(),
        };
        if let Err(error) = self.consume_expanded_words(&mut keyword) {
            return Err(self.finish_scalar_error(error));
        }
        let matched_keyword = keyword.matched.len() == keyword.bytes.len();
        if !matched_keyword && !keyword.matched.is_empty() {
            // Back the offender first, then the prefix as a separate level.
            // At EOF only the actual prefix exists.
            self.back_list(keyword.matched);
        }
        // A complete match consumes its prefix outright.
        Ok(matched_keyword)
    }
}

#[cfg(test)]
mod tests;
