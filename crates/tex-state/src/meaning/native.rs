//! Native command words retain complete opaque identities without an enum tag.

use super::{
    CHAR_CONTROL_BASE, DefinitionRef, EXPANDABLE_CONTROL_BASE, ExpandablePrimitive, FLAGS_SHIFT,
    FontId, InternalInteger, Meaning, MeaningFlags, OP_CHAR_GIVEN, OP_CHAR_TOKEN,
    OP_COUNT_REGISTER, OP_DIMEN_PARAM, OP_DIMEN_REGISTER, OP_END_V, OP_EXPANDABLE_PRIMITIVE,
    OP_FONT, OP_GLUE_PARAM, OP_INT_PARAM, OP_INTERNAL_INTEGER, OP_MACRO, OP_MATH_CHAR_GIVEN,
    OP_MU_GLUE_PARAM, OP_MUSKIP_REGISTER, OP_PAGE_DIMENSION, OP_PAGE_INTEGER, OP_RELAX,
    OP_SKIP_REGISTER, OP_TOK_PARAM, OP_TOKS_REGISTER, OP_UNDEFINED, OP_UNEXPANDABLE_PRIMITIVE,
    OPCODE_SHIFT, OPERAND_MASK, PageDimension, PageInteger, RawMeaning, ResolvedMeaning,
    UNEXPANDABLE_CONTROL_BASE, UnexpandablePrimitive,
};
use crate::token::Catcode;
use core::marker::PhantomData;
use core::num::NonZeroU64;

const KEY_SHIFT: u32 = 48;
const MACRO_KEY_BIT: u64 = 1 << 15;
const UNKNOWN_KEY: u64 = MACRO_KEY_BIT - 1;
const FONT_SLOT_BITS: u32 = 15;
const FONT_SLOT_MASK: u64 = (1 << FONT_SLOT_BITS) - 1;
const _: () = assert!(crate::font::MAX_FONT_COUNT == 1 << FONT_SLOT_BITS);

/// One generation-scoped native command and its actual opaque coordinate.
#[repr(C)]
pub struct MeaningWord<G> {
    command: u64,
    coordinate: NonZeroU64,
    _brand: PhantomData<fn(&G) -> &G>,
}

const _: () = assert!(core::mem::size_of::<MeaningWord<()>>() == 16);

impl<G> Clone for MeaningWord<G> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<G> Copy for MeaningWord<G> {}
impl<G> PartialEq for MeaningWord<G> {
    fn eq(&self, other: &Self) -> bool {
        self.command == other.command && self.coordinate == other.coordinate
    }
}
impl<G> Eq for MeaningWord<G> {}

impl<G> MeaningWord<G> {
    /// First native execution key in the contiguous literal-category range.
    pub const CHARACTER_KEY_BASE: u64 = CHAR_CONTROL_BASE;

    /// Native macro keys selected from immutable definition shape at publication.
    pub const EMPTY_MACRO_KEY: u64 = MACRO_KEY_BIT;
    pub const SIMPLE_MACRO_KEY: u64 = MACRO_KEY_BIT + 1;
    pub const MATCHING_MACRO_KEY: u64 = MACRO_KEY_BIT + 2;

    pub(crate) const UNDEFINED: Self = Self::scalar(0);

    const fn new(command: u64, coordinate: NonZeroU64) -> Self {
        Self {
            command,
            coordinate,
            _brand: PhantomData,
        }
    }

    /// Publishes a canonical scalar word as its actual command key and payload.
    #[must_use]
    #[inline(always)]
    pub const fn scalar(word: u64) -> Self {
        let opcode = (word >> OPCODE_SHIFT) as u8;
        let operand = word & OPERAND_MASK;
        if matches!(opcode, OP_FONT | OP_MACRO)
            || (opcode == OP_EXPANDABLE_PRIMITIVE && operand > u8::MAX as u64)
            || (opcode == OP_UNEXPANDABLE_PRIMITIVE
                && operand >= UNKNOWN_KEY - UNEXPANDABLE_CONTROL_BASE)
        {
            return Self::new(
                UNKNOWN_KEY << KEY_SHIFT,
                NonZeroU64::new(word).expect("unknown coordinate opcode is nonzero"),
            );
        }
        let payload = match opcode {
            OP_CHAR_TOKEN => operand >> 4,
            OP_EXPANDABLE_PRIMITIVE | OP_UNEXPANDABLE_PRIMITIVE => 0,
            _ => operand,
        };
        Self::new(
            (Meaning::runtime_word_control_code(word) << KEY_SHIFT) | payload,
            NonZeroU64::new(((word >> FLAGS_SHIFT) as u8 as u64) + 1)
                .expect("scalar flag coordinate is nonzero"),
        )
    }

    /// Publishes the packed literal payload directly into its final native meaning.
    #[must_use]
    #[inline(always)]
    pub const fn character_payload(payload: u32) -> Self {
        Self::new(
            ((CHAR_CONTROL_BASE + (payload & 15) as u64) << KEY_SHIFT) | (payload >> 4) as u64,
            NonZeroU64::MIN,
        )
    }

    /// Retains the complete identity of a font within TeX's existing bank bound.
    #[must_use]
    pub const fn font_value(font: FontId) -> Self {
        let words = font.identity().words();
        assert!(
            words[3] < (1 << FONT_SLOT_BITS),
            "native font exceeds the TeX font bank"
        );
        let namespace = (words[0] as u64) | ((words[1] as u64) << 32);
        Self::new(
            ((OP_FONT as u64) << KEY_SHIFT)
                | ((words[2] as u64) << FONT_SLOT_BITS)
                | words[3] as u64,
            NonZeroU64::new(namespace).expect("font namespace is nonzero"),
        )
    }

    #[must_use]
    pub const fn from_static(meaning: Meaning) -> Self {
        match meaning {
            Meaning::Font(font) => Self::font_value(font),
            scalar => Self::scalar(scalar.encode()),
        }
    }

    #[must_use]
    pub(crate) const fn published_macro(
        flags: MeaningFlags,
        definition: DefinitionRef<G>,
        parameter_len: u32,
        word_len: usize,
    ) -> Self {
        let key = if parameter_len != 0 {
            Self::MATCHING_MACRO_KEY
        } else if word_len == 0 {
            Self::EMPTY_MACRO_KEY
        } else {
            Self::SIMPLE_MACRO_KEY
        };
        Self::new(
            (key << KEY_SHIFT) | flags.bits() as u64,
            definition.packed(),
        )
    }

    pub(crate) const fn with_macro_flags(self, flags: MeaningFlags) -> Self {
        assert!(self.is_macro(), "flags require a published macro");
        Self::new(
            (self.execution_key() << KEY_SHIFT) | flags.bits() as u64,
            self.coordinate,
        )
    }

    /// The actual stored execution key; unsupported raw values use a reserved key.
    #[must_use]
    #[inline(always)]
    pub const fn execution_key(&self) -> u64 {
        self.command >> KEY_SHIFT
    }

    #[must_use]
    #[inline(always)]
    pub const fn is_macro(&self) -> bool {
        self.command & (MACRO_KEY_BIT << KEY_SHIFT) != 0
    }

    #[must_use]
    #[inline(always)]
    pub const fn is_font(&self) -> bool {
        self.execution_key() == OP_FONT as u64
    }

    /// The actual native operand; character categories and primitive identity
    /// belong to the execution key rather than a repeated scalar encoding.
    #[must_use]
    #[inline(always)]
    pub const fn operand(&self) -> u64 {
        self.command & OPERAND_MASK
    }

    #[must_use]
    #[inline(always)]
    pub const fn is_relax(&self) -> bool {
        self.execution_key() == OP_RELAX as u64
    }

    #[must_use]
    #[inline(always)]
    pub const fn is_undefined(&self) -> bool {
        self.execution_key() == OP_UNDEFINED as u64
    }

    #[must_use]
    #[inline(always)]
    pub const fn expandable_primitive(&self) -> Option<ExpandablePrimitive> {
        ExpandablePrimitive::from_execution_key(self.execution_key())
    }

    #[must_use]
    #[inline(always)]
    pub const fn unexpandable_primitive(&self) -> Option<UnexpandablePrimitive> {
        let key = self.execution_key();
        if key >= UNEXPANDABLE_CONTROL_BASE && key < UNKNOWN_KEY {
            UnexpandablePrimitive::from_operand(key - UNEXPANDABLE_CONTROL_BASE)
        } else {
            None
        }
    }

    #[must_use]
    #[inline(always)]
    pub const fn character(&self) -> Option<(char, Catcode)> {
        Self::character_from_parts(self.execution_key(), self.operand())
    }

    /// Character projection of the scalars lent by resident input admission.
    #[must_use]
    #[inline(always)]
    pub const fn character_from_parts(key: u64, operand: u64) -> Option<(char, Catcode)> {
        if key < CHAR_CONTROL_BASE || key >= EXPANDABLE_CONTROL_BASE {
            return None;
        }
        match (
            char::from_u32(operand as u32),
            Catcode::from_raw((key - CHAR_CONTROL_BASE) as u8),
        ) {
            (Some(ch), Some(cat)) => Some((ch, cat)),
            _ => None,
        }
    }

    /// Reconstructs the raw word for actual format, diagnostic or hash demand.
    #[must_use]
    pub const fn scalar_word(&self) -> Option<u64> {
        let key = self.execution_key();
        if self.is_macro() || key == OP_FONT as u64 {
            return None;
        }
        if key == UNKNOWN_KEY {
            return Some(self.coordinate.get());
        }
        let (opcode, operand) = if key >= UNEXPANDABLE_CONTROL_BASE {
            (
                OP_UNEXPANDABLE_PRIMITIVE as u64,
                key - UNEXPANDABLE_CONTROL_BASE,
            )
        } else if key >= EXPANDABLE_CONTROL_BASE {
            (
                OP_EXPANDABLE_PRIMITIVE as u64,
                key - EXPANDABLE_CONTROL_BASE,
            )
        } else if key >= CHAR_CONTROL_BASE {
            (
                OP_CHAR_TOKEN as u64,
                ((self.command & OPERAND_MASK) << 4) | (key - CHAR_CONTROL_BASE),
            )
        } else {
            (key, self.command & OPERAND_MASK)
        };
        Some((opcode << OPCODE_SHIFT) | ((self.coordinate.get() - 1) << FLAGS_SHIFT) | operand)
    }

    #[must_use]
    #[inline(always)]
    pub const fn macro_flags(&self) -> Option<MeaningFlags> {
        if self.is_macro() {
            Some(MeaningFlags::from_bits(self.command as u8))
        } else {
            None
        }
    }

    #[must_use]
    #[inline(always)]
    pub const fn macro_parts(&self) -> Option<(MeaningFlags, DefinitionRef<G>)> {
        if !self.is_macro() {
            return None;
        }
        let flags = MeaningFlags::from_bits(self.command as u8);
        Some((flags, DefinitionRef::from_packed(self.coordinate)))
    }

    #[must_use]
    #[inline(always)]
    pub const fn font_id(&self) -> Option<FontId> {
        if self.execution_key() != OP_FONT as u64 {
            return None;
        }
        Some(
            FontId::from_words([
                self.coordinate.get() as u32,
                (self.coordinate.get() >> 32) as u32,
                ((self.command & OPERAND_MASK) >> FONT_SLOT_BITS) as u32,
                (self.command & FONT_SLOT_MASK) as u32,
            ])
            .expect("native font retains its complete identity"),
        )
    }

    /// TeX's noexpand suppression property, including unrecognized raw
    /// expandable values. Endcsname remains a scanner delimiter.
    #[must_use]
    #[inline(always)]
    pub const fn suppressed_by_noexpand(&self) -> bool {
        let key = self.execution_key();
        self.is_macro()
            || self.is_undefined()
            || (key >= EXPANDABLE_CONTROL_BASE
                && key < UNEXPANDABLE_CONTROL_BASE
                && key != EXPANDABLE_CONTROL_BASE + ExpandablePrimitive::EndCsName as u64)
            || (key == UNKNOWN_KEY
                && (self.coordinate.get() >> OPCODE_SHIFT) as u8 == OP_EXPANDABLE_PRIMITIVE)
    }

    #[must_use]
    #[inline(always)]
    pub const fn is_outer(&self) -> bool {
        (self.execution_key() == Meaning::runtime_word_control_code(Meaning::END_TEMPLATE_WORD)
            && self.coordinate.get() == 1)
            || (self.is_macro()
                && MeaningFlags::from_bits(self.command as u8).contains(MeaningFlags::OUTER))
    }

    #[must_use]
    pub fn resolve(&self) -> ResolvedMeaning<G> {
        if let Some(meaning) = self.static_meaning() {
            ResolvedMeaning::Static(meaning)
        } else {
            let (flags, definition) = self.macro_parts().expect("native macro");
            ResolvedMeaning::Macro { flags, definition }
        }
    }

    #[must_use]
    pub(crate) const fn static_meaning(&self) -> Option<Meaning> {
        let key = self.execution_key();
        let operand = self.operand();
        let meaning = match key {
            key if key == OP_UNDEFINED as u64 => Meaning::Undefined,
            key if key == OP_RELAX as u64 => Meaning::Relax,
            _ if self.is_macro() => return None,
            key if key == OP_FONT as u64 => {
                return match self.font_id() {
                    Some(font) => Some(Meaning::Font(font)),
                    None => None,
                };
            }
            key if key == OP_CHAR_GIVEN as u64 => match char::from_u32(operand as u32) {
                Some(ch) => Meaning::CharGiven(ch),
                None => return Some(self.unknown_meaning()),
            },
            key if key >= CHAR_CONTROL_BASE && key < EXPANDABLE_CONTROL_BASE => {
                match self.character() {
                    Some((ch, cat)) => Meaning::CharToken { ch, cat },
                    None => return Some(self.unknown_meaning()),
                }
            }
            key if key >= EXPANDABLE_CONTROL_BASE && key < UNEXPANDABLE_CONTROL_BASE => {
                match self.expandable_primitive() {
                    Some(primitive) => Meaning::ExpandablePrimitive(primitive),
                    None => return Some(self.unknown_meaning()),
                }
            }
            key if key >= UNEXPANDABLE_CONTROL_BASE && key < UNKNOWN_KEY => {
                match self.unexpandable_primitive() {
                    Some(primitive) => Meaning::UnexpandablePrimitive(primitive),
                    None => return Some(self.unknown_meaning()),
                }
            }
            key if key == OP_MATH_CHAR_GIVEN as u64 && operand <= u16::MAX as u64 => {
                Meaning::MathCharGiven(operand as u16)
            }
            key if key == OP_COUNT_REGISTER as u64 && operand <= u16::MAX as u64 => {
                Meaning::CountRegister(operand as u16)
            }
            key if key == OP_DIMEN_REGISTER as u64 && operand <= u16::MAX as u64 => {
                Meaning::DimenRegister(operand as u16)
            }
            key if key == OP_SKIP_REGISTER as u64 && operand <= u16::MAX as u64 => {
                Meaning::SkipRegister(operand as u16)
            }
            key if key == OP_MUSKIP_REGISTER as u64 && operand <= u16::MAX as u64 => {
                Meaning::MuskipRegister(operand as u16)
            }
            key if key == OP_TOKS_REGISTER as u64 && operand <= u16::MAX as u64 => {
                Meaning::ToksRegister(operand as u16)
            }
            key if key == OP_INT_PARAM as u64 && operand <= u16::MAX as u64 => {
                Meaning::IntParam(operand as u16)
            }
            key if key == OP_DIMEN_PARAM as u64 && operand <= u16::MAX as u64 => {
                Meaning::DimenParam(operand as u16)
            }
            key if key == OP_GLUE_PARAM as u64 && operand <= u16::MAX as u64 => {
                Meaning::GlueParam(operand as u16)
            }
            key if key == OP_MU_GLUE_PARAM as u64 && operand <= u16::MAX as u64 => {
                Meaning::MuGlueParam(operand as u16)
            }
            key if key == OP_TOK_PARAM as u64 && operand <= u16::MAX as u64 => {
                Meaning::TokParam(operand as u16)
            }
            key if key == OP_PAGE_DIMENSION as u64 && operand <= u8::MAX as u64 => {
                match PageDimension::from_index(operand as u8) {
                    Some(dimension) => Meaning::PageDimension(dimension),
                    None => return Some(self.unknown_meaning()),
                }
            }
            key if key == OP_PAGE_INTEGER as u64 && operand <= u8::MAX as u64 => {
                match PageInteger::from_index(operand as u8) {
                    Some(integer) => Meaning::PageInteger(integer),
                    None => return Some(self.unknown_meaning()),
                }
            }
            key if key == OP_INTERNAL_INTEGER as u64 => {
                match InternalInteger::from_operand(operand) {
                    Some(integer) => Meaning::InternalInteger(integer),
                    None => return Some(self.unknown_meaning()),
                }
            }
            key if key == OP_END_V as u64 && operand == 0 => Meaning::EndV,
            _ => return Some(self.unknown_meaning()),
        };
        Some(meaning)
    }

    /// Invalid scalar payloads retain their exact raw meaning. Raw font words
    /// describe cold detached font slots, never live native font coordinates.
    const fn unknown_meaning(&self) -> Meaning {
        let word = self.scalar_word().expect("nonmacro scalar meaning");
        let op = (word >> OPCODE_SHIFT) as u8;
        let flags = MeaningFlags::from_bits((word >> FLAGS_SHIFT) as u8);
        let operand = word & OPERAND_MASK;
        if op == OP_FONT && operand <= u32::MAX as u64 {
            Meaning::Font(FontId::new(operand as u32))
        } else {
            Meaning::Unknown(RawMeaning { op, flags, operand })
        }
    }

    pub(crate) fn semantic_identity(&self, definition_identity: Option<u64>) -> Option<u64> {
        let macro_identity = if self.is_macro() {
            Some(definition_identity?)
        } else {
            None
        };
        Some(crate::state_hash::semantic_scalar_root(
            0x6d65_616e_696e_6731,
            |hasher| {
                if let Some(word) = self.scalar_word() {
                    hasher.u8(0);
                    hasher.u64(word);
                } else if let Some(font) = self.font_id() {
                    hasher.u8(1);
                    hasher.u32(font.raw());
                } else {
                    let (flags, _) = self.macro_parts().expect("native macro");
                    hasher.u8(2);
                    hasher.u8(flags.bits());
                    hasher.u64(macro_identity.expect("macro identity was resolved"));
                }
            },
        ))
    }
}

impl<G> core::fmt::Debug for MeaningWord<G> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if let Some(word) = self.scalar_word() {
            formatter.debug_tuple("Static").field(&word).finish()
        } else if let Some(font) = self.font_id() {
            formatter.debug_tuple("Font").field(&font).finish()
        } else {
            let (flags, _) = self.macro_parts().expect("native macro");
            formatter
                .debug_struct("Macro")
                .field("flags", &flags)
                .field("definition", &"DefinitionRef(..)")
                .finish()
        }
    }
}
