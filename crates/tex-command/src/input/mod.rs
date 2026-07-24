//! Private input state machines.

mod backup;
mod levels;
mod lines;
mod source;
mod summary;
mod tokenizer;

pub(crate) use levels::InputLevel;

/// Persistent input-stack ownership.
///
/// This state owns only future deliveries and semantic identity allocation.
/// Conditions, scanner policy, meanings, and host capabilities belong to
/// other ownership classes.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) struct InputState {
    pub(crate) levels: Vec<InputLevel>,
    pub(crate) next_level_identity: u64,
    pub(crate) next_source_identity: u64,
}
