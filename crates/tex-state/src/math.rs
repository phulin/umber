//! Math-list node payloads.

use crate::ids::NodeListId;
use crate::scaled::Scaled;

/// Number of classic TeX math families.
pub const MATH_FAMILY_COUNT: u8 = 16;

/// One of TeX's three math font selectors per family.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum MathFontSize {
    Text,
    Script,
    ScriptScript,
}

impl MathFontSize {
    #[must_use]
    pub const fn index(self) -> u16 {
        match self {
            Self::Text => 0,
            Self::Script => 1,
            Self::ScriptScript => 2,
        }
    }
}

/// TeX math styles stored as style nodes in an mlist.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum MathStyle {
    Display,
    Text,
    Script,
    ScriptScript,
}

/// A decoded math character field.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct MathChar {
    pub family: u8,
    pub character: char,
}

/// A noad field as described by tex.web.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum MathField {
    Empty,
    MathChar(MathChar),
    MathTextChar(MathChar),
    SubBox(NodeListId),
    SubMlist(NodeListId),
}

/// Ordinary noad classes.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum NoadClass {
    Ord,
    Op,
    Bin,
    Rel,
    Open,
    Close,
    Punct,
    Inner,
}

/// Limit placement override on operator noads.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum LimitType {
    DisplayLimits,
    Limits,
    NoLimits,
}

/// Specialized noad subtype.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum NoadKind {
    Normal(NoadClass),
    Operator(LimitType),
    Radical { delimiter: u32 },
    Accent { accent: MathChar },
    LeftDelimiter { delimiter: u32 },
    RightDelimiter { delimiter: u32 },
    Underline,
    Overline,
    VCenter,
}

/// A TeX noad with nucleus, subscript, and superscript fields.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct MathNoad {
    pub kind: NoadKind,
    pub nucleus: MathField,
    pub subscript: MathField,
    pub superscript: MathField,
}

impl MathNoad {
    #[must_use]
    pub fn new(kind: NoadKind, nucleus: MathField) -> Self {
        Self {
            kind,
            nucleus,
            subscript: MathField::Empty,
            superscript: MathField::Empty,
        }
    }
}

/// Generalized fraction noad payload.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct MathFraction {
    pub numerator: NodeListId,
    pub denominator: NodeListId,
    pub thickness: FractionThickness,
    pub left_delimiter: Option<u32>,
    pub right_delimiter: Option<u32>,
}

/// TeX's generalized fraction rule thickness.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum FractionThickness {
    Default,
    Explicit(Scaled),
}

/// A four-way math choice.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct MathChoice {
    pub display: NodeListId,
    pub text: NodeListId,
    pub script: NodeListId,
    pub script_script: NodeListId,
}

/// A completed math list appended to the enclosing list.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct MathListNode {
    pub display: bool,
    pub content: NodeListId,
}
