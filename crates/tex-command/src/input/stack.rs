//! Canonical input-stack replay and retirement mechanics.
#![allow(dead_code)] // consumed by the ordered raw-delivery implementation issues

use crate::CommandState;
use tex_state::DefinitionId;
use tex_state::token::OriginId;

use crate::macro_call::{MacroActivationId, MacroArguments};

use super::{
    InputLevel, InputLevelId, PackedTokenSpanHandle, ReplayTrace, RetirementBehavior,
    SourceNameClass, StoredReplayReason, TokenBehavior, TokenCursor,
};

/// One committed input-lifecycle transition.
///
/// `trace` explains the replay that ended but does not select `action`.
#[derive(Debug, Eq, Hash, PartialEq)]
pub(crate) struct InputRetirement {
    pub(crate) identity: InputLevelId,
    pub(crate) action: InputRetirementAction,
    pub(crate) reason: InputRetirementReason,
    /// tex.web §303's `name` classification of the level that ended, present
    /// exactly when a source level ended. §329's `end_file_reading` is the
    /// only retirement that consults it (`if name>17 then a_close`), and
    /// §307's token-list levels have no `name` classification at all.
    pub(crate) name_class: Option<SourceNameClass>,
    pub(crate) source: Option<tex_state::SourceId>,
    pub(crate) trace: Option<ReplayTrace>,
    /// Frame-owned e-TeX nesting ancestry moved out by source retirement.
    pub(crate) source_open_depths: Option<Box<super::SourceOpenDepths>>,
    /// Whether §362 must print this source retirement's bare `)` now.
    pub(crate) closes_file_frame: bool,
}

enum RetiredInputLevel<G> {
    Source {
        identity: InputLevelId,
        name_class: SourceNameClass,
        source: tex_state::SourceId,
        retirement: super::SourceRetirement,
        open_depths: Option<Box<super::SourceOpenDepths>>,
        framed: bool,
    },
    Tokens {
        identity: InputLevelId,
        behavior: TokenBehavior,
        retirement: RetirementBehavior,
        trace: ReplayTrace,
        replay: Option<super::ReplayPayloadId<G>>,
    },
}

impl<G> RetiredInputLevel<G> {
    fn borrowed(level: &InputLevel<G>) -> Self {
        match level {
            InputLevel::Source(source) => Self::Source {
                identity: source.identity(),
                name_class: source.name_class,
                source: source.cursor.current_backing().id,
                retirement: source.retirement,
                open_depths: source.open_depths.clone(),
                framed: source_level_is_framed(source),
            },
            InputLevel::Tokens(cursor) => Self::Tokens {
                identity: cursor.identity(),
                behavior: cursor.behavior,
                retirement: cursor.retirement,
                trace: cursor.trace.clone(),
                replay: match &cursor.span {
                    PackedTokenSpanHandle::Replay { replay, .. } => Some(*replay),
                    _ => None,
                },
            },
        }
    }

    fn owned(level: InputLevel<G>) -> Self {
        match level {
            InputLevel::Source(source) => {
                let framed = source_level_is_framed(&source);
                Self::Source {
                    identity: source.identity(),
                    name_class: source.name_class,
                    source: source.cursor.current_backing().id,
                    retirement: source.retirement,
                    open_depths: source.open_depths,
                    framed,
                }
            }
            InputLevel::Tokens(cursor) => Self::Tokens {
                identity: cursor.identity(),
                behavior: cursor.behavior,
                retirement: cursor.retirement,
                trace: cursor.trace,
                replay: match cursor.span {
                    PackedTokenSpanHandle::Replay { replay, .. } => Some(replay),
                    _ => None,
                },
            },
        }
    }
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
    /// tex.web §789's constant `omit_template`, installed in place of a
    /// column's ⟨v_j⟩ part. It is still `token_type=v_template` (§307); only
    /// the list differs, which is exactly how `end_token_list` tells the two
    /// apart when it names the level.
    AlignmentOmitTemplate,
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
    /// tex.web §360's `\read` pseudo-file: the level's one line has ended,
    /// which `get_next` reports as `cur_cmd:=cur_chr:=0` rather than by
    /// resuming the enclosing level.
    ReadLineEnded,
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
    AttemptRootInvariant,
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

impl<G> CommandState<G> {
    fn pop_retired_input_level(&mut self) -> Option<RetiredInputLevel<G>> {
        if !self.input.levels.records_history() {
            return self.input.levels.pop_owned().map(RetiredInputLevel::owned);
        }
        self.input.levels.pop_project(RetiredInputLevel::borrowed)
    }

    /// TeX82 one-word nodes owned by live command input and argument buffers.
    ///
    /// An execution operation that allocates recursively must compose its
    /// peak with these owners before the command stack can retire them.
    #[must_use]
    pub fn transient_dynamic_words(&self) -> usize {
        let arguments = self.scratch.argument_word_len();
        self.input.levels.iter().fold(arguments, |words, level| {
            let InputLevel::Tokens(cursor) = level else {
                return words;
            };
            let owned = match cursor.span {
                PackedTokenSpanHandle::Replay { replay, len }
                    if matches!(
                        self.input.replay.ownership(replay),
                        Some(
                            super::PackedTokenOwnership::Transient
                                | super::PackedTokenOwnership::BackedUp
                        )
                    ) =>
                {
                    len as usize
                }
                _ => 0,
            };
            words.saturating_add(owned)
        })
    }

    pub(crate) fn top_input_level_identity(&self) -> Option<InputLevelId> {
        self.input.levels.last().map(input_level_identity)
    }

    /// Installs one complete macro activation and its replacement-body level.
    ///
    /// The canonical replacement list remains in immutable storage. Only the
    /// already matched arguments are transient, and they have one activation
    /// owner before this level can become visible to delivery.
    pub(crate) fn push_macro_activation(
        &mut self,
        name: tex_state::interner::Symbol,
        definition: DefinitionId<G>,
        arguments: MacroArguments<G>,
        invocation: OriginId,
        replacement_len: usize,
    ) -> InputLevelId {
        let parameter_count = self
            .scratch
            .argument_count(arguments.frame())
            .expect("live macro argument record");
        let parameter_ptr = self
            .parameters
            .activations
            .iter()
            .map(|activation| {
                self.scratch
                    .argument_count(activation.arguments.frame())
                    .expect("live macro argument record")
            })
            .sum::<usize>()
            .saturating_add(parameter_count);
        self.stack_usage.record_parameter_push(parameter_ptr);
        let activation =
            self.parameters
                .push_activation(name, definition.clone(), arguments, invocation);
        self.push_token_level(
            PackedTokenSpanHandle::MacroReplacement {
                definition,
                len: u32::try_from(replacement_len).expect("macro replacement exceeds u32"),
            },
            TokenBehavior::MacroBody(activation),
            RetirementBehavior::Pop,
            ReplayTrace::MacroReplacement,
        )
    }

    pub(crate) fn push_token_level<P: super::PackedTokenSpanSource<G>>(
        &mut self,
        source: P,
        behavior: TokenBehavior,
        retirement: RetirementBehavior,
        trace: ReplayTrace,
    ) -> InputLevelId {
        let span = source
            .admit(&mut self.input.replay)
            .expect("generation replay lane admission");
        let identity = self.allocate_input_level_identity();
        let frame =
            super::packed_token_frame(identity, span.frame_len(), &behavior, retirement, &trace);
        self.push_input_level(InputLevel::Tokens(TokenCursor {
            span,
            behavior,
            retirement,
            trace,
            frame,
        }));
        identity
    }

    /// Pushes one attempt-local list owned by its enclosing attempt scope.
    ///
    /// The arena's scope stack, rather than this input level, owns storage.
    /// Exact-LIFO macro/scanner retirement therefore keeps this coordinate
    /// live without a copied high-water mark or an input-stack census.
    pub(crate) fn push_attempt_list_level(
        &mut self,
        list: crate::attempt::AttemptTokenListId,
        len: u32,
        behavior: TokenBehavior,
        retirement: RetirementBehavior,
        trace: ReplayTrace,
    ) -> Result<InputLevelId, crate::AttemptError> {
        self.attempt.arena().token_words(list)?;
        Ok(self.push_token_level(
            PackedTokenSpanHandle::AttemptList { list, len },
            behavior,
            retirement,
            trace,
        ))
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
        let range = self
            .scratch
            .argument_range(activation.arguments.frame(), slot)
            .ok()
            .flatten()
            .ok_or(ParameterReplayError::MissingArgument {
                activation: owner,
                slot,
            })?;
        let len = self.scratch.argument_len(range).map_err(|_| {
            ParameterReplayError::ArgumentRangeOutsideBuffer {
                activation: owner,
                slot,
            }
        })?;
        let identity = self.push_token_level(
            PackedTokenSpanHandle::MacroArgument {
                range,
                len: u32::try_from(len).expect("macro argument length exceeds u32"),
            },
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
    /// precisely its `param_start` activation.
    ///
    /// The v-template split mirrors tex.web §§325/§390 and §1131. An exhausted
    /// v-part first reports its frozen `end_template` boundary and stays
    /// reachable, because §325's `back_input` and §390's `macro_call` write
    /// `while (state=token_list)and(loc=null)and(token_type<>v_template) do
    /// end_token_list` -- v-template is their sole exception -- and §1131's
    /// `do_endv` walks the stack expecting to find that frame still live.
    /// After that boundary the frame is an ordinary depleted token list:
    /// §1131 only inspects it, and §357's `else begin end_token_list; goto
    /// restart; end` pops it the next time `get_next` reaches it. That is
    /// why the retained frame pops here rather than at a `do_endv` call site:
    /// with a non-empty `\everycr`, §799's `begin_token_list(every_cr,
    /// every_cr_text)` buries it and it survives the whole `\noalign` body.
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
            let RetiredInputLevel::Source {
                name_class,
                source: source_id,
                retirement,
                open_depths,
                framed,
                ..
            } = self
                .pop_retired_input_level()
                .expect("the inspected top source remains live")
            else {
                unreachable!("the inspected top level was not a source cursor");
            };
            let action = source_retirement_action(retirement);
            // TeX82 §362 tests and clears the process-global `force_eof`
            // only in §360's `name>17` (real-file) refill branch. §483's
            // `name<=17` read/terminal pseudo-sources retire without
            // consuming a pending forced EOF meant for their parent file.
            //
            // §362 also prints `)` for exactly that same `name>17` case.
            // `pop_input_level_at_end_of_job` deliberately does not mirror
            // it: its unconditional §1335 unwinding is the *other* closing
            // mechanism, `final_cleanup`'s `␣)` per still-open file, which
            // the engine renders from its own `open_parens` count rather than
            // from this queue.
            //
            // This exactly mirrors `push_source_level`'s open gate. Named
            // files and traced scantokens (numeric name 19) receive a close;
            // an unnamed File registration cannot manufacture an orphan.
            //
            // §362 clears the process-global `force_eof` for the same
            // `name>17` case, which is why both live under this gate.
            if name_class == SourceNameClass::File {
                self.timeline.record_force_eof(self.input.force_eof);
                self.input.force_eof = false;
            }
            return Ok(InputRetirement {
                identity: expected,
                action,
                reason: InputRetirementReason::Source,
                name_class: Some(name_class),
                source: Some(source_id),
                trace: None,
                source_open_depths: open_depths,
                closes_file_frame: framed,
            });
        };
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
            cursor
                .frame
                .add_flags(tex_state::packed_input::InputFrameFlags::RETAIN_AT_END);
            return Ok(InputRetirement {
                identity: expected,
                action: InputRetirementAction::VTemplateRetained,
                reason: input_retirement_reason(&cursor.behavior, &trace),
                name_class: None,
                source: None,
                trace: Some(trace),
                source_open_depths: None,
                closes_file_frame: false,
            });
        }

        self.validate_macro_body_retirement(&cursor.behavior)?;
        let RetiredInputLevel::Tokens {
            behavior,
            retirement,
            trace,
            replay,
            ..
        } = self
            .pop_retired_input_level()
            .expect("the inspected top level remains live")
        else {
            unreachable!("the inspected top level was a token cursor");
        };
        self.finish_macro_body_retirement(&behavior)
            .map_err(|_| InputRetirementError::AttemptRootInvariant)?;
        if let Some(replay) = replay {
            self.input
                .replay
                .release(replay)
                .map_err(|_| InputRetirementError::AttemptRootInvariant)?;
        }
        let action = match retirement {
            RetirementBehavior::Pop => InputRetirementAction::TokenListPopped,
            RetirementBehavior::StopAtEnd => InputRetirementAction::TerminalStop,
            // tex.web §357: a v-template that has already reported its frozen
            // `end_template` boundary is an ordinary depleted token list, so
            // the next `get_next` that reaches it runs `end_token_list`.
            RetirementBehavior::AwaitingVTemplateRetirement => {
                InputRetirementAction::VTemplatePopped
            }
            RetirementBehavior::RetainExhaustedVTemplate => {
                unreachable!("retained templates returned before popping")
            }
        };
        Ok(InputRetirement {
            identity: expected,
            action,
            reason: input_retirement_reason(&behavior, &trace),
            name_class: None,
            source: None,
            trace: Some(trace),
            source_open_depths: None,
            closes_file_frame: false,
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
        let level = self.pop_retired_input_level()?;
        let RetiredInputLevel::Tokens {
            identity,
            behavior,
            retirement,
            trace,
            replay,
        } = level
        else {
            let RetiredInputLevel::Source {
                identity,
                name_class,
                source,
                retirement,
                open_depths,
                ..
            } = level
            else {
                unreachable!("the popped level was not a token cursor");
            };
            let action = source_retirement_action(retirement);
            return Some(InputRetirement {
                identity,
                action,
                reason: InputRetirementReason::Source,
                name_class: Some(name_class),
                source: Some(source),
                trace: None,
                source_open_depths: open_depths,
                closes_file_frame: false,
            });
        };
        self.finish_macro_body_retirement(&behavior)
            .expect("final cleanup runs inside one direct operation");
        if let Some(replay) = replay {
            self.input
                .replay
                .release(replay)
                .expect("final cleanup preserves replay LIFO order");
        }
        let action = match retirement {
            RetirementBehavior::StopAtEnd => InputRetirementAction::TerminalStop,
            RetirementBehavior::Pop
            | RetirementBehavior::RetainExhaustedVTemplate
            | RetirementBehavior::AwaitingVTemplateRetirement => {
                InputRetirementAction::TokenListPopped
            }
        };
        Some(InputRetirement {
            identity,
            action,
            reason: input_retirement_reason(&behavior, &trace),
            name_class: None,
            source: None,
            trace: Some(trace),
            source_open_depths: None,
            closes_file_frame: false,
        })
    }

    /// Commits the one canonical input-frame push transition.
    pub(crate) fn push_input_level(&mut self, level: InputLevel<G>) {
        // TeX82 §321 checks `input_ptr` before `push_input` increments it.
        self.stack_usage.input_stack = self.stack_usage.input_stack.max(self.input.levels.len());
        self.input.levels.push(level);
    }

    pub(crate) fn allocate_input_level_identity(&mut self) -> InputLevelId {
        let identity = InputLevelId(self.input.next_level_identity);
        self.timeline
            .record_next_input_level_identity(self.input.next_level_identity);
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
        let actual = self.parameters.activations.last();
        if actual.map(|activation| activation.identity) == Some(*expected) {
            Ok(())
        } else {
            Err(InputRetirementError::MacroActivationOrder {
                expected: *expected,
                actual: actual.map(|activation| activation.identity),
            })
        }
    }

    fn finish_macro_body_retirement(
        &mut self,
        behavior: &TokenBehavior,
    ) -> Result<(), crate::execution_scratch::ScratchError> {
        if matches!(behavior, TokenBehavior::MacroBody(_))
            && let Some(arguments) = self.parameters.retire_last_activation()
        {
            self.scratch.pop_macro_frame(arguments.frame())?;
        }
        Ok(())
    }
}

fn input_retirement_reason(behavior: &TokenBehavior, trace: &ReplayTrace) -> InputRetirementReason {
    match behavior {
        TokenBehavior::UTemplate => return InputRetirementReason::AlignmentUTemplate,
        TokenBehavior::VTemplate => {
            return if matches!(trace, ReplayTrace::OmitTemplate) {
                InputRetirementReason::AlignmentOmitTemplate
            } else {
                InputRetirementReason::AlignmentVTemplate
            };
        }
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
        ReplayTrace::UTemplate | ReplayTrace::VTemplate | ReplayTrace::OmitTemplate => {
            unreachable!("alignment template behavior must accompany its replay trace")
        }
        ReplayTrace::Inserted | ReplayTrace::Transient(_) => InputRetirementReason::Recovery,
        ReplayTrace::Stored(reason) => InputRetirementReason::TokenList(*reason),
    }
}

fn source_level_is_framed<G>(source: &super::SourceLevel<G>) -> bool {
    match source.name_class {
        SourceNameClass::File => {
            source.cursor.backing.name.is_some()
                && source.cursor.backing.framing == crate::SourceFramingPolicy::Canonical
        }
        SourceNameClass::Scantokens(19) => true,
        SourceNameClass::Terminal
        | SourceNameClass::ReadStream(_)
        | SourceNameClass::Scantokens(_) => false,
    }
}

pub(crate) fn input_level_identity<G>(level: &InputLevel<G>) -> InputLevelId {
    match level {
        InputLevel::Source(level) => level.identity(),
        InputLevel::Tokens(level) => level.identity(),
    }
}

#[cfg(test)]
mod tests;

/// Names the canonical effect of exhausting one source level (tex.web §360).
const fn source_retirement_action(retirement: super::SourceRetirement) -> InputRetirementAction {
    match retirement {
        super::SourceRetirement::Pop => InputRetirementAction::SourcePopped,
        super::SourceRetirement::EndReadLine => InputRetirementAction::ReadLineEnded,
    }
}
