//! Synchronous native-word consumption by the actual continuing scalar scanner.

use super::{ExpandedReaderConsumer, ExpandedReaderRole};
use crate::{CommandError, CommandProcessor, CurrentCommand, DeliveryStatus};
use std::ops::ControlFlow;
use tex_state::interner::Symbol;
use tex_state::meaning::MeaningWord;
use tex_state::token::TracedTokenWord;

type ScalarConsumer<'a, Processor, G> = dyn FnMut(
        &mut Processor,
        TracedTokenWord,
        &MeaningWord<G>,
        Option<Symbol>,
        Option<&CurrentCommand<G>>,
    ) -> Result<ControlFlow<()>, CommandError>
    + 'a;

struct ScalarWords<'a, 'callback, Processor, G, const MAIN_CONTROL: bool> {
    consume: &'a mut ScalarConsumer<'callback, Processor, G>,
}

impl<'state, 'host, G, Facts: crate::CommandHostFacts<G>, const MAIN_CONTROL: bool>
    ExpandedReaderConsumer<CommandProcessor<'state, 'host, G, Facts>, G>
    for ScalarWords<'_, '_, CommandProcessor<'state, 'host, G, Facts>, G, MAIN_CONTROL>
{
    const NATIVE_TERMINAL: bool = true;

    fn native_terminal(
        &mut self,
        processor: &mut CommandProcessor<'state, 'host, G, Facts>,
        spelling: TracedTokenWord,
        meaning: &MeaningWord<G>,
        control_sequence: Option<Symbol>,
    ) -> Result<ControlFlow<()>, CommandError> {
        (self.consume)(processor, spelling, meaning, control_sequence, None)
    }

    fn terminal(
        &mut self,
        processor: &mut CommandProcessor<'state, 'host, G, Facts>,
        destination: &mut Option<CurrentCommand<G>>,
        status: DeliveryStatus,
    ) -> Result<ControlFlow<()>, CommandError> {
        let command = destination
            .as_ref()
            .ok_or_else(CommandError::input_invariant)?;
        if status == DeliveryStatus::AlignmentEndTemplate {
            processor.begin_scalar_alignment_v_template(command)?;
            return Ok(if MAIN_CONTROL && processor.is_observed() {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            });
        }
        (self.consume)(
            processor,
            command.spelling(),
            command.native_meaning(),
            command.control_sequence(),
            Some(command),
        )
    }
}

impl<G, Facts: crate::CommandHostFacts<G>> CommandProcessor<'_, '_, G, Facts> {
    pub fn consume_expanded_words(
        &mut self,
        consume: &mut ScalarConsumer<'_, Self, G>,
    ) -> Result<(), CommandError> {
        let mut destination = None;
        loop {
            let result = self.consume_expanded_with_mode::<false, false>(
                &mut destination,
                None,
                ExpandedReaderRole::Scanner,
                &mut ScalarWords::<_, _, false> { consume },
            )?;
            match result {
                DeliveryStatus::ReplayCompleted(_) => continue,
                DeliveryStatus::End
                | DeliveryStatus::Command
                | DeliveryStatus::PendingExpanded
                | DeliveryStatus::AlignmentClosingBrace => return Ok(()),
                _ => unreachable!("expanded scanner has no character consumer"),
            }
        }
    }

    /// Runs main control's native consumer inside the canonical reader.
    /// Accepted words never cross a command-product boundary. Real replay,
    /// EOF and error boundaries return to the caller; observation and TeX
    /// recovery lend their demanded record only during consumption.
    pub fn consume_main_control(
        &mut self,
        consume: &mut ScalarConsumer<'_, Self, G>,
    ) -> Result<DeliveryStatus, CommandError> {
        let mut destination = None;
        self.consume_expanded_with_mode::<false, false>(
            &mut destination,
            None,
            ExpandedReaderRole::MainControl,
            &mut ScalarWords::<_, _, true> { consume },
        )
        .map(|status| match status {
            DeliveryStatus::PendingExpanded | DeliveryStatus::AlignmentClosingBrace => {
                DeliveryStatus::Command
            }
            status => status,
        })
    }

    /// Executes the terminal consumer at TeX82 §1038's raw character
    /// lookahead. Only noncharacters enter x_token; adjacent characters own
    /// their raw delivery alone, including when observation is requested.
    pub fn consume_main_loop_lookahead(
        &mut self,
        consume: &mut ScalarConsumer<'_, Self, G>,
    ) -> Result<DeliveryStatus, CommandError> {
        let mut destination = None;
        self.consume_expanded_with_mode::<false, false>(
            &mut destination,
            None,
            ExpandedReaderRole::MainLoopLookahead,
            &mut ScalarWords::<_, _, true> { consume },
        )
        .map(|status| match status {
            DeliveryStatus::PendingExpanded | DeliveryStatus::AlignmentClosingBrace => {
                DeliveryStatus::Command
            }
            status => status,
        })
    }

    /// Ordinary synchronous consumers back up spelling; exceptional deliveries
    /// preserve their actual alignment correction and observed command identity.
    pub fn back_consumed_word(
        &mut self,
        spelling: TracedTokenWord,
        exceptional: Option<&CurrentCommand<G>>,
    ) -> Result<(), CommandError> {
        if let Some(command) = exceptional {
            self.back_input(*command)
        } else {
            self.back_input_token(spelling)
        }
    }

    /// Captures demanded diagnostic facts at the actual terminal consumer.
    pub fn capture_consumed_site(
        &self,
        spelling: TracedTokenWord,
        meaning: &MeaningWord<G>,
        control_sequence: Option<Symbol>,
        exceptional: Option<&CurrentCommand<G>>,
    ) -> tex_state::diagnostic::DiagnosticSite {
        if let Some(command) = exceptional {
            self.capture_diagnostic_site(Some(command))
        } else {
            self.capture_spelled_diagnostic_site(
                crate::command::CommandIdentity::from_meaning(meaning),
                spelling,
                control_sequence,
                meaning.resolve(),
            )
        }
    }
}

impl<G, Facts: crate::CommandHostFacts<G>> CommandProcessor<'_, '_, G, Facts> {
    /// TeX82 §1152's `@<Treat |cur_chr| as an active character@>`:
    ///
    /// ```text
    /// begin cur_cs:=cur_chr+active_base;
    /// cur_cmd:=eq_type(cur_cs); cur_chr:=equiv(cur_cs);
    /// x_token; back_input;
    /// end
    /// ```
    ///
    /// This is the whole of TeX's `\mathcode` escape hatch. §1155's
    /// `set_math_char` and §1151's `scan_math` both branch here when a
    /// character's `math_code` is `@'100000`, which is what makes plain
    /// TeX's ``\mathcode`\'="8000`` route `'` through the active `'` macro
    /// that builds `\prime` lists.
    ///
    /// The character is not backed up and reread. §1152 loads the
    /// `active_base + c` cell's meaning straight into `cur_cmd`/`cur_chr`,
    /// so there is no raw delivery for it at all: `x_token` expands that
    /// meaning in place -- observing a macro push, not a backup -- and only
    /// the unexpandable token expansion settles on is backed up, from where
    /// the caller rereads it. An active character bound to an unexpandable
    /// meaning still reaches §381's tail, so it is still observed as one
    /// expanded delivery and backed up unchanged.
    pub fn treat_as_active_character(
        &mut self,
        ch: char,
        origin: tex_state::token::OriginId,
    ) -> Result<(), CommandError> {
        use super::{ExpandedCommandAction, ExpansionDispatch, classify_native_command};
        use tex_state::token::{Catcode, Token};
        let spelling = TracedTokenWord::pack(
            Token::Char {
                ch,
                cat: Catcode::Active,
            },
            origin,
        );
        let (meaning, control_sequence) = self.resolve_token_word(spelling.token_word());
        let action = classify_native_command(spelling.token_word(), &meaning);
        if self.is_observed()
            || self.command.delivery_mode.requires_persistent_settlement()
            || matches!(
                action,
                ExpandedCommandAction::EndTemplate
                    | ExpandedCommandAction::Expand(ExpansionDispatch::Undefined)
            )
        {
            return self.treat_observed_or_exceptional_active_character(
                spelling,
                meaning,
                control_sequence,
            );
        }
        if action == ExpandedCommandAction::Return {
            #[cfg(feature = "profiling")]
            self.record_expanded_delivery();
            return self.back_input_token(spelling);
        }
        let mut consume = |processor: &mut Self,
                           spelling: TracedTokenWord,
                           _: &MeaningWord<G>,
                           _: Option<Symbol>,
                           exceptional: Option<&CurrentCommand<G>>| {
            processor.back_consumed_word(spelling, exceptional)?;
            Ok(ControlFlow::Break(()))
        };
        let mut destination = None;
        self.with_expanded_delivery(&mut destination, |processor, destination| {
            match action {
                ExpandedCommandAction::Expand(ExpansionDispatch::Macro) => {
                    let MeaningWord::Macro { flags, definition } = meaning else {
                        unreachable!("native active macro");
                    };
                    let name = control_sequence.ok_or_else(CommandError::input_invariant)?;
                    match processor.activate_read_macro(
                        spelling.token_word(),
                        origin,
                        flags,
                        definition,
                        name,
                    ) {
                        Ok(())
                        | Err(
                            CommandError::ParagraphInMacroArgument
                            | CommandError::OuterInMacroArgument,
                        ) => {}
                        Err(failure) => return Err(failure),
                    }
                }
                ExpandedCommandAction::Expand(ExpansionDispatch::Primitive(primitive)) => {
                    processor.expand_primitive(primitive, spelling, control_sequence, true)?;
                }
                _ => unreachable!("native active expansion"),
            }
            loop {
                let status = processor.expanded_delivery_loop::<false, false, false, _>(
                    destination,
                    None,
                    ExpandedReaderRole::Scanner,
                    &mut ScalarWords::<_, _, false> {
                        consume: &mut consume,
                    },
                )?;
                match status {
                    DeliveryStatus::ReplayCompleted(_) => continue,
                    DeliveryStatus::End
                    | DeliveryStatus::Command
                    | DeliveryStatus::PendingExpanded
                    | DeliveryStatus::AlignmentClosingBrace => return Ok(status),
                    _ => unreachable!("active expansion consumes the actual terminal word"),
                }
            }
        })
        .map(|_| ())
    }
}
