//! Canonical input-stack replay and retirement mechanics.
#![allow(dead_code)] // consumed by the ordered raw-delivery implementation issues

use crate::CommandState;
use tex_state::token::{Catcode, OriginId, Token, TokenWord};

use crate::execution_scratch::ArgumentSetId;

use super::{
    CompactSourceStepQueries, InputLevel, InputLevelId, PackedTokenSpanHandle, ReplayTrace,
    ResidentTokenRow, ResidentTokenStorage, RetirementBehavior, SourceControlSequenceKind,
    SourceNameClass, SourceToken, StoredReplayReason, TokenBehavior, TokenRowHeader,
};

/// Completed command-state outcome of one resident input transition.
///
/// Ordinary commands have already committed their exact work facts, received
/// their one-delivery suppression, and received alignment treatment. Only a
/// forbidden outer command crosses back to the processor, whose cold recovery
/// path owns diagnostics and input insertion.
#[derive(Debug)]
pub(crate) enum ResidentBoundary {
    /// A borrowed main-loop character run ended at an input boundary before
    /// another token was consumed.
    CharacterRunEnd,
    CharacterRunFailure(crate::CommandError),
    InvalidCharacter,
    NeedLine(InputLevelId),
    SourceExhausted(InputLevelId),
    TokenExhausted {
        identity: InputLevelId,
        resident_index: usize,
    },
    ReplayCompleted(crate::CommandReplayEpisode),
    Empty,
    Failure,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct SourceRegistrationCounters {
    pub(crate) checks: u64,
    pub(crate) calls: u64,
}

#[cfg(test)]
thread_local! {
    static SOURCE_REGISTRATION_COUNTERS: std::cell::Cell<SourceRegistrationCounters> =
        const { std::cell::Cell::new(SourceRegistrationCounters { checks: 0, calls: 0 }) };
}

#[cfg(test)]
pub(crate) fn source_registration_counters() -> SourceRegistrationCounters {
    SOURCE_REGISTRATION_COUNTERS.with(std::cell::Cell::get)
}

#[cfg(test)]
fn record_source_registration_check() {
    SOURCE_REGISTRATION_COUNTERS.with(|slot| {
        let mut counters = slot.get();
        counters.checks += 1;
        slot.set(counters);
    });
}

#[cfg(test)]
fn record_source_registration_call() {
    SOURCE_REGISTRATION_COUNTERS.with(|slot| {
        let mut counters = slot.get();
        counters.calls += 1;
        slot.set(counters);
    });
}

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
    /// Copy-only result of comparing the still-borrowed source ancestry with
    /// current group and conditional stacks before the row is popped.
    pub(crate) file_warning_boundary: Option<FileWarningBoundary>,
    /// Whether §362 must print this source retirement's bare `)` now.
    pub(crate) closes_file_frame: bool,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct FileWarningBoundary {
    pub(crate) group_start: u32,
    pub(crate) condition_start: u32,
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
    AttemptRootInvariant,
    NotRetainedVTemplate,
}

impl<G> CommandState<G> {
    /// Acquires, firms, registers, and accounts for one physical line on the
    /// current source. This is the only production transition that computes
    /// lower-buffer occupancy or changes TeX's retained `line` scalar.
    pub(crate) fn acquire_input_top_line(
        &mut self,
        state: &mut tex_state::CommandContext<'_, G>,
        create_control_sequences: bool,
        endlinechar: i32,
        firm: bool,
        pending_acquired_line: bool,
    ) -> Result<Option<super::PhysicalLine>, ()> {
        let _ = self.input.levels.mutate_top_source_lex(|_, slot| {
            register_pending_source_backings(state, &mut slot.cursor)
        });
        if state.tracked_region_is_active()
            && let Some((source, slot)) = self.input.levels.top_source()
        {
            super::observe_immutable_source(state, source, slot);
        }
        let physical = {
            let mut queries = LiveSourceQueries {
                state,
                create_control_sequences,
            };
            self.acquire_input_top_line_with_queries(
                endlinechar,
                firm,
                pending_acquired_line,
                &mut queries,
            )?
        };
        let _ = self.input.levels.mutate_top_source_lex(|_, slot| {
            register_pending_source_backings(state, &mut slot.cursor)
        });
        if state.tracked_region_is_active()
            && let Some((source, slot)) = self.input.levels.top_source()
        {
            super::observe_immutable_source(state, source, slot);
        }
        Ok(physical)
    }

    pub(crate) fn acquire_input_top_line_with_queries(
        &mut self,
        endlinechar: i32,
        firm: bool,
        pending_acquired_line: bool,
        queries: &mut dyn crate::SourceStepQueries,
    ) -> Result<Option<super::PhysicalLine>, ()> {
        let occupied_below_active = self.input.levels.occupied_source_buffer_slots_below_top();
        let buffer_start = 1_usize
            .saturating_add(self.terminal_buffer_slots)
            .saturating_add(occupied_below_active);
        let profile = self.profile();
        let old_next_source_identity = self.input.next_source_identity;
        let (physical, retained_line) = {
            let usage = &mut self.stack_usage;
            let input = &mut self.roots.input;
            let Some(result) = input.levels.mutate_top_source_cursor(|source, slot| {
                let identity = source.identity();
                let name_class = slot.name_class;
                if pending_acquired_line {
                    slot.cursor.pending_acquired_line = true;
                }
                let mut lines = super::LineBackingRegistry {
                    profile,
                    next_identity: &mut input.next_source_identity,
                    usage,
                    buffer_start,
                    name_class: Some(name_class),
                };
                slot.cursor
                    .load_next_line(endlinechar)
                    .map(|line| line.physical)
                    .map(|physical| {
                        lines.record_line_usage(&slot.cursor);
                        if firm {
                            slot.cursor
                                .firm_up_the_line(endlinechar, queries, &mut lines);
                            lines.record_line_usage(&slot.cursor);
                        }
                        let retained_line = match name_class {
                            SourceNameClass::File | SourceNameClass::Scantokens(_) => slot
                                .cursor
                                .line
                                .as_ref()
                                .map(|line| line.physical.number().min(i32::MAX as u64) as i32)
                                .unwrap_or(0),
                            SourceNameClass::Terminal | SourceNameClass::ReadStream(_) => 0,
                        };
                        debug_assert_eq!(source.identity(), identity);
                        (physical, retained_line)
                    })
            }) else {
                return Err(());
            };
            let Some(result) = result else {
                return Ok(None);
            };
            result
        };
        if self.input.next_source_identity != old_next_source_identity {
            self.timeline
                .record_next_source_identity(old_next_source_identity);
        }
        self.set_retained_file_line_number(retained_line);
        Ok(Some(physical))
    }

    /// Commits provenance for an exhausted source before any observer can see
    /// its retirement and before the owning input row is popped.
    ///
    /// This cold seam covers sources which never reached production physical
    /// acquisition, including forced EOF. Registration failures remain
    /// diagnostic-only and retry on a later exhaustion transition while the
    /// source is still live.
    pub(crate) fn register_exhausted_source_backings(
        &mut self,
        state: &mut tex_state::CommandContext<'_, G>,
        identity: InputLevelId,
    ) -> Result<(), ()> {
        self.input
            .levels
            .mutate_top_source_lex(|source, slot| {
                if source.identity() != identity {
                    return Err(());
                }
                register_pending_source_backings(state, &mut slot.cursor);
                Ok(())
            })
            .unwrap_or(Err(()))
    }

    pub(crate) fn set_retained_file_line_number(&mut self, line: i32) {
        if self.input.retained_file_line_number == line {
            return;
        }
        self.timeline
            .record_retained_file_line_number(self.input.retained_file_line_number);
        self.input.retained_file_line_number = line;
    }

    /// Retires an exhausted restartable token or macro-argument row selected
    /// by the resident delivery transition itself.
    ///
    /// `index` is the already-admitted top coordinate. Terminal token input
    /// and a v-template still waiting for `do_endv` deliberately return `None`
    /// so the processor's explicit cold handling remains authoritative. Once
    /// `do_endv` has completed, its awaiting v-template is a §357 resident
    /// restart just like any other exhausted token list.
    pub(crate) fn retire_resident_ordinary_input(
        &mut self,
        index: usize,
        observer: &mut Option<&mut dyn crate::CommandObserver>,
        immediate_write_retirement: &mut Option<InputLevelId>,
    ) -> Result<Option<crate::CommandReplayEpisode>, InputRetirementError> {
        let InputLevel::Resident(row) = self.input.levels.resident_at(index) else {
            unreachable!("resident coordinate selects a resident row");
        };
        let retirement = row.header.retirement();
        if !matches!(
            retirement,
            RetirementBehavior::Pop | RetirementBehavior::AwaitingVTemplateRetirement
        ) {
            return Ok(None);
        }
        let identity = row.header.identity();
        let reason = input_retirement_reason(&row.header.behavior(), &row.trace());
        let (macro_body, arguments, replay) = match &row.storage {
            ResidentTokenStorage::MacroBody(body) => (true, body.arguments, None),
            ResidentTokenStorage::Replay { replay, .. } => (false, None, Some(*replay)),
            ResidentTokenStorage::Durable(_)
            | ResidentTokenStorage::Attempt(_)
            | ResidentTokenStorage::MacroArgument(_) => (false, None, None),
        };
        let parameter_count = arguments.map_or(0, |arguments| {
            self.scratch
                .argument_count(arguments)
                .expect("live macro argument set")
        });
        self.input.levels.pop_resident(index);
        if macro_body {
            self.input.levels.retire_macro_body(parameter_count);
            if let Some(arguments) = arguments {
                self.scratch
                    .release_argument_set(arguments)
                    .map_err(|_| InputRetirementError::AttemptRootInvariant)?;
            }
            return Ok(self.settle_resident_retirement(
                identity,
                InputRetirementAction::TokenListPopped,
                InputRetirementReason::Macro,
                observer,
                immediate_write_retirement,
            ));
        }
        if let Some(replay) = replay {
            self.input
                .replay
                .release(replay)
                .map_err(|_| InputRetirementError::AttemptRootInvariant)?;
        }
        let action = match retirement {
            RetirementBehavior::Pop => InputRetirementAction::TokenListPopped,
            RetirementBehavior::AwaitingVTemplateRetirement => {
                InputRetirementAction::VTemplatePopped
            }
            RetirementBehavior::StopAtEnd | RetirementBehavior::RetainExhaustedVTemplate => {
                unreachable!("cold retirement returned before resident pop")
            }
        };
        #[cfg(test)]
        {
            self.raw_delivery_path_counters
                .resident_ordinary_retirements = self
                .raw_delivery_path_counters
                .resident_ordinary_retirements
                .saturating_add(1);
        }
        Ok(self.settle_resident_retirement(
            identity,
            action,
            reason,
            observer,
            immediate_write_retirement,
        ))
    }

    /// TeX82 one-word nodes owned by live command input and argument buffers.
    ///
    /// An execution operation that allocates recursively must compose its
    /// peak with these owners before the command stack can retire them.
    #[must_use]
    pub fn transient_dynamic_words(&self) -> usize {
        let arguments = self.scratch.argument_word_len();
        self.input.levels.iter().fold(arguments, |words, level| {
            let InputLevel::Resident(ResidentTokenRow {
                header,
                storage: ResidentTokenStorage::Replay { replay, .. },
            }) = level
            else {
                return words;
            };
            let owned = if matches!(
                self.input.replay.ownership(*replay),
                Some(
                    super::PackedTokenOwnership::Transient | super::PackedTokenOwnership::BackedUp
                )
            ) {
                header.frame.limit() as usize
            } else {
                0
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
        body: tex_state::ResidentMacroBody<G>,
        arguments: Option<ArgumentSetId<G>>,
        invocation: OriginId,
    ) -> InputLevelId {
        let parameter_count = arguments.map_or(0, |arguments| {
            self.scratch
                .argument_count(arguments)
                .expect("live macro argument set")
        });
        let parameter_ptr = self
            .input
            .levels
            .active_macro_parameters()
            .saturating_add(parameter_count);
        self.stack_usage.record_parameter_push(parameter_ptr);
        let identity = self.allocate_input_level_identity();
        let source = self.input.levels.current_source_context();
        let mut frame = super::packed_token_frame(
            identity,
            body.len(),
            &TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            &ReplayTrace::MacroReplacement,
        );
        frame.set_source_context(source);
        self.input.levels.push_macro_body(
            InputLevel::Resident(ResidentTokenRow {
                header: TokenRowHeader::new(
                    TokenBehavior::Ordinary,
                    RetirementBehavior::Pop,
                    ReplayTrace::MacroReplacement,
                    frame,
                ),
                storage: ResidentTokenStorage::MacroBody(super::MacroBodyCursor::new(
                    body, arguments, name, invocation,
                )),
            }),
            parameter_count,
        );
        identity
    }

    /// Accounts for TeX82's logical `begin_token_list` boundary when an empty
    /// simple macro needs no resident input row.
    pub(crate) fn record_empty_macro_activation(&mut self) {
        self.stack_usage.input_stack = self.stack_usage.input_stack.max(self.input.levels.len());
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
        let mut frame =
            super::packed_token_frame(identity, span.frame_len(), &behavior, retirement, &trace);
        frame.set_source_context(self.input.levels.current_source_context());
        let header = TokenRowHeader::new(behavior, retirement, trace, frame);
        let level = match span {
            PackedTokenSpanHandle::Replay { replay, .. } => {
                let resident = self
                    .input
                    .replay
                    .resident_cursor(replay)
                    .expect("admitted replay span has a resident coordinate");
                InputLevel::Resident(ResidentTokenRow {
                    header,
                    storage: ResidentTokenStorage::Replay {
                        replay,
                        cursor: resident,
                    },
                })
            }
            PackedTokenSpanHandle::DurableList { list, .. } => {
                InputLevel::Resident(ResidentTokenRow {
                    header,
                    storage: ResidentTokenStorage::Durable(list),
                })
            }
            PackedTokenSpanHandle::AttemptList { list, .. } => {
                InputLevel::Resident(ResidentTokenRow {
                    header,
                    storage: ResidentTokenStorage::Attempt(list),
                })
            }
        };
        self.push_input_level(level);
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
        let admitted = self.attempt.arena().token_words(list)?;
        if admitted.len() != len as usize {
            return Err(crate::AttemptError::InvalidCoordinate);
        }
        Ok(self.push_token_level(
            PackedTokenSpanHandle::AttemptList { list, len },
            behavior,
            retirement,
            trace,
        ))
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
        self.retire_exhausted_input_with_file_warning(expected, None)
    }

    pub(crate) fn retire_exhausted_input_with_file_warning(
        &mut self,
        expected: InputLevelId,
        file_warning_boundary: Option<FileWarningBoundary>,
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
        if let InputLevel::Resident(ResidentTokenRow {
            storage: ResidentTokenStorage::MacroBody(body),
            ..
        }) = level
        {
            let arguments = body.arguments;
            let parameter_count = arguments.map_or(0, |arguments| {
                self.scratch
                    .argument_count(arguments)
                    .expect("live macro argument set")
            });
            self.input.levels.pop_project(|_, _| ());
            self.input.levels.retire_macro_body(parameter_count);
            if let Some(arguments) = arguments {
                self.scratch
                    .release_argument_set(arguments)
                    .map_err(|_| InputRetirementError::AttemptRootInvariant)?;
            }
            return Ok(InputRetirement {
                identity: expected,
                action: InputRetirementAction::TokenListPopped,
                reason: InputRetirementReason::Macro,
                name_class: None,
                source: None,
                file_warning_boundary: None,
                closes_file_frame: false,
            });
        }

        if let InputLevel::Source(source) = level {
            let slot = self.input.levels.source_level_slot(source);
            let name_class = slot.name_class;
            let source_id = slot.cursor.current_backing().id;
            let retirement = slot.retirement;
            let framed = source_level_is_framed(slot);
            self.input.levels.pop_project(|_, _| ());
            self.restore_retained_line_after_source_pop();
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
                file_warning_boundary,
                closes_file_frame: framed,
            });
        }

        let InputLevel::Resident(row) = level else {
            unreachable!("validated input top is source or resident");
        };
        let behavior = row.header.behavior();
        let retirement = row.header.retirement();
        let reason = input_retirement_reason(&behavior, &row.trace());
        if retirement == RetirementBehavior::RetainExhaustedVTemplate {
            if !matches!(behavior, TokenBehavior::VTemplate) {
                return Err(InputRetirementError::NotRetainedVTemplate);
            }
            let retained = self.input.levels.retain_top_v_template();
            assert!(retained, "the inspected top level remains live");
            return Ok(InputRetirement {
                identity: expected,
                action: InputRetirementAction::VTemplateRetained,
                reason,
                name_class: None,
                source: None,
                file_warning_boundary: None,
                closes_file_frame: false,
            });
        }

        let replay = match &row.storage {
            ResidentTokenStorage::Replay { replay, .. } => Some(*replay),
            ResidentTokenStorage::Durable(_)
            | ResidentTokenStorage::Attempt(_)
            | ResidentTokenStorage::MacroArgument(_) => None,
            ResidentTokenStorage::MacroBody(_) => unreachable!("macro body returned above"),
        };
        self.input.levels.pop_project(|_, _| ());
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
            reason,
            name_class: None,
            source: None,
            file_warning_boundary: None,
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
        let level = self.input.levels.last()?;
        if let InputLevel::Resident(ResidentTokenRow {
            header,
            storage: ResidentTokenStorage::MacroBody(body),
        }) = level
        {
            let identity = header.identity();
            let arguments = body.arguments;
            let parameter_count = arguments.map_or(0, |arguments| {
                self.scratch
                    .argument_count(arguments)
                    .expect("final cleanup owns the live argument set")
            });
            self.input.levels.pop_project(|_, _| ());
            self.input.levels.retire_macro_body(parameter_count);
            if let Some(arguments) = arguments {
                self.scratch
                    .release_argument_set(arguments)
                    .expect("final cleanup retires the live argument set");
            }
            return Some(InputRetirement {
                identity,
                action: InputRetirementAction::TokenListPopped,
                reason: InputRetirementReason::Macro,
                name_class: None,
                source: None,
                file_warning_boundary: None,
                closes_file_frame: false,
            });
        }
        if let InputLevel::Source(source) = level {
            let identity = source.identity();
            let slot = self.input.levels.source_level_slot(source);
            let name_class = slot.name_class;
            let source = slot.cursor.current_backing().id;
            let retirement = slot.retirement;
            self.input.levels.pop_project(|_, _| ());
            self.restore_retained_line_after_source_pop();
            let action = source_retirement_action(retirement);
            return Some(InputRetirement {
                identity,
                action,
                reason: InputRetirementReason::Source,
                name_class: Some(name_class),
                source: Some(source),
                file_warning_boundary: None,
                closes_file_frame: false,
            });
        }
        let InputLevel::Resident(row) = level else {
            unreachable!("input top is source or resident");
        };
        let identity = row.header.identity();
        let retirement = row.header.retirement();
        let reason = input_retirement_reason(&row.header.behavior(), &row.trace());
        let replay = match &row.storage {
            ResidentTokenStorage::Replay { replay, .. } => Some(*replay),
            ResidentTokenStorage::Durable(_)
            | ResidentTokenStorage::Attempt(_)
            | ResidentTokenStorage::MacroArgument(_) => None,
            ResidentTokenStorage::MacroBody(_) => unreachable!("macro body returned above"),
        };
        self.input.levels.pop_project(|_, _| ());
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
            reason,
            name_class: None,
            source: None,
            file_warning_boundary: None,
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

    fn restore_retained_line_after_source_pop(&mut self) {
        let line = self
            .input
            .levels
            .iter()
            .rev()
            .find_map(|level| match level {
                InputLevel::Resident(_) => None,
                InputLevel::Source(source) => {
                    let slot = self.input.levels.source_level_slot(source);
                    Some(match slot.name_class {
                        SourceNameClass::File | SourceNameClass::Scantokens(_) => {
                            slot.cursor
                                .line
                                .as_ref()
                                .map_or_else(
                                    || slot.cursor.next_line_number.saturating_sub(1),
                                    |line| line.physical.number(),
                                )
                                .min(i32::MAX as u64) as i32
                        }
                        SourceNameClass::Terminal | SourceNameClass::ReadStream(_) => 0,
                    })
                }
            })
            .unwrap_or(0);
        self.set_retained_file_line_number(line);
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

fn source_level_is_framed<G>(slot: &super::SourceSlot<G>) -> bool {
    match slot.name_class {
        SourceNameClass::File => {
            slot.cursor.backing.name.is_some()
                && slot.cursor.backing.framing == crate::SourceFramingPolicy::Canonical
        }
        SourceNameClass::Scantokens(19) => true,
        SourceNameClass::Terminal
        | SourceNameClass::ReadStream(_)
        | SourceNameClass::Scantokens(_) => false,
    }
}

/// Registers every immutable backing currently owned by one source cursor.
///
/// Source-map provenance is diagnostic state, so registration failure does
/// not change TeX delivery. The cursor bit records the aggregate commit rather
/// than the attempt: a failed registration remains retryable, while the warm
/// path neither clones a descriptor nor allocates.
fn register_pending_source_backings<G>(
    state: &mut tex_state::CommandContext<'_, G>,
    cursor: &mut super::SourceCursor,
) {
    #[cfg(test)]
    record_source_registration_check();
    if !cursor.backing_registered {
        #[cfg(test)]
        record_source_registration_call();
        if state
            .register_source(cursor.backing.id, cursor.backing.source_descriptor())
            .is_ok()
        {
            cursor.backing_registered = true;
        }
    }
    #[cfg(test)]
    record_source_registration_check();
    if !cursor.line_backing_registered
        && let Some(backing) = cursor.line_backing.as_ref()
    {
        #[cfg(test)]
        record_source_registration_call();
        if state
            .register_source(backing.id, backing.source_descriptor())
            .is_ok()
        {
            cursor.line_backing_registered = true;
        }
    }
}

/// Live engine queries used while the one admitted source row is borrowed.
pub(super) struct LiveSourceQueries<'a, 'b, G> {
    pub(super) state: &'a mut tex_state::CommandContext<'b, G>,
    pub(super) create_control_sequences: bool,
}

impl<G> crate::SourceStepQueries for LiveSourceQueries<'_, '_, G> {
    fn catcode(&mut self, code: crate::CharacterCode) -> Catcode {
        self.state.catcode(crate::profile::token_character(code))
    }

    fn firm_up_the_line(&mut self, line: &str) -> Option<super::SourceRegistration> {
        use tex_state::env::banks::IntParam;
        if self.state.int_param(IntParam::PAUSING) <= 0
            || !self.state.interaction_permits_terminal_input()
        {
            return None;
        }
        let prompt = format!("\n{line}=>");
        let replacement = self
            .state
            .input_ln(tex_state::CommandLineSource::Terminal { prompt: &prompt })?;
        if replacement.is_empty() {
            return None;
        }
        Some(super::SourceRegistration::new(
            super::RegisteredSourceKind::Generated,
            replacement.into_bytes(),
        ))
    }
}

impl<G> CompactSourceStepQueries for LiveSourceQueries<'_, '_, G> {
    fn compact_control_word(&mut self, name: &str) -> TokenWord {
        let token = if self.create_control_sequences {
            Token::Cs(self.state.intern_hash_control_sequence(name))
        } else {
            self.state
                .known_control_sequence(name)
                .map_or_else(Token::undefined_control_sequence, Token::Cs)
        };
        TokenWord::pack(token)
    }

    fn compact_source_token(&mut self, source_token: &SourceToken) -> TokenWord {
        let token = match source_token {
            SourceToken::Character { code, catcode, .. } => Token::Char {
                ch: crate::profile::token_character(*code),
                cat: *catcode,
            },
            SourceToken::ControlSequence { name, kind, .. } => match kind {
                SourceControlSequenceKind::Active => Token::Char {
                    ch: crate::profile::token_character(name[0]),
                    cat: Catcode::Active,
                },
                SourceControlSequenceKind::Word
                | SourceControlSequenceKind::Symbol
                | SourceControlSequenceKind::Paragraph
                | SourceControlSequenceKind::Null => {
                    if name.len() == 1 {
                        let mut encoded = [0_u8; 4];
                        let spelling =
                            crate::profile::token_character(name[0]).encode_utf8(&mut encoded);
                        Token::Cs(self.state.intern_control_sequence(spelling))
                    } else {
                        let hashed = *kind == SourceControlSequenceKind::Word && name.len() > 1;
                        name.with_text(|name| {
                            if hashed && !self.create_control_sequences {
                                self.state
                                    .known_control_sequence(name)
                                    .map_or_else(Token::undefined_control_sequence, Token::Cs)
                            } else if hashed {
                                Token::Cs(self.state.intern_hash_control_sequence(name))
                            } else {
                                Token::Cs(self.state.intern_control_sequence(name))
                            }
                        })
                    }
                }
            },
        };
        TokenWord::pack(token)
    }
}

pub(crate) fn input_level_identity<G>(level: &InputLevel<G>) -> InputLevelId {
    match level {
        InputLevel::Source(level) => level.identity(),
        InputLevel::Resident(level) => level.header.identity(),
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
