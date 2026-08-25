//! Snapshot-owned page-builder state.

mod sequence;
#[cfg(test)]
mod state_hash;

use crate::glue::GlueSpec;
use crate::node::{Node, NodeTokenList};
use crate::scaled::Scaled;
use sequence::PageNodeSequence;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

/// TeX's `awful_bad` sentinel, `2^30 - 1`.
pub const AWFUL_BAD: i32 = 0o7777777777;

/// TeX's infinite penalty threshold.
pub const INF_PENALTY: i32 = 10_000;

/// TeX's forced-eject penalty threshold.
pub const EJECT_PENALTY: i32 = -INF_PENALTY;

/// TeX.web's page-break cost for infinitely bad, non-awful breaks.
pub const DEPLORABLE: i32 = 100_000;

/// One of TeX's user-visible `page_so_far` dimensions.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PageDimension {
    Goal,
    Total,
    Stretch,
    FilStretch,
    FillStretch,
    FilllStretch,
    Shrink,
    Depth,
}

impl PageDimension {
    /// Returns the TeX.web `page_so_far` index.
    #[must_use]
    pub const fn index(self) -> u8 {
        match self {
            Self::Goal => 0,
            Self::Total => 1,
            Self::Stretch => 2,
            Self::FilStretch => 3,
            Self::FillStretch => 4,
            Self::FilllStretch => 5,
            Self::Shrink => 6,
            Self::Depth => 7,
        }
    }

    /// Decodes a TeX.web `page_so_far` index.
    #[must_use]
    pub const fn from_index(index: u8) -> Option<Self> {
        match index {
            0 => Some(Self::Goal),
            1 => Some(Self::Total),
            2 => Some(Self::Stretch),
            3 => Some(Self::FilStretch),
            4 => Some(Self::FillStretch),
            5 => Some(Self::FilllStretch),
            6 => Some(Self::Shrink),
            7 => Some(Self::Depth),
            _ => None,
        }
    }
}

/// Page-builder integer quantities that are not Env integer parameters.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PageInteger {
    DeadCycles,
    InsertPenalties,
}

/// TeX82's single mark-class page mark slots.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PageMark {
    Top,
    First,
    Bot,
    SplitFirst,
    SplitBot,
}

impl PageMark {
    #[must_use]
    pub const fn index(self) -> u8 {
        match self {
            Self::Top => 0,
            Self::First => 1,
            Self::Bot => 2,
            Self::SplitFirst => 3,
            Self::SplitBot => 4,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct MarkClassState {
    marks: [Option<NodeTokenList>; 5],
}

impl Default for MarkClassState {
    fn default() -> Self {
        Self {
            marks: core::array::from_fn(|_| None),
        }
    }
}

impl MarkClassState {
    fn get(&self, mark: PageMark) -> Option<&NodeTokenList> {
        self.marks[usize::from(mark.index())].as_ref()
    }

    fn set(&mut self, mark: PageMark, value: NodeTokenList) {
        self.marks[usize::from(mark.index())] = Some(value);
    }

    fn clear(&mut self, mark: PageMark) {
        self.marks[usize::from(mark.index())] = None;
    }

    fn is_empty(&self) -> bool {
        self.marks.iter().all(Option::is_none)
    }
}

impl PageInteger {
    /// Returns the TeX.web `set_page_int` selector.
    #[must_use]
    pub const fn index(self) -> u8 {
        match self {
            Self::DeadCycles => 0,
            Self::InsertPenalties => 1,
        }
    }

    /// Decodes a TeX.web `set_page_int` selector.
    #[must_use]
    pub const fn from_index(index: u8) -> Option<Self> {
        match index {
            0 => Some(Self::DeadCycles),
            1 => Some(Self::InsertPenalties),
            _ => None,
        }
    }
}

/// The page contents state machine from TeX.web.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum PageContents {
    #[default]
    Empty,
    InsertsOnly,
    BoxThere,
}

impl PageContents {
    #[must_use]
    pub const fn has_box(self) -> bool {
        matches!(self, Self::BoxThere)
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        matches!(self, Self::Empty)
    }
}

/// A recorded best page break.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct PageBreak {
    index: usize,
}

impl PageBreak {
    #[must_use]
    pub const fn new(index: usize) -> Self {
        Self { index }
    }

    #[must_use]
    pub const fn index(self) -> usize {
        self.index
    }
}

/// A pending call to the future output-routine fire-up implementation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct PageFireUp {
    best_break: PageBreak,
    best_size: Scaled,
    trigger: PageBreak,
}

impl PageFireUp {
    #[must_use]
    pub const fn new(best_break: PageBreak, best_size: Scaled, trigger: PageBreak) -> Self {
        Self {
            best_break,
            best_size,
            trigger,
        }
    }

    #[must_use]
    pub const fn best_break(self) -> PageBreak {
        self.best_break
    }

    #[must_use]
    pub const fn best_size(self) -> Scaled {
        self.best_size
    }

    #[must_use]
    pub const fn trigger(self) -> PageBreak {
        self.trigger
    }
}

/// Per-class insertion status while the current page is being built.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum PageInsertionStatus {
    Inserting,
    SplitUp {
        broken_ins_index: usize,
        broken_at: Option<usize>,
    },
}

/// TeX.web page insertion record for one insertion class.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct PageInsertion {
    class: u16,
    status: PageInsertionStatus,
    height: Scaled,
    last_ins_index: Option<usize>,
    best_ins_index: Option<usize>,
}

impl PageInsertion {
    #[must_use]
    pub const fn new(class: u16, height: Scaled) -> Self {
        Self {
            class,
            status: PageInsertionStatus::Inserting,
            height,
            last_ins_index: None,
            best_ins_index: None,
        }
    }

    #[must_use]
    pub const fn class(&self) -> u16 {
        self.class
    }

    #[must_use]
    pub const fn status(&self) -> PageInsertionStatus {
        self.status
    }

    pub fn set_status(&mut self, status: PageInsertionStatus) {
        self.status = status;
    }

    #[must_use]
    pub const fn height(&self) -> Scaled {
        self.height
    }

    pub fn set_height(&mut self, height: Scaled) {
        self.height = height;
    }

    #[must_use]
    pub const fn last_ins_index(&self) -> Option<usize> {
        self.last_ins_index
    }

    pub fn set_last_ins_index(&mut self, index: Option<usize>) {
        self.last_ins_index = index;
    }

    #[must_use]
    pub const fn best_ins_index(&self) -> Option<usize> {
        self.best_ins_index
    }
}

/// Snapshot-owned state for TeX.web's page builder.
#[derive(Clone, Debug)]
pub(crate) struct PageBuilderState {
    contribution: VecDeque<Node>,
    current_page: PageNodeSequence,
    page_discards: Vec<Node>,
    split_discards: Vec<Node>,
    page_goal: Scaled,
    page_total: Scaled,
    page_stretch: Scaled,
    page_fil_stretch: Scaled,
    page_fill_stretch: Scaled,
    page_filll_stretch: Scaled,
    page_shrink: Scaled,
    page_depth: Scaled,
    page_max_depth: Scaled,
    contents: PageContents,
    last_glue: Option<GlueSpec>,
    last_penalty: i32,
    last_kern: Scaled,
    last_node_type: i32,
    insert_penalties: i32,
    dead_cycles: i32,
    least_page_cost: i32,
    best_page_break: Option<PageBreak>,
    best_size: Scaled,
    fire_up: Option<PageFireUp>,
    insertions: Vec<PageInsertion>,
    insertion_positions: Vec<Option<u16>>,
    top_mark: Option<NodeTokenList>,
    first_mark: Option<NodeTokenList>,
    bot_mark: Option<NodeTokenList>,
    split_first_mark: Option<NodeTokenList>,
    split_bot_mark: Option<NodeTokenList>,
    mark_classes: Vec<(u16, MarkClassState)>,
    mark_class_positions: Vec<Option<u16>>,
    tex82_dynamic_words: usize,
    etex_dynamic_words: usize,
}

/// Handle-free scalar half of a detached page-builder transition. Node, glue,
/// token-list, and font handles travel in one ordinary detached node graph.
#[cfg(test)]
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct PageMemoState {
    pub(crate) contribution_len: usize,
    pub(crate) current_page_len: usize,
    pub(crate) page_discards_len: usize,
    pub(crate) split_discards_len: usize,
    pub(crate) has_last_glue: bool,
    pub(crate) dimensions: [Scaled; 8],
    pub(crate) page_max_depth: Scaled,
    pub(crate) contents: u8,
    pub(crate) last_penalty: i32,
    pub(crate) last_kern: Scaled,
    pub(crate) last_node_type: i32,
    pub(crate) insert_penalties: i32,
    pub(crate) dead_cycles: i32,
    pub(crate) least_page_cost: i32,
    pub(crate) best_page_break: Option<PageBreak>,
    pub(crate) best_size: Scaled,
    pub(crate) fire_up: Option<PageFireUp>,
    pub(crate) insertions: Vec<PageInsertion>,
    pub(crate) insertion_positions: Vec<Option<u16>>,
}

impl Default for PageBuilderState {
    fn default() -> Self {
        Self {
            contribution: VecDeque::new(),
            current_page: PageNodeSequence::default(),
            page_discards: Vec::new(),
            split_discards: Vec::new(),
            page_goal: Scaled::from_raw(0),
            page_total: Scaled::from_raw(0),
            page_stretch: Scaled::from_raw(0),
            page_fil_stretch: Scaled::from_raw(0),
            page_fill_stretch: Scaled::from_raw(0),
            page_filll_stretch: Scaled::from_raw(0),
            page_shrink: Scaled::from_raw(0),
            page_depth: Scaled::from_raw(0),
            page_max_depth: Scaled::from_raw(0),
            contents: PageContents::Empty,
            last_glue: None,
            last_penalty: 0,
            last_kern: Scaled::from_raw(0),
            last_node_type: -1,
            insert_penalties: 0,
            dead_cycles: 0,
            least_page_cost: AWFUL_BAD,
            best_page_break: None,
            best_size: Scaled::from_raw(0),
            fire_up: None,
            insertions: Vec::new(),
            insertion_positions: Vec::new(),
            top_mark: None,
            first_mark: None,
            bot_mark: None,
            split_first_mark: None,
            split_bot_mark: None,
            mark_classes: Vec::new(),
            mark_class_positions: Vec::new(),
            tex82_dynamic_words: 0,
            etex_dynamic_words: 0,
        }
    }
}

impl PageBuilderState {
    /// Whether this checkpointable page state explicitly carries any live
    /// page-arena coordinate. A rootless state contributes no retained-prefix
    /// demand merely because the arena cursor has advanced.
    pub(crate) fn retains_page_node_handles(&self) -> bool {
        self.contribution
            .iter()
            .chain(self.current_page.iter())
            .chain(self.page_discards.iter())
            .chain(self.split_discards.iter())
            .any(node_retains_page_handle)
    }

    pub(crate) fn dynamic_memory_words(&self, etex_node_sizes: bool) -> usize {
        if etex_node_sizes {
            self.etex_dynamic_words
        } else {
            self.tex82_dynamic_words
        }
    }

    pub(crate) fn font_roots_are_live(
        &self,
        mut is_live: impl FnMut(crate::ids::FontId) -> bool,
    ) -> bool {
        self.contribution
            .iter()
            .chain(self.current_page.iter())
            .chain(self.page_discards.iter())
            .chain(self.split_discards.iter())
            .all(|node| {
                let mut live = true;
                node.visit_fonts(|font| live &= is_live(font));
                live
            })
    }

    #[cfg(test)]
    pub(crate) fn memo_parts(&self) -> (Vec<Node>, PageMemoState) {
        let mut nodes = Vec::with_capacity(
            self.contribution.len()
                + self.current_page.len()
                + self.page_discards.len()
                + self.split_discards.len()
                + usize::from(self.last_glue.is_some()),
        );
        nodes.extend(self.contribution.iter().cloned());
        nodes.extend(self.current_page.iter().cloned());
        nodes.extend(self.page_discards.iter().cloned());
        nodes.extend(self.split_discards.iter().cloned());
        if let Some(spec) = &self.last_glue {
            nodes.push(Node::Glue {
                spec: *spec,
                kind: crate::node::GlueKind::Normal,
                leader: None,
            });
        }
        let dimensions = [
            self.page_goal,
            self.page_total,
            self.page_stretch,
            self.page_fil_stretch,
            self.page_fill_stretch,
            self.page_filll_stretch,
            self.page_shrink,
            self.page_depth,
        ];
        let state = PageMemoState {
            contribution_len: self.contribution.len(),
            current_page_len: self.current_page.len(),
            page_discards_len: self.page_discards.len(),
            split_discards_len: self.split_discards.len(),
            has_last_glue: self.last_glue.is_some(),
            dimensions,
            page_max_depth: self.page_max_depth,
            contents: match self.contents {
                PageContents::Empty => 0,
                PageContents::InsertsOnly => 1,
                PageContents::BoxThere => 2,
            },
            last_penalty: self.last_penalty,
            last_kern: self.last_kern,
            last_node_type: self.last_node_type,
            insert_penalties: self.insert_penalties,
            dead_cycles: self.dead_cycles,
            least_page_cost: self.least_page_cost,
            best_page_break: self.best_page_break,
            best_size: self.best_size,
            fire_up: self.fire_up,
            insertions: self.insertions.clone(),
            insertion_positions: self.insertion_positions.clone(),
        };
        (nodes, state)
    }

    #[cfg(test)]
    pub(crate) fn install_memo_parts(
        &mut self,
        nodes: Vec<Node>,
        state: PageMemoState,
    ) -> Result<(), crate::MemoValueError> {
        let ordinary_len = state
            .contribution_len
            .checked_add(state.current_page_len)
            .and_then(|len| len.checked_add(state.page_discards_len))
            .and_then(|len| len.checked_add(state.split_discards_len))
            .ok_or(crate::MemoValueError::Invalid(
                "page transition lengths overflow",
            ))?;
        let expected_len = ordinary_len
            .checked_add(usize::from(state.has_last_glue))
            .ok_or(crate::MemoValueError::Invalid(
                "page transition lengths overflow",
            ))?;
        if nodes.len() != expected_len {
            return Err(crate::MemoValueError::Invalid(
                "page transition node lengths do not match",
            ));
        }
        let contents = match state.contents {
            0 => PageContents::Empty,
            1 => PageContents::InsertsOnly,
            2 => PageContents::BoxThere,
            _ => {
                return Err(crate::MemoValueError::Invalid(
                    "invalid page contents state",
                ));
            }
        };
        let mut cursor = 0;
        let take = |cursor: &mut usize, len: usize| {
            let start = *cursor;
            *cursor += len;
            start..*cursor
        };
        let contribution =
            VecDeque::from(nodes[take(&mut cursor, state.contribution_len)].to_vec());
        let current_nodes = nodes[take(&mut cursor, state.current_page_len)].to_vec();
        let page_discards = nodes[take(&mut cursor, state.page_discards_len)].to_vec();
        let split_discards = nodes[take(&mut cursor, state.split_discards_len)].to_vec();
        let last_glue = if state.has_last_glue {
            match &nodes[cursor] {
                Node::Glue { spec, .. } => Some(*spec),
                _ => return Err(crate::MemoValueError::Invalid("invalid last-glue sentinel")),
            }
        } else {
            None
        };
        let mut current_page = PageNodeSequence::default();
        for node in current_nodes {
            current_page.push(node);
        }
        self.contribution = contribution;
        self.current_page = current_page;
        self.page_discards = page_discards;
        self.split_discards = split_discards;
        self.refresh_dynamic_memory_words();
        self.page_goal = state.dimensions[0];
        self.page_total = state.dimensions[1];
        self.page_stretch = state.dimensions[2];
        self.page_fil_stretch = state.dimensions[3];
        self.page_fill_stretch = state.dimensions[4];
        self.page_filll_stretch = state.dimensions[5];
        self.page_shrink = state.dimensions[6];
        self.page_depth = state.dimensions[7];
        self.page_max_depth = state.page_max_depth;
        self.contents = contents;
        self.last_glue = last_glue;
        self.last_penalty = state.last_penalty;
        self.last_kern = state.last_kern;
        self.last_node_type = state.last_node_type;
        self.insert_penalties = state.insert_penalties;
        self.dead_cycles = state.dead_cycles;
        self.least_page_cost = state.least_page_cost;
        self.best_page_break = state.best_page_break;
        self.best_size = state.best_size;
        self.fire_up = state.fire_up;
        self.insertions = state.insertions;
        self.insertion_positions = state.insertion_positions;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            .saturating_add(
                self.contribution
                    .capacity()
                    .saturating_mul(std::mem::size_of::<Node>()),
            )
            .saturating_add(self.current_page.retained_bytes())
            .saturating_add(
                self.page_discards
                    .capacity()
                    .saturating_mul(std::mem::size_of::<Node>()),
            )
            .saturating_add(
                self.split_discards
                    .capacity()
                    .saturating_mul(std::mem::size_of::<Node>()),
            )
            .saturating_add(
                self.insertions
                    .capacity()
                    .saturating_mul(std::mem::size_of::<PageInsertion>()),
            )
            .saturating_add(
                self.insertion_positions
                    .capacity()
                    .saturating_mul(std::mem::size_of::<Option<u16>>()),
            )
            .saturating_add(
                self.mark_classes
                    .capacity()
                    .saturating_mul(std::mem::size_of::<(u16, MarkClassState)>()),
            )
            .saturating_add(
                self.mark_class_positions
                    .capacity()
                    .saturating_mul(std::mem::size_of::<Option<u16>>()),
            )
    }

    pub(crate) fn is_format_empty(&self) -> bool {
        self.contribution.is_empty()
            && self.current_page.len() == 0
            && self.page_goal == Scaled::from_raw(0)
            && self.page_total == Scaled::from_raw(0)
            && self.page_stretch == Scaled::from_raw(0)
            && self.page_fil_stretch == Scaled::from_raw(0)
            && self.page_fill_stretch == Scaled::from_raw(0)
            && self.page_filll_stretch == Scaled::from_raw(0)
            && self.page_shrink == Scaled::from_raw(0)
            && self.page_depth == Scaled::from_raw(0)
            && self.page_max_depth == Scaled::from_raw(0)
            && self.contents == PageContents::Empty
            && self.last_glue.is_none()
            && self.last_penalty == 0
            && self.last_kern == Scaled::from_raw(0)
            && self.last_node_type == -1
            && self.insert_penalties == 0
            && self.dead_cycles == 0
            && self.least_page_cost == AWFUL_BAD
            && self.best_page_break.is_none()
            && self.best_size == Scaled::from_raw(0)
            && self.fire_up.is_none()
            && self.insertions.is_empty()
            && self.insertion_positions.is_empty()
            && self.top_mark.is_none()
            && self.first_mark.is_none()
            && self.bot_mark.is_none()
            && self.split_first_mark.is_none()
            && self.split_bot_mark.is_none()
            && self.mark_classes.is_empty()
            && self.mark_class_positions.is_empty()
    }

    pub(crate) fn dimension(
        &self,
        dimension: PageDimension,
        output_routine_active: bool,
    ) -> Scaled {
        if self.contents.is_empty() && !output_routine_active {
            return match dimension {
                PageDimension::Goal => Scaled::MAX_DIMEN,
                _ => Scaled::from_raw(0),
            };
        }
        self.raw_dimension(dimension)
    }

    pub(crate) const fn raw_dimension(&self, dimension: PageDimension) -> Scaled {
        match dimension {
            PageDimension::Goal => self.page_goal,
            PageDimension::Total => self.page_total,
            PageDimension::Stretch => self.page_stretch,
            PageDimension::FilStretch => self.page_fil_stretch,
            PageDimension::FillStretch => self.page_fill_stretch,
            PageDimension::FilllStretch => self.page_filll_stretch,
            PageDimension::Shrink => self.page_shrink,
            PageDimension::Depth => self.page_depth,
        }
    }

    pub(crate) fn set_dimension(&mut self, dimension: PageDimension, value: Scaled) {
        match dimension {
            PageDimension::Goal => self.page_goal = value,
            PageDimension::Total => self.page_total = value,
            PageDimension::Stretch => self.page_stretch = value,
            PageDimension::FilStretch => self.page_fil_stretch = value,
            PageDimension::FillStretch => self.page_fill_stretch = value,
            PageDimension::FilllStretch => self.page_filll_stretch = value,
            PageDimension::Shrink => self.page_shrink = value,
            PageDimension::Depth => self.page_depth = value,
        }
    }

    pub(crate) const fn integer(&self, integer: PageInteger) -> i32 {
        match integer {
            PageInteger::DeadCycles => self.dead_cycles,
            PageInteger::InsertPenalties => self.insert_penalties,
        }
    }

    pub(crate) fn set_integer(&mut self, integer: PageInteger, value: i32) {
        match integer {
            PageInteger::DeadCycles => self.dead_cycles = value,
            PageInteger::InsertPenalties => self.insert_penalties = value,
        }
    }

    pub(crate) fn mark(&self, mark: PageMark) -> NodeTokenList {
        self.mark_root(mark).cloned().unwrap_or_default()
    }

    pub(crate) fn mark_value(&self, mark: PageMark) -> Option<&NodeTokenList> {
        self.mark_root(mark)
    }

    pub(crate) fn mark_root(&self, mark: PageMark) -> Option<&NodeTokenList> {
        match mark {
            PageMark::Top => self.top_mark.as_ref(),
            PageMark::First => self.first_mark.as_ref(),
            PageMark::Bot => self.bot_mark.as_ref(),
            PageMark::SplitFirst => self.split_first_mark.as_ref(),
            PageMark::SplitBot => self.split_bot_mark.as_ref(),
        }
    }

    pub(crate) fn set_mark(&mut self, mark: PageMark, value: NodeTokenList) {
        *match mark {
            PageMark::Top => &mut self.top_mark,
            PageMark::First => &mut self.first_mark,
            PageMark::Bot => &mut self.bot_mark,
            PageMark::SplitFirst => &mut self.split_first_mark,
            PageMark::SplitBot => &mut self.split_bot_mark,
        } = Some(value);
    }

    pub(crate) fn clear_mark(&mut self, mark: PageMark) {
        *match mark {
            PageMark::Top => &mut self.top_mark,
            PageMark::First => &mut self.first_mark,
            PageMark::Bot => &mut self.bot_mark,
            PageMark::SplitFirst => &mut self.split_first_mark,
            PageMark::SplitBot => &mut self.split_bot_mark,
        } = None;
    }

    #[cfg(test)]
    pub(crate) fn mark_class(&self, mark: PageMark, class: u16) -> NodeTokenList {
        self.mark_class_value(mark, class)
            .cloned()
            .unwrap_or_default()
    }

    pub(crate) fn mark_class_value(&self, mark: PageMark, class: u16) -> Option<&NodeTokenList> {
        if class == 0 {
            return self.mark_value(mark);
        }
        self.mark_class_position(class)
            .and_then(|position| self.mark_classes[position].1.get(mark))
    }

    pub(crate) fn set_mark_class(&mut self, mark: PageMark, class: u16, value: NodeTokenList) {
        if class == 0 {
            self.set_mark(mark, value);
            return;
        }
        self.ensure_mark_class(class).set(mark, value);
    }

    pub(crate) fn clear_mark_class(&mut self, mark: PageMark, class: u16) {
        if class == 0 {
            self.clear_mark(mark);
            return;
        }
        let Some(position) = self.mark_class_position(class) else {
            return;
        };
        self.mark_classes[position].1.clear(mark);
        if self.mark_classes[position].1.is_empty() {
            self.remove_mark_class(class, position);
        }
    }

    pub(crate) fn mark_class_ids(&self) -> impl Iterator<Item = u16> + '_ {
        self.mark_classes.iter().map(|(class, _)| *class)
    }

    fn mark_class_position(&self, class: u16) -> Option<usize> {
        self.mark_class_positions
            .get(usize::from(class))
            .copied()
            .flatten()
            .map(usize::from)
    }

    fn ensure_mark_class(&mut self, class: u16) -> &mut MarkClassState {
        let class_index = usize::from(class);
        if self.mark_class_positions.len() <= class_index {
            self.mark_class_positions.resize(class_index + 1, None);
        }
        if let Some(position) = self.mark_class_positions[class_index] {
            return &mut self.mark_classes[usize::from(position)].1;
        }

        let position = self
            .mark_classes
            .iter()
            .position(|(active, _)| *active > class)
            .unwrap_or(self.mark_classes.len());
        self.mark_classes
            .insert(position, (class, MarkClassState::default()));
        for active in self.mark_class_positions.iter_mut().flatten() {
            if usize::from(*active) >= position {
                *active = active
                    .checked_add(1)
                    .expect("active mark-class count fits u16");
            }
        }
        self.mark_class_positions[class_index] = Some(
            u16::try_from(position).expect("active mark-class count fits the e-TeX register space"),
        );
        &mut self.mark_classes[position].1
    }

    fn remove_mark_class(&mut self, class: u16, position: usize) {
        self.mark_classes.remove(position);
        self.mark_class_positions[usize::from(class)] = None;
        for active in self.mark_class_positions.iter_mut().flatten() {
            if usize::from(*active) > position {
                *active -= 1;
            }
        }
    }

    pub(crate) fn freeze_specs(
        &mut self,
        contents: PageContents,
        vsize: Scaled,
        max_depth: Scaled,
    ) {
        self.contents = contents;
        self.page_goal = vsize;
        self.page_max_depth = max_depth;
        self.page_depth = Scaled::from_raw(0);
        self.page_total = Scaled::from_raw(0);
        self.page_stretch = Scaled::from_raw(0);
        self.page_fil_stretch = Scaled::from_raw(0);
        self.page_fill_stretch = Scaled::from_raw(0);
        self.page_filll_stretch = Scaled::from_raw(0);
        self.page_shrink = Scaled::from_raw(0);
        self.least_page_cost = AWFUL_BAD;
        self.best_page_break = None;
        self.best_size = Scaled::from_raw(0);
        self.insertions.clear();
        self.insertion_positions.clear();
    }

    pub(crate) fn start_new_page(&mut self) {
        self.start_page_after_output();
        self.page_goal = Scaled::from_raw(0);
        self.page_total = Scaled::from_raw(0);
        self.page_stretch = Scaled::from_raw(0);
        self.page_fil_stretch = Scaled::from_raw(0);
        self.page_fill_stretch = Scaled::from_raw(0);
        self.page_filll_stretch = Scaled::from_raw(0);
        self.page_shrink = Scaled::from_raw(0);
    }

    /// TeX82 §1012's reset after `fire_up`: the page list and builder
    /// controls are empty, while `page_so_far` remains observable until §991
    /// freezes the next page's specifications.
    pub(crate) fn start_page_after_output(&mut self) {
        let released = dynamic_words(self.current_page.iter());
        self.release_dynamic_word_totals(released);
        self.current_page.clear();
        self.contents = PageContents::Empty;
        self.last_glue = None;
        self.last_penalty = 0;
        self.last_kern = Scaled::from_raw(0);
        self.last_node_type = -1;
        self.page_depth = Scaled::from_raw(0);
        self.page_max_depth = Scaled::from_raw(0);
        self.insert_penalties = 0;
        self.least_page_cost = AWFUL_BAD;
        self.best_page_break = None;
        self.best_size = Scaled::from_raw(0);
        self.fire_up = None;
        self.insertions.clear();
        self.insertion_positions.clear();
    }

    pub(crate) const fn contents(&self) -> PageContents {
        self.contents
    }

    pub(crate) fn set_contents(&mut self, contents: PageContents) {
        self.contents = contents;
    }

    pub(crate) const fn page_max_depth(&self) -> Scaled {
        self.page_max_depth
    }

    pub(crate) const fn insert_penalties(&self) -> i32 {
        self.insert_penalties
    }

    pub(crate) const fn least_page_cost(&self) -> i32 {
        self.least_page_cost
    }

    pub(crate) fn record_best_break(&mut self, break_index: usize, best_size: Scaled, cost: i32) {
        self.best_page_break = Some(PageBreak::new(break_index));
        self.best_size = best_size;
        self.least_page_cost = cost;
        for insertion in &mut self.insertions {
            insertion.best_ins_index = insertion.last_ins_index;
        }
    }

    pub(crate) fn record_fire_up(&mut self, trigger_index: usize) {
        let best_break = self
            .best_page_break
            .unwrap_or_else(|| PageBreak::new(trigger_index));
        self.fire_up = Some(PageFireUp::new(
            best_break,
            self.best_size,
            PageBreak::new(trigger_index),
        ));
    }

    pub(crate) const fn fire_up(&self) -> Option<PageFireUp> {
        self.fire_up
    }

    pub(crate) fn push_contribution(&mut self, node: Node) {
        self.allocate_dynamic_node(&node);
        self.contribution.push_back(node);
    }

    pub(crate) fn remove_contribution_range(
        &mut self,
        range: std::ops::RangeInclusive<usize>,
    ) -> Vec<Node> {
        let removed = self.contribution.drain(range).collect::<Vec<_>>();
        self.release_dynamic_nodes(&removed);
        removed
    }

    pub(crate) fn prepend_contribution(&mut self, node: Node) {
        self.allocate_dynamic_node(&node);
        self.contribution.push_front(node);
    }

    pub(crate) fn contribution(&self) -> &VecDeque<Node> {
        &self.contribution
    }

    pub(crate) fn contribution_front(&self) -> Option<&Node> {
        self.contribution.front()
    }

    pub(crate) fn contribution_second(&self) -> Option<&Node> {
        self.contribution.get(1)
    }

    pub(crate) fn pop_contribution_front(&mut self) -> Option<Node> {
        let node = self.contribution.pop_front()?;
        self.release_dynamic_node(&node);
        Some(node)
    }

    pub(crate) fn prepend_contributions(&mut self, nodes: Vec<Node>) {
        if nodes.is_empty() {
            return;
        }
        self.allocate_dynamic_nodes(&nodes);
        let mut queue = VecDeque::with_capacity(nodes.len() + self.contribution.len());
        queue.extend(nodes);
        queue.extend(self.contribution.iter().cloned());
        self.contribution = queue;
    }

    pub(crate) fn current_page(&self) -> impl DoubleEndedIterator<Item = &Node> {
        self.current_page.iter()
    }

    pub(crate) fn push_page_discard(&mut self, node: Node) {
        self.allocate_dynamic_node(&node);
        self.page_discards.push(node);
    }

    pub(crate) fn take_page_discards(&mut self) -> Vec<Node> {
        let nodes = std::mem::take(&mut self.page_discards);
        self.release_dynamic_nodes(&nodes);
        nodes
    }

    pub(crate) fn clear_page_discards(&mut self) {
        let nodes = std::mem::take(&mut self.page_discards);
        self.release_dynamic_nodes(&nodes);
    }

    pub(crate) fn set_split_discards(&mut self, nodes: Vec<Node>) {
        let added = dynamic_words(nodes.iter());
        let old = std::mem::replace(&mut self.split_discards, nodes);
        self.release_dynamic_nodes(&old);
        self.allocate_dynamic_word_totals(added);
    }

    pub(crate) fn take_split_discards(&mut self) -> Vec<Node> {
        let nodes = std::mem::take(&mut self.split_discards);
        self.release_dynamic_nodes(&nodes);
        nodes
    }

    pub(crate) fn clear_split_discards(&mut self) {
        let nodes = std::mem::take(&mut self.split_discards);
        self.release_dynamic_nodes(&nodes);
    }

    pub(crate) fn current_page_tail(&self) -> Option<&Node> {
        self.current_page.last()
    }

    pub(crate) fn current_page_len(&self) -> usize {
        self.current_page.len()
    }

    pub(crate) fn push_current_page(&mut self, node: Node) {
        self.allocate_dynamic_node(&node);
        self.current_page.push(node);
    }

    pub(crate) fn page_insertions(&self) -> &[PageInsertion] {
        &self.insertions
    }

    pub(crate) fn page_insertion(&self, class: u16) -> Option<PageInsertion> {
        self.insertion_positions
            .get(usize::from(class))
            .copied()
            .flatten()
            .map(|index| self.insertions[usize::from(index)])
    }

    pub(crate) fn upsert_page_insertion(&mut self, insertion: PageInsertion) {
        let class = insertion.class();
        let class_index = usize::from(class);
        if self.insertion_positions.len() <= class_index {
            self.insertion_positions.resize(class_index + 1, None);
        }
        if let Some(index) = self.insertion_positions[class_index] {
            self.insertions[usize::from(index)] = insertion;
            return;
        }

        // A class enters the active page at most once. Keep canonical class
        // order on that cold edge, then use the dense direct index for every
        // ordinary lookup and update.
        let index = self
            .insertions
            .iter()
            .position(|active| active.class() > class)
            .unwrap_or(self.insertions.len());
        self.insertions.insert(index, insertion);
        for position in self.insertion_positions.iter_mut().flatten() {
            if usize::from(*position) >= index {
                *position = position
                    .checked_add(1)
                    .expect("active insertion-class count fits u16");
            }
        }
        self.insertion_positions[class_index] = Some(
            u16::try_from(index).expect("active insertion-class count fits the TeX register space"),
        );
    }

    pub(crate) fn take_current_page_prefix(
        &mut self,
        split_index: usize,
    ) -> (Vec<Node>, Vec<Node>) {
        let nodes = self.current_page.take_prefix(split_index);
        self.release_dynamic_nodes(&nodes.0);
        self.release_dynamic_nodes(&nodes.1);
        nodes
    }

    fn allocate_dynamic_node(&mut self, node: &Node) {
        self.tex82_dynamic_words = self
            .tex82_dynamic_words
            .checked_add(node.tex_memory_words(false).1)
            .expect("page dynamic-memory accounting overflow");
        self.etex_dynamic_words = self
            .etex_dynamic_words
            .checked_add(node.tex_memory_words(true).1)
            .expect("page dynamic-memory accounting overflow");
    }

    fn release_dynamic_node(&mut self, node: &Node) {
        self.tex82_dynamic_words = self
            .tex82_dynamic_words
            .checked_sub(node.tex_memory_words(false).1)
            .expect("released more page dynamic memory than was live");
        self.etex_dynamic_words = self
            .etex_dynamic_words
            .checked_sub(node.tex_memory_words(true).1)
            .expect("released more page dynamic memory than was live");
    }

    fn allocate_dynamic_nodes(&mut self, nodes: &[Node]) {
        self.allocate_dynamic_word_totals(dynamic_words(nodes.iter()));
    }

    fn release_dynamic_nodes(&mut self, nodes: &[Node]) {
        self.release_dynamic_word_totals(dynamic_words(nodes.iter()));
    }

    fn allocate_dynamic_word_totals(&mut self, words: (usize, usize)) {
        self.tex82_dynamic_words = self
            .tex82_dynamic_words
            .checked_add(words.0)
            .expect("page dynamic-memory accounting overflow");
        self.etex_dynamic_words = self
            .etex_dynamic_words
            .checked_add(words.1)
            .expect("page dynamic-memory accounting overflow");
    }

    fn release_dynamic_word_totals(&mut self, words: (usize, usize)) {
        self.tex82_dynamic_words = self
            .tex82_dynamic_words
            .checked_sub(words.0)
            .expect("released more page dynamic memory than was live");
        self.etex_dynamic_words = self
            .etex_dynamic_words
            .checked_sub(words.1)
            .expect("released more page dynamic memory than was live");
    }

    #[cfg(test)]
    fn refresh_dynamic_memory_words(&mut self) {
        let nodes = self
            .contribution
            .iter()
            .chain(self.current_page.iter())
            .chain(self.page_discards.iter())
            .chain(self.split_discards.iter());
        let (tex82, etex) = nodes.fold((0_usize, 0_usize), |words, node| {
            (
                words.0.saturating_add(node.tex_memory_words(false).1),
                words.1.saturating_add(node.tex_memory_words(true).1),
            )
        });
        self.tex82_dynamic_words = tex82;
        self.etex_dynamic_words = etex;
    }

    pub(crate) fn update_last_from_node(&mut self, node: &Node) {
        self.last_glue = None;
        self.last_penalty = 0;
        self.last_kern = Scaled::from_raw(0);
        self.last_node_type = node.etex_type();
        match node {
            Node::Glue { spec, .. } => self.last_glue = Some(*spec),
            Node::Penalty(value) => self.last_penalty = *value,
            Node::Kern { amount, .. } => self.last_kern = *amount,
            _ => {}
        }
    }

    pub(crate) fn last_skip_ref(&self) -> Option<GlueSpec> {
        self.last_glue
    }

    pub(crate) const fn last_penalty(&self) -> i32 {
        self.last_penalty
    }

    pub(crate) const fn last_kern(&self) -> Scaled {
        self.last_kern
    }

    pub(crate) const fn last_node_type(&self) -> i32 {
        self.last_node_type
    }

    /// Whether the page builder's `last_glue` memo (TeX82 §996's "Update the
    /// values of `last_glue`...") currently names a real glue node, i.e. the
    /// most recently placed current-page item was itself glue. Unlike
    /// `last_skip`, which folds "no known last glue" into `GlueSpec::ZERO`,
    /// this distinguishes that case from a real zero-valued last glue -- the
    /// distinction `delete_last` (§1105) needs for `\unskip`'s apology.
    #[must_use]
    pub(crate) const fn has_last_glue(&self) -> bool {
        self.last_glue.is_some()
    }
}

fn dynamic_words<'a>(nodes: impl Iterator<Item = &'a Node>) -> (usize, usize) {
    nodes.fold((0_usize, 0_usize), |words, node| {
        (
            words.0.saturating_add(node.tex_memory_words(false).1),
            words.1.saturating_add(node.tex_memory_words(true).1),
        )
    })
}

fn node_retains_page_handle(node: &Node) -> bool {
    let mut retains = false;
    node.visit_node_lists(|list| retains |= !list.is_empty());
    retains
}

#[cfg(test)]
mod tests;
