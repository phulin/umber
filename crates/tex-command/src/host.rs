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

/// Opaque capability set installed by the executor for one bounded operation.
///
/// The fields remain private so host access can only be introduced as typed
/// command-core operations. This value is intentionally neither serializable
/// nor cloneable.
#[derive(Debug)]
pub struct CommandHostCapabilities {
    input: BTreeMap<String, SourceRegistration>,
    unavailable_input: BTreeSet<String>,
    unavailable_input_requests: BTreeSet<String>,
    input_probes: BTreeMap<String, FileEnquiryResource>,
    unavailable_input_probes: BTreeSet<String>,
    fonts: BTreeMap<PathBuf, FontResource>,
    images: Vec<(PdfImageRequest, PdfImageResource)>,
    job_name: String,
    conditional_state: ConditionalState,
    space_factor: Option<i32>,
    prev_depth: Option<Scaled>,
    prev_graf: Option<i32>,
    last_node: Option<LastNodeItem>,
    last_node_type: i32,
}

impl Default for CommandHostCapabilities {
    fn default() -> Self {
        Self {
            input: BTreeMap::new(),
            unavailable_input: BTreeSet::new(),
            unavailable_input_requests: BTreeSet::new(),
            input_probes: BTreeMap::new(),
            unavailable_input_probes: BTreeSet::new(),
            fonts: BTreeMap::new(),
            images: Vec::new(),
            job_name: String::new(),
            // A processor outside main control observes TeX's initial outer
            // vertical mode. Real execution replaces this per operation.
            conditional_state: ConditionalState::new(ConditionalMode::Vertical, false),
            space_factor: None,
            prev_depth: None,
            prev_graf: None,
            last_node: None,
            last_node_type: -1,
        }
    }
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
        self.fonts.insert(path.into(), resource);
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
    pub fn font(&self, path: &Path) -> Option<FontResource> {
        self.fonts.get(path).cloned()
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

    /// Installs the current executor-owned mode query result for this command
    /// operation. It is deliberately capability state rather than a field of
    /// `CommandState`, so snapshots never duplicate the mode nest.
    pub fn set_conditional_state(&mut self, state: ConditionalState) {
        self.conditional_state = state;
    }

    /// Installs the current horizontal-list space factor for one command
    /// operation. `None` records that the executor is not in horizontal mode,
    /// where TeX does not expose this internal quantity.
    pub fn set_space_factor(&mut self, space_factor: Option<i32>) {
        self.space_factor = space_factor;
    }

    /// Installs the current vertical list's `prev_depth` for one command
    /// operation. `None` records that the executor is not in vertical mode,
    /// where tex.web's §418 reports "Improper \prevdepth" and reads zero.
    pub fn set_prev_depth(&mut self, prev_depth: Option<Scaled>) {
        self.prev_depth = prev_depth;
    }

    /// Installs the nearest enclosing vertical level's `prev_graf` for one
    /// command operation, for `\prevgraf` (tex.web §422). `None` records
    /// tex.web's `mode=0` case, which reads zero.
    pub fn set_prev_graf(&mut self, prev_graf: Option<i32>) {
        self.prev_graf = prev_graf;
    }

    /// Installs the current list's tail-node classification for one command
    /// operation, for `\lastpenalty`/`\lastkern`/`\lastskip`. `None` records
    /// that the tail matches none of the three tracked node shapes.
    pub fn set_last_node(&mut self, last_node: Option<LastNodeItem>) {
        self.last_node = last_node;
    }

    /// Supplies e-TeX 2.6 `etex.ch` [26.424]'s effective-tail node code.
    pub fn set_last_node_type(&mut self, last_node_type: i32) {
        self.last_node_type = last_node_type;
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
#[derive(Debug)]
pub struct CommandHostContext<'a> {
    _capabilities: &'a mut CommandHostCapabilities,
}

impl<'a> CommandHostContext<'a> {
    /// Borrows the capabilities installed for one bounded operation.
    #[must_use]
    pub fn new(capabilities: &'a mut CommandHostCapabilities) -> Self {
        Self {
            _capabilities: capabilities,
        }
    }

    pub(crate) fn input(&self, name: &str) -> Option<SourceRegistration> {
        self._capabilities.input.get(name).cloned()
    }

    pub(crate) fn input_is_unavailable(&self, name: &str) -> bool {
        self._capabilities.unavailable_input.contains(name)
    }

    pub(crate) fn input_probe(&self, name: &str) -> Option<FileEnquiryResource> {
        self._capabilities.input_probe_resource(name)
    }

    pub(crate) fn input_probe_is_unavailable(&self, name: &str) -> bool {
        self._capabilities.input_probe_is_unavailable(name)
    }

    pub(crate) fn initialize_job_name(&mut self, filename: &str) {
        if self._capabilities.job_name.is_empty() {
            self._capabilities.set_startup_job_name(filename);
        }
    }

    /// Resolves a previously registered font only while the host capability
    /// is borrowed by a bounded replay operation.
    #[must_use]
    pub fn font(&self, path: &Path) -> Option<FontResource> {
        self._capabilities.font(path)
    }

    #[must_use]
    pub fn pdf_image(&self, request: &PdfImageRequest) -> Option<PdfImageResource> {
        self._capabilities.pdf_image(request)
    }

    pub(crate) fn job_name(&self) -> &str {
        &self._capabilities.job_name
    }

    pub(crate) const fn conditional_state(&self) -> ConditionalState {
        self._capabilities.conditional_state
    }

    #[must_use]
    pub(crate) const fn space_factor(&self) -> Option<i32> {
        self._capabilities.space_factor
    }

    #[must_use]
    pub(crate) const fn prev_depth(&self) -> Option<Scaled> {
        self._capabilities.prev_depth
    }

    #[must_use]
    pub(crate) const fn prev_graf(&self) -> Option<i32> {
        self._capabilities.prev_graf
    }

    #[must_use]
    pub(crate) const fn last_node(&self) -> Option<LastNodeItem> {
        self._capabilities.last_node
    }

    #[must_use]
    pub(crate) const fn last_node_type(&self) -> i32 {
        self._capabilities.last_node_type
    }
}

#[cfg(test)]
mod tests {
    use super::CommandHostCapabilities;

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
