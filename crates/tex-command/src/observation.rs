//! Test and instrumentation-only command semantic observation.
//!
//! These records deliberately belong to `tex-command`, rather than to the
//! fixture/oracle crate.  They are built from values already available at the
//! transition seam and are delivered only after the transition has committed.
//! In particular, an observer is non-fallible and never participates in
//! command state, snapshots, delivery, expansion, or scanner control flow.

#![cfg(any(test, feature = "instrumentation"))]

use tex_state::token::{Catcode, Token, TracedTokenWord};

use crate::{CurrentCommand, DeliveryStamp};

/// An owned, allocation-independent spelling used by command observation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ObservedToken {
    Character { character: char, catcode: Catcode },
    ControlSequence(String),
    Parameter(u8),
    FrozenEndTemplate,
    FrozenEndV,
    FrozenPrimitive(u16),
    FrozenOther,
}

/// Exact source and aggregate delivery provenance for an observed command.
///
/// The input-level identity and cursor slot identify the aggregate input
/// transition; the processor-local sequence distinguishes a later replay of
/// the same slot.  The opaque origin itself is intentionally not exposed:
/// allocation identities are not stable semantic data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommandProvenance {
    pub input_level: u64,
    pub position: u64,
    pub delivery_sequence: u64,
    pub has_origin: bool,
}

impl CommandProvenance {
    pub(crate) fn from_command(command: &CurrentCommand) -> Self {
        let stamp = command.delivery_stamp();
        Self::from_stamp(
            stamp,
            command.origin() != tex_state::token::OriginId::UNKNOWN,
        )
    }

    pub(crate) const fn from_stamp(stamp: DeliveryStamp, has_origin: bool) -> Self {
        Self {
            input_level: stamp.input_level(),
            position: stamp.position(),
            delivery_sequence: stamp.sequence(),
            has_origin,
        }
    }
}

/// The caller-visible delivery boundary which committed a command.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandDeliveryBoundary {
    Raw,
    Expanded,
}

/// A command delivery expressed without engine allocation identities.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandDeliveryRecord {
    pub boundary: CommandDeliveryBoundary,
    pub spelling: ObservedToken,
    pub command: String,
    pub provenance: CommandProvenance,
}

/// Logical input changes observable at the canonical raw-input seam.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputTransition {
    Retire,
    Stop,
    Backup,
    Recovery,
}

/// One input transition with its deterministic aggregate provenance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InputRecord {
    pub transition: InputTransition,
    pub level: u64,
    pub position: u64,
}

/// One backup or canonical recovery insertion.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryRecord {
    pub backup: bool,
    pub tokens: Vec<ObservedToken>,
}

/// A committed entry to or restoration from a live scanner episode.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScannerStatusRecord {
    pub entering: bool,
    pub status: String,
}

/// A completed scalar macro-match milestone.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MacroRecord {
    pub activation: bool,
    pub definition: u64,
    pub argument: Option<u8>,
    pub token_count: u64,
}

/// A committed condition-stack transition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConditionRecord {
    pub transition: &'static str,
    pub condition: u64,
    pub detail: String,
}

/// A completed typed scanner result. Values are rendered only from the
/// scanner's owned semantic result, never from aggregate allocation state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScannerRecord {
    pub kind: &'static str,
    pub value: String,
}

/// One `scan_toks` direct splice or completed immutable collection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TokenListRecord {
    pub transition: &'static str,
    pub token_count: u64,
}

/// A raw-delivery alignment adjustment or template lifecycle transition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AlignmentRecord {
    pub transition: &'static str,
    pub alignment: Option<u64>,
    pub align_state: i32,
}

/// A typed command-relevant assignment seam. Assignment dispatch owns the
/// payload in later slices; retaining this record here keeps the observer
/// union complete without depending on an oracle transport.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MutationRecord {
    pub target: &'static str,
    pub value: String,
}

/// A committed externally-visible command effect or final ordering marker.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EffectRecord {
    pub kind: &'static str,
    pub detail: String,
}

/// One committed command-core observation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommandObservation {
    Command(CommandDeliveryRecord),
    Input(InputRecord),
    Recovery(RecoveryRecord),
    ScannerStatus(ScannerStatusRecord),
    Macro(MacroRecord),
    Condition(ConditionRecord),
    Scanner(ScannerRecord),
    TokenList(TokenListRecord),
    Alignment(AlignmentRecord),
    Mutation(MutationRecord),
    Effect(EffectRecord),
}

/// Test/instrumentation sink for committed command-owned semantic records.
///
/// This interface is intentionally non-fallible. An instrumentation transport
/// must buffer or handle its own failures outside the command operation.
pub trait CommandObserver {
    fn committed(&mut self, observation: CommandObservation);
}

pub(crate) fn observed_token(
    token: TracedTokenWord,
    resolve: impl FnOnce(tex_state::interner::Symbol) -> String,
) -> ObservedToken {
    match token.semantic_token() {
        Token::Char { ch, cat } => ObservedToken::Character {
            character: ch,
            catcode: cat,
        },
        Token::Cs(symbol) => ObservedToken::ControlSequence(resolve(symbol)),
        Token::Param(slot) => ObservedToken::Parameter(slot),
        Token::Frozen(_) if token.semantic_token().is_frozen_end_template() => {
            ObservedToken::FrozenEndTemplate
        }
        Token::Frozen(_) if token.semantic_token().is_frozen_endv() => ObservedToken::FrozenEndV,
        Token::Frozen(frozen) => frozen
            .primitive_index()
            .map_or(ObservedToken::FrozenOther, ObservedToken::FrozenPrimitive),
    }
}
