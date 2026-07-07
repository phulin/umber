//! Meaning word encoding and decoding.

use crate::ids::TokenListId;

const OPCODE_SHIFT: u32 = 56;
const FLAGS_SHIFT: u32 = 48;
const OPERAND_MASK: u64 = (1 << FLAGS_SHIFT) - 1;

const OP_UNDEFINED: u8 = 0;
const OP_RELAX: u8 = 1;
const OP_MACRO: u8 = 2;
const OP_CHAR_GIVEN: u8 = 3;

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
        token_list: TokenListId,
    },
    CharGiven(char),
    Unknown(RawMeaning),
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
            Self::Macro { flags, token_list } => pack(OP_MACRO, flags, token_list.raw() as u64),
            Self::CharGiven(ch) => pack(OP_CHAR_GIVEN, MeaningFlags::EMPTY, ch as u64),
            Self::Unknown(raw) => pack(raw.op, MeaningFlags::EMPTY, raw.operand),
        }
    }

    /// Decodes `opcode:8 | flags:8 | operand:48` into a typed meaning.
    #[must_use]
    pub const fn decode(word: u64) -> Self {
        let op = (word >> OPCODE_SHIFT) as u8;
        let flags = MeaningFlags::from_bits((word >> FLAGS_SHIFT) as u8);
        let operand = word & OPERAND_MASK;

        match op {
            OP_UNDEFINED => Self::Undefined,
            OP_RELAX => Self::Relax,
            OP_MACRO => Self::Macro {
                flags,
                token_list: TokenListId::new(operand as u32),
            },
            OP_CHAR_GIVEN => match char::from_u32(operand as u32) {
                Some(ch) => Self::CharGiven(ch),
                None => Self::Unknown(RawMeaning { op, operand }),
            },
            _ => Self::Unknown(RawMeaning { op, operand }),
        }
    }
}

const fn pack(op: u8, flags: MeaningFlags, operand: u64) -> u64 {
    assert!(operand <= OPERAND_MASK, "meaning operand exceeds 48 bits");
    ((op as u64) << OPCODE_SHIFT) | ((flags.bits() as u64) << FLAGS_SHIFT) | operand
}

#[cfg(test)]
mod tests {
    use super::{Meaning, MeaningFlags, OPERAND_MASK, RawMeaning};
    use crate::ids::TokenListId;

    fn round_trip(meaning: Meaning) {
        assert_eq!(Meaning::decode(meaning.encode()), meaning);
    }

    #[test]
    fn undefined_is_the_all_zero_word() {
        // Fresh zeroed meaning segments decode as Undefined, so this exact
        // encoding is required for fresh-segment correctness.
        assert_eq!(Meaning::Undefined.encode(), 0);
        assert_eq!(Meaning::decode(0), Meaning::Undefined);
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
            token_list: TokenListId::new(0),
        });
        round_trip(Meaning::Macro {
            flags: MeaningFlags::EMPTY,
            token_list: TokenListId::new(u32::MAX),
        });
        round_trip(Meaning::CharGiven('\0'));
        round_trip(Meaning::CharGiven(char::MAX));
        round_trip(Meaning::Unknown(RawMeaning::testing_new(u8::MAX, 0)));
        round_trip(Meaning::Unknown(RawMeaning::testing_new(
            u8::MAX,
            OPERAND_MASK,
        )));
    }

    #[test]
    fn unknown_meaning_exposes_raw_parts_without_public_fields() {
        let word = Meaning::Unknown(RawMeaning::testing_new(200, 42)).encode();
        let Meaning::Unknown(raw) = Meaning::decode(word) else {
            panic!("expected unknown meaning");
        };

        assert_eq!(raw.op(), 200);
        assert_eq!(raw.operand(), 42);
        assert_eq!(Meaning::Unknown(raw).encode(), word);
    }
}
