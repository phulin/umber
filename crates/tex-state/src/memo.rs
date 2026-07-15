//! Detached, schema-versioned values exchanged with incremental memo caches.
//!
//! The public envelope is deliberately opaque: callers can retain, persist,
//! and validate bytes, but cannot obtain a live store handle from them. Import
//! always runs through [`crate::Universe`] and publishes only after complete
//! decoding and validation.

use crate::Universe;
use crate::glue::{GlueSpec, Order};
use crate::ids::{GlueId, MacroDefinitionId, TokenListId};
use crate::interner::ControlSequenceKind;
use crate::macro_store::MacroMeaning;
use crate::meaning::MeaningFlags;
use crate::token::{Catcode, Token};
use crate::world::ContentHash;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub const MEMO_VALUE_SCHEMA_VERSION: u32 = 1;
const ENVELOPE_MAGIC: [u8; 8] = *b"UMBRMEM\0";

/// Semantic result family carried by a detached memo envelope.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub enum MemoValueKind {
    Tokens = 1,
    Glue = 2,
    MacroMeaning = 3,
    Nodes = 4,
    Box = 5,
    Font = 6,
    InputTransition = 7,
    PageTransition = 8,
    Diagnostics = 9,
    VirtualEffects = 10,
    PureKernelPlan = 11,
    Artifact = 12,
}

/// Decode and import budgets applied before allocating live state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemoValueLimits {
    pub max_payload_bytes: usize,
    pub max_tokens: usize,
    pub max_string_bytes: usize,
}

impl Default for MemoValueLimits {
    fn default() -> Self {
        Self {
            max_payload_bytes: 64 * 1024 * 1024,
            max_tokens: 4 * 1024 * 1024,
            max_string_bytes: 16 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MemoValueError {
    Oversized {
        actual: usize,
        limit: usize,
    },
    Codec(String),
    BadMagic,
    StaleSchema {
        found: u32,
    },
    Kind {
        expected: MemoValueKind,
        found: MemoValueKind,
    },
    Integrity,
    Invalid(&'static str),
}

/// Opaque handle-free memo result with a strong canonical integrity identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DetachedMemoValue {
    kind: MemoValueKind,
    payload: Arc<[u8]>,
    integrity: ContentHash,
}

#[derive(Deserialize, Serialize)]
struct WireEnvelope {
    magic: [u8; 8],
    schema: u32,
    kind: MemoValueKind,
    payload: Vec<u8>,
    integrity: [u8; 32],
}

impl DetachedMemoValue {
    fn new(kind: MemoValueKind, payload: Vec<u8>) -> Self {
        let integrity = memo_integrity(kind, &payload);
        Self {
            kind,
            payload: payload.into(),
            integrity,
        }
    }

    #[must_use]
    pub const fn kind(&self) -> MemoValueKind {
        self.kind
    }

    #[must_use]
    pub const fn integrity(&self) -> ContentHash {
        self.integrity
    }

    #[must_use]
    pub fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>() + self.payload.len()
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, MemoValueError> {
        bincode::serialize(&WireEnvelope {
            magic: ENVELOPE_MAGIC,
            schema: MEMO_VALUE_SCHEMA_VERSION,
            kind: self.kind,
            payload: self.payload.to_vec(),
            integrity: self.integrity.bytes(),
        })
        .map_err(|error| MemoValueError::Codec(error.to_string()))
    }

    pub fn from_bytes(bytes: &[u8], limits: MemoValueLimits) -> Result<Self, MemoValueError> {
        if bytes.len() > limits.max_payload_bytes.saturating_add(128) {
            return Err(MemoValueError::Oversized {
                actual: bytes.len(),
                limit: limits.max_payload_bytes.saturating_add(128),
            });
        }
        let wire: WireEnvelope = bincode::deserialize(bytes)
            .map_err(|error| MemoValueError::Codec(error.to_string()))?;
        if wire.magic != ENVELOPE_MAGIC {
            return Err(MemoValueError::BadMagic);
        }
        if wire.schema != MEMO_VALUE_SCHEMA_VERSION {
            return Err(MemoValueError::StaleSchema { found: wire.schema });
        }
        if wire.payload.len() > limits.max_payload_bytes {
            return Err(MemoValueError::Oversized {
                actual: wire.payload.len(),
                limit: limits.max_payload_bytes,
            });
        }
        let integrity = ContentHash::new(wire.integrity);
        if integrity != memo_integrity(wire.kind, &wire.payload) {
            return Err(MemoValueError::Integrity);
        }
        Ok(Self {
            kind: wire.kind,
            payload: wire.payload.into(),
            integrity,
        })
    }

    fn decode<T: for<'de> Deserialize<'de>>(
        &self,
        expected: MemoValueKind,
    ) -> Result<T, MemoValueError> {
        if self.kind != expected {
            return Err(MemoValueError::Kind {
                expected,
                found: self.kind,
            });
        }
        bincode::deserialize(&self.payload)
            .map_err(|error| MemoValueError::Codec(error.to_string()))
    }
}

fn memo_integrity(kind: MemoValueKind, payload: &[u8]) -> ContentHash {
    let mut framed = Vec::with_capacity(5 + payload.len());
    framed.extend_from_slice(&MEMO_VALUE_SCHEMA_VERSION.to_le_bytes());
    framed.push(kind as u8);
    framed.extend_from_slice(payload);
    ContentHash::from_bytes(&framed)
}

#[derive(Clone, Debug, Deserialize, Serialize)]
enum DetachedToken {
    Char { ch: char, cat: u8 },
    Cs { active: bool, name: String },
    Param(u8),
    Frozen(u8),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct DetachedGlue {
    width: i32,
    stretch: i32,
    stretch_order: u8,
    shrink: i32,
    shrink_order: u8,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct DetachedMacro {
    flags: u8,
    parameter_text: Vec<DetachedToken>,
    replacement_text: Vec<DetachedToken>,
}

impl Universe {
    pub fn detach_token_list(&self, id: TokenListId) -> Result<DetachedMemoValue, MemoValueError> {
        let tokens = self
            .tokens(id)
            .iter()
            .copied()
            .map(|token| detach_token(self, token))
            .collect::<Vec<_>>();
        let payload = bincode::serialize(&tokens)
            .map_err(|error| MemoValueError::Codec(error.to_string()))?;
        Ok(DetachedMemoValue::new(MemoValueKind::Tokens, payload))
    }

    pub fn import_memo_token_list(
        &mut self,
        value: &DetachedMemoValue,
        limits: MemoValueLimits,
    ) -> Result<TokenListId, MemoValueError> {
        let detached: Vec<DetachedToken> = value.decode(MemoValueKind::Tokens)?;
        validate_tokens(&detached, limits)?;
        let mut tokens = Vec::with_capacity(detached.len());
        for token in detached {
            tokens.push(import_token(self, token)?);
        }
        Ok(self.intern_token_list(&tokens))
    }

    pub fn detach_glue(&self, id: GlueId) -> Result<DetachedMemoValue, MemoValueError> {
        let spec = self.glue(id);
        let payload = bincode::serialize(&DetachedGlue {
            width: spec.width.raw(),
            stretch: spec.stretch.raw(),
            stretch_order: spec.stretch_order as u8,
            shrink: spec.shrink.raw(),
            shrink_order: spec.shrink_order as u8,
        })
        .map_err(|error| MemoValueError::Codec(error.to_string()))?;
        Ok(DetachedMemoValue::new(MemoValueKind::Glue, payload))
    }

    pub fn import_memo_glue(
        &mut self,
        value: &DetachedMemoValue,
    ) -> Result<GlueId, MemoValueError> {
        let glue: DetachedGlue = value.decode(MemoValueKind::Glue)?;
        Ok(self.intern_glue(GlueSpec {
            width: crate::scaled::Scaled::from_raw(glue.width),
            stretch: crate::scaled::Scaled::from_raw(glue.stretch),
            stretch_order: decode_order(glue.stretch_order)?,
            shrink: crate::scaled::Scaled::from_raw(glue.shrink),
            shrink_order: decode_order(glue.shrink_order)?,
        }))
    }

    pub fn detach_macro_meaning(
        &self,
        id: MacroDefinitionId,
    ) -> Result<DetachedMemoValue, MemoValueError> {
        let meaning = self.macro_definition(id);
        let parameter_text = self
            .tokens(meaning.parameter_text())
            .iter()
            .copied()
            .map(|token| detach_token(self, token))
            .collect();
        let replacement_text = self
            .tokens(meaning.replacement_text())
            .iter()
            .copied()
            .map(|token| detach_token(self, token))
            .collect();
        let payload = bincode::serialize(&DetachedMacro {
            flags: meaning.flags().bits(),
            parameter_text,
            replacement_text,
        })
        .map_err(|error| MemoValueError::Codec(error.to_string()))?;
        Ok(DetachedMemoValue::new(MemoValueKind::MacroMeaning, payload))
    }

    pub fn import_memo_macro_meaning(
        &mut self,
        value: &DetachedMemoValue,
        limits: MemoValueLimits,
    ) -> Result<MacroDefinitionId, MemoValueError> {
        let detached: DetachedMacro = value.decode(MemoValueKind::MacroMeaning)?;
        validate_tokens(&detached.parameter_text, limits)?;
        validate_tokens(&detached.replacement_text, limits)?;
        let parameters = detached
            .parameter_text
            .into_iter()
            .map(|token| import_token(self, token))
            .collect::<Result<Vec<_>, _>>()?;
        let replacement = detached
            .replacement_text
            .into_iter()
            .map(|token| import_token(self, token))
            .collect::<Result<Vec<_>, _>>()?;
        let parameters = self.intern_token_list(&parameters);
        let replacement = self.intern_token_list(&replacement);
        Ok(self.intern_macro(MacroMeaning::new(
            MeaningFlags::from_bits(detached.flags),
            parameters,
            replacement,
        )))
    }
}

fn detach_token(universe: &Universe, token: Token) -> DetachedToken {
    match token {
        Token::Char { ch, cat } => DetachedToken::Char { ch, cat: cat as u8 },
        Token::Cs(symbol) => DetachedToken::Cs {
            active: universe.control_sequence_kind(symbol) == ControlSequenceKind::ActiveCharacter,
            name: universe.resolve(symbol).to_owned(),
        },
        Token::Param(slot) => DetachedToken::Param(slot),
        Token::Frozen(token) => DetachedToken::Frozen(if Token::Frozen(token).is_frozen_endv() {
            1
        } else {
            0
        }),
    }
}

fn validate_tokens(
    tokens: &[DetachedToken],
    limits: MemoValueLimits,
) -> Result<(), MemoValueError> {
    if tokens.len() > limits.max_tokens {
        return Err(MemoValueError::Oversized {
            actual: tokens.len(),
            limit: limits.max_tokens,
        });
    }
    let string_bytes = tokens.iter().try_fold(0usize, |total, token| {
        total.checked_add(match token {
            DetachedToken::Cs { name, .. } => name.len(),
            _ => 0,
        })
    });
    if string_bytes.is_none_or(|bytes| bytes > limits.max_string_bytes) {
        return Err(MemoValueError::Oversized {
            actual: string_bytes.unwrap_or(usize::MAX),
            limit: limits.max_string_bytes,
        });
    }
    Ok(())
}

fn import_token(universe: &mut Universe, token: DetachedToken) -> Result<Token, MemoValueError> {
    Ok(match token {
        DetachedToken::Char { ch, cat } => Token::Char {
            ch,
            cat: decode_catcode(cat)?,
        },
        DetachedToken::Cs { active, name } => {
            let symbol = if active {
                let mut chars = name.chars();
                let ch = chars
                    .next()
                    .ok_or(MemoValueError::Invalid("empty active character"))?;
                if chars.next().is_some() {
                    return Err(MemoValueError::Invalid(
                        "active character name is not one scalar",
                    ));
                }
                universe.intern_active_character(ch)
            } else {
                universe.intern(&name)
            };
            Token::Cs(symbol.symbol())
        }
        DetachedToken::Param(slot @ 1..=9) => Token::param(slot),
        DetachedToken::Param(_) => return Err(MemoValueError::Invalid("invalid parameter slot")),
        DetachedToken::Frozen(0) => Token::frozen_end_template(),
        DetachedToken::Frozen(1) => Token::frozen_endv(),
        DetachedToken::Frozen(_) => return Err(MemoValueError::Invalid("unknown frozen token")),
    })
}

fn decode_catcode(raw: u8) -> Result<Catcode, MemoValueError> {
    Ok(match raw {
        0 => Catcode::Escape,
        1 => Catcode::BeginGroup,
        2 => Catcode::EndGroup,
        3 => Catcode::MathShift,
        4 => Catcode::AlignmentTab,
        5 => Catcode::EndLine,
        6 => Catcode::Parameter,
        7 => Catcode::Superscript,
        8 => Catcode::Subscript,
        9 => Catcode::Ignored,
        10 => Catcode::Space,
        11 => Catcode::Letter,
        12 => Catcode::Other,
        13 => Catcode::Active,
        14 => Catcode::Comment,
        15 => Catcode::Invalid,
        _ => return Err(MemoValueError::Invalid("unknown catcode")),
    })
}

fn decode_order(raw: u8) -> Result<Order, MemoValueError> {
    Ok(match raw {
        0 => Order::Normal,
        1 => Order::Fil,
        2 => Order::Fill,
        3 => Order::Filll,
        _ => return Err(MemoValueError::Invalid("unknown glue order")),
    })
}

#[cfg(test)]
mod tests;
