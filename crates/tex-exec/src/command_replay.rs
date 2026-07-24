//! Command-core main-control seam used by canonical replay.
//!
//! This intentionally owns only typed execution mutations. Raw delivery,
//! expansion, macro calls, input nesting, and operand collection remain in
//! `tex-command`; no `InputStack` is accepted here.

use tex_command::{
    CommandError, CommandHostCapabilities, CommandHostContext, CommandProcessor, CommandRuntime,
    CommandState,
};
use tex_state::interner::Symbol;
use tex_state::macro_store::{MacroDefinitionProvenance, MacroMeaning};
use tex_state::meaning::{Meaning, MeaningFlags, UnexpandablePrimitive};
use tex_state::{TracedTokenList, Universe};

use crate::ExecError;

/// Replay-only command main control with command-owned source consumption.
#[derive(Debug, Default)]
pub struct CommandReplayControl {
    command: CommandState,
    runtime: CommandRuntime,
    capabilities: CommandHostCapabilities,
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
        apply_scanned_step(scanned, stores)
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
        _ => Ok(ScannedStep::Continue),
    }
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
