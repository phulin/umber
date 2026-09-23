use super::*;

/// A completed TeX82 math-character operand (`\\mathchar` or `\\mathaccent`).
///
/// The command processor validates the canonical 15-bit range before this
/// crosses the main-control boundary.  Replay therefore has neither an
/// integer scanner nor an invalid-code recovery path for the operand.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedMathCharacter {
    pub code: u16,
    pub recovered: bool,
    pub provenance: StructuredProvenance,
}

/// A completed TeX82 delimiter code.  `0` is the canonical missing-delimiter
/// replacement; the diagnostic and rejected-command replay remain command
/// owned.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedMathDelimiter {
    pub code: u32,
    pub recovered: bool,
    /// §1161 rejected a non-delimiter token and backed it up. The executor
    /// owns the resulting error report after the scanner borrow ends.
    pub missing_delimiter: bool,
    pub provenance: StructuredProvenance,
}

/// The font-size bank addressed by a math family assignment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MathFamilySize {
    Text,
    Script,
    ScriptScript,
}

impl MathFamilySize {
    /// Recognizes TeX82 §1234's `def_family` command code.
    ///
    /// `def_family`'s `chr_code` selects the size bank, and every routine that
    /// reaches one of the three primitives -- §415's font-identifier fetch,
    /// §577's `scan_font_ident`, and §1257's assignment -- needs the same
    /// mapping. `None` is "this command is not `def_family`".
    #[must_use]
    pub const fn of_primitive(primitive: UnexpandablePrimitive) -> Option<Self> {
        match primitive {
            UnexpandablePrimitive::TextFont => Some(Self::Text),
            UnexpandablePrimitive::ScriptFont => Some(Self::Script),
            UnexpandablePrimitive::ScriptScriptFont => Some(Self::ScriptScript),
            _ => None,
        }
    }
}

impl From<MathFamilySize> for tex_state::math::MathFontSize {
    fn from(size: MathFamilySize) -> Self {
        match size {
            MathFamilySize::Text => Self::Text,
            MathFamilySize::Script => Self::Script,
            MathFamilySize::ScriptScript => Self::ScriptScript,
        }
    }
}

/// The completed family index prefix of `\\textfont`, `\\scriptfont`, or
/// `\\scriptscriptfont`.  Resolving the following font meaning is deliberately
/// a separate typed operation, so source delivery cannot leak into replay.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedMathFamily {
    pub size: MathFamilySize,
    pub family: u8,
    pub recovered: bool,
    pub provenance: StructuredProvenance,
}

/// Placement selected by TeX82's math script controls.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MathScriptKind {
    Subscript,
    Superscript,
}

/// A script marker whose following math field is collected by the canonical
/// math-field episode.  Keeping the marker typed prevents replay from ever
/// reinterpreting `^` or `_` as source tokens.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedMathScript {
    pub kind: MathScriptKind,
    pub provenance: StructuredProvenance,
}

/// How the stomach must realize one completed math field.
///
/// TeX82 §1151's `scan_math` has exactly two outcomes. An unbraced field is
/// resolved *in place*, by the same procedure that fetched the command: its
/// six scalar cases (`letter`, `other_char`, `char_given`, `char_num`,
/// `math_char_num`, `math_given`, `delim_num`) each end by assigning a single
/// math code `c`, and nothing is ever re-read. A braced field is §1153's
/// ``back_input; scan_left_brace; ... push_math(math_group)`` -- the
/// mandatory brace is consumed and the subformula body is then read *live*
/// by ordinary main control, closed by §1186's `math_group` arm of
/// `handle_right_brace`. A braced field is therefore not command-owned
/// material at all, and must never be absorbed into a token list: doing so
/// backs the brace up a second time, opens an extra replay input level, and
/// swallows the closing brace that TeX delivers as a command.
///
/// A scalar field must not be absorbed and replayed either. §1151 never
/// pushes an input level for it, so a frozen-spelling replay delivers the
/// same command twice, opens and retires a level tex.web has no `token_type`
/// for, and reconstructs the field through a nested mlist -- which also
/// loses `c`'s class bits, because §1151 stores `math_type:=math_char` and
/// drops the class a noad would have carried (`umber2-johp.265`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MathFieldBody {
    /// TeX82 §1151's scalar outcome: the math code `c` its six cases
    /// produce, which the stomach stores as
    /// ``math_type:=math_char; character:=qi(c mod 256); fam:=...``.
    Character(u16),
    /// TeX82 §1153: `math_group`'s opening brace has been consumed and the
    /// body is live input the stomach reads through main control.
    OpenGroup,
    /// No field is available at all.
    Missing,
}

/// One completed math field, ready for the stomach to store.
///
/// Nothing here is deferred input: §1151 has already read, expanded, and
/// classified everything the field consumed, so the stomach receives a value
/// rather than a replay handle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MathFieldEpisode {
    pub body: MathFieldBody,
    pub provenance: StructuredProvenance,
}

/// The structural delimiter boundary selected by `\left`, `\right`, or
/// e-TeX's `\middle`. The corresponding delimiter scan is complete before
/// this value crosses the command boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MathDelimiterBoundary {
    pub kind: MathDelimiterBoundaryKind,
    pub delimiter: ScannedMathDelimiter,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MathDelimiterBoundaryKind {
    Left,
    Right,
    Middle,
}

/// The generalized-fraction form selected before its numerator is frozen.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MathFractionKind {
    Over,
    Atop,
    Above,
}

/// Completed command-owned operands of a generalized fraction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedMathFraction {
    pub kind: MathFractionKind,
    pub left_delimiter: Option<ScannedMathDelimiter>,
    pub right_delimiter: Option<ScannedMathDelimiter>,
    pub thickness: Option<Scaled>,
}

/// A completed `\\mskip` or `\\mkern` operand.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScannedMathMuMaterial {
    Glue(GlueSpec),
    Kern(Scaled),
}

/// Which side receives a display equation number.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EquationNumberSide {
    Right,
    Left,
}

/// Immutable entry request for `\\eqno` and `\\leqno`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedEquationNumber {
    pub side: EquationNumberSide,
}

/// The noad constructor selected by a math-text primitive. Its field is
/// completed by the dedicated canonical math-field episode, not by executor
/// source reads.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MathTextFieldKind {
    Ord,
    Op,
    Bin,
    Rel,
    Open,
    Close,
    Punct,
    Inner,
    Underline,
    Overline,
}

/// Immutable request kinds delivered from command processing to canonical main
/// control for TeX82 §§691–734.  Variants that introduce an mlist episode
/// deliberately contain no source cursor: the later stomach migration can
/// consume only the already-classified request and ask the same processor for
/// the next completed field/group episode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MathRequest {
    Character(ScannedMathCharacter),
    Family(ScannedMathFamily),
    TextField(MathTextFieldKind),
    Script(ScannedMathScript),
    Limits(MathLimitKind),
    Fraction(ScannedMathFraction),
    Style(MathStyleKind),
    Choice,
    Delimiter(ScannedMathDelimiter),
    Radical(ScannedMathDelimiter),
    Accent {
        character: Option<ScannedMathCharacter>,
    },
    MuMaterial(ScannedMathMuMaterial),
    EquationNumber(ScannedEquationNumber),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MathLimitKind {
    Limits,
    NoLimits,
    DisplayLimits,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MathStyleKind {
    Display,
    Text,
    Script,
    ScriptScript,
}
