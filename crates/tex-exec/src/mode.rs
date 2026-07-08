use tex_expand::EngineMode;
use tex_state::ids::FontId;
use tex_state::ids::GlueId;
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
    pub hang_indent: Scaled,
    pub hang_after: i32,
    pub looseness: i32,
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
    prev_depth: Option<Scaled>,
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

    pub fn pop_box(&mut self) -> Option<Node> {
        let pos = self
            .nodes
            .iter()
            .rposition(|node| matches!(node, Node::HList(_) | Node::VList(_)))?;
        Some(self.nodes.remove(pos))
    }

    pub fn pop_last_node(&mut self) -> Option<Node> {
        self.nodes.pop()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PendingHChar {
    pub font: FontId,
    pub ch: char,
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
        self.levels.push(ModeLevelSummary::new(mode));
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
}
