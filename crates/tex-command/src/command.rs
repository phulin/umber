//! Ephemeral current-command representation.

use tex_state::CommandContext;
use tex_state::interner::Symbol;
use tex_state::meaning::Meaning;
use tex_state::token::{Catcode, Token, TracedTokenWord};

/// One command delivery, equivalent to TeX's `cur_cmd`, `cur_chr`, `cur_cs`,
/// and `cur_tok`.
///
/// This value is call-local: it is absent at durable named checkpoints and
/// intentionally has no serialization or snapshot representation. Its
/// delivery stamp identifies the exact live cursor transition and is not
/// reconstructed from token equality.
#[derive(Debug, Eq, PartialEq)]
pub struct CurrentCommand {
    spelling: TracedTokenWord,
    meaning: Meaning,
    control_sequence: Option<Symbol>,
    delivery: DeliveryStamp,
}

impl CurrentCommand {
    /// Resolves one delivered spelling into TeX's effective current command.
    ///
    /// TeX82's `get_next` preserves the input token as `cur_tok` while it
    /// obtains `cur_cmd`/`cur_chr` from either that character spelling or the
    /// current meaning of its control sequence. Active characters use their
    /// separate control-sequence namespace; escaped control sequences retain
    /// their original spelling even after their meaning changes.
    #[allow(dead_code)] // invoked by the ordered canonical raw-delivery implementation
    pub(crate) fn resolve(
        spelling: TracedTokenWord,
        delivery: DeliveryStamp,
        state: &mut CommandContext<'_>,
    ) -> Self {
        let token = spelling.semantic_token();
        let (control_sequence, meaning) = match token {
            Token::Cs(symbol) => (Some(symbol), state.meaning(symbol)),
            Token::Char {
                ch,
                cat: Catcode::Active,
            } => {
                let symbol = state.intern_active_character(ch);
                (Some(symbol), state.meaning(symbol))
            }
            Token::Char { ch, cat } => (None, Meaning::CharToken { ch, cat }),
            // `out_param` is converted to a literal replay token before
            // meaning resolution (TeX.web, get_next). A stray parameter token
            // is nevertheless represented deterministically while recovery
            // remains the responsibility of the raw delivery loop.
            Token::Param(_) => (None, Meaning::Undefined),
            Token::Frozen(_) => (
                None,
                state
                    .frozen_primitive_meaning(token)
                    .unwrap_or(Meaning::Undefined),
            ),
        };
        Self {
            spelling,
            meaning,
            control_sequence,
            delivery,
        }
    }

    /// Returns the original token spelling, including its delivery origin.
    #[must_use]
    pub const fn spelling(&self) -> TracedTokenWord {
        self.spelling
    }

    /// Returns the effective meaning resolved at this delivery.
    #[must_use]
    pub const fn meaning(&self) -> Meaning {
        self.meaning
    }

    /// Returns the control-sequence identity, if this spelling resolves via
    /// a control-sequence meaning cell.
    #[must_use]
    pub const fn control_sequence(&self) -> Option<Symbol> {
        self.control_sequence
    }

    /// Returns the spelling's diagnostic origin.
    #[must_use]
    pub const fn origin(&self) -> tex_state::token::OriginId {
        self.spelling.origin()
    }

    /// Returns the execution-local proof of this exact input delivery.
    #[must_use]
    pub const fn delivery_stamp(&self) -> DeliveryStamp {
        self.delivery
    }
}

/// Proof of the exact input position that delivered a current command.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct DeliveryStamp {
    input_level: u64,
    position: u64,
}

impl DeliveryStamp {
    /// Constructs the stamp for the input-level position consumed by this
    /// delivery. Only the canonical raw-delivery loop may mint stamps.
    #[allow(dead_code)] // minted by the ordered canonical raw-delivery implementation
    pub(crate) const fn new(input_level: u64, position: u64) -> Self {
        Self {
            input_level,
            position,
        }
    }

    /// Returns the stable identity of the level that delivered the token.
    #[must_use]
    pub const fn input_level(&self) -> u64 {
        self.input_level
    }

    /// Returns the exact pre-retirement cursor position within that level.
    #[must_use]
    pub const fn position(&self) -> u64 {
        self.position
    }
}

#[cfg(test)]
mod tests;
