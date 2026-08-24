//! Generic, guarded construction and loaded execution of generated formats.

use std::cell::Cell;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::{
    FormatCacheClock, FormatCacheError, FormatCacheIdentity, FormatCacheStore, FormatEngineMode,
    FormatFingerprint, FormatFixtureIdentity, ValidatedFormatImage,
};
use sha2::{Digest, Sha256};
use tex_command::{
    CommandObserver, FileEnquiryResource, FontResource, RegisteredSourceKind, SourceRegistration,
};
use tex_exec::{CheckpointSink, ResourceNeed};
use tex_observe::{LiveSessionTranslator, LiveSource};
use tex_oracle::SchemaVersion;
use tex_oracle::{OracleBundle, decode_oracle_bundle, encode_oracle_bundle};
use tex_state::{
    FormatMaterializationConfig, JobClock, ProvenanceBudgets, ProvenanceDemand, World,
    with_format_destination,
};

use crate::{
    EngineMode, EngineSession, ResourceFulfillment, ResourceHost, ResourceOutcome, ResourceWorld,
    RunResult, SessionError,
};

const IDENTITY_DOMAIN: &[u8] = b"umber.loaded-format-fixture.v2\0";
// Version 11 carries TeX82 §§259/356/372 hash occupancy; version 10 carries
// §§125--130 allocator-coordinate extents; version 16 carries e-TeX change
// 17.11's pre-eqtb enhancement reset; version 15 carries tex.web §241's
// retained-INITEX clock initialization; version 14 carries Web2C
// [54/SyncTeX]'s extended parameter/pool image; version 9 carries the canonical
// string-pool construction lifecycle; version 7
// carried the earlier §200 token-list-head approximation; version 6
// introduced the serialized baseline field; version 5
// includes §§785/1038's raw character-loop delivery inside alignment cells;
// version 4 added §478's direct `the_toks` delivery and version 3 added §962's
// zero/nonletter edge-of-word pattern semantics.
// Bump whenever the construction engine or its detached evidence semantics
// change. Persistent entries contain both the format image and the evidence
// produced by that exact construction episode; accepting an entry from an
// older producer would bypass the current engine entirely.
const PRODUCER_CONTRACT_VERSION: u32 = 17;
// Version 2 carries the producing source identity on geometry observations.
const COMMAND_OBSERVATION_SCHEMA_VERSION: u32 = 2;

/// Positive cumulative limits for one format construction or loaded job.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FormatGenerationGuards {
    pub command_fuel: u64,
    pub wall_time: Duration,
    pub resident_bytes: u64,
}

impl FormatGenerationGuards {
    pub fn validate(self) -> Result<Self, FormatFixtureError> {
        if self.command_fuel == 0 || self.wall_time.is_zero() || self.resident_bytes == 0 {
            return Err(FormatFixtureError::UnboundedGuard);
        }
        Ok(self)
    }
}

/// One immutable member of a format recipe's typed resource closure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FormatResource {
    Input {
        logical_name: String,
        source_kind: RegisteredSourceKind,
        /// Recipe-owned transport bytes; materialization creates session-local storage.
        bytes: Vec<u8>,
    },
    Tfm {
        logical_name: String,
        /// Recipe-owned transport bytes; no live file or cache owner is retained.
        bytes: Vec<u8>,
    },
}

/// One immutable job-local resource supplied after a format is loaded.
///
/// These resources belong to the execution episode, not to the format recipe
/// or its cache identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LoadedFormatResource {
    Input {
        logical_name: String,
        resolved_name: String,
        source_kind: RegisteredSourceKind,
        bytes: Vec<u8>,
    },
    Tfm {
        logical_name: String,
        bytes: Vec<u8>,
    },
}

/// One box register selected for handle-free terminal projection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LoadedBoxOutlineDemand {
    pub register: u16,
    pub depth: u8,
}

/// Explicit terminal projection selected by a loaded-format caller.
///
/// Empty demand is intentionally free: ordinary format users do not walk
/// register banks, node arenas, or output channels merely because a job ends.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LoadedFormatProjectionDemand {
    pub count_registers: Vec<u16>,
    pub box_outlines: Vec<LoadedBoxOutlineDemand>,
    pub channels: bool,
}

/// One source-independent node in a detached box outline.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DetachedNodeOutlineEntry {
    pub path: Vec<usize>,
    pub kind: tex_state::node::NodeKind,
}

/// One requested box register after the loaded generation has quiesced.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DetachedBoxOutline {
    pub register: u16,
    pub nodes: Option<Vec<DetachedNodeOutlineEntry>>,
}

/// One materialized numbered-stream output owned by the loaded run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoadedFormatOutput {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
}

/// Exact memory-world channels captured before the loaded generation drops.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LoadedFormatChannels {
    pub terminal: Vec<u8>,
    pub log: Vec<u8>,
    /// Unpublished printable suffix captured before final stream publication.
    pub pending_effects: Vec<tex_state::EffectRecord>,
    pub outputs: Vec<LoadedFormatOutput>,
}

/// Sparse, handle-free projection of explicitly requested terminal state.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LoadedFormatProjection {
    pub counts: Vec<(u16, i32)>,
    pub boxes: Vec<DetachedBoxOutline>,
    pub channels: Option<LoadedFormatChannels>,
}

/// Complete host-independent recipe for one generated format.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FormatRecipe {
    pub engine: EngineMode,
    /// tex.web §934's configured exception-table capacity (`hyph_size`).
    pub hyphenation_exception_capacity: usize,
    /// web2c's selected `dump_name`, used by §61's terminal banner.
    pub format_name: String,
    /// The dump job name embedded by TeX82 §1328 and restored for §536's log.
    pub format_ident_name: String,
    pub construction_source_name: String,
    /// Handle-free INITEX input copied into the destination session on use.
    pub construction_source: Vec<u8>,
    pub resources: Vec<FormatResource>,
    /// Stable distribution value identity, never a runtime cache owner.
    pub distribution_identity: Vec<u8>,
    pub clock: JobClock,
    /// Driver-selected interaction for the construction episode.
    pub construction_interaction: tex_state::InteractionMode,
    /// Driver-selected TeX82 §79 diagnostic widths for construction.
    pub construction_error_context_widths: tex_state::print::ErrorContextWidths,
    pub guards: FormatGenerationGuards,
}

impl FormatRecipe {
    /// Hermetic raw TeX82 image: primitives and INITEX state, without Plain.
    #[must_use]
    pub fn raw_tex82() -> Self {
        Self {
            engine: EngineMode::Tex82,
            hyphenation_exception_capacity: 307,
            format_name: "raw-tex82".into(),
            format_ident_name: "raw-tex82".into(),
            construction_source_name: "raw-tex82.ini".into(),
            construction_source: b"\\dump\n".to_vec(),
            resources: Vec::new(),
            distribution_identity: b"repository-raw-tex82-v1".to_vec(),
            clock: JobClock {
                time: 12 * 60,
                second: 0,
                day: 1,
                month: 3,
                year: 2026,
            },
            construction_interaction: tex_state::InteractionMode::ErrorStop,
            construction_error_context_widths: tex_state::print::ErrorContextWidths::default(),
            guards: FormatGenerationGuards {
                command_fuel: 100_000,
                wall_time: Duration::from_secs(10),
                resident_bytes: 512 * 1024 * 1024,
            },
        }
    }

    /// Hermetic raw e-TeX 2.6 image: TeX82 and e-TeX primitives, without Plain.
    #[must_use]
    pub fn raw_etex26() -> Self {
        Self {
            engine: EngineMode::ETex,
            hyphenation_exception_capacity: 307,
            format_name: "raw-etex26".into(),
            format_ident_name: "raw-etex26".into(),
            construction_source_name: "raw-etex26.ini".into(),
            construction_source: b"\\dump\n".to_vec(),
            resources: Vec::new(),
            distribution_identity: b"repository-raw-etex26-v1".to_vec(),
            clock: JobClock {
                time: 12 * 60,
                second: 0,
                day: 1,
                month: 3,
                year: 2026,
            },
            construction_interaction: tex_state::InteractionMode::ErrorStop,
            construction_error_context_widths: tex_state::print::ErrorContextWidths::default(),
            guards: FormatGenerationGuards {
                command_fuel: 100_000,
                wall_time: Duration::from_secs(10),
                resident_bytes: 512 * 1024 * 1024,
            },
        }
    }

    /// Hermetic production pdfTeX 1.40.29 image, without a macro format.
    ///
    /// Construction runs in pdfTeX INITEX mode and terminates only through
    /// this recipe-owned `\dump`.
    #[must_use]
    pub fn production_pdftex14029() -> Self {
        Self {
            engine: EngineMode::PdfTex,
            hyphenation_exception_capacity: 307,
            format_name: "production".into(),
            format_ident_name: "production".into(),
            construction_source_name: "production-pdftex14029.ini".into(),
            construction_source: b"\\dump\n".to_vec(),
            resources: Vec::new(),
            distribution_identity: b"repository-production-pdftex14029-v1".to_vec(),
            clock: JobClock {
                time: 12 * 60,
                second: 0,
                day: 1,
                month: 3,
                year: 2026,
            },
            construction_interaction: tex_state::InteractionMode::ErrorStop,
            construction_error_context_widths: tex_state::print::ErrorContextWidths::default(),
            guards: FormatGenerationGuards {
                command_fuel: 100_000,
                wall_time: Duration::from_secs(10),
                resident_bytes: 512 * 1024 * 1024,
            },
        }
    }

    pub fn identity(&self) -> Result<FormatCacheIdentity, FormatFixtureError> {
        self.guards.validate()?;
        let profile = self.engine.command_profile();
        let semantic = framed_hash(&[
            &tex_state::CHECKPOINT_STATE_HASH_SCHEMA_VERSION.to_le_bytes(),
            &COMMAND_OBSERVATION_SCHEMA_VERSION.to_le_bytes(),
            &profile.to_stable_bytes(),
            &profile.fingerprint().get().to_le_bytes(),
            &(self.hyphenation_exception_capacity as u64).to_le_bytes(),
            &tex_oracle::ORACLE_BUNDLE_SCHEMA.to_le_bytes(),
            &(tex_oracle::MAX_BUNDLE_EVENTS_PER_STREAM as u64).to_le_bytes(),
            &(tex_oracle::MAX_BUNDLE_EVENT_BYTES as u64).to_le_bytes(),
            &(tex_oracle::MAX_BUNDLE_STRING_BYTES as u64).to_le_bytes(),
            &(tex_oracle::MAX_BUNDLE_NESTING_DEPTH as u64).to_le_bytes(),
            &(tex_oracle::MAX_BUNDLE_BYTES as u64).to_le_bytes(),
            &[interaction_tag(self.construction_interaction)],
            &(self.construction_error_context_widths.error_line() as u64).to_le_bytes(),
            &(self.construction_error_context_widths.half_error_line() as u64).to_le_bytes(),
        ]);
        let source = framed_hash(&[
            self.construction_source_name.as_bytes(),
            &self.construction_source,
        ]);
        let resources = resource_closure_hash(&self.resources);
        let closure = framed_hash(&[&source, &resources]);
        let guards = framed_hash(&[
            &self.guards.command_fuel.to_le_bytes(),
            &self.guards.wall_time.as_nanos().to_le_bytes(),
            &self.guards.resident_bytes.to_le_bytes(),
        ]);
        let producer = producer_contract(
            PRODUCER_CONTRACT_VERSION,
            &self.format_name,
            &self.format_ident_name,
        );
        Ok(FormatCacheIdentity::fixture(FormatFixtureIdentity {
            engine_mode: cache_mode(self.engine),
            distribution_snapshot: FormatFingerprint::sha256(&self.distribution_identity),
            format_closure: FormatFingerprint::new(closure),
            source_lock: FormatFingerprint::new(source),
            job_clock: FormatCacheClock {
                time: self.clock.time,
                second: self.clock.second,
                day: self.clock.day,
                month: self.clock.month,
                year: self.clock.year,
            },
            build_configuration: FormatFingerprint::sha256(build_feature_contract()),
            semantic_contract: FormatFingerprint::new(semantic),
            producer_contract: FormatFingerprint::new(producer),
            resource_closure: FormatFingerprint::new(resources),
            generation_guards: FormatFingerprint::new(guards),
        }))
    }
}

/// Validated cached bytes paired with the exact recipe that selected them.
#[derive(Clone, Debug)]
pub struct FormatFixture {
    recipe: FormatRecipe,
    image: ValidatedFormatImage,
    evidence: OracleBundle,
}

impl FormatFixture {
    /// Engine/profile identity authenticated by this fixture's recipe.
    #[must_use]
    pub const fn engine_mode(&self) -> EngineMode {
        self.recipe.engine
    }

    #[must_use]
    pub fn image(&self) -> &[u8] {
        self.image.as_bytes()
    }

    /// Detached construction-only canonical semantic and geometry evidence.
    #[must_use]
    pub fn construction_evidence(&self) -> &OracleBundle {
        &self.evidence
    }

    pub fn load(&self, world: World) -> Result<LoadedFormatFixture, FormatFixtureError> {
        Ok(LoadedFormatFixture {
            recipe: self.recipe.clone(),
            image: self.image.clone(),
            world,
            provenance_demand: ProvenanceDemand::default(),
            interaction_mode: None,
            error_context_widths: None,
        })
    }
}

/// Host-owned recipe for one fresh destination-local materialization.
pub struct LoadedFormatFixture {
    recipe: FormatRecipe,
    image: ValidatedFormatImage,
    world: World,
    provenance_demand: ProvenanceDemand,
    interaction_mode: Option<tex_state::InteractionMode>,
    error_context_widths: Option<tex_state::print::ErrorContextWidths>,
}

pub(crate) struct LoadedRunConfiguration {
    pub guards: FormatGenerationGuards,
    pub engine_binary: tex_exec::EngineBinaryIdentity,
    pub startup_line: String,
    pub completion: tex_exec::RootCompletionPolicy,
    pub projection: LoadedFormatProjectionDemand,
}

impl LoadedFormatFixture {
    /// Selects optional provenance consumers for this fresh loaded job.
    ///
    /// The policy is operational state applied only after format decoding, so
    /// it cannot alter or select the authenticated prepared-format bytes.
    pub(crate) fn with_provenance_demand(mut self, demand: tex_state::ProvenanceDemand) -> Self {
        self.provenance_demand = demand;
        self
    }

    /// Selects the job-local TeX interaction mode after format loading.
    ///
    /// Interaction is runtime control state and is deliberately excluded from
    /// the frozen format image.
    pub fn set_interaction_mode(&mut self, mode: tex_state::InteractionMode) {
        self.interaction_mode = Some(mode);
    }

    /// Selects the job-local TeX error-context widths after format loading.
    ///
    /// These widths are process/driver configuration and are deliberately
    /// excluded from the immutable format image.
    pub fn set_error_context_widths(&mut self, widths: tex_state::print::ErrorContextWidths) {
        self.error_context_widths = Some(widths);
    }

    pub fn run(
        self,
        source_name: &str,
        source: Arc<[u8]>,
        resources: &[LoadedFormatResource],
        observer: &mut dyn CommandObserver,
    ) -> Result<LoadedFormatRun, FormatFixtureError> {
        let guards = self.recipe.guards;
        let engine_binary = self.recipe.engine.binary_identity();
        self.run_configured(
            source_name,
            RegisteredSourceKind::Generated,
            source,
            resources,
            LoadedRunConfiguration {
                guards,
                engine_binary,
                startup_line: source_name.to_owned(),
                completion: tex_exec::RootCompletionPolicy::RequireTeXEnd,
                projection: LoadedFormatProjectionDemand::default(),
            },
            observer,
        )
    }

    pub(crate) fn run_configured(
        self,
        source_name: &str,
        source_kind: RegisteredSourceKind,
        source: Arc<[u8]>,
        resources: &[LoadedFormatResource],
        config: LoadedRunConfiguration,
        observer: &mut dyn CommandObserver,
    ) -> Result<LoadedFormatRun, FormatFixtureError> {
        let guards = config.guards.validate()?;
        let Self {
            recipe,
            image,
            world,
            provenance_demand,
            interaction_mode,
            error_context_widths,
        } = self;
        with_format_destination(crate::engine_interner_budget(), world, |destination| {
            destination.set_provenance_config(FormatMaterializationConfig {
                provenance_demand,
                provenance_budgets: ProvenanceBudgets::default(),
            });
            let staging = destination.stage(image.detached())?;
            destination
                .materialize(staging, |universe| {
                    recipe.engine.install_after_format(universe);
                    if let Some(mode) = interaction_mode {
                        universe.set_interaction_mode(mode);
                    }
                    if let Some(widths) = error_context_widths {
                        universe.set_error_context_widths(widths);
                    }
                    let mut session = EngineSession::new(universe, recipe.engine.command_profile());
                    session.set_preloaded_format(tex_exec::PreloadedFormat {
                        dump_name: recipe.format_name.clone(),
                        format_name: recipe.format_ident_name.clone(),
                        year: recipe.clock.year,
                        month: recipe.clock.month,
                        day: recipe.clock.day,
                    });
                    session.set_engine_binary(config.engine_binary);
                    session.set_fuel_limit(guards.command_fuel)?;
                    let source = tex_command::SourceRegistration::new(source_kind, source)
                        .with_name(format!("./{source_name}"));
                    match config.completion {
                        tex_exec::RootCompletionPolicy::RequireTeXEnd => session
                            .register_retained_root_with_invocation(
                                source_name,
                                &config.startup_line,
                                source,
                            )?,
                        tex_exec::RootCompletionPolicy::StopAtRootEof => session
                            .register_retained_fragment_with_invocation(
                                source_name,
                                &config.startup_line,
                                source,
                            )?,
                    };
                    let checkpoints = GuardCheckpoints::new(guards)?;
                    let mut checkpoint_sink = &checkpoints;
                    let result = session.run_with_observer(
                        &mut LoadedResourceHost::new(resources, &recipe.resources),
                        &mut checkpoint_sink,
                        observer,
                    );
                    let result = finish_guarded_run(result, &checkpoints)?;
                    drop(session);
                    let projection =
                        capture_loaded_projection(universe, &config.projection, config.completion)?;
                    Ok(LoadedFormatRun { result, projection })
                })
                .map_err(|error| {
                    tex_state::FormatError::InvalidState(format!(
                        "format destination publication failed: {error:?}"
                    ))
                })
        })
        .map_err(|error| FormatFixtureError::Format(error.to_string()))?
    }
}

struct LoadedResourceHost<'a> {
    job_resources: &'a [LoadedFormatResource],
    format_resources: &'a [FormatResource],
}

impl<'a> LoadedResourceHost<'a> {
    fn new(
        job_resources: &'a [LoadedFormatResource],
        format_resources: &'a [FormatResource],
    ) -> Self {
        Self {
            job_resources,
            format_resources,
        }
    }
}

impl ResourceHost for LoadedResourceHost<'_> {
    fn fulfill(&mut self, world: &mut ResourceWorld<'_>, need: &ResourceNeed) -> ResourceOutcome {
        match need {
            ResourceNeed::Input { name, .. } => self
                .job_resources
                .iter()
                .find_map(|resource| match resource {
                    LoadedFormatResource::Input {
                        logical_name,
                        resolved_name,
                        source_kind,
                        bytes,
                    } if logical_name == name => {
                        Some(ResourceOutcome::Fulfilled(ResourceFulfillment::Input {
                            name: logical_name.clone(),
                            source: SourceRegistration::new(
                                *source_kind,
                                Arc::<[u8]>::from(bytes.as_slice()),
                            )
                            .with_name(resolved_name.clone()),
                        }))
                    }
                    _ => None,
                })
                .or_else(|| {
                    self.format_resources
                        .iter()
                        .find_map(|resource| match resource {
                            FormatResource::Input {
                                logical_name,
                                source_kind,
                                bytes,
                            } if logical_name == name => {
                                Some(ResourceOutcome::Fulfilled(ResourceFulfillment::Input {
                                    name: logical_name.clone(),
                                    source: SourceRegistration::new(
                                        *source_kind,
                                        Arc::<[u8]>::from(bytes.as_slice()),
                                    )
                                    .with_name(format!("./{logical_name}")),
                                }))
                            }
                            _ => None,
                        })
                })
                .unwrap_or(ResourceOutcome::Unavailable),
            ResourceNeed::InputProbe { request } => self
                .job_resources
                .iter()
                .find_map(|resource| match resource {
                    LoadedFormatResource::Input {
                        logical_name,
                        resolved_name,
                        source_kind,
                        bytes,
                    } if logical_name == &request.name => Some(ResourceOutcome::Fulfilled(
                        ResourceFulfillment::InputProbe {
                            request: request.clone(),
                            resource: FileEnquiryResource::new(
                                SourceRegistration::new(
                                    *source_kind,
                                    Arc::<[u8]>::from(bytes.as_slice()),
                                )
                                .with_name(resolved_name.clone()),
                                None,
                            ),
                        },
                    )),
                    _ => None,
                })
                .or_else(|| {
                    self.format_resources
                        .iter()
                        .find_map(|resource| match resource {
                            FormatResource::Input {
                                logical_name,
                                source_kind,
                                bytes,
                            } if logical_name == &request.name => Some(ResourceOutcome::Fulfilled(
                                ResourceFulfillment::InputProbe {
                                    request: request.clone(),
                                    resource: FileEnquiryResource::new(
                                        SourceRegistration::new(
                                            *source_kind,
                                            Arc::<[u8]>::from(bytes.as_slice()),
                                        )
                                        .with_name(format!("./{logical_name}")),
                                        None,
                                    ),
                                },
                            )),
                            _ => None,
                        })
                })
                .unwrap_or(ResourceOutcome::Unavailable),
            ResourceNeed::Font { request } => self
                .job_resources
                .iter()
                .find_map(|resource| match resource {
                    LoadedFormatResource::Tfm {
                        logical_name,
                        bytes,
                    } if Path::new(logical_name).file_stem()
                        == Some(std::ffi::OsStr::new(&request.name)) =>
                    {
                        let content = world
                            .register_selected_file(
                                logical_name,
                                Arc::<[u8]>::from(bytes.as_slice()),
                            )
                            .ok()?;
                        Some(ResourceOutcome::Fulfilled(ResourceFulfillment::Font {
                            request: request.clone(),
                            resource: Box::new(FontResource::Tfm {
                                metrics: content,
                                opentype: None,
                            }),
                        }))
                    }
                    _ => None,
                })
                .or_else(|| {
                    self.format_resources
                        .iter()
                        .find_map(|resource| match resource {
                            FormatResource::Tfm {
                                logical_name,
                                bytes,
                            } if Path::new(logical_name).file_stem()
                                == Some(std::ffi::OsStr::new(&request.name)) =>
                            {
                                let content = world
                                    .register_selected_file(
                                        logical_name,
                                        Arc::<[u8]>::from(bytes.as_slice()),
                                    )
                                    .ok()?;
                                Some(ResourceOutcome::Fulfilled(ResourceFulfillment::Font {
                                    request: request.clone(),
                                    resource: Box::new(FontResource::Tfm {
                                        metrics: content,
                                        opentype: None,
                                    }),
                                }))
                            }
                            _ => None,
                        })
                })
                .unwrap_or(ResourceOutcome::Unavailable),
            ResourceNeed::PdfImage { .. } => ResourceOutcome::Unavailable,
        }
    }
}

fn capture_loaded_projection<G>(
    universe: &mut tex_state::Universe<G>,
    demand: &LoadedFormatProjectionDemand,
    completion: tex_exec::RootCompletionPolicy,
) -> Result<LoadedFormatProjection, FormatFixtureError> {
    let mut counts = Vec::with_capacity(demand.count_registers.len());
    for &register in &demand.count_registers {
        let value = universe
            .count(register)
            .map_err(|error| FormatFixtureError::Format(format!("count projection: {error:?}")))?;
        counts.push((register, value));
    }

    let mut boxes = Vec::with_capacity(demand.box_outlines.len());
    for request in &demand.box_outlines {
        let nodes = universe
            .box_register(request.register)
            .map_err(|error| FormatFixtureError::Format(format!("box projection: {error:?}")))?
            .map(|root| {
                let mut output = Vec::new();
                push_detached_node_outline(
                    universe,
                    root,
                    &mut Vec::new(),
                    request.depth,
                    &mut output,
                )?;
                Ok::<_, FormatFixtureError>(output)
            })
            .transpose()?;
        boxes.push(DetachedBoxOutline {
            register: request.register,
            nodes,
        });
    }

    let channels = if demand.channels {
        let source = universe.world();
        if completion == tex_exec::RootCompletionPolicy::RequireTeXEnd {
            let records = source.effect_records().to_vec();
            let mut destination = tex_state::World::memory_with_clock(source.job_clock());
            destination
                .publish_detached_effect_records(&records)
                .map_err(|error| {
                    FormatFixtureError::Format(format!("channel publication: {error:?}"))
                })?;
            let mut terminal = source.memory_terminal_output().unwrap_or_default().to_vec();
            terminal.extend_from_slice(destination.memory_terminal_output().unwrap_or_default());
            let mut log = source.memory_log_output().unwrap_or_default().to_vec();
            log.extend_from_slice(destination.memory_log_output().unwrap_or_default());
            let mut outputs = loaded_format_outputs(source);
            for output in loaded_format_outputs(&destination) {
                if let Some(existing) = outputs.iter_mut().find(|item| item.path == output.path) {
                    *existing = output;
                } else {
                    outputs.push(output);
                }
            }
            Some(LoadedFormatChannels {
                terminal,
                log,
                // This complete-job projection has already replayed `records`
                // into the detached per-sink bytes above. Returning the same
                // records as an unpublished suffix would make every outer
                // channel consumer publish the terminal episode twice.
                // Root-EOF fragments take the branch below and retain their
                // genuinely unpublished suffix instead.
                pending_effects: Vec::new(),
                outputs,
            })
        } else {
            Some(detach_loaded_channels(
                source,
                source.effect_records().to_vec(),
            ))
        }
    } else {
        None
    };

    Ok(LoadedFormatProjection {
        counts,
        boxes,
        channels,
    })
}

fn detach_loaded_channels(
    world: &tex_state::World,
    pending_effects: Vec<tex_state::EffectRecord>,
) -> LoadedFormatChannels {
    LoadedFormatChannels {
        terminal: world.memory_terminal_output().unwrap_or_default().to_vec(),
        log: world.memory_log_output().unwrap_or_default().to_vec(),
        pending_effects,
        outputs: loaded_format_outputs(world),
    }
}

fn loaded_format_outputs(world: &tex_state::World) -> Vec<LoadedFormatOutput> {
    world
        .memory_outputs()
        .into_iter()
        .flatten()
        .map(|output| LoadedFormatOutput {
            path: output.path().to_path_buf(),
            bytes: output.bytes().to_vec(),
        })
        .collect()
}

fn push_detached_node_outline<G>(
    universe: &tex_state::Universe<G>,
    root: tex_state::node_arena::DurableListId<G>,
    path: &mut Vec<usize>,
    depth: u8,
    output: &mut Vec<DetachedNodeOutlineEntry>,
) -> Result<(), FormatFixtureError> {
    let list = universe
        .node_list(root)
        .map_err(|error| FormatFixtureError::Format(format!("box outline root: {error:?}")))?;
    for (index, node) in list.nodes().iter().enumerate() {
        path.push(index);
        output.push(DetachedNodeOutlineEntry {
            path: path.clone(),
            kind: node.kind(),
        });
        if depth > 0
            && let tex_state::node::Node::HList(boxed) | tex_state::node::Node::VList(boxed) = node
        {
            push_detached_node_outline(universe, boxed.children, path, depth - 1, output)?;
        }
        path.pop();
    }
    Ok(())
}

pub struct LoadedFormatRun {
    pub result: RunResult,
    pub projection: LoadedFormatProjection,
}

/// Ensures one recipe image exists in the validated content-addressed cache.
pub fn ensure_format(
    cache: &FormatCacheStore,
    recipe: &FormatRecipe,
    launcher: &crate::FormatWorkerLauncher,
) -> Result<FormatFixture, FormatFixtureError> {
    let identity = recipe.identity()?;
    let entry = cache.ensure_entry::<FormatFixtureError>(
        &identity,
        |bytes| decode_oracle_bundle(bytes).map(|_| ()),
        || {
            let result = crate::format_worker::construct(Some(launcher), recipe)?;
            Ok((result.image, result.evidence))
        },
    )?;
    Ok(FormatFixture {
        recipe: recipe.clone(),
        image: entry.image().clone(),
        evidence: decode_oracle_bundle(entry.evidence()).map_err(FormatFixtureError::Evidence)?,
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ConstructionResult {
    pub image: Vec<u8>,
    pub evidence: Vec<u8>,
}

pub(crate) fn construct_format_in_worker(
    recipe: &FormatRecipe,
) -> Result<ConstructionResult, FormatFixtureError> {
    recipe.guards.validate()?;
    crate::with_engine_world(
        World::memory_with_clock(recipe.clock),
        |universe| -> Result<ConstructionResult, FormatFixtureError> {
            recipe.engine.prepare_initex(universe);
            // Web2C `tex.ch` [51.1332] selects `hyph_size` before INITEX;
            // tex.web §1308 then retains that exact compatibility constant in
            // the format consumed by the loaded job.
            universe.set_hyphenation_exception_capacity(recipe.hyphenation_exception_capacity);
            universe.set_interaction_mode(recipe.construction_interaction);
            universe.set_error_context_widths(recipe.construction_error_context_widths);
            let mut session =
                EngineSession::prepared_initex(universe, recipe.engine.command_profile());
            session.set_fuel_limit(recipe.guards.command_fuel)?;
            let source_bytes = Arc::<[u8]>::from(recipe.construction_source.as_slice());
            let root = session.register_retained_root_with_invocation(
                &recipe.construction_source_name,
                &recipe.construction_source_name,
                SourceRegistration::new(RegisteredSourceKind::Generated, source_bytes.clone())
                    .with_name(format!("./{}", recipe.construction_source_name)),
            )?;
            let mut observer = LiveSessionTranslator::for_root(
                SchemaVersion::V3,
                "terminal",
                LiveSource {
                    name: recipe.construction_source_name.clone(),
                    source: root,
                    bytes: source_bytes,
                },
            );
            let guards = GuardCheckpoints::new(recipe.guards)?;
            let mut checkpoints = &guards;
            let result = session.run_with_observer(
                &mut RecipeResourceHost::new(&recipe.resources),
                &mut checkpoints,
                &mut observer,
            );
            let result = finish_guarded_run(result, &guards)?;
            let format_dump = result
                .format_dump
                .ok_or(FormatFixtureError::ConstructionDidNotDump)?;
            let evidence = encode_oracle_bundle(&observer.finalize_detached_evidence())
                .map_err(FormatFixtureError::Evidence)?;
            Ok(ConstructionResult {
                image: format_dump.image.into_bytes(),
                evidence,
            })
        },
    )
    .map_err(|error| FormatFixtureError::Format(format!("{error:?}")))?
}

#[cfg(test)]
struct NoCheckpoints;
#[cfg(test)]
impl<G> CheckpointSink<G> for NoCheckpoints {
    fn wants_checkpoint(&self, _boundary: tex_exec::EngineBoundary) -> bool {
        false
    }

    fn checkpoint(&mut self, _checkpoint: tex_exec::EngineCheckpoint<G>) {}
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GuardFailure {
    WallTime,
    ResidentSet,
}

struct GuardCheckpoints {
    started: Instant,
    guards: FormatGenerationGuards,
    failure: Cell<Option<GuardFailure>>,
}

impl GuardCheckpoints {
    #[allow(
        clippy::disallowed_methods,
        reason = "native format-fixture guard measures host wall time independently of TeX's fixed job clock"
    )]
    fn new(guards: FormatGenerationGuards) -> Result<Self, FormatFixtureError> {
        current_resident_bytes()?;
        Ok(Self {
            started: Instant::now(),
            guards,
            failure: Cell::new(None),
        })
    }
}

impl<G> CheckpointSink<G> for &GuardCheckpoints {
    fn wants_checkpoint(&self, _boundary: tex_exec::EngineBoundary) -> bool {
        false
    }

    fn stop_requested(&self) -> bool {
        let failure = if self.started.elapsed() > self.guards.wall_time {
            Some(GuardFailure::WallTime)
        } else {
            match current_resident_bytes() {
                Ok(bytes) if bytes > self.guards.resident_bytes => Some(GuardFailure::ResidentSet),
                Ok(_) => None,
                Err(_) => Some(GuardFailure::ResidentSet),
            }
        };
        self.failure.set(failure);
        failure.is_some()
    }

    fn checkpoint(&mut self, _checkpoint: tex_exec::EngineCheckpoint<G>) {}
}

fn finish_guarded_run<T>(
    result: Result<T, SessionError>,
    guards: &GuardCheckpoints,
) -> Result<T, FormatFixtureError> {
    match (result, guards.failure.get()) {
        (Err(SessionError::CooperativeStopRequested), Some(GuardFailure::WallTime)) => {
            Err(FormatFixtureError::WallTimeExceeded)
        }
        (Err(SessionError::CooperativeStopRequested), Some(GuardFailure::ResidentSet)) => {
            Err(FormatFixtureError::ResidentSetExceeded)
        }
        (result, _) => result.map_err(FormatFixtureError::from),
    }
}

struct RecipeResourceHost<'a> {
    resources: &'a [FormatResource],
}

impl<'a> RecipeResourceHost<'a> {
    fn new(resources: &'a [FormatResource]) -> Self {
        Self { resources }
    }
}

impl ResourceHost for RecipeResourceHost<'_> {
    fn fulfill(&mut self, world: &mut ResourceWorld<'_>, need: &ResourceNeed) -> ResourceOutcome {
        match need {
            ResourceNeed::Input { name, .. } => self
                .resources
                .iter()
                .find_map(|resource| match resource {
                    FormatResource::Input {
                        logical_name,
                        source_kind,
                        bytes,
                    } if logical_name == name => {
                        Some(ResourceOutcome::Fulfilled(ResourceFulfillment::input(
                            logical_name,
                            *source_kind,
                            Arc::from(bytes.as_slice()),
                        )))
                    }
                    _ => None,
                })
                .unwrap_or(ResourceOutcome::Unavailable),
            ResourceNeed::InputProbe { request } => self
                .resources
                .iter()
                .find_map(|resource| match resource {
                    FormatResource::Input {
                        logical_name,
                        source_kind,
                        bytes,
                    } if logical_name == &request.name => Some(ResourceOutcome::Fulfilled(
                        ResourceFulfillment::InputProbe {
                            request: request.clone(),
                            resource: FileEnquiryResource::new(
                                SourceRegistration::new(
                                    *source_kind,
                                    Arc::<[u8]>::from(bytes.as_slice()),
                                ),
                                None,
                            ),
                        },
                    )),
                    _ => None,
                })
                .unwrap_or(ResourceOutcome::Unavailable),
            ResourceNeed::Font { request } => self
                .resources
                .iter()
                .find_map(|resource| match resource {
                    FormatResource::Tfm {
                        logical_name,
                        bytes,
                    } if Path::new(logical_name).file_stem()
                        == Some(std::ffi::OsStr::new(&request.name)) =>
                    {
                        let content = world
                            .register_selected_file(logical_name, Arc::from(bytes.as_slice()))
                            .ok()?;
                        Some(ResourceOutcome::Fulfilled(ResourceFulfillment::Font {
                            request: request.clone(),
                            resource: Box::new(FontResource::Tfm {
                                metrics: content,
                                opentype: None,
                            }),
                        }))
                    }
                    _ => None,
                })
                .unwrap_or(ResourceOutcome::Unavailable),
            ResourceNeed::PdfImage { .. } => ResourceOutcome::Unavailable,
        }
    }
}

#[cfg(target_os = "linux")]
fn current_resident_bytes() -> Result<u64, FormatFixtureError> {
    crate::linux_rss::resident_bytes(Path::new("/proc/self/statm"))
        .ok_or(FormatFixtureError::ResidentSetUnsupported)
}

#[cfg(not(target_os = "linux"))]
fn current_resident_bytes() -> Result<u64, FormatFixtureError> {
    Err(FormatFixtureError::ResidentSetUnsupported)
}

fn resource_closure_hash(resources: &[FormatResource]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(IDENTITY_DOMAIN);
    hasher.update((resources.len() as u64).to_le_bytes());
    for resource in resources {
        match resource {
            FormatResource::Input {
                logical_name,
                source_kind,
                bytes,
            } => {
                hasher.update([1, source_kind_tag(*source_kind)]);
                hash_field(&mut hasher, logical_name.as_bytes());
                hash_field(&mut hasher, bytes);
            }
            FormatResource::Tfm {
                logical_name,
                bytes,
            } => {
                hasher.update([2]);
                hash_field(&mut hasher, logical_name.as_bytes());
                hash_field(&mut hasher, bytes);
            }
        }
    }
    hasher.finalize().into()
}

const fn source_kind_tag(kind: RegisteredSourceKind) -> u8 {
    match kind {
        RegisteredSourceKind::World => 1,
        RegisteredSourceKind::Generated => 2,
        RegisteredSourceKind::EditorFragment => 3,
        RegisteredSourceKind::ReadLine => 4,
    }
}

const fn interaction_tag(mode: tex_state::InteractionMode) -> u8 {
    match mode {
        tex_state::InteractionMode::Batch => 0,
        tex_state::InteractionMode::Nonstop => 1,
        tex_state::InteractionMode::Scroll => 2,
        tex_state::InteractionMode::ErrorStop => 3,
    }
}

fn framed_hash(fields: &[&[u8]]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(IDENTITY_DOMAIN);
    hasher.update((fields.len() as u64).to_le_bytes());
    for field in fields {
        hash_field(&mut hasher, field);
    }
    hasher.finalize().into()
}

fn producer_contract(version: u32, format_name: &str, format_ident_name: &str) -> [u8; 32] {
    framed_hash(&[
        &version.to_le_bytes(),
        env!("CARGO_PKG_VERSION").as_bytes(),
        build_feature_contract(),
        format_name.as_bytes(),
        format_ident_name.as_bytes(),
    ])
}

const fn build_feature_contract() -> &'static [u8] {
    if cfg!(feature = "shadow") {
        b"umber-format-producer-v1;shadow"
    } else {
        b"umber-format-producer-v1;default"
    }
}

fn hash_field(hasher: &mut Sha256, field: &[u8]) {
    hasher.update((field.len() as u64).to_le_bytes());
    hasher.update(field);
}

const fn cache_mode(engine: EngineMode) -> FormatEngineMode {
    match engine {
        EngineMode::Tex82 => FormatEngineMode::Tex82,
        EngineMode::ETex => FormatEngineMode::ETex,
        EngineMode::PdfTex => FormatEngineMode::PdfTex,
        EngineMode::Latex => FormatEngineMode::Latex,
        EngineMode::PdfLatex => FormatEngineMode::PdfLatex,
    }
}

#[derive(Debug)]
pub enum FormatFixtureError {
    UnboundedGuard,
    WallTimeExceeded,
    ResidentSetExceeded,
    ResidentSetUnsupported,
    ConstructionDidNotDump,
    Format(String),
    Evidence(String),
    Cache(FormatCacheError),
    Session(Box<SessionError>),
    Fuel(tex_command::CommandFuelLimitError),
    WorkerSpawn(String),
    WorkerBootstrapUnregistered,
    WorkerProtocol(String),
    WorkerIdentityMismatch,
    WorkerCrashed(Option<i32>, String),
    Worker(String),
    ProviderProfileMismatch {
        expected: EngineMode,
        actual: EngineMode,
    },
    ProviderBinaryMismatch {
        engine: EngineMode,
        binary: tex_exec::EngineBinaryIdentity,
    },
    ProviderBackendMismatch {
        engine: EngineMode,
        backend: crate::OutputCapability,
    },
    World(String),
}

impl fmt::Display for FormatFixtureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for FormatFixtureError {}

impl From<FormatCacheError> for FormatFixtureError {
    fn from(error: FormatCacheError) -> Self {
        Self::Cache(error)
    }
}

impl From<umber_fetch::CacheError> for FormatFixtureError {
    fn from(error: umber_fetch::CacheError) -> Self {
        Self::Cache(FormatCacheError::from(error))
    }
}

impl From<SessionError> for FormatFixtureError {
    fn from(error: SessionError) -> Self {
        Self::Session(Box::new(error))
    }
}

impl From<tex_command::CommandFuelLimitError> for FormatFixtureError {
    fn from(error: tex_command::CommandFuelLimitError) -> Self {
        Self::Fuel(error)
    }
}

#[cfg(test)]
mod tests;
