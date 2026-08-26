//! Ordinary expanded-command delivery.

use std::fmt::Write as _;

use tex_state::env::banks::IntParam;
use tex_state::glue::{GlueSpec, Order};
use tex_state::interner::ControlSequenceKind;
use tex_state::meaning::{ExpandablePrimitive, Meaning, MeaningFlags, ResolvedMeaning};
use tex_state::page::PageMark;
use tex_state::scaled::Scaled;
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use posix_regex::{PosixRegexBuilder, compile::Error as PosixRegexError};

use crate::command::DeliveryStamp;
use crate::input::{
    BackedUpToken, BackupTreatment, InputLevelId, PackedTokenSpanHandle, ReplayTrace,
    RetirementBehavior, TokenBehavior,
};
use crate::macro_call::MacroArguments;
use crate::processor::status::{ScannerStatus, ScannerStatusVisibility};
use crate::profile::CommandProfile;
use crate::{
    CommandError, CommandReplayDelivery, CurrentCommand, RegisteredSourceKind, SourceNameClass,
    SourceRegistration,
};

use super::{
    AlignmentInterceptionPolicy, AlignmentLookahead, CommandProcessor, DeliveryMode,
    DeliveryPolicy, DeliveryStatus, ExpandedDeliveryPolicy, ExpandedObservationPolicy,
    FirstCommandPolicy, ReplayCompletionPolicy,
};

use crate::observation::{
    CommandDeliveryBoundary, CommandDeliveryRecord, CommandObservation, CommandProvenance,
    EffectRecord, InputReason, InputRecord, InputTransition, RecoveryKind, RecoveryRecord,
    TokenListRecord,
};

/// Operand state held by TeX82 §368 while `\expandafter` expands its second
/// command across an immutable host suspension.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct PendingExpandAfter<G> {
    first: CurrentCommand<G>,
    second: CurrentCommand<G>,
    child: Option<crate::execution_scratch::ChildContinuation<G, PendingExpandAfterDestination>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PendingExpandAfterDestination {
    ExpandingSecond,
}

impl<G> PendingExpandAfter<G> {
    pub(crate) fn take_child(&mut self) -> Option<crate::execution_scratch::ScannerFrameKey<G>> {
        self.child.take().map(|child| child.restore().0)
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct PendingPdfStringCompare<G> {
    phase: PdfStringComparePhase,
    child: Option<crate::execution_scratch::ChildContinuation<G, PdfStringCompareDestination>>,
}

impl<G> PendingPdfStringCompare<G> {
    pub(crate) fn take_child(&mut self) -> Option<crate::execution_scratch::ScannerFrameKey<G>> {
        self.child.take().map(|child| child.restore().0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PdfStringComparePhase {
    Left,
    Right { left: crate::AttemptTokenListId },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PdfStringCompareDestination {
    Scan,
}

/// Stable pending-diagnostic identity for TeX.web's `Missing \\endcsname
/// inserted` recovery. Rendering belongs to the diagnostic milestone.
pub(crate) const MISSING_ENDCSNAME_DIAGNOSTIC: u64 = 0x6373_6e61_6d65_0001;

/// Stable pending-diagnostic identity for pdftex.web §495's color-stack
/// capacity recovery.
pub(crate) const TOO_MANY_COLOR_STACKS_DIAGNOSTIC: u64 = 0x7064_6663_7300_0495;

/// TeX82's decimal rendering for a scaled quantity, including its `pt` unit.
fn format_scaled(value: Scaled) -> String {
    let mut output = String::new();
    append_format_scaled(value, &mut output);
    output
}

fn append_format_scaled(value: Scaled, output: &mut String) {
    let mut raw = i64::from(value.raw());
    if raw < 0 {
        output.push('-');
        raw = -raw;
    }
    let unity = i64::from(Scaled::UNITY);
    write!(output, "{}", raw / unity).expect("writing to String cannot fail");
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
}

fn format_glue(value: GlueSpec, unit: &str) -> String {
    let mut output = String::new();
    append_format_glue(value, unit, &mut output);
    output
}

/// pdfTeX's `utils.c` `escapestring` projection for a PDF literal-string body.
fn escape_pdf_literal_string(text: &str) -> String {
    fn append_byte(output: &mut String, byte: u8) {
        match byte {
            b'!'..=b'~' => {
                if matches!(byte, b'(' | b')' | b'\\') {
                    output.push('\\');
                }
                output.push(char::from(byte));
            }
            _ => write!(output, "\\{byte:03o}").expect("writing to String cannot fail"),
        }
    }

    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        if let Ok(byte) = u8::try_from(u32::from(character)) {
            append_byte(&mut escaped, byte);
        } else {
            let mut encoded = [0; 4];
            for byte in character.encode_utf8(&mut encoded).bytes() {
                append_byte(&mut escaped, byte);
            }
        }
    }
    escaped
}

/// pdfTeX's `utils.c` `escapehex` projection for a PDF hexadecimal string.
fn escape_pdf_hex(bytes: &[u8]) -> String {
    let mut escaped = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        write!(escaped, "{byte:02X}").expect("writing to String cannot fail");
    }
    escaped
}

/// pdfTeX's `utils.c` `unescapehex` projection for a PDF hexadecimal string.
fn unescape_pdf_hex(bytes: &[u8]) -> String {
    let mut unescaped = String::with_capacity(bytes.len().div_ceil(2));
    let mut high_nibble = None;
    for byte in bytes {
        let nibble = match byte {
            b'0'..=b'9' => byte - b'0',
            b'A'..=b'F' => byte - b'A' + 10,
            b'a'..=b'f' => byte - b'a' + 10,
            _ => continue,
        };
        if let Some(high) = high_nibble.take() {
            unescaped.push(char::from(high | nibble));
        } else {
            high_nibble = Some(nibble << 4);
        }
    }
    if let Some(high) = high_nibble {
        unescaped.push(char::from(high));
    }
    unescaped
}

fn append_format_glue(value: GlueSpec, unit: &str, output: &mut String) {
    append_scaled_with_unit(value.width, unit, output);
    for (label, component, order) in [
        (" plus ", value.stretch, value.stretch_order),
        (" minus ", value.shrink, value.shrink_order),
    ] {
        if component.raw() == 0 {
            continue;
        }
        output.push_str(label);
        append_scaled_without_unit(component, output);
        output.push_str(match order {
            Order::Normal => unit,
            Order::Fil => "fil",
            Order::Fill => "fill",
            Order::Filll => "filll",
        });
    }
}

fn append_scaled_without_unit(value: Scaled, output: &mut String) {
    let start = output.len();
    append_format_scaled(value, output);
    output.truncate(output.len() - "pt".len());
    debug_assert!(output.len() >= start);
}

fn append_scaled_with_unit(value: Scaled, unit: &str, output: &mut String) {
    append_scaled_without_unit(value, output);
    output.push_str(unit);
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
pub(super) enum ProtectedMacroHandling {
    Expand,
    Preserve,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum UndefinedHandling {
    Diagnose,
    Preserve,
}

fn static_meaning<G>(meaning: ResolvedMeaning<G>) -> Option<Meaning> {
    match meaning {
        ResolvedMeaning::Static(meaning) => Some(meaning),
        ResolvedMeaning::Macro { .. } => None,
    }
}

/// The finite expansion set selected by the pinned structural census.
///
/// These families execute against the borrowed live command in the one
/// processor episode. Everything else remains a cold arm in this same
/// interpreter; the profiling materialization counter records only that
/// explicit fallback boundary.
#[inline(always)]
#[cfg(feature = "profiling")]
fn is_ranked_fused_expansion<G>(meaning: &ResolvedMeaning<G>) -> bool {
    matches!(
        meaning,
        ResolvedMeaning::Macro { .. }
            | ResolvedMeaning::Static(Meaning::ExpandablePrimitive(
                ExpandablePrimitive::ExpandAfter
                    | ExpandablePrimitive::Fi
                    | ExpandablePrimitive::IfX
                    | ExpandablePrimitive::IfNum
                    | ExpandablePrimitive::If
                    | ExpandablePrimitive::CsName
                    | ExpandablePrimitive::NoExpand
                    | ExpandablePrimitive::Detokenize
                    | ExpandablePrimitive::String
                    | ExpandablePrimitive::IfFalse
                    | ExpandablePrimitive::RomanNumeral
                    | ExpandablePrimitive::Else
                    | ExpandablePrimitive::Expanded
                    | ExpandablePrimitive::IfCsName
                    | ExpandablePrimitive::Number
                    | ExpandablePrimitive::The
            ))
    )
}

impl<G> CommandProcessor<'_, '_, G> {
    /// Settles one raw command already delivered by the same processor
    /// episode. This is the capability-preflight seam: macro/expandable
    /// nesting, undefined-command recovery, and ordered raw/expanded
    /// observations remain in one borrow.
    #[doc(hidden)]
    pub fn settle_current_command(
        &mut self,
        command: CurrentCommand<G>,
    ) -> Result<Option<CurrentCommand<G>>, CommandError> {
        let mut destination = Some(command);
        let result = self.delivery_driver(
            DeliveryPolicy {
                mode: DeliveryMode::Expanded(ExpandedDeliveryPolicy {
                    fetch: ExpandedFetch::XToken,
                    protected_macros: ProtectedMacroHandling::Expand,
                    undefined: UndefinedHandling::Diagnose,
                    observation: ExpandedObservationPolicy::Commit,
                    first_command: FirstCommandPolicy::Ordinary,
                }),
                replay_completion: ReplayCompletionPolicy::Consume,
                alignment_interception: AlignmentInterceptionPolicy::Scalar,
            },
            &mut destination,
        )?;
        match result {
            DeliveryStatus::End => Ok(None),
            DeliveryStatus::Command => Ok(destination),
            _ => unreachable!("preflight settlement returns one command"),
        }
    }

    /// Delivers one ordinary expanded command through TeX.web's `get_x_token`.
    ///
    /// This thin canonical entry point selects the ordinary policy of the
    /// shared raw/expanded delivery driver. Expansion mutates canonical
    /// command state and restarts in that one driver; it never returns a
    /// push-bearing dispatch result or enters a second interpreter.
    pub fn get_x_token(&mut self) -> Result<Option<CurrentCommand<G>>, CommandError> {
        let mut destination = None;
        match self.get_x_token_into(&mut destination)? {
            DeliveryStatus::End => Ok(None),
            DeliveryStatus::Command => Ok(destination),
            _ => unreachable!("ordinary expanded delivery returns only commands"),
        }
    }

    /// Delivers one expanded command directly into caller-provided storage.
    pub fn get_x_token_into(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        self.apply_error_stop_recovery()?;
        self.get_x_token_from_into(None, ExpandedFetch::GetXToken, destination)
    }

    /// Delivers one expanded output-replay token, preserving protected macros
    /// in e-TeX/pdfTeX exactly as `get_x_or_protected` does.
    pub(crate) fn get_x_or_protected_with_replay_completion(
        &mut self,
    ) -> Result<Option<CommandReplayDelivery<G>>, CommandError> {
        let mut destination = None;
        let result = self.get_x_or_protected_with_replay_completion_into(&mut destination)?;
        Ok(match result {
            DeliveryStatus::End => None,
            DeliveryStatus::Command => Some(CommandReplayDelivery::Command(
                destination.expect("command status initializes destination"),
            )),
            DeliveryStatus::ReplayCompleted(episode) => {
                Some(CommandReplayDelivery::Completed(episode))
            }
            _ => unreachable!("protected delivery policy cannot produce this event"),
        })
    }

    /// Delivers protected replay-aware expansion into caller-provided storage.
    pub(crate) fn get_x_or_protected_with_replay_completion_into(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        self.apply_error_stop_recovery()?;
        let preserve = self.command.profile().capabilities().supports_etex();
        let result = self.delivery_driver(
            DeliveryPolicy {
                mode: DeliveryMode::Expanded(ExpandedDeliveryPolicy {
                    fetch: ExpandedFetch::GetXToken,
                    protected_macros: if preserve {
                        ProtectedMacroHandling::Preserve
                    } else {
                        ProtectedMacroHandling::Expand
                    },
                    undefined: UndefinedHandling::Diagnose,
                    observation: if preserve {
                        ExpandedObservationPolicy::RawOnly
                    } else {
                        ExpandedObservationPolicy::Commit
                    },
                    first_command: FirstCommandPolicy::Ordinary,
                }),
                replay_completion: ReplayCompletionPolicy::Surface,
                alignment_interception: AlignmentInterceptionPolicy::Scalar,
            },
            destination,
        )?;
        debug_assert!(matches!(
            result,
            DeliveryStatus::End | DeliveryStatus::Command | DeliveryStatus::ReplayCompleted(_)
        ));
        Ok(result)
    }

    /// Delivers one expanded command to a diagnostic host while preserving
    /// TeX82 §370's undefined command instead of consuming it after recovery.
    pub fn get_x_token_preserving_undefined(
        &mut self,
    ) -> Result<Option<CurrentCommand<G>>, CommandError> {
        self.apply_error_stop_recovery()?;
        let mut destination = None;
        let result = self.delivery_driver(
            DeliveryPolicy {
                mode: DeliveryMode::Expanded(ExpandedDeliveryPolicy {
                    fetch: ExpandedFetch::GetXToken,
                    protected_macros: ProtectedMacroHandling::Expand,
                    undefined: UndefinedHandling::Preserve,
                    observation: ExpandedObservationPolicy::Commit,
                    first_command: FirstCommandPolicy::Ordinary,
                }),
                replay_completion: ReplayCompletionPolicy::Consume,
                alignment_interception: AlignmentInterceptionPolicy::Scalar,
            },
            &mut destination,
        )?;
        match result {
            DeliveryStatus::End => Ok(None),
            DeliveryStatus::Command => Ok(destination),
            _ => unreachable!("ordinary expanded delivery returns only commands"),
        }
    }

    /// TeX.web §381's `x_token` entered with `cur_cmd`/`cur_chr` already set.
    ///
    /// §381 does not begin with `get_next`: it expands whatever the caller
    /// left in the current command and only then reads on. Ordinary delivery
    /// leaves nothing, which is [`Self::get_x_token`]; §1152 loads an active
    /// character's meaning directly and passes it here, so that meaning is
    /// expanded without ever having been delivered raw.
    fn get_x_token_from_into(
        &mut self,
        pending: Option<CurrentCommand<G>>,
        fetch: ExpandedFetch,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        debug_assert!(destination.is_none());
        *destination = pending;
        let result = self.delivery_driver(
            DeliveryPolicy {
                mode: DeliveryMode::Expanded(ExpandedDeliveryPolicy {
                    fetch,
                    protected_macros: ProtectedMacroHandling::Expand,
                    undefined: UndefinedHandling::Diagnose,
                    observation: ExpandedObservationPolicy::Commit,
                    first_command: FirstCommandPolicy::Ordinary,
                }),
                replay_completion: ReplayCompletionPolicy::Consume,
                alignment_interception: AlignmentInterceptionPolicy::Scalar,
            },
            destination,
        )?;
        debug_assert!(matches!(
            result,
            DeliveryStatus::End | DeliveryStatus::Command
        ));
        Ok(result)
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
        let command = CurrentCommand::<G>::resolve(spelling, stamp, None, false, None, &self.state);
        let mut destination = None;
        let status =
            self.get_x_token_from_into(Some(command), ExpandedFetch::XToken, &mut destination)?;
        let settled = match status {
            DeliveryStatus::End => return Ok(()),
            DeliveryStatus::Command => destination
                .take()
                .expect("command status initializes destination"),
            _ => unreachable!("ordinary expanded delivery returns only commands"),
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
    ) -> Result<Option<CurrentCommand<G>>, CommandError> {
        let mut destination = None;
        loop {
            match self.get_x_token_into(&mut destination)? {
                DeliveryStatus::End => return Ok(None),
                DeliveryStatus::Command => {}
                _ => unreachable!("ordinary expanded delivery returns only commands"),
            }
            let command = destination
                .as_ref()
                .expect("command status initializes destination");
            if !matches!(
                static_meaning(command.meaning()),
                Some(
                    Meaning::CharToken {
                        cat: Catcode::Space,
                        ..
                    } | Meaning::Relax
                )
            ) {
                return Ok(destination);
            }
            destination = None;
        }
    }

    /// TeX82 §406's `<Get the next non-blank non-call token>`:
    /// `repeat get_x_token until cur_cmd<>spacer`.
    ///
    /// Unlike §404's similarly named helper, this preserves `\relax`. The
    /// returned command is the exact expanded delivery that stopped the
    /// loop: callers such as §1045's `\ignorespaces` dispatch it in place
    /// without backing it up or rebuilding its provenance.
    pub fn next_non_blank_x_token(&mut self) -> Result<Option<CurrentCommand<G>>, CommandError> {
        let mut destination = None;
        loop {
            match self.get_x_token_into(&mut destination)? {
                DeliveryStatus::End => return Ok(None),
                DeliveryStatus::Command => {}
                _ => unreachable!("ordinary expanded delivery returns only commands"),
            }
            let command = destination
                .as_ref()
                .expect("command status initializes destination");
            if !matches!(
                static_meaning(command.meaning()),
                Some(Meaning::CharToken {
                    cat: Catcode::Space,
                    ..
                })
            ) {
                return Ok(destination);
            }
            destination = None;
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
    ) -> Result<Option<AlignmentLookahead<G>>, CommandError> {
        loop {
            let etex_protected_fetch = self.command.profile().capabilities().supports_etex();
            let mut destination = None;
            let result = self.delivery_driver(
                DeliveryPolicy {
                    mode: DeliveryMode::Expanded(ExpandedDeliveryPolicy {
                        fetch: ExpandedFetch::GetXToken,
                        protected_macros: if etex_protected_fetch {
                            ProtectedMacroHandling::Preserve
                        } else {
                            ProtectedMacroHandling::Expand
                        },
                        undefined: UndefinedHandling::Diagnose,
                        observation: if etex_protected_fetch {
                            ExpandedObservationPolicy::RawOnly
                        } else {
                            ExpandedObservationPolicy::DeferIfExpanded
                        },
                        first_command: FirstCommandPolicy::Ordinary,
                    }),
                    replay_completion: ReplayCompletionPolicy::Consume,
                    alignment_interception: AlignmentInterceptionPolicy::Scalar,
                },
                &mut destination,
            );
            let lookahead = match result? {
                DeliveryStatus::End => return Ok(None),
                DeliveryStatus::Command => AlignmentLookahead::Committed(
                    destination.expect("command status initializes destination"),
                ),
                DeliveryStatus::PendingExpanded => AlignmentLookahead::PendingExpanded(
                    destination.expect("pending status initializes destination"),
                ),
                _ => unreachable!("alignment lookahead consumes replay completions"),
            };
            if matches!(
                lookahead.command().meaning(),
                ResolvedMeaning::Static(Meaning::CharToken {
                    cat: Catcode::Space,
                    ..
                })
            ) {
                let _ = self.commit_alignment_lookahead_delivery(lookahead);
                continue;
            }
            return Ok(Some(lookahead));
        }
    }

    /// Commits a terminal TeX82 lookahead delivery that alignment control
    /// consumes instead of passing to an ordinary `back_input` branch.
    pub fn commit_alignment_lookahead_delivery(
        &mut self,
        lookahead: AlignmentLookahead<G>,
    ) -> CurrentCommand<G> {
        match lookahead {
            AlignmentLookahead::Committed(command) => command,
            AlignmentLookahead::PendingExpanded(command) => {
                self.observe_expanded_delivery(&command);
                command
            }
        }
    }

    /// Completes TeX82 §§785/791's ordinary `align_peek`/`init_col` branch.
    ///
    /// A command reached through §380's expansion loop is still pending only
    /// in Umber's observer transport. TeX has already completed
    /// `get_x_token`, so its expanded delivery precedes §789's `back_input`;
    /// the later replay above the u-template is a distinct delivery.
    pub fn back_alignment_lookahead(
        &mut self,
        lookahead: AlignmentLookahead<G>,
    ) -> Result<(), CommandError> {
        let command = self.commit_alignment_lookahead_delivery(lookahead);
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
    ) -> Result<Option<CommandReplayDelivery<G>>, CommandError> {
        let mut destination = None;
        let result = self.get_x_token_with_replay_completion_into(&mut destination)?;
        Ok(match result {
            DeliveryStatus::End => None,
            DeliveryStatus::Command => Some(CommandReplayDelivery::Command(
                destination.expect("command status initializes destination"),
            )),
            DeliveryStatus::ReplayCompleted(episode) => {
                Some(CommandReplayDelivery::Completed(episode))
            }
            _ => unreachable!("ordinary replay-aware delivery has no alignment event"),
        })
    }

    /// Delivers replay-aware expanded input into caller-provided storage.
    pub fn get_x_token_with_replay_completion_into(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        self.apply_error_stop_recovery()?;
        let result = self.delivery_driver(
            DeliveryPolicy {
                mode: DeliveryMode::Expanded(ExpandedDeliveryPolicy {
                    fetch: ExpandedFetch::GetXToken,
                    protected_macros: ProtectedMacroHandling::Expand,
                    undefined: UndefinedHandling::Diagnose,
                    observation: ExpandedObservationPolicy::Commit,
                    first_command: FirstCommandPolicy::Ordinary,
                }),
                replay_completion: ReplayCompletionPolicy::Surface,
                alignment_interception: AlignmentInterceptionPolicy::Scalar,
            },
            destination,
        )?;
        debug_assert!(matches!(
            result,
            DeliveryStatus::End | DeliveryStatus::Command | DeliveryStatus::ReplayCompleted(_)
        ));
        Ok(result)
    }

    /// Settles a raw command retained by the executor's capability preflight.
    ///
    /// This is TeX82's `x_token` entry with `cur_cmd` already set. It neither
    /// backs up nor redelivers the retained token. `main_loop` selects §1038's
    /// character fast-path policy for the first command only.
    pub fn settle_preflight_command(
        &mut self,
        command: CurrentCommand<G>,
        main_loop: bool,
    ) -> Result<Option<CommandReplayDelivery<G>>, CommandError> {
        let mut destination = None;
        let result = self.settle_preflight_command_into(command, main_loop, &mut destination)?;
        Ok(match result {
            DeliveryStatus::End => None,
            DeliveryStatus::Command => Some(CommandReplayDelivery::Command(
                destination.expect("command status initializes destination"),
            )),
            DeliveryStatus::ReplayCompleted(episode) => {
                Some(CommandReplayDelivery::Completed(episode))
            }
            _ => unreachable!("preflight settlement has no alignment event"),
        })
    }

    /// Settles a preflight command into caller-provided final storage.
    pub fn settle_preflight_command_into(
        &mut self,
        command: CurrentCommand<G>,
        main_loop: bool,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        self.apply_error_stop_recovery()?;
        debug_assert!(destination.is_none());
        *destination = Some(command);
        let result = self.delivery_driver(
            DeliveryPolicy {
                mode: DeliveryMode::Expanded(ExpandedDeliveryPolicy {
                    fetch: ExpandedFetch::XToken,
                    protected_macros: ProtectedMacroHandling::Expand,
                    undefined: UndefinedHandling::Diagnose,
                    observation: ExpandedObservationPolicy::Commit,
                    first_command: if main_loop {
                        FirstCommandPolicy::MainLoopCharacter
                    } else {
                        FirstCommandPolicy::Ordinary
                    },
                }),
                replay_completion: ReplayCompletionPolicy::Surface,
                alignment_interception: AlignmentInterceptionPolicy::Scalar,
            },
            destination,
        )?;
        debug_assert!(matches!(
            result,
            DeliveryStatus::End | DeliveryStatus::Command | DeliveryStatus::ReplayCompleted(_)
        ));
        Ok(result)
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
    pub fn main_loop_lookahead(
        &mut self,
    ) -> Result<Option<CommandReplayDelivery<G>>, CommandError> {
        let mut destination = None;
        let result = self.main_loop_lookahead_into(&mut destination)?;
        Ok(match result {
            DeliveryStatus::End => None,
            DeliveryStatus::Command => Some(CommandReplayDelivery::Command(
                destination.expect("command status initializes destination"),
            )),
            DeliveryStatus::ReplayCompleted(episode) => {
                Some(CommandReplayDelivery::Completed(episode))
            }
            _ => unreachable!("main-loop lookahead has no alignment event"),
        })
    }

    /// Delivers main-loop lookahead into caller-provided command storage.
    pub fn main_loop_lookahead_into(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        self.apply_error_stop_recovery()?;
        let result = self.delivery_driver(
            DeliveryPolicy {
                mode: DeliveryMode::Expanded(ExpandedDeliveryPolicy {
                    fetch: ExpandedFetch::XToken,
                    protected_macros: ProtectedMacroHandling::Expand,
                    undefined: UndefinedHandling::Diagnose,
                    observation: ExpandedObservationPolicy::Commit,
                    first_command: FirstCommandPolicy::MainLoopCharacter,
                }),
                replay_completion: ReplayCompletionPolicy::Surface,
                alignment_interception: AlignmentInterceptionPolicy::Scalar,
            },
            destination,
        )?;
        debug_assert!(matches!(
            result,
            DeliveryStatus::End | DeliveryStatus::Command | DeliveryStatus::ReplayCompleted(_)
        ));
        Ok(result)
    }

    /// TeX.web §380's expanded-fetch loop, in whichever of its two forms
    /// `fetch` names, optionally entered with the raw command §1038's
    /// lookahead has already fetched.
    pub(super) fn delivery_driver(
        &mut self,
        policy: DeliveryPolicy,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        self.last_delivery = None;
        match policy.mode {
            DeliveryMode::Raw => {
                debug_assert!(destination.is_none());
                self.raw_delivery_driver(policy, destination)
            }
            DeliveryMode::Expanded(expanded) => {
                let mut resumed_pending = false;
                if self
                    .scanner_resume
                    .as_ref()
                    .is_some_and(crate::ScannerFrameKey::is_expansion)
                {
                    let retained = self
                        .command
                        .scratch
                        .expansion_frame(
                            self.scanner_resume
                                .as_ref()
                                .expect("matched expansion frame"),
                        )
                        .map_err(crate::scan_toks::scratch_command_error)?;
                    if destination
                        .as_ref()
                        .is_some_and(|command| command != &retained.command)
                    {
                        let key = self.scanner_resume.take().expect("matched expansion frame");
                        self.abort_continuation(key)?;
                        return Err(CommandError::input_invariant());
                    }
                    *destination = Some(retained.command.clone());
                    resumed_pending = true;
                }
                if resumed_pending && let Some(command) = &destination {
                    self.resume_current_command(command);
                }
                let depth = self.command.transient.active_expansion_depth;
                self.command.transient.active_expansion_depth = depth
                    .checked_add(1)
                    .ok_or_else(|| CommandError::input_invariant())?;
                let result =
                    self.expanded_delivery_driver(policy, expanded, resumed_pending, destination);
                assert_eq!(
                    self.command.transient.active_expansion_depth,
                    depth + 1,
                    "nested delivery must balance expansion depth"
                );
                self.command.transient.active_expansion_depth = depth;
                result
            }
        }
    }

    fn raw_delivery_driver(
        &mut self,
        policy: DeliveryPolicy,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        loop {
            if destination.is_none() {
                self.last_delivery = None;
                self.charge_command_action()?;
                match self.get_next_canonical(destination)? {
                    DeliveryStatus::End => return Ok(DeliveryStatus::End),
                    DeliveryStatus::ReplayCompleted(episode) => {
                        if policy.replay_completion == ReplayCompletionPolicy::Surface {
                            return Ok(DeliveryStatus::ReplayCompleted(episode));
                        }
                        continue;
                    }
                    DeliveryStatus::Command => {}
                    _ => unreachable!("raw fetch returns only raw statuses"),
                }
            }

            if policy.alignment_interception == AlignmentInterceptionPolicy::Scalar
                && matches!(
                    destination
                        .as_ref()
                        .expect("raw destination contains a command")
                        .alignment_adjustment(),
                    crate::processor::AlignmentDeliveryAdjustment::Delimiter(_)
                )
            {
                self.begin_scalar_alignment_v_template(
                    destination
                        .as_ref()
                        .expect("raw destination contains a command"),
                )?;
                *destination = None;
                continue;
            }
            return Ok(DeliveryStatus::Command);
        }
    }

    fn expanded_delivery_driver(
        &mut self,
        policy: DeliveryPolicy,
        expanded: ExpandedDeliveryPolicy,
        resumed_pending: bool,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        let expansions_before = self.command.expansion.cumulative_expansions;
        let mut first = true;
        let mut suppress_first_expansion_trace = resumed_pending;
        loop {
            if destination.is_none() {
                self.last_delivery = None;
                self.charge_command_action()?;
                match self.get_next_canonical(destination)? {
                    DeliveryStatus::End => return Ok(DeliveryStatus::End),
                    DeliveryStatus::ReplayCompleted(episode) => {
                        if policy.replay_completion == ReplayCompletionPolicy::Surface {
                            return Ok(DeliveryStatus::ReplayCompleted(episode));
                        }
                        continue;
                    }
                    DeliveryStatus::Command => {}
                    _ => unreachable!("raw fetch returns only raw statuses"),
                }
            }
            let command = destination
                .as_ref()
                .expect("expanded destination contains a command");

            if std::mem::take(&mut first)
                && expanded.first_command == FirstCommandPolicy::MainLoopCharacter
                && is_main_loop_character(command.meaning())
            {
                return Ok(DeliveryStatus::Command);
            }
            if matches!(
                static_meaning(command.meaning()),
                Some(Meaning::ExpandablePrimitive(
                    ExpandablePrimitive::EndTemplate
                ))
            ) {
                if policy.alignment_interception == AlignmentInterceptionPolicy::Surface
                    && matches!(
                        command.alignment_adjustment(),
                        crate::processor::AlignmentDeliveryAdjustment::Delimiter(_)
                    )
                {
                    return Ok(DeliveryStatus::AlignmentEndTemplate);
                }
                // This loop's raw fetch is `get_next_with_replay_completion`,
                // which is §341's body without §342's tail, so §342's
                // consequence runs here through the same single helper
                // `get_next` and `get_token` use. `Ok(None)` is §789's
                // `goto restart`: the ⟨v_j⟩ template is live and no reader
                // ever sees the delimiter. Only frozen end-template input
                // from v-template exhaustion falls through to §380 below.
                if matches!(
                    command.alignment_adjustment(),
                    crate::processor::AlignmentDeliveryAdjustment::Delimiter(_)
                ) {
                    self.begin_scalar_alignment_v_template(command)?;
                    *destination = None;
                    continue;
                }
                if expanded.fetch == ExpandedFetch::XToken {
                    // §366 `expand` has no `end_template` shortcut: it routes
                    // straight to §375, which backs up a `frozen_endv` token
                    // for this loop's own `get_next` to reread.
                    self.insert_frozen_endv()?;
                    *destination = None;
                    continue;
                }
                destination
                    .as_mut()
                    .expect("expanded destination contains a command")
                    .convert_end_template_to_endv(self.state.frozen_endv_token());
                return Ok(self.finish_expanded_delivery(
                    destination
                        .as_ref()
                        .expect("expanded destination contains a command"),
                    expanded,
                    expansions_before,
                    policy.alignment_interception,
                ));
            }
            if (expanded.undefined == UndefinedHandling::Preserve
                && matches!(static_meaning(command.meaning()), Some(Meaning::Undefined)))
                || !is_expandable_command(command)
                || (expanded.protected_macros == ProtectedMacroHandling::Preserve
                    && matches!(
                        command.meaning(),
                        ResolvedMeaning::Macro { flags, .. }
                            if flags.contains(MeaningFlags::PROTECTED)
                    ))
            {
                return Ok(self.finish_expanded_delivery(
                    command,
                    expanded,
                    expansions_before,
                    policy.alignment_interception,
                ));
            }
            // TeX82 §394 aborts a non-`\long` macro call after its recovery
            // bookkeeping, then resumes the enclosing expanded-token loop.
            // A user paragraph has been backed up for that loop; an EOF
            // recovery paragraph was consumed by the failed match instead.
            match self.expand_with_trace(
                command,
                !std::mem::take(&mut suppress_first_expansion_trace),
            ) {
                // TeX82 §394 resumes expanded delivery after both an ordinary
                // runaway paragraph and §23's outer-validity recovery has
                // aborted a macro match. The latter leaves the recovered
                // outer token in backup input for its normal reread.
                Ok(())
                | Err(CommandError::ParagraphInMacroArgument)
                | Err(CommandError::OuterInMacroArgument) => {}
                Err(error) => {
                    return Err(error);
                }
            }
            *destination = None;
        }
    }

    fn finish_expanded_delivery(
        &mut self,
        command: &CurrentCommand<G>,
        policy: ExpandedDeliveryPolicy,
        expansions_before: u64,
        alignment: AlignmentInterceptionPolicy,
    ) -> DeliveryStatus {
        self.record_expanded_delivery();
        let pending = policy.observation == ExpandedObservationPolicy::DeferIfExpanded
            && self.command.expansion.cumulative_expansions != expansions_before;
        if policy.observation == ExpandedObservationPolicy::Commit
            || (policy.observation == ExpandedObservationPolicy::DeferIfExpanded && !pending)
        {
            self.observe_expanded_delivery(command);
        }
        if alignment == AlignmentInterceptionPolicy::Surface
            && self.command.alignment.needs_closing_brace_recovery(command)
        {
            return DeliveryStatus::AlignmentClosingBrace;
        }
        if pending {
            DeliveryStatus::PendingExpanded
        } else {
            DeliveryStatus::Command
        }
    }

    #[doc(hidden)]
    pub fn observe_expanded_delivery(&mut self, command: &CurrentCommand<G>) {
        observe!(self, {
            #[cfg(test)]
            {}
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
            CommandObservation::Command(CommandDeliveryRecord {
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
            })
        });
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
    pub(crate) fn expand(&mut self, command: &CurrentCommand<G>) -> Result<(), CommandError> {
        self.expand_with_trace(command, true)
    }

    /// Continues one expansion attempt while preserving §367's already
    /// emitted trace across an immutable-resource suspension.
    fn expand_with_trace(
        &mut self,
        command: &CurrentCommand<G>,
        mut report_trace: bool,
    ) -> Result<(), CommandError> {
        let mut expansion_resume = crate::state::PendingExpansionResume::Dispatch;
        if self.scanner_resume.is_some()
            && !self
                .scanner_resume
                .as_ref()
                .is_some_and(crate::ScannerFrameKey::is_expansion)
        {
            return Err(CommandError::input_invariant());
        }
        if self
            .scanner_resume
            .as_ref()
            .is_some_and(crate::ScannerFrameKey::is_expansion)
        {
            let key = self.scanner_resume.take().expect("matched expansion frame");
            let mut retained = self
                .command
                .scratch
                .take_expansion_frame(key)
                .map_err(crate::scan_toks::scratch_command_error)?;
            if retained.command != *command {
                if let Some(child) = retained.take_child() {
                    self.abort_continuation(child)?;
                }
                return Err(CommandError::input_invariant());
            }
            expansion_resume = retained.resume;
            if let Some(child) = retained.child.take() {
                let (key, destination) = child.restore();
                if destination != crate::state::PendingExpansionChildDestination::Dispatch {
                    return Err(CommandError::input_invariant());
                }
                self.scanner_resume = Some(key);
            }
            report_trace = false;
        }
        #[cfg(feature = "profiling")]
        {
            if !is_ranked_fused_expansion(command.meaning_ref()) {
                tex_state::measurement::record_hot_core_materialization(
                    tex_state::measurement::HotCoreMaterialization::ExpansionCommand,
                );
            }
            match command.meaning_ref() {
                ResolvedMeaning::Static(Meaning::ExpandablePrimitive(primitive)) => {
                    tex_state::measurement::record_hot_core_expandable_opcode(
                        usize::try_from(primitive.operand())
                            .expect("expandable primitive operand fits usize"),
                    );
                }
                ResolvedMeaning::Macro { .. } => {
                    tex_state::measurement::record_hot_core_macro_expansion();
                }
                ResolvedMeaning::Static(Meaning::Undefined) => {}
                _ => unreachable!("expand receives only expandable meanings"),
            }
        }
        if self.write_expansion_depth != 0 {
            self.record_write_expansion();
        }
        self.command.expansion.cumulative_expansions = self
            .command
            .expansion
            .cumulative_expansions
            .saturating_add(1);
        // TeX82 §367 traces non-macro expandable commands inside `expand`,
        // before the primitive consumes operands or changes the input stack.
        // Undefined control sequences reach the same branch through §370.
        // Macros and `end_template` take §366's other two branches and do not
        // cross this diagnostic boundary.
        if report_trace
            && self
                .state
                .int_param(tex_state::env::banks::IntParam::TRACING_COMMANDS)
                > 1
            && (matches!(
                static_meaning(command.meaning()),
                Some(Meaning::ExpandablePrimitive(primitive))
                    if primitive != ExpandablePrimitive::EndTemplate
            ) || matches!(static_meaning(command.meaning()), Some(Meaning::Undefined)))
        {
            self.print_command_trace(crate::PrintCommand::from_current(command));
        }
        let mut suspended_resume = None;
        let result = (|| {
            let meaning = match command.meaning_ref() {
                ResolvedMeaning::Static(meaning) => *meaning,
                ResolvedMeaning::Macro { .. } => {
                    match self.macro_call(command)? {
                        crate::macro_call::MacroCallOutcome::Activated => {}
                        crate::macro_call::MacroCallOutcome::PrefixMismatchRecovered => {}
                    }
                    return Ok(());
                }
            };
            match meaning {
                Meaning::ExpandablePrimitive(primitive)
                    if crate::conditionals::ConditionalKind::from_primitive(primitive)
                        .is_some() =>
                {
                    self.expand_conditional(
                        command,
                        false,
                        &mut expansion_resume,
                        &mut suspended_resume,
                    )
                }
                Meaning::ExpandablePrimitive(ExpandablePrimitive::Unless) => {
                    self.expand_unless(command, &mut expansion_resume, &mut suspended_resume)
                }
                Meaning::ExpandablePrimitive(
                    primitive @ (ExpandablePrimitive::Else
                    | ExpandablePrimitive::Or
                    | ExpandablePrimitive::Fi),
                ) => self.expand_conditional_delimiter(command, primitive),
                // TeX82 §375's `end_template` case replaces the inaccessible
                // sentinel that ended a v-template with the distinct frozen
                // `endv` token. Neither sentinel is a user-installable primitive;
                // §780 gives them only frozen control-sequence slots.
                Meaning::ExpandablePrimitive(ExpandablePrimitive::EndTemplate) => {
                    self.insert_frozen_endv()
                }
                Meaning::ExpandablePrimitive(ExpandablePrimitive::NoExpand) => {
                    self.expand_noexpand()
                }
                Meaning::ExpandablePrimitive(ExpandablePrimitive::ExpandAfter) => {
                    self.expand_expandafter()
                }
                Meaning::ExpandablePrimitive(ExpandablePrimitive::CsName) => {
                    self.expand_csname(command, &mut expansion_resume, &mut suspended_resume)
                }
                Meaning::ExpandablePrimitive(ExpandablePrimitive::String) => {
                    self.expand_string(command)
                }
                Meaning::ExpandablePrimitive(ExpandablePrimitive::Meaning) => {
                    self.expand_meaning(command)
                }
                Meaning::ExpandablePrimitive(ExpandablePrimitive::Number) => {
                    self.expand_number(command, false, &mut expansion_resume, &mut suspended_resume)
                }
                Meaning::ExpandablePrimitive(ExpandablePrimitive::RomanNumeral) => {
                    self.expand_number(command, true, &mut expansion_resume, &mut suspended_resume)
                }
                Meaning::ExpandablePrimitive(ExpandablePrimitive::The) => {
                    self.expand_the(command, &mut expansion_resume, &mut suspended_resume)
                }
                Meaning::ExpandablePrimitive(ExpandablePrimitive::Unexpanded) => {
                    self.expand_unexpanded()
                }
                Meaning::ExpandablePrimitive(ExpandablePrimitive::Expanded) => {
                    self.expand_expanded()
                }
                Meaning::ExpandablePrimitive(ExpandablePrimitive::Detokenize) => {
                    self.expand_detokenize(command)
                }
                Meaning::ExpandablePrimitive(ExpandablePrimitive::Scantokens) => {
                    self.expand_scantokens()
                }
                Meaning::ExpandablePrimitive(ExpandablePrimitive::FontName) => self
                    .expand_fontname(
                        command.copy_for_backup(),
                        &mut expansion_resume,
                        &mut suspended_resume,
                    ),
                // pdftex.web §470's `pdf_font_size_code` conversion prints the
                // selected font size as an ordinary scaled dimension.
                Meaning::ExpandablePrimitive(ExpandablePrimitive::PdfFontSize) => self
                    .expand_pdf_font_size(
                        command.copy_for_backup(),
                        &mut expansion_resume,
                        &mut suspended_resume,
                    ),
                // pdftex.web §470 scans e-TeX's extended box-register domain,
                // then queries typed hlist state for the first non-skipable node
                // at the requested edge.
                Meaning::ExpandablePrimitive(
                    primitive @ (ExpandablePrimitive::LeftMarginKern
                    | ExpandablePrimitive::RightMarginKern),
                ) => self.expand_margin_kern(
                    command.copy_for_backup(),
                    primitive,
                    &mut expansion_resume,
                    &mut suspended_resume,
                ),
                Meaning::ExpandablePrimitive(ExpandablePrimitive::Input) => {
                    self.expand_input(command.copy_for_backup())
                }
                Meaning::ExpandablePrimitive(ExpandablePrimitive::EndInput) => {
                    self.expand_endinput()
                }
                Meaning::ExpandablePrimitive(ExpandablePrimitive::JobName) => {
                    self.state.unsupported_host_capability();
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
                // pdfTeX §57.4 exposes the revision suffix independently of the
                // integer `\pdftexversion` parameter.
                Meaning::ExpandablePrimitive(ExpandablePrimitive::PdfTeXRevision) => {
                    self.push_rendered_text("27", command.origin());
                    Ok(())
                }
                // pdftex.web §§494 and 496--498 install `\pdftexbanner` as an
                // operand-free `convert`: `conv_toks` prints the process banner,
                // then returns it through the ordinary `str_toks`/`ins_list`
                // conversion path. `utils.c::makepdftexbanner` appends the pinned
                // TeX Live and kpathsea identities to pdftex.web §2's banner.
                Meaning::ExpandablePrimitive(ExpandablePrimitive::PdfTeXBanner) => {
                    self.push_rendered_text(
                    "This is pdfTeX, Version 3.141592653-2.6-1.40.29 (TeX Live 2026) kpathsea version 6.4.2",
                    command.origin(),
                );
                    Ok(())
                }
                // pdftex.web §§1587--1588 use the ordinary integer scanner for
                // the signed uniform bound, then advance the single checkpointed
                // MetaPost-derived stream shared with the operand-free normal
                // deviate conversion.
                Meaning::ExpandablePrimitive(ExpandablePrimitive::PdfUniformDeviate) => self
                    .expand_pdf_uniform_deviate(
                        command,
                        &mut expansion_resume,
                        &mut suspended_resume,
                    ),
                Meaning::ExpandablePrimitive(ExpandablePrimitive::PdfNormalDeviate) => {
                    let value = self.state.pdf_normal_deviate();
                    self.push_rendered_text(&value.to_string(), command.origin());
                    Ok(())
                }
                // pdftex.web §1590's `pdf_creation_date_code` conversion calls
                // `getcreationdate`, then returns the fixed job-start timestamp
                // through the ordinary `str_toks`/`ins_list` conversion path.
                // Both the LaTeX-compatible `\creationdate` spelling and
                // pdfTeX's `\pdfcreationdate` spelling share this meaning.
                Meaning::ExpandablePrimitive(ExpandablePrimitive::CreationDate) => {
                    let clock = self.state.job_clock();
                    self.push_rendered_text(&format_pdf_date(clock, 0), command.origin());
                    Ok(())
                }
                // pdfTeX and XeTeX change section [53a] report shell escape as
                // 0 (disabled), 1 (unrestricted), or 2 (restricted). Umber's
                // LaTeX compatibility spelling is an expandable alias over the
                // same tracked World policy used by `\pdfshellescape`.
                Meaning::ExpandablePrimitive(ExpandablePrimitive::ShellEscape) => {
                    let status = self
                        .state
                        .internal_integer(tex_state::meaning::InternalInteger::PdfShellEscape)
                        .expect("the shell-escape status is an integer enquiry");
                    self.push_rendered_text(&status.to_string(), command.origin());
                    Ok(())
                }
                Meaning::ExpandablePrimitive(ExpandablePrimitive::StringCompare) => {
                    self.expand_string_compare(command.copy_for_backup())
                }
                Meaning::ExpandablePrimitive(ExpandablePrimitive::PdfEscapeString) => {
                    self.expand_pdf_escape_string(command.copy_for_backup())
                }
                Meaning::ExpandablePrimitive(ExpandablePrimitive::PdfEscapeHex) => {
                    self.expand_pdf_escape_hex(command.copy_for_backup())
                }
                Meaning::ExpandablePrimitive(ExpandablePrimitive::PdfUnescapeHex) => {
                    self.expand_pdf_unescape_hex(command.copy_for_backup())
                }
                Meaning::ExpandablePrimitive(ExpandablePrimitive::PdfColorStackInit) => self
                    .expand_pdf_color_stack_init(
                        command.copy_for_backup(),
                        &mut expansion_resume,
                        &mut suspended_resume,
                    ),
                Meaning::ExpandablePrimitive(ExpandablePrimitive::PdfMatch) => self
                    .expand_pdf_match(
                        command.copy_for_backup(),
                        &mut expansion_resume,
                        &mut suspended_resume,
                    ),
                Meaning::ExpandablePrimitive(ExpandablePrimitive::PdfLastMatch) => self
                    .expand_pdf_last_match(
                        command.copy_for_backup(),
                        &mut expansion_resume,
                        &mut suspended_resume,
                    ),
                Meaning::ExpandablePrimitive(ExpandablePrimitive::PdfFileDump) => self
                    .expand_pdf_file_dump(
                        command.copy_for_backup(),
                        &mut expansion_resume,
                        &mut suspended_resume,
                    ),
                Meaning::ExpandablePrimitive(ExpandablePrimitive::FileSize) => self
                    .expand_pdf_file_size(
                        command.copy_for_backup(),
                        &mut expansion_resume,
                        &mut suspended_resume,
                    ),
                Meaning::ExpandablePrimitive(ExpandablePrimitive::PdfFileModificationDate) => self
                    .expand_pdf_file_modification_date(
                        command.copy_for_backup(),
                        &mut expansion_resume,
                        &mut suspended_resume,
                    ),
                Meaning::ExpandablePrimitive(ExpandablePrimitive::PdfMdFiveSum) => self
                    .expand_pdf_md_five_sum(
                        command.copy_for_backup(),
                        &mut expansion_resume,
                        &mut suspended_resume,
                    ),
                Meaning::ExpandablePrimitive(ExpandablePrimitive::PdfInsertHeight) => self
                    .expand_pdf_insert_height(
                        command.copy_for_backup(),
                        &mut expansion_resume,
                        &mut suspended_resume,
                    ),
                // pdftex.web §470's `pdf_ximage_bbox_code` conversion scans an
                // existing image object before its one-based page-box coordinate.
                // The enquiry reads detached metadata only; it never reserves an
                // image or writer object while expanding.
                Meaning::ExpandablePrimitive(ExpandablePrimitive::PdfXImageBBox) => self
                    .expand_pdf_ximage_bbox(command, &mut expansion_resume, &mut suspended_resume),
                // pdftex.web §1549's `pdf_xform_name_code` conversion scans a
                // form object number and prints its independent resource identity.
                // Unknown object numbers produce zero, matching the other PDF
                // object enquiries rather than manufacturing ledger state.
                Meaning::ExpandablePrimitive(ExpandablePrimitive::PdfXFormName) => self
                    .expand_pdf_xform_name(command, &mut expansion_resume, &mut suspended_resume),
                // pdftex.web §470's `pdf_page_ref_code` conversion scans a one-based
                // shipped-page number and prints its page-object identity. Pages
                // that do not exist yet expand to zero without reserving
                // speculative writer state; nonpositive operands are rejected by
                // the conversion's `pdf_error` guard.
                Meaning::ExpandablePrimitive(ExpandablePrimitive::PdfPageRef) => {
                    self.expand_pdf_page_ref(command, &mut expansion_resume, &mut suspended_resume)
                }
                // pdfTeX §57.1 consumes one raw token and, only for a registered
                // primitive spelling, replays the immutable frozen primitive.
                // The ordinary expanded loop then dispatches that original
                // meaning without consulting the shadowable live cell.
                Meaning::ExpandablePrimitive(ExpandablePrimitive::PdfPrimitive) => {
                    let mut destination = None;
                    match self.get_next_into(&mut destination)? {
                        DeliveryStatus::End => return Err(CommandError::input_invariant()),
                        DeliveryStatus::Command => {}
                        _ => unreachable!("ordinary raw delivery returns only commands"),
                    }
                    let target = destination
                        .take()
                        .expect("command status initializes destination");
                    let Some(symbol) = target.control_sequence() else {
                        return Ok(());
                    };
                    let name = self.state.resolve(symbol);
                    let Some(frozen) = self.state.primitive_token(name) else {
                        return Ok(());
                    };
                    self.back_input_token(TracedTokenWord::pack(frozen, target.origin()))
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
                ) => {
                    self.expand_mark_class(primitive, &mut expansion_resume, &mut suspended_resume)
                }
                Meaning::ExpandablePrimitive(primitive) => {
                    Err(CommandError::UnsupportedExpandablePrimitive(primitive))
                }
                // TeX82 §207 puts `undefined_cs` immediately above
                // `max_command`, so it reaches §366's `expand` and §367's
                // `othercases`. §370 reports the error and returns without
                // inserting a replacement token; §380 then restarts its one
                // expanded-fetch loop at the following input token.
                Meaning::Undefined => {
                    // §370 reports synchronously at this point. The executor
                    // owns the deferred report, so commit any earlier §537 open
                    // framing now; a later §362 close must remain queued behind
                    // this diagnostic instead of overtaking it.
                    self.command.render_file_framing_events(&mut self.state);
                    let context = self.command.output_open_context(&self.state);
                    self.command.semantic_diagnostics.push(
                        crate::CommandSemanticDiagnostic::UndefinedControlSequence { context },
                    );
                    if !self.command.profile().capabilities().supports_etex() {
                        // TeX82 §370 still owns the recoverable user-visible
                        // error above. The pinned e-TeX 2.6 observer has no
                        // diagnostic seam at that error site, so its detached
                        // event stream advances directly to the next input
                        // transition.
                        self.observe_command_diagnostic("undefined_control_sequence", command);
                    }
                    Ok(())
                }
                _ => Err(CommandError::input_invariant()),
            }
        })();
        if result
            .as_ref()
            .is_err_and(CommandError::is_resource_suspension)
        {
            let key =
                match self
                    .command
                    .scratch
                    .store_expansion_frame(crate::state::PendingExpansion {
                        command: command.clone(),
                        resume: suspended_resume
                            .take()
                            .unwrap_or(crate::state::PendingExpansionResume::Dispatch),
                        child: None,
                    }) {
                    Ok(key) => key,
                    Err(store_error) => {
                        if let Some(child) = self.scanner_resume.take() {
                            self.abort_continuation(child)?;
                        }
                        return Err(crate::scan_toks::scratch_command_error(store_error));
                    }
                };
            let child = crate::execution_scratch::ChildContinuation::capture(
                &mut self.scanner_resume,
                crate::state::PendingExpansionChildDestination::Dispatch,
            );
            match self.command.scratch.expansion_frame_mut(&key) {
                Ok(pending) => pending.child = child,
                Err(store_error) => {
                    let abort_result = if let Some(child) = child {
                        self.abort_continuation(child.restore().0)
                    } else {
                        Ok(())
                    };
                    let discard_result = self
                        .command
                        .scratch
                        .discard_expansion_frame(key)
                        .map_err(crate::scan_toks::scratch_command_error);
                    abort_result?;
                    discard_result?;
                    return Err(crate::scan_toks::scratch_command_error(store_error));
                }
            }
            self.scanner_resume = Some(key);
        } else if let Some(child) = self.scanner_resume.take() {
            self.abort_continuation(child)?;
            if result.is_ok() {
                return Err(CommandError::input_invariant());
            }
        }
        result
    }

    /// e-TeX 2.6 etex.ch §53a `pseudo_start`.
    fn expand_scantokens(&mut self) -> Result<(), CommandError> {
        let scanned = self.scan_toks(crate::scan_toks::ScanToksMode::GeneralText {
            purpose: "scantokens",
        })?;
        let mut text = self.attempt_token_list_string_text(scanned.replacement_text)?;
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
            .token_parameter(tex_state::env::banks::TokParam::EVERY_EOF)
            .expect("everyeof is an admitted token parameter");
        let tracing_scantokens = self.state.int_param(IntParam::TRACING_SCAN_TOKENS);
        let level = self
            .command
            .open_scantokens(
                SourceRegistration::new(RegisteredSourceKind::Generated, text.into_bytes()),
                every_eof,
                scantokens_numeric_name(tracing_scantokens),
            )
            .map_err(|_| CommandError::input_invariant())?;
        self.command.record_source_open_depths(
            level,
            self.state.group_lineages().into_boxed_slice(),
            self.command
                .conditions
                .frames
                .iter()
                .map(|frame| frame.identity.0)
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        );
        let source = self
            .command
            .active_source_snapshot()
            .ok_or(CommandError::input_invariant())?;
        // e-TeX 2.6 etex.ch §53a assigns `name=19` while
        // `\tracingscantokens>0`, and `name=18` otherwise. TeX82 §48's
        // initial character strings render those names as `^^S` and `^^R`.
        let source_name = scantokens_source_name(tracing_scantokens);
        let source_id = source.id;
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
            source: Some(source_id),
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
    fn expand_detokenize(&mut self, opener: &CurrentCommand<G>) -> Result<(), CommandError> {
        let scanned = self.scan_toks(crate::scan_toks::ScanToksMode::GeneralText {
            purpose: "detokenize",
        })?;
        let text = self.attempt_token_list_string_text(scanned.replacement_text)?;
        self.push_rendered_text(&text, opener.origin());
        Ok(())
    }

    /// pdftex.web §§495 and 1535's `\expanded` conversion.
    ///
    /// `scan_pdf_ext_toks` is exactly `scan_toks(false, true)`: it expands one
    /// balanced general-text argument and returns the resulting token list via
    /// `ins_list`. The inserted list therefore reenters the caller's current
    /// expansion loop instead of being rendered to characters.
    fn expand_expanded(&mut self) -> Result<(), CommandError> {
        let scanned = self.scan_toks(crate::scan_toks::ScanToksMode::General { expanded: true })?;
        let replacement = scanned.replacement_text;
        let words = self
            .command
            .attempt
            .arena()
            .token_words(replacement)
            .map_err(crate::scan_toks::attempt_command_error)?
            .to_vec();
        let first = words.first().map(|word| word.semantic_token());
        self.insert_expansion_list(PackedTokenSpanHandle::transient(words), first);
        Ok(())
    }

    /// pdftex.web §§495 and 1535's `compare_strings` conversion.
    ///
    /// Both operands are independently collected by `scan_pdf_ext_toks`,
    /// rendered through `tokens_to_string`, and compared lexicographically as
    /// pdfTeX string-pool bytes. Canonical pdfTeX input is byte-valued; UTF-8
    /// preserves that ordering for Umber's extended scalar domain as well.
    fn expand_string_compare(&mut self, opener: CurrentCommand<G>) -> Result<(), CommandError> {
        let pending = if self
            .scanner_resume
            .as_ref()
            .is_some_and(crate::ScannerFrameKey::is_pdf_string_compare)
        {
            let key = self
                .scanner_resume
                .take()
                .expect("matched pdf string-compare frame");
            Some(
                self.command
                    .scratch
                    .take_pdf_string_compare_frame(key)
                    .map_err(crate::scan_toks::scratch_command_error)?,
            )
        } else {
            None
        };
        let phase = pending
            .as_ref()
            .map_or(PdfStringComparePhase::Left, |pending| pending.phase);
        if let Some(mut pending) = pending
            && let Some(child) = pending.child.take()
        {
            let (key, destination) = child.restore();
            if destination != PdfStringCompareDestination::Scan {
                return Err(CommandError::input_invariant());
            }
            self.scanner_resume = Some(key);
        }
        let left = match phase {
            PdfStringComparePhase::Left => {
                match self.scan_toks(crate::scan_toks::ScanToksMode::General { expanded: true }) {
                    Ok(scanned) => scanned.replacement_text,
                    Err(error) => {
                        if error.is_resource_suspension() {
                            let key = self
                                .command
                                .scratch
                                .store_pdf_string_compare_frame(PendingPdfStringCompare {
                                    phase: PdfStringComparePhase::Left,
                                    child: crate::execution_scratch::ChildContinuation::capture(
                                        &mut self.scanner_resume,
                                        PdfStringCompareDestination::Scan,
                                    ),
                                })
                                .map_err(crate::scan_toks::scratch_command_error)?;
                            self.scanner_resume = Some(key);
                        }
                        return Err(error);
                    }
                }
            }
            PdfStringComparePhase::Right { left } => left,
        };
        let right = match self.scan_toks(crate::scan_toks::ScanToksMode::General { expanded: true })
        {
            Ok(scanned) => scanned,
            Err(error) => {
                if error.is_resource_suspension() {
                    let key = self
                        .command
                        .scratch
                        .store_pdf_string_compare_frame(PendingPdfStringCompare {
                            phase: PdfStringComparePhase::Right { left },
                            child: crate::execution_scratch::ChildContinuation::capture(
                                &mut self.scanner_resume,
                                PdfStringCompareDestination::Scan,
                            ),
                        })
                        .map_err(crate::scan_toks::scratch_command_error)?;
                    self.scanner_resume = Some(key);
                }
                return Err(error);
            }
        };
        let left = self.attempt_token_list_string_text(left)?;
        let right = self.attempt_token_list_string_text(right.replacement_text)?;
        let value = match left.as_bytes().cmp(right.as_bytes()) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        };
        self.push_rendered_text(&value.to_string(), opener.origin());
        Ok(())
    }

    /// pdftex.web §495's `pdf_escape_string_code` conversion.
    ///
    /// The operand is one expanded general-text token list. `tokens_to_string`
    /// first projects that list to pdfTeX's byte string, then `escapestring`
    /// writes a PDF literal-string body: parentheses and backslashes gain an
    /// escape prefix, while bytes outside `!` through `~` use three octal
    /// digits. The result reenters expansion as category-12 characters.
    fn expand_pdf_escape_string(&mut self, opener: CurrentCommand<G>) -> Result<(), CommandError> {
        let scanned = self.scan_toks(crate::scan_toks::ScanToksMode::General { expanded: true })?;
        let text = self.attempt_token_list_string_text(scanned.replacement_text)?;
        let escaped = escape_pdf_literal_string(&text);
        observe!(
            self,
            CommandObservation::TokenList(TokenListRecord {
                transition: "complete",
                purpose: "pdf_escape_string",
                tokens: escaped
                    .chars()
                    .map(|ch| {
                        self.observed_token(TracedTokenWord::pack(
                            Token::Char {
                                ch,
                                cat: Catcode::Other,
                            },
                            OriginId::UNKNOWN,
                        ))
                    })
                    .collect(),
            }),
        );
        self.push_rendered_text(&escaped, opener.origin());
        Ok(())
    }

    /// pdftex.web §§494 and 496--497's `pdf_escape_hex_code` conversion.
    ///
    /// The operand is one expanded general-text token list. `tokens_to_string`
    /// projects that list to pdfTeX bytes, then `escapehex` writes exactly two
    /// uppercase hexadecimal digits for every byte, without angle brackets.
    /// TeX82 §464's `str_toks` returns those digits as category-12 characters.
    fn expand_pdf_escape_hex(&mut self, opener: CurrentCommand<G>) -> Result<(), CommandError> {
        let scanned = self.scan_toks(crate::scan_toks::ScanToksMode::General { expanded: true })?;
        let bytes = self.attempt_token_list_bytes(scanned.replacement_text)?;
        let escaped = escape_pdf_hex(&bytes);
        observe!(
            self,
            CommandObservation::TokenList(TokenListRecord {
                transition: "complete",
                purpose: "pdf_escape_hex",
                tokens: escaped
                    .chars()
                    .map(|ch| {
                        self.observed_token(TracedTokenWord::pack(
                            Token::Char {
                                ch,
                                cat: Catcode::Other,
                            },
                            OriginId::UNKNOWN,
                        ))
                    })
                    .collect(),
            }),
        );
        self.push_rendered_text(&escaped, opener.origin());
        Ok(())
    }

    /// pdftex.web §§494 and 496--497's `pdf_unescape_hex_code` conversion.
    ///
    /// After one expanded general-text operand is projected to pdfTeX bytes,
    /// `unescapehex` ignores non-hexadecimal bytes, combines each pair of
    /// remaining digits case-insensitively, and pads a final high nibble with
    /// zero. TeX82 §464's `str_toks` makes a decoded space category 10 and
    /// every other decoded byte category 12.
    fn expand_pdf_unescape_hex(&mut self, opener: CurrentCommand<G>) -> Result<(), CommandError> {
        let scanned = self.scan_toks(crate::scan_toks::ScanToksMode::General { expanded: true })?;
        let bytes = self.attempt_token_list_bytes(scanned.replacement_text)?;
        let unescaped = unescape_pdf_hex(&bytes);
        observe!(
            self,
            CommandObservation::TokenList(TokenListRecord {
                transition: "complete",
                purpose: "pdf_unescape_hex",
                tokens: unescaped
                    .chars()
                    .map(|ch| {
                        self.observed_token(TracedTokenWord::pack(
                            Token::Char {
                                ch,
                                cat: if ch == ' ' {
                                    Catcode::Space
                                } else {
                                    Catcode::Other
                                },
                            },
                            OriginId::UNKNOWN,
                        ))
                    })
                    .collect(),
            }),
        );
        self.push_rendered_text(&unescaped, opener.origin());
        Ok(())
    }

    /// TeX.web's `\noexpand`: read normally, then replay exactly one target
    /// from a backed-up level carrying the non-sticky suppression treatment.
    fn expand_noexpand(&mut self) -> Result<(), CommandError> {
        let mut destination = None;
        match self.get_token_with_normal_scanner_status_into(&mut destination)? {
            DeliveryStatus::End => return Err(CommandError::input_invariant()),
            DeliveryStatus::Command => {}
            _ => unreachable!("ordinary token delivery returns only commands"),
        }
        let target = destination
            .take()
            .expect("command status initializes destination");
        self.back_input_with_treatment(target, BackupTreatment::SuppressExpandableControlSequence)
    }

    /// Reads one token with TeX82's temporary `scanner_status := normal`
    /// scope, restoring the complete prior scanner state before returning.
    ///
    /// Both `\noexpand` (§25) and `conv_toks`'s `\string`/`\meaning` cases
    /// (§27) need this scope: their operand is delivered normally even while
    /// an enclosing `\edef` is collecting replacement text.
    fn get_token_with_normal_scanner_status_into(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        if matches!(self.command.scanner.status(), ScannerStatus::Normal) {
            return self.get_token_into(destination);
        }

        let episode =
            self.begin_scanner_episode(ScannerStatus::Normal, ScannerStatusVisibility::Observed);
        let delivery = self.get_token_into(destination);
        self.finish_scanner_episode(episode);
        delivery
    }

    /// TeX.web's `\expandafter`: preserve the first token, expand (or back
    /// up) the second token, then put the first token above the resulting
    /// input. The first delivery is intentionally replayed through an
    /// explicit backed-up level because it is no longer the latest delivery.
    fn expand_expandafter(&mut self) -> Result<(), CommandError> {
        let pending = if self
            .scanner_resume
            .as_ref()
            .is_some_and(crate::ScannerFrameKey::is_expandafter)
        {
            let key = self
                .scanner_resume
                .take()
                .expect("matched expandafter frame");
            Some(
                self.command
                    .scratch
                    .take_expandafter_frame(key)
                    .map_err(crate::scan_toks::scratch_command_error)?,
            )
        } else {
            None
        };
        let (first, second) = if let Some(mut pending) = pending {
            self.resume_current_command(&pending.second);
            if let Some(child) = pending.child.take() {
                let (key, destination) = child.restore();
                if destination != PendingExpandAfterDestination::ExpandingSecond {
                    return Err(CommandError::input_invariant());
                }
                self.install_scanner_resume(Some(key));
            }
            (pending.first, pending.second)
        } else {
            let mut first = None;
            match self.get_token_into(&mut first)? {
                DeliveryStatus::End => return Err(CommandError::input_invariant()),
                DeliveryStatus::Command => {}
                _ => unreachable!("ordinary token delivery returns only commands"),
            }
            let first = first
                .take()
                .expect("command status initializes destination");
            let mut second = None;
            match self.get_token_into(&mut second)? {
                DeliveryStatus::End => return Err(CommandError::input_invariant()),
                DeliveryStatus::Command => {}
                _ => unreachable!("ordinary token delivery returns only commands"),
            }
            let second = second
                .take()
                .expect("command status initializes destination");
            (first, second)
        };
        if is_expandable_command(&second) {
            if let Err(error) = self.expand(&second) {
                if error.is_resource_suspension() {
                    let key = self
                        .command
                        .scratch
                        .store_expandafter_frame(PendingExpandAfter {
                            first,
                            second,
                            child: crate::execution_scratch::ChildContinuation::capture(
                                &mut self.scanner_resume,
                                PendingExpandAfterDestination::ExpandingSecond,
                            ),
                        })
                        .map_err(crate::scan_toks::scratch_command_error)?;
                    self.scanner_resume = Some(key);
                }
                return Err(error);
            }
            if self.scanner_resume.is_some() {
                return Err(CommandError::input_invariant());
            }
            self.replay_expandafter_first(first)?;
        } else {
            self.back_input(second)?;
            self.replay_expandafter_first(first)?;
        }
        Ok(())
    }

    /// TeX.web's `\\csname`: collect ordinary expanded character commands
    /// until the inaccessible `\\endcsname` boundary, then inject the one
    /// named control-sequence token through normal input delivery.
    fn expand_csname(
        &mut self,
        opener: &CurrentCommand<G>,
        resume: &mut crate::state::PendingExpansionResume,
        suspended: &mut Option<crate::state::PendingExpansionResume>,
    ) -> Result<(), CommandError> {
        let name = match std::mem::replace(resume, crate::state::PendingExpansionResume::Dispatch) {
            crate::state::PendingExpansionResume::Dispatch => String::new(),
            crate::state::PendingExpansionResume::CsName { name } => name,
            _ => return Err(CommandError::input_invariant()),
        };
        let mut suspended_name = None;
        let name = match self.scan_csname_characters(name, &mut suspended_name) {
            Ok(name) => name,
            Err(error) => {
                if error.is_resource_suspension() {
                    *suspended = suspended_name
                        .map(|name| crate::state::PendingExpansionResume::CsName { name });
                }
                return Err(error);
            }
        };
        let symbol = self.state.intern_relaxed_control_sequence(&name);
        self.back_input_token(TracedTokenWord::pack(Token::Cs(symbol), opener.origin()))
    }

    /// Collects TeX82 §372's expanded character list through `\\endcsname`.
    ///
    /// e-TeX 2.6 etex.ch [17.4765--4779] deliberately reuses this exact
    /// name-building scan for `\\ifcsname`; only the subsequent hash-table
    /// operation differs.
    pub(crate) fn scan_csname_characters(
        &mut self,
        mut name: String,
        suspended: &mut Option<String>,
    ) -> Result<String, CommandError> {
        // pdfTeX section 57 saves and restores the prior flag so nested name
        // scans remain true to ifincsname and unwind to their caller.
        let previous = std::mem::replace(&mut self.is_in_csname, true);
        let result = (|| {
            let mut destination = None;
            loop {
                let status = match self.get_x_token_into(&mut destination) {
                    Ok(status) => status,
                    Err(error) => {
                        if error.is_resource_suspension() {
                            *suspended = Some(name);
                        }
                        return Err(error);
                    }
                };
                match status {
                    DeliveryStatus::End => return Err(CommandError::input_invariant()),
                    DeliveryStatus::Command => {}
                    _ => unreachable!("ordinary expanded delivery returns only commands"),
                }
                let command = destination
                    .take()
                    .expect("command status initializes destination");
                match command.meaning() {
                    ResolvedMeaning::Static(Meaning::ExpandablePrimitive(
                        ExpandablePrimitive::EndCsName,
                    )) => break,
                    ResolvedMeaning::Static(Meaning::CharToken { ch, .. }) => name.push(ch),
                    _ => {
                        let rendered = print_esc_text(&self.state, "endcsname");
                        self.back_error_reporting(
                            command,
                            MISSING_ENDCSNAME_DIAGNOSTIC,
                            format!("Missing {rendered} inserted"),
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
        })();
        if let Ok(name) = &result {
            self.command
                .record_csname_buffer_usage(name.chars().count());
        }
        self.is_in_csname = previous;
        result
    }

    /// `\\string` observes spelling, never an effective control-sequence meaning.
    fn expand_string(&mut self, opener: &CurrentCommand<G>) -> Result<(), CommandError> {
        let mut destination = None;
        match self.get_token_with_normal_scanner_status_into(&mut destination)? {
            DeliveryStatus::End => return Err(CommandError::input_invariant()),
            DeliveryStatus::Command => {}
            _ => unreachable!("ordinary token delivery returns only commands"),
        }
        let target = destination
            .take()
            .expect("command status initializes destination");
        self.push_rendered_text(
            &string_text(&self.state, target.spelling().semantic_token()),
            opener.origin(),
        );
        Ok(())
    }

    fn expand_meaning(&mut self, opener: &CurrentCommand<G>) -> Result<(), CommandError> {
        let mut destination = None;
        match self.get_token_with_normal_scanner_status_into(&mut destination)? {
            DeliveryStatus::End => return Err(CommandError::input_invariant()),
            DeliveryStatus::Command => {}
            _ => unreachable!("ordinary token delivery returns only commands"),
        }
        let target = destination
            .take()
            .expect("command status initializes destination");
        let text = meaning_text(&mut self.state, &target);
        self.push_rendered_text(&text, opener.origin());
        Ok(())
    }

    fn expand_number(
        &mut self,
        opener: &CurrentCommand<G>,
        roman: bool,
        resume: &mut crate::state::PendingExpansionResume,
        suspended: &mut Option<crate::state::PendingExpansionResume>,
    ) -> Result<(), CommandError> {
        match std::mem::replace(resume, crate::state::PendingExpansionResume::Dispatch) {
            crate::state::PendingExpansionResume::Dispatch => {}
            crate::state::PendingExpansionResume::Number {
                roman: retained_roman,
            } if retained_roman == roman => {}
            _ => return Err(CommandError::input_invariant()),
        }
        let scan = self.scan_integer_retained();
        let value = self
            .retain_expansion_scalar(
                scan,
                crate::state::PendingExpansionResume::Number { roman },
                suspended,
            )?
            .value;
        let text = if roman {
            roman_numeral(value)
        } else {
            value.to_string()
        };
        self.push_rendered_text(&text, opener.origin());
        Ok(())
    }

    fn retain_expansion_scalar<T>(
        &mut self,
        scan: crate::RetainedScalarScan<G, T>,
        phase: crate::state::PendingExpansionResume,
        suspended: &mut Option<crate::state::PendingExpansionResume>,
    ) -> Result<T, CommandError> {
        match scan {
            crate::RetainedScalarScan::Complete(value) => Ok(value),
            crate::RetainedScalarScan::Suspended { error, child } => {
                self.install_scanner_resume(Some(child));
                *suspended = Some(phase);
                Err(error)
            }
            crate::RetainedScalarScan::Failed(error) => Err(error),
        }
    }

    /// Expands TeX82 `the_toks` after command-owned internal-quantity scanning.
    ///
    /// The internal scanner owns a primitive register's `scan_eight_bit_int`
    /// episode.  In particular, `\\the\\count21` must deliver both index digits
    /// before it backs up the next source token and installs rendered output.
    /// Reaching into the target meaning here would leave that index to a later
    /// scanner and changes the observable input ordering.
    fn expand_the(
        &mut self,
        opener: &CurrentCommand<G>,
        resume: &mut crate::state::PendingExpansionResume,
        suspended: &mut Option<crate::state::PendingExpansionResume>,
    ) -> Result<(), CommandError> {
        if !matches!(
            std::mem::replace(resume, crate::state::PendingExpansionResume::Dispatch),
            crate::state::PendingExpansionResume::Dispatch
                | crate::state::PendingExpansionResume::The
        ) {
            return Err(CommandError::input_invariant());
        }
        let scan = self.scan_internal_value_or_zero_retained();
        let target = self.retain_expansion_scalar(
            scan,
            crate::state::PendingExpansionResume::The,
            suspended,
        )?;
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
        if let Some(text) = render_the_value(&value) {
            self.push_rendered_text(&text, opener);
        } else {
            match value {
                // §466 copies the register's list rather than sharing its
                // durable source. The operation-local copy remains in the
                // attempt until this inserted level has copied its words.
                crate::InternalValue::Tokens { tokens } => {
                    let words = self
                        .command
                        .attempt
                        .arena()
                        .token_words(tokens)
                        .map_err(crate::scan_toks::attempt_command_error)?
                        .to_vec();
                    let first = words.first().map(|word| word.semantic_token());
                    self.insert_expansion_list(PackedTokenSpanHandle::transient(words), first);
                }
                crate::InternalValue::Font(symbol) => {
                    self.push_rendered_tokens([Token::Cs(symbol)], opener);
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
    fn expand_fontname(
        &mut self,
        opener: CurrentCommand<G>,
        resume: &mut crate::state::PendingExpansionResume,
        suspended: &mut Option<crate::state::PendingExpansionResume>,
    ) -> Result<(), CommandError> {
        if !matches!(
            std::mem::replace(resume, crate::state::PendingExpansionResume::Dispatch),
            crate::state::PendingExpansionResume::Dispatch
                | crate::state::PendingExpansionResume::FontName
        ) {
            return Err(CommandError::input_invariant());
        }
        let scan = self.scan_font_selector_retained();
        let font = self.retain_expansion_scalar(
            scan,
            crate::state::PendingExpansionResume::FontName,
            suspended,
        )?;
        let mut name = self.state.font_name(font);
        let size = self.state.font_size(font);
        if size != self.state.font_design_size(font) {
            // TeX82 §472 appends `at <size>pt` whenever the selected size
            // differs from the TFM design size. This text is inserted as
            // catcode-12/space tokens by `str_toks`, so it must be complete
            // before an enclosing `\edef` captures it.
            name.push_str(" at ");
            append_scaled_without_unit(size, &mut name);
            name.push_str("pt");
        }
        self.push_rendered_text(&name, opener.origin());
        Ok(())
    }

    fn expand_pdf_font_size(
        &mut self,
        opener: CurrentCommand<G>,
        resume: &mut crate::state::PendingExpansionResume,
        suspended: &mut Option<crate::state::PendingExpansionResume>,
    ) -> Result<(), CommandError> {
        if !matches!(
            std::mem::replace(resume, crate::state::PendingExpansionResume::Dispatch),
            crate::state::PendingExpansionResume::Dispatch
                | crate::state::PendingExpansionResume::PdfFontSize
        ) {
            return Err(CommandError::input_invariant());
        }
        let scan = self.scan_font_selector_retained();
        let font = self.retain_expansion_scalar(
            scan,
            crate::state::PendingExpansionResume::PdfFontSize,
            suspended,
        )?;
        let size = format_scaled(self.state.tracked_font_size(font));
        self.push_rendered_text(&size, opener.origin());
        Ok(())
    }

    fn expand_margin_kern(
        &mut self,
        opener: CurrentCommand<G>,
        primitive: ExpandablePrimitive,
        resume: &mut crate::state::PendingExpansionResume,
        suspended: &mut Option<crate::state::PendingExpansionResume>,
    ) -> Result<(), CommandError> {
        match std::mem::replace(resume, crate::state::PendingExpansionResume::Dispatch) {
            crate::state::PendingExpansionResume::Dispatch => {}
            crate::state::PendingExpansionResume::PdfMarginKern {
                primitive: retained,
            } if retained == primitive => {}
            _ => return Err(CommandError::input_invariant()),
        }
        let scan = self.scan_extended_register_index_retained();
        let index = self.retain_expansion_scalar(
            scan,
            crate::state::PendingExpansionResume::PdfMarginKern { primitive },
            suspended,
        )?;
        let side = match primitive {
            ExpandablePrimitive::LeftMarginKern => tex_state::node::MarginKernSide::Left,
            ExpandablePrimitive::RightMarginKern => tex_state::node::MarginKernSide::Right,
            _ => return Err(CommandError::input_invariant()),
        };
        let Some(amount) = self.state.box_margin_kern(index, side) else {
            return Err(CommandError::PdfNavigation(
                "pdfTeX error (marginkern): a non-empty hbox expected",
            ));
        };
        self.push_rendered_text(&format_scaled(amount), opener.origin());
        Ok(())
    }

    fn expand_input(&mut self, opener: CurrentCommand<G>) -> Result<(), CommandError> {
        if self.command.name_in_progress() {
            // TeX82 §§378/527 call §378's `insert_relax`: two distinct
            // `back_input` operations first restore the recursively
            // encountered `\input`, then place inaccessible `frozen_relax`
            // above it and retype only that second level as `inserted`. The
            // distinction is observable after the relax terminates the
            // active filename scan: its depleted inserted level retires, so
            // a diagnostic on the restored command says `<recently read>`.
            let opener_origin = opener.origin();
            self.back_input(opener)?;
            let frozen_relax = TracedTokenWord::pack(Token::frozen_relax(), opener_origin);
            let level = self.command.push_token_level(
                PackedTokenSpanHandle::backed_up([BackedUpToken {
                    spelling: frozen_relax,
                    source_provenance: None,
                }]),
                TokenBehavior::BackedUp(BackupTreatment::Ordinary),
                RetirementBehavior::Pop,
                ReplayTrace::Inserted,
            );
            self.observe_inserted_token_recovery(level, Token::frozen_relax());
            return Ok(());
        }
        let _input = self
            .open_registered_input()
            .map_err(|error| error.at_origin_unless_resource(opener.origin()))?;
        observe!(
            self,
            CommandObservation::Effect(EffectRecord {
                kind: crate::ObservationEffectKind::Input,
                channel: _input.file_name.packed(),
                value: crate::ObservationValue::None,
                source: Some(crate::observation::OpenedSourceSnapshot {
                    id: _input.source,
                    bytes: _input.bytes,
                }),
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
        if let Some(tokens) = self.state.page_mark_value(page_mark(primitive)).cloned() {
            self.push_mark_text(&tokens);
        }
        Ok(())
    }

    fn expand_mark_class(
        &mut self,
        primitive: ExpandablePrimitive,
        resume: &mut crate::state::PendingExpansionResume,
        suspended: &mut Option<crate::state::PendingExpansionResume>,
    ) -> Result<(), CommandError> {
        // e-TeX 2.6 `etex.ch` [26.1178] uses the same
        // `scan_register_num` as numbered marks and sparse registers.
        match std::mem::replace(resume, crate::state::PendingExpansionResume::Dispatch) {
            crate::state::PendingExpansionResume::Dispatch => {}
            crate::state::PendingExpansionResume::MarkClass {
                primitive: retained,
            } if retained == primitive => {}
            _ => return Err(CommandError::input_invariant()),
        }
        let scan = self.scan_extended_register_index_retained();
        let class = self.retain_expansion_scalar(
            scan,
            crate::state::PendingExpansionResume::MarkClass { primitive },
            suspended,
        )?;
        // e-TeX 2.6 etex.ch [25.386] makes class zero an exact alias for
        // TeX82's `cur_mark`, including its null-versus-empty pointer state.
        let tokens = self
            .state
            .page_mark_class_value(page_mark(primitive), class);
        if let Some(tokens) = tokens.cloned() {
            self.push_mark_text(&tokens);
        }
        Ok(())
    }

    /// Installs TeX82 §386's `mark_text` level for `\\topmark` and its kin.
    ///
    /// §386 is `begin_token_list(cur_mark[cur_chr], mark_text)`, a distinct
    /// §307 token type from §467's `inserted`: a mark's text is the stored list
    /// itself, never a copy handed back through `ins_list`.
    fn expand_pdf_match(
        &mut self,
        opener: CurrentCommand<G>,
        resume: &mut crate::state::PendingExpansionResume,
        suspended: &mut Option<crate::state::PendingExpansionResume>,
    ) -> Result<(), CommandError> {
        let pending = std::mem::replace(resume, crate::state::PendingExpansionResume::Dispatch);
        let (mut case_insensitive, mut subcount, mut option_phase, retained_pattern) = match pending
        {
            crate::state::PendingExpansionResume::Dispatch => (false, 10_u32, Some(0_u8), None),
            crate::state::PendingExpansionResume::PdfMatchOptions {
                case_insensitive,
                subcount,
                phase,
            } => (case_insensitive, subcount, Some(phase), None),
            crate::state::PendingExpansionResume::PdfMatchPattern {
                case_insensitive,
                subcount,
            } => (case_insensitive, subcount, None, None),
            crate::state::PendingExpansionResume::PdfMatchHaystack {
                case_insensitive,
                subcount,
                pattern,
            } => (case_insensitive, subcount, None, Some(pattern)),
            _ => return Err(CommandError::input_invariant()),
        };
        while let Some(phase) = option_phase {
            match phase {
                0 => {
                    let scan = self.scan_keyword_retained("icase");
                    if self
                        .retain_expansion_scalar(
                            scan,
                            crate::state::PendingExpansionResume::PdfMatchOptions {
                                case_insensitive,
                                subcount,
                                phase,
                            },
                            suspended,
                        )?
                        .value
                    {
                        case_insensitive = true;
                    } else {
                        option_phase = Some(1);
                        continue;
                    }
                }
                1 => {
                    let scan = self.scan_keyword_retained("subcount");
                    if self
                        .retain_expansion_scalar(
                            scan,
                            crate::state::PendingExpansionResume::PdfMatchOptions {
                                case_insensitive,
                                subcount,
                                phase,
                            },
                            suspended,
                        )?
                        .value
                    {
                        option_phase = Some(2);
                        continue;
                    }
                    option_phase = None;
                    continue;
                }
                2 => {
                    let scan = self.scan_integer_retained();
                    subcount = self
                        .retain_expansion_scalar(
                            scan,
                            crate::state::PendingExpansionResume::PdfMatchOptions {
                                case_insensitive,
                                subcount,
                                phase,
                            },
                            suspended,
                        )?
                        .value
                        .max(0) as u32;
                }
                _ => return Err(CommandError::input_invariant()),
            }
            option_phase = Some(0);
        }
        let pattern = if let Some(pattern) = retained_pattern {
            pattern
        } else {
            match self.scan_balanced_text(true) {
                Ok(pattern) => pattern.tokens,
                Err(error) => {
                    if error.is_resource_suspension() {
                        *suspended = Some(crate::state::PendingExpansionResume::PdfMatchPattern {
                            case_insensitive,
                            subcount,
                        });
                    }
                    return Err(error);
                }
            }
        };
        let haystack = match self.scan_balanced_text(true) {
            Ok(haystack) => haystack.tokens,
            Err(error) => {
                if error.is_resource_suspension() {
                    *suspended = Some(crate::state::PendingExpansionResume::PdfMatchHaystack {
                        case_insensitive,
                        subcount,
                        pattern,
                    });
                }
                return Err(error);
            }
        };
        let pattern = pdftex_c_string(self.attempt_token_list_bytes(pattern)?);
        let haystack = pdftex_c_string(self.attempt_token_list_bytes(haystack)?);
        let regex = match PosixRegexBuilder::new(&pattern)
            .with_default_classes()
            .extended(true)
            .compile()
        {
            Ok(regex) => regex.case_insensitive(case_insensitive),
            Err(error) => {
                self.pdftex_regex_warning(posix_regex_diagnostic(&error, &pattern));
                self.push_rendered_text("-1", opener.origin());
                return Ok(());
            }
        };
        let captures = regex.matches(&haystack, Some(1)).into_iter().next();
        let matched = captures.is_some();
        let captures = captures
            .unwrap_or_default()
            .iter()
            .take(subcount as usize)
            .map(|capture| {
                capture.map(|(start, end)| {
                    (
                        u32::try_from(start).expect("bounded TeX string offset fits u32"),
                        u32::try_from(end).expect("bounded TeX string offset fits u32"),
                    )
                })
            })
            .collect();
        self.state
            .set_pdf_match_state(haystack, captures, subcount, matched);
        self.push_rendered_text(if matched { "1" } else { "0" }, opener.origin());
        Ok(())
    }

    /// pdftex.web §495's `pdf_colorstack_init_code` conversion.
    fn expand_pdf_color_stack_init(
        &mut self,
        opener: CurrentCommand<G>,
        resume: &mut crate::state::PendingExpansionResume,
        suspended: &mut Option<crate::state::PendingExpansionResume>,
    ) -> Result<(), CommandError> {
        let (mut restore_at_page_start, mut option_phase, retained_mode) =
            match std::mem::replace(resume, crate::state::PendingExpansionResume::Dispatch) {
                crate::state::PendingExpansionResume::Dispatch => (false, Some(0_u8), None),
                crate::state::PendingExpansionResume::PdfColorStackInitOptions {
                    restore_at_page_start,
                    phase,
                } => (restore_at_page_start, Some(phase), None),
                crate::state::PendingExpansionResume::PdfColorStackInitText {
                    restore_at_page_start,
                    mode,
                } => (restore_at_page_start, None, Some(mode)),
                _ => return Err(CommandError::input_invariant()),
            };
        let mode = if let Some(mut phase) = option_phase.take() {
            loop {
                let keyword = match phase {
                    0 => "page",
                    1 => "direct",
                    2 => "page",
                    _ => return Err(CommandError::input_invariant()),
                };
                let scan = self.scan_keyword_retained(keyword);
                let matched = self
                    .retain_expansion_scalar(
                        scan,
                        crate::state::PendingExpansionResume::PdfColorStackInitOptions {
                            restore_at_page_start,
                            phase,
                        },
                        suspended,
                    )?
                    .value;
                match phase {
                    0 => {
                        restore_at_page_start = matched;
                        phase = 1;
                    }
                    1 if matched => break tex_state::PdfColorStackMode::Direct,
                    1 => phase = 2,
                    2 if matched => break tex_state::PdfColorStackMode::Page,
                    2 => break tex_state::PdfColorStackMode::Origin,
                    _ => unreachable!(),
                }
            }
        } else {
            retained_mode.expect("completed color-stack options retain their mode")
        };
        let initial = match self.scan_balanced_text(true) {
            Ok(initial) => initial.tokens,
            Err(error) => {
                if error.is_resource_suspension() {
                    *suspended = Some(
                        crate::state::PendingExpansionResume::PdfColorStackInitText {
                            restore_at_page_start,
                            mode,
                        },
                    );
                }
                return Err(error);
            }
        };
        let initial = self.attempt_token_list_bytes(initial)?;
        let id = match self
            .state
            .allocate_pdf_color_stack(mode, restore_at_page_start, initial)
        {
            Ok(id) => id,
            Err(_) => {
                self.report_recoverable(
                    TOO_MANY_COLOR_STACKS_DIAGNOSTIC,
                    "Too many color stacks".to_owned(),
                    &[
                        "The number of color stacks is limited to 32768.",
                        "I'll use the default color stack 0 here.",
                    ],
                );
                0
            }
        };
        self.push_rendered_text(&id.to_string(), opener.origin());
        Ok(())
    }

    fn expand_pdf_uniform_deviate(
        &mut self,
        opener: &CurrentCommand<G>,
        resume: &mut crate::state::PendingExpansionResume,
        suspended: &mut Option<crate::state::PendingExpansionResume>,
    ) -> Result<(), CommandError> {
        match std::mem::replace(resume, crate::state::PendingExpansionResume::Dispatch) {
            crate::state::PendingExpansionResume::Dispatch
            | crate::state::PendingExpansionResume::PdfUniformDeviate => {}
            _ => return Err(CommandError::input_invariant()),
        }
        let scan = self.scan_integer_retained();
        let bound = self
            .retain_expansion_scalar(
                scan,
                crate::state::PendingExpansionResume::PdfUniformDeviate,
                suspended,
            )?
            .value;
        let value = self.state.pdf_uniform_deviate(bound);
        self.push_rendered_text(&value.to_string(), opener.origin());
        Ok(())
    }

    fn expand_pdf_ximage_bbox(
        &mut self,
        opener: &CurrentCommand<G>,
        resume: &mut crate::state::PendingExpansionResume,
        suspended: &mut Option<crate::state::PendingExpansionResume>,
    ) -> Result<(), CommandError> {
        let object = match std::mem::replace(resume, crate::state::PendingExpansionResume::Dispatch)
        {
            crate::state::PendingExpansionResume::Dispatch
            | crate::state::PendingExpansionResume::PdfXImageObject => {
                let scan = self.scan_integer_retained();
                let object = self
                    .retain_expansion_scalar(
                        scan,
                        crate::state::PendingExpansionResume::PdfXImageObject,
                        suspended,
                    )?
                    .value;
                u32::try_from(object).ok()
            }
            crate::state::PendingExpansionResume::PdfXImageCoordinate { object } => Some(object),
            _ => return Err(CommandError::input_invariant()),
        };
        let id = object.and_then(|raw| tex_state::PdfExternalImageId::new(raw).ok());
        let Some(id) = id.filter(|id| self.state.pdf_external_image(*id).is_some()) else {
            return Err(CommandError::PdfNavigation(
                "pdfTeX error (ext1): cannot find referenced object.",
            ));
        };
        let object = id.raw();
        let scan = self.scan_integer_retained();
        let index = self
            .retain_expansion_scalar(
                scan,
                crate::state::PendingExpansionResume::PdfXImageCoordinate { object },
                suspended,
            )?
            .value;
        let metadata = self
            .state
            .pdf_external_image(id)
            .expect("validated external image remains present");
        let Some(coordinate) = u8::try_from(index)
            .ok()
            .and_then(|index| metadata.bbox_coordinate(index))
        else {
            return Err(CommandError::PdfNavigation(
                "pdfTeX error (pdfximagebbox): invalid parameter.",
            ));
        };
        self.push_rendered_text(&format_scaled(coordinate), opener.origin());
        Ok(())
    }

    fn expand_pdf_xform_name(
        &mut self,
        opener: &CurrentCommand<G>,
        resume: &mut crate::state::PendingExpansionResume,
        suspended: &mut Option<crate::state::PendingExpansionResume>,
    ) -> Result<(), CommandError> {
        match std::mem::replace(resume, crate::state::PendingExpansionResume::Dispatch) {
            crate::state::PendingExpansionResume::Dispatch
            | crate::state::PendingExpansionResume::PdfXFormName => {}
            _ => return Err(CommandError::input_invariant()),
        }
        let scan = self.scan_integer_retained();
        let object = self
            .retain_expansion_scalar(
                scan,
                crate::state::PendingExpansionResume::PdfXFormName,
                suspended,
            )?
            .value;
        let resource = u32::try_from(object)
            .ok()
            .and_then(|object| self.state.pdf_form_resource(object))
            .unwrap_or(0);
        self.push_rendered_text(&resource.to_string(), opener.origin());
        Ok(())
    }

    fn expand_pdf_page_ref(
        &mut self,
        opener: &CurrentCommand<G>,
        resume: &mut crate::state::PendingExpansionResume,
        suspended: &mut Option<crate::state::PendingExpansionResume>,
    ) -> Result<(), CommandError> {
        match std::mem::replace(resume, crate::state::PendingExpansionResume::Dispatch) {
            crate::state::PendingExpansionResume::Dispatch
            | crate::state::PendingExpansionResume::PdfPageRef => {}
            _ => return Err(CommandError::input_invariant()),
        }
        let scan = self.scan_integer_retained();
        let page = self
            .retain_expansion_scalar(
                scan,
                crate::state::PendingExpansionResume::PdfPageRef,
                suspended,
            )?
            .value;
        if page <= 0 {
            return Err(CommandError::PdfNavigation(
                "pdfTeX error (pageref): invalid page number",
            ));
        }
        let object = u32::try_from(page)
            .ok()
            .and_then(|page| self.state.pdf_page_object(page))
            .unwrap_or(0);
        self.push_rendered_text(&object.to_string(), opener.origin());
        Ok(())
    }

    fn expand_pdf_last_match(
        &mut self,
        opener: CurrentCommand<G>,
        resume: &mut crate::state::PendingExpansionResume,
        suspended: &mut Option<crate::state::PendingExpansionResume>,
    ) -> Result<(), CommandError> {
        match std::mem::replace(resume, crate::state::PendingExpansionResume::Dispatch) {
            crate::state::PendingExpansionResume::Dispatch
            | crate::state::PendingExpansionResume::PdfLastMatch => {}
            _ => return Err(CommandError::input_invariant()),
        }
        let scan = self.scan_integer_retained();
        let mut index = self
            .retain_expansion_scalar(
                scan,
                crate::state::PendingExpansionResume::PdfLastMatch,
                suspended,
            )?
            .value;
        if index < 0 {
            self.pdftex_match_number_diagnostic(index);
            index = 1;
        }
        let capture = u32::try_from(index)
            .ok()
            .and_then(|index| self.state.pdf_match_capture(index))
            .map(|(offset, bytes)| (offset, bytes.to_vec()));
        let mut rendered = match capture {
            Some((offset, _)) => format!("{offset}->"),
            None => "-1->".to_owned(),
        };
        if let Some((_, bytes)) = capture {
            rendered.extend(bytes.into_iter().map(char::from));
        }
        self.push_rendered_text(&rendered, opener.origin());
        Ok(())
    }

    /// pdftex.web §1590's `pdf_file_dump_code` conversion.
    ///
    /// The filename is scanned before the immutable input capability is
    /// consulted. An absent capability retains the corrected range and typed
    /// request, so the host retry neither repeats diagnostics nor rescans the
    /// consumed operands.
    fn expand_pdf_file_dump(
        &mut self,
        opener: CurrentCommand<G>,
        resume: &mut crate::state::PendingExpansionResume,
        suspended: &mut Option<crate::state::PendingExpansionResume>,
    ) -> Result<(), CommandError> {
        let (pending, scanned_range, scanned_options) =
            match std::mem::replace(resume, crate::state::PendingExpansionResume::Dispatch) {
                crate::state::PendingExpansionResume::Dispatch => (None, None, Some((0, 0, 0))),
                crate::state::PendingExpansionResume::PdfFileDumpOptions {
                    offset,
                    length,
                    phase,
                } => (None, None, Some((offset, length, phase))),
                crate::state::PendingExpansionResume::PdfFileDumpText { offset, length } => {
                    (None, Some((offset, length)), None)
                }
                crate::state::PendingExpansionResume::PdfFileDump(pending) => {
                    (Some(pending), None, None)
                }
                _ => return Err(CommandError::input_invariant()),
            };
        let (request, offset, length) = if let Some(pending) = pending {
            (pending.request, pending.offset, pending.length)
        } else {
            let (offset, length) = if let Some(range) = scanned_range {
                range
            } else {
                let (mut offset, mut length, mut phase) =
                    scanned_options.expect("unscanned dump options retain their cursor");
                loop {
                    match phase {
                        0 => {
                            let scan = self.scan_keyword_retained("offset");
                            if self
                                .retain_expansion_scalar(
                                    scan,
                                    crate::state::PendingExpansionResume::PdfFileDumpOptions {
                                        offset,
                                        length,
                                        phase,
                                    },
                                    suspended,
                                )?
                                .value
                            {
                                phase = 1;
                            } else {
                                phase = 2;
                            }
                        }
                        1 => {
                            let scan = self.scan_integer_retained();
                            offset = self
                                .retain_expansion_scalar(
                                    scan,
                                    crate::state::PendingExpansionResume::PdfFileDumpOptions {
                                        offset,
                                        length,
                                        phase,
                                    },
                                    suspended,
                                )?
                                .value;
                            if offset < 0 {
                                self.pdftex_file_range_diagnostic("offset", offset);
                                offset = 0;
                            }
                            phase = 2;
                        }
                        2 => {
                            let scan = self.scan_keyword_retained("length");
                            if self
                                .retain_expansion_scalar(
                                    scan,
                                    crate::state::PendingExpansionResume::PdfFileDumpOptions {
                                        offset,
                                        length,
                                        phase,
                                    },
                                    suspended,
                                )?
                                .value
                            {
                                phase = 3;
                            } else {
                                break;
                            }
                        }
                        3 => {
                            let scan = self.scan_integer_retained();
                            length = self
                                .retain_expansion_scalar(
                                    scan,
                                    crate::state::PendingExpansionResume::PdfFileDumpOptions {
                                        offset,
                                        length,
                                        phase,
                                    },
                                    suspended,
                                )?
                                .value;
                            if length < 0 {
                                self.pdftex_file_range_diagnostic("length", length);
                                length = 0;
                            }
                            break;
                        }
                        _ => return Err(CommandError::input_invariant()),
                    }
                }
                (offset, length)
            };
            let name = match self.scan_balanced_text(true) {
                Ok(name) => name.tokens,
                Err(error) => {
                    if error.is_resource_suspension() {
                        *suspended = Some(crate::state::PendingExpansionResume::PdfFileDumpText {
                            offset,
                            length,
                        });
                    }
                    return Err(error);
                }
            };
            let name = self
                .attempt_token_list_bytes(name)?
                .into_iter()
                .map(char::from)
                .collect::<String>();
            (
                crate::FileEnquiryRequest::new(name, crate::FileEnquiryIntent::Dump),
                offset,
                length,
            )
        };
        self.state.unsupported_host_capability();
        let Some(source) = self.host.input_probe(&request.name) else {
            return if self.host.input_probe_is_unavailable(&request.name) {
                Ok(())
            } else {
                *suspended = Some(crate::state::PendingExpansionResume::PdfFileDump(
                    crate::state::PendingFileEnquiry {
                        request: request.clone(),
                        offset,
                        length,
                    },
                ));
                Err(CommandError::MissingInputProbe(request))
            };
        };
        let start = usize::try_from(offset).expect("recovered file offset is nonnegative");
        let bytes = source.source().bytes();
        if start >= bytes.len() || length == 0 {
            return Ok(());
        }
        let end = start
            .saturating_add(usize::try_from(length).expect("recovered dump length is nonnegative"))
            .min(bytes.len());
        let mut rendered = String::with_capacity((end - start) * 2);
        for byte in &bytes[start..end] {
            use std::fmt::Write as _;
            write!(rendered, "{byte:02X}").expect("writing to a String cannot fail");
        }
        self.push_rendered_text(&rendered, opener.origin());
        Ok(())
    }

    /// pdftex.web §1590's `pdf_file_size_code` conversion.
    fn expand_pdf_file_size(
        &mut self,
        opener: CurrentCommand<G>,
        resume: &mut crate::state::PendingExpansionResume,
        suspended: &mut Option<crate::state::PendingExpansionResume>,
    ) -> Result<(), CommandError> {
        let pending =
            match std::mem::replace(resume, crate::state::PendingExpansionResume::Dispatch) {
                crate::state::PendingExpansionResume::Dispatch => None,
                crate::state::PendingExpansionResume::PdfFileSize(pending) => Some(pending),
                _ => return Err(CommandError::input_invariant()),
            };
        let request = if let Some(pending) = pending {
            pending.request
        } else {
            let name = self.scan_balanced_text(true)?.tokens;
            let name = self
                .attempt_token_list_bytes(name)?
                .into_iter()
                .map(char::from)
                .collect::<String>();
            crate::FileEnquiryRequest::new(name, crate::FileEnquiryIntent::Size)
        };
        self.state.unsupported_host_capability();
        let Some(source) = self.host.input_probe(&request.name) else {
            return if self.host.input_probe_is_unavailable(&request.name) {
                Ok(())
            } else {
                *suspended = Some(crate::state::PendingExpansionResume::PdfFileSize(
                    crate::state::PendingFileEnquiry {
                        request: request.clone(),
                        offset: 0,
                        length: 0,
                    },
                ));
                Err(CommandError::MissingInputProbe(request))
            };
        };
        self.push_rendered_text(&source.source().bytes().len().to_string(), opener.origin());
        Ok(())
    }

    /// pdftex.web §1590's `pdf_file_mod_date_code` conversion.
    fn expand_pdf_file_modification_date(
        &mut self,
        opener: CurrentCommand<G>,
        resume: &mut crate::state::PendingExpansionResume,
        suspended: &mut Option<crate::state::PendingExpansionResume>,
    ) -> Result<(), CommandError> {
        let pending =
            match std::mem::replace(resume, crate::state::PendingExpansionResume::Dispatch) {
                crate::state::PendingExpansionResume::Dispatch => None,
                crate::state::PendingExpansionResume::PdfFileModificationDate(pending) => {
                    Some(pending)
                }
                _ => return Err(CommandError::input_invariant()),
            };
        let request = if let Some(pending) = pending {
            pending.request
        } else {
            crate::FileEnquiryRequest::new(
                self.scan_pdf_file_name()?,
                crate::FileEnquiryIntent::ModificationDate,
            )
        };
        self.state.unsupported_host_capability();
        let Some(resource) = self.host.input_probe(&request.name) else {
            return if self.host.input_probe_is_unavailable(&request.name) {
                Ok(())
            } else {
                *suspended = Some(
                    crate::state::PendingExpansionResume::PdfFileModificationDate(
                        crate::state::PendingFileEnquiry {
                            request: request.clone(),
                            offset: 0,
                            length: 0,
                        },
                    ),
                );
                Err(CommandError::MissingInputProbe(request))
            };
        };
        if let Some(date) = resource.modification_date() {
            self.push_rendered_text(
                &format_pdf_date(date.clock, date.utc_offset_minutes),
                opener.origin(),
            );
        }
        Ok(())
    }

    /// pdftex.web §1590's string/file MD5 conversion.
    fn expand_pdf_md_five_sum(
        &mut self,
        opener: CurrentCommand<G>,
        resume: &mut crate::state::PendingExpansionResume,
        suspended: &mut Option<crate::state::PendingExpansionResume>,
    ) -> Result<(), CommandError> {
        use md5::{Digest, Md5};
        let (pending, scanned_file, scan_file_keyword) =
            match std::mem::replace(resume, crate::state::PendingExpansionResume::Dispatch) {
                crate::state::PendingExpansionResume::Dispatch => (None, None, true),
                crate::state::PendingExpansionResume::PdfMdFiveSumFile => (None, None, true),
                crate::state::PendingExpansionResume::PdfMdFiveSumText { file } => {
                    (None, Some(file), false)
                }
                crate::state::PendingExpansionResume::PdfMdFiveSum(pending) => {
                    (Some(pending), None, false)
                }
                _ => return Err(CommandError::input_invariant()),
            };
        let file = if pending.is_some() {
            true
        } else if let Some(file) = scanned_file {
            file
        } else if scan_file_keyword {
            let scan = self.scan_keyword_retained("file");
            self.retain_expansion_scalar(
                scan,
                crate::state::PendingExpansionResume::PdfMdFiveSumFile,
                suspended,
            )?
            .value
        } else {
            return Err(CommandError::input_invariant());
        };
        let mut bytes = if let Some(pending) = &pending {
            pending.request.name.as_bytes().to_vec()
        } else {
            let tokens = match self.scan_balanced_text(true) {
                Ok(tokens) => tokens.tokens,
                Err(error) => {
                    if error.is_resource_suspension() {
                        *suspended =
                            Some(crate::state::PendingExpansionResume::PdfMdFiveSumText { file });
                    }
                    return Err(error);
                }
            };
            self.attempt_token_list_bytes(tokens)?
        };
        if file {
            let request = pending.map_or_else(
                || {
                    let name = bytes.iter().copied().map(char::from).collect::<String>();
                    crate::FileEnquiryRequest::new(name, crate::FileEnquiryIntent::MdFiveSum)
                },
                |pending| pending.request,
            );
            self.state.unsupported_host_capability();
            let Some(resource) = self.host.input_probe(&request.name) else {
                return if self.host.input_probe_is_unavailable(&request.name) {
                    Ok(())
                } else {
                    *suspended = Some(crate::state::PendingExpansionResume::PdfMdFiveSum(
                        crate::state::PendingFileEnquiry {
                            request: request.clone(),
                            offset: 0,
                            length: 0,
                        },
                    ));
                    Err(CommandError::MissingInputProbe(request))
                };
            };
            bytes = resource.source().bytes().to_vec();
        }
        let digest = Md5::digest(bytes);
        let rendered = digest
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect::<String>();
        self.push_rendered_text(&rendered, opener.origin());
        Ok(())
    }

    fn scan_pdf_file_name(&mut self) -> Result<String, CommandError> {
        let tokens = self.scan_balanced_text(true)?.tokens;
        Ok(self
            .attempt_token_list_bytes(tokens)?
            .into_iter()
            .map(char::from)
            .collect())
    }

    /// pdftex.web §1590's `pdf_insert_ht_code` conversion reads the height
    /// accumulated in the live page-builder insertion record. Missing classes
    /// use pdfTeX's literal `0pt`; present zero heights use `print_scaled` and
    /// therefore remain distinguishable as `0.0pt`.
    fn expand_pdf_insert_height(
        &mut self,
        opener: CurrentCommand<G>,
        resume: &mut crate::state::PendingExpansionResume,
        suspended: &mut Option<crate::state::PendingExpansionResume>,
    ) -> Result<(), CommandError> {
        if !matches!(
            std::mem::replace(resume, crate::state::PendingExpansionResume::Dispatch),
            crate::state::PendingExpansionResume::Dispatch
                | crate::state::PendingExpansionResume::PdfInsertHeight
        ) {
            return Err(CommandError::input_invariant());
        }
        let scan = self.scan_extended_register_index_retained();
        let class = self.retain_expansion_scalar(
            scan,
            crate::state::PendingExpansionResume::PdfInsertHeight,
            suspended,
        )?;
        let rendered = self
            .host
            .page_insertion_height(class)
            .map_or_else(|| "0pt".to_owned(), format_scaled);
        self.push_rendered_text(&rendered, opener.origin());
        Ok(())
    }

    fn pdftex_file_range_diagnostic(&mut self, kind: &str, value: i32) {
        let label = if kind == "offset" {
            "file offset"
        } else {
            "dump length"
        };
        self.command.semantic_diagnostics.push(
            crate::CommandSemanticDiagnostic::PdfExpansionMessage {
                text: format!("! Bad {label} ({value})."),
            },
        );
    }

    fn pdftex_regex_warning(&mut self, message: &str) {
        self.command.semantic_diagnostics.push(
            crate::CommandSemanticDiagnostic::PdfExpansionMessage {
                text: format!("pdfTeX warning: pdftex: \\pdfmatch: {message}"),
            },
        );
    }

    fn pdftex_match_number_diagnostic(&mut self, value: i32) {
        self.command.semantic_diagnostics.push(
            crate::CommandSemanticDiagnostic::PdfExpansionMessage {
                text: format!("! Bad match number ({value})."),
            },
        );
    }

    fn push_mark_text(&mut self, tokens: &tex_state::node::NodeTokenList) {
        let words = tokens.words();
        let level = self.command.push_token_level(
            PackedTokenSpanHandle::stored_semantic(words),
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
                source: None,
                level: level.0,
                position: 0,
            }),
        );
    }

    fn attempt_token_list_string_text(
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

    fn attempt_token_list_bytes(
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
    fn push_rendered_text(&mut self, text: &str, parent: OriginId) {
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

    fn push_rendered_tokens(&mut self, tokens: impl IntoIterator<Item = Token>, parent: OriginId) {
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

    fn replay_expandafter_first(&mut self, command: CurrentCommand<G>) -> Result<(), CommandError> {
        self.conserve_input_stack()?;
        self.undo_alignment_delivery(&command);
        let level = self.command.push_token_level(
            PackedTokenSpanHandle::backed_up([BackedUpToken {
                spelling: command.spelling(),
                source_provenance: command.source_provenance(),
            }]),
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
                source: None,
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
        definition: tex_state::DefinitionId<G>,
        call_site: OriginId,
        arguments: MacroArguments<G>,
    ) -> InputLevelId {
        let definition_view = self.state.definition(definition.clone());
        let parent = self.command.parameters.parent_invocation();
        let replacement_len = definition_view.replacement_text().len();
        let invocation = call_site;
        let _ = parent;
        self.command
            .push_macro_activation(name, definition, arguments, invocation, replacement_len)
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

pub(crate) fn render_the_value(value: &crate::InternalValue) -> Option<String> {
    match value {
        crate::InternalValue::Integer(value) => Some(value.to_string()),
        crate::InternalValue::Dimension(value) => Some(format_scaled(*value)),
        crate::InternalValue::Glue(value) => Some(format_glue(*value, "pt")),
        crate::InternalValue::MuGlue(value) => Some(format_glue(*value, "mu")),
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
pub(crate) fn is_main_loop_character<G>(meaning: ResolvedMeaning<G>) -> bool {
    matches!(
        meaning,
        ResolvedMeaning::Static(
            Meaning::CharToken {
                cat: Catcode::Letter | Catcode::Other,
                ..
            } | Meaning::CharGiven(_)
        )
    )
}

fn is_expandable<G>(meaning: ResolvedMeaning<G>) -> bool {
    matches!(meaning, ResolvedMeaning::Macro { .. })
        || matches!(
            meaning,
            ResolvedMeaning::Static(Meaning::ExpandablePrimitive(primitive))
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
pub(crate) fn is_expandable_command<G>(command: &CurrentCommand<G>) -> bool {
    is_expandable(command.meaning())
        || (matches!(static_meaning(command.meaning()), Some(Meaning::Undefined))
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

pub(crate) fn string_text<G>(state: &tex_state::CommandContext<'_, G>, token: Token) -> String {
    let mut text = String::new();
    append_string_text(state, token, &mut text);
    text
}

pub(crate) fn append_string_text<G>(
    state: &tex_state::CommandContext<'_, G>,
    token: Token,
    text: &mut String,
) {
    match token {
        Token::Cs(symbol) => {
            let escape = state.untracked_int_param(IntParam::ESCAPE_CHAR);
            if let Some(ch) = char::from_u32(u32::try_from(escape).unwrap_or(u32::MAX)) {
                text.push(ch);
            }
            text.push_str(state.resolve(symbol));
        }
        Token::Char { ch, .. } => text.push(ch),
        Token::Param(slot) => write!(text, "#{slot}").expect("writing to String cannot fail"),
        Token::Frozen(_) => text.push_str("\\relax"),
    }
}

/// TeX82 §262's `print_cs`, including its delimiter after a control word.
///
/// This is distinct from §263's `sprint_cs` spelling used by `\show` before
/// `=` and from §213's `\string`: named control words and `null_cs` append a
/// space, while active characters and single nonletter control symbols do not.
pub(crate) fn print_cs_text<G>(
    state: &mut tex_state::CommandContext<'_, G>,
    symbol: tex_state::interner::Symbol,
) -> String {
    let mut text = String::new();
    append_print_cs_text(state, symbol, &mut text);
    text
}

pub(crate) fn append_print_cs_text<G>(
    state: &mut tex_state::CommandContext<'_, G>,
    symbol: tex_state::interner::Symbol,
    text: &mut String,
) {
    let name = state.resolve(symbol);
    match state.control_sequence_kind(symbol) {
        ControlSequenceKind::ActiveCharacter => {
            text.push_str(name);
            return;
        }
        ControlSequenceKind::Null => {
            append_print_esc_text(state, "csname", text);
            append_print_esc_text(state, "endcsname", text);
            text.push(' ');
            return;
        }
        ControlSequenceKind::SingleCharacter
        | ControlSequenceKind::Named
        | ControlSequenceKind::Internal => {}
    }

    append_string_text(state, Token::Cs(symbol), text);
    let mut characters = name.chars();
    match (characters.next(), characters.next()) {
        (Some(character), None) if state.catcode(character) != Catcode::Letter => {}
        _ => text.push(' '),
    }
}

pub(crate) fn meaning_text<G>(
    state: &mut tex_state::CommandContext<'_, G>,
    command: &CurrentCommand<G>,
) -> String {
    let mut text = String::new();
    append_meaning_text_with_token_selector(state, command, false, &mut text);
    text
}

/// TeX82 §§59, 262, and 296's `print_meaning` through an active selector.
///
/// `\meaning` builds a string, but `\show` prints a macro or mark token list
/// directly. Character tokens in the latter path therefore observe the live
/// `\newlinechar` instead of always using their context-free `^^` spelling.
pub(crate) fn selector_meaning_text<G>(
    state: &mut tex_state::CommandContext<'_, G>,
    command: &CurrentCommand<G>,
) -> String {
    let mut text = String::new();
    append_meaning_text_with_token_selector(state, command, true, &mut text);
    text
}

fn append_meaning_text_with_token_selector<G>(
    state: &mut tex_state::CommandContext<'_, G>,
    command: &CurrentCommand<G>,
    active_selector: bool,
    text: &mut String,
) {
    if let ResolvedMeaning::Macro { flags, definition } = command.meaning() {
        let macro_meaning = state.definition(definition);
        if flags.contains(MeaningFlags::PROTECTED) {
            append_print_esc_text(state, "protected", text);
        }
        if flags.contains(MeaningFlags::LONG) {
            append_print_esc_text(state, "long", text);
        }
        if flags.contains(MeaningFlags::OUTER) {
            append_print_esc_text(state, "outer", text);
        }
        if flags.bits()
            & (MeaningFlags::PROTECTED | MeaningFlags::LONG | MeaningFlags::OUTER).bits()
            != 0
        {
            text.push(' ');
        }
        text.push_str("macro:");
        append_meaning_token_words(state, macro_meaning.parameter_text(), active_selector, text);
        text.push_str("->");
        append_meaning_token_words(
            state,
            macro_meaning.replacement_text(),
            active_selector,
            text,
        );
        return;
    }
    let ResolvedMeaning::Static(meaning) = command.meaning() else {
        unreachable!("macro meanings returned above")
    };
    match meaning {
        Meaning::Undefined => text.push_str("undefined"),
        Meaning::Relax => append_print_esc_text(state, "relax", text),
        Meaning::CharToken { ch, cat } => append_character_command_text(ch, cat, text),
        Meaning::CharGiven(ch) => {
            text.push_str("the character ");
            append_printable_character_text(ch, text);
        }
        Meaning::MathCharGiven(value) => {
            write!(text, "\\mathchar\"{value:X}").expect("writing to String cannot fail");
        }
        Meaning::CountRegister(index) => {
            write!(text, "\\count{index}").expect("writing to String cannot fail");
        }
        Meaning::DimenRegister(index) => {
            write!(text, "\\dimen{index}").expect("writing to String cannot fail");
        }
        Meaning::SkipRegister(index) => {
            write!(text, "\\skip{index}").expect("writing to String cannot fail");
        }
        Meaning::MuskipRegister(index) => {
            write!(text, "\\muskip{index}").expect("writing to String cannot fail");
        }
        Meaning::ToksRegister(index) => {
            write!(text, "\\toks{index}").expect("writing to String cannot fail");
        }
        meaning @ (Meaning::IntParam(_)
        | Meaning::InternalInteger(_)
        | Meaning::DimenParam(_)
        | Meaning::GlueParam(_)
        | Meaning::MuGlueParam(_)
        | Meaning::TokParam(_)
        | Meaning::PageDimension(_)
        | Meaning::PageInteger(_)) => {
            append_meaning_control_sequence_text(state, command, meaning, text);
        }
        Meaning::Font(font) => {
            text.push_str("select font ");
            text.push_str(state.font_external_name(font));
            let size = state.font_size(font);
            if size != state.font_design_size(font) {
                text.push_str(" at ");
                append_scaled_without_unit(size, text);
            }
        }
        Meaning::ExpandablePrimitive(ExpandablePrimitive::EndTemplate) => {
            append_print_esc_text(state, "outer", text);
            text.push_str(" endtemplate:");
        }
        Meaning::ExpandablePrimitive(
            primitive @ (ExpandablePrimitive::TopMark
            | ExpandablePrimitive::FirstMark
            | ExpandablePrimitive::BotMark
            | ExpandablePrimitive::SplitFirstMark
            | ExpandablePrimitive::SplitBotMark),
        ) => {
            append_meaning_control_sequence_text(
                state,
                command,
                Meaning::ExpandablePrimitive(primitive),
                text,
            );
            text.push(':');
            let tokens = state.page_mark(page_mark(primitive));
            append_meaning_token_words(state, tokens.words(), active_selector, text);
        }
        meaning @ (Meaning::ExpandablePrimitive(_) | Meaning::UnexpandablePrimitive(_)) => {
            append_meaning_control_sequence_text(state, command, meaning, text);
        }
        Meaning::EndV => text.push_str("end of alignment template"),
        Meaning::Unknown(_) => text.push_str("unknown"),
    }
}

pub(crate) fn append_meaning_token_words<G>(
    state: &tex_state::CommandContext<'_, G>,
    tokens: &[tex_state::token::TokenWord],
    active_selector: bool,
    text: &mut String,
) {
    let mut index = 0;
    while index < tokens.len() {
        let token = tokens[index].token().expect("durable token word is valid");
        if let Token::Char {
            ch,
            cat: Catcode::Parameter,
        } = token
            && let Some(Token::Param(slot)) = tokens.get(index + 1).and_then(|word| word.token())
        {
            let raw = [ch, char::from(b'0' + slot)]
                .into_iter()
                .collect::<String>();
            if active_selector {
                state.append_selector_string_text(&raw, text);
            } else {
                text.push_str(&raw);
            }
            index += 2;
            continue;
        }
        if active_selector {
            state.append_token_selector_text(token, text);
        } else {
            state.append_token_show_text(token, text);
        }
        index += 1;
    }
}

/// The copyable portion of a delivered command needed by TeX82 §298.
///
/// This is captured from `CurrentCommand<G>`, not reconstructed from `Meaning`,
/// so the delivered control-sequence identity remains available across the
/// executor's transactional scan/apply seam.
#[derive(Debug, Eq, PartialEq)]
pub struct PrintCommand<G> {
    meaning: ResolvedMeaning<G>,
    control_sequence: Option<tex_state::interner::Symbol>,
}

impl<G> PrintCommand<G> {
    #[must_use]
    pub fn from_current(command: &CurrentCommand<G>) -> Self {
        Self {
            meaning: command.meaning(),
            control_sequence: command.control_sequence(),
        }
    }

    #[must_use]
    pub(crate) fn meaning(&self) -> ResolvedMeaning<G> {
        self.meaning.clone()
    }
}

impl<G> Clone for PrintCommand<G> {
    fn clone(&self) -> Self {
        Self {
            meaning: self.meaning.clone(),
            control_sequence: self.control_sequence,
        }
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
pub fn print_cmd_chr_text<G>(
    state: &tex_state::CommandContext<'_, G>,
    command: PrintCommand<G>,
) -> String {
    let mut text = String::new();
    append_print_cmd_chr_text(state, command, &mut text);
    text
}

/// Appends TeX82 §298's `print_cmd_chr` representation to caller-owned text.
pub fn append_print_cmd_chr_text<G>(
    state: &tex_state::CommandContext<'_, G>,
    command: PrintCommand<G>,
    text: &mut String,
) {
    if let ResolvedMeaning::Macro { flags, .. } = command.meaning {
        if flags.contains(MeaningFlags::PROTECTED) {
            append_print_esc_text(state, "protected", text);
        }
        if flags.contains(MeaningFlags::LONG) {
            append_print_esc_text(state, "long", text);
        }
        if flags.contains(MeaningFlags::OUTER) {
            append_print_esc_text(state, "outer", text);
        }
        if flags.bits()
            & (MeaningFlags::PROTECTED | MeaningFlags::LONG | MeaningFlags::OUTER).bits()
            != 0
        {
            text.push(' ');
        }
        text.push_str("macro");
        return;
    }
    let ResolvedMeaning::Static(meaning) = command.meaning else {
        unreachable!("macro meanings returned above")
    };
    match meaning {
        Meaning::Undefined => text.push_str("undefined"),
        Meaning::Relax => append_print_esc_text(state, "relax", text),
        Meaning::ExpandablePrimitive(ExpandablePrimitive::EndTemplate) => {
            append_print_esc_text(state, "outer", text);
            text.push_str(" endtemplate");
        }
        Meaning::CharToken { ch, cat } => append_character_command_text(ch, cat, text),
        Meaning::CharGiven(ch) => {
            append_print_esc_text(state, "char", text);
            write!(text, "\"{:X}", ch as u32).expect("writing to String cannot fail");
        }
        Meaning::MathCharGiven(value) => {
            append_print_esc_text(state, "mathchar", text);
            write!(text, "\"{value:X}").expect("writing to String cannot fail");
        }
        Meaning::CountRegister(index) => append_escaped_index(state, "count", index, text),
        Meaning::DimenRegister(index) => append_escaped_index(state, "dimen", index, text),
        Meaning::SkipRegister(index) => append_escaped_index(state, "skip", index, text),
        Meaning::MuskipRegister(index) => append_escaped_index(state, "muskip", index, text),
        Meaning::ToksRegister(index) => append_escaped_index(state, "toks", index, text),
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
            append_print_command_control_sequence_text(state, command, meaning, text);
        }
        Meaning::Font(font) => {
            text.push_str("select font ");
            text.push_str(state.font_external_name(font));
            let size = state.font_size(font);
            if size != state.font_design_size(font) {
                text.push_str(" at ");
                append_scaled_without_unit(size, text);
                text.push_str("pt");
            }
        }
        Meaning::EndV => text.push_str("end of alignment template"),
        Meaning::Unknown(_) => text.push_str("[unknown command code!]"),
    }
}

fn append_escaped_index<G>(
    state: &tex_state::CommandContext<'_, G>,
    name: &str,
    index: u16,
    text: &mut String,
) {
    append_print_esc_text(state, name, text);
    write!(text, "{index}").expect("writing to String cannot fail");
}

fn append_print_command_control_sequence_text<G>(
    state: &tex_state::CommandContext<'_, G>,
    command: PrintCommand<G>,
    meaning: Meaning,
    text: &mut String,
) {
    let name = state
        .primitive_name(meaning)
        .or_else(|| command.control_sequence.map(|symbol| state.resolve(symbol)));
    if let Some(name) = name {
        append_print_esc_text(state, name, text);
    } else {
        text.push_str("undefined");
    }
}

fn append_meaning_control_sequence_text<G>(
    state: &tex_state::CommandContext<'_, G>,
    command: &CurrentCommand<G>,
    meaning: Meaning,
    text: &mut String,
) {
    let name = state.primitive_name(meaning).or_else(|| {
        command
            .control_sequence()
            .map(|symbol| state.resolve(symbol))
    });
    if let Some(name) = name {
        text.push('\\');
        text.push_str(name);
    } else {
        text.push_str("undefined");
    }
}

/// TeX82 §298's character-command cases used by `print_meaning`.
pub fn character_command_text(ch: char, cat: Catcode) -> String {
    let mut text = String::new();
    append_character_command_text(ch, cat, &mut text);
    text
}

/// Appends TeX82 §298's character-command representation.
pub fn append_character_command_text(ch: char, cat: Catcode, text: &mut String) {
    match cat {
        Catcode::BeginGroup => text.push_str("begin-group character "),
        Catcode::EndGroup => text.push_str("end-group character "),
        Catcode::MathShift => text.push_str("math shift character "),
        Catcode::AlignmentTab => text.push_str("alignment tab character "),
        Catcode::Parameter => text.push_str("macro parameter character "),
        Catcode::Superscript => text.push_str("superscript character "),
        Catcode::Subscript => text.push_str("subscript character "),
        Catcode::Space => {
            text.push_str("blank space  ");
            return;
        }
        Catcode::Letter => text.push_str("the letter "),
        Catcode::Other => text.push_str("the character "),
        // `get_next` maps a category-5 character to `car_ret` with its
        // character code as operand. It is therefore §298's non-`cr_code`
        // branch, whose vocabulary is `\crcr`.
        Catcode::EndLine => {
            text.push_str("\\crcr");
            return;
        }
        Catcode::Escape
        | Catcode::Ignored
        | Catcode::Active
        | Catcode::Comment
        | Catcode::Invalid => {
            text.push_str("[uncommandable character ");
            append_printable_character_text(ch, text);
            text.push(']');
            return;
        }
    }
    append_printable_character_text(ch, text);
}

/// TeX82 §§49/59's one-character string spelling used by §298.
///
/// Rendering happens before the completed diagnostic reaches its live output
/// selector, so generated caret notation must not be reinterpreted through
/// `\newlinechar` character by character.
fn append_printable_character_text(ch: char, text: &mut String) {
    tex_state::token_show::append_tex_print_char(ch, text);
}

/// TeX82 §63's `print_esc`: the current `\escapechar`, when it names a
/// character, followed by `name`.
///
/// §63 prints no escape at all when `\escapechar` is outside a character's
/// range, which is why the prefix is conditional rather than a hard-coded
/// backslash.
#[must_use]
pub fn print_esc_text<G>(state: &tex_state::CommandContext<'_, G>, name: &str) -> String {
    let mut text = String::with_capacity(name.len() + 1);
    append_print_esc_text(state, name, &mut text);
    text
}

/// Appends TeX82 §63's `print_esc` representation.
pub fn append_print_esc_text<G>(
    state: &tex_state::CommandContext<'_, G>,
    name: &str,
    text: &mut String,
) {
    if let Ok(escape) = u8::try_from(state.untracked_int_param(IntParam::ESCAPE_CHAR)) {
        text.push(char::from(escape));
    }
    text.push_str(name);
}

/// TeX82 §298's `print_cmd_chr` representation for a delivered token.
///
/// Diagnostics use this same renderer as `\meaning`; consequently Rust enum
/// spellings cannot leak into ordinary terminal or transcript output.
#[must_use]
pub fn command_token_text<G>(state: &mut tex_state::CommandContext<'_, G>, token: Token) -> String {
    let mut text = String::new();
    append_command_token_text(state, token, &mut text);
    text
}

/// Appends TeX82 §298's representation for a delivered token.
pub fn append_command_token_text<G>(
    state: &mut tex_state::CommandContext<'_, G>,
    token: Token,
    text: &mut String,
) {
    match token {
        Token::Char { ch, cat } => append_character_command_text(ch, cat, text),
        Token::Param(slot) => {
            write!(text, "macro parameter character #{slot}")
                .expect("writing to String cannot fail");
        }
        Token::Frozen(_) => text.push_str("end of alignment template"),
        Token::Cs(symbol) => {
            let meaning = state.meaning(symbol);
            let name = match meaning {
                ResolvedMeaning::Static(meaning) => state.primitive_name(meaning),
                ResolvedMeaning::Macro { .. } => None,
            }
            .unwrap_or_else(|| state.resolve(symbol));
            append_print_esc_text(state, name, text);
        }
    }
}

/// The string pdfTeX builds by selecting `new_string` around `show_token_list`.
///
/// Character tokens remain raw (with parameter characters doubled), while
/// control-sequence spelling and its separator observe the live escape
/// character and catcode table. The returned value owns no token-list handle,
/// so it remains stable when a typed resource continuation resumes the
/// enclosing command.
pub(crate) fn token_slice_string_text<G>(
    state: &mut tex_state::CommandContext<'_, G>,
    tokens: &[Token],
) -> String {
    let mut text = String::new();
    let _ = state.int_param(IntParam::ESCAPE_CHAR);
    for &token in tokens {
        state.append_token_string_text(token, &mut text);
    }
    text
}

/// TeX82's `show_token_list` representation used by `\\meaning` distinguishes
/// a printed control word from following letter tokens with one space.  That
/// delimiter belongs to the rendered definition, not to source input.
pub(crate) fn token_list_token_text<G>(
    state: &tex_state::CommandContext<'_, G>,
    token: Token,
) -> String {
    let mut text = String::new();
    append_token_list_token_text(state, token, &mut text);
    text
}

pub(crate) fn append_token_list_token_text<G>(
    state: &tex_state::CommandContext<'_, G>,
    token: Token,
    text: &mut String,
) {
    let name = match token {
        Token::Cs(_) | Token::Char { .. } | Token::Param(_) => {
            state.append_token_show_text(token, text);
            return;
        }
        // tex.web gives every frozen equivalent a real eqtb `text()`, so §294
        // displays one exactly as it displays the ordinary control sequence of
        // the same name: `frozen_par` is `\par`, not its `\relax`-like
        // meaning.
        Token::Frozen(_) => match state.frozen_primitive_name(token) {
            Some(name) => name,
            None => {
                append_string_text(state, token, text);
                return;
            }
        },
    };
    // TeX82 §§63/294: `show_token_list` renders control sequences through
    // `print_cs`, and every escape prefix that `print_cs` emits comes from
    // the live `\escapechar`. This matters for backed-up recovery tokens:
    // §1064 inserts a closer ahead of the offending command, then §314
    // pseudoprints that command while the current integer parameters remain
    // in force.
    if name.is_empty() {
        append_print_esc_text(state, "csname", text);
        append_print_esc_text(state, "endcsname", text);
    } else {
        append_print_esc_text(state, name, text);
    }
    let mut chars = name.chars();
    match (chars.next(), chars.next()) {
        (Some(character), None) if state.untracked_catcode(character) != Catcode::Letter => {}
        _ => text.push(' '),
    }
}

fn pdftex_c_string(mut bytes: Vec<u8>) -> Vec<u8> {
    if let Some(nul) = bytes.iter().position(|byte| *byte == 0) {
        bytes.truncate(nul);
    }
    bytes
}

fn posix_regex_diagnostic(error: &PosixRegexError, pattern: &[u8]) -> &'static str {
    if matches!(error, PosixRegexError::EOF) && has_unclosed_bracket(pattern) {
        return "brackets ([ ]) not balanced";
    }
    match error {
        PosixRegexError::Expected(b']', _) => "brackets ([ ]) not balanced",
        PosixRegexError::Expected(b')', _) => "parentheses not balanced",
        PosixRegexError::IllegalRange => "invalid character range",
        PosixRegexError::IntegerOverflow | PosixRegexError::EmptyRepetition => {
            "invalid repetition count(s)"
        }
        PosixRegexError::InvalidBackRef(_) => "invalid back reference",
        PosixRegexError::LeadingRepetition => "repetition-operator operand invalid",
        PosixRegexError::UnclosedRepetition => "braces not balanced",
        PosixRegexError::UnknownClass(_) => "invalid character class",
        PosixRegexError::UnknownCollation => "invalid collating element",
        PosixRegexError::EOF | PosixRegexError::Expected(_, _) => {
            "premature end of regular expression"
        }
        PosixRegexError::UnexpectedToken(_) => "invalid regular expression",
    }
}

fn has_unclosed_bracket(pattern: &[u8]) -> bool {
    let mut escaped = false;
    let mut bracket = false;
    for &byte in pattern {
        if escaped {
            escaped = false;
        } else if byte == b'\\' {
            escaped = true;
        } else if byte == b'[' {
            bracket = true;
        } else if byte == b']' {
            bracket = false;
        }
    }
    bracket
}

fn roman_numeral(value: i32) -> String {
    let mut output = String::new();
    append_roman_numeral(value, &mut output);
    output
}

fn append_roman_numeral(value: i32, output: &mut String) {
    if value <= 0 {
        return;
    }
    let mut remaining = value;
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
}

fn format_pdf_date(clock: tex_state::JobClock, utc_offset_minutes: i16) -> String {
    use std::fmt::Write as _;
    let mut date = format!(
        "D:{:04}{:02}{:02}{:02}{:02}{:02}",
        clock.year,
        clock.month,
        clock.day,
        clock.time.div_euclid(60),
        clock.time.rem_euclid(60),
        clock.second,
    );
    if utc_offset_minutes == 0 {
        date.push('Z');
    } else {
        let sign = if utc_offset_minutes < 0 { '-' } else { '+' };
        let absolute = i32::from(utc_offset_minutes).abs();
        write!(date, "{sign}{:02}'{:02}'", absolute / 60, absolute % 60)
            .expect("writing to a String cannot fail");
    }
    date
}

#[cfg(test)]
mod tests;

/// Future-relevant expansion facts.
///
/// Resource fuel is deliberately absent: [`crate::CommandFuel`] is a
/// monotonic owner lent to processor episodes and is not restored with
/// semantic state. Discardable scratch allocation and profiling likewise
/// remain outside this state.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) struct ExpansionState {
    pub(crate) cumulative_expansions: u64,
    pub(crate) next_resource_resolution: u64,
    pub(crate) pending_diagnostics: Vec<u64>,
    pub(crate) observed_dependencies: Vec<u64>,
    pub(crate) semantic_barriers: Vec<u64>,
    pub(crate) profile: CommandProfile,
}
