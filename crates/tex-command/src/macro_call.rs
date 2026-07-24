//! Private canonical scalar macro-call state machine.

/// Persistent ownership of live macro-argument activations.
///
/// Activations are separate from the input-level representation so nested
/// token input can later resolve TeX's nearest live `param_start` equivalent
/// without moving parameter ownership into a cursor.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) struct ParameterState {
    pub(crate) activations: Vec<MacroActivation>,
}

/// Placeholder identity for one live macro activation.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct MacroActivation {
    pub(crate) definition: u64,
    pub(crate) invocation: u64,
}
