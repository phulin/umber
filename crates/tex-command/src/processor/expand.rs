//! Fused resident-input advancement and raw/expanded command delivery.

use tex_state::meaning::{ExpandablePrimitive, Meaning, MeaningFlags, ResolvedMeaning};
use tex_state::token::{Catcode, OriginId, Token, TokenWord, TracedTokenWord};

use crate::command::DeliveryStamp;
use crate::input::{
    InputLevel, InputLevelId, ResidentCommandInterception, ResidentCommandTransition,
    SourceNameClass,
};
use crate::macro_call::ArgumentSet;
use crate::profile::CommandProfile;
use crate::{CommandError, CommandReplayDelivery, CurrentCommand};

use super::end_input::{RetirementHandoff, SourceExhaustionStatus};
use super::expand_render::format_pdf_date;
use super::{
    AlignmentInterceptionPolicy, AlignmentLookahead, CommandProcessor, DeliveryErrorSlot,
    DeliveryFailed, DeliveryMode, DeliveryPolicy, DeliveryStatus, ExpandedDeliveryPolicy,
    ExpandedObservationPolicy, FirstCommandPolicy, ReplayCompletionPolicy,
};

use crate::observation::{
    CommandDeliveryBoundary, CommandDeliveryRecord, CommandObservation, CommandProvenance,
    InputReason, InputRecord, InputTransition,
};

/// TeX82 §345's invalid source-character report.
const INVALID_SOURCE_CHARACTER_DIAGNOSTIC: u64 = 0x636f_6e64_0000_0345;

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

fn static_meaning<G>(meaning: &ResolvedMeaning<G>) -> Option<Meaning> {
    match meaning {
        ResolvedMeaning::Static(meaning) => Some(*meaning),
        ResolvedMeaning::Macro { .. } => None,
    }
}

/// The one decision TeX.web §380 makes after raw delivery has resolved the
/// current meaning.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExpandedCommandAction {
    Return,
    EndTemplate,
    Expand(ExpansionDispatch),
}

/// The exact TeX.web §366 branch selected by expanded-command
/// classification. This is call-local control flow, not a retained meaning
/// representation: a resource suspension continues to own only its one
/// `CurrentCommand` and re-borrows that meaning when the operation resumes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ExpansionDispatch {
    Macro,
    Primitive(ExpandablePrimitive),
    Undefined,
}

#[cfg(test)]
thread_local! {
    static EXPANDED_CLASSIFICATIONS: core::cell::Cell<u64> = const { core::cell::Cell::new(0) };
}

#[cfg(test)]
fn expanded_classifications() -> u64 {
    EXPANDED_CLASSIFICATIONS.with(core::cell::Cell::get)
}

#[inline(always)]
fn classify_expanded_command<G>(
    command: &CurrentCommand<G>,
    protected: ProtectedMacroHandling,
    undefined: UndefinedHandling,
) -> ExpandedCommandAction {
    #[cfg(test)]
    EXPANDED_CLASSIFICATIONS.with(|counter| counter.set(counter.get().saturating_add(1)));

    match command.meaning_ref() {
        ResolvedMeaning::Macro { flags, .. }
            if protected == ProtectedMacroHandling::Preserve
                && flags.contains(MeaningFlags::PROTECTED) =>
        {
            ExpandedCommandAction::Return
        }
        ResolvedMeaning::Macro { .. } => ExpandedCommandAction::Expand(ExpansionDispatch::Macro),
        ResolvedMeaning::Static(Meaning::ExpandablePrimitive(ExpandablePrimitive::EndTemplate)) => {
            ExpandedCommandAction::EndTemplate
        }
        ResolvedMeaning::Static(Meaning::ExpandablePrimitive(ExpandablePrimitive::EndCsName)) => {
            ExpandedCommandAction::Return
        }
        ResolvedMeaning::Static(Meaning::ExpandablePrimitive(primitive)) => {
            ExpandedCommandAction::Expand(ExpansionDispatch::Primitive(*primitive))
        }
        ResolvedMeaning::Static(Meaning::Undefined)
            if undefined == UndefinedHandling::Diagnose
                && !matches!(command.spelling().semantic_token(), Token::Param(_)) =>
        {
            ExpandedCommandAction::Expand(ExpansionDispatch::Undefined)
        }
        ResolvedMeaning::Static(_) => ExpandedCommandAction::Return,
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
fn is_ranked_fused_expansion(dispatch: ExpansionDispatch) -> bool {
    matches!(
        dispatch,
        ExpansionDispatch::Macro
            | ExpansionDispatch::Primitive(
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
            )
    )
}

impl<G> CommandProcessor<'_, '_, G> {
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
        self.get_x_token_from_into(None, ExpandedFetch::GetXToken, destination)
    }

    /// Delivers protected replay-aware expansion into caller-provided storage.
    pub(crate) fn get_x_or_protected_with_replay_completion_into(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
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
        let stamp = DeliveryStamp::new(0, 0);
        self.next_delivery_sequence = self.next_delivery_sequence.wrapping_add(1);
        let command = CurrentCommand::<G>::resolve(spelling, stamp, None, false, None, self.state);
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
                static_meaning(command.meaning_ref()),
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
                static_meaning(command.meaning_ref()),
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

    /// Delivers main-control preflight through one raw-fetch/classification
    /// loop. An ordinary unexpandable command publishes its canonical expanded
    /// observation directly, without completing a second expanded-driver
    /// episode; a macro, expandable primitive, or undefined command continues
    /// in place through the canonical expanded loop.
    pub fn preflight_command_into(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        let result = self.delivery_driver(
            DeliveryPolicy {
                mode: DeliveryMode::Expanded(ExpandedDeliveryPolicy {
                    fetch: ExpandedFetch::GetXToken,
                    protected_macros: ProtectedMacroHandling::Expand,
                    undefined: UndefinedHandling::Diagnose,
                    observation: ExpandedObservationPolicy::Commit,
                    first_command: FirstCommandPolicy::PreflightRaw,
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

    /// Resumes one genuinely suspended expansion from its stable parked root.
    /// The key is the executor's only command-related retry owner; consuming
    /// it moves the command once into `destination` before scalar expansion
    /// continues at the retained typed phase.
    pub fn resume_expansion_into(
        &mut self,
        key: crate::ExpansionWorkKey<G>,
        main_loop: bool,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        debug_assert!(destination.is_none());
        self.install_expansion_resume(key);
        self.delivery_driver(
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
        )
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
        self.invalidate_delivery_freshness();
        let mut error = DeliveryErrorSlot::empty();
        let result = match policy.mode {
            DeliveryMode::Raw => {
                debug_assert!(destination.is_none());
                self.delivery_state_machine::<false>(policy, None, false, destination, &mut error)
            }
            DeliveryMode::Expanded(expanded) => {
                self.expanded_delivery_entry(policy, expanded, destination, &mut error)
            }
        };
        match result {
            Ok(status) => Ok(status),
            Err(failure) => {
                // A resource suspension has already moved the exact command
                // into its typed expansion frame. Every other failure abandons
                // the delivery. In both cases the caller slot and its
                // DefinitionId owner must be empty, just as they were before
                // raw delivery started, and a later episode must mint a fresh
                // delivery proof.
                destination.take();
                self.invalidate_delivery_freshness();
                Err(error.take(failure))
            }
        }
    }

    fn expanded_delivery_entry(
        &mut self,
        policy: DeliveryPolicy,
        expanded: ExpandedDeliveryPolicy,
        destination: &mut Option<CurrentCommand<G>>,
        error: &mut DeliveryErrorSlot,
    ) -> Result<DeliveryStatus, DeliveryFailed> {
        let key = self.expansion_resume.take().or_else(|| {
            self.scanner_resume
                .as_ref()
                .is_some_and(crate::ScannerFrameKey::is_expansion)
                .then(|| {
                    let wrapper = self
                        .scanner_resume
                        .take()
                        .expect("matched expansion wrapper");
                    self.command
                        .scratch
                        .take_expansion_key(wrapper)
                        .expect("live wrapper owns expansion work")
                })
        });
        let resumed_pending = key.is_some();
        if let Some(key) = key {
            let mut retained = match self.command.scratch.resume_expansion(key) {
                Ok(retained) => retained,
                Err(failure) => {
                    return error.fail(crate::scan_toks::scratch_command_error(failure));
                }
            };
            if destination
                .as_ref()
                .is_some_and(|command| command != &retained.command)
            {
                if let Some(child) = retained.take_child()
                    && let Err(failure) = self.abort_continuation(child)
                {
                    return error.fail(failure);
                }
                return error.fail(CommandError::input_invariant());
            }
            if let Some(child) = retained.child.take() {
                let (key, child_destination) = child.restore();
                if child_destination != crate::state::PendingExpansionChildDestination::Dispatch {
                    return error.fail(CommandError::input_invariant());
                }
                self.scanner_resume = Some(key);
            }
            self.resumed_expansion = Some(retained.resume);
            *destination = Some(retained.command);
        }
        if resumed_pending && let Some(command) = &destination {
            self.resume_current_command(command);
        }
        let depth = self.command.transient.active_expansion_depth;
        let Some(active_depth) = depth.checked_add(1) else {
            return error.fail(CommandError::input_invariant());
        };
        self.command.transient.active_expansion_depth = active_depth;
        let result = self.delivery_state_machine::<true>(
            policy,
            Some(expanded),
            resumed_pending,
            destination,
            error,
        );
        assert_eq!(
            self.command.transient.active_expansion_depth, active_depth,
            "nested delivery must balance expansion depth"
        );
        self.command.transient.active_expansion_depth = depth;
        result
    }

    /// Advances resident input and inspects its resolved command in one
    /// destination-directed state machine. Cold transitions re-enter only
    /// after the command typestate borrow has ended.
    #[inline(always)]
    fn delivery_state_machine<const EXPANDED: bool>(
        &mut self,
        policy: DeliveryPolicy,
        expanded: Option<ExpandedDeliveryPolicy>,
        resumed_pending: bool,
        destination: &mut Option<CurrentCommand<G>>,
        error: &mut DeliveryErrorSlot,
    ) -> Result<DeliveryStatus, DeliveryFailed> {
        let expansions_before = self.command.expansion.cumulative_expansions;
        let mut first = true;
        let mut suppress_first_expansion_trace = resumed_pending;
        let mut fetch = destination.is_none();
        loop {
            if fetch {
                self.invalidate_delivery_freshness();
                if let Err(failure) = self.charge_command_action() {
                    return error.fail(failure);
                }
                if destination.is_none() {
                    *destination = Some(CurrentCommand::empty());
                }
                let raw_status = loop {
                    let transition = self
                        .command
                        .advance_resident_command_into(
                            self.state,
                            self.fuel,
                            self.create_source_control_sequences,
                            destination
                                .as_mut()
                                .expect("delivery machine owns its reusable command slot")
                                .empty_for_raw_delivery(),
                            (&mut self.observer, &mut self.immediate_write_retirement),
                        )
                        .map_err(|()| CommandError::input_invariant());
                    let transition = match transition {
                        Ok(transition) => transition,
                        Err(failure) => {
                            destination.take();
                            return error.fail(failure);
                        }
                    };
                    let interception = match transition {
                        ResidentCommandTransition::Empty => {
                            observe!(
                                self,
                                CommandObservation::Input(InputRecord {
                                    transition: InputTransition::Stop,
                                    reason: InputReason::Source,
                                    source_name: Some(SourceNameClass::Terminal),
                                    source: None,
                                    level: 0,
                                    position: 0,
                                }),
                            );
                            let restarts = match self.raw_end_restarts() {
                                Ok(restarts) => restarts,
                                Err(failure) => return error.fail(failure),
                            };
                            if restarts {
                                continue;
                            }
                            destination.take();
                            break DeliveryStatus::End;
                        }
                        ResidentCommandTransition::Delivered { interception } => interception,
                        ResidentCommandTransition::ParameterPushed(parameter_level) => {
                            observe!(
                                self,
                                CommandObservation::Input(InputRecord {
                                    transition: InputTransition::Push,
                                    reason: InputReason::Parameter,
                                    source_name: None,
                                    source: None,
                                    level: parameter_level.0,
                                    position: 0,
                                }),
                            );
                            continue;
                        }
                        ResidentCommandTransition::InvalidCharacter => {
                            self.report_recoverable(
                                INVALID_SOURCE_CHARACTER_DIAGNOSTIC,
                                "Text line contains an invalid character".into(),
                                &[
                                    "A funny symbol that I can't read has just been input.",
                                    "Continue, and I'll forget that it ever happened.",
                                ],
                            );
                            continue;
                        }
                        ResidentCommandTransition::NeedLine(identity) => {
                            let line = match self.acquire_source_line(true) {
                                Ok(line) => line,
                                Err(failure) => return error.fail(failure),
                            };
                            let exhausted = if line.is_none() {
                                match self.finish_exhausted_source(identity) {
                                    Ok(status) => {
                                        matches!(status, SourceExhaustionStatus::End)
                                    }
                                    Err(failure) => return error.fail(failure),
                                }
                            } else {
                                false
                            };
                            if exhausted {
                                let restarts = match self.raw_end_restarts() {
                                    Ok(restarts) => restarts,
                                    Err(failure) => return error.fail(failure),
                                };
                                if restarts {
                                    continue;
                                }
                                destination.take();
                                break DeliveryStatus::End;
                            }
                            continue;
                        }
                        ResidentCommandTransition::SourceExhausted(identity) => {
                            #[cfg(test)]
                            {
                                self.command
                                    .raw_delivery_path_counters
                                    .cold_source_retirements = self
                                    .command
                                    .raw_delivery_path_counters
                                    .cold_source_retirements
                                    .saturating_add(1);
                            }
                            let exhausted = match self.finish_exhausted_source(identity) {
                                Ok(status) => matches!(status, SourceExhaustionStatus::End),
                                Err(failure) => return error.fail(failure),
                            };
                            if exhausted {
                                let restarts = match self.raw_end_restarts() {
                                    Ok(restarts) => restarts,
                                    Err(failure) => return error.fail(failure),
                                };
                                if restarts {
                                    continue;
                                }
                                destination.take();
                                break DeliveryStatus::End;
                            }
                            continue;
                        }
                        ResidentCommandTransition::TokenExhausted { identity, .. } => {
                            #[cfg(test)]
                            {
                                self.command
                                    .raw_delivery_path_counters
                                    .exhaustion_status_relays = self
                                    .command
                                    .raw_delivery_path_counters
                                    .exhaustion_status_relays
                                    .saturating_add(1);
                            }
                            let Some((index, active_source)) = self
                                .command
                                .input
                                .levels
                                .last()
                                .and_then(|level| match level {
                                    InputLevel::Tokens(cursor) if cursor.identity() == identity => {
                                        Some((
                                            cursor.frame.position(),
                                            cursor.frame.source_context(),
                                        ))
                                    }
                                    InputLevel::MacroArgument(cursor)
                                        if cursor.identity() == identity =>
                                    {
                                        Some((
                                            cursor.frame.position() as u32,
                                            cursor.frame.source_context(),
                                        ))
                                    }
                                    _ => None,
                                })
                            else {
                                return error.fail(CommandError::input_invariant());
                            };
                            let handoff = match self.retire_input_top(identity) {
                                Ok(handoff) => handoff,
                                Err(failure) => return error.fail(failure),
                            };
                            match handoff {
                                RetirementHandoff::Stop => {
                                    let restarts = match self.raw_end_restarts() {
                                        Ok(restarts) => restarts,
                                        Err(failure) => return error.fail(failure),
                                    };
                                    if restarts {
                                        continue;
                                    }
                                    destination.take();
                                    break DeliveryStatus::End;
                                }
                                RetirementHandoff::Continue => continue,
                                RetirementHandoff::Completed(episode) => {
                                    destination.take();
                                    break DeliveryStatus::ReplayCompleted(episode);
                                }
                                RetirementHandoff::EndV(level) => {
                                    let _resolution = destination
                                        .as_mut()
                                        .expect("delivery machine owns its reusable command slot")
                                        .empty_for_raw_delivery()
                                        .write_resolved_delivery(
                                            TokenWord::pack(self.state.frozen_end_template_token()),
                                            OriginId::UNKNOWN,
                                            level.0,
                                            u64::from(index),
                                            active_source,
                                            false,
                                            None,
                                            false,
                                            self.state,
                                        );
                                    #[cfg(feature = "profiling")]
                                    {
                                        self.fuel.record_raw_delivery(
                                            !matches!(
                                                self.command.scanner.status(),
                                                crate::processor::ScannerStatus::Normal
                                            ),
                                            _resolution.meaning_lookup(),
                                            crate::fuel::RawDeliveryKind::SyntheticEndV,
                                        );
                                    }
                                    ResidentCommandInterception::Ready
                                }
                            }
                        }
                        ResidentCommandTransition::ReplayCompleted(episode) => {
                            destination.take();
                            break DeliveryStatus::ReplayCompleted(episode);
                        }
                    };

                    let command = destination
                        .as_mut()
                        .expect("resident delivery initializes the command slot");
                    self.next_delivery_sequence = self.next_delivery_sequence.wrapping_add(1);
                    self.publish_delivery_freshness(command.delivery_stamp());
                    if matches!(interception, ResidentCommandInterception::Outer)
                        && let Err(failure) = self.check_outer_validity_entry(command)
                    {
                        destination.take();
                        return error.fail(failure);
                    }
                    if self.is_observed() {
                        self.observe_resident_command(command);
                    }
                    break DeliveryStatus::Command;
                };
                match raw_status {
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
            if !EXPANDED {
                if policy.alignment_interception == AlignmentInterceptionPolicy::Scalar
                    && matches!(
                        destination
                            .as_ref()
                            .expect("raw destination contains a command")
                            .alignment_adjustment(),
                        crate::processor::AlignmentDeliveryAdjustment::Delimiter(_)
                    )
                {
                    if let Err(failure) = self.begin_scalar_alignment_v_template(
                        destination
                            .as_ref()
                            .expect("raw destination contains a command"),
                    ) {
                        return error.fail(failure);
                    }
                    destination.take();
                    fetch = true;
                    continue;
                }
                return Ok(DeliveryStatus::Command);
            }
            let expanded = expanded.expect("expanded specialization owns its policy");
            let command = destination
                .as_ref()
                .expect("expanded destination contains a command");

            let first = std::mem::take(&mut first);
            let action =
                classify_expanded_command(command, expanded.protected_macros, expanded.undefined);
            if first && expanded.first_command == FirstCommandPolicy::MainLoopCharacter {
                if is_main_loop_character(command.meaning_ref()) {
                    return Ok(DeliveryStatus::Command);
                }
                if action == ExpandedCommandAction::Return {
                    debug_assert_eq!(expanded.observation, ExpandedObservationPolicy::Commit);
                    self.observe_expanded_delivery(command);
                    return Ok(DeliveryStatus::Command);
                }
            }
            if first
                && expanded.first_command == FirstCommandPolicy::PreflightRaw
                && action == ExpandedCommandAction::Return
            {
                debug_assert_eq!(expanded.observation, ExpandedObservationPolicy::Commit);
                self.observe_expanded_delivery(command);
                return Ok(DeliveryStatus::Command);
            }
            match action {
                ExpandedCommandAction::EndTemplate => {
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
                        if let Err(failure) = self.begin_scalar_alignment_v_template(command) {
                            return error.fail(failure);
                        }
                        fetch = true;
                        continue;
                    }
                    if expanded.fetch == ExpandedFetch::XToken {
                        // §366 `expand` has no `end_template` shortcut: it routes
                        // straight to §375, which backs up a `frozen_endv` token
                        // for this loop's own `get_next` to reread.
                        if let Err(failure) = self.insert_frozen_endv() {
                            return error.fail(failure);
                        }
                        fetch = true;
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
                ExpandedCommandAction::Return => {
                    return Ok(self.finish_expanded_delivery(
                        command,
                        expanded,
                        expansions_before,
                        policy.alignment_interception,
                    ));
                }
                ExpandedCommandAction::Expand(dispatch) => {
                    // TeX82 §394 aborts a non-`\long` macro call after its
                    // recovery bookkeeping, then resumes the enclosing
                    // expanded-token loop. A user paragraph has been backed
                    // up for that loop; an EOF recovery paragraph was consumed
                    // by the failed match instead.
                    let report_trace = !std::mem::take(&mut suppress_first_expansion_trace);
                    let failure = match self.expand_into(destination, Some(dispatch), report_trace)
                    {
                        Ok(()) => {
                            fetch = true;
                            continue;
                        }
                        Err(failure) => failure,
                    };
                    // TeX82 §394 resumes expanded delivery after both an
                    // ordinary runaway paragraph and §23's outer-validity
                    // recovery has aborted a macro match. The latter leaves
                    // the recovered outer token in backup input for its
                    // normal reread.
                    match failure {
                        CommandError::ParagraphInMacroArgument
                        | CommandError::OuterInMacroArgument => {
                            fetch = true;
                        }
                        failure => return error.fail(failure),
                    }
                }
            }
        }
    }

    fn finish_expanded_delivery(
        &mut self,
        command: &CurrentCommand<G>,
        policy: ExpandedDeliveryPolicy,
        expansions_before: u64,
        alignment: AlignmentInterceptionPolicy,
    ) -> DeliveryStatus {
        #[cfg(feature = "profiling")]
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
                    self.current_delivery_sequence(),
                    command.origin(),
                    self.direct_source_provenance(command),
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

    /// Expands the command in one caller-owned destination, moving that sole
    /// owner into parked work only across an immutable-resource suspension.
    /// Resumption restores the same value into the destination before
    /// continuing, while preserving §367's already emitted trace.
    pub(crate) fn expand_into(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
        classified: Option<ExpansionDispatch>,
        mut report_trace: bool,
    ) -> Result<(), CommandError> {
        let resumed_here = self.resumed_expansion.is_some();
        let mut expansion_resume = self
            .resumed_expansion
            .take()
            .unwrap_or(crate::state::PendingExpansionResume::Dispatch);
        if !resumed_here
            && self.scanner_resume.is_some()
            && !self
                .scanner_resume
                .as_ref()
                .is_some_and(crate::ScannerFrameKey::is_expansion)
        {
            return Err(CommandError::input_invariant());
        }
        if !resumed_here
            && self
                .scanner_resume
                .as_ref()
                .is_some_and(crate::ScannerFrameKey::is_expansion)
        {
            let wrapper = self
                .scanner_resume
                .take()
                .expect("matched expansion wrapper");
            let key = self
                .command
                .scratch
                .take_expansion_key(wrapper)
                .map_err(crate::scan_toks::scratch_command_error)?;
            let mut retained = self
                .command
                .scratch
                .resume_expansion(key)
                .map_err(crate::scan_toks::scratch_command_error)?;
            if destination.is_some() {
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
            *destination = Some(retained.command);
            self.resume_current_command(
                destination
                    .as_ref()
                    .expect("resumed expansion restores its command destination"),
            );
            report_trace = false;
        }
        let command = destination
            .as_mut()
            .ok_or_else(CommandError::input_invariant)?;
        let dispatch = if let Some(dispatch) = classified {
            dispatch
        } else {
            match classify_expanded_command(
                command,
                ProtectedMacroHandling::Expand,
                UndefinedHandling::Diagnose,
            ) {
                ExpandedCommandAction::Expand(dispatch) => dispatch,
                // Direct callers implement TeX82 §366 `expand`, where the
                // `end_template` branch inserts frozen `endv`; only §380's
                // expanded-delivery classifier handles it inline.
                ExpandedCommandAction::EndTemplate => {
                    ExpansionDispatch::Primitive(ExpandablePrimitive::EndTemplate)
                }
                ExpandedCommandAction::Return => return Err(CommandError::input_invariant()),
            }
        };
        #[cfg(feature = "profiling")]
        {
            if !is_ranked_fused_expansion(dispatch) {
                tex_state::measurement::record_hot_core_materialization(
                    tex_state::measurement::HotCoreMaterialization::ExpansionCommand,
                );
            }
            match dispatch {
                ExpansionDispatch::Primitive(primitive) => {
                    tex_state::measurement::record_hot_core_expandable_opcode(
                        usize::try_from(primitive.operand())
                            .expect("expandable primitive operand fits usize"),
                    );
                }
                ExpansionDispatch::Macro => {
                    tex_state::measurement::record_hot_core_macro_expansion();
                }
                ExpansionDispatch::Undefined => {}
            }
        }
        #[cfg(feature = "profiling")]
        if self.write_expansion_depth != 0 {
            self.record_write_expansion();
        }
        self.command
            .timeline
            .record_cumulative_expansions(self.command.expansion.cumulative_expansions);
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
        let traceable = matches!(
            dispatch,
            ExpansionDispatch::Primitive(primitive)
                if primitive != ExpandablePrimitive::EndTemplate
        ) || dispatch == ExpansionDispatch::Undefined;
        if report_trace
            && traceable
            && self
                .state
                .int_param(tex_state::env::banks::IntParam::TRACING_COMMANDS)
                > 1
        {
            self.print_command_trace(crate::PrintCommand::from_current(command));
        }
        let mut suspended_resume = None;
        let result = (|| {
            match dispatch {
                ExpansionDispatch::Macro => {
                    match self.macro_call(command)? {
                        crate::macro_call::MacroCallOutcome::Activated => {}
                        crate::macro_call::MacroCallOutcome::PrefixMismatchRecovered => {}
                    }
                    Ok(())
                }
                ExpansionDispatch::Undefined => {
                    #[cfg(feature = "profiling")]
                    tex_state::measurement::record_hot_core_undefined_expansion();
                    let context = self.command.output_open_context(self.state);
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
                ExpansionDispatch::Primitive(primitive)
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
                ExpansionDispatch::Primitive(ExpandablePrimitive::Unless) => {
                    self.expand_unless(command, &mut expansion_resume, &mut suspended_resume)
                }
                ExpansionDispatch::Primitive(
                    primitive @ (ExpandablePrimitive::Else
                    | ExpandablePrimitive::Or
                    | ExpandablePrimitive::Fi),
                ) => self.expand_conditional_delimiter(command, primitive),
                // TeX82 §375's `end_template` case replaces the inaccessible
                // sentinel that ended a v-template with the distinct frozen
                // `endv` token. Neither sentinel is a user-installable primitive;
                // §780 gives them only frozen control-sequence slots.
                ExpansionDispatch::Primitive(ExpandablePrimitive::EndTemplate) => {
                    self.insert_frozen_endv()
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::NoExpand) => {
                    self.expand_noexpand()
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::ExpandAfter) => {
                    self.expand_expandafter()
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::CsName) => {
                    self.expand_csname(command, &mut expansion_resume, &mut suspended_resume)
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::String) => {
                    self.expand_string(command)
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::Meaning) => {
                    self.expand_meaning(command)
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::Number) => {
                    self.expand_number(command, false, &mut expansion_resume, &mut suspended_resume)
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::RomanNumeral) => {
                    self.expand_number(command, true, &mut expansion_resume, &mut suspended_resume)
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::The) => {
                    self.expand_the(command, &mut expansion_resume, &mut suspended_resume)
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::Unexpanded) => {
                    self.expand_unexpanded()
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::Expanded) => {
                    self.expand_expanded()
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::Detokenize) => {
                    self.expand_detokenize(command)
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::Scantokens) => {
                    self.expand_scantokens()
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::FontName) => self
                    .expand_fontname(
                        command.copy_for_backup(),
                        &mut expansion_resume,
                        &mut suspended_resume,
                    ),
                // pdftex.web §470's `pdf_font_size_code` conversion prints the
                // selected font size as an ordinary scaled dimension.
                ExpansionDispatch::Primitive(ExpandablePrimitive::PdfFontSize) => self
                    .expand_pdf_font_size(
                        command.copy_for_backup(),
                        &mut expansion_resume,
                        &mut suspended_resume,
                    ),
                // pdftex.web §470 scans e-TeX's extended box-register domain,
                // then queries typed hlist state for the first non-skipable node
                // at the requested edge.
                ExpansionDispatch::Primitive(
                    primitive @ (ExpandablePrimitive::LeftMarginKern
                    | ExpandablePrimitive::RightMarginKern),
                ) => self.expand_margin_kern(
                    command.copy_for_backup(),
                    primitive,
                    &mut expansion_resume,
                    &mut suspended_resume,
                ),
                ExpansionDispatch::Primitive(ExpandablePrimitive::Input) => {
                    self.expand_input(command.copy_for_backup())
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::EndInput) => {
                    self.expand_endinput()
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::JobName) => {
                    self.state.unsupported_host_capability();
                    let job_name = self.host.job_name().to_owned();
                    self.push_rendered_text(&job_name, command.origin());
                    Ok(())
                }
                // e-TeX 2.6 etex.ch §3211 installs `\eTeXrevision` as a
                // `convert` command; §1387 prints the immutable revision string
                // through TeX82 §470's ordinary conversion-token path.
                ExpansionDispatch::Primitive(ExpandablePrimitive::ETeXRevision) => {
                    self.push_rendered_text(".6", command.origin());
                    Ok(())
                }
                // pdfTeX §57.4 exposes the revision suffix independently of the
                // integer `\pdftexversion` parameter.
                ExpansionDispatch::Primitive(ExpandablePrimitive::PdfTeXRevision) => {
                    self.push_rendered_text("27", command.origin());
                    Ok(())
                }
                // pdftex.web §§494 and 496--498 install `\pdftexbanner` as an
                // operand-free `convert`: `conv_toks` prints the process banner,
                // then returns it through the ordinary `str_toks`/`ins_list`
                // conversion path. `utils.c::makepdftexbanner` appends the pinned
                // TeX Live and kpathsea identities to pdftex.web §2's banner.
                ExpansionDispatch::Primitive(ExpandablePrimitive::PdfTeXBanner) => {
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
                ExpansionDispatch::Primitive(ExpandablePrimitive::PdfUniformDeviate) => self
                    .expand_pdf_uniform_deviate(
                        command,
                        &mut expansion_resume,
                        &mut suspended_resume,
                    ),
                ExpansionDispatch::Primitive(ExpandablePrimitive::PdfNormalDeviate) => {
                    let value = self.state.pdf_normal_deviate();
                    self.push_rendered_text(&value.to_string(), command.origin());
                    Ok(())
                }
                // pdftex.web §1590's `pdf_creation_date_code` conversion calls
                // `getcreationdate`, then returns the fixed job-start timestamp
                // through the ordinary `str_toks`/`ins_list` conversion path.
                // Both the LaTeX-compatible `\creationdate` spelling and
                // pdfTeX's `\pdfcreationdate` spelling share this meaning.
                ExpansionDispatch::Primitive(ExpandablePrimitive::CreationDate) => {
                    let clock = self.state.job_clock();
                    self.push_rendered_text(&format_pdf_date(clock, 0), command.origin());
                    Ok(())
                }
                // pdfTeX and XeTeX change section [53a] report shell escape as
                // 0 (disabled), 1 (unrestricted), or 2 (restricted). Umber's
                // LaTeX compatibility spelling is an expandable alias over the
                // same tracked World policy used by `\pdfshellescape`.
                ExpansionDispatch::Primitive(ExpandablePrimitive::ShellEscape) => {
                    let status = self
                        .state
                        .internal_integer(tex_state::meaning::InternalInteger::PdfShellEscape)
                        .expect("the shell-escape status is an integer enquiry");
                    self.push_rendered_text(&status.to_string(), command.origin());
                    Ok(())
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::StringCompare) => {
                    self.expand_string_compare(command.copy_for_backup())
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::PdfEscapeString) => {
                    self.expand_pdf_escape_string(command.copy_for_backup())
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::PdfEscapeHex) => {
                    self.expand_pdf_escape_hex(command.copy_for_backup())
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::PdfUnescapeHex) => {
                    self.expand_pdf_unescape_hex(command.copy_for_backup())
                }
                ExpansionDispatch::Primitive(ExpandablePrimitive::PdfColorStackInit) => self
                    .expand_pdf_color_stack_init(
                        command.copy_for_backup(),
                        &mut expansion_resume,
                        &mut suspended_resume,
                    ),
                ExpansionDispatch::Primitive(ExpandablePrimitive::PdfMatch) => self
                    .expand_pdf_match(
                        command.copy_for_backup(),
                        &mut expansion_resume,
                        &mut suspended_resume,
                    ),
                ExpansionDispatch::Primitive(ExpandablePrimitive::PdfLastMatch) => self
                    .expand_pdf_last_match(
                        command.copy_for_backup(),
                        &mut expansion_resume,
                        &mut suspended_resume,
                    ),
                ExpansionDispatch::Primitive(ExpandablePrimitive::PdfFileDump) => self
                    .expand_pdf_file_dump(
                        command.copy_for_backup(),
                        &mut expansion_resume,
                        &mut suspended_resume,
                    ),
                ExpansionDispatch::Primitive(ExpandablePrimitive::FileSize) => self
                    .expand_pdf_file_size(
                        command.copy_for_backup(),
                        &mut expansion_resume,
                        &mut suspended_resume,
                    ),
                ExpansionDispatch::Primitive(ExpandablePrimitive::PdfFileModificationDate) => self
                    .expand_pdf_file_modification_date(
                        command.copy_for_backup(),
                        &mut expansion_resume,
                        &mut suspended_resume,
                    ),
                ExpansionDispatch::Primitive(ExpandablePrimitive::PdfMdFiveSum) => self
                    .expand_pdf_md_five_sum(
                        command.copy_for_backup(),
                        &mut expansion_resume,
                        &mut suspended_resume,
                    ),
                ExpansionDispatch::Primitive(ExpandablePrimitive::PdfInsertHeight) => self
                    .expand_pdf_insert_height(
                        command.copy_for_backup(),
                        &mut expansion_resume,
                        &mut suspended_resume,
                    ),
                // pdftex.web §470's `pdf_ximage_bbox_code` conversion scans an
                // existing image object before its one-based page-box coordinate.
                // The enquiry reads detached metadata only; it never reserves an
                // image or writer object while expanding.
                ExpansionDispatch::Primitive(ExpandablePrimitive::PdfXImageBBox) => self
                    .expand_pdf_ximage_bbox(command, &mut expansion_resume, &mut suspended_resume),
                // pdftex.web §1549's `pdf_xform_name_code` conversion scans a
                // form object number and prints its independent resource identity.
                // Unknown object numbers produce zero, matching the other PDF
                // object enquiries rather than manufacturing ledger state.
                ExpansionDispatch::Primitive(ExpandablePrimitive::PdfXFormName) => self
                    .expand_pdf_xform_name(command, &mut expansion_resume, &mut suspended_resume),
                // pdftex.web §470's `pdf_page_ref_code` conversion scans a one-based
                // shipped-page number and prints its page-object identity. Pages
                // that do not exist yet expand to zero without reserving
                // speculative writer state; nonpositive operands are rejected by
                // the conversion's `pdf_error` guard.
                ExpansionDispatch::Primitive(ExpandablePrimitive::PdfPageRef) => {
                    self.expand_pdf_page_ref(command, &mut expansion_resume, &mut suspended_resume)
                }
                // pdfTeX §57.1 consumes one raw token and, only for a registered
                // primitive spelling, replays the immutable frozen primitive.
                // The ordinary expanded loop then dispatches that original
                // meaning without consulting the shadowable live cell.
                ExpansionDispatch::Primitive(ExpandablePrimitive::PdfPrimitive) => {
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
                ExpansionDispatch::Primitive(
                    primitive @ (ExpandablePrimitive::TopMark
                    | ExpandablePrimitive::FirstMark
                    | ExpandablePrimitive::BotMark
                    | ExpandablePrimitive::SplitFirstMark
                    | ExpandablePrimitive::SplitBotMark),
                ) => self.expand_mark(primitive),
                ExpansionDispatch::Primitive(
                    primitive @ (ExpandablePrimitive::TopMarks
                    | ExpandablePrimitive::FirstMarks
                    | ExpandablePrimitive::BotMarks
                    | ExpandablePrimitive::SplitFirstMarks
                    | ExpandablePrimitive::SplitBotMarks),
                ) => {
                    self.expand_mark_class(primitive, &mut expansion_resume, &mut suspended_resume)
                }
                ExpansionDispatch::Primitive(primitive) => {
                    Err(CommandError::UnsupportedExpandablePrimitive(primitive))
                }
            }
        })();
        if result
            .as_ref()
            .is_err_and(CommandError::is_resource_suspension)
        {
            let child = crate::execution_scratch::ChildContinuation::capture(
                &mut self.scanner_resume,
                crate::state::PendingExpansionChildDestination::Dispatch,
            );
            let error = result.expect_err("matched resource suspension");
            let pending = crate::state::PendingExpansion {
                command: destination
                    .take()
                    .expect("suspension moves the command out of its destination"),
                resume: suspended_resume
                    .take()
                    .unwrap_or(crate::state::PendingExpansionResume::Dispatch),
                child,
            };
            return match self.command.scratch.store_expansion_frame(pending) {
                Ok(key) => {
                    self.scanner_resume = Some(key);
                    Err(error)
                }
                Err((store_error, mut pending)) => {
                    if let Some(child) = pending.take_child()
                        && let Err(failure) = self.abort_continuation(child)
                    {
                        return Err(failure);
                    }
                    Err(crate::scan_toks::scratch_command_error(store_error))
                }
            };
        } else if let Some(child) = self.scanner_resume.take() {
            self.abort_continuation(child)?;
            if result.is_ok() {
                return Err(CommandError::input_invariant());
            }
        }
        result
    }

    pub(super) fn retain_expansion_scalar<T>(
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
        definition_region: tex_state::DefinitionRegionLease<G>,
        call_site: OriginId,
        arguments: Option<ArgumentSet<G>>,
        replacement_len: usize,
    ) -> InputLevelId {
        let invocation = call_site;
        self.command.push_macro_activation(
            name,
            definition,
            definition_region,
            arguments,
            invocation,
            replacement_len,
        )
    }
}

/// TeX82 §1038's raw-accepted set: `letter`, `other_char`, and `char_given`.
///
/// These are exactly the three commands §1034's inner loop can continue on
/// without expanding, so they are the only ones the lookahead delivers
/// straight out of `get_next`.
pub(crate) fn is_main_loop_character<G>(meaning: &ResolvedMeaning<G>) -> bool {
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

/// TeX82 §366's `cur_cmd>max_command` test for Umber's resolved command.
///
/// `Meaning::Undefined` normally represents §207's `undefined_cs` command,
/// which is expanded solely to perform §370's diagnostic recovery. A compact
/// out-parameter token also carries that meaning as its invalid-slot recovery,
/// but its command remains `out_param<max_command`; its token spelling keeps
/// the two command identities distinct here.
pub(crate) fn is_expandable_command<G>(command: &CurrentCommand<G>) -> bool {
    let meaning = command.meaning_ref();
    matches!(meaning, ResolvedMeaning::Macro { .. })
        || matches!(meaning, ResolvedMeaning::Static(Meaning::ExpandablePrimitive(primitive)) if *primitive != ExpandablePrimitive::EndCsName)
        || (matches!(meaning, ResolvedMeaning::Static(Meaning::Undefined))
            && !matches!(command.spelling().semantic_token(), Token::Param(_)))
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
    pub(crate) profile: CommandProfile,
}
