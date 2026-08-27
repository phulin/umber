//! Private command diagnostics and typed resource needs.

use std::panic::Location;
use tex_state::PrepareMagDiagnostic;
use tex_state::scaled::DimensionError;
use tex_state::token::OriginId;

/// Recoverable diagnostics emitted by a dimension scan that still produces
/// TeX's capped or substituted value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DimensionDiagnostic {
    MissingNumber,
    IllegalUnit { inserted: InsertedUnit },
    IncompatibleGlueUnits,
    TooLarge,
    IllegalMagnification { attempted: i32 },
    IncompatibleMagnification { attempted: i32, retained: i32 },
}

/// The unit TeX inserts while recovering an invalid dimension.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InsertedUnit {
    Pt,
    Mu,
}

impl std::fmt::Display for DimensionDiagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingNumber => f.write_str("Missing number, treated as zero"),
            Self::IllegalUnit {
                inserted: InsertedUnit::Pt,
            } => f.write_str("Illegal unit of measure (pt inserted)"),
            Self::IllegalUnit {
                inserted: InsertedUnit::Mu,
            } => f.write_str("Illegal unit of measure (mu inserted)"),
            Self::IncompatibleGlueUnits => f.write_str("Incompatible glue units"),
            Self::TooLarge => f.write_str("Dimension too large"),
            Self::IllegalMagnification { .. } => {
                f.write_str("Illegal magnification has been changed to 1000")
            }
            Self::IncompatibleMagnification { attempted, .. } => write!(
                f,
                "Incompatible magnification ({attempted}); the previous value will be retained"
            ),
        }
    }
}

impl From<DimensionError> for DimensionDiagnostic {
    fn from(value: DimensionError) -> Self {
        match value {
            DimensionError::TooLarge => Self::TooLarge,
        }
    }
}

impl From<PrepareMagDiagnostic> for DimensionDiagnostic {
    fn from(value: PrepareMagDiagnostic) -> Self {
        match value {
            PrepareMagDiagnostic::IllegalMagnification { attempted } => {
                Self::IllegalMagnification { attempted }
            }
            PrepareMagDiagnostic::IncompatibleMagnification {
                attempted,
                retained,
            } => Self::IncompatibleMagnification {
                attempted,
                retained,
            },
        }
    }
}

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
    FuelExhausted {
        limit: u64,
        burned: u64,
        work: crate::CommandWorkCounters,
    },
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
    MissingInput { name: String, original_name: String },
    /// A non-opening file enquiry has no retained bytes or authoritative
    /// absence yet and must suspend for a typed host probe.
    MissingInputProbe(crate::FileEnquiryRequest),
    /// An otherwise-originless command failure annotated by the expandable
    /// delivery which triggered it. Typed resource suspensions deliberately
    /// remain unwrapped so the host can retry them.
    AtOrigin {
        error: Box<CommandError>,
        origin: OriginId,
    },
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
    /// Carries §81's non-local exit from the processor-owned interaction
    /// transition to the driver.
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

    pub(crate) const fn is_resource_suspension(&self) -> bool {
        matches!(self, Self::MissingInput { .. } | Self::MissingInputProbe(_))
    }

    pub(crate) fn at_origin_unless_resource(self, origin: OriginId) -> Self {
        if matches!(
            self,
            Self::MissingInput { .. } | Self::MissingInputProbe(_) | Self::AtOrigin { .. }
        ) {
            self
        } else {
            Self::AtOrigin {
                error: Box::new(self),
                origin,
            }
        }
    }
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FuelExhausted {
                limit,
                burned,
                work,
            } => write!(
                formatter,
                "canonical command fuel exhausted after {burned} actions (limit {limit}); command work: fuel_charges={} token_frame_steps={} expanded_deliveries={} meaning_lookups={} scanner_tokens={} write_expansions={}",
                work.fuel_charges,
                work.token_frame_steps,
                work.expanded_deliveries,
                work.meaning_lookups,
                work.scanner_tokens,
                work.write_expansions,
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
            Self::MissingInput { name, .. } => {
                write!(formatter, "input source `{name}` is unavailable")
            }
            Self::MissingInputProbe(request) => {
                write!(formatter, "input enquiry `{}` is unresolved", request.name)
            }
            Self::AtOrigin { error, .. } => std::fmt::Display::fmt(error, formatter),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dimension_diagnostic_conversion_and_units_are_canonical_values() {
        assert_eq!(
            DimensionDiagnostic::from(DimensionError::TooLarge),
            DimensionDiagnostic::TooLarge
        );
        assert_eq!(
            DimensionDiagnostic::IllegalUnit {
                inserted: InsertedUnit::Mu,
            }
            .to_string(),
            "Illegal unit of measure (mu inserted)"
        );
        assert_eq!(
            DimensionDiagnostic::from(PrepareMagDiagnostic::IncompatibleMagnification {
                attempted: 1200,
                retained: 1000,
            }),
            DimensionDiagnostic::IncompatibleMagnification {
                attempted: 1200,
                retained: 1000,
            }
        );
    }
}
