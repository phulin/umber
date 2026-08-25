use smallvec::{SmallVec, smallvec};
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
}

impl ModeList {
    #[must_use]
    pub fn nodes(&self) -> &[Node] {
        self.sequence.semantic()
    }

    #[must_use]
    pub fn physical_nodes(&self) -> &[Node] {
        self.sequence.physical()
    }

    pub fn take_nodes(&mut self) -> Vec<Node> {
        std::mem::take(&mut self.sequence).take().0
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
        self.pending_hchars = Some(PendingHRun::new(font, ch, origin, self.nodes().len()));
    }

    pub(crate) fn pending_hchars(&self) -> Option<&PendingHRun> {
        self.pending_hchars.as_ref()
    }

    pub(crate) fn set_pending_hchars(&mut self, pending: PendingHRun) {
        self.pending_hchars = Some(pending);
    }

    pub(crate) fn take_pending_hchars(&mut self) -> Option<PendingHRun> {
        self.pending_hchars.take()
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
        self.sequence.mutate_semantic(|nodes| nodes.pop())
    }

    pub(crate) fn remove_node_range(
        &mut self,
        range: std::ops::RangeInclusive<usize>,
    ) -> Vec<Node> {
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
        self.align_state = Some(state);
    }

    pub fn with_align_state_mut<R>(
        &mut self,
        mutate: impl for<'a> FnOnce(&'a mut AlignState) -> R,
    ) -> Option<R> {
        self.align_state.as_mut().map(mutate)
    }

    pub fn take_align_state(&mut self) -> Option<AlignState> {
        self.align_state.take()
    }

    #[must_use]
    pub fn incomplete_fraction(&self) -> Option<&IncompleteFraction> {
        self.incomplete_fraction.as_ref()
    }

    pub fn set_incomplete_fraction(&mut self, fraction: IncompleteFraction) {
        self.incomplete_fraction = Some(fraction);
    }

    pub fn take_incomplete_fraction(&mut self) -> Option<IncompleteFraction> {
        self.incomplete_fraction.take()
    }

    pub fn set_display_interrupt(&mut self, interrupt: DisplayInterrupt) {
        self.display_interrupt = Some(interrupt);
    }

    #[must_use]
    pub const fn display_interrupt(&self) -> Option<&DisplayInterrupt> {
        self.display_interrupt.as_ref()
    }

    pub fn take_display_interrupt(&mut self) -> Option<DisplayInterrupt> {
        self.display_interrupt.take()
    }

    pub fn set_display_eq_no(&mut self, eq_no: DisplayEqNo) {
        self.display_eq_no = Some(eq_no);
    }

    #[must_use]
    pub const fn display_eq_no(&self) -> Option<&DisplayEqNo> {
        self.display_eq_no.as_ref()
    }

    pub fn take_display_eq_no(&mut self) -> Option<DisplayEqNo> {
        self.display_eq_no.take()
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
pub(crate) struct ModeListMutation<'a> {
    list: &'a mut ModeList,
    journal: Option<journal::ListJournal<'a>>,
}

impl ModeListMutation<'_> {
    pub(crate) fn push(&mut self, node: Node) {
        self.list.push(node);
    }

    pub(crate) fn nodes(&self) -> &[Node] {
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
        self.record_node(index);
        self.list.with_node_mut(index, mutate)
    }

    pub(crate) fn with_last_node_mut<R>(
        &mut self,
        mutate: impl for<'a> FnOnce(&'a mut Node) -> R,
    ) -> Option<R> {
        if let Some(index) = self.list.nodes().len().checked_sub(1) {
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
        if let Some(journal) = &mut self.journal {
            journal.record_pending_projection(self.list.pending_hchars.as_ref());
        }
        self.list.begin_pending_hchars(font, ch, origin);
    }

    pub(crate) fn set_pending_hchars(&mut self, pending: PendingHRun) {
        if let Some(journal) = &mut self.journal {
            journal.record_pending_value(self.list.pending_hchars.as_ref());
        }
        self.list.set_pending_hchars(pending);
    }

    pub(crate) fn take_pending_hchars(&mut self) -> Option<PendingHRun> {
        if let Some(journal) = &mut self.journal {
            journal.record_pending_value(self.list.pending_hchars.as_ref());
        }
        self.list.take_pending_hchars()
    }

    pub(crate) fn with_pending_hchars_mut<R>(
        &mut self,
        mutate: impl FnOnce(&mut PendingHRun) -> R,
    ) -> Option<R> {
        if let Some(journal) = &mut self.journal {
            journal.record_pending_projection(self.list.pending_hchars.as_ref());
        }
        self.list.pending_hchars.as_mut().map(mutate)
    }

    pub(crate) fn set_space_factor(&mut self, value: i32) {
        if let Some(journal) = &mut self.journal {
            journal.record_space_factor(self.list.space_factor);
        }
        self.list.set_space_factor(value);
    }

    pub(crate) fn space_factor(&self) -> i32 {
        self.list.space_factor()
    }

    pub(crate) fn set_no_boundary(&mut self, value: bool) {
        if let Some(journal) = &mut self.journal {
            journal.record_no_boundary(self.list.no_boundary);
        }
        self.list.set_no_boundary(value);
    }

    pub(crate) fn set_hyphen_context(&mut self, language: u8, left: u8, right: u8) {
        if let Some(journal) = &mut self.journal {
            journal.record_hyphen_context((
                self.list.hyphen_language,
                self.list.left_hyphen_min,
                self.list.right_hyphen_min,
            ));
        }
        self.list.set_hyphen_context(language, left, right);
    }

    #[cfg(test)]
    pub(crate) fn set_hyphen_language(&mut self, language: u8) {
        if let Some(journal) = &mut self.journal {
            journal.record_hyphen_context((
                self.list.hyphen_language,
                self.list.left_hyphen_min,
                self.list.right_hyphen_min,
            ));
        }
        self.list.set_hyphen_language(language);
    }

    pub(crate) fn set_prev_depth(&mut self, depth: Scaled) {
        if let Some(journal) = &mut self.journal {
            journal.record_prev_depth(self.list.prev_depth);
        }
        self.list.set_prev_depth(depth);
    }

    pub(crate) fn set_prev_graf(&mut self, lines: i32) {
        if let Some(journal) = &mut self.journal {
            journal.record_prev_graf(self.list.prev_graf);
        }
        self.list.set_prev_graf(lines);
    }

    #[cfg(test)]
    pub(crate) fn set_align_state(&mut self, state: AlignState) {
        if let Some(journal) = &mut self.journal {
            journal.record_align_state(self.list.align_state.clone());
        }
        self.list.set_align_state(state);
    }

    #[cfg(test)]
    pub(crate) fn with_align_state_mut<R>(
        &mut self,
        mutate: impl for<'a> FnOnce(&'a mut AlignState) -> R,
    ) -> Option<R> {
        if let Some(journal) = &mut self.journal {
            journal.record_align_state(self.list.align_state.clone());
        }
        self.list.with_align_state_mut(mutate)
    }

    #[cfg(test)]
    pub(crate) fn take_align_state(&mut self) -> Option<AlignState> {
        if let Some(journal) = &mut self.journal {
            journal.record_align_state(self.list.align_state.clone());
        }
        self.list.take_align_state()
    }

    pub(crate) fn set_incomplete_fraction(&mut self, fraction: IncompleteFraction) {
        if let Some(journal) = &mut self.journal {
            journal.record_incomplete_fraction(self.list.incomplete_fraction.clone());
        }
        self.list.set_incomplete_fraction(fraction);
    }

    pub(crate) fn take_incomplete_fraction(&mut self) -> Option<IncompleteFraction> {
        if let Some(journal) = &mut self.journal {
            journal.record_incomplete_fraction(self.list.incomplete_fraction.clone());
        }
        self.list.take_incomplete_fraction()
    }

    pub(crate) fn incomplete_fraction(&self) -> Option<&IncompleteFraction> {
        self.list.incomplete_fraction()
    }

    pub(crate) fn set_display_interrupt(&mut self, interrupt: DisplayInterrupt) {
        if let Some(journal) = &mut self.journal {
            journal.record_display_interrupt(self.list.display_interrupt.clone());
        }
        self.list.set_display_interrupt(interrupt);
    }

    pub(crate) fn take_display_interrupt(&mut self) -> Option<DisplayInterrupt> {
        if let Some(journal) = &mut self.journal {
            journal.record_display_interrupt(self.list.display_interrupt.clone());
        }
        self.list.take_display_interrupt()
    }

    pub(crate) fn set_display_eq_no(&mut self, eq_no: DisplayEqNo) {
        if let Some(journal) = &mut self.journal {
            journal.record_display_eq_no(self.list.display_eq_no.clone());
        }
        self.list.set_display_eq_no(eq_no);
    }

    pub(crate) fn take_display_eq_no(&mut self) -> Option<DisplayEqNo> {
        if let Some(journal) = &mut self.journal {
            journal.record_display_eq_no(self.list.display_eq_no.clone());
        }
        self.list.take_display_eq_no()
    }

    pub(crate) fn set_display_alignment(&mut self, nodes: Vec<Node>, prev_depth: Option<Scaled>) {
        self.record_nodes();
        if let Some(journal) = &mut self.journal {
            journal.record_prev_depth(self.list.prev_depth);
            journal.record_display_alignment(self.list.display_alignment);
        }
        self.list.set_display_alignment(nodes, prev_depth);
    }

    pub(crate) fn take_display_alignment(&mut self) -> Option<(Vec<Node>, Option<Scaled>)> {
        if self.list.display_alignment {
            self.record_nodes();
        }
        if let Some(journal) = &mut self.journal {
            journal.record_prev_depth(self.list.prev_depth);
            journal.record_display_alignment(self.list.display_alignment);
        }
        self.list.take_display_alignment()
    }

    fn record_node(&mut self, index: usize) {
        if let Some(journal) = &mut self.journal
            && self.list.nodes().get(index).is_some()
        {
            journal.record_nodes(&self.list.sequence);
        }
    }

    fn record_nodes(&mut self) {
        if let Some(journal) = &mut self.journal {
            journal.record_nodes(&self.list.sequence);
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
    }

    pub fn start_cell(&mut self, column: usize, span_count: u16) {
        self.current_col = column;
        self.current_span = span_count;
    }

    pub fn finish_cell(&mut self, next_column: usize) {
        self.current_col = next_column;
        self.current_span = 1;
    }

    pub fn finish_row(&mut self) {
        self.current_row += 1;
        self.current_col = 0;
        self.current_span = 1;
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
}

impl PendingHRun {
    pub(crate) fn new(font: FontId, ch: char, origin: OriginId, insertion_index: usize) -> Self {
        Self {
            first: PendingHChar { font, ch, origin },
            current: PendingHRunChar::new(font, ch, origin),
            insertion_index,
            source: vec![PendingHChar { font, ch, origin }],
            script: tex_fonts::character_script(ch),
        }
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
        mutate(&mut self.list)
    }

    pub(crate) fn list_mutation(&mut self) -> ModeListMutation<'_> {
        ModeListMutation {
            list: &mut self.list,
            journal: None,
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

    pub(crate) fn font_roots_are_live(&self, mut is_live: impl FnMut(FontId) -> bool) -> bool {
        self.levels.iter().all(|level| {
            let list = &level.list;
            let nodes_are_live = list.nodes().iter().all(|node| {
                let mut live = true;
                node.visit_fonts(|font| live &= is_live(font));
                live
            });
            let pending_is_live = list.pending_hchars.as_ref().is_none_or(|pending| {
                is_live(pending.first.font)
                    && is_live(pending.current.font)
                    && pending.source.iter().all(|source| is_live(source.font))
            });
            nodes_are_live && pending_is_live
        })
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
        #[cfg(test)]
        SEMANTIC_FINGERPRINT_CALLS.with(|calls| calls.set(calls.get() + 1));
        universe.engine_boundary_hash(0x6d6f_6465_5f6e_6573, |projection| {
            projection.usize(self.levels.len());
            for level in self.levels.iter() {
                hash_mode(level.mode, projection);
                projection.i32(level.entry_line);
                hash_mode_list(&level.list, universe, projection);
            }
        })
    }
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
    projection.nodes(list.nodes());
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

/// Explicit stack of TeX mode levels.
pub struct ModeNest {
    levels: Vec<ModeLevelSummary>,
    journal: journal::ModeJournal,
    /// TeX82 §216's maximum pre-push `nest_ptr`. This runtime diagnostic is
    /// intentionally absent from summaries, semantic equality, and hashes.
    max_nest_stack: usize,
}

impl Clone for ModeNest {
    fn clone(&self) -> Self {
        Self {
            levels: self.levels.clone(),
            journal: journal::ModeJournal::enabled(self.levels.len()),
            max_nest_stack: self.max_nest_stack,
        }
    }
}

impl std::fmt::Debug for ModeNest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ModeNest")
            .field("levels", &self.levels)
            .finish()
    }
}

impl PartialEq for ModeNest {
    fn eq(&self, other: &Self) -> bool {
        self.levels == other.levels
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
            levels,
            journal: journal::ModeJournal::enabled(1),
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
            journal: journal::ModeJournal::enabled(summary.levels.len()),
            levels: summary.levels,
            max_nest_stack: 0,
        })
    }

    #[must_use]
    pub fn summary(&self) -> ModeNestSummary {
        ModeNestSummary {
            levels: self.levels.clone(),
        }
    }

    /// Publishes every live native-node sidecar at an externally visible
    /// episode boundary. The immutable page roots are carried by the mode
    /// summary; subsequent mutation invalidates only the affected sidecar.
    pub(crate) fn publish_node_sidecars<G>(&mut self, stores: &mut Universe<G>) {
        for level in &mut self.levels {
            level.list.sequence.publish_sidecars(stores);
        }
    }

    #[must_use]
    pub fn depth(&self) -> usize {
        self.levels.len()
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
        self.levels
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
        if self.levels.len() > Self::TEX82_NEST_SIZE {
            return Err(ExecError::Fatal(tex_command::FatalError::overflow(
                "semantic nest size",
                Self::TEX82_NEST_SIZE as i32,
            )));
        }
        self.max_nest_stack = self.max_nest_stack.max(self.levels.len().saturating_sub(1));
        let mut level = ModeLevelSummary::new(mode);
        level.set_entry_line(entry_line);
        if matches!(mode, Mode::Horizontal | Mode::RestrictedHorizontal) {
            level.mutate_list(|list| list.set_space_factor(1000));
        }
        self.levels_mut_for_push().push(level);
        self.journal.record_level_push();
        Ok(())
    }

    fn levels_mut_for_push(&mut self) -> &mut Vec<ModeLevelSummary> {
        &mut self.levels
    }

    pub fn pop(&mut self) -> Result<ModeLevelSummary, ExecError> {
        if self.levels.len() == 1 {
            return Err(ExecError::CannotPopBaseMode);
        }
        if self.current_list().pending_hchars().is_some() {
            return Err(ExecError::UncommittedPendingHchars);
        }
        let popped = self
            .levels
            .pop()
            .expect("length checked before popping mode level");
        self.journal.record_level_pop(popped.clone());
        Ok(popped)
    }

    pub fn current_list(&self) -> &ModeList {
        self.levels
            .last()
            .expect("ModeNest always has at least one level")
            .list()
    }

    /// Appends one owned node to the current mode list through its journaled
    /// mutation boundary.
    pub fn push_current_node(&mut self, node: Node) {
        self.current_list_mutation().push(node);
    }

    pub(crate) fn current_list_mutation(&mut self) -> ModeListMutation<'_> {
        let index = self.levels.len() - 1;
        let (levels, journal) = (&mut self.levels, &mut self.journal);
        let level = levels
            .last_mut()
            .expect("ModeNest always has at least one level");
        ModeListMutation {
            list: &mut level.list,
            journal: journal.list(index),
        }
    }

    pub(crate) fn list_mutation(&mut self, index: usize) -> Option<ModeListMutation<'_>> {
        let (levels, journal) = (&mut self.levels, &mut self.journal);
        levels.get_mut(index).map(|level| ModeListMutation {
            list: &mut level.list,
            journal: journal.list(index),
        })
    }

    #[must_use]
    pub fn enclosing_vertical_prev_graf(&self) -> i32 {
        let index = self.enclosing_vertical_index();
        self.levels[index].list().prev_graf()
    }

    #[must_use]
    pub fn enclosing_vertical_prev_depth(&self) -> Option<Scaled> {
        let index = self.enclosing_vertical_index();
        self.levels[index].list().prev_depth()
    }

    pub fn set_enclosing_vertical_prev_graf(&mut self, lines: i32) {
        let index = self.enclosing_vertical_index();
        self.list_mutation(index)
            .expect("enclosing vertical level exists")
            .set_prev_graf(lines);
    }

    fn enclosing_vertical_index(&self) -> usize {
        self.levels
            .iter()
            .rposition(|level| matches!(level.mode(), Mode::Vertical | Mode::InternalVertical))
            .expect("base vertical level is always present")
    }
}

#[cfg(test)]
mod tests;
