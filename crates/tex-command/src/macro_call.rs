//! Private canonical scalar macro-call state machine.
#![allow(dead_code)] // consumed by the ordered macro-replay implementation issue

use std::sync::Arc;

use tex_state::ids::MacroDefinitionId;
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use crate::input::SharedTokenBuffer;

/// Persistent ownership of live macro-argument activations.
///
/// This is the sole owner of the activation chain. Macro-body input behavior
/// carries a typed activation identity, while parameter payloads retain shared
/// ownership of the one contiguous argument allocation.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) struct ParameterState {
    pub(crate) activations: Vec<MacroActivation>,
    pub(crate) next_activation_identity: u64,
}

/// Typed identity of one live macro activation.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct MacroActivationId(pub(crate) u64);

/// One live macro call and the materialized arguments it owns.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct MacroActivation {
    pub(crate) identity: MacroActivationId,
    pub(crate) definition: MacroDefinitionId,
    pub(crate) arguments: MacroArguments,
    pub(crate) invocation: OriginId,
}

/// One contiguous macro-argument allocation and its at-most-nine ranges.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) struct MacroArguments {
    pub(crate) buffer: SharedTokenBuffer,
    pub(crate) ranges: [Option<MacroArgumentRange>; 9],
}

/// Incremental construction of one canonical macro activation.
///
/// The scalar matcher completes arguments in definition order. Each completed
/// argument is appended once to this one buffer; its slot records only a
/// half-open range, so parameter replay never duplicates argument tokens.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) struct MacroArgumentBuilder {
    tokens: Vec<TracedTokenWord>,
    ranges: [Option<MacroArgumentRange>; 9],
    next_slot: u8,
}

/// A malformed attempt to finish one macro argument.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum MacroArgumentBuildError {
    InvalidSlot(u8),
    OutOfOrderSlot { expected: u8, actual: u8 },
}

/// Compact interpretation of a parameter-related replacement token.
///
/// `Token::Param` is the canonical compact out-parameter representation.
/// A literal parameter character reaches a replacement list only through
/// TeX's `##` escape, and remains an ordinary character during replay.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum MacroParameterEscape {
    OutParameter(u8),
    EscapedParameter,
}

impl MacroParameterEscape {
    pub(crate) const fn classify(token: Token) -> Option<Self> {
        match token {
            Token::Param(slot) => Some(Self::OutParameter(slot)),
            Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
            } => Some(Self::EscapedParameter),
            _ => None,
        }
    }
}

/// A half-open range within a macro activation's shared argument buffer.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct MacroArgumentRange {
    start: usize,
    end: usize,
}

impl MacroArgumentRange {
    pub(crate) const fn new(start: usize, end: usize) -> Option<Self> {
        if start <= end {
            Some(Self { start, end })
        } else {
            None
        }
    }

    pub(crate) const fn start(self) -> usize {
        self.start
    }

    pub(crate) const fn end(self) -> usize {
        self.end
    }
}

impl MacroArgumentBuilder {
    /// Completes the next argument in canonical definition order.
    pub(crate) fn complete(
        &mut self,
        slot: u8,
        argument: impl IntoIterator<Item = TracedTokenWord>,
    ) -> Result<(), MacroArgumentBuildError> {
        if !(1..=9).contains(&slot) {
            return Err(MacroArgumentBuildError::InvalidSlot(slot));
        }
        let expected = self.next_slot + 1;
        if slot != expected {
            return Err(MacroArgumentBuildError::OutOfOrderSlot {
                expected,
                actual: slot,
            });
        }
        let start = self.tokens.len();
        self.tokens.extend(argument);
        let end = self.tokens.len();
        self.ranges[usize::from(slot - 1)] = MacroArgumentRange::new(start, end);
        self.next_slot = slot;
        Ok(())
    }

    /// Freezes the single shared argument allocation for one activation.
    #[must_use]
    pub(crate) fn finish(self) -> MacroArguments {
        MacroArguments {
            buffer: SharedTokenBuffer::new(Arc::from(self.tokens)),
            ranges: self.ranges,
        }
    }
}

impl ParameterState {
    /// Installs the sole owner of a macro activation before its body level is
    /// pushed. The caller immediately associates the returned identity with
    /// `TokenBehavior::MacroBody`, making retirement an atomic ownership pair.
    pub(crate) fn push_activation(
        &mut self,
        definition: MacroDefinitionId,
        arguments: MacroArguments,
        invocation: OriginId,
    ) -> MacroActivationId {
        let identity = MacroActivationId(self.next_activation_identity);
        self.next_activation_identity = self.next_activation_identity.wrapping_add(1);
        self.activations.push(MacroActivation {
            identity,
            definition,
            arguments,
            invocation,
        });
        identity
    }

    pub(crate) fn parent_invocation(&self) -> OriginId {
        self.activations
            .last()
            .map(|activation| activation.invocation)
            .unwrap_or(OriginId::UNKNOWN)
    }
}

#[cfg(test)]
mod tests;
