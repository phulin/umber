//! Shared stack-local state for non-suspending token scanners.
//!
//! The input owner and output owner stay with their callers.  This cursor is
//! only the semantic state which advances beside an already-admitted input
//! span: brace depth and the first-token facts needed by macro arguments.

use tex_state::token::{Catcode, TokenWord, TracedTokenWord};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ScannerCursor {
    brace_depth: u32,
    outer_group: OuterGroup,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum OuterGroup {
    #[default]
    Unstarted,
    Candidate,
    Disqualified,
}

impl ScannerCursor {
    pub(crate) const fn argument_started(self) -> bool {
        !matches!(self.outer_group, OuterGroup::Unstarted)
    }

    pub(crate) const fn brace_depth(self) -> u32 {
        self.brace_depth
    }

    pub(crate) const fn removable_outer_group(self) -> bool {
        matches!(self.outer_group, OuterGroup::Candidate) && self.brace_depth == 0
    }

    /// Advances the spelling facts needed to strip a single outer group.
    /// Paragraph rejection belongs to the matcher before this accepted write.
    #[inline(always)]
    pub(crate) fn settle_argument_word(&mut self, word: TokenWord) -> u32 {
        let delta = match word.literal_catcode() {
            Some(Catcode::BeginGroup) => 1_i8,
            Some(Catcode::EndGroup) => -1_i8,
            _ => 0,
        };
        self.settle_argument_delta(delta)
    }

    /// Settles an admitted spelling whose literal brace class is already known.
    #[inline(always)]
    pub(crate) fn settle_argument_delta(&mut self, delta: i8) -> u32 {
        match self.outer_group {
            OuterGroup::Unstarted => {
                self.outer_group = if delta == 1 {
                    OuterGroup::Candidate
                } else {
                    OuterGroup::Disqualified
                };
            }
            OuterGroup::Candidate if self.brace_depth == 0 => {
                self.outer_group = OuterGroup::Disqualified
            }
            OuterGroup::Candidate | OuterGroup::Disqualified => {}
        }
        match delta {
            1 => self.brace_depth = self.brace_depth.saturating_add(1),
            -1 => self.brace_depth = self.brace_depth.saturating_sub(1),
            _ => {}
        }
        self.brace_depth
    }

    /// TeX82 §477 balanced-body transition. The closing token which reaches
    /// depth zero belongs to the delimiter and is not written to the sink.
    #[inline(always)]
    pub(crate) fn settle_balanced(&mut self, token: TracedTokenWord) -> bool {
        if token.token_word().literal_catcode() == Some(Catcode::BeginGroup) {
            self.brace_depth = self.brace_depth.saturating_add(1);
        } else if token.token_word().literal_catcode() == Some(Catcode::EndGroup)
            && self.brace_depth != 0
        {
            self.brace_depth -= 1;
        }
        token.token_word().literal_catcode() == Some(Catcode::EndGroup) && self.brace_depth == 0
    }

    pub(crate) fn open_balanced_body(&mut self) {
        self.brace_depth = 1;
    }

    /// Settles a batched balanced-body run whose interior braces never closed
    /// the body.
    pub(crate) fn advance_balanced_depth(&mut self, delta: i32) {
        self.brace_depth = self
            .brace_depth
            .checked_add_signed(delta)
            .filter(|depth| *depth != 0)
            .expect("a replacement run never closes its balanced body");
    }
}
