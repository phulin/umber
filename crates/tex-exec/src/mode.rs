use tex_expand::EngineMode;
use tex_state::node::Node;
use tex_state::scaled::Scaled;

use crate::ExecError;

/// TeX's sentinel depth used before any vertical-list box has established a baseline.
pub const IGNORE_DEPTH: Scaled = Scaled::from_raw(-65_536_000);

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

    #[must_use]
    pub const fn prev_depth(&self) -> Option<Scaled> {
        self.prev_depth
    }

    pub fn set_prev_depth(&mut self, depth: Scaled) {
        self.prev_depth = Some(depth);
    }

    pub fn pop_box(&mut self) -> Option<Node> {
        let pos = self
            .nodes
            .iter()
            .rposition(|node| matches!(node, Node::HList(_) | Node::VList(_)))?;
        Some(self.nodes.remove(pos))
    }
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
