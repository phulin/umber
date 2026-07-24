//! Executor-facing token-list assignment scans.
//!
//! The command processor owns all operand consumption for these assignments:
//! register numbers, the optional equals sign, and `scan_toks` collection.
//! Replay receives only the frozen completed request and can therefore apply
//! the aggregate mutation without acquiring a second input path.

use tex_state::TracedTokenList;
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use crate::{CommandError, CommandProcessor};

/// A completed TeX token-register assignment operand.
///
/// The register number remains signed until the executor applies its usual
/// TeX register-range validation. The token list is already frozen by the
/// command-owned `scan_toks` collector.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedTokenRegisterAssignment {
    pub index: i32,
    pub tokens: TracedTokenList,
}

impl CommandProcessor<'_> {
    /// Scans the operand sequence of TeX82's `\toks` assignment.
    ///
    /// This follows `scan_int`, `scan_optional_equals`, and unexpanded
    /// `scan_toks` (TeX.web §§403 and 470). In particular, a non-equals token
    /// is backed up by the command processor before `scan_toks` begins.
    pub fn scan_token_register_assignment(
        &mut self,
    ) -> Result<ScannedTokenRegisterAssignment, CommandError> {
        let index = self.scan_integer()?.value;
        let _ = self.scan_optional_equals()?;
        let tokens = self.scan_balanced_text(false)?.tokens;
        Ok(ScannedTokenRegisterAssignment { index, tokens })
    }

    /// Scans a token-parameter assignment such as `\output={...}`.
    ///
    /// TeX82 retains the outer braces in token parameters, unlike `\toks`
    /// registers. The command processor owns both that representation choice
    /// and optional-equals consumption, so replay receives one frozen value.
    pub fn scan_token_parameter_assignment(&mut self) -> Result<TracedTokenList, CommandError> {
        let _ = self.scan_optional_equals()?;
        let scanned = self.scan_balanced_text(false)?;
        let mut tokens = Vec::new();
        tokens.push(TracedTokenWord::pack(
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            OriginId::UNKNOWN,
        ));
        tokens.extend(
            self.state
                .tokens(scanned.tokens.token_list())
                .iter()
                .copied()
                .map(|token| TracedTokenWord::pack(token, OriginId::UNKNOWN)),
        );
        tokens.push(TracedTokenWord::pack(
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
            OriginId::UNKNOWN,
        ));
        Ok(self.state.finish_traced_token_list(&tokens))
    }
}
