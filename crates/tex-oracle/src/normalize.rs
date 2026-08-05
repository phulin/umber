use serde::{Deserialize, Serialize};

use crate::Event;

/// Event plus its deterministic zero-based position in the stream.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NormalizedEvent {
    pub sequence: u64,
    pub semantic: Event,
}

/// Stateless-value normalizer plus deterministic event numbering.
///
/// Canonicalization is intentionally narrow: it converts CRLF/CR in textual
/// fields to LF. Semantic atoms such as control-sequence spellings are left
/// byte-for-byte intact. It does not hide semantic values, reorder events,
/// rewrite source names, or inspect host paths.
#[derive(Clone, Debug, Default)]
pub struct Normalizer {
    next_sequence: u64,
}

impl Normalizer {
    #[must_use]
    pub const fn new() -> Self {
        Self { next_sequence: 0 }
    }

    #[must_use]
    pub fn normalize(&mut self, mut event: Event) -> NormalizedEvent {
        normalize_event(&mut event);
        let sequence = self.next_sequence;
        self.next_sequence = self
            .next_sequence
            .checked_add(1)
            .expect("oracle event sequence overflowed");
        NormalizedEvent {
            sequence,
            semantic: event,
        }
    }
}

fn normalize_event(event: &mut Event) {
    event.view_mut().normalize();
}
