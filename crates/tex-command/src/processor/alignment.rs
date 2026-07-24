//! Alignment-template delivery state.
//!
//! TeX.web §789--§790 makes `align_state` a property of raw token delivery,
//! not of the executor's mode nest.  Consequently the executor may request
//! lifecycle transitions here, but only `get_next` classifies a delivered tab,
//! `\span`, or row terminator.

use tex_state::meaning::{Meaning, UnexpandablePrimitive};
use tex_state::token::{Catcode, Token};

use crate::CurrentCommand;

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
}

/// Exact identities of the templates selected for one cell.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct AlignmentCellTemplates {
    /// Executor-owned u-template identity.
    pub u_template: u64,
    /// Executor-owned v-template identity.
    pub v_template: u64,
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
}

impl std::fmt::Display for AlignmentLifecycleError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::NoActiveAlignment => "no alignment delivery context is active",
            Self::WrongAlignment => "alignment delivery identity does not match",
            Self::CellAlreadyActive => "an alignment cell delivery is already active",
            Self::NoActiveCell => "no alignment cell delivery is active",
            Self::NoSuspendedAlignment => "no outer alignment delivery context is suspended",
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
        });
        self.align_state = CELL_ALIGN_STATE;
        Ok(())
    }

    pub(crate) fn finish_cell(
        &mut self,
        alignment: AlignmentIdentity,
    ) -> Result<AlignmentCellTemplates, AlignmentLifecycleError> {
        self.require_alignment(alignment)?;
        let cell = self
            .active_cell
            .take()
            .ok_or(AlignmentLifecycleError::NoActiveCell)?;
        if cell.alignment != alignment {
            return Err(AlignmentLifecycleError::WrongAlignment);
        }
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
}
