//! Alignment-template delivery.

/// Persistent ownership for alignment-sensitive raw delivery.
///
/// This is the command core's only `align_state`. Execution modes must not
/// duplicate the brace-depth state.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) struct AlignmentDeliveryState {
    pub(crate) align_state: i32,
    pub(crate) suspended: Vec<SuspendedAlignment>,
    pub(crate) active_cell: Option<ActiveCellDelivery>,
}

/// Placeholder for one suspended nested alignment delivery context.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct SuspendedAlignment {
    pub(crate) alignment: u64,
    pub(crate) align_state: i32,
}

/// Placeholder identities for an active cell and its exact templates.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct ActiveCellDelivery {
    pub(crate) alignment: u64,
    pub(crate) u_template: u64,
    pub(crate) v_template: u64,
}
