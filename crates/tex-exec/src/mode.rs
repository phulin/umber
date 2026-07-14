use std::sync::Arc;
use tex_expand::EngineMode;
use tex_state::ids::FontId;
use tex_state::ids::GlueId;
use tex_state::ids::NodeListId;
use tex_state::ids::TokenListId;
use tex_state::math::FractionThickness;
use tex_state::node::Node;
use tex_state::scaled::Scaled;
use tex_state::{EngineBoundaryHasher, Universe};

use crate::ExecError;

/// TeX's sentinel depth used before any vertical-list box has established a baseline.
pub const IGNORE_DEPTH: Scaled = Scaled::from_raw(-65_536_000);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParagraphParams {
    pub left_skip: GlueId,
    pub right_skip: GlueId,
    pub par_fill_skip: GlueId,
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

/// The list-under-construction owned by one mode level.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModeList {
    nodes: Arc<Vec<Node>>,
    align_state: Option<AlignState>,
    incomplete_fraction: Option<IncompleteFraction>,
    display_interrupt: Option<DisplayInterrupt>,
    display_eq_no: Option<DisplayEqNo>,
    prev_depth: Option<Scaled>,
    prev_graf: i32,
    pending_hchars: Option<PendingHRun>,
    space_factor: i32,
    no_boundary: bool,
    hyphen_language: u8,
}

impl ModeList {
    #[must_use]
    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    pub fn take_nodes(&mut self) -> Vec<Node> {
        Arc::try_unwrap(std::mem::take(&mut self.nodes)).unwrap_or_else(|shared| (*shared).clone())
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn push(&mut self, node: Node) {
        Arc::make_mut(&mut self.nodes).push(node);
    }

    pub fn append(&mut self, nodes: impl IntoIterator<Item = Node>) {
        Arc::make_mut(&mut self.nodes).extend(nodes);
    }

    pub(crate) fn push_reconstituted(
        &mut self,
        insertion: Option<(usize, Node)>,
        first: Node,
        second: Option<Node>,
        third: Option<Node>,
    ) {
        let target = Arc::make_mut(&mut self.nodes);
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
    }

    pub(crate) fn begin_pending_hchars(&mut self, font: FontId, ch: char) {
        debug_assert!(self.pending_hchars.is_none());
        self.pending_hchars = Some(PendingHRun::new(font, ch, self.nodes.len()));
    }

    pub(crate) fn pending_hchars(&self) -> Option<PendingHRun> {
        self.pending_hchars.clone()
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
        match self.nodes.last() {
            Some(Node::HList(_)) | Some(Node::VList(_)) => {}
            _ => return None,
        }
        let mut node = Arc::make_mut(&mut self.nodes)
            .pop()
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
        Arc::make_mut(&mut self.nodes).pop()
    }

    pub fn last_node_mut(&mut self) -> Option<&mut Node> {
        Arc::make_mut(&mut self.nodes).last_mut()
    }

    #[must_use]
    pub fn align_state(&self) -> Option<&AlignState> {
        self.align_state.as_ref()
    }

    pub fn set_align_state(&mut self, state: AlignState) {
        self.align_state = Some(state);
    }

    pub fn align_state_mut(&mut self) -> Option<&mut AlignState> {
        self.align_state.as_mut()
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AlignColumn {
    pub u_template: TokenListId,
    pub v_template: TokenListId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AlignState {
    kind: AlignmentKind,
    pack_spec: AlignmentPackSpec,
    columns: Vec<AlignColumn>,
    tabskips: Vec<GlueId>,
    default_tabskip: GlueId,
    loop_start: Option<usize>,
    current_row: usize,
    current_col: usize,
    current_span: u16,
    brace_depth: i32,
    suppress_redundant_cr: bool,
}

impl AlignState {
    #[must_use]
    pub fn new(
        kind: AlignmentKind,
        pack_spec: AlignmentPackSpec,
        columns: Vec<AlignColumn>,
        tabskips: Vec<GlueId>,
        default_tabskip: GlueId,
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
            brace_depth: 0,
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
    pub fn tabskips(&self) -> &[GlueId] {
        &self.tabskips
    }

    #[must_use]
    pub const fn default_tabskip(&self) -> GlueId {
        self.default_tabskip
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
    pub const fn brace_depth(&self) -> i32 {
        self.brace_depth
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
    pub fn tabskip_for_boundary(&self, boundary: usize) -> GlueId {
        if let Some(tabskip) = self.tabskips.get(boundary) {
            return *tabskip;
        }
        let Some(column) = boundary.checked_sub(1) else {
            return self.default_tabskip;
        };
        let Some(loop_start) = self.loop_start else {
            return self.default_tabskip;
        };
        let Some(repeat_len) = self.columns.len().checked_sub(loop_start) else {
            return self.default_tabskip;
        };
        if repeat_len == 0 || column < loop_start {
            return self.default_tabskip;
        }
        let repeated_column = loop_start + (column - loop_start) % repeat_len;
        self.tabskips
            .get(repeated_column + 1)
            .copied()
            .unwrap_or(self.default_tabskip)
    }

    pub fn start_row(&mut self) {
        self.current_col = 0;
        self.current_span = 1;
        self.brace_depth = 0;
    }

    pub fn start_cell(&mut self, column: usize, span_count: u16) {
        self.current_col = column;
        self.current_span = span_count;
        self.brace_depth = 0;
    }

    pub fn finish_cell(&mut self, next_column: usize) {
        self.current_col = next_column;
        self.current_span = 1;
        self.brace_depth = 0;
    }

    pub fn finish_row(&mut self) {
        self.current_row += 1;
        self.current_col = 0;
        self.current_span = 1;
        self.brace_depth = 0;
    }

    pub fn increment_brace_depth(&mut self) {
        self.brace_depth += 1;
    }

    pub fn decrement_brace_depth(&mut self) {
        self.brace_depth -= 1;
    }

    pub fn set_brace_depth(&mut self, value: i32) {
        self.brace_depth = value;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PendingHChar {
    pub font: FontId,
    pub ch: char,
}

/// Streaming state for the unresolved tail of one horizontal character run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PendingHRun {
    pub(crate) first: PendingHChar,
    pub(crate) current: PendingHRunChar,
    pub(crate) node_start: usize,
}

impl PendingHRun {
    pub(crate) fn new(font: FontId, ch: char, node_start: usize) -> Self {
        Self {
            first: PendingHChar { font, ch },
            current: PendingHRunChar::new(font, ch),
            node_start,
        }
    }
}

/// Current glyph and original-character range carried through ligature folding.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PendingHRunChar {
    pub(crate) font: FontId,
    pub(crate) ch: char,
    pub(crate) orig: Vec<char>,
    pub(crate) ligature_present: bool,
}

impl PendingHRunChar {
    pub(crate) fn new(font: FontId, ch: char) -> Self {
        Self {
            font,
            ch,
            orig: vec![ch],
            ligature_present: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct IncompleteFraction {
    pub numerator: NodeListId,
    pub thickness: FractionThickness,
    pub left_delimiter: Option<u32>,
    pub right_delimiter: Option<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DisplayInterrupt {
    pub active_directions: Vec<tex_state::node::Direction>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DisplayEqNo {
    pub side: EqNoSide,
    pub display: NodeListId,
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
    list: ModeList,
}

impl ModeLevelSummary {
    #[must_use]
    pub fn new(mode: Mode) -> Self {
        Self {
            mode,
            list: ModeList::default(),
        }
    }

    #[must_use]
    pub const fn mode(&self) -> Mode {
        self.mode
    }

    #[must_use]
    pub fn list(&self) -> &ModeList {
        &self.list
    }

    pub fn list_mut(&mut self) -> &mut ModeList {
        &mut self.list
    }
}

/// Snapshot-coverable summary of the whole mode nest.
#[derive(Clone, Debug, PartialEq)]
pub struct ModeNestSummary {
    levels: Arc<Vec<ModeLevelSummary>>,
}

impl ModeNestSummary {
    #[must_use]
    pub fn levels(&self) -> &[ModeLevelSummary] {
        &self.levels
    }

    pub(crate) fn shares_root_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.levels, &other.levels)
    }

    pub(crate) fn semantic_fingerprint(&self, universe: &Universe) -> u64 {
        universe.engine_boundary_hash(0x6d6f_6465_5f6e_6573, |projection| {
            projection.usize(self.levels.len());
            for level in self.levels.iter() {
                hash_mode(level.mode, projection);
                hash_mode_list(&level.list, projection);
            }
        })
    }
}

fn hash_mode(mode: Mode, projection: &mut EngineBoundaryHasher<'_>) {
    projection.u8(match mode {
        Mode::Vertical => 0,
        Mode::InternalVertical => 1,
        Mode::Horizontal => 2,
        Mode::RestrictedHorizontal => 3,
        Mode::Math => 4,
        Mode::DisplayMath => 5,
    });
}

fn hash_mode_list(list: &ModeList, projection: &mut EngineBoundaryHasher<'_>) {
    projection.nodes(&list.nodes);
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
                projection.token_list(column.u_template);
                projection.token_list(column.v_template);
            }
            projection.usize(align.tabskips.len());
            for &tabskip in &align.tabskips {
                projection.glue(tabskip);
            }
            projection.glue(align.default_tabskip);
            hash_optional_usize(align.loop_start, projection);
            projection.usize(align.current_row);
            projection.usize(align.current_col);
            projection.u16(align.current_span);
            projection.i32(align.brace_depth);
            projection.bool(align.suppress_redundant_cr);
        }
        None => projection.bool(false),
    }
    match &list.incomplete_fraction {
        Some(fraction) => {
            projection.bool(true);
            projection.node_list(fraction.numerator);
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
                    tex_state::node::Direction::BeginR => 1,
                    tex_state::node::Direction::EndL => 2,
                    tex_state::node::Direction::EndR => 3,
                });
            }
        }
        None => projection.bool(false),
    }
    match list.display_eq_no {
        Some(eq_no) => {
            projection.bool(true);
            projection.u8(match eq_no.side {
                EqNoSide::Left => 0,
                EqNoSide::Right => 1,
            });
            projection.node_list(eq_no.display);
        }
        None => projection.bool(false),
    }
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
            projection.usize(pending.node_start);
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
}

fn hash_optional_usize(value: Option<usize>, projection: &mut EngineBoundaryHasher<'_>) {
    match value {
        Some(value) => {
            projection.bool(true);
            projection.usize(value);
        }
        None => projection.bool(false),
    }
}

fn hash_optional_u32(value: Option<u32>, projection: &mut EngineBoundaryHasher<'_>) {
    match value {
        Some(value) => {
            projection.bool(true);
            projection.u32(value);
        }
        None => projection.bool(false),
    }
}

/// Explicit stack of TeX mode levels.
#[derive(Clone, Debug, PartialEq)]
pub struct ModeNest {
    levels: Arc<Vec<ModeLevelSummary>>,
}

impl Default for ModeNest {
    fn default() -> Self {
        Self::new()
    }
}

impl ModeNest {
    /// Creates the outer main vertical nest level.
    #[must_use]
    pub fn new() -> Self {
        Self {
            levels: Arc::new(vec![ModeLevelSummary::new(Mode::Vertical)]),
        }
    }

    /// Rehydrates a nest from snapshot summary state.
    pub fn from_summary(summary: ModeNestSummary) -> Result<Self, ExecError> {
        if summary.levels.is_empty() {
            return Err(ExecError::EmptyModeNestSummary);
        }
        Ok(Self {
            levels: summary.levels,
        })
    }

    #[must_use]
    pub fn summary(&self) -> ModeNestSummary {
        ModeNestSummary {
            levels: self.levels.clone(),
        }
    }

    #[must_use]
    pub fn depth(&self) -> usize {
        self.levels.len()
    }

    #[must_use]
    pub fn current_mode(&self) -> Mode {
        self.levels
            .last()
            .expect("ModeNest always has at least one level")
            .mode()
    }

    pub fn push(&mut self, mode: Mode) {
        let mut level = ModeLevelSummary::new(mode);
        if matches!(mode, Mode::Horizontal | Mode::RestrictedHorizontal) {
            level.list_mut().set_space_factor(1000);
        }
        Arc::make_mut(&mut self.levels).push(level);
    }

    pub fn pop(&mut self) -> Result<ModeLevelSummary, ExecError> {
        if self.levels.len() == 1 {
            return Err(ExecError::CannotPopBaseMode);
        }
        Ok(Arc::make_mut(&mut self.levels)
            .pop()
            .expect("length checked before popping mode level"))
    }

    pub fn current_list(&self) -> &ModeList {
        self.levels
            .last()
            .expect("ModeNest always has at least one level")
            .list()
    }

    pub fn current_list_mut(&mut self) -> &mut ModeList {
        Arc::make_mut(&mut self.levels)
            .last_mut()
            .expect("ModeNest always has at least one level")
            .list_mut()
    }

    pub(crate) fn list(&self, index: usize) -> Option<&ModeList> {
        self.levels.get(index).map(ModeLevelSummary::list)
    }

    pub(crate) fn list_mut(&mut self, index: usize) -> Option<&mut ModeList> {
        Arc::make_mut(&mut self.levels)
            .get_mut(index)
            .map(ModeLevelSummary::list_mut)
    }

    #[must_use]
    pub fn enclosing_vertical_prev_graf(&self) -> i32 {
        let index = self.enclosing_vertical_index();
        self.levels[index].list().prev_graf()
    }

    pub fn set_enclosing_vertical_prev_graf(&mut self, lines: i32) {
        let index = self.enclosing_vertical_index();
        Arc::make_mut(&mut self.levels)[index]
            .list_mut()
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
