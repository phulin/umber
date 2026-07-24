//! Executor-facing structured scanners owned by the command input machine.
//!
//! These wrappers intentionally expose frozen values, provenance, and the
//! canonical filename termination only.  Input levels, raw tokens, and macro
//! argument frames remain private to `tex-command`.

use tex_state::glue::GlueSpec;
use tex_state::interner::Symbol;
use tex_state::meaning::{Meaning, UnexpandablePrimitive};
use tex_state::scaled::Scaled;
use tex_state::token::{Catcode, OriginId, Token};
use tex_state::{SourceId, TracedTokenList};

use crate::processor::status::{
    AlignmentId, AlignmentScanContext, ScannerStatus, ScannerWarning, TokenBuilderId,
};
use crate::scan_toks::{ScanToksMode, ScannedToks};
use crate::{AlignmentCellTemplates, AlignmentPreamble, CommandError, CommandProcessor};

/// Provenance for a completed structured scan.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StructuredProvenance {
    /// Origin of the first non-ignored token accepted by the scan.
    pub primary: OriginId,
}

/// A balanced token list frozen through the aggregate token store.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedBalancedText {
    pub tokens: TracedTokenList,
    pub provenance: StructuredProvenance,
}

/// The two immutable lists collected for a macro definition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedMacroDefinition {
    pub parameter_text: TracedTokenList,
    pub replacement_text: TracedTokenList,
    pub provenance: StructuredProvenance,
}

/// A completed TeX82 `\let` or `\futurelet` assignment.
///
/// The command processor owns every raw operand delivery, including the
/// optional equals sign and `\futurelet`'s lookahead replay. Replay receives
/// only the target and its already-resolved source meaning.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedLetAssignment {
    pub target: Symbol,
    pub source: Option<Symbol>,
    pub meaning: Meaning,
}

/// The command-owned operand prefix of TeX82's `\setbox` assignment.
///
/// The following box command deliberately remains a normal main-control
/// delivery.  This preserves TeX82's `scan_int`/optional-equals recovery
/// before executor-owned box construction takes over.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedSetBoxAssignment {
    pub index: i32,
}

/// A completed named glue-parameter assignment.
///
/// Command processing owns the optional equals sign and scalar glue scan;
/// replay receives only the parameter selector and its finished value.  The
/// `mu` flag preserves the TeX distinction between ordinary and math glue
/// parameters without exposing another input path to the executor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedGlueParameterAssignment {
    pub index: u16,
    pub value: GlueSpec,
    pub mu: bool,
}

/// A completed TeX82 `\hrule` or `\vrule` specification.
///
/// The command processor owns the expanded `width`, `height`, and `depth`
/// keyword scans and their scalar operands. Replay receives only these final
/// dimensions, so applying a rule cannot open another source-consumption path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedRuleSpec {
    pub width: Option<Scaled>,
    pub height: Option<Scaled>,
    pub depth: Option<Scaled>,
}

/// The canonical boundary that stopped an unbraced filename scan.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileNameTermination {
    Group,
    Space,
    NonCharacter,
    EndOfInput,
}

/// A filename scanned from expanded command-owned input.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScannedFileName {
    pub name: String,
    pub termination: FileNameTermination,
    pub provenance: StructuredProvenance,
}

/// One successfully opened capability-registered input source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegisteredInput {
    pub file_name: ScannedFileName,
    pub source: SourceId,
}

impl CommandProcessor<'_> {
    /// Scans the register number and optional equals sign of `\setbox`.
    ///
    /// TeX.web's `prefixed_command` dispatches `set_box` to `scan_int` then
    /// `scan_optional_equals`; the latter must retain its ordinary backup
    /// transition when the equals sign is present.
    pub fn scan_setbox_assignment(&mut self) -> Result<ScannedSetBoxAssignment, CommandError> {
        let index = self.scan_integer()?.value;
        let _ = self.scan_optional_equals()?;
        Ok(ScannedSetBoxAssignment { index })
    }

    /// Scans TeX82's named glue-parameter assignment operand.
    ///
    /// This follows `scan_optional_equals` and `scan_glue`, retaining their
    /// canonical backup and alignment-delivery transitions before replay
    /// applies the aggregate mutation.
    pub fn scan_glue_parameter_assignment(
        &mut self,
        index: u16,
        mu: bool,
    ) -> Result<ScannedGlueParameterAssignment, CommandError> {
        let _ = self.scan_optional_equals()?;
        let value = self.scan_glue(mu)?.value;
        Ok(ScannedGlueParameterAssignment { index, value, mu })
    }

    /// Scans the complete expanded specification of a TeX82 rule.
    ///
    /// This is TeX.web's `scan_rule_spec`: keyword recognition and dimension
    /// scanning stay in command control, including failed-keyword replay.
    pub fn scan_rule_spec(
        &mut self,
        primitive: UnexpandablePrimitive,
    ) -> Result<ScannedRuleSpec, CommandError> {
        let default_rule = Scaled::from_raw(26_214);
        let (mut width, mut height, mut depth) = if primitive == UnexpandablePrimitive::VRule {
            (Some(default_rule), None, None)
        } else {
            (None, Some(default_rule), Some(Scaled::from_raw(0)))
        };
        loop {
            if self.scan_keyword("width")?.value {
                width = Some(self.scan_dimension()?.value);
            } else if self.scan_keyword("height")?.value {
                height = Some(self.scan_dimension()?.value);
            } else if self.scan_keyword("depth")?.value {
                depth = Some(self.scan_dimension()?.value);
            } else {
                break;
            }
        }
        Ok(ScannedRuleSpec {
            width,
            height,
            depth,
        })
    }

    /// Validates a box body's required opening brace, then restores it for
    /// executor-owned group entry.
    pub fn scan_box_group_opening(&mut self) -> Result<(), CommandError> {
        let opening = self.scan_left_brace(true)?;
        self.back_input(opening)
    }

    /// Validates an alignment preamble's required opening brace, then restores
    /// it for canonical preamble delivery.
    ///
    /// TeX.web's `init_align` consumes this brace through expanded command
    /// input before `get_preamble_token` starts.  The subsequent
    /// `back_input` is therefore not an executor replay path: it is the one
    /// command-owned backup level whose literal-brace adjustment is undone
    /// before the preamble sees the same brace again.
    pub fn scan_alignment_preamble_opening(&mut self) -> Result<(), CommandError> {
        let opening = self.scan_left_brace(true)?;
        self.back_input(opening)
    }

    /// Replays the alignment preamble's opening brace into the next scanner
    /// episode.
    ///
    /// TeX.web's first `get_preamble_token` consumes the backup made by
    /// `init_align`, then backs that same brace up once more before the
    /// preamble scanner starts.  Retiring the exhausted first backup is part
    /// of this input transition, so it must remain command-owned rather than
    /// being recreated by replay main control.
    pub fn replay_alignment_preamble_opening(&mut self) -> Result<(), CommandError> {
        let opening = self.scan_left_brace(true)?;
        self.back_input_after_backup_replay(opening)
    }

    /// Delivers the first alignment cell's source opening brace, then backs
    /// it up before the selected u-template is installed.
    ///
    /// This is TeX82's `init_row` ordering: the source brace changes and then
    /// restores `align_state` through ordinary command delivery, while the
    /// executor receives no raw token.
    pub fn scan_alignment_cell_opening(&mut self) -> Result<(), CommandError> {
        let opening = self.scan_left_brace(true)?;
        self.back_input(opening)
    }

    /// Enters TeX82's live alignment-preamble scanner episode.
    ///
    /// `init_align` establishes `scanner_status := aligning` after its
    /// required brace has been replayed and backed up, but before the first
    /// `get_preamble_token` retires that backup.  The status therefore belongs
    /// to the command-owned input transition, rather than to executor replay
    /// or the preamble parser.
    pub fn begin_alignment_preamble_scan(&mut self) -> Result<(), CommandError> {
        // `get_preamble_token` first observes the brace replayed by the
        // second backup, then `init_align` installs its live scanner status.
        // Its next raw fetch retires that backup before reading the first
        // preamble token.
        let _ = self.scan_left_brace(true)?;
        let alignment = self
            .command
            .alignment
            .active_alignment
            .ok_or(CommandError::InputInvariant)?;
        self.command
            .alignment
            .set_preamble_phase(alignment)
            .map_err(|_| CommandError::InputInvariant)?;
        let _prior =
            self.command
                .begin_scanner_status(ScannerStatus::Aligning(AlignmentScanContext {
                    alignment: AlignmentId(alignment.raw()),
                    builder: TokenBuilderId(0),
                    warning: ScannerWarning(0),
                }));
        self.observe_scanner_status(true);
        #[cfg(any(test, feature = "instrumentation"))]
        self.observe(crate::CommandObservation::Alignment(
            crate::AlignmentRecord {
                transition: "preamble_start",
                alignment: Some(alignment.raw()),
                align_state: self.command.alignment.align_state,
                delimiter: None,
                previous_align_state: None,
            },
        ));
        let _ = self.get_token()?;
        let mut columns = Vec::new();
        let mut u_template = Vec::new();
        let mut v_template = Vec::new();
        let mut in_v_template = false;
        loop {
            let command = self.get_next()?.ok_or(CommandError::InputInvariant)?;
            let token = command.spelling().semantic_token();
            if matches!(
                token,
                Token::Char {
                    cat: Catcode::Parameter,
                    ..
                }
            ) && !in_v_template
            {
                in_v_template = true;
                continue;
            }
            let ends_column = matches!(
                token,
                Token::Char {
                    cat: Catcode::AlignmentTab,
                    ..
                }
            );
            let ends_preamble = matches!(
                command.meaning(),
                Meaning::UnexpandablePrimitive(
                    UnexpandablePrimitive::Cr | UnexpandablePrimitive::CrCr
                )
            );
            if ends_column || ends_preamble {
                columns.push(AlignmentCellTemplates {
                    u_template: (!u_template.is_empty())
                        .then(|| self.state.finish_traced_token_list(&u_template)),
                    v_template: self.state.finish_traced_token_list(&v_template),
                });
                u_template.clear();
                v_template.clear();
                in_v_template = false;
                if ends_preamble {
                    break;
                }
                continue;
            }
            if in_v_template {
                v_template.push(command.spelling());
            } else {
                u_template.push(command.spelling());
            }
        }
        self.command
            .alignment
            .complete_preamble(alignment, AlignmentPreamble { columns })
            .map_err(|_| CommandError::InputInvariant)?;
        #[cfg(any(test, feature = "instrumentation"))]
        self.observe(crate::CommandObservation::Alignment(
            crate::AlignmentRecord {
                transition: "preamble_finish",
                alignment: Some(alignment.raw()),
                align_state: self.command.alignment.align_state,
                delimiter: None,
                previous_align_state: None,
            },
        ));
        // TeX's `fin_align` boundary becomes observable before `scanner_status`
        // returns to normal. Retain the live aligning episode while publishing
        // its completion, then restore normal status; otherwise an exit record
        // loses its `aligning` identity and reverses the canonical ordering.
        self.observe_scanner_status(false);
        let _ = self.command.begin_scanner_status(ScannerStatus::Normal);
        Ok(())
    }

    /// Scans TeX's balanced general text through the canonical `scan_toks`
    /// collector. `expanded` controls its TeX82 expanded-collection mode.
    pub fn scan_balanced_text(
        &mut self,
        expanded: bool,
    ) -> Result<ScannedBalancedText, CommandError> {
        // Expanded general text (for `\message`, for example) enters its
        // absorbing episode before the required opening brace. The ordinary
        // token-list assignment scan retains TeX82's initial validation and
        // replay, whose backup transition is separately observable.
        if !expanded {
            let opening = self.scan_left_brace(true)?;
            self.back_input(opening)?;
        }
        let scanned = self.scan_toks(ScanToksMode::General { expanded })?;
        Ok(ScannedBalancedText {
            tokens: scanned.replacement_text,
            provenance: provenance(&scanned),
        })
    }

    /// Scans a macro parameter text and replacement text without exposing the
    /// temporary macro-argument matcher or its input frames.
    pub fn scan_macro_definition(
        &mut self,
        expanded: bool,
    ) -> Result<ScannedMacroDefinition, CommandError> {
        let scanned = self.scan_toks(ScanToksMode::MacroDefinition { expanded })?;
        Ok(ScannedMacroDefinition {
            parameter_text: scanned.parameter_text,
            replacement_text: scanned.replacement_text,
            provenance: provenance(&scanned),
        })
    }

    /// Scans TeX82's raw `\let` operand sequence.
    ///
    /// `future` selects `future_let`: the first two raw tokens following the
    /// target are restored in their original order after the second token's
    /// meaning has been captured.
    pub fn scan_let_assignment(
        &mut self,
        future: bool,
    ) -> Result<ScannedLetAssignment, CommandError> {
        let target = self
            .next_non_space_raw()?
            .and_then(|command| command.control_sequence())
            .ok_or(CommandError::InputInvariant)?;
        let (source, meaning) = if future {
            let first = self.get_token()?.ok_or(CommandError::InputInvariant)?;
            let second = self.get_token()?.ok_or(CommandError::InputInvariant)?;
            let source = second.control_sequence();
            let meaning = second.meaning();
            self.replay_raw_commands([first, second]);
            (source, meaning)
        } else {
            let mut source = self.get_token()?.ok_or(CommandError::InputInvariant)?;
            if matches!(source.meaning(), Meaning::CharToken { ch: '=', .. }) {
                source = self.get_token()?.ok_or(CommandError::InputInvariant)?;
                if matches!(
                    source.meaning(),
                    Meaning::CharToken {
                        cat: Catcode::Space,
                        ..
                    }
                ) {
                    source = self.get_token()?.ok_or(CommandError::InputInvariant)?;
                }
            }
            (source.control_sequence(), source.meaning())
        };
        Ok(ScannedLetAssignment {
            target,
            source,
            meaning,
        })
    }

    /// TeX's `scan_file_name`, returning a typed boundary instead of an input
    /// cursor or a backed-up raw command.
    pub fn scan_file_name(&mut self) -> Result<ScannedFileName, CommandError> {
        let first = loop {
            let command = self.get_x_token()?.ok_or(CommandError::InputInvariant)?;
            if !matches!(
                command.meaning(),
                Meaning::CharToken {
                    cat: Catcode::Space,
                    ..
                }
            ) {
                break command;
            }
        };
        let provenance = StructuredProvenance {
            primary: first.origin(),
        };
        let grouped = matches!(
            first.meaning(),
            Meaning::CharToken {
                cat: Catcode::BeginGroup,
                ..
            }
        );
        // `scan_file_name` replays its first non-space token before consuming
        // the filename. TeX82 exposes this `back_input` hand-off, and it
        // keeps the group-opening case on the same ordinary delivery path.
        self.back_input(first)?;
        let mut name = String::new();
        let mut quoted = false;
        let mut next = None;
        let termination = loop {
            let command = match next.take() {
                Some(command) => command,
                None => match self.get_x_token()? {
                    Some(command) => command,
                    None => break FileNameTermination::EndOfInput,
                },
            };
            match command.meaning() {
                Meaning::CharToken {
                    cat: Catcode::BeginGroup,
                    ..
                } if grouped => {}
                Meaning::CharToken { ch: '"', .. } => quoted = !quoted,
                Meaning::CharToken {
                    cat: Catcode::EndGroup,
                    ..
                } if grouped && !quoted => {
                    break FileNameTermination::Group;
                }
                Meaning::CharToken {
                    cat: Catcode::Space,
                    ..
                } if !grouped && !quoted => {
                    break FileNameTermination::Space;
                }
                Meaning::CharToken { ch, .. } => name.push(ch),
                _ if !grouped => {
                    self.back_input(command)?;
                    break FileNameTermination::NonCharacter;
                }
                _ => return Err(CommandError::InputInvariant),
            }
        };
        if name.is_empty() {
            return Err(CommandError::InputInvariant);
        }
        Ok(ScannedFileName {
            name,
            termination,
            provenance,
        })
    }

    /// Scans and opens one input through the borrow-scoped registered-input
    /// capability. No filesystem or host lookup escapes this boundary.
    pub fn open_registered_input(&mut self) -> Result<RegisteredInput, CommandError> {
        let file_name = self.scan_file_name()?;
        let source = self
            .host
            .input(&file_name.name)
            .ok_or(CommandError::MissingInput)?;
        let source = self
            .command
            .register_source(source)
            .map_err(|_| CommandError::InputInvariant)?;
        self.command
            .open_registered_source(source)
            .map_err(|_| CommandError::InputInvariant)?;
        Ok(RegisteredInput { file_name, source })
    }

    fn next_non_space_raw(&mut self) -> Result<Option<crate::CurrentCommand>, CommandError> {
        loop {
            let Some(command) = self.get_token()? else {
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

    fn replay_raw_commands(&mut self, commands: [crate::CurrentCommand; 2]) {
        for command in &commands {
            self.undo_alignment_delivery(command);
        }
        self.command.push_token_level(
            crate::input::TokenPayload::BackedUp(crate::input::SharedBackedUpBuffer::new(
                commands.map(|command| crate::input::BackedUpToken {
                    spelling: command.spelling(),
                    source_range: command.source_range(),
                }),
            )),
            crate::input::TokenBehavior::BackedUp(crate::input::BackupTreatment::Ordinary),
            crate::input::RetirementBehavior::Pop,
            crate::input::ReplayTrace::BackedUp,
        );
    }
}

fn provenance(scanned: &ScannedToks) -> StructuredProvenance {
    StructuredProvenance {
        primary: scanned.primary,
    }
}

#[cfg(test)]
mod tests;
