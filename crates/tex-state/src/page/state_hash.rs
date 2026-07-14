use super::sequence::{PageNodeSequence, PageNodeTree, PageTailNode};
use super::{
    MarkClassState, PageBreak, PageBuilderState, PageContents, PageDimension, PageInsertion,
    PageInsertionStatus,
};
use crate::ids::{GlueId, TokenListId};
use crate::node::Node;
use crate::state_hash::{CachedProjection, StateHashComponent, StateHashFragment, StateHasher};
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
const PAGE_NODE_ITEM_DOMAIN: u64 = 0x7061_6765_5f69_746d;

#[derive(Clone, Debug, Default)]
pub(crate) struct PageHashCache {
    insertions: Option<CachedProjection<Arc<Vec<PageInsertion>>>>,
    mark_classes: Option<CachedProjection<Arc<BTreeMap<u16, MarkClassState>>>>,
    contribution: Option<CachedProjection<Arc<VecDeque<Node>>>>,
    page_discards: Option<CachedProjection<Arc<Vec<Node>>>>,
    split_discards: Option<CachedProjection<Arc<Vec<Node>>>>,
}

impl PageHashCache {
    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }
}

fn project_page_tree(
    tree: &Arc<PageNodeTree>,
    hash_nodes: &mut impl FnMut(&[Node], &mut StateHasher) -> usize,
) -> StateHashFragment {
    let projection = match tree.as_ref() {
        PageNodeTree::Leaf { projection, .. } | PageNodeTree::Branch { projection, .. } => {
            projection
        }
    };
    *projection.get_or_init(|| match tree.as_ref() {
        PageNodeTree::Leaf { nodes, .. } => StateHashFragment::from_measured_builder_counted(
            PAGE_NODE_CHUNK_DOMAIN,
            StateHashComponent::PageCurrent,
            |projection| hash_nodes(nodes, projection),
        ),
        PageNodeTree::Branch {
            height,
            len,
            left,
            right,
            ..
        } => {
            let left = project_page_tree(left, hash_nodes);
            let right = project_page_tree(right, hash_nodes);
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
    })
}

/// Cheap semantic-root key for checkpoint hash-base reuse.
///
/// Scalars compare by value and collections compare by immutable root. A miss
/// merely recomputes the canonical page projection; no pointer enters its
/// fingerprint.
#[derive(Clone, Debug, PartialEq)]
struct PageStateHashScalars {
    page_goal: crate::scaled::Scaled,
    page_total: crate::scaled::Scaled,
    page_stretch: crate::scaled::Scaled,
    page_fil_stretch: crate::scaled::Scaled,
    page_fill_stretch: crate::scaled::Scaled,
    page_filll_stretch: crate::scaled::Scaled,
    page_shrink: crate::scaled::Scaled,
    page_depth: crate::scaled::Scaled,
    page_max_depth: crate::scaled::Scaled,
    contents: PageContents,
    last_glue: Option<GlueId>,
    last_penalty: i32,
    last_kern: crate::scaled::Scaled,
    last_node_type: i32,
    insert_penalties: i32,
    dead_cycles: i32,
    least_page_cost: i32,
    best_page_break: Option<PageBreak>,
    best_size: crate::scaled::Scaled,
    fire_up: Option<super::PageFireUp>,
    top_mark: TokenListId,
    first_mark: TokenListId,
    bot_mark: TokenListId,
    split_first_mark: TokenListId,
    split_bot_mark: TokenListId,
}

/// Discardable semantic-root key that does not retain mutable page buffers.
///
/// The actual `Snapshot` strongly owns its `PageBuilderState`. The rolling
/// state-hash base only needs allocation identity to detect an unchanged root,
/// so weak forest/tail keys avoid forcing the next `Arc::make_mut` to clone
/// node storage after the checkpoint callback releases its snapshot.
#[derive(Clone, Debug)]
pub(crate) struct PageStateHashCursor {
    scalars: PageStateHashScalars,
    contribution: Arc<VecDeque<Node>>,
    current_page_len: usize,
    current_page_forest: Weak<Vec<Arc<PageNodeTree>>>,
    current_page_tail: Weak<Vec<PageTailNode>>,
    page_discards: Arc<Vec<Node>>,
    split_discards: Arc<Vec<Node>>,
    insertions: Arc<Vec<PageInsertion>>,
    mark_classes: Arc<BTreeMap<u16, MarkClassState>>,
}

impl PartialEq for PageStateHashCursor {
    fn eq(&self, other: &Self) -> bool {
        self.scalars == other.scalars
            && self.current_page_len == other.current_page_len
            && Arc::ptr_eq(&self.contribution, &other.contribution)
            && Weak::ptr_eq(&self.current_page_forest, &other.current_page_forest)
            && Weak::ptr_eq(&self.current_page_tail, &other.current_page_tail)
            && Arc::ptr_eq(&self.page_discards, &other.page_discards)
            && Arc::ptr_eq(&self.split_discards, &other.split_discards)
            && Arc::ptr_eq(&self.insertions, &other.insertions)
            && Arc::ptr_eq(&self.mark_classes, &other.mark_classes)
    }
}

impl Eq for PageStateHashCursor {}

impl PageBuilderState {
    pub(crate) fn state_hash_cursor(&self) -> PageStateHashCursor {
        PageStateHashCursor {
            scalars: PageStateHashScalars {
                page_goal: self.page_goal,
                page_total: self.page_total,
                page_stretch: self.page_stretch,
                page_fil_stretch: self.page_fil_stretch,
                page_fill_stretch: self.page_fill_stretch,
                page_filll_stretch: self.page_filll_stretch,
                page_shrink: self.page_shrink,
                page_depth: self.page_depth,
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
                top_mark: self.top_mark,
                first_mark: self.first_mark,
                bot_mark: self.bot_mark,
                split_first_mark: self.split_first_mark,
                split_bot_mark: self.split_bot_mark,
            },
            contribution: Arc::clone(&self.contribution),
            current_page_len: self.current_page.len,
            current_page_forest: Arc::downgrade(&self.current_page.forest),
            current_page_tail: Arc::downgrade(&self.current_page.tail),
            page_discards: Arc::clone(&self.page_discards),
            split_discards: Arc::clone(&self.split_discards),
            insertions: Arc::clone(&self.insertions),
            mark_classes: Arc::clone(&self.mark_classes),
        }
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
        let current_page = project_page_nodes(&self.current_page, &mut hash_nodes);
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
    cached: &mut Option<CachedProjection<Arc<T>>>,
    root: &Arc<T>,
    domain: u64,
    component: StateHashComponent,
    build: impl FnOnce(&mut StateHasher) -> usize,
) -> StateHashFragment {
    if let Some(fragment) = cached
        .as_ref()
        .and_then(|cached| cached.fragment_if(|cached_root| Arc::ptr_eq(cached_root, root)))
    {
        return fragment;
    }
    let fragment = StateHashFragment::from_measured_builder_counted(domain, component, build);
    *cached = Some(CachedProjection::new(Arc::clone(root), fragment));
    fragment
}

fn project_page_nodes(
    nodes: &PageNodeSequence,
    hash_nodes: &mut impl FnMut(&[Node], &mut StateHasher) -> usize,
) -> StateHashFragment {
    let tail = project_page_tail(&nodes.tail, hash_nodes);
    StateHashFragment::from_measured_builder(
        PAGE_CURRENT_DOMAIN,
        StateHashComponent::PageCurrent,
        0,
        |projection| {
            projection.usize(nodes.len());
            projection.usize(nodes.forest.len());
            for root in nodes.forest.iter() {
                project_page_tree(root, hash_nodes).apply(projection);
            }
            tail.apply(projection);
        },
    )
}

fn project_page_tail(
    nodes: &[PageTailNode],
    hash_nodes: &mut impl FnMut(&[Node], &mut StateHasher) -> usize,
) -> StateHashFragment {
    for tail_node in nodes {
        tail_node.projection.get_or_init(|| {
            StateHashFragment::from_measured_builder_counted(
                PAGE_NODE_ITEM_DOMAIN,
                StateHashComponent::PageCurrent,
                |projection| hash_nodes(std::slice::from_ref(&tail_node.node), projection),
            )
        });
    }
    StateHashFragment::from_measured_builder(
        PAGE_NODE_CHUNK_DOMAIN,
        StateHashComponent::PageCurrent,
        0,
        |projection| {
            projection.usize(nodes.len());
            for tail_node in nodes {
                tail_node
                    .projection
                    .get()
                    .expect("page tail projection was initialized")
                    .apply(projection);
            }
        },
    )
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
