//! Borrow-scoped host capabilities.

use std::collections::{BTreeMap, BTreeSet};

use crate::{PdfImageRequest, SourceRegistration};
use std::path::{Path, PathBuf};
use tex_state::glue::GlueSpec;
use tex_state::scaled::Scaled;
use tex_state::world::FileContent;

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
}

impl FileEnquiryResource {
    #[must_use]
    pub fn new(
        source: SourceRegistration,
        modification_date: Option<tex_state::FileModificationDate>,
    ) -> Self {
        Self {
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

    #[must_use]
    pub const fn modification_date(&self) -> Option<tex_state::FileModificationDate> {
        self.modification_date
    }
}

/// Immutable font bytes selected by the host for a canonical `\\font` replay.
///
/// This transient capability value is never retained by command state or
/// snapshots: a missing entry is an explicit resource suspension at the
/// replay boundary.
#[derive(Clone, Debug)]
pub enum FontResource {
    /// The host completed lookup and determined that no font is available.
    /// This differs from an absent capability entry, which suspends replay.
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
    chunks: Vec<Vec<FontResource>>,
    len: u32,
}

impl HostFontResources {
    fn push(&mut self, resource: FontResource) -> HostFontResourceId {
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
            .push(resource);
        self.len = self
            .len
            .checked_add(1)
            .expect("font capability count exceeds u32");
        id
    }

    fn get(&self, id: HostFontResourceId) -> Option<&FontResource> {
        let raw = id.0 as usize;
        self.chunks
            .get(raw / HOST_FONT_RESOURCE_CHUNK_CAPACITY)?
            .get(raw % HOST_FONT_RESOURCE_CHUNK_CAPACITY)
    }

    fn replace(&mut self, id: HostFontResourceId, resource: FontResource) {
        let raw = id.0 as usize;
        self.chunks[raw / HOST_FONT_RESOURCE_CHUNK_CAPACITY]
            [raw % HOST_FONT_RESOURCE_CHUNK_CAPACITY] = resource;
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
    unavailable_input: BTreeSet<String>,
    unavailable_input_requests: BTreeSet<String>,
    input_probes: BTreeMap<String, FileEnquiryResource>,
    unavailable_input_probes: BTreeSet<String>,
    font_paths: BTreeMap<PathBuf, HostFontResourceId>,
    font_resources: HostFontResources,
    images: Vec<(PdfImageRequest, PdfImageResource)>,
    job_name: String,
}

impl CommandHostCapabilities {
    /// Installs immutable backing for one logical `\input` request.
    ///
    /// Acquisition is complete before this capability is constructed.  The
    /// command machine can therefore request only retained bytes and never
    /// opens files itself.
    pub fn register_input(&mut self, name: impl Into<String>, source: SourceRegistration) {
        let name = name.into();
        self.unavailable_input.remove(&name);
        self.unavailable_input_requests.remove(&name);
        self.input.insert(name, source);
    }

    /// Records a completed host lookup which found no input backing.
    pub fn mark_input_unavailable(&mut self, name: impl Into<String>) {
        let name = name.into();
        let has_area = name.chars().any(|ch| matches!(ch, '/' | '\\' | ':'));
        self.unavailable_input_requests.insert(name.clone());
        for candidate in input_lookup_candidates(&name, has_area) {
            self.input.remove(&candidate);
            self.unavailable_input.insert(candidate);
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
        self.input_probes.insert(name, resource);
    }

    /// Records a completed non-opening lookup which found no backing.
    pub fn mark_input_probe_unavailable(&mut self, name: impl Into<String>) {
        let name = name.into();
        self.input_probes.remove(&name);
        self.unavailable_input_probes.insert(name);
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
            .retain(|candidate| !same_current_directory_name(candidate, name));
        self.unavailable_input_requests
            .retain(|candidate| !same_current_directory_name(candidate, name));
        self.unavailable_input_probes
            .retain(|candidate| !same_current_directory_name(candidate, name));
    }

    /// Registers a host-acquired immutable font resource for one request path.
    pub fn register_font(&mut self, path: impl Into<PathBuf>, resource: FontResource) {
        let path = path.into();
        if let Some(id) = self.font_paths.get(&path).copied() {
            self.font_resources.replace(id, resource);
            return;
        }
        let id = self.font_resources.push(resource);
        self.font_paths.insert(path, id);
    }

    /// Registers a validated immutable image response for its exact request.
    pub fn register_pdf_image(&mut self, request: PdfImageRequest, resource: PdfImageResource) {
        if let Some((_, existing)) = self
            .images
            .iter_mut()
            .find(|(key, _)| key.same_resource_as(&request))
        {
            *existing = resource;
        } else {
            self.images.push((request, resource));
        }
    }

    #[must_use]
    pub fn pdf_image(&self, request: &PdfImageRequest) -> Option<PdfImageResource> {
        self.images
            .iter()
            .find(|(key, _)| key.same_resource_as(request))
            .map(|(_, resource)| resource.clone())
    }

    /// Borrows a registered font resource for one replay operation. The
    /// capability owner itself is transient and excluded from snapshots.
    #[must_use]
    pub fn font(&self, path: &Path) -> Option<&FontResource> {
        let id = self.font_paths.get(path)?;
        self.font_resources.get(*id)
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
        self.unavailable_input.contains(name)
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
            || self.unavailable_input_probes.contains(name)
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
    fn prev_depth(&mut self, state: &tex_state::CommandContext<'_, G>) -> Option<Scaled>;
    fn prev_graf(&mut self) -> Option<i32>;
    fn last_node(&mut self, state: &tex_state::CommandContext<'_, G>) -> Option<LastNodeItem>;
    fn last_node_type(&mut self, state: &tex_state::CommandContext<'_, G>) -> i32;
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
        }
    }

    pub(crate) fn input(&self, name: &str) -> Option<SourceRegistration> {
        self.capabilities.input.get(name).cloned()
    }

    pub(crate) fn input_is_unavailable(&self, name: &str) -> bool {
        self.capabilities.unavailable_input.contains(name)
    }

    pub(crate) fn input_probe(&self, name: &str) -> Option<FileEnquiryResource> {
        self.capabilities.input_probe_resource(name)
    }

    pub(crate) fn input_probe_is_unavailable(&self, name: &str) -> bool {
        self.capabilities.input_probe_is_unavailable(name)
    }

    pub(crate) fn initialize_job_name(&mut self, filename: &str) {
        if self.capabilities.job_name.is_empty() {
            self.capabilities.set_startup_job_name(filename);
        }
    }

    /// Resolves a previously registered font only while the host capability
    /// is borrowed by a bounded replay operation.
    #[must_use]
    pub fn font(&self, path: &Path) -> Option<&FontResource> {
        self.capabilities.font(path)
    }

    #[must_use]
    pub fn pdf_image(&self, request: &PdfImageRequest) -> Option<PdfImageResource> {
        self.capabilities.pdf_image(request)
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
    use super::{CommandHostCapabilities, FontResource, HostFontResourceId};
    use std::path::PathBuf;

    #[test]
    fn font_capability_index_is_compact_and_stable_across_ordered_insertions() {
        assert_eq!(std::mem::size_of::<HostFontResourceId>(), 4);
        #[cfg(target_pointer_width = "64")]
        assert_eq!(std::mem::size_of::<FontResource>(), 512);
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
            Some(FontResource::Unavailable)
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
