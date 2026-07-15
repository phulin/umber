use std::fmt;

use tex_expand::ExpandError;
use tex_expand::scan::ScanToksError;
use tex_lex::LexError;
use tex_state::FontParameterError;
use tex_state::ProvenanceResolver;
use tex_state::Universe;
use tex_state::WorldError;
use tex_state::meaning::ExpandablePrimitive;
use tex_state::provenance::DiagnosticSite;
use tex_state::token::{OriginId, Token, TracedTokenWord};

use crate::Mode;

#[derive(Debug)]
pub enum ExecError {
    Captured {
        error: Box<ExecError>,
        site: DiagnosticSite,
    },
    Expand(ExpandError),
    Lex(LexError),
    ScanToks(ScanToksError),
    ScanGlue(tex_expand::scan_glue::ScanGlueError),
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
    EmptyModeNestSummary,
    CannotPopBaseMode,
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
    MissingTracedToken {
        context: TracedTokenWord,
    },
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
    MisplacedNoAlign,
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
    VSplitNeedsVBox,
    Box255NotVoidBeforeOutput,
    OutputRoutineBox255NotVoid,
    OutputLoop {
        dead_cycles: i32,
    },
}

impl fmt::Display for ExecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Captured { error, .. } => write!(f, "{error}"),
            Self::Expand(err) => write!(f, "{err}"),
            Self::Lex(err) => write!(f, "{err}"),
            Self::ScanToks(err) => write!(f, "{err}"),
            Self::ScanGlue(err) => write!(f, "{err}"),
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
            Self::EmptyModeNestSummary => write!(f, "mode nest summary has no levels"),
            Self::CannotPopBaseMode => write!(f, "cannot pop the base vertical mode level"),
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
            Self::MissingTracedToken { .. } => f.write_str("missing token while scanning input"),
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
            Self::MisplacedNoAlign => write!(f, "Misplaced \\noalign."),
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
                write!(f, "You can't use `\\{name}' in vertical mode.")
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
            Self::VSplitNeedsVBox => write!(f, "\\vsplit needs a \\vbox"),
            Self::Box255NotVoidBeforeOutput => write!(f, "\\box255 is not void"),
            Self::OutputRoutineBox255NotVoid => {
                write!(f, "Output routine didn't use all of \\box255")
            }
            Self::OutputLoop { dead_cycles } => {
                write!(f, "Output loop---{dead_cycles} consecutive dead cycles")
            }
        }
    }
}

impl std::error::Error for ExecError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Captured { error, .. } => Some(error),
            Self::Expand(err) => Some(err),
            Self::Lex(err) => Some(err),
            Self::ScanToks(err) => Some(err),
            Self::ScanGlue(err) => Some(err),
            Self::World(err) => Some(err),
            Self::FontParse(err) => Some(err),
            Self::PdfFontMap(err) => Some(err),
            Self::FontOpen { .. }
            | Self::PdfGlyphToUnicode(_)
            | Self::EmptyModeNestSummary
            | Self::CannotPopBaseMode
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
            | Self::MissingPrefixedCommand
            | Self::PrefixWithNonAssignment { .. }
            | Self::PrefixWithNonDefinition { .. }
            | Self::MissingControlSequence { .. }
            | Self::ExpectedControlSequence { .. }
            | Self::MissingToken { .. }
            | Self::MissingTracedToken { .. }
            | Self::InvalidLetRhs { .. }
            | Self::UnsupportedAssignmentTarget
            | Self::RegisterNumberOutOfRange(_)
            | Self::ArithmeticOverflow
            | Self::InvalidCode { .. }
            | Self::BadPrevGraf(_)
            | Self::MissingHashInAlignmentPreamble
            | Self::ExtraHashInAlignmentPreamble
            | Self::MisplacedNoAlign
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
            Self::Expand(err) => err.primary_origin(),
            Self::ScanGlue(err) => err.primary_origin(),
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
            | Self::PrefixWithNonAssignment { origin, .. }
            | Self::ExpectedControlSequence { origin, .. }
            | Self::InvalidLetRhs { origin, .. }
            | Self::UnimplementedTypesetting { origin, .. } => Some(*origin),
            Self::MissingTracedToken { context } => Some(context.origin()),
            Self::MissingLeaderPayload { context }
            | Self::LeadersNotFollowedByProperGlue { context } => Some(context.origin()),
            Self::PrefixWithNonDefinition { origin } => *origin,
            Self::Lex(err) => err.diagnostic_site().primary_origin(),
            Self::ScanToks(_)
            | Self::World(_)
            | Self::FontParse(_)
            | Self::PdfFontMap(_)
            | Self::PdfGlyphToUnicode(_)
            | Self::FontOpen { .. }
            | Self::FontParameter(_)
            | Self::FontExpansion(_)
            | Self::FontExpansionConfig(_)
            | Self::CannotCopyFont(_)
            | Self::EmptyModeNestSummary
            | Self::CannotPopBaseMode
            | Self::MissingPrefixedCommand
            | Self::MissingControlSequence { .. }
            | Self::MissingToken { .. }
            | Self::UnsupportedAssignmentTarget
            | Self::RegisterNumberOutOfRange(_)
            | Self::ArithmeticOverflow
            | Self::InvalidCode { .. }
            | Self::BadPrevGraf(_)
            | Self::MissingHashInAlignmentPreamble
            | Self::ExtraHashInAlignmentPreamble
            | Self::MisplacedNoAlign
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
            Self::Lex(err) => err.diagnostic_site().clone(),
            Self::Expand(err) => err.diagnostic_site(),
            _ => DiagnosticSite::new(self.primary_origin(), [], None),
        }
    }

    pub(crate) fn capture(self, input: &tex_lex::InputStack) -> Self {
        if matches!(self, Self::Captured { .. }) {
            return self;
        }
        let inherited = self.diagnostic_site();
        if inherited.expansion_head().is_some() {
            return Self::Captured {
                error: Box::new(self),
                site: inherited,
            };
        }
        let site =
            input.diagnostic_site(self.primary_origin(), inherited.related().iter().copied());
        if site.expansion_head().is_none() {
            self
        } else {
            Self::Captured {
                error: Box::new(self),
                site,
            }
        }
    }

    /// Renders this error with lazy provenance context from the live universe.
    #[must_use]
    pub fn format_with_provenance(&self, stores: &Universe) -> String {
        ProvenanceResolver::new(stores).render_diagnostic_site(
            &self.message_with_token_names(stores),
            &self.diagnostic_site(),
        )
    }

    fn message_with_token_names(&self, stores: &Universe) -> String {
        match self {
            Self::Captured { error, .. } => error.message_with_token_names(stores),
            Self::UnimplementedTypesetting {
                mode,
                token,
                operation,
                ..
            } => format!(
                "typesetting path is not implemented yet: {operation} in {mode:?} for token {}",
                tex_expand::token_text(stores, *token)
            ),
            _ => self.to_string(),
        }
    }
}

impl From<ExpandError> for ExecError {
    fn from(value: ExpandError) -> Self {
        Self::Expand(value)
    }
}

impl From<LexError> for ExecError {
    fn from(value: LexError) -> Self {
        Self::Lex(value)
    }
}

impl From<ScanToksError> for ExecError {
    fn from(value: ScanToksError) -> Self {
        Self::ScanToks(value)
    }
}

impl From<tex_expand::scan_glue::ScanGlueError> for ExecError {
    fn from(value: tex_expand::scan_glue::ScanGlueError) -> Self {
        Self::ScanGlue(value)
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
