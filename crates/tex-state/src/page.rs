//! Snapshot-owned page-builder state.

#[cfg(test)]
mod state_hash;

use crate::fork_arena::{CheckpointMark, ChunkPool, ForkArena, ForkArenaCounters, ForkArenaError};
use crate::glue::GlueSpec;
use crate::node::{Node, NodeTokenList};
use crate::node_arena::{NodeCursor, NodeCursorIter, PageListId, PageNodeArena};
use crate::node_region::{NodeCheckpointMark, NodePool, NodeRegionId, PageClosureBuildMark};
use crate::node_sequence::SemanticSequenceIdentity;
use crate::page_node_arena::{PageListSpan, PageMaterialRegion, PageMaterialView};
use crate::scaled::Scaled;
use ahash::RandomState;
use serde::{Deserialize, Serialize};
use std::hash::{BuildHasher, Hash, Hasher};
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

/// Opaque page-builder position bracketing one TeX82 §1054 end-job
/// ejection.
///
/// This is a semantic state coordinate, not a wall-clock or retry count. It
/// advances only when §994 consumes a contribution or §1012 changes the
/// current page/output state, and it is restored with the page-builder
/// journal. A caller may therefore prove that a backed-up `\end` reached a
/// different page-builder state before it is delivered again.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PageBuilderProgressToken(u64);

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

/// Borrowed canonical contribution list.
#[derive(Clone, Copy)]
pub struct PageContributionView<'a> {
    nodes: NodeCursor<'a>,
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
            candidate: false,
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
        self.nodes.len()
    }

    #[must_use]
    pub fn is_empty(self) -> bool {
        self.len() == 0
    }

    #[must_use]
    pub fn get(self, index: usize) -> Option<crate::NodeView<'a>> {
        self.nodes.get(index)
    }

    #[must_use]
    pub fn front(self) -> Option<crate::NodeView<'a>> {
        self.get(0)
    }

    #[must_use]
    pub fn back(self) -> Option<crate::NodeView<'a>> {
        self.len().checked_sub(1).and_then(|index| self.get(index))
    }

    pub fn iter(self) -> PageContributionIter<'a> {
        self.nodes.iter()
    }

    #[must_use]
    pub fn to_vec(self) -> Vec<Node> {
        self.iter()
            .map(|node| node.to_owned_with(std::convert::identity))
            .collect()
    }
}

pub type PageContributionIter<'a> = NodeCursorIter<'a>;
pub(crate) type PageCurrentIter<'a> = NodeCursorIter<'a>;

/// Snapshot-owned state for TeX.web's page builder.
pub(crate) struct PageBuilderState {
    contribution: PageListSpan,
    current_page: PageListSpan,
    page_discards: PageListSpan,
    split_discards: PageListSpan,
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
    progress: PageBuilderProgressToken,
    end_job_ejection: Option<PageBuilderProgressToken>,
    resume_after_output: bool,
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
    page_node_root_count: usize,
    identity_enabled: bool,
    semantic_roots: PageSemanticRoots,
    checkpoint_journal: PageCheckpointJournal,
    /// Suffix opened after box255 packaging. Production succession consumes
    /// it to move a self-contained next-page closure or to select the one
    /// explicit interleaved-prefix copy fallback.
    output_successor_build: Option<PageClosureBuildMark>,
}

/// One node detached from a page lane together with its coarse journal move
/// coordinate. The coordinate is meaningful only to the page generation that
/// produced it; callers may inspect the node but must return the carrier to a
/// page destination or explicitly discard it through [`CommandContext`].
#[must_use = "a detached page node must be returned to a page destination or explicitly discarded"]
#[derive(Debug)]
pub struct PageNodeCarrier {
    list: PageListSpan,
}

impl PageNodeCarrier {
    #[must_use]
    pub const fn list(&self) -> PageListId {
        self.list.list()
    }
}

/// Bounded root of one page-builder state on its generation timeline.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PageCheckpointMark {
    timeline: u64,
    frame: u64,
    cursor: usize,
    journal: CheckpointMark<PageBuilderJournalLane>,
    scalars: PageScalars,
    roots: PagePayloadRoots,
    semantic_roots: PageSemanticRoots,
    reachable_state_identity_root: Option<u64>,
}

#[derive(Clone, Copy, Debug, Default)]
struct PagePayloadRoots {
    contribution: PageListSpan,
    current_page: PageListSpan,
    page_discards: PageListSpan,
    split_discards: PageListSpan,
}

struct PageCheckpointFrame {
    id: u64,
    cursor: usize,
}

struct PageCheckpointJournal {
    timeline: u64,
    next_frame: u64,
    frames: Vec<PageCheckpointFrame>,
    inverse_pool: ChunkPool<PageInverse>,
    inverses: ForkArena<PageInverse, PageBuilderJournalLane>,
    applied: usize,
    candidate_root_frame: Option<u64>,
    replay_work: u64,
    accepted_rewind_work: u64,
    accepted_redo_work: u64,
    accepted_rewind_transitions: u64,
    accepted_redo_transitions: u64,
    checkpoint_captures: u64,
    candidate_acceptances: u64,
    candidate_rejections: u64,
    candidate_rewind_work: u64,
}

pub(crate) struct AcceptedPageTail {
    origin: usize,
    selected: usize,
    prefix_frames: usize,
    accepted_rewind_work: u64,
    future_frames: Vec<PageCheckpointFrame>,
    origin_scalars: PageScalars,
    origin_semantic_roots: PageSemanticRoots,
    journal: CheckpointMark<PageBuilderJournalLane>,
}

enum PageBuilderJournalLane {}

enum PageInverse {
    Noop,
    Scalars(PageScalars),
    Contribution(PageListSpan),
    CurrentPage(PageListSpan),
    PageDiscards(PageListSpan),
    SplitDiscards(PageListSpan),
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
    progress: PageBuilderProgressToken,
    end_job_ejection: Option<PageBuilderProgressToken>,
    resume_after_output: bool,
    tex82_dynamic_words: usize,
    etex_dynamic_words: usize,
    page_node_root_count: usize,
}

/// Borrowed mutable projection of the canonical values changed by one page
/// inverse. Keeping this projection disjoint from `PageCheckpointJournal`
/// lets the chunk-owned journal swap values in place without moving the
/// journal out of `PageBuilderState` or creating a second canonical state.
struct PageInverseTarget<'a> {
    roots: [&'a mut PageListSpan; 4],
    dimensions: [&'a mut Scaled; 9],
    contents: &'a mut PageContents,
    last_glue: &'a mut Option<GlueSpec>,
    last_penalty: &'a mut i32,
    last_kern: &'a mut Scaled,
    last_node_type: &'a mut i32,
    insert_penalties: &'a mut i32,
    dead_cycles: &'a mut i32,
    least_page_cost: &'a mut i32,
    best_page_break: &'a mut Option<PageBreak>,
    best_size: &'a mut Scaled,
    fire_up: &'a mut Option<PageFireUp>,
    progress: &'a mut PageBuilderProgressToken,
    end_job_ejection: &'a mut Option<PageBuilderProgressToken>,
    resume_after_output: &'a mut bool,
    tex82_dynamic_words: &'a mut usize,
    etex_dynamic_words: &'a mut usize,
    page_node_root_count: &'a mut usize,
    insertions: &'a mut Vec<PageInsertion>,
    insertion_positions: &'a mut Vec<Option<u16>>,
    marks: [&'a mut Option<NodeTokenList>; 5],
    mark_classes: &'a mut Vec<(u16, MarkClassState)>,
    mark_class_positions: &'a mut Vec<Option<u16>>,
}

impl PageInverseTarget<'_> {
    fn scalar_snapshot(&self) -> PageScalars {
        PageScalars {
            dimensions: [
                *self.dimensions[0],
                *self.dimensions[1],
                *self.dimensions[2],
                *self.dimensions[3],
                *self.dimensions[4],
                *self.dimensions[5],
                *self.dimensions[6],
                *self.dimensions[7],
            ],
            page_max_depth: *self.dimensions[8],
            contents: *self.contents,
            last_glue: *self.last_glue,
            last_penalty: *self.last_penalty,
            last_kern: *self.last_kern,
            last_node_type: *self.last_node_type,
            insert_penalties: *self.insert_penalties,
            dead_cycles: *self.dead_cycles,
            least_page_cost: *self.least_page_cost,
            best_page_break: *self.best_page_break,
            best_size: *self.best_size,
            fire_up: *self.fire_up,
            progress: *self.progress,
            end_job_ejection: *self.end_job_ejection,
            resume_after_output: *self.resume_after_output,
            tex82_dynamic_words: *self.tex82_dynamic_words,
            etex_dynamic_words: *self.etex_dynamic_words,
            page_node_root_count: *self.page_node_root_count,
        }
    }

    fn restore_scalars(&mut self, old: PageScalars) {
        for (destination, value) in self.dimensions.iter_mut().take(8).zip(old.dimensions) {
            **destination = value;
        }
        *self.dimensions[8] = old.page_max_depth;
        *self.contents = old.contents;
        *self.last_glue = old.last_glue;
        *self.last_penalty = old.last_penalty;
        *self.last_kern = old.last_kern;
        *self.last_node_type = old.last_node_type;
        *self.insert_penalties = old.insert_penalties;
        *self.dead_cycles = old.dead_cycles;
        *self.least_page_cost = old.least_page_cost;
        *self.best_page_break = old.best_page_break;
        *self.best_size = old.best_size;
        *self.fire_up = old.fire_up;
        *self.progress = old.progress;
        *self.end_job_ejection = old.end_job_ejection;
        *self.resume_after_output = old.resume_after_output;
        *self.tex82_dynamic_words = old.tex82_dynamic_words;
        *self.etex_dynamic_words = old.etex_dynamic_words;
        *self.page_node_root_count = old.page_node_root_count;
    }

    fn page_insertion(&self, class: u16) -> Option<PageInsertion> {
        self.insertion_positions
            .get(usize::from(class))
            .copied()
            .flatten()
            .map(|index| self.insertions[usize::from(index)])
    }

    fn restore_insertion(&mut self, class: u16, old: Option<PageInsertion>) {
        let current = self
            .insertion_positions
            .get(usize::from(class))
            .copied()
            .flatten()
            .map(usize::from);
        match (current, old) {
            (Some(position), Some(insertion)) => self.insertions[position] = insertion,
            (Some(position), None) => {
                self.insertions.remove(position);
                self.insertion_positions[usize::from(class)] = None;
                for (shifted, insertion) in self.insertions[position..].iter().enumerate() {
                    self.insertion_positions[usize::from(insertion.class())] = Some(
                        u16::try_from(position + shifted)
                            .expect("active insertion-class count fits u16"),
                    );
                }
            }
            (None, Some(insertion)) => {
                let position = self
                    .insertions
                    .binary_search_by_key(&class, PageInsertion::class)
                    .unwrap_or_else(|position| position);
                self.insertions.insert(position, insertion);
                if self.insertion_positions.len() <= usize::from(class) {
                    self.insertion_positions
                        .resize(usize::from(class).saturating_add(1), None);
                }
                for (shifted, insertion) in self.insertions[position..].iter().enumerate() {
                    self.insertion_positions[usize::from(insertion.class())] = Some(
                        u16::try_from(position + shifted)
                            .expect("active insertion-class count fits u16"),
                    );
                }
            }
            (None, None) => {}
        }
    }

    fn mark_class_position(&self, class: u16) -> Option<usize> {
        self.mark_class_positions
            .get(usize::from(class))
            .copied()
            .flatten()
            .map(usize::from)
    }

    fn swap_mark_class(&mut self, class: u16, old: &mut Option<MarkClassState>) {
        let current = self.mark_class_position(class);
        match (current, old.as_mut()) {
            (Some(position), Some(state)) => {
                std::mem::swap(&mut self.mark_classes[position].1, state);
            }
            (Some(position), None) => {
                let (_, state) = self.mark_classes.remove(position);
                *old = Some(state);
                self.mark_class_positions[usize::from(class)] = None;
                for (shifted, (class, _)) in self.mark_classes[position..].iter().enumerate() {
                    self.mark_class_positions[usize::from(*class)] = Some(
                        u16::try_from(position + shifted)
                            .expect("active mark-class count fits u16"),
                    );
                }
            }
            (None, Some(_)) => {
                let position = self
                    .mark_classes
                    .binary_search_by_key(&class, |(active, _)| *active)
                    .unwrap_or_else(|position| position);
                let state = old.take().expect("matched a retained mark class");
                self.mark_classes.insert(position, (class, state));
                if self.mark_class_positions.len() <= usize::from(class) {
                    self.mark_class_positions
                        .resize(usize::from(class).saturating_add(1), None);
                }
                for (shifted, (class, _)) in self.mark_classes[position..].iter().enumerate() {
                    self.mark_class_positions[usize::from(*class)] = Some(
                        u16::try_from(position + shifted)
                            .expect("active mark-class count fits u16"),
                    );
                }
            }
            (None, None) => {}
        }
    }

    fn toggle(&mut self, inverse: &mut PageInverse) {
        let old = std::mem::replace(inverse, PageInverse::Noop);
        *inverse = match old {
            PageInverse::Noop => unreachable!("page inverse slot is occupied"),
            PageInverse::Scalars(old) => {
                let current = self.scalar_snapshot();
                self.restore_scalars(old);
                PageInverse::Scalars(current)
            }
            PageInverse::Contribution(mut old) => {
                std::mem::swap(self.roots[0], &mut old);
                PageInverse::Contribution(old)
            }
            PageInverse::CurrentPage(mut old) => {
                std::mem::swap(self.roots[1], &mut old);
                PageInverse::CurrentPage(old)
            }
            PageInverse::PageDiscards(mut old) => {
                std::mem::swap(self.roots[2], &mut old);
                PageInverse::PageDiscards(old)
            }
            PageInverse::SplitDiscards(mut old) => {
                std::mem::swap(self.roots[3], &mut old);
                PageInverse::SplitDiscards(old)
            }
            PageInverse::InsertionsReplace {
                mut insertions,
                mut positions,
            } => {
                std::mem::swap(self.insertions, &mut insertions);
                std::mem::swap(self.insertion_positions, &mut positions);
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
                for (destination, retained) in self.marks.iter_mut().zip(old.iter_mut()) {
                    std::mem::swap(*destination, retained);
                }
                PageInverse::Marks(old)
            }
            PageInverse::MarkClass { class, mut old } => {
                self.swap_mark_class(class, &mut old);
                PageInverse::MarkClass { class, old }
            }
        };
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct PageSemanticRoots {
    contribution: SemanticSequenceIdentity,
    page_discards: SemanticSequenceIdentity,
    split_discards: SemanticSequenceIdentity,
    insertions: u64,
    marks: u64,
}

/// Owner-relative handle for one retained page checkpoint row.
///
/// The key intentionally carries no raw list or arena coordinate.  Those
/// remain private rows under the matching [`PageRegion`] owner.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PageRegionCheckpointKey {
    region: NodeRegionId,
    boundary: u64,
}

#[derive(Clone, Copy)]
struct PageRegionCheckpoint {
    key: PageRegionCheckpointKey,
    nodes: NodeCheckpointMark,
    builder: PageCheckpointMark,
}

/// Scalar observations of explicit page-region lifecycle transitions.
///
/// These counters never participate in liveness decisions.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PageRegionCounters {
    /// Page-building regions opened.
    pub page_regions_started: u64,
    /// Old page regions moved into checkpoint history.
    pub page_regions_retained: u64,
    /// Old page regions dropped after output or final pruning.
    pub page_regions_dropped: u64,
    /// Exact edit forks opened in a selected page region.
    pub page_region_forks: u64,
    /// Later accepted page regions detached wholesale at edit start.
    pub later_page_regions_detached: u64,
    /// Nodes copied during explicit held-over evacuation.
    pub held_over_nodes_copied: u64,
    /// Independently transferable held-over envelopes moved without copying.
    pub held_over_envelopes_moved: u64,
    /// Page-to-durable nodes copied only by a structural fallback.
    pub page_to_durable_nodes_copied: u64,
    /// Nodes copied by TeX's explicit `\copy`/`\unhcopy` operations.
    pub tex_copy_nodes_copied: u64,
    /// Nodes copied because retained checkpoint/group history owns the source.
    pub history_preservation_nodes_copied: u64,
    /// Nodes copied while settling a non-self-contained nested closure.
    pub nested_closure_nodes_copied: u64,
    /// Nodes visited by bounded closure transfer/copy validation.
    pub node_closure_scan_nodes: u64,
    /// Foreign node roots rejected at a region boundary.
    pub cross_region_node_reference_rejections: u64,
    /// Private retained checkpoint rows released by the outer history owner.
    pub checkpoint_rows_released: u64,
}

/// Exact work attributed to PageBuilder checkpoint publication and candidate
/// settlement. Zero-valued scan/copy fields are explicit regression guards:
/// they describe forbidden whole-lane work rather than omitted measurement.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PageCandidateSettlementCounters {
    pub checkpoint_captures: u64,
    pub checkpoint_capture_records_scanned: u64,
    pub candidate_selections: u64,
    pub selected_journal_records_rewound: u64,
    pub candidate_acceptances: u64,
    pub acceptance_payload_records_scanned: u64,
    pub candidate_rejections: u64,
    pub rejected_candidate_records_rewound: u64,
    pub rejected_prior_records_redone: u64,
    pub canonical_lane_records_scanned: u64,
    pub canonical_values_copied: u64,
    pub prior_journal_chunks_released: u64,
    pub candidate_journal_chunks_released: u64,
}

/// Work and remaining storage after releasing one private page checkpoint row.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PageRegionReleaseReceipt {
    pub rows_released: u64,
    pub regions_retired: u64,
    pub retained_regions: usize,
    pub retained_rows: usize,
    pub row_capacity: usize,
}

/// Exclusive owner for one page-building period.
///
/// Page node payload/descriptors, all four live PageBuilder roots and scalar
/// state, its reversible journal, and every retained row for the period move
/// together in this aggregate.  Raw page roots are therefore never sibling
/// owners in `Universe`.
pub struct PageRegion {
    nodes: PageMaterialRegion,
    builder: PageBuilderState,
    checkpoints: Vec<PageRegionCheckpoint>,
    next_boundary: u64,
    counters: PageRegionCounters,
}

pub(crate) struct AcceptedPageRegionTail {
    builder: AcceptedPageTail,
    selected_boundary: u64,
    later_rows: Vec<PageRegionCheckpoint>,
}

/// Move-only document-order ownership of accepted page-building periods.
///
/// The final entry is the only current region. Earlier entries exist only
/// because their contiguous checkpoint interval is non-empty; no node-root
/// census participates in their retention.
pub(crate) struct PageRegionHistory {
    pool: NodePool,
    regions: Vec<PageRegion>,
    pending_successor: Option<PreparedPageRegionSuccessor>,
    candidate_counters: PageCandidateSettlementCounters,
}

pub(crate) struct AcceptedPageRegionHistoryTail {
    selected: AcceptedPageRegionTail,
    later_regions: Vec<PageRegion>,
    selected_region: NodeRegionId,
}

/// Result of completing one page lifetime and opening its successor.
pub struct PageRegionSuccession {
    current: PageRegion,
    retained_prior: Option<PageRegion>,
    held_over: PageListId,
}

/// Proof that the live executor mode nest admitted every root against the
/// current page region and retained no root across page succession.
///
/// The constructor is kept at the admitted [`crate::CommandContext`]
/// boundary; production succession consumes the receipt instead of accepting
/// an unchecked boolean or a naked region id.
pub struct ModeListRegionPreflight {
    pub(crate) region: NodeRegionId,
}

struct PreparedPageRegionSuccessor {
    current: Option<PageRegion>,
    /// Test-only compatibility projection for the focused region seam. The
    /// production transition carries the complete PageBuilder root set and
    /// therefore has no naked root to return.
    held_over: Option<PageListId>,
    /// A completely consumed predecessor whose construction suffix can keep
    /// its arena identity, sealed chunks, and mutable partial tail.
    unique_adoption: Option<(PageClosureBuildMark, PagePayloadRoots)>,
    /// A retained predecessor whose sealed arena prefix becomes the immutable
    /// prefix of one isolated current lineage.
    shared_prefix: Option<(PageClosureBuildMark, PagePayloadRoots)>,
}

impl Default for PageRegionHistory {
    fn default() -> Self {
        let mut pool = NodePool::new();
        let current = PageRegion::new(&mut pool);
        Self {
            pool,
            regions: vec![current],
            pending_successor: None,
            candidate_counters: PageCandidateSettlementCounters::default(),
        }
    }
}

impl PageRegionHistory {
    pub(crate) fn current(&self) -> &PageRegion {
        self.regions
            .last()
            .expect("page history always has a current region")
    }

    fn current_mut(&mut self) -> &mut PageRegion {
        self.regions
            .last_mut()
            .expect("page history always has a current region")
    }

    fn region(&self, key: PageRegionCheckpointKey) -> Option<&PageRegion> {
        self.regions.iter().find(|region| region.id() == key.region)
    }

    pub(crate) fn nodes(&self) -> PageMaterialView<'_> {
        PageMaterialView::new(&self.pool, &self.current().nodes)
    }

    pub(crate) fn nodes_mut(&mut self) -> PageNodeArena<'_> {
        let current = self
            .regions
            .last_mut()
            .expect("page history always has a current region");
        PageNodeArena::new(&mut self.pool, &mut current.nodes)
    }

    pub(crate) fn builder(&self) -> &PageBuilderState {
        self.current().builder()
    }

    pub(crate) fn builder_mut(&mut self) -> &mut PageBuilderState {
        self.current_mut().builder_mut()
    }

    pub(crate) fn release_rootless_current_suffix(&mut self) -> Result<usize, ForkArenaError> {
        let current = self
            .regions
            .last_mut()
            .expect("page history always has a current region");
        if current.builder.retains_page_node_handles() {
            return Err(ForkArenaError::InvalidRegion);
        }
        let retained = current
            .checkpoints
            .last()
            .map(|checkpoint| checkpoint.nodes);
        current
            .nodes
            .release_rootless_suffix(&mut self.pool, retained)
    }

    pub(crate) fn parts_mut(&mut self) -> (PageNodeArena<'_>, &mut PageBuilderState) {
        let current = self
            .regions
            .last_mut()
            .expect("page history always has a current region");
        (
            PageNodeArena::new(&mut self.pool, &mut current.nodes),
            &mut current.builder,
        )
    }

    pub(crate) fn seal_checkpoint(&mut self) -> Result<PageRegionCheckpointKey, ForkArenaError> {
        let current = self
            .regions
            .last_mut()
            .expect("page history always has a current region");
        let key = current.seal_checkpoint(&mut self.pool)?;
        self.candidate_counters.checkpoint_captures = self
            .candidate_counters
            .checkpoint_captures
            .saturating_add(1);
        Ok(key)
    }

    pub(crate) fn validates_checkpoint(&self, key: PageRegionCheckpointKey) -> bool {
        self.region(key)
            .is_some_and(|region| region.validates_checkpoint(&self.pool, key))
    }

    /// Releases the private row named by an outer retained checkpoint.
    ///
    /// The key's region/boundary pair identifies the row directly; node roots
    /// are never traversed.  When a noncurrent region loses its final row, its
    /// complete node-region envelopes are returned to the shared pool before
    /// the region descriptor is removed, generation-invalidating every stale
    /// coordinate into that owner.
    pub(crate) fn release_checkpoint(
        &mut self,
        key: PageRegionCheckpointKey,
    ) -> Result<PageRegionReleaseReceipt, ForkArenaError> {
        if self.pending_successor.is_some() {
            return Err(ForkArenaError::ActiveBatch);
        }
        let region_index = self
            .regions
            .iter()
            .position(|region| region.id() == key.region)
            .ok_or(ForkArenaError::InvalidCheckpoint)?;
        let builder_mark = self.regions[region_index].release_checkpoint_row(key)?;
        self.regions[region_index]
            .builder_mut()
            .commit_transaction(builder_mark);
        self.regions[region_index].counters.checkpoint_rows_released = self.regions[region_index]
            .counters
            .checkpoint_rows_released
            .saturating_add(1);

        let mut regions_retired = 0;
        let current = self.regions.len().saturating_sub(1);
        if region_index != current && self.regions[region_index].checkpoints.is_empty() {
            let region = self.regions.remove(region_index);
            region.retire(&mut self.pool)?;
            regions_retired = 1;
            self.current_mut().counters.page_regions_dropped = self
                .current()
                .counters
                .page_regions_dropped
                .saturating_add(1);
        }
        Ok(PageRegionReleaseReceipt {
            rows_released: 1,
            regions_retired,
            retained_regions: self.regions.len(),
            retained_rows: self
                .regions
                .iter()
                .map(|region| region.checkpoints.len())
                .sum(),
            row_capacity: self
                .regions
                .iter()
                .map(|region| region.checkpoints.capacity())
                .sum(),
        })
    }

    pub(crate) fn validates_node_checkpoint(
        &self,
        key: PageRegionCheckpointKey,
        mark: NodeCheckpointMark,
    ) -> bool {
        self.region(key).is_some_and(|region| {
            region.checkpoint(key).is_some_and(|checkpoint| {
                checkpoint.nodes == mark
                    && PageMaterialView::new(&self.pool, &region.nodes).can_restore_checkpoint(mark)
            })
        })
    }

    pub(crate) fn arena_checkpoint(
        &self,
        key: PageRegionCheckpointKey,
    ) -> Option<NodeCheckpointMark> {
        self.region(key)
            .and_then(|region| region.arena_checkpoint(key))
    }

    pub(crate) fn checkpoint_identity_root(
        &self,
        key: PageRegionCheckpointKey,
    ) -> Option<Option<u64>> {
        self.region(key)
            .and_then(|region| region.checkpoint_identity_root(key))
    }

    pub(crate) fn begin_checkpoint_candidate(
        &mut self,
        key: PageRegionCheckpointKey,
    ) -> Result<AcceptedPageRegionHistoryTail, ForkArenaError> {
        let selected = self
            .regions
            .iter()
            .position(|region| region.validates_checkpoint(&self.pool, key))
            .ok_or(ForkArenaError::InvalidCheckpoint)?;
        let before = self.regions[selected]
            .builder
            .candidate_settlement_counters();
        let later_regions = self.regions.split_off(selected.saturating_add(1));
        let selected_region = self.current().id();
        self.current_mut().counters.later_page_regions_detached = self
            .current()
            .counters
            .later_page_regions_detached
            .saturating_add(later_regions.len() as u64);
        let selected = self
            .regions
            .last_mut()
            .expect("page history always has a current region")
            .begin_checkpoint_candidate(&mut self.pool, key)?;
        let after = self.current().builder.candidate_settlement_counters();
        self.candidate_counters.candidate_selections = self
            .candidate_counters
            .candidate_selections
            .saturating_add(after.candidate_selections - before.candidate_selections);
        self.candidate_counters.selected_journal_records_rewound = self
            .candidate_counters
            .selected_journal_records_rewound
            .saturating_add(
                after.selected_journal_records_rewound - before.selected_journal_records_rewound,
            );
        Ok(AcceptedPageRegionHistoryTail {
            selected,
            later_regions,
            selected_region,
        })
    }

    pub(crate) fn reject_checkpoint_candidate(
        &mut self,
        mut tail: AcceptedPageRegionHistoryTail,
    ) -> Result<(), ForkArenaError> {
        let selected = self
            .regions
            .iter()
            .position(|region| region.id() == tail.selected_region)
            .ok_or(ForkArenaError::InvalidRegion)?;
        self.regions.truncate(selected.saturating_add(1));
        let before = self
            .regions
            .last()
            .expect("selected page region remains current")
            .builder
            .candidate_settlement_counters();
        self.regions
            .last_mut()
            .expect("page history always has a current region")
            .reject_checkpoint_candidate(&mut self.pool, tail.selected)?;
        let after = self
            .regions
            .last()
            .expect("selected page region remains current")
            .builder
            .candidate_settlement_counters();
        self.candidate_counters.candidate_rejections = self
            .candidate_counters
            .candidate_rejections
            .saturating_add(after.candidate_rejections - before.candidate_rejections);
        self.candidate_counters.rejected_candidate_records_rewound = self
            .candidate_counters
            .rejected_candidate_records_rewound
            .saturating_add(
                after.rejected_candidate_records_rewound
                    - before.rejected_candidate_records_rewound,
            );
        self.candidate_counters.rejected_prior_records_redone = self
            .candidate_counters
            .rejected_prior_records_redone
            .saturating_add(
                after.rejected_prior_records_redone - before.rejected_prior_records_redone,
            );
        self.candidate_counters.candidate_journal_chunks_released = self
            .candidate_counters
            .candidate_journal_chunks_released
            .saturating_add(
                after.candidate_journal_chunks_released - before.candidate_journal_chunks_released,
            );
        self.regions.append(&mut tail.later_regions);
        Ok(())
    }

    pub(crate) fn accept_checkpoint_candidate(
        &mut self,
        mut tail: AcceptedPageRegionHistoryTail,
    ) -> Result<(), ForkArenaError> {
        let selected = self
            .regions
            .iter_mut()
            .find(|region| region.id() == tail.selected_region)
            .ok_or(ForkArenaError::InvalidRegion)?;
        let before = selected.builder.candidate_settlement_counters();
        selected.accept_checkpoint_candidate(&mut self.pool, tail.selected)?;
        let after = selected.builder.candidate_settlement_counters();
        self.candidate_counters.candidate_acceptances = self
            .candidate_counters
            .candidate_acceptances
            .saturating_add(after.candidate_acceptances - before.candidate_acceptances);
        self.candidate_counters.prior_journal_chunks_released = self
            .candidate_counters
            .prior_journal_chunks_released
            .saturating_add(
                after.prior_journal_chunks_released - before.prior_journal_chunks_released,
            );
        for region in tail.later_regions.drain(..) {
            region
                .retire(&mut self.pool)
                .expect("accepted history tail contains quiescent regions");
        }
        Ok(())
    }

    pub(crate) fn restore_checkpoint(
        &mut self,
        key: PageRegionCheckpointKey,
    ) -> Result<(), ForkArenaError> {
        let selected = self
            .regions
            .iter()
            .position(|region| region.validates_checkpoint(&self.pool, key))
            .ok_or(ForkArenaError::InvalidCheckpoint)?;
        let mut later_regions = self.regions.split_off(selected.saturating_add(1));
        let restored = self
            .regions
            .last_mut()
            .expect("page history always has a current region")
            .restore_checkpoint(&mut self.pool, key);
        if let Err(error) = restored {
            self.regions.append(&mut later_regions);
            return Err(error);
        }
        for region in later_regions {
            region
                .retire(&mut self.pool)
                .expect("restored history suffix contains quiescent regions");
        }
        Ok(())
    }

    pub(crate) fn retained_bytes(&self) -> usize {
        self.regions.iter().map(PageRegion::retained_bytes).sum()
    }

    #[must_use]
    pub(crate) const fn candidate_settlement_counters(&self) -> PageCandidateSettlementCounters {
        self.candidate_counters
    }

    #[cfg(test)]
    pub(crate) fn finish_shipout(
        &mut self,
        held_over: PageListId,
    ) -> Result<PageListId, ForkArenaError> {
        self.prepare_shipout(held_over)?;
        Ok(self
            .commit_prepared_shipout()
            .expect("just-prepared page succession commits"))
    }

    #[cfg(test)]
    pub(crate) fn prepare_shipout(&mut self, held_over: PageListId) -> Result<(), ForkArenaError> {
        if self.pending_successor.is_some() {
            return Err(ForkArenaError::InvalidRegion);
        }
        self.pending_successor = Some(
            self.regions
                .last_mut()
                .expect("page history always has a current region")
                .prepare_successor(&mut self.pool, held_over)?,
        );
        Ok(())
    }

    /// Prepares the production page transition from the complete live
    /// PageBuilder owner. All four owner-relative roots and the scalar/value
    /// state needed by the next page move together; callers never enumerate
    /// roots or retain a raw coordinate across the transition.
    pub(crate) fn prepare_production_shipout(&mut self) -> Result<(), ForkArenaError> {
        if self.pending_successor.is_some() {
            return Err(ForkArenaError::InvalidRegion);
        }
        self.pending_successor = Some(
            self.regions
                .last_mut()
                .expect("page history always has a current region")
                .prepare_production_successor(&mut self.pool)?,
        );
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn arm_output_successor_build(&mut self) {
        let current = self
            .regions
            .last_mut()
            .expect("page history always has a current region");
        let mark = PageNodeArena::new(&mut self.pool, &mut current.nodes)
            .begin_closure_build()
            .expect("test output successor boundary");
        current.builder.arm_output_successor_build(mark);
    }

    pub(crate) fn cancel_prepared_shipout(&mut self) {
        if let Some(prepared) = self.pending_successor.take() {
            match (
                prepared.current,
                prepared.unique_adoption,
                prepared.shared_prefix,
            ) {
                (Some(current), None, None) => current
                    .retire(&mut self.pool)
                    .expect("canceled successor is a quiescent unpublished region"),
                (None, Some((build, _)), None) | (None, None, Some((build, _))) => self
                    .regions
                    .last_mut()
                    .expect("page history always has a current region")
                    .builder
                    .arm_output_successor_build(build),
                _ => unreachable!("prepared successor has exactly one ownership plan"),
            }
        }
    }

    pub(crate) fn commit_prepared_shipout(&mut self) -> Result<PageListId, ForkArenaError> {
        let prepared = self
            .pending_successor
            .take()
            .ok_or(ForkArenaError::InvalidRegion)?;
        let old = self
            .regions
            .pop()
            .expect("page history always has a current region");
        let succession = old.commit_successor(&mut self.pool, prepared);
        let PageRegionSuccession {
            current,
            retained_prior,
            held_over,
        } = succession;
        if let Some(prior) = retained_prior {
            self.regions.push(prior);
        }
        self.regions.push(current);
        Ok(held_over)
    }
}

impl PageRegion {
    #[must_use]
    pub(crate) fn new(pool: &mut NodePool) -> Self {
        Self {
            nodes: PageMaterialRegion::new(pool),
            builder: PageBuilderState::default(),
            checkpoints: Vec::with_capacity(64),
            next_boundary: 1,
            counters: PageRegionCounters {
                page_regions_started: 1,
                ..PageRegionCounters::default()
            },
        }
    }

    fn retire(self, pool: &mut NodePool) -> Result<(), ForkArenaError> {
        self.nodes.retire(pool)
    }

    #[must_use]
    pub(crate) const fn id(&self) -> NodeRegionId {
        self.nodes.region_id()
    }

    #[cfg(test)]
    fn nodes<'a>(&'a self, pool: &'a NodePool) -> PageMaterialView<'a> {
        PageMaterialView::new(pool, &self.nodes)
    }

    #[cfg(test)]
    fn nodes_mut<'a>(&'a mut self, pool: &'a mut NodePool) -> PageNodeArena<'a> {
        PageNodeArena::new(pool, &mut self.nodes)
    }

    #[cfg(test)]
    fn parts_mut<'a>(
        &'a mut self,
        pool: &'a mut NodePool,
    ) -> (PageNodeArena<'a>, &'a mut PageBuilderState) {
        (PageNodeArena::new(pool, &mut self.nodes), &mut self.builder)
    }

    #[must_use]
    pub(crate) const fn builder(&self) -> &PageBuilderState {
        &self.builder
    }

    #[must_use]
    pub(crate) const fn builder_mut(&mut self) -> &mut PageBuilderState {
        &mut self.builder
    }

    pub(crate) fn seal_checkpoint(
        &mut self,
        pool: &mut NodePool,
    ) -> Result<PageRegionCheckpointKey, ForkArenaError> {
        let mut nodes = PageNodeArena::new(pool, &mut self.nodes);
        let boundary = nodes.seal_boundary()?;
        let node_mark = nodes.checkpoint_mark(boundary)?;
        let builder = self.builder.checkpoint_mark();
        let key = PageRegionCheckpointKey {
            region: self.id(),
            boundary: self.next_boundary,
        };
        self.next_boundary = self
            .next_boundary
            .checked_add(1)
            .ok_or(ForkArenaError::CapacityOverflow)?;
        self.checkpoints.push(PageRegionCheckpoint {
            key,
            nodes: node_mark,
            builder,
        });
        Ok(key)
    }

    fn checkpoint(&self, key: PageRegionCheckpointKey) -> Option<PageRegionCheckpoint> {
        (key.region == self.id())
            .then(|| {
                self.checkpoints
                    .iter()
                    .find(|checkpoint| checkpoint.key == key)
                    .copied()
            })
            .flatten()
    }

    fn release_checkpoint_row(
        &mut self,
        key: PageRegionCheckpointKey,
    ) -> Result<PageCheckpointMark, ForkArenaError> {
        if key.region != self.id() {
            return Err(ForkArenaError::InvalidCheckpoint);
        }
        let position = self
            .checkpoints
            .binary_search_by_key(&key.boundary, |row| row.key.boundary)
            .map_err(|_| ForkArenaError::InvalidCheckpoint)?;
        Ok(self.checkpoints.remove(position).builder)
    }

    #[must_use]
    pub(crate) fn validates_checkpoint(
        &self,
        pool: &NodePool,
        key: PageRegionCheckpointKey,
    ) -> bool {
        let nodes = PageMaterialView::new(pool, &self.nodes);
        self.checkpoint(key).is_some_and(|checkpoint| {
            nodes.can_restore_checkpoint(checkpoint.nodes)
                && self.builder.validates_checkpoint_mark(checkpoint.builder)
        })
    }

    pub(crate) fn arena_checkpoint(
        &self,
        key: PageRegionCheckpointKey,
    ) -> Option<NodeCheckpointMark> {
        self.checkpoint(key).map(|checkpoint| checkpoint.nodes)
    }

    pub(crate) fn checkpoint_identity_root(
        &self,
        key: PageRegionCheckpointKey,
    ) -> Option<Option<u64>> {
        self.checkpoint(key)
            .map(|checkpoint| checkpoint.builder.reachable_state_identity_root())
    }

    pub(crate) fn begin_checkpoint_candidate(
        &mut self,
        pool: &mut NodePool,
        key: PageRegionCheckpointKey,
    ) -> Result<AcceptedPageRegionTail, ForkArenaError> {
        let checkpoint = self
            .checkpoint(key)
            .filter(|checkpoint| {
                PageMaterialView::new(pool, &self.nodes).can_restore_checkpoint(checkpoint.nodes)
                    && self.builder.validates_checkpoint_mark(checkpoint.builder)
            })
            .ok_or(ForkArenaError::InvalidCheckpoint)?;
        let selected = self
            .checkpoints
            .iter()
            .position(|row| row.key == key)
            .expect("validated page-region checkpoint row remains present");
        let builder = self.builder.begin_checkpoint_candidate(checkpoint.builder);
        PageNodeArena::new(pool, &mut self.nodes)
            .begin_checkpoint_candidate(checkpoint.nodes)
            .expect("complete page-region preflight makes arena fork infallible");
        let later_rows = self.checkpoints.split_off(selected.saturating_add(1));
        self.counters.page_region_forks = self.counters.page_region_forks.saturating_add(1);
        Ok(AcceptedPageRegionTail {
            builder,
            selected_boundary: key.boundary,
            later_rows,
        })
    }

    pub(crate) fn reject_checkpoint_candidate(
        &mut self,
        pool: &mut NodePool,
        mut tail: AcceptedPageRegionTail,
    ) -> Result<(), ForkArenaError> {
        let boundary = PageNodeArena::new(pool, &mut self.nodes).seal_boundary()?;
        self.builder
            .prepare_checkpoint_candidate_rejection(&tail.builder);
        PageNodeArena::new(pool, &mut self.nodes).reject_checkpoint_candidate(boundary)?;
        self.builder
            .finish_checkpoint_candidate_rejection(tail.builder);
        let selected = self
            .checkpoints
            .iter()
            .position(|row| row.key.boundary == tail.selected_boundary)
            .expect("selected page-region row remains in the unchanged prefix");
        self.checkpoints.truncate(selected.saturating_add(1));
        self.checkpoints.append(&mut tail.later_rows);
        Ok(())
    }

    pub(crate) fn accept_checkpoint_candidate(
        &mut self,
        pool: &mut NodePool,
        tail: AcceptedPageRegionTail,
    ) -> Result<(), ForkArenaError> {
        let boundary = PageNodeArena::new(pool, &mut self.nodes).seal_boundary()?;
        debug_assert!(
            self.checkpoints
                .iter()
                .any(|row| row.key.boundary == tail.selected_boundary)
        );
        self.builder
            .prepare_checkpoint_candidate_acceptance(tail.builder);
        PageNodeArena::new(pool, &mut self.nodes).accept_checkpoint_candidate(boundary)
    }

    pub(crate) fn restore_checkpoint(
        &mut self,
        pool: &mut NodePool,
        key: PageRegionCheckpointKey,
    ) -> Result<(), ForkArenaError> {
        let checkpoint = self
            .checkpoint(key)
            .filter(|checkpoint| {
                PageMaterialView::new(pool, &self.nodes).can_restore_checkpoint(checkpoint.nodes)
                    && self.builder.validates_checkpoint_mark(checkpoint.builder)
            })
            .ok_or(ForkArenaError::InvalidCheckpoint)?;
        self.builder.restore_checkpoint_mark(checkpoint.builder);
        PageNodeArena::new(pool, &mut self.nodes).restore_checkpoint(checkpoint.nodes)
    }

    #[must_use]
    pub fn counters(&self) -> PageRegionCounters {
        let durable = self.nodes.durable_transition_counters();
        PageRegionCounters {
            page_to_durable_nodes_copied: durable.page_to_durable_nodes_copied,
            tex_copy_nodes_copied: durable.tex_copy_nodes_copied,
            history_preservation_nodes_copied: durable.history_preservation_nodes_copied,
            nested_closure_nodes_copied: durable.nested_closure_nodes_copied,
            node_closure_scan_nodes: self
                .counters
                .node_closure_scan_nodes
                .saturating_add(durable.node_closure_scan_nodes),
            ..self.counters
        }
    }

    #[must_use]
    pub(crate) const fn material_counters(&self) -> crate::fork_arena::ForkArenaCounters {
        self.nodes.counters()
    }

    #[must_use]
    pub(crate) fn retained_bytes(&self) -> usize {
        self.builder.retained_bytes().saturating_add(
            self.checkpoints
                .capacity()
                .saturating_mul(core::mem::size_of::<PageRegionCheckpoint>()),
        )
    }

    /// Ends this page-building period and establishes the next one.
    ///
    /// The caller supplies the exact held-over closure selected by the
    /// page-break traversal.  Only that closure is recursively copied into
    /// the new region; unrelated shipped and historical material is never
    /// visited.  An old region with retained rows moves into history as one
    /// owner, while an uncheckpointed old region drops wholesale.
    fn prepare_successor(
        &mut self,
        pool: &mut NodePool,
        held_over: PageListId,
    ) -> Result<PreparedPageRegionSuccessor, ForkArenaError> {
        if !PageMaterialView::new(pool, &self.nodes).contains(held_over) {
            self.counters.cross_region_node_reference_rejections = self
                .counters
                .cross_region_node_reference_rejections
                .saturating_add(1);
            return Err(ForkArenaError::InvalidRegion);
        }
        let mut current = PageRegion::new(pool);
        current
            .nodes
            .inherit_durable_transition_counters_from(&self.nodes);
        if PageMaterialView::new(pool, &self.nodes).semantic_identity_enabled() {
            PageNodeArena::new(pool, &mut current.nodes).enable_semantic_identity();
        }
        let (held_over, copied) = match PageMaterialRegion::copy_closure_between(
            pool,
            &mut current.nodes,
            &self.nodes,
            held_over,
        ) {
            Ok(copied) => copied,
            Err(error) => {
                current
                    .retire(pool)
                    .expect("failed successor remains a quiescent empty region");
                return Err(error);
            }
        };
        current.counters = self.counters;
        current.counters.page_regions_started =
            current.counters.page_regions_started.saturating_add(1);
        current.counters.held_over_nodes_copied = current
            .counters
            .held_over_nodes_copied
            .saturating_add(copied as u64);
        if !held_over.is_empty() {
            let mut nodes = PageNodeArena::new(pool, &mut current.nodes);
            current
                .builder
                .push_current_page_list(&mut nodes, held_over);
        }
        Ok(PreparedPageRegionSuccessor {
            current: Some(current),
            held_over: Some(held_over),
            unique_adoption: None,
            shared_prefix: None,
        })
    }

    /// Builds the next production owner from every live PageBuilder root.
    /// This is deliberately separate from the focused one-root test seam:
    /// production must never infer liveness from a caller-selected naked
    /// coordinate.
    fn prepare_production_successor(
        &mut self,
        pool: &mut NodePool,
    ) -> Result<PreparedPageRegionSuccessor, ForkArenaError> {
        let build = self.builder.take_output_successor_build();
        let roots = self.builder.payload_roots();
        for root in [
            roots.contribution,
            roots.current_page,
            roots.page_discards,
            roots.split_discards,
        ] {
            if PageMaterialView::new(pool, &self.nodes)
                .span_list(root)
                .is_err()
            {
                self.counters.cross_region_node_reference_rejections = self
                    .counters
                    .cross_region_node_reference_rejections
                    .saturating_add(1);
                return Err(ForkArenaError::InvalidRegion);
            }
        }

        let root_lists = [
            roots.contribution.list(),
            roots.current_page.list(),
            roots.page_discards.list(),
            roots.split_discards.list(),
        ];
        let uniquely_adoptable = self.checkpoints.is_empty()
            && build.as_ref().is_some_and(|build| {
                self.nodes
                    .preflight_unique_successor_adoption(pool, build, root_lists)
                    .is_ok()
            });
        if uniquely_adoptable {
            return Ok(PreparedPageRegionSuccessor {
                current: None,
                held_over: None,
                unique_adoption: Some((
                    build.expect("unique-successor preflight requires one build"),
                    roots,
                )),
                shared_prefix: None,
            });
        }

        let shareable_retained_prefix = !self.checkpoints.is_empty()
            && build.as_ref().is_some_and(|build| {
                self.nodes
                    .can_share_sealed_prefix(pool, build, root_lists)
                    .is_ok()
            });
        if shareable_retained_prefix {
            return Ok(PreparedPageRegionSuccessor {
                current: None,
                held_over: None,
                unique_adoption: None,
                shared_prefix: Some((
                    build.expect("retained-prefix preflight requires one build"),
                    roots,
                )),
            });
        }

        let mut current = PageRegion::new(pool);
        current
            .nodes
            .inherit_durable_transition_counters_from(&self.nodes);
        if PageMaterialView::new(pool, &self.nodes).semantic_identity_enabled() {
            PageNodeArena::new(pool, &mut current.nodes).enable_semantic_identity();
        }

        let mut fallback_build = None;
        if let Some(build) = build {
            let one_self_contained_root = !roots.contribution.is_empty()
                && roots.current_page.is_empty()
                && roots.page_discards.is_empty()
                && roots.split_discards.is_empty();
            if one_self_contained_root {
                match PageMaterialRegion::move_built_closure_between(
                    pool,
                    &mut current.nodes,
                    &mut self.nodes,
                    build,
                    roots.contribution.list(),
                ) {
                    Ok((contribution, scanned)) => {
                        let contribution = PageNodeArena::new(pool, &mut current.nodes)
                            .admit_span(contribution)?;
                        let roots = PagePayloadRoots {
                            contribution,
                            current_page: PageListSpan::empty(),
                            page_discards: PageListSpan::empty(),
                            split_discards: PageListSpan::empty(),
                        };
                        current.builder = self.builder.successor_with_roots(roots);
                        current.counters = self.counters;
                        current.counters.page_regions_started =
                            current.counters.page_regions_started.saturating_add(1);
                        current.counters.held_over_envelopes_moved =
                            current.counters.held_over_envelopes_moved.saturating_add(1);
                        current.counters.node_closure_scan_nodes = current
                            .counters
                            .node_closure_scan_nodes
                            .saturating_add(scanned);
                        return Ok(PreparedPageRegionSuccessor {
                            current: Some(current),
                            held_over: None,
                            unique_adoption: None,
                            shared_prefix: None,
                        });
                    }
                    Err((_error, Some(build))) => fallback_build = Some(build),
                    Err((_error, None)) => {}
                }
            } else {
                fallback_build = Some(build);
            }
        }
        let copied = (|| {
            let contribution = PageMaterialRegion::copy_closure_between(
                pool,
                &mut current.nodes,
                &self.nodes,
                roots.contribution.list(),
            )?;
            let current_page = PageMaterialRegion::copy_closure_between(
                pool,
                &mut current.nodes,
                &self.nodes,
                roots.current_page.list(),
            )?;
            let page_discards = PageMaterialRegion::copy_closure_between(
                pool,
                &mut current.nodes,
                &self.nodes,
                roots.page_discards.list(),
            )?;
            let split_discards = PageMaterialRegion::copy_closure_between(
                pool,
                &mut current.nodes,
                &self.nodes,
                roots.split_discards.list(),
            )?;
            let spans = {
                let arena = PageNodeArena::new(pool, &mut current.nodes);
                (
                    arena.admit_span(contribution.0)?,
                    arena.admit_span(current_page.0)?,
                    arena.admit_span(page_discards.0)?,
                    arena.admit_span(split_discards.0)?,
                )
            };
            Ok::<_, ForkArenaError>((
                PagePayloadRoots {
                    contribution: spans.0,
                    current_page: spans.1,
                    page_discards: spans.2,
                    split_discards: spans.3,
                },
                contribution
                    .1
                    .saturating_add(current_page.1)
                    .saturating_add(page_discards.1)
                    .saturating_add(split_discards.1),
            ))
        })();
        let (roots, copied) = match copied {
            Ok(copied) => copied,
            Err(error) => {
                if let Some(build) = fallback_build {
                    self.nodes
                        .cancel_closure_build(pool, build)
                        .expect("failed fallback releases its successor suffix");
                }
                current
                    .retire(pool)
                    .expect("failed production successor remains quiescent");
                return Err(error);
            }
        };
        if let Some(build) = fallback_build {
            self.nodes
                .cancel_closure_build(pool, build)
                .expect("copied fallback releases its old successor suffix");
        }
        current.builder = self.builder.successor_with_roots(roots);
        current.counters = self.counters;
        current.counters.page_regions_started =
            current.counters.page_regions_started.saturating_add(1);
        current.counters.held_over_nodes_copied = current
            .counters
            .held_over_nodes_copied
            .saturating_add(copied as u64);
        Ok(PreparedPageRegionSuccessor {
            current: Some(current),
            held_over: None,
            unique_adoption: None,
            shared_prefix: None,
        })
    }

    fn commit_successor(
        mut self,
        pool: &mut NodePool,
        prepared: PreparedPageRegionSuccessor,
    ) -> PageRegionSuccession {
        if let Some((build, roots)) = prepared.unique_adoption {
            debug_assert!(prepared.current.is_none());
            let root_lists = [
                roots.contribution.list(),
                roots.current_page.list(),
                roots.page_discards.list(),
                roots.split_discards.list(),
            ];
            self.nodes
                .adopt_unique_successor(pool, build, root_lists)
                .expect("prepared unique successor remains adoptable");
            let builder = self.builder.successor_with_roots(roots);
            let mut counters = self.counters;
            counters.page_regions_started = counters.page_regions_started.saturating_add(1);
            counters.page_regions_dropped = counters.page_regions_dropped.saturating_add(1);
            if root_lists.iter().any(|root| !root.is_empty()) {
                counters.held_over_envelopes_moved =
                    counters.held_over_envelopes_moved.saturating_add(1);
            }
            return PageRegionSuccession {
                current: PageRegion {
                    nodes: self.nodes,
                    builder,
                    checkpoints: Vec::with_capacity(64),
                    next_boundary: 1,
                    counters,
                },
                retained_prior: None,
                held_over: PageListId::empty(),
            };
        }
        if let Some((build, roots)) = prepared.shared_prefix {
            debug_assert!(prepared.current.is_none());
            debug_assert!(!self.checkpoints.is_empty());
            let root_lists = [
                roots.contribution.list(),
                roots.current_page.list(),
                roots.page_discards.list(),
                roots.split_discards.list(),
            ];
            let nodes = PageMaterialRegion::share_sealed_prefix_from(
                pool,
                &mut self.nodes,
                build,
                root_lists,
            )
            .expect("prepared retained prefix remains shareable");
            let builder = self.builder.successor_with_roots(roots);
            let mut counters = self.counters;
            counters.page_regions_started = counters.page_regions_started.saturating_add(1);
            counters.page_regions_retained = counters.page_regions_retained.saturating_add(1);
            self.counters.page_regions_retained =
                self.counters.page_regions_retained.saturating_add(1);
            return PageRegionSuccession {
                current: PageRegion {
                    nodes,
                    builder,
                    checkpoints: Vec::with_capacity(64),
                    next_boundary: 1,
                    counters,
                },
                retained_prior: Some(self),
                held_over: PageListId::empty(),
            };
        }
        let PreparedPageRegionSuccessor {
            current,
            held_over,
            unique_adoption: None,
            shared_prefix: None,
        } = prepared
        else {
            unreachable!("unique adoption returned before materialized succession")
        };
        let mut current = current.expect("materialized successor owns its next region");
        let retained_prior = if self.checkpoints.is_empty() {
            current.counters.page_regions_dropped =
                current.counters.page_regions_dropped.saturating_add(1);
            self.retire(pool)
                .expect("uncheckpointed prior page region is quiescent");
            None
        } else {
            self.counters.page_regions_retained =
                self.counters.page_regions_retained.saturating_add(1);
            current.counters.page_regions_retained =
                current.counters.page_regions_retained.saturating_add(1);
            Some(self)
        };
        PageRegionSuccession {
            current,
            retained_prior,
            held_over: held_over.unwrap_or(PageListId::empty()),
        }
    }

    #[allow(clippy::result_large_err)] // Failed succession must return the exclusive page region.
    pub fn finish_shipout(
        mut self,
        pool: &mut NodePool,
        held_over: PageListId,
    ) -> Result<PageRegionSuccession, (ForkArenaError, Self)> {
        let prepared = match self.prepare_successor(pool, held_over) {
            Ok(prepared) => prepared,
            Err(error) => return Err((error, self)),
        };
        Ok(self.commit_successor(pool, prepared))
    }
}

impl PageRegionSuccession {
    /// The newly current page region.
    #[must_use]
    pub const fn current(&self) -> &PageRegion {
        &self.current
    }

    /// The prior page owner while at least one retained row still names it.
    #[must_use]
    pub const fn retained_prior(&self) -> Option<&PageRegion> {
        self.retained_prior.as_ref()
    }

    /// Owner-relative root of the evacuated closure in the new region.
    #[must_use]
    pub const fn held_over(&self) -> PageListId {
        self.held_over
    }

    /// Removes one retained boundary and drops its old page owner when the
    /// contiguous interval becomes empty.
    pub fn prune_retained_checkpoint(
        &mut self,
        pool: &mut NodePool,
        key: PageRegionCheckpointKey,
    ) -> bool {
        let Some(prior) = &mut self.retained_prior else {
            return false;
        };
        let Some(position) = prior.checkpoints.iter().position(|row| row.key == key) else {
            return false;
        };
        prior.checkpoints.remove(position);
        if prior.checkpoints.is_empty() {
            self.retained_prior
                .take()
                .expect("validated retained prior remains present")
                .retire(pool)
                .expect("pruned prior page region is quiescent");
            self.current.counters.page_regions_dropped =
                self.current.counters.page_regions_dropped.saturating_add(1);
        }
        true
    }
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
            contribution: PageListSpan::empty(),
            current_page: PageListSpan::empty(),
            page_discards: PageListSpan::empty(),
            split_discards: PageListSpan::empty(),
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
            progress: PageBuilderProgressToken(0),
            end_job_ejection: None,
            resume_after_output: false,
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
            page_node_root_count: 0,
            identity_enabled: false,
            semantic_roots: PageSemanticRoots::default(),
            checkpoint_journal: PageCheckpointJournal {
                timeline: NEXT_PAGE_TIMELINE.fetch_add(1, Ordering::Relaxed),
                next_frame: 1,
                frames: Vec::with_capacity(64),
                inverse_pool: ChunkPool::default(),
                inverses: ForkArena::new(),
                applied: 0,
                candidate_root_frame: None,
                replay_work: 0,
                accepted_rewind_work: 0,
                accepted_redo_work: 0,
                accepted_rewind_transitions: 0,
                accepted_redo_transitions: 0,
                checkpoint_captures: 0,
                candidate_acceptances: 0,
                candidate_rejections: 0,
                candidate_rewind_work: 0,
            },
            output_successor_build: None,
        }
    }
}

impl PageBuilderState {
    fn journal_and_inverse_target(
        &mut self,
    ) -> (&mut PageCheckpointJournal, PageInverseTarget<'_>) {
        let Self {
            contribution,
            current_page,
            page_discards,
            split_discards,
            page_goal,
            page_total,
            page_stretch,
            page_fil_stretch,
            page_fill_stretch,
            page_filll_stretch,
            page_shrink,
            page_depth,
            page_max_depth,
            contents,
            last_glue,
            last_penalty,
            last_kern,
            last_node_type,
            insert_penalties,
            dead_cycles,
            least_page_cost,
            best_page_break,
            best_size,
            fire_up,
            progress,
            end_job_ejection,
            resume_after_output,
            insertions,
            insertion_positions,
            top_mark,
            first_mark,
            bot_mark,
            split_first_mark,
            split_bot_mark,
            mark_classes,
            mark_class_positions,
            tex82_dynamic_words,
            etex_dynamic_words,
            page_node_root_count,
            checkpoint_journal,
            ..
        } = self;
        (
            checkpoint_journal,
            PageInverseTarget {
                roots: [contribution, current_page, page_discards, split_discards],
                dimensions: [
                    page_goal,
                    page_total,
                    page_stretch,
                    page_fil_stretch,
                    page_fill_stretch,
                    page_filll_stretch,
                    page_shrink,
                    page_depth,
                    page_max_depth,
                ],
                contents,
                last_glue,
                last_penalty,
                last_kern,
                last_node_type,
                insert_penalties,
                dead_cycles,
                least_page_cost,
                best_page_break,
                best_size,
                fire_up,
                progress,
                end_job_ejection,
                resume_after_output,
                tex82_dynamic_words,
                etex_dynamic_words,
                page_node_root_count,
                insertions,
                insertion_positions,
                marks: [
                    top_mark,
                    first_mark,
                    bot_mark,
                    split_first_mark,
                    split_bot_mark,
                ],
                mark_classes,
                mark_class_positions,
            },
        )
    }

    fn advance_progress(&mut self) {
        self.progress.0 = self
            .progress
            .0
            .checked_add(1)
            .expect("page-builder progress space exhausted");
    }

    /// Starts one TeX82 §1054 ejection only after the preceding attempt made
    /// canonical page-builder progress.
    pub(crate) fn begin_end_job_ejection(
        &mut self,
    ) -> Result<PageBuilderProgressToken, PageBuilderProgressToken> {
        let position = self.progress;
        if self.end_job_ejection == Some(position) {
            #[cfg(feature = "profiling")]
            crate::measurement::record_hot_core_page_builder_transition(
                crate::measurement::HotCorePageBuilderTransition::EndJobIdenticalState,
            );
            return Err(position);
        }
        self.record_scalars();
        self.end_job_ejection = Some(position);
        #[cfg(feature = "profiling")]
        crate::measurement::record_hot_core_page_builder_transition(
            crate::measurement::HotCorePageBuilderTransition::EndJobEjectionStarted,
        );
        Ok(position)
    }

    pub(crate) fn complete_end_job_ejection(&mut self, started: PageBuilderProgressToken) -> bool {
        let progressed = self.progress.0 != started.0;
        #[cfg(feature = "profiling")]
        crate::measurement::record_hot_core_page_builder_transition(if progressed {
            crate::measurement::HotCorePageBuilderTransition::EndJobEjectionProgressed
        } else {
            crate::measurement::HotCorePageBuilderTransition::EndJobIdenticalState
        });
        progressed
    }

    pub(crate) fn finish_end_job(&mut self) {
        if self.end_job_ejection.is_some() || self.resume_after_output {
            self.record_scalars();
            self.end_job_ejection = None;
            self.resume_after_output = false;
        }
    }

    pub(crate) const fn resume_after_output_pending(&self) -> bool {
        self.resume_after_output
    }

    pub(crate) fn take_resume_after_output(&mut self) -> bool {
        if !self.resume_after_output {
            return false;
        }
        self.record_scalars();
        self.resume_after_output = false;
        #[cfg(feature = "profiling")]
        crate::measurement::record_hot_core_page_builder_transition(
            crate::measurement::HotCorePageBuilderTransition::OutputBuilderResumed,
        );
        true
    }

    #[cfg(test)]
    pub(crate) const fn progress_token(&self) -> PageBuilderProgressToken {
        self.progress
    }

    pub(crate) fn arm_output_successor_build(&mut self, mark: PageClosureBuildMark) {
        assert!(
            self.output_successor_build.replace(mark).is_none(),
            "one page output owns one successor build suffix"
        );
    }

    fn take_output_successor_build(&mut self) -> Option<PageClosureBuildMark> {
        self.output_successor_build.take()
    }

    fn payload_roots(&self) -> PagePayloadRoots {
        PagePayloadRoots {
            contribution: self.contribution,
            current_page: self.current_page,
            page_discards: self.page_discards,
            split_discards: self.split_discards,
        }
    }

    pub(crate) fn payload_root_lists(&self) -> [PageListId; 4] {
        let roots = self.payload_roots();
        [
            roots.contribution.list(),
            roots.current_page.list(),
            roots.page_discards.list(),
            roots.split_discards.list(),
        ]
    }

    /// Starts a fresh page timeline from the live state which escaped the
    /// completed page region. Historical journal rows stay with the old
    /// owner; only canonical current values and rebranded roots enter the new
    /// region, so per-page history cannot accumulate across succession.
    fn successor_with_roots(&self, roots: PagePayloadRoots) -> Self {
        let mut successor = Self {
            contribution: roots.contribution,
            current_page: roots.current_page,
            page_discards: roots.page_discards,
            split_discards: roots.split_discards,
            ..Self::default()
        };
        successor.restore_scalars(self.scalar_snapshot());
        successor.insertions = self.insertions.clone();
        successor.insertion_positions = self.insertion_positions.clone();
        successor.top_mark = self.top_mark;
        successor.first_mark = self.first_mark;
        successor.bot_mark = self.bot_mark;
        successor.split_first_mark = self.split_first_mark;
        successor.split_bot_mark = self.split_bot_mark;
        successor.mark_classes = self.mark_classes.clone();
        successor.mark_class_positions = self.mark_class_positions.clone();
        successor.identity_enabled = self.identity_enabled;
        successor.semantic_roots = self.semantic_roots;
        successor.semantic_roots.contribution = list_identity(roots.contribution);
        successor.semantic_roots.page_discards = list_identity(roots.page_discards);
        successor.semantic_roots.split_discards = list_identity(roots.split_discards);
        successor
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) const fn contribution_root(&self) -> PageListId {
        self.contribution.list()
    }

    pub(crate) fn enable_reachable_state_identity(&mut self) {
        if self.identity_enabled {
            return;
        }
        assert!(
            self.contribution.is_empty()
                && self.current_page.is_empty()
                && self.page_discards.is_empty()
                && self.split_discards.is_empty()
                && self.insertions.is_empty()
                && self.mark_classes.is_empty()
                && self.top_mark.is_none()
                && self.first_mark.is_none()
                && self.bot_mark.is_none()
                && self.split_first_mark.is_none()
                && self.split_bot_mark.is_none(),
            "page semantic identity must be selected before execution"
        );
        self.identity_enabled = true;
        self.semantic_roots.insertions = self
            .insertions
            .iter()
            .fold(0_u64, |root, value| root ^ insertion_identity(*value));
        self.semantic_roots.marks = self.current_marks_identity();
    }

    fn reachable_state_identity_root(&self) -> Option<u64> {
        if !self.identity_enabled {
            return None;
        }
        let mut hasher = page_identity_hasher(b"umber-page-semantic-root-v1");
        1_u16.hash(&mut hasher);
        match self.contents {
            PageContents::Empty => 0_u8,
            PageContents::InsertsOnly => 1,
            PageContents::BoxThere => 2,
        }
        .hash(&mut hasher);
        for value in [
            self.page_goal,
            self.page_total,
            self.page_stretch,
            self.page_fil_stretch,
            self.page_fill_stretch,
            self.page_filll_stretch,
            self.page_shrink,
            self.page_depth,
            self.page_max_depth,
        ] {
            value.raw().hash(&mut hasher);
        }
        match self.last_glue {
            Some(glue) => {
                1_u8.hash(&mut hasher);
                hash_page_glue(glue, &mut hasher);
            }
            None => 0_u8.hash(&mut hasher),
        }
        self.last_penalty.hash(&mut hasher);
        self.last_kern.raw().hash(&mut hasher);
        self.last_node_type.hash(&mut hasher);
        self.insert_penalties.hash(&mut hasher);
        self.dead_cycles.hash(&mut hasher);
        self.least_page_cost.hash(&mut hasher);
        self.best_page_break
            .map(|value| value.index() as u64)
            .hash(&mut hasher);
        self.best_size.raw().hash(&mut hasher);
        self.fire_up
            .map(|value| {
                (
                    value.best_break().index() as u64,
                    value.best_size().raw(),
                    value.trigger().index() as u64,
                )
            })
            .hash(&mut hasher);
        for sequence in [
            list_identity(self.contribution),
            list_identity(self.current_page),
            list_identity(self.page_discards),
            list_identity(self.split_discards),
        ] {
            (sequence.len() as u64).hash(&mut hasher);
            sequence.raw().hash(&mut hasher);
        }
        self.semantic_roots.insertions.hash(&mut hasher);
        self.semantic_roots.marks.hash(&mut hasher);
        Some(hasher.finish())
    }

    pub(crate) fn checkpoint_mark(&mut self) -> PageCheckpointMark {
        let boundary = self
            .checkpoint_journal
            .inverses
            .seal_boundary(&mut self.checkpoint_journal.inverse_pool)
            .expect("page checkpoint seals its reversible journal");
        let journal = self
            .checkpoint_journal
            .inverses
            .checkpoint_mark(boundary)
            .expect("page journal boundary becomes a fixed checkpoint mark");
        self.checkpoint_journal.checkpoint_captures = self
            .checkpoint_journal
            .checkpoint_captures
            .saturating_add(1);
        let frame = self.checkpoint_journal.next_frame;
        self.checkpoint_journal.next_frame = self
            .checkpoint_journal
            .next_frame
            .checked_add(1)
            .expect("page checkpoint identity space exhausted");
        let cursor = self.checkpoint_journal.applied;
        let scalars = self.scalar_snapshot();
        let direct_roots = PagePayloadRoots {
            contribution: self.contribution,
            current_page: self.current_page,
            page_discards: self.page_discards,
            split_discards: self.split_discards,
        };
        self.checkpoint_journal
            .frames
            .push(PageCheckpointFrame { id: frame, cursor });
        PageCheckpointMark {
            timeline: self.checkpoint_journal.timeline,
            frame,
            cursor,
            journal,
            scalars,
            roots: direct_roots,
            semantic_roots: self.semantic_roots,
            reachable_state_identity_root: self.reachable_state_identity_root(),
        }
    }

    pub(crate) fn commit_transaction(&mut self, mark: PageCheckpointMark) {
        debug_assert!(self.validates_checkpoint_mark(mark));
        self.checkpoint_journal
            .frames
            .retain(|frame| frame.id != mark.frame);
        if self.checkpoint_journal.frames.is_empty() {
            let boundary = self
                .checkpoint_journal
                .inverses
                .seal_boundary(&mut self.checkpoint_journal.inverse_pool)
                .expect("quiescent page journal seals before prefix release");
            let head = self
                .checkpoint_journal
                .inverses
                .checkpoint_mark(boundary)
                .expect("sealed page journal head is a fixed mark");
            self.checkpoint_journal
                .inverses
                .release_accepted_prefix(&mut self.checkpoint_journal.inverse_pool, head)
                .expect("unrooted page journal releases whole prefix chunks");
        }
    }

    pub(crate) fn rollback_transaction(&mut self, mark: PageCheckpointMark) {
        self.restore_checkpoint_mark(mark);
        self.checkpoint_journal
            .frames
            .retain(|frame| frame.id != mark.frame);
        if self.checkpoint_journal.frames.is_empty() {
            self.checkpoint_journal
                .inverses
                .release_accepted_prefix(&mut self.checkpoint_journal.inverse_pool, mark.journal)
                .expect("rolled-back unrooted page journal releases whole prefix chunks");
        }
    }

    pub(crate) fn validates_checkpoint_mark(&self, mark: PageCheckpointMark) -> bool {
        mark.timeline == self.checkpoint_journal.timeline
            && mark.cursor <= self.checkpoint_journal.applied
            && self
                .checkpoint_journal
                .inverses
                .validates_checkpoint(mark.journal)
            && mark.frame != 0
            && mark.frame < self.checkpoint_journal.next_frame
            && self
                .checkpoint_journal
                .frames
                .iter()
                .any(|frame| frame.id == mark.frame && frame.cursor == mark.cursor)
    }

    pub(crate) fn restore_checkpoint_mark(&mut self, mark: PageCheckpointMark) {
        debug_assert!(self.validates_checkpoint_mark(mark));
        if self.checkpoint_journal.candidate_root_frame.is_some() {
            self.rewind_current_page_journal(mark);
            self.checkpoint_journal
                .inverses
                .restore_current_checkpoint(&mut self.checkpoint_journal.inverse_pool, mark.journal)
                .expect("candidate-local page restore truncates only current chunks");
        } else {
            self.rewind_accepted_page_journal(mark);
            self.checkpoint_journal
                .inverses
                .restore_accepted_checkpoint(
                    &mut self.checkpoint_journal.inverse_pool,
                    mark.journal,
                )
                .expect("validated same-generation page restore prunes its journal suffix");
        }
        self.semantic_roots = mark.semantic_roots;
    }

    pub(crate) fn begin_checkpoint_candidate(
        &mut self,
        mark: PageCheckpointMark,
    ) -> AcceptedPageTail {
        debug_assert!(self.validates_checkpoint_mark(mark));
        debug_assert!(self.checkpoint_journal.candidate_root_frame.is_none());
        let origin = self.checkpoint_journal.applied;
        let origin_scalars = self.scalar_snapshot();
        let origin_semantic_roots = self.semantic_roots;
        let rewind_before = self.checkpoint_journal.replay_work;
        self.rewind_accepted_page_journal(mark);
        let accepted_rewind_work = self
            .checkpoint_journal
            .replay_work
            .saturating_sub(rewind_before);
        self.checkpoint_journal.accepted_rewind_work = self
            .checkpoint_journal
            .accepted_rewind_work
            .saturating_add(accepted_rewind_work);
        self.checkpoint_journal.accepted_rewind_transitions = self
            .checkpoint_journal
            .accepted_rewind_transitions
            .saturating_add(1);
        self.contribution = mark.roots.contribution;
        self.current_page = mark.roots.current_page;
        self.page_discards = mark.roots.page_discards;
        self.split_discards = mark.roots.split_discards;
        self.restore_scalars(mark.scalars);
        self.semantic_roots = mark.semantic_roots;
        self.checkpoint_journal
            .inverses
            .begin_checkpoint_candidate(&mut self.checkpoint_journal.inverse_pool, mark.journal)
            .expect("validated page journal opens one accepted/current fork");
        let selected_frame = self
            .checkpoint_journal
            .frames
            .iter()
            .position(|frame| frame.id == mark.frame)
            .expect("validated page checkpoint retains its frame");
        let prefix_frames = selected_frame.saturating_add(1);
        let future_frames = self.checkpoint_journal.frames.split_off(prefix_frames);
        self.checkpoint_journal.applied = mark.cursor;
        let root_frame = self.checkpoint_journal.next_frame;
        self.checkpoint_journal.next_frame = self
            .checkpoint_journal
            .next_frame
            .checked_add(1)
            .expect("page candidate frame space exhausted");
        self.checkpoint_journal.frames.push(PageCheckpointFrame {
            id: root_frame,
            cursor: mark.cursor,
        });
        self.checkpoint_journal.candidate_root_frame = Some(root_frame);
        AcceptedPageTail {
            origin,
            selected: mark.cursor,
            prefix_frames,
            accepted_rewind_work,
            future_frames,
            origin_scalars,
            origin_semantic_roots,
            journal: mark.journal,
        }
    }

    /// Rewinds candidate roots and drops every current-lineage inverse before
    /// the page-material arena releases candidate chunks.
    pub(crate) fn prepare_checkpoint_candidate_rejection(&mut self, tail: &AcceptedPageTail) {
        let (journal, mut target) = self.journal_and_inverse_target();
        let replay_work = &mut journal.replay_work;
        let mut candidate_rewind_work = 0_u64;
        journal
            .inverses
            .visit_current_checkpoint_suffix_mut_reverse(
                &mut journal.inverse_pool,
                tail.journal,
                |inverse| {
                    target.toggle(inverse);
                    *replay_work = replay_work.saturating_add(1);
                    candidate_rewind_work = candidate_rewind_work.saturating_add(1);
                },
            )
            .expect("candidate page journal suffix rewinds in place");
        journal.applied = tail.selected;
        journal.candidate_rewind_work = journal
            .candidate_rewind_work
            .saturating_add(candidate_rewind_work);
        self.checkpoint_journal.candidate_root_frame = None;
        self.checkpoint_journal.frames.truncate(tail.prefix_frames);
    }

    /// Reattaches the accepted journal and redoes its roots only after the
    /// page-material arena has reattached the detached accepted chunks.
    pub(crate) fn finish_checkpoint_candidate_rejection(&mut self, mut tail: AcceptedPageTail) {
        debug_assert_eq!(self.checkpoint_journal.frames.len(), tail.prefix_frames);
        debug_assert_eq!(self.checkpoint_journal.applied, tail.selected);
        self.checkpoint_journal
            .frames
            .append(&mut tail.future_frames);
        let redo_before = self.checkpoint_journal.replay_work;
        let (journal, mut target) = self.journal_and_inverse_target();
        let replay_work = &mut journal.replay_work;
        journal
            .inverses
            .visit_detached_checkpoint_suffix_mut(&mut journal.inverse_pool, |inverse| {
                target.toggle(inverse);
                *replay_work = replay_work.saturating_add(1);
            })
            .expect("detached accepted page journal redoes in place");
        let boundary = journal
            .inverses
            .seal_boundary(&mut journal.inverse_pool)
            .expect("candidate page journal seals before rejection");
        journal
            .inverses
            .reject_checkpoint_candidate(&mut journal.inverse_pool, boundary)
            .expect("page rejection drops current chunks and reattaches prior chunks");
        journal.applied = tail.origin;
        journal.candidate_rejections = journal.candidate_rejections.saturating_add(1);
        let accepted_redo_work = self
            .checkpoint_journal
            .replay_work
            .saturating_sub(redo_before);
        assert_eq!(
            accepted_redo_work, tail.accepted_rewind_work,
            "rejection redoes exactly the accepted span rewound at edit start"
        );
        self.checkpoint_journal.accepted_redo_work = self
            .checkpoint_journal
            .accepted_redo_work
            .saturating_add(accepted_redo_work);
        self.checkpoint_journal.accepted_redo_transitions = self
            .checkpoint_journal
            .accepted_redo_transitions
            .saturating_add(1);
        self.restore_scalars(tail.origin_scalars);
        self.semantic_roots = tail.origin_semantic_roots;
    }

    /// Drops every accepted-suffix root before the page-material arena prunes
    /// the detached accepted chunks.
    pub(crate) fn prepare_checkpoint_candidate_acceptance(&mut self, tail: AcceptedPageTail) {
        let root = self
            .checkpoint_journal
            .candidate_root_frame
            .take()
            .expect("candidate page timeline owns one root frame");
        let root_index = tail.prefix_frames;
        let removed = self.checkpoint_journal.frames.remove(root_index);
        assert_eq!(
            removed.id, root,
            "candidate root frame remains at its fixed mark"
        );
        let boundary = self
            .checkpoint_journal
            .inverses
            .seal_boundary(&mut self.checkpoint_journal.inverse_pool)
            .expect("candidate page journal seals before acceptance");
        self.checkpoint_journal
            .inverses
            .accept_checkpoint_candidate(&mut self.checkpoint_journal.inverse_pool, boundary)
            .expect("page acceptance retires superseded prior chunks");
        self.checkpoint_journal.candidate_acceptances = self
            .checkpoint_journal
            .candidate_acceptances
            .saturating_add(1);
        drop(tail);
    }

    fn record_page_inverse(&mut self, inverse: PageInverse) {
        if !self.checkpoint_journal.frames.is_empty() {
            let mut builder = self
                .checkpoint_journal
                .inverses
                .begin_builder(&mut self.checkpoint_journal.inverse_pool)
                .expect("page journal owns the sole active builder");
            builder
                .push(inverse)
                .expect("one page inverse fits its fixed-chunk journal");
            let _ = builder
                .seal()
                .expect("page inverse publishes without materialization");
            self.checkpoint_journal.applied += 1;
        }
    }

    fn rewind_accepted_page_journal(&mut self, mark: PageCheckpointMark) {
        let (journal, mut target) = self.journal_and_inverse_target();
        let replay_work = &mut journal.replay_work;
        journal
            .inverses
            .visit_accepted_checkpoint_suffix_mut_reverse(
                &mut journal.inverse_pool,
                mark.journal,
                |inverse| {
                    target.toggle(inverse);
                    *replay_work = replay_work.saturating_add(1);
                },
            )
            .expect("validated accepted page journal rewinds to its fixed mark");
        journal.applied = mark.cursor;
    }

    fn rewind_current_page_journal(&mut self, mark: PageCheckpointMark) {
        let (journal, mut target) = self.journal_and_inverse_target();
        let replay_work = &mut journal.replay_work;
        journal
            .inverses
            .visit_current_checkpoint_suffix_mut_reverse(
                &mut journal.inverse_pool,
                mark.journal,
                |inverse| {
                    target.toggle(inverse);
                    *replay_work = replay_work.saturating_add(1);
                },
            )
            .expect("validated candidate page journal rewinds to its fixed mark");
        journal.applied = mark.cursor;
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
            progress: self.progress,
            end_job_ejection: self.end_job_ejection,
            resume_after_output: self.resume_after_output,
            tex82_dynamic_words: self.tex82_dynamic_words,
            etex_dynamic_words: self.etex_dynamic_words,
            page_node_root_count: self.page_node_root_count,
        }
    }

    fn record_scalars(&mut self) {
        if !self.checkpoint_journal.frames.is_empty() {
            let old = self.scalar_snapshot();
            self.record_page_inverse(PageInverse::Scalars(old));
        }
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
        self.progress = old.progress;
        self.end_job_ejection = old.end_job_ejection;
        self.resume_after_output = old.resume_after_output;
        self.tex82_dynamic_words = old.tex82_dynamic_words;
        self.etex_dynamic_words = old.etex_dynamic_words;
        self.page_node_root_count = old.page_node_root_count;
    }

    #[cfg(feature = "profiling")]
    pub(crate) const fn checkpoint_replay_work(&self) -> u64 {
        self.checkpoint_journal.replay_work
    }

    #[must_use]
    fn candidate_settlement_counters(&self) -> PageCandidateSettlementCounters {
        let ForkArenaCounters {
            candidate_chunks_truncated,
            obsolete_chunks_pruned,
            ..
        } = self.checkpoint_journal.inverses.counters();
        PageCandidateSettlementCounters {
            checkpoint_captures: self.checkpoint_journal.checkpoint_captures,
            checkpoint_capture_records_scanned: 0,
            candidate_selections: self.checkpoint_journal.accepted_rewind_transitions,
            selected_journal_records_rewound: self.checkpoint_journal.accepted_rewind_work,
            candidate_acceptances: self.checkpoint_journal.candidate_acceptances,
            acceptance_payload_records_scanned: 0,
            candidate_rejections: self.checkpoint_journal.candidate_rejections,
            rejected_candidate_records_rewound: self.checkpoint_journal.candidate_rewind_work,
            rejected_prior_records_redone: self.checkpoint_journal.accepted_redo_work,
            canonical_lane_records_scanned: 0,
            canonical_values_copied: 0,
            prior_journal_chunks_released: obsolete_chunks_pruned,
            candidate_journal_chunks_released: candidate_chunks_truncated,
        }
    }

    #[cfg(test)]
    pub(crate) const fn accepted_replay_work(&self) -> [u64; 4] {
        [
            self.checkpoint_journal.accepted_rewind_work,
            self.checkpoint_journal.accepted_redo_work,
            self.checkpoint_journal.accepted_rewind_transitions,
            self.checkpoint_journal.accepted_redo_transitions,
        ]
    }

    /// Whether this checkpointable page state explicitly carries any live
    /// page-arena coordinate. A rootless state contributes no retained-prefix
    /// demand merely because the arena cursor has advanced.
    pub(crate) fn retains_page_node_handles(&self) -> bool {
        self.page_node_root_count != 0
            || !self.contribution.is_empty()
            || !self.current_page.is_empty()
            || !self.page_discards.is_empty()
            || !self.split_discards.is_empty()
    }

    #[cfg(test)]
    pub(crate) fn memo_parts(&self, arena: &PageNodeArena) -> (Vec<Node>, PageMemoState) {
        let mut nodes = Vec::with_capacity(
            self.contribution.len()
                + self.current_page.len()
                + self.page_discards.len()
                + self.split_discards.len()
                + usize::from(self.last_glue.is_some()),
        );
        for root in [
            self.contribution,
            self.current_page,
            self.page_discards,
            self.split_discards,
        ] {
            nodes.extend(
                arena
                    .span_node_cursor(root)
                    .expect("memo root belongs to the caller-owned page arena")
                    .iter()
                    .map(|node| node.to_owned_with(std::convert::identity)),
            );
        }
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
        arena: &mut PageNodeArena,
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
        let contribution_range = take(&mut cursor, state.contribution_len);
        let current_page_range = take(&mut cursor, state.current_page_len);
        let page_discards_range = take(&mut cursor, state.page_discards_len);
        let split_discards_range = take(&mut cursor, state.split_discards_len);
        let last_glue = if state.has_last_glue {
            match &nodes[cursor] {
                Node::Glue { spec, .. } => Some(*spec),
                _ => return Err(crate::MemoValueError::Invalid("invalid last-glue sentinel")),
            }
        } else {
            None
        };
        self.contribution = arena
            .publish_owned_span(nodes[contribution_range].iter().cloned())
            .map_err(|_| crate::MemoValueError::Invalid("invalid contribution page nodes"))?;
        self.current_page = arena
            .publish_owned_span(nodes[current_page_range].iter().cloned())
            .map_err(|_| crate::MemoValueError::Invalid("invalid current-page nodes"))?;
        self.page_discards = arena
            .publish_owned_span(nodes[page_discards_range].iter().cloned())
            .map_err(|_| crate::MemoValueError::Invalid("invalid page discards"))?;
        self.split_discards = arena
            .publish_owned_span(nodes[split_discards_range].iter().cloned())
            .map_err(|_| crate::MemoValueError::Invalid("invalid split discards"))?;
        self.refresh_dynamic_memory_words(arena);
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
        self.rebuild_semantic_roots_from_values();
        Ok(())
    }

    #[cfg(test)]
    fn rebuild_semantic_roots_from_values(&mut self) {
        let marks = self.current_marks_identity();
        self.semantic_roots = PageSemanticRoots {
            contribution: list_identity(self.contribution),
            page_discards: list_identity(self.page_discards),
            split_discards: list_identity(self.split_discards),
            insertions: self
                .insertions
                .iter()
                .fold(0_u64, |root, value| root ^ insertion_identity(*value)),
            marks,
        };
    }

    fn current_marks_identity(&self) -> u64 {
        let mut marks = 0_u64;
        for (mark, value) in [
            (PageMark::Top, self.top_mark.as_ref()),
            (PageMark::First, self.first_mark.as_ref()),
            (PageMark::Bot, self.bot_mark.as_ref()),
            (PageMark::SplitFirst, self.split_first_mark.as_ref()),
            (PageMark::SplitBot, self.split_bot_mark.as_ref()),
        ] {
            if let Some(value) = value {
                marks ^= mark_identity(0, mark, value);
            }
        }
        for (class, state) in &self.mark_classes {
            for mark in [
                PageMark::Top,
                PageMark::First,
                PageMark::Bot,
                PageMark::SplitFirst,
                PageMark::SplitBot,
            ] {
                if let Some(value) = state.get(mark) {
                    marks ^= mark_identity(*class, mark, value);
                }
            }
        }
        marks
    }

    pub(crate) fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
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
            .saturating_add(self.checkpoint_journal.inverse_pool.allocated_heap_bytes())
            .saturating_add(
                self.checkpoint_journal
                    .frames
                    .capacity()
                    .saturating_mul(std::mem::size_of::<PageCheckpointFrame>()),
            )
    }

    pub(crate) fn is_format_empty(&self) -> bool {
        self.contribution.is_empty()
            && self.current_page.is_empty()
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
        match mark {
            PageMark::Top => self.top_mark.as_ref(),
            PageMark::First => self.first_mark.as_ref(),
            PageMark::Bot => self.bot_mark.as_ref(),
            PageMark::SplitFirst => self.split_first_mark.as_ref(),
            PageMark::SplitBot => self.split_bot_mark.as_ref(),
        }
    }

    pub(crate) fn set_mark(&mut self, mark: PageMark, value: NodeTokenList) {
        if self.identity_enabled {
            if let Some(old) = self.mark_value(mark) {
                self.semantic_roots.marks ^= mark_identity(0, mark, old);
            }
            self.semantic_roots.marks ^= mark_identity(0, mark, &value);
        }
        if !self.checkpoint_journal.frames.is_empty() {
            self.record_page_inverse(PageInverse::Marks([
                self.top_mark,
                self.first_mark,
                self.bot_mark,
                self.split_first_mark,
                self.split_bot_mark,
            ]));
        }
        *match mark {
            PageMark::Top => &mut self.top_mark,
            PageMark::First => &mut self.first_mark,
            PageMark::Bot => &mut self.bot_mark,
            PageMark::SplitFirst => &mut self.split_first_mark,
            PageMark::SplitBot => &mut self.split_bot_mark,
        } = Some(value);
    }

    pub(crate) fn clear_mark(&mut self, mark: PageMark) {
        if self.identity_enabled
            && let Some(old) = self.mark_value(mark)
        {
            self.semantic_roots.marks ^= mark_identity(0, mark, old);
        }
        if !self.checkpoint_journal.frames.is_empty() {
            self.record_page_inverse(PageInverse::Marks([
                self.top_mark,
                self.first_mark,
                self.bot_mark,
                self.split_first_mark,
                self.split_bot_mark,
            ]));
        }
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
        if self.identity_enabled {
            if let Some(old) = self.mark_class_value(mark, class) {
                self.semantic_roots.marks ^= mark_identity(class, mark, old);
            }
            self.semantic_roots.marks ^= mark_identity(class, mark, &value);
        }
        if !self.checkpoint_journal.frames.is_empty() {
            let old = self
                .mark_class_position(class)
                .map(|position| self.mark_classes[position].1.clone());
            self.record_page_inverse(PageInverse::MarkClass { class, old });
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
        if self.identity_enabled
            && let Some(old) = self.mark_classes[position].1.get(mark)
        {
            self.semantic_roots.marks ^= mark_identity(class, mark, old);
        }
        if !self.checkpoint_journal.frames.is_empty() {
            self.record_page_inverse(PageInverse::MarkClass {
                class,
                old: Some(self.mark_classes[position].1.clone()),
            });
        }
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
            candidate: false,
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

    pub(crate) fn start_new_page(&mut self, arena: &PageNodeArena) {
        self.start_page_after_output(arena);
        // This reset is used by INITEX terminal cleanup, not by §1012's
        // return into a live `build_page` invocation.
        self.resume_after_output = false;
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
    pub(crate) fn start_page_after_output(&mut self, arena: &PageNodeArena) {
        self.record_scalars();
        self.advance_progress();
        self.resume_after_output = true;
        let current = arena
            .span_node_cursor(self.current_page)
            .expect("current page root belongs to the live arena");
        let (released, released_page_roots) = list_dynamic_usage(current);
        if !self.checkpoint_journal.frames.is_empty() {
            let current_page = std::mem::take(&mut self.current_page);
            let insertions = std::mem::take(&mut self.insertions);
            let positions = std::mem::take(&mut self.insertion_positions);
            self.record_page_inverse(PageInverse::CurrentPage(current_page));
            self.record_page_inverse(PageInverse::InsertionsReplace {
                insertions,
                positions,
            });
        } else {
            self.current_page = PageListSpan::empty();
            self.insertions.clear();
            self.insertion_positions.clear();
        }
        self.release_dynamic_word_totals(released);
        self.page_node_root_count = self
            .page_node_root_count
            .checked_sub(released_page_roots)
            .expect("released more page roots than were live");
        self.semantic_roots.insertions = 0;
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
            if self.identity_enabled {
                self.semantic_roots.insertions ^= insertion_identity(*insertion);
            }
            insertion.best_ins_index = insertion.last_ins_index;
            if self.identity_enabled {
                self.semantic_roots.insertions ^= insertion_identity(*insertion);
            }
        }
    }

    pub(crate) fn record_fire_up(&mut self, trigger_index: usize) {
        self.record_scalars();
        self.advance_progress();
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

    pub(crate) fn push_contribution(&mut self, arena: &mut PageNodeArena, node: Node) {
        self.record_scalars();
        self.record_page_inverse(PageInverse::Contribution(self.contribution));
        self.allocate_dynamic_node(&node);
        let node = arena
            .publish_owned_unique([node])
            .expect("page arena accepts contribution");
        self.contribution = arena
            .append_unique_to_span(self.contribution, node)
            .expect("page contribution roots belong to the live arena");
        self.semantic_roots.contribution = list_identity(self.contribution);
    }

    pub(crate) fn remove_contribution_range(
        &mut self,
        arena: &mut PageNodeArena,
        range: std::ops::RangeInclusive<usize>,
    ) -> PageNodeCarrier {
        let start = *range.start();
        let end = range.end().saturating_add(1);
        assert!(start <= end && end <= self.contribution.len());
        self.record_scalars();
        self.record_page_inverse(PageInverse::Contribution(self.contribution));
        let removed = arena
            .slice_span(self.contribution, start..end)
            .expect("removed contribution range belongs to the live arena");
        let prefix = arena
            .slice_span(self.contribution, 0..start)
            .expect("contribution prefix belongs to the live arena");
        let suffix = arena
            .slice_span(self.contribution, end..self.contribution.len())
            .expect("contribution suffix belongs to the live arena");
        let removed_view = arena
            .span_node_cursor(removed)
            .expect("removed contribution remains live");
        let (words, roots) = list_dynamic_usage(removed_view);
        self.release_dynamic_word_totals(words);
        self.page_node_root_count = self
            .page_node_root_count
            .checked_sub(roots)
            .expect("released more page roots than were live");
        self.contribution = arena
            .compose_spans(&[prefix, suffix])
            .expect("remaining contribution ranges compose");
        self.semantic_roots.contribution = list_identity(self.contribution);
        PageNodeCarrier { list: removed }
    }

    pub(crate) fn prepend_contribution(&mut self, arena: &mut PageNodeArena, node: Node) {
        self.record_scalars();
        self.record_page_inverse(PageInverse::Contribution(self.contribution));
        self.allocate_dynamic_node(&node);
        let node = arena
            .publish_owned_span([node])
            .expect("page arena accepts contribution");
        self.contribution = arena
            .compose_spans(&[node, self.contribution])
            .expect("page contribution roots belong to the live arena");
        self.semantic_roots.contribution = list_identity(self.contribution);
    }

    pub(crate) fn contribution<'a>(&self, arena: &'a PageNodeArena) -> PageContributionView<'a> {
        PageContributionView {
            nodes: arena
                .span_node_cursor(self.contribution)
                .expect("page contribution root belongs to the live arena"),
        }
    }

    pub(crate) fn contribution_front<'a>(
        &self,
        arena: &'a PageNodeArena,
    ) -> Option<crate::NodeView<'a>> {
        self.contribution(arena).front()
    }

    pub(crate) fn contribution_second<'a>(
        &self,
        arena: &'a PageNodeArena,
    ) -> Option<crate::NodeView<'a>> {
        self.contribution(arena).get(1)
    }

    pub(crate) fn pop_contribution_front(
        &mut self,
        arena: &mut PageNodeArena,
    ) -> Option<PageNodeCarrier> {
        if self.contribution.is_empty() {
            return None;
        }
        let node = arena.span_node_cursor(self.contribution).ok()?.get(0)?;
        let words = node.tex_memory_words(false).1;
        let etex_words = node.tex_memory_words(true).1;
        let retains_root = node_retains_page_handle(node);
        self.record_scalars();
        self.advance_progress();
        self.record_page_inverse(PageInverse::Contribution(self.contribution));
        let removed = arena.slice_span(self.contribution, 0..1).ok()?;
        self.contribution = arena
            .slice_span(self.contribution, 1..self.contribution.len())
            .ok()?;
        self.release_dynamic_word_totals((words, etex_words));
        self.page_node_root_count = self
            .page_node_root_count
            .checked_sub(usize::from(retains_root))
            .expect("released more page roots than were live");
        self.semantic_roots.contribution = list_identity(self.contribution);
        Some(PageNodeCarrier { list: removed })
    }

    pub(crate) fn prepend_contributions(&mut self, arena: &mut PageNodeArena, nodes: PageListId) {
        if nodes.is_empty() {
            return;
        }
        self.record_scalars();
        self.record_page_inverse(PageInverse::Contribution(self.contribution));
        let nodes = arena
            .admit_span(nodes)
            .expect("heldover page contribution belongs to the live owner");
        let view = arena
            .span_node_cursor(nodes)
            .expect("heldover page contribution is live");
        let (words, roots) = list_dynamic_usage(view);
        self.allocate_dynamic_word_totals(words);
        self.page_node_root_count = self.page_node_root_count.saturating_add(roots);
        self.contribution = arena
            .compose_spans(&[nodes, self.contribution])
            .expect("heldover and live contribution roots compose");
        self.semantic_roots.contribution = list_identity(self.contribution);
    }

    pub(crate) fn append_contributions(&mut self, arena: &mut PageNodeArena, nodes: PageListId) {
        if nodes.is_empty() {
            return;
        }
        self.record_scalars();
        self.record_page_inverse(PageInverse::Contribution(self.contribution));
        let nodes = arena
            .admit_span(nodes)
            .expect("page contribution list belongs to the live owner");
        let view = arena
            .span_node_cursor(nodes)
            .expect("page contribution list is live");
        let (words, roots) = list_dynamic_usage(view);
        self.allocate_dynamic_word_totals(words);
        self.page_node_root_count = self.page_node_root_count.saturating_add(roots);
        self.contribution = arena
            .compose_spans(&[self.contribution, nodes])
            .expect("page contribution roots compose");
        self.semantic_roots.contribution = list_identity(self.contribution);
    }

    pub(crate) fn append_unique_contributions(
        &mut self,
        arena: &mut PageNodeArena,
        nodes: crate::page_node_arena::UniquePageList,
    ) {
        let list = nodes.list();
        if list.is_empty() {
            return;
        }
        self.record_scalars();
        self.record_page_inverse(PageInverse::Contribution(self.contribution));
        let span = arena
            .admit_span(list)
            .expect("unique contribution belongs to the live owner");
        let view = arena
            .span_node_cursor(span)
            .expect("unique contribution remains live");
        let (words, roots) = list_dynamic_usage(view);
        self.allocate_dynamic_word_totals(words);
        self.page_node_root_count = self.page_node_root_count.saturating_add(roots);
        self.contribution = arena
            .append_unique_to_span(self.contribution, nodes)
            .expect("unique contribution suffix splices into the live root");
        self.semantic_roots.contribution = list_identity(self.contribution);
    }

    pub(crate) fn current_page<'a>(&self, arena: &'a PageNodeArena) -> PageCurrentIter<'a> {
        arena
            .span_node_cursor(self.current_page)
            .expect("current page root belongs to the live arena")
            .iter()
    }

    pub(crate) fn push_page_discard(&mut self, arena: &mut PageNodeArena, node: Node) {
        self.record_scalars();
        self.record_page_inverse(PageInverse::PageDiscards(self.page_discards));
        self.allocate_dynamic_node(&node);
        let node = arena
            .publish_owned_unique([node])
            .expect("page arena accepts discard");
        self.page_discards = arena
            .append_unique_to_span(self.page_discards, node)
            .expect("page discard roots compose");
        self.semantic_roots.page_discards = list_identity(self.page_discards);
    }

    pub(crate) fn push_page_discard_carrier(
        &mut self,
        arena: &mut PageNodeArena,
        carrier: PageNodeCarrier,
    ) {
        self.record_scalars();
        self.record_page_inverse(PageInverse::PageDiscards(self.page_discards));
        self.page_discards = arena
            .compose_spans(&[self.page_discards, carrier.list])
            .expect("page discard carrier belongs to the live arena");
        self.allocate_list_dynamic_usage(arena, carrier.list);
        self.semantic_roots.page_discards = list_identity(self.page_discards);
    }

    pub(crate) fn take_page_discards(&mut self, arena: &PageNodeArena) -> PageListId {
        self.record_scalars();
        self.record_page_inverse(PageInverse::PageDiscards(self.page_discards));
        let nodes = std::mem::take(&mut self.page_discards);
        self.release_list_dynamic_usage(arena, nodes);
        self.semantic_roots.page_discards = SemanticSequenceIdentity::empty();
        nodes.list()
    }

    pub(crate) fn clear_page_discards(&mut self, arena: &PageNodeArena) {
        self.record_scalars();
        self.record_page_inverse(PageInverse::PageDiscards(self.page_discards));
        self.release_list_dynamic_usage(arena, self.page_discards);
        self.page_discards = PageListSpan::empty();
        self.semantic_roots.page_discards = SemanticSequenceIdentity::empty();
    }

    pub(crate) fn set_split_discards(&mut self, arena: &PageNodeArena, nodes: PageListId) {
        self.record_scalars();
        self.record_page_inverse(PageInverse::SplitDiscards(self.split_discards));
        self.release_list_dynamic_usage(arena, self.split_discards);
        let nodes = arena
            .admit_span(nodes)
            .expect("split discards belong to the live page owner");
        self.allocate_list_dynamic_usage(arena, nodes);
        self.semantic_roots.split_discards = list_identity(nodes);
        self.split_discards = nodes;
    }

    pub(crate) fn take_split_discards(&mut self, arena: &PageNodeArena) -> PageListId {
        self.record_scalars();
        self.record_page_inverse(PageInverse::SplitDiscards(self.split_discards));
        let nodes = std::mem::take(&mut self.split_discards);
        self.release_list_dynamic_usage(arena, nodes);
        self.semantic_roots.split_discards = SemanticSequenceIdentity::empty();
        nodes.list()
    }

    pub(crate) fn clear_split_discards(&mut self, arena: &PageNodeArena) {
        self.record_scalars();
        self.record_page_inverse(PageInverse::SplitDiscards(self.split_discards));
        self.release_list_dynamic_usage(arena, self.split_discards);
        self.split_discards = PageListSpan::empty();
        self.semantic_roots.split_discards = SemanticSequenceIdentity::empty();
    }

    pub(crate) fn current_page_tail<'a>(
        &self,
        arena: &'a PageNodeArena,
    ) -> Option<crate::node_arena::NodeView<'a>> {
        arena
            .span_node_cursor(self.current_page)
            .expect("current page root belongs to the live arena")
            .last()
    }

    pub(crate) fn current_page_len(&self) -> usize {
        self.current_page.len()
    }

    pub(crate) fn push_current_page(&mut self, arena: &mut PageNodeArena, node: Node) {
        self.record_scalars();
        self.record_page_inverse(PageInverse::CurrentPage(self.current_page));
        self.allocate_dynamic_node(&node);
        let node = arena
            .publish_owned_unique([node])
            .expect("page arena accepts current node");
        self.current_page = arena
            .append_unique_to_span(self.current_page, node)
            .expect("current page roots compose");
    }

    pub(crate) fn push_current_page_carrier(
        &mut self,
        arena: &mut PageNodeArena,
        carrier: PageNodeCarrier,
    ) {
        self.record_scalars();
        self.record_page_inverse(PageInverse::CurrentPage(self.current_page));
        self.allocate_list_dynamic_usage(arena, carrier.list);
        self.current_page = arena
            .compose_spans(&[self.current_page, carrier.list])
            .expect("current page carrier belongs to the live arena");
    }

    pub(crate) fn push_current_page_list(&mut self, arena: &mut PageNodeArena, list: PageListId) {
        if list.is_empty() {
            return;
        }
        self.record_scalars();
        self.record_page_inverse(PageInverse::CurrentPage(self.current_page));
        let list = arena
            .admit_span(list)
            .expect("current-page list belongs to the live owner");
        let nodes = arena
            .span_node_cursor(list)
            .expect("current-page list is live");
        let (words, roots) = list_dynamic_usage(nodes);
        self.allocate_dynamic_word_totals(words);
        self.page_node_root_count = self.page_node_root_count.saturating_add(roots);
        self.current_page = arena
            .compose_spans(&[self.current_page, list])
            .expect("current-page roots compose");
    }

    pub(crate) fn push_current_page_replacement(
        &mut self,
        arena: &mut PageNodeArena,
        carrier: PageNodeCarrier,
        replacement: Node,
    ) {
        self.discard_carrier(carrier);
        self.push_current_page(arena, replacement);
    }

    pub(crate) fn discard_carrier(&mut self, _carrier: PageNodeCarrier) {}

    /// Removes one logical current-page tail.
    #[cfg(any(test, feature = "profiling"))]
    pub(crate) fn pop_current_page(&mut self, arena: &mut PageNodeArena) -> Option<Node> {
        let len = self.current_page.len();
        let node = arena
            .span_list(self.current_page)
            .ok()?
            .get(len.checked_sub(1)?)?
            .clone();
        self.record_scalars();
        self.record_page_inverse(PageInverse::CurrentPage(self.current_page));
        self.current_page = arena.slice_span(self.current_page, 0..len - 1).ok()?;
        self.release_dynamic_node(&node);
        Some(node)
    }

    pub(crate) fn page_insertions(&self) -> PageInsertionView<'_> {
        PageInsertionView { page: self }
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
        if self.identity_enabled {
            if let Some(old) = self.page_insertion(class) {
                self.semantic_roots.insertions ^= insertion_identity(old);
            }
            self.semantic_roots.insertions ^= insertion_identity(insertion);
        }
        let old = (!self.checkpoint_journal.frames.is_empty())
            .then(|| self.page_insertion(class))
            .flatten();
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
        arena: &mut PageNodeArena,
        split_index: usize,
    ) -> (PageListId, PageListId) {
        self.record_scalars();
        self.record_page_inverse(PageInverse::CurrentPage(self.current_page));
        let split_index = split_index.min(self.current_page.len());
        let prefix = arena
            .slice_span(self.current_page, 0..split_index)
            .expect("current-page prefix belongs to the live arena");
        let suffix = arena
            .slice_span(self.current_page, split_index..self.current_page.len())
            .expect("current-page suffix belongs to the live arena");
        let current = arena
            .span_list(self.current_page)
            .expect("current page belongs to the live arena");
        let mut words = (0_usize, 0_usize);
        let mut roots = 0_usize;
        current.for_each(|node| {
            words.0 = words.0.saturating_add(node.tex_memory_words(false).1);
            words.1 = words.1.saturating_add(node.tex_memory_words(true).1);
            roots += usize::from(node_retains_page_handle(node.into()));
        });
        let _ = current;
        self.current_page = PageListSpan::empty();
        self.release_dynamic_word_totals(words);
        self.page_node_root_count = self
            .page_node_root_count
            .checked_sub(roots)
            .expect("released more page roots than were live");
        (prefix.list(), suffix.list())
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
            .checked_add(usize::from(node_retains_page_handle(node.into())))
            .expect("page root accounting overflow");
    }

    #[cfg(any(test, feature = "profiling"))]
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
            .checked_sub(usize::from(node_retains_page_handle(node.into())))
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

    fn allocate_list_dynamic_usage(&mut self, arena: &PageNodeArena, list: PageListSpan) {
        let nodes = arena
            .span_node_cursor(list)
            .expect("page list belongs to the live arena");
        let (words, roots) = list_dynamic_usage(nodes);
        self.allocate_dynamic_word_totals(words);
        self.page_node_root_count = self
            .page_node_root_count
            .checked_add(roots)
            .expect("page root accounting overflow");
    }

    fn release_list_dynamic_usage(&mut self, arena: &PageNodeArena, list: PageListSpan) {
        let nodes = arena
            .span_node_cursor(list)
            .expect("page list belongs to the live arena");
        let (words, roots) = list_dynamic_usage(nodes);
        self.release_dynamic_word_totals(words);
        self.page_node_root_count = self
            .page_node_root_count
            .checked_sub(roots)
            .expect("released more page roots than were live");
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
    fn refresh_dynamic_memory_words(&mut self, arena: &PageNodeArena) {
        let (tex82, etex) = [
            self.contribution,
            self.current_page,
            self.page_discards,
            self.split_discards,
        ]
        .into_iter()
        .fold((0_usize, 0_usize), |words, root| {
            arena
                .span_node_cursor(root)
                .expect("dynamic-memory root belongs to the page arena")
                .iter()
                .fold(words, |words, node| {
                    (
                        words.0.saturating_add(node.tex_memory_words(false).1),
                        words.1.saturating_add(node.tex_memory_words(true).1),
                    )
                })
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

impl PageCheckpointMark {
    #[must_use]
    pub(crate) const fn reachable_state_identity_root(self) -> Option<u64> {
        self.reachable_state_identity_root
    }
}

fn page_identity_hasher(domain: &[u8]) -> ahash::AHasher {
    let state = RandomState::with_seeds(
        0x756d_6265_725f_7061,
        0x6765_5f69_6465_6e74,
        0x6974_795f_7631_5f66,
        0x6978_6564_5f73_6565,
    );
    let mut hasher = state.build_hasher();
    hasher.write(domain);
    hasher
}

fn hash_page_glue(glue: GlueSpec, hasher: &mut impl Hasher) {
    glue.width.raw().hash(hasher);
    glue.stretch.raw().hash(hasher);
    (glue.stretch_order as u8).hash(hasher);
    glue.shrink.raw().hash(hasher);
    (glue.shrink_order as u8).hash(hasher);
}

fn insertion_identity(insertion: PageInsertion) -> u64 {
    let mut hasher = page_identity_hasher(b"umber-page-insertion-v1");
    insertion.class.hash(&mut hasher);
    match insertion.status {
        PageInsertionStatus::Inserting => 0_u8.hash(&mut hasher),
        PageInsertionStatus::SplitUp {
            broken_ins_index,
            broken_at,
        } => {
            1_u8.hash(&mut hasher);
            (broken_ins_index as u64).hash(&mut hasher);
            broken_at.map(|value| value as u64).hash(&mut hasher);
        }
    }
    insertion.height.raw().hash(&mut hasher);
    insertion
        .last_ins_index
        .map(|value| value as u64)
        .hash(&mut hasher);
    insertion
        .best_ins_index
        .map(|value| value as u64)
        .hash(&mut hasher);
    hasher.finish()
}

fn mark_identity(class: u16, mark: PageMark, value: &NodeTokenList) -> u64 {
    let mut hasher = page_identity_hasher(b"umber-page-mark-v1");
    class.hash(&mut hasher);
    mark.index().hash(&mut hasher);
    value.hash(&mut hasher);
    hasher.finish()
}

fn list_dynamic_usage(nodes: NodeCursor<'_>) -> ((usize, usize), usize) {
    let mut words = (0_usize, 0_usize);
    let mut roots = 0_usize;
    nodes.for_each(|node| {
        words.0 = words.0.saturating_add(node.tex_memory_words(false).1);
        words.1 = words.1.saturating_add(node.tex_memory_words(true).1);
        roots = roots.saturating_add(usize::from(node_retains_page_handle(node)));
    });
    (words, roots)
}

trait PageListRoot {
    fn list_id(self) -> PageListId;
}

impl PageListRoot for PageListId {
    fn list_id(self) -> PageListId {
        self
    }
}

impl PageListRoot for PageListSpan {
    fn list_id(self) -> PageListId {
        self.list()
    }
}

fn list_identity(list: impl PageListRoot) -> SemanticSequenceIdentity {
    let list = list.list_id();
    SemanticSequenceIdentity::from_raw(list.semantic_identity().unwrap_or(0), list.len())
}

fn node_retains_page_handle(node: crate::NodeView<'_>) -> bool {
    let mut retains = false;
    node.visit_semantic_node_lists(|list: &PageListId| retains |= !list.is_empty());
    retains
}

#[cfg(test)]
mod tests;
