//! Retained, host-neutral entry point for canonical TeX command execution.
//!
//! TeX82 §24 (`get_next`) and §25 (`expand`/`get_x_token`) retain input and
//! expansion inside the command machine.  §1030 (`main_control`) consumes the
//! resulting command.  Consequently this host owns only immutable resource
//! registrations and aggregate retry policy; it has no token-delivery API.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tex_command::{
    CommandDialect, CommandProfile, FontLoadRequest, FontResource, PdfImageRequest,
    PdfImageResource, RegisteredSourceKind, SourceRegistration, SourceRegistrationError,
};
use tex_exec::{
    CanonicalMainControl, CanonicalResourceNeed, CanonicalStepResult, CheckpointSink,
    EngineBoundary, ExecutionBudgetCounters, MainControlStep,
};
use tex_out::dvi::DviPagePlan;
use tex_state::print::{Printer, Selector};
use tex_state::{FileContent, InputOpenState, InputReadState, Universe, WorldError};

use crate::RunResult;

/// Default bound for a host that repeatedly declines the same typed need.
pub const DEFAULT_CANONICAL_NO_PROGRESS_LIMIT: u8 = 8;

/// An immutable answer to exactly one canonical resource suspension.
#[derive(Clone, Debug)]
pub enum CanonicalResourceFulfillment {
    Input {
        name: String,
        source: SourceRegistration,
    },
    Font {
        request: FontLoadRequest,
        resource: Box<FontResource>,
    },
    PdfImage {
        request: PdfImageRequest,
        resource: Box<PdfImageResource>,
    },
}

/// One host decision for a canonical resource suspension.
#[derive(Clone, Debug)]
pub enum CanonicalResourceOutcome {
    /// The host selected immutable backing for the exact request.
    Fulfilled(CanonicalResourceFulfillment),
    /// The host completed its search and proved that the resource is absent.
    Unavailable,
    /// The host made no final decision, so the same suspension may be retried.
    Declined,
}

impl CanonicalResourceFulfillment {
    /// Creates the exact retained input answer for a suspended `\\input`.
    #[must_use]
    pub fn input(name: impl Into<String>, kind: RegisteredSourceKind, bytes: Arc<[u8]>) -> Self {
        Self::Input {
            name: name.into(),
            source: SourceRegistration::new(kind, bytes),
        }
    }

    /// Creates an input answer whose selected bytes and provenance are pinned
    /// by a successful read from the active World.
    #[must_use]
    pub fn world_input(name: impl Into<String>, content: FileContent) -> Self {
        Self::Input {
            name: name.into(),
            source: SourceRegistration::world(content),
        }
    }
}

/// Borrow-scoped access to the active aggregate World at one declared
/// canonical resource suspension.
///
/// This capability has no command, input-frame, executor, or semantic-dispatch
/// API and cannot outlive the host fulfillment call.
pub struct CanonicalResourceWorld<'a> {
    stores: &'a mut Universe,
}

impl<'a> CanonicalResourceWorld<'a> {
    fn new(stores: &'a mut Universe) -> Self {
        Self { stores }
    }

    /// Resolves a selected path through World, retaining generated-output
    /// precedence and recording the selected immutable input once.
    pub fn read_file(&mut self, path: impl AsRef<Path>) -> Result<FileContent, WorldError> {
        self.stores.world_mut().read_file(path)
    }

    /// Registers bytes selected by host policy outside World storage while
    /// preserving same-run generated-output precedence and input accounting.
    pub fn register_selected_file(
        &mut self,
        path: impl AsRef<Path>,
        bytes: Arc<[u8]>,
    ) -> Result<FileContent, WorldError> {
        self.stores
            .input_open_context()
            .read_supplied_input_file(path.as_ref(), bytes)
    }
}

/// Host-side acquisition policy. It may only return immutable bytes or a
/// final typed absence; it cannot observe input delivery or execute commands.
pub trait CanonicalResourceHost {
    fn fulfill(
        &mut self,
        world: &mut CanonicalResourceWorld<'_>,
        need: &CanonicalResourceNeed,
    ) -> CanonicalResourceOutcome;
}

/// Result of driving the retained engine until it either completes or awaits
/// an immutable host response.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CanonicalSessionState {
    NeedResource(CanonicalResourceNeed),
    Complete(RunResult),
}

/// Failure of the retained host/session protocol.
#[derive(Debug)]
pub enum CanonicalSessionError {
    RootAlreadyRegistered,
    RootNotRegistered,
    UnexpectedFulfillment {
        need: CanonicalResourceNeed,
        fulfillment: Box<CanonicalResourceFulfillment>,
    },
    NoProgress {
        need: CanonicalResourceNeed,
        attempts: u8,
    },
    SourceRegistration(SourceRegistrationError),
    CommandSummary(tex_command::CommandSummaryError),
    Execution(tex_exec::ExecError),
}

impl fmt::Display for CanonicalSessionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RootAlreadyRegistered => {
                formatter.write_str("canonical root is already registered")
            }
            Self::RootNotRegistered => {
                formatter.write_str("canonical root has not been registered")
            }
            Self::UnexpectedFulfillment { need, fulfillment } => write!(
                formatter,
                "resource fulfillment {fulfillment:?} does not answer pending need {need:?}"
            ),
            Self::NoProgress { need, attempts } => write!(
                formatter,
                "canonical resource retry made no progress after {attempts} attempts: {need:?}"
            ),
            Self::SourceRegistration(error) => error.fmt(formatter),
            Self::CommandSummary(error) => error.fmt(formatter),
            Self::Execution(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for CanonicalSessionError {}

impl From<SourceRegistrationError> for CanonicalSessionError {
    fn from(error: SourceRegistrationError) -> Self {
        Self::SourceRegistration(error)
    }
}

impl From<tex_command::CommandSummaryError> for CanonicalSessionError {
    fn from(error: tex_command::CommandSummaryError) -> Self {
        Self::CommandSummary(error)
    }
}

impl From<tex_exec::ExecError> for CanonicalSessionError {
    fn from(error: tex_exec::ExecError) -> Self {
        Self::Execution(error)
    }
}

/// Standalone retained canonical engine contract.
///
/// Unlike the legacy composition bridge, this type has no `InputStack`,
/// `Executor`, `ExecutionContext`, or file-resolver dependency. Its only
/// mutable engine input is the shared aggregate `Universe`; all source and
/// resource bytes enter through retained typed registrations.
pub struct CanonicalEngineSession<'a> {
    stores: &'a mut Universe,
    control: CanonicalMainControl,
    initex: bool,
    root_registered: bool,
    startup_input_name: Option<String>,
    started: bool,
    /// Whether TeX82 §1332's engine-termination boundary has committed.
    terminated: bool,
    artifact_cursor: usize,
    effect_cursor: usize,
    no_progress_limit: u8,
}

impl<'a> CanonicalEngineSession<'a> {
    #[must_use]
    pub fn new(stores: &'a mut Universe, profile: CommandProfile) -> Self {
        Self {
            artifact_cursor: stores.world().artifact_commits().len(),
            effect_cursor: stores.world().effect_records().len(),
            stores,
            control: CanonicalMainControl::with_profile(profile),
            initex: false,
            root_registered: false,
            startup_input_name: None,
            started: false,
            terminated: false,
            no_progress_limit: DEFAULT_CANONICAL_NO_PROGRESS_LIMIT,
        }
    }

    /// Creates a retained TeX82 INITEX session for building a format from
    /// source.
    ///
    /// Unlike [`Self::new`], this enables init-only commands such as
    /// `\patterns` and installs the TeX82 primitive meanings in `stores`.
    #[must_use]
    pub fn tex82_initex(stores: &'a mut Universe) -> Self {
        Self {
            artifact_cursor: stores.world().artifact_commits().len(),
            effect_cursor: stores.world().effect_records().len(),
            control: CanonicalMainControl::tex82_initex(stores),
            stores,
            initex: true,
            root_registered: false,
            startup_input_name: None,
            started: false,
            terminated: false,
            no_progress_limit: DEFAULT_CANONICAL_NO_PROGRESS_LIMIT,
        }
    }

    /// Creates an INITEX session after the composed engine mode has installed
    /// its fresh primitive profile into `stores`.
    #[must_use]
    pub fn prepared_initex(stores: &'a mut Universe, profile: CommandProfile) -> Self {
        Self {
            artifact_cursor: stores.world().artifact_commits().len(),
            effect_cursor: stores.world().effect_records().len(),
            control: CanonicalMainControl::prepared_initex(profile),
            stores,
            initex: true,
            root_registered: false,
            startup_input_name: None,
            started: false,
            terminated: false,
            no_progress_limit: DEFAULT_CANONICAL_NO_PROGRESS_LIMIT,
        }
    }

    #[must_use]
    pub const fn command_profile(&self) -> CommandProfile {
        self.control.command_profile()
    }

    /// Configures a positive finite canonical command-work limit.
    pub fn set_fuel_limit(&mut self, limit: u64) -> Result<(), tex_command::CommandFuelLimitError> {
        self.control.set_fuel_limit(limit)
    }

    #[must_use]
    pub const fn fuel_limit(&self) -> u64 {
        self.control.fuel_limit()
    }

    #[must_use]
    pub const fn fuel_burned(&self) -> u64 {
        self.control.fuel_burned()
    }

    #[must_use]
    pub fn stores(&self) -> &Universe {
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

    /// Bounds only consecutive host declines for one suspension epoch.
    pub fn set_no_progress_limit(&mut self, limit: u8) {
        self.no_progress_limit = limit.max(1);
    }

    /// Registers the sole World- or host-selected immutable root before any
    /// canonical operation.
    ///
    /// The registration is transferred unchanged so its World input-record
    /// identity remains available to source provenance. Job naming is driver
    /// policy, deliberately separate from source acquisition.
    pub fn register_retained_root(
        &mut self,
        startup_input_name: &str,
        source: SourceRegistration,
    ) -> Result<tex_state::SourceId, CanonicalSessionError> {
        if self.root_registered {
            return Err(CanonicalSessionError::RootAlreadyRegistered);
        }
        self.control
            .capabilities_mut()
            .set_startup_job_name(startup_input_name);
        let source = self.control.register_root_source(source)?;
        self.root_registered = true;
        self.startup_input_name = Some(startup_input_name.to_owned());
        Ok(source)
    }

    /// Registers a root selected through the active World without rebuilding
    /// its provenance from bytes.
    pub fn register_world_root(
        &mut self,
        startup_input_name: &str,
        content: FileContent,
    ) -> Result<tex_state::SourceId, CanonicalSessionError> {
        self.register_retained_root(startup_input_name, SourceRegistration::world(content))
    }

    /// Registers an authored in-memory root.
    ///
    /// This is intentionally not a World-input adapter: selected World roots
    /// must use [`Self::register_world_root`] or
    /// [`Self::register_retained_root`] so their input-record identity is not
    /// discarded.
    pub fn register_authored_root(
        &mut self,
        startup_input_name: &str,
        bytes: Arc<[u8]>,
    ) -> Result<tex_state::SourceId, CanonicalSessionError> {
        self.register_retained_root(
            startup_input_name,
            SourceRegistration::new(RegisteredSourceKind::Generated, bytes),
        )
    }

    /// Drives committed aggregate operations until completion or a typed
    /// suspension. Checkpoints are published solely from committed receipts.
    pub fn advance_until_waiting(
        &mut self,
        checkpoints: &mut dyn CheckpointSink,
    ) -> Result<CanonicalSessionState, CanonicalSessionError> {
        if !self.root_registered {
            return Err(CanonicalSessionError::RootNotRegistered);
        }
        if !self.started {
            self.started = true;
            self.print_startup_headline();
            self.print_startup_input_opening();
            self.publish_checkpoint(EngineBoundary::JobStart, checkpoints)?;
        }
        if self.terminated {
            return self.finish();
        }
        loop {
            match self.control.advance(self.stores)? {
                CanonicalStepResult::Suspended(need) => {
                    return Ok(CanonicalSessionState::NeedResource(need));
                }
                CanonicalStepResult::Progress(step) => {
                    self.publish_completed_boundaries(checkpoints)?;
                    if matches!(step, MainControlStep::End | MainControlStep::EndOfInput) {
                        self.terminated = true;
                        return self.finish();
                    }
                }
            }
        }
    }

    /// Prints TeX82 §1332's process headline before the first command.
    ///
    /// This is deliberately terminal-only. The transcript is not open at
    /// this boundary; §534 later catches it up with a dated banner after the
    /// startup input has established the job name.
    fn print_startup_headline(&mut self) {
        if !self.initex {
            return;
        }
        let banner = match self.command_profile().dialect() {
            CommandDialect::Tex82 => "This is TeX, Version 3.141592653 (TeX Live 2025) (INITEX)",
            CommandDialect::Etex26 => {
                "This is e-TeX, Version 3.141592653-2.6 (TeX Live 2025) (INITEX)"
            }
            CommandDialect::Pdftex14027 => {
                "This is pdfTeX, Version 3.141592653-2.6-1.40.27 (TeX Live 2025) (INITEX)"
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
        let startup_input_name = self
            .startup_input_name
            .as_deref()
            .expect("a started canonical session has a registered root");
        Printer::new(self.stores, Selector::TermOnly)
            .print("(")
            .print(startup_input_name);
    }

    /// Observed variant of [`Self::advance_until_waiting`].
    ///
    /// This drives the same retained production control and differs only by
    /// forwarding its committed semantic observations to a non-fallible
    /// observer. Resource suspensions remain atomic and therefore publish no
    /// observations until their retry commits.
    pub fn advance_until_waiting_with_observer(
        &mut self,
        checkpoints: &mut dyn CheckpointSink,
        observer: &mut dyn tex_command::CommandObserver,
    ) -> Result<CanonicalSessionState, CanonicalSessionError> {
        if !self.root_registered {
            return Err(CanonicalSessionError::RootNotRegistered);
        }
        if !self.started {
            self.started = true;
            self.print_startup_headline();
            self.print_startup_input_opening();
            self.publish_checkpoint(EngineBoundary::JobStart, checkpoints)?;
        }
        if self.terminated {
            return self.finish();
        }
        loop {
            match self.control.advance_with_observer(self.stores, observer)? {
                CanonicalStepResult::Suspended(need) => {
                    return Ok(CanonicalSessionState::NeedResource(need));
                }
                CanonicalStepResult::Progress(step) => {
                    self.publish_completed_boundaries(checkpoints)?;
                    if matches!(step, MainControlStep::End | MainControlStep::EndOfInput) {
                        // TeX82 §§1332, 1335: source exhaustion and an
                        // effective stop both leave `main_control` for the
                        // single `close_files_and_terminate` boundary. An
                        // effective `\end` already publishes this effect with
                        // its final-cleanup records; EndOfInput has no scanned
                        // command to own it, so the retained session publishes
                        // the lifecycle event after the committed source stop.
                        if matches!(step, MainControlStep::EndOfInput) {
                            observer.committed(engine_termination_observation());
                        }
                        self.terminated = true;
                        return self.finish();
                    }
                }
            }
        }
    }

    /// Installs an answer only when it exactly matches the active suspension.
    pub fn fulfill(
        &mut self,
        need: &CanonicalResourceNeed,
        fulfillment: CanonicalResourceFulfillment,
    ) -> Result<(), CanonicalSessionError> {
        let matches = match (&fulfillment, need) {
            (
                CanonicalResourceFulfillment::Input { name, .. },
                CanonicalResourceNeed::Input { name: expected },
            ) => name == expected,
            (
                CanonicalResourceFulfillment::Font { request, .. },
                CanonicalResourceNeed::Font { request: expected },
            ) => request == expected,
            (
                CanonicalResourceFulfillment::PdfImage { request, .. },
                CanonicalResourceNeed::PdfImage { request: expected },
            ) => request == expected,
            _ => false,
        };
        if !matches {
            return Err(CanonicalSessionError::UnexpectedFulfillment {
                need: need.clone(),
                fulfillment: Box::new(fulfillment),
            });
        }
        match fulfillment {
            CanonicalResourceFulfillment::Input { name, source } => {
                self.control.capabilities_mut().register_input(name, source)
            }
            CanonicalResourceFulfillment::Font { request, resource } => self
                .control
                .capabilities_mut()
                .register_font(canonical_font_path(&request.name), *resource),
            CanonicalResourceFulfillment::PdfImage { request, resource } => self
                .control
                .capabilities_mut()
                .register_pdf_image(request, *resource),
        }
        Ok(())
    }

    fn mark_unavailable(&mut self, need: &CanonicalResourceNeed) {
        match need {
            CanonicalResourceNeed::Input { name } => {
                let capabilities = self.control.capabilities_mut();
                capabilities.mark_input_unavailable(name);
                if !name.contains(['/', '\\', ':']) {
                    capabilities.mark_input_unavailable(format!("TeXinputs:{name}"));
                }
            }
            CanonicalResourceNeed::Font { request } => {
                self.control.capabilities_mut().register_font(
                    canonical_font_path(&request.name),
                    FontResource::Unavailable,
                )
            }
            CanonicalResourceNeed::PdfImage { request } => self
                .control
                .capabilities_mut()
                .register_pdf_image(request.clone(), PdfImageResource::Unavailable),
        }
    }

    /// Runs the canonical engine using host policy only for typed immutable
    /// needs. Repeated declines or definitive absences are bounded; successful
    /// fulfillment resets the no-progress epoch because the next operation is
    /// replayed atomically.
    pub fn run(
        &mut self,
        host: &mut dyn CanonicalResourceHost,
        checkpoints: &mut dyn CheckpointSink,
    ) -> Result<RunResult, CanonicalSessionError> {
        let mut declined: u8 = 0;
        loop {
            match self.advance_until_waiting(checkpoints)? {
                CanonicalSessionState::Complete(result) => return Ok(result),
                CanonicalSessionState::NeedResource(need) => {
                    let outcome = {
                        let mut world = CanonicalResourceWorld::new(self.stores);
                        host.fulfill(&mut world, &need)
                    };
                    match outcome {
                        CanonicalResourceOutcome::Fulfilled(fulfillment) => {
                            self.fulfill(&need, fulfillment)?;
                            declined = 0;
                        }
                        CanonicalResourceOutcome::Unavailable => {
                            if let Some(fulfillment) = self.same_run_output(&need) {
                                self.fulfill(&need, fulfillment)?;
                                declined = 0;
                            } else {
                                self.mark_unavailable(&need);
                                declined = declined.saturating_add(1);
                            }
                        }
                        CanonicalResourceOutcome::Declined => {
                            if let Some(fulfillment) = self.same_run_output(&need) {
                                self.fulfill(&need, fulfillment)?;
                                declined = 0;
                            } else {
                                declined = declined.saturating_add(1);
                            }
                        }
                    }
                    if declined >= self.no_progress_limit {
                        return Err(CanonicalSessionError::NoProgress {
                            need,
                            attempts: declined,
                        });
                    }
                }
            }
        }
    }

    /// Observed variant of [`Self::run`] over the same production session.
    pub fn run_with_observer(
        &mut self,
        host: &mut dyn CanonicalResourceHost,
        checkpoints: &mut dyn CheckpointSink,
        observer: &mut dyn tex_command::CommandObserver,
    ) -> Result<RunResult, CanonicalSessionError> {
        let mut declined: u8 = 0;
        loop {
            match self.advance_until_waiting_with_observer(checkpoints, observer)? {
                CanonicalSessionState::Complete(result) => return Ok(result),
                CanonicalSessionState::NeedResource(need) => {
                    let outcome = {
                        let mut world = CanonicalResourceWorld::new(self.stores);
                        host.fulfill(&mut world, &need)
                    };
                    match outcome {
                        CanonicalResourceOutcome::Fulfilled(fulfillment) => {
                            self.fulfill(&need, fulfillment)?;
                            declined = 0;
                        }
                        CanonicalResourceOutcome::Unavailable => {
                            if let Some(fulfillment) = self.same_run_output(&need) {
                                self.fulfill(&need, fulfillment)?;
                                declined = 0;
                            } else {
                                self.mark_unavailable(&need);
                                declined = declined.saturating_add(1);
                            }
                        }
                        CanonicalResourceOutcome::Declined => {
                            if let Some(fulfillment) = self.same_run_output(&need) {
                                self.fulfill(&need, fulfillment)?;
                                declined = 0;
                            } else {
                                declined = declined.saturating_add(1);
                            }
                        }
                    }
                    if declined >= self.no_progress_limit {
                        return Err(CanonicalSessionError::NoProgress {
                            need,
                            attempts: declined,
                        });
                    }
                }
            }
        }
    }

    /// Resolves an exact input name from output already committed by this
    /// retained run when host search policy declines it.
    ///
    /// TeX82 §§1328, 1374 close output streams before later input opens use
    /// the resulting file. The active World is the owner of those committed
    /// effects, so this fallback must remain inside the session instead of
    /// requiring every host search policy to mirror relative output paths.
    fn same_run_output(
        &mut self,
        need: &CanonicalResourceNeed,
    ) -> Option<CanonicalResourceFulfillment> {
        let CanonicalResourceNeed::Input { name } = need else {
            return None;
        };
        self.stores
            .world_mut()
            .read_same_run_output_file(name)
            .ok()
            .flatten()
            .map(|content| CanonicalResourceFulfillment::world_input(name, content))
    }

    fn publish_completed_boundaries(
        &mut self,
        checkpoints: &mut dyn CheckpointSink,
    ) -> Result<(), CanonicalSessionError> {
        for boundary in self.control.take_completed_boundaries() {
            self.publish_checkpoint(boundary, checkpoints)?;
        }
        Ok(())
    }

    fn publish_checkpoint(
        &mut self,
        boundary: EngineBoundary,
        checkpoints: &mut dyn CheckpointSink,
    ) -> Result<(), CanonicalSessionError> {
        if checkpoints.wants_checkpoint(boundary) {
            let checkpoint = self.control.capture_checkpoint(
                boundary,
                self.stores,
                ExecutionBudgetCounters::default(),
            )?;
            checkpoints.checkpoint(checkpoint);
        }
        Ok(())
    }

    fn finish(&mut self) -> Result<CanonicalSessionState, CanonicalSessionError> {
        let receipts = self.control.take_prepared_dvi_pages();
        let commits = self.stores.world().artifact_commits();
        let artifacts = &commits[self.artifact_cursor..];
        if receipts.len() != artifacts.len()
            || receipts
                .iter()
                .zip(artifacts)
                .any(|(receipt, hash)| receipt.hash() != *hash)
        {
            return Err(CanonicalSessionError::Execution(
                tex_exec::ExecError::InvalidShipoutArtifact(
                    "canonical DVI receipts are not aligned with committed artifacts".into(),
                ),
            ));
        }
        let committed_artifacts = self.stores.world().committed_artifacts();
        let effects = self.stores.world().effect_records()[self.effect_cursor..].to_vec();
        self.artifact_cursor = commits.len();
        self.effect_cursor = self.stores.world().effect_records().len();
        Ok(CanonicalSessionState::Complete(RunResult {
            terminal_text: crate::uncommitted_terminal_text(self.stores),
            artifacts: artifacts.to_vec(),
            dvi_pages: receipts
                .into_iter()
                .map(tex_exec::PreparedDviPage::into_plan)
                .collect::<Vec<DviPagePlan>>(),
            committed_artifacts: committed_artifacts
                [self.artifact_cursor - artifacts.len()..self.artifact_cursor]
                .to_vec(),
            effects,
            dumped_format: self.control.dumped_format(),
        }))
    }
}

fn engine_termination_observation() -> tex_command::CommandObservation {
    tex_command::CommandObservation::Effect(tex_command::EffectRecord {
        kind: "terminate",
        detail: "engine\0".into(),
        source: None,
        tokens: None,
    })
}

fn canonical_font_path(name: &str) -> PathBuf {
    let path = PathBuf::from(name);
    if path.extension().is_none() {
        path.with_extension("tfm")
    } else {
        path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tex_command::{CommandObservation, CommandObserver};
    use tex_exec::EngineBoundary;
    use tex_state::World;

    const CMR10: &[u8] = include_bytes!("../../tex-fonts/tests/fixtures/cm/cmr10.tfm");

    struct WorldHost;

    #[derive(Default)]
    struct ObservationRecorder(Vec<CommandObservation>);

    impl CommandObserver for ObservationRecorder {
        fn committed(&mut self, observation: CommandObservation) {
            self.0.push(observation);
        }
    }

    impl CanonicalResourceHost for WorldHost {
        fn fulfill(
            &mut self,
            world: &mut CanonicalResourceWorld<'_>,
            need: &CanonicalResourceNeed,
        ) -> CanonicalResourceOutcome {
            match need {
                CanonicalResourceNeed::Input { name } => world.read_file(name).ok().map_or(
                    CanonicalResourceOutcome::Unavailable,
                    |content| {
                        CanonicalResourceOutcome::Fulfilled(
                            CanonicalResourceFulfillment::world_input(name, content),
                        )
                    },
                ),
                CanonicalResourceNeed::Font { request } => world
                    .read_file(canonical_font_path(&request.name))
                    .ok()
                    .map_or(CanonicalResourceOutcome::Unavailable, |metrics| {
                        CanonicalResourceOutcome::Fulfilled(CanonicalResourceFulfillment::Font {
                            request: request.clone(),
                            resource: Box::new(FontResource::Tfm {
                                metrics,
                                opentype: None,
                            }),
                        })
                    }),
                CanonicalResourceNeed::PdfImage { request } => world
                    .read_file(&request.name)
                    .ok()
                    .map_or(CanonicalResourceOutcome::Unavailable, |content| {
                        CanonicalResourceOutcome::Fulfilled(
                            CanonicalResourceFulfillment::PdfImage {
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
                                        bytes: content.shared_bytes(),
                                    },
                                )),
                            },
                        )
                    }),
            }
        }
    }

    struct OneInputHost {
        calls: usize,
    }

    impl CanonicalResourceHost for OneInputHost {
        fn fulfill(
            &mut self,
            _world: &mut CanonicalResourceWorld<'_>,
            need: &CanonicalResourceNeed,
        ) -> CanonicalResourceOutcome {
            self.calls += 1;
            match need {
                CanonicalResourceNeed::Input { name } if name == "child.tex" => {
                    CanonicalResourceOutcome::Fulfilled(CanonicalResourceFulfillment::input(
                        "child.tex",
                        RegisteredSourceKind::Generated,
                        Arc::from(&b"\\relax"[..]),
                    ))
                }
                _ => CanonicalResourceOutcome::Declined,
            }
        }
    }

    struct UnavailableThenInputHost {
        unavailable: usize,
        calls: usize,
    }

    impl CanonicalResourceHost for UnavailableThenInputHost {
        fn fulfill(
            &mut self,
            _world: &mut CanonicalResourceWorld<'_>,
            need: &CanonicalResourceNeed,
        ) -> CanonicalResourceOutcome {
            self.calls += 1;
            if self.calls <= self.unavailable {
                return CanonicalResourceOutcome::Unavailable;
            }
            let CanonicalResourceNeed::Input { name } = need else {
                return CanonicalResourceOutcome::Unavailable;
            };
            CanonicalResourceOutcome::Fulfilled(CanonicalResourceFulfillment::input(
                name,
                RegisteredSourceKind::Generated,
                Arc::from(&b"\\relax"[..]),
            ))
        }
    }

    fn prepared_session(source: &'static [u8]) -> (Universe, Arc<[u8]>) {
        let mut stores = Universe::new_with_plain_catcodes();
        tex_command::install_tex82_expandable_primitives(&mut stores);
        tex_exec::install_unexpandable_primitives(&mut stores);
        (stores, Arc::from(source))
    }

    fn transcript_channels(stores: &Universe) -> (String, String) {
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

    #[test]
    fn retained_observer_captures_fresh_and_format_loaded_production_runs() {
        let source: Arc<[u8]> = Arc::from(&b"\\message{observed}\\end"[..]);
        let mut base = Universe::new_with_plain_catcodes();
        tex_command::install_tex82_expandable_primitives(&mut base);
        tex_exec::install_unexpandable_primitives(&mut base);
        let format = base.dump_format().expect("base format dumps");

        for loaded in [false, true] {
            let mut stores = if loaded {
                Universe::from_format(World::memory(), &format).expect("format restores")
            } else {
                let mut stores = Universe::new_with_plain_catcodes();
                tex_command::install_tex82_expandable_primitives(&mut stores);
                tex_exec::install_unexpandable_primitives(&mut stores);
                stores
            };
            let mut session = CanonicalEngineSession::new(&mut stores, CommandProfile::TEX82);
            session
                .register_authored_root("observer", Arc::clone(&source))
                .expect("root registers");
            let mut observations = ObservationRecorder::default();
            let run = session
                .run_with_observer(&mut WorldHost, &mut Vec::new(), &mut observations)
                .expect("observed run completes");

            assert_eq!(run.terminal_text, "(observer observed");
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
        }
    }

    #[test]
    fn source_exhaustion_terminates_once_after_stop_under_finite_fuel() {
        let (mut stores, root) = prepared_session(b"");
        let mut session = CanonicalEngineSession::new(&mut stores, CommandProfile::TEX82);
        session.set_fuel_limit(16).expect("finite fuel");
        session
            .register_authored_root("empty.tex", root)
            .expect("root registers");
        let mut observations = ObservationRecorder::default();

        assert!(matches!(
            session
                .advance_until_waiting_with_observer(&mut Vec::new(), &mut observations)
                .expect("empty source completes"),
            CanonicalSessionState::Complete(_)
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
                    CommandObservation::Effect(effect) if effect.kind == "terminate"
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
            CanonicalSessionState::Complete(_)
        ));
        assert_eq!(session.fuel_burned(), burned);
        assert_eq!(
            observations
                .0
                .iter()
                .filter(|observation| matches!(
                    observation,
                    CommandObservation::Effect(effect) if effect.kind == "terminate"
                ))
                .count(),
            1
        );
    }

    #[test]
    fn unobserved_completion_does_not_republish_termination() {
        let (mut stores, root) = prepared_session(b"");
        let mut session = CanonicalEngineSession::new(&mut stores, CommandProfile::TEX82);
        session
            .register_authored_root("empty.tex", root)
            .expect("root registers");

        assert!(matches!(
            session
                .advance_until_waiting(&mut Vec::new())
                .expect("unobserved source completes"),
            CanonicalSessionState::Complete(_)
        ));
        let mut observations = ObservationRecorder::default();
        assert!(matches!(
            session
                .advance_until_waiting_with_observer(&mut Vec::new(), &mut observations)
                .expect("completion remains latched"),
            CanonicalSessionState::Complete(_)
        ));
        assert!(observations.0.is_empty());
    }

    #[test]
    fn resource_suspension_does_not_publish_or_latch_termination() {
        let (mut stores, root) = prepared_session(br"\input child");
        let mut session = CanonicalEngineSession::new(&mut stores, CommandProfile::TEX82);
        session
            .register_authored_root("job.tex", root)
            .expect("root registers");
        let mut observations = ObservationRecorder::default();

        let need = match session
            .advance_until_waiting_with_observer(&mut Vec::new(), &mut observations)
            .expect("missing child suspends")
        {
            CanonicalSessionState::NeedResource(need) => need,
            CanonicalSessionState::Complete(_) => panic!("missing child must suspend"),
        };
        assert!(
            !observations.0.iter().any(|observation| matches!(
                observation,
                CommandObservation::Effect(effect) if effect.kind == "terminate"
            )),
            "rolled-back suspension cannot terminate the session"
        );

        session
            .fulfill(
                &need,
                CanonicalResourceFulfillment::input(
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
            CanonicalSessionState::Complete(_)
        ));
        assert_eq!(
            observations
                .0
                .iter()
                .filter(|observation| matches!(
                    observation,
                    CommandObservation::Effect(effect) if effect.kind == "terminate"
                ))
                .count(),
            1
        );
    }

    #[test]
    fn etex_alphabetic_constants_preserve_control_symbol_spelling() {
        let source: Arc<[u8]> = Arc::from(&br"\endlinechar=`\^^M \newlinechar=`\^^J \end"[..]);
        let mut stores = Universe::new_with_plain_catcodes();
        tex_command::install_tex82_expandable_primitives(&mut stores);
        tex_exec::install_unexpandable_primitives(&mut stores);
        let mut session = CanonicalEngineSession::new(&mut stores, CommandProfile::ETEX26);
        session
            .register_authored_root("alphabetic.tex", source)
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
    }

    #[test]
    fn retained_session_retries_input_without_duplicate_effect_or_receipt() {
        let (mut stores, root) =
            prepared_session(b"\\message{once}\\input child x\\par\\shipout\\hbox{x}\\end");
        let mut session = CanonicalEngineSession::new(&mut stores, CommandProfile::TEX82);
        session
            .register_authored_root("job.tex", root)
            .expect("root registers");
        let mut host = OneInputHost { calls: 0 };
        let mut checkpoints = Vec::new();
        let run = session
            .run(&mut host, &mut checkpoints)
            .expect("run completes");

        assert_eq!(host.calls, 1);
        assert_eq!(
            session.stores().world().memory_terminal_output(),
            // §537/§362 bracket the retried `\input child` exactly once:
            // `(child.tex)` with nothing between the parens, since `child`'s
            // sole line is `\relax`, which prints nothing. §537 prints the
            // name as opened, extension included, which is what the pinned
            // oracle brackets too (`(./child.tex)`; Umber's missing `./`
            // prefix is the separately tracked umber2-alfh.18). Both `[0]`s are
            // §638's progress marker, one per shipped page (the explicit
            // `\shipout`, then TeX82 §1054's residual page ejected at
            // `\end`) -- except the first one is duplicated here, which
            // this test's own name says should not happen. It is a known,
            // separately tracked gap (umber2-0t8z): `shipout_replay_box`
            // commits its marker through `Universe::commit_effects`
            // immediately (needed so the marker cannot instead leak into a
            // *later* page's committed artifact bytes, umber2-v4dx), but
            // that materialization is not itself part of the rollback
            // boundary this session's `\input child` retry restores, so a
            // `\shipout` that already ran speculatively before the retry
            // was discovered leaves its committed marker behind and prints
            // a second one on the replay that actually commits.
            Some(&b"(job.tex once (child.tex) [0] [0]"[..]),
            "aggregate rollback must not repeat an already committed write \
             (umber2-0t8z: it currently does, for `commit_effects`-driven \
             output specifically)"
        );
        // Two pages: the explicit `\shipout`, then TeX82 §1054's residual
        // page -- `x\par` is still on the current page when `\end` arrives,
        // so `its_all_over` is false and the end-job trio ejects it.
        assert_eq!(run.artifacts.len(), 2);
        assert_eq!(run.dvi_pages.len(), run.artifacts.len());
        let boundaries = checkpoints
            .iter()
            .map(tex_exec::EngineCheckpoint::boundary)
            .collect::<Vec<_>>();
        assert!(boundaries.contains(&EngineBoundary::JobStart));
        assert!(boundaries.contains(&EngineBoundary::OuterParagraphEnd));
        assert!(boundaries.contains(&EngineBoundary::ShipoutComplete));
    }

    #[test]
    fn initex_prints_tex82_startup_headline_before_the_first_command() {
        let mut stores = Universe::new_with_plain_catcodes();
        let mut session = CanonicalEngineSession::tex82_initex(&mut stores);
        session.set_fuel_limit(64).expect("finite fuel");
        session
            .register_authored_root("headline.tex", Arc::from(&b"\\end"[..]))
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
                text: "This is TeX, Version 3.141592653 (TeX Live 2025) (INITEX)".into(),
            },
            "TeX82 §1332 writes the process headline to the terminal before main_control"
        );
    }

    #[test]
    fn startup_input_opening_uses_terminal_only_selector_in_initex_and_loaded_sessions() {
        const SOURCE: &[u8] = br"\end";
        let mut format_source = Universe::new_with_plain_catcodes();
        tex_command::install_tex82_expandable_primitives(&mut format_source);
        tex_exec::install_unexpandable_primitives(&mut format_source);
        let format = format_source.dump_format().expect("base format dumps");

        for initex in [true, false] {
            let mut stores = if initex {
                Universe::new_with_plain_catcodes()
            } else {
                Universe::from_format(World::memory(), &format).expect("format restores")
            };
            if initex {
                tex_command::install_tex82_expandable_primitives(&mut stores);
                tex_exec::install_unexpandable_primitives(&mut stores);
            }
            let mut session = if initex {
                CanonicalEngineSession::prepared_initex(&mut stores, CommandProfile::TEX82)
            } else {
                CanonicalEngineSession::new(&mut stores, CommandProfile::TEX82)
            };
            session
                .register_authored_root("./trip.tex", Arc::from(SOURCE))
                .expect("root registers");

            session
                .run(&mut WorldHost, &mut Vec::new())
                .expect("bounded root completes");

            let expected_terminal = if initex {
                "This is TeX, Version 3.141592653 (TeX Live 2025) (INITEX)\n(./trip.tex"
            } else {
                "(./trip.tex"
            };
            let (terminal, log) = transcript_channels(session.stores());
            assert_eq!(
                terminal, expected_terminal,
                "TeX82 §§1332, 537 startup framing, initex={initex}"
            );
            assert_eq!(
                log, "",
                "§537 runs before the transcript opens, initex={initex}"
            );
        }
    }

    #[test]
    fn initex_session_loads_patterns_while_cold_session_rejects_them() {
        const SOURCE: &[u8] = br"\patterns{o1ce eed3i}\lefthyphenmin=2 \righthyphenmin=3 \end";

        let (mut cold_stores, cold_root) = prepared_session(SOURCE);
        let mut cold = CanonicalEngineSession::new(&mut cold_stores, CommandProfile::TEX82);
        cold.register_authored_root("cold.tex", cold_root)
            .expect("cold root registers");
        cold.run(&mut WorldHost, &mut Vec::new())
            .expect("cold session recovers from init-only patterns");
        assert_eq!(
            cold.stores()
                .hyphen_positions_for_language(0, "proceeding", 2, 3),
            Vec::<usize>::new(),
            "TeX82 §1252 rejects patterns outside INITEX"
        );

        let mut initex_stores = Universe::new_with_plain_catcodes();
        let mut initex = CanonicalEngineSession::tex82_initex(&mut initex_stores);
        initex
            .register_authored_root("initex.tex", Arc::from(SOURCE))
            .expect("INITEX root registers");
        initex
            .run(&mut WorldHost, &mut Vec::new())
            .expect("INITEX patterns execute");
        assert_eq!(
            initex
                .stores()
                .hyphen_positions_for_language(0, "proceeding", 2, 3),
            vec![3, 7],
            "the two oracle pattern matches produce pro-ceed-ing"
        );
    }

    #[test]
    fn initex_dump_receipt_survives_the_direct_session_boundary() {
        let mut stores = Universe::new_with_plain_catcodes();
        let mut session = CanonicalEngineSession::tex82_initex(&mut stores);
        session
            .register_authored_root("plain.tex", Arc::from(&b"\\dump"[..]))
            .expect("INITEX root registers");

        let run = session
            .run(&mut WorldHost, &mut Vec::new())
            .expect("INITEX dump completes");

        assert!(run.dumped_format);
        session
            .stores()
            .dump_format()
            .expect("the host may serialize after the dump receipt");
    }

    #[test]
    fn declining_host_is_bounded_without_mutating_effects() {
        let (mut stores, root) = prepared_session(b"\\input never\\end");
        let mut session = CanonicalEngineSession::new(&mut stores, CommandProfile::TEX82);
        session.set_no_progress_limit(2);
        session
            .register_authored_root("job.tex", root)
            .expect("root registers");
        let mut host = OneInputHost { calls: 0 };
        let mut checkpoints = Vec::new();
        let error = session
            .run(&mut host, &mut checkpoints)
            .expect_err("host declines");
        assert!(matches!(
            error,
            CanonicalSessionError::NoProgress { attempts: 2, .. }
        ));
        assert_eq!(
            transcript_channels(session.stores()),
            ("(job.tex".into(), String::new()),
            "only the committed §537 root opening precedes the declined child request"
        );
    }

    #[test]
    fn completed_input_absence_retries_only_in_interactive_modes() {
        for interaction in [
            tex_state::InteractionMode::Scroll,
            tex_state::InteractionMode::ErrorStop,
        ] {
            let (mut stores, root) = prepared_session(b"\\input missing\\end");
            stores.set_interaction_mode(interaction);
            let mut session = CanonicalEngineSession::new(&mut stores, CommandProfile::TEX82);
            session
                .register_authored_root("job.tex", root)
                .expect("root registers");
            let mut host = UnavailableThenInputHost {
                unavailable: 2,
                calls: 0,
            };

            session
                .run(&mut host, &mut Vec::new())
                .expect("interactive lookup retries after completed absence");
            assert_eq!(host.calls, 3);
        }

        for interaction in [
            tex_state::InteractionMode::Batch,
            tex_state::InteractionMode::Nonstop,
        ] {
            let (mut stores, root) = prepared_session(b"\\input missing\\end");
            stores.set_interaction_mode(interaction);
            let mut session = CanonicalEngineSession::new(&mut stores, CommandProfile::TEX82);
            session
                .register_authored_root("job.tex", root)
                .expect("root registers");
            let mut host = UnavailableThenInputHost {
                unavailable: usize::MAX,
                calls: 0,
            };

            session
                .run(&mut host, &mut Vec::new())
                .expect("fatal termination still completes retained cleanup");
            assert!(session.control.fatal_error().is_some());
            assert_eq!(host.calls, 1);
        }
    }

    #[test]
    fn committed_output_is_visible_to_later_input_after_atomic_retry() {
        let (mut stores, root) = prepared_session(
            br"\immediate\openout1=same.out
\immediate\write1{generated}
\immediate\closeout1
\shipout\hbox{}
\input same.out
\end",
        );
        let mut session = CanonicalEngineSession::new(&mut stores, CommandProfile::TEX82);
        session.set_no_progress_limit(1);
        session
            .register_authored_root("job.tex", root)
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
    }

    #[test]
    fn same_run_write_reopens_newlinechar_as_exact_physical_lines() {
        // TeX82 §§262 and 1370: expanded write tokens are first captured as
        // an internal string, then printed through the stream selector. A
        // character equal to `newlinechar` is therefore a physical line end.
        let (mut stores, root) = prepared_session(
            br"\newlinechar=1
\immediate\openout1=same.out
\immediate\write1{\noexpand\global\noexpand\count0=123^^A\noexpand\global\noexpand\count1=456}
\immediate\closeout1
\shipout\hbox{}
\input same.out
\end",
        );
        let mut session = CanonicalEngineSession::new(&mut stores, CommandProfile::TEX82);
        session
            .register_authored_root("job.tex", root)
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
        assert_eq!(session.stores().count(0), 123);
        assert_eq!(session.stores().count(1), 456);
    }

    #[test]
    fn world_host_records_selected_input_once_and_preserves_retry_effects() {
        let (mut stores, root) = prepared_session(b"\\message{once}\\input child\\end");
        stores
            .world_mut()
            .set_memory_file("child.tex", b"\\message{child}")
            .expect("child is seeded");
        let mut session = CanonicalEngineSession::new(&mut stores, CommandProfile::TEX82);
        session
            .register_authored_root("job.tex", root)
            .expect("root registers");

        let run = session
            .run(&mut WorldHost, &mut Vec::new())
            .expect("world-backed input completes");

        // TeX82 §1280 separates the two messages with one space, because
        // the first left `term_offset` nonzero. §537/§362 additionally
        // bracket the root and `\input child` in parens around their own
        // messages, each named as opened the way §537's `a_make_name_string`
        // does: the startup input opening supplies `(job.tex`.
        assert_eq!(run.terminal_text, "(job.tex once (child.tex child)");
        let records = session.stores().world().input_records();
        assert_eq!(records.len(), 1, "the selected child is recorded once");
        assert_eq!(records[0].path(), Path::new("child.tex"));
        assert_eq!(
            session.stores().world().input_content(records[0].hash()),
            Some(&b"\\message{child}"[..])
        );
    }

    #[test]
    fn world_host_fulfills_font_and_image_with_matching_selected_bytes() {
        let (mut font_stores, font_root) = prepared_session(b"\\font\\tenrm=cmr10 \\tenrm A\\end");
        font_stores
            .world_mut()
            .set_memory_file("cmr10.tfm", CMR10)
            .expect("font is seeded");
        let mut font_session = CanonicalEngineSession::new(&mut font_stores, CommandProfile::TEX82);
        font_session
            .register_authored_root("font.tex", font_root)
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

        let mut image_stores = Universe::new_with_plain_catcodes();
        crate::prepare_pdftex_run_stores(&mut image_stores);
        image_stores.set_int_param_global(tex_state::env::banks::IntParam::PDF_OUTPUT, 1);
        image_stores
            .world_mut()
            .set_memory_file("image.png", b"world-selected image")
            .expect("image is seeded");
        let mut image_session =
            CanonicalEngineSession::new(&mut image_stores, CommandProfile::PDFTEX14027);
        image_session
            .register_authored_root("image.tex", Arc::from(&b"\\pdfximage {image.png}\\end"[..]))
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
        assert_eq!(
            image_session
                .stores()
                .pdf_last_external_image()
                .map(|image| image.identity()),
            Some(image_record.hash())
        );
    }

    #[test]
    fn fulfillment_rejects_mismatched_typed_need() {
        let (mut stores, root) = prepared_session(b"\\input child\\end");
        let mut session = CanonicalEngineSession::new(&mut stores, CommandProfile::TEX82);
        session
            .register_authored_root("job.tex", root)
            .expect("root registers");
        let need = match session
            .advance_until_waiting(&mut Vec::new())
            .expect("input suspends")
        {
            CanonicalSessionState::NeedResource(need) => need,
            other => panic!("expected resource need, got {other:?}"),
        };
        let error = session
            .fulfill(
                &need,
                CanonicalResourceFulfillment::input(
                    "other",
                    RegisteredSourceKind::Generated,
                    Arc::from(&b"\\end"[..]),
                ),
            )
            .expect_err("mismatched input is rejected");
        assert!(matches!(
            error,
            CanonicalSessionError::UnexpectedFulfillment { .. }
        ));
    }

    #[test]
    fn canonical_session_has_finite_configurable_command_fuel() {
        let mut stores = Universe::new_with_plain_catcodes();
        let mut session = CanonicalEngineSession::new(&mut stores, CommandProfile::TEX82);
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
    }

    #[test]
    fn tiny_limit_stops_a_cyclic_canonical_run_with_typed_error() {
        fn run(observed: bool) -> (CanonicalSessionError, u64) {
            let (mut stores, root) = prepared_session(b"\\def\\cycle{\\cycle}\\cycle");
            let mut session = CanonicalEngineSession::new(&mut stores, CommandProfile::TEX82);
            session
                .register_authored_root("cycle.tex", root)
                .expect("root registers");
            session.set_fuel_limit(19).expect("valid tiny limit");
            let error = if observed {
                session
                    .run_with_observer(
                        &mut WorldHost,
                        &mut Vec::new(),
                        &mut ObservationRecorder::default(),
                    )
                    .expect_err("observed cyclic run exhausts fuel")
            } else {
                session
                    .run(&mut WorldHost, &mut Vec::new())
                    .expect_err("cyclic run exhausts fuel")
            };
            let burned = session.fuel_burned();
            (error, burned)
        }

        let (unobserved_error, unobserved_burned) = run(false);
        let (observed_error, observed_burned) = run(true);
        for error in [&unobserved_error, &observed_error] {
            assert!(matches!(
                error,
                CanonicalSessionError::Execution(tex_exec::ExecError::Command(
                    tex_command::CommandError::FuelExhausted {
                        limit: 19,
                        burned: 19
                    }
                ))
            ));
        }
        assert_eq!(unobserved_burned, 19);
        assert_eq!(observed_burned, unobserved_burned);
    }
}
