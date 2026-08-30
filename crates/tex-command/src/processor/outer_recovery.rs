//! Scanner-status interception and terminal runaway recovery.

use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use crate::command::CurrentCommand;
use crate::error::CommandError;
use crate::input::SourceNameClass;
use crate::input::{
    InputLevel, PackedTokenSpanHandle, ReplayTrace, RetirementBehavior, TokenBehavior,
};
use crate::observation::{
    AlignmentRecord, CommandObservation, DiagnosticArgument, DiagnosticRecord, InputReason,
    InputRecord, InputTransition, RecoveryKind, RecoveryRecord,
};

use super::CommandProcessor;
use super::status::{EofLegality, RecoveryContext, ScannerStatus};

/// TeX82 §336's `Incomplete \if...; all text was ignored after line N`.
const INCOMPLETE_CONDITIONAL_DIAGNOSTIC: u64 = 0x636f_6e64_0000_0336;

/// TeX82 §338's `File ended` / `Forbidden control sequence found` while
/// scanning a definition, use, preamble or text.
pub(crate) const RUNAWAY_SCAN_DIAGNOSTIC: u64 = 0x636f_6e64_0000_0338;

impl<G> CommandProcessor<'_, '_, G> {
    pub(super) fn check_outer_validity_entry(
        &mut self,
        command: &mut CurrentCommand<G>,
    ) -> Result<(), CommandError> {
        if !command.is_outer() || matches!(self.command.scanner.status(), ScannerStatus::Normal) {
            return Ok(());
        }

        let recovery = self.command.scanner.recovery_context();
        // The pinned TeX82 observer instruments `check_outer_validity` at
        // entry, before §23 backs up the forbidden outer control sequence.
        // Keep this command-owned: the backup still belongs to raw delivery,
        // but the diagnostic describes the live scanner episode that selected
        // its recovery.
        self.observe_outer_validity_diagnostic(&recovery.status, false);
        if matches!(recovery.status, ScannerStatus::Matching(_)) {
            self.outer_recovered_while_matching = true;
        }
        if matches!(recovery.status, ScannerStatus::Absorbing(_)) {
            self.outer_recovered_while_absorbing = true;
        }
        // TeX82 §336 deliberately does not back up an outer control
        // sequence delivered by a `\read` pseudo-file (`name=1..17`).  The
        // one-line source remains live until §483's `end_file_reading`, so
        // the recovery context must continue to name that read frame rather
        // than manufacture a `<to be read again>` token-list level.
        let delivered_by_read = self.command.input.levels.iter().any(|level| {
            let InputLevel::Source(source) = level else {
                return false;
            };
            source.identity().0 == command.delivery_stamp().input_level()
                && matches!(
                    self.command
                        .input
                        .levels
                        .source_level_slot(source)
                        .name_class,
                    SourceNameClass::ReadStream(_)
                )
        });
        if !delivered_by_read {
            self.back_input(command.copy_for_backup())?;
        }
        self.install_outer_recovery(recovery, false)?;
        command.recover_as_space();
        Ok(())
    }
    /// Recovers terminal input only while a scanner episode is live. The
    /// inserted tokens are then delivered through this same raw loop.
    pub(super) fn recover_runaway_eof(&mut self) -> Result<bool, CommandError> {
        let recovery = self.command.scanner.recovery_context();
        if matches!(self.command.scanner.eof_legality(), EofLegality::Legal) {
            return Ok(false);
        }
        if matches!(recovery.status, ScannerStatus::Matching(_)) {
            // TeX82 §23 calls `check_outer_validity` after retiring an input
            // file at EOF. Its frozen `\par` ends the failed §394 match; it
            // is not an ordinary paragraph that `back_error` replays after
            // the expansion returns.
            self.eof_recovered_while_matching = true;
        }
        self.observe_outer_validity_diagnostic(&recovery.status, true);
        self.install_outer_recovery(recovery, true)?;
        Ok(true)
    }
    /// TeX.web's `check_outer_validity` recovery table. Primitive insertions
    /// are frozen tokens, retaining their original meanings if user code has
    /// reassigned their visible spellings.
    fn install_outer_recovery(
        &mut self,
        recovery: RecoveryContext,
        at_file_end: bool,
    ) -> Result<(), CommandError> {
        let RecoveryContext { status, .. } = recovery;
        if matches!(status, ScannerStatus::Aligning(_)) {
            // TeX82 §23's `check_outer_validity` reports the aligning
            // recovery before `ins_error` inserts inaccessible frozen `\cr`.
            // This remains a command-owned observation: the executor has no
            // raw scanner-status or token-list recovery capability.
            self.observe(CommandObservation::Alignment(AlignmentRecord {
                transition: "outer_validity",
                alignment: self
                    .command
                    .alignment
                    .active_alignment
                    .map(|identity| identity.raw()),
                nesting: self.command.alignment_observation_nesting(),
                align_state: self.command.alignment.align_state,
                delimiter: None,
                previous_align_state: None,
            }));
        }
        let (frozen_recovery_name, recovery_kind) = match &status {
            ScannerStatus::Skipping(_) => (Some("fi"), RecoveryKind::InsertedToken),
            ScannerStatus::Matching(_) => (Some("par"), RecoveryKind::InsertedControlSequence),
            // TeX82's `check_outer_validity` inserts frozen `\\cr` before
            // its required follow-up right brace. The recovery event denotes
            // the inaccessible control sequence alone; raw delivery owns
            // the whole displayed insertion.
            ScannerStatus::Aligning(_) => (Some("cr"), RecoveryKind::InsertedControlSequence),
            ScannerStatus::Normal | ScannerStatus::Defining(_) | ScannerStatus::Absorbing(_) => {
                (None, RecoveryKind::InsertedToken)
            }
        };
        let (first_token, second_token) = match status {
            ScannerStatus::Normal => return Ok(()),
            ScannerStatus::Skipping(_) => (self.frozen_primitive("fi")?, None),
            ScannerStatus::Defining(_) | ScannerStatus::Absorbing(_) => (right_brace(), None),
            ScannerStatus::Matching(_) => (self.frozen_primitive("par")?, None),
            ScannerStatus::Aligning(_) => (self.frozen_primitive("cr")?, Some(right_brace())),
        };
        // TeX82 §23 leaves `scanner_status := aligning` live while its
        // inserted frozen `\cr` finishes `init_align`'s preamble scan.
        // `get_preamble_token` therefore owns the one aligning-to-normal
        // transition at typed preamble completion. Every other recovery
        // remains an immediate scanner-episode exit before its inserted input
        // is delivered.
        let retains_aligning_until_preamble_completion =
            matches!(status, ScannerStatus::Aligning(_))
                && self.command.alignment.active_alignment.is_some();
        if !retains_aligning_until_preamble_completion {
            self.command.scanner.clear_for_recovery();
        }
        let observed_tokens = std::iter::once(first_token)
            .chain(second_token)
            .enumerate()
            .filter_map(|(index, token)| {
                if index != 0 && matches!(&status, ScannerStatus::Aligning(_)) {
                    return None;
                }
                (index == 0)
                    .then_some(frozen_recovery_name)
                    .flatten()
                    .map(|name| crate::observation::ObservedToken::ControlSequence(name.into()))
                    .or_else(|| Some(self.observed_token(token)))
            })
            .collect();
        let level = self.command.push_token_level(
            PackedTokenSpanHandle::transient(std::iter::once(first_token).chain(second_token)),
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::Inserted,
        );
        if matches!(&status, ScannerStatus::Aligning(_)) {
            self.command.alignment.pending_outer_recovery_cr = Some(self.frozen_primitive("cr")?);
        }
        // §336 ends with `ins_error`, which is `back_input` as an *inserted*
        // level and only then `error`. The context therefore renders with the
        // frozen recovery token already on the stack, which is what puts the
        // `<inserted text>` line above the source line.
        // §338's `<Tell the user what has run away and try to recover>`, the
        // sibling branch of §336's incomplete-conditional report. `runaway`'s
        // own heading and partial list are still owed (umber2-2iil); this is
        // the `error` §338 ends with.
        if let Some((kind, warning_index)) = match &status {
            ScannerStatus::Defining(context) => Some(("definition", context.target)),
            ScannerStatus::Matching(context) => Some(("use", Some(context.macro_name))),
            ScannerStatus::Aligning(context) => Some(("preamble", context.owner)),
            ScannerStatus::Absorbing(context) => Some(("text", context.owner)),
            ScannerStatus::Normal | ScannerStatus::Skipping(_) => None,
        } {
            // §338 chooses its opening on `cur_cs`: a forbidden `\outer`
            // control sequence rather than the file simply running out.
            let opening = if at_file_end {
                "File ended"
            } else {
                "Forbidden control sequence found"
            };
            let name = warning_index.map_or_else(String::new, |symbol| {
                let spelling = self.state.resolve(symbol).to_owned();
                super::expand_render::print_esc_text(self.state, &spelling)
            });
            let context = self.command.output_open_context(self.state);
            let heading = match &status {
                ScannerStatus::Defining(_) => "Runaway definition?",
                ScannerStatus::Matching(_) => "Runaway argument?",
                ScannerStatus::Aligning(_) => "Runaway preamble?",
                ScannerStatus::Absorbing(_) => "Runaway text?",
                ScannerStatus::Normal | ScannerStatus::Skipping(_) => unreachable!(),
            };
            let partial = match &status {
                ScannerStatus::Aligning(context) => self
                    .command
                    .transient
                    .builders
                    .iter()
                    .find(|builder| builder.identity == context.builder.0)
                    .and_then(|builder| {
                        self.command
                            .attempt
                            .arena()
                            .token_buffer(builder.tokens)
                            .ok()
                    })
                    .map_or_else(String::new, |tokens| {
                        tokens.iter().fold(String::new(), |mut text, token| {
                            super::expand_render::append_token_list_token_text(
                                self.state,
                                token.semantic_token(),
                                &mut text,
                            );
                            text
                        })
                    }),
                _ => String::new(),
            };
            self.command
                .semantic_diagnostics
                .push(crate::CommandSemanticDiagnostic::Recoverable {
                    identity: RUNAWAY_SCAN_DIAGNOSTIC,
                    runaway: Some(crate::state::RunawayPrelude { heading, partial }),
                    message: format!("{opening} while scanning {kind} of {name}"),
                    help: &[
                        "I suspect you have forgotten a `}', causing me",
                        "to read past where you wanted me to stop.",
                        "I'll try to recover; but if the error is serious,",
                        "you'd better type `E' or `X' now and fix your file.",
                    ],
                    context,
                    integer_error: None,
                });
        }
        if let ScannerStatus::Skipping(skipping) = &status {
            let name = super::expand_render::print_esc_text(
                self.state,
                skipping.conditional.canonical_name(),
            );
            let message = format!(
                "Incomplete {name}; all text was ignored after line {}",
                skipping.skip_line
            );
            let help: &'static [&'static str] = if at_file_end {
                // §336 replaces only the first help line when the failure was
                // the file running out rather than a forbidden control
                // sequence appearing in the skipped text.
                &[
                    "The file ended while I was skipping conditional text.",
                    "This kind of error happens when you say `\\if...' and forget",
                    "the matching `\\fi'. I've inserted a `\\fi'; this might work.",
                ]
            } else {
                &[
                    "A forbidden control sequence occurred in skipped text.",
                    "This kind of error happens when you say `\\if...' and forget",
                    "the matching `\\fi'. I've inserted a `\\fi'; this might work.",
                ]
            };
            let context = self.command.output_open_context(self.state);
            self.command
                .semantic_diagnostics
                .push(crate::CommandSemanticDiagnostic::Recoverable {
                    identity: INCOMPLETE_CONDITIONAL_DIAGNOSTIC,
                    runaway: None,
                    message,
                    help,
                    context,
                    integer_error: None,
                });
        }
        observe!(
            self,
            CommandObservation::Input(InputRecord {
                transition: InputTransition::Recovery,
                reason: InputReason::Recovery,
                source_name: None,
                source: None,
                level: level.0,
                position: 0,
            }),
        );
        observe!(
            self,
            CommandObservation::Recovery(RecoveryRecord {
                kind: recovery_kind,
                tokens: observed_tokens,
            }),
        );
        Ok(())
    }
    pub(crate) fn set_runaway_partial(&mut self, identity: u64, tokens: &[TracedTokenWord]) {
        // TeX82 §262 carries `match_chr` from each match token into later
        // out-parameter rendering. The compact representation stores a
        // nonstandard match character immediately before its slot token.
        let mut raw = String::new();
        let mut match_marker = '#';
        let mut index = 0;
        while index < tokens.len() {
            let token = tokens[index].semantic_token();
            if let Token::Char {
                ch,
                cat: Catcode::Parameter,
            } = token
                && let Some(Token::Param(slot)) =
                    tokens.get(index + 1).map(|word| word.semantic_token())
            {
                match_marker = ch;
                raw.push(ch);
                raw.push(char::from(b'0' + slot));
                index += 2;
                continue;
            }
            if let Token::Param(slot) = token {
                raw.push(match_marker);
                raw.push(char::from(b'0' + slot));
            } else {
                super::expand_render::append_token_list_token_text(self.state, token, &mut raw);
            }
            index += 1;
        }
        let mut rendered = String::new();
        self.state.append_selector_string_text(&raw, &mut rendered);
        // TeX82 §306 reads the current scanner's live list synchronously. A
        // command trace discovered by the same raw-delivery episode can
        // already be queued behind the deferred report, so update the newest
        // report owned by this scanner episode rather than assuming it is the
        // queue tail. In particular, a later scan must not overwrite a
        // completed §396 runaway-argument pseudoprint still awaiting output.
        let Some(runaway) =
            self.command
                .semantic_diagnostics
                .iter_mut()
                .rev()
                .find_map(|diagnostic| match diagnostic {
                    crate::CommandSemanticDiagnostic::Recoverable {
                        identity: diagnostic_identity,
                        runaway: Some(runaway),
                        ..
                    } if *diagnostic_identity == identity => Some(runaway),
                    _ => None,
                })
        else {
            return;
        };
        runaway.partial = rendered;
    }
    fn observe_outer_validity_diagnostic(&mut self, status: &ScannerStatus, at_eof: bool) {
        let arguments = if self.command.profile().capabilities().supports_etex() {
            // The e-TeX 2.6 oracle's §336 seam records the outer-validity
            // boundary without scanner metadata. Scanner status remains a
            // separate canonical transition.
            Vec::new()
        } else {
            vec![DiagnosticArgument::Name(
                match status {
                    ScannerStatus::Normal => "normal",
                    ScannerStatus::Skipping(_) => "skipping",
                    ScannerStatus::Defining(_) => "defining",
                    ScannerStatus::Matching(_) => "matching",
                    ScannerStatus::Aligning(_) => "aligning",
                    ScannerStatus::Absorbing(_) => "absorbing",
                }
                .into(),
            )]
        };
        self.observe(CommandObservation::Diagnostic(DiagnosticRecord {
            severity: "error",
            diagnostic: if at_eof {
                "outer_validity_eof"
            } else {
                "outer_validity_control_sequence"
            },
            arguments,
        }));
        if at_eof && matches!(status, ScannerStatus::Skipping(_)) {
            self.observe(CommandObservation::Diagnostic(DiagnosticRecord {
                severity: "error",
                diagnostic: "conditional_incomplete",
                arguments: Vec::new(),
            }));
        }
    }
    fn frozen_primitive(&self, name: &str) -> Result<TracedTokenWord, CommandError> {
        Ok(TracedTokenWord::pack(
            self.frozen_primitive_token(name)?,
            OriginId::UNKNOWN,
        ))
    }
}

fn right_brace() -> TracedTokenWord {
    TracedTokenWord::pack(
        Token::Char {
            ch: '}',
            cat: Catcode::EndGroup,
        },
        OriginId::UNKNOWN,
    )
}
