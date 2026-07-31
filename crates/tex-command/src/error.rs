//! Private command diagnostics and typed resource needs.

use std::panic::Location;

/// The Rust call site that raised an [`CommandError::InputInvariant`].
///
/// `InputInvariant` is a shared sentinel used by dozens of call sites across
/// `tex-command`'s scanners, conditionals, and macro machinery (see
/// `docs/testing_infrastructure.md`'s note on diagnosing a canonical
/// divergence): the variant alone does not pin which one fired. Capturing the
/// constructing call site with `#[track_caller]` gives every future
/// `CommandError::InputInvariant` an immediate, precise origin without
/// per-caller plumbing. Equality intentionally ignores the captured site so
/// existing structural comparisons against this variant are unaffected.
#[derive(Clone, Copy)]
pub struct InputInvariantOrigin(&'static Location<'static>);

impl std::fmt::Debug for InputInvariantOrigin {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self.0, formatter)
    }
}

impl std::fmt::Display for InputInvariantOrigin {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self.0, formatter)
    }
}

impl PartialEq for InputInvariantOrigin {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl Eq for InputInvariantOrigin {}

/// A command-core operation could not preserve its input-state invariant.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommandError {
    /// The command-owned resource ledger cannot fund another canonical
    /// delivery action.
    FuelExhausted { limit: u64, burned: u64 },
    /// A stale or malformed input-level transition was observed.
    InputInvariant(InputInvariantOrigin),
    /// A backup did not name the most recent raw delivery in this processor.
    StaleDelivery,
    /// A macro's compulsory parameter text did not match its invocation.
    MacroPrefixMismatch,
    /// A non-`\long` macro argument ended at a paragraph token.
    ParagraphInMacroArgument,
    /// An outer token was recovered while a macro argument was being matched.
    OuterInMacroArgument,
    /// The installed input capability has no immutable backing for a
    /// requested logical filename.
    MissingInput(String),
    /// This expansion slice has not installed the primitive's canonical
    /// scalar handler yet.
    UnsupportedExpandablePrimitive(tex_state::meaning::ExpandablePrimitive),
    /// A pdfTeX navigation scanner rejected an action, identifier, or view.
    PdfNavigation(&'static str),
    /// TeX82 §93 `succumb`: the job is over. This is not a recoverable command
    /// failure; it is §81 `jump_out` unwinding to the driver, which latches the
    /// terminal state and ends the job.
    Fatal(crate::FatalError),
}

impl From<tex_state::print::JumpOut> for CommandError {
    /// Lets a scanner write `report.error().jump_out()?` and have `?` carry
    /// §81's non-local exit the rest of the way to the driver.
    fn from(jump: tex_state::print::JumpOut) -> Self {
        Self::Fatal(jump.into())
    }
}

impl CommandError {
    /// Constructs [`CommandError::InputInvariant`], capturing the Rust
    /// call site that raised it so a canonical divergence names its true
    /// origin instead of the shared, otherwise-unresolvable sentinel.
    #[track_caller]
    pub(crate) fn input_invariant() -> Self {
        Self::InputInvariant(InputInvariantOrigin(Location::caller()))
    }
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FuelExhausted { limit, burned } => write!(
                formatter,
                "canonical command fuel exhausted after {burned} actions (limit {limit})"
            ),
            Self::InputInvariant(origin) => {
                write!(
                    formatter,
                    "command input-state invariant failed (at {origin})"
                )
            }
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
            Self::MissingInput(name) => write!(formatter, "input source `{name}` is unavailable"),
            Self::UnsupportedExpandablePrimitive(primitive) => {
                write!(
                    formatter,
                    "expandable primitive {primitive:?} is not installed"
                )
            }
            Self::PdfNavigation(message) => formatter.write_str(message),
            Self::Fatal(fatal) => write!(formatter, "irrecoverable error: {fatal}"),
        }
    }
}

impl std::error::Error for CommandError {}
