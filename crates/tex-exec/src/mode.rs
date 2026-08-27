use ahash::RandomState;
use smallvec::{SmallVec, smallvec};
use std::hash::{BuildHasher, Hash, Hasher};
use tex_state::glue::GlueSpec;
use tex_state::ids::FontId;
use tex_state::math::FractionThickness;
use tex_state::node::{BoxNode, Node, NodeTokenList};
use tex_state::node_arena::PageListId;
use tex_state::scaled::Scaled;
use tex_state::token::OriginId;
use tex_state::{CommandContext, EngineBoundaryHasher, EngineMode, Universe};

use crate::ExecError;

#[cfg(test)]
thread_local! {
    static SEMANTIC_FINGERPRINT_CALLS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

mod journal;
pub(crate) use journal::Cursor as ModeJournalCursor;

/// TeX's sentinel depth used before any vertical-list box has established a baseline.
pub const IGNORE_DEPTH: Scaled = Scaled::from_raw(-65_536_000);

/// Returns the engine's live ignored-depth sentinel.
///
/// TeX82 and original e-TeX use the fixed `IGNORE_DEPTH` constant. pdfTeX
/// exposes that value as the assignable `\pdfignoreddimen` parameter and
/// consults the live cell at every prevdepth initialization and comparison.
pub(crate) fn ignored_depth<G>(stores: &CommandContext<'_, G>) -> Scaled {
    if stores.primitive_resolved("pdfignoreddimen").is_some() {
        stores.dimen_param(tex_state::env::banks::DimenParam::PDF_IGNORED_DIMEN)
    } else {
        IGNORE_DEPTH
    }
}

/// Returns the ignored-depth sentinel through a prebound immutable registry
/// handle on the ordinary executor capability-refresh path.
pub(crate) fn ignored_depth_with_handle<G>(
    stores: &CommandContext<'_, G>,
    pdf_ignore_depth: Option<tex_state::PrimitiveHandle<G>>,
) -> Scaled {
    if pdf_ignore_depth
        .and_then(|handle| stores.resolve_primitive_handle(handle))
        .is_some()
    {
        stores.dimen_param(tex_state::env::banks::DimenParam::PDF_IGNORED_DIMEN)
    } else {
        IGNORE_DEPTH
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParagraphParams {
    pub left_skip: GlueSpec,
    pub right_skip: GlueSpec,
    pub par_fill_skip: GlueSpec,
    pub par_shape: Vec<tex_state::ParagraphShapeLine>,
    pub prev_graf: i32,
    pub hang_indent: Scaled,
    pub hang_after: i32,
    pub looseness: i32,
    pub pretolerance: i32,
    pub tolerance: i32,
    pub line_penalty: i32,
    pub hyphen_penalty: i32,
    pub ex_hyphen_penalty: i32,
    pub adj_demerits: i32,
    pub double_hyphen_demerits: i32,
    pub final_hyphen_demerits: i32,
    pub last_line_fit: i32,
    pub emergency_stretch: Scaled,
    pub hsize: Scaled,
    pub interline_penalty: i32,
    pub club_penalty: i32,
    pub widow_penalty: i32,
    pub display_widow_penalty: i32,
    pub broken_penalty: i32,
    pub interline_penalties: Vec<i32>,
    pub club_penalties: Vec<i32>,
    pub widow_penalties: Vec<i32>,
    pub display_widow_penalties: Vec<i32>,
}

/// One of TeX's six semantic modes.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Mode {
    Vertical,
    InternalVertical,
    Horizontal,
    RestrictedHorizontal,
    Math,
    DisplayMath,
}

impl Mode {
    /// The three-way mode family used by `\ifvmode`, `\ifhmode`, `\ifmmode`.
    #[must_use]
    pub const fn engine_mode(self) -> EngineMode {
        match self {
            Self::Vertical | Self::InternalVertical => EngineMode::Vertical,
            Self::Horizontal | Self::RestrictedHorizontal => EngineMode::Horizontal,
            Self::Math | Self::DisplayMath => EngineMode::Math,
        }
    }

    /// Whether TeX's `\ifinner` predicate is true in this mode.
    #[must_use]
    pub const fn is_inner(self) -> bool {
        matches!(
            self,
            Self::InternalVertical | Self::RestrictedHorizontal | Self::Math
        )
    }
}

impl ModeNest {
    /// Projects the live executor-owned mode nest for command conditionals.
    #[must_use]
    pub fn conditional_state(&self) -> tex_command::ConditionalState {
        let mode = match self.current_mode().engine_mode() {
            EngineMode::Vertical => tex_command::ConditionalMode::Vertical,
            EngineMode::Horizontal => tex_command::ConditionalMode::Horizontal,
            EngineMode::Math => tex_command::ConditionalMode::Math,
        };
        tex_command::ConditionalState::new(mode, self.current_mode().is_inner())
    }
}

/// The list-under-construction owned by one mode level.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModeList {
    sequence: tex_state::node_sequence::NodeSequence,
    align_state: Option<AlignState>,
    incomplete_fraction: Option<IncompleteFraction>,
    display_interrupt: Option<DisplayInterrupt>,
    display_eq_no: Option<DisplayEqNo>,
    display_alignment: bool,
    prev_depth: Option<Scaled>,
    prev_graf: i32,
    pending_hchars: Option<PendingHRun>,
    space_factor: i32,
    no_boundary: bool,
    hyphen_language: u8,
    left_hyphen_min: u8,
    right_hyphen_min: u8,
    component_roots: ModeComponentRoots,
    identity_enabled: bool,
    semantic_identity_root: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct ModeComponentRoots {
    align: u64,
    incomplete_fraction: u64,
    display_interrupt: u64,
    display_eq_no: u64,
    pending_hchars: u64,
}

impl ModeList {
    fn refresh_semantic_identity_root(&mut self) {
        if self.identity_enabled {
            self.semantic_identity_root = mode_list_semantic_identity(self);
        }
    }

    fn enable_semantic_identity(&mut self) {
        if self.identity_enabled {
            return;
        }
        self.identity_enabled = true;
        self.sequence.enable_semantic_identity();
        self.component_roots.align = self
            .align_state
            .as_mut()
            .map_or(0, |state| state.enable_semantic_identity());
        self.component_roots.incomplete_fraction = self
            .incomplete_fraction
            .as_ref()
            .map_or(0, incomplete_fraction_identity);
        self.component_roots.display_interrupt = self
            .display_interrupt
            .as_ref()
            .map_or(0, display_interrupt_identity);
        self.component_roots.display_eq_no = self
            .display_eq_no
            .as_ref()
            .map_or(0, display_eq_no_identity);
        self.component_roots.pending_hchars = self
            .pending_hchars
            .as_mut()
            .map_or(0, |run| run.enable_semantic_identity());
        self.refresh_semantic_identity_root();
    }

    fn is_checkpoint_rootless(&self) -> bool {
        self.nodes().is_empty()
            && self.pending_hchars.is_none()
            && self.align_state.is_none()
            && self.incomplete_fraction.is_none()
            && self.display_interrupt.is_none()
            && self.display_eq_no.is_none()
            && !self.display_alignment
    }
    #[must_use]
    pub fn nodes(&self) -> tex_state::node_sequence::NodeSequenceView<'_> {
        self.sequence.semantic_view()
    }

    #[must_use]
    pub fn physical_nodes(&self) -> tex_state::node_sequence::NodeSequenceView<'_> {
        self.sequence.physical_view()
    }

    pub fn take_nodes(&mut self) -> Vec<Node> {
        if self.sequence.has_candidate() {
            return self
                .sequence
                .take_candidate_semantic()
                .expect("checked candidate sequence");
        }
        std::mem::take(&mut self.sequence).into_semantic()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.sequence.semantic().is_empty()
    }

    pub fn push(&mut self, node: Node) {
        self.sequence.push_mirrored(node);
    }

    pub fn append(&mut self, nodes: impl IntoIterator<Item = Node>) {
        self.sequence.extend_mirrored(nodes);
    }

    /// Mutates one pre-existing node without allowing the mutable reference to
    /// escape this list's write barrier.
    pub(crate) fn with_node_mut<R>(
        &mut self,
        index: usize,
        mutate: impl FnOnce(&mut Node) -> R,
    ) -> Option<R> {
        self.sequence
            .mutate_semantic(|nodes| nodes.get_mut(index).map(mutate))
    }

    #[cfg(test)]
    pub(crate) fn with_reconstitution_target<R>(
        &mut self,
        mutate: impl for<'a> FnOnce(&'a mut Vec<Node>) -> R,
    ) -> R {
        self.sequence.mutate_semantic(mutate)
    }

    #[cfg(test)]
    pub(crate) fn push_reconstituted(
        &mut self,
        insertion: Option<(usize, Node)>,
        first: Node,
        second: Option<Node>,
        third: Option<Node>,
    ) {
        self.sequence.mutate_semantic(|target| {
            target.reserve(
                usize::from(insertion.is_some())
                    + 1
                    + usize::from(second.is_some())
                    + usize::from(third.is_some()),
            );
            if let Some((index, node)) = insertion {
                target.insert(index, node);
            }
            target.push(first);
            if let Some(node) = second {
                target.push(node);
            }
            if let Some(node) = third {
                target.push(node);
            }
        });
    }

    pub(crate) fn begin_pending_hchars(&mut self, font: FontId, ch: char, origin: OriginId) {
        debug_assert!(self.pending_hchars.is_none());
        let mut pending = PendingHRun::new(font, ch, origin, self.nodes().len());
        if self.identity_enabled {
            pending.enable_semantic_identity();
        }
        self.component_roots.pending_hchars = pending.semantic_identity_root;
        self.pending_hchars = Some(pending);
    }

    pub(crate) fn pending_hchars(&self) -> Option<&PendingHRun> {
        self.pending_hchars.as_ref()
    }

    pub(crate) fn set_pending_hchars(&mut self, mut pending: PendingHRun) {
        if self.identity_enabled {
            pending.enable_semantic_identity();
        }
        self.component_roots.pending_hchars = pending.semantic_identity_root;
        self.pending_hchars = Some(pending);
    }

    pub(crate) fn take_pending_hchars(&mut self) -> Option<PendingHRun> {
        let value = self.pending_hchars.take();
        self.component_roots.pending_hchars = 0;
        value
    }

    #[must_use]
    pub const fn space_factor(&self) -> i32 {
        if self.space_factor == 0 {
            1000
        } else {
            self.space_factor
        }
    }

    #[must_use]
    pub const fn raw_space_factor(&self) -> i32 {
        self.space_factor
    }

    pub fn set_space_factor(&mut self, value: i32) {
        self.space_factor = value;
    }

    #[must_use]
    pub const fn no_boundary(&self) -> bool {
        self.no_boundary
    }

    pub fn set_no_boundary(&mut self, value: bool) {
        self.no_boundary = value;
    }

    #[must_use]
    pub const fn hyphen_language(&self) -> u8 {
        self.hyphen_language
    }

    pub fn set_hyphen_language(&mut self, language: u8) {
        self.hyphen_language = language;
    }

    #[must_use]
    pub const fn left_hyphen_min(&self) -> u8 {
        self.left_hyphen_min
    }

    #[must_use]
    pub const fn right_hyphen_min(&self) -> u8 {
        self.right_hyphen_min
    }

    pub fn set_hyphen_context(&mut self, language: u8, left: u8, right: u8) {
        self.hyphen_language = language;
        self.left_hyphen_min = left;
        self.right_hyphen_min = right;
    }

    #[must_use]
    pub const fn prev_depth(&self) -> Option<Scaled> {
        self.prev_depth
    }

    pub fn set_prev_depth(&mut self, depth: Scaled) {
        self.prev_depth = Some(depth);
    }

    #[must_use]
    pub const fn prev_graf(&self) -> i32 {
        self.prev_graf
    }

    pub fn set_prev_graf(&mut self, lines: i32) {
        self.prev_graf = lines;
    }

    /// Removes TeX's `tail` only when it is an hbox or vbox.
    ///
    /// `\lastbox` must not search backwards past intervening material. The
    /// removed box also loses any raise/lower shift before it is used in its
    /// new box context, matching TeX82's `shift_amount(cur_box) := 0`.
    pub fn take_last_box(&mut self) -> Option<Node> {
        match self.nodes().last() {
            Some(Node::HList(_)) | Some(Node::VList(_)) => {}
            _ => return None,
        }
        let mut node = self
            .sequence
            .mutate_semantic(|nodes| nodes.pop())
            .expect("tail was just inspected");
        match &mut node {
            Node::HList(box_node) | Node::VList(box_node) => {
                box_node.shift = Scaled::from_raw(0);
            }
            _ => unreachable!("tail was checked to be a box"),
        }
        Some(node)
    }

    pub fn pop_last_node(&mut self) -> Option<Node> {
        if self.sequence.has_candidate() {
            return self.sequence.pop_candidate();
        }
        self.sequence.mutate_semantic(|nodes| nodes.pop())
    }

    pub(crate) fn remove_node_range(
        &mut self,
        range: std::ops::RangeInclusive<usize>,
    ) -> Vec<Node> {
        if self.sequence.has_candidate() {
            return self
                .sequence
                .mutate_candidate_semantic(|nodes| nodes.drain(range).collect())
                .expect("checked candidate sequence");
        }
        self.sequence
            .mutate_semantic(|nodes| nodes.drain(range).collect())
    }

    /// Mutates the tail node without allowing its mutable reference to escape.
    pub(crate) fn with_last_node_mut<R>(
        &mut self,
        mutate: impl FnOnce(&mut Node) -> R,
    ) -> Option<R> {
        self.sequence
            .mutate_semantic(|nodes| nodes.last_mut().map(mutate))
    }

    #[must_use]
    pub fn align_state(&self) -> Option<&AlignState> {
        self.align_state.as_ref()
    }

    pub fn set_align_state(&mut self, state: AlignState) {
        let mut state = state;
        if self.identity_enabled {
            state.enable_semantic_identity();
        }
        self.component_roots.align = state.semantic_identity_root;
        self.align_state = Some(state);
    }

    pub fn with_align_state_mut<R>(
        &mut self,
        mutate: impl for<'a> FnOnce(&'a mut AlignState) -> R,
    ) -> Option<R> {
        let state = self.align_state.as_mut()?;
        let result = mutate(state);
        self.component_roots.align = state.semantic_identity_root;
        Some(result)
    }

    pub fn take_align_state(&mut self) -> Option<AlignState> {
        let value = self.align_state.take();
        self.component_roots.align = 0;
        value
    }

    #[must_use]
    pub fn incomplete_fraction(&self) -> Option<&IncompleteFraction> {
        self.incomplete_fraction.as_ref()
    }

    pub fn set_incomplete_fraction(&mut self, fraction: IncompleteFraction) {
        if self.identity_enabled {
            self.component_roots.incomplete_fraction = incomplete_fraction_identity(&fraction);
        }
        self.incomplete_fraction = Some(fraction);
    }

    pub fn take_incomplete_fraction(&mut self) -> Option<IncompleteFraction> {
        let value = self.incomplete_fraction.take();
        self.component_roots.incomplete_fraction = 0;
        value
    }

    pub fn set_display_interrupt(&mut self, interrupt: DisplayInterrupt) {
        if self.identity_enabled {
            self.component_roots.display_interrupt = display_interrupt_identity(&interrupt);
        }
        self.display_interrupt = Some(interrupt);
    }

    #[must_use]
    pub const fn display_interrupt(&self) -> Option<&DisplayInterrupt> {
        self.display_interrupt.as_ref()
    }

    pub fn take_display_interrupt(&mut self) -> Option<DisplayInterrupt> {
        let value = self.display_interrupt.take();
        self.component_roots.display_interrupt = 0;
        value
    }

    pub fn set_display_eq_no(&mut self, eq_no: DisplayEqNo) {
        if self.identity_enabled {
            self.component_roots.display_eq_no = display_eq_no_identity(&eq_no);
        }
        self.display_eq_no = Some(eq_no);
    }

    #[must_use]
    pub const fn display_eq_no(&self) -> Option<&DisplayEqNo> {
        self.display_eq_no.as_ref()
    }

    pub fn take_display_eq_no(&mut self) -> Option<DisplayEqNo> {
        let value = self.display_eq_no.take();
        self.component_roots.display_eq_no = 0;
        value
    }

    pub fn set_display_alignment(&mut self, nodes: Vec<Node>, prev_depth: Option<Scaled>) {
        // A display alignment owns the whole display-mode list: §1206 permits
        // assignments before the closing `$$`, but no additional material.
        debug_assert!(!self.display_alignment);
        debug_assert!(self.nodes().is_empty());
        self.append(nodes);
        self.prev_depth = prev_depth;
        self.display_alignment = true;
    }

    pub(crate) const fn has_display_alignment(&self) -> bool {
        self.display_alignment
    }

    pub fn take_display_alignment(&mut self) -> Option<(Vec<Node>, Option<Scaled>)> {
        if !std::mem::take(&mut self.display_alignment) {
            return None;
        }
        Some((self.take_nodes(), self.prev_depth))
    }
}

/// A typed, short-lived write capability for one mode list.
///
/// The capability deliberately does not implement `DerefMut` or expose its
/// backing list. Operations either consume/replace owned values or execute a
/// higher-ranked closure whose mutable borrow cannot escape.
enum ModeListBorrow<'a> {
    Direct(&'a mut ModeList),
    Shared(std::cell::RefMut<'a, ModeList>),
}

impl std::ops::Deref for ModeListBorrow<'_> {
    type Target = ModeList;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Direct(list) => list,
            Self::Shared(list) => list,
        }
    }
}

impl std::ops::DerefMut for ModeListBorrow<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match self {
            Self::Direct(list) => list,
            Self::Shared(list) => list,
        }
    }
}

enum ModeListJournalBorrow<'a> {
    None,
    Shared {
        journal: std::cell::RefMut<'a, journal::ModeJournal>,
        index: usize,
    },
}

pub(crate) struct ModeListMutation<'a> {
    list: ModeListBorrow<'a>,
    journal: ModeListJournalBorrow<'a>,
}

impl Drop for ModeListMutation<'_> {
    fn drop(&mut self) {
        self.list.refresh_semantic_identity_root();
    }
}

impl ModeListMutation<'_> {
    fn journal_is_active(&self) -> bool {
        match &self.journal {
            ModeListJournalBorrow::None => false,
            ModeListJournalBorrow::Shared { journal, .. } => journal.has_active_frame(),
        }
    }
    fn list_journal(&mut self) -> Option<journal::ListJournal<'_>> {
        match &mut self.journal {
            ModeListJournalBorrow::None => None,
            ModeListJournalBorrow::Shared { journal, index } => journal.list(*index),
        }
    }

    pub(crate) fn push(&mut self, node: Node) {
        self.list.push(node);
    }

    pub(crate) fn nodes(&self) -> tex_state::node_sequence::NodeSequenceView<'_> {
        self.list.nodes()
    }

    pub(crate) fn append(&mut self, nodes: impl IntoIterator<Item = Node>) {
        self.list.append(nodes);
    }

    pub(crate) fn take_nodes(&mut self) -> Vec<Node> {
        self.record_nodes();
        self.list.take_nodes()
    }

    pub(crate) fn pop_last_node(&mut self) -> Option<Node> {
        self.record_nodes();
        self.list.pop_last_node()
    }

    pub(crate) fn remove_node_range(
        &mut self,
        range: std::ops::RangeInclusive<usize>,
    ) -> Vec<Node> {
        self.record_nodes();
        self.list.remove_node_range(range)
    }

    pub(crate) fn with_node_mut<R>(
        &mut self,
        index: usize,
        mutate: impl for<'a> FnOnce(&'a mut Node) -> R,
    ) -> Option<R> {
        if self.list.sequence.has_candidate() {
            return self.list.sequence.with_candidate_node_mut(index, mutate);
        }
        self.record_node(index);
        self.list.with_node_mut(index, mutate)
    }

    pub(crate) fn with_last_node_mut<R>(
        &mut self,
        mutate: impl for<'a> FnOnce(&'a mut Node) -> R,
    ) -> Option<R> {
        if let Some(index) = self.list.nodes().len().checked_sub(1) {
            if self.list.sequence.has_candidate() {
                return self.list.sequence.with_candidate_node_mut(index, mutate);
            }
            self.record_node(index);
        }
        self.list.with_last_node_mut(mutate)
    }

    #[cfg(test)]
    pub(crate) fn with_reconstitution_target<R>(
        &mut self,
        mutate: impl for<'a> FnOnce(&'a mut Vec<Node>) -> R,
    ) -> R {
        self.record_nodes();
        self.list.with_reconstitution_target(mutate)
    }

    #[cfg(test)]
    pub(crate) fn push_reconstituted(
        &mut self,
        insertion: Option<(usize, Node)>,
        first: Node,
        second: Option<Node>,
        third: Option<Node>,
    ) {
        if insertion.is_some() {
            self.record_nodes();
        }
        self.list
            .push_reconstituted(insertion, first, second, third);
    }

    pub(crate) fn begin_pending_hchars(&mut self, font: FontId, ch: char, origin: OriginId) {
        if self.journal_is_active() {
            let old = self
                .list
                .pending_hchars
                .as_ref()
                .map(journal::PendingHRunProjection::capture);
            if let Some(mut journal) = self.list_journal() {
                journal.record_pending_projection(old);
            }
        }
        self.list.begin_pending_hchars(font, ch, origin);
    }

    pub(crate) fn set_pending_hchars(&mut self, pending: PendingHRun) {
        if self.journal_is_active() {
            let old = self.list.take_pending_hchars();
            if let Some(mut journal) = self.list_journal() {
                journal.record_pending_owned(old);
            }
        }
        self.list.set_pending_hchars(pending);
    }

    pub(crate) fn take_pending_hchars(&mut self) -> Option<PendingHRun> {
        let old = self
            .journal_is_active()
            .then(|| self.list.pending_hchars.clone());
        if let (Some(old), Some(mut journal)) = (old, self.list_journal()) {
            journal.record_pending_value(old.as_ref());
        }
        self.list.take_pending_hchars()
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn with_pending_hchars_mut<R>(
        &mut self,
        mutate: impl FnOnce(&mut PendingHRun) -> R,
    ) -> Option<R> {
        if self.journal_is_active() {
            let old = self
                .list
                .pending_hchars
                .as_ref()
                .map(journal::PendingHRunProjection::capture);
            if let Some(mut journal) = self.list_journal() {
                journal.record_pending_projection(old);
            }
        }
        let pending = self.list.pending_hchars.as_mut()?;
        let result = mutate(pending);
        pending.source_identity_root = pending_source_identity(&pending.source);
        pending.refresh_semantic_identity_root();
        self.list.component_roots.pending_hchars = pending.semantic_identity_root;
        Some(result)
    }

    pub(crate) fn append_pending_hchar(
        &mut self,
        font: FontId,
        ch: char,
        origin: OriginId,
        script: Option<tex_fonts::Script>,
    ) -> bool {
        if self.journal_is_active() {
            let old = self
                .list
                .pending_hchars
                .as_ref()
                .map(journal::PendingHRunProjection::capture);
            if let Some(mut journal) = self.list_journal() {
                journal.record_pending_projection(old);
            }
        }
        let Some(pending) = self.list.pending_hchars.as_mut() else {
            return false;
        };
        pending.append_character(font, ch, origin, script);
        self.list.component_roots.pending_hchars = pending.semantic_identity_root;
        true
    }

    pub(crate) fn pending_hchars(&self) -> Option<&PendingHRun> {
        self.list.pending_hchars()
    }

    pub(crate) fn set_space_factor(&mut self, value: i32) {
        let old = self.list.space_factor;
        if let Some(mut journal) = self.list_journal() {
            journal.record_space_factor(old);
        }
        self.list.set_space_factor(value);
    }

    pub(crate) fn space_factor(&self) -> i32 {
        self.list.space_factor()
    }

    pub(crate) fn set_no_boundary(&mut self, value: bool) {
        let old = self.list.no_boundary;
        if let Some(mut journal) = self.list_journal() {
            journal.record_no_boundary(old);
        }
        self.list.set_no_boundary(value);
    }

    pub(crate) fn set_hyphen_context(&mut self, language: u8, left: u8, right: u8) {
        let old = (
            self.list.hyphen_language,
            self.list.left_hyphen_min,
            self.list.right_hyphen_min,
        );
        if let Some(mut journal) = self.list_journal() {
            journal.record_hyphen_context(old);
        }
        self.list.set_hyphen_context(language, left, right);
    }

    #[cfg(test)]
    pub(crate) fn set_hyphen_language(&mut self, language: u8) {
        let old = (
            self.list.hyphen_language,
            self.list.left_hyphen_min,
            self.list.right_hyphen_min,
        );
        if let Some(mut journal) = self.list_journal() {
            journal.record_hyphen_context(old);
        }
        self.list.set_hyphen_language(language);
    }

    pub(crate) fn set_prev_depth(&mut self, depth: Scaled) {
        let old = self.list.prev_depth;
        if let Some(mut journal) = self.list_journal() {
            journal.record_prev_depth(old);
        }
        self.list.set_prev_depth(depth);
    }

    pub(crate) fn set_prev_graf(&mut self, lines: i32) {
        let old = self.list.prev_graf;
        if let Some(mut journal) = self.list_journal() {
            journal.record_prev_graf(old);
        }
        self.list.set_prev_graf(lines);
    }

    #[cfg(test)]
    pub(crate) fn set_align_state(&mut self, state: AlignState) {
        let old = self.list.take_align_state();
        if let Some(mut journal) = self.list_journal() {
            journal.record_align_state(old);
        }
        self.list.set_align_state(state);
    }

    #[cfg(test)]
    pub(crate) fn with_align_state_mut<R>(
        &mut self,
        mutate: impl for<'a> FnOnce(&'a mut AlignState) -> R,
    ) -> Option<R> {
        let old = self.list.align_state.clone();
        if let Some(mut journal) = self.list_journal() {
            journal.record_align_state(old);
        }
        self.list.with_align_state_mut(mutate)
    }

    #[cfg(test)]
    pub(crate) fn take_align_state(&mut self) -> Option<AlignState> {
        let old = self.list.align_state.clone();
        if let Some(mut journal) = self.list_journal() {
            journal.record_align_state(old);
        }
        self.list.take_align_state()
    }

    pub(crate) fn set_incomplete_fraction(&mut self, fraction: IncompleteFraction) {
        let old = self.list.take_incomplete_fraction();
        if let Some(mut journal) = self.list_journal() {
            journal.record_incomplete_fraction(old);
        }
        self.list.set_incomplete_fraction(fraction);
    }

    pub(crate) fn take_incomplete_fraction(&mut self) -> Option<IncompleteFraction> {
        let old = self.list.incomplete_fraction.clone();
        if let Some(mut journal) = self.list_journal() {
            journal.record_incomplete_fraction(old);
        }
        self.list.take_incomplete_fraction()
    }

    pub(crate) fn incomplete_fraction(&self) -> Option<&IncompleteFraction> {
        self.list.incomplete_fraction()
    }

    pub(crate) fn set_display_interrupt(&mut self, interrupt: DisplayInterrupt) {
        let old = self.list.take_display_interrupt();
        if let Some(mut journal) = self.list_journal() {
            journal.record_display_interrupt(old);
        }
        self.list.set_display_interrupt(interrupt);
    }

    pub(crate) fn take_display_interrupt(&mut self) -> Option<DisplayInterrupt> {
        let old = self.list.display_interrupt.clone();
        if let Some(mut journal) = self.list_journal() {
            journal.record_display_interrupt(old);
        }
        self.list.take_display_interrupt()
    }

    pub(crate) fn set_display_eq_no(&mut self, eq_no: DisplayEqNo) {
        let old = self.list.take_display_eq_no();
        if let Some(mut journal) = self.list_journal() {
            journal.record_display_eq_no(old);
        }
        self.list.set_display_eq_no(eq_no);
    }

    pub(crate) fn take_display_eq_no(&mut self) -> Option<DisplayEqNo> {
        let old = self.list.display_eq_no.clone();
        if let Some(mut journal) = self.list_journal() {
            journal.record_display_eq_no(old);
        }
        self.list.take_display_eq_no()
    }

    pub(crate) fn set_display_alignment(&mut self, nodes: Vec<Node>, prev_depth: Option<Scaled>) {
        self.record_nodes();
        let old_prev_depth = self.list.prev_depth;
        let old_display_alignment = self.list.display_alignment;
        if let Some(mut journal) = self.list_journal() {
            journal.record_prev_depth(old_prev_depth);
            journal.record_display_alignment(old_display_alignment);
        }
        self.list.set_display_alignment(nodes, prev_depth);
    }

    pub(crate) fn take_display_alignment(&mut self) -> Option<(Vec<Node>, Option<Scaled>)> {
        if self.list.display_alignment {
            self.record_nodes();
        }
        let old_prev_depth = self.list.prev_depth;
        let old_display_alignment = self.list.display_alignment;
        if let Some(mut journal) = self.list_journal() {
            journal.record_prev_depth(old_prev_depth);
            journal.record_display_alignment(old_display_alignment);
        }
        self.list.take_display_alignment()
    }

    fn record_node(&mut self, index: usize) {
        if !self.list.sequence.has_candidate()
            && self.journal_is_active()
            && self.list.nodes().get(index).is_some()
        {
            let needs_nodes = self
                .list_journal()
                .is_some_and(|journal| journal.needs_nodes());
            if !needs_nodes {
                return;
            }
            let old = self.list.sequence.clone();
            if let Some(mut journal) = self.list_journal() {
                journal.record_nodes(&old);
            }
        }
    }

    fn record_nodes(&mut self) {
        if !self.journal_is_active() || self.list.sequence.has_candidate() {
            return;
        }
        let needs_nodes = self
            .list_journal()
            .is_some_and(|journal| journal.needs_nodes());
        if !needs_nodes {
            return;
        }
        let old = self.list.sequence.clone();
        if let Some(mut journal) = self.list_journal() {
            journal.record_nodes(&old);
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AlignmentKind {
    HAlign,
    VAlign,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AlignmentPackSpec {
    Natural,
    Exactly(Scaled),
    Spread(Scaled),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AlignColumn {
    pub u_template: NodeTokenList,
    pub v_template: NodeTokenList,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AlignState {
    kind: AlignmentKind,
    pack_spec: AlignmentPackSpec,
    columns: Vec<AlignColumn>,
    tabskips: Vec<GlueSpec>,
    default_tabskip: GlueSpec,
    loop_start: Option<usize>,
    current_row: usize,
    current_col: usize,
    current_span: u16,
    suppress_redundant_cr: bool,
    identity_enabled: bool,
    definition_identity_root: u64,
    semantic_identity_root: u64,
}

impl AlignState {
    #[must_use]
    pub fn new(
        kind: AlignmentKind,
        pack_spec: AlignmentPackSpec,
        columns: Vec<AlignColumn>,
        tabskips: Vec<GlueSpec>,
        default_tabskip: GlueSpec,
        loop_start: Option<usize>,
    ) -> Self {
        Self {
            kind,
            pack_spec,
            columns,
            tabskips,
            default_tabskip,
            loop_start,
            current_row: 0,
            current_col: 0,
            current_span: 1,
            suppress_redundant_cr: false,
            identity_enabled: false,
            definition_identity_root: 0,
            semantic_identity_root: 0,
        }
    }

    #[must_use]
    pub const fn kind(&self) -> AlignmentKind {
        self.kind
    }

    #[must_use]
    pub const fn pack_spec(&self) -> AlignmentPackSpec {
        self.pack_spec
    }

    #[must_use]
    pub fn columns(&self) -> &[AlignColumn] {
        &self.columns
    }

    #[must_use]
    pub fn tabskips(&self) -> &[GlueSpec] {
        &self.tabskips
    }

    #[must_use]
    pub fn default_tabskip(&self) -> &GlueSpec {
        &self.default_tabskip
    }

    #[must_use]
    pub const fn loop_start(&self) -> Option<usize> {
        self.loop_start
    }

    #[must_use]
    pub const fn current_row(&self) -> usize {
        self.current_row
    }

    #[must_use]
    pub const fn current_col(&self) -> usize {
        self.current_col
    }

    #[must_use]
    pub const fn current_span(&self) -> u16 {
        self.current_span
    }

    #[must_use]
    pub const fn suppress_redundant_cr(&self) -> bool {
        self.suppress_redundant_cr
    }

    pub fn set_suppress_redundant_cr(&mut self, value: bool) {
        self.suppress_redundant_cr = value;
        self.refresh_semantic_identity_root();
    }

    #[must_use]
    pub fn column_for(&self, column: usize) -> Option<&AlignColumn> {
        if column < self.columns.len() {
            return self.columns.get(column);
        }
        let loop_start = self.loop_start?;
        let repeat_len = self.columns.len().checked_sub(loop_start)?;
        if repeat_len == 0 {
            return None;
        }
        let resolved = loop_start + (column - loop_start) % repeat_len;
        self.columns.get(resolved)
    }

    #[must_use]
    pub fn tabskip_for_boundary(&self, boundary: usize) -> &GlueSpec {
        if let Some(tabskip) = self.tabskips.get(boundary) {
            return tabskip;
        }
        let Some(column) = boundary.checked_sub(1) else {
            return &self.default_tabskip;
        };
        let Some(loop_start) = self.loop_start else {
            return &self.default_tabskip;
        };
        let Some(repeat_len) = self.columns.len().checked_sub(loop_start) else {
            return &self.default_tabskip;
        };
        if repeat_len == 0 || column < loop_start {
            return &self.default_tabskip;
        }
        let repeated_column = loop_start + (column - loop_start) % repeat_len;
        self.tabskips
            .get(repeated_column + 1)
            .unwrap_or(&self.default_tabskip)
    }

    pub fn start_row(&mut self) {
        self.current_col = 0;
        self.current_span = 1;
        self.refresh_semantic_identity_root();
    }

    pub fn start_cell(&mut self, column: usize, span_count: u16) {
        self.current_col = column;
        self.current_span = span_count;
        self.refresh_semantic_identity_root();
    }

    pub fn finish_cell(&mut self, next_column: usize) {
        self.current_col = next_column;
        self.current_span = 1;
        self.refresh_semantic_identity_root();
    }

    pub fn finish_row(&mut self) {
        self.current_row += 1;
        self.current_col = 0;
        self.current_span = 1;
        self.refresh_semantic_identity_root();
    }

    fn refresh_semantic_identity_root(&mut self) {
        if !self.identity_enabled {
            return;
        }
        let mut hasher = mode_identity_hasher(b"umber-mode-alignment-state-v1");
        self.definition_identity_root.hash(&mut hasher);
        (self.current_row as u64).hash(&mut hasher);
        (self.current_col as u64).hash(&mut hasher);
        self.current_span.hash(&mut hasher);
        self.suppress_redundant_cr.hash(&mut hasher);
        self.semantic_identity_root = hasher.finish();
    }

    fn enable_semantic_identity(&mut self) -> u64 {
        if !self.identity_enabled {
            self.identity_enabled = true;
            self.definition_identity_root = alignment_definition_identity(self);
            self.refresh_semantic_identity_root();
        }
        self.semantic_identity_root
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingHChar {
    pub font: FontId,
    pub ch: char,
    pub origin: OriginId,
}

/// Streaming state for the unresolved tail of one horizontal character run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PendingHRun {
    pub(crate) first: PendingHChar,
    pub(crate) current: PendingHRunChar,
    /// Absolute position in this mode list where a left-boundary node belongs.
    pub(crate) insertion_index: usize,
    pub(crate) source: Vec<PendingHChar>,
    pub(crate) script: tex_fonts::Script,
    identity_enabled: bool,
    source_identity_root: u64,
    semantic_identity_root: u64,
}

impl PendingHRun {
    pub(crate) fn new(font: FontId, ch: char, origin: OriginId, insertion_index: usize) -> Self {
        let first = PendingHChar { font, ch, origin };
        Self {
            first: first.clone(),
            current: PendingHRunChar::new(font, ch, origin),
            insertion_index,
            source: vec![first],
            script: tex_fonts::character_script(ch),
            identity_enabled: false,
            source_identity_root: 0,
            semantic_identity_root: 0,
        }
    }

    fn append_character(
        &mut self,
        font: FontId,
        ch: char,
        origin: OriginId,
        script: Option<tex_fonts::Script>,
    ) {
        if let Some(script) = script {
            self.script = script;
        }
        let source = PendingHChar { font, ch, origin };
        if self.identity_enabled {
            self.source_identity_root = self
                .source_identity_root
                .rotate_left(27)
                .wrapping_mul(0x9e37_79b1_85eb_ca87)
                .wrapping_add(pending_char_identity(&source));
        }
        self.source.push(source);
        self.current = PendingHRunChar::new(font, ch, origin);
        self.refresh_semantic_identity_root();
    }

    fn refresh_semantic_identity_root(&mut self) {
        if !self.identity_enabled {
            return;
        }
        let mut hasher = mode_identity_hasher(b"umber-mode-pending-run-v1");
        pending_char_identity(&self.first).hash(&mut hasher);
        (self.insertion_index as u64).hash(&mut hasher);
        (self.source.len() as u64).hash(&mut hasher);
        self.source_identity_root.hash(&mut hasher);
        pending_current_identity(&self.current).hash(&mut hasher);
        (self.script as u32).hash(&mut hasher);
        self.semantic_identity_root = hasher.finish();
    }

    fn enable_semantic_identity(&mut self) -> u64 {
        if !self.identity_enabled {
            self.identity_enabled = true;
            self.source_identity_root = pending_source_identity(&self.source);
            self.refresh_semantic_identity_root();
        }
        self.semantic_identity_root
    }
}

/// Current glyph and original-character range carried through ligature folding.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PendingHRunChar {
    pub(crate) font: FontId,
    pub(crate) ch: char,
    pub(crate) orig: SmallVec<[char; 4]>,
    pub(crate) origins: SmallVec<[OriginId; 4]>,
    pub(crate) ligature_present: bool,
    pub(crate) left_hit: bool,
    pub(crate) right_hit: bool,
}

impl PendingHRunChar {
    pub(crate) fn new(font: FontId, ch: char, origin: OriginId) -> Self {
        Self {
            font,
            ch,
            orig: smallvec![ch],
            origins: smallvec![origin],
            ligature_present: false,
            left_hit: false,
            right_hit: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct IncompleteFraction {
    pub numerator: PageListId,
    pub thickness: FractionThickness,
    pub left_delimiter: Option<u32>,
    pub right_delimiter: Option<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DisplayInterrupt {
    pub active_directions: Vec<tex_state::node::Direction>,
    pub prototype: Option<BoxNode>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DisplayEqNo {
    pub side: EqNoSide,
    pub display: PageListId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EqNoSide {
    Left,
    Right,
}

/// Snapshot-summary state for one mode level.
#[derive(Clone, Debug, PartialEq)]
pub struct ModeLevelSummary {
    mode: Mode,
    entry_line: i32,
    list: ModeList,
}

impl ModeLevelSummary {
    #[must_use]
    pub fn new(mode: Mode) -> Self {
        Self {
            mode,
            entry_line: 0,
            list: ModeList::default(),
        }
    }

    #[must_use]
    pub const fn entry_line(&self) -> i32 {
        self.entry_line
    }

    pub(crate) const fn set_entry_line(&mut self, line: i32) {
        self.entry_line = line;
    }

    #[must_use]
    pub const fn mode(&self) -> Mode {
        self.mode
    }

    #[must_use]
    pub fn list(&self) -> &ModeList {
        &self.list
    }

    pub(crate) fn mutate_list<R>(
        &mut self,
        mutate: impl for<'a> FnOnce(&'a mut ModeList) -> R,
    ) -> R {
        let result = mutate(&mut self.list);
        self.list.refresh_semantic_identity_root();
        result
    }

    pub(crate) fn list_mutation(&mut self) -> ModeListMutation<'_> {
        ModeListMutation {
            list: ModeListBorrow::Direct(&mut self.list),
            journal: ModeListJournalBorrow::None,
        }
    }
}

/// Snapshot-coverable summary of the whole mode nest.
#[derive(Clone, Debug, PartialEq)]
pub struct ModeNestSummary {
    levels: Vec<ModeLevelSummary>,
}

impl ModeNestSummary {
    #[must_use]
    pub fn levels(&self) -> &[ModeLevelSummary] {
        &self.levels
    }

    /// Whether this summary has any explicit checkpointable page coordinate.
    pub(crate) fn retains_page_node_handles(&self) -> bool {
        self.levels.iter().any(|level| {
            let list = &level.list;
            list.sequence.retains_page_node_handles()
                || list
                    .incomplete_fraction
                    .as_ref()
                    .is_some_and(|fraction| !fraction.numerator.is_empty())
                || list
                    .display_interrupt
                    .as_ref()
                    .and_then(|interrupt| interrupt.prototype.as_ref())
                    .is_some_and(|prototype| !prototype.children.is_empty())
                || list
                    .display_eq_no
                    .as_ref()
                    .is_some_and(|eqno| !eqno.display.is_empty())
        })
    }

    pub(crate) fn semantic_fingerprint<G>(&self, universe: &Universe<G>) -> u64 {
        semantic_fingerprint_levels(&self.levels, universe)
    }
}

fn semantic_fingerprint_levels<G>(levels: &[ModeLevelSummary], universe: &Universe<G>) -> u64 {
    #[cfg(test)]
    SEMANTIC_FINGERPRINT_CALLS.with(|calls| calls.set(calls.get() + 1));
    universe.engine_boundary_hash(0x6d6f_6465_5f6e_6573, |projection| {
        projection.usize(levels.len());
        for level in levels {
            hash_mode(level.mode, projection);
            projection.i32(level.entry_line);
            hash_mode_list(&level.list, universe, projection);
        }
    })
}

#[cfg(test)]
pub(crate) fn reset_semantic_fingerprint_calls_for_test() {
    SEMANTIC_FINGERPRINT_CALLS.with(|calls| calls.set(0));
}

#[cfg(test)]
pub(crate) fn semantic_fingerprint_calls_for_test() -> u64 {
    SEMANTIC_FINGERPRINT_CALLS.with(std::cell::Cell::get)
}

fn hash_mode<G>(mode: Mode, projection: &mut EngineBoundaryHasher<'_, G>) {
    projection.u8(match mode {
        Mode::Vertical => 0,
        Mode::InternalVertical => 1,
        Mode::Horizontal => 2,
        Mode::RestrictedHorizontal => 3,
        Mode::Math => 4,
        Mode::DisplayMath => 5,
    });
}

fn hash_mode_list<G>(
    list: &ModeList,
    universe: &Universe<G>,
    projection: &mut EngineBoundaryHasher<'_, G>,
) {
    projection.nodes_iter(list.nodes().iter());
    match &list.align_state {
        Some(align) => {
            projection.bool(true);
            projection.u8(match align.kind {
                AlignmentKind::HAlign => 0,
                AlignmentKind::VAlign => 1,
            });
            match align.pack_spec {
                AlignmentPackSpec::Natural => projection.u8(0),
                AlignmentPackSpec::Exactly(size) => {
                    projection.u8(1);
                    projection.i32(size.raw());
                }
                AlignmentPackSpec::Spread(size) => {
                    projection.u8(2);
                    projection.i32(size.raw());
                }
            }
            projection.usize(align.columns.len());
            for column in &align.columns {
                hash_node_tokens(&column.u_template, projection);
                hash_node_tokens(&column.v_template, projection);
            }
            projection.usize(align.tabskips.len());
            for tabskip in &align.tabskips {
                hash_node_glue(*tabskip, projection);
            }
            hash_node_glue(align.default_tabskip, projection);
            hash_optional_usize(align.loop_start, projection);
            projection.usize(align.current_row);
            projection.usize(align.current_col);
            projection.u16(align.current_span);
            projection.bool(align.suppress_redundant_cr);
        }
        None => projection.bool(false),
    }
    match &list.incomplete_fraction {
        Some(fraction) => {
            projection.bool(true);
            projection.page_node_list(universe, fraction.numerator);
            match fraction.thickness {
                FractionThickness::Default => projection.u8(0),
                FractionThickness::Explicit(size) => {
                    projection.u8(1);
                    projection.i32(size.raw());
                }
            }
            hash_optional_u32(fraction.left_delimiter, projection);
            hash_optional_u32(fraction.right_delimiter, projection);
        }
        None => projection.bool(false),
    }
    match &list.display_interrupt {
        Some(interrupt) => {
            projection.bool(true);
            projection.usize(interrupt.active_directions.len());
            for direction in &interrupt.active_directions {
                projection.u8(match direction {
                    tex_state::node::Direction::BeginL => 0,
                    tex_state::node::Direction::EndL => 1,
                    tex_state::node::Direction::BeginR => 2,
                    tex_state::node::Direction::EndR => 3,
                    tex_state::node::Direction::BeginM => 4,
                    tex_state::node::Direction::EndM => 5,
                });
            }
            match &interrupt.prototype {
                Some(prototype) => {
                    projection.bool(true);
                    projection.nodes(&[Node::HList(*prototype)]);
                }
                None => projection.bool(false),
            }
        }
        None => projection.bool(false),
    }
    match &list.display_eq_no {
        Some(eq_no) => {
            projection.bool(true);
            projection.u8(match eq_no.side {
                EqNoSide::Left => 0,
                EqNoSide::Right => 1,
            });
            projection.page_node_list(universe, eq_no.display);
        }
        None => projection.bool(false),
    }
    projection.bool(list.display_alignment);
    match list.prev_depth {
        Some(depth) => {
            projection.bool(true);
            projection.i32(depth.raw());
        }
        None => projection.bool(false),
    }
    projection.i32(list.prev_graf);
    match &list.pending_hchars {
        Some(pending) => {
            projection.bool(true);
            projection.font(pending.first.font);
            projection.u32(pending.first.ch as u32);
            projection.usize(pending.insertion_index);
            projection.usize(pending.source.len());
            for source in &pending.source {
                projection.font(source.font);
                projection.u32(source.ch as u32);
            }
            projection.font(pending.current.font);
            projection.u32(pending.current.ch as u32);
            projection.usize(pending.current.orig.len());
            for ch in &pending.current.orig {
                projection.u32(*ch as u32);
            }
            projection.bool(pending.current.ligature_present);
        }
        None => projection.bool(false),
    }
    projection.i32(list.space_factor);
    projection.bool(list.no_boundary);
    projection.u8(list.hyphen_language);
    projection.u8(list.left_hyphen_min);
    projection.u8(list.right_hyphen_min);
}

fn hash_node_tokens<G>(
    tokens: &tex_state::node::NodeTokenList,
    projection: &mut EngineBoundaryHasher<'_, G>,
) {
    projection.usize(tokens.words().len());
    for token in tokens.words() {
        projection.u32(token.raw());
    }
}

fn hash_node_glue<G>(
    glue: tex_state::glue::GlueSpec,
    projection: &mut EngineBoundaryHasher<'_, G>,
) {
    projection.i32(glue.width.raw());
    projection.i32(glue.stretch.raw());
    projection.u8(glue.stretch_order as u8);
    projection.i32(glue.shrink.raw());
    projection.u8(glue.shrink_order as u8);
}

fn hash_optional_usize<G>(value: Option<usize>, projection: &mut EngineBoundaryHasher<'_, G>) {
    match value {
        Some(value) => {
            projection.bool(true);
            projection.usize(value);
        }
        None => projection.bool(false),
    }
}

fn hash_optional_u32<G>(value: Option<u32>, projection: &mut EngineBoundaryHasher<'_, G>) {
    match value {
        Some(value) => {
            projection.bool(true);
            projection.u32(value);
        }
        None => projection.bool(false),
    }
}

const MODE_LIST_IDENTITY_DOMAIN: &[u8] = b"umber-mode-list-semantic-root-v1";
const MODE_NEST_IDENTITY_DOMAIN: &[u8] = b"umber-mode-nest-semantic-root-v1";

fn mode_identity_hasher(domain: &[u8]) -> ahash::AHasher {
    let state = RandomState::with_seeds(
        0x756d_6265_725f_6d6f,
        0x6465_5f69_6465_6e74,
        0x6974_795f_7631_5f66,
        0x6978_6564_5f73_6565,
    );
    let mut hasher = state.build_hasher();
    hasher.write(domain);
    hasher
}

fn hash_optional_scaled(value: Option<Scaled>, hasher: &mut impl Hasher) {
    value.map(Scaled::raw).hash(hasher);
}

fn hash_node_token_list(tokens: &NodeTokenList, hasher: &mut impl Hasher) {
    tokens.hash(hasher);
}

fn hash_glue_value(glue: GlueSpec, hasher: &mut impl Hasher) {
    glue.width.raw().hash(hasher);
    glue.stretch.raw().hash(hasher);
    (glue.stretch_order as u8).hash(hasher);
    glue.shrink.raw().hash(hasher);
    (glue.shrink_order as u8).hash(hasher);
}

fn alignment_definition_identity(align: &AlignState) -> u64 {
    let mut hasher = mode_identity_hasher(b"umber-mode-alignment-definition-v1");
    match align.kind {
        AlignmentKind::HAlign => 0_u8.hash(&mut hasher),
        AlignmentKind::VAlign => 1_u8.hash(&mut hasher),
    }
    match align.pack_spec {
        AlignmentPackSpec::Natural => 0_u8.hash(&mut hasher),
        AlignmentPackSpec::Exactly(size) => {
            1_u8.hash(&mut hasher);
            size.raw().hash(&mut hasher);
        }
        AlignmentPackSpec::Spread(size) => {
            2_u8.hash(&mut hasher);
            size.raw().hash(&mut hasher);
        }
    }
    (align.columns.len() as u64).hash(&mut hasher);
    for column in &align.columns {
        hash_node_token_list(&column.u_template, &mut hasher);
        hash_node_token_list(&column.v_template, &mut hasher);
    }
    (align.tabskips.len() as u64).hash(&mut hasher);
    for &tabskip in &align.tabskips {
        hash_glue_value(tabskip, &mut hasher);
    }
    hash_glue_value(align.default_tabskip, &mut hasher);
    align.loop_start.map(|value| value as u64).hash(&mut hasher);
    hasher.finish()
}

fn incomplete_fraction_identity(fraction: &IncompleteFraction) -> u64 {
    let mut hasher = mode_identity_hasher(b"umber-mode-incomplete-fraction-v1");
    fraction.numerator.hash(&mut hasher);
    match fraction.thickness {
        FractionThickness::Default => 0_u8.hash(&mut hasher),
        FractionThickness::Explicit(size) => {
            1_u8.hash(&mut hasher);
            size.raw().hash(&mut hasher);
        }
    }
    fraction.left_delimiter.hash(&mut hasher);
    fraction.right_delimiter.hash(&mut hasher);
    hasher.finish()
}

fn display_interrupt_identity(interrupt: &DisplayInterrupt) -> u64 {
    let mut hasher = mode_identity_hasher(b"umber-mode-display-interrupt-v1");
    (interrupt.active_directions.len() as u64).hash(&mut hasher);
    for direction in &interrupt.active_directions {
        match direction {
            tex_state::node::Direction::BeginL => 0_u8,
            tex_state::node::Direction::EndL => 1,
            tex_state::node::Direction::BeginR => 2,
            tex_state::node::Direction::EndR => 3,
            tex_state::node::Direction::BeginM => 4,
            tex_state::node::Direction::EndM => 5,
        }
        .hash(&mut hasher);
    }
    match &interrupt.prototype {
        Some(prototype) => {
            1_u8.hash(&mut hasher);
            let node = Node::HList(*prototype);
            tex_state::node_sequence::SemanticSequenceIdentity::from_nodes([&node])
                .raw()
                .hash(&mut hasher);
        }
        None => 0_u8.hash(&mut hasher),
    }
    hasher.finish()
}

fn display_eq_no_identity(eq_no: &DisplayEqNo) -> u64 {
    let mut hasher = mode_identity_hasher(b"umber-mode-display-eqno-v1");
    match eq_no.side {
        EqNoSide::Left => 0_u8,
        EqNoSide::Right => 1,
    }
    .hash(&mut hasher);
    eq_no.display.hash(&mut hasher);
    hasher.finish()
}

fn pending_char_identity(value: &PendingHChar) -> u64 {
    let mut hasher = mode_identity_hasher(b"umber-mode-pending-char-v1");
    value.font.hash(&mut hasher);
    (value.ch as u32).hash(&mut hasher);
    hasher.finish()
}

fn pending_current_identity(value: &PendingHRunChar) -> u64 {
    let mut hasher = mode_identity_hasher(b"umber-mode-pending-current-v1");
    value.font.hash(&mut hasher);
    (value.ch as u32).hash(&mut hasher);
    (value.orig.len() as u64).hash(&mut hasher);
    for &ch in &value.orig {
        (ch as u32).hash(&mut hasher);
    }
    value.ligature_present.hash(&mut hasher);
    value.left_hit.hash(&mut hasher);
    value.right_hit.hash(&mut hasher);
    hasher.finish()
}

fn pending_source_identity(source: &[PendingHChar]) -> u64 {
    source.iter().fold(0_u64, |root, value| {
        root.rotate_left(27)
            .wrapping_mul(0x9e37_79b1_85eb_ca87)
            .wrapping_add(pending_char_identity(value))
    })
}

fn mode_list_semantic_identity(list: &ModeList) -> u64 {
    let mut hasher = mode_identity_hasher(MODE_LIST_IDENTITY_DOMAIN);
    (list.sequence.semantic_identity().len() as u64).hash(&mut hasher);
    list.sequence.semantic_identity().raw().hash(&mut hasher);
    list.component_roots.align.hash(&mut hasher);
    list.component_roots.incomplete_fraction.hash(&mut hasher);
    list.component_roots.display_interrupt.hash(&mut hasher);
    list.component_roots.display_eq_no.hash(&mut hasher);
    list.display_alignment.hash(&mut hasher);
    hash_optional_scaled(list.prev_depth, &mut hasher);
    list.prev_graf.hash(&mut hasher);
    list.component_roots.pending_hchars.hash(&mut hasher);
    list.space_factor.hash(&mut hasher);
    list.no_boundary.hash(&mut hasher);
    list.hyphen_language.hash(&mut hasher);
    list.left_hyphen_min.hash(&mut hasher);
    list.right_hyphen_min.hash(&mut hasher);
    hasher.finish()
}

fn mode_nest_semantic_identity(levels: &[ModeLevelSummary]) -> u64 {
    let mut hasher = mode_identity_hasher(MODE_NEST_IDENTITY_DOMAIN);
    1_u16.hash(&mut hasher);
    (levels.len() as u64).hash(&mut hasher);
    for level in levels {
        match level.mode {
            Mode::Vertical => 0_u8,
            Mode::InternalVertical => 1,
            Mode::Horizontal => 2,
            Mode::RestrictedHorizontal => 3,
            Mode::Math => 4,
            Mode::DisplayMath => 5,
        }
        .hash(&mut hasher);
        level.entry_line.hash(&mut hasher);
        level.list.semantic_identity_root.hash(&mut hasher);
    }
    hasher.finish()
}

/// Explicit stack of TeX mode levels.
struct ModeNestStorage {
    levels: Vec<ModeLevelSummary>,
    journal: journal::ModeJournal,
    identity_enabled: bool,
}

#[derive(Clone, Copy, Default)]
struct ModeListRoot {
    semantic: usize,
    physical: usize,
    page_node_root_count: usize,
    sequence_identity: tex_state::node_sequence::SemanticSequenceIdentity,
    semantic_identity_root: u64,
    component_roots: ModeComponentRoots,
    display_alignment: bool,
    prev_depth: Option<Scaled>,
    prev_graf: i32,
    space_factor: i32,
    no_boundary: bool,
    hyphen_language: u8,
    left_hyphen_min: u8,
    right_hyphen_min: u8,
}

impl ModeNestStorage {
    fn placeholder() -> Self {
        let levels = vec![ModeLevelSummary::new(Mode::Vertical)];
        Self {
            journal: journal::ModeJournal::candidate(levels.len()),
            levels,
            identity_enabled: false,
        }
    }
}

/// Opaque bounded root of one mode timeline position.
pub(crate) struct ModeCheckpoint {
    owner: std::rc::Rc<std::cell::RefCell<ModeNestStorage>>,
    cursor: ModeJournalCursor,
    flat_root: Option<ModeLevelSummary>,
    list_roots: Box<[ModeListRoot; 41]>,
    list_root_count: u8,
    reachable_state_identity_root: Option<u64>,
}

impl Clone for ModeCheckpoint {
    fn clone(&self) -> Self {
        Self {
            owner: std::rc::Rc::clone(&self.owner),
            cursor: self.cursor,
            flat_root: self.flat_root.clone(),
            list_roots: self.list_roots.clone(),
            list_root_count: self.list_root_count,
            reachable_state_identity_root: self.reachable_state_identity_root,
        }
    }
}

impl std::fmt::Debug for ModeCheckpoint {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ModeCheckpoint")
            .field("generation", &self.cursor.generation)
            .field("frame", &self.cursor.frame_id)
            .field("cursor", &self.cursor.cursor)
            .finish_non_exhaustive()
    }
}

impl ModeCheckpoint {
    #[cfg(feature = "profiling")]
    pub(crate) fn replay_work(&self) -> u64 {
        self.owner.borrow().replay_work()
    }

    pub(crate) fn retention_owner_address(&self) -> usize {
        std::rc::Rc::as_ptr(&self.owner) as usize
    }

    pub(crate) const fn reachable_state_identity_root(&self) -> Option<u64> {
        self.reachable_state_identity_root
    }

    pub(crate) fn retained_owner_bytes(&self) -> usize {
        let storage = self.owner.borrow();
        std::mem::size_of::<ModeNestStorage>()
            .saturating_add(
                storage
                    .levels
                    .capacity()
                    .saturating_mul(std::mem::size_of::<ModeLevelSummary>()),
            )
            .saturating_add(storage.journal.retained_bytes())
    }

    pub(crate) fn retains_page_node_handles(&self) -> bool {
        let storage = self.owner.borrow();
        storage.levels.iter().any(|level| {
            let list = &level.list;
            list.sequence.retains_page_node_handles()
                || list
                    .incomplete_fraction
                    .as_ref()
                    .is_some_and(|fraction| !fraction.numerator.is_empty())
                || list
                    .display_interrupt
                    .as_ref()
                    .and_then(|interrupt| interrupt.prototype.as_ref())
                    .is_some_and(|prototype| !prototype.children.is_empty())
                || list
                    .display_eq_no
                    .as_ref()
                    .is_some_and(|eqno| !eqno.display.is_empty())
        })
    }

    pub(crate) fn summary(&self) -> ModeNestSummary {
        ModeNestSummary {
            levels: self.owner.borrow().levels.clone(),
        }
    }

    fn restore_storage(&self) -> Result<(), ExecError> {
        let mut storage = self.owner.borrow_mut();
        storage
            .restore_checkpoint_cursor(self.cursor)
            .map_err(|_| ExecError::EmptyModeNestSummary)?;
        let count = usize::from(self.list_root_count);
        if storage.levels.len() != count {
            return Err(ExecError::EmptyModeNestSummary);
        }
        for (level, root) in storage.levels.iter_mut().zip(&self.list_roots[..count]) {
            level.list.display_alignment = root.display_alignment;
            level.list.prev_depth = root.prev_depth;
            level.list.prev_graf = root.prev_graf;
            level.list.space_factor = root.space_factor;
            level.list.no_boundary = root.no_boundary;
            level.list.hyphen_language = root.hyphen_language;
            level.list.left_hyphen_min = root.left_hyphen_min;
            level.list.right_hyphen_min = root.right_hyphen_min;
            level.list.sequence.restore_checkpoint_lengths(
                root.semantic,
                root.physical,
                root.page_node_root_count,
                root.sequence_identity,
            );
            level.list.semantic_identity_root = root.semantic_identity_root;
            level.list.component_roots = root.component_roots;
        }
        Ok(())
    }
}

pub struct ModeNest {
    storage: std::rc::Rc<std::cell::RefCell<ModeNestStorage>>,
    loan: Option<ModeNestLoan>,
    /// TeX82 §216's maximum pre-push `nest_ptr`. This runtime diagnostic is
    /// intentionally absent from summaries, semantic equality, and hashes.
    max_nest_stack: usize,
}

enum ModeNestLoan {
    Flat {
        origin: std::rc::Rc<std::cell::RefCell<ModeNestStorage>>,
        accepted: Box<ModeNestStorage>,
    },
    Rooted {
        origin: std::rc::Rc<std::cell::RefCell<ModeNestStorage>>,
        source_journal: journal::ModeJournal,
        candidate_cursor: ModeJournalCursor,
        accepted_identity_roots: Box<[(u64, ModeComponentRoots); 41]>,
    },
}

impl Clone for ModeNest {
    fn clone(&self) -> Self {
        let storage = self.storage.borrow();
        let levels = storage.levels.clone();
        Self {
            storage: std::rc::Rc::new(std::cell::RefCell::new(ModeNestStorage {
                journal: journal::ModeJournal::enabled(levels.len()),
                levels,
                identity_enabled: storage.identity_enabled,
            })),
            loan: None,
            max_nest_stack: self.max_nest_stack,
        }
    }
}

impl std::fmt::Debug for ModeNest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ModeNest")
            .field("levels", &self.storage.borrow().levels)
            .finish()
    }
}

impl PartialEq for ModeNest {
    fn eq(&self, other: &Self) -> bool {
        self.storage.borrow().levels == other.storage.borrow().levels
    }
}

impl Drop for ModeNest {
    fn drop(&mut self) {
        let Some(loan) = self.loan.take() else {
            return;
        };
        if let ModeNestLoan::Flat { origin, accepted } = loan {
            let candidate = std::mem::replace(
                &mut *self.storage.borrow_mut(),
                ModeNestStorage::placeholder(),
            );
            drop(candidate);
            *origin.borrow_mut() = *accepted;
            return;
        }
        if let ModeNestLoan::Rooted {
            origin,
            source_journal,
            candidate_cursor,
            accepted_identity_roots,
        } = loan
        {
            let mut storage = self.storage.borrow_mut();
            storage
                .rollback_journal(candidate_cursor)
                .expect("candidate mode journal remains innermost");
            for level in &mut storage.levels {
                level.list.sequence.reject_candidate();
            }
            for (level, (root, components)) in storage
                .levels
                .iter_mut()
                .zip(accepted_identity_roots.iter().copied())
            {
                level.list.semantic_identity_root = root;
                level.list.component_roots = components;
            }
            storage.journal = source_journal;
            let returned = std::mem::replace(&mut *storage, ModeNestStorage::placeholder());
            drop(storage);
            *origin.borrow_mut() = returned;
            return;
        }
        unreachable!("all mode loans are handled above");
    }
}

impl Default for ModeNest {
    fn default() -> Self {
        Self::new()
    }
}

impl ModeNest {
    /// TeX82 §11's maximum number of simultaneously saved semantic levels.
    const TEX82_NEST_SIZE: usize = 40;
    const MAX_LIVE_LEVELS: usize = Self::TEX82_NEST_SIZE + 1;

    /// Creates the outer main vertical nest level.
    #[must_use]
    pub fn new() -> Self {
        // The semantic stack is permanently bounded, so allocate its complete
        // pointer array once just as TeX82 does.
        let mut levels = Vec::with_capacity(Self::MAX_LIVE_LEVELS);
        levels.push(ModeLevelSummary::new(Mode::Vertical));
        Self {
            storage: std::rc::Rc::new(std::cell::RefCell::new(ModeNestStorage {
                levels,
                journal: journal::ModeJournal::enabled(1),
                identity_enabled: false,
            })),
            loan: None,
            max_nest_stack: 0,
        }
    }

    /// Rehydrates a nest from snapshot summary state.
    pub fn from_summary(summary: ModeNestSummary) -> Result<Self, ExecError> {
        if summary.levels.is_empty() {
            return Err(ExecError::EmptyModeNestSummary);
        }
        if summary.levels.len() > Self::MAX_LIVE_LEVELS {
            return Err(ExecError::Fatal(tex_command::FatalError::overflow(
                "semantic nest size",
                Self::TEX82_NEST_SIZE as i32,
            )));
        }
        Ok(Self {
            storage: std::rc::Rc::new(std::cell::RefCell::new(ModeNestStorage {
                journal: journal::ModeJournal::enabled(summary.levels.len()),
                levels: summary.levels,
                identity_enabled: false,
            })),
            loan: None,
            max_nest_stack: 0,
        })
    }

    #[must_use]
    pub fn summary(&self) -> ModeNestSummary {
        ModeNestSummary {
            levels: self.storage.borrow().levels.clone(),
        }
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) fn reachable_state_identity_root(&self) -> Option<u64> {
        let storage = self.storage.borrow();
        storage
            .identity_enabled
            .then(|| mode_nest_semantic_identity(&storage.levels))
    }

    /// Enables semantic-root maintenance for one convergence session.
    #[doc(hidden)]
    pub fn enable_reachable_state_identity(&mut self) {
        let mut storage = self.storage.borrow_mut();
        if storage.identity_enabled {
            return;
        }
        assert!(
            storage.levels.len() == 1 && storage.levels[0].list.is_checkpoint_rootless(),
            "mode semantic identity must be selected before execution"
        );
        for level in &mut storage.levels {
            level.list.enable_semantic_identity();
        }
        storage.identity_enabled = true;
    }

    pub(crate) fn checkpoint(&mut self) -> ModeCheckpoint {
        let (flat_root, list_roots, list_root_count, reachable_state_identity_root) = {
            let storage = self.storage.borrow();
            let flat_root = (storage.levels.len() == 1
                && storage.levels[0].list.is_checkpoint_rootless())
            .then(|| storage.levels[0].clone());
            let mut list_roots = Box::new([ModeListRoot::default(); 41]);
            for (root, level) in list_roots.iter_mut().zip(&storage.levels) {
                *root = ModeListRoot {
                    semantic: level.list.sequence.semantic_view().len(),
                    physical: level.list.sequence.physical_view().len(),
                    page_node_root_count: level.list.sequence.page_node_root_count(),
                    sequence_identity: level.list.sequence.semantic_identity(),
                    semantic_identity_root: level.list.semantic_identity_root,
                    component_roots: level.list.component_roots,
                    display_alignment: level.list.display_alignment,
                    prev_depth: level.list.prev_depth,
                    prev_graf: level.list.prev_graf,
                    space_factor: level.list.space_factor,
                    no_boundary: level.list.no_boundary,
                    hyphen_language: level.list.hyphen_language,
                    left_hyphen_min: level.list.left_hyphen_min,
                    right_hyphen_min: level.list.right_hyphen_min,
                };
            }
            (
                flat_root,
                list_roots,
                u8::try_from(storage.levels.len()).expect("mode depth fits u8"),
                storage
                    .identity_enabled
                    .then(|| mode_nest_semantic_identity(&storage.levels)),
            )
        };
        let cursor = self.storage.borrow_mut().begin_journal();
        ModeCheckpoint {
            owner: std::rc::Rc::clone(&self.storage),
            cursor,
            flat_root,
            list_roots,
            list_root_count,
            reachable_state_identity_root,
        }
    }

    pub(crate) fn restore_checkpoint(
        &mut self,
        checkpoint: &ModeCheckpoint,
    ) -> Result<(), ExecError> {
        checkpoint.restore_storage()?;
        self.storage = std::rc::Rc::clone(&checkpoint.owner);
        Ok(())
    }

    pub(crate) fn fork_checkpoint(checkpoint: &ModeCheckpoint) -> Result<Self, ExecError> {
        if let Some(root) = &checkpoint.flat_root {
            let accepted = std::mem::replace(
                &mut *checkpoint.owner.borrow_mut(),
                ModeNestStorage::placeholder(),
            );
            return Ok(Self {
                storage: std::rc::Rc::new(std::cell::RefCell::new(ModeNestStorage {
                    journal: journal::ModeJournal::candidate(1),
                    levels: vec![root.clone()],
                    identity_enabled: checkpoint.reachable_state_identity_root.is_some(),
                })),
                loan: Some(ModeNestLoan::Flat {
                    origin: std::rc::Rc::clone(&checkpoint.owner),
                    accepted: Box::new(accepted),
                }),
                max_nest_stack: 0,
            });
        }
        let mut forked = std::mem::replace(
            &mut *checkpoint.owner.borrow_mut(),
            ModeNestStorage::placeholder(),
        );
        let list_root_count = usize::from(checkpoint.list_root_count);
        if forked.levels.len() != list_root_count {
            *checkpoint.owner.borrow_mut() = forked;
            return Err(ExecError::EmptyModeNestSummary);
        }
        let mut accepted_identity_roots = Box::new([(0, ModeComponentRoots::default()); 41]);
        for (saved, level) in accepted_identity_roots.iter_mut().zip(&forked.levels) {
            *saved = (
                level.list.semantic_identity_root,
                level.list.component_roots,
            );
        }
        for (level, root) in forked
            .levels
            .iter_mut()
            .zip(&checkpoint.list_roots[..list_root_count])
        {
            level.list.display_alignment = root.display_alignment;
            level.list.prev_depth = root.prev_depth;
            level.list.prev_graf = root.prev_graf;
            level.list.space_factor = root.space_factor;
            level.list.no_boundary = root.no_boundary;
            level.list.hyphen_language = root.hyphen_language;
            level.list.left_hyphen_min = root.left_hyphen_min;
            level.list.right_hyphen_min = root.right_hyphen_min;
            level.list.semantic_identity_root = root.semantic_identity_root;
            level.list.component_roots = root.component_roots;
            level.list.sequence.begin_candidate(
                root.semantic,
                root.physical,
                root.page_node_root_count,
                root.sequence_identity,
            );
        }
        let level_count = forked.levels.len();
        let source_journal = std::mem::replace(
            &mut forked.journal,
            journal::ModeJournal::candidate(level_count),
        );
        let candidate_cursor = forked.begin_journal();
        Ok(Self {
            storage: std::rc::Rc::new(std::cell::RefCell::new(forked)),
            loan: Some(ModeNestLoan::Rooted {
                origin: std::rc::Rc::clone(&checkpoint.owner),
                source_journal,
                candidate_cursor,
                accepted_identity_roots,
            }),
            max_nest_stack: 0,
        })
    }

    #[must_use]
    pub fn depth(&self) -> usize {
        self.storage.borrow().levels.len()
    }

    /// TeX82 §216's maximum `nest_ptr` observed before a semantic push.
    #[must_use]
    pub const fn maximum_saved_depth(&self) -> usize {
        self.max_nest_stack
    }

    /// Retains a job's operational high-water mark across restoration of an
    /// earlier semantic mode summary. The summary itself remains free of
    /// diagnostic accounting, so fresh format/session materialization still
    /// starts from zero.
    pub(crate) fn retain_maximum_saved_depth(&mut self, maximum: usize) {
        self.max_nest_stack = self.max_nest_stack.max(maximum);
    }

    #[must_use]
    pub fn current_mode(&self) -> Mode {
        self.storage
            .borrow()
            .levels
            .last()
            .expect("ModeNest always has at least one level")
            .mode()
    }

    /// Saves the current level and enters a new empty semantic level.
    ///
    /// TeX82 §216 checks `nest_size` before copying `cur_list` into the
    /// semantic stack. Accordingly, a rejected push leaves every live level
    /// and the journal unchanged.
    pub fn push(&mut self, mode: Mode) -> Result<(), ExecError> {
        self.push_at_line(mode, 0)
    }

    /// Enters a semantic level while retaining TeX's `mode_line` diagnostic
    /// context. A negative line identifies the output-routine level.
    pub(crate) fn push_at_line(&mut self, mode: Mode, entry_line: i32) -> Result<(), ExecError> {
        let depth = self.storage.borrow().levels.len();
        if depth > Self::TEX82_NEST_SIZE {
            return Err(ExecError::Fatal(tex_command::FatalError::overflow(
                "semantic nest size",
                Self::TEX82_NEST_SIZE as i32,
            )));
        }
        self.max_nest_stack = self.max_nest_stack.max(depth.saturating_sub(1));
        let mut level = ModeLevelSummary::new(mode);
        level.set_entry_line(entry_line);
        if matches!(mode, Mode::Horizontal | Mode::RestrictedHorizontal) {
            level.mutate_list(|list| list.set_space_factor(1000));
        }
        let mut storage = self.storage.borrow_mut();
        storage.levels.push(level);
        storage.journal.record_level_push();
        Ok(())
    }

    pub fn pop(&mut self) -> Result<ModeLevelSummary, ExecError> {
        if self.storage.borrow().levels.len() == 1 {
            return Err(ExecError::CannotPopBaseMode);
        }
        if self.current_list().pending_hchars().is_some() {
            return Err(ExecError::UncommittedPendingHchars);
        }
        let mut storage = self.storage.borrow_mut();
        let popped = storage
            .levels
            .pop()
            .expect("length checked before popping mode level");
        storage.journal.record_level_pop(popped.clone());
        Ok(popped)
    }

    pub fn current_list(&self) -> std::cell::Ref<'_, ModeList> {
        std::cell::Ref::map(self.storage.borrow(), |storage| {
            storage
                .levels
                .last()
                .expect("ModeNest always has at least one level")
                .list()
        })
    }

    /// Appends one owned node to the current mode list through its journaled
    /// mutation boundary.
    pub fn push_current_node(&mut self, node: Node) {
        self.current_list_mutation().push(node);
    }

    pub(crate) fn current_list_mutation(&mut self) -> ModeListMutation<'_> {
        let storage = self.storage.borrow_mut();
        let index = storage.levels.len() - 1;
        let (levels, journal) = std::cell::RefMut::map_split(storage, |storage| {
            (&mut storage.levels, &mut storage.journal)
        });
        let list = std::cell::RefMut::map(levels, |levels| {
            &mut levels
                .last_mut()
                .expect("ModeNest always has at least one level")
                .list
        });
        ModeListMutation {
            list: ModeListBorrow::Shared(list),
            journal: ModeListJournalBorrow::Shared { journal, index },
        }
    }

    pub(crate) fn list_mutation(&mut self, index: usize) -> Option<ModeListMutation<'_>> {
        let storage = self.storage.borrow_mut();
        storage.levels.get(index)?;
        let (levels, journal) = std::cell::RefMut::map_split(storage, |storage| {
            (&mut storage.levels, &mut storage.journal)
        });
        let list = std::cell::RefMut::map(levels, |levels| &mut levels[index].list);
        Some(ModeListMutation {
            list: ModeListBorrow::Shared(list),
            journal: ModeListJournalBorrow::Shared { journal, index },
        })
    }

    #[must_use]
    pub fn enclosing_vertical_prev_graf(&self) -> i32 {
        let storage = self.storage.borrow();
        let index = enclosing_vertical_index(&storage.levels);
        storage.levels[index].list().prev_graf()
    }

    #[must_use]
    pub fn enclosing_vertical_prev_depth(&self) -> Option<Scaled> {
        let storage = self.storage.borrow();
        let index = enclosing_vertical_index(&storage.levels);
        storage.levels[index].list().prev_depth()
    }

    pub fn set_enclosing_vertical_prev_graf(&mut self, lines: i32) {
        let index = enclosing_vertical_index(&self.storage.borrow().levels);
        self.list_mutation(index)
            .expect("enclosing vertical level exists")
            .set_prev_graf(lines);
    }
}

fn enclosing_vertical_index(levels: &[ModeLevelSummary]) -> usize {
    levels
        .iter()
        .rposition(|level| matches!(level.mode(), Mode::Vertical | Mode::InternalVertical))
        .expect("base vertical level is always present")
}

#[cfg(test)]
mod tests;
