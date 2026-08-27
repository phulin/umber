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
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_PAGE_TIMELINE: AtomicU64 = AtomicU64::new(1);

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

#[derive(Clone, Copy, Debug)]
struct InsertionLaneRecord {
    class: u16,
    value: Option<PageInsertion>,
}

#[derive(Clone, Debug)]
struct MarkLaneRecord {
    class: u16,
    mark: PageMark,
    value: Option<NodeTokenList>,
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

/// Borrowed logical contribution deque across the candidate's bounded regions.
#[derive(Clone, Copy, Debug)]
pub struct PageContributionView<'a> {
    front: Option<&'a VecDeque<Node>>,
    prior: Option<(&'a VecDeque<Node>, usize, usize)>,
    back: &'a VecDeque<Node>,
}

pub struct PageInsertionView<'a> {
    page: &'a PageBuilderState,
}

impl PageInsertionView<'_> {
    pub fn iter(&self) -> PageInsertionIter<'_> {
        PageInsertionIter {
            page: self.page,
            normal_index: 0,
            candidate_class: 0,
            candidate: self.page.checkpoint_journal.fork.is_some(),
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.iter().count()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.iter().next().is_none()
    }

    #[must_use]
    pub fn to_vec(&self) -> Vec<PageInsertion> {
        self.iter().collect()
    }
}

pub struct PageInsertionIter<'a> {
    page: &'a PageBuilderState,
    normal_index: usize,
    candidate_class: u32,
    candidate: bool,
}

impl Iterator for PageInsertionIter<'_> {
    type Item = PageInsertion;

    fn next(&mut self) -> Option<Self::Item> {
        if !self.candidate {
            let value = self.page.insertions.get(self.normal_index).copied();
            self.normal_index += usize::from(value.is_some());
            return value;
        }
        while self.candidate_class <= u32::from(u16::MAX) {
            let class = u16::try_from(self.candidate_class).expect("candidate class fits u16");
            self.candidate_class += 1;
            if let Some(value) = self.page.page_insertion(class) {
                return Some(value);
            }
        }
        None
    }
}

pub(crate) struct MarkClassIdIter<'a> {
    page: &'a PageBuilderState,
    normal_index: usize,
    candidate_class: u32,
    candidate: bool,
}

impl Iterator for MarkClassIdIter<'_> {
    type Item = u16;

    fn next(&mut self) -> Option<Self::Item> {
        if !self.candidate {
            let value = self
                .page
                .mark_classes
                .get(self.normal_index)
                .map(|(class, _)| *class);
            self.normal_index += usize::from(value.is_some());
            return value;
        }
        while self.candidate_class <= u32::from(u16::MAX) {
            let class = u16::try_from(self.candidate_class).expect("candidate class fits u16");
            self.candidate_class += 1;
            if [
                PageMark::Top,
                PageMark::First,
                PageMark::Bot,
                PageMark::SplitFirst,
                PageMark::SplitBot,
            ]
            .into_iter()
            .any(|mark| self.page.mark_class_value(mark, class).is_some())
            {
                return Some(class);
            }
        }
        None
    }
}

impl<'a> PageContributionView<'a> {
    #[must_use]
    pub fn len(self) -> usize {
        self.front.map_or(0, VecDeque::len)
            + self.prior.map_or(0, |(_, start, end)| end - start)
            + self.back.len()
    }

    #[must_use]
    pub fn is_empty(self) -> bool {
        self.len() == 0
    }

    #[must_use]
    pub fn get(self, index: usize) -> Option<&'a Node> {
        let front_len = self.front.map_or(0, VecDeque::len);
        if index < front_len {
            return self.front.and_then(|front| front.get(index));
        }
        let index = index - front_len;
        let prior_len = self.prior.map_or(0, |(_, start, end)| end - start);
        if index < prior_len {
            let (prior, start, _) = self.prior?;
            return prior.get(start + index);
        }
        self.back.get(index - prior_len)
    }

    #[must_use]
    pub fn front(self) -> Option<&'a Node> {
        self.get(0)
    }

    #[must_use]
    pub fn back(self) -> Option<&'a Node> {
        self.len().checked_sub(1).and_then(|index| self.get(index))
    }

    pub fn iter(self) -> PageContributionIter<'a> {
        PageContributionIter {
            view: self,
            front: 0,
            back: self.len(),
        }
    }

    #[must_use]
    pub fn to_vec(self) -> Vec<Node> {
        self.iter().cloned().collect()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PageContributionIter<'a> {
    view: PageContributionView<'a>,
    front: usize,
    back: usize,
}

impl<'a> Iterator for PageContributionIter<'a> {
    type Item = &'a Node;

    fn next(&mut self) -> Option<Self::Item> {
        if self.front == self.back {
            return None;
        }
        let index = self.front;
        self.front += 1;
        self.view.get(index)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.back - self.front;
        (len, Some(len))
    }
}

impl DoubleEndedIterator for PageContributionIter<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.front == self.back {
            return None;
        }
        self.back -= 1;
        self.view.get(self.back)
    }
}

impl ExactSizeIterator for PageContributionIter<'_> {}

pub(crate) struct PageCurrentIter<'a> {
    prior: Option<(&'a PageNodeSequence, usize)>,
    current: &'a PageNodeSequence,
    front: usize,
    back: usize,
}

impl<'a> PageCurrentIter<'a> {
    fn get(&self, index: usize) -> Option<&'a Node> {
        let prior_len = self.prior.map_or(0, |(_, end)| end);
        if index < prior_len {
            self.prior.and_then(|(prior, _)| prior.get(index))
        } else {
            self.current.get(index - prior_len)
        }
    }
}

impl<'a> Iterator for PageCurrentIter<'a> {
    type Item = &'a Node;

    fn next(&mut self) -> Option<Self::Item> {
        if self.front == self.back {
            return None;
        }
        let index = self.front;
        self.front += 1;
        self.get(index)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.back - self.front;
        (len, Some(len))
    }
}

impl DoubleEndedIterator for PageCurrentIter<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.front == self.back {
            return None;
        }
        self.back -= 1;
        self.get(self.back)
    }
}

impl ExactSizeIterator for PageCurrentIter<'_> {}

/// Snapshot-owned state for TeX.web's page builder.
#[derive(Clone)]
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
    insertion_lane: Vec<InsertionLaneRecord>,
    mark_lane: Vec<MarkLaneRecord>,
    tex82_dynamic_words: usize,
    etex_dynamic_words: usize,
    page_node_root_count: usize,
    checkpoint_journal: PageCheckpointJournal,
}

/// Bounded root of one page-builder state on its generation timeline.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PageCheckpointMark {
    timeline: u64,
    frame: u64,
    cursor: usize,
    scalars: PageScalars,
    roots: PagePayloadRoots,
}

#[derive(Clone, Copy, Debug, Default)]
struct PagePayloadRoots {
    contribution_start: usize,
    contribution_end: usize,
    current_page_end: usize,
    page_discards_end: usize,
    split_discards_end: usize,
    insertion_end: usize,
    mark_end: usize,
}

#[derive(Clone)]
struct PageCheckpointFrame {
    id: u64,
    cursor: usize,
}

#[derive(Clone)]
struct PageCheckpointJournal {
    timeline: u64,
    next_frame: u64,
    frames: Vec<PageCheckpointFrame>,
    inverses: Vec<PageInverse>,
    applied: usize,
    fork: Option<Box<PageForkJournal>>,
    replay_work: u64,
}

#[derive(Clone)]
struct PageForkJournal {
    origin_timeline: u64,
    origin: usize,
    target: usize,
    future: Vec<PageInverse>,
    future_frames: Vec<PageCheckpointFrame>,
    flat_origin: Option<Box<PagePayload>>,
    origin_scalars: Option<PageScalars>,
    target_roots: Option<PagePayloadRoots>,
    contribution_front: VecDeque<Node>,
}

#[derive(Clone, Default)]
struct PagePayload {
    contribution: VecDeque<Node>,
    current_page: PageNodeSequence,
    page_discards: Vec<Node>,
    split_discards: Vec<Node>,
    insertions: Vec<PageInsertion>,
    insertion_positions: Vec<Option<u16>>,
    top_mark: Option<NodeTokenList>,
    first_mark: Option<NodeTokenList>,
    bot_mark: Option<NodeTokenList>,
    split_first_mark: Option<NodeTokenList>,
    split_bot_mark: Option<NodeTokenList>,
    mark_classes: Vec<(u16, MarkClassState)>,
    mark_class_positions: Vec<Option<u16>>,
    insertion_lane: Vec<InsertionLaneRecord>,
    mark_lane: Vec<MarkLaneRecord>,
}

#[derive(Clone)]
enum PageInverse {
    Noop,
    Scalars(PageScalars),
    ContributionPushBack(Option<Node>),
    ContributionPushFront(Option<Node>),
    ContributionRemoved {
        start: usize,
        count: usize,
        nodes: Option<Vec<Node>>,
    },
    ContributionPoppedFront(Option<Node>),
    ContributionPrepended {
        count: usize,
        nodes: Option<Vec<Node>>,
    },
    CurrentPagePush(Option<Node>),
    CurrentPageReplace(PageNodeSequence),
    PageDiscardsPush(Option<Node>),
    PageDiscardsReplace(Vec<Node>),
    SplitDiscardsReplace(Vec<Node>),
    InsertionsReplace {
        insertions: Vec<PageInsertion>,
        positions: Vec<Option<u16>>,
    },
    InsertionUpsert {
        class: u16,
        old: Option<PageInsertion>,
    },
    Marks([Option<NodeTokenList>; 5]),
    MarkClass {
        class: u16,
        old: Option<MarkClassState>,
    },
}

#[derive(Clone, Copy, Debug)]
struct PageScalars {
    dimensions: [Scaled; 8],
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
    tex82_dynamic_words: usize,
    etex_dynamic_words: usize,
    page_node_root_count: usize,
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
            insertion_lane: Vec::new(),
            mark_lane: Vec::new(),
            tex82_dynamic_words: 0,
            etex_dynamic_words: 0,
            page_node_root_count: 0,
            checkpoint_journal: PageCheckpointJournal {
                timeline: NEXT_PAGE_TIMELINE.fetch_add(1, Ordering::Relaxed),
                next_frame: 1,
                frames: Vec::with_capacity(64),
                // Page inverses include move-only list carriers and are much
                // larger than the journal's scalar records.  Grow this lane
                // on demand instead of charging every page builder for a
                // payload-sized speculative reserve.
                inverses: Vec::new(),
                applied: 0,
                fork: None,
                replay_work: 0,
            },
        }
    }
}

impl PageBuilderState {
    pub(crate) fn checkpoint_mark(&mut self) -> PageCheckpointMark {
        let frame = self.checkpoint_journal.next_frame;
        self.checkpoint_journal.next_frame = self
            .checkpoint_journal
            .next_frame
            .checked_add(1)
            .expect("page checkpoint identity space exhausted");
        let cursor = self.checkpoint_journal.applied;
        let scalars = self.scalar_snapshot();
        let roots = PagePayloadRoots {
            contribution_start: 0,
            contribution_end: self.contribution.len(),
            current_page_end: self.current_page.len(),
            page_discards_end: self.page_discards.len(),
            split_discards_end: self.split_discards.len(),
            insertion_end: self.insertion_lane.len(),
            mark_end: self.mark_lane.len(),
        };
        self.checkpoint_journal
            .frames
            .push(PageCheckpointFrame { id: frame, cursor });
        PageCheckpointMark {
            timeline: self.checkpoint_journal.timeline,
            frame,
            cursor,
            scalars,
            roots,
        }
    }

    pub(crate) fn commit_transaction(&mut self, mark: PageCheckpointMark) {
        debug_assert!(self.validates_checkpoint_mark(mark));
        self.checkpoint_journal
            .frames
            .retain(|frame| frame.id != mark.frame);
        if self.checkpoint_journal.frames.is_empty() {
            self.checkpoint_journal.inverses.clear();
            self.checkpoint_journal.applied = 0;
        }
    }

    pub(crate) fn rollback_transaction(&mut self, mark: PageCheckpointMark) {
        self.restore_checkpoint_mark(mark);
        self.checkpoint_journal.inverses.truncate(mark.cursor);
        self.checkpoint_journal.applied = mark.cursor;
        self.checkpoint_journal
            .frames
            .retain(|frame| frame.id != mark.frame);
        if self.checkpoint_journal.frames.is_empty() {
            self.checkpoint_journal.inverses.clear();
            self.checkpoint_journal.applied = 0;
        }
    }

    pub(crate) fn validates_checkpoint_mark(&self, mark: PageCheckpointMark) -> bool {
        mark.timeline == self.checkpoint_journal.timeline
            && mark.cursor <= self.checkpoint_journal.inverses.len()
            && mark.frame != 0
            && mark.frame < self.checkpoint_journal.next_frame
    }

    pub(crate) fn restore_checkpoint_mark(&mut self, mark: PageCheckpointMark) {
        debug_assert!(self.validates_checkpoint_mark(mark));
        while self.checkpoint_journal.applied > mark.cursor {
            self.checkpoint_journal.applied -= 1;
            self.toggle_page_inverse(self.checkpoint_journal.applied);
        }
        while self.checkpoint_journal.applied < mark.cursor {
            let index = self.checkpoint_journal.applied;
            self.toggle_page_inverse(index);
            self.checkpoint_journal.applied += 1;
        }
    }

    pub(crate) fn begin_checkpoint_fork(&mut self, mark: PageCheckpointMark) {
        debug_assert!(self.validates_checkpoint_mark(mark));
        debug_assert!(self.checkpoint_journal.fork.is_none());
        let origin = self.checkpoint_journal.applied;
        {
            let origin_timeline = self.checkpoint_journal.timeline;
            let origin_scalars = self.scalar_snapshot();
            let flat_origin = Box::new(self.take_payload());
            self.restore_scalars(mark.scalars);
            let future = std::mem::take(&mut self.checkpoint_journal.inverses);
            let future_frames = std::mem::take(&mut self.checkpoint_journal.frames);
            self.checkpoint_journal.applied = 0;
            self.checkpoint_journal.timeline = NEXT_PAGE_TIMELINE.fetch_add(1, Ordering::Relaxed);
            self.checkpoint_journal.fork = Some(Box::new(PageForkJournal {
                origin_timeline,
                origin,
                target: 0,
                future,
                future_frames,
                flat_origin: Some(flat_origin),
                origin_scalars: Some(origin_scalars),
                target_roots: Some(mark.roots),
                contribution_front: VecDeque::new(),
            }));
            return;
        }
    }

    pub(crate) fn reject_checkpoint_fork(&mut self) {
        let fork = self
            .checkpoint_journal
            .fork
            .take()
            .expect("candidate page timeline owns one fork");
        if let Some(origin) = fork.flat_origin {
            let _candidate = self.take_payload();
            self.install_payload(*origin);
            self.restore_scalars(
                fork.origin_scalars
                    .expect("flat page fork retains origin scalars"),
            );
            self.checkpoint_journal.inverses = fork.future;
            self.checkpoint_journal.frames = fork.future_frames;
            self.checkpoint_journal.applied = fork.origin;
            self.checkpoint_journal.timeline = fork.origin_timeline;
            return;
        }
        while self.checkpoint_journal.applied > fork.target {
            self.checkpoint_journal.applied -= 1;
            self.toggle_page_inverse(self.checkpoint_journal.applied);
        }
        self.checkpoint_journal.inverses.truncate(fork.target);
        self.checkpoint_journal.inverses.extend(fork.future);
        self.checkpoint_journal.frames.extend(fork.future_frames);
        while self.checkpoint_journal.applied < fork.origin {
            let index = self.checkpoint_journal.applied;
            self.toggle_page_inverse(index);
            self.checkpoint_journal.applied += 1;
        }
    }

    pub(crate) fn commit_checkpoint_fork(&mut self) {
        let Some(fork) = self.checkpoint_journal.fork.take() else {
            return;
        };
        let Some(mut accepted) = fork.flat_origin.map(|origin| *origin) else {
            return;
        };
        let roots = fork.target_roots.expect("owner fork retains page roots");
        let mut candidate = self.take_payload();

        for _ in 0..roots.contribution_start {
            let _ = accepted.contribution.pop_front();
        }
        accepted
            .contribution
            .truncate(roots.contribution_end - roots.contribution_start);
        let mut contribution = fork.contribution_front;
        contribution.append(&mut accepted.contribution);
        contribution.append(&mut candidate.contribution);
        candidate.contribution = contribution;

        let (mut current_page, _) = accepted.current_page.take_prefix(roots.current_page_end);
        current_page.extend(candidate.current_page.into_nodes());
        candidate.current_page = PageNodeSequence::from_nodes(current_page);

        accepted.page_discards.truncate(roots.page_discards_end);
        accepted.page_discards.append(&mut candidate.page_discards);
        candidate.page_discards = accepted.page_discards;
        accepted.split_discards.truncate(roots.split_discards_end);
        accepted
            .split_discards
            .append(&mut candidate.split_discards);
        candidate.split_discards = accepted.split_discards;

        accepted.insertion_lane.truncate(roots.insertion_end);
        accepted
            .insertion_lane
            .append(&mut candidate.insertion_lane);
        candidate.insertion_lane = accepted.insertion_lane;
        accepted.mark_lane.truncate(roots.mark_end);
        accepted.mark_lane.append(&mut candidate.mark_lane);
        candidate.mark_lane = accepted.mark_lane;
        self.install_payload(candidate);
        self.rebuild_canonical_lane_values();
    }

    fn rebuild_canonical_lane_values(&mut self) {
        let mut insertions = std::collections::BTreeMap::new();
        for record in &self.insertion_lane {
            if let Some(value) = record.value {
                insertions.insert(record.class, value);
            } else {
                insertions.remove(&record.class);
            }
        }
        self.insertions = insertions.into_values().collect();
        self.rebuild_insertion_positions();

        self.top_mark = None;
        self.first_mark = None;
        self.bot_mark = None;
        self.split_first_mark = None;
        self.split_bot_mark = None;
        self.mark_classes.clear();
        self.mark_class_positions.clear();
        let records = self.mark_lane.clone();
        for record in records {
            if record.class == 0 {
                *match record.mark {
                    PageMark::Top => &mut self.top_mark,
                    PageMark::First => &mut self.first_mark,
                    PageMark::Bot => &mut self.bot_mark,
                    PageMark::SplitFirst => &mut self.split_first_mark,
                    PageMark::SplitBot => &mut self.split_bot_mark,
                } = record.value;
            } else if let Some(value) = record.value {
                self.ensure_mark_class(record.class).set(record.mark, value);
            } else if let Some(position) = self.mark_class_position(record.class) {
                self.mark_classes[position].1.clear(record.mark);
                if self.mark_classes[position].1.is_empty() {
                    self.remove_mark_class(record.class, position);
                }
            }
        }
    }

    pub(crate) const fn has_checkpoint_fork(&self) -> bool {
        self.checkpoint_journal.fork.is_some()
    }

    fn record_page_inverse(&mut self, inverse: PageInverse) {
        if !self.checkpoint_journal.frames.is_empty() {
            if self.checkpoint_journal.applied < self.checkpoint_journal.inverses.len() {
                self.checkpoint_journal
                    .inverses
                    .truncate(self.checkpoint_journal.applied);
                self.checkpoint_journal
                    .frames
                    .retain(|frame| frame.cursor <= self.checkpoint_journal.applied);
            }
            self.checkpoint_journal.inverses.push(inverse);
            self.checkpoint_journal.applied += 1;
        }
    }

    fn scalar_snapshot(&self) -> PageScalars {
        PageScalars {
            dimensions: [
                self.page_goal,
                self.page_total,
                self.page_stretch,
                self.page_fil_stretch,
                self.page_fill_stretch,
                self.page_filll_stretch,
                self.page_shrink,
                self.page_depth,
            ],
            page_max_depth: self.page_max_depth,
            contents: self.contents,
            last_glue: self.last_glue,
            last_penalty: self.last_penalty,
            last_kern: self.last_kern,
            last_node_type: self.last_node_type,
            insert_penalties: self.insert_penalties,
            dead_cycles: self.dead_cycles,
            least_page_cost: self.least_page_cost,
            best_page_break: self.best_page_break,
            best_size: self.best_size,
            fire_up: self.fire_up,
            tex82_dynamic_words: self.tex82_dynamic_words,
            etex_dynamic_words: self.etex_dynamic_words,
            page_node_root_count: self.page_node_root_count,
        }
    }

    fn take_payload(&mut self) -> PagePayload {
        PagePayload {
            contribution: std::mem::take(&mut self.contribution),
            current_page: std::mem::take(&mut self.current_page),
            page_discards: std::mem::take(&mut self.page_discards),
            split_discards: std::mem::take(&mut self.split_discards),
            insertions: std::mem::take(&mut self.insertions),
            insertion_positions: std::mem::take(&mut self.insertion_positions),
            top_mark: self.top_mark.take(),
            first_mark: self.first_mark.take(),
            bot_mark: self.bot_mark.take(),
            split_first_mark: self.split_first_mark.take(),
            split_bot_mark: self.split_bot_mark.take(),
            mark_classes: std::mem::take(&mut self.mark_classes),
            mark_class_positions: std::mem::take(&mut self.mark_class_positions),
            insertion_lane: std::mem::take(&mut self.insertion_lane),
            mark_lane: std::mem::take(&mut self.mark_lane),
        }
    }

    fn install_payload(&mut self, payload: PagePayload) {
        self.contribution = payload.contribution;
        self.current_page = payload.current_page;
        self.page_discards = payload.page_discards;
        self.split_discards = payload.split_discards;
        self.insertions = payload.insertions;
        self.insertion_positions = payload.insertion_positions;
        self.top_mark = payload.top_mark;
        self.first_mark = payload.first_mark;
        self.bot_mark = payload.bot_mark;
        self.split_first_mark = payload.split_first_mark;
        self.split_bot_mark = payload.split_bot_mark;
        self.mark_classes = payload.mark_classes;
        self.mark_class_positions = payload.mark_class_positions;
        self.insertion_lane = payload.insertion_lane;
        self.mark_lane = payload.mark_lane;
    }

    fn record_scalars(&mut self) {
        if !self.checkpoint_journal.frames.is_empty() {
            let old = self.scalar_snapshot();
            self.record_page_inverse(PageInverse::Scalars(old));
        }
    }

    fn toggle_page_inverse(&mut self, index: usize) {
        self.checkpoint_journal.replay_work = self.checkpoint_journal.replay_work.saturating_add(1);
        let inverse = std::mem::replace(
            &mut self.checkpoint_journal.inverses[index],
            PageInverse::Noop,
        );
        let inverse = match inverse {
            PageInverse::Noop => unreachable!("page inverse slot is occupied"),
            PageInverse::Scalars(old) => {
                let current = self.scalar_snapshot();
                self.restore_scalars(old);
                PageInverse::Scalars(current)
            }
            PageInverse::ContributionPushBack(mut node) => {
                if let Some(node) = node.take() {
                    self.contribution.push_back(node);
                } else {
                    node = self.contribution.pop_back();
                }
                PageInverse::ContributionPushBack(node)
            }
            PageInverse::ContributionPushFront(mut node) => {
                if let Some(node) = node.take() {
                    self.contribution.push_front(node);
                } else {
                    node = self.contribution.pop_front();
                }
                PageInverse::ContributionPushFront(node)
            }
            PageInverse::ContributionRemoved {
                start,
                count,
                mut nodes,
            } => {
                if let Some(restored) = nodes.take() {
                    for (offset, node) in restored.into_iter().enumerate() {
                        self.contribution.insert(start + offset, node);
                    }
                } else {
                    nodes = Some(self.contribution.drain(start..start + count).collect());
                }
                PageInverse::ContributionRemoved {
                    start,
                    count,
                    nodes,
                }
            }
            PageInverse::ContributionPoppedFront(mut node) => {
                if let Some(restored) = node.take() {
                    self.contribution.push_front(restored);
                } else {
                    node = self.contribution.pop_front();
                }
                PageInverse::ContributionPoppedFront(node)
            }
            PageInverse::ContributionPrepended { count, mut nodes } => {
                if let Some(restored) = nodes.take() {
                    for node in restored.into_iter().rev() {
                        self.contribution.push_front(node);
                    }
                } else {
                    nodes = Some(self.contribution.drain(..count).collect());
                }
                PageInverse::ContributionPrepended { count, nodes }
            }
            PageInverse::CurrentPagePush(mut node) => {
                if let Some(restored) = node.take() {
                    self.current_page.push(restored);
                } else {
                    node = self.current_page.pop();
                }
                PageInverse::CurrentPagePush(node)
            }
            PageInverse::CurrentPageReplace(mut old) => {
                std::mem::swap(&mut self.current_page, &mut old);
                PageInverse::CurrentPageReplace(old)
            }
            PageInverse::PageDiscardsPush(mut node) => {
                if let Some(restored) = node.take() {
                    self.page_discards.push(restored);
                } else {
                    node = self.page_discards.pop();
                }
                PageInverse::PageDiscardsPush(node)
            }
            PageInverse::PageDiscardsReplace(mut old) => {
                std::mem::swap(&mut self.page_discards, &mut old);
                PageInverse::PageDiscardsReplace(old)
            }
            PageInverse::SplitDiscardsReplace(mut old) => {
                std::mem::swap(&mut self.split_discards, &mut old);
                PageInverse::SplitDiscardsReplace(old)
            }
            PageInverse::InsertionsReplace {
                mut insertions,
                mut positions,
            } => {
                std::mem::swap(&mut self.insertions, &mut insertions);
                std::mem::swap(&mut self.insertion_positions, &mut positions);
                PageInverse::InsertionsReplace {
                    insertions,
                    positions,
                }
            }
            PageInverse::InsertionUpsert { class, mut old } => {
                let current = self.page_insertion(class);
                self.restore_insertion(class, old);
                old = current;
                PageInverse::InsertionUpsert { class, old }
            }
            PageInverse::Marks(mut old) => {
                let current = [
                    self.top_mark.clone(),
                    self.first_mark.clone(),
                    self.bot_mark.clone(),
                    self.split_first_mark.clone(),
                    self.split_bot_mark.clone(),
                ];
                [
                    self.top_mark,
                    self.first_mark,
                    self.bot_mark,
                    self.split_first_mark,
                    self.split_bot_mark,
                ] = old;
                old = current;
                PageInverse::Marks(old)
            }
            PageInverse::MarkClass { class, mut old } => {
                let current = self
                    .mark_class_position(class)
                    .map(|position| self.mark_classes[position].1.clone());
                self.restore_mark_class(class, old);
                old = current;
                PageInverse::MarkClass { class, old }
            }
        };
        self.checkpoint_journal.inverses[index] = inverse;
    }

    fn restore_scalars(&mut self, old: PageScalars) {
        self.page_goal = old.dimensions[0];
        self.page_total = old.dimensions[1];
        self.page_stretch = old.dimensions[2];
        self.page_fil_stretch = old.dimensions[3];
        self.page_fill_stretch = old.dimensions[4];
        self.page_filll_stretch = old.dimensions[5];
        self.page_shrink = old.dimensions[6];
        self.page_depth = old.dimensions[7];
        self.page_max_depth = old.page_max_depth;
        self.contents = old.contents;
        self.last_glue = old.last_glue;
        self.last_penalty = old.last_penalty;
        self.last_kern = old.last_kern;
        self.last_node_type = old.last_node_type;
        self.insert_penalties = old.insert_penalties;
        self.dead_cycles = old.dead_cycles;
        self.least_page_cost = old.least_page_cost;
        self.best_page_break = old.best_page_break;
        self.best_size = old.best_size;
        self.fire_up = old.fire_up;
        self.tex82_dynamic_words = old.tex82_dynamic_words;
        self.etex_dynamic_words = old.etex_dynamic_words;
        self.page_node_root_count = old.page_node_root_count;
    }

    #[cfg(feature = "profiling")]
    pub(crate) const fn checkpoint_replay_work(&self) -> u64 {
        self.checkpoint_journal.replay_work
    }

    fn restore_insertion(&mut self, class: u16, old: Option<PageInsertion>) {
        if let Some(position) = self.page_insertion_position(class) {
            self.insertions.remove(position);
        }
        if let Some(insertion) = old {
            self.insertions.push(insertion);
            self.insertions.sort_by_key(PageInsertion::class);
        }
        self.rebuild_insertion_positions();
    }

    fn page_insertion_position(&self, class: u16) -> Option<usize> {
        self.insertion_positions
            .get(usize::from(class))
            .copied()
            .flatten()
            .map(usize::from)
    }

    fn rebuild_insertion_positions(&mut self) {
        self.insertion_positions.fill(None);
        for (position, insertion) in self.insertions.iter().enumerate() {
            let class = usize::from(insertion.class());
            if self.insertion_positions.len() <= class {
                self.insertion_positions.resize(class + 1, None);
            }
            self.insertion_positions[class] =
                Some(u16::try_from(position).expect("active insertion-class count fits u16"));
        }
    }

    fn restore_mark_class(&mut self, class: u16, old: Option<MarkClassState>) {
        if let Some(position) = self.mark_class_position(class) {
            self.remove_mark_class(class, position);
        }
        if let Some(state) = old {
            let position = self
                .mark_classes
                .binary_search_by_key(&class, |(active, _)| *active)
                .unwrap_or_else(|position| position);
            self.mark_classes.insert(position, (class, state));
            self.rebuild_mark_class_positions();
        }
    }

    fn rebuild_mark_class_positions(&mut self) {
        self.mark_class_positions.fill(None);
        for (position, (class, _)) in self.mark_classes.iter().enumerate() {
            let class = usize::from(*class);
            if self.mark_class_positions.len() <= class {
                self.mark_class_positions.resize(class + 1, None);
            }
            self.mark_class_positions[class] =
                Some(u16::try_from(position).expect("active mark-class count fits u16"));
        }
    }
    /// Whether this checkpointable page state explicitly carries any live
    /// page-arena coordinate. A rootless state contributes no retained-prefix
    /// demand merely because the arena cursor has advanced.
    pub(crate) fn retains_page_node_handles(&self) -> bool {
        self.page_node_root_count != 0
    }

    pub(crate) fn dynamic_memory_words(&self, etex_node_sizes: bool) -> usize {
        if etex_node_sizes {
            self.etex_dynamic_words
        } else {
            self.tex82_dynamic_words
        }
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
        self.record_scalars();
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
        self.record_scalars();
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
        if let Some(fork) = &self.checkpoint_journal.fork
            && let (Some(origin), Some(roots)) = (&fork.flat_origin, fork.target_roots)
        {
            return self
                .mark_lane
                .iter()
                .rev()
                .find(|record| record.class == 0 && record.mark == mark)
                .map(|record| record.value.as_ref())
                .unwrap_or_else(|| {
                    origin.mark_lane[..roots.mark_end]
                        .iter()
                        .rev()
                        .find(|record| record.class == 0 && record.mark == mark)
                        .and_then(|record| record.value.as_ref())
                });
        }
        match mark {
            PageMark::Top => self.top_mark.as_ref(),
            PageMark::First => self.first_mark.as_ref(),
            PageMark::Bot => self.bot_mark.as_ref(),
            PageMark::SplitFirst => self.split_first_mark.as_ref(),
            PageMark::SplitBot => self.split_bot_mark.as_ref(),
        }
    }

    pub(crate) fn set_mark(&mut self, mark: PageMark, value: NodeTokenList) {
        if !self.checkpoint_journal.frames.is_empty() {
            self.record_page_inverse(PageInverse::Marks([
                self.top_mark.clone(),
                self.first_mark.clone(),
                self.bot_mark.clone(),
                self.split_first_mark.clone(),
                self.split_bot_mark.clone(),
            ]));
        }
        self.mark_lane.push(MarkLaneRecord {
            class: 0,
            mark,
            value: Some(value.clone()),
        });
        *match mark {
            PageMark::Top => &mut self.top_mark,
            PageMark::First => &mut self.first_mark,
            PageMark::Bot => &mut self.bot_mark,
            PageMark::SplitFirst => &mut self.split_first_mark,
            PageMark::SplitBot => &mut self.split_bot_mark,
        } = Some(value);
    }

    pub(crate) fn clear_mark(&mut self, mark: PageMark) {
        if !self.checkpoint_journal.frames.is_empty() {
            self.record_page_inverse(PageInverse::Marks([
                self.top_mark.clone(),
                self.first_mark.clone(),
                self.bot_mark.clone(),
                self.split_first_mark.clone(),
                self.split_bot_mark.clone(),
            ]));
        }
        self.mark_lane.push(MarkLaneRecord {
            class: 0,
            mark,
            value: None,
        });
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
        if let Some(fork) = &self.checkpoint_journal.fork
            && let (Some(origin), Some(roots)) = (&fork.flat_origin, fork.target_roots)
        {
            return self
                .mark_lane
                .iter()
                .rev()
                .find(|record| record.class == class && record.mark == mark)
                .map(|record| record.value.as_ref())
                .unwrap_or_else(|| {
                    origin.mark_lane[..roots.mark_end]
                        .iter()
                        .rev()
                        .find(|record| record.class == class && record.mark == mark)
                        .and_then(|record| record.value.as_ref())
                });
        }
        self.mark_class_position(class)
            .and_then(|position| self.mark_classes[position].1.get(mark))
    }

    pub(crate) fn set_mark_class(&mut self, mark: PageMark, class: u16, value: NodeTokenList) {
        if class == 0 {
            self.set_mark(mark, value);
            return;
        }
        if !self.checkpoint_journal.frames.is_empty() {
            let old = self
                .mark_class_position(class)
                .map(|position| self.mark_classes[position].1.clone());
            self.record_page_inverse(PageInverse::MarkClass { class, old });
        }
        self.mark_lane.push(MarkLaneRecord {
            class,
            mark,
            value: Some(value.clone()),
        });
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
        if !self.checkpoint_journal.frames.is_empty() {
            self.record_page_inverse(PageInverse::MarkClass {
                class,
                old: Some(self.mark_classes[position].1.clone()),
            });
        }
        self.mark_lane.push(MarkLaneRecord {
            class,
            mark,
            value: None,
        });
        self.mark_classes[position].1.clear(mark);
        if self.mark_classes[position].1.is_empty() {
            self.remove_mark_class(class, position);
        }
    }

    pub(crate) fn mark_class_ids(&self) -> MarkClassIdIter<'_> {
        MarkClassIdIter {
            page: self,
            normal_index: 0,
            candidate_class: 1,
            candidate: self.checkpoint_journal.fork.is_some(),
        }
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
        self.record_scalars();
        if !self.checkpoint_journal.frames.is_empty() {
            let insertions = std::mem::take(&mut self.insertions);
            let positions = std::mem::take(&mut self.insertion_positions);
            self.record_page_inverse(PageInverse::InsertionsReplace {
                insertions,
                positions,
            });
        } else {
            self.insertions.clear();
            self.insertion_positions.clear();
        }
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
        self.record_scalars();
        self.insertion_lane
            .extend(self.insertions.iter().map(|insertion| InsertionLaneRecord {
                class: insertion.class(),
                value: None,
            }));
        let released = dynamic_words(self.current_page.iter());
        let released_page_roots = self
            .current_page
            .iter()
            .filter(|node| node_retains_page_handle(node))
            .count();
        if !self.checkpoint_journal.frames.is_empty() {
            let current_page = std::mem::take(&mut self.current_page);
            let insertions = std::mem::take(&mut self.insertions);
            let positions = std::mem::take(&mut self.insertion_positions);
            self.record_page_inverse(PageInverse::CurrentPageReplace(current_page));
            self.record_page_inverse(PageInverse::InsertionsReplace {
                insertions,
                positions,
            });
        } else {
            self.current_page.clear();
            self.insertions.clear();
            self.insertion_positions.clear();
        }
        self.release_dynamic_word_totals(released);
        self.page_node_root_count = self
            .page_node_root_count
            .checked_sub(released_page_roots)
            .expect("released more page roots than were live");
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
    }

    pub(crate) const fn contents(&self) -> PageContents {
        self.contents
    }

    pub(crate) fn set_contents(&mut self, contents: PageContents) {
        self.record_scalars();
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
        self.record_scalars();
        if !self.checkpoint_journal.frames.is_empty() {
            self.record_page_inverse(PageInverse::InsertionsReplace {
                insertions: self.insertions.clone(),
                positions: self.insertion_positions.clone(),
            });
        }
        self.best_page_break = Some(PageBreak::new(break_index));
        self.best_size = best_size;
        self.least_page_cost = cost;
        for insertion in &mut self.insertions {
            insertion.best_ins_index = insertion.last_ins_index;
        }
    }

    pub(crate) fn record_fire_up(&mut self, trigger_index: usize) {
        self.record_scalars();
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
        self.record_scalars();
        self.record_page_inverse(PageInverse::ContributionPushBack(None));
        self.allocate_dynamic_node(&node);
        self.contribution.push_back(node);
    }

    pub(crate) fn remove_contribution_range(
        &mut self,
        range: std::ops::RangeInclusive<usize>,
    ) -> Vec<Node> {
        let start = *range.start();
        let removed = self.contribution.drain(range).collect::<Vec<_>>();
        if !removed.is_empty() {
            self.record_scalars();
            self.record_page_inverse(PageInverse::ContributionRemoved {
                start,
                count: removed.len(),
                nodes: Some(removed.clone()),
            });
        }
        self.release_dynamic_nodes(&removed);
        removed
    }

    pub(crate) fn prepend_contribution(&mut self, node: Node) {
        self.record_scalars();
        self.record_page_inverse(PageInverse::ContributionPushFront(None));
        self.allocate_dynamic_node(&node);
        if let Some(fork) = &mut self.checkpoint_journal.fork
            && fork.flat_origin.is_some()
            && fork.target_roots.is_some()
        {
            fork.contribution_front.push_front(node);
            return;
        }
        self.contribution.push_front(node);
    }

    pub(crate) fn contribution(&self) -> PageContributionView<'_> {
        if let Some(fork) = &self.checkpoint_journal.fork
            && let (Some(origin), Some(roots)) = (&fork.flat_origin, fork.target_roots)
        {
            return PageContributionView {
                front: Some(&fork.contribution_front),
                prior: Some((
                    &origin.contribution,
                    roots.contribution_start,
                    roots.contribution_end,
                )),
                back: &self.contribution,
            };
        }
        PageContributionView {
            front: None,
            prior: None,
            back: &self.contribution,
        }
    }

    pub(crate) fn contribution_front(&self) -> Option<&Node> {
        self.contribution().front()
    }

    pub(crate) fn contribution_second(&self) -> Option<&Node> {
        self.contribution().get(1)
    }

    pub(crate) fn pop_contribution_front(&mut self) -> Option<Node> {
        if self
            .checkpoint_journal
            .fork
            .as_ref()
            .is_some_and(|fork| fork.flat_origin.is_some() && fork.target_roots.is_some())
        {
            let fork = self
                .checkpoint_journal
                .fork
                .as_mut()
                .expect("checked candidate page fork");
            let node = if let Some(node) = fork.contribution_front.pop_front() {
                node
            } else {
                let roots = fork.target_roots.as_mut().expect("checked page roots");
                if roots.contribution_start < roots.contribution_end {
                    let node = fork
                        .flat_origin
                        .as_ref()
                        .expect("checked accepted page owner")
                        .contribution[roots.contribution_start]
                        .clone();
                    roots.contribution_start += 1;
                    node
                } else {
                    self.contribution.pop_front()?
                }
            };
            self.release_dynamic_node(&node);
            return Some(node);
        }
        let node = self.contribution.pop_front()?;
        self.record_scalars();
        self.record_page_inverse(PageInverse::ContributionPoppedFront(Some(node.clone())));
        self.release_dynamic_node(&node);
        Some(node)
    }

    pub(crate) fn prepend_contributions(&mut self, nodes: Vec<Node>) {
        if nodes.is_empty() {
            return;
        }
        self.record_scalars();
        self.record_page_inverse(PageInverse::ContributionPrepended {
            count: nodes.len(),
            nodes: None,
        });
        self.allocate_dynamic_nodes(&nodes);
        if let Some(fork) = &mut self.checkpoint_journal.fork
            && fork.flat_origin.is_some()
            && fork.target_roots.is_some()
        {
            let old_front = std::mem::take(&mut fork.contribution_front);
            fork.contribution_front.extend(nodes);
            fork.contribution_front.extend(old_front);
            return;
        }
        let mut queue = VecDeque::with_capacity(nodes.len() + self.contribution.len());
        queue.extend(nodes);
        queue.extend(self.contribution.iter().cloned());
        self.contribution = queue;
    }

    pub(crate) fn current_page(&self) -> PageCurrentIter<'_> {
        let prior = self.checkpoint_journal.fork.as_ref().and_then(|fork| {
            Some((
                &fork.flat_origin.as_ref()?.current_page,
                fork.target_roots?.current_page_end,
            ))
        });
        let prior_len = prior.map_or(0, |(_, end)| end);
        PageCurrentIter {
            prior,
            current: &self.current_page,
            front: 0,
            back: prior_len + self.current_page.len(),
        }
    }

    pub(crate) fn push_page_discard(&mut self, node: Node) {
        self.record_scalars();
        self.record_page_inverse(PageInverse::PageDiscardsPush(None));
        self.allocate_dynamic_node(&node);
        self.page_discards.push(node);
    }

    pub(crate) fn take_page_discards(&mut self) -> Vec<Node> {
        self.record_scalars();
        if let Some(fork) = &mut self.checkpoint_journal.fork
            && let (Some(origin), Some(roots)) = (&fork.flat_origin, &mut fork.target_roots)
        {
            let mut nodes = origin.page_discards[..roots.page_discards_end].to_vec();
            roots.page_discards_end = 0;
            nodes.append(&mut self.page_discards);
            self.release_dynamic_nodes(&nodes);
            return nodes;
        }
        if !self.checkpoint_journal.frames.is_empty() {
            self.record_page_inverse(PageInverse::PageDiscardsReplace(self.page_discards.clone()));
        }
        let nodes = std::mem::take(&mut self.page_discards);
        self.release_dynamic_nodes(&nodes);
        nodes
    }

    pub(crate) fn clear_page_discards(&mut self) {
        self.record_scalars();
        if let Some(fork) = &mut self.checkpoint_journal.fork
            && let Some(roots) = &mut fork.target_roots
        {
            roots.page_discards_end = 0;
            let nodes = std::mem::take(&mut self.page_discards);
            self.release_dynamic_nodes(&nodes);
            return;
        }
        let nodes = std::mem::take(&mut self.page_discards);
        self.release_dynamic_nodes(&nodes);
        if !self.checkpoint_journal.frames.is_empty() {
            self.record_page_inverse(PageInverse::PageDiscardsReplace(nodes));
        }
    }

    pub(crate) fn set_split_discards(&mut self, nodes: Vec<Node>) {
        self.record_scalars();
        let added = dynamic_words(nodes.iter());
        let added_page_roots = nodes
            .iter()
            .filter(|node| node_retains_page_handle(node))
            .count();
        let old = std::mem::replace(&mut self.split_discards, nodes);
        self.release_dynamic_nodes(&old);
        if !self.checkpoint_journal.frames.is_empty() {
            self.record_page_inverse(PageInverse::SplitDiscardsReplace(old));
        }
        self.allocate_dynamic_word_totals(added);
        self.page_node_root_count = self
            .page_node_root_count
            .checked_add(added_page_roots)
            .expect("page root accounting overflow");
    }

    pub(crate) fn take_split_discards(&mut self) -> Vec<Node> {
        self.record_scalars();
        if let Some(fork) = &mut self.checkpoint_journal.fork
            && let (Some(origin), Some(roots)) = (&fork.flat_origin, &mut fork.target_roots)
        {
            let mut nodes = origin.split_discards[..roots.split_discards_end].to_vec();
            roots.split_discards_end = 0;
            nodes.append(&mut self.split_discards);
            self.release_dynamic_nodes(&nodes);
            return nodes;
        }
        if !self.checkpoint_journal.frames.is_empty() {
            self.record_page_inverse(PageInverse::SplitDiscardsReplace(
                self.split_discards.clone(),
            ));
        }
        let nodes = std::mem::take(&mut self.split_discards);
        self.release_dynamic_nodes(&nodes);
        nodes
    }

    pub(crate) fn clear_split_discards(&mut self) {
        self.record_scalars();
        if let Some(fork) = &mut self.checkpoint_journal.fork
            && let Some(roots) = &mut fork.target_roots
        {
            roots.split_discards_end = 0;
            let nodes = std::mem::take(&mut self.split_discards);
            self.release_dynamic_nodes(&nodes);
            return;
        }
        let nodes = std::mem::take(&mut self.split_discards);
        self.release_dynamic_nodes(&nodes);
        if !self.checkpoint_journal.frames.is_empty() {
            self.record_page_inverse(PageInverse::SplitDiscardsReplace(nodes));
        }
    }

    pub(crate) fn current_page_tail(&self) -> Option<&Node> {
        self.current_page().next_back()
    }

    pub(crate) fn current_page_len(&self) -> usize {
        self.current_page().len()
    }

    pub(crate) fn push_current_page(&mut self, node: Node) {
        self.record_scalars();
        self.record_page_inverse(PageInverse::CurrentPagePush(None));
        self.allocate_dynamic_node(&node);
        self.current_page.push(node);
    }

    pub(crate) fn page_insertions(&self) -> PageInsertionView<'_> {
        PageInsertionView { page: self }
    }

    pub(crate) fn page_insertion(&self, class: u16) -> Option<PageInsertion> {
        if let Some(fork) = &self.checkpoint_journal.fork
            && let (Some(origin), Some(roots)) = (&fork.flat_origin, fork.target_roots)
        {
            return self
                .insertion_lane
                .iter()
                .rev()
                .find(|record| record.class == class)
                .map(|record| record.value)
                .unwrap_or_else(|| {
                    origin.insertion_lane[..roots.insertion_end]
                        .iter()
                        .rev()
                        .find(|record| record.class == class)
                        .and_then(|record| record.value)
                });
        }
        self.insertion_positions
            .get(usize::from(class))
            .copied()
            .flatten()
            .map(|index| self.insertions[usize::from(index)])
    }

    pub(crate) fn upsert_page_insertion(&mut self, insertion: PageInsertion) {
        let class = insertion.class();
        let old = (!self.checkpoint_journal.frames.is_empty())
            .then(|| self.page_insertion(class))
            .flatten();
        self.insertion_lane.push(InsertionLaneRecord {
            class,
            value: Some(insertion),
        });
        if !self.checkpoint_journal.frames.is_empty() {
            self.record_page_inverse(PageInverse::InsertionUpsert { class, old });
        }
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
        self.record_scalars();
        if let Some(fork) = &mut self.checkpoint_journal.fork
            && let (Some(origin), Some(roots)) = (&fork.flat_origin, &mut fork.target_roots)
        {
            let mut logical = origin
                .current_page
                .iter()
                .take(roots.current_page_end)
                .cloned()
                .collect::<Vec<_>>();
            logical.extend(self.current_page.iter().cloned());
            roots.current_page_end = 0;
            self.current_page.clear();
            let split_index = split_index.min(logical.len());
            let after = logical.split_off(split_index);
            self.release_dynamic_nodes(&logical);
            self.release_dynamic_nodes(&after);
            return (logical, after);
        }
        if !self.checkpoint_journal.frames.is_empty() {
            self.record_page_inverse(PageInverse::CurrentPageReplace(self.current_page.clone()));
        }
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
        self.page_node_root_count = self
            .page_node_root_count
            .checked_add(usize::from(node_retains_page_handle(node)))
            .expect("page root accounting overflow");
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
        self.page_node_root_count = self
            .page_node_root_count
            .checked_sub(usize::from(node_retains_page_handle(node)))
            .expect("released more page roots than were live");
    }

    fn allocate_dynamic_nodes(&mut self, nodes: &[Node]) {
        self.allocate_dynamic_word_totals(dynamic_words(nodes.iter()));
        self.page_node_root_count = self
            .page_node_root_count
            .checked_add(
                nodes
                    .iter()
                    .filter(|node| node_retains_page_handle(node))
                    .count(),
            )
            .expect("page root accounting overflow");
    }

    fn release_dynamic_nodes(&mut self, nodes: &[Node]) {
        self.release_dynamic_word_totals(dynamic_words(nodes.iter()));
        self.page_node_root_count = self
            .page_node_root_count
            .checked_sub(
                nodes
                    .iter()
                    .filter(|node| node_retains_page_handle(node))
                    .count(),
            )
            .expect("released more page roots than were live");
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
        self.record_scalars();
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
