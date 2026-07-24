//! Command-core main-control seam used by canonical replay.
//!
//! This intentionally owns only typed execution mutations. Raw delivery,
//! expansion, macro calls, input nesting, and operand collection remain in
//! `tex-command`; no `InputStack` is accepted here.

use tex_command::{
    AlignmentDelivery, AlignmentIdentity, AlignmentRequest, CommandError, CommandHostCapabilities,
    CommandHostContext, CommandProcessor, CommandProfile, CommandRuntime, CommandState,
};
#[cfg(any(test, feature = "instrumentation"))]
use tex_command::{CommandObservation, CommandObserver, MutationRecord, ObservedToken};
use tex_state::env::banks::{IntParam, TokParam};
use tex_state::glue::GlueSpec;
use tex_state::interner::Symbol;
use tex_state::macro_store::{MacroDefinitionProvenance, MacroMeaning};
use tex_state::meaning::{Meaning, MeaningFlags, UnexpandablePrimitive};
use tex_state::scaled::Scaled;
use tex_state::token::Catcode;
#[cfg(any(test, feature = "instrumentation"))]
use tex_state::token::Token;
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
    /// Creates a fresh canonical TeX82 INITEX replay environment.
    ///
    /// The primitive definitions are installed from the engine's static TeX82
    /// registries, before any fixture or host source is registered.
    #[must_use]
    pub fn tex82_initex(stores: &mut Universe) -> Self {
        tex_expand::install_expandable_primitives(stores);
        crate::install_unexpandable_primitives(stores);
        Self {
            command: CommandState::new(CommandProfile::TEX82),
            ..Self::default()
        }
    }

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
                    scan_command(&mut processor, command, false, MeaningFlags::EMPTY, false)?
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
        let starts_paragraph = matches!(
            self.modes.current_mode(),
            Mode::Vertical | Mode::InternalVertical
        );
        let scanned = {
            let mut processor = CommandProcessor::new(
                &mut self.command,
                &mut self.runtime,
                stores.command_context(),
                CommandHostContext::new(&mut self.capabilities),
            );
            scan_step(&mut processor, starts_paragraph)?
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

    /// Delivers and executes one replay command while forwarding committed
    /// command-owned observations in their original order.
    #[cfg(any(test, feature = "instrumentation"))]
    pub fn step_with_observer(
        &mut self,
        stores: &mut Universe,
        observer: &mut dyn CommandObserver,
    ) -> Result<ReplayStep, ExecError> {
        let starts_paragraph = matches!(
            self.modes.current_mode(),
            Mode::Vertical | Mode::InternalVertical
        );
        let scanned = {
            let mut processor = CommandProcessor::new(
                &mut self.command,
                &mut self.runtime,
                stores.command_context(),
                CommandHostContext::new(&mut self.capabilities),
            )
            .with_observer(observer);
            scan_step(&mut processor, starts_paragraph)?
        };
        let mutation = applied_mutation_observation(&scanned, stores);
        let result = apply_scanned_step(
            scanned,
            stores,
            &mut self.modes,
            &mut self.next_alignment_identity,
            &mut self.active_alignment,
            &mut self.command,
        );
        if result.is_ok()
            && let Some(mutation) = mutation
        {
            observer.committed(CommandObservation::Mutation(mutation));
        }
        result
    }

    /// Scans TeX's initial terminal filename through the canonical command
    /// path, retaining every committed observation for the caller.
    #[cfg(any(test, feature = "instrumentation"))]
    pub fn scan_startup_file_name(
        &mut self,
        stores: &mut Universe,
        observer: &mut dyn CommandObserver,
    ) -> Result<String, ExecError> {
        let mut processor = CommandProcessor::new(
            &mut self.command,
            &mut self.runtime,
            stores.command_context(),
            CommandHostContext::new(&mut self.capabilities),
        )
        .with_observer(observer);
        let first =
            processor
                .get_x_token()
                .map_err(command_error)?
                .ok_or(ExecError::MissingToken {
                    context: "terminal filename",
                })?;
        processor.back_input(first).map_err(command_error)?;
        let mut filename = String::new();
        loop {
            let command =
                processor
                    .get_x_token()
                    .map_err(command_error)?
                    .ok_or(ExecError::MissingToken {
                        context: "terminal filename",
                    })?;
            match command.spelling().semantic_token() {
                Token::Char {
                    cat: Catcode::Space,
                    ..
                } => return Ok(filename),
                Token::Char { ch, .. } => filename.push(ch),
                _ => {
                    return Err(ExecError::MissingToken {
                        context: "terminal filename character",
                    });
                }
            }
        }
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
    Dimen {
        index: u16,
        value: Scaled,
        global: bool,
    },
    Skip {
        index: u16,
        value: GlueSpec,
        global: bool,
    },
    Toks {
        index: u16,
        tokens: TracedTokenList,
        global: bool,
    },
    IntParam {
        index: u16,
        value: i32,
        global: bool,
    },
    TokParam {
        index: u16,
        tokens: TracedTokenList,
        global: bool,
    },
    CodeTable {
        primitive: UnexpandablePrimitive,
        character: char,
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
    ParagraphStart,
    Character,
}

fn scan_step(
    processor: &mut CommandProcessor<'_>,
    starts_paragraph: bool,
) -> Result<ScannedStep, ExecError> {
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
    scan_command(processor, command, global, flags, starts_paragraph)
}

fn scan_command(
    processor: &mut CommandProcessor<'_>,
    command: tex_command::CurrentCommand,
    global: bool,
    flags: MeaningFlags,
    starts_paragraph: bool,
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
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Dimen) => {
            let index = processor.scan_integer().map_err(command_error)?.value;
            let _ = processor.scan_optional_equals().map_err(command_error)?;
            let value = processor.scan_dimension().map_err(command_error)?.value;
            let index =
                u16::try_from(index).map_err(|_| ExecError::RegisterNumberOutOfRange(index))?;
            Ok(ScannedStep::Dimen {
                index,
                value,
                global,
            })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Skip) => {
            let index = processor.scan_integer().map_err(command_error)?.value;
            let _ = processor.scan_optional_equals().map_err(command_error)?;
            let value = processor.scan_glue(false).map_err(command_error)?.value;
            let index =
                u16::try_from(index).map_err(|_| ExecError::RegisterNumberOutOfRange(index))?;
            Ok(ScannedStep::Skip {
                index,
                value,
                global,
            })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Toks) => {
            let assignment = processor
                .scan_token_register_assignment()
                .map_err(command_error)?;
            let index = u16::try_from(assignment.index)
                .map_err(|_| ExecError::RegisterNumberOutOfRange(assignment.index))?;
            Ok(ScannedStep::Toks {
                index,
                tokens: assignment.tokens,
                global,
            })
        }
        Meaning::IntParam(index) => {
            let _ = processor.scan_optional_equals().map_err(command_error)?;
            let value = processor.scan_integer().map_err(command_error)?.value;
            Ok(ScannedStep::IntParam {
                index,
                value,
                global,
            })
        }
        Meaning::TokParam(index) => {
            let tokens = processor
                .scan_token_parameter_assignment()
                .map_err(command_error)?;
            Ok(ScannedStep::TokParam {
                index,
                tokens,
                global,
            })
        }
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::CatCode | UnexpandablePrimitive::LcCode),
        ) => {
            let character = processor.scan_integer().map_err(command_error)?.value;
            let _ = processor.scan_optional_equals().map_err(command_error)?;
            let value = processor.scan_integer().map_err(command_error)?.value;
            let character = u32::try_from(character)
                .ok()
                .and_then(char::from_u32)
                .ok_or(ExecError::InvalidCode {
                    context: "code-table character",
                    value: character,
                })?;
            Ok(ScannedStep::CodeTable {
                primitive,
                character,
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
            let target = next_non_space_raw(processor)?
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
        } if starts_paragraph => {
            // TeX82 main_control backs up the first ordinary character before
            // new_graf. The command processor retains exact-delivery proof
            // for that operation; executor replay only applies its typed mode
            // transition after this borrow ends.
            processor.back_input(command).map_err(command_error)?;
            Ok(ScannedStep::ParagraphStart)
        }
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

fn next_non_space_raw(
    processor: &mut CommandProcessor<'_>,
) -> Result<Option<tex_command::CurrentCommand>, ExecError> {
    loop {
        let Some(command) = processor.get_token().map_err(command_error)? else {
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

/// Captures executor-owned mutation data before structural application, then
/// emits it only after that application commits. This preserves the command
/// observer's canonical order without asking `tex-command` to inspect an
/// executor-owned aggregate mutation.
#[cfg(any(test, feature = "instrumentation"))]
fn applied_mutation_observation(
    scanned: &ScannedStep,
    stores: &Universe,
) -> Option<MutationRecord> {
    if let ScannedStep::Count {
        index,
        value,
        global,
    } = scanned
    {
        return Some(MutationRecord {
            target: "register",
            value: format!("count:{index}={value}"),
            key: None,
            tokens: None,
            global: *global,
        });
    }
    if let ScannedStep::Dimen {
        index,
        value,
        global,
    } = scanned
    {
        return Some(MutationRecord {
            target: "register",
            value: format!("scaled:{}", value.raw()),
            key: Some(format!("dimen:{index}")),
            tokens: None,
            global: *global,
        });
    }
    if let ScannedStep::Skip {
        index,
        value,
        global,
    } = scanned
    {
        return Some(MutationRecord {
            target: "register",
            value: format!(
                "glue:width={};stretch={};stretch_order={:?};shrink={};shrink_order={:?}",
                value.width.raw(),
                value.stretch.raw(),
                value.stretch_order,
                value.shrink.raw(),
                value.shrink_order,
            ),
            key: Some(format!("skip:{index}")),
            tokens: None,
            global: *global,
        });
    }
    if let ScannedStep::Toks {
        index,
        tokens,
        global,
    } = scanned
    {
        return Some(MutationRecord {
            target: "register",
            value: "tokens".into(),
            key: Some(format!("toks:{index}")),
            tokens: Some(
                stores
                    .tokens(tokens.token_list())
                    .iter()
                    .copied()
                    .map(|token| observed_macro_token(token, stores))
                    .collect(),
            ),
            global: *global,
        });
    }
    if let ScannedStep::TokParam {
        index,
        tokens,
        global,
    } = scanned
    {
        return Some(MutationRecord {
            target: "parameter",
            value: "tokens".into(),
            key: Some(format!("token_parameter:{index}")),
            tokens: Some(
                stores
                    .tokens(tokens.token_list())
                    .iter()
                    .copied()
                    .map(|token| observed_macro_token(token, stores))
                    .collect(),
            ),
            global: *global,
        });
    }
    if let ScannedStep::IntParam {
        index,
        value,
        global,
    } = scanned
    {
        return Some(MutationRecord {
            target: "parameter",
            value: format!("integer_parameter:{index}={value}"),
            key: None,
            tokens: None,
            global: *global,
        });
    }
    if let ScannedStep::CodeTable {
        primitive,
        character,
        value,
        global,
    } = scanned
    {
        let (target, value) = match primitive {
            UnexpandablePrimitive::CatCode => {
                ("catcode", format!("{}={value}", u32::from(*character)))
            }
            UnexpandablePrimitive::LcCode => (
                "code_table",
                format!("lccode:{}={value}", u32::from(*character)),
            ),
            _ => unreachable!("only code-table primitives are scanned"),
        };
        return Some(MutationRecord {
            target,
            value,
            key: None,
            tokens: None,
            global: *global,
        });
    }
    let ScannedStep::MacroDefinition {
        target,
        parameter_text,
        replacement_text,
        global,
        ..
    } = scanned
    else {
        return None;
    };
    let mut tokens = stores
        .tokens(parameter_text.token_list())
        .iter()
        .copied()
        .map(|token| match token {
            Token::Param(_) => ObservedToken::MacroMatch,
            token => observed_macro_token(token, stores),
        })
        .collect::<Vec<_>>();
    tokens.push(ObservedToken::MacroEndMatch);
    tokens.extend(
        stores
            .tokens(replacement_text.token_list())
            .iter()
            .copied()
            .map(|token| observed_macro_token(token, stores)),
    );
    Some(MutationRecord {
        target: "meaning",
        value: "macro definition".into(),
        key: Some(stores.resolve(*target).to_owned()),
        tokens: Some(tokens),
        global: *global,
    })
}

#[cfg(any(test, feature = "instrumentation"))]
fn observed_macro_token(token: Token, stores: &Universe) -> ObservedToken {
    match token {
        Token::Char { ch, cat } => ObservedToken::Character {
            character: ch,
            catcode: cat,
        },
        Token::Cs(symbol) => ObservedToken::ControlSequence(stores.resolve(symbol).to_owned()),
        Token::Param(slot) => ObservedToken::Parameter(slot),
        Token::Frozen(_) if token.is_frozen_end_template() => ObservedToken::FrozenEndTemplate,
        Token::Frozen(_) if token.is_frozen_endv() => ObservedToken::FrozenEndV,
        Token::Frozen(frozen) => frozen
            .primitive_index()
            .map_or(ObservedToken::FrozenOther, ObservedToken::FrozenPrimitive),
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
        ScannedStep::Dimen {
            index,
            value,
            global,
        } => {
            if global {
                stores.set_dimen_global(index, value);
            } else {
                stores.set_dimen(index, value);
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::Skip {
            index,
            value,
            global,
        } => {
            let value = stores.intern_glue(value);
            if global {
                stores.set_skip_global(index, value);
            } else {
                stores.set_skip(index, value);
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::Toks {
            index,
            tokens,
            global,
        } => {
            if global {
                stores.set_toks_global(index, tokens.token_list());
            } else {
                stores.set_toks(index, tokens.token_list());
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::IntParam {
            index,
            value,
            global,
        } => {
            let parameter = IntParam::new(index);
            if global {
                stores.set_int_param_global(parameter, value);
            } else {
                stores.set_int_param(parameter, value);
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::TokParam {
            index,
            tokens,
            global,
        } => {
            let parameter = TokParam::new(index);
            if global {
                stores.set_tok_param_global(parameter, tokens.token_list());
            } else {
                stores.set_tok_param(parameter, tokens.token_list());
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::CodeTable {
            primitive,
            character,
            value,
            global,
        } => {
            match primitive {
                UnexpandablePrimitive::CatCode => {
                    let value = match value {
                        0 => Catcode::Escape,
                        1 => Catcode::BeginGroup,
                        2 => Catcode::EndGroup,
                        3 => Catcode::MathShift,
                        4 => Catcode::AlignmentTab,
                        5 => Catcode::EndLine,
                        6 => Catcode::Parameter,
                        7 => Catcode::Superscript,
                        8 => Catcode::Subscript,
                        9 => Catcode::Ignored,
                        10 => Catcode::Space,
                        11 => Catcode::Letter,
                        12 => Catcode::Other,
                        13 => Catcode::Active,
                        14 => Catcode::Comment,
                        15 => Catcode::Invalid,
                        _ => {
                            return Err(ExecError::InvalidCode {
                                context: "\\catcode",
                                value,
                            });
                        }
                    };
                    if global {
                        stores.set_catcode_global(character, value);
                    } else {
                        stores.set_catcode(character, value);
                    }
                }
                UnexpandablePrimitive::LcCode => {
                    let value = u32::try_from(value).map_err(|_| ExecError::InvalidCode {
                        context: "\\lccode",
                        value,
                    })?;
                    if global {
                        stores.set_lccode_global(character, value);
                    } else {
                        stores.set_lccode(character, value);
                    }
                }
                _ => unreachable!("only code-table primitives are scanned"),
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
        ScannedStep::ParagraphStart => {
            if matches!(
                modes.current_mode(),
                Mode::Vertical | Mode::InternalVertical
            ) {
                modes.push(Mode::Horizontal);
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
