//! Shared stack-local state for non-suspending token scanners.
//!
//! The input owner and output owner stay with their callers.  This cursor is
//! only the semantic state which advances beside an already-admitted input
//! span: brace depth and the first-token facts needed by macro arguments.

use tex_state::token::{Catcode, TokenWord};

use crate::token_collector::ClassifiedToken;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ScannerCursor {
    brace_depth: u32,
    word_count: u32,
    rejects_non_long_paragraph: bool,
    outer_group_candidate: bool,
}

impl ScannerCursor {
    pub(crate) const fn brace_depth(self) -> u32 {
        self.brace_depth
    }

    pub(crate) const fn rejects_non_long_paragraph(self) -> bool {
        self.rejects_non_long_paragraph
    }

    pub(crate) const fn removable_outer_group(self) -> bool {
        self.outer_group_candidate && self.brace_depth == 0
    }

    /// Settles one scalar token after its caller has written it to the final
    /// sink. Classification, brace balance, and first-token state advance in
    /// this one cursor and are never represented in a second pending object.
    #[inline(always)]
    pub(crate) fn settle_argument(
        &mut self,
        token: ClassifiedToken,
        paragraph_checked: bool,
    ) -> u32 {
        self.rejects_non_long_paragraph |= token.rejects_non_long_paragraph(paragraph_checked);
        let delta = match token.spelling().literal_catcode() {
            Some(Catcode::BeginGroup) => 1_i8,
            Some(Catcode::EndGroup) => -1_i8,
            _ => 0,
        };
        if self.word_count == 0 {
            self.outer_group_candidate = delta == 1;
        } else if self.brace_depth == 0 {
            self.outer_group_candidate = false;
        }
        self.word_count = self.word_count.saturating_add(1);
        match delta {
            1 => self.brace_depth = self.brace_depth.saturating_add(1),
            -1 => self.brace_depth = self.brace_depth.saturating_sub(1),
            _ => {}
        }
        self.brace_depth
    }

    /// Settles a run whose admission predicate proved that every word is an
    /// ordinary non-brace character and not a paragraph command. The output
    /// sink and this cursor consequently advance once for the complete run.
    #[inline(always)]
    pub(crate) fn settle_plain_run(&mut self, count: u32) {
        if count == 0 {
            return;
        }
        if self.word_count == 0 || self.brace_depth == 0 {
            self.outer_group_candidate = false;
        }
        self.word_count = self.word_count.saturating_add(count);
    }

    /// TeX82 §477 balanced-body transition. The closing token which reaches
    /// depth zero belongs to the delimiter and is not written to the sink.
    #[inline(always)]
    pub(crate) fn settle_balanced(&mut self, token: ClassifiedToken) -> bool {
        if token.spelling_is_begin_group() {
            self.brace_depth = self.brace_depth.saturating_add(1);
        } else if token.spelling_is_end_group() && self.brace_depth != 0 {
            self.brace_depth -= 1;
        }
        token.spelling_is_end_group() && self.brace_depth == 0
    }

    pub(crate) fn open_balanced_body(&mut self) {
        self.brace_depth = 1;
    }

    /// Settles one balanced-body spelling without requiring a fully resolved
    /// command.  Expanded token collectors make this transition from the
    /// hot command word: only the literal catcode contributes to the body
    /// depth, exactly as [`Self::settle_balanced`] does for a classified
    /// command.
    #[inline(always)]
    pub(crate) fn settle_balanced_word(&mut self, word: TokenWord) -> bool {
        match word.literal_catcode() {
            Some(Catcode::BeginGroup) => {
                self.brace_depth = self.brace_depth.saturating_add(1);
            }
            Some(Catcode::EndGroup) if self.brace_depth != 0 => {
                self.brace_depth -= 1;
            }
            _ => {}
        }
        matches!(word.literal_catcode(), Some(Catcode::EndGroup)) && self.brace_depth == 0
    }
}
