//! Retained, host-neutral entry point for canonical TeX command execution.
//!
//! TeX82 §24 (`get_next`) and §25 (`expand`/`get_x_token`) retain input and
//! expansion inside the command machine.  §1030 (`main_control`) consumes the
//! resulting command.  Consequently this host owns only immutable resource
//! registrations and aggregate retry policy; it has no token-delivery API.

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use tex_command::{
    CommandProfile, FontLoadRequest, FontResource, PdfImageRequest, PdfImageResource,
    RegisteredSourceKind, SourceRegistration, SourceRegistrationError,
};
use tex_exec::{
    CanonicalMainControl, CanonicalResourceNeed, CanonicalStepResult, CheckpointSink,
    EngineBoundary, ExecutionBudgetCounters, MainControlStep,
};
use tex_out::dvi::DviPagePlan;
use tex_state::Universe;

use crate::RunResult;

/// Default bound for a host that repeatedly declines the same typed need.
pub const DEFAULT_CANONICAL_NO_PROGRESS_LIMIT: u8 = 8;

/// An immutable answer to exactly one canonical resource suspension.
#[derive(Clone, Debug)]
pub enum CanonicalResourceFulfillment {
    Input {
        name: String,
        kind: RegisteredSourceKind,
        bytes: Arc<[u8]>,
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

impl CanonicalResourceFulfillment {
    /// Creates the exact retained input answer for a suspended `\\input`.
    #[must_use]
    pub fn input(name: impl Into<String>, kind: RegisteredSourceKind, bytes: Arc<[u8]>) -> Self {
        Self::Input {
            name: name.into(),
            kind,
            bytes,
        }
    }
}

/// Host-side acquisition policy. It may only return immutable bytes or a
/// final typed absence; it cannot observe input delivery or execute commands.
pub trait CanonicalResourceHost {
    fn fulfill(&mut self, need: &CanonicalResourceNeed) -> Option<CanonicalResourceFulfillment>;
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
    root_registered: bool,
    started: bool,
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
            root_registered: false,
            started: false,
            no_progress_limit: DEFAULT_CANONICAL_NO_PROGRESS_LIMIT,
        }
    }

    #[must_use]
    pub const fn command_profile(&self) -> CommandProfile {
        self.control.command_profile()
    }

    #[must_use]
    pub fn stores(&self) -> &Universe {
        self.stores
    }

    /// Bounds only consecutive host declines for one suspension epoch.
    pub fn set_no_progress_limit(&mut self, limit: u8) {
        self.no_progress_limit = limit.max(1);
    }

    /// Registers the sole immutable root before any canonical operation.
    pub fn register_root(
        &mut self,
        filename: &str,
        kind: RegisteredSourceKind,
        bytes: Arc<[u8]>,
    ) -> Result<tex_state::SourceId, CanonicalSessionError> {
        if self.root_registered {
            return Err(CanonicalSessionError::RootAlreadyRegistered);
        }
        self.control
            .capabilities_mut()
            .set_startup_job_name(filename);
        let source = self
            .control
            .register_root_source(SourceRegistration::new(kind, bytes))?;
        self.root_registered = true;
        Ok(source)
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
            self.publish_checkpoint(EngineBoundary::JobStart, checkpoints)?;
        }
        loop {
            match self.control.advance(self.stores)? {
                CanonicalStepResult::Suspended(need) => {
                    return Ok(CanonicalSessionState::NeedResource(need));
                }
                CanonicalStepResult::Progress(step) => {
                    self.publish_completed_boundaries(checkpoints)?;
                    if matches!(step, MainControlStep::End | MainControlStep::EndOfInput) {
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
            CanonicalResourceFulfillment::Input { name, kind, bytes } => self
                .control
                .capabilities_mut()
                .register_input(name, SourceRegistration::new(kind, bytes)),
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

    /// Runs the canonical engine using host policy only for typed immutable
    /// needs. A declining host is bounded; successful fulfillment resets the
    /// no-progress epoch because the next operation is replayed atomically.
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
                    let Some(fulfillment) = host.fulfill(&need) else {
                        declined = declined.saturating_add(1);
                        if declined >= self.no_progress_limit {
                            return Err(CanonicalSessionError::NoProgress {
                                need,
                                attempts: declined,
                            });
                        }
                        continue;
                    };
                    self.fulfill(&need, fulfillment)?;
                    declined = 0;
                }
            }
        }
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
            dumped_format: false,
        }))
    }
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
    use tex_exec::EngineBoundary;

    struct OneInputHost {
        calls: usize,
    }

    impl CanonicalResourceHost for OneInputHost {
        fn fulfill(
            &mut self,
            need: &CanonicalResourceNeed,
        ) -> Option<CanonicalResourceFulfillment> {
            self.calls += 1;
            match need {
                CanonicalResourceNeed::Input { name } if name == "child" => {
                    Some(CanonicalResourceFulfillment::input(
                        "child",
                        RegisteredSourceKind::Generated,
                        Arc::from(&b"\\relax"[..]),
                    ))
                }
                _ => None,
            }
        }
    }

    fn prepared_session(source: &'static [u8]) -> (Universe, Arc<[u8]>) {
        let mut stores = Universe::new();
        tex_expand::install_expandable_primitives(&mut stores);
        tex_exec::install_unexpandable_primitives(&mut stores);
        (stores, Arc::from(source))
    }

    #[test]
    fn retained_session_retries_input_without_duplicate_effect_or_receipt() {
        let (mut stores, root) =
            prepared_session(b"\\message{once}\\input child x\\par\\shipout\\hbox{x}\\end");
        let mut session = CanonicalEngineSession::new(&mut stores, CommandProfile::TEX82);
        session
            .register_root("job.tex", RegisteredSourceKind::Generated, root)
            .expect("root registers");
        let mut host = OneInputHost { calls: 0 };
        let mut checkpoints = Vec::new();
        let run = session
            .run(&mut host, &mut checkpoints)
            .expect("run completes");

        assert_eq!(host.calls, 1);
        assert_eq!(
            session.stores().world().memory_terminal_output(),
            Some(&b"once"[..]),
            "aggregate rollback must not repeat an already committed write"
        );
        assert_eq!(run.artifacts.len(), 1);
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
    fn declining_host_is_bounded_without_mutating_effects() {
        let (mut stores, root) = prepared_session(b"\\input never\\end");
        let mut session = CanonicalEngineSession::new(&mut stores, CommandProfile::TEX82);
        session.set_no_progress_limit(2);
        session
            .register_root("job.tex", RegisteredSourceKind::Generated, root)
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
        assert!(session.stores().world().effect_records().is_empty());
    }
}
