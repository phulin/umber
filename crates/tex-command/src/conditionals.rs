//! Private independent conditional state machine.

/// Independent persistent condition stack.
///
/// Condition frames never belong to input levels or the aggregate
/// environment save stack.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) struct ConditionStack {
    pub(crate) frames: Vec<ConditionFrame>,
    pub(crate) next_identity: u64,
}

/// Placeholder for TeX's `cur_if`, `if_limit`, and `if_line` state.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct ConditionFrame {
    pub(crate) identity: u64,
    pub(crate) kind: u16,
    pub(crate) limit: u16,
    pub(crate) source_line: u64,
}
