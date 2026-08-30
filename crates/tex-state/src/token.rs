//! Token and catcode values.

use crate::interner::Symbol;

/// Inaccessible TeX82 control tokens used only by engine-owned input replay.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct FrozenToken(u16);

impl FrozenToken {
    pub(crate) const END_TEMPLATE: Self = Self(0);
    pub(crate) const END_V: Self = Self(1);
    pub(crate) const RELAX: Self = Self(60_000);
    /// TeX82 §222's `undefined_control_sequence`: the single dummy eqtb
    /// location `id_lookup` returns (§259) whenever a multiletter name is
    /// absent from the hash table and `no_new_control_sequence` is set. It is
    /// never a hash entry, so it can carry no interned spelling.
    pub(crate) const UNDEFINED_CONTROL_SEQUENCE: Self = Self(u16::MAX - 1);
    const PRIMITIVE_BASE: u16 = 2;
    /// The lowest reserved sentinel value. Every frozen token at or above it
    /// is an engine sentinel rather than a registered primitive, so the
    /// primitive range is closed by one bound instead of by a growing list of
    /// individually excluded values.
    const SENTINEL_BASE: u16 = Self::RELAX.0;

    pub(crate) const fn primitive(index: u16) -> Self {
        assert!(
            index < Self::SENTINEL_BASE - Self::PRIMITIVE_BASE,
            "frozen primitive index overlaps the sentinel range"
        );
        Self(Self::PRIMITIVE_BASE + index)
    }

    #[must_use]
    pub const fn primitive_index(self) -> Option<u16> {
        if self.0 >= Self::PRIMITIVE_BASE && self.0 < Self::SENTINEL_BASE {
            Some(self.0 - Self::PRIMITIVE_BASE)
        } else {
            None
        }
    }

    #[must_use]
    pub(crate) const fn raw(self) -> u16 {
        self.0
    }

    #[must_use]
    pub(crate) const fn from_raw(raw: u16) -> Self {
        Self(raw)
    }
}

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

impl Catcode {
    #[must_use]
    pub const fn from_raw(raw: u8) -> Option<Self> {
        catcode_from_raw(raw)
    }
}

/// A frozen TeX token.
///
/// Parameter tokens carry the future macro parameter slot number, `1..=9`.
/// Frozen tokens are inaccessible engine sentinels; the lexer and control-
/// sequence interner cannot manufacture them. Macro matching and sentinel
/// delivery semantics live in the gullet; this type only stores the compact
/// token representation.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Token {
    Char {
        ch: char,
        cat: Catcode,
    },
    Cs(Symbol),
    Param(u8),
    /// An inaccessible engine-owned token, never an interned control sequence.
    Frozen(FrozenToken),
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

    /// Returns TeX82's inaccessible `frozen_end_template` token.
    #[must_use]
    pub(crate) const fn frozen_end_template() -> Self {
        Self::Frozen(FrozenToken::END_TEMPLATE)
    }

    /// Returns TeX82's inaccessible `frozen_endv` token.
    #[must_use]
    pub(crate) const fn frozen_endv() -> Self {
        Self::Frozen(FrozenToken::END_V)
    }

    /// Returns TeX82's inaccessible `frozen_relax` token.
    #[must_use]
    pub const fn frozen_relax() -> Self {
        Self::Frozen(FrozenToken::RELAX)
    }

    pub(crate) const fn frozen_primitive(index: u16) -> Self {
        Self::Frozen(FrozenToken::primitive(index))
    }

    /// Whether this is TeX82's inaccessible `frozen_end_template` token.
    #[must_use]
    pub const fn is_frozen_end_template(self) -> bool {
        matches!(self, Self::Frozen(FrozenToken::END_TEMPLATE))
    }

    /// Whether this is TeX82's inaccessible `frozen_endv` token.
    #[must_use]
    pub const fn is_frozen_endv(self) -> bool {
        matches!(self, Self::Frozen(FrozenToken::END_V))
    }

    /// Whether this is TeX82's inaccessible `frozen_relax` token.
    #[must_use]
    pub const fn is_frozen_relax(self) -> bool {
        matches!(self, Self::Frozen(FrozenToken::RELAX))
    }

    /// Returns TeX82 §222's dummy `undefined_control_sequence` location.
    ///
    /// §259's `id_lookup` yields it for every multiletter name that is not
    /// already in the hash table while `no_new_control_sequence` holds, so
    /// all such names share this one inaccessible identity.
    #[must_use]
    pub const fn undefined_control_sequence() -> Self {
        Self::Frozen(FrozenToken::UNDEFINED_CONTROL_SEQUENCE)
    }

    /// Whether this is TeX82 §222's dummy `undefined_control_sequence`.
    #[must_use]
    pub const fn is_undefined_control_sequence(self) -> bool {
        matches!(self, Self::Frozen(FrozenToken::UNDEFINED_CONTROL_SEQUENCE))
    }
}

const _: () = assert!(core::mem::size_of::<Token>() == 8);

/// Canonical token-only word used by the compact command core.
///
/// Bits 31..30 identify character, control-sequence, parameter, or frozen
/// tokens. Bits 29..0 contain the exact operand. Source and provenance
/// coordinates deliberately live beside this value rather than inside it.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TokenWord(u32);

impl TokenWord {
    const KIND_SHIFT: u32 = 30;
    const PAYLOAD_MASK: u32 = (1 << Self::KIND_SHIFT) - 1;
    const KIND_CHAR: u32 = 0;
    const KIND_CS: u32 = 1;
    const KIND_PARAM: u32 = 2;
    const KIND_FROZEN: u32 = 3;
    const CATCODE_BITS: u32 = 4;

    /// Packs one exact semantic token without source or provenance ownership.
    #[must_use]
    pub fn pack(token: Token) -> Self {
        let (kind, payload) = match token {
            Token::Char { ch, cat } => (
                Self::KIND_CHAR,
                ((ch as u32) << Self::CATCODE_BITS) | cat as u32,
            ),
            Token::Cs(symbol) => {
                assert!(
                    symbol.raw() <= Self::PAYLOAD_MASK,
                    "control-sequence symbol exceeds packed token capacity"
                );
                (Self::KIND_CS, symbol.raw())
            }
            Token::Param(slot) => {
                assert!(
                    (1..=9).contains(&slot),
                    "parameter token slot must be in 1..=9"
                );
                (Self::KIND_PARAM, u32::from(slot))
            }
            Token::Frozen(token) => (Self::KIND_FROZEN, u32::from(token.raw())),
        };
        debug_assert!(payload <= Self::PAYLOAD_MASK);
        Self((kind << Self::KIND_SHIFT) | payload)
    }

    /// Unpacks a token word after validating its kind-specific operand.
    #[must_use]
    pub fn token(self) -> Option<Token> {
        let kind = self.0 >> Self::KIND_SHIFT;
        let payload = self.0 & Self::PAYLOAD_MASK;
        match kind {
            Self::KIND_CHAR => unpack_char_payload(payload),
            Self::KIND_CS => Some(Token::Cs(Symbol::from_packed_slot(payload))),
            Self::KIND_PARAM => match payload {
                1..=9 => Some(Token::Param(payload as u8)),
                _ => None,
            },
            Self::KIND_FROZEN if payload <= u16::MAX as u32 => {
                Some(Token::Frozen(FrozenToken::from_raw(payload as u16)))
            }
            _ => None,
        }
    }

    /// Decodes a word whose validity was established at construction or
    /// arena admission.
    #[must_use]
    #[inline(always)]
    pub fn semantic_token(self) -> Token {
        let kind = self.0 >> Self::KIND_SHIFT;
        let payload = self.0 & Self::PAYLOAD_MASK;
        match kind {
            Self::KIND_CHAR => {
                let raw_cat = (payload & 0xF) as usize;
                let ch = char::from_u32(payload >> Self::CATCODE_BITS)
                    .expect("packed token scalar is valid");
                Token::Char {
                    ch,
                    cat: ALL_CATCODES[raw_cat],
                }
            }
            Self::KIND_CS => Token::Cs(Symbol::from_packed_slot(payload)),
            Self::KIND_PARAM => Token::Param(payload as u8),
            Self::KIND_FROZEN => Token::Frozen(FrozenToken::from_raw(payload as u16)),
            _ => unreachable!("two-bit packed-token kind"),
        }
    }

    /// Returns the macro out-parameter slot encoded by this word, if any.
    ///
    /// Raw input delivery uses this narrow projection while it already holds
    /// a resident stored-token word. Ordinary source characters and literal
    /// macro arguments never need to reconstruct a complete [`Token`] merely
    /// to reject this one internal token kind.
    #[must_use]
    #[inline(always)]
    pub const fn out_parameter_slot(self) -> Option<u8> {
        if self.0 >> Self::KIND_SHIFT == Self::KIND_PARAM {
            Some((self.0 & Self::PAYLOAD_MASK) as u8)
        } else {
            None
        }
    }

    pub const fn raw(self) -> u32 {
        self.0
    }

    pub(crate) const fn from_raw(raw: u32) -> Self {
        Self(raw)
    }
}

const _: () = assert!(core::mem::size_of::<TokenWord>() == 4);

/// Provenance origin handle carried by traced token words.
///
/// `OriginId(0)` is reserved for the Unknown/Bootstrap origin. Later
/// provenance allocation saturates overflow to this id instead of failing
/// compilation, because provenance is diagnostic data rather than TeX
/// semantics.
#[derive(Clone, Copy, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct OriginId(u32);

impl OriginId {
    const ARENA_TAG: u32 = 1 << 31;
    const PAYLOAD_MASK: u32 = Self::ARENA_TAG - 1;

    /// Unknown or bootstrap provenance.
    pub const UNKNOWN: Self = Self(0);
    pub(crate) const NOEXPAND_FALLBACK: Self = Self(u32::MAX);

    /// Returns the packed origin id value.
    #[must_use]
    pub(crate) const fn raw(self) -> u32 {
        self.0
    }

    /// Reconstructs an origin id from its packed representation.
    #[must_use]
    pub(crate) const fn from_raw(raw: u32) -> Self {
        Self(raw)
    }

    #[must_use]
    pub(crate) const fn direct_source(position: crate::source_map::SourcePos) -> Option<Self> {
        let Some(raw) = position.raw().checked_add(1) else {
            return None;
        };
        if raw <= Self::PAYLOAD_MASK as u64 {
            Some(Self(raw as u32))
        } else {
            None
        }
    }

    #[must_use]
    pub(crate) const fn arena(index: u32) -> Option<Self> {
        if index < Self::PAYLOAD_MASK {
            Some(Self(Self::ARENA_TAG | index))
        } else {
            None
        }
    }

    #[must_use]
    pub(crate) const fn decode(self) -> OriginEncoding {
        if self.0 == Self::NOEXPAND_FALLBACK.0 {
            OriginEncoding::NoExpandFallback
        } else if self.0 == 0 {
            OriginEncoding::Unknown
        } else if self.0 & Self::ARENA_TAG == 0 {
            OriginEncoding::DirectSource(crate::source_map::SourcePos::from_origin_payload(
                self.0 - 1,
            ))
        } else {
            OriginEncoding::Arena(self.0 & Self::PAYLOAD_MASK)
        }
    }

    /// Benchmark-only inspection used to count direct deliveries without a
    /// production hot-path counter write.
    #[cfg(feature = "testing")]
    #[must_use]
    pub const fn is_direct_source(self) -> bool {
        self.0 != 0 && self.0 & Self::ARENA_TAG == 0
    }
}

impl core::fmt::Debug for OriginId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("OriginId(..)")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OriginEncoding {
    Unknown,
    NoExpandFallback,
    DirectSource(crate::source_map::SourcePos),
    Arena(u32),
}

const _: () = assert!(core::mem::size_of::<OriginId>() == 4);

/// A token plus mandatory provenance packed into one word.
///
/// Layout: bits 63..62 are token kind, bits 61..32 are a 30-bit token payload,
/// and bits 31..0 are `OriginId`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TracedTokenWord(u64);

impl TracedTokenWord {
    const PAYLOAD_SHIFT: u32 = 32;

    /// Packs a semantic token with its origin.
    #[must_use]
    pub fn pack(token: Token, origin: OriginId) -> Self {
        Self::from_parts(TokenWord::pack(token), origin)
    }

    /// Combines independently packed semantic and source-coordinate words.
    #[must_use]
    pub const fn from_parts(token: TokenWord, origin: OriginId) -> Self {
        Self((token.raw() as u64) << Self::PAYLOAD_SHIFT | origin.raw() as u64)
    }

    /// Reconstructs a traced word from raw bits.
    #[cfg(test)]
    #[must_use]
    pub(crate) const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    /// Returns the token origin.
    #[must_use]
    pub const fn origin(self) -> OriginId {
        OriginId::from_raw(self.0 as u32)
    }

    /// Returns the token-only word without changing its semantic identity.
    #[must_use]
    pub const fn token_word(self) -> TokenWord {
        TokenWord((self.0 >> Self::PAYLOAD_SHIFT) as u32)
    }

    /// Unpacks the semantic token, or `None` if the raw word is not a valid token.
    #[must_use]
    pub fn token(self) -> Option<Token> {
        self.token_word().token()
    }

    /// Unpacks the semantic token through the validity invariant established
    /// by [`Self::pack`]. Unlike [`Self::token`], this hot-path operation does
    /// not revalidate an opaque value that external code cannot construct.
    #[must_use]
    #[inline(always)]
    pub fn semantic_token(self) -> Token {
        self.token_word().semantic_token()
    }

    /// Unpacks both token and origin, or `None` if the raw token bits are invalid.
    #[must_use]
    pub fn unpack(self) -> Option<(Token, OriginId)> {
        Some((self.token()?, self.origin()))
    }
}

const _: () = assert!(core::mem::size_of::<TracedTokenWord>() == 8);

const ALL_CATCODES: [Catcode; 16] = [
    Catcode::Escape,
    Catcode::BeginGroup,
    Catcode::EndGroup,
    Catcode::MathShift,
    Catcode::AlignmentTab,
    Catcode::EndLine,
    Catcode::Parameter,
    Catcode::Superscript,
    Catcode::Subscript,
    Catcode::Ignored,
    Catcode::Space,
    Catcode::Letter,
    Catcode::Other,
    Catcode::Active,
    Catcode::Comment,
    Catcode::Invalid,
];

fn unpack_char_payload(payload: u32) -> Option<Token> {
    let cat = catcode_from_raw((payload & 0xF) as u8)?;
    let usv = payload >> TokenWord::CATCODE_BITS;
    Some(Token::Char {
        ch: char::from_u32(usv)?,
        cat,
    })
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
