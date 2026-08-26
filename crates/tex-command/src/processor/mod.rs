//! Ephemeral command-processor orchestration.

pub(crate) mod alignment;
pub(crate) mod expand;
mod next;
pub(crate) use next::RUNAWAY_SCAN_DIAGNOSTIC;
mod observe;
pub(crate) mod status;
#[cfg(test)]
mod tests;

use tex_state::CommandContext;

use crate::{
    CommandError, CommandFuel, CommandFuelLedger, CommandHostContext, CommandState, DeliveryStamp,
};

use crate::input::InputLevelId;

use crate::observation::CommandObserver;

pub(crate) use alignment::CELL_ALIGN_STATE;
pub use alignment::{
    AlignmentCellDelimiter, AlignmentCellTemplates, AlignmentDelivery, AlignmentDeliveryEvent,
    AlignmentIdentity, AlignmentLifecycleError, AlignmentPreamble, AlignmentRequest,
    AlignmentRequestResult, FinishedAlignmentCell, PreparedAlignmentCellTemplates,
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
    meaning_text, print_cs_text, render_the_value, selector_meaning_text, string_text,
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
pub enum AlignmentLookahead<G> {
    Committed(crate::CurrentCommand<G>),
    PendingExpanded(crate::CurrentCommand<G>),
}

impl<G> AlignmentLookahead<G> {
    #[must_use]
    pub const fn command(&self) -> &crate::CurrentCommand<G> {
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
    alignment_interception: AlignmentInterceptionPolicy,
}

/// Compact outcome from a destination-directed command delivery request.
///
/// Command-bearing variants initialize the caller's command destination;
/// non-command variants leave it empty. This keeps the large ephemeral
/// command out of return-value envelopes on the hot path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeliveryStatus {
    End,
    Command,
    PendingExpanded,
    ReplayCompleted(crate::CommandReplayEpisode),
    AlignmentEndTemplate,
    AlignmentClosingBrace,
}
pub(crate) use status::{ScannerState, ScannerStatus};

/// Borrow-only capability facade for one bounded executor operation.
///
/// The processor owns no semantic or host state and therefore cannot outlive
/// the borrows that construct it. All future raw delivery, expansion,
/// scanners, conditionals, and primitives operate through this single
/// aggregate facade.
#[allow(dead_code)] // later canonical command operations consume every capability
pub struct CommandProcessor<'episode, 'admission, G> {
    pub(crate) command: &'episode mut CommandState<G>,
    pub(crate) state: CommandContext<'admission, G>,
    pub(crate) host: CommandHostContext<'episode>,
    observer: Option<&'episode mut dyn CommandObserver>,
    fuel: ProcessorFuel<'episode>,
    diagnostic_effects: &'episode mut tex_state::diagnostic::DiagnosticEffects,
    /// The §53 write scanner registers its replay level here solely to name
    /// that level in detached observation. This is processor-local observer
    /// metadata: raw delivery neither reads replay provenance nor lets this
    /// value affect input semantics.
    immediate_write_retirement: Option<InputLevelId>,
    pending_file_warning_context: Option<(InputLevelId, String)>,
    /// Only the immediately preceding raw delivery may be backed up. This is
    /// processor-local so stamps cannot survive a snapshot or a new episode.
    last_delivery: Option<DeliveryStamp>,
    /// The non-numeric command that completed the most recent integer scan.
    /// It remains backed up in input; dimension scanning uses the semantic
    /// fact to decide whether that replay is a decimal point or a unit.
    pub(crate) last_integer_terminator: Option<crate::CurrentCommand<G>>,
    next_delivery_sequence: u64,
    /// Move-only scanner capability temporarily carried by the exact caller
    /// continuation while a fresh processor borrow performs its retry.
    pub(crate) scanner_resume: Option<crate::execution_scratch::ScannerFrameKey<G>>,
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
    /// pdfTeX section 57's dynamically scoped control-sequence-name flag.
    pub(crate) is_in_csname: bool,
    /// Whether this bounded processor episode runs inside TeX82's active
    /// output routine. Page dimensions remain observable there after §1012
    /// clears the page list and until §991 freezes the next page.
    pub(crate) output_routine_active: bool,
    /// Canonical glue-node identity retained only while an internal glue or
    /// e-TeX expression result remains pointer-identical to its source.
    pub(crate) scanned_glue_identity: Option<tex_state::GlueId<G>>,
    pub(crate) scanned_glue_register: Option<(bool, u16)>,
    /// Nesting of TeX82's artificial deferred-write expansion episode.
    /// This is operational call-stack state, never snapshot state.
    pub(crate) write_expansion_depth: u32,
    command_trace_mode_prefix: Option<String>,
    command_trace_printed: bool,
    command_trace_count: usize,
    /// TeX82 §365's temporary permission belongs exclusively to the next
    /// source-tokenization step. Canonical token and command delivery never
    /// inspect it.
    create_source_control_sequences: bool,
}

/// Opaque observation-order cursor retained when executor preflight suspends
/// a command-processor episode after consuming scanner input.
///
/// It carries no input or semantic owner: those remain in [`CommandState`].
/// Restoring it only keeps delivery sequence metadata continuous when the
/// typed command continuation resumes in a fresh borrow episode.
#[derive(Clone, Copy, Debug)]
pub struct CommandDeliveryCursor(u64);

impl<G> CommandProcessor<'_, '_, G> {
    /// Number of transient matched-argument word reads in this generation.
    ///
    /// The profiling gate asserts that ordinary successful macro matching
    /// does not increase this counter: paragraph and removable-outer-group
    /// decisions consume first-scan facts instead. Tracing, diagnostics, and
    /// external observation may intentionally materialize token text and do
    /// increase it.
    #[cfg(feature = "profiling")]
    pub fn macro_argument_match_word_reads(&self) -> u64 {
        self.command.scratch.match_word_reads()
    }

    /// Captures the next observation delivery sequence for a typed retry.
    #[must_use]
    pub const fn delivery_cursor(&self) -> CommandDeliveryCursor {
        CommandDeliveryCursor(self.next_delivery_sequence)
    }

    /// Restores observation ordering for a typed retry in a fresh borrow.
    pub fn resume_delivery_cursor(&mut self, cursor: CommandDeliveryCursor) {
        self.last_delivery = None;
        self.next_delivery_sequence = cursor.0;
    }

    /// Continues scanning an executor-retained settled command in a fresh
    /// borrow episode.
    ///
    /// `CurrentCommand` fields are private and its delivery stamp was minted
    /// by this command machine, so the executor can move the ephemeral value
    /// across its mutation-free preflight seam without backing up or
    /// redelivering the token. The next scanner delivery remains strictly
    /// later than the resumed stamp.
    pub fn resume_current_command(&mut self, command: &crate::CurrentCommand<G>) {
        let stamp = command.delivery_stamp();
        self.last_delivery = Some(stamp);
        self.next_delivery_sequence = self
            .next_delivery_sequence
            .max(stamp.sequence().wrapping_add(1));
    }

    pub(crate) fn pending_scanner_frame(
        &self,
    ) -> Result<Option<&crate::scan_toks::PendingScanToks<G>>, crate::execution_scratch::ScratchError>
    {
        self.scanner_resume
            .as_ref()
            .map(|key| self.command.scratch.scanner_frame(key))
            .transpose()
    }

    #[must_use]
    pub fn take_scanner_resume(&mut self) -> Option<crate::ScannerFrameKey<G>> {
        self.scanner_resume.take()
    }

    pub fn install_scanner_resume(&mut self, key: Option<crate::ScannerFrameKey<G>>) {
        assert!(
            self.scanner_resume.is_none(),
            "a processor retry accepts exactly one scanner-frame capability"
        );
        self.scanner_resume = key;
    }

    /// Returns the outermost expandable command retained by a nested resource
    /// suspension. The executor uses this exact command, rather than the
    /// command that originally entered settlement, as its typed retry seam.
    #[must_use]
    pub fn pending_expansion_command(&self) -> Option<&crate::CurrentCommand<G>> {
        self.scanner_resume
            .as_ref()
            .and_then(|key| self.command.scratch.expansion_frame(key).ok())
            .map(|pending| &pending.command)
    }

    /// Captures TeX82 §82's `show_context` while this processor still owns
    /// the live command input cursor.
    #[must_use]
    pub fn error_context(&self) -> String {
        self.command.output_open_context(&self.state)
    }

    /// Captures tex.web's live input `line` while a delivered command still
    /// owns its source level. Cold consumers retain this scalar when apply
    /// may run after the exhausted source has been retired.
    #[must_use]
    pub fn current_file_line_number(&self) -> u32 {
        self.command.current_file_line_number()
    }

    /// Returns the immutable command dialect and character mode for this job.
    #[must_use]
    pub fn profile(&self) -> crate::CommandProfile {
        self.command.profile()
    }

    /// Returns the archived expansion coordinate for the innermost live macro
    /// body. Executor material carries this compact value; output,
    /// observation, and continuation boundaries materialize or detach it only
    /// when they actually publish provenance.
    #[must_use]
    pub fn active_macro_origin(&self) -> Option<tex_state::token::OriginId> {
        self.command.parameters.active_invocation_origin()
    }

    /// Returns the glue node retained by the most recent glue scan when the
    /// result is still pointer-identical to an internal source quantity.
    #[must_use]
    pub const fn scanned_glue_identity(&self) -> Option<tex_state::GlueId<G>> {
        self.scanned_glue_identity
    }

    #[must_use]
    pub const fn scanned_glue_register(&self) -> Option<(bool, u16)> {
        self.scanned_glue_register
    }

    /// TeX82 §578's `find_font_dimen` decision for a scanned parameter number.
    ///
    /// §578 resolves `n<=0` to the same `fmem_ptr` scratch cell as an
    /// unusable positive number, and it decides *before* §1253 scans
    /// `=<dimen>` -- which is why the scan asks here rather than letting the
    /// eventual write fail: §579's context is the one at this cursor, not the
    /// one after the whole assignment has been consumed.
    #[must_use]
    pub fn font_dimen_writable(&mut self, font: tex_state::ids::FontId, number: i32) -> bool {
        u32::try_from(number)
            .ok()
            .filter(|number| *number > 0)
            .is_some_and(|number| self.state.font_dimen_writable(font, number))
    }

    /// TeX82 §578's `find_font_dimen(false)` decision for enquiries.
    #[must_use]
    pub fn font_dimen_readable(&mut self, font: tex_state::ids::FontId, number: i32) -> bool {
        u32::try_from(number)
            .ok()
            .is_some_and(|number| self.state.font_dimen_readable(font, number))
    }
}

impl<'episode, 'admission, G> CommandProcessor<'episode, 'admission, G> {
    pub(crate) fn begin_diagnostic(&mut self) -> tex_state::diagnostic::Diagnostic<'_> {
        self.state.begin_diagnostic(self.diagnostic_effects)
    }

    pub(crate) fn has_pending_diagnostic_effects(&self) -> bool {
        !self.diagnostic_effects.is_empty()
    }

    /// Retires this borrow facade and returns its unique admitted state view.
    ///
    /// Callers use this linear handoff when one command episode must perform
    /// a state-only mutation before resuming command delivery under the same
    /// admission. No state owner or processor-local continuation is cloned.
    #[must_use]
    pub fn into_context(self) -> CommandContext<'admission, G> {
        self.state
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

    fn fuel_mut(&mut self) -> &mut CommandFuel {
        match self {
            Self::Owned(fuel) => fuel.fuel_mut(),
            Self::Shared(fuel) => fuel,
        }
    }
}

impl<'episode, 'admission, G> CommandProcessor<'episode, 'admission, G> {
    pub(crate) fn copy_durable_token_list_into_attempt(
        &mut self,
        tokens: Option<tex_state::TokenListId<G>>,
    ) -> Result<crate::AttemptTokenListId, CommandError> {
        let arena = self.command.attempt.arena_mut();
        match tokens {
            Some(tokens) => {
                arena.allocate_token_list(self.state.token_list(tokens).iter().map(|word| {
                    tex_state::token::TracedTokenWord::from_parts(
                        word,
                        tex_state::token::OriginId::UNKNOWN,
                    )
                }))
            }
            None => arena.allocate_token_list([]),
        }
        .map_err(crate::scan_toks::attempt_command_error)
    }

    /// Prints TeX82 §§299/1030's command trace at the fetch boundary.
    ///
    /// This must run before operand scanning because restricted scanners can
    /// report recoverable errors of their own before the command completes.
    pub fn print_command_trace(&mut self, command: PrintCommand<G>) {
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
    pub(crate) fn print_unless_command_trace(&mut self, operand: PrintCommand<G>) {
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
        let mut output = self.begin_diagnostic();
        output.print_nl(&text);
        output.end(false);
        self.publish_diagnostics_before_operand_scan();
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
        command: &'episode mut CommandState<G>,
        state: CommandContext<'admission, G>,
        host: CommandHostContext<'episode>,
        diagnostic_effects: &'episode mut tex_state::diagnostic::DiagnosticEffects,
    ) -> Self {
        Self::from_parts(
            command,
            state,
            host,
            ProcessorFuel::Owned(CommandFuelLedger::default()),
            diagnostic_effects,
            None,
        )
    }

    /// Borrows a session-owned command interpreter without constructing an
    /// intermediate owned fuel ledger or independently selecting evidence.
    ///
    /// Production main control uses this constructor for every short-lived
    /// `Universe` borrow facade. The command state, fuel, and optional
    /// observer all remain owned by the persistent engine session.
    #[must_use]
    pub fn borrowed(
        command: &'episode mut CommandState<G>,
        state: CommandContext<'admission, G>,
        host: CommandHostContext<'episode>,
        fuel: &'episode mut CommandFuel,
        observer: Option<&'episode mut dyn CommandObserver>,
        diagnostic_effects: &'episode mut tex_state::diagnostic::DiagnosticEffects,
    ) -> Self {
        Self::from_parts(
            command,
            state,
            host,
            ProcessorFuel::Shared(fuel),
            diagnostic_effects,
            observer,
        )
    }

    fn from_parts(
        command: &'episode mut CommandState<G>,
        mut state: CommandContext<'admission, G>,
        host: CommandHostContext<'episode>,
        fuel: ProcessorFuel<'episode>,
        diagnostic_effects: &'episode mut tex_state::diagnostic::DiagnosticEffects,
        observer: Option<&'episode mut dyn CommandObserver>,
    ) -> Self {
        command.observe_tracked_dependencies(&mut state);
        Self {
            command,
            state,
            host,
            observer,
            fuel,
            diagnostic_effects,
            immediate_write_retirement: None,
            pending_file_warning_context: None,
            last_delivery: None,
            last_integer_terminator: None,
            next_delivery_sequence: 0,
            scanner_resume: None,
            read_line_ended: false,
            outer_recovered_while_matching: false,
            outer_recovered_while_absorbing: false,
            eof_recovered_while_matching: false,
            expression_depth: 0,
            is_in_csname: false,
            output_routine_active: false,
            scanned_glue_identity: None,
            scanned_glue_register: None,
            write_expansion_depth: 0,
            command_trace_mode_prefix: None,
            command_trace_printed: false,
            command_trace_count: 0,
            create_source_control_sequences: false,
        }
    }

    /// Lends a run-owned monotonic ledger to this processor episode.
    #[must_use]
    pub fn with_fuel(mut self, fuel: &'episode mut CommandFuel) -> Self {
        self.fuel = ProcessorFuel::Shared(fuel);
        self
    }

    /// Supplies TeX82's executor-owned output-routine state to scalar scans.
    ///
    /// The command processor does not own the mode/group stack that makes
    /// this fact authoritative. Main control therefore lends the detached
    /// boolean for this episode instead of duplicating that state here.
    pub fn set_output_routine_active(&mut self, active: bool) {
        self.output_routine_active = active;
    }

    /// Publishes already-complete command diagnostics before operand scans.
    ///
    /// TeX82 §1030 prints a fetched command synchronously before its case arm
    /// scans operands. A later scanner error writes through the live World
    /// reporter, so the detached trace must cross its outer publication seam
    /// first. The collector remains operation-local; this exposes neither
    /// World nor partial-line state to command code.
    pub fn publish_diagnostics_before_operand_scan(&mut self) {
        self.state
            .publish_diagnostic_effects_before_synchronous_print(self.diagnostic_effects);
    }

    pub(crate) fn charge_command_action(&mut self) -> Result<(), crate::CommandError> {
        self.fuel.charge()
    }

    pub(crate) fn record_token_frame(&mut self, scanner: bool) {
        self.fuel.fuel_mut().record_token_frame(scanner);
    }

    pub(crate) fn record_expanded_delivery(&mut self) {
        self.fuel.fuel_mut().record_expanded_delivery();
    }

    pub(crate) fn record_meaning_lookup(&mut self) {
        self.fuel.fuel_mut().record_meaning_lookup();
    }

    pub(crate) fn record_write_expansion(&mut self) {
        self.fuel.fuel_mut().record_write_expansion();
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
    pub fn with_observer(mut self, observer: &'episode mut dyn CommandObserver) -> Self {
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
