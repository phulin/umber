//! Canonical input-stack replay and retirement mechanics.
#![allow(dead_code)] // consumed by the ordered raw-delivery implementation issues

use crate::CommandState;
use tex_state::ids::{MacroDefinitionId, OriginListId, TokenListId};
use tex_state::token::OriginId;

use crate::macro_call::{MacroActivationId, MacroArguments};

use super::{
    InputLevel, InputLevelId, ReplayTrace, RetirementBehavior, StoredReplayReason, TokenBehavior,
    TokenCursor, TokenPayload,
};

/// One committed input-lifecycle transition.
///
/// `trace` explains the replay that ended but does not select `action`.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct InputRetirement {
    pub(crate) identity: InputLevelId,
    pub(crate) action: InputRetirementAction,
    pub(crate) reason: InputRetirementReason,
    pub(crate) trace: Option<ReplayTrace>,
}

/// Observer-visible class of an exhausted input level.
///
/// It is derived only after retirement has selected its canonical action, so
/// replay explanation cannot influence input semantics.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum InputRetirementReason {
    Source,
    Backup,
    Macro,
    Parameter,
    AlignmentUTemplate,
    AlignmentVTemplate,
    Recovery,
    /// A stored token list, carrying which one it is: tex.web names an input
    /// level by its §307 `token_type`, so a retirement that reported only
    /// "some token list" could not be named at the observation boundary.
    TokenList(StoredReplayReason),
}

/// Canonical effect of exhausting or explicitly retiring one input level.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum InputRetirementAction {
    SourcePopped,
    TokenListPopped,
    TerminalStop,
    ScantokensClosed,
    VTemplateRetained,
    VTemplatePopped,
}

/// Why an exact input level could not be retired.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum InputRetirementError {
    NoInput,
    LevelChanged {
        expected: InputLevelId,
        actual: InputLevelId,
    },
    MacroActivationOrder {
        expected: MacroActivationId,
        actual: Option<MacroActivationId>,
    },
    VTemplateAlreadyRetained,
    NotRetainedVTemplate,
}

/// Result of applying canonical `OutParameter` handling to one delivery.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum OutParameterReplay {
    /// Parameter input is literal and therefore cannot substitute itself.
    Literal,
    /// A range owned by the matching macro activation was pushed.
    Pushed(InputLevelId),
}

/// Why an `OutParameter` could not resolve through the live `param_start`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum ParameterReplayError {
    NoInput,
    LevelChanged {
        expected: InputLevelId,
        actual: InputLevelId,
    },
    InvalidSlot(u8),
    NoMacroOwner,
    MissingActivation(MacroActivationId),
    MissingArgument {
        activation: MacroActivationId,
        slot: u8,
    },
    ArgumentRangeOutsideBuffer {
        activation: MacroActivationId,
        slot: u8,
    },
}

impl CommandState {
    /// Installs one complete macro activation and its replacement-body level.
    ///
    /// The canonical replacement list remains in immutable storage. Only the
    /// already matched arguments are transient, and they have one activation
    /// owner before this level can become visible to delivery.
    pub(crate) fn push_macro_activation(
        &mut self,
        definition: MacroDefinitionId,
        arguments: MacroArguments,
        invocation: OriginId,
        replacement_tokens: TokenListId,
        replacement_origins: OriginListId,
    ) -> InputLevelId {
        let activation = self
            .parameters
            .push_activation(definition, arguments, invocation);
        self.push_token_level(
            TokenPayload::Stored {
                tokens: replacement_tokens,
                origins: replacement_origins,
            },
            TokenBehavior::MacroBody(activation),
            RetirementBehavior::Pop,
            ReplayTrace::MacroReplacement,
        )
    }

    pub(crate) fn push_token_level(
        &mut self,
        payload: TokenPayload,
        behavior: TokenBehavior,
        retirement: RetirementBehavior,
        trace: ReplayTrace,
    ) -> InputLevelId {
        let identity = self.allocate_input_level_identity();
        self.input.levels.push(InputLevel::Tokens(TokenCursor {
            payload,
            behavior,
            retirement,
            trace,
            index: 0,
            identity,
        }));
        identity
    }

    /// Applies TeX's `param_start` ownership rule to one `OutParameter`.
    ///
    /// The owner is the nearest macro-body level at or below the delivering
    /// level, not merely the newest activation record. Parameter replay is
    /// literal, so an `OutParameter` delivered by a parameter level is not
    /// recursively substituted. This is the typed ownership counterpart of
    /// TeX.web §§357 and 359 (pdfTeX.web §§379 and 381).
    pub(crate) fn replay_out_parameter(
        &mut self,
        delivering_level: InputLevelId,
        slot: u8,
    ) -> Result<OutParameterReplay, ParameterReplayError> {
        let actual = self
            .input
            .levels
            .last()
            .map(input_level_identity)
            .ok_or(ParameterReplayError::NoInput)?;
        if actual != delivering_level {
            return Err(ParameterReplayError::LevelChanged {
                expected: delivering_level,
                actual,
            });
        }
        if matches!(
            self.input.levels.last(),
            Some(InputLevel::Tokens(TokenCursor {
                behavior: TokenBehavior::Parameter,
                ..
            }))
        ) {
            return Ok(OutParameterReplay::Literal);
        }
        if !(1..=9).contains(&slot) {
            return Err(ParameterReplayError::InvalidSlot(slot));
        }

        let mut owner = None;
        for level in self.input.levels.iter().rev() {
            match level {
                InputLevel::Tokens(TokenCursor {
                    behavior: TokenBehavior::MacroBody(candidate),
                    ..
                }) => {
                    owner = Some(*candidate);
                    break;
                }
                InputLevel::Tokens(_) => {}
                InputLevel::Source(_) => break,
            }
        }
        let owner = owner.ok_or(ParameterReplayError::NoMacroOwner)?;
        let activation = self
            .parameters
            .activations
            .iter()
            .find(|activation| activation.identity == owner)
            .ok_or(ParameterReplayError::MissingActivation(owner))?;
        let range = activation.arguments.ranges[usize::from(slot - 1)].ok_or(
            ParameterReplayError::MissingArgument {
                activation: owner,
                slot,
            },
        )?;
        if range.end() > activation.arguments.buffer.len() {
            return Err(ParameterReplayError::ArgumentRangeOutsideBuffer {
                activation: owner,
                slot,
            });
        }
        let payload = TokenPayload::ArgumentRange {
            buffer: activation.arguments.buffer.clone(),
            range,
        };
        let identity = self.push_token_level(
            payload,
            TokenBehavior::Parameter,
            RetirementBehavior::Pop,
            ReplayTrace::MacroParameter { slot },
        );
        Ok(OutParameterReplay::Pushed(identity))
    }

    /// Commits the end-of-level action selected by the exhausted top level.
    ///
    /// Exact identity prevents a stale delivery from retiring a replacement
    /// level. All popped payloads drop here, so transient allocations live
    /// exactly as long as their last cursor/snapshot owner; stored payloads
    /// drop only their immutable store handles. Macro-body retirement removes
    /// precisely its `param_start` activation. The v-template split mirrors
    /// TeX.web §§325 and 1131: exhaustion leaves the frame reachable until
    /// successful `do_endv`, which then performs the final pop.
    pub(crate) fn retire_exhausted_input(
        &mut self,
        expected: InputLevelId,
    ) -> Result<InputRetirement, InputRetirementError> {
        let level = self
            .input
            .levels
            .last()
            .ok_or(InputRetirementError::NoInput)?;
        let actual = input_level_identity(level);
        if actual != expected {
            return Err(InputRetirementError::LevelChanged { expected, actual });
        }

        let InputLevel::Tokens(cursor) = level else {
            self.input.levels.pop();
            return Ok(InputRetirement {
                identity: expected,
                action: InputRetirementAction::SourcePopped,
                reason: InputRetirementReason::Source,
                trace: None,
            });
        };
        if cursor.retirement == RetirementBehavior::AwaitingVTemplateRetirement {
            return Err(InputRetirementError::VTemplateAlreadyRetained);
        }
        if cursor.retirement == RetirementBehavior::RetainExhaustedVTemplate {
            if !matches!(cursor.behavior, TokenBehavior::VTemplate) {
                return Err(InputRetirementError::NotRetainedVTemplate);
            }
            let trace = cursor.trace.clone();
            let InputLevel::Tokens(cursor) = self
                .input
                .levels
                .last_mut()
                .expect("the inspected top level remains live")
            else {
                unreachable!("the inspected top level was a token cursor");
            };
            cursor.retirement = RetirementBehavior::AwaitingVTemplateRetirement;
            return Ok(InputRetirement {
                identity: expected,
                action: InputRetirementAction::VTemplateRetained,
                reason: input_retirement_reason(&cursor.behavior, &trace),
                trace: Some(trace),
            });
        }

        self.validate_macro_body_retirement(&cursor.behavior)?;
        let InputLevel::Tokens(cursor) = self
            .input
            .levels
            .pop()
            .expect("the inspected top level remains live")
        else {
            unreachable!("the inspected top level was a token cursor");
        };
        self.finish_macro_body_retirement(&cursor.behavior);
        let action = match cursor.retirement {
            RetirementBehavior::Pop => InputRetirementAction::TokenListPopped,
            RetirementBehavior::StopAtEnd => InputRetirementAction::TerminalStop,
            RetirementBehavior::CloseScantokens => InputRetirementAction::ScantokensClosed,
            RetirementBehavior::RetainExhaustedVTemplate
            | RetirementBehavior::AwaitingVTemplateRetirement => {
                unreachable!("retained templates returned before popping")
            }
        };
        Ok(InputRetirement {
            identity: expected,
            action,
            reason: input_retirement_reason(&cursor.behavior, &cursor.trace),
            trace: Some(cursor.trace),
        })
    }

    /// Retires the exact exhausted v-template after successful `do_endv`.
    pub(crate) fn retire_retained_v_template(
        &mut self,
        expected: InputLevelId,
    ) -> Result<InputRetirement, InputRetirementError> {
        let level = self
            .input
            .levels
            .last()
            .ok_or(InputRetirementError::NoInput)?;
        let actual = input_level_identity(level);
        if actual != expected {
            return Err(InputRetirementError::LevelChanged { expected, actual });
        }
        let InputLevel::Tokens(TokenCursor {
            retirement: RetirementBehavior::AwaitingVTemplateRetirement,
            behavior: TokenBehavior::VTemplate,
            ..
        }) = level
        else {
            return Err(InputRetirementError::NotRetainedVTemplate);
        };
        let InputLevel::Tokens(cursor) = self
            .input
            .levels
            .pop()
            .expect("the inspected retained template remains live")
        else {
            unreachable!("the inspected level was a token cursor");
        };
        Ok(InputRetirement {
            identity: expected,
            action: InputRetirementAction::VTemplatePopped,
            reason: input_retirement_reason(&cursor.behavior, &cursor.trace),
            trace: Some(cursor.trace),
        })
    }

    /// Pops one input level for TeX82 §1335's `final_cleanup` unwinding.
    ///
    /// §1335 runs `while input_ptr>0 do if state=token_list then
    /// end_token_list else end_file_reading` once §1054's `its_all_over` has
    /// returned true.  Both arms are unconditional pops: the job is over, no
    /// level will ever be read again, and none of the exhaustion-time rules
    /// [`Self::retire_exhausted_input`] enforces (exact expected identity,
    /// macro-activation order, v-template retention) applies.  Levels
    /// therefore unwind top-down here even when a macro body, an alignment
    /// template, or a partially consumed source is still live.
    pub(crate) fn pop_input_level_at_end_of_job(&mut self) -> Option<InputRetirement> {
        let level = self.input.levels.pop()?;
        let identity = input_level_identity(&level);
        let InputLevel::Tokens(cursor) = level else {
            return Some(InputRetirement {
                identity,
                action: InputRetirementAction::SourcePopped,
                reason: InputRetirementReason::Source,
                trace: None,
            });
        };
        self.finish_macro_body_retirement(&cursor.behavior);
        let action = match cursor.retirement {
            RetirementBehavior::StopAtEnd => InputRetirementAction::TerminalStop,
            RetirementBehavior::CloseScantokens => InputRetirementAction::ScantokensClosed,
            RetirementBehavior::Pop
            | RetirementBehavior::RetainExhaustedVTemplate
            | RetirementBehavior::AwaitingVTemplateRetirement => {
                InputRetirementAction::TokenListPopped
            }
        };
        Some(InputRetirement {
            identity,
            action,
            reason: input_retirement_reason(&cursor.behavior, &cursor.trace),
            trace: Some(cursor.trace),
        })
    }

    fn allocate_input_level_identity(&mut self) -> InputLevelId {
        let identity = InputLevelId(self.input.next_level_identity);
        self.input.next_level_identity = self.input.next_level_identity.wrapping_add(1);
        identity
    }

    fn validate_macro_body_retirement(
        &self,
        behavior: &TokenBehavior,
    ) -> Result<(), InputRetirementError> {
        let TokenBehavior::MacroBody(expected) = behavior else {
            return Ok(());
        };
        let actual = self
            .parameters
            .activations
            .last()
            .map(|activation| activation.identity);
        if actual == Some(*expected) {
            Ok(())
        } else {
            Err(InputRetirementError::MacroActivationOrder {
                expected: *expected,
                actual,
            })
        }
    }

    fn finish_macro_body_retirement(&mut self, behavior: &TokenBehavior) {
        if matches!(behavior, TokenBehavior::MacroBody(_)) {
            self.parameters.activations.pop();
        }
    }
}

fn input_retirement_reason(behavior: &TokenBehavior, trace: &ReplayTrace) -> InputRetirementReason {
    match behavior {
        TokenBehavior::UTemplate => return InputRetirementReason::AlignmentUTemplate,
        TokenBehavior::VTemplate => return InputRetirementReason::AlignmentVTemplate,
        TokenBehavior::Ordinary
        | TokenBehavior::Recovery
        | TokenBehavior::MacroBody(_)
        | TokenBehavior::Parameter
        | TokenBehavior::BackedUp(_) => {}
    }
    match trace {
        ReplayTrace::BackedUp => InputRetirementReason::Backup,
        ReplayTrace::MacroReplacement => InputRetirementReason::Macro,
        ReplayTrace::MacroParameter { .. } => InputRetirementReason::Parameter,
        ReplayTrace::UTemplate | ReplayTrace::VTemplate => {
            unreachable!("alignment template behavior must accompany its replay trace")
        }
        ReplayTrace::Inserted | ReplayTrace::Transient(_) => InputRetirementReason::Recovery,
        ReplayTrace::Stored(reason) => InputRetirementReason::TokenList(*reason),
    }
}

pub(crate) fn input_level_identity(level: &InputLevel) -> InputLevelId {
    match level {
        InputLevel::Source(level) => level.identity,
        InputLevel::Tokens(level) => level.identity,
    }
}

#[cfg(test)]
mod tests;
