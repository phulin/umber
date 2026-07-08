use tex_expand::EngineMode;

use crate::ExecError;

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

/// Placeholder for the list-under-construction owned by each nest level.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum ListBuilderSummary {
    #[default]
    Empty,
}

/// Snapshot-summary state for one mode level.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ModeLevelSummary {
    mode: Mode,
    list: ListBuilderSummary,
}

impl ModeLevelSummary {
    #[must_use]
    pub const fn new(mode: Mode) -> Self {
        Self {
            mode,
            list: ListBuilderSummary::Empty,
        }
    }

    #[must_use]
    pub const fn mode(self) -> Mode {
        self.mode
    }

    #[must_use]
    pub const fn list(self) -> ListBuilderSummary {
        self.list
    }
}

/// Snapshot-coverable summary of the whole mode nest.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
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
#[derive(Clone, Debug, Eq, PartialEq)]
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
}
