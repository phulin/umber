//! Borrow-scoped host capabilities.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;

use crate::{FontLoadRequest, PdfImageRequest, SourceRegistration, SourceRole};
use tex_state::InputReadState;
use tex_state::glue::GlueSpec;
use tex_state::scaled::Scaled;
use tex_state::world::{
    ContentHash, FileContent, FileModificationDate, InputDependency, InputDependencyAccess,
    InputDependencyOutcome, InputOrigin, WorldError,
};

/// Returns the exact transient capability key for a canonical font request.
///
/// TeX TFM names receive §1257's default `.tfm` extension. Umber's explicit
/// `opentype:` namespace is already a complete typed resource name and must
/// never be rewritten as a TFM path.
#[must_use]
pub fn canonical_font_resource_path(name: &str) -> PathBuf {
    let mut path = PathBuf::from(name);
    if !name.starts_with("opentype:") && path.extension().is_none() {
        path.set_extension("tfm");
    }
    path
}

/// The host-neutral identity of one immutable resource request.
///
/// This is the retained protocol shared by command producers and executor
/// fallback drivers. It remains separate from `tex_state::ResourceNeed`,
/// whose integer identity belongs to the older resolver contract.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResourceNeed {
    /// TeX82's `start_input` scanned this logical filename (§529 / §1030+),
    /// but the host has not supplied its immutable source registration.
    Input { name: String, original_name: String },
    /// A non-opening `\openin` or pdfTeX file enquiry needs bytes or
    /// authoritative absence.
    InputProbe { request: FileEnquiryRequest },
    /// TeX82's `new_font` completed its filename and size scan (§1254), but
    /// the host has not supplied the immutable font bytes.
    Font { request: FontLoadRequest },
    /// pdfTeX's `scan_image` completed an immutable request, but its retained
    /// bytes and validated metadata have not been supplied by the host.
    PdfImage { request: PdfImageRequest },
}

/// Typed answer to one [`ResourceNeed`].
#[derive(Clone, Debug)]
pub enum ResourceFulfillment {
    Input {
        name: String,
        source: SourceRegistration,
    },
    /// Immutable bytes answering a non-opening pdfTeX file enquiry or
    /// `\openin` probe. This remains distinct from required input backing so
    /// a later opening read can upgrade host dependency accounting.
    InputProbe {
        request: FileEnquiryRequest,
        resource: FileEnquiryResource,
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

/// An owned, actionable failure while resolving a resource.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResourceFailure {
    World(WorldError),
    Message(String),
}

impl ResourceFailure {
    #[must_use]
    pub fn message(message: impl Into<String>) -> Self {
        Self::Message(message.into())
    }

    #[must_use]
    pub fn world(error: WorldError) -> Self {
        Self::World(error)
    }

    #[must_use]
    pub fn as_world_error(&self) -> Option<&WorldError> {
        match self {
            Self::World(error) => Some(error),
            Self::Message(_) => None,
        }
    }
}

impl From<WorldError> for ResourceFailure {
    fn from(error: WorldError) -> Self {
        Self::World(error)
    }
}

impl fmt::Display for ResourceFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::World(error) => error.fmt(formatter),
            Self::Message(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for ResourceFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::World(error) => Some(error),
            Self::Message(_) => None,
        }
    }
}

/// Outcome of one synchronous provider call.
#[derive(Clone, Debug)]
pub enum ResourceOutcome {
    Fulfilled(ResourceFulfillment),
    Unavailable,
    Declined,
    Failed(ResourceFailure),
}

/// Owned provider result, including semantic reads performed while producing
/// it. Dependencies belong beside the installed capability; they do not
/// enter request identity or form a second resource cache.
#[derive(Clone, Debug)]
pub struct ResourceResolution {
    pub outcome: ResourceOutcome,
    pub dependencies: Vec<InputDependency>,
}

/// Result of settling a provider answer in the capability owner.
///
/// A fulfilled result carries the original active payload for the cold
/// consumer. The capability owner strips active World records before storing
/// the retained form. File payloads may keep a shared, non-authoritative
/// `InputRecordId` cache hint; World validates that hint against the current
/// timeline and exact bytes before reuse, so the hint cannot retain World
/// state or become a mailbox or parser continuation.
#[derive(Debug)]
pub enum ResourceInstallOutcome {
    Fulfilled(ResourceFulfillment),
    Unavailable,
    Declined,
    Failed(ResourceFailure),
}

/// Rewrites retained resolver observations for the content selected by one
/// actual use. Missing candidate attempts remain useful search facts, while
/// stale Present hashes from an older winner are discarded before the current
/// path/hash is recorded.
#[must_use]
pub fn dependencies_for_actual_content(
    dependencies: &[InputDependency],
    path: &Path,
    hash: ContentHash,
) -> Arc<[InputDependency]> {
    let access = dependencies
        .iter()
        .find(|dependency| dependency.path() == path)
        .map_or(InputDependencyAccess::RequiredRead, InputDependency::access);
    let mut actual = dependencies
        .iter()
        .filter(|dependency| {
            matches!(dependency.outcome(), InputDependencyOutcome::Missing)
                && dependency.path() != path
        })
        .cloned()
        .collect::<Vec<_>>();
    actual.push(InputDependency::new(
        path.to_owned(),
        InputDependencyOutcome::Present(hash),
        access,
    ));
    actual.into()
}

impl ResourceResolution {
    #[must_use]
    pub fn new(outcome: ResourceOutcome, dependencies: Vec<InputDependency>) -> Self {
        Self {
            outcome,
            dependencies,
        }
    }

    #[must_use]
    pub fn outcome(outcome: ResourceOutcome) -> Self {
        Self::new(outcome, Vec::new())
    }
}

/// Semantic bookkeeping performed while a host resolves one immutable
/// resource. This remains useful to public executor fallback consumers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResourceReplayEffect {
    InputDependency {
        path: PathBuf,
        outcome: InputDependencyOutcome,
        access: InputDependencyAccess,
    },
}

impl ResourceReplayEffect {
    /// Detaches the semantic input fact carried by this retained answer.
    #[must_use]
    pub fn input_dependency(&self) -> InputDependency {
        match self {
            Self::InputDependency {
                path,
                outcome,
                access,
            } => InputDependency::new(path.clone(), *outcome, *access),
        }
    }
}

impl ResourceFulfillment {
    #[must_use]
    pub fn input(
        name: impl Into<String>,
        kind: crate::RegisteredSourceKind,
        bytes: Arc<[u8]>,
    ) -> Self {
        Self::Input {
            name: name.into(),
            source: SourceRegistration::new(kind, bytes),
        }
    }

    #[must_use]
    pub fn input_with_role(
        name: impl Into<String>,
        kind: crate::RegisteredSourceKind,
        bytes: Arc<[u8]>,
        role: SourceRole,
    ) -> Self {
        Self::Input {
            name: name.into(),
            source: SourceRegistration::new(kind, bytes).with_role(role),
        }
    }

    #[must_use]
    pub fn world_input(name: impl Into<String>, content: FileContent) -> Self {
        Self::Input {
            name: name.into(),
            source: SourceRegistration::world(content),
        }
    }

    #[must_use]
    pub fn world_input_with_role(
        name: impl Into<String>,
        content: FileContent,
        role: SourceRole,
    ) -> Self {
        Self::Input {
            name: name.into(),
            source: SourceRegistration::world(content).with_role(role),
        }
    }

    #[must_use]
    pub fn world_input_probe(request: FileEnquiryRequest, content: FileContent) -> Self {
        Self::InputProbe {
            request,
            resource: FileEnquiryResource::world(content),
        }
    }
}

/// Why canonical command processing needs a non-opening file lookup.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum FileEnquiryIntent {
    OpenInProbe,
    Size,
    ModificationDate,
    MdFiveSum,
    Dump,
}

/// Complete identity of one host-neutral file enquiry.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct FileEnquiryRequest {
    pub name: String,
    pub intent: FileEnquiryIntent,
}

impl FileEnquiryRequest {
    #[must_use]
    pub fn new(name: impl Into<String>, intent: FileEnquiryIntent) -> Self {
        Self {
            name: name.into(),
            intent,
        }
    }
}

/// Immutable answer to a non-opening file enquiry.
#[derive(Clone, Debug)]
pub struct FileEnquiryResource {
    source: SourceRegistration,
    modification_date: Option<tex_state::FileModificationDate>,
    record_hint: Arc<Mutex<Option<tex_state::InputRecordId>>>,
}

impl FileEnquiryResource {
    #[must_use]
    pub fn new(
        source: SourceRegistration,
        modification_date: Option<tex_state::FileModificationDate>,
    ) -> Self {
        let record_hint = source.active_world_record();
        Self {
            record_hint: Arc::new(Mutex::new(record_hint)),
            source,
            modification_date,
        }
    }

    #[must_use]
    pub fn world(content: FileContent) -> Self {
        let modification_date = content.modification_date();
        Self::new(SourceRegistration::world(content), modification_date)
    }

    #[must_use]
    pub fn source(&self) -> &SourceRegistration {
        &self.source
    }

    /// Returns the retained form with its active World record stripped.
    ///
    /// The shared record hint is cache metadata only. Each use validates it
    /// against the current World timeline and exact selected bytes, then
    /// refreshes it or registers a new record when validation fails.
    #[must_use]
    pub(crate) fn without_world_record(&self) -> Self {
        Self {
            source: self.source.without_world_record(),
            modification_date: self.modification_date,
            record_hint: Arc::clone(&self.record_hint),
        }
    }

    /// Re-registers the retained source backing for one actual enquiry.
    ///
    /// The source helper gives current same-run output precedence and returns
    /// `None` for a generated-only selection whose output was rolled back.
    pub fn actual_use(&self, input: &mut dyn InputReadState) -> Result<Option<Self>, WorldError> {
        let Some(source) = self.source.actual_use_retained(
            input,
            *self
                .record_hint
                .lock()
                .expect("file enquiry record hint mutex is not poisoned"),
        )?
        else {
            return Ok(None);
        };
        *self
            .record_hint
            .lock()
            .expect("file enquiry record hint mutex is not poisoned") =
            source.active_world_record();
        let modification_date = source.modification_date().or(self.modification_date);
        Ok(Some(Self {
            source,
            modification_date,
            record_hint: Arc::clone(&self.record_hint),
        }))
    }

    pub fn dependencies_for_actual_use(&self, active: &Self) -> Arc<[InputDependency]> {
        self.source.dependencies_for_actual_use(&active.source)
    }

    /// Attaches semantic dependencies to this retained enquiry answer while
    /// preserving its modification metadata.
    #[must_use]
    pub fn with_input_dependencies(
        mut self,
        dependencies: impl Into<Arc<[InputDependency]>>,
    ) -> Self {
        self.source = self.source.with_input_dependencies(dependencies);
        self
    }

    #[must_use]
    pub const fn modification_date(&self) -> Option<tex_state::FileModificationDate> {
        self.modification_date
    }
}

/// Immutable font bytes selected by the host for a canonical `\\font` replay.
///
/// This transient capability value is never retained by command state or
/// snapshots: a missing entry is an explicit resource need at the replay
/// boundary.
#[derive(Clone, Debug)]
pub enum FontResource {
    /// The host completed lookup and determined that no font is available.
    /// This differs from an absent capability entry, which requests replay.
    Unavailable,
    Tfm {
        metrics: FileContent,
        opentype: Option<tex_fonts::OpenTypeFont>,
    },
    MappedTfm {
        metrics: FileContent,
        opentype: tex_fonts::OpenTypeFont,
        encoding_map: tex_fonts::LegacyEncodingMap,
    },
    ClassicTfmFallback {
        metrics: FileContent,
    },
    OpenType(tex_fonts::OpenTypeFont),
}

/// Retained bytes for a host-selected font metrics file.
///
/// The active World record is stripped before retention. The shared record
/// hint is non-authoritative cache metadata: actual use validates it against
/// the current World timeline and exact bytes, so it cannot retain World
/// state.
#[derive(Clone, Debug)]
#[doc(hidden)]
pub struct RetainedFileContent {
    path: PathBuf,
    bytes: tex_state::SharedBytes,
    record_hint: Arc<Mutex<Option<tex_state::InputRecordId>>>,
    modification_date: Option<FileModificationDate>,
    origin: InputOrigin,
}

impl RetainedFileContent {
    fn from_active(content: &FileContent) -> Self {
        Self {
            path: content.path().to_owned(),
            bytes: content.shared_bytes(),
            record_hint: Arc::new(Mutex::new(Some(content.record()))),
            modification_date: content.modification_date(),
            origin: content.origin(),
        }
    }

    fn actual_use(
        &self,
        input: &mut dyn InputReadState,
        dependencies: &[InputDependency],
    ) -> Result<Option<FileContent>, WorldError> {
        let content = input.read_selected_input_record(
            &self.path,
            *self
                .record_hint
                .lock()
                .expect("retained file record hint mutex is not poisoned"),
            self.bytes.clone(),
            self.modification_date,
            self.origin,
            dependencies,
        )?;
        if let Some(content) = &content {
            *self
                .record_hint
                .lock()
                .expect("retained file record hint mutex is not poisoned") = Some(content.record());
        }
        Ok(content)
    }
}

/// Retained capability payload for one host-selected font answer.
///
/// Nested file payloads have their active World records stripped. Any shared
/// record hint is only validated cache metadata and cannot retain World state;
/// a failed validation registers a current record before the answer is used.
#[derive(Clone, Debug)]
pub enum RetainedFontResource {
    Unavailable,
    Tfm {
        metrics: RetainedFileContent,
        opentype: Option<tex_fonts::OpenTypeFont>,
    },
    MappedTfm {
        metrics: RetainedFileContent,
        opentype: tex_fonts::OpenTypeFont,
        encoding_map: tex_fonts::LegacyEncodingMap,
    },
    ClassicTfmFallback {
        metrics: RetainedFileContent,
    },
    OpenType(tex_fonts::OpenTypeFont),
}

impl RetainedFontResource {
    fn from_active(resource: &FontResource) -> Self {
        match resource {
            FontResource::Unavailable => Self::Unavailable,
            FontResource::Tfm { metrics, opentype } => Self::Tfm {
                metrics: RetainedFileContent::from_active(metrics),
                opentype: opentype.clone(),
            },
            FontResource::MappedTfm {
                metrics,
                opentype,
                encoding_map,
            } => Self::MappedTfm {
                metrics: RetainedFileContent::from_active(metrics),
                opentype: opentype.clone(),
                encoding_map: encoding_map.clone(),
            },
            FontResource::ClassicTfmFallback { metrics } => Self::ClassicTfmFallback {
                metrics: RetainedFileContent::from_active(metrics),
            },
            FontResource::OpenType(selection) => Self::OpenType(selection.clone()),
        }
    }

    pub fn actual_use(
        &self,
        input: &mut dyn InputReadState,
        dependencies: &[InputDependency],
    ) -> Result<Option<FontResource>, WorldError> {
        let resource = match self {
            Self::Unavailable => FontResource::Unavailable,
            Self::Tfm { metrics, opentype } => {
                let Some(metrics) = metrics.actual_use(input, dependencies)? else {
                    return Ok(None);
                };
                FontResource::Tfm {
                    metrics,
                    opentype: opentype.clone(),
                }
            }
            Self::MappedTfm {
                metrics,
                opentype,
                encoding_map,
            } => {
                let Some(metrics) = metrics.actual_use(input, dependencies)? else {
                    return Ok(None);
                };
                FontResource::MappedTfm {
                    metrics,
                    opentype: opentype.clone(),
                    encoding_map: encoding_map.clone(),
                }
            }
            Self::ClassicTfmFallback { metrics } => {
                let Some(metrics) = metrics.actual_use(input, dependencies)? else {
                    return Ok(None);
                };
                FontResource::ClassicTfmFallback { metrics }
            }
            Self::OpenType(selection) => FontResource::OpenType(selection.clone()),
        };
        Ok(Some(resource))
    }
}

/// Compact coordinate into the capability owner's immutable font-resource
/// payloads. Paths remain lookup keys only; moving the ordered index must not
/// shift the much larger resource values.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct HostFontResourceId(u32);

const HOST_FONT_RESOURCE_CHUNK_CAPACITY: usize = 32;

/// One authoritative append owner for host-selected immutable font inputs.
/// Fixed-capacity chunks prevent later registration from relocating earlier
/// wide resources; ordered lookup retains only their compact coordinates.
#[derive(Debug, Default)]
struct HostFontResources {
    chunks: Vec<Vec<HostFontResource>>,
    len: u32,
}

#[derive(Debug)]
struct HostFontResource {
    resource: RetainedFontResource,
    dependencies: Arc<[InputDependency]>,
}

impl HostFontResources {
    fn push_retained(
        &mut self,
        resource: RetainedFontResource,
        dependencies: Arc<[InputDependency]>,
    ) -> HostFontResourceId {
        if self
            .chunks
            .last()
            .is_none_or(|chunk| chunk.len() == HOST_FONT_RESOURCE_CHUNK_CAPACITY)
        {
            self.chunks
                .push(Vec::with_capacity(HOST_FONT_RESOURCE_CHUNK_CAPACITY));
        }
        let id = HostFontResourceId(self.len);
        self.chunks
            .last_mut()
            .expect("font resource owner has an append chunk")
            .push(HostFontResource {
                resource,
                dependencies,
            });
        self.len = self
            .len
            .checked_add(1)
            .expect("font capability count exceeds u32");
        id
    }

    fn get(&self, id: HostFontResourceId) -> Option<&RetainedFontResource> {
        let raw = id.0 as usize;
        self.chunks
            .get(raw / HOST_FONT_RESOURCE_CHUNK_CAPACITY)?
            .get(raw % HOST_FONT_RESOURCE_CHUNK_CAPACITY)
            .map(|entry| &entry.resource)
    }

    fn dependencies(&self, id: HostFontResourceId) -> Option<&[InputDependency]> {
        let raw = id.0 as usize;
        self.chunks
            .get(raw / HOST_FONT_RESOURCE_CHUNK_CAPACITY)?
            .get(raw % HOST_FONT_RESOURCE_CHUNK_CAPACITY)
            .map(|entry| entry.dependencies.as_ref())
    }

    fn replace_retained(
        &mut self,
        id: HostFontResourceId,
        resource: RetainedFontResource,
        dependencies: Arc<[InputDependency]>,
    ) {
        let raw = id.0 as usize;
        let entry = &mut self.chunks[raw / HOST_FONT_RESOURCE_CHUNK_CAPACITY]
            [raw % HOST_FONT_RESOURCE_CHUNK_CAPACITY];
        entry.resource = resource;
        entry.dependencies = dependencies;
    }

    #[cfg(test)]
    const fn len(&self) -> u32 {
        self.len
    }
}

/// Host-completed result for a canonical pdfTeX image request.
///
/// Parsed metadata and retained immutable bytes are safe to hand to the
/// engine; a missing map entry means acquisition has not completed and must
/// suspend the aggregate operation instead.
#[derive(Clone, Debug)]
pub enum PdfImageResource {
    Unavailable,
    Invalid(String),
    Available(tex_state::PdfExternalImageSource),
}

/// Physical input provenance retained alongside a value-only parsed image.
/// Unlike source and TFM capabilities, images do not retain a live
/// `FileContent`; this small selection fact lets the executor detect a
/// generated-output replacement before reusing parsed bytes.
#[derive(Clone, Debug)]
struct RetainedImageSelection {
    path: PathBuf,
    origin: InputOrigin,
}

#[derive(Debug)]
struct HostPdfImageResource {
    request: PdfImageRequest,
    resource: PdfImageResource,
    dependencies: Arc<[InputDependency]>,
    selection: Option<RetainedImageSelection>,
}

/// The executor facts observed by TeX's mode predicates.
///
/// This is a copy-only query result, never persistent command state.  The
/// executor refreshes it for each bounded command operation from its mode
/// nest; command processing merely consumes the answer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConditionalState {
    mode: ConditionalMode,
    inner: bool,
}

impl ConditionalState {
    #[must_use]
    pub const fn new(mode: ConditionalMode, inner: bool) -> Self {
        Self { mode, inner }
    }

    #[must_use]
    pub const fn mode(self) -> ConditionalMode {
        self.mode
    }

    #[must_use]
    pub const fn is_inner(self) -> bool {
        self.inner
    }
}

/// TeX's three mode families, projected by the executor-owned mode nest.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConditionalMode {
    Vertical,
    Horizontal,
    Math,
}

/// TeX82 §424's "last item in the current list" fetch result, for
/// `\lastpenalty`, `\lastkern`, and `\lastskip`.
///
/// This is a copy-only query result refreshed by the executor for each
/// bounded command operation, exactly like [`ConditionalState`] and the
/// horizontal-mode space factor: the current list's tail node (or, in the
/// outer vertical list, the page builder's own `last_glue`/`last_penalty`/
/// `last_kern` memo) is executor- and page-owned state that command
/// processing only observes. `None` means the tail node matched none of
/// these three shapes (an empty list, a character, or any other node type),
/// in which case every one of the three primitives reads its level-specific
/// zero, per tex.web's "Fetch an item in the current node, if appropriate".
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LastNodeItem {
    /// The tail is a penalty node: `\lastpenalty` reads its value.
    Penalty(i32),
    /// The tail is a kern node: `\lastkern` reads its width.
    Kern(Scaled),
    /// The tail is a glue node: `\lastskip` reads its specification.
    Glue(GlueSpec),
    /// The tail is a glue node created in mu units (TeX82's `mu_glue`
    /// subtype, e.g. from `\mskip`): `\lastskip` reads it at `mu_val` level.
    MuGlue(GlueSpec),
}

/// Long-lived immutable-resource capabilities owned by the executor.
///
/// Live mode and list facts deliberately do not live here. They are sampled
/// through [`CommandHostFacts`] at the exact scanner or conditional that
/// consumes them, so ordinary command delivery writes no cold fact cache.
/// This value is intentionally neither serializable nor cloneable.
#[derive(Debug, Default)]
pub struct CommandHostCapabilities {
    input: BTreeMap<String, SourceRegistration>,
    unavailable_input: BTreeMap<String, Arc<[InputDependency]>>,
    unavailable_input_requests: BTreeSet<String>,
    input_probes: BTreeMap<String, FileEnquiryResource>,
    unavailable_input_probes: BTreeMap<String, Arc<[InputDependency]>>,
    font_paths: BTreeMap<PathBuf, HostFontResourceId>,
    font_resources: HostFontResources,
    images: Vec<HostPdfImageResource>,
    job_name: String,
}

impl CommandHostCapabilities {
    /// Installs a typed answer for the exact resource request that produced
    /// it. Keeping validation here gives command producers and executor
    /// fallback ledgers one canonical capability mutation path.
    pub fn install_resource_answer(
        &mut self,
        need: &ResourceNeed,
        fulfillment: ResourceFulfillment,
        dependencies: impl Into<Arc<[InputDependency]>>,
    ) -> Result<ResourceFulfillment, Box<ResourceFulfillment>> {
        let dependencies = dependencies.into();
        match (need, fulfillment) {
            (
                ResourceNeed::Input { name: expected, .. },
                ResourceFulfillment::Input { name, source },
            ) if expected == &name => {
                self.register_input_with_dependencies(
                    name.clone(),
                    source.without_world_record(),
                    Arc::clone(&dependencies),
                );
                Ok(ResourceFulfillment::Input { name, source })
            }
            (
                ResourceNeed::InputProbe { request: expected },
                ResourceFulfillment::InputProbe { request, resource },
            ) if expected == &request => {
                self.register_input_probe_with_dependencies(
                    request.name.clone(),
                    resource.without_world_record(),
                    Arc::clone(&dependencies),
                );
                Ok(ResourceFulfillment::InputProbe { request, resource })
            }
            (
                ResourceNeed::Font { request: expected },
                ResourceFulfillment::Font { request, resource },
            ) if expected == &request => {
                self.register_retained_font_with_dependencies(
                    canonical_font_resource_path(&request.name),
                    RetainedFontResource::from_active(&resource),
                    Arc::clone(&dependencies),
                );
                Ok(ResourceFulfillment::Font { request, resource })
            }
            (
                ResourceNeed::PdfImage { request: expected },
                ResourceFulfillment::PdfImage { request, resource },
            ) if expected == &request => {
                let resource = *resource;
                self.register_pdf_image_with_dependencies(
                    request.clone(),
                    resource.clone(),
                    Arc::clone(&dependencies),
                );
                Ok(ResourceFulfillment::PdfImage {
                    request,
                    resource: Box::new(resource),
                })
            }
            (_, fulfillment) => Err(Box::new(fulfillment)),
        }
    }

    /// Applies one provider resolution through the canonical capability
    /// installer while preserving the original active fulfillment for the
    /// current cold consumer.
    pub fn install_resource_resolution(
        &mut self,
        need: &ResourceNeed,
        resolution: ResourceResolution,
        register_texinputs_alias: bool,
    ) -> Result<ResourceInstallOutcome, Box<ResourceFulfillment>> {
        let ResourceResolution {
            outcome,
            dependencies,
        } = resolution;
        match outcome {
            ResourceOutcome::Fulfilled(fulfillment) => self
                .install_resource_answer(need, fulfillment, dependencies)
                .map(ResourceInstallOutcome::Fulfilled),
            ResourceOutcome::Unavailable => {
                self.install_resource_unavailable(need, register_texinputs_alias, dependencies);
                Ok(ResourceInstallOutcome::Unavailable)
            }
            ResourceOutcome::Declined => Ok(ResourceInstallOutcome::Declined),
            ResourceOutcome::Failed(failure) => Ok(ResourceInstallOutcome::Failed(failure)),
        }
    }

    /// Installs an authoritative absence for the exact typed request.
    pub fn install_resource_unavailable(
        &mut self,
        need: &ResourceNeed,
        register_texinputs_alias: bool,
        dependencies: impl Into<Arc<[InputDependency]>>,
    ) {
        let dependencies = dependencies.into();
        match need {
            ResourceNeed::Input { name, .. } => {
                self.mark_input_unavailable_with_dependencies(name, Arc::clone(&dependencies));
                if register_texinputs_alias && !name.contains(['/', '\\', ':']) {
                    self.mark_input_unavailable_with_dependencies(
                        format!("TeXinputs:{name}"),
                        dependencies,
                    );
                }
            }
            ResourceNeed::InputProbe { request } => {
                self.mark_input_probe_unavailable_with_dependencies(&request.name, dependencies)
            }
            ResourceNeed::Font { request } => self.register_font_with_dependencies(
                canonical_font_resource_path(&request.name),
                FontResource::Unavailable,
                dependencies,
            ),
            ResourceNeed::PdfImage { request } => self.register_pdf_image_with_dependencies(
                request.clone(),
                PdfImageResource::Unavailable,
                dependencies,
            ),
        }
    }

    /// Installs immutable backing for one logical `\input` request.
    ///
    /// Acquisition is complete before this capability is constructed.  The
    /// command machine can therefore request only retained bytes and never
    /// opens files itself.
    pub fn register_input(&mut self, name: impl Into<String>, source: SourceRegistration) {
        let name = name.into();
        self.unavailable_input.remove(&name);
        self.unavailable_input_requests.remove(&name);
        self.input.insert(name, source.without_world_record());
    }

    /// Installs immutable input backing together with the semantic reads that
    /// produced it. The metadata is retained beside the one source binding;
    /// it is not a second cache or a copy of the source bytes.
    pub fn register_input_with_dependencies(
        &mut self,
        name: impl Into<String>,
        source: SourceRegistration,
        dependencies: impl Into<Arc<[InputDependency]>>,
    ) {
        let name = name.into();
        self.unavailable_input.remove(&name);
        self.unavailable_input_requests.remove(&name);
        self.input.insert(
            name,
            source
                .without_world_record()
                .with_input_dependencies(dependencies),
        );
    }

    /// Records a completed host lookup which found no input backing.
    pub fn mark_input_unavailable(&mut self, name: impl Into<String>) {
        self.mark_input_unavailable_with_dependencies(name, Arc::from([]));
    }

    /// Records a completed missing-input lookup and retains its semantic
    /// probe/read facts with every canonical input alias it settles.
    pub fn mark_input_unavailable_with_dependencies(
        &mut self,
        name: impl Into<String>,
        dependencies: impl Into<Arc<[InputDependency]>>,
    ) {
        let name = name.into();
        let has_area = name.chars().any(|ch| matches!(ch, '/' | '\\' | ':'));
        let dependencies = dependencies.into();
        self.unavailable_input_requests.insert(name.clone());
        for candidate in input_lookup_candidates(&name, has_area) {
            self.input.remove(&candidate);
            self.unavailable_input
                .insert(candidate, Arc::clone(&dependencies));
        }
    }

    /// Installs immutable backing for a non-opening file enquiry.
    ///
    /// Probe backing is deliberately not promoted to [`Self::register_input`]:
    /// a later required read must revisit the host so dependency accounting
    /// can upgrade an authoritative probe to a required read. A prior required
    /// read may, however, answer a later probe from the stronger capability.
    pub fn register_input_probe(&mut self, name: impl Into<String>, resource: FileEnquiryResource) {
        let name = name.into();
        self.unavailable_input_probes.remove(&name);
        self.input_probes
            .insert(name, resource.without_world_record());
    }

    /// Installs a non-opening answer with the semantic observations made by
    /// the host resolver that produced it.
    pub fn register_input_probe_with_dependencies(
        &mut self,
        name: impl Into<String>,
        resource: FileEnquiryResource,
        dependencies: impl Into<Arc<[InputDependency]>>,
    ) {
        let name = name.into();
        self.unavailable_input_probes.remove(&name);
        self.input_probes.insert(
            name,
            resource
                .without_world_record()
                .with_input_dependencies(dependencies),
        );
    }

    /// Records a completed non-opening lookup which found no backing.
    pub fn mark_input_probe_unavailable(&mut self, name: impl Into<String>) {
        self.mark_input_probe_unavailable_with_dependencies(name, Arc::from([]));
    }

    /// Records a completed missing non-opening lookup with its semantic facts.
    pub fn mark_input_probe_unavailable_with_dependencies(
        &mut self,
        name: impl Into<String>,
        dependencies: impl Into<Arc<[InputDependency]>>,
    ) {
        let name = name.into();
        self.input_probes.remove(&name);
        self.unavailable_input_probes
            .insert(name, dependencies.into());
    }

    /// Invalidates retained absence for a path created by this TeX run.
    ///
    /// TeX82 §1275 attempts every `\openin` against the current filename
    /// namespace. An earlier failed probe therefore cannot settle a later
    /// open after §1375 has executed an immediate `\openout` for that path.
    /// Leading current-directory spellings are equivalent at this boundary;
    /// unrelated negative acquisitions remain authoritative.
    pub fn invalidate_input_unavailability_for_output(&mut self, name: &str) {
        self.unavailable_input
            .retain(|candidate, _| !same_current_directory_name(candidate, name));
        self.unavailable_input_requests
            .retain(|candidate| !same_current_directory_name(candidate, name));
        self.unavailable_input_probes
            .retain(|candidate, _| !same_current_directory_name(candidate, name));
    }

    /// Drops image answers whose search touched an output path that has just
    /// been opened. Images retain parsed bytes rather than an active
    /// [`FileContent`] record, so the next use must resolve the current
    /// output before reusing those bytes. Missing image attempts are dropped
    /// as well: an output can turn an earlier negative into a positive.
    pub fn invalidate_pdf_images_for_output(&mut self, name: &Path) {
        self.images.retain(|image| {
            !image
                .dependencies
                .iter()
                .any(|dependency| same_current_directory_path(dependency.path(), name))
        });
    }

    /// Drops an authoritative missing-font answer when its output path is
    /// opened. Positive font answers remain in place and validate their
    /// selected metrics through the actual-use helper.
    pub fn invalidate_font_unavailability_for_output(&mut self, name: &Path) {
        let paths = self
            .font_paths
            .iter()
            .filter_map(|(path, id)| {
                (matches!(
                    self.font_resources.get(*id),
                    Some(RetainedFontResource::Unavailable)
                ) && same_current_directory_path(path, name))
                .then_some(path.clone())
            })
            .collect::<Vec<_>>();
        for path in paths {
            self.font_paths.remove(&path);
        }
    }

    /// Registers a host-acquired immutable font resource for one request path.
    pub fn register_font(&mut self, path: impl Into<PathBuf>, resource: FontResource) {
        self.register_font_with_dependencies(path, resource, Arc::from([]));
    }

    /// Registers a font together with the semantic reads used to resolve it.
    pub fn register_font_with_dependencies(
        &mut self,
        path: impl Into<PathBuf>,
        resource: FontResource,
        dependencies: impl Into<Arc<[InputDependency]>>,
    ) {
        self.register_retained_font_with_dependencies(
            path,
            RetainedFontResource::from_active(&resource),
            dependencies,
        );
    }

    fn register_retained_font_with_dependencies(
        &mut self,
        path: impl Into<PathBuf>,
        resource: RetainedFontResource,
        dependencies: impl Into<Arc<[InputDependency]>>,
    ) {
        let path = path.into();
        let dependencies = dependencies.into();
        if let Some(id) = self.font_paths.get(&path).copied() {
            self.font_resources
                .replace_retained(id, resource, dependencies);
            return;
        }
        let id = self.font_resources.push_retained(resource, dependencies);
        self.font_paths.insert(path, id);
    }

    /// Registers a validated immutable image response for its exact request.
    pub fn register_pdf_image(&mut self, request: PdfImageRequest, resource: PdfImageResource) {
        self.register_pdf_image_with_dependencies(request, resource, Arc::from([]));
    }

    /// Registers an image response together with the semantic reads used to
    /// resolve that exact request.
    pub fn register_pdf_image_with_dependencies(
        &mut self,
        request: PdfImageRequest,
        resource: PdfImageResource,
        dependencies: impl Into<Arc<[InputDependency]>>,
    ) {
        let dependencies = dependencies.into();
        let selection = retained_image_selection(&dependencies);
        if let Some(existing) = self
            .images
            .iter_mut()
            .find(|image| image.request.same_resource_as(&request))
        {
            existing.resource = resource;
            existing.dependencies = dependencies;
            existing.selection = selection;
        } else {
            self.images.push(HostPdfImageResource {
                request,
                resource,
                dependencies,
                selection,
            });
        }
    }

    #[must_use]
    pub fn pdf_image(&self, request: &PdfImageRequest) -> Option<PdfImageResource> {
        self.images
            .iter()
            .find(|image| image.request.same_resource_as(request))
            .map(|image| image.resource.clone())
    }

    #[must_use]
    pub fn pdf_image_with_dependencies(
        &self,
        request: &PdfImageRequest,
    ) -> Option<(PdfImageResource, Arc<[InputDependency]>)> {
        self.images
            .iter()
            .find(|image| image.request.same_resource_as(request))
            .map(|image| (image.resource.clone(), Arc::clone(&image.dependencies)))
    }

    /// Returns the selected physical path and its retained origin for a
    /// value-only image capability.
    #[must_use]
    pub fn pdf_image_selection(&self, request: &PdfImageRequest) -> Option<(PathBuf, InputOrigin)> {
        self.images
            .iter()
            .find(|image| image.request.same_resource_as(request))
            .and_then(|image| image.selection.as_ref())
            .map(|selection| (selection.path.clone(), selection.origin))
    }

    /// Refreshes an image's origin after provider settlement. The active host
    /// read already happened; this only records its current same-run status.
    pub fn set_pdf_image_origin(&mut self, request: &PdfImageRequest, origin: InputOrigin) {
        if let Some(image) = self
            .images
            .iter_mut()
            .find(|image| image.request.same_resource_as(request))
            && let Some(selection) = image.selection.as_mut()
        {
            selection.origin = origin;
        }
    }

    /// Drops one positive image answer after its selected output bytes have
    /// changed. The next cold use must ask the provider to parse current
    /// bytes and metadata.
    pub fn invalidate_pdf_image(&mut self, request: &PdfImageRequest) {
        self.images
            .retain(|image| !image.request.same_resource_as(request));
    }

    /// Borrows a registered font resource for one replay operation. The
    /// capability owner itself is transient and excluded from snapshots.
    #[must_use]
    pub fn font(&self, path: &Path) -> Option<&RetainedFontResource> {
        let id = self.font_paths.get(path)?;
        self.font_resources.get(*id)
    }

    /// Drops a positive input capability whose selected backing is no longer
    /// valid in the current World timeline. The next use must resolve again.
    pub fn invalidate_input_resource(&mut self, name: &str) {
        self.input.remove(name);
    }

    /// Drops a positive probe capability whose selected backing is no longer
    /// valid in the current World timeline. A required input capability used
    /// as a stronger probe answer is dropped from the same exact namespace.
    pub fn invalidate_input_probe_resource(&mut self, name: &str) {
        self.input.remove(name);
        self.input_probes.remove(name);
    }

    /// Drops a positive font capability whose metrics output disappeared.
    pub fn invalidate_font_resource(&mut self, path: &Path) {
        self.font_paths.remove(path);
    }

    #[must_use]
    pub fn font_dependencies(&self, path: &Path) -> Option<Arc<[InputDependency]>> {
        let id = self.font_paths.get(path)?;
        self.font_resources
            .dependencies(*id)
            .map(|dependencies| dependencies.to_vec().into())
    }

    /// Borrows immutable bytes selected by the host for an input-stream
    /// request.  This is intentionally separate from command-owned `\\input`
    /// registration: replay may pin the same bytes in World without gaining a
    /// source cursor.
    #[must_use]
    pub fn input_resource(&self, name: &str) -> Option<SourceRegistration> {
        self.input.get(name).cloned()
    }

    /// Reports that the host has authoritatively completed lookup without a
    /// backing resource. This differs from an absent entry, which requests a
    /// retry through the retained resource protocol.
    #[must_use]
    pub fn input_resource_is_unavailable(&self, name: &str) -> bool {
        self.unavailable_input.contains_key(name)
    }

    #[must_use]
    pub fn input_unavailable_dependencies(&self, name: &str) -> Arc<[InputDependency]> {
        self.unavailable_input
            .get(name)
            .cloned()
            .unwrap_or_else(|| Arc::from([]))
    }

    /// Borrows bytes acquired for a non-opening enquiry. A prior required
    /// input is a stronger acquisition and can answer the same enquiry.
    #[must_use]
    pub fn input_probe_resource(&self, name: &str) -> Option<FileEnquiryResource> {
        self.input
            .get(name)
            .map(|source| FileEnquiryResource::new(source.clone(), source.modification_date()))
            .or_else(|| self.input_probes.get(name).cloned())
    }

    /// Reports an authoritative absence for a non-opening enquiry.
    #[must_use]
    pub fn input_probe_is_unavailable(&self, name: &str) -> bool {
        self.unavailable_input_requests.contains(name)
            || self.unavailable_input_probes.contains_key(name)
    }

    #[must_use]
    pub fn input_probe_dependencies(&self, name: &str) -> Arc<[InputDependency]> {
        if let Some(source) = self.input.get(name) {
            return Arc::from(source.input_dependencies());
        }
        if let Some(resource) = self.input_probes.get(name) {
            return Arc::from(resource.source().input_dependencies());
        }
        self.unavailable_input_probes
            .get(name)
            .cloned()
            .or_else(|| self.unavailable_input.get(name).cloned())
            .unwrap_or_else(|| Arc::from([]))
    }

    /// Sets the immutable job name presented by `\jobname` for this command
    /// operation.
    pub fn set_job_name(&mut self, name: impl Into<String>) {
        self.job_name = name.into();
    }

    /// Returns the job name installed by [`Self::set_job_name`] or
    /// [`Self::set_startup_job_name`].
    ///
    /// tex.web §1333's `close_files_and_terminate` prints this as the
    /// transcript file's stem (`slow_print(log_name)`, where `log_name` is
    /// this name with `.log` appended); that print lives in `tex-exec`, past
    /// this crate's no-printing boundary, so it needs read access rather than
    /// the `pub(crate)` accessor [`CommandHostContext::job_name`] already
    /// gives command-internal callers.
    #[must_use]
    pub fn job_name(&self) -> &str {
        &self.job_name
    }

    /// Installs the TeX job name selected by startup input.
    ///
    /// TeX's filename scanner separates the supplied terminal filename into
    /// area, name, and extension; `\\jobname` renders the name alone.  This
    /// keeps that environment-neutral lifecycle fact at the typed host
    /// boundary, rather than deriving conversion text from an observer or a
    /// fixture path.
    pub fn set_startup_job_name(&mut self, filename: &str) {
        let leaf = filename
            .rsplit(['/', '\\'])
            .next()
            .expect("splitting a string always yields one component");
        let name = leaf.rsplit_once('.').map_or(leaf, |(stem, _)| stem);
        self.set_job_name(name);
    }
}

/// Borrowed executor authority for live mode and effective-tail enquiries.
///
/// Each method is one semantic fact request. Implementations sample the live
/// owner when called; they must not prefill a whole-operation cache. The
/// command processor retains this borrow only for its synchronous episode, so
/// no mode/list owner or fact payload can enter command state or suspension.
pub trait CommandHostFacts<G> {
    fn conditional_state(&mut self) -> ConditionalState;
    fn space_factor(&mut self) -> Option<i32>;
    /// Returns the executor mode name at a cold diagnostic report boundary.
    /// Implementations must sample the live mode nest; the command core never
    /// stores this fact in snapshots or delivery state.
    fn diagnostic_mode_name(&mut self) -> &'static str {
        "vertical mode"
    }
    fn prev_depth(&mut self, state: &tex_state::CommandContext<'_, G>) -> Option<Scaled>;
    fn prev_graf(&mut self) -> Option<i32>;
    fn last_node(&mut self, state: &tex_state::CommandContext<'_, G>) -> Option<LastNodeItem>;
    fn last_node_type(&mut self, state: &tex_state::CommandContext<'_, G>) -> i32;
}

/// Cold-only provider for resources that are absent from command capabilities.
/// Implementations receive a closure-scoped command-state world view and must
/// return owned data before the borrow ends. Input reads and their dependency
/// facts are performed through that view; the returned dependency list mirrors
/// those already-recorded facts so an initial installation does not apply the
/// same World mutation twice. A later capability hit records the retained
/// dependencies as a normal semantic reuse.
pub trait ResourceProvider<G> {
    fn resolve(
        &mut self,
        state: &mut tex_state::CommandContext<'_, G>,
        need: &ResourceNeed,
    ) -> ResourceResolution;
}

/// Exact initial outer-vertical facts for command processors without an
/// executor mode nest, such as tokenizer/scanner fixtures and stream tools.
///
/// This zero-sized provider is not a cache: every method directly describes
/// TeX's initial mode and empty list.
#[derive(Debug, Default)]
struct InitialCommandHostFacts;

impl<G> CommandHostFacts<G> for InitialCommandHostFacts {
    fn conditional_state(&mut self) -> ConditionalState {
        ConditionalState::new(ConditionalMode::Vertical, false)
    }

    fn space_factor(&mut self) -> Option<i32> {
        None
    }

    fn diagnostic_mode_name(&mut self) -> &'static str {
        "vertical mode"
    }

    fn prev_depth(&mut self, _state: &tex_state::CommandContext<'_, G>) -> Option<Scaled> {
        None
    }

    fn prev_graf(&mut self) -> Option<i32> {
        Some(0)
    }

    fn last_node(&mut self, _state: &tex_state::CommandContext<'_, G>) -> Option<LastNodeItem> {
        None
    }

    fn last_node_type(&mut self, _state: &tex_state::CommandContext<'_, G>) -> i32 {
        -1
    }
}

/// Returns the exact ordered names tried by canonical `\input` lookup.
/// Keeping acquisition settlement on this same helper prevents an
/// authoritative answer for a bare name from leaving its bounded TeXinputs
/// fallback unresolved.
pub(crate) fn input_lookup_candidates(packed_name: &str, has_area: bool) -> Vec<String> {
    let mut candidates = vec![packed_name.to_owned()];
    if !has_area {
        candidates.push(format!("TeXinputs:{packed_name}"));
    }
    candidates
}

fn same_current_directory_name(left: &str, right: &str) -> bool {
    trim_current_directory_prefix(left) == trim_current_directory_prefix(right)
}

fn retained_image_selection(dependencies: &[InputDependency]) -> Option<RetainedImageSelection> {
    dependencies
        .iter()
        .find(|dependency| matches!(dependency.outcome(), InputDependencyOutcome::Present(_)))
        .map(|dependency| RetainedImageSelection {
            path: dependency.path().to_owned(),
            origin: InputOrigin::External,
        })
}

fn same_current_directory_path(left: &Path, right: &Path) -> bool {
    trim_current_directory_prefix_path(left) == trim_current_directory_prefix_path(right)
}

fn trim_current_directory_prefix_path(mut path: &Path) -> &Path {
    while let Ok(rest) = path.strip_prefix(".") {
        path = rest;
    }
    path
}

fn trim_current_directory_prefix(mut name: &str) -> &str {
    while let Some(rest) = name.strip_prefix("./") {
        name = rest;
    }
    name
}

/// A non-owning host-capability boundary for one command-processor operation.
///
/// The mutable borrow makes the capability scope explicit and prevents the
/// context from entering owned command state, snapshots, or formats.
pub struct CommandHostContext<'a, G> {
    capabilities: &'a mut CommandHostCapabilities,
    facts: CommandHostFactAccess<'a, G>,
    resource_provider: Option<&'a mut dyn ResourceProvider<G>>,
    attempted_resource: Option<&'a mut Option<ResourceNeed>>,
}

enum CommandHostFactAccess<'a, G> {
    Initial(InitialCommandHostFacts),
    Borrowed(&'a mut dyn CommandHostFacts<G>),
}

impl<'a, G> CommandHostContext<'a, G> {
    /// Borrows resource capabilities for a processor outside an executor.
    /// Such processors observe TeX's exact initial outer-vertical facts.
    #[must_use]
    pub fn new(capabilities: &'a mut CommandHostCapabilities) -> Self {
        Self {
            capabilities,
            facts: CommandHostFactAccess::Initial(InitialCommandHostFacts),
            resource_provider: None,
            attempted_resource: None,
        }
    }

    /// Borrows resource capabilities and the live executor fact provider for
    /// one synchronous processor episode.
    #[must_use]
    pub fn with_facts(
        capabilities: &'a mut CommandHostCapabilities,
        facts: &'a mut dyn CommandHostFacts<G>,
    ) -> Self {
        Self {
            capabilities,
            facts: CommandHostFactAccess::Borrowed(facts),
            resource_provider: None,
            attempted_resource: None,
        }
    }

    /// Borrows capabilities, live facts, and a cold-only provider for one
    /// synchronous command episode. The optional attempt slot is a
    /// call-scoped sideband consumed by the outer driver when a provider
    /// declines after already making its one host call.
    #[must_use]
    pub fn with_facts_and_resource_provider(
        capabilities: &'a mut CommandHostCapabilities,
        facts: &'a mut dyn CommandHostFacts<G>,
        resource_provider: &'a mut dyn ResourceProvider<G>,
        attempted_resource: &'a mut Option<ResourceNeed>,
    ) -> Self {
        Self {
            capabilities,
            facts: CommandHostFactAccess::Borrowed(facts),
            resource_provider: Some(resource_provider),
            attempted_resource: Some(attempted_resource),
        }
    }

    /// Borrows capabilities and a cold-only provider with initial facts.
    #[must_use]
    pub fn with_resource_provider(
        capabilities: &'a mut CommandHostCapabilities,
        resource_provider: &'a mut dyn ResourceProvider<G>,
        attempted_resource: &'a mut Option<ResourceNeed>,
    ) -> Self {
        Self {
            capabilities,
            facts: CommandHostFactAccess::Initial(InitialCommandHostFacts),
            resource_provider: Some(resource_provider),
            attempted_resource: Some(attempted_resource),
        }
    }

    /// Calls the provider synchronously, if one is installed. The returned
    /// resolution owns every payload and dependency, so no provider or World
    /// borrow survives this call.
    pub(crate) fn resolve_resource(
        &mut self,
        state: &mut tex_state::CommandContext<'_, G>,
        need: &ResourceNeed,
    ) -> Option<ResourceResolution> {
        let provider = self.resource_provider.as_deref_mut()?;
        let resolution = provider.resolve(state, need);
        if matches!(resolution.outcome, ResourceOutcome::Declined)
            && let Some(attempted) = self.attempted_resource.as_deref_mut()
        {
            *attempted = Some(need.clone());
        }
        Some(resolution)
    }

    /// Resolves and installs one provider answer in the canonical capability
    /// owner. `None` means this context has no provider and callers should
    /// preserve their ordinary missing-capability path.
    pub(crate) fn resolve_and_install_resource(
        &mut self,
        state: &mut tex_state::CommandContext<'_, G>,
        need: &ResourceNeed,
        register_texinputs_alias: bool,
    ) -> Result<Option<ResourceInstallOutcome>, crate::CommandError> {
        let Some(resolution) = self.resolve_resource(state, need) else {
            return Ok(None);
        };
        self.capabilities
            .install_resource_resolution(need, resolution, register_texinputs_alias)
            .map(Some)
            .map_err(|_| crate::CommandError::ResourceFailure {
                need: Box::new(need.clone()),
                failure: Box::new(crate::ResourceFailure::message(
                    "resource provider returned a mismatched typed answer",
                )),
            })
            .and_then(|outcome| match outcome {
                Some(ResourceInstallOutcome::Failed(failure)) => {
                    Err(crate::CommandError::ResourceFailure {
                        need: Box::new(need.clone()),
                        failure: Box::new(failure),
                    })
                }
                other => Ok(other),
            })
    }

    pub(crate) fn input(&self, name: &str) -> Option<SourceRegistration> {
        self.capabilities.input.get(name).cloned()
    }

    pub(crate) fn input_is_unavailable(&self, name: &str) -> bool {
        self.capabilities.unavailable_input.contains_key(name)
    }

    pub(crate) fn input_unavailable_dependencies(&self, name: &str) -> Arc<[InputDependency]> {
        self.capabilities.input_unavailable_dependencies(name)
    }

    pub(crate) fn input_probe(&self, name: &str) -> Option<FileEnquiryResource> {
        self.capabilities.input_probe_resource(name)
    }

    pub(crate) fn input_probe_is_unavailable(&self, name: &str) -> bool {
        self.capabilities.input_probe_is_unavailable(name)
    }

    pub(crate) fn input_probe_dependencies(&self, name: &str) -> Arc<[InputDependency]> {
        self.capabilities.input_probe_dependencies(name)
    }

    pub(crate) fn initialize_job_name(&mut self, filename: &str) {
        if self.capabilities.job_name.is_empty() {
            self.capabilities.set_startup_job_name(filename);
        }
    }

    /// Resolves a previously registered font only while the host capability
    /// is borrowed by a bounded replay operation.
    #[must_use]
    pub fn font(&self, path: &Path) -> Option<&RetainedFontResource> {
        self.capabilities.font(path)
    }

    pub(crate) fn invalidate_input_resource(&mut self, name: &str) {
        self.capabilities.invalidate_input_resource(name);
    }

    pub(crate) fn invalidate_input_probe_resource(&mut self, name: &str) {
        self.capabilities.invalidate_input_probe_resource(name);
    }

    #[must_use]
    pub fn font_dependencies(&self, path: &Path) -> Option<Arc<[InputDependency]>> {
        self.capabilities.font_dependencies(path)
    }

    #[must_use]
    pub fn pdf_image(&self, request: &PdfImageRequest) -> Option<PdfImageResource> {
        self.capabilities.pdf_image(request)
    }

    #[must_use]
    pub fn pdf_image_with_dependencies(
        &self,
        request: &PdfImageRequest,
    ) -> Option<(PdfImageResource, Arc<[InputDependency]>)> {
        self.capabilities.pdf_image_with_dependencies(request)
    }

    pub(crate) fn job_name(&self) -> &str {
        &self.capabilities.job_name
    }

    pub(crate) fn conditional_state(&mut self) -> ConditionalState {
        match &mut self.facts {
            CommandHostFactAccess::Initial(facts) => {
                <InitialCommandHostFacts as CommandHostFacts<G>>::conditional_state(facts)
            }
            CommandHostFactAccess::Borrowed(facts) => facts.conditional_state(),
        }
    }

    #[must_use]
    pub(crate) fn space_factor(&mut self) -> Option<i32> {
        match &mut self.facts {
            CommandHostFactAccess::Initial(facts) => {
                <InitialCommandHostFacts as CommandHostFacts<G>>::space_factor(facts)
            }
            CommandHostFactAccess::Borrowed(facts) => facts.space_factor(),
        }
    }

    #[must_use]
    pub(crate) fn diagnostic_mode_name(&mut self) -> &'static str {
        match &mut self.facts {
            CommandHostFactAccess::Initial(facts) => {
                <InitialCommandHostFacts as CommandHostFacts<G>>::diagnostic_mode_name(facts)
            }
            CommandHostFactAccess::Borrowed(facts) => facts.diagnostic_mode_name(),
        }
    }

    #[must_use]
    pub(crate) fn prev_depth(
        &mut self,
        state: &tex_state::CommandContext<'_, G>,
    ) -> Option<Scaled> {
        match &mut self.facts {
            CommandHostFactAccess::Initial(facts) => facts.prev_depth(state),
            CommandHostFactAccess::Borrowed(facts) => facts.prev_depth(state),
        }
    }

    #[must_use]
    pub(crate) fn prev_graf(&mut self) -> Option<i32> {
        match &mut self.facts {
            CommandHostFactAccess::Initial(facts) => {
                <InitialCommandHostFacts as CommandHostFacts<G>>::prev_graf(facts)
            }
            CommandHostFactAccess::Borrowed(facts) => facts.prev_graf(),
        }
    }

    #[must_use]
    pub(crate) fn last_node(
        &mut self,
        state: &tex_state::CommandContext<'_, G>,
    ) -> Option<LastNodeItem> {
        match &mut self.facts {
            CommandHostFactAccess::Initial(facts) => facts.last_node(state),
            CommandHostFactAccess::Borrowed(facts) => facts.last_node(state),
        }
    }

    #[must_use]
    pub(crate) fn last_node_type(&mut self, state: &tex_state::CommandContext<'_, G>) -> i32 {
        match &mut self.facts {
            CommandHostFactAccess::Initial(facts) => facts.last_node_type(state),
            CommandHostFactAccess::Borrowed(facts) => facts.last_node_type(state),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CommandHostCapabilities, FileEnquiryResource, FontResource, HostFontResourceId,
        RetainedFontResource,
    };
    use crate::RegisteredSourceKind;
    use std::path::PathBuf;
    use tex_state::{ContentHash, InputDependency, InputDependencyAccess, InputDependencyOutcome};

    fn dependency(path: &str) -> InputDependency {
        InputDependency::new(
            path,
            InputDependencyOutcome::Present(ContentHash::from_bytes(path.as_bytes())),
            InputDependencyAccess::RequiredRead,
        )
    }

    #[test]
    fn retained_capability_bindings_keep_only_their_read_metadata() {
        let mut capabilities = CommandHostCapabilities::default();
        let input_dependency = dependency("/vfs/cached.tex");
        let probe_dependency = dependency("/vfs/probe.cfg");
        let font_dependency = dependency("/vfs/cmr10.tfm");

        capabilities.register_input_with_dependencies(
            "cached.tex",
            super::SourceRegistration::new(RegisteredSourceKind::Generated, b"\\relax".as_slice()),
            vec![input_dependency.clone()],
        );
        assert_eq!(
            capabilities
                .input_resource("cached.tex")
                .expect("input binding")
                .input_dependencies(),
            [input_dependency]
        );

        capabilities.register_input_probe_with_dependencies(
            "probe.cfg",
            FileEnquiryResource::new(
                super::SourceRegistration::new(RegisteredSourceKind::Generated, b"cfg".as_slice()),
                None,
            ),
            vec![probe_dependency.clone()],
        );
        assert_eq!(
            capabilities.input_probe_dependencies("probe.cfg").as_ref(),
            std::slice::from_ref(&probe_dependency)
        );

        capabilities.register_font_with_dependencies(
            "cmr10.tfm",
            FontResource::Unavailable,
            vec![font_dependency.clone()],
        );
        assert_eq!(
            capabilities
                .font_dependencies(PathBuf::from("cmr10.tfm").as_path())
                .expect("font binding")
                .as_ref(),
            [font_dependency]
        );

        capabilities.mark_input_probe_unavailable_with_dependencies(
            "missing.cfg",
            vec![probe_dependency.clone()],
        );
        assert_eq!(
            capabilities
                .input_probe_dependencies("missing.cfg")
                .as_ref(),
            std::slice::from_ref(&probe_dependency)
        );
    }

    #[test]
    fn font_capability_index_is_compact_and_stable_across_ordered_insertions() {
        assert_eq!(std::mem::size_of::<HostFontResourceId>(), 4);
        #[cfg(target_pointer_width = "64")]
        assert_eq!(std::mem::size_of::<FontResource>(), 504);
        assert!(
            std::mem::size_of::<(PathBuf, HostFontResourceId)>()
                < std::mem::size_of::<(PathBuf, FontResource)>(),
            "the ordered path index must not carry the wide resource payload"
        );

        let mut capabilities = CommandHostCapabilities::default();
        let retained = PathBuf::from("retained.tfm");
        capabilities.register_font(&retained, FontResource::Unavailable);
        let retained_id = capabilities.font_paths[&retained];
        let retained_address = std::ptr::from_ref(
            capabilities
                .font(&retained)
                .expect("registered resource is available"),
        );
        for index in 0..64 {
            capabilities.register_font(format!("before-{index:02}.tfm"), FontResource::Unavailable);
        }

        assert_eq!(capabilities.font_paths[&retained], retained_id);
        assert_eq!(
            std::ptr::from_ref(
                capabilities
                    .font(&retained)
                    .expect("retained resource is available"),
            ),
            retained_address,
            "later registration must not relocate an immutable resource"
        );
        assert!(matches!(
            capabilities.font(&retained),
            Some(RetainedFontResource::Unavailable)
        ));
        capabilities.register_font(&retained, FontResource::Unavailable);
        assert_eq!(capabilities.font_resources.len(), 65);
    }

    #[test]
    fn startup_job_name_is_the_filename_stem_without_its_area() {
        let mut capabilities = CommandHostCapabilities::default();
        capabilities.set_startup_job_name("inputs/annual.report.tex");

        assert_eq!(capabilities.job_name, "annual.report");
    }

    #[test]
    fn unavailable_bare_input_settles_only_its_canonical_aliases() {
        let mut capabilities = CommandHostCapabilities::default();
        capabilities.mark_input_probe_unavailable("absent.tex");
        capabilities.mark_input_unavailable("absent.tex");

        assert!(capabilities.input_resource_is_unavailable("absent.tex"));
        assert!(capabilities.input_resource_is_unavailable("TeXinputs:absent.tex"));
        assert!(!capabilities.input_resource_is_unavailable("other.tex"));
        assert!(capabilities.input_probe_is_unavailable("absent.tex"));
        assert!(!capabilities.input_probe_is_unavailable("TeXinputs:absent.tex"));
        assert!(!capabilities.input_probe_is_unavailable("other.tex"));
    }

    #[test]
    fn required_and_probe_unavailability_keep_their_request_namespaces() {
        let mut capabilities = CommandHostCapabilities::default();
        capabilities.mark_input_unavailable("inputs/absent.tex");
        capabilities.mark_input_probe_unavailable("probe.tex");

        // TeX82 §537 input settlement may cover the bounded aliases that the
        // opening lookup itself would try. An explicit area has no such alias.
        assert!(capabilities.input_resource_is_unavailable("inputs/absent.tex"));
        assert!(!capabilities.input_resource_is_unavailable("TeXinputs:inputs/absent.tex"));

        // TeX82 §1275 and pdftex.web §1590 enquire about the exact packed
        // name. A stronger required answer settles that same name, while a
        // probe-only answer neither settles an input nor inherits aliases.
        assert!(capabilities.input_probe_is_unavailable("inputs/absent.tex"));
        assert!(capabilities.input_probe_is_unavailable("probe.tex"));
        assert!(!capabilities.input_resource_is_unavailable("probe.tex"));
        assert!(!capabilities.input_probe_is_unavailable("TeXinputs:probe.tex"));
    }

    #[test]
    fn same_run_output_invalidates_only_equivalent_input_absence() {
        let mut capabilities = CommandHostCapabilities::default();
        capabilities.mark_input_unavailable("generated.csv");
        capabilities.mark_input_probe_unavailable("./generated.csv");
        capabilities.mark_input_probe_unavailable("unchanged.csv");

        capabilities.invalidate_input_unavailability_for_output("././generated.csv");

        assert!(!capabilities.input_resource_is_unavailable("generated.csv"));
        assert!(!capabilities.input_probe_is_unavailable("generated.csv"));
        assert!(!capabilities.input_probe_is_unavailable("./generated.csv"));
        assert!(capabilities.input_probe_is_unavailable("unchanged.csv"));
        assert!(capabilities.input_resource_is_unavailable("TeXinputs:generated.csv"));
    }
}
