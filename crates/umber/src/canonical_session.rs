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
    CommandProfile, FontLoadRequest, FontResource, PdfImageRequest, PdfImageResource,
    RegisteredSourceKind, SourceRegistration, SourceRegistrationError,
};
use tex_exec::{
    CanonicalMainControl, CanonicalResourceNeed, CanonicalStepResult, CheckpointSink,
    EngineBoundary, ExecutionBudgetCounters, MainControlStep,
};
use tex_out::dvi::DviPagePlan;
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
    ) -> Option<CanonicalResourceFulfillment>;
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
        job_name: &str,
        source: SourceRegistration,
    ) -> Result<tex_state::SourceId, CanonicalSessionError> {
        if self.root_registered {
            return Err(CanonicalSessionError::RootAlreadyRegistered);
        }
        self.control
            .capabilities_mut()
            .set_startup_job_name(job_name);
        let source = self.control.register_root_source(source)?;
        self.root_registered = true;
        Ok(source)
    }

    /// Registers a root selected through the active World without rebuilding
    /// its provenance from bytes.
    pub fn register_world_root(
        &mut self,
        job_name: &str,
        content: FileContent,
    ) -> Result<tex_state::SourceId, CanonicalSessionError> {
        self.register_retained_root(job_name, SourceRegistration::world(content))
    }

    /// Registers an authored in-memory root.
    ///
    /// This is intentionally not a World-input adapter: selected World roots
    /// must use [`Self::register_world_root`] or
    /// [`Self::register_retained_root`] so their input-record identity is not
    /// discarded.
    pub fn register_authored_root(
        &mut self,
        job_name: &str,
        bytes: Arc<[u8]>,
    ) -> Result<tex_state::SourceId, CanonicalSessionError> {
        self.register_retained_root(
            job_name,
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
                    let fulfillment = {
                        let mut world = CanonicalResourceWorld::new(self.stores);
                        host.fulfill(&mut world, &need)
                    };
                    let Some(fulfillment) = fulfillment else {
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
            dumped_format: self.control.dumped_format(),
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

    const CMR10: &[u8] = include_bytes!("../../tex-fonts/tests/fixtures/cm/cmr10.tfm");

    struct WorldHost;

    impl CanonicalResourceHost for WorldHost {
        fn fulfill(
            &mut self,
            world: &mut CanonicalResourceWorld<'_>,
            need: &CanonicalResourceNeed,
        ) -> Option<CanonicalResourceFulfillment> {
            match need {
                CanonicalResourceNeed::Input { name } => world
                    .read_file(format!("{name}.tex"))
                    .ok()
                    .map(|content| CanonicalResourceFulfillment::world_input(name, content)),
                CanonicalResourceNeed::Font { request } => world
                    .read_file(canonical_font_path(&request.name))
                    .ok()
                    .map(|metrics| CanonicalResourceFulfillment::Font {
                        request: request.clone(),
                        resource: Box::new(FontResource::Tfm {
                            metrics,
                            opentype: None,
                        }),
                    }),
                CanonicalResourceNeed::PdfImage { request } => world
                    .read_file(&request.name)
                    .ok()
                    .map(|content| CanonicalResourceFulfillment::PdfImage {
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
        let mut stores = Universe::new_with_plain_catcodes();
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
            Some(&b"once"[..]),
            "aggregate rollback must not repeat an already committed write"
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
        assert!(session.stores().world().effect_records().is_empty());
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
        // the first left `term_offset` nonzero.
        assert_eq!(run.terminal_text, "once child");
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
            .register_authored_root("image.tex", Arc::from(&b"\\pdfximage image.png\\end"[..]))
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
}
