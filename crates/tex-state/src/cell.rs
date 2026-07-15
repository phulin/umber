//! Packed environment cell identifiers.

const BANK_SHIFT: u32 = 31;
const GLOBAL_SHIFT: u32 = 30;
const INDEX_MASK: u32 = (1 << GLOBAL_SHIFT) - 1;

/// The bank tag encoded in a [`CellId`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[repr(u8)]
pub enum BankTag {
    Meaning = 0,
    Count = 1,
    Dimen = 2,
    Skip = 3,
    Toks = 4,
    Box = 5,
    IntParam = 6,
    DimenParam = 7,
    GlueParam = 8,
    TokParam = 9,
    Muskip = 10,
    FontDimen = 11,
    FontParamLen = 12,
    FontHyphenChar = 13,
    FontSkewChar = 14,
    CurrentFont = 15,
    MathFamilyFont = 16,
    PdfLpCode = 17,
    PdfRpCode = 18,
    PdfEfCode = 19,
    PdfTagCode = 20,
    PdfKnbsCode = 21,
    PdfStbsCode = 22,
    PdfShbsCode = 23,
    PdfKnbcCode = 24,
    PdfKnacCode = 25,
    PdfNoLigatures = 26,
}

impl BankTag {
    pub(crate) const fn from_bits(bits: u32) -> Self {
        match bits {
            0 => Self::Meaning,
            1 => Self::Count,
            2 => Self::Dimen,
            3 => Self::Skip,
            4 => Self::Toks,
            5 => Self::Box,
            6 => Self::IntParam,
            7 => Self::DimenParam,
            8 => Self::GlueParam,
            9 => Self::TokParam,
            10 => Self::Muskip,
            11 => Self::FontDimen,
            12 => Self::FontParamLen,
            13 => Self::FontHyphenChar,
            14 => Self::FontSkewChar,
            15 => Self::CurrentFont,
            16 => Self::MathFamilyFont,
            17 => Self::PdfLpCode,
            18 => Self::PdfRpCode,
            19 => Self::PdfEfCode,
            20 => Self::PdfTagCode,
            21 => Self::PdfKnbsCode,
            22 => Self::PdfStbsCode,
            23 => Self::PdfShbsCode,
            24 => Self::PdfKnbcCode,
            25 => Self::PdfKnacCode,
            26 => Self::PdfNoLigatures,
            _ => panic!("unknown cell bank tag"),
        }
    }
}

/// A packed environment cell id: `bank:5 | global:1 | index:30`.
///
/// The 30-bit index matches the complete compact [`crate::interner::Symbol`]
/// domain. The key uses 36 bits of a `u64`; widening from `u32` does not grow
/// journal undo records because their following `u64` words already imposed
/// eight-byte alignment.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CellId(u64);

impl CellId {
    #[allow(dead_code)]
    pub(crate) const fn new(bank: BankTag, index: u32) -> Self {
        assert!(index <= INDEX_MASK, "cell index exceeds 30 bits");
        Self(((bank as u64) << BANK_SHIFT) | index as u64)
    }

    #[allow(dead_code)]
    pub(crate) const fn new_global(bank: BankTag, index: u32) -> Self {
        assert!(index <= INDEX_MASK, "cell index exceeds 30 bits");
        Self(((bank as u64) << BANK_SHIFT) | (1_u64 << GLOBAL_SHIFT) | index as u64)
    }

    /// Decodes a detached raw cell key, rejecting reserved bank tags and bits.
    #[must_use]
    pub(crate) const fn from_raw(raw: u64) -> Option<Self> {
        let bank = raw >> BANK_SHIFT;
        if bank <= BankTag::PdfNoLigatures as u64 {
            Some(Self(raw))
        } else {
            None
        }
    }

    /// Returns the raw packed cell id.
    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }

    /// Returns the encoded bank tag.
    #[must_use]
    pub const fn bank(self) -> BankTag {
        BankTag::from_bits((self.0 >> BANK_SHIFT) as u32)
    }

    /// Returns whether this cell id marks a global assignment journal record.
    #[must_use]
    pub const fn is_global(self) -> bool {
        (self.0 & (1_u64 << GLOBAL_SHIFT)) != 0
    }

    /// Returns the encoded cell index.
    #[must_use]
    pub const fn index(self) -> u32 {
        (self.0 as u32) & INDEX_MASK
    }
}

#[cfg(test)]
mod tests;
