//! Alignment interception at the canonical delivery boundary.

use tex_state::meaning::{ExpandablePrimitive, Meaning};

use crate::command::CurrentCommand;
use crate::error::CommandError;
use crate::input::{InputLevel, RetirementBehavior, TokenBehavior};
use crate::observation::{AlignmentRecord, CommandObservation};
use crate::{AlignmentDelivery, AlignmentDeliveryEvent};

use super::alignment::CELL_ALIGN_STATE;
use super::expand::{ExpandedFetch, ProtectedMacroHandling, UndefinedHandling};
use super::{
    AlignmentInterceptionPolicy, CommandProcessor, ExpandedObservationPolicy, FirstCommandPolicy,
    ReplayCompletionPolicy,
};

impl<G> CommandProcessor<'_, '_, G> {
    /// Delivers one expanded command, separating an intercepted alignment
    /// delimiter from ordinary main-control delivery.
    ///
    /// No executor-side classifier is involved: `get_next` has already made
    /// the canonical `align_state` decision before this method observes the
    /// frozen `end_template` meaning.
    ///
    /// This must use the completion-aware raw fetch, not plain `get_next`.
    /// An alignment cell's body can itself contain an executor-owned replay
    /// episode (a math field, math-group/choice branch, or discretionary
    /// part -- for example `\vphantom`'s `\mathchoice` inside an inline `$#$`
    /// cell template). Plain `get_next` silently swallows that episode's
    /// retirement and keeps cascading to whatever real token follows, which
    /// can belong to the *enclosing* context rather than the episode; the
    /// caller (`scan_alignment_delivery_step`) needs `Completed` surfaced so
    /// it can report `ColdOperation::ReplayCompleted` exactly as ordinary
    /// (non-alignment) `scan_step` already does via
    /// `get_x_token_with_replay_completion`.
    ///
    /// `main_loop_active` reports whether `main_control` is parked at TeX82
    /// §1034's `main_loop_lookahead` rather than at §1030's `big_switch`. An
    /// alignment cell body is ordinary `main_control` material, so the same
    /// §1038 rule holds inside it: the first fetch is a bare `get_next`, and
    /// a `letter`/`other_char`/`char_given` never reaches `x_token`.
    ///
    /// It also selects which of §380's two expanded fetches this is, and the
    /// two disagree about the `end_template` that closes a cell's ⟨v_j⟩
    /// template -- see [`ExpandedFetch`].
    pub fn get_x_alignment_delivery(
        &mut self,
        main_loop_active: bool,
    ) -> Result<Option<AlignmentDelivery<G>>, CommandError> {
        let mut destination = None;
        let delivery = self.get_x_alignment_delivery_into(main_loop_active, &mut destination)?;
        Ok(match delivery {
            super::DeliveryStatus::End => None,
            super::DeliveryStatus::Command => Some(AlignmentDelivery::Command(
                destination.expect("command status initializes destination"),
            )),
            super::DeliveryStatus::ReplayCompleted(episode) => {
                Some(AlignmentDelivery::Completed(episode))
            }
            super::DeliveryStatus::AlignmentEndTemplate => Some(AlignmentDelivery::Event(
                AlignmentDeliveryEvent::EndTemplate(
                    destination.expect("alignment status initializes destination"),
                ),
            )),
            super::DeliveryStatus::AlignmentClosingBrace => Some(AlignmentDelivery::Event(
                AlignmentDeliveryEvent::ClosingBrace(
                    destination.expect("alignment status initializes destination"),
                ),
            )),
            super::DeliveryStatus::PendingExpanded => {
                unreachable!("alignment delivery commits terminal observations")
            }
        })
    }
    /// Delivers active-cell input into caller-provided command storage.
    pub fn get_x_alignment_delivery_into(
        &mut self,
        main_loop_active: bool,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<super::DeliveryStatus, CommandError> {
        let fetch = if main_loop_active {
            ExpandedFetch::XToken
        } else {
            ExpandedFetch::GetXToken
        };
        let delivery = self.command_delivery_entry(
            fetch,
            ProtectedMacroHandling::Expand,
            UndefinedHandling::Diagnose,
            ExpandedObservationPolicy::Commit,
            if main_loop_active {
                FirstCommandPolicy::MainLoopCharacter
            } else {
                FirstCommandPolicy::Ordinary
            },
            ReplayCompletionPolicy::Surface,
            AlignmentInterceptionPolicy::Surface,
            destination,
        )?;
        Ok(delivery)
    }
    /// Hands an intercepted delimiter from `end_template` main control back
    /// to canonical input, then starts the active cell's v-template above it.
    /// The delimiter is never classified by an executor-side loop: after the
    /// suffix retires, raw `get_next` sees it again with its original spelling.
    pub fn begin_alignment_v_template(
        &mut self,
        alignment: crate::AlignmentIdentity,
        event: AlignmentDeliveryEvent<G>,
    ) -> Result<(), CommandError> {
        let (saved_delimiter, delimiter_line) = match event {
            AlignmentDeliveryEvent::EndTemplate(delimiter) => {
                if !self.delivery_is_fresh(&delimiter)
                    || !matches!(
                        delimiter.meaning(),
                        tex_state::ResolvedMeaning::Static(Meaning::ExpandablePrimitive(
                            ExpandablePrimitive::EndTemplate
                        ))
                    )
                {
                    return Err(CommandError::StaleDelivery);
                }
                self.invalidate_delivery_freshness();
                (
                    Self::saved_alignment_delimiter(&delimiter)?,
                    delimiter
                        .direct_source_line_number()
                        .unwrap_or_else(|| self.command.current_file_line_number()),
                )
            }
            AlignmentDeliveryEvent::ClosingBrace(_) => {
                return Err(CommandError::input_invariant());
            }
        };
        self.start_alignment_v_template(alignment, saved_delimiter, delimiter_line)
    }
    /// Completes TeX82 §1131's `do_endv` input-stack proof and cell
    /// transition. The processor performs the proof because `loc_field=null`
    /// for stored token lists can only be decided while the immutable token
    /// store is borrowed; the persistent command state deliberately owns no
    /// such store capability.
    pub fn finish_alignment_cell(
        &mut self,
        alignment: crate::AlignmentIdentity,
    ) -> Result<crate::FinishedAlignmentCell, CommandError> {
        let v_level = self
            .command
            .alignment
            .active_v_template_level(alignment)
            .map_err(|_| CommandError::input_invariant())?;
        let mut found = false;
        for level in self.command.input.levels.iter().rev() {
            // TeX82 §1131 walks downward only while `state=token_list` and
            // `loc=null`. A live token either in the v-template itself or in
            // an interposed token-list frame is the canonical interwoven-
            // preamble fatal path, not an internal Rust invariant failure.
            match level {
                level @ InputLevel::Resident(row) => {
                    let cursor = &row.header;
                    if level.stored_is_exhausted() != Some(true) {
                        break;
                    }
                    if cursor.identity() == v_level
                        && matches!(cursor.behavior(), TokenBehavior::VTemplate)
                        && matches!(
                            cursor.retirement(),
                            RetirementBehavior::AwaitingVTemplateRetirement
                        )
                    {
                        found = true;
                        break;
                    }
                }
                InputLevel::Source(_) => break,
            }
        }
        if !found {
            return Err(CommandError::Fatal(crate::FatalError::emergency_stop(
                "(interwoven alignment preambles are not allowed)",
            )));
        }
        // TeX82 §791 performs this independently of §1131's input-stack
        // proof: another preamble may leave the expected exhausted frame in
        // place while its brace sentinel is interwoven with the active cell.
        if self.command.alignment.align_state < 500_000 {
            return Err(CommandError::Fatal(crate::FatalError::emergency_stop(
                "(interwoven alignment preambles are not allowed)",
            )));
        }
        self.command
            .finish_alignment_cell_after_input_proof(alignment)
            .map_err(|_| CommandError::input_invariant())
    }
    fn saved_alignment_delimiter(
        command: &CurrentCommand<G>,
    ) -> Result<crate::AlignmentCellDelimiter, CommandError> {
        match command.alignment_adjustment() {
            crate::processor::AlignmentDeliveryAdjustment::Delimiter(
                crate::processor::alignment::AlignmentDelimiter::Tab,
            ) => Ok(crate::AlignmentCellDelimiter::Tab),
            crate::processor::AlignmentDeliveryAdjustment::Delimiter(
                crate::processor::alignment::AlignmentDelimiter::Span,
            ) => Ok(crate::AlignmentCellDelimiter::Span),
            crate::processor::AlignmentDeliveryAdjustment::Delimiter(
                crate::processor::alignment::AlignmentDelimiter::Cr
                | crate::processor::alignment::AlignmentDelimiter::CrCr,
            ) => Ok(crate::AlignmentCellDelimiter::Row),
            _ => Err(CommandError::input_invariant()),
        }
    }
    /// TeX82 saves this typed outcome in `extra_info(cur_align)` for
    /// `fin_col`; after `do_endv` it selects structural continuation without
    /// re-entering raw delimiter delivery.
    fn start_alignment_v_template(
        &mut self,
        alignment: crate::AlignmentIdentity,
        saved_delimiter: crate::AlignmentCellDelimiter,
        delimiter_line: u32,
    ) -> Result<(), CommandError> {
        self.command
            .begin_alignment_v_template(self.state, alignment, saved_delimiter, delimiter_line)
            .map_err(|_| CommandError::input_invariant())?;
        if let Some(input) = self
            .command
            .alignment_v_template_push_observation(alignment)
        {
            self.observe(CommandObservation::Input(input));
            if let Some(template) = self
                .command
                .alignment_v_template_push_alignment_observation(alignment)
            {
                self.observe(CommandObservation::Alignment(template));
            }
            self.observe(CommandObservation::Alignment(AlignmentRecord {
                transition: "state_change",
                alignment: Some(alignment.raw()),
                nesting: self.command.alignment_observation_nesting(),
                align_state: self.command.alignment.align_state,
                delimiter: None,
                previous_align_state: Some(CELL_ALIGN_STATE),
            }));
        }
        Ok(())
    }
    /// Completes TeX82 §760's v-template insertion when a scalar
    /// `get_x_token` lookahead reaches an active-cell delimiter. `get_next`
    /// has already made the delimiter decision, so this accepts only that
    /// typed adjustment and never reclassifies its spelling.
    pub(crate) fn begin_scalar_alignment_v_template(
        &mut self,
        command: &CurrentCommand<G>,
    ) -> Result<(), CommandError> {
        let alignment = self
            .command
            .alignment
            .active_alignment
            .ok_or(CommandError::input_invariant())?;
        let delimiter = Self::saved_alignment_delimiter(command)?;
        let delimiter_line = command
            .direct_source_line_number()
            .unwrap_or_else(|| self.command.current_file_line_number());
        if !self.delivery_is_fresh(command) {
            return Err(CommandError::StaleDelivery);
        }
        self.invalidate_delivery_freshness();
        self.start_alignment_v_template(alignment, delimiter, delimiter_line)
    }
}
