//! Ephemeral current-command representation.

use tex_state::CommandContext;
use tex_state::interner::Symbol;
use tex_state::meaning::Meaning;
use tex_state::token::{Catcode, Token, TracedTokenWord};

use crate::SourceRange;

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
    source_range: Option<SourceRange>,
    alignment_adjustment: crate::processor::AlignmentDeliveryAdjustment,
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
        source_range: Option<SourceRange>,
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
            source_range,
            alignment_adjustment: crate::processor::AlignmentDeliveryAdjustment::None,
        }
    }

    /// Replaces the effective meaning while retaining the exact delivered
    /// spelling and stamp. This is solely TeX82's one-delivery `\\noexpand`
    /// treatment in `get_next` (TeX.web §379).
    pub(crate) fn suppress_expandable(&mut self) {
        if matches!(
            self.meaning,
            Meaning::Macro { .. } | Meaning::ExpandablePrimitive(_)
        ) {
            self.meaning = Meaning::Relax;
        }
    }

    /// Converts the effective current command to TeX82's recovery space
    /// while preserving the original spelling for diagnostics and exact input
    /// replay. This is the final step of `check_outer_validity`.
    pub(crate) fn recover_as_space(&mut self) {
        self.meaning = Meaning::CharToken {
            ch: ' ',
            cat: Catcode::Space,
        };
        self.control_sequence = None;
    }

    /// Replaces an intercepted alignment terminator's effective meaning while
    /// preserving its spelling and delivery proof (TeX.web `get_next`).
    pub(crate) fn convert_to_end_template(&mut self) {
        self.meaning =
            Meaning::ExpandablePrimitive(tex_state::meaning::ExpandablePrimitive::EndTemplate);
        self.control_sequence = None;
    }

    pub(crate) fn set_alignment_adjustment(
        &mut self,
        adjustment: crate::processor::AlignmentDeliveryAdjustment,
    ) {
        self.alignment_adjustment = adjustment;
    }

    pub(crate) const fn alignment_adjustment(
        &self,
    ) -> crate::processor::AlignmentDeliveryAdjustment {
        self.alignment_adjustment
    }

    /// Whether this delivery is an outer macro command.
    pub(crate) const fn is_outer(&self) -> bool {
        matches!(
            self.meaning,
            Meaning::Macro { flags, .. } if flags.contains(tex_state::meaning::MeaningFlags::OUTER)
        )
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

    /// Returns the direct source spelling range when this command was
    /// delivered from a registered physical source rather than replayed.
    #[must_use]
    pub const fn source_range(&self) -> Option<SourceRange> {
        self.source_range
    }

    /// Makes a fresh copy for the input backup path. `CurrentCommand` itself
    /// remains deliberately non-`Clone` at the public boundary.
    pub(crate) const fn copy_for_backup(&self) -> Self {
        Self {
            spelling: self.spelling,
            meaning: self.meaning,
            control_sequence: self.control_sequence,
            delivery: self.delivery,
            source_range: self.source_range,
            alignment_adjustment: self.alignment_adjustment,
        }
    }
}

/// Proof of one exact input transition that delivered a current command.
///
/// Position identifies the cursor slot, while `sequence` distinguishes a
/// later delivery after that slot was rewound.  It is deliberately not a
/// provenance identity and is valid only within the processor episode that
/// minted it.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct DeliveryStamp {
    input_level: u64,
    position: u64,
    sequence: u64,
}

impl DeliveryStamp {
    /// Constructs the stamp for the input-level position consumed by this
    /// delivery. Only the canonical raw-delivery loop may mint stamps.
    #[allow(dead_code)] // minted by the ordered canonical raw-delivery implementation
    pub(crate) const fn new(input_level: u64, position: u64, sequence: u64) -> Self {
        Self {
            input_level,
            position,
            sequence,
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

    /// Returns the unique sequence within the live processor episode.
    #[must_use]
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }
}

#[cfg(test)]
mod tests;
