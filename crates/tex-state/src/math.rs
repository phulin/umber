//! Math-list node payloads.

use crate::node_arena::PageListId;
use crate::scaled::Scaled;
use crate::token::OriginId;
use std::hash::{Hash, Hasher};

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
#[derive(Clone, Copy, Debug, serde::Deserialize, serde::Serialize)]
pub struct MathChar {
    pub family: u8,
    pub character: char,
    /// Diagnostic-only source provenance; excluded from TeX semantics.
    #[serde(skip, default)]
    pub origin: OriginId,
}

impl PartialEq for MathChar {
    fn eq(&self, other: &Self) -> bool {
        self.family == other.family && self.character == other.character
    }
}

impl Eq for MathChar {}

impl Hash for MathChar {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.family.hash(state);
        self.character.hash(state);
    }
}

/// A noad field as described by tex.web.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MathField<List = PageListId> {
    Empty,
    MathChar(MathChar),
    MathTextChar(MathChar),
    SubBox(List),
    SubMlist(List),
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
    Radical {
        delimiter: u32,
    },
    Accent {
        accent: MathChar,
    },
    LeftDelimiter {
        delimiter: u32,
    },
    RightDelimiter {
        delimiter: u32,
    },
    /// e-TeX `\middle`: sized with its surrounding `\left...\right` group.
    MiddleDelimiter {
        delimiter: u32,
    },
    Underline,
    Overline,
    VCenter,
}

/// A TeX noad with nucleus, subscript, and superscript fields.
#[derive(Clone, Debug, PartialEq)]
pub struct MathNoad<List = PageListId> {
    pub kind: NoadKind,
    pub nucleus: MathField<List>,
    pub subscript: MathField<List>,
    pub superscript: MathField<List>,
}

impl<List> MathNoad<List> {
    #[must_use]
    pub fn new(kind: NoadKind, nucleus: MathField<List>) -> Self {
        Self {
            kind,
            nucleus,
            subscript: MathField::Empty,
            superscript: MathField::Empty,
        }
    }

    pub(crate) fn map_lists<Other>(self, mut map: impl FnMut(List) -> Other) -> MathNoad<Other> {
        MathNoad {
            kind: self.kind,
            nucleus: self.nucleus.map_list(&mut map),
            subscript: self.subscript.map_list(&mut map),
            superscript: self.superscript.map_list(map),
        }
    }
}

impl<List> MathField<List> {
    pub(crate) fn map_list<Other>(self, map: impl FnOnce(List) -> Other) -> MathField<Other> {
        match self {
            Self::Empty => MathField::Empty,
            Self::MathChar(value) => MathField::MathChar(value),
            Self::MathTextChar(value) => MathField::MathTextChar(value),
            Self::SubBox(value) => MathField::SubBox(map(value)),
            Self::SubMlist(value) => MathField::SubMlist(map(value)),
        }
    }
}

/// Generalized fraction noad payload.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MathFraction<List = PageListId> {
    pub numerator: List,
    pub denominator: List,
    pub thickness: FractionThickness,
    pub left_delimiter: Option<u32>,
    pub right_delimiter: Option<u32>,
}

impl<List> MathFraction<List> {
    pub(crate) fn map_lists<Other>(
        self,
        mut map: impl FnMut(List) -> Other,
    ) -> MathFraction<Other> {
        MathFraction {
            numerator: map(self.numerator),
            denominator: map(self.denominator),
            thickness: self.thickness,
            left_delimiter: self.left_delimiter,
            right_delimiter: self.right_delimiter,
        }
    }
}

/// TeX's generalized fraction rule thickness.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum FractionThickness {
    Default,
    Explicit(Scaled),
}

/// A four-way math choice.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MathChoice<List = PageListId> {
    pub display: List,
    pub text: List,
    pub script: List,
    pub script_script: List,
}

impl<List> MathChoice<List> {
    pub(crate) fn map_lists<Other>(self, mut map: impl FnMut(List) -> Other) -> MathChoice<Other> {
        MathChoice {
            display: map(self.display),
            text: map(self.text),
            script: map(self.script),
            script_script: map(self.script_script),
        }
    }
}

/// A completed math list appended to the enclosing list.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MathListNode<List = PageListId> {
    pub display: bool,
    pub content: List,
}

impl<List> MathListNode<List> {
    pub(crate) fn map_list<Other>(self, map: impl FnOnce(List) -> Other) -> MathListNode<Other> {
        MathListNode {
            display: self.display,
            content: map(self.content),
        }
    }
}
