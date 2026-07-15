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

/// Region-specific ending input state. The payload is a versioned semantic
/// transition understood by the region that produced it; consumed inputs are
/// pinned by content identity rather than `InputRecordId`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DetachedInputTransition {
    pub transition_schema: u32,
    pub consumed_inputs: Vec<[u8; 32]>,
    pub semantic_payload: Vec<u8>,
}

/// Handle-free page-builder transition consumed by the page replay layer.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DetachedPageTransition {
    pub transition_schema: u32,
    pub semantic_payload: Vec<u8>,
}

/// One deterministic diagnostic with content-relative provenance.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DetachedDiagnostic {
    pub code: String,
    pub message: String,
    /// Character/token ordinal in the current memo input, never an `OriginId`.
    pub input_ordinal: Option<u32>,
}

/// A virtual effect record. Decoding this value does not apply or materialize it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DetachedVirtualEffect {
    pub operation: String,
    pub stream: Option<u8>,
    pub payload: Vec<u8>,
}

/// A pure-kernel result whose inner schema is owned by that kernel.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DetachedPureKernelPlan {
    pub kernel: String,
    pub plan_schema: u32,
    pub payload: Vec<u8>,
}

/// Already-detached artifact bytes with their own codec schema.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DetachedArtifact {
    pub artifact_schema: u32,
    pub payload: Vec<u8>,
}

/// Decode and import budgets applied before allocating live state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemoValueLimits {
    pub max_payload_bytes: usize,
    pub max_tokens: usize,
    pub max_nodes: usize,
    pub max_string_bytes: usize,
}

impl Default for MemoValueLimits {
    fn default() -> Self {
        Self {
            max_payload_bytes: 64 * 1024 * 1024,
            max_tokens: 4 * 1024 * 1024,
            max_nodes: 4 * 1024 * 1024,
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

    pub(crate) fn from_payload(kind: MemoValueKind, payload: Vec<u8>) -> Self {
        Self::new(kind, payload)
    }

    pub(crate) fn payload(&self, expected: MemoValueKind) -> Result<&[u8], MemoValueError> {
        if self.kind != expected {
            return Err(MemoValueError::Kind {
                expected,
                found: self.kind,
            });
        }
        Ok(&self.payload)
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

    fn encode<T: Serialize>(kind: MemoValueKind, value: &T) -> Result<Self, MemoValueError> {
        let payload =
            bincode::serialize(value).map_err(|error| MemoValueError::Codec(error.to_string()))?;
        Ok(Self::new(kind, payload))
    }

    pub fn from_input_transition(value: &DetachedInputTransition) -> Result<Self, MemoValueError> {
        Self::encode(MemoValueKind::InputTransition, value)
    }

    pub fn input_transition(
        &self,
        limits: MemoValueLimits,
    ) -> Result<DetachedInputTransition, MemoValueError> {
        let value: DetachedInputTransition = self.decode(MemoValueKind::InputTransition)?;
        validate_payload(value.semantic_payload.len(), limits)?;
        if value.consumed_inputs.len() > limits.max_tokens {
            return Err(MemoValueError::Oversized {
                actual: value.consumed_inputs.len(),
                limit: limits.max_tokens,
            });
        }
        Ok(value)
    }

    pub fn from_page_transition(value: &DetachedPageTransition) -> Result<Self, MemoValueError> {
        Self::encode(MemoValueKind::PageTransition, value)
    }

    pub fn page_transition(
        &self,
        limits: MemoValueLimits,
    ) -> Result<DetachedPageTransition, MemoValueError> {
        let value: DetachedPageTransition = self.decode(MemoValueKind::PageTransition)?;
        validate_payload(value.semantic_payload.len(), limits)?;
        Ok(value)
    }

    pub fn from_diagnostics(value: &[DetachedDiagnostic]) -> Result<Self, MemoValueError> {
        Self::encode(MemoValueKind::Diagnostics, &value)
    }

    pub fn diagnostics(
        &self,
        limits: MemoValueLimits,
    ) -> Result<Vec<DetachedDiagnostic>, MemoValueError> {
        let value: Vec<DetachedDiagnostic> = self.decode(MemoValueKind::Diagnostics)?;
        validate_entry_strings(
            value.len(),
            value
                .iter()
                .map(|item| item.code.len() + item.message.len()),
            limits,
        )?;
        Ok(value)
    }

    pub fn from_virtual_effects(value: &[DetachedVirtualEffect]) -> Result<Self, MemoValueError> {
        Self::encode(MemoValueKind::VirtualEffects, &value)
    }

    pub fn virtual_effects(
        &self,
        limits: MemoValueLimits,
    ) -> Result<Vec<DetachedVirtualEffect>, MemoValueError> {
        let value: Vec<DetachedVirtualEffect> = self.decode(MemoValueKind::VirtualEffects)?;
        validate_entry_strings(
            value.len(),
            value
                .iter()
                .map(|item| item.operation.len() + item.payload.len()),
            limits,
        )?;
        Ok(value)
    }

    pub fn from_pure_kernel_plan(value: &DetachedPureKernelPlan) -> Result<Self, MemoValueError> {
        Self::encode(MemoValueKind::PureKernelPlan, value)
    }

    pub fn pure_kernel_plan(
        &self,
        limits: MemoValueLimits,
    ) -> Result<DetachedPureKernelPlan, MemoValueError> {
        let value: DetachedPureKernelPlan = self.decode(MemoValueKind::PureKernelPlan)?;
        validate_payload(value.kernel.len() + value.payload.len(), limits)?;
        Ok(value)
    }

    pub fn from_artifact(value: &DetachedArtifact) -> Result<Self, MemoValueError> {
        Self::encode(MemoValueKind::Artifact, value)
    }

    pub fn artifact(&self, limits: MemoValueLimits) -> Result<DetachedArtifact, MemoValueError> {
        let value: DetachedArtifact = self.decode(MemoValueKind::Artifact)?;
        validate_payload(value.payload.len(), limits)?;
        Ok(value)
    }
}

fn validate_payload(actual: usize, limits: MemoValueLimits) -> Result<(), MemoValueError> {
    if actual > limits.max_payload_bytes {
        return Err(MemoValueError::Oversized {
            actual,
            limit: limits.max_payload_bytes,
        });
    }
    Ok(())
}

fn validate_entry_strings(
    count: usize,
    mut lengths: impl Iterator<Item = usize>,
    limits: MemoValueLimits,
) -> Result<(), MemoValueError> {
    if count > limits.max_tokens {
        return Err(MemoValueError::Oversized {
            actual: count,
            limit: limits.max_tokens,
        });
    }
    let bytes = lengths.try_fold(0usize, usize::checked_add);
    if bytes.is_none_or(|bytes| bytes > limits.max_payload_bytes) {
        return Err(MemoValueError::Oversized {
            actual: bytes.unwrap_or(usize::MAX),
            limit: limits.max_payload_bytes,
        });
    }
    Ok(())
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
    for token in tokens {
        match token {
            DetachedToken::Char { cat, .. } => {
                decode_catcode(*cat)?;
            }
            DetachedToken::Cs { active: true, name } => {
                let mut chars = name.chars();
                if chars.next().is_none() {
                    return Err(MemoValueError::Invalid("empty active character"));
                }
                if chars.next().is_some() {
                    return Err(MemoValueError::Invalid(
                        "active character name is not one scalar",
                    ));
                }
            }
            DetachedToken::Param(1..=9)
            | DetachedToken::Frozen(0..=1)
            | DetachedToken::Cs { active: false, .. } => {}
            DetachedToken::Param(_) => {
                return Err(MemoValueError::Invalid("invalid parameter slot"));
            }
            DetachedToken::Frozen(_) => {
                return Err(MemoValueError::Invalid("unknown frozen token"));
            }
        }
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
