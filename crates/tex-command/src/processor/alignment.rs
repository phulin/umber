//! Alignment-template delivery state.
//!
//! TeX.web §789--§790 makes `align_state` a property of raw token delivery,
//! not of the executor's mode nest.  Consequently the executor may request
//! lifecycle transitions here, but only `get_next` classifies a delivered tab,
//! `\span`, or row terminator.

use tex_state::input::TracedTokenList;
use tex_state::meaning::{Meaning, UnexpandablePrimitive};
use tex_state::token::{Catcode, Token};

use crate::CurrentCommand;
use crate::input::InputLevelId;

pub(crate) const PREAMBLE_ALIGN_STATE: i32 = -1_000_000;
pub(crate) const TEMPLATE_ALIGN_STATE: i32 = 1_000_000;
const CELL_ALIGN_STATE: i32 = 0;

/// Stable identity supplied by the executor for one structural alignment.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AlignmentIdentity(u64);

impl AlignmentIdentity {
    /// Creates an executor-owned identity for a structurally active alignment.
    #[must_use]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    #[allow(dead_code)]
    pub(crate) const fn raw(self) -> u64 {
        self.0
    }
}

/// Exact identities of the templates selected for one cell.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct AlignmentCellTemplates {
    /// Prefix replayed before the cell body. `None` is TeX's `\omit` path.
    pub u_template: Option<TracedTokenList>,
    /// Suffix replayed between the intercepted delimiter and its re-delivery.
    pub v_template: TracedTokenList,
}

/// A structural transition requested by the executor.
///
/// These requests deliberately contain no token spelling, command meaning,
/// or delimiter kind.  `tex-exec` owns row/cell packaging and asks for these
/// lifecycle changes; canonical raw delivery remains the only place that can
/// decide that a delivered command is an alignment delimiter.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum AlignmentRequest {
    /// Start scanning an alignment preamble.
    Begin(AlignmentIdentity),
    /// Restart preamble scanning for a repeated preamble column.
    Preamble(AlignmentIdentity),
    /// Start one executor-selected cell and its optional u-template.
    BeginCell {
        alignment: AlignmentIdentity,
        templates: AlignmentCellTemplates,
    },
    /// Retire the exhausted v-template for one finished cell.
    FinishCell(AlignmentIdentity),
    /// Preserve the outer raw-delivery context before entering a nested alignment.
    Suspend(AlignmentIdentity),
    /// Restore the outer raw-delivery context after a nested alignment.
    Resume(AlignmentIdentity),
    /// Tear down a completed alignment delivery context.
    Finish(AlignmentIdentity),
}

/// Result material returned by a structural alignment request.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum AlignmentRequestResult {
    /// The request changed lifecycle state without returning template data.
    Applied,
    /// A finished cell returned the exact templates that were active for it.
    FinishedCell(AlignmentCellTemplates),
}

/// A command-core event that the executor must handle at an alignment boundary.
///
/// The contained command is intentionally opaque.  It can only be handed back
/// to [`crate::CommandProcessor::begin_alignment_v_template`], which backs up
/// the exact delivery before installing the v-template.
#[derive(Debug, Eq, PartialEq)]
pub enum AlignmentDeliveryEvent {
    /// `get_next` intercepted an active-cell delimiter and delivered frozen
    /// `end_template` instead.
    EndTemplate(crate::CurrentCommand),
}

/// One expanded delivery while the executor is running an alignment cell.
///
/// Ordinary commands continue to main control. An intercepted delimiter is
/// represented separately so that only the command processor decides when to
/// enter the v-template transition.
#[derive(Debug, Eq, PartialEq)]
pub enum AlignmentDelivery {
    /// An ordinary expanded command for executor main control.
    Command(crate::CurrentCommand),
    /// A command-core alignment event.
    Event(AlignmentDeliveryEvent),
}

/// A lifecycle request did not match the currently active alignment.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum AlignmentLifecycleError {
    /// A requested transition needs an active alignment, but none is live.
    NoActiveAlignment,
    /// A transition named a different structural alignment.
    WrongAlignment,
    /// The requested operation is invalid while a cell is already active.
    CellAlreadyActive,
    /// The requested operation needs a current active cell.
    NoActiveCell,
    /// Resume was requested without a suspended outer alignment.
    NoSuspendedAlignment,
    /// The cell has not reached the point where its v-template may start.
    UTemplateStillActive,
    /// The cell has no retained, exhausted v-template to retire.
    VTemplateNotExhausted,
}

impl std::fmt::Display for AlignmentLifecycleError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::NoActiveAlignment => "no alignment delivery context is active",
            Self::WrongAlignment => "alignment delivery identity does not match",
            Self::CellAlreadyActive => "an alignment cell delivery is already active",
            Self::NoActiveCell => "no alignment cell delivery is active",
            Self::NoSuspendedAlignment => "no outer alignment delivery context is suspended",
            Self::UTemplateStillActive => "the alignment u-template is still active",
            Self::VTemplateNotExhausted => "the alignment v-template is not exhausted",
        })
    }
}

impl std::error::Error for AlignmentLifecycleError {}

/// Persistent ownership for alignment-sensitive raw delivery.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) struct AlignmentDeliveryState {
    pub(crate) align_state: i32,
    pub(crate) active_alignment: Option<AlignmentIdentity>,
    pub(crate) suspended: Vec<SuspendedAlignment>,
    pub(crate) active_cell: Option<ActiveCellDelivery>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct SuspendedAlignment {
    pub(crate) alignment: AlignmentIdentity,
    pub(crate) align_state: i32,
    pub(crate) active_cell: Option<ActiveCellDelivery>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct ActiveCellDelivery {
    pub(crate) alignment: AlignmentIdentity,
    pub(crate) templates: AlignmentCellTemplates,
    pub(crate) u_level: Option<InputLevelId>,
    pub(crate) v_level: Option<InputLevelId>,
}

/// The one semantic alignment adjustment made by a raw delivery.
///
/// It is stored on `CurrentCommand`, so `back_input` can reverse exactly that
/// transition without inspecting the replayed spelling.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) enum AlignmentDeliveryAdjustment {
    #[default]
    None,
    BeginGroup,
    EndGroup,
    Delimiter,
}

impl AlignmentDeliveryState {
    pub(crate) fn begin_alignment(&mut self, alignment: AlignmentIdentity) {
        self.active_alignment = Some(alignment);
        self.active_cell = None;
        self.align_state = PREAMBLE_ALIGN_STATE;
    }

    pub(crate) fn set_preamble_phase(
        &mut self,
        alignment: AlignmentIdentity,
    ) -> Result<(), AlignmentLifecycleError> {
        self.require_alignment(alignment)?;
        self.active_cell = None;
        self.align_state = PREAMBLE_ALIGN_STATE;
        Ok(())
    }

    pub(crate) fn begin_cell(
        &mut self,
        alignment: AlignmentIdentity,
        templates: AlignmentCellTemplates,
    ) -> Result<(), AlignmentLifecycleError> {
        self.require_alignment(alignment)?;
        if self.active_cell.is_some() {
            return Err(AlignmentLifecycleError::CellAlreadyActive);
        }
        self.active_cell = Some(ActiveCellDelivery {
            alignment,
            templates,
            u_level: None,
            v_level: None,
        });
        self.align_state = if templates.u_template.is_some() {
            TEMPLATE_ALIGN_STATE
        } else {
            CELL_ALIGN_STATE
        };
        Ok(())
    }

    pub(crate) fn attach_u_template(
        &mut self,
        alignment: AlignmentIdentity,
        level: InputLevelId,
    ) -> Result<(), AlignmentLifecycleError> {
        let cell = self.active_cell_mut(alignment)?;
        cell.u_level = Some(level);
        Ok(())
    }

    pub(crate) fn finish_u_template(&mut self, level: InputLevelId) -> bool {
        let Some(cell) = self.active_cell.as_mut() else {
            return false;
        };
        if cell.u_level != Some(level) {
            return false;
        }
        cell.u_level = None;
        self.align_state = CELL_ALIGN_STATE;
        true
    }

    pub(crate) fn begin_v_template(
        &mut self,
        alignment: AlignmentIdentity,
        level: InputLevelId,
    ) -> Result<(), AlignmentLifecycleError> {
        let cell = self.active_cell_mut(alignment)?;
        if cell.u_level.is_some() {
            return Err(AlignmentLifecycleError::UTemplateStillActive);
        }
        cell.v_level = Some(level);
        self.align_state = TEMPLATE_ALIGN_STATE;
        Ok(())
    }

    pub(crate) fn v_template(
        &self,
        alignment: AlignmentIdentity,
    ) -> Result<TracedTokenList, AlignmentLifecycleError> {
        let cell = self.active_cell_ref(alignment)?;
        if cell.u_level.is_some() {
            return Err(AlignmentLifecycleError::UTemplateStillActive);
        }
        Ok(cell.templates.v_template)
    }

    pub(crate) fn active_v_template_level(
        &self,
        alignment: AlignmentIdentity,
    ) -> Result<InputLevelId, AlignmentLifecycleError> {
        self.active_cell_ref(alignment)?
            .v_level
            .ok_or(AlignmentLifecycleError::VTemplateNotExhausted)
    }

    pub(crate) fn finish_cell(
        &mut self,
        alignment: AlignmentIdentity,
        v_level: InputLevelId,
    ) -> Result<AlignmentCellTemplates, AlignmentLifecycleError> {
        self.require_alignment(alignment)?;
        let cell = self.active_cell_ref(alignment)?;
        if cell.v_level != Some(v_level) {
            return Err(AlignmentLifecycleError::VTemplateNotExhausted);
        }
        let cell = self
            .active_cell
            .take()
            .expect("active cell was just checked");
        self.align_state = TEMPLATE_ALIGN_STATE;
        Ok(cell.templates)
    }

    pub(crate) fn suspend_alignment(
        &mut self,
        alignment: AlignmentIdentity,
    ) -> Result<(), AlignmentLifecycleError> {
        self.require_alignment(alignment)?;
        self.suspended.push(SuspendedAlignment {
            alignment,
            align_state: self.align_state,
            active_cell: self.active_cell.take(),
        });
        self.active_alignment = None;
        self.align_state = CELL_ALIGN_STATE;
        Ok(())
    }

    pub(crate) fn resume_alignment(
        &mut self,
        alignment: AlignmentIdentity,
    ) -> Result<(), AlignmentLifecycleError> {
        let suspended = self
            .suspended
            .pop()
            .ok_or(AlignmentLifecycleError::NoSuspendedAlignment)?;
        if suspended.alignment != alignment {
            self.suspended.push(suspended);
            return Err(AlignmentLifecycleError::WrongAlignment);
        }
        self.active_alignment = Some(alignment);
        self.align_state = suspended.align_state;
        self.active_cell = suspended.active_cell;
        Ok(())
    }

    pub(crate) fn finish_alignment(
        &mut self,
        alignment: AlignmentIdentity,
    ) -> Result<(), AlignmentLifecycleError> {
        self.require_alignment(alignment)?;
        self.active_alignment = None;
        self.active_cell = None;
        self.align_state = CELL_ALIGN_STATE;
        Ok(())
    }

    pub(crate) fn classify_delivery(
        &mut self,
        command: &mut CurrentCommand,
    ) -> AlignmentDeliveryAdjustment {
        match command.spelling().semantic_token() {
            Token::Char {
                cat: Catcode::BeginGroup,
                ..
            } => {
                self.align_state += 1;
                AlignmentDeliveryAdjustment::BeginGroup
            }
            Token::Char {
                cat: Catcode::EndGroup,
                ..
            } => {
                self.align_state -= 1;
                AlignmentDeliveryAdjustment::EndGroup
            }
            Token::Char {
                cat: Catcode::AlignmentTab,
                ..
            } if self.active_cell.is_some() && self.align_state == CELL_ALIGN_STATE => {
                self.intercept_delimiter(command)
            }
            _ if self.active_cell.is_some()
                && self.align_state == CELL_ALIGN_STATE
                && matches!(
                    command.meaning(),
                    Meaning::UnexpandablePrimitive(
                        UnexpandablePrimitive::Cr
                            | UnexpandablePrimitive::CrCr
                            | UnexpandablePrimitive::Span
                    )
                ) =>
            {
                self.intercept_delimiter(command)
            }
            _ => AlignmentDeliveryAdjustment::None,
        }
    }

    pub(crate) fn undo_delivery(&mut self, adjustment: AlignmentDeliveryAdjustment) {
        match adjustment {
            AlignmentDeliveryAdjustment::None => {}
            AlignmentDeliveryAdjustment::BeginGroup => self.align_state -= 1,
            AlignmentDeliveryAdjustment::EndGroup => self.align_state += 1,
            AlignmentDeliveryAdjustment::Delimiter => self.align_state = CELL_ALIGN_STATE,
        }
    }

    pub(crate) fn undo_delimiter_begin_group_delivery(&mut self) {
        self.align_state -= 1;
    }

    fn intercept_delimiter(&mut self, command: &mut CurrentCommand) -> AlignmentDeliveryAdjustment {
        self.align_state = TEMPLATE_ALIGN_STATE;
        command.convert_to_end_template();
        AlignmentDeliveryAdjustment::Delimiter
    }

    fn require_alignment(
        &self,
        alignment: AlignmentIdentity,
    ) -> Result<(), AlignmentLifecycleError> {
        match self.active_alignment {
            None => Err(AlignmentLifecycleError::NoActiveAlignment),
            Some(active) if active != alignment => Err(AlignmentLifecycleError::WrongAlignment),
            Some(_) => Ok(()),
        }
    }

    fn active_cell_ref(
        &self,
        alignment: AlignmentIdentity,
    ) -> Result<&ActiveCellDelivery, AlignmentLifecycleError> {
        self.require_alignment(alignment)?;
        let cell = self
            .active_cell
            .as_ref()
            .ok_or(AlignmentLifecycleError::NoActiveCell)?;
        if cell.alignment != alignment {
            return Err(AlignmentLifecycleError::WrongAlignment);
        }
        Ok(cell)
    }

    fn active_cell_mut(
        &mut self,
        alignment: AlignmentIdentity,
    ) -> Result<&mut ActiveCellDelivery, AlignmentLifecycleError> {
        self.require_alignment(alignment)?;
        let cell = self
            .active_cell
            .as_mut()
            .ok_or(AlignmentLifecycleError::NoActiveCell)?;
        if cell.alignment != alignment {
            return Err(AlignmentLifecycleError::WrongAlignment);
        }
        Ok(cell)
    }
}
