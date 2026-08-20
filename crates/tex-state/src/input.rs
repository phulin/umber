//! Storage-independent input and provenance vocabulary.
//!
//! Live input stacks belong to `tex-command`; detached command input belongs
//! to its continuation schema. This module deliberately contains only the
//! small values shared with state-owned source maps, provenance, and error
//! presentation. It owns no token list, origin list, source backing, or
//! snapshot graph.

use crate::token::{Catcode, Token};

/// The two non-depth sentinels assigned by TeX's alignment driver.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum AlignmentScannerPhase {
    /// Preamble scanning: top-level `&` and `\cr` delimit templates.
    Preamble,
    /// Row lookahead and u-template replay: delimiter interception is disabled.
    BetweenEntries,
}

impl AlignmentScannerPhase {
    /// Returns TeX's sentinel `align_state` value for this scanner phase.
    #[must_use]
    pub const fn align_state(self) -> i32 {
        match self {
            Self::Preamble => -1_000_000,
            Self::BetweenEntries => 1_000_000,
        }
    }
}

/// Which immutable replay characters a direct span consumer accepts.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum LiteralSpanPolicy {
    /// Characters inert to an expanded replacement scanner.
    ExpandedReplacement,
    /// Ordinary text characters accepted by horizontal main control.
    HorizontalText,
}

impl LiteralSpanPolicy {
    #[must_use]
    pub const fn accepts(self, token: Token) -> bool {
        match (self, token) {
            (
                Self::ExpandedReplacement,
                Token::Char {
                    cat:
                        Catcode::BeginGroup | Catcode::EndGroup | Catcode::Parameter | Catcode::Active,
                    ..
                },
            ) => false,
            (Self::ExpandedReplacement, Token::Char { .. }) => true,
            (
                Self::HorizontalText,
                Token::Char {
                    cat: Catcode::Letter | Catcode::Other | Catcode::Space,
                    ..
                },
            ) => true,
            (
                Self::ExpandedReplacement | Self::HorizontalText,
                Token::Cs(_) | Token::Param(_) | Token::Frozen(_),
            )
            | (Self::HorizontalText, Token::Char { .. }) => false,
        }
    }
}

/// Generation/session-owned source coordinate used only while its source
/// registry is live. Detached boundaries translate it to a source recipe.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SourceId(u32);

impl SourceId {
    #[must_use]
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }
}

/// Why a token list is being replayed, used only for diagnostics and compact
/// provenance records.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TokenListReplayKind {
    MacroBody,
    MacroArgument,
    NoExpand,
    BackedUp,
    Unexpanded,
    EveryPar,
    EveryHBox,
    EveryVBox,
    EveryJob,
    EveryCr,
    Mark,
    OutputRoutine,
    Inserted,
    ScantokensEveryEof,
    AlignmentUTemplate,
    AlignmentVTemplate,
}
