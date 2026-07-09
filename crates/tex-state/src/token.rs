//! Token and catcode values.

use crate::interner::Symbol;

/// TeX category codes, shared by lexing and token storage.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Catcode {
    Escape = 0,
    BeginGroup = 1,
    EndGroup = 2,
    MathShift = 3,
    AlignmentTab = 4,
    EndLine = 5,
    Parameter = 6,
    Superscript = 7,
    Subscript = 8,
    Ignored = 9,
    Space = 10,
    Letter = 11,
    Other = 12,
    Active = 13,
    Comment = 14,
    Invalid = 15,
}

/// A frozen TeX token.
///
/// Parameter tokens carry the future macro parameter slot number, `1..=9`.
/// Macro matching semantics live in the gullet; this type only stores the
/// compact token representation.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Token {
    Char { ch: char, cat: Catcode },
    Cs(Symbol),
    Param(u8),
}

impl Token {
    /// Creates a macro-parameter token.
    #[must_use]
    pub const fn param(slot: u8) -> Self {
        assert!(
            slot >= 1 && slot <= 9,
            "parameter token slot must be in 1..=9"
        );
        Self::Param(slot)
    }
}

const _: () = assert!(core::mem::size_of::<Token>() == 8);

/// Provenance origin handle carried by traced token words.
///
/// `OriginId(0)` is reserved for the Unknown/Bootstrap origin. Later
/// provenance allocation saturates overflow to this id instead of failing
/// compilation, because provenance is diagnostic data rather than TeX
/// semantics.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct OriginId(u32);

impl OriginId {
    /// Unknown or bootstrap provenance.
    pub const UNKNOWN: Self = Self(0);

    /// Returns the packed origin id value.
    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }

    /// Reconstructs an origin id from its packed representation.
    #[must_use]
    pub const fn from_raw(raw: u32) -> Self {
        Self(raw)
    }
}

const _: () = assert!(core::mem::size_of::<OriginId>() == 4);

/// A token plus mandatory provenance packed into one word.
///
/// Layout: bits 63..62 are token kind, bits 61..32 are a 30-bit token payload,
/// and bits 31..0 are `OriginId`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TracedTokenWord(u64);

impl TracedTokenWord {
    const KIND_SHIFT: u32 = 62;
    const PAYLOAD_SHIFT: u32 = 32;
    const PAYLOAD_MASK: u32 = (1 << 30) - 1;
    const KIND_CHAR: u64 = 0;
    const KIND_CS: u64 = 1;
    const KIND_PARAM: u64 = 2;
    const CATCODE_BITS: u32 = 4;
    const USV_BITS: u32 = 21;
    const USV_MASK: u32 = (1 << Self::USV_BITS) - 1;

    /// Packs a semantic token with its origin.
    #[must_use]
    pub fn pack(token: Token, origin: OriginId) -> Self {
        let (kind, payload) = match token {
            Token::Char { ch, cat } => {
                let payload = ((ch as u32) << Self::CATCODE_BITS) | cat as u32;
                debug_assert!(payload <= Self::PAYLOAD_MASK);
                (Self::KIND_CHAR, payload)
            }
            Token::Cs(symbol) => {
                debug_assert!(symbol.raw() <= Self::PAYLOAD_MASK);
                (Self::KIND_CS, symbol.raw())
            }
            Token::Param(slot) => {
                debug_assert!(slot < 16);
                (Self::KIND_PARAM, u32::from(slot))
            }
        };
        Self(
            (kind << Self::KIND_SHIFT)
                | (u64::from(payload) << Self::PAYLOAD_SHIFT)
                | u64::from(origin.raw()),
        )
    }

    /// Returns the raw packed word.
    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }

    /// Reconstructs a traced word from raw bits.
    #[must_use]
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    /// Returns the token origin.
    #[must_use]
    pub const fn origin(self) -> OriginId {
        OriginId::from_raw(self.0 as u32)
    }

    /// Unpacks the semantic token, or `None` if the raw word is not a valid token.
    #[must_use]
    pub fn token(self) -> Option<Token> {
        let kind = self.0 >> Self::KIND_SHIFT;
        let payload = ((self.0 >> Self::PAYLOAD_SHIFT) as u32) & Self::PAYLOAD_MASK;
        match kind {
            Self::KIND_CHAR => unpack_char_payload(payload),
            Self::KIND_CS => Some(Token::Cs(Symbol::new(payload))),
            Self::KIND_PARAM => match payload {
                1..=9 => Some(Token::Param(payload as u8)),
                _ => None,
            },
            _ => None,
        }
    }

    /// Unpacks both token and origin, or `None` if the raw token bits are invalid.
    #[must_use]
    pub fn unpack(self) -> Option<(Token, OriginId)> {
        Some((self.token()?, self.origin()))
    }
}

const _: () = assert!(core::mem::size_of::<TracedTokenWord>() == 8);

fn unpack_char_payload(payload: u32) -> Option<Token> {
    let cat = catcode_from_raw((payload & 0xF) as u8)?;
    let usv = (payload >> TracedTokenWord::CATCODE_BITS) & TracedTokenWord::USV_MASK;
    Some(Token::Char {
        ch: char::from_u32(usv)?,
        cat,
    })
}

fn catcode_from_raw(raw: u8) -> Option<Catcode> {
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
