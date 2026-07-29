//! Borrow-scoped host capabilities.

use std::collections::BTreeMap;

use crate::{PdfImageRequest, SourceRegistration};
use std::path::{Path, PathBuf};
use tex_state::glue::GlueSpec;
use tex_state::scaled::Scaled;
use tex_state::world::FileContent;

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
        opentype: Option<tex_fonts::OpenTypeProgramSelection>,
    },
    MappedTfm {
        metrics: FileContent,
        opentype: tex_fonts::OpenTypeProgramSelection,
        encoding_map: tex_fonts::LegacyEncodingMap,
    },
    ClassicTfmFallback {
        metrics: FileContent,
    },
    OpenType(tex_fonts::OpenTypeProgramSelection),
}

/// Host-completed result for a canonical pdfTeX image request.
///
/// Parsed metadata and retained immutable bytes are safe to hand to the
/// engine; a missing map entry means acquisition has not completed and must
/// suspend the aggregate operation instead.
#[derive(Clone, Debug)]
pub enum PdfImageResource {
    Unavailable,
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
        self.input.insert(name.into(), source);
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

    /// Sets the immutable job name presented by `\jobname` for this command
    /// operation.
    pub fn set_job_name(&mut self, name: impl Into<String>) {
        self.job_name = name.into();
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
}
