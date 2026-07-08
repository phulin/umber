//! Meaning word encoding and decoding.

use crate::ids::MacroDefinitionId;

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
    MathCharGiven(u16),
    CountRegister(u16),
    DimenRegister(u16),
    SkipRegister(u16),
    MuskipRegister(u16),
    ToksRegister(u16),
    IntParam(u16),
    DimenParam(u16),
    GlueParam(u16),
    TokParam(u16),
    ExpandablePrimitive(ExpandablePrimitive),
    UnexpandablePrimitive(UnexpandablePrimitive),
    Unknown(RawMeaning),
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
    Read,
    BeginGroup,
    EndGroup,
    AfterGroup,
    AfterAssignment,
    Show,
    ShowThe,
    ShowTokens,
    Message,
    ErrMessage,
    ShowLists,
    Uppercase,
    Lowercase,
    IgnoreSpaces,
    End,
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
            Self::Read => 32,
            Self::BeginGroup => 33,
            Self::EndGroup => 34,
            Self::AfterGroup => 35,
            Self::AfterAssignment => 36,
            Self::Show => 37,
            Self::ShowThe => 38,
            Self::ShowTokens => 39,
            Self::Message => 40,
            Self::ErrMessage => 41,
            Self::ShowLists => 42,
            Self::Uppercase => 43,
            Self::Lowercase => 44,
            Self::IgnoreSpaces => 45,
            Self::End => 46,
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
            32 => Some(Self::Read),
            33 => Some(Self::BeginGroup),
            34 => Some(Self::EndGroup),
            35 => Some(Self::AfterGroup),
            36 => Some(Self::AfterAssignment),
            37 => Some(Self::Show),
            38 => Some(Self::ShowThe),
            39 => Some(Self::ShowTokens),
            40 => Some(Self::Message),
            41 => Some(Self::ErrMessage),
            42 => Some(Self::ShowLists),
            43 => Some(Self::Uppercase),
            44 => Some(Self::Lowercase),
            45 => Some(Self::IgnoreSpaces),
            46 => Some(Self::End),
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
    operand: u64,
}

impl RawMeaning {
    /// Creates a raw meaning for tests that cover the word codec directly.
    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub const fn testing_new(op: u8, operand: u64) -> Self {
        assert!(operand <= OPERAND_MASK, "meaning operand exceeds 48 bits");
        Self { op, operand }
    }

    /// Returns the raw opcode.
    #[must_use]
    pub const fn op(self) -> u8 {
        self.op
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
            Self::TokParam(index) => pack(OP_TOK_PARAM, MeaningFlags::EMPTY, index as u64),
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
            Self::Unknown(raw) => pack(raw.op, MeaningFlags::EMPTY, raw.operand),
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
                None => Self::Unknown(RawMeaning { op, operand }),
            },
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
            OP_TOK_PARAM if operand <= u16::MAX as u64 => Self::TokParam(operand as u16),
            OP_EXPANDABLE_PRIMITIVE => match ExpandablePrimitive::from_operand(operand) {
                Some(primitive) => Self::ExpandablePrimitive(primitive),
                None => Self::Unknown(RawMeaning { op, operand }),
            },
            OP_UNEXPANDABLE_PRIMITIVE => match UnexpandablePrimitive::from_operand(operand) {
                Some(primitive) => Self::UnexpandablePrimitive(primitive),
                None => Self::Unknown(RawMeaning { op, operand }),
            },
            _ => Self::Unknown(RawMeaning { op, operand }),
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

#[cfg(test)]
mod tests {
    use super::{
        ExpandablePrimitive, Meaning, MeaningFlags, OPERAND_MASK, RawMeaning, UnexpandablePrimitive,
    };
    use crate::ids::MacroDefinitionId;

    fn round_trip(meaning: Meaning) {
        assert_eq!(Meaning::decode_stored(meaning.encode()), meaning);
    }

    #[test]
    fn undefined_is_the_all_zero_word() {
        // Fresh zeroed meaning segments decode as Undefined, so this exact
        // encoding is required for fresh-segment correctness.
        assert_eq!(Meaning::Undefined.encode(), 0);
        assert_eq!(Meaning::decode_stored(0), Meaning::Undefined);
    }

    #[test]
    fn meaning_variants_round_trip() {
        round_trip(Meaning::Undefined);
        round_trip(Meaning::Relax);
        round_trip(Meaning::Macro {
            flags: MeaningFlags::LONG
                | MeaningFlags::OUTER
                | MeaningFlags::PROTECTED
                | MeaningFlags::FROZEN,
            definition: MacroDefinitionId::new(0),
        });
        round_trip(Meaning::Macro {
            flags: MeaningFlags::EMPTY,
            definition: MacroDefinitionId::new(u32::MAX),
        });
        round_trip(Meaning::CharGiven('\0'));
        round_trip(Meaning::CharGiven(char::MAX));
        round_trip(Meaning::ExpandablePrimitive(
            ExpandablePrimitive::ExpandAfter,
        ));
        round_trip(Meaning::ExpandablePrimitive(ExpandablePrimitive::NoExpand));
        round_trip(Meaning::ExpandablePrimitive(ExpandablePrimitive::CsName));
        round_trip(Meaning::ExpandablePrimitive(ExpandablePrimitive::EndCsName));
        round_trip(Meaning::ExpandablePrimitive(ExpandablePrimitive::String));
        round_trip(Meaning::ExpandablePrimitive(ExpandablePrimitive::Number));
        round_trip(Meaning::ExpandablePrimitive(
            ExpandablePrimitive::RomanNumeral,
        ));
        round_trip(Meaning::ExpandablePrimitive(ExpandablePrimitive::Meaning));
        round_trip(Meaning::ExpandablePrimitive(ExpandablePrimitive::The));
        round_trip(Meaning::ExpandablePrimitive(ExpandablePrimitive::Input));
        round_trip(Meaning::ExpandablePrimitive(ExpandablePrimitive::EndInput));
        round_trip(Meaning::ExpandablePrimitive(ExpandablePrimitive::JobName));
        round_trip(Meaning::ExpandablePrimitive(ExpandablePrimitive::FontName));
        round_trip(Meaning::ExpandablePrimitive(ExpandablePrimitive::TopMark));
        round_trip(Meaning::ExpandablePrimitive(ExpandablePrimitive::FirstMark));
        round_trip(Meaning::ExpandablePrimitive(ExpandablePrimitive::BotMark));
        round_trip(Meaning::ExpandablePrimitive(
            ExpandablePrimitive::SplitFirstMark,
        ));
        round_trip(Meaning::ExpandablePrimitive(
            ExpandablePrimitive::SplitBotMark,
        ));
        round_trip(Meaning::UnexpandablePrimitive(
            UnexpandablePrimitive::FutureLet,
        ));
        round_trip(Meaning::Unknown(RawMeaning::testing_new(u8::MAX, 0)));
        round_trip(Meaning::Unknown(RawMeaning::testing_new(
            u8::MAX,
            OPERAND_MASK,
        )));
    }

    #[test]
    fn unknown_meaning_exposes_raw_parts_without_public_fields() {
        let word = Meaning::Unknown(RawMeaning::testing_new(200, 42)).encode();
        let Meaning::Unknown(raw) = Meaning::decode_stored(word) else {
            panic!("expected unknown meaning");
        };

        assert_eq!(raw.op(), 200);
        assert_eq!(raw.operand(), 42);
        assert_eq!(Meaning::Unknown(raw).encode(), word);
    }
}
