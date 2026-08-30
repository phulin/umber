//! Executor-facing fact publication from the authoritative command owner.

use super::{CommandSemanticDiagnostic, CommandState};
use crate::AlignmentRecord;
use crate::input::StoredReplayReason;
use crate::processor::AlignmentIdentity;

fn stored_replay_name(reason: StoredReplayReason) -> &'static str {
    match reason {
        StoredReplayReason::EveryPar => "everypar",
        StoredReplayReason::EveryMath => "everymath",
        StoredReplayReason::EveryDisplay => "everydisplay",
        StoredReplayReason::EveryHBox => "everyhbox",
        StoredReplayReason::EveryVBox => "everyvbox",
        StoredReplayReason::EveryJob => "everyjob",
        StoredReplayReason::EveryCr => "everycr",
        StoredReplayReason::OutputRoutine
        | StoredReplayReason::EveryEof
        | StoredReplayReason::Mark
        | StoredReplayReason::Write
        | StoredReplayReason::Discretionary => {
            unreachable!("only executor-requested named lists are queued here")
        }
    }
}

impl<G> CommandState<G> {
    /// Takes the pushes of executor-requested named token lists, in order.
    ///
    /// The executor publishes them with the rest of the operation's committed
    /// records, which is where tex.web's own trace has them: inside the
    /// `new_graf`/`box_end`/`init_math` transition that installed the level.
    #[must_use]
    pub fn publish_named_token_list_pushes(
        &mut self,
        state: &mut tex_state::CommandContext<'_, G>,
        diagnostic_effects: &mut tex_state::diagnostic::DiagnosticEffects,
    ) -> Vec<crate::InputRecord> {
        self.named_token_list_pushes
            .drain(..)
            .map(|(level, reason, tokens)| {
                // TeX82 §§323 and 1145 trace a named token list at
                // `begin_token_list`, while its token_type still identifies
                // the list.  Publishing at the executor/command-state seam
                // preserves that context even when the list has one token
                // and is exhausted by the next main-control delivery.
                if state.int_param(tex_state::env::banks::IntParam::TRACING_MACROS) > 1 {
                    let mut text = String::new();
                    crate::processor::expand_render::append_print_esc_text(
                        state,
                        stored_replay_name(reason),
                        &mut text,
                    );
                    text.push_str("->");
                    for word in state.token_list(tokens) {
                        let token = word.token().expect("durable token word is valid");
                        crate::processor::expand_render::append_token_list_token_text(
                            state, token, &mut text,
                        );
                    }
                    let mut output = state.begin_diagnostic(diagnostic_effects);
                    output.print_nl(&text);
                    output.end(false);
                }
                crate::InputRecord {
                    transition: crate::InputTransition::Push,
                    reason: crate::processor::stored_input_reason(reason),
                    source_name: None,
                    source: None,
                    level: level.0,
                    position: 0,
                }
            })
            .collect()
    }

    /// Transfers semantic diagnostics committed by completed command episodes.
    ///
    /// The executor claims the existing ordered vector allocation inside the
    /// same aggregate operation that ran the episode; command state retains a
    /// fresh empty queue for later work. If a later action suspends or fails,
    /// aggregate rollback restores both this queue and the input cursor from
    /// the pre-step snapshot, so retry reproduces the diagnostic exactly once.
    #[must_use]
    pub fn take_semantic_diagnostics(&mut self) -> Vec<CommandSemanticDiagnostic> {
        std::mem::take(&mut self.semantic_diagnostics)
    }

    /// Returns the committed observation for an executor-applied alignment
    /// begin transition.
    #[must_use]
    pub fn alignment_begin_observation(&self) -> Option<AlignmentRecord> {
        self.alignment
            .active_alignment
            .map(|alignment| AlignmentRecord {
                transition: "begin",
                alignment: Some(alignment.raw()),
                nesting: self.alignment_observation_nesting(),
                align_state: self.alignment.align_state,
                delimiter: None,
                previous_align_state: None,
            })
    }

    /// Reports whether TeX82's alignment-aware `get_next` path is active.
    #[must_use]
    pub fn alignment_scanner_is_active(&self) -> bool {
        self.alignment.active_alignment.is_some()
    }

    /// One-based portable nesting for the active or just-suspended alignment.
    #[must_use]
    pub fn alignment_observation_nesting(&self) -> Option<u32> {
        u32::try_from(self.alignment.align_stack.len())
            .ok()
            .filter(|depth| *depth != 0)
    }

    /// Returns the committed observation for a command-owned outer alignment
    /// suspension.
    #[must_use]
    pub fn alignment_suspend_observation(&self) -> Option<AlignmentRecord> {
        let saved = self.alignment.align_stack.last().copied();
        self.alignment
            .suspended
            .last()
            .map(|suspended| AlignmentRecord {
                transition: "suspend",
                alignment: Some(suspended.alignment.raw()),
                nesting: u32::try_from(self.alignment.suspended.len())
                    .ok()
                    .filter(|depth| *depth != 0),
                align_state: saved.unwrap_or(self.alignment.align_state),
                delimiter: None,
                previous_align_state: None,
            })
    }

    /// Returns the committed observation after a saved outer alignment has
    /// resumed its command-owned delivery state.
    #[must_use]
    pub fn alignment_resume_observation(&self) -> Option<AlignmentRecord> {
        self.alignment
            .active_alignment
            .map(|alignment| AlignmentRecord {
                transition: "resume",
                alignment: Some(alignment.raw()),
                nesting: self.alignment_observation_nesting(),
                align_state: self.alignment.align_state,
                delimiter: None,
                previous_align_state: None,
            })
    }

    /// Returns the committed observation for TeX82 `fin_align` immediately
    /// before it removes the active delivery context.
    #[must_use]
    pub fn alignment_finish_observation(
        &self,
        alignment: AlignmentIdentity,
    ) -> Option<AlignmentRecord> {
        (self.alignment.active_alignment == Some(alignment)).then_some(AlignmentRecord {
            transition: "finish",
            alignment: Some(alignment.raw()),
            nesting: self.alignment_observation_nesting(),
            align_state: self.alignment.align_state,
            delimiter: None,
            previous_align_state: None,
        })
    }

    /// Returns the state transition committed by TeX82's omit-cell branch.
    #[must_use]
    pub fn alignment_omit_cell_observation(
        &self,
        alignment: AlignmentIdentity,
    ) -> Option<AlignmentRecord> {
        let cell = self.alignment.active_cell.as_ref()?;
        (cell.alignment == alignment).then_some(AlignmentRecord {
            transition: "state_change",
            alignment: Some(alignment.raw()),
            nesting: self.alignment_observation_nesting(),
            align_state: self.alignment.align_state,
            delimiter: None,
            previous_align_state: cell.omit_previous_align_state,
        })
    }

    /// Returns the committed input push for a just-installed u-template.
    #[must_use]
    pub fn alignment_u_template_push_observation(
        &self,
        alignment: AlignmentIdentity,
    ) -> Option<crate::InputRecord> {
        let cell = self.alignment.active_cell.as_ref()?;
        (cell.alignment == alignment).then_some(())?;
        cell.u_level.map(|level| crate::InputRecord {
            transition: crate::InputTransition::Push,
            reason: crate::InputReason::AlignmentUTemplate,
            source_name: None,
            source: None,
            level: level.0,
            position: 0,
        })
    }

    /// Returns the command-owned alignment transition paired with the
    /// u-template input push.
    #[must_use]
    pub fn alignment_u_template_push_alignment_observation(
        &self,
        alignment: AlignmentIdentity,
    ) -> Option<crate::AlignmentRecord> {
        self.alignment_u_template_push_observation(alignment)
            .map(|_| crate::AlignmentRecord {
                transition: "u_template_push",
                alignment: Some(alignment.raw()),
                nesting: self.alignment_observation_nesting(),
                align_state: self.alignment.align_state,
                delimiter: None,
                previous_align_state: None,
            })
    }

    /// Returns the committed observation for an executor-selected first cell.
    #[must_use]
    pub fn alignment_cell_begin_observation(&self) -> Option<AlignmentRecord> {
        self.alignment
            .active_cell
            .as_ref()
            .map(|cell| AlignmentRecord {
                transition: "state_change",
                alignment: Some(cell.alignment.raw()),
                nesting: self.alignment_observation_nesting(),
                align_state: self.alignment.align_state,
                delimiter: None,
                previous_align_state: None,
            })
    }

    /// Returns the committed v-template push made after a command-owned
    /// delimiter interception.
    #[must_use]
    pub fn alignment_v_template_push_observation(
        &self,
        alignment: AlignmentIdentity,
    ) -> Option<crate::InputRecord> {
        let cell = self.alignment.active_cell.as_ref()?;
        (cell.alignment == alignment).then_some(())?;
        cell.v_level.map(|level| crate::InputRecord {
            transition: crate::InputTransition::Push,
            reason: crate::InputReason::AlignmentVTemplate,
            source_name: None,
            source: None,
            level: level.0,
            position: 0,
        })
    }

    /// Returns the template lifecycle transition paired with the v-template
    /// input push, without exposing template tokens to the executor.
    #[must_use]
    pub fn alignment_v_template_push_alignment_observation(
        &self,
        alignment: AlignmentIdentity,
    ) -> Option<AlignmentRecord> {
        self.alignment_v_template_push_observation(alignment)
            .map(|_| AlignmentRecord {
                transition: if self
                    .alignment
                    .active_cell
                    .as_ref()
                    .is_some_and(|cell| cell.omit)
                {
                    "omit_template_push"
                } else {
                    "v_template_push"
                },
                alignment: Some(alignment.raw()),
                nesting: self.alignment_observation_nesting(),
                align_state: crate::processor::CELL_ALIGN_STATE,
                delimiter: None,
                previous_align_state: None,
            })
    }

    /// Returns TeX82 §791 `fin_col`'s committed state change.
    #[must_use]
    pub fn alignment_cell_finish_observation(
        &self,
        alignment: AlignmentIdentity,
    ) -> Option<crate::AlignmentRecord> {
        let cell = self.alignment.active_cell.as_ref()?;
        if cell.alignment != alignment || cell.v_level.is_none() {
            return None;
        }
        Some(crate::AlignmentRecord {
            transition: "state_change",
            alignment: Some(alignment.raw()),
            nesting: self.alignment_observation_nesting(),
            align_state: 1_000_000,
            delimiter: None,
            previous_align_state: None,
        })
    }

    /// Takes the command-owned observation published when `fin_col` changes
    /// an exhausted saved tab or span into a row ending.
    pub fn take_alignment_extra_tab_recovery_observation(
        &mut self,
    ) -> Option<crate::AlignmentRecord> {
        let alignment = self.alignment.extra_tab_recovery.take()?;
        Some(crate::AlignmentRecord {
            transition: "extra_tab",
            alignment: Some(alignment.raw()),
            nesting: self.alignment_observation_nesting(),
            align_state: 1_000_000,
            delimiter: None,
            previous_align_state: None,
        })
    }
}
