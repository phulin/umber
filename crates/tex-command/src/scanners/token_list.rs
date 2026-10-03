//! Token-list operand grammar on the canonical command processor.
//! Target selection stays in its consuming assignment arm. Collected and
//! retained lists return their actual owners without assignment packets.

use tex_state::env::banks::TokParam;
use tex_state::interner::Symbol;
use tex_state::meaning::{Meaning, MeaningWord, UnexpandablePrimitive};
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use crate::scan_toks::ScanToksMode;
use crate::{AttemptTokenListId, CommandError, CommandProcessor};

impl<G, Facts: crate::CommandHostFacts<G>> CommandProcessor<'_, '_, G, Facts> {
    /// Scans optional equals and a token-list value into its actual owner.
    /// Copied lists retain their physical pointer even when empty. Newly
    /// collected empty text is null; only nonempty collected output text
    /// receives the enclosing brace pair required by TeX82 section 1226.
    pub fn scan_token_list_value(
        &mut self,
        owner: Symbol,
        enclose_collected: bool,
    ) -> Result<
        (
            Option<AttemptTokenListId>,
            Option<tex_state::TokenListId<G>>,
        ),
        CommandError,
    > {
        self.scan_optional_equals_retained().into_result()?;
        let mut value = None;
        self.consume_expanded_words(&mut |processor, spelling, meaning, _, exceptional| {
            let meaning = match meaning {
                MeaningWord::Static(word) => Meaning::from_runtime_word(*word),
                _ => Meaning::Undefined,
            };
            let collected = match meaning {
                Meaning::CharToken {
                    cat: Catcode::Space,
                    ..
                }
                | Meaning::Relax => {
                    return Ok(std::ops::ControlFlow::Continue(()));
                }

                Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Toks) => {
                    let index = processor
                        .scan_profile_register_index_retained()
                        .into_result()?;
                    let source = processor
                        .state
                        .token_register(index)
                        .expect("scanner produced an admitted token-register index")
                        .clone();
                    value = Some((None, source));
                    return Ok(std::ops::ControlFlow::Break(()));
                }
                Meaning::ToksRegister(index) => {
                    let source = processor
                        .state
                        .token_register(index)
                        .expect("meaning contains an admitted token-register index")
                        .clone();
                    value = Some((None, source));
                    return Ok(std::ops::ControlFlow::Break(()));
                }
                Meaning::TokParam(index) => {
                    let source = processor
                        .state
                        .token_parameter(TokParam::new(index))
                        .expect("meaning contains an admitted token-parameter index");
                    value = Some((None, source));
                    return Ok(std::ops::ControlFlow::Break(()));
                }
                Meaning::CharToken {
                    cat: Catcode::BeginGroup,
                    ..
                } => {
                    // TeX82 has already delivered the required opening brace to
                    // choose the non-internal branch. Back it up once, then let
                    // `scan_toks` install absorbing status before it redelivers
                    // that exact token. Re-scanning it through `scan_left_brace`
                    // would add a second raw delivery before that transition.
                    let primary = spelling.origin();
                    processor.back_consumed_word(spelling, exceptional)?;
                    processor
                        .scan_toks(ScanToksMode::GeneralAfterOpening {
                            expanded: false,
                            primary,
                            owner: Some(owner),
                        })?
                        .replacement_text
                }
                _ => {
                    processor.back_consumed_word(spelling, exceptional)?;
                    processor
                        .scan_toks(ScanToksMode::GeneralFor {
                            expanded: false,
                            owner,
                        })?
                        .replacement_text
                }
            };
            if processor
                .command
                .attempt
                .token_words(&collected)
                .map_err(crate::scan_toks::attempt_command_error)?
                .is_empty()
            {
                processor
                    .command
                    .attempt
                    .discard_token_list(collected)
                    .map_err(crate::scan_toks::attempt_command_error)?;
                value = Some((None, None));
                return Ok(std::ops::ControlFlow::Break(()));
            }
            if !enclose_collected {
                value = Some((Some(collected), None));
                return Ok(std::ops::ControlFlow::Break(()));
            }
            let writer = processor
                .command
                .attempt
                .allocate_token_buffer()
                .map_err(crate::scan_toks::attempt_command_error)?;
            processor
                .command
                .attempt
                .push_buffer_token(
                    &writer,
                    TracedTokenWord::pack(
                        Token::Char {
                            ch: '{',
                            cat: Catcode::BeginGroup,
                        },
                        OriginId::UNKNOWN,
                    ),
                )
                .map_err(crate::scan_toks::attempt_command_error)?;
            processor
                .command
                .attempt
                .consume_token_list_into_buffer(collected, &writer)
                .map_err(crate::scan_toks::attempt_command_error)?;
            processor
                .command
                .attempt
                .push_buffer_token(
                    &writer,
                    TracedTokenWord::pack(
                        Token::Char {
                            ch: '}',
                            cat: Catcode::EndGroup,
                        },
                        OriginId::UNKNOWN,
                    ),
                )
                .map_err(crate::scan_toks::attempt_command_error)?;
            value = Some((
                Some(
                    processor
                        .command
                        .attempt
                        .finish_token_buffer(writer)
                        .map_err(crate::scan_toks::attempt_command_error)?,
                ),
                None,
            ));
            Ok(std::ops::ControlFlow::Break(()))
        })?;
        value.ok_or_else(CommandError::input_invariant)
    }
}

#[cfg(test)]
mod tests;
