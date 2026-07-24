//! Command-core main-control seam used by canonical replay.
//!
//! This intentionally owns only typed execution mutations. Raw delivery,
//! expansion, macro calls, input nesting, and operand collection remain in
//! `tex-command`; no `InputStack` is accepted here.

use tex_command::{
    AlignmentDelivery, AlignmentIdentity, AlignmentRequest, CommandError, CommandHostCapabilities,
    CommandHostContext, CommandProcessor, CommandRuntime, CommandState,
};
use tex_state::interner::Symbol;
use tex_state::macro_store::{MacroDefinitionProvenance, MacroMeaning};
use tex_state::meaning::{Meaning, MeaningFlags, UnexpandablePrimitive};
use tex_state::token::Catcode;
use tex_state::{PrintSink, TracedTokenList, Universe};

use crate::{ExecError, Mode, ModeNest};

/// Replay-only command main control with command-owned source consumption.
#[derive(Debug, Default)]
pub struct CommandReplayControl {
    command: CommandState,
    runtime: CommandRuntime,
    capabilities: CommandHostCapabilities,
    modes: ModeNest,
    next_alignment_identity: u64,
    active_alignment: Option<AlignmentIdentity>,
}

impl CommandReplayControl {
    /// Borrows canonical command state for source registration and snapshots.
    #[must_use]
    pub fn command_mut(&mut self) -> &mut CommandState {
        &mut self.command
    }

    /// Borrows executor-installed host capabilities for the next operation.
    #[must_use]
    pub fn capabilities_mut(&mut self) -> &mut CommandHostCapabilities {
        &mut self.capabilities
    }

    /// Returns the replay projection of TeX's current execution mode.
    #[must_use]
    pub fn current_mode(&self) -> Mode {
        self.modes.current_mode()
    }

    /// Returns the structural alignment started by the most recent replayed
    /// `\halign` or `\valign`, if it has not yet been finished.
    #[must_use]
    pub fn active_alignment(&self) -> Option<AlignmentIdentity> {
        self.active_alignment
    }

    /// Applies an executor-selected alignment lifecycle transition.
    ///
    /// The request contains no token spelling, so this cannot create another
    /// delimiter-classification or source-consumption path.
    pub fn apply_alignment_request(&mut self, request: AlignmentRequest) -> Result<(), ExecError> {
        let finished = matches!(request, AlignmentRequest::Finish(_));
        let identity = match request {
            AlignmentRequest::Begin(identity)
            | AlignmentRequest::Preamble(identity)
            | AlignmentRequest::FinishCell(identity)
            | AlignmentRequest::Suspend(identity)
            | AlignmentRequest::Resume(identity)
            | AlignmentRequest::Finish(identity) => identity,
            AlignmentRequest::BeginCell { alignment, .. } => alignment,
        };
        self.command
            .apply_alignment_request(request)
            .map(|_| ())
            .map_err(|_| ExecError::MissingToken {
                context: "alignment lifecycle",
            })?;
        if finished && self.active_alignment == Some(identity) {
            self.active_alignment = None;
        }
        Ok(())
    }

    /// Delivers one expanded command for an active alignment cell.
    ///
    /// In particular, the opaque end-template event is returned to the same
    /// command processor episode that delivered it, so the processor alone
    /// backs up the delimiter and installs the selected v-template.
    pub fn alignment_step(
        &mut self,
        alignment: AlignmentIdentity,
        stores: &mut Universe,
    ) -> Result<ReplayStep, ExecError> {
        let scanned = {
            let mut processor = CommandProcessor::new(
                &mut self.command,
                &mut self.runtime,
                stores.command_context(),
                CommandHostContext::new(&mut self.capabilities),
            );
            match processor
                .get_x_alignment_delivery()
                .map_err(command_error)?
            {
                None => ScannedStep::EndOfInput,
                Some(AlignmentDelivery::Command(command)) => {
                    scan_command(&mut processor, command, false, MeaningFlags::EMPTY)?
                }
                Some(AlignmentDelivery::Event(event)) => {
                    processor
                        .begin_alignment_v_template(alignment, event)
                        .map_err(command_error)?;
                    ScannedStep::Continue
                }
            }
        };
        apply_scanned_step(
            scanned,
            stores,
            &mut self.modes,
            &mut self.next_alignment_identity,
            &mut self.active_alignment,
            &mut self.command,
        )
    }

    /// Delivers and executes one replay command through the command processor.
    pub fn step(&mut self, stores: &mut Universe) -> Result<ReplayStep, ExecError> {
        let scanned = {
            let mut processor = CommandProcessor::new(
                &mut self.command,
                &mut self.runtime,
                stores.command_context(),
                CommandHostContext::new(&mut self.capabilities),
            );
            scan_step(&mut processor)?
        };
        apply_scanned_step(
            scanned,
            stores,
            &mut self.modes,
            &mut self.next_alignment_identity,
            &mut self.active_alignment,
            &mut self.command,
        )
    }
}

/// The structural outcome of one typed replay operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReplayStep {
    Continue,
    EndOfInput,
    End,
}

enum ScannedStep {
    Continue,
    EndOfInput,
    End,
    Count {
        index: u16,
        value: i32,
        global: bool,
    },
    MacroDefinition {
        target: Symbol,
        flags: MeaningFlags,
        global: bool,
        parameter_text: TracedTokenList,
        replacement_text: TracedTokenList,
        definition_origin: tex_state::token::OriginId,
    },
    Message {
        tokens: TracedTokenList,
    },
    BeginAlignment {
        vertical: bool,
    },
    Paragraph,
    MathShift,
    Character,
}

fn scan_step(processor: &mut CommandProcessor<'_>) -> Result<ScannedStep, ExecError> {
    let Some(mut command) = processor.get_x_token().map_err(command_error)? else {
        return Ok(ScannedStep::EndOfInput);
    };
    let mut global = false;
    let mut flags = MeaningFlags::EMPTY;
    loop {
        match command.meaning() {
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Global) => global = true,
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Long) => {
                flags = flags | MeaningFlags::LONG
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Outer) => {
                flags = flags | MeaningFlags::OUTER
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Protected) => {
                flags = flags | MeaningFlags::PROTECTED
            }
            _ => break,
        }
        command = next_non_space(processor)?.ok_or(ExecError::MissingPrefixedCommand)?;
    }
    scan_command(processor, command, global, flags)
}

fn scan_command(
    processor: &mut CommandProcessor<'_>,
    command: tex_command::CurrentCommand,
    global: bool,
    flags: MeaningFlags,
) -> Result<ScannedStep, ExecError> {
    match command.meaning() {
        Meaning::UnexpandablePrimitive(
            UnexpandablePrimitive::End | UnexpandablePrimitive::Dump,
        ) => Ok(ScannedStep::End),
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Count) => {
            let index = processor.scan_integer().map_err(command_error)?.value;
            let _ = processor.scan_optional_equals().map_err(command_error)?;
            let value = processor.scan_integer().map_err(command_error)?.value;
            let index =
                u16::try_from(index).map_err(|_| ExecError::RegisterNumberOutOfRange(index))?;
            Ok(ScannedStep::Count {
                index,
                value,
                global,
            })
        }
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::Def
            | UnexpandablePrimitive::Edef
            | UnexpandablePrimitive::Gdef
            | UnexpandablePrimitive::Xdef),
        ) => {
            let target = next_non_space(processor)?
                .and_then(|target| target.control_sequence())
                .ok_or(ExecError::MissingControlSequence {
                    context: "macro definition",
                })?;
            let expanded = matches!(
                primitive,
                UnexpandablePrimitive::Edef | UnexpandablePrimitive::Xdef
            );
            let definition = processor
                .scan_macro_definition(expanded)
                .map_err(command_error)?;
            Ok(ScannedStep::MacroDefinition {
                target,
                flags,
                global: global
                    || matches!(
                        primitive,
                        UnexpandablePrimitive::Gdef | UnexpandablePrimitive::Xdef
                    ),
                parameter_text: definition.parameter_text,
                replacement_text: definition.replacement_text,
                definition_origin: definition.provenance.primary,
            })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Message) => {
            let tokens = processor.scan_balanced_text(true).map_err(command_error)?;
            Ok(ScannedStep::Message {
                tokens: tokens.tokens,
            })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::HAlign) => {
            Ok(ScannedStep::BeginAlignment { vertical: false })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::VAlign) => {
            Ok(ScannedStep::BeginAlignment { vertical: true })
        }
        Meaning::UnexpandablePrimitive(
            UnexpandablePrimitive::Par | UnexpandablePrimitive::EndGraf,
        ) => Ok(ScannedStep::Paragraph),
        Meaning::CharToken {
            cat: Catcode::MathShift,
            ..
        } => Ok(ScannedStep::MathShift),
        Meaning::CharToken {
            cat: Catcode::Letter | Catcode::Other,
            ..
        } => Ok(ScannedStep::Character),
        _ => Ok(ScannedStep::Continue),
    }
}

fn replay_text(tokens: &[tex_state::token::Token]) -> String {
    tokens
        .iter()
        .filter_map(|token| match token {
            tex_state::token::Token::Char { ch, .. } => Some(*ch),
            _ => None,
        })
        .collect()
}

fn next_non_space(
    processor: &mut CommandProcessor<'_>,
) -> Result<Option<tex_command::CurrentCommand>, ExecError> {
    loop {
        let Some(command) = processor.get_x_token().map_err(command_error)? else {
            return Ok(None);
        };
        if !matches!(
            command.meaning(),
            Meaning::CharToken {
                cat: tex_state::token::Catcode::Space,
                ..
            }
        ) {
            return Ok(Some(command));
        }
    }
}

fn apply_scanned_step(
    scanned: ScannedStep,
    stores: &mut Universe,
    modes: &mut ModeNest,
    next_alignment_identity: &mut u64,
    active_alignment: &mut Option<AlignmentIdentity>,
    command: &mut CommandState,
) -> Result<ReplayStep, ExecError> {
    match scanned {
        ScannedStep::Continue => Ok(ReplayStep::Continue),
        ScannedStep::EndOfInput => Ok(ReplayStep::EndOfInput),
        ScannedStep::End => Ok(ReplayStep::End),
        ScannedStep::Count {
            index,
            value,
            global,
        } => {
            if global {
                stores.set_count_global(index, value);
            } else {
                stores.set_count(index, value);
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::MacroDefinition {
            target,
            flags,
            global,
            parameter_text,
            replacement_text,
            definition_origin,
        } => {
            let meaning = MacroMeaning::new(
                flags,
                parameter_text.token_list(),
                replacement_text.token_list(),
            );
            let provenance = MacroDefinitionProvenance::new(
                definition_origin,
                parameter_text.origin_list(),
                replacement_text.origin_list(),
            );
            if global {
                stores.set_macro_meaning_global_with_provenance(target, meaning, provenance);
            } else {
                stores.set_macro_meaning_with_provenance(target, meaning, provenance);
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::Message { tokens } => {
            let text = replay_text(stores.tokens(tokens.token_list()));
            stores
                .world_mut()
                .write_text(PrintSink::TerminalAndLog, &text);
            Ok(ReplayStep::Continue)
        }
        ScannedStep::BeginAlignment { vertical } => {
            let identity = AlignmentIdentity::new(*next_alignment_identity);
            *next_alignment_identity = next_alignment_identity.wrapping_add(1);
            command
                .apply_alignment_request(AlignmentRequest::Begin(identity))
                .map_err(|_| ExecError::MissingToken {
                    context: "alignment lifecycle",
                })?;
            *active_alignment = Some(identity);
            if vertical && modes.current_mode() == Mode::Vertical {
                modes.push(Mode::InternalVertical);
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::Paragraph => {
            if matches!(
                modes.current_mode(),
                Mode::Horizontal | Mode::RestrictedHorizontal
            ) {
                let _ = modes.pop()?;
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::MathShift => {
            match modes.current_mode() {
                Mode::Math | Mode::DisplayMath => {
                    let _ = modes.pop()?;
                }
                Mode::Vertical => modes.push(Mode::DisplayMath),
                _ => modes.push(Mode::Math),
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::Character => {
            if modes.current_mode() == Mode::Vertical {
                modes.push(Mode::Horizontal);
            }
            Ok(ReplayStep::Continue)
        }
    }
}

fn command_error(error: CommandError) -> ExecError {
    match error {
        CommandError::MissingInput => ExecError::MissingToken { context: "\\input" },
        _ => ExecError::MissingToken {
            context: "command processor",
        },
    }
}

#[cfg(test)]
mod tests;
