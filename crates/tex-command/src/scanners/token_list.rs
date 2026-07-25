//! Executor-facing token-list assignment scans.
//!
//! The command processor owns all operand consumption for these assignments:
//! register numbers, the optional equals sign, and `scan_toks` collection.
//! Replay receives only the frozen completed request and can therefore apply
//! the aggregate mutation without acquiring a second input path.

use tex_state::TracedTokenList;
use tex_state::meaning::{Meaning, UnexpandablePrimitive};
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use crate::{CommandError, CommandProcessor};

/// A completed TeX token-register assignment operand.
///
/// Both register numbers are TeX82 eight-bit quantities. The token list is
/// already frozen by the command-owned collector or copied from an internal
/// token-list value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedTokenRegisterAssignment {
    pub index: u16,
    pub tokens: TracedTokenList,
}

enum TokenListRightHandSide {
    Collected(TracedTokenList),
    Internal(TracedTokenList),
}

impl CommandProcessor<'_> {
    /// Scans the operand sequence of TeX82's `\toks` assignment.
    ///
    /// This follows `scan_eight_bit_int`, `scan_optional_equals`, and the
    /// internal-token-list branch before unexpanded `scan_toks` (TeX.web
    /// §§403 and 470). A non-internal RHS is backed up before the collector
    /// begins, preserving `scan_left_brace` recovery.
    pub fn scan_token_register_assignment(
        &mut self,
    ) -> Result<ScannedTokenRegisterAssignment, CommandError> {
        let index = self.scan_eight_bit_register_index()?;
        let _ = self.scan_optional_equals()?;
        let tokens = self.scan_token_list_right_hand_side()?.tokens();
        Ok(ScannedTokenRegisterAssignment { index, tokens })
    }

    /// Scans a token-parameter assignment such as `\output={...}`.
    ///
    /// TeX82 retains the outer braces in token parameters, unlike `\toks`
    /// registers. The command processor owns both that representation choice
    /// and optional-equals consumption, so replay receives one frozen value.
    pub fn scan_token_parameter_assignment(&mut self) -> Result<TracedTokenList, CommandError> {
        let _ = self.scan_optional_equals()?;
        let scanned = match self.scan_token_list_right_hand_side()? {
            TokenListRightHandSide::Collected(tokens) => tokens,
            TokenListRightHandSide::Internal(tokens) => return Ok(tokens),
        };
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
                .tokens(scanned.token_list())
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

    fn scan_token_list_right_hand_side(&mut self) -> Result<TokenListRightHandSide, CommandError> {
        let command = self.next_non_space_non_relax_x_token()?;
        let tokens = match command.meaning() {
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Toks) => {
                let index = self.scan_eight_bit_register_index()?;
                self.state.toks(index)
            }
            Meaning::ToksRegister(index) => self.state.toks(index),
            Meaning::TokParam(index) => self
                .state
                .tok_param(tex_state::env::banks::TokParam::new(index)),
            _ => {
                self.back_input(command)?;
                return Ok(TokenListRightHandSide::Collected(
                    self.scan_balanced_text(false)?.tokens,
                ));
            }
        };
        Ok(TokenListRightHandSide::Internal(
            TracedTokenList::synthetic(tokens),
        ))
    }

    fn next_non_space_non_relax_x_token(&mut self) -> Result<crate::CurrentCommand, CommandError> {
        loop {
            let command = self.get_x_token()?.ok_or(CommandError::InputInvariant)?;
            match command.meaning() {
                Meaning::CharToken {
                    cat: Catcode::Space,
                    ..
                }
                | Meaning::Relax => continue,
                _ => return Ok(command),
            }
        }
    }
}

impl TokenListRightHandSide {
    fn tokens(self) -> TracedTokenList {
        match self {
            Self::Collected(tokens) | Self::Internal(tokens) => tokens,
        }
    }
}
