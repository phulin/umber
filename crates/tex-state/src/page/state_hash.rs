use super::{PageBreak, PageBuilderState, PageContents, PageDimension, PageInsertionStatus};
use crate::glue::GlueSpec;
use crate::node::{Node, NodeTokenList};
use crate::state_hash::{StateHashComponent, StateHashFragment, StateHasher};
use std::collections::VecDeque;

const PAGE_PROJECTION_DOMAIN: u64 = 0x7061_6765_5f70_726a;
const PAGE_SCALARS_DOMAIN: u64 = 0x7061_6765_5f73_6361;
const PAGE_INSERTIONS_DOMAIN: u64 = 0x7061_6765_5f69_6e73;
const PAGE_MARK_CLASSES_DOMAIN: u64 = 0x7061_6765_5f6d_6172;
const PAGE_CONTRIBUTION_DOMAIN: u64 = 0x7061_6765_5f63_6f6e;
const PAGE_CURRENT_DOMAIN: u64 = 0x7061_6765_5f63_7572;
const PAGE_DISCARDS_DOMAIN: u64 = 0x7061_6765_5f64_6973;
const SPLIT_DISCARDS_DOMAIN: u64 = 0x7370_6c69_745f_6469;

/// Page semantic hashes retain no runtime roots. The cache remains a zero-size
/// call-site capability while the final checkpoint layer owns any detached
/// fragment reuse.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct PageHashCache;

/// Bounded value cursor used only to detect an unchanged page projection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PageStateHashCursor {
    dimensions: [crate::scaled::Scaled; 9],
    contents: PageContents,
    lengths: [usize; 6],
    last_penalty: i32,
    last_kern: crate::scaled::Scaled,
    last_node_type: i32,
    insert_penalties: i32,
    dead_cycles: i32,
    least_page_cost: i32,
    best_page_break: Option<PageBreak>,
    best_size: crate::scaled::Scaled,
    fire_up: Option<super::PageFireUp>,
}

impl PageBuilderState {
    pub(crate) fn state_hash_cursor(&self) -> PageStateHashCursor {
        PageStateHashCursor {
            dimensions: [
                self.page_goal,
                self.page_total,
                self.page_stretch,
                self.page_fil_stretch,
                self.page_fill_stretch,
                self.page_filll_stretch,
                self.page_shrink,
                self.page_depth,
                self.page_max_depth,
            ],
            contents: self.contents,
            lengths: [
                self.contribution.len(),
                self.current_page.len(),
                self.page_discards.len(),
                self.split_discards.len(),
                self.insertions.len(),
                self.mark_classes.len(),
            ],
            last_penalty: self.last_penalty,
            last_kern: self.last_kern,
            last_node_type: self.last_node_type,
            insert_penalties: self.insert_penalties,
            dead_cycles: self.dead_cycles,
            least_page_cost: self.least_page_cost,
            best_page_break: self.best_page_break,
            best_size: self.best_size,
            fire_up: self.fire_up,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn hash_semantic(
        &self,
        hasher: &mut StateHasher,
        _cache: &mut PageHashCache,
        mut hash_queue: impl FnMut(&VecDeque<Node>, &mut StateHasher) -> usize,
        mut hash_nodes: impl FnMut(&[Node], &mut StateHasher) -> usize,
        mut hash_glue: impl FnMut(GlueSpec, &mut StateHasher),
        mut hash_tokens: impl FnMut(&NodeTokenList, &mut StateHasher),
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
                    Some(glue) => {
                        projection.bool(true);
                        hash_glue(glue, projection);
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
                    &self.top_mark,
                    &self.first_mark,
                    &self.bot_mark,
                    &self.split_first_mark,
                    &self.split_bot_mark,
                ] {
                    projection.bool(mark.is_some());
                    if let Some(mark) = mark {
                        hash_tokens(mark, projection);
                    }
                }
            },
        );

        let insertions = StateHashFragment::from_measured_builder_counted(
            PAGE_INSERTIONS_DOMAIN,
            StateHashComponent::PageInsertions,
            |projection| {
                projection.usize(self.insertions.len());
                for insertion in &self.insertions {
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

        let mark_classes = StateHashFragment::from_measured_builder_counted(
            PAGE_MARK_CLASSES_DOMAIN,
            StateHashComponent::PageMarks,
            |projection| {
                projection.usize(self.mark_classes.len());
                for (&class, marks) in &self.mark_classes {
                    projection.u16(class);
                    for mark in &marks.marks {
                        projection.bool(mark.is_some());
                        if let Some(mark) = mark {
                            hash_tokens(mark, projection);
                        }
                    }
                }
                self.mark_classes.len()
            },
        );

        let contribution = StateHashFragment::from_measured_builder_counted(
            PAGE_CONTRIBUTION_DOMAIN,
            StateHashComponent::PageContribution,
            |projection| hash_queue(&self.contribution, projection),
        );
        let current_page = StateHashFragment::from_measured_builder_counted(
            PAGE_CURRENT_DOMAIN,
            StateHashComponent::PageCurrent,
            |projection| hash_nodes(self.current_page.as_slice(), projection),
        );
        let page_discards = StateHashFragment::from_measured_builder_counted(
            PAGE_DISCARDS_DOMAIN,
            StateHashComponent::PageDiscards,
            |projection| hash_nodes(&self.page_discards, projection),
        );
        let split_discards = StateHashFragment::from_measured_builder_counted(
            SPLIT_DISCARDS_DOMAIN,
            StateHashComponent::PageDiscards,
            |projection| hash_nodes(&self.split_discards, projection),
        );

        StateHashFragment::from_exact_builder(PAGE_PROJECTION_DOMAIN, |projection| {
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

fn hash_optional_usize(value: Option<usize>, hasher: &mut StateHasher) {
    match value {
        Some(value) => {
            hasher.bool(true);
            hasher.usize(value);
        }
        None => hasher.bool(false),
    }
}
