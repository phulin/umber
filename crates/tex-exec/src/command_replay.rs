//! Command-core main-control seam used by canonical replay.
//!
//! This intentionally owns only typed execution mutations. Raw delivery,
//! expansion, macro calls, input nesting, and operand collection remain in
//! `tex-command`; no `InputStack` is accepted here.

use tex_command::{
    AlignmentCellDelimiter, AlignmentCellOpening, AlignmentCellTemplates, AlignmentDelivery,
    AlignmentIdentity, AlignmentRequest, AlignmentRequestResult, CommandError,
    CommandHostCapabilities, CommandHostContext, CommandProcessor, CommandProfile, CommandRuntime,
    CommandState,
};
#[cfg(any(test, feature = "instrumentation"))]
use tex_command::{
    CommandObservation, CommandObserver, EffectRecord, MutationRecord, ObservedToken,
};
use tex_state::env::banks::{GlueParam, IntParam, TokParam};
use tex_state::glue::GlueSpec;
use tex_state::interner::Symbol;
use tex_state::macro_store::{MacroDefinitionProvenance, MacroMeaning};
use tex_state::meaning::{Meaning, MeaningFlags, UnexpandablePrimitive};
use tex_state::node::{GlueKind, Node};
use tex_state::scaled::Scaled;
use tex_state::token::Catcode;
#[cfg(any(test, feature = "instrumentation"))]
use tex_state::token::Token;
use tex_state::{GroupKind, PrintSink, TracedTokenList, Universe};
use tex_typeset::PackSpec;

use crate::{ExecError, Mode, ModeNest};

/// Replay-only command main control with command-owned source consumption.
#[derive(Debug, Default)]
pub struct CommandReplayControl {
    command: CommandState,
    runtime: CommandRuntime,
    capabilities: CommandHostCapabilities,
    modes: ModeNest,
    next_alignment_identity: u64,
    active_alignment: Option<ActiveReplayAlignment>,
    boxes: ReplayBoxes,
}

#[derive(Clone, Copy, Debug)]
struct SetBoxTarget {
    index: u16,
    global: bool,
}

#[derive(Clone, Copy, Debug)]
struct ActiveReplayBox {
    target: Option<SetBoxTarget>,
    opening_brace_replay: bool,
    body_opener_pending: bool,
    depth: u32,
}

#[derive(Clone, Debug)]
struct ActiveReplayAlignment {
    identity: AlignmentIdentity,
    columns: Vec<AlignmentCellTemplates>,
    repeat_start: Option<usize>,
    column: usize,
    preamble_opening_pending: bool,
    preamble_opening_replay_pending: bool,
    preamble_start_pending: bool,
    cell_opening_pending: bool,
    next_cell_opening_pending: bool,
    align_peek_pending: bool,
    align_peek_after_noalign: bool,
    noalign_depth: Option<u32>,
}

#[derive(Debug, Default)]
struct ReplayBoxes {
    pending_setbox: Option<SetBoxTarget>,
    active_boxes: Vec<ActiveReplayBox>,
    suspended_alignments: Vec<ActiveReplayAlignment>,
    recovery_simple_group_pending: bool,
    recovery_simple_group_open: bool,
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
            next_alignment_identity: 1,
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
            .as_ref()
            .map(|alignment| alignment.identity)
    }

    /// Applies an executor-selected alignment lifecycle transition.
    ///
    /// The request contains no token spelling, so this cannot create another
    /// delimiter-classification or source-consumption path.
    pub fn apply_alignment_request(&mut self, request: AlignmentRequest) -> Result<(), ExecError> {
        let finished = matches!(request, AlignmentRequest::Finish(_));
        let preamble = matches!(request, AlignmentRequest::Preamble(_));
        let identity = match request {
            AlignmentRequest::Begin(identity)
            | AlignmentRequest::Preamble(identity)
            | AlignmentRequest::PrepareCellLookahead(identity)
            | AlignmentRequest::InstallCellTemplate(identity)
            | AlignmentRequest::InstallOmitCellTemplate(identity)
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
        if finished
            && self.active_alignment.as_ref().map(|active| active.identity) == Some(identity)
        {
            self.active_alignment = None;
            if let Some(outer) = self.boxes.suspended_alignments.pop() {
                self.command
                    .apply_alignment_request(AlignmentRequest::Resume(outer.identity))
                    .map_err(|_| ExecError::MissingToken {
                        context: "nested alignment resumption",
                    })?;
                self.active_alignment = Some(outer);
            }
        }
        if preamble
            && let Some(active) = self.active_alignment.as_mut()
            && active.identity == identity
        {
            active.preamble_opening_pending = false;
            active.preamble_opening_replay_pending = false;
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
        let innermost_group = stores.innermost_group_kind();
        let scanned = {
            let mut processor = CommandProcessor::new(
                &mut self.command,
                &mut self.runtime,
                stores.command_context(),
                CommandHostContext::new(&mut self.capabilities),
            );
            scan_alignment_delivery_step(
                &mut processor,
                alignment,
                &ReplayBoxes::default(),
                innermost_group,
            )?
        };
        apply_scanned_step(
            scanned,
            stores,
            &mut self.modes,
            &mut self.next_alignment_identity,
            &mut self.active_alignment,
            &mut self.command,
            &mut self.boxes,
        )
    }

    /// Delivers and executes one replay command through the command processor.
    pub fn step(&mut self, stores: &mut Universe) -> Result<ReplayStep, ExecError> {
        let starts_paragraph = matches!(
            self.modes.current_mode(),
            Mode::Vertical | Mode::InternalVertical
        );
        let alignment_preamble = alignment_preamble(self.active_alignment.as_mut());
        let innermost_group = stores.innermost_group_kind();
        let scanned = {
            let mut processor = CommandProcessor::new(
                &mut self.command,
                &mut self.runtime,
                stores.command_context(),
                CommandHostContext::new(&mut self.capabilities),
            );
            scan_replay_step(
                &mut processor,
                starts_paragraph,
                &self.boxes,
                alignment_preamble,
                innermost_group,
            )?
        };
        apply_scanned_step(
            scanned,
            stores,
            &mut self.modes,
            &mut self.next_alignment_identity,
            &mut self.active_alignment,
            &mut self.command,
            &mut self.boxes,
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
        let alignment_preamble = alignment_preamble(self.active_alignment.as_mut());
        let innermost_group = stores.innermost_group_kind();
        let scanned = {
            let mut processor = CommandProcessor::new(
                &mut self.command,
                &mut self.runtime,
                stores.command_context(),
                CommandHostContext::new(&mut self.capabilities),
            )
            .with_observer(observer);
            scan_replay_step(
                &mut processor,
                starts_paragraph,
                &self.boxes,
                alignment_preamble,
                innermost_group,
            )?
        };
        let mutation = applied_mutation_observation(&scanned, stores);
        let effect = applied_effect_observation(&scanned, stores);
        let begins_alignment = matches!(&scanned, ScannedStep::BeginAlignment { .. });
        let suspends_alignment = begins_alignment && self.active_alignment.is_some();
        let begins_alignment_cell = matches!(&scanned, ScannedStep::AlignmentPreambleStart { .. });
        let installs_u_template = match &scanned {
            ScannedStep::AlignmentCellOpening {
                alignment,
                opening: AlignmentCellOpening::Template,
            } => Some(*alignment),
            // `align_peek` already fetched and backed up the first nonblank
            // command before it calls TeX82's `init_col`.
            ScannedStep::AlignmentPeekCell {
                alignment,
                omit: false,
            } => Some(*alignment),
            _ => None,
        };
        let installs_omit_cell = match &scanned {
            ScannedStep::AlignmentCellOpening {
                alignment,
                opening: AlignmentCellOpening::Omit,
            } => Some(*alignment),
            ScannedStep::AlignmentPeekCell {
                alignment,
                omit: true,
            } => Some(*alignment),
            _ => None,
        };
        let finishes_alignment_cell = match &scanned {
            ScannedStep::AlignmentCellFinish { alignment } => {
                self.command.alignment_cell_finish_observations(*alignment)
            }
            _ => None,
        };
        let finishes_alignment = match &scanned {
            ScannedStep::AlignmentFinish { alignment } => {
                self.command.alignment_finish_observation(*alignment)
            }
            _ => None,
        };
        let result = apply_scanned_step(
            scanned,
            stores,
            &mut self.modes,
            &mut self.next_alignment_identity,
            &mut self.active_alignment,
            &mut self.command,
            &mut self.boxes,
        );
        if result.is_ok() {
            if suspends_alignment
                && let Some(alignment) = self.command.alignment_suspend_observation()
            {
                observer.committed(CommandObservation::Alignment(alignment));
            }
            if begins_alignment && let Some(alignment) = self.command.alignment_begin_observation()
            {
                observer.committed(CommandObservation::Alignment(alignment));
            }
            if begins_alignment_cell
                && let Some(alignment) = self.command.alignment_cell_begin_observation()
            {
                observer.committed(CommandObservation::Alignment(alignment));
            }
            if let Some(alignment) = installs_u_template
                && let Some(input) = self
                    .command
                    .alignment_u_template_push_observation(alignment)
            {
                observer.committed(CommandObservation::Input(input));
                if let Some(template) = self
                    .command
                    .alignment_u_template_push_alignment_observation(alignment)
                {
                    observer.committed(CommandObservation::Alignment(template));
                }
            }
            if let Some(alignment) = installs_omit_cell
                && let Some(omit) = self.command.alignment_omit_cell_observation(alignment)
            {
                observer.committed(CommandObservation::Alignment(omit));
            }
            if let Some(finish) = finishes_alignment_cell {
                observer.committed(CommandObservation::Alignment(finish.state_change));
                if let Some(retirement) = finish.backed_up_endv_retirement {
                    observer.committed(CommandObservation::Input(retirement));
                }
                observer.committed(CommandObservation::Input(finish.v_template_retirement));
                observer.committed(CommandObservation::Alignment(finish.template_retirement));
            }
            if let Some(finish) = finishes_alignment {
                observer.committed(CommandObservation::Alignment(finish));
                if let Some(resume) = self.command.alignment_resume_observation() {
                    observer.committed(CommandObservation::Alignment(resume));
                }
            }
            if let Some(mutation) = mutation {
                observer.committed(CommandObservation::Mutation(mutation));
            }
            if let Some(effect) = effect {
                observer.committed(CommandObservation::Effect(effect));
            }
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
    HorizontalSkip {
        value: GlueSpec,
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
    GlueParam {
        index: u16,
        value: GlueSpec,
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
    Let {
        target: Symbol,
        source: Option<Symbol>,
        meaning: Meaning,
        global: bool,
    },
    Rule {
        width: Option<Scaled>,
        height: Option<Scaled>,
        depth: Option<Scaled>,
        horizontal: bool,
    },
    Message {
        tokens: TracedTokenList,
    },
    BeginAlignment {
        vertical: bool,
    },
    AlignmentPreambleOpening {
        alignment: AlignmentIdentity,
    },
    AlignmentPreambleOpeningReplay {
        alignment: AlignmentIdentity,
    },
    AlignmentPreambleStart {
        alignment: AlignmentIdentity,
    },
    AlignmentCellOpening {
        alignment: AlignmentIdentity,
        opening: AlignmentCellOpening,
    },
    /// TeX82's `do_endv` completed a command-owned v-template.  Applying
    /// this result retires that exact frame before the backed-up delimiter
    /// resumes through `get_next`.
    AlignmentCellFinish {
        alignment: AlignmentIdentity,
    },
    /// TeX82 §37 delivered the alignment-closing right brace, so `fin_align`
    /// must complete before the outer suspended delivery context resumes.
    AlignmentFinish {
        alignment: AlignmentIdentity,
    },
    /// TeX82 §37 has consumed `\\noalign` and its compulsory opening brace.
    /// Command control owns both deliveries; the executor now owns the
    /// no-align group's structural entry.
    BeginNoAlign {
        alignment: AlignmentIdentity,
    },
    AlignmentRecovery {
        opens_simple_group: bool,
    },
    BeginSimpleGroup,
    EndSimpleGroup,
    AlignmentPeekCell {
        alignment: AlignmentIdentity,
        omit: bool,
    },
    NoAlignBeginGroup {
        alignment: AlignmentIdentity,
    },
    NoAlignEndGroup {
        alignment: AlignmentIdentity,
    },
    SetBox(SetBoxTarget),
    BeginVBox,
    ReplayBoxOpeningBrace,
    BoxBeginGroup,
    BoxEndGroup,
    Paragraph,
    MathShift,
    ParagraphStart,
    Character,
}

/// Selects the one command-owned scanner that may consume input before
/// ordinary main control.  Alignment preamble setup validates and backs up
/// its opening brace twice through successive command-owned backup levels;
/// only the second replay reaches TeX82's live preamble scanner.
fn scan_replay_step(
    processor: &mut CommandProcessor<'_>,
    starts_paragraph: bool,
    boxes: &ReplayBoxes,
    alignment_preamble: Option<(AlignmentIdentity, AlignmentPreamblePhase)>,
    innermost_group: Option<GroupKind>,
) -> Result<ScannedStep, ExecError> {
    if let Some((alignment, phase)) = alignment_preamble {
        return match phase {
            AlignmentPreamblePhase::Opening => {
                processor
                    .scan_alignment_preamble_opening()
                    .map_err(command_error)?;
                Ok(ScannedStep::AlignmentPreambleOpening { alignment })
            }
            AlignmentPreamblePhase::ReplayOpening => {
                processor
                    .replay_alignment_preamble_opening()
                    .map_err(command_error)?;
                Ok(ScannedStep::AlignmentPreambleOpeningReplay { alignment })
            }
            AlignmentPreamblePhase::Start => {
                processor
                    .begin_alignment_preamble_scan()
                    .map_err(command_error)?;
                Ok(ScannedStep::AlignmentPreambleStart { alignment })
            }
            AlignmentPreamblePhase::CellOpening => {
                let opening = processor
                    .scan_alignment_cell_opening()
                    .map_err(command_error)?;
                Ok(ScannedStep::AlignmentCellOpening { alignment, opening })
            }
            AlignmentPreamblePhase::NextCellOpening => {
                let opening = processor
                    .scan_alignment_next_cell_opening()
                    .map_err(command_error)?;
                Ok(ScannedStep::AlignmentCellOpening { alignment, opening })
            }
            AlignmentPreamblePhase::AlignPeek { after_noalign } => {
                scan_alignment_peek(processor, alignment, after_noalign)
            }
            AlignmentPreamblePhase::NoAlignBody => scan_noalign_body(processor, alignment, boxes),
            AlignmentPreamblePhase::CellDelivery => {
                scan_alignment_delivery_step(processor, alignment, boxes, innermost_group)
            }
        };
    }
    scan_step(processor, starts_paragraph, boxes)
}

#[derive(Clone, Copy)]
enum AlignmentPreamblePhase {
    Opening,
    ReplayOpening,
    Start,
    CellOpening,
    NextCellOpening,
    AlignPeek { after_noalign: bool },
    NoAlignBody,
    CellDelivery,
}

fn alignment_preamble(
    active: Option<&mut ActiveReplayAlignment>,
) -> Option<(AlignmentIdentity, AlignmentPreamblePhase)> {
    let active = active?;
    if active.preamble_opening_pending {
        Some((active.identity, AlignmentPreamblePhase::Opening))
    } else if active.preamble_opening_replay_pending {
        Some((active.identity, AlignmentPreamblePhase::ReplayOpening))
    } else if active.preamble_start_pending {
        Some((active.identity, AlignmentPreamblePhase::Start))
    } else if active.cell_opening_pending {
        Some((active.identity, AlignmentPreamblePhase::CellOpening))
    } else if active.next_cell_opening_pending {
        Some((active.identity, AlignmentPreamblePhase::NextCellOpening))
    } else if active.align_peek_pending {
        let after_noalign = active.align_peek_after_noalign;
        active.align_peek_after_noalign = false;
        Some((
            active.identity,
            AlignmentPreamblePhase::AlignPeek { after_noalign },
        ))
    } else if active.noalign_depth.is_some() {
        Some((active.identity, AlignmentPreamblePhase::NoAlignBody))
    } else {
        Some((active.identity, AlignmentPreamblePhase::CellDelivery))
    }
}

/// TeX82 §37's post-row lookahead.  This is deliberately separate from
/// `init_col`: `\\noalign` consumes its opening brace directly, whereas an
/// ordinary next-cell command is backed up for template installation.
fn scan_alignment_peek(
    processor: &mut CommandProcessor<'_>,
    alignment: AlignmentIdentity,
    after_noalign: bool,
) -> Result<ScannedStep, ExecError> {
    processor
        .begin_alignment_peek(after_noalign)
        .map_err(command_error)?;
    let command = next_non_space(processor)?.ok_or(ExecError::MissingToken {
        context: "alignment lookahead",
    })?;
    match command.meaning() {
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::NoAlign) => {
            processor
                .scan_alignment_noalign_opening()
                .map_err(command_error)?;
            Ok(ScannedStep::BeginNoAlign { alignment })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::CrCr) => Ok(ScannedStep::Continue),
        Meaning::CharToken {
            cat: Catcode::EndGroup,
            ..
        } => Ok(ScannedStep::AlignmentFinish { alignment }),
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Omit) => {
            Ok(ScannedStep::AlignmentPeekCell {
                alignment,
                omit: true,
            })
        }
        _ => {
            processor.back_input(command).map_err(command_error)?;
            Ok(ScannedStep::AlignmentPeekCell {
                alignment,
                omit: false,
            })
        }
    }
}

fn scan_noalign_body(
    processor: &mut CommandProcessor<'_>,
    alignment: AlignmentIdentity,
    boxes: &ReplayBoxes,
) -> Result<ScannedStep, ExecError> {
    let Some(command) = processor.get_x_token().map_err(command_error)? else {
        return Ok(ScannedStep::EndOfInput);
    };
    match command.meaning() {
        Meaning::CharToken {
            cat: Catcode::BeginGroup,
            ..
        } => Ok(ScannedStep::NoAlignBeginGroup { alignment }),
        Meaning::CharToken {
            cat: Catcode::EndGroup,
            ..
        } => Ok(ScannedStep::NoAlignEndGroup { alignment }),
        _ => scan_command(processor, command, false, MeaningFlags::EMPTY, false, boxes),
    }
}

/// Delivers one active cell command through the command-owned alignment
/// boundary.  This remains separate from preamble and opener scans because a
/// completed scanner (such as a rule specification) can leave a backed-up
/// delimiter ready for the next main-control step.
fn scan_alignment_delivery_step(
    processor: &mut CommandProcessor<'_>,
    alignment: AlignmentIdentity,
    boxes: &ReplayBoxes,
    innermost_group: Option<GroupKind>,
) -> Result<ScannedStep, ExecError> {
    match processor
        .get_x_alignment_delivery()
        .map_err(command_error)?
    {
        None => Ok(ScannedStep::EndOfInput),
        Some(AlignmentDelivery::Command(command)) => {
            if matches!(command.meaning(), Meaning::EndV) {
                // Replay's structural alignment group is deliberately not a
                // Universe group: the surrounding box owns that stack slot.
                // A recovery-opened simple group is the bounded exception
                // that TeX82 §1131 must close through `off_save` first.
                if boxes.recovery_simple_group_open {
                    let closer = match innermost_group {
                        Some(GroupKind::MathShift) => tex_state::token::Token::Char {
                            ch: '$',
                            cat: Catcode::MathShift,
                        },
                        Some(GroupKind::SemiSimple) => {
                            return Err(ExecError::MissingToken {
                                context: "endgroup off_save replay",
                            });
                        }
                        Some(_) => tex_state::token::Token::Char {
                            ch: '}',
                            cat: Catcode::EndGroup,
                        },
                        None => return Ok(ScannedStep::Continue),
                    };
                    processor
                        .recover_endv_off_save(command, closer)
                        .map_err(command_error)?;
                    return Ok(ScannedStep::Continue);
                }
                return Ok(ScannedStep::AlignmentCellFinish { alignment });
            }
            scan_command(processor, command, false, MeaningFlags::EMPTY, false, boxes)
        }
        Some(AlignmentDelivery::Event(event)) => {
            match event {
                tex_command::AlignmentDeliveryEvent::EndTemplate(_) => processor
                    .begin_alignment_v_template(alignment, event)
                    .map_err(command_error)?,
                tex_command::AlignmentDeliveryEvent::UnbalancedDelimiter(_) => {
                    let recovery = processor
                        .recover_alignment_unbalanced_delimiter(event)
                        .map_err(command_error)?;
                    return Ok(ScannedStep::AlignmentRecovery {
                        opens_simple_group: matches!(
                            recovery,
                            tex_state::token::Token::Char {
                                cat: Catcode::BeginGroup,
                                ..
                            }
                        ),
                    });
                }
                tex_command::AlignmentDeliveryEvent::ClosingBrace(_) => {
                    // TeX82 §1103 selects this executor-owned align_group
                    // branch. Raw brace backup/correction and frozen-\cr
                    // insertion remain entirely command-owned.
                    processor
                        .recover_alignment_closing_brace(event)
                        .map_err(command_error)?;
                }
            }
            Ok(ScannedStep::Continue)
        }
    }
}

fn scan_step(
    processor: &mut CommandProcessor<'_>,
    starts_paragraph: bool,
    boxes: &ReplayBoxes,
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
    scan_command(processor, command, global, flags, starts_paragraph, boxes)
}

fn scan_command(
    processor: &mut CommandProcessor<'_>,
    command: tex_command::CurrentCommand,
    global: bool,
    flags: MeaningFlags,
    starts_paragraph: bool,
    boxes: &ReplayBoxes,
) -> Result<ScannedStep, ExecError> {
    // `align_error`'s inserted brace is an actual execution group, even when
    // it appears inside a replayed box body.  It must therefore win over the
    // box body's brace-depth bookkeeping so §1131 can observe it at end-v.
    if boxes.recovery_simple_group_pending
        && matches!(
            command.meaning(),
            Meaning::CharToken {
                cat: Catcode::BeginGroup,
                ..
            }
        )
    {
        return Ok(ScannedStep::BeginSimpleGroup);
    }
    if boxes.recovery_simple_group_open
        && matches!(
            command.meaning(),
            Meaning::CharToken {
                cat: Catcode::EndGroup,
                ..
            }
        )
    {
        return Ok(ScannedStep::EndSimpleGroup);
    }
    if boxes
        .active_boxes
        .last()
        .is_some_and(|box_state| box_state.opening_brace_replay)
        && matches!(
            command.meaning(),
            Meaning::CharToken {
                cat: Catcode::BeginGroup,
                ..
            }
        )
    {
        processor
            .back_input_after_backup_replay(command)
            .map_err(command_error)?;
        return Ok(ScannedStep::ReplayBoxOpeningBrace);
    }
    if boxes
        .active_boxes
        .last()
        .is_some_and(|box_state| !box_state.opening_brace_replay)
    {
        match command.meaning() {
            Meaning::CharToken {
                cat: Catcode::BeginGroup,
                ..
            } => return Ok(ScannedStep::BoxBeginGroup),
            Meaning::CharToken {
                cat: Catcode::EndGroup,
                ..
            } => return Ok(ScannedStep::BoxEndGroup),
            _ => {}
        }
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
        // TeX82 §458 leaves `scan_glue` entirely in the command machine.
        // Main control receives only its completed typed specification, so a
        // u-template's numeric operand retains the canonical `back_input`
        // and replay sequence before this layer appends the glue node.
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::HSkip) => {
            let value = processor.scan_glue(false).map_err(command_error)?.value;
            Ok(ScannedStep::HorizontalSkip { value })
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
        Meaning::GlueParam(index) => {
            let assignment = processor
                .scan_glue_parameter_assignment(index, false)
                .map_err(command_error)?;
            Ok(ScannedStep::GlueParam {
                index: assignment.index,
                value: assignment.value,
                global,
            })
        }
        Meaning::MuGlueParam(index) => {
            let assignment = processor
                .scan_glue_parameter_assignment(index, true)
                .map_err(command_error)?;
            Ok(ScannedStep::GlueParam {
                index: assignment.index,
                value: assignment.value,
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
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Let) => {
            let assignment = processor
                .scan_let_assignment(false)
                .map_err(command_error)?;
            Ok(ScannedStep::Let {
                target: assignment.target,
                source: assignment.source,
                meaning: assignment.meaning,
                global,
            })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::FutureLet) => {
            let assignment = processor.scan_let_assignment(true).map_err(command_error)?;
            Ok(ScannedStep::Let {
                target: assignment.target,
                source: assignment.source,
                meaning: assignment.meaning,
                global,
            })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Message) => {
            let tokens = processor.scan_balanced_text(true).map_err(command_error)?;
            Ok(ScannedStep::Message {
                tokens: tokens.tokens,
            })
        }
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::HRule | UnexpandablePrimitive::VRule),
        ) => {
            let spec = processor.scan_rule_spec(primitive).map_err(command_error)?;
            Ok(ScannedStep::Rule {
                width: spec.width,
                height: spec.height,
                depth: spec.depth,
                horizontal: primitive == UnexpandablePrimitive::HRule,
            })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::SetBox) => {
            let assignment = processor.scan_setbox_assignment().map_err(command_error)?;
            let index = u16::try_from(assignment.index)
                .map_err(|_| ExecError::RegisterNumberOutOfRange(assignment.index))?;
            Ok(ScannedStep::SetBox(SetBoxTarget { index, global }))
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::VBox) => {
            processor.scan_box_group_opening().map_err(command_error)?;
            Ok(ScannedStep::BeginVBox)
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
    if let ScannedStep::GlueParam {
        index,
        value,
        global,
    } = scanned
    {
        return Some(MutationRecord {
            target: "parameter",
            value: format!(
                "glue:width={};stretch={};stretch_order={:?};shrink={};shrink_order={:?}",
                value.width.raw(),
                value.stretch.raw(),
                value.stretch_order,
                value.shrink.raw(),
                value.shrink_order,
            ),
            key: Some(format!("glue_parameter:{index}")),
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
    if let ScannedStep::Let {
        target,
        source,
        meaning,
        global,
    } = scanned
    {
        return Some(MutationRecord {
            target: "meaning",
            value: let_mutation_value(*meaning, *source, stores),
            key: Some(stores.resolve(*target).to_owned()),
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

/// Captures an executor-owned observable effect before application, then
/// emits it only after that application commits through the replay seam.
#[cfg(any(test, feature = "instrumentation"))]
fn applied_effect_observation(scanned: &ScannedStep, stores: &Universe) -> Option<EffectRecord> {
    let ScannedStep::Message { tokens } = scanned else {
        return None;
    };
    Some(EffectRecord {
        kind: "message",
        detail: replay_text(stores.tokens(tokens.token_list())),
    })
}

#[cfg(any(test, feature = "instrumentation"))]
fn let_mutation_value(meaning: Meaning, source: Option<Symbol>, stores: &Universe) -> String {
    match meaning {
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::BeginGroup) => "begin_group".into(),
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::EndGroup) => "end_group".into(),
        _ => source.map_or_else(
            || format!("{meaning:?}"),
            |source| stores.resolve(source).to_owned(),
        ),
    }
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

/// Applies TeX82 `fin_col`'s saved-delimiter selection after `do_endv`.
///
/// The delimiter was classified and retained by `tex-command` at the original
/// `get_next` boundary.  This code receives only its typed outcome, chooses
/// the next frozen template pair, and lets command-owned lookahead/back-up
/// prepare the next entry.
fn begin_next_replay_alignment_cell(
    alignment: AlignmentIdentity,
    delimiter: AlignmentCellDelimiter,
    command: &mut CommandState,
    active_alignment: &mut Option<ActiveReplayAlignment>,
) -> Result<(), ExecError> {
    let active = active_alignment
        .as_mut()
        .filter(|active| active.identity == alignment)
        .ok_or(ExecError::MissingToken {
            context: "active replay alignment",
        })?;
    // Focused lifecycle tests may construct a command-state cell directly,
    // without replaying a preamble.  There is then no executor template
    // selection to perform after the otherwise complete command transition.
    if active.columns.is_empty() {
        return Ok(());
    }
    let next_column = match delimiter {
        AlignmentCellDelimiter::Tab | AlignmentCellDelimiter::Span => active
            .column
            .checked_add(1)
            .ok_or(ExecError::ArithmeticOverflow)?,
        AlignmentCellDelimiter::Row => 0,
    };
    active.column = if next_column < active.columns.len() {
        next_column
    } else if let Some(repeat_start) = active.repeat_start {
        let repeat_len =
            active
                .columns
                .len()
                .checked_sub(repeat_start)
                .ok_or(ExecError::MissingToken {
                    context: "alignment periodic-preamble boundary",
                })?;
        if repeat_len == 0 {
            return Err(ExecError::MissingToken {
                context: "alignment periodic-preamble columns",
            });
        }
        repeat_start + (next_column - repeat_start) % repeat_len
    } else {
        next_column
    };
    let templates = active
        .columns
        .get(active.column)
        .copied()
        .ok_or(ExecError::MissingToken {
            context: "next alignment preamble column",
        })?;
    match delimiter {
        AlignmentCellDelimiter::Row => active.align_peek_pending = true,
        AlignmentCellDelimiter::Tab | AlignmentCellDelimiter::Span => {
            command
                .apply_alignment_request(AlignmentRequest::BeginCell {
                    alignment,
                    templates,
                })
                .map_err(|_| ExecError::MissingToken {
                    context: "alignment next-cell lifecycle",
                })?;
            active.next_cell_opening_pending = true;
        }
    }
    Ok(())
}

fn apply_scanned_step(
    scanned: ScannedStep,
    stores: &mut Universe,
    modes: &mut ModeNest,
    next_alignment_identity: &mut u64,
    active_alignment: &mut Option<ActiveReplayAlignment>,
    command: &mut CommandState,
    boxes: &mut ReplayBoxes,
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
        ScannedStep::HorizontalSkip { value } => {
            modes.current_list_mut().push(Node::Glue {
                spec: stores.intern_glue(value),
                kind: GlueKind::Normal,
                leader: None,
            });
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
        ScannedStep::GlueParam {
            index,
            value,
            global,
        } => {
            let parameter = GlueParam::new(index);
            let value = stores.intern_glue(value);
            if global {
                stores.set_glue_param_global(parameter, value);
            } else {
                stores.set_glue_param(parameter, value);
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
        ScannedStep::Let {
            target,
            source,
            meaning,
            global,
        } => {
            let _ = source;
            if global {
                stores.set_meaning_global(target, meaning);
            } else {
                stores.set_meaning(target, meaning);
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::Rule {
            width,
            height,
            depth,
            horizontal,
        } => {
            if horizontal
                && matches!(
                    modes.current_mode(),
                    Mode::Horizontal | Mode::RestrictedHorizontal
                )
            {
                let _ = modes.pop()?;
            }
            modes.current_list_mut().push(Node::Rule {
                width,
                height,
                depth,
            });
            Ok(ReplayStep::Continue)
        }
        ScannedStep::Message { tokens } => {
            let text = replay_text(stores.tokens(tokens.token_list()));
            stores
                .world_mut()
                .write_text(PrintSink::TerminalAndLog, &text);
            Ok(ReplayStep::Continue)
        }
        ScannedStep::SetBox(target) => {
            boxes.pending_setbox = Some(target);
            Ok(ReplayStep::Continue)
        }
        ScannedStep::BeginVBox => {
            let target = boxes.pending_setbox.take();
            stores.enter_group_with_kind(GroupKind::VBox);
            modes.push(Mode::InternalVertical);
            boxes.active_boxes.push(ActiveReplayBox {
                target,
                opening_brace_replay: true,
                body_opener_pending: true,
                depth: 1,
            });
            Ok(ReplayStep::Continue)
        }
        ScannedStep::BeginSimpleGroup => {
            stores.enter_group_with_kind(GroupKind::Simple);
            boxes.recovery_simple_group_pending = false;
            boxes.recovery_simple_group_open = true;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::EndSimpleGroup => {
            stores
                .leave_group_with_kind(GroupKind::Simple)
                .map_err(|_| ExecError::MissingToken {
                    context: "simple recovery group",
                })?;
            boxes.recovery_simple_group_open = false;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::AlignmentRecovery { opens_simple_group } => {
            boxes.recovery_simple_group_pending = opens_simple_group;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::ReplayBoxOpeningBrace => {
            let box_state = boxes
                .active_boxes
                .last_mut()
                .ok_or(ExecError::MissingToken {
                    context: "box opening brace",
                })?;
            box_state.opening_brace_replay = false;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::BoxBeginGroup => {
            let box_state = boxes
                .active_boxes
                .last_mut()
                .ok_or(ExecError::MissingToken {
                    context: "box group",
                })?;
            if box_state.body_opener_pending {
                // The first replayed brace is the box body's required opener;
                // its scope is represented by the VBox group entered above.
                box_state.body_opener_pending = false;
            } else {
                box_state.depth = box_state.depth.saturating_add(1);
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::BoxEndGroup => {
            let box_state = boxes
                .active_boxes
                .last_mut()
                .ok_or(ExecError::MissingToken {
                    context: "box group",
                })?;
            if box_state.depth > 1 {
                box_state.depth -= 1;
                return Ok(ReplayStep::Continue);
            }
            let box_state = boxes.active_boxes.pop().expect("active box was checked");
            let _ = modes.pop()?;
            let children = stores.freeze_node_list(&[]);
            let packed = crate::packing_params::vpack(
                stores,
                children,
                PackSpec::Natural,
                crate::packing_params::vpack_params(stores),
            );
            let boxed = stores.freeze_node_list(&[Node::VList(packed.node)]);
            stores
                .leave_group_with_kind(GroupKind::VBox)
                .map_err(|_| ExecError::MissingToken {
                    context: "vbox group",
                })?;
            if let Some(target) = box_state.target {
                if target.global {
                    stores.set_box_reg_global(target.index, boxed);
                } else {
                    stores.set_box_reg(target.index, boxed);
                }
            } else {
                modes.current_list_mut().push(Node::VList(packed.node));
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::BeginAlignment { vertical } => {
            if let Some(outer) = active_alignment.take() {
                command
                    .apply_alignment_request(AlignmentRequest::Suspend(outer.identity))
                    .map_err(|_| ExecError::MissingToken {
                        context: "nested alignment suspension",
                    })?;
                boxes.suspended_alignments.push(outer);
            }
            let identity = AlignmentIdentity::new(*next_alignment_identity);
            *next_alignment_identity = next_alignment_identity.wrapping_add(1);
            command
                .apply_alignment_request(AlignmentRequest::Begin(identity))
                .map_err(|_| ExecError::MissingToken {
                    context: "alignment lifecycle",
                })?;
            *active_alignment = Some(ActiveReplayAlignment {
                identity,
                columns: Vec::new(),
                repeat_start: None,
                column: 0,
                preamble_opening_pending: true,
                preamble_opening_replay_pending: false,
                preamble_start_pending: false,
                cell_opening_pending: false,
                next_cell_opening_pending: false,
                align_peek_pending: false,
                align_peek_after_noalign: false,
                noalign_depth: None,
            });
            if vertical && modes.current_mode() == Mode::Vertical {
                modes.push(Mode::InternalVertical);
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::AlignmentPreambleOpening { alignment } => {
            command
                .apply_alignment_request(AlignmentRequest::Preamble(alignment))
                .map_err(|_| ExecError::MissingToken {
                    context: "alignment preamble lifecycle",
                })?;
            if let Some(active) = active_alignment.as_mut()
                && active.identity == alignment
            {
                active.preamble_opening_pending = false;
                active.preamble_opening_replay_pending = true;
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::AlignmentPreambleOpeningReplay { alignment } => {
            if let Some(active) = active_alignment.as_mut()
                && active.identity == alignment
            {
                active.preamble_opening_replay_pending = false;
                active.preamble_start_pending = true;
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::AlignmentPreambleStart { alignment } => {
            let preamble = command
                .take_completed_alignment_preamble(alignment)
                .map_err(|_| ExecError::MissingToken {
                    context: "completed alignment preamble",
                })?;
            let templates = preamble
                .columns
                .first()
                .copied()
                .ok_or(ExecError::MissingToken {
                    context: "first alignment preamble column",
                })?;
            command
                .apply_alignment_request(AlignmentRequest::BeginCell {
                    alignment,
                    templates,
                })
                .map_err(|_| ExecError::MissingToken {
                    context: "alignment first-cell lifecycle",
                })?;
            if let Some(active) = active_alignment.as_mut()
                && active.identity == alignment
            {
                active.columns = preamble.columns;
                active.repeat_start = preamble.repeat_start;
                active.column = 0;
                active.preamble_start_pending = false;
                active.cell_opening_pending = true;
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::BeginNoAlign { alignment } => {
            let active = active_alignment
                .as_mut()
                .filter(|active| active.identity == alignment)
                .ok_or(ExecError::MissingToken {
                    context: "active replay alignment",
                })?;
            active.align_peek_pending = false;
            active.noalign_depth = Some(1);
            stores.enter_group_with_kind(GroupKind::NoAlign);
            if matches!(
                modes.current_mode(),
                Mode::Horizontal | Mode::RestrictedHorizontal
            ) {
                let _ = modes.pop()?;
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::AlignmentPeekCell { alignment, omit } => {
            let active = active_alignment
                .as_mut()
                .filter(|active| active.identity == alignment)
                .ok_or(ExecError::MissingToken {
                    context: "active replay alignment",
                })?;
            let templates =
                active
                    .columns
                    .get(active.column)
                    .copied()
                    .ok_or(ExecError::MissingToken {
                        context: "next alignment preamble column",
                    })?;
            command
                .apply_alignment_request(AlignmentRequest::BeginCell {
                    alignment,
                    templates,
                })
                .map_err(|_| ExecError::MissingToken {
                    context: "alignment next-row lifecycle",
                })?;
            active.align_peek_pending = false;
            if omit {
                command
                    .apply_alignment_request(AlignmentRequest::PrepareCellLookahead(alignment))
                    .map_err(|_| ExecError::MissingToken {
                        context: "alignment omit lookahead lifecycle",
                    })?;
                command
                    .apply_alignment_request(AlignmentRequest::InstallOmitCellTemplate(alignment))
                    .map_err(|_| ExecError::MissingToken {
                        context: "alignment omit-cell lifecycle",
                    })?;
            } else {
                // TeX82 §37 now calls `init_col`, which immediately pushes
                // the selected u-template above the command backed up by
                // `align_peek`. A second lookahead would re-deliver that
                // command before the template is installed.
                command
                    .apply_alignment_request(AlignmentRequest::InstallCellTemplate(alignment))
                    .map_err(|_| ExecError::MissingToken {
                        context: "alignment next-row cell-template lifecycle",
                    })?;
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::NoAlignBeginGroup { alignment } => {
            let active = active_alignment
                .as_mut()
                .filter(|active| active.identity == alignment)
                .ok_or(ExecError::MissingToken {
                    context: "active replay alignment",
                })?;
            let depth = active.noalign_depth.ok_or(ExecError::MissingToken {
                context: "noalign group",
            })?;
            active.noalign_depth = Some(depth.saturating_add(1));
            Ok(ReplayStep::Continue)
        }
        ScannedStep::NoAlignEndGroup { alignment } => {
            let active = active_alignment
                .as_mut()
                .filter(|active| active.identity == alignment)
                .ok_or(ExecError::MissingToken {
                    context: "active replay alignment",
                })?;
            let depth = active.noalign_depth.ok_or(ExecError::MissingToken {
                context: "noalign group",
            })?;
            if depth > 1 {
                active.noalign_depth = Some(depth - 1);
                return Ok(ReplayStep::Continue);
            }
            active.noalign_depth = None;
            active.align_peek_pending = true;
            active.align_peek_after_noalign = true;
            stores
                .leave_group_with_kind(GroupKind::NoAlign)
                .map_err(|_| ExecError::MissingToken {
                    context: "noalign group",
                })?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::AlignmentCellOpening { alignment, opening } => {
            command
                .apply_alignment_request(match opening {
                    AlignmentCellOpening::Template => {
                        AlignmentRequest::InstallCellTemplate(alignment)
                    }
                    AlignmentCellOpening::Omit => {
                        AlignmentRequest::InstallOmitCellTemplate(alignment)
                    }
                })
                .map_err(|_| ExecError::MissingToken {
                    context: "alignment cell-template lifecycle",
                })?;
            if let Some(active) = active_alignment.as_mut()
                && active.identity == alignment
            {
                active.cell_opening_pending = false;
                active.next_cell_opening_pending = false;
            }
            Ok(ReplayStep::Continue)
        }
        ScannedStep::AlignmentCellFinish { alignment } => {
            let finished = command
                .apply_alignment_request(AlignmentRequest::FinishCell(alignment))
                .map_err(|_| ExecError::MissingToken {
                    context: "alignment end-v lifecycle",
                })?;
            let AlignmentRequestResult::FinishedCell(finished) = finished else {
                unreachable!("FinishCell returns its saved delimiter");
            };
            begin_next_replay_alignment_cell(
                alignment,
                finished.delimiter,
                command,
                active_alignment,
            )?;
            Ok(ReplayStep::Continue)
        }
        ScannedStep::AlignmentFinish { alignment } => {
            if active_alignment.as_ref().map(|active| active.identity) != Some(alignment) {
                return Err(ExecError::MissingToken {
                    context: "active replay alignment",
                });
            }
            command
                .apply_alignment_request(AlignmentRequest::Finish(alignment))
                .map_err(|_| ExecError::MissingToken {
                    context: "alignment finish lifecycle",
                })?;
            *active_alignment = None;
            if let Some(outer) = boxes.suspended_alignments.pop() {
                command
                    .apply_alignment_request(AlignmentRequest::Resume(outer.identity))
                    .map_err(|_| ExecError::MissingToken {
                        context: "nested alignment resumption",
                    })?;
                *active_alignment = Some(outer);
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
