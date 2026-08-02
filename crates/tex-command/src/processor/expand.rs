//! Ordinary expanded-command delivery.

use tex_state::env::banks::IntParam;
use tex_state::glue::{GlueSpec, Order};
use tex_state::ids::{MacroDefinitionId, OriginListId, TokenListId};
use tex_state::interner::ControlSequenceKind;
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
use crate::{
    CommandError, CommandReplayDelivery, CurrentCommand, RegisteredSourceKind, SourceNameClass,
    SourceRegistration,
};

use super::CommandProcessor;

use crate::observation::{
    CommandDeliveryBoundary, CommandDeliveryRecord, CommandObservation, CommandProvenance,
    EffectRecord, InputReason, InputRecord, InputTransition, RecoveryKind, RecoveryRecord,
};

/// Stable pending-diagnostic identity for TeX.web's `Missing \\endcsname
/// inserted` recovery. Rendering belongs to the diagnostic milestone.
pub(crate) const MISSING_ENDCSNAME_DIAGNOSTIC: u64 = 0x6373_6e61_6d65_0001;

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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ProtectedMacroHandling {
    Expand,
    Preserve,
}

impl CommandProcessor<'_> {
    /// Delivers one ordinary expanded command through TeX.web's `get_x_token`.
    ///
    /// This is the sole production expanded loop. Expansion mutates the
    /// canonical command state and restarts here; it never returns a
    /// push-bearing dispatch result or enters a second interpreter.
    pub fn get_x_token(&mut self) -> Result<Option<CurrentCommand>, CommandError> {
        self.apply_error_stop_recovery()?;
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
            match self.expanded_delivery(
                pending.take(),
                fetch,
                true,
                ProtectedMacroHandling::Expand,
            )? {
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
    pub fn next_non_blank_non_relax_x_token(
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

    /// TeX82 §406's `<Get the next non-blank non-call token>`:
    /// `repeat get_x_token until cur_cmd<>spacer`.
    ///
    /// Unlike §404's similarly named helper, this preserves `\relax`. The
    /// returned command is the exact expanded delivery that stopped the
    /// loop: callers such as §1045's `\ignorespaces` dispatch it in place
    /// without backing it up or rebuilding its provenance.
    pub fn next_non_blank_x_token(&mut self) -> Result<Option<CurrentCommand>, CommandError> {
        loop {
            let Some(command) = self.get_x_token()? else {
                return Ok(None);
            };
            if !matches!(
                command.meaning(),
                Meaning::CharToken {
                    cat: Catcode::Space,
                    ..
                }
            ) {
                return Ok(Some(command));
            }
        }
    }

    /// TeX82 §§785/791's shared alignment lookahead fetch.
    ///
    /// TeX82's `get_x_token` commits the terminal expanded command before
    /// `init_col` backs an ordinary command up. The backup is later read
    /// again above its u-template, producing a second raw/expanded delivery.
    /// Spacers skipped by §406 are complete deliveries and are committed here
    /// normally.
    ///
    /// e-TeX 2.6 change sections [37.785] and [37.791] replace that helper
    /// with `get_x_or_protected`. Its terminal unexpandable command comes
    /// straight from `get_token`, so neither skipped spacers nor a consumed
    /// `\noalign`, `\crcr`, `\omit`, or closing brace has an expanded
    /// delivery. A protected macro is likewise terminal and is backed up as
    /// the first command of the next cell.
    pub fn next_alignment_lookahead(
        &mut self,
    ) -> Result<Option<(CurrentCommand, bool)>, CommandError> {
        loop {
            let expansions_before = self.command.expansion.cumulative_expansions;
            self.command.transient.active_expansion_depth += 1;
            let etex_protected_fetch = self.command.profile().capabilities().supports_etex();
            let result = self.expanded_delivery(
                None,
                ExpandedFetch::GetXToken,
                false,
                if etex_protected_fetch {
                    ProtectedMacroHandling::Preserve
                } else {
                    ProtectedMacroHandling::Expand
                },
            );
            self.command.transient.active_expansion_depth -= 1;
            let Some(delivery) = result? else {
                return Ok(None);
            };
            let CommandReplayDelivery::Command(command) = delivery else {
                continue;
            };
            if matches!(
                command.meaning(),
                Meaning::CharToken {
                    cat: Catcode::Space,
                    ..
                }
            ) {
                if !etex_protected_fetch {
                    self.observe_expanded_delivery(&command);
                }
                continue;
            }
            // A command that §406 fetched directly has completed the ordinary
            // get_x_token boundary already. Only the result reached through
            // §381's expansion loop remains pending across §789's back_input.
            let expanded_through_call =
                self.command.expansion.cumulative_expansions != expansions_before;
            if !expanded_through_call && !etex_protected_fetch {
                self.observe_expanded_delivery(&command);
            }
            return Ok(Some((
                command,
                expanded_through_call && !etex_protected_fetch,
            )));
        }
    }

    /// Commits a terminal TeX82 lookahead delivery that alignment control
    /// consumes instead of passing to an ordinary `back_input` branch.
    pub fn commit_alignment_lookahead_delivery(&mut self, command: &CurrentCommand) {
        self.observe_expanded_delivery(command);
    }

    /// Completes TeX82 §§785/791's ordinary `align_peek`/`init_col` branch.
    ///
    /// A command reached through §380's expansion loop is still pending only
    /// in Umber's observer transport. TeX has already completed
    /// `get_x_token`, so its expanded delivery precedes §789's `back_input`;
    /// the later replay above the u-template is a distinct delivery.
    pub fn back_alignment_lookahead(
        &mut self,
        command: CurrentCommand,
        pending_expanded_delivery: bool,
    ) -> Result<(), CommandError> {
        if pending_expanded_delivery {
            self.commit_alignment_lookahead_delivery(&command);
        }
        self.back_input(command)
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
        self.apply_error_stop_recovery()?;
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
        self.apply_error_stop_recovery()?;
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
        self.expanded_delivery(
            Some(command),
            ExpandedFetch::XToken,
            true,
            ProtectedMacroHandling::Expand,
        )
    }

    fn get_x_token_scalar(&mut self) -> Result<Option<CommandReplayDelivery>, CommandError> {
        self.expanded_delivery(
            None,
            ExpandedFetch::GetXToken,
            true,
            ProtectedMacroHandling::Expand,
        )
    }

    /// TeX.web §380's expanded-fetch loop, in whichever of its two forms
    /// `fetch` names, optionally entered with the raw command §1038's
    /// lookahead has already fetched.
    fn expanded_delivery(
        &mut self,
        mut pending: Option<CurrentCommand>,
        fetch: ExpandedFetch,
        observe_final: bool,
        protected_macros: ProtectedMacroHandling,
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
                if observe_final {
                    self.observe_expanded_delivery(&command);
                }
                return Ok(Some(CommandReplayDelivery::Command(command)));
            }
            if !is_expandable_command(&command)
                || (protected_macros == ProtectedMacroHandling::Preserve
                    && matches!(
                        command.meaning(),
                        Meaning::Macro { flags, .. }
                            if flags.contains(MeaningFlags::PROTECTED)
                    ))
            {
                if observe_final {
                    self.observe_expanded_delivery(&command);
                }
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

    pub(crate) fn observe_expanded_delivery(&mut self, command: &CurrentCommand) {
        let (command_name, command_operand) =
            crate::observation::canonical_current_command_identity_for_profile(
                self.command.profile(),
                command,
            );
        let spelling = self.observed_command_spelling(command);
        let semantic_operand = crate::observation::canonical_sparse_register_operand(
            self.command.profile(),
            command.meaning(),
        );
        self.observe(CommandObservation::Command(CommandDeliveryRecord {
            boundary: CommandDeliveryBoundary::Expanded,
            spelling,
            command: command_name,
            command_operand,
            semantic_operand,
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
        self.command.expansion.cumulative_expansions = self
            .command
            .expansion
            .cumulative_expansions
            .saturating_add(1);
        // TeX82 §367 traces non-macro expandable commands inside `expand`,
        // before the primitive consumes operands or changes the input stack.
        // Macros and `end_template` take §366's other two branches and do not
        // cross this diagnostic boundary.
        if self
            .state
            .int_param(tex_state::env::banks::IntParam::TRACING_COMMANDS)
            > 1
            && matches!(
                command.meaning(),
                Meaning::ExpandablePrimitive(primitive)
                    if primitive != ExpandablePrimitive::EndTemplate
            )
        {
            self.print_command_trace(crate::PrintCommand::from_current(&command));
        }
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
                match self.macro_call(command)? {
                    crate::macro_call::MacroCallOutcome::Activated => {}
                    crate::macro_call::MacroCallOutcome::PrefixMismatchRecovered => {}
                }
                Ok(())
            }
            // TeX82 §375's `end_template` case replaces the inaccessible
            // sentinel that ended a v-template with the distinct frozen
            // `endv` token. Neither sentinel is a user-installable primitive;
            // §780 gives them only frozen control-sequence slots.
            Meaning::ExpandablePrimitive(ExpandablePrimitive::EndTemplate) => {
                self.insert_frozen_endv()
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
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Unexpanded) => {
                self.expand_unexpanded()
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Detokenize) => {
                self.expand_detokenize(command)
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Scantokens) => {
                self.expand_scantokens()
            }
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
            // e-TeX 2.6 etex.ch §3211 installs `\eTeXrevision` as a
            // `convert` command; §1387 prints the immutable revision string
            // through TeX82 §470's ordinary conversion-token path.
            Meaning::ExpandablePrimitive(ExpandablePrimitive::ETeXRevision) => {
                self.push_rendered_text(".6", command.origin());
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
            // TeX82 §207 puts `undefined_cs` immediately above
            // `max_command`, so it reaches §366's `expand` and §367's
            // `othercases`. §370 reports the error and returns without
            // inserting a replacement token; §380 then restarts its one
            // expanded-fetch loop at the following input token.
            Meaning::Undefined => {
                let context = self.command.output_open_context(&self.state);
                self.command
                    .semantic_diagnostics
                    .push(crate::CommandSemanticDiagnostic::UndefinedControlSequence { context });
                if !self.command.profile().capabilities().supports_etex() {
                    // TeX82 §370 still owns the recoverable user-visible
                    // error above. The pinned e-TeX 2.6 observer has no
                    // diagnostic seam at that error site, so its detached
                    // event stream advances directly to the next input
                    // transition.
                    self.observe_command_diagnostic("undefined_control_sequence", &command);
                }
                Ok(())
            }
            _ => Err(CommandError::input_invariant()),
        }
    }

    /// e-TeX 2.6 etex.ch §53a `pseudo_start`.
    fn expand_scantokens(&mut self) -> Result<(), CommandError> {
        let scanned = self.scan_toks(crate::scan_toks::ScanToksMode::GeneralText {
            purpose: "scantokens",
        })?;
        let mut text =
            token_list_string_text(&mut self.state, scanned.replacement_text.token_list());
        let newline = self.state.int_param(IntParam::NEWLINE_CHAR);
        if let Some(newline) = char::from_u32(u32::try_from(newline).unwrap_or(u32::MAX))
            && newline != '\n'
        {
            text = text
                .chars()
                .map(|ch| if ch == newline { '\n' } else { ch })
                .collect();
        }
        // etex.ch appends one sentinel space before splitting the string.
        // The pseudo-input representation is line-oriented, so a final LF
        // expresses that final record without becoming source text itself.
        text.push('\n');
        let every_eof = self
            .state
            .tok_param_option(tex_state::env::banks::TokParam::EVERY_EOF)
            .map(tex_state::TracedTokenList::synthetic);
        let tracing_scantokens = self.state.int_param(IntParam::TRACING_SCAN_TOKENS);
        let level = self
            .command
            .open_scantokens(
                SourceRegistration::new(RegisteredSourceKind::Generated, text.into_bytes()),
                every_eof,
                scantokens_numeric_name(tracing_scantokens),
            )
            .map_err(|_| CommandError::input_invariant())?;
        let source = self
            .command
            .active_source_snapshot()
            .ok_or(CommandError::input_invariant())?;
        // e-TeX 2.6 etex.ch §53a assigns `name=19` while
        // `\tracingscantokens>0`, and `name=18` otherwise. TeX82 §48's
        // initial character strings render those names as `^^S` and `^^R`.
        let source_name = scantokens_source_name(tracing_scantokens);
        self.observe(CommandObservation::GeneratedSource(
            crate::GeneratedSourceRecord {
                name: source_name.to_owned(),
                source,
            },
        ));
        self.observe(CommandObservation::Input(InputRecord {
            transition: InputTransition::Push,
            reason: InputReason::Source,
            // e-TeX 2.6 etex.ch §53a `pseudo_start` first calls
            // `begin_file_reading`, which establishes and observes the new
            // level while its §328 default is still `name=0`. Only after
            // that transition does e-TeX assign the pseudo-file name used
            // during tokenization and retirement. The level remains
            // file-like in command state, but its push is the transient
            // terminal-class transition the reference engine performs.
            source_name: Some(SourceNameClass::Terminal),
            level: level.0,
            position: 0,
        }));
        Ok(())
    }

    /// e-TeX 2.6 etex.ch §53a's `\detokenize`.
    ///
    /// `scan_general_text` collects without expansion, `token_show` renders
    /// the frozen spelling exactly as for `\scantokens`, and `str_toks`
    /// projects the resulting string to category-10 spaces and category-12
    /// other characters.
    fn expand_detokenize(&mut self, opener: CurrentCommand) -> Result<(), CommandError> {
        let scanned = self.scan_toks(crate::scan_toks::ScanToksMode::GeneralText {
            purpose: "detokenize",
        })?;
        let text = token_list_string_text(&mut self.state, scanned.replacement_text.token_list());
        self.push_rendered_text(&text, opener.origin());
        Ok(())
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
        let name = self.scan_csname_characters()?;
        let symbol = self.state.intern_relaxed_control_sequence(&name);
        let origin = self
            .state
            .synthesized_origin(SynthesizedOriginKind::Expansion, opener.origin());
        self.back_input_token(TracedTokenWord::pack(Token::Cs(symbol), origin))
    }

    /// Collects TeX82 §372's expanded character list through `\\endcsname`.
    ///
    /// e-TeX 2.6 etex.ch [17.4765--4779] deliberately reuses this exact
    /// name-building scan for `\\ifcsname`; only the subsequent hash-table
    /// operation differs.
    pub(crate) fn scan_csname_characters(&mut self) -> Result<String, CommandError> {
        let mut name = String::new();
        loop {
            let Some(command) = self.get_x_token()? else {
                return Err(CommandError::input_invariant());
            };
            match command.meaning() {
                Meaning::ExpandablePrimitive(ExpandablePrimitive::EndCsName) => break,
                Meaning::CharToken { ch, .. } => name.push(ch),
                _ => {
                    let name = print_esc_text(&self.state, "endcsname");
                    self.back_error_reporting(
                        command,
                        MISSING_ENDCSNAME_DIAGNOSTIC,
                        format!("Missing {name} inserted"),
                        &[
                            "The control sequence marked <to be read again> should",
                            "not appear between \\csname and \\endcsname.",
                        ],
                    )?;
                    break;
                }
            }
        }

        Ok(name)
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
        let target = self.scan_internal_value_or_zero()?;
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

    /// TeX82 §471's `font_name_code: scan_font_ident` and §472's
    /// `print(font_name[cur_val])`.
    ///
    /// `\fontname` owns no operand reading of its own: §577's
    /// `scan_font_ident` is the only routine that turns a command into a
    /// font, including its invalid-identifier recovery to `nullfont`.
    fn expand_fontname(&mut self, opener: CurrentCommand) -> Result<(), CommandError> {
        let font = self.scan_font_selector()?;
        self.push_rendered_text(&self.state.font_name(font), opener.origin());
        Ok(())
    }

    fn expand_input(&mut self, opener: CurrentCommand) -> Result<(), CommandError> {
        if self.command.name_in_progress() {
            // TeX82 §§378/527: restore the recursively encountered `\input`,
            // then place inaccessible `frozen_relax` above it. The active
            // filename scan stops empty at the relax; ordinary expansion
            // later reaches the restored input.
            let origin = self
                .state
                .synthesized_origin(SynthesizedOriginKind::Expansion, opener.origin());
            let frozen_relax = TracedTokenWord::pack(Token::frozen_relax(), origin);
            self.insert_expansion_list(
                TokenPayload::Transient(SharedTokenBuffer::new(vec![
                    frozen_relax,
                    opener.spelling(),
                ])),
                Some(Token::frozen_relax()),
            );
            return Ok(());
        }
        let _input = self.open_registered_input()?;
        observe!(
            self,
            CommandObservation::Effect(EffectRecord {
                kind: "input",
                detail: _input.file_name.packed(),
                source: Some(crate::observation::OpenedSourceSnapshot {
                    id: _input.source,
                    bytes: _input.bytes,
                }),
                tokens: None,
            }),
        );
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
        if let Some(tokens) = self.state.page_mark_value(page_mark(primitive)) {
            self.push_mark_text(tokens);
        }
        Ok(())
    }

    fn expand_mark_class(&mut self, primitive: ExpandablePrimitive) -> Result<(), CommandError> {
        // e-TeX 2.6 `etex.ch` [26.1178] uses the same
        // `scan_register_num` as numbered marks and sparse registers.
        let class = self.scan_extended_register_index()?;
        // e-TeX 2.6 etex.ch [25.386] makes class zero an exact alias for
        // TeX82's `cur_mark`, including its null-versus-empty pointer state.
        let tokens = self
            .state
            .page_mark_class_value(page_mark(primitive), class);
        if let Some(tokens) = tokens {
            self.push_mark_text(tokens);
        }
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
        observe!(
            self,
            CommandObservation::Input(InputRecord {
                transition: InputTransition::Push,
                reason: InputReason::Mark,
                source_name: None,
                level: level.0,
                position: 0,
            }),
        );
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
    pub(crate) fn insert_expansion_list(&mut self, payload: TokenPayload, first: Option<Token>) {
        self.insert_expansion_list_with_behavior(payload, first, TokenBehavior::Recovery);
    }

    fn insert_expansion_list_with_behavior(
        &mut self,
        payload: TokenPayload,
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
        if self.is_observed() {
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
        name: tex_state::interner::Symbol,
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
            name,
            definition,
            arguments,
            invocation,
            replacement_tokens,
            replacement_origins,
        )
    }
}

/// e-TeX 2.6 etex.ch §53a's two pseudo-file names, rendered through TeX82
/// §48's initial character strings.
fn scantokens_source_name(tracing_scantokens: i32) -> &'static str {
    if tracing_scantokens > 0 { "^^S" } else { "^^R" }
}

fn scantokens_numeric_name(tracing_scantokens: i32) -> u8 {
    if tracing_scantokens > 0 { 19 } else { 18 }
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

/// TeX82 §366's `cur_cmd>max_command` test for Umber's resolved command.
///
/// `Meaning::Undefined` normally represents §207's `undefined_cs` command,
/// which is expanded solely to perform §370's diagnostic recovery. A compact
/// out-parameter token also carries that meaning as its invalid-slot recovery,
/// but its command remains `out_param<max_command`; its token spelling keeps
/// the two command identities distinct here.
pub(crate) fn is_expandable_command(command: &CurrentCommand) -> bool {
    is_expandable(command.meaning())
        || (matches!(command.meaning(), Meaning::Undefined)
            && !matches!(command.spelling().semantic_token(), Token::Param(_)))
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

/// TeX82 §262's `print_cs`, including its delimiter after a control word.
///
/// This is distinct from §263's `sprint_cs` spelling used by `\show` before
/// `=` and from §213's `\string`: named control words and `null_cs` append a
/// space, while active characters and single nonletter control symbols do not.
pub(crate) fn print_cs_text(
    state: &mut tex_state::CommandContext<'_>,
    symbol: tex_state::interner::Symbol,
) -> String {
    let name = state.resolve(symbol);
    if state.control_sequence_kind(symbol) == ControlSequenceKind::ActiveCharacter {
        return name.to_owned();
    }

    let mut text = string_text(state, Token::Cs(symbol));
    let mut characters = name.chars();
    match (characters.next(), characters.next()) {
        (Some(character), None) if state.catcode(character) != Catcode::Letter => {}
        _ => text.push(' '),
    }
    text
}

pub(crate) fn meaning_text(
    state: &tex_state::CommandContext<'_>,
    command: &CurrentCommand,
) -> String {
    match command.meaning() {
        Meaning::Undefined => "undefined".to_owned(),
        Meaning::Relax => print_esc_text(state, "relax"),
        Meaning::CharToken { ch, cat } => character_command_text(ch, cat),
        Meaning::CharGiven(ch) => format!("the character {ch}"),
        Meaning::MathCharGiven(value) => format!("\\mathchar\"{value:X}"),
        Meaning::CountRegister(index) => format!("\\count{index}"),
        Meaning::DimenRegister(index) => format!("\\dimen{index}"),
        Meaning::SkipRegister(index) => format!("\\skip{index}"),
        Meaning::MuskipRegister(index) => format!("\\muskip{index}"),
        Meaning::ToksRegister(index) => format!("\\toks{index}"),
        meaning @ (Meaning::IntParam(_)
        | Meaning::InternalInteger(_)
        | Meaning::DimenParam(_)
        | Meaning::GlueParam(_)
        | Meaning::MuGlueParam(_)
        | Meaning::TokParam(_)
        | Meaning::PageDimension(_)
        | Meaning::PageInteger(_)) => meaning_control_sequence_text(state, command, meaning),
        Meaning::Font(font) => format!("select font {}", state.font_name(font)),
        Meaning::Macro { flags, definition } => {
            // `\\meaning` prints the definition, not a live macro-body input
            // frame.  A completed macro call retires its activation and body,
            // whereas the definition's parameter and replacement lists remain
            // immutable state owned by the meaning.
            let macro_meaning = state.macro_definition(definition);
            let mut prefix = String::new();
            if flags.contains(MeaningFlags::PROTECTED) {
                prefix.push_str("\\protected");
            }
            if flags.contains(MeaningFlags::LONG) {
                prefix.push_str("\\long");
            }
            if flags.contains(MeaningFlags::OUTER) {
                prefix.push_str("\\outer");
            }
            if !prefix.is_empty() {
                prefix.push(' ');
            }
            prefix.push_str("macro");
            format!(
                "{prefix}:{}->{}",
                token_list_text(state, macro_meaning.parameter_text()),
                token_list_text(state, macro_meaning.replacement_text()),
            )
        }
        meaning @ (Meaning::ExpandablePrimitive(_) | Meaning::UnexpandablePrimitive(_)) => {
            meaning_control_sequence_text(state, command, meaning)
        }
        Meaning::EndV => "end of alignment template".to_owned(),
        Meaning::Unknown(_) => "unknown".to_owned(),
    }
}

/// The copyable portion of a delivered command needed by TeX82 §298.
///
/// This is captured from `CurrentCommand`, not reconstructed from `Meaning`,
/// so the delivered control-sequence identity remains available across the
/// executor's transactional scan/apply seam.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PrintCommand {
    meaning: Meaning,
    control_sequence: Option<tex_state::interner::Symbol>,
}

impl PrintCommand {
    #[must_use]
    pub const fn from_current(command: &CurrentCommand) -> Self {
        Self {
            meaning: command.meaning(),
            control_sequence: command.control_sequence(),
        }
    }

    #[must_use]
    pub(crate) const fn meaning(self) -> Meaning {
        self.meaning
    }
}

/// TeX82 §298's `print_cmd_chr` representation of one delivered command.
///
/// The input is the full ephemeral equivalent of `cur_cmd`, `cur_chr`, and
/// `cur_cs`, rather than a decoded `Meaning`. This keeps command-class
/// vocabulary independent of the token spelling: a control-sequence alias of
/// a primitive prints the primitive, while aliases of character commands keep
/// their character command class.
#[must_use]
pub fn print_cmd_chr_text(state: &tex_state::CommandContext<'_>, command: PrintCommand) -> String {
    match command.meaning {
        Meaning::Undefined => "undefined".to_owned(),
        Meaning::Relax => print_esc_text(state, "relax"),
        Meaning::Macro { flags, .. } => {
            let mut text = String::new();
            if flags.contains(MeaningFlags::PROTECTED) {
                text.push_str(&print_esc_text(state, "protected"));
            }
            if flags.contains(MeaningFlags::LONG) {
                text.push_str(&print_esc_text(state, "long"));
            }
            if flags.contains(MeaningFlags::OUTER) {
                text.push_str(&print_esc_text(state, "outer"));
            }
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str("macro");
            text
        }
        Meaning::CharToken { ch, cat } => character_command_text(ch, cat),
        Meaning::CharGiven(ch) => format!("{}\"{:X}", print_esc_text(state, "char"), ch as u32),
        Meaning::MathCharGiven(value) => {
            format!("{}\"{value:X}", print_esc_text(state, "mathchar"))
        }
        Meaning::CountRegister(index) => format!("{}{index}", print_esc_text(state, "count")),
        Meaning::DimenRegister(index) => format!("{}{index}", print_esc_text(state, "dimen")),
        Meaning::SkipRegister(index) => format!("{}{index}", print_esc_text(state, "skip")),
        Meaning::MuskipRegister(index) => format!("{}{index}", print_esc_text(state, "muskip")),
        Meaning::ToksRegister(index) => format!("{}{index}", print_esc_text(state, "toks")),
        meaning @ (Meaning::IntParam(_)
        | Meaning::InternalInteger(_)
        | Meaning::DimenParam(_)
        | Meaning::GlueParam(_)
        | Meaning::MuGlueParam(_)
        | Meaning::TokParam(_)
        | Meaning::PageDimension(_)
        | Meaning::PageInteger(_)
        | Meaning::ExpandablePrimitive(_)
        | Meaning::UnexpandablePrimitive(_)) => {
            print_command_control_sequence_text(state, command, meaning)
        }
        Meaning::Font(font) => {
            let mut text = format!("select font {}", state.font_external_name(font));
            let size = state.font_size(font);
            if size != state.font_design_size(font) {
                text.push_str(" at ");
                text.push_str(&tex_state::scaled::print_scaled(size));
                text.push_str("pt");
            }
            text
        }
        Meaning::EndV => "end of alignment template".to_owned(),
        Meaning::Unknown(_) => "[unknown command code!]".to_owned(),
    }
}

fn print_command_control_sequence_text(
    state: &tex_state::CommandContext<'_>,
    command: PrintCommand,
    meaning: Meaning,
) -> String {
    let name = state
        .primitive_name(meaning)
        .or_else(|| command.control_sequence.map(|symbol| state.resolve(symbol)));
    name.map_or_else(
        || "undefined".to_owned(),
        |name| print_esc_text(state, name),
    )
}

fn meaning_control_sequence_text(
    state: &tex_state::CommandContext<'_>,
    command: &CurrentCommand,
    meaning: Meaning,
) -> String {
    let name = state.primitive_name(meaning).or_else(|| {
        command
            .control_sequence()
            .map(|symbol| state.resolve(symbol))
    });
    name.map_or_else(|| "undefined".to_owned(), |name| format!("\\{name}"))
}

/// TeX82 §298's character-command cases used by `print_meaning`.
pub fn character_command_text(ch: char, cat: Catcode) -> String {
    match cat {
        Catcode::BeginGroup => format!("begin-group character {ch}"),
        Catcode::EndGroup => format!("end-group character {ch}"),
        Catcode::MathShift => format!("math shift character {ch}"),
        Catcode::AlignmentTab => format!("alignment tab character {ch}"),
        Catcode::Parameter => format!("macro parameter character {ch}"),
        Catcode::Superscript => format!("superscript character {ch}"),
        Catcode::Subscript => format!("subscript character {ch}"),
        Catcode::Space => "blank space  ".to_owned(),
        Catcode::Letter => format!("the letter {ch}"),
        Catcode::Other => format!("the character {ch}"),
        // `get_next` maps a category-5 character to `car_ret` with its
        // character code as operand. It is therefore §298's non-`cr_code`
        // branch, whose vocabulary is `\crcr`.
        Catcode::EndLine => "\\crcr".to_owned(),
        Catcode::Escape
        | Catcode::Ignored
        | Catcode::Active
        | Catcode::Comment
        | Catcode::Invalid => format!("[uncommandable character {ch}]"),
    }
}

/// TeX82 §63's `print_esc`: the current `\escapechar`, when it names a
/// character, followed by `name`.
///
/// §63 prints no escape at all when `\escapechar` is outside a character's
/// range, which is why the prefix is conditional rather than a hard-coded
/// backslash.
#[must_use]
pub fn print_esc_text(state: &tex_state::CommandContext<'_>, name: &str) -> String {
    let mut text = String::with_capacity(name.len() + 1);
    if let Ok(escape) = u8::try_from(state.int_param(IntParam::ESCAPE_CHAR)) {
        text.push(char::from(escape));
    }
    text.push_str(name);
    text
}

/// TeX82 §298's `print_cmd_chr` representation for a delivered token.
///
/// Diagnostics use this same renderer as `\meaning`; consequently Rust enum
/// spellings cannot leak into ordinary terminal or transcript output.
#[must_use]
pub fn command_token_text(state: &mut tex_state::CommandContext<'_>, token: Token) -> String {
    match token {
        Token::Char { ch, cat } => character_command_text(ch, cat),
        Token::Param(slot) => format!("macro parameter character #{slot}"),
        Token::Frozen(_) => "end of alignment template".to_owned(),
        Token::Cs(symbol) => {
            let meaning = state.meaning(symbol);
            state.primitive_name(meaning).map_or_else(
                || print_esc_text(state, state.resolve(symbol)),
                |name| print_esc_text(state, name),
            )
        }
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

/// The string pdfTeX builds by selecting `new_string` around `show_token_list`.
///
/// Character tokens remain raw (with parameter characters doubled), while
/// control-sequence spelling and its separator observe the live escape
/// character and catcode table. The returned value owns no token-list handle,
/// so it remains stable when an aggregate resource suspension rolls back and
/// rescans the command.
pub(crate) fn token_list_string_text(
    state: &mut tex_state::CommandContext<'_>,
    tokens: TokenListId,
) -> String {
    let tokens = state.tokens(tokens).to_vec();
    let mut text = String::new();
    for token in tokens {
        match token {
            Token::Char { ch, cat } => {
                text.push(ch);
                if cat == Catcode::Parameter {
                    text.push(ch);
                }
            }
            Token::Param(slot) => {
                text.push('#');
                text.push(char::from(b'0' + slot));
            }
            Token::Frozen(_) => text.push_str("\\endtemplate"),
            Token::Cs(symbol) => {
                let name = state.resolve(symbol).to_owned();
                if state.control_sequence_kind(symbol) == ControlSequenceKind::ActiveCharacter {
                    text.push_str(&name);
                    continue;
                }
                let escape = state.int_param(IntParam::ESCAPE_CHAR);
                if let Ok(escape) = u8::try_from(escape) {
                    text.push(char::from(escape));
                }
                if name.is_empty() {
                    text.push_str("csname");
                    if let Ok(escape) = u8::try_from(escape) {
                        text.push(char::from(escape));
                    }
                    text.push_str("endcsname");
                } else {
                    text.push_str(&name);
                }
                let mut chars = name.chars();
                match (chars.next(), chars.next()) {
                    (Some(ch), None) if state.catcode(ch) != Catcode::Letter => {}
                    _ => text.push(' '),
                }
            }
        }
    }
    text
}

/// TeX82's `show_token_list` representation used by `\\meaning` distinguishes
/// a printed control word from following letter tokens with one space.  That
/// delimiter belongs to the rendered definition, not to source input.
pub(crate) fn token_list_token_text(state: &tex_state::CommandContext<'_>, token: Token) -> String {
    let name = match token {
        Token::Cs(symbol) => {
            if state.control_sequence_kind(symbol) == ControlSequenceKind::ActiveCharacter {
                return state.resolve(symbol).to_owned();
            }
            state.resolve(symbol)
        }
        // tex.web gives every frozen equivalent a real eqtb `text()`, so §294
        // displays one exactly as it displays the ordinary control sequence of
        // the same name: `frozen_par` is `\par`, not its `\relax`-like
        // meaning.
        Token::Frozen(_) => match state.frozen_primitive_name(token) {
            Some(name) => name,
            None => return string_text(state, token),
        },
        Token::Char {
            ch,
            cat: Catcode::Parameter,
        } => {
            // TeX82 §§262/315: `show_token_list` prints one stored `match`
            // token as two parameter characters; storage remains singular.
            return format!("{ch}{ch}");
        }
        _ => return string_text(state, token),
    };
    // TeX82 §§63/294: `show_token_list` renders control sequences through
    // `print_cs`, and every escape prefix that `print_cs` emits comes from
    // the live `\escapechar`. This matters for backed-up recovery tokens:
    // §1064 inserts a closer ahead of the offending command, then §314
    // pseudoprints that command while the current integer parameters remain
    // in force.
    let mut text = if name.is_empty() {
        format!(
            "{}{}",
            print_esc_text(state, "csname"),
            print_esc_text(state, "endcsname")
        )
    } else {
        print_esc_text(state, name)
    };
    let mut chars = name.chars();
    let control_symbol = matches!((chars.next(), chars.next()), (Some(_), None));
    if !control_symbol || name.chars().next().is_some_and(char::is_alphabetic) {
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
/// Resource fuel is deliberately absent: [`crate::CommandFuel`] is a
/// monotonic owner lent to processor episodes and is not restored with
/// semantic state. Caches and profiling likewise belong to
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
