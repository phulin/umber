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

#[cfg(test)]
mod tests;
