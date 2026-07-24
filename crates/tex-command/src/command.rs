//! Ephemeral current-command representation.

use tex_state::interner::Symbol;
use tex_state::meaning::Meaning;
use tex_state::token::TracedTokenWord;

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

/// Proof of the exact input position that delivered a current command.
#[derive(Debug, Eq, PartialEq)]
struct DeliveryStamp {
    input_level: u64,
    position: u64,
}
