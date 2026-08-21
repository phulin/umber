use std::fmt;

use tex_command::{CommandError, FatalError};
use tex_state::FontParameterError;
use tex_state::WorldError;
use tex_state::meaning::ExpandablePrimitive;
use tex_state::meaning::UnexpandablePrimitive;
use tex_state::provenance::DiagnosticSite;
use tex_state::provenance::OriginRef;
use tex_state::token::{OriginId, Token, TracedTokenWord};
use tex_state::{ColdProvenanceDemand, CommandContext, DiagnosticOriginRequest};

use crate::Mode;

/// Arena-independent source evidence frozen before a failed step
/// rolls speculative provenance back.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FrozenDiagnosticOrigin {
    Root(tex_state::RootSpanId),
    Generated {
        span: tex_state::DetachedGeneratedSourceSpan,
        fallback: tex_state::ResolvedSourceLocation,
    },
    Resolved(tex_state::ResolvedSourceLocation),
}

/// Failure-only, content-free snapshot captured before a failed step rolls
/// its live input and group stacks back.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FrozenDiagnosticContext {
    pub cause_kind: &'static str,
    pub input_frame_count: usize,
    pub input_frame_tail: Vec<&'static str>,
    pub group_depth: u32,
    pub group_tail: Vec<FrozenDiagnosticGroup>,
}

/// One bounded group-stack entry in [`FrozenDiagnosticContext`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrozenDiagnosticGroup {
    pub kind: &'static str,
    pub entered_line: u32,
}

/// Frozen source and causal-stack evidence kept behind the captured-error
/// allocation boundary so ordinary `Result<_, ExecError>` values stay small.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FrozenDiagnosticEvidence {
    pub origin: Option<FrozenDiagnosticOrigin>,
    pub context: Option<FrozenDiagnosticContext>,
}

impl FrozenDiagnosticContext {
    pub(crate) fn capture<G>(
        stores: &CommandContext<'_, G>,
        input_context: (usize, Vec<&'static str>),
        cause_kind: &'static str,
    ) -> Self {
        freeze_diagnostic_context(stores, input_context, cause_kind)
    }
}

#[derive(Debug)]
pub enum ExecError {
    ExecutionAlreadyTerminated,
    ExecutionCancelled,
    CumulativeFuelExceeded {
        limit: u64,
        attempted: u64,
    },
    ResourceBudgetExceeded {
        resource: &'static str,
        limit: u64,
        attempted: u64,
    },
    Captured {
        error: Box<ExecError>,
        site: DiagnosticSite,
        frozen: Option<Box<FrozenDiagnosticEvidence>>,
    },
    NeedResource(crate::ResolverResourceNeed),
    World(WorldError),
    FontParse(tex_fonts::ParseError),
    PdfFontMap(tex_fonts::PdfFontMapError),
    PdfGlyphToUnicode(String),
    FontOpen {
        name: String,
        message: String,
    },
    FontParameter(FontParameterError),
    FontExpansion(tex_typeset::expansion::FontExpansionError),
    FontExpansionConfig(tex_state::font::FontExpansionConfigError),
    CannotCopyFont(&'static str),
    OpenTypeMathUnsupported,
    EmptyModeNestSummary,
    CannotPopBaseMode,
    /// A mode level cannot be discarded until its buffered horizontal run has
    /// been materialized into the list it belongs to.
    UncommittedPendingHchars,
    UndefinedControlSequence {
        name: String,
        origin: OriginId,
    },
    UnexpectedMacroDelivery {
        name: String,
        origin: OriginId,
    },
    UnexpectedExpandableDelivery {
        token: Token,
        primitive: ExpandablePrimitive,
        origin: OriginId,
    },
    ExtraConditionalControl {
        primitive: ExpandablePrimitive,
        origin: OriginId,
    },
    ExtraEndCsName {
        origin: OriginId,
    },
    TooManyRightBraces {
        origin: OriginId,
    },
    ExtraRightBraceOrForgottenEndgroup {
        origin: OriginId,
    },
    ExtraRightBraceOrForgottenDollar {
        origin: OriginId,
    },
    ExtraEndGroup {
        origin: OriginId,
    },
    EndGroupMismatch {
        started_by: &'static str,
        origin: OriginId,
    },
    MathShiftGroupMismatch {
        started_by: &'static str,
        origin: OriginId,
    },
    UnsupportedCommand {
        token: Token,
        opcode: u8,
        origin: OriginId,
    },
    /// A `Meaning::UnexpandablePrimitive` variant reached `scan_command`'s
    /// exhaustive fallback classifier without a named dispatch arm.
    ///
    /// This is deliberately distinct from `UnsupportedCommand` (an opcode
    /// `meaning.rs` itself does not recognize): every variant here is a real,
    /// named TeX82/e-TeX/pdfTeX primitive that main control simply
    /// does not route yet in the current mode, either because no dispatch
    /// arm has been written for it or because it is legal only in a
    /// different mode (e.g. a math-noad primitive reached outside math
    /// mode). See `docs/tex_command_core.md`'s dispatch-completeness
    /// invariant and umber2-johp.69: converting this from a silent
    /// `ColdOperation::Continue` into a loud, named failure is the point --
    /// the alternative silently drops the primitive's own operand tokens
    /// into the document as literal text arbitrarily far downstream.
    UnimplementedPrimitive {
        primitive: UnexpandablePrimitive,
        mode: Mode,
        origin: OriginId,
    },
    /// A non-`UnexpandablePrimitive` `Meaning` variant reached
    /// `scan_command`'s exhaustive fallback classifier without a named
    /// dispatch arm.
    ///
    /// This is `UnimplementedPrimitive`'s sibling one level up the meaning
    /// word, and exists for the same reason (umber2-johp.108): main
    /// control's `Meaning`-level match used to end in a silent
    /// `_ => Ok(ColdOperation::Continue)`, which turned "this meaning has no
    /// dispatch" into "succeeded and consumed nothing" and left the
    /// command's own operand tokens to be typeset as literal text
    /// arbitrarily far downstream. Every variant reported here is a real
    /// TeX82/e-TeX/pdfTeX command class that main control does not
    /// route yet; the `Meaning` payload names the exact gap.
    UnimplementedMeaning {
        meaning: tex_state::meaning::Meaning,
        mode: Mode,
        origin: OriginId,
    },
    MissingPrefixedCommand,
    PrefixWithNonAssignment {
        token: Token,
        origin: OriginId,
    },
    PrefixWithNonDefinition {
        origin: Option<OriginId>,
    },
    MissingControlSequence {
        context: &'static str,
    },
    ExpectedControlSequence {
        context: &'static str,
        token: Token,
        origin: OriginId,
    },
    MissingToken {
        context: &'static str,
    },
    /// A command-core operation scanned an input name, but its
    /// borrow-scoped host capability has not supplied immutable bytes yet.
    MissingInput {
        name: String,
        original_name: String,
    },
    /// A non-opening file probe has not received bytes or authoritative
    /// absence from the retained host yet.
    MissingInputProbe {
        request: tex_command::FileEnquiryRequest,
    },
    /// A font definition completed scanning, but its transient
    /// host capability has not supplied the immutable resource yet.
    MissingFont {
        request: tex_command::FontLoadRequest,
    },
    MissingPdfImage {
        request: tex_command::PdfImageRequest,
    },
    MissingTracedToken {
        context: TracedTokenWord,
    },
    /// A command-core operation failed with a `CommandError` other
    /// than `MissingInput` or `PdfNavigation`, which map to their own
    /// dedicated variants above. This preserves the originating variant and
    /// message instead of collapsing it into a generic `MissingToken`.
    Command(CommandError),
    InvalidLetRhs {
        token: Token,
        origin: OriginId,
    },
    UnsupportedAssignmentTarget,
    RegisterNumberOutOfRange(i32),
    ArithmeticOverflow,
    InvalidCode {
        context: &'static str,
        value: i32,
    },
    BadPrevGraf(i32),
    MissingHashInAlignmentPreamble,
    ExtraHashInAlignmentPreamble,
    MisplacedOmit,
    MissingLeaderPayload {
        context: TracedTokenWord,
    },
    LeadersNotFollowedByProperGlue {
        context: TracedTokenWord,
    },
    HRuleHereExceptLeaders,
    CannotDeleteFromCurrentPage {
        command: &'static str,
    },
    ReadNeedsTo,
    ReadNotImplemented,
    FileEndedWithinRead,
    TerminalReadEof,
    UnimplementedTypesetting {
        mode: Mode,
        token: Token,
        origin: OriginId,
        operation: &'static str,
    },
    UnsupportedShipoutNode {
        node: &'static str,
    },
    InvalidShipoutArtifact(String),
    PdfOutputModeChanged,
    PdfVersionChanged,
    PdfDraftModeChanged,
    PdfObjectCapacity,
    PdfReferencedObjectNotFound,
    PdfXFormVoidBox,
    PdfImmediateReservedObject,
    PdfExtensionInDviMode(&'static str),
    PdfDeferredNodeInDviMode(&'static str),
    PdfDuplicateOpenAction,
    PdfImageOpen {
        name: String,
        message: String,
    },
    PdfActionTypeMissing,
    PdfActionOnlyGoto(&'static str),
    PdfActionIdentifierTypeMissing,
    PdfActionPositiveIdentifier(&'static str),
    PdfActionGotoFileNum,
    PdfActionWindowRequiresGotoFile,
    PdfEndLinkWithoutStart,
    PdfLinkInVerticalMode(&'static str),
    PdfDestinationIdentifierMissing,
    PdfDestinationKindMissing,
    PdfDestinationInForm,
    PdfThreadIdentifierMissing,
    PdfThreadInForm,
    /// A command-owned pdfTeX navigation scanner emitted an ext diagnostic.
    PdfNavigation(&'static str),
    VSplitNeedsVBox,
    Box255NotVoidBeforeOutput,
    OutputRoutineBox255NotVoid,
    OutputLoop {
        dead_cycles: i32,
    },
    /// TeX82 §93 `succumb`: `history:=fatal_error_stop; jump_out`.
    ///
    /// This variant is the Rust spelling of §81's non-local `goto end_of_TEX`.
    /// It propagates by `?` through every active frame exactly as `jump_out`
    /// cuts across every active procedure level, and the main-control driver
    /// -- the only frame that corresponds to `end_of_TEX` -- converts it into
    /// the session's terminal state instead of an error return. No other
    /// handler may catch it, recover from it, or roll back over it.
    Fatal(FatalError),
}

impl ExecError {
    /// The fatal payload this error is carrying, if any.
    ///
    /// `Captured` wraps an inner error with a diagnostic site, so the search
    /// has to look through it; a fatal error stays fatal however deeply the
    /// diagnostic machinery has annotated it.
    #[must_use]
    pub fn as_fatal(&self) -> Option<FatalError> {
        match self {
            Self::Fatal(fatal) | Self::Command(CommandError::Fatal(fatal)) => Some(*fatal),
            Self::Captured { error, .. } => error.as_fatal(),
            _ => None,
        }
    }

    /// Whether this is a navigation-family pdfTeX `pdf_error` which must
    /// cross §93 `succumb` before the driver returns the same typed failure.
    #[must_use]
    pub fn is_pdftex_navigation_fatal(&self) -> bool {
        match self {
            Self::PdfNavigation(_) => true,
            Self::Captured { error, .. } => error.is_pdftex_navigation_fatal(),
            _ => false,
        }
    }
}

impl fmt::Display for ExecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ExecutionAlreadyTerminated => f.write_str("execution run is already terminal"),
            Self::ExecutionCancelled => f.write_str("execution run was cancelled"),
            Self::CumulativeFuelExceeded { limit, attempted } => write!(
                f,
                "execution cumulative fuel limit {limit} exceeded at {attempted}"
            ),
            Self::ResourceBudgetExceeded {
                resource,
                limit,
                attempted,
            } => write!(
                f,
                "execution {resource} budget {limit} exceeded at {attempted}"
            ),
            Self::Captured { error, .. } => write!(f, "{error}"),
            Self::NeedResource(need) => write!(
                f,
                "resource request {} requires host resolution",
                need.request_index()
            ),
            Self::World(err) => write!(f, "{err}"),
            Self::FontParse(err) => write!(f, "{err}"),
            Self::PdfFontMap(err) => write!(f, "{err}"),
            Self::PdfGlyphToUnicode(message) => {
                write!(f, "pdfTeX error (\\pdfglyphtounicode): {message}")
            }
            Self::FontOpen { name, message } => {
                write!(f, "could not open TFM for font {name}: {message}")
            }
            Self::FontParameter(err) => write!(f, "{err:?}"),
            Self::FontExpansion(err) => write!(f, "pdfTeX error (font expansion): {err}"),
            Self::FontExpansionConfig(err) => {
                write!(f, "pdfTeX error (font expansion): {err}")
            }
            Self::CannotCopyFont(reason) => {
                write!(f, "pdfTeX error (\\pdfcopyfont): {reason}")
            }
            Self::OpenTypeMathUnsupported => write!(
                f,
                "OpenType-only fonts cannot be assigned to classic TeX math families; MATH parameter synthesis is not implemented"
            ),
            Self::EmptyModeNestSummary => write!(f, "mode nest summary has no levels"),
            Self::CannotPopBaseMode => write!(f, "cannot pop the base vertical mode level"),
            Self::UncommittedPendingHchars => write!(
                f,
                "cannot pop a mode level with uncommitted pending horizontal characters"
            ),
            Self::UndefinedControlSequence { name, .. } => {
                write!(f, "undefined control sequence \\{name}")
            }
            Self::UnexpectedMacroDelivery { name, .. } => {
                write!(f, "macro \\{name} reached execution without expansion")
            }
            Self::UnexpectedExpandableDelivery {
                token, primitive, ..
            } => write!(
                f,
                "expandable primitive {primitive:?} reached execution as delivered token {token:?}"
            ),
            Self::ExtraConditionalControl { primitive, .. } => {
                write!(f, "extra conditional control {primitive:?}")
            }
            Self::ExtraEndCsName { .. } => write!(f, "extra \\endcsname"),
            Self::TooManyRightBraces { .. } => write!(f, "Too many }}'s."),
            Self::ExtraRightBraceOrForgottenEndgroup { .. } => {
                write!(f, "Extra }}, or forgotten \\endgroup.")
            }
            Self::ExtraRightBraceOrForgottenDollar { .. } => {
                write!(f, "Extra }}, or forgotten $.")
            }
            Self::ExtraEndGroup { .. } => write!(f, "Extra \\endgroup."),
            Self::EndGroupMismatch { started_by, .. } => {
                write!(f, "\\endgroup ended a group started by {started_by}")
            }
            Self::MathShiftGroupMismatch { started_by, .. } => {
                write!(f, "$ ended a group started by {started_by}")
            }
            Self::UnsupportedCommand { token, opcode, .. } => {
                write!(
                    f,
                    "unsupported unexpandable opcode {opcode} for token {token:?}"
                )
            }
            Self::UnimplementedPrimitive { primitive, mode, .. } => write!(
                f,
                "main-control execution does not dispatch \\{primitive:?} in {mode:?} mode yet"
            ),
            Self::UnimplementedMeaning { meaning, mode, .. } => write!(
                f,
                "main-control execution does not dispatch {meaning:?} in {mode:?} mode yet"
            ),
            Self::MissingPrefixedCommand => write!(f, "You can't use a prefix with `end of input'"),
            Self::PrefixWithNonAssignment { token, .. } => {
                write!(f, "You can't use a prefix with `{token:?}'")
            }
            Self::PrefixWithNonDefinition { .. } => {
                write!(f, "You can't use a prefix with `\\let'")
            }
            Self::MissingControlSequence { context } => {
                write!(f, "missing control sequence after {context}")
            }
            Self::ExpectedControlSequence { context, token, .. } => {
                write!(
                    f,
                    "expected control sequence after {context}, got {token:?}"
                )
            }
            Self::MissingToken { context } => write!(f, "missing token while scanning {context}"),
            Self::MissingInput { name, .. } => {
                write!(f, "input source `{name}` is unavailable")
            }
            Self::MissingInputProbe { request } => {
                write!(f, "input enquiry `{}` is unresolved", request.name)
            }
            Self::MissingFont { request } => {
                write!(f, "font resource `{}` is unavailable", request.name)
            }
            Self::MissingPdfImage { request } => {
                write!(f, "image resource `{}` is unavailable", request.name)
            }
            Self::MissingTracedToken { .. } => f.write_str("missing token while scanning input"),
            Self::Command(err) => write!(f, "{err}"),
            Self::InvalidLetRhs { token, .. } => {
                write!(f, "\\let cannot assign macro parameter token {token:?}")
            }
            Self::UnsupportedAssignmentTarget => write!(f, "unsupported assignment target"),
            Self::RegisterNumberOutOfRange(value) => {
                write!(f, "register number {value} is out of range")
            }
            Self::ArithmeticOverflow => write!(f, "Arithmetic overflow"),
            Self::InvalidCode { context, value } => {
                write!(f, "Invalid code ({value}) while scanning {context}")
            }
            Self::BadPrevGraf(value) => write!(f, "Bad \\prevgraf ({value})"),
            Self::MissingHashInAlignmentPreamble => {
                write!(f, "Missing # inserted in alignment preamble.")
            }
            Self::ExtraHashInAlignmentPreamble => {
                write!(f, "Only one # is allowed per tab.")
            }
            Self::MisplacedOmit => write!(f, "Misplaced \\omit."),
            Self::MissingLeaderPayload { .. } => write!(f, "A <box> was supposed to be here."),
            Self::LeadersNotFollowedByProperGlue { .. } => {
                write!(f, "Leaders not followed by proper glue.")
            }
            Self::HRuleHereExceptLeaders => {
                write!(f, "You can't use `\\hrule' here except with leaders.")
            }
            Self::CannotDeleteFromCurrentPage { command } => {
                write!(f, "You can't use `{command}' in vertical mode.")
            }
            Self::ReadNeedsTo => write!(f, "Missing `to' inserted for \\read"),
            Self::ReadNotImplemented => write!(f, "I can't \\read from terminal in nonstop modes"),
            Self::FileEndedWithinRead => write!(f, "File ended within \\read"),
            Self::TerminalReadEof => write!(f, "End of file on the terminal"),
            Self::UnimplementedTypesetting {
                mode,
                token,
                operation,
                ..
            } => write!(
                f,
                "typesetting path is not implemented yet: {operation} in {mode:?} for token {token:?}"
            ),
            Self::UnsupportedShipoutNode { node } => {
                write!(
                    f,
                    "shipout artifact lowering does not support {node} nodes yet"
                )
            }
            Self::InvalidShipoutArtifact(error) => write!(f, "{error}"),
            Self::PdfOutputModeChanged => write!(
                f,
                "pdfTeX error (setup): \\pdfoutput can only be changed before anything is written to the output"
            ),
            Self::PdfVersionChanged => write!(
                f,
                "pdfTeX error (setup): PDF version cannot be changed after data is written to the PDF file"
            ),
            Self::PdfDraftModeChanged => write!(
                f,
                "pdfTeX error (setup): \\pdfdraftmode can only be changed before anything is written to the output"
            ),
            Self::PdfObjectCapacity => f.write_str("pdfTeX error (obj): too many PDF objects."),
            Self::PdfReferencedObjectNotFound => {
                f.write_str("pdfTeX error (ext1): cannot find referenced object.")
            }
            Self::PdfXFormVoidBox => {
                f.write_str("pdfTeX error (ext1): \\pdfxform cannot be used with a void box")
            }
            Self::PdfImmediateReservedObject => f.write_str(
                "pdfTeX error (ext1): `\\pdfobj reserveobjnum' cannot be used with \\immediate.",
            ),
            Self::PdfExtensionInDviMode(name) => write!(
                f,
                "pdfTeX error (\\{name}): not allowed in DVI mode (\\pdfoutput <= 0)."
            ),
            Self::PdfDeferredNodeInDviMode(name) => write!(
                f,
                "pdfTeX error (ext4): \\{name} used while \\pdfoutput is not set."
            ),
            Self::PdfDuplicateOpenAction => {
                f.write_str("pdfTeX error (ext1): duplicate of openaction")
            }
            Self::PdfImageOpen { name, message } => {
                write!(f, "pdfTeX error (ext5): cannot read image file {name}: {message}")
            }
            Self::PdfActionTypeMissing => f.write_str("pdfTeX error (ext1): action type missing"),
            Self::PdfActionOnlyGoto(option) => write!(
                f,
                "pdfTeX error (ext1): only GoTo action can be used with `{option}'"
            ),
            Self::PdfActionIdentifierTypeMissing => {
                f.write_str("pdfTeX error (ext1): identifier type missing")
            }
            Self::PdfActionPositiveIdentifier(kind) => {
                write!(f, "pdfTeX error (ext1): {kind} must be positive")
            }
            Self::PdfActionGotoFileNum => f.write_str(
                "pdfTeX error (ext1): `goto' option cannot be used with both `file' and `num'",
            ),
            Self::PdfActionWindowRequiresGotoFile => f.write_str(
                "pdfTeX error (ext1): `newwindow'/`nonewwindow' must be used with `goto' and `file' option",
            ),
            Self::PdfEndLinkWithoutStart => {
                f.write_str("pdfTeX error (ext1): \u{005c}pdfendlink without \u{005c}pdfstartlink")
            }
            Self::PdfLinkInVerticalMode(name) => {
                write!(
                    f,
                    "pdfTeX error (ext1): \\{name} cannot be used in vertical mode"
                )
            }
            Self::PdfDestinationIdentifierMissing => {
                f.write_str("pdfTeX error (ext4): destination identifier type missing")
            }
            Self::PdfDestinationKindMissing => {
                f.write_str("pdfTeX error (ext4): destination type missing")
            }
            Self::PdfDestinationInForm => {
                f.write_str("pdfTeX error (ext4): destinations cannot be inside an XForm")
            }
            Self::PdfThreadIdentifierMissing => {
                f.write_str("pdfTeX error (ext4): thread identifier type missing")
            }
            Self::PdfThreadInForm => {
                f.write_str("pdfTeX error (ext4): threads cannot be inside an XForm")
            }
            Self::PdfNavigation(message) => f.write_str(message),
            Self::VSplitNeedsVBox => write!(f, "\\vsplit needs a \\vbox"),
            Self::Box255NotVoidBeforeOutput => write!(f, "\\box255 is not void"),
            Self::OutputRoutineBox255NotVoid => {
                write!(f, "Output routine didn't use all of \\box255")
            }
            Self::OutputLoop { dead_cycles } => {
                write!(f, "Output loop---{dead_cycles} consecutive dead cycles")
            }
            Self::Fatal(fatal) => write!(f, "irrecoverable error: {fatal}"),
        }
    }
}

impl std::error::Error for ExecError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Captured { error, .. } => Some(error),
            Self::World(err) => Some(err),
            Self::FontParse(err) => Some(err),
            Self::PdfFontMap(err) => Some(err),
            Self::Command(err) => Some(err),
            Self::NeedResource(_)
            | Self::ExecutionAlreadyTerminated
            | Self::ExecutionCancelled
            | Self::CumulativeFuelExceeded { .. }
            | Self::ResourceBudgetExceeded { .. }
            | Self::FontOpen { .. }
            | Self::Fatal(_)
            | Self::PdfGlyphToUnicode(_)
            | Self::EmptyModeNestSummary
            | Self::CannotPopBaseMode
            | Self::UncommittedPendingHchars
            | Self::UndefinedControlSequence { .. }
            | Self::UnexpectedMacroDelivery { .. }
            | Self::UnexpectedExpandableDelivery { .. }
            | Self::ExtraConditionalControl { .. }
            | Self::ExtraEndCsName { .. }
            | Self::TooManyRightBraces { .. }
            | Self::ExtraRightBraceOrForgottenEndgroup { .. }
            | Self::ExtraRightBraceOrForgottenDollar { .. }
            | Self::ExtraEndGroup { .. }
            | Self::EndGroupMismatch { .. }
            | Self::MathShiftGroupMismatch { .. }
            | Self::UnsupportedCommand { .. }
            | Self::UnimplementedPrimitive { .. }
            | Self::UnimplementedMeaning { .. }
            | Self::MissingPrefixedCommand
            | Self::PrefixWithNonAssignment { .. }
            | Self::PrefixWithNonDefinition { .. }
            | Self::MissingControlSequence { .. }
            | Self::ExpectedControlSequence { .. }
            | Self::MissingToken { .. }
            | Self::MissingInput { .. }
            | Self::MissingInputProbe { .. }
            | Self::MissingFont { .. }
            | Self::MissingPdfImage { .. }
            | Self::MissingTracedToken { .. }
            | Self::InvalidLetRhs { .. }
            | Self::UnsupportedAssignmentTarget
            | Self::RegisterNumberOutOfRange(_)
            | Self::ArithmeticOverflow
            | Self::InvalidCode { .. }
            | Self::BadPrevGraf(_)
            | Self::MissingHashInAlignmentPreamble
            | Self::ExtraHashInAlignmentPreamble
            | Self::MisplacedOmit
            | Self::MissingLeaderPayload { .. }
            | Self::LeadersNotFollowedByProperGlue { .. }
            | Self::HRuleHereExceptLeaders
            | Self::CannotDeleteFromCurrentPage { .. }
            | Self::ReadNeedsTo
            | Self::ReadNotImplemented
            | Self::FileEndedWithinRead
            | Self::TerminalReadEof
            | Self::FontParameter(_)
            | Self::FontExpansion(_)
            | Self::FontExpansionConfig(_)
            | Self::CannotCopyFont(_)
            | Self::OpenTypeMathUnsupported
            | Self::UnimplementedTypesetting { .. }
            | Self::UnsupportedShipoutNode { .. }
            | Self::InvalidShipoutArtifact(_)
            | Self::PdfOutputModeChanged
            | Self::PdfVersionChanged
            | Self::PdfDraftModeChanged
            | Self::PdfObjectCapacity
            | Self::PdfReferencedObjectNotFound
            | Self::PdfXFormVoidBox
            | Self::PdfImmediateReservedObject
            | Self::PdfExtensionInDviMode(_)
            | Self::PdfDeferredNodeInDviMode(_)
            | Self::PdfDuplicateOpenAction
            | Self::PdfImageOpen { .. }
            | Self::PdfActionTypeMissing
            | Self::PdfActionOnlyGoto(_)
            | Self::PdfActionIdentifierTypeMissing
            | Self::PdfActionPositiveIdentifier(_)
            | Self::PdfActionGotoFileNum
            | Self::PdfActionWindowRequiresGotoFile
            | Self::PdfEndLinkWithoutStart
            | Self::PdfLinkInVerticalMode(_)
            | Self::PdfDestinationIdentifierMissing
            | Self::PdfDestinationKindMissing
            | Self::PdfDestinationInForm
            | Self::PdfThreadIdentifierMissing
            | Self::PdfThreadInForm
            | Self::PdfNavigation(_)
            | Self::VSplitNeedsVBox
            | Self::Box255NotVoidBeforeOutput
            | Self::OutputRoutineBox255NotVoid
            | Self::OutputLoop { .. } => None,
        }
    }
}

impl ExecError {
    #[must_use]
    pub fn primary_origin(&self) -> Option<OriginId> {
        match self {
            Self::Captured { site, .. } => site.primary_origin(),
            Self::NeedResource(_)
            | Self::ExecutionAlreadyTerminated
            | Self::ExecutionCancelled
            | Self::CumulativeFuelExceeded { .. }
            | Self::ResourceBudgetExceeded { .. } => None,
            Self::UndefinedControlSequence { origin, .. }
            | Self::UnexpectedMacroDelivery { origin, .. }
            | Self::UnexpectedExpandableDelivery { origin, .. }
            | Self::ExtraConditionalControl { origin, .. }
            | Self::ExtraEndCsName { origin }
            | Self::TooManyRightBraces { origin }
            | Self::ExtraRightBraceOrForgottenEndgroup { origin }
            | Self::ExtraRightBraceOrForgottenDollar { origin }
            | Self::ExtraEndGroup { origin }
            | Self::EndGroupMismatch { origin, .. }
            | Self::MathShiftGroupMismatch { origin, .. }
            | Self::UnsupportedCommand { origin, .. }
            | Self::UnimplementedPrimitive { origin, .. }
            | Self::UnimplementedMeaning { origin, .. }
            | Self::PrefixWithNonAssignment { origin, .. }
            | Self::ExpectedControlSequence { origin, .. }
            | Self::InvalidLetRhs { origin, .. }
            | Self::UnimplementedTypesetting { origin, .. } => Some(*origin),
            Self::MissingTracedToken { context } => Some(context.origin()),
            Self::MissingLeaderPayload { context }
            | Self::LeadersNotFollowedByProperGlue { context } => Some(context.origin()),
            Self::PrefixWithNonDefinition { origin } => *origin,
            Self::World(_)
            | Self::FontParse(_)
            | Self::PdfFontMap(_)
            | Self::PdfGlyphToUnicode(_)
            | Self::FontOpen { .. }
            | Self::FontParameter(_)
            | Self::FontExpansion(_)
            | Self::FontExpansionConfig(_)
            | Self::CannotCopyFont(_)
            | Self::OpenTypeMathUnsupported
            | Self::EmptyModeNestSummary
            | Self::CannotPopBaseMode
            | Self::UncommittedPendingHchars
            | Self::MissingPrefixedCommand
            | Self::MissingControlSequence { .. }
            | Self::MissingToken { .. }
            | Self::MissingInput { .. }
            | Self::MissingInputProbe { .. }
            | Self::MissingFont { .. }
            | Self::MissingPdfImage { .. }
            | Self::Fatal(_)
            | Self::Command(_)
            | Self::UnsupportedAssignmentTarget
            | Self::RegisterNumberOutOfRange(_)
            | Self::ArithmeticOverflow
            | Self::InvalidCode { .. }
            | Self::BadPrevGraf(_)
            | Self::MissingHashInAlignmentPreamble
            | Self::ExtraHashInAlignmentPreamble
            | Self::MisplacedOmit
            | Self::HRuleHereExceptLeaders
            | Self::CannotDeleteFromCurrentPage { .. }
            | Self::ReadNeedsTo
            | Self::ReadNotImplemented
            | Self::FileEndedWithinRead
            | Self::TerminalReadEof
            | Self::UnsupportedShipoutNode { .. }
            | Self::InvalidShipoutArtifact(_)
            | Self::PdfOutputModeChanged
            | Self::PdfVersionChanged
            | Self::PdfDraftModeChanged
            | Self::PdfObjectCapacity
            | Self::PdfReferencedObjectNotFound
            | Self::PdfXFormVoidBox
            | Self::PdfImmediateReservedObject
            | Self::PdfExtensionInDviMode(_)
            | Self::PdfDeferredNodeInDviMode(_)
            | Self::PdfDuplicateOpenAction
            | Self::PdfImageOpen { .. }
            | Self::PdfActionTypeMissing
            | Self::PdfActionOnlyGoto(_)
            | Self::PdfActionIdentifierTypeMissing
            | Self::PdfActionPositiveIdentifier(_)
            | Self::PdfActionGotoFileNum
            | Self::PdfActionWindowRequiresGotoFile
            | Self::PdfEndLinkWithoutStart
            | Self::PdfLinkInVerticalMode(_)
            | Self::PdfDestinationIdentifierMissing
            | Self::PdfDestinationKindMissing
            | Self::PdfDestinationInForm
            | Self::PdfThreadIdentifierMissing
            | Self::PdfThreadInForm
            | Self::PdfNavigation(_)
            | Self::VSplitNeedsVBox
            | Self::Box255NotVoidBeforeOutput
            | Self::OutputRoutineBox255NotVoid
            | Self::OutputLoop { .. } => None,
        }
    }

    #[must_use]
    pub fn diagnostic_site(&self) -> DiagnosticSite {
        match self {
            Self::Captured { site, .. } => site.clone(),
            _ => DiagnosticSite::new(self.primary_origin(), [], None),
        }
    }

    /// Attaches the command delivery which owns a failed step when
    /// the error did not already identify a more specific source origin.
    /// Scanner and expansion errors keep their nested diagnostic site; only
    /// an originless boundary error inherits the triggering command span.
    pub(crate) fn capture_command_origin(self, origin: OriginId) -> Self {
        if matches!(
            self,
            Self::NeedResource(_) | Self::MissingFont { .. } | Self::MissingPdfImage { .. }
        ) {
            return self;
        }
        let inherited = self.diagnostic_site();
        if inherited.primary_origin().is_some() {
            return self;
        }
        Self::Captured {
            error: Box::new(self),
            site: DiagnosticSite::new(
                Some(origin),
                inherited.related().iter().copied(),
                inherited.expansion_head(),
            ),
            frozen: None,
        }
    }

    /// Captures the triggering delivery's structural root. Formatting still
    /// walks and renders it only when a diagnostic consumer requests text.
    pub(crate) fn capture_command_origin_ref(self, origin: OriginRef) -> Self {
        if matches!(
            self,
            Self::NeedResource(_) | Self::MissingFont { .. } | Self::MissingPdfImage { .. }
        ) {
            return self;
        }
        let inherited = self.diagnostic_site();
        if inherited.primary_origin().is_some() {
            return self;
        }
        Self::Captured {
            error: Box::new(self),
            site: DiagnosticSite::rooted(Some(origin), [], None),
            frozen: None,
        }
    }

    /// Freezes the primary diagnostic origin without retaining speculative
    /// provenance arena entries past rollback.
    pub(crate) fn freeze_diagnostic_origin<G>(
        mut self,
        stores: &mut CommandContext<'_, G>,
        input_context: (usize, Vec<&'static str>),
    ) -> Self {
        if let Self::Captured { site, frozen, .. } = &mut self
            && frozen.is_none()
        {
            *frozen = Some(Box::new(FrozenDiagnosticEvidence {
                origin: site
                    .primary_origin()
                    .and_then(|origin| freeze_diagnostic_origin(stores, origin)),
                context: Some(freeze_diagnostic_context(
                    stores,
                    input_context.clone(),
                    "terminal-execution",
                )),
            }));
        }
        if matches!(self, Self::Captured { .. } | Self::PdfXFormVoidBox)
            || self.as_fatal().is_none()
        {
            self
        } else {
            let site = self.diagnostic_site();
            let frozen = site
                .primary_origin()
                .and_then(|origin| freeze_diagnostic_origin(stores, origin));
            Self::Captured {
                error: Box::new(self),
                site,
                frozen: Some(Box::new(FrozenDiagnosticEvidence {
                    origin: frozen,
                    context: Some(freeze_diagnostic_context(
                        stores,
                        input_context,
                        "terminal-execution",
                    )),
                })),
            }
        }
    }

    #[must_use]
    pub fn frozen_diagnostic_origin(&self) -> Option<&FrozenDiagnosticOrigin> {
        match self {
            Self::Captured { frozen, .. } => frozen.as_deref()?.origin.as_ref(),
            _ => None,
        }
    }

    #[must_use]
    pub fn frozen_diagnostic_context(&self) -> Option<&FrozenDiagnosticContext> {
        match self {
            Self::Captured { frozen, .. } => frozen.as_deref()?.context.as_ref(),
            _ => None,
        }
    }

    /// Renders this error with lazy provenance context from the live universe.
    #[must_use]
    pub fn format_with_provenance<G>(&self, stores: &mut CommandContext<'_, G>) -> String {
        let message = self.message_with_token_names(stores);
        let Some(origin) = self.diagnostic_site().primary_origin() else {
            return message;
        };
        stores
            .detach_diagnostic_origin(
                origin,
                DiagnosticOriginRequest {
                    demand: ColdProvenanceDemand::Diagnostic,
                    message: &message,
                },
            )
            .map_or(message, |diagnostic| diagnostic.rendered_site)
    }

    fn message_with_token_names<G>(&self, stores: &CommandContext<'_, G>) -> String {
        match self {
            Self::Captured { error, .. } => error.message_with_token_names(stores),
            Self::UnimplementedTypesetting {
                mode,
                token,
                operation,
                ..
            } => format!(
                "typesetting path is not implemented yet: {operation} in {mode:?} for token {}",
                tex_state::token_show::token_text(stores, *token)
            ),
            _ => self.to_string(),
        }
    }
}

fn freeze_diagnostic_origin<G>(
    stores: &mut CommandContext<'_, G>,
    origin: OriginId,
) -> Option<FrozenDiagnosticOrigin> {
    let detached = stores
        .detach_diagnostic_origin(
            origin,
            DiagnosticOriginRequest {
                demand: ColdProvenanceDemand::Diagnostic,
                message: "",
            },
        )
        .ok()?;
    match (detached.generated_origin, detached.resolved_source) {
        (Some(span), Some(fallback)) => Some(FrozenDiagnosticOrigin::Generated { span, fallback }),
        (_, Some(location)) => Some(FrozenDiagnosticOrigin::Resolved(location)),
        _ => None,
    }
}

fn freeze_diagnostic_context<G>(
    stores: &CommandContext<'_, G>,
    input_context: (usize, Vec<&'static str>),
    cause_kind: &'static str,
) -> FrozenDiagnosticContext {
    const TAIL_LIMIT: usize = 8;

    FrozenDiagnosticContext {
        cause_kind,
        input_frame_count: input_context.0,
        input_frame_tail: input_context.1.into_iter().take(TAIL_LIMIT).collect(),
        group_depth: u32::try_from(stores.group_frames().len()).unwrap_or(u32::MAX),
        group_tail: stores
            .group_frames()
            .iter()
            .rev()
            .take(TAIL_LIMIT)
            .map(|frame| FrozenDiagnosticGroup {
                kind: diagnostic_group_kind(frame.kind()),
                entered_line: frame.entered_line(),
            })
            .collect(),
    }
}

const fn diagnostic_group_kind(kind: tex_state::GroupKind) -> &'static str {
    use tex_state::GroupKind as Kind;
    match kind {
        Kind::Simple => "simple",
        Kind::HBox => "hbox",
        Kind::AdjustedHBox => "adjusted-hbox",
        Kind::VBox => "vbox",
        Kind::VTop => "vtop",
        Kind::SemiSimple => "semi-simple",
        Kind::MathShift => "math-shift",
        Kind::Align => "align",
        Kind::NoAlign => "no-align",
        Kind::Output => "output",
        Kind::Math => "math",
        Kind::Disc => "disc",
        Kind::Insert => "insert",
        Kind::VCenter => "vcenter",
        Kind::MathChoice => "math-choice",
        Kind::MathLeft => "math-left",
    }
}

impl From<tex_state::print::JumpOut> for ExecError {
    /// Lets an executor site write `report.error().jump_out()?` and have `?`
    /// carry tex.web §81's non-local exit up to the driver, which is the one
    /// frame that corresponds to `end_of_TEX`.
    fn from(jump: tex_state::print::JumpOut) -> Self {
        Self::Fatal(jump.into())
    }
}

impl From<WorldError> for ExecError {
    fn from(value: WorldError) -> Self {
        Self::World(value)
    }
}

impl From<tex_fonts::ParseError> for ExecError {
    fn from(value: tex_fonts::ParseError) -> Self {
        Self::FontParse(value)
    }
}

impl From<tex_fonts::PdfFontMapError> for ExecError {
    fn from(value: tex_fonts::PdfFontMapError) -> Self {
        Self::PdfFontMap(value)
    }
}

impl From<tex_out::SerializeError> for ExecError {
    fn from(error: tex_out::SerializeError) -> Self {
        Self::InvalidShipoutArtifact(error.to_string())
    }
}

impl From<FontParameterError> for ExecError {
    fn from(value: FontParameterError) -> Self {
        Self::FontParameter(value)
    }
}

impl From<tex_typeset::expansion::FontExpansionError> for ExecError {
    fn from(value: tex_typeset::expansion::FontExpansionError) -> Self {
        Self::FontExpansion(value)
    }
}

impl From<tex_state::font::FontExpansionConfigError> for ExecError {
    fn from(value: tex_state::font::FontExpansionConfigError) -> Self {
        Self::FontExpansionConfig(value)
    }
}
