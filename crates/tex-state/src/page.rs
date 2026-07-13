//! Snapshot-owned page-builder state.

use crate::glue::GlueSpec;
use crate::ids::{GlueId, TokenListId};
use crate::node::Node;
use crate::scaled::Scaled;
use crate::state_hash::{StateHashComponent, StateHashFragment, StateHasher};
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Weak};

const PAGE_PROJECTION_DOMAIN: u64 = 0x7061_6765_5f70_726a;
const PAGE_SCALARS_DOMAIN: u64 = 0x7061_6765_5f73_6361;
const PAGE_INSERTIONS_DOMAIN: u64 = 0x7061_6765_5f69_6e73;
const PAGE_MARK_CLASSES_DOMAIN: u64 = 0x7061_6765_5f6d_6172;
const PAGE_CONTRIBUTION_DOMAIN: u64 = 0x7061_6765_5f63_6f6e;
const PAGE_CURRENT_DOMAIN: u64 = 0x7061_6765_5f63_7572;
const PAGE_DISCARDS_DOMAIN: u64 = 0x7061_6765_5f64_6973;
const SPLIT_DISCARDS_DOMAIN: u64 = 0x7370_6c69_745f_6469;
const PAGE_NODE_CHUNK_DOMAIN: u64 = 0x7061_6765_5f63_686b;
const PAGE_NODE_CHUNK_LEN: usize = 64;

#[derive(Clone, Debug, Default)]
pub(crate) struct PageHashCache {
    insertions: Option<CachedArcProjection<Vec<PageInsertion>>>,
    mark_classes: Option<CachedArcProjection<BTreeMap<u16, MarkClassState>>>,
    contribution: Option<CachedArcProjection<VecDeque<Node>>>,
    current_page_trees: BTreeMap<usize, CachedTreeProjection>,
    current_page_tail: Option<CachedArcProjection<Vec<Node>>>,
    page_discards: Option<CachedArcProjection<Vec<Node>>>,
    split_discards: Option<CachedArcProjection<Vec<Node>>>,
}

impl PageHashCache {
    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }
}

#[derive(Clone, Debug)]
struct CachedArcProjection<T> {
    root: Arc<T>,
    fragment: StateHashFragment,
}

#[derive(Clone, Debug)]
struct CachedTreeProjection {
    root: Weak<PageNodeTree>,
    fragment: StateHashFragment,
}

/// Canonical persistent sequence for the growing current page.
///
/// Full 64-node leaves form a binary forest whose shape is the binary
/// decomposition of the full-leaf count. Appending a full leaf merges only the
/// carry path, while snapshots share every unaffected subtree. A bounded tail
/// holds fewer than 64 nodes. Shape depends only on content position, and the
/// representation carries no mutation-maintained hash state.
#[derive(Clone, Debug, Default)]
struct PageNodeSequence {
    forest: Arc<Vec<Arc<PageNodeTree>>>,
    tail: Arc<Vec<Node>>,
    len: usize,
}

#[derive(Debug)]
enum PageNodeTree {
    Leaf(Vec<Node>),
    Branch {
        height: u8,
        len: usize,
        left: Arc<PageNodeTree>,
        right: Arc<PageNodeTree>,
    },
}

impl PageNodeTree {
    fn height(&self) -> u8 {
        match self {
            Self::Leaf(_) => 0,
            Self::Branch { height, .. } => *height,
        }
    }

    fn len(&self) -> usize {
        match self {
            Self::Leaf(nodes) => nodes.len(),
            Self::Branch { len, .. } => *len,
        }
    }

    fn get(&self, index: usize) -> Option<&Node> {
        match self {
            Self::Leaf(nodes) => nodes.get(index),
            Self::Branch { left, right, .. } => {
                let left_len = left.len();
                if index < left_len {
                    left.get(index)
                } else {
                    right.get(index - left_len)
                }
            }
        }
    }
}

struct PageNodeIter<'a> {
    nodes: &'a PageNodeSequence,
    front: usize,
    back: usize,
}

impl<'a> Iterator for PageNodeIter<'a> {
    type Item = &'a Node;

    fn next(&mut self) -> Option<Self::Item> {
        if self.front == self.back {
            return None;
        }
        let node = self.nodes.get(self.front);
        self.front += 1;
        node
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.back - self.front;
        (remaining, Some(remaining))
    }
}

impl DoubleEndedIterator for PageNodeIter<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.front == self.back {
            return None;
        }
        self.back -= 1;
        self.nodes.get(self.back)
    }
}

impl ExactSizeIterator for PageNodeIter<'_> {}

impl PartialEq for PageNodeSequence {
    fn eq(&self, other: &Self) -> bool {
        self.len == other.len && self.iter().eq(other.iter())
    }
}

impl PageNodeSequence {
    fn iter(&self) -> PageNodeIter<'_> {
        PageNodeIter {
            nodes: self,
            front: 0,
            back: self.len,
        }
    }

    fn last(&self) -> Option<&Node> {
        self.get(self.len.checked_sub(1)?)
    }

    const fn len(&self) -> usize {
        self.len
    }

    fn push(&mut self, node: Node) {
        let tail = Arc::make_mut(&mut self.tail);
        tail.push(node);
        self.len += 1;
        if tail.len() != PAGE_NODE_CHUNK_LEN {
            return;
        }

        let leaf = Arc::new(PageNodeTree::Leaf(std::mem::take(tail)));
        let forest = Arc::make_mut(&mut self.forest);
        let mut carry = leaf;
        while forest
            .last()
            .is_some_and(|root| root.height() == carry.height())
        {
            let left = forest.pop().expect("equal-height forest root exists");
            carry = Arc::new(PageNodeTree::Branch {
                height: carry.height() + 1,
                len: left.len() + carry.len(),
                left,
                right: carry,
            });
        }
        forest.push(carry);
    }

    fn clear(&mut self) {
        *self = Self::default();
    }

    fn take_prefix(&mut self, split_index: usize) -> (Vec<Node>, Vec<Node>) {
        let split_index = split_index.min(self.len);
        let mut nodes = self.iter().cloned().collect::<Vec<_>>();
        let after = nodes.split_off(split_index);
        self.clear();
        (nodes, after)
    }

    fn get(&self, mut index: usize) -> Option<&Node> {
        if index >= self.len {
            return None;
        }
        for root in self.forest.iter() {
            if index < root.len() {
                return root.get(index);
            }
            index -= root.len();
        }
        self.tail.get(index)
    }
}

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MarkClassState {
    marks: [TokenListId; 5],
}

impl Default for MarkClassState {
    fn default() -> Self {
        Self {
            marks: [TokenListId::EMPTY; 5],
        }
    }
}

impl MarkClassState {
    fn get(self, mark: PageMark) -> TokenListId {
        self.marks[usize::from(mark.index())]
    }

    fn set(&mut self, mark: PageMark, value: TokenListId) {
        self.marks[usize::from(mark.index())] = value;
    }

    fn is_empty(self) -> bool {
        self.marks.iter().all(|mark| *mark == TokenListId::EMPTY)
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
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
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
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
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
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PageInsertionStatus {
    Inserting,
    SplitUp {
        broken_ins_index: usize,
        broken_at: Option<usize>,
    },
}

/// TeX.web page insertion record for one insertion class.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
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
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PageBuilderState {
    contribution: Arc<VecDeque<Node>>,
    current_page: PageNodeSequence,
    page_discards: Arc<Vec<Node>>,
    split_discards: Arc<Vec<Node>>,
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
    last_glue: Option<GlueId>,
    last_penalty: i32,
    last_kern: Scaled,
    last_node_type: i32,
    insert_penalties: i32,
    dead_cycles: i32,
    least_page_cost: i32,
    best_page_break: Option<PageBreak>,
    best_size: Scaled,
    fire_up: Option<PageFireUp>,
    insertions: Arc<Vec<PageInsertion>>,
    top_mark: TokenListId,
    first_mark: TokenListId,
    bot_mark: TokenListId,
    split_first_mark: TokenListId,
    split_bot_mark: TokenListId,
    mark_classes: Arc<BTreeMap<u16, MarkClassState>>,
}

/// Cheap semantic-root key for checkpoint hash-base reuse.
///
/// Scalars compare by value and collections compare by immutable root. A miss
/// merely recomputes the canonical page projection; no pointer enters its
/// fingerprint.
#[derive(Clone, Debug)]
pub(crate) struct PageStateHashCursor(PageBuilderState);

impl PartialEq for PageStateHashCursor {
    fn eq(&self, other: &Self) -> bool {
        let left = &self.0;
        let right = &other.0;
        left.page_goal == right.page_goal
            && left.page_total == right.page_total
            && left.page_stretch == right.page_stretch
            && left.page_fil_stretch == right.page_fil_stretch
            && left.page_fill_stretch == right.page_fill_stretch
            && left.page_filll_stretch == right.page_filll_stretch
            && left.page_shrink == right.page_shrink
            && left.page_depth == right.page_depth
            && left.page_max_depth == right.page_max_depth
            && left.contents == right.contents
            && left.last_glue == right.last_glue
            && left.last_penalty == right.last_penalty
            && left.last_kern == right.last_kern
            && left.last_node_type == right.last_node_type
            && left.insert_penalties == right.insert_penalties
            && left.dead_cycles == right.dead_cycles
            && left.least_page_cost == right.least_page_cost
            && left.best_page_break == right.best_page_break
            && left.best_size == right.best_size
            && left.fire_up == right.fire_up
            && left.top_mark == right.top_mark
            && left.first_mark == right.first_mark
            && left.bot_mark == right.bot_mark
            && left.split_first_mark == right.split_first_mark
            && left.split_bot_mark == right.split_bot_mark
            && left.current_page.len == right.current_page.len
            && Arc::ptr_eq(&left.contribution, &right.contribution)
            && Arc::ptr_eq(&left.current_page.forest, &right.current_page.forest)
            && Arc::ptr_eq(&left.current_page.tail, &right.current_page.tail)
            && Arc::ptr_eq(&left.page_discards, &right.page_discards)
            && Arc::ptr_eq(&left.split_discards, &right.split_discards)
            && Arc::ptr_eq(&left.insertions, &right.insertions)
            && Arc::ptr_eq(&left.mark_classes, &right.mark_classes)
    }
}

impl Eq for PageStateHashCursor {}

impl Default for PageBuilderState {
    fn default() -> Self {
        Self {
            contribution: Arc::new(VecDeque::new()),
            current_page: PageNodeSequence::default(),
            page_discards: Arc::new(Vec::new()),
            split_discards: Arc::new(Vec::new()),
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
            insertions: Arc::new(Vec::new()),
            top_mark: TokenListId::EMPTY,
            first_mark: TokenListId::EMPTY,
            bot_mark: TokenListId::EMPTY,
            split_first_mark: TokenListId::EMPTY,
            split_bot_mark: TokenListId::EMPTY,
            mark_classes: Arc::new(BTreeMap::new()),
        }
    }
}

impl PageBuilderState {
    pub(crate) fn state_hash_cursor(&self) -> PageStateHashCursor {
        PageStateHashCursor(self.clone())
    }

    pub(crate) fn is_format_empty(&self) -> bool {
        let mut state = self.clone();
        state.clear_page_discards();
        state.clear_split_discards();
        state == Self::default()
    }

    pub(crate) fn dimension(&self, dimension: PageDimension) -> Scaled {
        if self.contents.is_empty() && self.fire_up.is_none() {
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

    pub(crate) const fn mark(&self, mark: PageMark) -> TokenListId {
        match mark {
            PageMark::Top => self.top_mark,
            PageMark::First => self.first_mark,
            PageMark::Bot => self.bot_mark,
            PageMark::SplitFirst => self.split_first_mark,
            PageMark::SplitBot => self.split_bot_mark,
        }
    }

    pub(crate) fn set_mark(&mut self, mark: PageMark, value: TokenListId) {
        match mark {
            PageMark::Top => self.top_mark = value,
            PageMark::First => self.first_mark = value,
            PageMark::Bot => self.bot_mark = value,
            PageMark::SplitFirst => self.split_first_mark = value,
            PageMark::SplitBot => self.split_bot_mark = value,
        }
    }

    pub(crate) fn mark_class(&self, mark: PageMark, class: u16) -> TokenListId {
        if class == 0 {
            return self.mark(mark);
        }
        self.mark_classes
            .get(&class)
            .copied()
            .map_or(TokenListId::EMPTY, |marks| marks.get(mark))
    }

    pub(crate) fn set_mark_class(&mut self, mark: PageMark, class: u16, value: TokenListId) {
        if class == 0 {
            self.set_mark(mark, value);
            return;
        }
        let classes = Arc::make_mut(&mut self.mark_classes);
        let mut marks = classes.get(&class).copied().unwrap_or_default();
        marks.set(mark, value);
        if marks.is_empty() {
            classes.remove(&class);
        } else {
            classes.insert(class, marks);
        }
    }

    pub(crate) fn mark_class_ids(&self) -> impl Iterator<Item = u16> + '_ {
        self.mark_classes.keys().copied()
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
        Arc::make_mut(&mut self.insertions).clear();
    }

    pub(crate) fn start_new_page(&mut self) {
        self.current_page.clear();
        self.page_goal = Scaled::from_raw(0);
        self.page_total = Scaled::from_raw(0);
        self.page_stretch = Scaled::from_raw(0);
        self.page_fil_stretch = Scaled::from_raw(0);
        self.page_fill_stretch = Scaled::from_raw(0);
        self.page_filll_stretch = Scaled::from_raw(0);
        self.page_shrink = Scaled::from_raw(0);
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
        Arc::make_mut(&mut self.insertions).clear();
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

    pub(crate) const fn best_page_break(&self) -> Option<PageBreak> {
        self.best_page_break
    }

    pub(crate) const fn best_size(&self) -> Scaled {
        self.best_size
    }

    pub(crate) fn record_best_break(&mut self, break_index: usize, best_size: Scaled, cost: i32) {
        self.best_page_break = Some(PageBreak::new(break_index));
        self.best_size = best_size;
        self.least_page_cost = cost;
        for insertion in Arc::make_mut(&mut self.insertions) {
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
        Arc::make_mut(&mut self.contribution).push_back(node);
    }

    pub(crate) fn prepend_contribution(&mut self, node: Node) {
        Arc::make_mut(&mut self.contribution).push_front(node);
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

    pub(crate) fn contribution_tail(&self) -> Option<&Node> {
        self.contribution.back()
    }

    pub(crate) fn pop_contribution_front(&mut self) -> Option<Node> {
        if self.contribution.is_empty() {
            None
        } else {
            Arc::make_mut(&mut self.contribution).pop_front()
        }
    }

    pub(crate) fn pop_contribution_tail(&mut self) -> Option<Node> {
        Arc::make_mut(&mut self.contribution).pop_back()
    }

    pub(crate) fn prepend_contributions(&mut self, nodes: Vec<Node>) {
        if nodes.is_empty() {
            return;
        }
        let mut queue = VecDeque::with_capacity(nodes.len() + self.contribution.len());
        queue.extend(nodes);
        queue.extend(self.contribution.iter().cloned());
        self.contribution = Arc::new(queue);
    }

    pub(crate) fn current_page(&self) -> impl DoubleEndedIterator<Item = &Node> {
        self.current_page.iter()
    }

    pub(crate) fn page_discards(&self) -> &[Node] {
        &self.page_discards
    }

    pub(crate) fn push_page_discard(&mut self, node: Node) {
        Arc::make_mut(&mut self.page_discards).push(node);
    }

    pub(crate) fn take_page_discards(&mut self) -> Vec<Node> {
        std::mem::take(Arc::make_mut(&mut self.page_discards))
    }

    pub(crate) fn clear_page_discards(&mut self) {
        Arc::make_mut(&mut self.page_discards).clear();
    }

    pub(crate) fn split_discards(&self) -> &[Node] {
        &self.split_discards
    }

    pub(crate) fn set_split_discards(&mut self, nodes: Vec<Node>) {
        self.split_discards = Arc::new(nodes);
    }

    pub(crate) fn take_split_discards(&mut self) -> Vec<Node> {
        std::mem::take(Arc::make_mut(&mut self.split_discards))
    }

    pub(crate) fn clear_split_discards(&mut self) {
        Arc::make_mut(&mut self.split_discards).clear();
    }

    pub(crate) fn current_page_tail(&self) -> Option<&Node> {
        self.current_page.last()
    }

    pub(crate) fn current_page_len(&self) -> usize {
        self.current_page.len()
    }

    pub(crate) fn push_current_page(&mut self, node: Node) {
        self.current_page.push(node);
    }

    pub(crate) fn page_insertions(&self) -> &[PageInsertion] {
        &self.insertions
    }

    pub(crate) fn page_insertion(&self, class: u16) -> Option<PageInsertion> {
        self.insertions
            .binary_search_by_key(&class, PageInsertion::class)
            .ok()
            .map(|index| self.insertions[index])
    }

    pub(crate) fn upsert_page_insertion(&mut self, insertion: PageInsertion) {
        match self
            .insertions
            .binary_search_by_key(&insertion.class(), PageInsertion::class)
        {
            Ok(index) => Arc::make_mut(&mut self.insertions)[index] = insertion,
            Err(index) => Arc::make_mut(&mut self.insertions).insert(index, insertion),
        }
    }

    pub(crate) fn take_current_page_prefix(
        &mut self,
        split_index: usize,
    ) -> (Vec<Node>, Vec<Node>) {
        self.current_page.take_prefix(split_index)
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

    pub(crate) fn last_skip(&self, glue: impl FnOnce(GlueId) -> GlueSpec) -> GlueSpec {
        self.last_glue.map_or(GlueSpec::ZERO, glue)
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

    pub(crate) fn hash_semantic(
        &self,
        hasher: &mut StateHasher,
        cache: &mut PageHashCache,
        mut hash_queue: impl FnMut(&VecDeque<Node>, &mut StateHasher) -> usize,
        mut hash_nodes: impl FnMut(&[Node], &mut StateHasher) -> usize,
        mut hash_glue: impl FnMut(GlueId, &mut StateHasher),
        mut hash_tokens: impl FnMut(TokenListId, &mut StateHasher),
    ) {
        let scalars = StateHashFragment::from_measured_builder(
            PAGE_SCALARS_DOMAIN,
            StateHashComponent::PageScalars,
            1,
            |projection| {
                projection.u8(match self.contents {
                    PageContents::Empty => 0,
                    PageContents::InsertsOnly => 1,
                    PageContents::BoxThere => 2,
                });
                for dimension in [
                    PageDimension::Goal,
                    PageDimension::Total,
                    PageDimension::Stretch,
                    PageDimension::FilStretch,
                    PageDimension::FillStretch,
                    PageDimension::FilllStretch,
                    PageDimension::Shrink,
                    PageDimension::Depth,
                ] {
                    projection.i32(self.raw_dimension(dimension).raw());
                }
                projection.i32(self.page_max_depth.raw());
                match self.last_glue {
                    Some(id) => {
                        projection.bool(true);
                        hash_glue(id, projection);
                    }
                    None => projection.bool(false),
                }
                projection.i32(self.last_penalty);
                projection.i32(self.last_kern.raw());
                projection.i32(self.last_node_type);
                projection.i32(self.insert_penalties);
                projection.i32(self.dead_cycles);
                projection.i32(self.least_page_cost);
                hash_optional_usize(self.best_page_break.map(PageBreak::index), projection);
                projection.i32(self.best_size.raw());
                match self.fire_up {
                    Some(fire_up) => {
                        projection.bool(true);
                        projection.usize(fire_up.best_break().index());
                        projection.i32(fire_up.best_size().raw());
                        projection.usize(fire_up.trigger().index());
                    }
                    None => projection.bool(false),
                }
                for mark in [
                    self.top_mark,
                    self.first_mark,
                    self.bot_mark,
                    self.split_first_mark,
                    self.split_bot_mark,
                ] {
                    hash_tokens(mark, projection);
                }
            },
        );
        let insertions = project_arc(
            &mut cache.insertions,
            &self.insertions,
            PAGE_INSERTIONS_DOMAIN,
            StateHashComponent::PageInsertions,
            |projection| {
                projection.usize(self.insertions.len());
                for insertion in self.insertions.iter() {
                    projection.u16(insertion.class);
                    match insertion.status {
                        PageInsertionStatus::Inserting => projection.u8(0),
                        PageInsertionStatus::SplitUp {
                            broken_ins_index,
                            broken_at,
                        } => {
                            projection.u8(1);
                            projection.usize(broken_ins_index);
                            hash_optional_usize(broken_at, projection);
                        }
                    }
                    projection.i32(insertion.height.raw());
                    hash_optional_usize(insertion.last_ins_index, projection);
                    hash_optional_usize(insertion.best_ins_index, projection);
                }
                self.insertions.len()
            },
        );
        let mark_classes = project_arc(
            &mut cache.mark_classes,
            &self.mark_classes,
            PAGE_MARK_CLASSES_DOMAIN,
            StateHashComponent::PageMarks,
            |projection| {
                projection.usize(self.mark_classes.len());
                for (&class, marks) in self.mark_classes.iter() {
                    projection.u16(class);
                    for mark in marks.marks {
                        hash_tokens(mark, projection);
                    }
                }
                self.mark_classes.len()
            },
        );
        let contribution = project_arc(
            &mut cache.contribution,
            &self.contribution,
            PAGE_CONTRIBUTION_DOMAIN,
            StateHashComponent::PageContribution,
            |projection| hash_queue(&self.contribution, projection),
        );
        let current_page = project_page_nodes(
            &mut cache.current_page_trees,
            &mut cache.current_page_tail,
            &self.current_page,
            &mut hash_nodes,
        );
        let page_discards = project_arc(
            &mut cache.page_discards,
            &self.page_discards,
            PAGE_DISCARDS_DOMAIN,
            StateHashComponent::PageDiscards,
            |projection| hash_nodes(&self.page_discards, projection),
        );
        let split_discards = project_arc(
            &mut cache.split_discards,
            &self.split_discards,
            SPLIT_DISCARDS_DOMAIN,
            StateHashComponent::PageDiscards,
            |projection| hash_nodes(&self.split_discards, projection),
        );

        StateHashFragment::from_builder(PAGE_PROJECTION_DOMAIN, |projection| {
            scalars.apply(projection);
            insertions.apply(projection);
            mark_classes.apply(projection);
            contribution.apply(projection);
            current_page.apply(projection);
            page_discards.apply(projection);
            split_discards.apply(projection);
        })
        .apply(hasher);
    }
}

fn project_arc<T>(
    cached: &mut Option<CachedArcProjection<T>>,
    root: &Arc<T>,
    domain: u64,
    component: StateHashComponent,
    build: impl FnOnce(&mut StateHasher) -> usize,
) -> StateHashFragment {
    if let Some(cached) = cached
        && Arc::ptr_eq(&cached.root, root)
    {
        return cached.fragment;
    }
    let fragment = StateHashFragment::from_measured_builder_counted(domain, component, build);
    *cached = Some(CachedArcProjection {
        root: Arc::clone(root),
        fragment,
    });
    fragment
}

fn project_page_nodes(
    cached_trees: &mut BTreeMap<usize, CachedTreeProjection>,
    cached_tail: &mut Option<CachedArcProjection<Vec<Node>>>,
    nodes: &PageNodeSequence,
    hash_nodes: &mut impl FnMut(&[Node], &mut StateHasher) -> usize,
) -> StateHashFragment {
    if nodes.len == 0 {
        cached_trees.clear();
    }
    let mut roots = Vec::with_capacity(nodes.forest.len());
    for root in nodes.forest.iter() {
        roots.push(project_page_tree(cached_trees, root, hash_nodes));
    }
    let tail = project_arc(
        cached_tail,
        &nodes.tail,
        PAGE_NODE_CHUNK_DOMAIN,
        StateHashComponent::PageCurrent,
        |projection| hash_nodes(&nodes.tail, projection),
    );
    StateHashFragment::from_measured_builder(
        PAGE_CURRENT_DOMAIN,
        StateHashComponent::PageCurrent,
        0,
        |projection| {
            projection.usize(nodes.len());
            projection.usize(roots.len());
            for root in roots {
                root.apply(projection);
            }
            tail.apply(projection);
        },
    )
}

fn project_page_tree(
    cache: &mut BTreeMap<usize, CachedTreeProjection>,
    root: &Arc<PageNodeTree>,
    hash_nodes: &mut impl FnMut(&[Node], &mut StateHasher) -> usize,
) -> StateHashFragment {
    let key = Arc::as_ptr(root) as usize;
    if let Some(cached) = cache.get(&key)
        && cached
            .root
            .upgrade()
            .is_some_and(|cached_root| Arc::ptr_eq(&cached_root, root))
    {
        return cached.fragment;
    }
    let fragment = match root.as_ref() {
        PageNodeTree::Leaf(nodes) => StateHashFragment::from_measured_builder_counted(
            PAGE_NODE_CHUNK_DOMAIN,
            StateHashComponent::PageCurrent,
            |projection| hash_nodes(nodes, projection),
        ),
        PageNodeTree::Branch {
            height,
            len,
            left,
            right,
        } => {
            let left = project_page_tree(cache, left, hash_nodes);
            let right = project_page_tree(cache, right, hash_nodes);
            StateHashFragment::from_measured_builder(
                PAGE_NODE_CHUNK_DOMAIN,
                StateHashComponent::PageCurrent,
                0,
                |projection| {
                    projection.u8(*height);
                    projection.usize(*len);
                    left.apply(projection);
                    right.apply(projection);
                },
            )
        }
    };
    cache.insert(
        key,
        CachedTreeProjection {
            root: Arc::downgrade(root),
            fragment,
        },
    );
    fragment
}

fn hash_optional_usize(value: Option<usize>, hasher: &mut StateHasher) {
    match value {
        Some(value) => {
            hasher.bool(true);
            hasher.usize(value);
        }
        None => hasher.bool(false),
    }
}

#[cfg(test)]
mod tests;
