//! Packed environment cell identifiers.

const BANK_SHIFT: u32 = 27;
const GLOBAL_SHIFT: u32 = 26;
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
}

impl BankTag {
    const fn from_bits(bits: u32) -> Self {
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
            _ => panic!("unknown cell bank tag"),
        }
    }
}

/// A packed environment cell id: `bank:5 | global:1 | index:26`.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CellId(u32);

impl CellId {
    #[allow(dead_code)]
    pub(crate) const fn new(bank: BankTag, index: u32) -> Self {
        assert!(index <= INDEX_MASK, "cell index exceeds 26 bits");
        Self(((bank as u32) << BANK_SHIFT) | index)
    }

    #[allow(dead_code)]
    pub(crate) const fn new_global(bank: BankTag, index: u32) -> Self {
        assert!(index <= INDEX_MASK, "cell index exceeds 26 bits");
        Self(((bank as u32) << BANK_SHIFT) | (1 << GLOBAL_SHIFT) | index)
    }

    /// Returns the raw packed cell id.
    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }

    /// Returns the encoded bank tag.
    #[must_use]
    pub const fn bank(self) -> BankTag {
        BankTag::from_bits(self.0 >> BANK_SHIFT)
    }

    /// Returns whether this cell id marks a global assignment journal record.
    #[must_use]
    pub const fn is_global(self) -> bool {
        (self.0 & (1 << GLOBAL_SHIFT)) != 0
    }

    /// Returns the encoded cell index.
    #[must_use]
    pub const fn index(self) -> u32 {
        self.0 & INDEX_MASK
    }
}

#[cfg(test)]
mod tests;
