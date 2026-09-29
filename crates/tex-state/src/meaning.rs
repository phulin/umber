//! Meaning word encoding and decoding.

use crate::definition_arena::DefinitionRef;
use crate::ids::FontId;
use crate::page::{PageDimension, PageInteger};
use crate::token::Catcode;

/// Opaque fixed-width operand word used by the hot command boundary.
///
/// The command class decides whether these bits are a packed static meaning or
/// an admitted macro-definition coordinate. Font commands retain their opaque
/// runtime identity at the command owner because it cannot be compressed to a
/// dense slot without losing its namespace and generation. Safe constructors
/// prevent a caller from manufacturing a definition coordinate.
#[repr(transparent)]
pub struct CommandOperandWord<G> {
    raw: u64,
    _brand: core::marker::PhantomData<fn(&G) -> &G>,
}

impl<G> Clone for CommandOperandWord<G> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<G> Copy for CommandOperandWord<G> {}

impl<G> core::fmt::Debug for CommandOperandWord<G> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("CommandOperandWord(..)")
    }
}

impl<G> PartialEq for CommandOperandWord<G> {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw
    }
}

impl<G> Eq for CommandOperandWord<G> {}

impl<G> core::hash::Hash for CommandOperandWord<G> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.raw.hash(state);
    }
}

impl<G> CommandOperandWord<G> {
    #[doc(hidden)]
    #[must_use]
    pub const fn scalar(raw: u64) -> Self {
        Self {
            raw,
            _brand: core::marker::PhantomData,
        }
    }

    #[doc(hidden)]
    #[must_use]
    pub const fn definition(definition: DefinitionRef<G>) -> Self {
        Self::scalar(definition.runtime_word().get())
    }

    #[doc(hidden)]
    #[must_use]
    pub const fn scalar_value(self) -> u64 {
        self.raw
    }

    #[doc(hidden)]
    #[must_use]
    pub fn definition_value(self) -> DefinitionRef<G> {
        DefinitionRef::from_runtime_word(
            core::num::NonZeroU64::new(self.raw).expect("definition operand is nonzero"),
        )
    }
}

const _: () = assert!(core::mem::size_of::<CommandOperandWord<()>>() == 8);

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
const OP_END_V: u8 = 22;

/// Direct command class carried by a validated packed static meaning.
///
/// This is intentionally coarser than [`Meaning`]: the hot command loop needs
/// only TeX's command-class branch, while scanners and execution materialize
/// the full semantic value at their boundary.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum StaticCommandClass {
    Undefined,
    Relax,
    Character,
    Expandable,
    EndV,
    Unexpandable,
    Value,
}

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
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Meaning {
    Undefined,
    Relax,
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
    /// TeX82's inaccessible end-v command after `end_template` expansion.
    ///
    /// This is deliberately distinct from `EndTemplate`: `get_next` delivers
    /// the latter so outer-validity checks run, while `get_x_token` converts
    /// it to this unexpandable command for `do_endv`.
    EndV,
    UnexpandablePrimitive(UnexpandablePrimitive),
    Unknown(RawMeaning),
}

/// One generation-scoped packed meaning cell.
///
/// Scalar meanings retain TeX's compact `opcode:8 | flags:8 | operand:48`
/// representation. Meanings that contain live coordinates carry those
/// coordinates directly: neither a definition nor a font identity may be
/// reconstructed from a dense slot on an ordinary state read.
pub enum MeaningWord<G> {
    Static(u64),
    Font(FontId),
    Macro {
        flags: MeaningFlags,
        definition: DefinitionRef<G>,
    },
}

/// Profiling-only structural census for direct command delivery.
#[cfg(any(test, feature = "profiling"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DirectCommandDeliveryCounters {
    pub dense_row_accesses: u64,
    pub dense_row_decodes: u64,
    pub macro_owner_acquisitions: u64,
    pub meaning_word_clones: u64,
    pub resolved_meaning_clones: u64,
}

#[cfg(any(test, feature = "profiling"))]
thread_local! {
    static DIRECT_COMMAND_DELIVERY_COUNTERS: core::cell::Cell<DirectCommandDeliveryCounters> =
        const { core::cell::Cell::new(DirectCommandDeliveryCounters {
            dense_row_accesses: 0,
            dense_row_decodes: 0,
            macro_owner_acquisitions: 0,
            meaning_word_clones: 0,
            resolved_meaning_clones: 0,
        }) };
}

#[cfg(any(test, feature = "profiling"))]
fn update_direct_command_delivery_counters(
    update: impl FnOnce(&mut DirectCommandDeliveryCounters),
) {
    DIRECT_COMMAND_DELIVERY_COUNTERS.with(|slot| {
        let mut counters = slot.get();
        update(&mut counters);
        slot.set(counters);
    });
}

/// Returns the current thread's profiling-only direct-delivery census.
#[cfg(any(test, feature = "profiling"))]
#[must_use]
pub fn direct_command_delivery_counters() -> DirectCommandDeliveryCounters {
    DIRECT_COMMAND_DELIVERY_COUNTERS.with(core::cell::Cell::get)
}

#[cfg(any(test, feature = "profiling"))]
pub(crate) fn record_dense_meaning_row_access() {
    update_direct_command_delivery_counters(|counters| {
        counters.dense_row_accesses = counters.dense_row_accesses.saturating_add(1);
    });
}

impl<G> Clone for MeaningWord<G> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<G> Copy for MeaningWord<G> {}

impl<G> core::fmt::Debug for MeaningWord<G> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Static(word) => formatter.debug_tuple("Static").field(word).finish(),
            Self::Font(font) => formatter.debug_tuple("Font").field(font).finish(),
            Self::Macro { flags, .. } => formatter
                .debug_struct("Macro")
                .field("flags", flags)
                .field("definition", &"DefinitionRef(..)")
                .finish(),
        }
    }
}

impl<G> PartialEq for MeaningWord<G> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Static(left), Self::Static(right)) => left == right,
            (Self::Font(left), Self::Font(right)) => left == right,
            (
                Self::Macro {
                    flags: left_flags,
                    definition: left_definition,
                },
                Self::Macro {
                    flags: right_flags,
                    definition: right_definition,
                },
            ) => left_flags == right_flags && left_definition == right_definition,
            (Self::Static(_), Self::Font(_) | Self::Macro { .. })
            | (Self::Font(_), Self::Static(_) | Self::Macro { .. })
            | (Self::Macro { .. }, Self::Static(_) | Self::Font(_)) => false,
        }
    }
}

impl<G> Eq for MeaningWord<G> {}

impl<G> MeaningWord<G> {
    pub(crate) const UNDEFINED: Self = Self::Static(0);

    #[must_use]
    pub const fn from_static(meaning: Meaning) -> Self {
        match meaning {
            Meaning::Font(font) => Self::Font(font),
            scalar => Self::Static(scalar.encode()),
        }
    }

    #[must_use]
    pub const fn macro_definition(flags: MeaningFlags, definition: DefinitionRef<G>) -> Self {
        Self::Macro { flags, definition }
    }

    #[must_use]
    pub fn resolve(&self) -> ResolvedMeaning<G> {
        match self {
            Self::Static(word) => ResolvedMeaning::Static(Meaning::decode_stored(*word)),
            Self::Font(font) => ResolvedMeaning::Static(Meaning::Font(*font)),
            Self::Macro { flags, definition } => ResolvedMeaning::Macro {
                flags: *flags,
                definition: *definition,
            },
        }
    }

    /// Borrows the scalar meaning without cloning a generation-local owner.
    #[must_use]
    pub(crate) const fn static_meaning(&self) -> Option<Meaning> {
        match self {
            Self::Static(word) => Some(Meaning::decode_stored(*word)),
            Self::Font(font) => Some(Meaning::Font(*font)),
            Self::Macro { .. } => None,
        }
    }

    /// Returns the exact live font coordinate retained by this meaning.
    #[must_use]
    pub(crate) const fn font(&self) -> Option<FontId> {
        match self {
            Self::Font(font) => Some(*font),
            Self::Static(_) | Self::Macro { .. } => None,
        }
    }

    pub(crate) fn semantic_identity(&self, definition_identity: Option<u64>) -> Option<u64> {
        let definition_identity = match self {
            Self::Macro { .. } => Some(definition_identity?),
            Self::Static(_) | Self::Font(_) => None,
        };
        Some(crate::state_hash::semantic_scalar_root(
            0x6d65_616e_696e_6731,
            |hasher| match self {
                Self::Static(word) => {
                    hasher.u8(0);
                    hasher.u64(*word);
                }
                Self::Font(font) => {
                    hasher.u8(1);
                    hasher.u32(font.raw());
                }
                Self::Macro { flags, definition } => {
                    hasher.u8(2);
                    hasher.u8(flags.bits());
                    let _ = definition;
                    hasher.u64(definition_identity.expect("macro identity was resolved"));
                }
            },
        ))
    }
}

impl<G> MeaningWord<G> {
    /// Decodes this borrowed canonical row directly into the caller's final
    /// command slot.
    ///
    /// The row borrow ends before this call returns. Static meanings are
    /// copied into the resident slot, while a macro row copies only its compact
    /// non-owning definition key.
    #[inline(always)]
    pub(crate) fn write_command_into(
        &self,
        target: &mut impl crate::token::PackedCommandTarget<G>,
    ) {
        #[cfg(any(test, feature = "profiling"))]
        update_direct_command_delivery_counters(|counters| {
            counters.dense_row_decodes = counters.dense_row_decodes.saturating_add(1);
        });
        match self {
            MeaningWord::Static(word) => {
                target.write_static_meaning_word(*word);
            }
            MeaningWord::Font(font) => target.write_font_meaning(*font),
            MeaningWord::Macro { flags, definition } => {
                #[cfg(any(test, feature = "profiling"))]
                update_direct_command_delivery_counters(|counters| {
                    counters.macro_owner_acquisitions =
                        counters.macro_owner_acquisitions.saturating_add(1);
                });
                target.write_macro_meaning(*flags, *definition);
            }
        }
    }
}

/// Decoded meaning returned by an admitted generation borrow.
pub enum ResolvedMeaning<G> {
    Static(Meaning),
    Macro {
        flags: MeaningFlags,
        definition: DefinitionRef<G>,
    },
}

impl<G> Clone for ResolvedMeaning<G> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<G> Copy for ResolvedMeaning<G> {}

impl<G> core::fmt::Debug for ResolvedMeaning<G> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Static(meaning) => formatter.debug_tuple("Static").field(meaning).finish(),
            Self::Macro { flags, .. } => formatter
                .debug_struct("Macro")
                .field("flags", flags)
                .field("definition", &"DefinitionRef(..)")
                .finish(),
        }
    }
}

impl<G> PartialEq for ResolvedMeaning<G> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Static(left), Self::Static(right)) => left == right,
            (
                Self::Macro {
                    flags: left_flags,
                    definition: left_definition,
                },
                Self::Macro {
                    flags: right_flags,
                    definition: right_definition,
                },
            ) => left_flags == right_flags && left_definition == right_definition,
            (Self::Static(_), Self::Macro { .. }) | (Self::Macro { .. }, Self::Static(_)) => false,
        }
    }
}

impl<G> Eq for ResolvedMeaning<G> {}

impl<G> PartialEq<Meaning> for ResolvedMeaning<G> {
    fn eq(&self, other: &Meaning) -> bool {
        matches!(self, Self::Static(meaning) if meaning == other)
    }
}

impl<G> PartialEq<ResolvedMeaning<G>> for Meaning {
    fn eq(&self, other: &ResolvedMeaning<G>) -> bool {
        other == self
    }
}

impl<G> core::hash::Hash for ResolvedMeaning<G> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        match self {
            Self::Static(meaning) => {
                0u8.hash(state);
                meaning.hash(state);
            }
            Self::Macro { flags, definition } => {
                1u8.hash(state);
                flags.hash(state);
                definition.hash(state);
            }
        }
    }
}

/// Read-only internal integer quantities.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[repr(u8)]
pub enum InternalInteger {
    /// Badness of the most recent glue setting.
    Badness = 0,
    /// Current physical input line number.
    InputLineNumber = 1,
    /// e-TeX major version number.
    ETeXVersion = 2,
    /// pdfTeX's numeric release identity (`1.40.x` -> `140`).
    PdfTeXVersion = 9,
    PdfElapsedTime = 10,
    PdfRandomSeed = 11,
    PdfShellEscape = 12,
    PdfLastObject = 13,
    PdfLastAnnot = 17,
    PdfLastLink = 18,
    PdfLastXPos = 14,
    PdfLastYPos = 15,
    PdfLastXForm = 16,
    PdfLastXImage = 21,
    /// pdfTeX's global multi-purpose return value.
    PdfReturnValue = 22,
    /// Number of pages in the most recently registered external image.
    PdfLastXImagePages = 23,
    /// Bits per component in the most recently registered raster image.
    PdfLastXImageColorDepth = 24,
    CurrentGroupLevel = 3,
    CurrentGroupType = 4,
    CurrentIfLevel = 5,
    CurrentIfType = 6,
    CurrentIfBranch = 7,
    LastNodeType = 8,
}

impl InternalInteger {
    #[must_use]
    pub const fn operand(self) -> u64 {
        self as u64
    }

    #[must_use]
    #[inline]
    pub const fn from_operand(operand: u64) -> Option<Self> {
        match operand {
            0 => Some(Self::Badness),
            1 => Some(Self::InputLineNumber),
            2 => Some(Self::ETeXVersion),
            9 => Some(Self::PdfTeXVersion),
            10 => Some(Self::PdfElapsedTime),
            11 => Some(Self::PdfRandomSeed),
            12 => Some(Self::PdfShellEscape),
            13 => Some(Self::PdfLastObject),
            14 => Some(Self::PdfLastXPos),
            15 => Some(Self::PdfLastYPos),
            16 => Some(Self::PdfLastXForm),
            17 => Some(Self::PdfLastAnnot),
            18 => Some(Self::PdfLastLink),
            21 => Some(Self::PdfLastXImage),
            22 => Some(Self::PdfReturnValue),
            23 => Some(Self::PdfLastXImagePages),
            24 => Some(Self::PdfLastXImageColorDepth),
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
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[repr(u8)]
pub enum ExpandablePrimitive {
    ExpandAfter = 0,
    NoExpand = 1,
    CsName = 2,
    EndCsName = 3,
    String = 4,
    Number = 5,
    RomanNumeral = 6,
    Meaning = 7,
    The = 8,
    Input = 9,
    EndInput = 10,
    JobName = 11,
    FontName = 12,
    TopMark = 13,
    FirstMark = 14,
    BotMark = 15,
    SplitFirstMark = 16,
    SplitBotMark = 17,
    TopMarks = 48,
    FirstMarks = 49,
    BotMarks = 50,
    SplitFirstMarks = 51,
    SplitBotMarks = 52,
    IfTrue = 18,
    IfFalse = 19,
    If = 20,
    IfCat = 21,
    IfX = 22,
    IfNum = 23,
    IfDim = 24,
    IfOdd = 25,
    IfCase = 26,
    IfVMode = 27,
    IfHMode = 28,
    IfMMode = 29,
    IfInner = 30,
    IfVoid = 31,
    IfHBox = 32,
    IfVBox = 33,
    IfEof = 34,
    Else = 35,
    Or = 36,
    Fi = 37,
    /// TeX's inaccessible outer end-template command, aliasable via `\let`.
    EndTemplate = 38,
    /// e-TeX's expansion-suppressing general-text primitive.
    Unexpanded = 39,
    /// e-TeX's token-to-character general-text primitive.
    Detokenize = 40,
    Unless = 41,
    Scantokens = 42,
    ETeXVersion = 43,
    ETeXRevision = 44,
    IfDefined = 45,
    IfCsName = 46,
    IfFontChar = 47,
    /// pdfTeX's message-style balanced-text expansion primitive.
    Expanded = 53,
    /// Umber's neutral file-size enquiry for the LaTeX extension contract.
    FileSize = 54,
    /// Engine-neutral lexicographic comparison of two expanded strings.
    StringCompare = 55,
    /// Disabled/enabled shell-escape status for the LaTeX extension contract.
    ShellEscape = 56,
    /// Immutable UTC job-creation timestamp for the LaTeX extension contract.
    CreationDate = 57,
    /// e-TeX's enquiry for expansion inside a live `\csname` scan.
    IfInCsName = 58,
    /// The catcode-12 pdfTeX patch-level suffix (`.27`).
    PdfTeXRevision = 59,
    /// The full pinned pdfTeX engine banner.
    PdfTeXBanner = 60,
    PdfFontSize = 61,
    LeftMarginKern = 62,
    RightMarginKern = 63,
    PdfFontName = 64,
    PdfFontObjectNumber = 65,
    PdfInsertHeight = 81,
    PdfXImageBBox = 82,
    PdfColorStackInit = 83,
    PdfXFormName = 84,
    PdfPageRef = 85,
    /// Accesses the immutable original primitive table.
    PdfPrimitive = 66,
    /// Tests current meaning against the same-spelling original primitive.
    IfPdfPrimitive = 67,
    /// Compares absolute integer magnitudes.
    IfPdfAbsNum = 68,
    /// Compares absolute dimension magnitudes.
    IfPdfAbsDim = 69,
    PdfEscapeString = 70,
    PdfEscapeName = 71,
    PdfEscapeHex = 72,
    PdfUnescapeHex = 73,
    PdfFileModificationDate = 74,
    PdfMdFiveSum = 75,
    PdfFileDump = 76,
    PdfMatch = 77,
    PdfLastMatch = 78,
    PdfUniformDeviate = 79,
    PdfNormalDeviate = 80,
}

impl ExpandablePrimitive {
    #[must_use]
    pub const fn operand(self) -> u64 {
        self as u64
    }

    #[must_use]
    #[inline]
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
            59 => Some(Self::PdfTeXRevision),
            60 => Some(Self::PdfTeXBanner),
            61 => Some(Self::PdfFontSize),
            62 => Some(Self::LeftMarginKern),
            63 => Some(Self::RightMarginKern),
            64 => Some(Self::PdfFontName),
            65 => Some(Self::PdfFontObjectNumber),
            66 => Some(Self::PdfPrimitive),
            67 => Some(Self::IfPdfPrimitive),
            68 => Some(Self::IfPdfAbsNum),
            69 => Some(Self::IfPdfAbsDim),
            70 => Some(Self::PdfEscapeString),
            71 => Some(Self::PdfEscapeName),
            72 => Some(Self::PdfEscapeHex),
            73 => Some(Self::PdfUnescapeHex),
            74 => Some(Self::PdfFileModificationDate),
            75 => Some(Self::PdfMdFiveSum),
            76 => Some(Self::PdfFileDump),
            77 => Some(Self::PdfMatch),
            78 => Some(Self::PdfLastMatch),
            79 => Some(Self::PdfUniformDeviate),
            80 => Some(Self::PdfNormalDeviate),
            81 => Some(Self::PdfInsertHeight),
            82 => Some(Self::PdfXImageBBox),
            83 => Some(Self::PdfColorStackInit),
            84 => Some(Self::PdfXFormName),
            85 => Some(Self::PdfPageRef),
            _ => None,
        }
    }
}

/// Unexpandable primitive opcodes represented directly in meaning words.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[repr(u16)]
pub enum UnexpandablePrimitive {
    Def = 0,
    Edef = 1,
    Gdef = 2,
    Xdef = 3,
    Let = 4,
    FutureLet = 5,
    GlobalDefs = 6,
    Global = 7,
    Long = 8,
    Outer = 9,
    Protected = 10,
    Count = 11,
    Dimen = 12,
    Skip = 13,
    Muskip = 14,
    Toks = 15,
    CountDef = 16,
    DimenDef = 17,
    SkipDef = 18,
    MuskipDef = 19,
    ToksDef = 20,
    CharDef = 21,
    MathCharDef = 22,
    Advance = 23,
    Multiply = 24,
    Divide = 25,
    CatCode = 26,
    LcCode = 27,
    UcCode = 28,
    SfCode = 29,
    MathCode = 30,
    DelCode = 31,
    Font = 32,
    FontDimen = 33,
    HyphenChar = 34,
    SkewChar = 35,
    Patterns = 96,
    Hyphenation = 97,
    Par = 89,
    Indent = 91,
    NoIndent = 92,
    ParShape = 93,
    PrevDepth = 94,
    PrevGraf = 103,
    HAlign = 156,
    VAlign = 157,
    NoAlign = 158,
    Omit = 162,
    Cr = 159,
    CrCr = 160,
    Span = 161,
    HBox = 56,
    VBox = 57,
    VTop = 58,
    SetBox = 59,
    Box = 60,
    Copy = 61,
    VSplit = 116,
    UnHBox = 62,
    UnHCopy = 168,
    UnVBox = 63,
    UnVCopy = 169,
    LastBox = 64,
    Wd = 65,
    Ht = 66,
    Dp = 67,
    Raise = 68,
    Lower = 69,
    MoveLeft = 70,
    MoveRight = 71,
    Char = 76,
    Kern = 73,
    HSkip = 74,
    VSkip = 75,
    Leaders = 163,
    CLeaders = 164,
    XLeaders = 165,
    HFil = 77,
    HFill = 78,
    HSs = 79,
    HFilNeg = 80,
    VFil = 104,
    VFill = 105,
    VSs = 106,
    VFilNeg = 107,
    Penalty = 81,
    VRule = 82,
    HRule = 108,
    ControlSpace = 167,
    ItalicCorrection = 83,
    Discretionary = 84,
    DiscretionaryHyphen = 85,
    NoBoundary = 86,
    SpaceFactor = 87,
    Accent = 88,
    Mark = 101,
    Marks = 199,
    VAdjust = 102,
    Insert = 115,
    UnPenalty = 109,
    UnKern = 110,
    UnSkip = 111,
    LastPenalty = 112,
    LastKern = 113,
    LastSkip = 114,
    OpenIn = 36,
    CloseIn = 37,
    OpenOut = 38,
    CloseOut = 39,
    Immediate = 166,
    Write = 55,
    Read = 40,
    ReadLine = 176,
    FontCharWd = 177,
    FontCharHt = 178,
    FontCharDp = 179,
    FontCharIc = 180,
    ParShapeLength = 200,
    ParShapeIndent = 201,
    ParShapeDimen = 202,
    InterLinePenalties = 203,
    ClubPenalties = 204,
    WidowPenalties = 205,
    DisplayWidowPenalties = 206,
    PageDiscards = 207,
    SplitDiscards = 208,
    InteractionMode = 181,
    NumExpr = 182,
    DimExpr = 183,
    GlueExpr = 184,
    MuExpr = 185,
    GlueStretch = 186,
    GlueShrink = 187,
    GlueStretchOrder = 188,
    GlueShrinkOrder = 189,
    GlueToMu = 190,
    MuToGlue = 191,
    ShowGroups = 192,
    ShowIfs = 193,
    BeginL = 194,
    EndL = 195,
    BeginR = 196,
    EndR = 197,
    Middle = 198,
    Shipout = 99,
    BeginGroup = 41,
    EndGroup = 42,
    AfterGroup = 43,
    AfterAssignment = 44,
    Show = 45,
    ShowBox = 72,
    ShowThe = 46,
    ShowTokens = 47,
    Message = 48,
    ErrMessage = 49,
    ShowLists = 50,
    Special = 100,
    Uppercase = 51,
    Lowercase = 52,
    IgnoreSpaces = 53,
    MathChar = 117,
    Delimiter = 118,
    TextFont = 119,
    ScriptFont = 120,
    ScriptScriptFont = 121,
    MathOrd = 122,
    MathOp = 123,
    MathBin = 124,
    MathRel = 125,
    MathOpen = 126,
    MathClose = 127,
    MathPunct = 128,
    MathInner = 129,
    Underline = 130,
    Overline = 131,
    Limits = 132,
    NoLimits = 133,
    DisplayLimits = 134,
    Over = 135,
    Atop = 136,
    Above = 137,
    OverWithDelims = 138,
    AtopWithDelims = 139,
    AboveWithDelims = 140,
    Radical = 141,
    MathAccent = 142,
    VCenter = 143,
    MSkip = 144,
    MKern = 145,
    NonScript = 146,
    MathChoice = 147,
    Left = 152,
    Right = 153,
    EqNo = 154,
    LeftEqNo = 155,
    DisplayStyle = 148,
    TextStyle = 149,
    ScriptStyle = 150,
    ScriptScriptStyle = 151,
    BatchMode = 172,
    NonstopMode = 173,
    ScrollMode = 174,
    ErrorStopMode = 175,
    End = 54,
    Dump = 170,
    SetLanguage = 171,
    PdfLpCode = 210,
    PdfRpCode = 211,
    PdfEfCode = 212,
    PdfTagCode = 213,
    PdfKnbsCode = 214,
    PdfStbsCode = 215,
    PdfShbsCode = 216,
    PdfKnbcCode = 217,
    PdfKnacCode = 218,
    PdfNoLigatures = 219,
    LetterspaceFont = 220,
    PdfCopyFont = 221,
    PdfFontExpand = 222,
    PdfFontAttr = 223,
    PdfIncludeChars = 224,
    PdfMapFile = 225,
    PdfMapLine = 226,
    PdfGlyphToUnicode = 227,
    PdfNoBuiltinToUnicode = 228,
    PdfLiteral = 238,
    PdfSetMatrix = 239,
    PdfSave = 240,
    PdfRestore = 241,
    PdfColorStack = 242,
    PdfSavePos = 243,
    PdfSnapRefPoint = 244,
    PdfSnapY = 245,
    PdfSnapYComp = 246,
    PdfXForm = 251,
    PdfRefXForm = 252,
    PdfResetTimer = 229,
    PdfSetRandomSeed = 230,
    PdfObject = 231,
    PdfReferenceObject = 232,
    PdfInfo = 233,
    PdfCatalog = 234,
    PdfNames = 235,
    PdfTrailer = 236,
    PdfTrailerId = 237,
    PdfInterwordSpaceOn = 247,
    PdfInterwordSpaceOff = 248,
    PdfFakeSpace = 249,
    PdfSpaceFont = 250,
    PdfAnnot = 255,
    PdfStartLink = 256,
    PdfEndLink = 257,
    PdfRunningLinkOn = 258,
    PdfRunningLinkOff = 259,
    /// Starts a paragraph only when TeX is currently in vertical mode.
    QuitVMode = 265,
    PdfOutline = 260,
    PdfDest = 261,
    PdfThread = 262,
    PdfStartThread = 263,
    PdfEndThread = 264,
    PdfXImage = 253,
    PdfRefXImage = 254,
}

impl UnexpandablePrimitive {
    #[must_use]
    pub const fn operand(self) -> u64 {
        self as u64
    }

    #[must_use]
    #[inline]
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
            91 => Some(Self::Indent),
            92 => Some(Self::NoIndent),
            93 => Some(Self::ParShape),
            94 => Some(Self::PrevDepth),
            103 => Some(Self::PrevGraf),
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
            210 => Some(Self::PdfLpCode),
            211 => Some(Self::PdfRpCode),
            212 => Some(Self::PdfEfCode),
            213 => Some(Self::PdfTagCode),
            214 => Some(Self::PdfKnbsCode),
            215 => Some(Self::PdfStbsCode),
            216 => Some(Self::PdfShbsCode),
            217 => Some(Self::PdfKnbcCode),
            218 => Some(Self::PdfKnacCode),
            219 => Some(Self::PdfNoLigatures),
            220 => Some(Self::LetterspaceFont),
            221 => Some(Self::PdfCopyFont),
            222 => Some(Self::PdfFontExpand),
            223 => Some(Self::PdfFontAttr),
            224 => Some(Self::PdfIncludeChars),
            225 => Some(Self::PdfMapFile),
            226 => Some(Self::PdfMapLine),
            227 => Some(Self::PdfGlyphToUnicode),
            228 => Some(Self::PdfNoBuiltinToUnicode),
            229 => Some(Self::PdfResetTimer),
            230 => Some(Self::PdfSetRandomSeed),
            231 => Some(Self::PdfObject),
            232 => Some(Self::PdfReferenceObject),
            233 => Some(Self::PdfInfo),
            234 => Some(Self::PdfCatalog),
            235 => Some(Self::PdfNames),
            236 => Some(Self::PdfTrailer),
            237 => Some(Self::PdfTrailerId),
            238 => Some(Self::PdfLiteral),
            239 => Some(Self::PdfSetMatrix),
            240 => Some(Self::PdfSave),
            241 => Some(Self::PdfRestore),
            242 => Some(Self::PdfColorStack),
            243 => Some(Self::PdfSavePos),
            244 => Some(Self::PdfSnapRefPoint),
            245 => Some(Self::PdfSnapY),
            246 => Some(Self::PdfSnapYComp),
            247 => Some(Self::PdfInterwordSpaceOn),
            248 => Some(Self::PdfInterwordSpaceOff),
            249 => Some(Self::PdfFakeSpace),
            250 => Some(Self::PdfSpaceFont),
            251 => Some(Self::PdfXForm),
            252 => Some(Self::PdfRefXForm),
            253 => Some(Self::PdfXImage),
            254 => Some(Self::PdfRefXImage),
            255 => Some(Self::PdfAnnot),
            256 => Some(Self::PdfStartLink),
            257 => Some(Self::PdfEndLink),
            258 => Some(Self::PdfRunningLinkOn),
            259 => Some(Self::PdfRunningLinkOff),
            260 => Some(Self::PdfOutline),
            261 => Some(Self::PdfDest),
            262 => Some(Self::PdfThread),
            263 => Some(Self::PdfStartThread),
            264 => Some(Self::PdfEndThread),
            265 => Some(Self::QuitVMode),
            _ => None,
        }
    }
}

/// An unknown raw meaning word decoded from environment storage.
///
/// The fields are intentionally private so downstream code can preserve and
/// re-encode unknown meanings without minting arbitrary meaning words.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
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
    /// Classifies a validated runtime word without decoding a [`Meaning`].
    #[doc(hidden)]
    #[must_use]
    #[inline]
    pub const fn runtime_word_class(word: u64) -> StaticCommandClass {
        match (word >> OPCODE_SHIFT) as u8 {
            OP_UNDEFINED => StaticCommandClass::Undefined,
            OP_RELAX => StaticCommandClass::Relax,
            OP_CHAR_GIVEN | OP_CHAR_TOKEN => StaticCommandClass::Character,
            OP_EXPANDABLE_PRIMITIVE => StaticCommandClass::Expandable,
            OP_END_V => StaticCommandClass::EndV,
            OP_UNEXPANDABLE_PRIMITIVE => StaticCommandClass::Unexpandable,
            _ => StaticCommandClass::Value,
        }
    }

    /// Returns the operand of a validated runtime word without semantic decode.
    #[doc(hidden)]
    #[must_use]
    #[inline]
    pub const fn runtime_word_operand(word: u64) -> u64 {
        word & OPERAND_MASK
    }

    /// Decodes a validated runtime static-meaning word.
    ///
    /// Command delivery deliberately retains this word until a rich semantic
    /// boundary. The packed word was produced by [`Meaning::encode`] or read
    /// from a validated meaning cell, so callers do not use this as a format
    /// or arbitrary-integer constructor.
    #[doc(hidden)]
    #[must_use]
    #[inline]
    pub const fn from_runtime_word(word: u64) -> Self {
        Self::decode_stored(word)
    }

    /// Packed word of [`Meaning::Undefined`], for per-token placeholders.
    pub const UNDEFINED_WORD: u64 = Self::Undefined.encode();
    /// Packed word of [`Meaning::Relax`].
    pub const RELAX_WORD: u64 = Self::Relax.encode();
    /// Packed word of [`Meaning::EndV`].
    pub const END_V_WORD: u64 = Self::EndV.encode();
    /// Packed word of the outer `\endtemplate` primitive.
    pub const END_TEMPLATE_WORD: u64 =
        Self::ExpandablePrimitive(ExpandablePrimitive::EndTemplate).encode();

    /// Packs a character-token meaning without the general encoder's
    /// dispatch; every literal character delivery takes this path.
    #[must_use]
    #[inline(always)]
    pub const fn char_token_word(ch: char, cat: Catcode) -> u64 {
        pack(
            OP_CHAR_TOKEN,
            MeaningFlags::EMPTY,
            ((ch as u64) << 4) | cat as u64,
        )
    }

    /// Encodes this meaning into `opcode:8 | flags:8 | operand:48`.
    #[must_use]
    #[inline]
    pub const fn encode(self) -> u64 {
        match self {
            Self::Undefined => pack(OP_UNDEFINED, MeaningFlags::EMPTY, 0),
            Self::Relax => pack(OP_RELAX, MeaningFlags::EMPTY, 0),
            Self::CharGiven(ch) => pack(OP_CHAR_GIVEN, MeaningFlags::EMPTY, ch as u64),
            Self::CharToken { ch, cat } => Self::char_token_word(ch, cat),
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
            Self::EndV => pack(OP_END_V, MeaningFlags::EMPTY, 0),
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
    #[inline]
    pub(crate) const fn decode_stored(word: u64) -> Self {
        let op = (word >> OPCODE_SHIFT) as u8;
        let flags = MeaningFlags::from_bits((word >> FLAGS_SHIFT) as u8);
        let operand = word & OPERAND_MASK;

        match op {
            OP_UNDEFINED => Self::Undefined,
            OP_RELAX => Self::Relax,
            // Runtime definition coordinates are never decoded from an
            // unvalidated integer. Cold formats materialize a fresh
            // generation-local id before constructing `MeaningWord::Macro`.
            OP_MACRO => Self::Unknown(RawMeaning { op, flags, operand }),
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
            OP_END_V if operand == 0 => Self::EndV,
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

#[inline]
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
#[path = "meaning/tests.rs"]
mod tests;
