//! Retained, host-neutral entry point for TeX command execution.
//!
//! TeX82 §24 (`get_next`) and §25 (`expand`/`get_x_token`) retain input and
//! expansion inside the command machine.  §1030 (`main_control`) consumes the
//! resulting command. Consequently this host owns only immutable resource
//! registration and typed continuation fulfillment; it has no token-delivery
//! or aggregate retry API.

use std::fmt;
use std::sync::Arc;

use tex_command::{
    CommandDeliveryBoundary, CommandDialect, CommandObservation, CommandObserver, CommandProfile,
    ObservedToken, RegisteredSourceKind, SourceRegistration, SourceRegistrationError,
};
use tex_exec::{
    CanonicalStepFailure, CanonicalStepResult, CanonicalStepRunner, CheckpointSink, DiagnosticStep,
    DiagnosticStepResult, MainControl, ResourceFulfillment, ResourceHost, ResourceNeed,
    ResourceOutcome, ResourceWorld,
};
use tex_out::dvi::DviPagePlan;
use tex_state::print::{Printer, Selector};
use tex_state::{FileContent, Universe};

use crate::{RunResult, TexRunStatus};

fn map_step_failure(error: CanonicalStepFailure) -> SessionError {
    match error {
        CanonicalStepFailure::Execution(error) => SessionError::Execution(error),
        CanonicalStepFailure::Checkpoint(error) => SessionError::CommandSummary(error),
    }
}

/// Default bound for a host that repeatedly declines the same typed need.
pub const DEFAULT_NO_PROGRESS_LIMIT: u8 = 8;

/// Typed, host-neutral statistics projected from committed command
/// deliveries. The counters are observational and never participate in TeX
/// execution or user-facing CLI behavior.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ExpansionStats {
    pub token_frame_steps: u64,
    pub provenance_resolutions: u64,
    pub character_tokens: u64,
    pub meaning_lookups: u64,
    pub meaning_cache_hits: u64,
    pub meaning_cache_misses: u64,
    pub literal_spans: u64,
    pub literal_tokens: u64,
    pub segmentation_cache_hits: u64,
    pub segmentation_cache_misses: u64,
    pub builder_appends: u64,
    pub source_text_span_attempts: u64,
    pub source_text_spans: u64,
    pub source_text_tokens: u64,
    pub frame_step_nanos: u64,
    pub provenance_nanos: u64,
    pub classification_meaning_nanos: u64,
    pub builder_append_nanos: u64,
    pub frame_step_timer_samples: u64,
    pub provenance_timer_samples: u64,
    pub classification_meaning_timer_samples: u64,
    pub builder_append_timer_samples: u64,
}

impl ExpansionStats {
    #[must_use]
    pub fn character_fraction(self) -> f64 {
        if self.token_frame_steps == 0 {
            0.0
        } else {
            self.character_tokens as f64 / self.token_frame_steps as f64
        }
    }

    #[must_use]
    pub fn mean_literal_run(self) -> f64 {
        if self.literal_spans == 0 {
            0.0
        } else {
            self.literal_tokens as f64 / self.literal_spans as f64
        }
    }

    #[must_use]
    pub fn mean_source_text_run(self) -> f64 {
        if self.source_text_spans == 0 {
            0.0
        } else {
            self.source_text_tokens as f64 / self.source_text_spans as f64
        }
    }

    #[must_use]
    pub fn attributed_nanos(self) -> u64 {
        self.frame_step_nanos
            .saturating_add(self.provenance_nanos)
            .saturating_add(self.classification_meaning_nanos)
            .saturating_add(self.builder_append_nanos)
    }
}

#[derive(Default)]
struct ExpansionObserver {
    stats: ExpansionStats,
    in_source_span: bool,
    in_literal_span: bool,
}

impl CommandObserver for ExpansionObserver {
    fn committed(&mut self, observation: CommandObservation) {
        let CommandObservation::Command(command) = observation else {
            return;
        };
        if command.boundary != CommandDeliveryBoundary::Expanded {
            return;
        }
        self.stats.token_frame_steps = self.stats.token_frame_steps.saturating_add(1);
        self.stats.meaning_lookups = self.stats.meaning_lookups.saturating_add(1);
        let character = matches!(command.spelling, ObservedToken::Character { .. });
        if character {
            self.stats.character_tokens = self.stats.character_tokens.saturating_add(1);
            self.stats.literal_tokens = self.stats.literal_tokens.saturating_add(1);
            if !self.in_literal_span {
                self.stats.literal_spans = self.stats.literal_spans.saturating_add(1);
            }
        }
        self.in_literal_span = character;
        if command.provenance.has_origin {
            self.stats.provenance_resolutions = self.stats.provenance_resolutions.saturating_add(1);
        }
        let source = command.provenance.source_range.is_some();
        self.stats.source_text_span_attempts =
            self.stats.source_text_span_attempts.saturating_add(1);
        if source {
            self.stats.source_text_tokens = self.stats.source_text_tokens.saturating_add(1);
            if !self.in_source_span {
                self.stats.source_text_spans = self.stats.source_text_spans.saturating_add(1);
            }
        }
        self.in_source_span = source;
    }
}

/// Explicit driver adapter for TeX82's startup `**` line and any §530
/// replacement filename lines.
///
/// Engine crates never read ambient stdin. Native, WebAssembly, and test
/// drivers provide the bounded line source appropriate to their host.
pub trait StartupInput {
    /// Returns the next line after presenting `prompt`, or `None` at EOF.
    fn read_line(&mut self, prompt: &str) -> Option<String>;
}

fn same_run_input_fulfillment(name: &str, content: FileContent) -> ResourceFulfillment {
    ResourceFulfillment::Input {
        name: name.to_owned(),
        // TeX82 §537 prints the name of the file that was actually opened,
        // not the spelling scanned after `\input`. The same-run fallback
        // selects this job-local path directly, matching the `./name`
        // resolved spelling supplied by the loaded-format host for other
        // files beside the job.
        source: SourceRegistration::world(content).with_name(format!("./{name}")),
    }
}

/// Result of driving the retained engine until it either completes or awaits
/// an immutable host response.
#[derive(Debug)]
pub enum SessionState {
    NeedResource(ResourceNeed),
    Complete(Box<RunResult>),
}

/// Failure of the retained host/session protocol.
#[derive(Debug)]
pub enum SessionError {
    RootAlreadyRegistered,
    RootNotRegistered,
    StartupInputExhausted,
    StartupFileUnavailable {
        name: String,
    },
    UnexpectedFulfillment {
        need: Box<ResourceNeed>,
        fulfillment: Box<ResourceFulfillment>,
    },
    NoProgress {
        need: ResourceNeed,
        attempts: u8,
    },
    CooperativeStopRequested,
    SourceRegistration(SourceRegistrationError),
    CommandSummary(tex_command::CommandSummaryError),
    Execution(tex_exec::ExecError),
    FormatDump(tex_exec::FormatDumpError),
    World(tex_state::WorldError),
}

impl fmt::Display for SessionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RootAlreadyRegistered => formatter.write_str("root is already registered"),
            Self::RootNotRegistered => formatter.write_str("root has not been registered"),
            Self::StartupInputExhausted => {
                formatter.write_str("startup terminal input was exhausted")
            }
            Self::StartupFileUnavailable { name } => {
                write!(formatter, "startup input file is unavailable: {name}")
            }
            Self::UnexpectedFulfillment { need, fulfillment } => write!(
                formatter,
                "resource fulfillment {fulfillment:?} does not answer pending need {need:?}"
            ),
            Self::NoProgress { need, attempts } => write!(
                formatter,
                "resource retry made no progress after {attempts} attempts: {need:?}"
            ),
            Self::CooperativeStopRequested => {
                formatter.write_str("session stopped by its cooperative guard")
            }
            Self::SourceRegistration(error) => error.fmt(formatter),
            Self::CommandSummary(error) => error.fmt(formatter),
            Self::Execution(error) => error.fmt(formatter),
            Self::FormatDump(error) => error.fmt(formatter),
            Self::World(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for SessionError {}

impl From<SourceRegistrationError> for SessionError {
    fn from(error: SourceRegistrationError) -> Self {
        Self::SourceRegistration(error)
    }
}

impl From<tex_command::CommandSummaryError> for SessionError {
    fn from(error: tex_command::CommandSummaryError) -> Self {
        Self::CommandSummary(error)
    }
}

impl From<tex_exec::ExecError> for SessionError {
    fn from(error: tex_exec::ExecError) -> Self {
        Self::Execution(error)
    }
}

impl From<tex_state::WorldError> for SessionError {
    fn from(error: tex_state::WorldError) -> Self {
        Self::World(error)
    }
}

/// Standalone retained engine contract.
///
/// This type accepts no raw input cursor, execution-context bridge, or
/// file-resolver dependency. Its only mutable engine input is the shared
/// aggregate `Universe`; all source and resource bytes enter through retained
/// typed registrations.
pub struct EngineSession<'a, G> {
    stores: &'a mut Universe<G>,
    control: MainControl<G>,
    initex: bool,
    loaded_job_framing: bool,
    root_registered: bool,
    root_framing_is_command_owned: bool,
    startup_input_name: Option<String>,
    startup_invocation_line: Option<String>,
    started: bool,
    headline_printed: bool,
    /// Whether TeX82 §1332's engine-termination boundary has committed.
    terminated: bool,
    artifact_cursor: usize,
    effect_cursor: usize,
    terminal_text_cursor: tex_state::EffectPos,
    project_root_body_terminal_text: bool,
    terminal_input_cursor: Option<tex_state::TerminalInputPosition>,
    no_progress_limit: u8,
    mode_transitions: Vec<tex_exec::Mode>,
    output_ledger: tex_exec::OutputLedger,
    retry_materialization: Option<tex_state::MemoryMaterializationCheckpoint>,
}

impl<'a, G> EngineSession<'a, G> {
    /// Advances the retained command machine in analysis mode without
    /// invoking ordinary typesetting main control.
    pub fn diagnostic_expand_step(
        &mut self,
        host: &mut dyn ResourceHost,
    ) -> Result<DiagnosticStep, SessionError> {
        let mut declined: u8 = 0;
        loop {
            match self.control.diagnostic_expand_step(self.stores)? {
                DiagnosticStepResult::Progress(step) => return Ok(step),
                DiagnosticStepResult::Suspended(need) => {
                    declined = if self.answer_need(host, &need)? {
                        0
                    } else {
                        declined.saturating_add(1)
                    };
                    if declined >= self.no_progress_limit {
                        return Err(SessionError::NoProgress {
                            need,
                            attempts: declined,
                        });
                    }
                }
            }
        }
    }
    #[must_use]
    pub fn new(stores: &'a mut Universe<G>, profile: CommandProfile) -> Self {
        Self {
            artifact_cursor: stores.world().artifact_commits().len(),
            effect_cursor: stores.world().effect_records().len(),
            terminal_text_cursor: stores.world().effect_pos(),
            project_root_body_terminal_text: false,
            terminal_input_cursor: None,
            stores,
            control: MainControl::with_profile(profile),
            initex: false,
            loaded_job_framing: false,
            root_registered: false,
            root_framing_is_command_owned: false,
            startup_input_name: None,
            startup_invocation_line: None,
            started: false,
            headline_printed: false,
            terminated: false,
            no_progress_limit: DEFAULT_NO_PROGRESS_LIMIT,
            mode_transitions: vec![tex_exec::Mode::Vertical],
            output_ledger: tex_exec::OutputLedger::default(),
            retry_materialization: None,
        }
    }

    /// Creates a retained TeX82 INITEX session for building a format from
    /// source.
    ///
    /// Unlike [`Self::new`], this enables init-only commands such as
    /// `\patterns` and installs the TeX82 primitive meanings in `stores`.
    #[must_use]
    pub fn tex82_initex(stores: &'a mut Universe<G>) -> Self {
        Self {
            artifact_cursor: stores.world().artifact_commits().len(),
            effect_cursor: stores.world().effect_records().len(),
            terminal_text_cursor: stores.world().effect_pos(),
            project_root_body_terminal_text: false,
            terminal_input_cursor: None,
            control: MainControl::tex82_initex(stores),
            stores,
            initex: true,
            loaded_job_framing: false,
            root_registered: false,
            root_framing_is_command_owned: false,
            startup_input_name: None,
            startup_invocation_line: None,
            started: false,
            headline_printed: false,
            terminated: false,
            no_progress_limit: DEFAULT_NO_PROGRESS_LIMIT,
            mode_transitions: vec![tex_exec::Mode::Vertical],
            output_ledger: tex_exec::OutputLedger::default(),
            retry_materialization: None,
        }
    }

    /// Creates an INITEX session after the composed engine mode has installed
    /// its fresh primitive profile into `stores`.
    #[must_use]
    pub fn prepared_initex(stores: &'a mut Universe<G>, profile: CommandProfile) -> Self {
        Self {
            artifact_cursor: stores.world().artifact_commits().len(),
            effect_cursor: stores.world().effect_records().len(),
            terminal_text_cursor: stores.world().effect_pos(),
            project_root_body_terminal_text: false,
            terminal_input_cursor: None,
            control: MainControl::prepared_initex(profile),
            stores,
            initex: true,
            loaded_job_framing: false,
            root_registered: false,
            root_framing_is_command_owned: false,
            startup_input_name: None,
            startup_invocation_line: None,
            started: false,
            headline_printed: false,
            terminated: false,
            no_progress_limit: DEFAULT_NO_PROGRESS_LIMIT,
            mode_transitions: vec![tex_exec::Mode::Vertical],
            output_ledger: tex_exec::OutputLedger::default(),
            retry_materialization: None,
        }
    }

    #[must_use]
    pub fn command_profile(&self) -> CommandProfile {
        self.control.command_profile()
    }

    /// Configures a positive finite command-work limit.
    pub fn set_fuel_limit(&mut self, limit: u64) -> Result<(), tex_command::CommandFuelLimitError> {
        self.control.set_fuel_limit(limit)
    }

    /// Declares the immutable format identity used to frame a loaded job.
    pub fn set_preloaded_format(&mut self, format: tex_exec::PreloadedFormat) {
        self.control.set_preloaded_format(format);
        self.loaded_job_framing = true;
    }

    /// Selects the engine binary identity used by loaded-job startup framing.
    pub fn set_engine_binary(&mut self, binary: tex_exec::EngineBinaryIdentity) {
        self.control.set_engine_binary(binary);
    }

    #[must_use]
    pub const fn fuel_limit(&self) -> u64 {
        self.control.fuel_limit()
    }

    #[must_use]
    pub const fn fuel_burned(&self) -> u64 {
        self.control.fuel_burned()
    }

    /// Reports why production command episodes returned to this retained
    /// session. The counters are operational and never enter checkpoints.
    #[must_use]
    pub const fn episode_telemetry(&self) -> tex_exec::EpisodeTelemetry {
        self.control.episode_telemetry()
    }

    #[must_use]
    pub fn stores(&self) -> &Universe<G> {
        self.stores
    }

    /// Reports the live execution mode at the top of the mode nest.
    ///
    /// This is diagnostic-only: it exists so a host driving the retained
    /// session (for example the `first_failure_locator` example) can
    /// attribute a suspension or error to the mode active when it occurred.
    #[must_use]
    pub fn current_mode(&self) -> tex_exec::Mode {
        self.control.current_mode()
    }

    /// Renders one execution error through this session's admitted
    /// diagnostic and provenance boundary.
    pub fn format_execution_error(&mut self, error: &tex_exec::ExecError) -> String {
        let mut context = self
            .stores
            .command_context()
            .expect("retained session has a live generation");
        error.format_with_provenance(&mut context)
    }

    /// Bounds only consecutive host declines for one suspension epoch.
    pub fn set_no_progress_limit(&mut self, limit: u8) {
        self.no_progress_limit = limit.max(1);
    }

    /// Projects [`RunResult::terminal_text`] to the authored root body while
    /// retaining canonical startup/final-cleanup framing in `effects`.
    pub(crate) fn project_terminal_text_to_root_body(&mut self) {
        self.project_root_body_terminal_text = true;
    }

    /// Registers the sole World- or host-selected complete job before any
    /// canonical operation.
    ///
    /// The registration is transferred unchanged so its World input-record
    /// identity remains available to source provenance. Job naming is driver
    /// policy, deliberately separate from source acquisition.
    pub fn register_retained_root(
        &mut self,
        startup_input_name: &str,
        source: SourceRegistration,
    ) -> Result<tex_state::SourceId, SessionError> {
        self.register_retained_root_with_invocation(startup_input_name, startup_input_name, source)
    }

    /// Registers a complete job while retaining the §534 invocation line.
    ///
    /// The line may include driver syntax such as web2c's `&format`; job-name
    /// derivation and source identity continue to use `startup_input_name`.
    pub fn register_retained_root_with_invocation(
        &mut self,
        startup_input_name: &str,
        startup_invocation_line: &str,
        source: SourceRegistration,
    ) -> Result<tex_state::SourceId, SessionError> {
        self.register_retained_root_with_policy(
            startup_input_name,
            startup_invocation_line,
            source,
            tex_exec::RootCompletionPolicy::RequireTeXEnd,
        )
    }

    /// Registers an authored fragment whose root EOF is the host boundary.
    ///
    /// This does not synthesize `\end` or run TeX's final cleanup. Complete
    /// jobs must use [`Self::register_retained_root_with_invocation`] and
    /// provide their own canonical terminator.
    pub fn register_retained_fragment_with_invocation(
        &mut self,
        startup_input_name: &str,
        startup_invocation_line: &str,
        source: SourceRegistration,
    ) -> Result<tex_state::SourceId, SessionError> {
        self.register_retained_root_with_policy(
            startup_input_name,
            startup_invocation_line,
            source,
            tex_exec::RootCompletionPolicy::StopAtRootEof,
        )
    }

    fn register_retained_root_with_policy(
        &mut self,
        startup_input_name: &str,
        startup_invocation_line: &str,
        source: SourceRegistration,
        completion: tex_exec::RootCompletionPolicy,
    ) -> Result<tex_state::SourceId, SessionError> {
        if self.root_registered {
            return Err(SessionError::RootAlreadyRegistered);
        }
        self.control
            .capabilities_mut()
            .set_startup_job_name(startup_input_name);
        self.root_framing_is_command_owned = source.name().is_some();
        self.control.set_root_completion_policy(completion);
        if completion == tex_exec::RootCompletionPolicy::StopAtRootEof {
            self.terminal_input_cursor = Some(self.stores.capture_terminal_input_position());
        }
        self.control.record_retained_startup_strings(
            self.stores,
            startup_input_name,
            source.name(),
        );
        let source = self.control.register_root_source(source)?;
        self.root_registered = true;
        self.startup_input_name = Some(startup_input_name.to_owned());
        self.startup_invocation_line = Some(startup_invocation_line.to_owned());
        Ok(source)
    }

    /// Registers a root selected through the active World without rebuilding
    /// its provenance from bytes.
    pub fn register_world_root(
        &mut self,
        startup_input_name: &str,
        content: FileContent,
    ) -> Result<tex_state::SourceId, SessionError> {
        self.register_retained_root(startup_input_name, SourceRegistration::world(content))
    }

    /// Registers a complete authored in-memory job.
    ///
    /// This is intentionally not a World-input adapter: selected World roots
    /// must use [`Self::register_world_root`] or
    /// [`Self::register_retained_root`] so their input-record identity is not
    /// discarded. The source must provide its own canonical terminator.
    pub fn register_authored_job(
        &mut self,
        startup_input_name: &str,
        bytes: Arc<[u8]>,
    ) -> Result<tex_state::SourceId, SessionError> {
        self.register_retained_root(
            startup_input_name,
            SourceRegistration::new(RegisteredSourceKind::Generated, bytes),
        )
    }

    /// Registers an in-memory fragment which completes at its root EOF.
    pub fn register_authored_fragment(
        &mut self,
        startup_input_name: &str,
        bytes: Arc<[u8]>,
    ) -> Result<tex_state::SourceId, SessionError> {
        self.register_retained_fragment_with_invocation(
            startup_input_name,
            startup_input_name,
            SourceRegistration::new(RegisteredSourceKind::Generated, bytes),
        )
    }

    /// Acquires TeX's startup filename and immutable root through explicit
    /// driver adapters, including §530's interactive replacement loop.
    ///
    /// The resource host remains the sole owner of lookup policy. A missing
    /// name is retried only in scroll/error-stop modes; batch/nonstop return
    /// the canonical fatal file outcome without consulting another line.
    pub fn acquire_startup_root(
        &mut self,
        input: &mut dyn StartupInput,
        host: &mut dyn ResourceHost,
    ) -> Result<tex_state::SourceId, SessionError> {
        if self.root_registered {
            return Err(SessionError::RootAlreadyRegistered);
        }
        self.print_startup_headline();
        let mut prompt = "**";
        loop {
            let line = input
                .read_line(prompt)
                .ok_or(SessionError::StartupInputExhausted)?;
            let name = startup_file_name(&line);
            let need = ResourceNeed::Input {
                name: name.clone(),
                original_name: name.clone(),
            };
            let outcome = {
                let mut world = ResourceWorld::new(self.stores);
                host.fulfill(&mut world, &need)
            };
            if let ResourceOutcome::Fulfilled(fulfillment) = outcome {
                let ResourceFulfillment::Input {
                    name: fulfilled_name,
                    source,
                } = fulfillment
                else {
                    return Err(SessionError::UnexpectedFulfillment {
                        need: Box::new(need),
                        fulfillment: Box::new(fulfillment),
                    });
                };
                if fulfilled_name != name {
                    return Err(SessionError::UnexpectedFulfillment {
                        need: Box::new(need),
                        fulfillment: Box::new(ResourceFulfillment::Input {
                            name: fulfilled_name,
                            source,
                        }),
                    });
                }
                self.control
                    .begin_job_after_terminal_headline_for_input(self.stores, &line, &name);
                self.control.capabilities_mut().set_startup_job_name(&name);
                let id = self
                    .control
                    .register_startup_root_source(self.stores, source, &name)?;
                self.root_registered = true;
                self.root_framing_is_command_owned = true;
                self.startup_invocation_line = Some(line);
                self.startup_input_name = Some(name);
                return Ok(id);
            }
            let permits_terminal_input = self
                .stores
                .command_context()
                .map(|context| context.interaction_permits_terminal_input())
                .unwrap_or(false);
            if !permits_terminal_input {
                let mut report = self.stores.print_err("Emergency stop");
                report.help(&["*** (job aborted, file error in nonstop mode)"]);
                report.succumb();
                return Err(SessionError::StartupFileUnavailable { name });
            }
            Printer::new(self.stores, Selector::TermOnly)
                .print_nl(&format!("! I can't find file `{name}'."))
                .print_ln()
                .print("Please type another input file name: ");
            prompt = "";
        }
    }

    /// Drives committed aggregate operations until completion or a typed
    /// suspension. Checkpoints are published solely from committed receipts.
    pub fn advance_until_waiting(
        &mut self,
        checkpoints: &mut dyn CheckpointSink<G>,
    ) -> Result<SessionState, SessionError> {
        self.ensure_started(checkpoints)?;
        self.advance_inner(checkpoints, None)
    }

    fn ensure_started(
        &mut self,
        checkpoints: &mut dyn CheckpointSink<G>,
    ) -> Result<(), SessionError> {
        if !self.root_registered {
            return Err(SessionError::RootNotRegistered);
        }
        if !self.started {
            self.started = true;
            if self.initex {
                // tex.web §§61/241 print the INITEX process headline before
                // `open_log_file`, whose one-shot job initialization refreshes
                // the volatile clock cells before the first input line.
                self.print_startup_headline();
                let input = self
                    .startup_input_name
                    .as_deref()
                    .expect("a started session has a root");
                let invocation = self
                    .startup_invocation_line
                    .as_deref()
                    .expect("a started session has a startup invocation");
                self.control.begin_job_after_terminal_headline_for_input(
                    self.stores,
                    invocation,
                    input,
                );
            } else if self.loaded_job_framing {
                let input = self
                    .startup_input_name
                    .as_deref()
                    .expect("a started session has a root");
                let invocation = self
                    .startup_invocation_line
                    .as_deref()
                    .expect("a started session has a startup invocation");
                self.control
                    .begin_job_for_input(self.stores, invocation, input);
            }
            self.print_startup_headline();
            self.print_startup_input_opening();
            if self.project_root_body_terminal_text && !self.root_framing_is_command_owned {
                self.terminal_text_cursor = self.stores.world().effect_pos();
            }
            self.output_ledger
                .commit_job_start(&mut self.control, self.stores, checkpoints)?;
        }
        Ok(())
    }

    /// Prints TeX82 §1332's process headline before the first command.
    ///
    /// This is deliberately terminal-only. The transcript is not open at
    /// this boundary; §534 later catches it up with a dated banner after the
    /// startup input has established the job name.
    fn print_startup_headline(&mut self) {
        if !self.initex || std::mem::replace(&mut self.headline_printed, true) {
            return;
        }
        let banner = match self.command_profile().dialect() {
            CommandDialect::Tex82 => "This is TeX, Version 3.141592653 (TeX Live 2026) (INITEX)",
            CommandDialect::Etex26 => {
                "This is e-TeX, Version 3.141592653-2.6 (TeX Live 2026) (INITEX)"
            }
            CommandDialect::Pdftex14029 => {
                "This is pdfTeX, Version 3.141592653-2.6-1.40.29 (TeX Live 2026) (INITEX)"
            }
        };
        Printer::new(self.stores, Selector::TermOnly)
            .print(banner)
            .print_ln();
    }

    /// Prints TeX82 §537's successful `start_input` filename opening.
    ///
    /// The root has already been selected and opened before the retained
    /// session starts, but the selector-visible framing still belongs at the
    /// same boundary as TeX's successful file open. The transcript is not
    /// open yet, so this write is terminal-only in both INITEX and
    /// format-loaded sessions.
    fn print_startup_input_opening(&mut self) {
        if self.root_framing_is_command_owned {
            return;
        }
        let startup_input_name = self
            .startup_input_name
            .as_deref()
            .expect("a started session has a registered root");
        if self.initex || self.loaded_job_framing {
            self.control
                .open_startup_input_after_log(self.stores, startup_input_name);
        } else {
            self.control
                .open_startup_input(self.stores, startup_input_name);
        }
    }

    /// Observed variant of [`Self::advance_until_waiting`].
    ///
    /// This drives the same retained production control and differs only by
    /// forwarding its committed semantic observations to a non-fallible
    /// observer. Resource suspensions remain atomic and therefore publish no
    /// observations until their retry commits.
    pub fn advance_until_waiting_with_observer(
        &mut self,
        checkpoints: &mut dyn CheckpointSink<G>,
        observer: &mut dyn tex_command::CommandObserver,
    ) -> Result<SessionState, SessionError> {
        self.ensure_started(checkpoints)?;
        self.advance_inner(checkpoints, Some(observer))
    }

    fn advance_inner(
        &mut self,
        checkpoints: &mut dyn CheckpointSink<G>,
        mut observer: Option<&mut dyn tex_command::CommandObserver>,
    ) -> Result<SessionState, SessionError> {
        if self.terminated {
            return self.finish();
        }
        loop {
            let mut runner =
                CanonicalStepRunner::new(&mut self.control, self.stores, &mut self.output_ledger);
            let result = match observer.as_deref_mut() {
                Some(observer) => {
                    runner.step_with_observer(checkpoints, &tex_exec::Cancellation::new(), observer)
                }
                None => runner.step_completing_fatal(checkpoints, &tex_exec::Cancellation::new()),
            };
            if let Some(checkpoint) = self.retry_materialization.take() {
                let reconciled = self
                    .stores
                    .world_mut()
                    .reconcile_memory_retry_materialization(&checkpoint);
                if !reconciled {
                    self.retry_materialization = Some(checkpoint);
                }
            }
            match result {
                CanonicalStepResult::ResourceNeed(need) => {
                    self.retry_materialization =
                        self.stores.world().memory_materialization_checkpoint();
                    return Ok(SessionState::NeedResource(need));
                }
                CanonicalStepResult::Progress(_) | CanonicalStepResult::Committed(_) => {
                    self.record_current_mode();
                    if checkpoints.stop_requested() {
                        return Err(SessionError::CooperativeStopRequested);
                    }
                }
                CanonicalStepResult::Completed(step) => {
                    self.record_current_mode();
                    if matches!(step, tex_exec::MainControlStep::EndOfInput)
                        && let Some(observer) = observer.as_deref_mut()
                    {
                        observer.committed(engine_termination_observation());
                    }
                    self.terminated = true;
                    return self.finish();
                }
                CanonicalStepResult::Failed(error) => return Err(map_step_failure(error)),
            }
        }
    }

    /// Installs an answer only when it exactly matches the active suspension.
    pub fn fulfill(
        &mut self,
        need: &ResourceNeed,
        fulfillment: ResourceFulfillment,
    ) -> Result<(), SessionError> {
        self.output_ledger
            .fulfill(&mut self.control, need, fulfillment)
            .map_err(|fulfillment| SessionError::UnexpectedFulfillment {
                need: Box::new(need.clone()),
                fulfillment,
            })
    }

    fn mark_unavailable(&mut self, need: &ResourceNeed) {
        self.output_ledger
            .mark_unavailable(&mut self.control, need, true);
    }

    /// Runs the engine using host policy only for typed immutable
    /// needs. Repeated declines or definitive absences are bounded; successful
    /// fulfillment resets the no-progress epoch because the next operation is
    /// replayed atomically.
    pub fn run(
        &mut self,
        host: &mut dyn ResourceHost,
        checkpoints: &mut dyn CheckpointSink<G>,
    ) -> Result<RunResult, SessionError> {
        self.run_inner(host, checkpoints, None)
    }

    /// Runs through the same retained resource protocol while projecting
    /// committed expanded-delivery and provenance counters for profiling.
    pub fn run_with_expansion_stats(
        &mut self,
        host: &mut dyn ResourceHost,
        checkpoints: &mut dyn CheckpointSink<G>,
    ) -> Result<(RunResult, ExpansionStats), SessionError> {
        let mut observer = ExpansionObserver::default();
        let result = self.run_with_observer(host, checkpoints, &mut observer)?;
        Ok((result, observer.stats))
    }

    /// Observed variant of [`Self::run`] over the same production session.
    pub fn run_with_observer(
        &mut self,
        host: &mut dyn ResourceHost,
        checkpoints: &mut dyn CheckpointSink<G>,
        observer: &mut dyn tex_command::CommandObserver,
    ) -> Result<RunResult, SessionError> {
        self.run_inner(host, checkpoints, Some(observer))
    }

    fn run_inner(
        &mut self,
        host: &mut dyn ResourceHost,
        checkpoints: &mut dyn CheckpointSink<G>,
        mut observer: Option<&mut dyn tex_command::CommandObserver>,
    ) -> Result<RunResult, SessionError> {
        let mut declined: u8 = 0;
        loop {
            let state = match observer.as_deref_mut() {
                Some(observer) => {
                    self.advance_until_waiting_with_observer(checkpoints, observer)?
                }
                None => self.advance_until_waiting(checkpoints)?,
            };
            match state {
                SessionState::Complete(result) => return Ok(*result),
                SessionState::NeedResource(need) => {
                    declined = if self.answer_need(host, &need)? {
                        0
                    } else {
                        declined.saturating_add(1)
                    };
                    if declined >= self.no_progress_limit {
                        return Err(SessionError::NoProgress {
                            need,
                            attempts: declined,
                        });
                    }
                }
            }
        }
    }

    fn answer_need(
        &mut self,
        host: &mut dyn ResourceHost,
        need: &ResourceNeed,
    ) -> Result<bool, SessionError> {
        let outcome = {
            let mut world = ResourceWorld::new(self.stores);
            host.fulfill(&mut world, need)
        };
        if let ResourceOutcome::Fulfilled(fulfillment) = outcome {
            self.fulfill(need, fulfillment)?;
            return Ok(true);
        }
        if let Some(fulfillment) = self.same_run_output(need) {
            self.fulfill(need, fulfillment)?;
            return Ok(true);
        }
        if matches!(outcome, ResourceOutcome::Unavailable) {
            self.mark_unavailable(need);
        }
        Ok(false)
    }

    /// Resolves an exact input name from output already committed by this
    /// retained run when host search policy declines it.
    ///
    /// TeX82 §§1328, 1374 close output streams before later input opens use
    /// the resulting file. The active World is the owner of those committed
    /// effects, so this fallback must remain inside the session instead of
    /// requiring every host search policy to mirror relative output paths.
    fn same_run_output(&mut self, need: &ResourceNeed) -> Option<ResourceFulfillment> {
        let name = match need {
            ResourceNeed::Input { name, .. } => name,
            ResourceNeed::InputProbe { request } => &request.name,
            ResourceNeed::Font { .. } | ResourceNeed::PdfImage { .. } => {
                return None;
            }
        };
        let content = self
            .stores
            .world_mut()
            .read_same_run_output_file(name)
            .ok()
            .flatten()?;
        Some(match need {
            ResourceNeed::Input { .. } => same_run_input_fulfillment(name, content),
            ResourceNeed::InputProbe { request } => {
                ResourceFulfillment::world_input_probe(request.clone(), content)
            }
            ResourceNeed::Font { .. } | ResourceNeed::PdfImage { .. } => {
                unreachable!("non-file resources returned above")
            }
        })
    }

    fn record_current_mode(&mut self) {
        let mode = self.control.current_mode();
        if self.mode_transitions.last() != Some(&mode) {
            self.mode_transitions.push(mode);
        }
    }

    fn finish(&mut self) -> Result<SessionState, SessionError> {
        self.control.finalize_pdf_navigation(self.stores);
        let receipts = self.control.take_prepared_dvi_pages();
        let dvi_pages = receipts
            .iter()
            .cloned()
            .map(tex_exec::PreparedDviPage::into_plan)
            .collect::<Vec<DviPagePlan>>();
        let dvi_output = if !self.loaded_job_framing || dvi_pages.is_empty() {
            None
        } else {
            let dvi = crate::dvi_from_page_plans(&dvi_pages).map_err(|error| {
                SessionError::Execution(tex_exec::ExecError::InvalidShipoutArtifact(format!(
                    "DVI serialization failed before job framing: {error}"
                )))
            })?;
            Some(tex_exec::DviJobOutput {
                file_name: self.control.dvi_output_name(self.stores)?,
                byte_len: dvi.len() as u64,
            })
        };
        if self.loaded_job_framing {
            self.control.finish_job(self.stores, dvi_output, None);
        }
        let commits = self.stores.world().artifact_commits();
        let artifact_start = self.artifact_cursor.min(commits.len());
        let artifacts = commits[artifact_start..].to_vec();
        let emits_dvi = !self
            .control
            .command_profile()
            .capabilities()
            .supports_pdftex();
        if (emits_dvi && receipts.len() != artifacts.len())
            || receipts
                .iter()
                .zip(&artifacts)
                .any(|(receipt, hash)| receipt.hash() != *hash)
        {
            return Err(SessionError::Execution(
                tex_exec::ExecError::InvalidShipoutArtifact(
                    "DVI receipts are not aligned with committed artifacts".into(),
                ),
            ));
        }
        let committed_artifacts =
            self.stores.world().committed_artifacts()[artifact_start..commits.len()].to_vec();
        let effect_records = self.stores.world().effect_records();
        let effect_start = self.effect_cursor.min(effect_records.len());
        let effects = effect_records[effect_start..].to_vec();
        let terminal_text = if self.project_root_body_terminal_text {
            let effect_end = self.stores.world().effect_pos().raw();
            let effect_base = effect_end.saturating_sub(effect_records.len() as u64);
            let index = |position: tex_state::EffectPos| {
                usize::try_from(position.raw().saturating_sub(effect_base))
                    .unwrap_or(usize::MAX)
                    .min(effect_records.len())
            };
            let terminal_start = index(self.terminal_text_cursor);
            let terminal_end = index(
                self.control
                    .job_body_effect_end()
                    .unwrap_or_else(|| self.stores.world().effect_pos()),
            )
            .max(terminal_start);
            crate::terminal_text_from_effects(&effect_records[terminal_start..terminal_end])
        } else {
            crate::uncommitted_terminal_text(self.stores)
        };
        self.artifact_cursor = commits.len();
        self.effect_cursor = effect_records.len();
        if let Some(position) = self.terminal_input_cursor.take() {
            self.stores.restore_terminal_input_position(position)?;
        }
        let format_dump = self
            .control
            .take_format_dump(self.stores)
            .map_err(SessionError::FormatDump)?;
        Ok(SessionState::Complete(Box::new(RunResult {
            terminal_text,
            status: TexRunStatus::from_error_history(self.stores.world().error_channel().history()),
            mode_transitions: self.mode_transitions.clone(),
            fatal: self.control.fatal_error(),
            artifacts,
            dvi_pages,
            committed_artifacts,
            effects,
            format_dump,
        })))
    }
}

fn engine_termination_observation() -> tex_command::CommandObservation {
    tex_command::CommandObservation::Effect(tex_command::EffectRecord {
        kind: tex_command::ObservationEffectKind::Terminate,
        channel: "engine".into(),
        value: tex_command::ObservationValue::None,
        source: None,
    })
}

fn startup_file_name(line: &str) -> String {
    let supplied = line.trim().split_ascii_whitespace().next().unwrap_or("");
    if supplied
        .rsplit(['/', '\\'])
        .next()
        .is_some_and(|leaf| leaf.contains('.'))
    {
        supplied.to_owned()
    } else {
        format!("{supplied}.tex")
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::path::Path;

    use super::*;
    use tex_command::{CommandObservation, CommandObserver, FontResource, PdfImageResource};
    use tex_exec::{EngineBoundary, canonical_font_resource_path};
    use tex_state::World;

    const CMR10: &[u8] = include_bytes!("../../tex-fonts/tests/fixtures/cm/cmr10.tfm");

    fn packed_episode_font() -> tex_fonts::LoadedFont {
        let mut characters = vec![None; 256];
        characters[usize::from(b'A')] = Some(tex_fonts::CharMetrics {
            width: tex_state::scaled::Scaled::from_raw(500),
            height: tex_state::scaled::Scaled::from_raw(300),
            depth: tex_state::scaled::Scaled::from_raw(100),
            italic_correction: tex_state::scaled::Scaled::from_raw(0),
            tag: tex_fonts::MetricCharTag::None,
        });
        tex_fonts::LoadedFont::new(
            "batchfont",
            "batchfont.tfm",
            tex_out::ContentHash::from_bytes(b"batchfont").bytes(),
            0x64b2_0012,
            tex_state::scaled::Scaled::from_raw(10 * tex_state::scaled::Scaled::UNITY),
            tex_state::scaled::Scaled::from_raw(10 * tex_state::scaled::Scaled::UNITY),
            vec![tex_state::scaled::Scaled::from_raw(0); 7],
            tex_fonts::FontMetrics::new(characters, Vec::new(), None, None, Vec::new()),
        )
    }

    struct WorldHost;

    struct StartupLines {
        lines: VecDeque<String>,
        prompts: Vec<String>,
    }

    impl StartupLines {
        fn new(lines: &[&str]) -> Self {
            Self {
                lines: lines.iter().map(|line| (*line).to_owned()).collect(),
                prompts: Vec::new(),
            }
        }
    }

    impl StartupInput for StartupLines {
        fn read_line(&mut self, prompt: &str) -> Option<String> {
            self.prompts.push(prompt.to_owned());
            self.lines.pop_front()
        }
    }

    #[derive(Default)]
    struct ObservationRecorder(Vec<CommandObservation>);

    impl CommandObserver for ObservationRecorder {
        fn committed(&mut self, observation: CommandObservation) {
            self.0.push(observation);
        }
    }

    impl ResourceHost for WorldHost {
        fn fulfill(
            &mut self,
            world: &mut ResourceWorld<'_>,
            need: &ResourceNeed,
        ) -> ResourceOutcome {
            match need {
                ResourceNeed::Input { name, .. } => {
                    world
                        .read_file(name)
                        .ok()
                        .map_or(ResourceOutcome::Unavailable, |content| {
                            ResourceOutcome::Fulfilled(ResourceFulfillment::world_input(
                                name, content,
                            ))
                        })
                }
                ResourceNeed::InputProbe { request } => world.read_file(&request.name).ok().map_or(
                    ResourceOutcome::Unavailable,
                    |content| {
                        ResourceOutcome::Fulfilled(ResourceFulfillment::world_input_probe(
                            request.clone(),
                            content,
                        ))
                    },
                ),
                ResourceNeed::Font { request } => world
                    .read_file(canonical_font_resource_path(&request.name))
                    .ok()
                    .map_or(ResourceOutcome::Unavailable, |metrics| {
                        ResourceOutcome::Fulfilled(ResourceFulfillment::Font {
                            request: request.clone(),
                            resource: Box::new(FontResource::Tfm {
                                metrics,
                                opentype: None,
                            }),
                        })
                    }),
                ResourceNeed::PdfImage { request } => world.read_file(&request.name).ok().map_or(
                    ResourceOutcome::Unavailable,
                    |content| {
                        ResourceOutcome::Fulfilled(ResourceFulfillment::PdfImage {
                            request: request.clone(),
                            resource: Box::new(PdfImageResource::Available(
                                tex_state::PdfExternalImageSource {
                                    identity: content.hash(),
                                    metadata: tex_state::PdfExternalImageMetadata::Raster(
                                        tex_state::PdfRasterImageMetadata::placeholder(),
                                    ),
                                    natural_width: tex_state::scaled::Scaled::from_raw(
                                        tex_state::scaled::Scaled::UNITY,
                                    ),
                                    natural_height: tex_state::scaled::Scaled::from_raw(
                                        tex_state::scaled::Scaled::UNITY,
                                    ),
                                    bytes: content.bytes().to_vec(),
                                },
                            )),
                        })
                    },
                ),
            }
        }
    }

    #[test]
    fn retained_session_enters_packed_episode_and_resumes_after_output_checkpoint() {
        let source = Arc::<[u8]>::from(
            &br"\count0=0\count1=0\count2=0\def\e#1{\advance\count0by#1\global\advance\count1by#1\ifnum#1<5\global\advance\count2by1\else\global\advance\count2by2\fi A\kern#1sp}\shipout\hbox{\e{1}\e{8}}\end"[..],
        );
        with_fresh_stores(|stores| {
            crate::prepare_run_stores(stores);
            let mut session = EngineSession::new(stores, CommandProfile::TEX82);
            let mut context = session.stores.command_context().expect("admit font setup");
            let font = context.intern_font(packed_episode_font());
            context
                .assign_current_font(font, tex_state::AssignmentScope::Global)
                .expect("select packed-episode font");
            drop(context);
            session
                .register_authored_job("packed.tex", source)
                .expect("root registers");
            let mut checkpoints = Vec::new();
            let result = session
                .run(&mut WorldHost, &mut checkpoints)
                .expect("retained batch job completes");

            assert_eq!(result.artifacts.len(), 1);
            assert_eq!(result.dvi_pages.len(), 1);
            assert_eq!(
                checkpoints
                    .iter()
                    .map(tex_exec::EngineCheckpoint::boundary)
                    .collect::<Vec<_>>(),
                [EngineBoundary::JobStart, EngineBoundary::ShipoutComplete]
            );
            let telemetry = session.episode_telemetry();
            assert_eq!(
                telemetry.semantic_barriers(tex_exec::SemanticEpisodeBarrier::Output),
                1
            );
            assert_eq!(telemetry.terminals(), 1);
        });
    }

    fn with_fresh_stores<R>(
        use_stores: impl for<'id> FnOnce(&mut Universe<tex_state::GenerationBrand<'id>>) -> R,
    ) -> R {
        crate::with_engine_universe(use_stores).expect("fresh engine-session test universe")
    }

    fn with_startup_session<R>(
        interaction: tex_state::InteractionMode,
        use_stores: impl for<'id> FnOnce(&mut Universe<tex_state::GenerationBrand<'id>>) -> R,
    ) -> R {
        with_fresh_stores(|stores| {
            stores.set_interaction_mode(interaction);
            stores
                .world_mut()
                .set_memory_file("paper.tex", b"\\end".to_vec())
                .expect("startup fixture registers");
            use_stores(stores)
        })
    }

    #[test]
    fn startup_acquisition_orders_banner_log_echo_and_root_open() {
        with_startup_session(tex_state::InteractionMode::ErrorStop, |stores| {
            let mut session = EngineSession::tex82_initex(stores);
            let mut lines = StartupLines::new(&["paper.tex"]);
            session
                .acquire_startup_root(&mut lines, &mut WorldHost)
                .expect("startup root opens");
            session
                .run(&mut WorldHost, &mut Vec::new())
                .expect("startup job completes");

            assert_eq!(lines.prompts, ["**"]);
            assert_eq!(session.control.capabilities_mut().job_name(), "paper");
            let (terminal, log) = transcript_channels(session.stores());
            assert!(terminal.starts_with("This is TeX"));
            assert!(
                terminal.contains("(paper.tex"),
                "terminal={terminal:?} log={log:?}"
            );
            assert!(
                log.find("This is TeX").expect("log banner")
                    < log.find("**paper.tex").expect("startup echo")
            );
            let echo = log.find("**paper.tex");
            let opening = log.find("(paper.tex");
            assert!(
                echo.zip(opening)
                    .is_some_and(|(echo, opening)| echo < opening),
                "terminal={terminal:?} log={log:?}"
            );
        });
    }

    #[test]
    fn expansion_stats_project_only_committed_expanded_deliveries_with_provenance() {
        with_fresh_stores(|stores| {
            let mut session = EngineSession::tex82_initex(stores);
            session
                .register_authored_job("stats.tex", b"\\number42\\end".to_vec().into())
                .expect("stats root registers");
            let (_, stats) = session
                .run_with_expansion_stats(&mut WorldHost, &mut Vec::new())
                .expect("stats fixture completes");
            assert_eq!(
                stats,
                ExpansionStats {
                    token_frame_steps: 10,
                    provenance_resolutions: 9,
                    character_tokens: 5,
                    meaning_lookups: 10,
                    literal_spans: 2,
                    literal_tokens: 5,
                    source_text_span_attempts: 10,
                    source_text_spans: 1,
                    source_text_tokens: 3,
                    ..ExpansionStats::default()
                }
            );
            assert!(stats.character_fraction().is_finite());
            assert!(stats.mean_source_text_run().is_finite());
        });
    }

    #[test]
    fn startup_acquisition_retries_once_and_applies_default_tex_extension() {
        with_startup_session(tex_state::InteractionMode::Scroll, |stores| {
            let mut session = EngineSession::tex82_initex(stores);
            let mut lines = StartupLines::new(&["missing", "paper"]);
            session
                .acquire_startup_root(&mut lines, &mut WorldHost)
                .expect("replacement root opens");

            assert_eq!(lines.prompts, ["**", ""]);
            assert_eq!(session.control.capabilities_mut().job_name(), "paper");
            let (terminal, log) = transcript_channels(session.stores());
            assert_eq!(
                terminal.matches("I can't find file `missing.tex'").count(),
                1
            );
            assert_eq!(
                terminal
                    .matches("Please type another input file name")
                    .count(),
                1
            );
            // §534 echoes the replacement terminal buffer; §529's default `.tex`
            // extension belongs to the selected filename, not that buffer.
            assert!(log.contains("**paper\n"));
            assert!(!log.contains("**paper.tex"));
            assert!(!log.contains("missing"));
        });
    }

    #[test]
    fn startup_acquisition_aborts_without_replacement_in_noninteractive_modes() {
        for interaction in [
            tex_state::InteractionMode::Nonstop,
            tex_state::InteractionMode::Batch,
        ] {
            with_startup_session(interaction, |stores| {
                let mut session = EngineSession::tex82_initex(stores);
                let mut lines = StartupLines::new(&["missing", "paper"]);
                let error = session
                    .acquire_startup_root(&mut lines, &mut WorldHost)
                    .expect_err("missing startup root is fatal");

                assert!(matches!(
                    error,
                    SessionError::StartupFileUnavailable { ref name }
                        if name == "missing.tex"
                ));
                assert_eq!(lines.prompts, ["**"]);
                assert_eq!(
                    session.stores().world().error_channel().history(),
                    tex_state::print::ErrorHistory::FatalErrorStop
                );
            });
        }
    }

    struct OneInputHost {
        calls: usize,
    }

    impl ResourceHost for OneInputHost {
        fn fulfill(
            &mut self,
            _world: &mut ResourceWorld<'_>,
            need: &ResourceNeed,
        ) -> ResourceOutcome {
            self.calls += 1;
            match need {
                ResourceNeed::Input { name, .. } if name == "child.tex" => {
                    ResourceOutcome::Fulfilled(ResourceFulfillment::input(
                        "child.tex",
                        RegisteredSourceKind::Generated,
                        Arc::from(&b"\\relax"[..]),
                    ))
                }
                _ => ResourceOutcome::Declined,
            }
        }
    }

    struct MissingThenReplacementHost {
        replacement: Option<&'static str>,
        calls: Vec<String>,
    }

    impl ResourceHost for MissingThenReplacementHost {
        fn fulfill(
            &mut self,
            _world: &mut ResourceWorld<'_>,
            need: &ResourceNeed,
        ) -> ResourceOutcome {
            let ResourceNeed::Input { name, .. } = need else {
                return ResourceOutcome::Unavailable;
            };
            self.calls.push(name.clone());
            if self.replacement == Some(name) {
                ResourceOutcome::Fulfilled(ResourceFulfillment::input(
                    name,
                    RegisteredSourceKind::Generated,
                    Arc::from(&b"\\relax"[..]),
                ))
            } else {
                ResourceOutcome::Unavailable
            }
        }
    }

    fn with_prepared_session<R>(
        source: &'static [u8],
        use_session: impl for<'id> FnOnce(
            &mut Universe<tex_state::GenerationBrand<'id>>,
            Arc<[u8]>,
        ) -> R,
    ) -> R {
        with_fresh_stores(|stores| {
            crate::prepare_run_stores(stores);
            use_session(stores, Arc::from(source))
        })
    }

    fn tex82_format_image() -> tex_state::DetachedFormatImage {
        with_fresh_stores(|stores| {
            crate::prepare_run_stores(stores);
            let mut session = EngineSession::prepared_initex(stores, CommandProfile::TEX82);
            session
                .register_authored_job("format.tex", Arc::from(&b"\\dump"[..]))
                .expect("format root registers");
            session
                .run(&mut WorldHost, &mut Vec::new())
                .expect("test format completes")
                .format_dump
                .expect("test format dump")
                .image
        })
    }

    fn with_fresh_or_loaded<R>(
        loaded: bool,
        format: &tex_state::DetachedFormatImage,
        use_stores: impl for<'id> FnOnce(&mut Universe<tex_state::GenerationBrand<'id>>) -> R,
    ) -> R {
        let mut use_stores = Some(use_stores);
        if loaded {
            tex_state::with_materialized_format(
                crate::engine_interner_budget(),
                World::memory(),
                format,
                |stores| use_stores.take().expect("single format callback")(stores),
            )
            .expect("format restores")
        } else {
            with_fresh_stores(|stores| {
                crate::prepare_run_stores(stores);
                use_stores.take().expect("single fresh callback")(stores)
            })
        }
    }

    fn transcript_channels<G>(stores: &Universe<G>) -> (String, String) {
        let mut terminal = String::new();
        let mut log = String::new();
        for effect in stores.world().effect_records() {
            let tex_state::EffectRecord::StreamWrite { sink, text } = effect else {
                continue;
            };
            match sink {
                tex_state::PrintSink::Terminal => terminal.push_str(text),
                tex_state::PrintSink::Log => log.push_str(text),
                tex_state::PrintSink::TerminalAndLog => {
                    terminal.push_str(text);
                    log.push_str(text);
                }
                tex_state::PrintSink::Stream(_) => {}
            }
        }
        (terminal, log)
    }

    fn hyphen_positions<G>(stores: &mut Universe<G>, word: &str) -> Vec<usize> {
        stores
            .command_context()
            .expect("admit hyphenation test context")
            .hyphen_positions_for_language(0, word, 2, 3)
    }

    #[test]
    fn retained_observer_captures_fresh_and_format_loaded_production_runs() {
        let source: Arc<[u8]> = Arc::from(&b"\\message{observed}\\end"[..]);
        let format = tex82_format_image();

        for loaded in [false, true] {
            with_fresh_or_loaded(loaded, &format, |stores| {
                let mut session = EngineSession::new(stores, CommandProfile::TEX82);
                session
                    .register_authored_job("observer", Arc::clone(&source))
                    .expect("root registers");
                let mut observations = ObservationRecorder::default();
                let run = session
                    .run_with_observer(&mut WorldHost, &mut Vec::new(), &mut observations)
                    .expect("observed run completes");

                assert_eq!(run.terminal_text, "(observer observed )");
                assert!(
                    observations
                        .0
                        .iter()
                        .any(|event| matches!(event, CommandObservation::Command(_))),
                    "loaded={loaded}: command delivery is observed"
                );
                assert!(
                    observations
                        .0
                        .iter()
                        .any(|event| matches!(event, CommandObservation::Effect(_))),
                    "loaded={loaded}: committed effects are observed"
                );
                assert!(matches!(
                    observations.0.last(),
                    Some(CommandObservation::Effect(effect))
                        if effect.kind == tex_command::ObservationEffectKind::Terminate
                ));
                assert_eq!(
                    observations
                        .0
                        .iter()
                        .filter(|event| matches!(
                            event,
                            CommandObservation::Effect(effect)
                                if effect.kind == tex_command::ObservationEffectKind::Terminate
                        ))
                        .count(),
                    1,
                    "loaded={loaded}: the retained session owns one terminal observation"
                );
            });
        }
    }

    /// tex.web §82 ends *every* recoverable error with the same 100-error
    /// branch, not just the handful of sites that happen to inspect it.
    ///
    /// `\insert255`, exercised by the test below, was one of only three call
    /// sites in the engine that read `error`'s verdict; the other 55 dropped
    /// it, so a document raising a hundred of any other error ran straight
    /// past the point tex.web stops (`umber2-er8c`). §370's undefined control
    /// sequence is one of those 55 and reaches the limit through
    /// `diagnostics::report_undefined_control_sequence`, which now propagates
    /// §81's `jump_out` like the rest.
    #[test]
    fn the_hundredth_undefined_control_sequence_ends_the_job() {
        // Five past the limit: §82 stops at the hundredth, so reaching the
        // end of this source at all would mean the branch never fired.
        let mut source = String::new();
        for index in 0..105 {
            source.push_str(&format!("\\undefined{index}"));
        }
        source.push_str("\\end");
        with_prepared_session(b"", |stores, _| {
            stores.set_interaction_mode(tex_state::InteractionMode::Nonstop);
            let mut session = EngineSession::new(stores, CommandProfile::TEX82);
            session
                .set_fuel_limit(100_000)
                .expect("bounded fatal microfixture fuel");
            session
                .register_authored_job("undefined.tex", Arc::from(source.into_bytes()))
                .expect("root registers");

            let run = session
                .run_with_observer(
                    &mut WorldHost,
                    &mut Vec::new(),
                    &mut ObservationRecorder::default(),
                )
                .expect("TeX fatal stop reaches engine termination");

            assert_eq!(run.status, TexRunStatus::Fatal);

            assert_eq!(
                session.control.fatal_error(),
                Some(tex_command::FatalError::TooManyErrors)
            );
            assert_eq!(
                session.stores().world().error_channel().error_count(),
                100,
                "§82 stops at its hundredth error, not later"
            );
            assert_eq!(
                session.stores().world().error_channel().history(),
                tex_state::print::ErrorHistory::FatalErrorStop
            );
        });
    }

    #[test]
    fn fatal_completion_terminates_once_after_diagnostic_with_fatal_history() {
        let mut source = String::from("\\setbox0=\\vbox{");
        for _ in 0..100 {
            source.push_str("\\insert255{}");
        }
        source.push_str("}\\end");
        with_prepared_session(b"", |stores, _| {
            stores.set_interaction_mode(tex_state::InteractionMode::Nonstop);
            let mut session = EngineSession::new(stores, CommandProfile::TEX82);
            session
                .set_fuel_limit(100_000)
                .expect("bounded fatal microfixture fuel");
            session
                .register_authored_job("fatal.tex", Arc::from(source.into_bytes()))
                .expect("root registers");
            let mut observations = ObservationRecorder::default();

            session
                .run_with_observer(&mut WorldHost, &mut Vec::new(), &mut observations)
                .expect("TeX fatal stop reaches engine termination");

            assert_eq!(
                session.control.fatal_error(),
                Some(tex_command::FatalError::TooManyErrors)
            );
            assert_eq!(
                session.stores().world().error_channel().history(),
                tex_state::print::ErrorHistory::FatalErrorStop
            );
            assert!(matches!(
                observations.0.as_slice(),
                [.., CommandObservation::Diagnostic(_), CommandObservation::Effect(effect)]
                    if effect.kind == tex_command::ObservationEffectKind::Terminate
            ));
            assert_eq!(
                observations
                    .0
                    .iter()
                    .filter(|event| matches!(
                        event,
                        CommandObservation::Effect(effect)
                            if effect.kind == tex_command::ObservationEffectKind::Terminate
                    ))
                    .count(),
                1
            );
        });
    }

    #[test]
    fn recovered_error_completes_with_unsuccessful_status_after_termination() {
        with_prepared_session(b"\\undefined\\end", |stores, root| {
            stores.set_interaction_mode(tex_state::InteractionMode::Nonstop);
            let mut session = EngineSession::new(stores, CommandProfile::TEX82);
            session
                .register_authored_job("recovered.tex", root)
                .expect("root registers");
            let mut observations = ObservationRecorder::default();

            let run = session
                .run_with_observer(&mut WorldHost, &mut Vec::new(), &mut observations)
                .expect("recoverable diagnostic does not abort execution");

            assert_eq!(run.status, TexRunStatus::CompletedWithErrors);
            assert!(run.fatal.is_none());
            assert!(run.terminal_text.contains("Undefined control sequence"));
            assert!(matches!(
                observations.0.last(),
                Some(CommandObservation::Effect(effect))
                    if effect.kind == tex_command::ObservationEffectKind::Terminate
            ));
        });
    }

    #[test]
    fn source_exhaustion_terminates_once_after_stop_under_finite_fuel() {
        with_prepared_session(b"", |stores, root| {
            let mut session = EngineSession::new(stores, CommandProfile::TEX82);
            session.set_fuel_limit(16).expect("finite fuel");
            session
                .register_authored_fragment("empty.tex", root)
                .expect("root registers");
            let mut observations = ObservationRecorder::default();

            assert!(matches!(
                session
                    .advance_until_waiting_with_observer(&mut Vec::new(), &mut observations)
                    .expect("empty source completes"),
                SessionState::Complete(_)
            ));
            let stop = observations
                .0
                .iter()
                .position(|observation| {
                    matches!(
                        observation,
                        CommandObservation::Input(input)
                            if input.transition == tex_command::InputTransition::Stop
                                && input.reason == tex_command::InputReason::Source
                    )
                })
                .expect("terminal source stop");
            let terminations = observations
                .0
                .iter()
                .enumerate()
                .filter(|(_, observation)| {
                    matches!(
                        observation,
                        CommandObservation::Effect(effect)
                            if effect.kind == tex_command::ObservationEffectKind::Terminate
                    )
                })
                .map(|(index, _)| index)
                .collect::<Vec<_>>();
            assert_eq!(terminations, vec![stop + 1]);
            let burned = session.fuel_burned();
            assert!(burned <= session.fuel_limit());

            assert!(matches!(
                session
                    .advance_until_waiting_with_observer(&mut Vec::new(), &mut observations)
                    .expect("completed session remains complete"),
                SessionState::Complete(_)
            ));
            assert_eq!(session.fuel_burned(), burned);
            assert_eq!(
                observations
                    .0
                    .iter()
                    .filter(|observation| matches!(
                        observation,
                        CommandObservation::Effect(effect)
                            if effect.kind == tex_command::ObservationEffectKind::Terminate
                    ))
                    .count(),
                1
            );
        });
    }

    #[test]
    fn explicit_fragment_eof_completes_without_terminal_input_or_final_cleanup() {
        with_prepared_session(b"\\global\\count0=7", |stores, root| {
            stores
                .world_mut()
                .push_memory_terminal_line("\\global\\count0=99\\end")
                .expect("terminal line is staged");
            let mut session = EngineSession::new(stores, CommandProfile::TEX82);
            session
                .register_authored_fragment("fragment", root)
                .expect("fragment registers");

            let run = session
                .run(&mut WorldHost, &mut Vec::new())
                .expect("fragment EOF completes");

            assert_eq!(session.stores().count(0), Ok(7));
            assert_eq!(
                session.stores().world().stream_bufs().terminal_input_next(),
                0
            );
            assert!(run.fatal.is_none());
        });
    }

    #[test]
    fn complete_job_eof_is_one_mode_specific_fatal_termination() {
        for interaction in [
            tex_state::InteractionMode::Batch,
            tex_state::InteractionMode::Nonstop,
            tex_state::InteractionMode::Scroll,
            tex_state::InteractionMode::ErrorStop,
        ] {
            with_prepared_session(b"", |stores, root| {
                stores.set_interaction_mode(interaction);
                let mut session = EngineSession::new(stores, CommandProfile::TEX82);
                session.set_fuel_limit(32).expect("finite EOF fuel");
                session
                    .register_authored_job("missing-end.tex", root)
                    .expect("job registers");

                let run = session
                    .run(&mut WorldHost, &mut Vec::new())
                    .expect("fatal EOF reaches terminal completion");
                let help = if matches!(
                    interaction,
                    tex_state::InteractionMode::Scroll | tex_state::InteractionMode::ErrorStop
                ) {
                    "End of file on the terminal!"
                } else {
                    "*** (job aborted, no legal \\end found)"
                };
                assert_eq!(
                    run.fatal,
                    Some(tex_command::FatalError::emergency_stop(help)),
                    "interaction {interaction:?}"
                );
                assert_eq!(run.status, TexRunStatus::Fatal);
                assert_eq!(
                    session.stores().world().error_channel().history(),
                    tex_state::print::ErrorHistory::FatalErrorStop
                );
                let burned = session.fuel_burned();
                assert!(burned <= session.fuel_limit());
                assert!(matches!(
                    session
                        .advance_until_waiting(&mut Vec::new())
                        .expect("fatal completion stays latched"),
                    SessionState::Complete(_)
                ));
                assert_eq!(session.fuel_burned(), burned);
            });
        }
    }

    #[test]
    fn interactive_root_eof_executes_terminal_lines_until_end() {
        for interaction in [
            tex_state::InteractionMode::Scroll,
            tex_state::InteractionMode::ErrorStop,
        ] {
            with_prepared_session(b"", |stores, root| {
                stores.set_interaction_mode(interaction);
                stores
                    .world_mut()
                    .push_memory_terminal_line("")
                    .expect("empty terminal line is staged");
                stores
                    .world_mut()
                    .push_memory_terminal_line("\\global\\count0=42\\end")
                    .expect("terminating terminal line is staged");
                let mut session = EngineSession::new(stores, CommandProfile::TEX82);
                session.set_fuel_limit(64).expect("finite terminal fuel");
                session
                    .register_authored_job("terminal-end.tex", root)
                    .expect("job registers");
                let mut observations = ObservationRecorder::default();

                let run = session
                    .run_with_observer(&mut WorldHost, &mut Vec::new(), &mut observations)
                    .expect("terminal end completes the job");

                assert!(run.fatal.is_none(), "interaction {interaction:?}");
                assert_eq!(session.stores().count(0), Ok(42));
                assert_eq!(
                    session.stores().world().stream_bufs().terminal_input_next(),
                    2
                );
                assert!(observations.0.iter().any(|observation| matches!(
                    observation,
                    CommandObservation::Input(input)
                        if input.source_name == Some(tex_command::SourceNameClass::Terminal)
                )));
                assert!(session.fuel_burned() <= session.fuel_limit());
            });
        }
    }

    #[test]
    fn unobserved_completion_does_not_republish_termination() {
        with_prepared_session(b"", |stores, root| {
            let mut session = EngineSession::new(stores, CommandProfile::TEX82);
            session
                .register_authored_fragment("empty.tex", root)
                .expect("root registers");

            assert!(matches!(
                session
                    .advance_until_waiting(&mut Vec::new())
                    .expect("unobserved source completes"),
                SessionState::Complete(_)
            ));
            let mut observations = ObservationRecorder::default();
            assert!(matches!(
                session
                    .advance_until_waiting_with_observer(&mut Vec::new(), &mut observations)
                    .expect("completion remains latched"),
                SessionState::Complete(_)
            ));
            assert!(observations.0.is_empty());
        });
    }

    #[test]
    fn resource_suspension_does_not_publish_or_latch_termination() {
        with_prepared_session(br"\input child\end", |stores, root| {
            let mut session = EngineSession::new(stores, CommandProfile::TEX82);
            session
                .register_authored_job("job.tex", root)
                .expect("root registers");
            let mut observations = ObservationRecorder::default();

            let need = match session
                .advance_until_waiting_with_observer(&mut Vec::new(), &mut observations)
                .expect("missing child suspends")
            {
                SessionState::NeedResource(need) => need,
                SessionState::Complete(_) => panic!("missing child must suspend"),
            };
            assert!(
                !observations.0.iter().any(|observation| matches!(
                    observation,
                    CommandObservation::Effect(effect)
                        if effect.kind == tex_command::ObservationEffectKind::Terminate
                )),
                "rolled-back suspension cannot terminate the session"
            );

            session
                .fulfill(
                    &need,
                    ResourceFulfillment::input(
                        "child.tex",
                        RegisteredSourceKind::Generated,
                        Arc::from(&b""[..]),
                    ),
                )
                .expect("child fulfillment matches");
            assert!(matches!(
                session
                    .advance_until_waiting_with_observer(&mut Vec::new(), &mut observations)
                    .expect("retry completes"),
                SessionState::Complete(_)
            ));
            assert_eq!(
                observations
                    .0
                    .iter()
                    .filter(|observation| matches!(
                        observation,
                        CommandObservation::Effect(effect)
                            if effect.kind == tex_command::ObservationEffectKind::Terminate
                    ))
                    .count(),
                1
            );
        });
    }

    #[test]
    fn etex_alphabetic_constants_preserve_control_symbol_spelling() {
        let source: Arc<[u8]> = Arc::from(&br"\endlinechar=`\^^M \newlinechar=`\^^J \end"[..]);
        with_fresh_stores(|stores| {
            crate::prepare_etex_run_stores(stores);
            let mut session = EngineSession::new(stores, CommandProfile::ETEX26);
            session
                .register_authored_job("alphabetic.tex", source)
                .expect("root registers");
            let mut observations = ObservationRecorder::default();
            session
                .run_with_observer(&mut WorldHost, &mut Vec::new(), &mut observations)
                .expect("assignment microfixture completes");

            let spellings = observations
                .0
                .iter()
                .filter_map(|event| match event {
                    CommandObservation::Command(record)
                        if record.boundary == tex_command::CommandDeliveryBoundary::Raw =>
                    {
                        Some(&record.spelling)
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert!(spellings.iter().any(|spelling| {
                matches!(
                    spelling,
                    tex_command::ObservedToken::ControlSequence(name) if name == "\r"
                )
            }));
            assert!(spellings.iter().any(|spelling| {
                matches!(
                    spelling,
                    tex_command::ObservedToken::ControlSequence(name) if name == "\n"
                )
            }));
        });
    }

    #[test]
    fn retained_session_retries_input_without_duplicate_effect_or_receipt() {
        with_prepared_session(
            b"\\message{once}\\shipout\\hbox{x}\\input child \\end",
            |stores, root| {
                let mut session = EngineSession::new(stores, CommandProfile::TEX82);
                session
                    .register_authored_job("job.tex", root)
                    .expect("root registers");
                let mut host = OneInputHost { calls: 0 };
                let mut checkpoints = Vec::new();
                let run = session
                    .run(&mut host, &mut checkpoints)
                    .expect("run completes");

                assert_eq!(host.calls, 1);
                assert_eq!(
                    session.stores().world().memory_terminal_output(),
                    // The first attempt materializes §638's `[0]` before the later
                    // `\input` suspends. The replay reaches that same marker again,
                    // but the retained session reconciles the repeated suffix. The
                    // input framing remains virtual because no later shipout commits
                    // it in this fragment.
                    Some(&b"(job.tex once [0]"[..]),
                    "aggregate rollback must not repeat a materialized write"
                );
                assert_eq!(run.artifacts.len(), 1);
                assert_eq!(run.dvi_pages.len(), run.artifacts.len());
                let boundaries = checkpoints
                    .iter()
                    .map(tex_exec::EngineCheckpoint::boundary)
                    .collect::<Vec<_>>();
                assert!(boundaries.contains(&EngineBoundary::JobStart));
                assert!(boundaries.contains(&EngineBoundary::ShipoutComplete));
            },
        );
    }

    #[test]
    fn initex_prints_tex82_startup_headline_before_the_first_command() {
        with_fresh_stores(|stores| {
            let mut session = EngineSession::tex82_initex(stores);
            session.set_fuel_limit(64).expect("finite fuel");
            session
                .register_authored_job("headline.tex", Arc::from(&b"\\end"[..]))
                .expect("INITEX root registers");

            session
                .run(&mut WorldHost, &mut Vec::new())
                .expect("bounded INITEX source completes");

            let first = session
                .stores()
                .world()
                .effect_records()
                .first()
                .expect("startup headline is the first effect");
            assert_eq!(
                first,
                &tex_state::EffectRecord::StreamWrite {
                    sink: tex_state::PrintSink::Terminal,
                    text: "This is TeX, Version 3.141592653 (TeX Live 2026) (INITEX)".into(),
                },
                "TeX82 §1332 writes the process headline to the terminal before main_control"
            );
        });
    }

    #[test]
    fn retained_initex_refreshes_the_job_clock_before_the_first_command() {
        let clock = tex_state::JobClock {
            time: 13 * 60 + 36,
            second: 7,
            day: 9,
            month: 7,
            year: 2026,
        };
        crate::with_engine_world(World::memory_with_clock(clock), |stores| {
            crate::prepare_run_stores(stores);
            let mut session = EngineSession::prepared_initex(stores, CommandProfile::TEX82);
            session
                .register_authored_job("clock.tex", Arc::from(&b"\\end"[..]))
                .expect("INITEX root registers");
            session
                .run(&mut WorldHost, &mut Vec::new())
                .expect("bounded INITEX source completes");
            drop(session);

            let context = stores.command_context().expect("admit clock projection");
            assert_eq!(
                context.int_param(tex_state::env::banks::IntParam::TIME),
                clock.time
            );
            assert_eq!(
                context.int_param(tex_state::env::banks::IntParam::DAY),
                clock.day
            );
            assert_eq!(
                context.int_param(tex_state::env::banks::IntParam::MONTH),
                clock.month
            );
            assert_eq!(
                context.int_param(tex_state::env::banks::IntParam::YEAR),
                clock.year
            );
        })
        .expect("fresh engine-session test universe");
    }

    #[test]
    fn startup_input_opening_uses_terminal_only_selector_in_initex_and_loaded_sessions() {
        const SOURCE: &[u8] = br"\end";
        let format = tex82_format_image();

        for initex in [true, false] {
            with_fresh_or_loaded(!initex, &format, |stores| {
                let mut session = if initex {
                    EngineSession::prepared_initex(stores, CommandProfile::TEX82)
                } else {
                    EngineSession::new(stores, CommandProfile::TEX82)
                };
                session
                    .register_authored_job("./trip.tex", Arc::from(SOURCE))
                    .expect("root registers");

                session
                    .run(&mut WorldHost, &mut Vec::new())
                    .expect("bounded root completes");

                let expected_terminal = if initex {
                    "This is TeX, Version 3.141592653 (TeX Live 2026) (INITEX)\n(./trip.tex )"
                } else {
                    "(./trip.tex )"
                };
                let (terminal, log) = transcript_channels(session.stores());
                assert_eq!(
                    terminal, expected_terminal,
                    "TeX82 §§1332, 537 startup framing, initex={initex}"
                );
                if initex {
                    assert!(
                        log.starts_with("This is TeX, Version 3.141592653 (TeX Live 2026)"),
                        "§536 opens the transcript with the engine banner: {log:?}"
                    );
                    assert!(
                        log.ends_with("\n**./trip.tex\n(./trip.tex )"),
                        "§§534, 537, 1335 preserve startup echo/open/close order: {log:?}"
                    );
                } else {
                    assert_eq!(
                        log, " )",
                        "a framing-neutral loaded session retains its host-owned startup prefix"
                    );
                }
            });
        }
    }

    #[test]
    fn initex_session_loads_patterns_while_cold_session_rejects_them() {
        const SOURCE: &[u8] = br"\patterns{o1ce eed3i}\lefthyphenmin=2 \righthyphenmin=3 \end";

        with_prepared_session(SOURCE, |cold_stores, cold_root| {
            let mut cold = EngineSession::new(cold_stores, CommandProfile::TEX82);
            cold.register_authored_job("cold.tex", cold_root)
                .expect("cold root registers");
            cold.run(&mut WorldHost, &mut Vec::new())
                .expect("cold session recovers from init-only patterns");
            drop(cold);
            assert_eq!(
                hyphen_positions(cold_stores, "proceeding"),
                Vec::<usize>::new(),
                "TeX82 §1252 rejects patterns outside INITEX"
            );

            with_fresh_stores(|initex_stores| {
                crate::prepare_run_stores(initex_stores);
                let mut initex =
                    EngineSession::prepared_initex(initex_stores, CommandProfile::TEX82);
                initex
                    .register_authored_job("initex.tex", Arc::from(SOURCE))
                    .expect("INITEX root registers");
                initex
                    .run(&mut WorldHost, &mut Vec::new())
                    .expect("INITEX patterns execute");
                drop(initex);
                assert_eq!(
                    hyphen_positions(initex_stores, "proceeding"),
                    vec![3, 7],
                    "the two oracle pattern matches produce pro-ceed-ing"
                );
            });
        });
    }

    #[test]
    fn initex_dump_receipt_survives_the_direct_session_boundary() {
        with_fresh_stores(|stores| {
            let mut session = EngineSession::tex82_initex(stores);
            session
                .register_authored_job("plain.tex", Arc::from(&b"\\dump"[..]))
                .expect("INITEX root registers");

            let run = session
                .run(&mut WorldHost, &mut Vec::new())
                .expect("INITEX dump completes");

            assert!(run.format_dump.is_some());
            assert!(
                !run.format_dump
                    .expect("format dump")
                    .image
                    .as_bytes()
                    .is_empty()
            );
        });
    }

    #[test]
    fn declining_host_is_bounded_without_mutating_effects() {
        with_prepared_session(b"\\input never\\end", |stores, root| {
            let mut session = EngineSession::new(stores, CommandProfile::TEX82);
            session.set_no_progress_limit(2);
            session
                .register_authored_job("job.tex", root)
                .expect("root registers");
            let mut host = OneInputHost { calls: 0 };
            let mut checkpoints = Vec::new();
            let error = session
                .run(&mut host, &mut checkpoints)
                .expect_err("host declines");
            assert!(matches!(
                error,
                SessionError::NoProgress { attempts: 2, .. }
            ));
            assert_eq!(
                transcript_channels(session.stores()),
                ("(job.tex".into(), String::new()),
                "only the committed §537 root opening precedes the declined child request"
            );
        });
    }

    #[test]
    fn completed_input_absence_retries_only_in_interactive_modes() {
        for interaction in [
            tex_state::InteractionMode::Scroll,
            tex_state::InteractionMode::ErrorStop,
        ] {
            with_prepared_session(b"\\input missing\\end", |stores, root| {
                stores.set_interaction_mode(interaction);
                stores
                    .world_mut()
                    .push_memory_terminal_line("replacement")
                    .expect("replacement filename is staged");
                let mut session = EngineSession::new(stores, CommandProfile::TEX82);
                session
                    .register_authored_job("job.tex", root)
                    .expect("root registers");
                let mut host = MissingThenReplacementHost {
                    replacement: Some("replacement.tex"),
                    calls: Vec::new(),
                };

                let run = session
                    .run(&mut host, &mut Vec::new())
                    .expect("interactive lookup retries after completed absence");
                assert_eq!(host.calls, ["missing.tex", "replacement.tex"]);
                assert_eq!(run.status, TexRunStatus::Success);
                assert_eq!(run.fatal, None);
                assert_eq!(
                    session.stores().world().error_channel().history(),
                    tex_state::print::ErrorHistory::Spotless
                );
                let (terminal, log) = transcript_channels(session.stores());
                for output in [&terminal, &log] {
                    assert_eq!(
                        output.matches("! I can't find file `missing.tex'.").count(),
                        1,
                        "output={output:?}"
                    );
                    assert!(output.contains("l.1 \\input missing"), "output={output:?}");
                    assert!(!output.contains("Emergency stop"), "output={output:?}");
                }
                assert_eq!(
                    terminal
                        .matches("Please type another input file name: ")
                        .count(),
                    1,
                    "terminal={terminal:?}"
                );
                assert_eq!(
                    log.matches("Please type another input file name: replacement")
                        .count(),
                    1,
                    "log={log:?}"
                );
            });
        }

        for interaction in [
            tex_state::InteractionMode::Batch,
            tex_state::InteractionMode::Nonstop,
        ] {
            with_prepared_session(b"\\input missing\\end", |stores, root| {
                stores.set_interaction_mode(interaction);
                let mut session = EngineSession::new(stores, CommandProfile::TEX82);
                session
                    .register_authored_job("job.tex", root)
                    .expect("root registers");
                let mut host = MissingThenReplacementHost {
                    replacement: None,
                    calls: Vec::new(),
                };

                let run = session
                    .run(&mut host, &mut Vec::new())
                    .expect("fatal termination still completes retained cleanup");
                let fatal = tex_command::FatalError::emergency_stop(
                    "job aborted, file error in nonstop mode",
                );
                assert_eq!(run.fatal, Some(fatal));
                assert_eq!(run.status, TexRunStatus::Fatal);
                assert_eq!(session.control.fatal_error(), Some(fatal));
                assert_eq!(host.calls, ["missing.tex"]);
                assert_eq!(
                    session.stores().world().error_channel().history(),
                    tex_state::print::ErrorHistory::FatalErrorStop
                );
                let (terminal, log) = transcript_channels(session.stores());
                assert_eq!(
                    log.matches("! I can't find file `missing.tex'.").count(),
                    1,
                    "log={log:?}"
                );
                assert_eq!(
                    log.matches("Please type another input file name").count(),
                    1,
                    "log={log:?}"
                );
                assert_eq!(log.matches("! Emergency stop.").count(), 1, "log={log:?}");
                assert_eq!(
                    log.matches("*** (job aborted, file error in nonstop mode)")
                        .count(),
                    1,
                    "log={log:?}"
                );
                assert_eq!(
                    log.matches("l.1 \\input missing").count(),
                    2,
                    "§530 and §93 each render the live context: log={log:?}"
                );
                assert_eq!(
                    terminal.contains("! I can't find file `missing.tex'."),
                    interaction == tex_state::InteractionMode::Nonstop,
                    "terminal={terminal:?}"
                );
                assert!(matches!(
                    session
                        .advance_until_waiting(&mut Vec::new())
                        .expect("fatal retained cleanup stays complete"),
                    SessionState::Complete(result) if result.fatal == Some(fatal)
                ));
                assert_eq!(host.calls, ["missing.tex"]);
            });
        }
    }

    #[test]
    fn committed_output_is_visible_to_later_input_after_atomic_retry() {
        with_prepared_session(
            br"\immediate\openout1=same.out
\immediate\write1{generated}
\immediate\closeout1
\shipout\hbox{}
\input same.out
\end",
            |stores, root| {
                let mut session = EngineSession::new(stores, CommandProfile::TEX82);
                session.set_no_progress_limit(1);
                session
                    .register_authored_job("job.tex", root)
                    .expect("root registers");
                let mut host = OneInputHost { calls: 0 };

                let run = session
                    .run(&mut host, &mut Vec::new())
                    .expect("same-run output makes retry progress");

                assert_eq!(host.calls, 1, "host policy gets one bounded opportunity");
                assert_eq!(
                    session.stores().world().memory_output("same.out"),
                    Some(&b"generated\n"[..]),
                    "retry neither duplicates nor loses the committed output"
                );
                assert_eq!(run.artifacts.len(), 2);
                assert!(
                    session
                        .stores()
                        .world()
                        .input_records()
                        .iter()
                        .any(|record| {
                            record.path() == Path::new("same.out")
                                && record.origin() == tex_state::InputOrigin::SameRunGenerated
                        })
                );
            },
        );
    }

    #[test]
    fn same_run_input_fulfillment_retains_its_resolved_local_name() {
        with_fresh_stores(|stores| {
            stores
                .world_mut()
                .set_memory_file("same.out", b"generated")
                .expect("same-run stand-in is seeded");
            let content = stores
                .world_mut()
                .read_file("same.out")
                .expect("selected bytes are retained");

            let ResourceFulfillment::Input { name, source } =
                same_run_input_fulfillment("same.out", content)
            else {
                panic!("same-run input helper returned a non-input fulfillment");
            };

            assert_eq!(name, "same.out");
            assert_eq!(source.name().map(AsRef::as_ref), Some("./same.out"));
        });
    }

    #[test]
    fn same_run_write_reopens_newlinechar_as_exact_physical_lines() {
        // TeX82 §§262 and 1370: expanded write tokens are first captured as
        // an internal string, then printed through the stream selector. A
        // character equal to `newlinechar` is therefore a physical line end.
        with_prepared_session(
            br"\newlinechar=1
\immediate\openout1=same.out
\immediate\write1{\noexpand\global\noexpand\count0=123^^A\noexpand\global\noexpand\count1=456}
\immediate\closeout1
\shipout\hbox{}
\input same.out
\end",
            |stores, root| {
                let mut session = EngineSession::new(stores, CommandProfile::TEX82);
                session
                    .register_authored_job("job.tex", root)
                    .expect("root registers");
                let mut host = OneInputHost { calls: 0 };

                session
                    .run(&mut host, &mut Vec::new())
                    .expect("same-run output is written and reopened");

                assert_eq!(
                    session.stores().world().memory_output("same.out"),
                    Some(&b"\\global \\count 0=123\n\\global \\count 1=456\n"[..])
                );
                assert!(
                    session
                        .stores()
                        .world()
                        .input_records()
                        .iter()
                        .any(|record| record.path() == Path::new("same.out")
                            && record.origin() == tex_state::InputOrigin::SameRunGenerated)
                );
                assert_eq!(session.stores().count(0), Ok(123));
                assert_eq!(session.stores().count(1), Ok(456));
            },
        );
    }

    #[test]
    fn world_host_records_selected_input_once_and_preserves_retry_effects() {
        with_prepared_session(b"\\message{once}\\input child\\end", |stores, root| {
            stores
                .world_mut()
                .set_memory_file("child.tex", b"\\message{child}")
                .expect("child is seeded");
            let mut session = EngineSession::new(stores, CommandProfile::TEX82);
            session
                .register_authored_job("job.tex", root)
                .expect("root registers");

            let run = session
                .run(&mut WorldHost, &mut Vec::new())
                .expect("world-backed input completes");

            // TeX82 §1280 separates the two messages with one space, because
            // the first left `term_offset` nonzero. §537/§362 additionally
            // bracket the root and `\input child` in parens around their own
            // messages, each named as opened the way §537's `a_make_name_string`
            // does: the startup input opening supplies `(job.tex`, and §1335
            // closes that still-open root after the child reached ordinary EOF.
            assert_eq!(run.terminal_text, "(job.tex once (child.tex child) )");
            let records = session.stores().world().input_records();
            assert_eq!(records.len(), 1, "the selected child is recorded once");
            assert_eq!(records[0].path(), Path::new("child.tex"));
            assert_eq!(
                session.stores().world().input_content(records[0].hash()),
                Some(&b"\\message{child}"[..])
            );
        });
    }

    #[test]
    fn world_host_fulfills_font_and_image_with_matching_selected_bytes() {
        with_prepared_session(
            b"\\font\\tenrm=cmr10 \\tenrm A\\end",
            |font_stores, font_root| {
                font_stores
                    .world_mut()
                    .set_memory_file("cmr10.tfm", CMR10)
                    .expect("font is seeded");
                let mut font_session = EngineSession::new(font_stores, CommandProfile::TEX82);
                font_session
                    .register_authored_job("font.tex", font_root)
                    .expect("font root registers");
                font_session
                    .run(&mut WorldHost, &mut Vec::new())
                    .expect("world-backed font completes");
                let font_record = font_session
                    .stores()
                    .world()
                    .input_records()
                    .first()
                    .expect("selected font is recorded");
                assert_eq!(font_record.path(), Path::new("cmr10.tfm"));
                assert_eq!(font_record.len(), CMR10.len());

                with_fresh_stores(|image_stores| {
                    crate::prepare_pdftex_run_stores(image_stores);
                    image_stores
                        .command_context()
                        .expect("admit PDF output setup")
                        .assign_int_param(
                            tex_state::env::banks::IntParam::PDF_OUTPUT,
                            1,
                            tex_state::AssignmentScope::Global,
                        )
                        .expect("enable PDF output");
                    image_stores
                        .world_mut()
                        .set_memory_file("image.png", b"world-selected image")
                        .expect("image is seeded");
                    let mut image_session =
                        EngineSession::new(image_stores, CommandProfile::PDFTEX14029);
                    image_session
                        .register_authored_job(
                            "image.tex",
                            Arc::from(&b"\\pdfximage {image.png}\\end"[..]),
                        )
                        .expect("image root registers");
                    image_session
                        .run(&mut WorldHost, &mut Vec::new())
                        .expect("world-backed image completes");
                    let image_record = image_session
                        .stores()
                        .world()
                        .input_records()
                        .first()
                        .expect("selected image is recorded");
                    assert_eq!(image_record.path(), Path::new("image.png"));
                    let image_hash = image_record.hash();
                    drop(image_session);
                    let context = image_stores
                        .command_context()
                        .expect("admit image result reader");
                    let image = context
                        .internal_integer(tex_state::meaning::InternalInteger::PdfLastXImage)
                        .and_then(|raw| u32::try_from(raw).ok())
                        .and_then(|raw| tex_state::PdfExternalImageId::new(raw).ok())
                        .and_then(|id| context.pdf_external_image_record(id));
                    assert_eq!(image.map(|image| image.identity()), Some(image_hash));
                });
            },
        );
    }

    #[test]
    fn fulfillment_rejects_mismatched_typed_need() {
        with_prepared_session(b"\\input child\\end", |stores, root| {
            let mut session = EngineSession::new(stores, CommandProfile::TEX82);
            session
                .register_authored_job("job.tex", root)
                .expect("root registers");
            let need = match session
                .advance_until_waiting(&mut Vec::new())
                .expect("input suspends")
            {
                SessionState::NeedResource(need) => need,
                other => panic!("expected resource need, got {other:?}"),
            };
            let error = session
                .fulfill(
                    &need,
                    ResourceFulfillment::input(
                        "other",
                        RegisteredSourceKind::Generated,
                        Arc::from(&b"\\end"[..]),
                    ),
                )
                .expect_err("mismatched input is rejected");
            assert!(matches!(error, SessionError::UnexpectedFulfillment { .. }));
        });
    }

    #[test]
    fn engine_session_has_finite_configurable_command_fuel() {
        with_fresh_stores(|stores| {
            let mut session = EngineSession::new(stores, CommandProfile::TEX82);
            assert_eq!(
                session.fuel_limit(),
                tex_command::DEFAULT_COMMAND_FUEL_LIMIT
            );
            assert_ne!(session.fuel_limit(), u64::MAX);
            session.set_fuel_limit(17).expect("valid finite limit");
            assert_eq!(session.fuel_limit(), 17);
            assert_eq!(session.fuel_burned(), 0);
            for invalid in [0, tex_command::MAX_COMMAND_FUEL_LIMIT + 1, u64::MAX] {
                assert!(session.set_fuel_limit(invalid).is_err());
                assert_eq!(session.fuel_limit(), 17);
            }
        });
    }

    #[test]
    fn tiny_limit_stops_a_cyclic_run_with_typed_error() {
        fn run(observed: bool) -> (SessionError, u64) {
            with_prepared_session(b"\\def\\cycle{\\cycle}\\cycle", |stores, root| {
                let mut session = EngineSession::new(stores, CommandProfile::TEX82);
                session
                    .register_authored_job("cycle.tex", root)
                    .expect("root registers");
                session.set_fuel_limit(19).expect("valid tiny limit");
                let mut observations = ObservationRecorder::default();
                let error = if observed {
                    session
                        .run_with_observer(&mut WorldHost, &mut Vec::new(), &mut observations)
                        .expect_err("observed cyclic run exhausts fuel")
                } else {
                    session
                        .run(&mut WorldHost, &mut Vec::new())
                        .expect_err("cyclic run exhausts fuel")
                };
                assert!(
                    !observations.0.iter().any(|event| matches!(
                        event,
                        CommandObservation::Effect(effect)
                            if effect.kind == tex_command::ObservationEffectKind::Terminate
                    )),
                    "fuel exhaustion is an aborted outcome, not engine completion"
                );
                let burned = session.fuel_burned();
                (error, burned)
            })
        }

        let (unobserved_error, unobserved_burned) = run(false);
        let (observed_error, observed_burned) = run(true);
        for error in [&unobserved_error, &observed_error] {
            assert!(matches!(
                error,
                SessionError::Execution(tex_exec::ExecError::Command(
                    tex_command::CommandError::FuelExhausted {
                        limit: 19,
                        burned: 19,
                        ..
                    }
                ))
            ));
        }
        assert_eq!(unobserved_burned, 19);
        assert_eq!(observed_burned, unobserved_burned);
    }
}
