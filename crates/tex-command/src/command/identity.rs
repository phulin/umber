//! Demanded observer and diagnostic command identity.

use super::CurrentCommand;
use tex_state::meaning::ExpandablePrimitive;

/// The command-code identity of a current delivery.
///
/// TeX82's `get_next` gives an expandable control sequence replayed by
/// `\\noexpand` the inaccessible frozen-`\\relax` command identity: its
/// effective meaning is `relax`, but its `cur_chr` is `no_expand_flag` (257),
/// rather than the ordinary `relax` value (256). Separately, TeX82 §25's
/// `\expandafter`, `\csname`, and its `\endcsname` boundary have dedicated
/// command identities rather than the generic expandable fallback. TeX82's
/// text conversions likewise share the `convert` command with a selector
/// owned by the delivered primitive. Its diagnostic commands likewise share
/// `xray`, with a selector installed by the primitive table. These remain
/// ephemeral with the current delivery and are never stored in snapshots or
/// input payloads.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum CommandIdentity {
    Ordinary,
    ExpandAfter,
    CsName,
    EndCsName,
    Convert(ConvertSelector),
    XRay(XRaySelector),
    NoExpandFrozenRelax,
}

/// TeX82's `convert` selectors.
///
/// TeX.web §35 installs the classic conversion primitives with these values;
/// §27's `conv_toks` then consumes the selector while retaining the original
/// `convert` current-command identity throughout the conversion episode.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum ConvertSelector {
    Number,
    RomanNumeral,
    String,
    Meaning,
    FontName,
    JobName,
}

impl ConvertSelector {
    const fn from_primitive(primitive: tex_state::meaning::ExpandablePrimitive) -> Option<Self> {
        use tex_state::meaning::ExpandablePrimitive;

        match primitive {
            ExpandablePrimitive::Number => Some(Self::Number),
            ExpandablePrimitive::RomanNumeral => Some(Self::RomanNumeral),
            ExpandablePrimitive::String => Some(Self::String),
            ExpandablePrimitive::Meaning => Some(Self::Meaning),
            ExpandablePrimitive::FontName => Some(Self::FontName),
            ExpandablePrimitive::JobName => Some(Self::JobName),
            _ => None,
        }
    }

    pub(crate) const fn operand(self) -> i64 {
        match self {
            Self::Number => 0,
            Self::RomanNumeral => 1,
            Self::String => 2,
            Self::Meaning => 3,
            Self::FontName => 4,
            Self::JobName => 5,
        }
    }
}

/// TeX82's `xray` selectors.
///
/// TeX.web §18 installs the diagnostic primitives under the shared `xray`
/// command: `\show`, `\showbox`, `\showthe`, and `\showlists`. Their
/// selector is current-command identity; the executor still owns each
/// primitive's distinct typed diagnostic behavior.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum XRaySelector {
    Show,
    ShowBox,
    ShowThe,
    ShowLists,
}

impl XRaySelector {
    const fn from_primitive(primitive: tex_state::meaning::UnexpandablePrimitive) -> Option<Self> {
        use tex_state::meaning::UnexpandablePrimitive;

        match primitive {
            UnexpandablePrimitive::Show => Some(Self::Show),
            UnexpandablePrimitive::ShowBox => Some(Self::ShowBox),
            UnexpandablePrimitive::ShowThe => Some(Self::ShowThe),
            UnexpandablePrimitive::ShowLists => Some(Self::ShowLists),
            _ => None,
        }
    }

    pub(crate) const fn operand(self) -> i64 {
        match self {
            Self::Show => 0,
            Self::ShowBox => 1,
            Self::ShowThe => 2,
            Self::ShowLists => 3,
        }
    }
}

impl CommandIdentity {
    pub(super) fn from_command<G>(command: &CurrentCommand<G>) -> Self {
        if let Some(primitive) = command.expandable_primitive() {
            return match primitive {
                ExpandablePrimitive::ExpandAfter => Self::ExpandAfter,
                ExpandablePrimitive::CsName => Self::CsName,
                ExpandablePrimitive::EndCsName => Self::EndCsName,
                other => {
                    ConvertSelector::from_primitive(other).map_or(Self::Ordinary, Self::Convert)
                }
            };
        }
        command
            .unexpandable_primitive()
            .and_then(XRaySelector::from_primitive)
            .map_or(Self::Ordinary, Self::XRay)
    }
}
