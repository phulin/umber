//! Ephemeral command-processor orchestration.

pub(crate) mod alignment;
pub(crate) mod expand;
mod next;
pub(crate) use next::RUNAWAY_SCAN_DIAGNOSTIC;
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
use expand::ExpandedFetch;
pub(crate) use expand::ExpansionState;
pub use expand::{
    PrintCommand, append_character_command_text, append_command_token_text,
    append_print_cmd_chr_text, append_print_esc_text, character_command_text, command_token_text,
    print_cmd_chr_text, print_esc_text,
};
pub(crate) use expand::{
    meaning_text, print_cs_text, render_the_value, string_text, token_list_string_text,
};
pub(crate) use next::stored_input_reason;

/// One profile-aware alignment lookahead command and the ownership of its
/// terminal expanded-delivery observation.
///
/// TeX82's `get_x_token` completes that observation before §789 backs the
/// command up.  Umber defers only the expansion-produced terminal long enough
/// to preserve the canonical observation-before-backup order; e-TeX's
/// `get_x_or_protected` terminal path has no expanded observation at all.
#[derive(Debug)]
pub enum AlignmentLookahead {
    Committed(crate::CurrentCommand),
    PendingExpanded(crate::CurrentCommand),
}

impl AlignmentLookahead {
    #[must_use]
    pub const fn command(&self) -> &crate::CurrentCommand {
        match self {
            Self::Committed(command) | Self::PendingExpanded(command) => command,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ReplayCompletionPolicy {
    Consume,
    Surface,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ControlSequenceCreation {
    Forbid,
    Allow,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ExpandedObservationPolicy {
    Commit,
    RawOnly,
    DeferIfExpanded,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FirstCommandPolicy {
    Ordinary,
    MainLoopCharacter,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum AlignmentInterceptionPolicy {
    Scalar,
    Surface,
    None,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ExpandedDeliveryPolicy {
    fetch: ExpandedFetch,
    protected_macros: expand::ProtectedMacroHandling,
    undefined: expand::UndefinedHandling,
    observation: ExpandedObservationPolicy,
    first_command: FirstCommandPolicy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DeliveryMode {
    Raw,
    Expanded(ExpandedDeliveryPolicy),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct DeliveryPolicy {
    mode: DeliveryMode,
    replay_completion: ReplayCompletionPolicy,
    control_sequence_creation: ControlSequenceCreation,
    alignment_interception: AlignmentInterceptionPolicy,
}

#[derive(Debug)]
pub(super) enum DeliveryEvent {
    Command(crate::CurrentCommand),
    PendingExpanded(crate::CurrentCommand),
    ReplayCompleted(crate::CommandReplayEpisode),
    Alignment(AlignmentDeliveryEvent),
}
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
    command_trace_count: usize,
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

    /// TeX82 §578's `find_font_dimen(false)` decision for enquiries.
    #[must_use]
    pub fn font_dimen_readable(&self, font: tex_state::ids::FontId, number: i32) -> bool {
        u32::try_from(number)
            .ok()
            .is_some_and(|number| self.state.font_dimen_readable(font, number))
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
        let mut command_text = String::new();
        expand::append_print_cmd_chr_text(&self.state, command, &mut command_text);
        self.print_command_trace_text(command_text, conditional_suffix);
    }

    /// Prints e-TeX §28.498's merged `\unless` conditional command.
    pub(crate) fn print_unless_command_trace(&mut self, operand: PrintCommand) {
        let conditional_suffix = self.command_trace_conditional_suffix(operand.meaning());
        let mut command = String::new();
        crate::processor::expand::append_print_esc_text(&self.state, "unless", &mut command);
        crate::processor::expand::append_print_cmd_chr_text(&self.state, operand, &mut command);
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
        self.command_trace_count = self.command_trace_count.saturating_add(1);
        // A recoverable error raised inside §366's expansion is synchronous:
        // §82 finishes before §380 fetches and traces the next command. When
        // its World-facing report is queued, queue this later trace behind it
        // as well so the executor preserves that call-stack order.
        if self.command.expanding_deferred_write() || !self.command.semantic_diagnostics.is_empty()
        {
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
        self.command_trace_count = self.command_trace_count.saturating_add(1);
        self.command_trace_mode_prefix.take()
    }

    /// Whether this processor episode crossed a command-trace boundary.
    #[must_use]
    pub const fn command_trace_printed(&self) -> bool {
        self.command_trace_printed
    }

    /// Number of §299 command traces printed during this processor episode.
    ///
    /// Nested operations use this to distinguish a trace they emitted from
    /// an earlier main-control trace in the same borrow.
    #[must_use]
    pub const fn command_trace_count(&self) -> usize {
        self.command_trace_count
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
            command_trace_count: 0,
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
        self.state.untracked_int_param(parameter)
    }

    /// Installs a non-fallible external semantic observer for this bounded
    /// processor episode.
    ///
    /// An episode without an external observer builds no records: every site goes through the
    /// `observe!` macro, which does not evaluate its payload unless the shared
    /// runtime predicate returns true.
    #[must_use]
    pub fn with_observer(mut self, observer: &'a mut dyn CommandObserver) -> Self {
        self.observer = Some(observer);
        self
    }

    /// Whether this episode has an observation consumer active.
    ///
    /// Observation-only: no delivery, expansion, scanner, conditional, or
    /// alignment decision may branch on this, and no committed artifact may
    /// differ by it.
    #[must_use]
    pub(crate) fn is_observed(&self) -> bool {
        self.observer.is_some()
    }

    /// Offers a constructed record to the attached external observer.
    pub(crate) fn observe(&mut self, observation: crate::observation::CommandObservation) {
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
