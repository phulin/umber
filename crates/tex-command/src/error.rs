//! Private command diagnostics and typed resource needs.

/// A command-core operation could not preserve its input-state invariant.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CommandError {
    /// A stale or malformed input-level transition was observed.
    InputInvariant,
    /// A backup did not name the most recent raw delivery in this processor.
    StaleDelivery,
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InputInvariant => formatter.write_str("command input-state invariant failed"),
            Self::StaleDelivery => formatter.write_str("command delivery is no longer current"),
        }
    }
}

impl std::error::Error for CommandError {}
