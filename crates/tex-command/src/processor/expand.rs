//! Ordinary expanded-command delivery.

use tex_state::env::banks::IntParam;
use tex_state::glue::{GlueSpec, Order};
use tex_state::ids::{MacroDefinitionId, OriginListId, TokenListId};
use tex_state::meaning::{ExpandablePrimitive, Meaning, MeaningFlags};
use tex_state::page::PageMark;
use tex_state::provenance::SynthesizedOriginKind;
use tex_state::scaled::Scaled;
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use crate::command::DeliveryStamp;
use crate::input::{
    BackedUpToken, BackupTreatment, InputLevelId, ReplayTrace, RetirementBehavior,
    SharedBackedUpBuffer, SharedTokenBuffer, TokenBehavior, TokenPayload,
};
use crate::macro_call::MacroArguments;
use crate::processor::status::ScannerStatus;
use crate::profile::CommandProfile;
use crate::{CommandError, CommandReplayDelivery, CurrentCommand};

use super::CommandProcessor;

#[cfg(any(test, feature = "instrumentation"))]
use crate::observation::{
    CommandDeliveryBoundary, CommandDeliveryRecord, CommandObservation, CommandProvenance,
    EffectRecord, InputReason, InputRecord, InputTransition, RecoveryKind, RecoveryRecord,
};

/// Stable pending-diagnostic identity for TeX.web's `Missing \\endcsname
/// inserted` recovery. Rendering belongs to the diagnostic milestone.
const MISSING_ENDCSNAME_DIAGNOSTIC: u64 = 0x6373_6e61_6d65_0001;

/// TeX82's decimal rendering for a scaled quantity, including its `pt` unit.
fn format_scaled(value: Scaled) -> String {
    let mut raw = i64::from(value.raw());
    let mut output = String::new();
    if raw < 0 {
        output.push('-');
        raw = -raw;
    }
    let unity = i64::from(Scaled::UNITY);
    output.push_str(&(raw / unity).to_string());
    output.push('.');
    let mut scaled = 10 * (raw % unity) + 5;
    let mut delta = 10;
    loop {
        if delta > unity {
            scaled += 0o100000 - 50_000;
        }
        output.push(char::from(
            b'0' + u8::try_from(scaled / unity).expect("scaled digit fits u8"),
        ));
        scaled = 10 * (scaled % unity);
        delta *= 10;
        if scaled <= delta {
            break;
        }
    }
    output.push_str("pt");
    output
}

fn format_glue(value: GlueSpec, unit: &str) -> String {
    let mut output = format_scaled(value.width);
    replace_scaled_unit(&mut output, unit);
    for (label, component, order) in [
        (" plus ", value.stretch, value.stretch_order),
        (" minus ", value.shrink, value.shrink_order),
    ] {
        if component.raw() == 0 {
            continue;
        }
        output.push_str(label);
        let mut component = format_scaled(component);
        replace_scaled_unit(&mut component, unit);
        output.push_str(component.trim_end_matches(unit));
        output.push_str(match order {
            Order::Normal => unit,
            Order::Fil => "fil",
            Order::Fill => "fill",
            Order::Filll => "filll",
        });
    }
    output
}

fn replace_scaled_unit(value: &mut String, unit: &str) {
    if unit != "pt" {
        value.truncate(value.len() - "pt".len());
        value.push_str(unit);
    }
}

/// Which of TeX82 §380's two expanded-fetch procedures is driving delivery.
///
/// `get_x_token` and `x_token` agree on every command but one. §380's
/// `get_x_token` disposes of an `end_template` itself --
/// `cur_cs:=frozen_endv; cur_cmd:=endv; goto done` -- rewriting the live
/// command without touching the input stack. `x_token` has no such case: it
/// calls §366 `expand` for everything above `max_command`, and §375's
/// ``@<Insert a token containing |frozen_endv|@>`` is
/// `cur_tok:=cs_token_flag+frozen_endv; back_input`, so a backup level is
/// pushed and `x_token`'s own `get_next` rereads the token as a fresh raw
/// `endv` delivery.
///
/// The difference is observable, not cosmetic: the `x_token` form emits a
/// backup push, its recovery record, and a raw `endv` delivery that the
/// `get_x_token` form never produces, and it leaves the backup level to be
/// retired after `endv` has been acted on. Callers must therefore say which
/// procedure they are, never inherit a default.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ExpandedFetch {
    /// §380's `get_x_token`, reached from §1030's `big_switch`.
    GetXToken,
    /// §380's `x_token`: §1038's `main_loop_lookahead` after its bare
    /// `get_next`, and §1152's active-character treatment, both of which
    /// enter expansion with a command already in hand.
    XToken,
}

impl CommandProcessor<'_> {
    /// Delivers one ordinary expanded command through TeX.web's `get_x_token`.
    ///
    /// This is the sole production expanded loop. Expansion mutates the
    /// canonical command state and restarts here; it never returns a
    /// push-bearing dispatch result or enters a second interpreter.
    pub fn get_x_token(&mut self) -> Result<Option<CurrentCommand>, CommandError> {
        self.get_x_token_from(None, ExpandedFetch::GetXToken)
    }

    /// TeX.web §381's `x_token` entered with `cur_cmd`/`cur_chr` already set.
    ///
    /// §381 does not begin with `get_next`: it expands whatever the caller
    /// left in the current command and only then reads on. Ordinary delivery
    /// leaves nothing, which is [`Self::get_x_token`]; §1152 loads an active
    /// character's meaning directly and passes it here, so that meaning is
    /// expanded without ever having been delivered raw.
    fn get_x_token_from(
        &mut self,
        mut pending: Option<CurrentCommand>,
        fetch: ExpandedFetch,
    ) -> Result<Option<CurrentCommand>, CommandError> {
        self.command.transient.active_expansion_depth += 1;
        let result = loop {
            match self.expanded_delivery(pending.take(), fetch)? {
                Some(CommandReplayDelivery::Command(command)) => break Ok(Some(command)),
                Some(CommandReplayDelivery::Completed(_)) => continue,
                None => break Ok(None),
            }
        };
        self.command.transient.active_expansion_depth -= 1;
        result
    }

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
        origin: OriginId,
    ) -> Result<(), CommandError> {
        let spelling = TracedTokenWord::pack(
            Token::Char {
                ch,
                cat: Catcode::Active,
            },
            origin,
        );
        let stamp = DeliveryStamp::new(0, 0, self.next_delivery_sequence);
        self.next_delivery_sequence = self.next_delivery_sequence.wrapping_add(1);
        let command = CurrentCommand::resolve(spelling, stamp, None, false, &mut self.state);
        let Some(settled) = self.get_x_token_from(Some(command), ExpandedFetch::XToken)? else {
            return Ok(());
        };
        // §325 needs only `cur_tok`; the settled token is `x_token`'s result
        // rather than a delivery this call is undoing, exactly as in §326.
        self.back_input_saved(settled)
    }

    /// TeX82 §404's `<Get the next non-blank non-relax non-call token>`:
    /// `repeat get_x_token until (cur_cmd<>spacer)and(cur_cmd<>relax)`.
    ///
    /// This is the shared spelling of that module, used by §403's
    /// `scan_left_brace`, §1078, §1084, §1151's `scan_math`, §1160's
    /// non-radical `scan_delimiter`, §1211's `prefixed_command`, §1226 and
    /// §1270's `scan_optional_equals`. It differs from §406's
    /// `<Get the next non-blank non-call token>` only by also skipping
    /// `\relax`, and the two are not interchangeable: §1160 classifies the
    /// token it stops on, so a `\relax` that reached it as a command rather
    /// than as a skipped filler would scan as an invalid delimiter.
    pub(crate) fn next_non_blank_non_relax_x_token(
        &mut self,
    ) -> Result<Option<CurrentCommand>, CommandError> {
        loop {
            let Some(command) = self.get_x_token()? else {
                return Ok(None);
            };
            if !matches!(
                command.meaning(),
                Meaning::CharToken {
                    cat: Catcode::Space,
                    ..
                } | Meaning::Relax
            ) {
                return Ok(Some(command));
            }
        }
    }

    /// Delivers one expanded command or the completion of an executor-owned
    /// stored replay episode.
    ///
    /// Completion is published after the command machine has retired and
    /// observed the exact stored level, but before it resumes the enclosing
    /// source.  Callers must finish the corresponding isolated execution
    /// lifecycle before requesting another delivery.
    pub fn get_x_token_with_replay_completion(
        &mut self,
    ) -> Result<Option<CommandReplayDelivery>, CommandError> {
        self.command.transient.active_expansion_depth += 1;
        let result = self.get_x_token_scalar();
        self.command.transient.active_expansion_depth -= 1;
        result
    }

    /// Delivers one command through TeX82 §1038's `main_loop_lookahead`.
    ///
    /// `main_control`'s inner character loop (§1034) never returns to
    /// `big_switch`'s `get_x_token` between adjacent characters. §1038 fetches
    /// the next command with a bare `get_next` -- "set only `cur_cmd` and
    /// `cur_chr`, for speed" -- and jumps straight back into the loop when
    /// that raw command is `letter`, `other_char`, or `char_given`. Only a
    /// raw command outside that set reaches `x_token`, which is the sole
    /// reason a run of ordinary characters produces one raw delivery each and
    /// no expanded delivery at all.
    ///
    /// `char_num` is deliberately *not* in the raw set: §1038 accepts it only
    /// after `x_token`, because `\char` can be reached by expansion.
    pub fn main_loop_lookahead(&mut self) -> Result<Option<CommandReplayDelivery>, CommandError> {
        self.command.transient.active_expansion_depth += 1;
        let result = self.main_loop_lookahead_scalar();
        self.command.transient.active_expansion_depth -= 1;
        result
    }

    fn main_loop_lookahead_scalar(
        &mut self,
    ) -> Result<Option<CommandReplayDelivery>, CommandError> {
        let Some(delivery) = self.get_next_with_replay_completion()? else {
            return Ok(None);
        };
        let CommandReplayDelivery::Command(command) = delivery else {
            return Ok(Some(delivery));
        };
        if is_main_loop_character(command.meaning()) {
            return Ok(Some(CommandReplayDelivery::Command(command)));
        }
        self.expanded_delivery(Some(command), ExpandedFetch::XToken)
    }

    fn get_x_token_scalar(&mut self) -> Result<Option<CommandReplayDelivery>, CommandError> {
        self.expanded_delivery(None, ExpandedFetch::GetXToken)
    }

    /// TeX.web §380's expanded-fetch loop, in whichever of its two forms
    /// `fetch` names, optionally entered with the raw command §1038's
    /// lookahead has already fetched.
    fn expanded_delivery(
        &mut self,
        mut pending: Option<CurrentCommand>,
        fetch: ExpandedFetch,
    ) -> Result<Option<CommandReplayDelivery>, CommandError> {
        loop {
            let command = match pending.take() {
                Some(command) => command,
                None => {
                    let Some(delivery) = self.get_next_with_replay_completion()? else {
                        return Ok(None);
                    };
                    let CommandReplayDelivery::Command(command) = delivery else {
                        return Ok(Some(delivery));
                    };
                    command
                }
            };
            if matches!(
                command.meaning(),
                Meaning::ExpandablePrimitive(ExpandablePrimitive::EndTemplate)
            ) {
                // This loop's raw fetch is `get_next_with_replay_completion`,
                // which is §341's body without §342's tail, so §342's
                // consequence runs here through the same single helper
                // `get_next` and `get_token` use. `Ok(None)` is §789's
                // `goto restart`: the ⟨v_j⟩ template is live and no reader
                // ever sees the delimiter. Only frozen end-template input
                // from v-template exhaustion falls through to §380 below.
                let Some(mut command) = self.insert_alignment_entry_v_template(command)? else {
                    continue;
                };
                if fetch == ExpandedFetch::XToken {
                    // §366 `expand` has no `end_template` shortcut: it routes
                    // straight to §375, which backs up a `frozen_endv` token
                    // for this loop's own `get_next` to reread.
                    self.insert_frozen_endv()?;
                    continue;
                }
                command.convert_end_template_to_endv(self.state.frozen_endv_token());
                #[cfg(any(test, feature = "instrumentation"))]
                self.observe_expanded_delivery(&command);
                return Ok(Some(CommandReplayDelivery::Command(command)));
            }
            if !is_expandable(command.meaning()) {
                #[cfg(any(test, feature = "instrumentation"))]
                self.observe_expanded_delivery(&command);
                return Ok(Some(CommandReplayDelivery::Command(command)));
            }
            // TeX82 §394 aborts a non-`\long` macro call after its recovery
            // bookkeeping, then resumes the enclosing expanded-token loop.
            // A user paragraph has been backed up for that loop; an EOF
            // recovery paragraph was consumed by the failed match instead.
            match self.expand(command) {
                // TeX82 §394 resumes expanded delivery after both an ordinary
                // runaway paragraph and §23's outer-validity recovery has
                // aborted a macro match. The latter leaves the recovered
                // outer token in backup input for its normal reread.
                Ok(())
                | Err(CommandError::ParagraphInMacroArgument)
                | Err(CommandError::OuterInMacroArgument) => {}
                Err(error) => return Err(error),
            }
        }
    }

    #[cfg(any(test, feature = "instrumentation"))]
    pub(crate) fn observe_expanded_delivery(&mut self, command: &CurrentCommand) {
        let (command_name, command_operand) =
            crate::observation::canonical_current_command_identity_for_profile(
                self.command.profile(),
                command,
            );
        let spelling = self.observed_command_spelling(command);
        self.observe(CommandObservation::Command(CommandDeliveryRecord {
            boundary: CommandDeliveryBoundary::Expanded,
            spelling,
            command: command_name,
            command_operand,
            provenance: CommandProvenance::from_stamp(
                command.delivery_stamp(),
                command.origin(),
                command.direct_source_provenance(),
            ),
        }));
    }

    /// TeX82 §375's ``@<Insert a token containing |frozen_endv|@>``:
    ///
    /// ```text
    /// begin cur_tok:=cs_token_flag+frozen_endv; back_input;
    /// end
    /// ```
    ///
    /// This is §366 `expand`'s entire `end_template` case, and the reason
    /// §780 installs *two* frozen `\endtemplate` control sequences: the one
    /// stored in a template (`frozen_end_template`, command code
    /// `end_template`) is `>outer_call`, so §336's `check_outer_validity`
    /// still catches a template that ends inside an unfinished scan, and only
    /// once it has been delivered is it replaced by `frozen_endv`, whose
    /// command code is the ordinary unexpandable `endv`.
    ///
    /// §325's stack-conservation loop stops at a `v_template` level, so the
    /// exhausted template stays on the stack underneath this backup and
    /// retires only after `endv` has been acted on.
    pub(crate) fn insert_frozen_endv(&mut self) -> Result<(), CommandError> {
        let frozen_endv = self.state.frozen_endv_token();
        self.back_input_token(TracedTokenWord::pack(frozen_endv, OriginId::UNKNOWN))
    }

    /// TeX.web's scalar `expand`: each case changes the active input/state
    /// directly, then returns to [`Self::get_x_token_scalar`].
    pub(crate) fn expand(&mut self, command: CurrentCommand) -> Result<(), CommandError> {
        self.command.expansion.cumulative_expansions =
            self.command.expansion.cumulative_expansions.wrapping_add(1);
        match command.meaning() {
            Meaning::ExpandablePrimitive(primitive)
                if crate::conditionals::ConditionalKind::from_primitive(primitive).is_some() =>
            {
                self.expand_conditional(command, false)
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Unless) => {
                self.expand_unless(command)
            }
            Meaning::ExpandablePrimitive(
                primitive @ (ExpandablePrimitive::Else
                | ExpandablePrimitive::Or
                | ExpandablePrimitive::Fi),
            ) => self.expand_conditional_delimiter(command, primitive),
            Meaning::Macro { .. } => {
                self.macro_call(command)?;
                Ok(())
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::NoExpand) => self.expand_noexpand(),
            Meaning::ExpandablePrimitive(ExpandablePrimitive::ExpandAfter) => {
                self.expand_expandafter()
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::CsName) => {
                self.expand_csname(command)
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::String) => {
                self.expand_string(command)
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Meaning) => {
                self.expand_meaning(command)
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Number) => {
                self.expand_number(command, false)
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::RomanNumeral) => {
                self.expand_number(command, true)
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::The) => self.expand_the(command),
            Meaning::ExpandablePrimitive(ExpandablePrimitive::FontName) => {
                self.expand_fontname(command)
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Input) => self.expand_input(command),
            Meaning::ExpandablePrimitive(ExpandablePrimitive::EndInput) => self.expand_endinput(),
            Meaning::ExpandablePrimitive(ExpandablePrimitive::JobName) => {
                let job_name = self.host.job_name().to_owned();
                self.push_rendered_text(&job_name, command.origin());
                Ok(())
            }
            Meaning::ExpandablePrimitive(
                primitive @ (ExpandablePrimitive::TopMark
                | ExpandablePrimitive::FirstMark
                | ExpandablePrimitive::BotMark
                | ExpandablePrimitive::SplitFirstMark
                | ExpandablePrimitive::SplitBotMark),
            ) => self.expand_mark(primitive),
            Meaning::ExpandablePrimitive(
                primitive @ (ExpandablePrimitive::TopMarks
                | ExpandablePrimitive::FirstMarks
                | ExpandablePrimitive::BotMarks
                | ExpandablePrimitive::SplitFirstMarks
                | ExpandablePrimitive::SplitBotMarks),
            ) => self.expand_mark_class(primitive),
            Meaning::ExpandablePrimitive(primitive) => {
                Err(CommandError::UnsupportedExpandablePrimitive(primitive))
            }
            _ => Err(CommandError::input_invariant()),
        }
    }

    /// TeX.web's `\noexpand`: read normally, then replay exactly one target
    /// from a backed-up level carrying the non-sticky suppression treatment.
    fn expand_noexpand(&mut self) -> Result<(), CommandError> {
        let target = self
            .get_token_with_normal_scanner_status()?
            .ok_or(CommandError::input_invariant())?;
        self.back_input_with_treatment(target, BackupTreatment::SuppressExpandableControlSequence)
    }

    /// Reads one token with TeX82's temporary `scanner_status := normal`
    /// scope, restoring the complete prior scanner state before returning.
    ///
    /// Both `\noexpand` (§25) and `conv_toks`'s `\string`/`\meaning` cases
    /// (§27) need this scope: their operand is delivered normally even while
    /// an enclosing `\edef` is collecting replacement text.
    fn get_token_with_normal_scanner_status(
        &mut self,
    ) -> Result<Option<CurrentCommand>, CommandError> {
        if matches!(self.command.scanner.status(), ScannerStatus::Normal) {
            return self.get_token();
        }

        let prior = self.command.begin_scanner_status(ScannerStatus::Normal);
        self.observe_scanner_status_transition(
            prior.status().clone(),
            self.command.scanner.status().clone(),
        );
        let target = self.get_token();
        self.observe_scanner_status_transition(
            self.command.scanner.status().clone(),
            prior.status().clone(),
        );
        self.command.restore_scanner_status(prior);
        target
    }

    /// TeX.web's `\expandafter`: preserve the first token, expand (or back
    /// up) the second token, then put the first token above the resulting
    /// input. The first delivery is intentionally replayed through an
    /// explicit backed-up level because it is no longer the latest delivery.
    fn expand_expandafter(&mut self) -> Result<(), CommandError> {
        let first = self.get_token()?.ok_or(CommandError::input_invariant())?;
        let second = self.get_token()?.ok_or(CommandError::input_invariant())?;
        if is_expandable(second.meaning()) {
            self.expand(second)?;
        } else {
            self.back_input(second)?;
        }
        self.replay_expandafter_first(first)?;
        Ok(())
    }

    /// TeX.web's `\\csname`: collect ordinary expanded character commands
    /// until the inaccessible `\\endcsname` boundary, then inject the one
    /// named control-sequence token through normal input delivery.
    fn expand_csname(&mut self, opener: CurrentCommand) -> Result<(), CommandError> {
        let mut name = String::new();
        loop {
            let Some(command) = self.get_x_token()? else {
                return Err(CommandError::input_invariant());
            };
            match command.meaning() {
                Meaning::ExpandablePrimitive(ExpandablePrimitive::EndCsName) => break,
                Meaning::CharToken { ch, .. } => name.push(ch),
                _ => {
                    self.back_error(command, MISSING_ENDCSNAME_DIAGNOSTIC)?;
                    break;
                }
            }
        }

        let symbol = self.state.intern_relaxed_control_sequence(&name);
        let origin = self
            .state
            .synthesized_origin(SynthesizedOriginKind::Expansion, opener.origin());
        self.back_input_token(TracedTokenWord::pack(Token::Cs(symbol), origin))
    }

    /// `\\string` observes spelling, never an effective control-sequence meaning.
    fn expand_string(&mut self, opener: CurrentCommand) -> Result<(), CommandError> {
        let target = self
            .get_token_with_normal_scanner_status()?
            .ok_or(CommandError::input_invariant())?;
        self.push_rendered_text(
            &string_text(&self.state, target.spelling().semantic_token()),
            opener.origin(),
        );
        Ok(())
    }

    fn expand_meaning(&mut self, opener: CurrentCommand) -> Result<(), CommandError> {
        let target = self
            .get_token_with_normal_scanner_status()?
            .ok_or(CommandError::input_invariant())?;
        self.push_rendered_text(&meaning_text(&self.state, &target), opener.origin());
        Ok(())
    }

    fn expand_number(&mut self, opener: CurrentCommand, roman: bool) -> Result<(), CommandError> {
        let value = self.scan_integer()?.value;
        let text = if roman {
            roman_numeral(value)
        } else {
            value.to_string()
        };
        self.push_rendered_text(&text, opener.origin());
        Ok(())
    }

    /// Expands TeX82 `the_toks` after command-owned internal-quantity scanning.
    ///
    /// The internal scanner owns a primitive register's `scan_eight_bit_int`
    /// episode.  In particular, `\\the\\count21` must deliver both index digits
    /// before it backs up the next source token and installs rendered output.
    /// Reaching into the target meaning here would leave that index to a later
    /// scanner and changes the observable input ordering.
    fn expand_the(&mut self, opener: CurrentCommand) -> Result<(), CommandError> {
        let Some(target) = self.scan_internal_value()? else {
            return Err(CommandError::input_invariant());
        };
        self.expand_the_value(opener.origin(), target.value)
    }

    /// Installs one TeX82 §467 `ins_the_toks` result.
    ///
    /// §465's `the_toks` produces a token list for every `cur_val_level`: the
    /// scalar levels through `@<Convert |cur_val| to a token list@>`, `ident_val`
    /// as the font's own control-sequence token, and `tok_val` as a copy of the
    /// register or parameter. §467 then hands _all_ of them to the same
    /// `ins_list`, so none of the three may install a differently classified
    /// input level.
    pub(crate) fn expand_the_value(
        &mut self,
        opener: OriginId,
        value: crate::InternalValue,
    ) -> Result<(), CommandError> {
        if let Some(text) = render_the_value(value) {
            self.push_rendered_text(&text, opener);
        } else {
            match value {
                // §466 copies the register's list rather than sharing its
                // reference, which Umber's immutable stored list already is:
                // reassigning the register cannot mutate this payload.
                crate::InternalValue::Tokens { tokens } => {
                    let first = self.state.tokens(tokens).first().copied();
                    self.insert_expansion_list(
                        TokenPayload::Stored {
                            tokens,
                            origins: OriginListId::EMPTY,
                        },
                        first,
                    );
                }
                crate::InternalValue::Font(symbol) => {
                    self.push_rendered_tokens(vec![Token::Cs(symbol)], opener);
                }
                _ => unreachable!("non-token internal values are rendered above"),
            }
        }
        Ok(())
    }

    fn expand_fontname(&mut self, opener: CurrentCommand) -> Result<(), CommandError> {
        let target = self.get_token()?.ok_or(CommandError::input_invariant())?;
        let Meaning::Font(font) = target.meaning() else {
            return Err(CommandError::input_invariant());
        };
        self.push_rendered_text(&self.state.font_name(font), opener.origin());
        Ok(())
    }

    fn expand_input(&mut self, opener: CurrentCommand) -> Result<(), CommandError> {
        let _input = self.open_registered_input()?;
        #[cfg(any(test, feature = "instrumentation"))]
        self.observe(CommandObservation::Effect(EffectRecord {
            kind: "input",
            detail: _input.file_name.name,
            tokens: None,
        }));
        let _ = opener;
        Ok(())
    }

    fn expand_endinput(&mut self) -> Result<(), CommandError> {
        self.command
            .end_current_source_after_current_line()
            .then_some(())
            .ok_or(CommandError::input_invariant())
    }

    fn expand_mark(&mut self, primitive: ExpandablePrimitive) -> Result<(), CommandError> {
        self.push_mark_text(self.state.page_mark(page_mark(primitive)));
        Ok(())
    }

    fn expand_mark_class(&mut self, primitive: ExpandablePrimitive) -> Result<(), CommandError> {
        let class = self.scan_integer()?.value;
        let class = u16::try_from(class).unwrap_or(0);
        self.push_mark_text(self.state.page_mark_class(page_mark(primitive), class));
        Ok(())
    }

    /// Installs TeX82 §386's `mark_text` level for `\\topmark` and its kin.
    ///
    /// §386 is `begin_token_list(cur_mark[cur_chr], mark_text)`, a distinct
    /// §307 token type from §467's `inserted`: a mark's text is the stored list
    /// itself, never a copy handed back through `ins_list`.
    fn push_mark_text(&mut self, tokens: TokenListId) {
        let level = self.command.push_token_level(
            TokenPayload::Stored {
                tokens,
                origins: OriginListId::EMPTY,
            },
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::Stored(crate::input::StoredReplayReason::Mark),
        );
        #[cfg(not(any(test, feature = "instrumentation")))]
        let _ = level;
        #[cfg(any(test, feature = "instrumentation"))]
        self.observe(CommandObservation::Input(InputRecord {
            transition: InputTransition::Push,
            reason: InputReason::Mark,
            source_name: None,
            level: level.0,
            position: 0,
        }));
    }

    /// Installs TeX82 §470 `conv_toks` output as an inserted recovery level.
    ///
    /// Conversion output is not an ordinary token-list replay: §470 ends with
    /// `ins_list(link(temp_head))`, so it carries §307's `inserted` token type.
    /// Keeping that identity on the live input frame makes both retirement and
    /// detached observation follow the actual input transition, rather than
    /// asking a trace adapter to recognize rendered text later.
    fn push_rendered_text(&mut self, text: &str, parent: OriginId) {
        self.push_rendered_tokens(
            text.chars()
                .map(|ch| Token::Char {
                    ch,
                    cat: if ch == ' ' {
                        tex_state::token::Catcode::Space
                    } else {
                        tex_state::token::Catcode::Other
                    },
                })
                .collect(),
            parent,
        );
    }

    fn push_rendered_tokens(&mut self, tokens: Vec<Token>, parent: OriginId) {
        let origin = self
            .state
            .synthesized_origin(SynthesizedOriginKind::ValueRendering, parent);
        let first = tokens.first().copied();
        let tokens = tokens
            .into_iter()
            .map(|token| TracedTokenWord::pack(token, origin))
            .collect::<Vec<_>>();
        self.insert_expansion_list(
            TokenPayload::Transient(SharedTokenBuffer::new(tokens)),
            first,
        );
    }

    /// Performs TeX82 §323's `ins_list` for one expansion result.
    ///
    /// Every expansion that hands tokens back to the scanner -- §467's
    /// `ins_the_toks` and §470's `conv_toks` -- reaches the input stack through
    /// this one macro, so they share one installation here rather than each
    /// choosing its own token type. `first` is the inserted list's leading
    /// token: §323's trace seam reports the current token of the level it just
    /// pushed, and an empty inserted list has none to report.
    fn insert_expansion_list(&mut self, payload: TokenPayload, first: Option<Token>) {
        let level = self.command.push_token_level(
            payload,
            TokenBehavior::Recovery,
            RetirementBehavior::Pop,
            ReplayTrace::Inserted,
        );
        #[cfg(not(any(test, feature = "instrumentation")))]
        {
            let _ = (level, first);
        }
        #[cfg(any(test, feature = "instrumentation"))]
        {
            self.observe(CommandObservation::Input(InputRecord {
                transition: InputTransition::Recovery,
                reason: InputReason::Recovery,
                source_name: None,
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

    fn replay_expandafter_first(&mut self, command: CurrentCommand) -> Result<(), CommandError> {
        self.conserve_input_stack()?;
        self.undo_alignment_delivery(&command);
        let level = self.command.push_token_level(
            TokenPayload::BackedUp(SharedBackedUpBuffer::new(vec![BackedUpToken {
                spelling: command.spelling(),
                source_provenance: command.source_provenance(),
            }])),
            TokenBehavior::BackedUp(BackupTreatment::Ordinary),
            RetirementBehavior::Pop,
            ReplayTrace::BackedUp,
        );
        #[cfg(not(any(test, feature = "instrumentation")))]
        let _ = level;
        #[cfg(any(test, feature = "instrumentation"))]
        {
            // TeX82 §25's `back_input` is part of the expandafter lifecycle:
            // after expanding its second token, the saved first token must be
            // a visible ordinary backup before raw delivery resumes.
            self.observe(CommandObservation::Input(InputRecord {
                transition: InputTransition::Backup,
                reason: InputReason::Backup,
                source_name: None,
                level: level.0,
                position: 0,
            }));
            self.observe(CommandObservation::Recovery(RecoveryRecord {
                kind: RecoveryKind::Backup,
                tokens: vec![self.observed_command_spelling(&command)],
            }));
        }
        Ok(())
    }

    /// Creates one invocation provenance node and atomically exposes its
    /// activation/body ownership pair to the input stack.
    ///
    /// The scalar macro matcher owns argument matching and calls this only
    /// after it has completed every range. Nested invocations use the live
    /// activation chain, not a replay trace, as their provenance parent.
    #[allow(dead_code)] // consumed by the ordered scalar macro matcher issue
    pub(crate) fn push_macro_activation(
        &mut self,
        definition: MacroDefinitionId,
        call_site: OriginId,
        arguments: MacroArguments,
        replacement_tokens: TokenListId,
        replacement_origins: OriginListId,
    ) -> InputLevelId {
        let definition_origin = self
            .state
            .macro_definition_provenance(definition)
            .definition_origin();
        let parent = self.command.parameters.parent_invocation();
        let invocation =
            self.state
                .macro_invocation_origin(definition, call_site, definition_origin, parent);
        self.command.push_macro_activation(
            definition,
            arguments,
            invocation,
            replacement_tokens,
            replacement_origins,
        )
    }
}

pub(crate) fn render_the_value(value: crate::InternalValue) -> Option<String> {
    match value {
        crate::InternalValue::Integer(value) => Some(value.to_string()),
        crate::InternalValue::Dimension(value) => Some(format_scaled(value)),
        crate::InternalValue::Glue(value) => Some(format_glue(value, "pt")),
        crate::InternalValue::MuGlue(value) => Some(format_glue(value, "mu")),
        crate::InternalValue::Font(_) => None,
        crate::InternalValue::Tokens { .. } => None,
    }
}

/// Classifies TeX82 §323's inserted-list trace seam by its leading token.
///
/// §289's `cs_token_flag` splits the token space in two, and §323 reports the
/// inserted list's first token on whichever side of it that token falls:
/// control sequences (including §353's active characters and tex.web's frozen
/// sentinels) are one recovery operation, character and `out_param` tokens the
/// other. Deriving the classification from the observed token keeps every
/// caller of `ins_list` -- rendered conversion text, a copied token register, a
/// font identifier -- on the same rule instead of asserting one per call site.
#[cfg(any(test, feature = "instrumentation"))]
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

/// TeX82 §1038's raw-accepted set: `letter`, `other_char`, and `char_given`.
///
/// These are exactly the three commands §1034's inner loop can continue on
/// without expanding, so they are the only ones the lookahead delivers
/// straight out of `get_next`.
pub(crate) fn is_main_loop_character(meaning: Meaning) -> bool {
    matches!(
        meaning,
        Meaning::CharToken {
            cat: Catcode::Letter | Catcode::Other,
            ..
        } | Meaning::CharGiven(_)
    )
}

fn is_expandable(meaning: Meaning) -> bool {
    matches!(meaning, Meaning::Macro { .. })
        || matches!(
            meaning,
            Meaning::ExpandablePrimitive(primitive)
                if primitive != ExpandablePrimitive::EndCsName
        )
}

fn page_mark(primitive: ExpandablePrimitive) -> PageMark {
    match primitive {
        ExpandablePrimitive::TopMark | ExpandablePrimitive::TopMarks => PageMark::Top,
        ExpandablePrimitive::FirstMark | ExpandablePrimitive::FirstMarks => PageMark::First,
        ExpandablePrimitive::BotMark | ExpandablePrimitive::BotMarks => PageMark::Bot,
        ExpandablePrimitive::SplitFirstMark | ExpandablePrimitive::SplitFirstMarks => {
            PageMark::SplitFirst
        }
        ExpandablePrimitive::SplitBotMark | ExpandablePrimitive::SplitBotMarks => {
            PageMark::SplitBot
        }
        _ => unreachable!("only mark primitives reach page_mark"),
    }
}

pub(crate) fn string_text(state: &tex_state::CommandContext<'_>, token: Token) -> String {
    match token {
        Token::Cs(symbol) => {
            let mut text = String::new();
            let escape = state.int_param(IntParam::ESCAPE_CHAR);
            if let Some(ch) = char::from_u32(u32::try_from(escape).unwrap_or(u32::MAX)) {
                text.push(ch);
            }
            text.push_str(state.resolve(symbol));
            text
        }
        Token::Char { ch, .. } => ch.to_string(),
        Token::Param(slot) => format!("#{slot}"),
        Token::Frozen(_) => "\\relax".to_owned(),
    }
}

pub(crate) fn meaning_text(
    state: &tex_state::CommandContext<'_>,
    command: &CurrentCommand,
) -> String {
    match command.meaning() {
        Meaning::Undefined => "undefined".to_owned(),
        Meaning::Relax => "\\relax".to_owned(),
        Meaning::CharToken { ch, cat } => format!("the character {ch} (catcode {})", cat as u8),
        Meaning::CharGiven(ch) => format!("the character {ch}"),
        Meaning::CountRegister(index) => format!("\\count{index}"),
        Meaning::ToksRegister(index) => format!("\\toks{index}"),
        Meaning::IntParam(index) => format!("integer parameter {index}"),
        Meaning::TokParam(index) => format!("token parameter {index}"),
        Meaning::Font(font) => format!("select font {}", state.font_name(font)),
        Meaning::Macro { flags, definition } => {
            // `\\meaning` prints the definition, not a live macro-body input
            // frame.  A completed macro call retires its activation and body,
            // whereas the definition's parameter and replacement lists remain
            // immutable state owned by the meaning.
            let macro_meaning = state.macro_definition(definition);
            let prefix = match (
                flags.contains(MeaningFlags::LONG),
                flags.contains(MeaningFlags::OUTER),
            ) {
                (false, false) => "macro".to_owned(),
                // TeX82's `print_cmd_chr` uses `print_esc` for these command
                // identities, so the escape character is part of conv_toks'
                // inserted spelling, not source provenance.
                (true, false) => "\\long macro".to_owned(),
                (false, true) => "\\outer macro".to_owned(),
                (true, true) => "\\long\\outer macro".to_owned(),
            };
            format!(
                "{prefix}:{}->{}",
                token_list_text(state, macro_meaning.parameter_text()),
                token_list_text(state, macro_meaning.replacement_text()),
            )
        }
        Meaning::ExpandablePrimitive(_) | Meaning::UnexpandablePrimitive(_) => command
            .control_sequence()
            .map(|symbol| format!("\\{}", state.resolve(symbol)))
            .unwrap_or_else(|| "primitive".to_owned()),
        other => format!("{other:?}"),
    }
}

fn token_list_text(state: &tex_state::CommandContext<'_>, tokens: TokenListId) -> String {
    state
        .tokens(tokens)
        .iter()
        .copied()
        .map(|token| token_list_token_text(state, token))
        .collect()
}

/// TeX82's `show_token_list` representation used by `\\meaning` distinguishes
/// a printed control word from following letter tokens with one space.  That
/// delimiter belongs to the rendered definition, not to source input.
fn token_list_token_text(state: &tex_state::CommandContext<'_>, token: Token) -> String {
    let Token::Cs(symbol) = token else {
        return string_text(state, token);
    };
    let name = state.resolve(symbol);
    let mut text = format!("\\{name}");
    if name.chars().last().is_some_and(char::is_alphabetic) {
        text.push(' ');
    }
    text
}

fn roman_numeral(value: i32) -> String {
    if value <= 0 {
        return String::new();
    }
    let mut remaining = value;
    let mut output = String::new();
    for (amount, glyph) in [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ] {
        while remaining >= amount {
            output.push_str(glyph);
            remaining -= amount;
        }
    }
    output
}

#[cfg(test)]
mod tests;

/// Future-relevant expansion facts.
///
/// Per-request fuel is deliberately absent: it is call-local and recreated
/// when an executor step is retried. Caches and profiling likewise belong to
/// [`crate::CommandRuntime`].
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) struct ExpansionState {
    pub(crate) cumulative_expansions: u64,
    pub(crate) next_resource_resolution: u64,
    pub(crate) pending_diagnostics: Vec<u64>,
    pub(crate) observed_dependencies: Vec<u64>,
    pub(crate) semantic_barriers: Vec<u64>,
    pub(crate) profile: CommandProfile,
}
