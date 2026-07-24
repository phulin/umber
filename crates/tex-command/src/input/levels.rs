//! Source and stored-token input levels.

/// Placeholder for one future-relevant input level.
///
/// Cursor payload and replay behavior will replace this shell when raw
/// command delivery is implemented. The stable identity already belongs to
/// the semantic input stack and therefore survives snapshots.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct InputLevel {
    pub(crate) identity: u64,
}
