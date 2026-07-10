use tex_expand::EngineMode;
use tex_state::ids::FontId;
use tex_state::ids::GlueId;
use tex_state::ids::NodeListId;
use tex_state::ids::TokenListId;
use tex_state::math::FractionThickness;
use tex_state::node::Node;
use tex_state::scaled::Scaled;

use crate::ExecError;

/// TeX's sentinel depth used before any vertical-list box has established a baseline.
pub const IGNORE_DEPTH: Scaled = Scaled::from_raw(-65_536_000);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParagraphShape {
    lines: Vec<ParagraphShapeLine>,
}

impl ParagraphShape {
    #[must_use]
    pub fn new(lines: Vec<ParagraphShapeLine>) -> Self {
        Self { lines }
    }

    #[must_use]
    pub fn lines(&self) -> &[ParagraphShapeLine] {
        &self.lines
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParagraphShapeLine {
    pub indent: Scaled,
    pub width: Scaled,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParagraphParams {
    pub left_skip: GlueId,
    pub right_skip: GlueId,
    pub par_fill_skip: GlueId,
    pub par_shape: Option<ParagraphShape>,
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
    pub emergency_stretch: Scaled,
    pub hsize: Scaled,
    pub interline_penalty: i32,
    pub club_penalty: i32,
    pub widow_penalty: i32,
    pub broken_penalty: i32,
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
    nodes: Vec<Node>,
    align_state: Option<AlignState>,
    incomplete_fraction: Option<IncompleteFraction>,
    display_interrupt: Option<DisplayInterrupt>,
    display_eq_no: Option<DisplayEqNo>,
    prev_depth: Option<Scaled>,
    prev_graf: i32,
    par_shape: Option<ParagraphShape>,
    pending_hchars: Vec<PendingHChar>,
    space_factor: i32,
    no_boundary: bool,
}

impl ModeList {
    #[must_use]
    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    pub fn take_nodes(&mut self) -> Vec<Node> {
        std::mem::take(&mut self.nodes)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn push(&mut self, node: Node) {
        self.nodes.push(node);
    }

    pub fn append(&mut self, nodes: impl IntoIterator<Item = Node>) {
        self.nodes.extend(nodes);
    }

    pub fn push_pending_hchar(&mut self, font: FontId, ch: char) {
        self.pending_hchars.push(PendingHChar { font, ch });
    }

    pub fn take_pending_hchars(&mut self) -> Vec<PendingHChar> {
        std::mem::take(&mut self.pending_hchars)
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

    pub fn set_par_shape(&mut self, shape: ParagraphShape) {
        self.par_shape = Some(shape);
    }

    #[must_use]
    pub fn par_shape(&self) -> Option<&ParagraphShape> {
        self.par_shape.as_ref()
    }

    pub fn reset_par_shape(&mut self) {
        self.par_shape = None;
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
        let mut node = self.nodes.pop().expect("tail was just inspected");
        match &mut node {
            Node::HList(box_node) | Node::VList(box_node) => {
                box_node.shift = Scaled::from_raw(0);
            }
            _ => unreachable!("tail was checked to be a box"),
        }
        Some(node)
    }

    pub fn pop_last_node(&mut self) -> Option<Node> {
        self.nodes.pop()
    }

    pub fn last_node_mut(&mut self) -> Option<&mut Node> {
        self.nodes.last_mut()
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
        self.tabskips
            .get(boundary)
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

#[derive(Clone, Debug, PartialEq)]
pub struct IncompleteFraction {
    pub numerator: NodeListId,
    pub thickness: FractionThickness,
    pub left_delimiter: Option<u32>,
    pub right_delimiter: Option<u32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DisplayInterrupt {
    pub pre_display_size: Scaled,
    pub display_width: Scaled,
    pub display_indent: Scaled,
    pub saved_pre_display_size: Scaled,
    pub saved_display_width: Scaled,
    pub saved_display_indent: Scaled,
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
    levels: Vec<ModeLevelSummary>,
}

impl ModeNestSummary {
    #[must_use]
    pub fn levels(&self) -> &[ModeLevelSummary] {
        &self.levels
    }
}

/// Explicit stack of TeX mode levels.
#[derive(Clone, Debug, PartialEq)]
pub struct ModeNest {
    levels: Vec<ModeLevelSummary>,
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
            levels: vec![ModeLevelSummary::new(Mode::Vertical)],
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
        self.levels.push(level);
    }

    pub fn pop(&mut self) -> Result<ModeLevelSummary, ExecError> {
        if self.levels.len() == 1 {
            return Err(ExecError::CannotPopBaseMode);
        }
        Ok(self
            .levels
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
        self.levels
            .last_mut()
            .expect("ModeNest always has at least one level")
            .list_mut()
    }

    pub(crate) fn list(&self, index: usize) -> Option<&ModeList> {
        self.levels.get(index).map(ModeLevelSummary::list)
    }

    pub(crate) fn list_mut(&mut self, index: usize) -> Option<&mut ModeList> {
        self.levels.get_mut(index).map(ModeLevelSummary::list_mut)
    }

    #[must_use]
    pub fn enclosing_vertical_prev_graf(&self) -> i32 {
        let index = self.enclosing_vertical_index();
        self.levels[index].list().prev_graf()
    }

    pub fn set_enclosing_vertical_prev_graf(&mut self, lines: i32) {
        let index = self.enclosing_vertical_index();
        self.levels[index].list_mut().set_prev_graf(lines);
    }

    fn enclosing_vertical_index(&self) -> usize {
        self.levels
            .iter()
            .rposition(|level| matches!(level.mode(), Mode::Vertical | Mode::InternalVertical))
            .expect("base vertical level is always present")
    }
}
