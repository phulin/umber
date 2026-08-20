//! Handle-free, schema-versioned values exchanged with cold memo caches.
//!
//! Live values enter this boundary only through generation-branded coordinates.
//! Detachment resolves them under one admitted borrow and emits logical values.
//! Materialization decodes and validates the complete payload before publishing
//! one destination-local arena batch. No live coordinate, owner, cursor, or
//! borrow enters the envelope.

use crate::definition_arena::DefinitionId;
use crate::durable_arena::{GlueId, TokenListId};
use crate::glue::{GlueSpec, Order};
use crate::interner::ControlSequenceKind;
use crate::meaning::{MeaningFlags, MeaningWord};
use crate::token::{Catcode, FrozenToken, Token, TokenWord};
use crate::universe::{DefinitionPromotion, PromotionError, TokenListPromotion, Universe};
use crate::world::ContentHash;
use serde::{Deserialize, Serialize};

pub const MEMO_VALUE_SCHEMA_VERSION: u32 = 2;
const ENVELOPE_MAGIC: [u8; 8] = *b"UMBRMEM\0";

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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DetachedInputTransition {
    pub transition_schema: u32,
    pub consumed_inputs: Vec<[u8; 32]>,
    pub semantic_payload: Vec<u8>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DetachedPageTransition {
    pub transition_schema: u32,
    pub semantic_payload: Vec<u8>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DetachedDiagnostic {
    pub code: String,
    pub message: String,
    /// Ordinal in the memo input, never a live provenance coordinate.
    pub input_ordinal: Option<u32>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DetachedVirtualEffect {
    pub operation: String,
    pub stream: Option<u8>,
    pub payload: Vec<u8>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DetachedPureKernelPlan {
    pub kernel: String,
    pub plan_schema: u32,
    pub payload: Vec<u8>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DetachedArtifact {
    pub artifact_schema: u32,
    pub payload: Vec<u8>,
}

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
    LiveState,
    Publication(PromotionError),
}

impl From<PromotionError> for MemoValueError {
    fn from(error: PromotionError) -> Self {
        Self::Publication(error)
    }
}

/// Opaque detached memo result. Its owned payload is the sole value authority.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct DetachedMemoValue {
    kind: MemoValueKind,
    payload: Vec<u8>,
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
            payload,
            integrity,
        }
    }

    pub(crate) fn from_payload(kind: MemoValueKind, payload: Vec<u8>) -> Self {
        Self::new(kind, payload)
    }

    pub(crate) fn payload(&self, expected: MemoValueKind) -> Result<&[u8], MemoValueError> {
        self.require_kind(expected)?;
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
        core::mem::size_of::<Self>() + self.payload.len()
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, MemoValueError> {
        bincode::serialize(&WireEnvelope {
            magic: ENVELOPE_MAGIC,
            schema: MEMO_VALUE_SCHEMA_VERSION,
            kind: self.kind,
            payload: self.payload.clone(),
            integrity: self.integrity.bytes(),
        })
        .map_err(codec_error)
    }

    pub fn from_bytes(bytes: &[u8], limits: MemoValueLimits) -> Result<Self, MemoValueError> {
        check_limit(bytes.len(), limits.max_payload_bytes.saturating_add(128))?;
        let wire: WireEnvelope = bincode::deserialize(bytes).map_err(codec_error)?;
        if wire.magic != ENVELOPE_MAGIC {
            return Err(MemoValueError::BadMagic);
        }
        if wire.schema != MEMO_VALUE_SCHEMA_VERSION {
            return Err(MemoValueError::StaleSchema { found: wire.schema });
        }
        check_limit(wire.payload.len(), limits.max_payload_bytes)?;
        let integrity = ContentHash::new(wire.integrity);
        if integrity != memo_integrity(wire.kind, &wire.payload) {
            return Err(MemoValueError::Integrity);
        }
        Ok(Self {
            kind: wire.kind,
            payload: wire.payload,
            integrity,
        })
    }

    fn require_kind(&self, expected: MemoValueKind) -> Result<(), MemoValueError> {
        if self.kind == expected {
            Ok(())
        } else {
            Err(MemoValueError::Kind {
                expected,
                found: self.kind,
            })
        }
    }

    fn decode<T: for<'de> Deserialize<'de>>(
        &self,
        expected: MemoValueKind,
    ) -> Result<T, MemoValueError> {
        self.require_kind(expected)?;
        bincode::deserialize(&self.payload).map_err(codec_error)
    }

    fn encode<T: Serialize + ?Sized>(
        kind: MemoValueKind,
        value: &T,
    ) -> Result<Self, MemoValueError> {
        Ok(Self::new(
            kind,
            bincode::serialize(value).map_err(codec_error)?,
        ))
    }

    pub fn from_input_transition(value: &DetachedInputTransition) -> Result<Self, MemoValueError> {
        Self::encode(MemoValueKind::InputTransition, value)
    }

    pub fn input_transition(
        &self,
        limits: MemoValueLimits,
    ) -> Result<DetachedInputTransition, MemoValueError> {
        let value: DetachedInputTransition = self.decode(MemoValueKind::InputTransition)?;
        check_limit(value.semantic_payload.len(), limits.max_payload_bytes)?;
        check_limit(value.consumed_inputs.len(), limits.max_tokens)?;
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
        check_limit(value.semantic_payload.len(), limits.max_payload_bytes)?;
        Ok(value)
    }

    pub fn from_diagnostics(value: &[DetachedDiagnostic]) -> Result<Self, MemoValueError> {
        Self::encode(MemoValueKind::Diagnostics, value)
    }

    pub fn diagnostics(
        &self,
        limits: MemoValueLimits,
    ) -> Result<Vec<DetachedDiagnostic>, MemoValueError> {
        let value: Vec<DetachedDiagnostic> = self.decode(MemoValueKind::Diagnostics)?;
        validate_entries(
            value.len(),
            value
                .iter()
                .map(|v| v.code.len().saturating_add(v.message.len())),
            limits,
        )?;
        Ok(value)
    }

    pub fn from_virtual_effects(value: &[DetachedVirtualEffect]) -> Result<Self, MemoValueError> {
        Self::encode(MemoValueKind::VirtualEffects, value)
    }

    pub fn virtual_effects(
        &self,
        limits: MemoValueLimits,
    ) -> Result<Vec<DetachedVirtualEffect>, MemoValueError> {
        let value: Vec<DetachedVirtualEffect> = self.decode(MemoValueKind::VirtualEffects)?;
        validate_entries(
            value.len(),
            value
                .iter()
                .map(|v| v.operation.len().saturating_add(v.payload.len())),
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
        check_limit(
            value.kernel.len().saturating_add(value.payload.len()),
            limits.max_payload_bytes,
        )?;
        Ok(value)
    }

    pub fn from_artifact(value: &DetachedArtifact) -> Result<Self, MemoValueError> {
        Self::encode(MemoValueKind::Artifact, value)
    }

    pub fn artifact(&self, limits: MemoValueLimits) -> Result<DetachedArtifact, MemoValueError> {
        let value: DetachedArtifact = self.decode(MemoValueKind::Artifact)?;
        check_limit(value.payload.len(), limits.max_payload_bytes)?;
        Ok(value)
    }

    /// Fully decodes and validates a token-list DTO without touching live state.
    pub fn stage_token_list(
        &self,
        limits: MemoValueLimits,
    ) -> Result<StagedMemoTokenList, MemoValueError> {
        let tokens: Vec<DetachedToken> = self.decode(MemoValueKind::Tokens)?;
        validate_tokens(&tokens, limits)?;
        Ok(StagedMemoTokenList { tokens })
    }

    pub fn stage_glue(&self) -> Result<StagedMemoGlue, MemoValueError> {
        let glue: DetachedGlue = self.decode(MemoValueKind::Glue)?;
        Ok(StagedMemoGlue {
            value: glue.into_glue()?,
        })
    }

    pub fn stage_macro(&self, limits: MemoValueLimits) -> Result<StagedMemoMacro, MemoValueError> {
        let value: DetachedMacro = self.decode(MemoValueKind::MacroMeaning)?;
        validate_tokens(&value.parameter_text, limits)?;
        validate_tokens(&value.replacement_text, limits)?;
        Ok(StagedMemoMacro { value })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
enum DetachedToken {
    Char { ch: char, cat: u8 },
    Cs { active: bool, name: String },
    Param(u8),
    Frozen(DetachedFrozenToken),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
enum DetachedFrozenToken {
    EndTemplate,
    EndV,
    Relax,
    UndefinedControlSequence,
    ExpandedTextBoundary,
    Primitive(String),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct DetachedGlue {
    width: i32,
    stretch: i32,
    stretch_order: u8,
    shrink: i32,
    shrink_order: u8,
}

impl DetachedGlue {
    fn from_glue(value: GlueSpec) -> Self {
        Self {
            width: value.width.raw(),
            stretch: value.stretch.raw(),
            stretch_order: value.stretch_order as u8,
            shrink: value.shrink.raw(),
            shrink_order: value.shrink_order as u8,
        }
    }

    fn into_glue(self) -> Result<GlueSpec, MemoValueError> {
        Ok(GlueSpec {
            width: crate::scaled::Scaled::from_raw(self.width),
            stretch: crate::scaled::Scaled::from_raw(self.stretch),
            stretch_order: decode_order(self.stretch_order)?,
            shrink: crate::scaled::Scaled::from_raw(self.shrink),
            shrink_order: decode_order(self.shrink_order)?,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct DetachedMacro {
    flags: u8,
    parameter_text: Vec<DetachedToken>,
    replacement_text: Vec<DetachedToken>,
}

#[derive(Clone, Debug)]
pub struct StagedMemoTokenList {
    tokens: Vec<DetachedToken>,
}
#[derive(Clone, Debug)]
pub struct StagedMemoGlue {
    value: GlueSpec,
}
#[derive(Clone, Debug)]
pub struct StagedMemoMacro {
    value: DetachedMacro,
}

impl<G> Universe<G> {
    /// Explicit cold-demand detachment of one generation-local token list.
    pub fn detach_token_list(
        &self,
        id: TokenListId<G>,
    ) -> Result<DetachedMemoValue, MemoValueError> {
        let admitted = self.admitted().map_err(|_| MemoValueError::LiveState)?;
        let tokens = admitted
            .token_list(id)
            .iter()
            .copied()
            .map(|word| detach_token(self, word.semantic_token()))
            .collect::<Result<Vec<_>, _>>()?;
        DetachedMemoValue::encode(MemoValueKind::Tokens, &tokens)
    }

    pub fn publish_memo_token_list(
        &mut self,
        staged: StagedMemoTokenList,
    ) -> Result<TokenListId<G>, MemoValueError> {
        let words = staged
            .tokens
            .into_iter()
            .map(|token| import_token(self, token))
            .collect::<Result<Vec<_>, _>>()?;
        let promotions = [TokenListPromotion { words: &words }];
        Ok(self.promote_values(&[], &promotions, &[], &[])?.token_lists[0])
    }

    pub fn import_memo_token_list(
        &mut self,
        value: &DetachedMemoValue,
        limits: MemoValueLimits,
    ) -> Result<TokenListId<G>, MemoValueError> {
        self.publish_memo_token_list(value.stage_token_list(limits)?)
    }

    pub fn detach_glue(&self, id: GlueId<G>) -> Result<DetachedMemoValue, MemoValueError> {
        DetachedMemoValue::encode(
            MemoValueKind::Glue,
            &DetachedGlue::from_glue(self.glue_value(id)),
        )
    }

    pub fn publish_memo_glue(
        &mut self,
        staged: StagedMemoGlue,
    ) -> Result<GlueId<G>, MemoValueError> {
        Ok(self.promote_values(&[], &[], &[staged.value], &[])?.glue[0])
    }

    pub fn import_memo_glue(
        &mut self,
        value: &DetachedMemoValue,
    ) -> Result<GlueId<G>, MemoValueError> {
        self.publish_memo_glue(value.stage_glue()?)
    }

    pub fn detach_macro_meaning(
        &self,
        flags: MeaningFlags,
        id: DefinitionId<G>,
    ) -> Result<DetachedMemoValue, MemoValueError> {
        let admitted = self.admitted().map_err(|_| MemoValueError::LiveState)?;
        let definition = admitted.definition(id);
        let parameter_text = definition
            .parameter_text()
            .iter()
            .copied()
            .map(|word| detach_token(self, word.semantic_token()))
            .collect::<Result<Vec<_>, _>>()?;
        let replacement_text = definition
            .replacement_text()
            .iter()
            .copied()
            .map(|word| detach_token(self, word.semantic_token()))
            .collect::<Result<Vec<_>, _>>()?;
        DetachedMemoValue::encode(
            MemoValueKind::MacroMeaning,
            &DetachedMacro {
                flags: flags.bits(),
                parameter_text,
                replacement_text,
            },
        )
    }

    pub fn publish_memo_macro(
        &mut self,
        staged: StagedMemoMacro,
    ) -> Result<MeaningWord<G>, MemoValueError> {
        let flags = MeaningFlags::from_bits(staged.value.flags);
        let parameter_text = staged
            .value
            .parameter_text
            .into_iter()
            .map(|token| import_token(self, token))
            .collect::<Result<Vec<_>, _>>()?;
        let replacement_text = staged
            .value
            .replacement_text
            .into_iter()
            .map(|token| import_token(self, token))
            .collect::<Result<Vec<_>, _>>()?;
        let definitions = [DefinitionPromotion {
            parameter_text: &parameter_text,
            replacement_text: &replacement_text,
        }];
        let id = self
            .promote_values(&definitions, &[], &[], &[])?
            .definitions[0];
        Ok(MeaningWord::macro_definition(flags, id))
    }

    pub fn import_memo_macro_meaning(
        &mut self,
        value: &DetachedMemoValue,
        limits: MemoValueLimits,
    ) -> Result<MeaningWord<G>, MemoValueError> {
        self.publish_memo_macro(value.stage_macro(limits)?)
    }
}

fn detach_token<G>(universe: &Universe<G>, token: Token) -> Result<DetachedToken, MemoValueError> {
    Ok(match token {
        Token::Char { ch, cat } => DetachedToken::Char { ch, cat: cat as u8 },
        Token::Cs(symbol) => DetachedToken::Cs {
            active: universe.control_sequence_kind(symbol)
                == Some(ControlSequenceKind::ActiveCharacter),
            name: universe
                .resolve(symbol)
                .ok_or(MemoValueError::LiveState)?
                .to_owned(),
        },
        Token::Param(slot) => DetachedToken::Param(slot),
        Token::Frozen(frozen) => DetachedToken::Frozen(match frozen.raw() {
            0 => DetachedFrozenToken::EndTemplate,
            1 => DetachedFrozenToken::EndV,
            60_000 => DetachedFrozenToken::Relax,
            raw if raw == u16::MAX - 1 => DetachedFrozenToken::UndefinedControlSequence,
            u16::MAX => DetachedFrozenToken::ExpandedTextBoundary,
            _ => DetachedFrozenToken::Primitive(
                universe
                    .frozen_primitive_meaning(Token::Frozen(frozen))
                    .and_then(|meaning| universe.primitive_name(meaning))
                    .ok_or(MemoValueError::Invalid("unknown frozen primitive"))?
                    .to_owned(),
            ),
        }),
    })
}

fn import_token<G>(
    universe: &mut Universe<G>,
    token: DetachedToken,
) -> Result<TokenWord, MemoValueError> {
    let token = match token {
        DetachedToken::Char { ch, cat } => Token::Char {
            ch,
            cat: decode_catcode(cat)?,
        },
        DetachedToken::Cs { active, name } => {
            let id = if active {
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
            }
            .map_err(|_| MemoValueError::LiveState)?;
            Token::Cs(id.symbol())
        }
        DetachedToken::Param(slot @ 1..=9) => Token::param(slot),
        DetachedToken::Param(_) => return Err(MemoValueError::Invalid("invalid parameter slot")),
        DetachedToken::Frozen(kind) => match kind {
            DetachedFrozenToken::EndTemplate => Token::frozen_end_template(),
            DetachedFrozenToken::EndV => Token::frozen_endv(),
            DetachedFrozenToken::Relax => Token::frozen_relax(),
            DetachedFrozenToken::UndefinedControlSequence => Token::undefined_control_sequence(),
            DetachedFrozenToken::ExpandedTextBoundary => {
                Token::Frozen(FrozenToken::from_raw(u16::MAX))
            }
            DetachedFrozenToken::Primitive(name) => universe
                .primitive_token(&name)
                .ok_or(MemoValueError::Invalid("unknown frozen primitive"))?,
        },
    };
    Ok(TokenWord::pack(token))
}

fn validate_tokens(
    tokens: &[DetachedToken],
    limits: MemoValueLimits,
) -> Result<(), MemoValueError> {
    check_limit(tokens.len(), limits.max_tokens)?;
    let bytes = tokens
        .iter()
        .try_fold(0usize, |total, token| {
            let len = match token {
                DetachedToken::Cs { name, .. }
                | DetachedToken::Frozen(DetachedFrozenToken::Primitive(name)) => name.len(),
                _ => 0,
            };
            total.checked_add(len)
        })
        .ok_or(MemoValueError::Oversized {
            actual: usize::MAX,
            limit: limits.max_string_bytes,
        })?;
    check_limit(bytes, limits.max_string_bytes)?;
    for token in tokens {
        match token {
            DetachedToken::Char { cat, .. } => {
                decode_catcode(*cat)?;
            }
            DetachedToken::Cs { active: true, name } if name.chars().count() != 1 => {
                return Err(MemoValueError::Invalid(
                    "active character name is not one scalar",
                ));
            }
            DetachedToken::Param(1..=9) => {}
            DetachedToken::Param(_) => {
                return Err(MemoValueError::Invalid("invalid parameter slot"));
            }
            DetachedToken::Frozen(DetachedFrozenToken::Primitive(name)) if name.is_empty() => {
                return Err(MemoValueError::Invalid("empty frozen primitive name"));
            }
            _ => {}
        }
    }
    Ok(())
}

fn validate_entries(
    count: usize,
    mut lengths: impl Iterator<Item = usize>,
    limits: MemoValueLimits,
) -> Result<(), MemoValueError> {
    check_limit(count, limits.max_tokens)?;
    let bytes = lengths
        .try_fold(0usize, usize::checked_add)
        .ok_or(MemoValueError::Oversized {
            actual: usize::MAX,
            limit: limits.max_payload_bytes,
        })?;
    check_limit(bytes, limits.max_payload_bytes)
}

fn check_limit(actual: usize, limit: usize) -> Result<(), MemoValueError> {
    if actual > limit {
        Err(MemoValueError::Oversized { actual, limit })
    } else {
        Ok(())
    }
}

fn codec_error(error: impl core::fmt::Display) -> MemoValueError {
    MemoValueError::Codec(error.to_string())
}

fn memo_integrity(kind: MemoValueKind, payload: &[u8]) -> ContentHash {
    let mut framed = Vec::with_capacity(5 + payload.len());
    framed.extend_from_slice(&MEMO_VALUE_SCHEMA_VERSION.to_le_bytes());
    framed.push(kind as u8);
    framed.extend_from_slice(payload);
    ContentHash::from_bytes(&framed)
}

fn decode_catcode(raw: u8) -> Result<Catcode, MemoValueError> {
    Catcode::from_raw(raw).ok_or(MemoValueError::Invalid("unknown catcode"))
}

fn decode_order(raw: u8) -> Result<Order, MemoValueError> {
    match raw {
        0 => Ok(Order::Normal),
        1 => Ok(Order::Fil),
        2 => Ok(Order::Fill),
        3 => Ok(Order::Filll),
        _ => Err(MemoValueError::Invalid("unknown glue order")),
    }
}

#[cfg(test)]
mod tests;
