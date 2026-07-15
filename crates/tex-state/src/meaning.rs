//! Meaning word encoding and decoding.

use crate::ids::{FontId, MacroDefinitionId};
use crate::page::{PageDimension, PageInteger};
use crate::token::Catcode;

const OPCODE_SHIFT: u32 = 56;
const FLAGS_SHIFT: u32 = 48;
const OPERAND_MASK: u64 = (1 << FLAGS_SHIFT) - 1;

const OP_UNDEFINED: u8 = 0;
const OP_RELAX: u8 = 1;
const OP_MACRO: u8 = 2;
const OP_CHAR_GIVEN: u8 = 3;
const OP_EXPANDABLE_PRIMITIVE: u8 = 4;
const OP_UNEXPANDABLE_PRIMITIVE: u8 = 5;
const OP_MATH_CHAR_GIVEN: u8 = 6;
const OP_COUNT_REGISTER: u8 = 7;
const OP_DIMEN_REGISTER: u8 = 8;
const OP_SKIP_REGISTER: u8 = 9;
const OP_MUSKIP_REGISTER: u8 = 10;
const OP_TOKS_REGISTER: u8 = 11;
const OP_INT_PARAM: u8 = 12;
const OP_DIMEN_PARAM: u8 = 13;
const OP_GLUE_PARAM: u8 = 14;
const OP_TOK_PARAM: u8 = 15;
const OP_FONT: u8 = 16;
const OP_PAGE_DIMENSION: u8 = 17;
const OP_PAGE_INTEGER: u8 = 18;
const OP_MU_GLUE_PARAM: u8 = 19;
const OP_CHAR_TOKEN: u8 = 20;
const OP_INTERNAL_INTEGER: u8 = 21;

/// Bitflags carried by meaning words.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MeaningFlags(u8);

impl MeaningFlags {
    pub const EMPTY: Self = Self(0);
    pub const LONG: Self = Self(1 << 0);
    pub const OUTER: Self = Self(1 << 1);
    pub const PROTECTED: Self = Self(1 << 2);
    pub const FROZEN: Self = Self(1 << 3);

    /// Creates flags from raw bits.
    #[must_use]
    pub const fn from_bits(bits: u8) -> Self {
        Self(bits)
    }

    /// Returns the raw flag bits.
    #[must_use]
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// Returns whether all bits in `flag` are set.
    #[must_use]
    pub const fn contains(self, flag: Self) -> bool {
        (self.0 & flag.0) == flag.0
    }
}

impl core::ops::BitOr for MeaningFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

/// A decoded meaning word.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Meaning {
    Undefined,
    Relax,
    Macro {
        flags: MeaningFlags,
        definition: MacroDefinitionId,
    },
    CharGiven(char),
    CharToken {
        ch: char,
        cat: Catcode,
    },
    MathCharGiven(u16),
    CountRegister(u16),
    DimenRegister(u16),
    SkipRegister(u16),
    MuskipRegister(u16),
    ToksRegister(u16),
    IntParam(u16),
    DimenParam(u16),
    GlueParam(u16),
    MuGlueParam(u16),
    TokParam(u16),
    PageDimension(PageDimension),
    PageInteger(PageInteger),
    InternalInteger(InternalInteger),
    Font(FontId),
    ExpandablePrimitive(ExpandablePrimitive),
    UnexpandablePrimitive(UnexpandablePrimitive),
    Unknown(RawMeaning),
}

/// Read-only internal integer quantities.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InternalInteger {
    /// Badness of the most recent glue setting.
    Badness,
    /// Current physical input line number.
    InputLineNumber,
    /// e-TeX major version number.
    ETeXVersion,
    CurrentGroupLevel,
    CurrentGroupType,
    CurrentIfLevel,
    CurrentIfType,
    CurrentIfBranch,
    LastNodeType,
}

impl InternalInteger {
    #[must_use]
    pub const fn operand(self) -> u64 {
        match self {
            Self::Badness => 0,
            Self::InputLineNumber => 1,
            Self::ETeXVersion => 2,
            Self::CurrentGroupLevel => 3,
            Self::CurrentGroupType => 4,
            Self::CurrentIfLevel => 5,
            Self::CurrentIfType => 6,
            Self::CurrentIfBranch => 7,
            Self::LastNodeType => 8,
        }
    }

    #[must_use]
    pub const fn from_operand(operand: u64) -> Option<Self> {
        match operand {
            0 => Some(Self::Badness),
            1 => Some(Self::InputLineNumber),
            2 => Some(Self::ETeXVersion),
            3 => Some(Self::CurrentGroupLevel),
            4 => Some(Self::CurrentGroupType),
            5 => Some(Self::CurrentIfLevel),
            6 => Some(Self::CurrentIfType),
            7 => Some(Self::CurrentIfBranch),
            8 => Some(Self::LastNodeType),
            _ => None,
        }
    }
}

/// Expandable primitive opcodes represented directly in meaning words.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExpandablePrimitive {
    ExpandAfter,
    NoExpand,
    CsName,
    EndCsName,
    String,
    Number,
    RomanNumeral,
    Meaning,
    The,
    Input,
    EndInput,
    JobName,
    FontName,
    TopMark,
    FirstMark,
    BotMark,
    SplitFirstMark,
    SplitBotMark,
    TopMarks,
    FirstMarks,
    BotMarks,
    SplitFirstMarks,
    SplitBotMarks,
    IfTrue,
    IfFalse,
    If,
    IfCat,
    IfX,
    IfNum,
    IfDim,
    IfOdd,
    IfCase,
    IfVMode,
    IfHMode,
    IfMMode,
    IfInner,
    IfVoid,
    IfHBox,
    IfVBox,
    IfEof,
    Else,
    Or,
    Fi,
    /// TeX's inaccessible outer end-template command, aliasable via `\let`.
    EndTemplate,
    /// e-TeX's expansion-suppressing general-text primitive.
    Unexpanded,
    /// e-TeX's token-to-character general-text primitive.
    Detokenize,
    Unless,
    Scantokens,
    ETeXVersion,
    ETeXRevision,
    IfDefined,
    IfCsName,
    IfFontChar,
    /// pdfTeX's message-style balanced-text expansion primitive.
    Expanded,
    /// Umber's neutral file-size enquiry for the LaTeX extension contract.
    FileSize,
    /// Engine-neutral lexicographic comparison of two expanded strings.
    StringCompare,
    /// Disabled/enabled shell-escape status for the LaTeX extension contract.
    ShellEscape,
    /// Immutable UTC job-creation timestamp for the LaTeX extension contract.
    CreationDate,
    /// e-TeX's enquiry for expansion inside a live `\csname` scan.
    IfInCsName,
}

impl ExpandablePrimitive {
    #[must_use]
    pub const fn operand(self) -> u64 {
        match self {
            Self::ExpandAfter => 0,
            Self::NoExpand => 1,
            Self::CsName => 2,
            Self::EndCsName => 3,
            Self::String => 4,
            Self::Number => 5,
            Self::RomanNumeral => 6,
            Self::Meaning => 7,
            Self::The => 8,
            Self::Input => 9,
            Self::EndInput => 10,
            Self::JobName => 11,
            Self::FontName => 12,
            Self::TopMark => 13,
            Self::FirstMark => 14,
            Self::BotMark => 15,
            Self::SplitFirstMark => 16,
            Self::SplitBotMark => 17,
            Self::IfTrue => 18,
            Self::IfFalse => 19,
            Self::If => 20,
            Self::IfCat => 21,
            Self::IfX => 22,
            Self::IfNum => 23,
            Self::IfDim => 24,
            Self::IfOdd => 25,
            Self::IfCase => 26,
            Self::IfVMode => 27,
            Self::IfHMode => 28,
            Self::IfMMode => 29,
            Self::IfInner => 30,
            Self::IfVoid => 31,
            Self::IfHBox => 32,
            Self::IfVBox => 33,
            Self::IfEof => 34,
            Self::Else => 35,
            Self::Or => 36,
            Self::Fi => 37,
            Self::EndTemplate => 38,
            Self::Unexpanded => 39,
            Self::Detokenize => 40,
            Self::Unless => 41,
            Self::Scantokens => 42,
            Self::ETeXVersion => 43,
            Self::ETeXRevision => 44,
            Self::IfDefined => 45,
            Self::IfCsName => 46,
            Self::IfFontChar => 47,
            Self::TopMarks => 48,
            Self::FirstMarks => 49,
            Self::BotMarks => 50,
            Self::SplitFirstMarks => 51,
            Self::SplitBotMarks => 52,
            Self::Expanded => 53,
            Self::FileSize => 54,
            Self::StringCompare => 55,
            Self::ShellEscape => 56,
            Self::CreationDate => 57,
            Self::IfInCsName => 58,
        }
    }

    #[must_use]
    pub const fn from_operand(operand: u64) -> Option<Self> {
        match operand {
            0 => Some(Self::ExpandAfter),
            1 => Some(Self::NoExpand),
            2 => Some(Self::CsName),
            3 => Some(Self::EndCsName),
            4 => Some(Self::String),
            5 => Some(Self::Number),
            6 => Some(Self::RomanNumeral),
            7 => Some(Self::Meaning),
            8 => Some(Self::The),
            9 => Some(Self::Input),
            10 => Some(Self::EndInput),
            11 => Some(Self::JobName),
            12 => Some(Self::FontName),
            13 => Some(Self::TopMark),
            14 => Some(Self::FirstMark),
            15 => Some(Self::BotMark),
            16 => Some(Self::SplitFirstMark),
            17 => Some(Self::SplitBotMark),
            18 => Some(Self::IfTrue),
            19 => Some(Self::IfFalse),
            20 => Some(Self::If),
            21 => Some(Self::IfCat),
            22 => Some(Self::IfX),
            23 => Some(Self::IfNum),
            24 => Some(Self::IfDim),
            25 => Some(Self::IfOdd),
            26 => Some(Self::IfCase),
            27 => Some(Self::IfVMode),
            28 => Some(Self::IfHMode),
            29 => Some(Self::IfMMode),
            30 => Some(Self::IfInner),
            31 => Some(Self::IfVoid),
            32 => Some(Self::IfHBox),
            33 => Some(Self::IfVBox),
            34 => Some(Self::IfEof),
            35 => Some(Self::Else),
            36 => Some(Self::Or),
            37 => Some(Self::Fi),
            38 => Some(Self::EndTemplate),
            39 => Some(Self::Unexpanded),
            40 => Some(Self::Detokenize),
            41 => Some(Self::Unless),
            42 => Some(Self::Scantokens),
            43 => Some(Self::ETeXVersion),
            44 => Some(Self::ETeXRevision),
            45 => Some(Self::IfDefined),
            46 => Some(Self::IfCsName),
            47 => Some(Self::IfFontChar),
            48 => Some(Self::TopMarks),
            49 => Some(Self::FirstMarks),
            50 => Some(Self::BotMarks),
            51 => Some(Self::SplitFirstMarks),
            52 => Some(Self::SplitBotMarks),
            53 => Some(Self::Expanded),
            54 => Some(Self::FileSize),
            55 => Some(Self::StringCompare),
            56 => Some(Self::ShellEscape),
            57 => Some(Self::CreationDate),
            58 => Some(Self::IfInCsName),
            _ => None,
        }
    }
}

/// Unexpandable primitive opcodes represented directly in meaning words.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnexpandablePrimitive {
    Def,
    Edef,
    Gdef,
    Xdef,
    Let,
    FutureLet,
    GlobalDefs,
    Global,
    Long,
    Outer,
    Protected,
    Count,
    Dimen,
    Skip,
    Muskip,
    Toks,
    CountDef,
    DimenDef,
    SkipDef,
    MuskipDef,
    ToksDef,
    CharDef,
    MathCharDef,
    Advance,
    Multiply,
    Divide,
    CatCode,
    LcCode,
    UcCode,
    SfCode,
    MathCode,
    DelCode,
    Font,
    FontDimen,
    HyphenChar,
    SkewChar,
    Patterns,
    Hyphenation,
    Par,
    EndGraf,
    Indent,
    NoIndent,
    ParShape,
    PrevDepth,
    PrevGraf,
    NoInterlineSkip,
    HAlign,
    VAlign,
    NoAlign,
    Omit,
    Cr,
    CrCr,
    Span,
    HBox,
    VBox,
    VTop,
    SetBox,
    Box,
    Copy,
    VSplit,
    UnHBox,
    UnHCopy,
    UnVBox,
    UnVCopy,
    LastBox,
    Wd,
    Ht,
    Dp,
    Raise,
    Lower,
    MoveLeft,
    MoveRight,
    Char,
    Kern,
    HSkip,
    VSkip,
    Leaders,
    CLeaders,
    XLeaders,
    HFil,
    HFill,
    HSs,
    HFilNeg,
    VFil,
    VFill,
    VSs,
    VFilNeg,
    Penalty,
    VRule,
    HRule,
    ControlSpace,
    ItalicCorrection,
    Discretionary,
    DiscretionaryHyphen,
    NoBoundary,
    SpaceFactor,
    Accent,
    Mark,
    Marks,
    VAdjust,
    Insert,
    UnPenalty,
    UnKern,
    UnSkip,
    LastPenalty,
    LastKern,
    LastSkip,
    OpenIn,
    CloseIn,
    OpenOut,
    CloseOut,
    Immediate,
    Write,
    Read,
    ReadLine,
    FontCharWd,
    FontCharHt,
    FontCharDp,
    FontCharIc,
    ParShapeLength,
    ParShapeIndent,
    ParShapeDimen,
    InterLinePenalties,
    ClubPenalties,
    WidowPenalties,
    DisplayWidowPenalties,
    PageDiscards,
    SplitDiscards,
    InteractionMode,
    NumExpr,
    DimExpr,
    GlueExpr,
    MuExpr,
    GlueStretch,
    GlueShrink,
    GlueStretchOrder,
    GlueShrinkOrder,
    GlueToMu,
    MuToGlue,
    ShowGroups,
    ShowIfs,
    BeginL,
    EndL,
    BeginR,
    EndR,
    Middle,
    Shipout,
    BeginGroup,
    EndGroup,
    AfterGroup,
    AfterAssignment,
    Show,
    ShowBox,
    ShowThe,
    ShowTokens,
    Message,
    ErrMessage,
    ShowLists,
    ShowHyphens,
    Special,
    Uppercase,
    Lowercase,
    IgnoreSpaces,
    MathChar,
    Delimiter,
    TextFont,
    ScriptFont,
    ScriptScriptFont,
    MathOrd,
    MathOp,
    MathBin,
    MathRel,
    MathOpen,
    MathClose,
    MathPunct,
    MathInner,
    Underline,
    Overline,
    Limits,
    NoLimits,
    DisplayLimits,
    Over,
    Atop,
    Above,
    OverWithDelims,
    AtopWithDelims,
    AboveWithDelims,
    Radical,
    MathAccent,
    VCenter,
    MSkip,
    MKern,
    NonScript,
    MathChoice,
    Left,
    Right,
    EqNo,
    LeftEqNo,
    DisplayStyle,
    TextStyle,
    ScriptStyle,
    ScriptScriptStyle,
    BatchMode,
    NonstopMode,
    ScrollMode,
    ErrorStopMode,
    End,
    Dump,
    SetLanguage,
}

impl UnexpandablePrimitive {
    #[must_use]
    pub const fn operand(self) -> u64 {
        match self {
            Self::Def => 0,
            Self::Edef => 1,
            Self::Gdef => 2,
            Self::Xdef => 3,
            Self::Let => 4,
            Self::FutureLet => 5,
            Self::GlobalDefs => 6,
            Self::Global => 7,
            Self::Long => 8,
            Self::Outer => 9,
            Self::Protected => 10,
            Self::Count => 11,
            Self::Dimen => 12,
            Self::Skip => 13,
            Self::Muskip => 14,
            Self::Toks => 15,
            Self::CountDef => 16,
            Self::DimenDef => 17,
            Self::SkipDef => 18,
            Self::MuskipDef => 19,
            Self::ToksDef => 20,
            Self::CharDef => 21,
            Self::MathCharDef => 22,
            Self::Advance => 23,
            Self::Multiply => 24,
            Self::Divide => 25,
            Self::CatCode => 26,
            Self::LcCode => 27,
            Self::UcCode => 28,
            Self::SfCode => 29,
            Self::MathCode => 30,
            Self::DelCode => 31,
            Self::Font => 32,
            Self::FontDimen => 33,
            Self::HyphenChar => 34,
            Self::SkewChar => 35,
            Self::Patterns => 96,
            Self::Hyphenation => 97,
            Self::Par => 89,
            Self::EndGraf => 90,
            Self::Indent => 91,
            Self::NoIndent => 92,
            Self::ParShape => 93,
            Self::PrevDepth => 94,
            Self::PrevGraf => 103,
            Self::NoInterlineSkip => 95,
            Self::HAlign => 156,
            Self::VAlign => 157,
            Self::NoAlign => 158,
            Self::Omit => 162,
            Self::Cr => 159,
            Self::CrCr => 160,
            Self::Span => 161,
            Self::HBox => 56,
            Self::VBox => 57,
            Self::VTop => 58,
            Self::SetBox => 59,
            Self::Box => 60,
            Self::Copy => 61,
            Self::VSplit => 116,
            Self::UnHBox => 62,
            Self::UnHCopy => 168,
            Self::UnVBox => 63,
            Self::UnVCopy => 169,
            Self::LastBox => 64,
            Self::Wd => 65,
            Self::Ht => 66,
            Self::Dp => 67,
            Self::Raise => 68,
            Self::Lower => 69,
            Self::MoveLeft => 70,
            Self::MoveRight => 71,
            Self::Char => 76,
            Self::Kern => 73,
            Self::HSkip => 74,
            Self::VSkip => 75,
            Self::Leaders => 163,
            Self::CLeaders => 164,
            Self::XLeaders => 165,
            Self::HFil => 77,
            Self::HFill => 78,
            Self::HSs => 79,
            Self::HFilNeg => 80,
            Self::VFil => 104,
            Self::VFill => 105,
            Self::VSs => 106,
            Self::VFilNeg => 107,
            Self::Penalty => 81,
            Self::VRule => 82,
            Self::HRule => 108,
            Self::ControlSpace => 167,
            Self::ItalicCorrection => 83,
            Self::Discretionary => 84,
            Self::DiscretionaryHyphen => 85,
            Self::NoBoundary => 86,
            Self::SpaceFactor => 87,
            Self::Accent => 88,
            Self::Mark => 101,
            Self::VAdjust => 102,
            Self::Insert => 115,
            Self::UnPenalty => 109,
            Self::UnKern => 110,
            Self::UnSkip => 111,
            Self::LastPenalty => 112,
            Self::LastKern => 113,
            Self::LastSkip => 114,
            Self::OpenIn => 36,
            Self::CloseIn => 37,
            Self::OpenOut => 38,
            Self::CloseOut => 39,
            Self::Immediate => 166,
            Self::Write => 55,
            Self::Read => 40,
            Self::Shipout => 99,
            Self::BeginGroup => 41,
            Self::EndGroup => 42,
            Self::AfterGroup => 43,
            Self::AfterAssignment => 44,
            Self::Show => 45,
            Self::ShowBox => 72,
            Self::ShowThe => 46,
            Self::ShowTokens => 47,
            Self::Message => 48,
            Self::ErrMessage => 49,
            Self::ShowLists => 50,
            Self::ShowHyphens => 98,
            Self::Special => 100,
            Self::Uppercase => 51,
            Self::Lowercase => 52,
            Self::IgnoreSpaces => 53,
            Self::MathChar => 117,
            Self::Delimiter => 118,
            Self::TextFont => 119,
            Self::ScriptFont => 120,
            Self::ScriptScriptFont => 121,
            Self::MathOrd => 122,
            Self::MathOp => 123,
            Self::MathBin => 124,
            Self::MathRel => 125,
            Self::MathOpen => 126,
            Self::MathClose => 127,
            Self::MathPunct => 128,
            Self::MathInner => 129,
            Self::Underline => 130,
            Self::Overline => 131,
            Self::Limits => 132,
            Self::NoLimits => 133,
            Self::DisplayLimits => 134,
            Self::Over => 135,
            Self::Atop => 136,
            Self::Above => 137,
            Self::OverWithDelims => 138,
            Self::AtopWithDelims => 139,
            Self::AboveWithDelims => 140,
            Self::Radical => 141,
            Self::MathAccent => 142,
            Self::VCenter => 143,
            Self::MSkip => 144,
            Self::MKern => 145,
            Self::NonScript => 146,
            Self::MathChoice => 147,
            Self::DisplayStyle => 148,
            Self::TextStyle => 149,
            Self::ScriptStyle => 150,
            Self::ScriptScriptStyle => 151,
            Self::Left => 152,
            Self::Right => 153,
            Self::EqNo => 154,
            Self::LeftEqNo => 155,
            Self::End => 54,
            Self::Dump => 170,
            Self::SetLanguage => 171,
            Self::BatchMode => 172,
            Self::NonstopMode => 173,
            Self::ScrollMode => 174,
            Self::ErrorStopMode => 175,
            Self::ReadLine => 176,
            Self::FontCharWd => 177,
            Self::FontCharHt => 178,
            Self::FontCharDp => 179,
            Self::FontCharIc => 180,
            Self::InteractionMode => 181,
            Self::NumExpr => 182,
            Self::DimExpr => 183,
            Self::GlueExpr => 184,
            Self::MuExpr => 185,
            Self::GlueStretch => 186,
            Self::GlueShrink => 187,
            Self::GlueStretchOrder => 188,
            Self::GlueShrinkOrder => 189,
            Self::GlueToMu => 190,
            Self::MuToGlue => 191,
            Self::ShowGroups => 192,
            Self::ShowIfs => 193,
            Self::BeginL => 194,
            Self::EndL => 195,
            Self::BeginR => 196,
            Self::EndR => 197,
            Self::Middle => 198,
            Self::Marks => 199,
            Self::ParShapeLength => 200,
            Self::ParShapeIndent => 201,
            Self::ParShapeDimen => 202,
            Self::InterLinePenalties => 203,
            Self::ClubPenalties => 204,
            Self::WidowPenalties => 205,
            Self::DisplayWidowPenalties => 206,
            Self::PageDiscards => 207,
            Self::SplitDiscards => 208,
        }
    }

    #[must_use]
    pub const fn from_operand(operand: u64) -> Option<Self> {
        match operand {
            0 => Some(Self::Def),
            1 => Some(Self::Edef),
            2 => Some(Self::Gdef),
            3 => Some(Self::Xdef),
            4 => Some(Self::Let),
            5 => Some(Self::FutureLet),
            6 => Some(Self::GlobalDefs),
            7 => Some(Self::Global),
            8 => Some(Self::Long),
            9 => Some(Self::Outer),
            10 => Some(Self::Protected),
            11 => Some(Self::Count),
            12 => Some(Self::Dimen),
            13 => Some(Self::Skip),
            14 => Some(Self::Muskip),
            15 => Some(Self::Toks),
            16 => Some(Self::CountDef),
            17 => Some(Self::DimenDef),
            18 => Some(Self::SkipDef),
            19 => Some(Self::MuskipDef),
            20 => Some(Self::ToksDef),
            21 => Some(Self::CharDef),
            22 => Some(Self::MathCharDef),
            23 => Some(Self::Advance),
            24 => Some(Self::Multiply),
            25 => Some(Self::Divide),
            26 => Some(Self::CatCode),
            27 => Some(Self::LcCode),
            28 => Some(Self::UcCode),
            29 => Some(Self::SfCode),
            30 => Some(Self::MathCode),
            31 => Some(Self::DelCode),
            32 => Some(Self::Font),
            33 => Some(Self::FontDimen),
            34 => Some(Self::HyphenChar),
            35 => Some(Self::SkewChar),
            96 => Some(Self::Patterns),
            97 => Some(Self::Hyphenation),
            89 => Some(Self::Par),
            90 => Some(Self::EndGraf),
            91 => Some(Self::Indent),
            92 => Some(Self::NoIndent),
            93 => Some(Self::ParShape),
            94 => Some(Self::PrevDepth),
            103 => Some(Self::PrevGraf),
            95 => Some(Self::NoInterlineSkip),
            156 => Some(Self::HAlign),
            157 => Some(Self::VAlign),
            158 => Some(Self::NoAlign),
            162 => Some(Self::Omit),
            159 => Some(Self::Cr),
            160 => Some(Self::CrCr),
            161 => Some(Self::Span),
            56 => Some(Self::HBox),
            57 => Some(Self::VBox),
            58 => Some(Self::VTop),
            59 => Some(Self::SetBox),
            60 => Some(Self::Box),
            61 => Some(Self::Copy),
            116 => Some(Self::VSplit),
            62 => Some(Self::UnHBox),
            168 => Some(Self::UnHCopy),
            63 => Some(Self::UnVBox),
            169 => Some(Self::UnVCopy),
            64 => Some(Self::LastBox),
            65 => Some(Self::Wd),
            66 => Some(Self::Ht),
            67 => Some(Self::Dp),
            68 => Some(Self::Raise),
            69 => Some(Self::Lower),
            70 => Some(Self::MoveLeft),
            71 => Some(Self::MoveRight),
            76 => Some(Self::Char),
            73 => Some(Self::Kern),
            74 => Some(Self::HSkip),
            75 => Some(Self::VSkip),
            163 => Some(Self::Leaders),
            164 => Some(Self::CLeaders),
            165 => Some(Self::XLeaders),
            77 => Some(Self::HFil),
            78 => Some(Self::HFill),
            79 => Some(Self::HSs),
            80 => Some(Self::HFilNeg),
            104 => Some(Self::VFil),
            105 => Some(Self::VFill),
            106 => Some(Self::VSs),
            107 => Some(Self::VFilNeg),
            81 => Some(Self::Penalty),
            82 => Some(Self::VRule),
            108 => Some(Self::HRule),
            167 => Some(Self::ControlSpace),
            83 => Some(Self::ItalicCorrection),
            84 => Some(Self::Discretionary),
            85 => Some(Self::DiscretionaryHyphen),
            86 => Some(Self::NoBoundary),
            87 => Some(Self::SpaceFactor),
            88 => Some(Self::Accent),
            101 => Some(Self::Mark),
            102 => Some(Self::VAdjust),
            115 => Some(Self::Insert),
            109 => Some(Self::UnPenalty),
            110 => Some(Self::UnKern),
            111 => Some(Self::UnSkip),
            112 => Some(Self::LastPenalty),
            113 => Some(Self::LastKern),
            114 => Some(Self::LastSkip),
            36 => Some(Self::OpenIn),
            37 => Some(Self::CloseIn),
            38 => Some(Self::OpenOut),
            39 => Some(Self::CloseOut),
            166 => Some(Self::Immediate),
            55 => Some(Self::Write),
            40 => Some(Self::Read),
            99 => Some(Self::Shipout),
            41 => Some(Self::BeginGroup),
            42 => Some(Self::EndGroup),
            43 => Some(Self::AfterGroup),
            44 => Some(Self::AfterAssignment),
            45 => Some(Self::Show),
            72 => Some(Self::ShowBox),
            46 => Some(Self::ShowThe),
            47 => Some(Self::ShowTokens),
            48 => Some(Self::Message),
            49 => Some(Self::ErrMessage),
            50 => Some(Self::ShowLists),
            98 => Some(Self::ShowHyphens),
            100 => Some(Self::Special),
            51 => Some(Self::Uppercase),
            52 => Some(Self::Lowercase),
            53 => Some(Self::IgnoreSpaces),
            117 => Some(Self::MathChar),
            118 => Some(Self::Delimiter),
            119 => Some(Self::TextFont),
            120 => Some(Self::ScriptFont),
            121 => Some(Self::ScriptScriptFont),
            122 => Some(Self::MathOrd),
            123 => Some(Self::MathOp),
            124 => Some(Self::MathBin),
            125 => Some(Self::MathRel),
            126 => Some(Self::MathOpen),
            127 => Some(Self::MathClose),
            128 => Some(Self::MathPunct),
            129 => Some(Self::MathInner),
            130 => Some(Self::Underline),
            131 => Some(Self::Overline),
            132 => Some(Self::Limits),
            133 => Some(Self::NoLimits),
            134 => Some(Self::DisplayLimits),
            135 => Some(Self::Over),
            136 => Some(Self::Atop),
            137 => Some(Self::Above),
            138 => Some(Self::OverWithDelims),
            139 => Some(Self::AtopWithDelims),
            140 => Some(Self::AboveWithDelims),
            141 => Some(Self::Radical),
            142 => Some(Self::MathAccent),
            143 => Some(Self::VCenter),
            144 => Some(Self::MSkip),
            145 => Some(Self::MKern),
            146 => Some(Self::NonScript),
            147 => Some(Self::MathChoice),
            148 => Some(Self::DisplayStyle),
            149 => Some(Self::TextStyle),
            150 => Some(Self::ScriptStyle),
            151 => Some(Self::ScriptScriptStyle),
            152 => Some(Self::Left),
            153 => Some(Self::Right),
            154 => Some(Self::EqNo),
            155 => Some(Self::LeftEqNo),
            54 => Some(Self::End),
            170 => Some(Self::Dump),
            171 => Some(Self::SetLanguage),
            172 => Some(Self::BatchMode),
            173 => Some(Self::NonstopMode),
            174 => Some(Self::ScrollMode),
            175 => Some(Self::ErrorStopMode),
            176 => Some(Self::ReadLine),
            177 => Some(Self::FontCharWd),
            178 => Some(Self::FontCharHt),
            179 => Some(Self::FontCharDp),
            180 => Some(Self::FontCharIc),
            181 => Some(Self::InteractionMode),
            182 => Some(Self::NumExpr),
            183 => Some(Self::DimExpr),
            184 => Some(Self::GlueExpr),
            185 => Some(Self::MuExpr),
            186 => Some(Self::GlueStretch),
            187 => Some(Self::GlueShrink),
            188 => Some(Self::GlueStretchOrder),
            189 => Some(Self::GlueShrinkOrder),
            190 => Some(Self::GlueToMu),
            191 => Some(Self::MuToGlue),
            192 => Some(Self::ShowGroups),
            193 => Some(Self::ShowIfs),
            194 => Some(Self::BeginL),
            195 => Some(Self::EndL),
            196 => Some(Self::BeginR),
            197 => Some(Self::EndR),
            198 => Some(Self::Middle),
            199 => Some(Self::Marks),
            200 => Some(Self::ParShapeLength),
            201 => Some(Self::ParShapeIndent),
            202 => Some(Self::ParShapeDimen),
            203 => Some(Self::InterLinePenalties),
            204 => Some(Self::ClubPenalties),
            205 => Some(Self::WidowPenalties),
            206 => Some(Self::DisplayWidowPenalties),
            207 => Some(Self::PageDiscards),
            208 => Some(Self::SplitDiscards),
            _ => None,
        }
    }
}

/// An unknown raw meaning word decoded from environment storage.
///
/// The fields are intentionally private so downstream code can preserve and
/// re-encode unknown meanings without minting arbitrary meaning words.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RawMeaning {
    op: u8,
    flags: MeaningFlags,
    operand: u64,
}

impl RawMeaning {
    /// Creates a raw meaning for tests that cover the word codec directly.
    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub const fn testing_new(op: u8, operand: u64) -> Self {
        assert!(operand <= OPERAND_MASK, "meaning operand exceeds 48 bits");
        Self {
            op,
            flags: MeaningFlags::EMPTY,
            operand,
        }
    }

    /// Creates a raw meaning with explicit flags for codec tests.
    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub const fn testing_new_with_flags(op: u8, flags: MeaningFlags, operand: u64) -> Self {
        assert!(operand <= OPERAND_MASK, "meaning operand exceeds 48 bits");
        Self { op, flags, operand }
    }

    /// Returns the raw opcode.
    #[must_use]
    pub const fn op(self) -> u8 {
        self.op
    }

    /// Returns the raw flag byte.
    #[must_use]
    pub const fn flags(self) -> MeaningFlags {
        self.flags
    }

    /// Returns the raw operand.
    #[must_use]
    pub const fn operand(self) -> u64 {
        self.operand
    }
}

impl Meaning {
    /// Encodes this meaning into `opcode:8 | flags:8 | operand:48`.
    #[must_use]
    pub const fn encode(self) -> u64 {
        match self {
            Self::Undefined => pack(OP_UNDEFINED, MeaningFlags::EMPTY, 0),
            Self::Relax => pack(OP_RELAX, MeaningFlags::EMPTY, 0),
            Self::Macro { flags, definition } => pack(OP_MACRO, flags, definition.raw() as u64),
            Self::CharGiven(ch) => pack(OP_CHAR_GIVEN, MeaningFlags::EMPTY, ch as u64),
            Self::CharToken { ch, cat } => pack(
                OP_CHAR_TOKEN,
                MeaningFlags::EMPTY,
                ((ch as u64) << 4) | cat as u64,
            ),
            Self::MathCharGiven(value) => {
                pack(OP_MATH_CHAR_GIVEN, MeaningFlags::EMPTY, value as u64)
            }
            Self::CountRegister(index) => {
                pack(OP_COUNT_REGISTER, MeaningFlags::EMPTY, index as u64)
            }
            Self::DimenRegister(index) => {
                pack(OP_DIMEN_REGISTER, MeaningFlags::EMPTY, index as u64)
            }
            Self::SkipRegister(index) => pack(OP_SKIP_REGISTER, MeaningFlags::EMPTY, index as u64),
            Self::MuskipRegister(index) => {
                pack(OP_MUSKIP_REGISTER, MeaningFlags::EMPTY, index as u64)
            }
            Self::ToksRegister(index) => pack(OP_TOKS_REGISTER, MeaningFlags::EMPTY, index as u64),
            Self::IntParam(index) => pack(OP_INT_PARAM, MeaningFlags::EMPTY, index as u64),
            Self::DimenParam(index) => pack(OP_DIMEN_PARAM, MeaningFlags::EMPTY, index as u64),
            Self::GlueParam(index) => pack(OP_GLUE_PARAM, MeaningFlags::EMPTY, index as u64),
            Self::MuGlueParam(index) => pack(OP_MU_GLUE_PARAM, MeaningFlags::EMPTY, index as u64),
            Self::TokParam(index) => pack(OP_TOK_PARAM, MeaningFlags::EMPTY, index as u64),
            Self::PageDimension(dimension) => pack(
                OP_PAGE_DIMENSION,
                MeaningFlags::EMPTY,
                dimension.index() as u64,
            ),
            Self::PageInteger(integer) => {
                pack(OP_PAGE_INTEGER, MeaningFlags::EMPTY, integer.index() as u64)
            }
            Self::InternalInteger(integer) => {
                pack(OP_INTERNAL_INTEGER, MeaningFlags::EMPTY, integer.operand())
            }
            Self::Font(id) => pack(OP_FONT, MeaningFlags::EMPTY, id.raw() as u64),
            Self::ExpandablePrimitive(primitive) => pack(
                OP_EXPANDABLE_PRIMITIVE,
                MeaningFlags::EMPTY,
                primitive.operand(),
            ),
            Self::UnexpandablePrimitive(primitive) => pack(
                OP_UNEXPANDABLE_PRIMITIVE,
                MeaningFlags::EMPTY,
                primitive.operand(),
            ),
            Self::Unknown(raw) => pack(raw.op, raw.flags, raw.operand),
        }
    }

    /// Decodes a stored `opcode:8 | flags:8 | operand:48` word.
    #[must_use]
    pub(crate) const fn decode_stored(word: u64) -> Self {
        let op = (word >> OPCODE_SHIFT) as u8;
        let flags = MeaningFlags::from_bits((word >> FLAGS_SHIFT) as u8);
        let operand = word & OPERAND_MASK;

        match op {
            OP_UNDEFINED => Self::Undefined,
            OP_RELAX => Self::Relax,
            OP_MACRO => Self::Macro {
                flags,
                definition: MacroDefinitionId::new(operand as u32),
            },
            OP_CHAR_GIVEN => match char::from_u32(operand as u32) {
                Some(ch) => Self::CharGiven(ch),
                None => Self::Unknown(RawMeaning { op, flags, operand }),
            },
            OP_CHAR_TOKEN => {
                let ch = char::from_u32((operand >> 4) as u32);
                let cat = catcode_from_raw((operand & 0xF) as u8);
                match (ch, cat) {
                    (Some(ch), Some(cat)) => Self::CharToken { ch, cat },
                    _ => Self::Unknown(RawMeaning { op, flags, operand }),
                }
            }
            OP_MATH_CHAR_GIVEN if operand <= u16::MAX as u64 => Self::MathCharGiven(operand as u16),
            OP_COUNT_REGISTER if operand <= u16::MAX as u64 => Self::CountRegister(operand as u16),
            OP_DIMEN_REGISTER if operand <= u16::MAX as u64 => Self::DimenRegister(operand as u16),
            OP_SKIP_REGISTER if operand <= u16::MAX as u64 => Self::SkipRegister(operand as u16),
            OP_MUSKIP_REGISTER if operand <= u16::MAX as u64 => {
                Self::MuskipRegister(operand as u16)
            }
            OP_TOKS_REGISTER if operand <= u16::MAX as u64 => Self::ToksRegister(operand as u16),
            OP_INT_PARAM if operand <= u16::MAX as u64 => Self::IntParam(operand as u16),
            OP_DIMEN_PARAM if operand <= u16::MAX as u64 => Self::DimenParam(operand as u16),
            OP_GLUE_PARAM if operand <= u16::MAX as u64 => Self::GlueParam(operand as u16),
            OP_MU_GLUE_PARAM if operand <= u16::MAX as u64 => Self::MuGlueParam(operand as u16),
            OP_TOK_PARAM if operand <= u16::MAX as u64 => Self::TokParam(operand as u16),
            OP_PAGE_DIMENSION if operand <= u8::MAX as u64 => {
                match PageDimension::from_index(operand as u8) {
                    Some(dimension) => Self::PageDimension(dimension),
                    None => Self::Unknown(RawMeaning { op, flags, operand }),
                }
            }
            OP_PAGE_INTEGER if operand <= u8::MAX as u64 => {
                match PageInteger::from_index(operand as u8) {
                    Some(integer) => Self::PageInteger(integer),
                    None => Self::Unknown(RawMeaning { op, flags, operand }),
                }
            }
            OP_INTERNAL_INTEGER => match InternalInteger::from_operand(operand) {
                Some(integer) => Self::InternalInteger(integer),
                None => Self::Unknown(RawMeaning { op, flags, operand }),
            },
            OP_FONT if operand <= u32::MAX as u64 => Self::Font(FontId::new(operand as u32)),
            OP_EXPANDABLE_PRIMITIVE => match ExpandablePrimitive::from_operand(operand) {
                Some(primitive) => Self::ExpandablePrimitive(primitive),
                None => Self::Unknown(RawMeaning { op, flags, operand }),
            },
            OP_UNEXPANDABLE_PRIMITIVE => match UnexpandablePrimitive::from_operand(operand) {
                Some(primitive) => Self::UnexpandablePrimitive(primitive),
                None => Self::Unknown(RawMeaning { op, flags, operand }),
            },
            _ => Self::Unknown(RawMeaning { op, flags, operand }),
        }
    }

    /// Decodes a raw meaning word for explicit testing/fuzzing builds.
    #[cfg(feature = "testing")]
    #[must_use]
    pub const fn testing_decode(word: u64) -> Self {
        Self::decode_stored(word)
    }
}

const fn pack(op: u8, flags: MeaningFlags, operand: u64) -> u64 {
    assert!(operand <= OPERAND_MASK, "meaning operand exceeds 48 bits");
    ((op as u64) << OPCODE_SHIFT) | ((flags.bits() as u64) << FLAGS_SHIFT) | operand
}

const fn catcode_from_raw(raw: u8) -> Option<Catcode> {
    match raw {
        0 => Some(Catcode::Escape),
        1 => Some(Catcode::BeginGroup),
        2 => Some(Catcode::EndGroup),
        3 => Some(Catcode::MathShift),
        4 => Some(Catcode::AlignmentTab),
        5 => Some(Catcode::EndLine),
        6 => Some(Catcode::Parameter),
        7 => Some(Catcode::Superscript),
        8 => Some(Catcode::Subscript),
        9 => Some(Catcode::Ignored),
        10 => Some(Catcode::Space),
        11 => Some(Catcode::Letter),
        12 => Some(Catcode::Other),
        13 => Some(Catcode::Active),
        14 => Some(Catcode::Comment),
        15 => Some(Catcode::Invalid),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
