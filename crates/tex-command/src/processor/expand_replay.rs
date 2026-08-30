//! Expansion-result construction and replay installation.

use tex_state::env::banks::IntParam;
use tex_state::token::{OriginId, Token, TracedTokenWord};

use crate::CommandError;
use crate::input::{PackedTokenSpanHandle, ReplayTrace, RetirementBehavior, TokenBehavior};
use crate::observation::{
    CommandObservation, InputReason, InputRecord, InputTransition, RecoveryKind, RecoveryRecord,
};

use super::CommandProcessor;

impl<G> CommandProcessor<'_, '_, G> {
    pub(super) fn attempt_token_list_string_text(
        &mut self,
        tokens: crate::AttemptTokenListId,
    ) -> Result<String, CommandError> {
        let words = self
            .command
            .attempt
            .arena()
            .token_words(tokens)
            .map_err(crate::scan_toks::attempt_command_error)?
            .to_vec();
        let mut text = String::new();
        let _ = self.state.int_param(IntParam::ESCAPE_CHAR);
        for word in words {
            self.state
                .append_token_string_text(word.semantic_token(), &mut text);
        }
        Ok(text)
    }

    pub(super) fn attempt_token_list_bytes(
        &mut self,
        tokens: crate::AttemptTokenListId,
    ) -> Result<Vec<u8>, CommandError> {
        Ok(self
            .attempt_token_list_string_text(tokens)?
            .chars()
            .map(|ch| {
                u8::try_from(u32::from(ch))
                    .expect("pdfTeX profile expanded strings contain only byte characters")
            })
            .collect())
    }

    /// Installs TeX82 §470 `conv_toks` output as an inserted recovery level.
    ///
    /// Conversion output is not an ordinary token-list replay: §470 ends with
    /// `ins_list(link(temp_head))`, so it carries §307's `inserted` token type.
    /// Keeping that identity on the live input frame makes both retirement and
    /// detached observation follow the actual input transition, rather than
    /// asking a trace adapter to recognize rendered text later.
    pub(super) fn push_rendered_text(&mut self, text: &str, parent: OriginId) {
        self.push_rendered_tokens(
            text.chars().map(|ch| Token::Char {
                ch,
                cat: if ch == ' ' {
                    tex_state::token::Catcode::Space
                } else {
                    tex_state::token::Catcode::Other
                },
            }),
            parent,
        );
    }

    pub(super) fn push_rendered_tokens(
        &mut self,
        tokens: impl IntoIterator<Item = Token>,
        parent: OriginId,
    ) {
        let mut tokens = tokens.into_iter();
        let first = tokens.next();
        let payload = PackedTokenSpanHandle::transient(
            first
                .into_iter()
                .chain(tokens)
                .map(|token| TracedTokenWord::pack(token, parent)),
        );
        self.insert_expansion_list(payload, first);
    }

    /// Performs TeX82 §323's `ins_list` for one expansion result.
    ///
    /// Every expansion that hands tokens back to the scanner -- §467's
    /// `ins_the_toks` and §470's `conv_toks` -- reaches the input stack through
    /// this one macro, so they share one installation here rather than each
    /// choosing its own token type. `first` is the inserted list's leading
    /// token: §323's trace seam reports the current token of the level it just
    /// pushed, and an empty inserted list has none to report.
    pub(crate) fn insert_expansion_list<P: crate::input::PackedTokenSpanSource<G>>(
        &mut self,
        payload: P,
        first: Option<Token>,
    ) {
        self.insert_expansion_list_with_behavior(payload, first, TokenBehavior::Recovery);
    }

    fn insert_expansion_list_with_behavior<P: crate::input::PackedTokenSpanSource<G>>(
        &mut self,
        payload: P,
        first: Option<Token>,
        behavior: TokenBehavior,
    ) {
        let level = self.command.push_token_level(
            payload,
            behavior,
            RetirementBehavior::Pop,
            ReplayTrace::Inserted,
        );
        if self.is_observed() {
            self.observe(CommandObservation::Input(InputRecord {
                transition: InputTransition::Recovery,
                reason: InputReason::Recovery,
                source_name: None,
                source: None,
                level: level.0,
                position: 0,
            }));
            if let Some(first) = first {
                let observed = self.observed_token(TracedTokenWord::pack(first, OriginId::UNKNOWN));
                self.observe(CommandObservation::Recovery(RecoveryRecord {
                    kind: inserted_recovery_kind(&observed),
                    tokens: vec![observed],
                }));
            }
        }
    }
}

fn inserted_recovery_kind(token: &crate::observation::ObservedToken) -> RecoveryKind {
    use crate::observation::ObservedToken;
    match token {
        ObservedToken::Character { .. } | ObservedToken::Parameter(_) => {
            RecoveryKind::InsertedToken
        }
        ObservedToken::ControlSequence(_)
        | ObservedToken::MacroMatch
        | ObservedToken::MacroEndMatch
        | ObservedToken::FrozenEndTemplate
        | ObservedToken::FrozenEndV
        | ObservedToken::FrozenPrimitive(_)
        | ObservedToken::FrozenOther => RecoveryKind::InsertedControlSequence,
    }
}
