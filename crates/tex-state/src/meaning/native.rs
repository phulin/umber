//! Native command words retain complete opaque identities without an enum tag.

use super::{
    DefinitionRef, FLAGS_SHIFT, FontId, Meaning, MeaningFlags, OP_FONT, OP_MACRO, OPCODE_SHIFT,
    OPERAND_MASK, ResolvedMeaning,
};
use core::marker::PhantomData;
use core::num::NonZeroU64;

const RAW_SCALAR: u8 = u8::MAX;
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
    pub(crate) const UNDEFINED: Self = Self::scalar(0);

    const fn new(command: u64, coordinate: NonZeroU64) -> Self {
        Self {
            command,
            coordinate,
            _brand: PhantomData,
        }
    }

    /// Stores a scalar word; reserved coordinate opcodes remain scalar values.
    #[must_use]
    #[inline(always)]
    pub const fn scalar(word: u64) -> Self {
        match (word >> OPCODE_SHIFT) as u8 {
            OP_FONT | OP_MACRO | RAW_SCALAR => Self::new(
                (RAW_SCALAR as u64) << OPCODE_SHIFT,
                NonZeroU64::new(word).expect("reserved scalar opcode is nonzero"),
            ),
            _ => Self::new(word, NonZeroU64::MIN),
        }
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
            ((OP_FONT as u64) << OPCODE_SHIFT)
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
    pub const fn macro_definition(flags: MeaningFlags, definition: DefinitionRef<G>) -> Self {
        Self::new(
            ((OP_MACRO as u64) << OPCODE_SHIFT) | ((flags.bits() as u64) << FLAGS_SHIFT),
            definition.packed(),
        )
    }

    #[inline(always)]
    const fn opcode(&self) -> u8 {
        (self.command >> OPCODE_SHIFT) as u8
    }

    #[must_use]
    #[inline(always)]
    pub const fn is_macro(&self) -> bool {
        self.opcode() == OP_MACRO
    }

    #[must_use]
    #[inline(always)]
    pub const fn is_font(&self) -> bool {
        self.opcode() == OP_FONT
    }

    /// Classifies the actual command without projecting a coordinate.
    #[must_use]
    #[inline(always)]
    pub const fn control_code(&self) -> Option<u64> {
        match self.opcode() {
            OP_MACRO => None,
            RAW_SCALAR => Some(Meaning::runtime_word_control_code(self.coordinate.get())),
            _ => Some(Meaning::runtime_word_control_code(self.command)),
        }
    }

    #[must_use]
    #[inline(always)]
    pub const fn scalar_word(&self) -> Option<u64> {
        match self.opcode() {
            OP_MACRO | OP_FONT => None,
            RAW_SCALAR => Some(self.coordinate.get()),
            _ => Some(self.command),
        }
    }

    #[must_use]
    #[inline(always)]
    pub const fn macro_flags(&self) -> Option<MeaningFlags> {
        if self.is_macro() {
            Some(MeaningFlags::from_bits((self.command >> FLAGS_SHIFT) as u8))
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
        let flags = MeaningFlags::from_bits((self.command >> FLAGS_SHIFT) as u8);
        Some((flags, DefinitionRef::from_packed(self.coordinate)))
    }

    #[must_use]
    #[inline(always)]
    pub const fn font_id(&self) -> Option<FontId> {
        if self.opcode() != OP_FONT {
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

    #[must_use]
    #[inline(always)]
    pub const fn is_outer(&self) -> bool {
        self.command == Meaning::END_TEMPLATE_WORD
            || (self.is_macro()
                && MeaningFlags::from_bits((self.command >> FLAGS_SHIFT) as u8)
                    .contains(MeaningFlags::OUTER))
    }

    #[must_use]
    pub fn resolve(&self) -> ResolvedMeaning<G> {
        if let Some(word) = self.scalar_word() {
            ResolvedMeaning::Static(Meaning::decode_stored(word))
        } else if let Some(font) = self.font_id() {
            ResolvedMeaning::Static(Meaning::Font(font))
        } else {
            let (flags, definition) = self.macro_parts().expect("native macro");
            ResolvedMeaning::Macro { flags, definition }
        }
    }

    #[must_use]
    pub(crate) const fn static_meaning(&self) -> Option<Meaning> {
        if let Some(word) = self.scalar_word() {
            Some(Meaning::decode_stored(word))
        } else if let Some(font) = self.font_id() {
            Some(Meaning::Font(font))
        } else {
            None
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
