//! Private command diagnostics and typed resource needs.

/// A command-core operation could not preserve its input-state invariant.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CommandError {
    /// A stale or malformed input-level transition was observed.
    InputInvariant,
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InputInvariant => formatter.write_str("command input-state invariant failed"),
        }
    }
}

impl std::error::Error for CommandError {}
