//! Compact typed control vocabulary for parked expansion work.

use crate::execution_scratch::ScannerFrameKey;
use tex_state::token::OriginId;

use super::{ExpansionChild, ExpansionCommandSlot, ExpansionNameMark};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TraceState {
    Unseen,
    Complete,
    UnlessOperandPending,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ExpandAfterSecondDestination;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CsNameTokenDestination;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct IfCsNameTokenDestination;

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum ExpandAfterPhase<G> {
    NeedOperands,
    AwaitSecond {
        child: ExpansionChild<G, ExpandAfterSecondDestination>,
    },
    ReplayFirst,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct ExpandAfterControl<G> {
    pub(crate) opener: ExpansionCommandSlot<G>,
    pub(crate) saved_first: Option<ExpansionCommandSlot<G>>,
    pub(crate) phase: ExpandAfterPhase<G>,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum CsNamePhase<G> {
    Collecting,
    AwaitToken(ExpansionChild<G, CsNameTokenDestination>),
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct CsNameControl<G> {
    pub(crate) opener: ExpansionCommandSlot<G>,
    pub(crate) name: ExpansionNameMark,
    pub(crate) phase: CsNamePhase<G>,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum IfCsNamePhase<G> {
    Collecting,
    AwaitToken(ExpansionChild<G, IfCsNameTokenDestination>),
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct IfCsNameControl<G> {
    pub(crate) opener: ExpansionCommandSlot<G>,
    pub(crate) condition: crate::processor::status::ConditionId,
    pub(crate) inverted: bool,
    pub(crate) name: ExpansionNameMark,
    pub(crate) phase: IfCsNamePhase<G>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct IfNumberLeftDestination;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct IfNumberRightDestination;

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct ScannerChild<G, D> {
    key: ScannerFrameKey<G>,
    destination: D,
}

impl<G, D> ScannerChild<G, D> {
    pub(crate) fn new(key: ScannerFrameKey<G>, destination: D) -> Self {
        Self { key, destination }
    }

    pub(crate) fn restore(self) -> (ScannerFrameKey<G>, D) {
        (self.key, self.destination)
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum IfNumberPhase<G> {
    NeedLeft,
    AwaitLeft(ScannerChild<G, IfNumberLeftDestination>),
    NeedRelation {
        left: i32,
    },
    AwaitRight {
        left: i32,
        relation: u8,
        child: ScannerChild<G, IfNumberRightDestination>,
    },
    Complete,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct IfNumberControl<G> {
    pub(crate) opener: ExpansionCommandSlot<G>,
    pub(crate) condition: crate::processor::status::ConditionId,
    pub(crate) inverted: bool,
    pub(crate) phase: IfNumberPhase<G>,
}

/// The compact continuation for an expanded `\the` operand.
///
/// `\the` is a particularly important continuation because the operand is
/// itself delivered through the expanded-token loop.  Keeping only the
/// opener's origin here lets that loop consume an arbitrary chain of
/// `\the`/macro expansions without retaining a `CurrentCommand` or entering
/// a second delivery routine.  The target command is always the hot command
/// currently owned by the delivery loop and is materialised only at the
/// scalar scanner boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TheControl {
    pub(crate) opener: OriginId,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum UnlessPhase<G> {
    NeedConditional,
    DispatchConditional {
        command: ExpansionCommandSlot<G>,
        trace: TraceState,
    },
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct UnlessControl<G> {
    pub(crate) opener: ExpansionCommandSlot<G>,
    pub(crate) phase: UnlessPhase<G>,
}

/// Primitive PCs are variant-specific, so impossible route/payload pairs
/// cannot be assembled through a generic bag of fields.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum PrimitiveControl<G> {
    CsName(CsNameControl<G>),
    IfCsName(IfCsNameControl<G>),
    IfNumber(IfNumberControl<G>),
    Unless(UnlessControl<G>),
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum ExpansionControl<G> {
    Dispatch {
        command: ExpansionCommandSlot<G>,
        trace: TraceState,
    },
    Suspended {
        command: ExpansionCommandSlot<G>,
        resume: crate::state::PendingExpansionResume,
        delivery_expanded: bool,
        child: Option<
            crate::execution_scratch::ChildContinuation<
                G,
                crate::state::PendingExpansionChildDestination,
            >,
        >,
    },
    ExpandAfter(ExpandAfterControl<G>),
    The(TheControl),
    Primitive(PrimitiveControl<G>),
}
