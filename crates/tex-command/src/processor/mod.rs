//! Ephemeral command-processor orchestration.

pub(crate) mod alignment;
pub(crate) mod expand;
mod next;
mod observe;
pub(crate) mod status;

use tex_state::CommandContext;

use crate::{
    CommandFuel, CommandFuelLedger, CommandHostContext, CommandReplayEpisode, CommandRuntime,
    CommandState, DeliveryStamp,
};

use crate::input::InputLevelId;

use crate::observation::CommandObserver;

pub(crate) use alignment::CELL_ALIGN_STATE;
#[cfg(test)]
pub(crate) use alignment::TOP_LEVEL_ALIGN_STATE;
#[cfg(test)]
pub(crate) use alignment::{ActiveCellDelivery, SuspendedAlignment};
pub use alignment::{
    AlignmentCellDelimiter, AlignmentCellTemplates, AlignmentDelivery, AlignmentDeliveryEvent,
    AlignmentIdentity, AlignmentLifecycleError, AlignmentPreamble, AlignmentRequest,
    AlignmentRequestResult, FinishedAlignmentCell,
};
pub(crate) use alignment::{AlignmentDeliveryAdjustment, AlignmentDeliveryState};
pub(crate) use expand::ExpansionState;
pub use expand::{
    PrintCommand, character_command_text, command_token_text, print_cmd_chr_text, print_esc_text,
};
pub(crate) use expand::{
    meaning_text, print_cs_text, render_the_value, string_text, token_list_string_text,
};
pub(crate) use next::stored_input_reason;
#[cfg(test)]
pub(crate) use status::{
    AbsorbingContext, AlignmentId, AlignmentScanContext, ArgumentBuilderId, ConditionId,
    DefinitionContext, MatchingContext, ScannerWarning, SkippingContext, TokenBuilderId,
};
pub(crate) use status::{ScannerState, ScannerStatus};

/// Borrow-only capability facade for one bounded executor operation.
///
/// The processor owns no semantic or host state and therefore cannot outlive
/// the borrows that construct it. All future raw delivery, expansion,
/// scanners, conditionals, and primitives operate through this single
/// aggregate facade.
#[allow(dead_code)] // later canonical command operations consume every capability
pub struct CommandProcessor<'a> {
    pub(crate) command: &'a mut CommandState,
    runtime: &'a mut CommandRuntime,
    pub(crate) state: CommandContext<'a>,
    pub(crate) host: CommandHostContext<'a>,
    observer: Option<&'a mut dyn CommandObserver>,
    fuel: ProcessorFuel<'a>,
    /// The §53 write scanner registers its replay level here solely to name
    /// that level in detached observation. This is processor-local observer
    /// metadata: raw delivery neither reads replay provenance nor lets this
    /// value affect input semantics.
    immediate_write_retirement: Option<InputLevelId>,
    /// TeX82 §1370 drives deferred write text through `scan_toks` from an
    /// active expanded-command episode. Section 478 consumes `\the` inside
    /// that collector, so the write driver asks the collector to forward the
    /// otherwise internal expanded delivery to its observer.
    pub(crate) observe_write_direct_expansion: bool,
    pending_file_warning_context: Option<(InputLevelId, String)>,
    /// Only the immediately preceding raw delivery may be backed up. This is
    /// processor-local so stamps cannot survive a snapshot or a new episode.
    last_delivery: Option<DeliveryStamp>,
    /// Completion published by raw retirement to the episode-aware expanded
    /// delivery boundary. It is processor-local because retirement itself is
    /// already represented by command state.
    pub(crate) replay_completion: Option<CommandReplayEpisode>,
    /// The non-numeric command that completed the most recent integer scan.
    /// It remains backed up in input; dimension scanning uses the semantic
    /// fact to decide whether that replay is a decimal point or a unit.
    pub(crate) last_integer_terminator: Option<crate::CurrentCommand>,
    next_delivery_sequence: u64,
    /// Set only by canonical outer-validity recovery while a scalar macro
    /// matcher owns `ScannerStatus::Matching`.
    /// tex.web §360 has just ended a `\\read` pseudo-file's only line.
    ///
    /// §360 answers that with `cur_cmd:=0; cur_chr:=0; return` -- a plain
    /// return from `get_next`, with no `check_outer_validity` and no runaway
    /// report. Ordinary end of input is a different thing entirely, so the
    /// two must not share one `None`.
    pub(crate) read_line_ended: bool,
    pub(crate) outer_recovered_while_matching: bool,
    pub(crate) outer_recovered_while_absorbing: bool,
    /// Set only when terminal EOF invokes TeX82's `check_outer_validity`
    /// recovery while a scalar macro matcher is live. The inserted frozen
    /// `\\par` terminates the failed match, but must not become a visible
    /// §394 `back_error` replay token.
    pub(crate) eof_recovered_while_matching: bool,
    /// Web2C's process-local `expand_depth_count` contribution from nested
    /// e-TeX expression primitives. Parentheses use `scan_expr`'s explicit
    /// stack and do not enter this counter.
    pub(crate) expression_depth: u32,
    /// Canonical glue-node identity retained only while an internal glue or
    /// e-TeX expression result remains pointer-identical to its source.
    pub(crate) scanned_glue_identity: Option<tex_state::ids::GlueId>,
    pub(crate) scanned_glue_skip_index: Option<u16>,
    command_trace_mode_prefix: Option<String>,
    command_trace_printed: bool,
}

impl CommandProcessor<'_> {
    /// Captures TeX82 §82's `show_context` while this processor still owns
    /// the live command input cursor.
    #[must_use]
    pub fn error_context(&self) -> String {
        self.command.output_open_context(&self.state)
    }

    /// Returns the immutable command dialect and character mode for this job.
    #[must_use]
    pub const fn profile(&self) -> crate::CommandProfile {
        self.command.profile()
    }

    /// Returns the glue node retained by the most recent glue scan when the
    /// result is still pointer-identical to an internal source quantity.
    #[must_use]
    pub const fn scanned_glue_identity(&self) -> Option<tex_state::ids::GlueId> {
        self.scanned_glue_identity
    }

    #[must_use]
    pub const fn scanned_glue_skip_index(&self) -> Option<u16> {
        self.scanned_glue_skip_index
    }

    /// TeX82 §578's `find_font_dimen` decision for a scanned parameter number.
    ///
    /// §578 resolves `n<=0` to the same `fmem_ptr` scratch cell as an
    /// unusable positive number, and it decides *before* §1253 scans
    /// `=<dimen>` -- which is why the scan asks here rather than letting the
    /// eventual write fail: §579's context is the one at this cursor, not the
    /// one after the whole assignment has been consumed.
    #[must_use]
    pub fn font_dimen_writable(&self, font: tex_state::ids::FontId, number: i32) -> bool {
        u32::try_from(number)
            .ok()
            .filter(|number| *number > 0)
            .is_some_and(|number| self.state.font_dimen_writable(font, number))
    }
}

enum ProcessorFuel<'a> {
    Owned(CommandFuelLedger),
    Shared(&'a mut CommandFuel),
}

impl ProcessorFuel<'_> {
    fn charge(&mut self) -> Result<(), crate::CommandError> {
        match self {
            Self::Owned(fuel) => fuel.fuel_mut().charge(),
            Self::Shared(fuel) => fuel.charge(),
        }
    }
}

impl<'a> CommandProcessor<'a> {
    /// Prints TeX82 §§299/1030's command trace at the fetch boundary.
    ///
    /// This must run before operand scanning because restricted scanners can
    /// report recoverable errors of their own before the command completes.
    pub fn print_command_trace(&mut self, command: PrintCommand) {
        // TeX82 §537 prints an input file's opening before reading its
        // first line; §§299/1030 trace only after the resulting command
        // has been fetched. Host-neutral input queues the framing transition,
        // so commit every transition already reached at this exact fetch
        // boundary before printing the trace. A close reached later during
        // expansion remains queued behind any earlier diagnostic.
        self.command.render_file_framing_events(&mut self.state);
        let conditional_suffix = self.command_trace_conditional_suffix(command.meaning());
        let command = print_cmd_chr_text(&self.state, command);
        self.print_command_trace_text(command, conditional_suffix);
    }

    /// Prints e-TeX §28.498's merged `\unless` conditional command.
    pub(crate) fn print_unless_command_trace(&mut self, operand: PrintCommand) {
        let conditional_suffix = self.command_trace_conditional_suffix(operand.meaning());
        let command = crate::processor::expand::print_esc_text(&self.state, "unless")
            + &print_cmd_chr_text(&self.state, operand);
        self.print_command_trace_text(command, conditional_suffix);
    }

    fn print_command_trace_text(&mut self, command: String, conditional_suffix: String) {
        let mode_prefix = self.command_trace_mode_prefix.take();
        let mut text = String::from("{");
        if let Some(mode_prefix) = mode_prefix.as_deref() {
            text.push_str(mode_prefix);
            text.push_str(": ");
        }
        text.push_str(&command);
        text.push_str(&conditional_suffix);
        text.push('}');
        self.command_trace_printed = true;
        if self.command.expanding_deferred_write() {
            self.command
                .semantic_diagnostics
                .push(crate::CommandSemanticDiagnostic::Trace {
                    text,
                    force_newline: false,
                });
            return;
        }
        let mut output = self.state.begin_diagnostic();
        output.print_nl(&text);
        output.end(false);
    }

    /// Supplies §299's mode prefix for the next command trace in this
    /// processor episode. Expansion may consume it before main control sees
    /// the resulting unexpandable command.
    pub fn set_command_trace_mode_prefix(&mut self, mode_prefix: Option<String>) {
        self.command_trace_mode_prefix = mode_prefix;
    }

    /// Claims TeX82 §299's pending mode prefix for an expansion-side command
    /// trace and records that `shown_mode` must advance in the executor.
    pub(crate) fn claim_command_trace_mode_prefix(&mut self) -> Option<String> {
        self.command_trace_printed = true;
        self.command_trace_mode_prefix.take()
    }

    /// Whether this processor episode crossed a command-trace boundary.
    #[must_use]
    pub const fn command_trace_printed(&self) -> bool {
        self.command_trace_printed
    }

    /// Borrows every ownership domain needed by one command operation.
    #[must_use]
    pub fn new(
        command: &'a mut CommandState,
        runtime: &'a mut CommandRuntime,
        state: CommandContext<'a>,
        host: CommandHostContext<'a>,
    ) -> Self {
        Self {
            command,
            runtime,
            state,
            host,
            observer: None,
            observe_write_direct_expansion: false,
            fuel: ProcessorFuel::Owned(CommandFuelLedger::default()),
            immediate_write_retirement: None,
            pending_file_warning_context: None,
            last_delivery: None,
            replay_completion: None,
            last_integer_terminator: None,
            next_delivery_sequence: 0,
            read_line_ended: false,
            outer_recovered_while_matching: false,
            outer_recovered_while_absorbing: false,
            eof_recovered_while_matching: false,
            expression_depth: 0,
            scanned_glue_identity: None,
            scanned_glue_skip_index: None,
            command_trace_mode_prefix: None,
            command_trace_printed: false,
        }
    }

    /// Lends a run-owned monotonic ledger to this processor episode.
    #[must_use]
    pub fn with_fuel(mut self, fuel: &'a mut CommandFuel) -> Self {
        self.fuel = ProcessorFuel::Shared(fuel);
        self
    }

    pub(crate) fn charge_command_action(&mut self) -> Result<(), crate::CommandError> {
        self.fuel.charge()
    }

    /// Claims command-owned semantic diagnostics in detection order.
    pub fn take_semantic_diagnostics(&mut self) -> Vec<crate::CommandSemanticDiagnostic> {
        self.command.take_semantic_diagnostics()
    }

    /// Reads a live integer parameter while main control selects an
    /// assignment policy for a completed command operation.
    #[must_use]
    pub fn int_param(&self, parameter: tex_state::env::banks::IntParam) -> i32 {
        self.state.int_param(parameter)
    }

    /// Installs a non-fallible semantic observer for this bounded processor
    /// episode.
    ///
    /// An episode without one pays a single `Option` test per observation
    /// site and builds no records: every site goes through the `observe!`
    /// macro, which does not evaluate its payload unless this returns true.
    #[must_use]
    pub fn with_observer(mut self, observer: &'a mut dyn CommandObserver) -> Self {
        self.observer = Some(observer);
        self
    }

    /// Whether this episode publishes observations.
    ///
    /// Observation-only: no delivery, expansion, scanner, conditional, or
    /// alignment decision may branch on this, and no committed artifact may
    /// differ by it.
    #[must_use]
    pub(crate) fn is_observed(&self) -> bool {
        self.observer.is_some() || self.command.paragraph_input_is_recording()
    }

    /// Records a completed typed mutation selected by the replay consumer.
    ///
    /// The command processor remains the sole owner of the observer stream;
    /// replay supplies only a value it has already scanned through this
    /// processor and will apply after the processor borrow ends.
    pub fn observe_typed_mutation(&mut self, target: &'static str, value: impl Into<String>) {
        self.observe(crate::observation::CommandObservation::Mutation(
            crate::observation::MutationRecord {
                target,
                value: value.into(),
                key: None,
                tokens: None,
                global: false,
            },
        ));
    }

    pub(crate) fn observe(&mut self, observation: crate::observation::CommandObservation) {
        self.command.record_paragraph_observation(&observation);
        if let Some(observer) = self.observer.as_deref_mut() {
            observer.committed(observation);
        }
    }

    /// Registers the write-list lifetime established by TeX82 §53's
    /// `write_out`. The scanner owns this classification; raw delivery only
    /// consumes the already-registered observer identity when the level ends.
    pub(crate) fn observe_immediate_write_retirement(&mut self, level: InputLevelId) {
        debug_assert!(self.immediate_write_retirement.is_none());
        self.immediate_write_retirement = Some(level);
    }

    /// Returns whether the just-retired raw level is the §53 write-list level.
    /// This deliberately consumes identity rather than consulting `ReplayTrace`:
    /// trace/provenance explains an input frame but cannot select delivery
    /// observation semantics.
    pub(crate) fn take_immediate_write_retirement(&mut self, level: InputLevelId) -> bool {
        if self.immediate_write_retirement == Some(level) {
            self.immediate_write_retirement = None;
            true
        } else {
            false
        }
    }
}
