use std::fmt;

use tex_expand::ExpandError;
use tex_expand::scan::ScanToksError;
use tex_lex::LexError;
use tex_state::WorldError;
use tex_state::meaning::ExpandablePrimitive;
use tex_state::token::Token;

use crate::Mode;

#[derive(Debug)]
pub enum ExecError {
    Expand(ExpandError),
    Lex(LexError),
    ScanToks(ScanToksError),
    ScanGlue(tex_expand::scan_glue::ScanGlueError),
    World(WorldError),
    EmptyModeNestSummary,
    CannotPopBaseMode,
    UndefinedControlSequence {
        name: String,
    },
    UnexpectedMacroDelivery {
        name: String,
    },
    UnexpectedExpandableDelivery {
        token: Token,
        primitive: ExpandablePrimitive,
    },
    ExtraConditionalControl(ExpandablePrimitive),
    ExtraEndCsName,
    TooManyRightBraces,
    ExtraRightBraceOrForgottenEndgroup,
    ExtraEndGroup,
    EndGroupMismatch {
        started_by: &'static str,
    },
    UnsupportedCommand {
        token: Token,
        opcode: u8,
    },
    MissingPrefixedCommand,
    PrefixWithNonAssignment {
        token: Token,
    },
    PrefixWithNonDefinition,
    MissingControlSequence {
        context: &'static str,
    },
    ExpectedControlSequence {
        context: &'static str,
        token: Token,
    },
    MissingToken {
        context: &'static str,
    },
    InvalidLetRhs {
        token: Token,
    },
    UnsupportedAssignmentTarget,
    RegisterNumberOutOfRange(i32),
    ArithmeticOverflow,
    InvalidCode {
        context: &'static str,
        value: i32,
    },
    ReadNeedsTo,
    ReadNotImplemented,
    FileEndedWithinRead,
    TerminalReadEof,
    UnimplementedTypesetting {
        mode: Mode,
        token: Token,
        operation: &'static str,
    },
}

impl fmt::Display for ExecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Expand(err) => write!(f, "{err}"),
            Self::Lex(err) => write!(f, "{err}"),
            Self::ScanToks(err) => write!(f, "{err}"),
            Self::ScanGlue(err) => write!(f, "{err}"),
            Self::World(err) => write!(f, "{err}"),
            Self::EmptyModeNestSummary => write!(f, "mode nest summary has no levels"),
            Self::CannotPopBaseMode => write!(f, "cannot pop the base vertical mode level"),
            Self::UndefinedControlSequence { name } => {
                write!(f, "undefined control sequence \\{name}")
            }
            Self::UnexpectedMacroDelivery { name } => {
                write!(f, "macro \\{name} reached execution without expansion")
            }
            Self::UnexpectedExpandableDelivery { token, primitive } => write!(
                f,
                "expandable primitive {primitive:?} reached execution as delivered token {token:?}"
            ),
            Self::ExtraConditionalControl(primitive) => {
                write!(f, "extra conditional control {primitive:?}")
            }
            Self::ExtraEndCsName => write!(f, "extra \\endcsname"),
            Self::TooManyRightBraces => write!(f, "Too many }}'s."),
            Self::ExtraRightBraceOrForgottenEndgroup => {
                write!(f, "Extra }}, or forgotten \\endgroup.")
            }
            Self::ExtraEndGroup => write!(f, "Extra \\endgroup."),
            Self::EndGroupMismatch { started_by } => {
                write!(f, "\\endgroup ended a group started by {started_by}")
            }
            Self::UnsupportedCommand { token, opcode } => {
                write!(
                    f,
                    "unsupported unexpandable opcode {opcode} for token {token:?}"
                )
            }
            Self::MissingPrefixedCommand => write!(f, "You can't use a prefix with `end of input'"),
            Self::PrefixWithNonAssignment { token } => {
                write!(f, "You can't use a prefix with `{token:?}'")
            }
            Self::PrefixWithNonDefinition => write!(f, "You can't use a prefix with `\\let'"),
            Self::MissingControlSequence { context } => {
                write!(f, "missing control sequence after {context}")
            }
            Self::ExpectedControlSequence { context, token } => {
                write!(
                    f,
                    "expected control sequence after {context}, got {token:?}"
                )
            }
            Self::MissingToken { context } => write!(f, "missing token while scanning {context}"),
            Self::InvalidLetRhs { token } => {
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
            Self::ReadNeedsTo => write!(f, "Missing `to' inserted for \\read"),
            Self::ReadNotImplemented => write!(f, "I can't \\read from terminal in nonstop modes"),
            Self::FileEndedWithinRead => write!(f, "File ended within \\read"),
            Self::TerminalReadEof => write!(f, "End of file on the terminal"),
            Self::UnimplementedTypesetting {
                mode,
                token,
                operation,
            } => write!(
                f,
                "typesetting path is not implemented yet: {operation} in {mode:?} for token {token:?}"
            ),
        }
    }
}

impl std::error::Error for ExecError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Expand(err) => Some(err),
            Self::Lex(err) => Some(err),
            Self::ScanToks(err) => Some(err),
            Self::ScanGlue(err) => Some(err),
            Self::World(err) => Some(err),
            Self::EmptyModeNestSummary
            | Self::CannotPopBaseMode
            | Self::UndefinedControlSequence { .. }
            | Self::UnexpectedMacroDelivery { .. }
            | Self::UnexpectedExpandableDelivery { .. }
            | Self::ExtraConditionalControl(_)
            | Self::ExtraEndCsName
            | Self::TooManyRightBraces
            | Self::ExtraRightBraceOrForgottenEndgroup
            | Self::ExtraEndGroup
            | Self::EndGroupMismatch { .. }
            | Self::UnsupportedCommand { .. }
            | Self::MissingPrefixedCommand
            | Self::PrefixWithNonAssignment { .. }
            | Self::PrefixWithNonDefinition
            | Self::MissingControlSequence { .. }
            | Self::ExpectedControlSequence { .. }
            | Self::MissingToken { .. }
            | Self::InvalidLetRhs { .. }
            | Self::UnsupportedAssignmentTarget
            | Self::RegisterNumberOutOfRange(_)
            | Self::ArithmeticOverflow
            | Self::InvalidCode { .. }
            | Self::ReadNeedsTo
            | Self::ReadNotImplemented
            | Self::FileEndedWithinRead
            | Self::TerminalReadEof
            | Self::UnimplementedTypesetting { .. } => None,
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
