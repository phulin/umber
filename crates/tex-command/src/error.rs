//! Private command diagnostics and typed resource needs.

/// A command-core operation could not preserve its input-state invariant.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CommandError {
    /// A stale or malformed input-level transition was observed.
    InputInvariant,
    /// A backup did not name the most recent raw delivery in this processor.
    StaleDelivery,
    /// A macro's compulsory parameter text did not match its invocation.
    MacroPrefixMismatch,
    /// A non-`\long` macro argument ended at a paragraph token.
    ParagraphInMacroArgument,
    /// An outer token was recovered while a macro argument was being matched.
    OuterInMacroArgument,
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InputInvariant => formatter.write_str("command input-state invariant failed"),
            Self::StaleDelivery => formatter.write_str("command delivery is no longer current"),
            Self::MacroPrefixMismatch => {
                formatter.write_str("macro invocation does not match its definition")
            }
            Self::ParagraphInMacroArgument => {
                formatter.write_str("paragraph ended before macro argument was complete")
            }
            Self::OuterInMacroArgument => {
                formatter.write_str("outer token found while scanning macro argument")
            }
        }
    }
}

impl std::error::Error for CommandError {}
