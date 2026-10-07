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

/// The Rust call site that raised a [`CommandErrorKind::InputInvariant`].
///
/// `InputInvariant` is a shared sentinel used by dozens of call sites across
/// `tex-command`'s scanners, conditionals, and macro machinery (see
/// `docs/testing_infrastructure.md`'s note on diagnosing a canonical
/// divergence): the variant alone does not pin which one fired. Capturing the
/// constructing call site with `#[track_caller]` gives every future
/// `CommandErrorKind::InputInvariant` an immediate, precise origin without
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

/// Cold work snapshot retained only after fuel exhaustion.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FuelExhaustion {
    pub limit: u64,
    pub burned: u64,
    pub work: crate::CommandWorkCounters,
}

/// Owned filenames at a missing-input replay boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MissingInputDiagnostic {
    pub name: String,
    pub original_name: String,
}

/// The original resource need and its owned failure cause.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceFailureDiagnostic {
    pub need: crate::ResourceNeed,
    pub failure: crate::ResourceFailure,
}

/// One cold failure owner; successful command calls carry no payload.
pub type CommandError = Box<CommandFailure>;

/// Actual failure facts, allocated only when an operation fails.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandFailure {
    pub kind: CommandErrorKind,
    pub origin: Option<OriginId>,
}

/// The typed reason a command-core operation failed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommandErrorKind {
    /// The command-owned resource ledger cannot fund another canonical
    /// delivery action.
    FuelExhausted(FuelExhaustion),
    /// A stale or malformed input-level transition was observed.
    InputInvariant(InputInvariantOrigin),
    /// A captured diagnostic context no longer names the live input.
    StaleDiagnosticContext,
    /// A macro's compulsory parameter text did not match its invocation.
    MacroPrefixMismatch,
    /// A non-`\long` macro argument ended at a paragraph token.
    ParagraphInMacroArgument,
    /// An outer token was recovered while a macro argument was being matched.
    OuterInMacroArgument,
    /// The installed input capability has no immutable backing for a
    /// requested logical filename.
    MissingInput(MissingInputDiagnostic),
    /// A non-opening file enquiry has no retained bytes or authoritative
    /// absence yet and requests a typed host probe.
    MissingInputProbe(crate::FileEnquiryRequest),
    /// A synchronous resource provider failed while resolving a typed need.
    /// The original need and owned cause remain intact so the executor does
    /// not misclassify a real failure as an authoritative absence.
    ResourceFailure(Box<ResourceFailureDiagnostic>),
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
        CommandFailure::fatal(jump.into())
    }
}

impl From<crate::execution_scratch::ScratchError> for CommandError {
    fn from(error: crate::execution_scratch::ScratchError) -> Self {
        crate::scan_toks::scratch_command_error(error)
    }
}

impl CommandFailure {
    #[cold]
    #[must_use]
    pub fn new(kind: CommandErrorKind) -> CommandError {
        Box::new(Self { kind, origin: None })
    }

    #[cold]
    pub(crate) fn fuel_exhausted(
        limit: u64,
        burned: u64,
        work: crate::CommandWorkCounters,
    ) -> CommandError {
        Self::new(CommandErrorKind::FuelExhausted(FuelExhaustion {
            limit,
            burned,
            work,
        }))
    }

    #[cold]
    pub(crate) fn missing_input(name: String, original_name: String) -> CommandError {
        Self::new(CommandErrorKind::MissingInput(MissingInputDiagnostic {
            name,
            original_name,
        }))
    }

    #[cold]
    pub(crate) fn resource_failure(
        need: crate::ResourceNeed,
        failure: crate::ResourceFailure,
    ) -> CommandError {
        Self::new(CommandErrorKind::ResourceFailure(Box::new(
            ResourceFailureDiagnostic { need, failure },
        )))
    }

    #[cold]
    pub(crate) fn pdf_navigation(message: &'static str) -> CommandError {
        Self::new(CommandErrorKind::PdfNavigation(message))
    }

    #[cold]
    pub(crate) fn fatal(fatal: crate::FatalError) -> CommandError {
        Self::new(CommandErrorKind::Fatal(fatal))
    }

    /// Captures the exact constructing Rust call site at the failure boundary.
    #[cold]
    #[track_caller]
    pub(crate) fn input_invariant() -> CommandError {
        Self::new(CommandErrorKind::InputInvariant(InputInvariantOrigin(
            Location::caller(),
        )))
    }

    pub(crate) const fn is_resource_suspension(&self) -> bool {
        matches!(
            self.kind,
            CommandErrorKind::MissingInput(_) | CommandErrorKind::MissingInputProbe(_)
        )
    }

    pub(crate) fn at_origin_unless_resource(mut self: Box<Self>, origin: OriginId) -> CommandError {
        if self.origin.is_none() && !self.is_resource_suspension() {
            self.origin = Some(origin);
        }
        self
    }
}

impl std::fmt::Display for CommandFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.kind {
            CommandErrorKind::FuelExhausted(fuel) => {
                let FuelExhaustion {
                    limit,
                    burned,
                    work,
                } = fuel;
                write!(
                    formatter,
                    "canonical command fuel exhausted after {burned} actions (limit {limit}); command work: fuel_charges={} token_frame_steps={} expanded_deliveries={} meaning_lookups={} scanner_tokens={} write_expansions={}",
                    work.fuel_charges,
                    work.token_frame_steps,
                    work.expanded_deliveries,
                    work.meaning_lookups,
                    work.scanner_tokens,
                    work.write_expansions,
                )?;
                #[cfg(feature = "profiling")]
                write!(
                    formatter,
                    " raw_source_deliveries={} raw_stored_token_deliveries={} raw_macro_argument_deliveries={} raw_synthetic_end_v_deliveries={}",
                    work.raw_delivery_kinds[0],
                    work.raw_delivery_kinds[1],
                    work.raw_delivery_kinds[2],
                    work.raw_delivery_kinds[3],
                )?;
                Ok(())
            }
            CommandErrorKind::InputInvariant(origin) => {
                write!(
                    formatter,
                    "command input-state invariant failed (at {origin})"
                )
            }
            CommandErrorKind::StaleDiagnosticContext => {
                formatter.write_str("diagnostic context is no longer current")
            }
            CommandErrorKind::MacroPrefixMismatch => {
                formatter.write_str("macro invocation does not match its definition")
            }
            CommandErrorKind::ParagraphInMacroArgument => {
                formatter.write_str("paragraph ended before macro argument was complete")
            }
            CommandErrorKind::OuterInMacroArgument => {
                formatter.write_str("outer token found while scanning macro argument")
            }
            CommandErrorKind::MissingInput(input) => {
                write!(formatter, "input source `{}` is unavailable", input.name)
            }
            CommandErrorKind::MissingInputProbe(request) => {
                write!(formatter, "input enquiry `{}` is unresolved", request.name)
            }
            CommandErrorKind::ResourceFailure(resource) => {
                write!(
                    formatter,
                    "resource resolution failed: {}",
                    resource.failure
                )
            }
            CommandErrorKind::UnsupportedExpandablePrimitive(primitive) => {
                write!(
                    formatter,
                    "expandable primitive {primitive:?} is not installed"
                )
            }
            CommandErrorKind::PdfNavigation(message) => formatter.write_str(message),
            CommandErrorKind::Fatal(fatal) => write!(formatter, "irrecoverable error: {fatal}"),
        }
    }
}

impl std::error::Error for CommandFailure {}

#[cfg(test)]
mod tests;
