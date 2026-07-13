use super::sequence::{PageNodeSequence, PageNodeTree};
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

/// Maximum number of derived page-tree projections retained by one timeline.
///
/// A normal TeX page is far smaller than this. Crossing the limit first drops
/// dead weak roots, then evicts arbitrary derived entries. Eviction can only
/// cause canonical recomputation; it cannot affect checkpoint semantics.
const PAGE_TREE_CACHE_LIMIT: usize = 4_096;

#[derive(Clone, Debug)]
pub(crate) struct PageHashCache {
    insertions: Option<CachedProjection<Arc<Vec<PageInsertion>>>>,
    mark_classes: Option<CachedProjection<Arc<BTreeMap<u16, MarkClassState>>>>,
    contribution: Option<CachedProjection<Arc<VecDeque<Node>>>>,
    current_page_trees: BTreeMap<usize, CachedProjection<Weak<PageNodeTree>>>,
    current_page_tail: Option<CachedProjection<Arc<Vec<Node>>>>,
    page_discards: Option<CachedProjection<Arc<Vec<Node>>>>,
    split_discards: Option<CachedProjection<Arc<Vec<Node>>>>,
    tree_limit: usize,
}

impl Default for PageHashCache {
    fn default() -> Self {
        Self::with_tree_limit(PAGE_TREE_CACHE_LIMIT)
    }
}

impl PageHashCache {
    fn with_tree_limit(tree_limit: usize) -> Self {
        Self {
            insertions: None,
            mark_classes: None,
            contribution: None,
            current_page_trees: BTreeMap::new(),
            current_page_tail: None,
            page_discards: None,
            split_discards: None,
            tree_limit,
        }
    }

    pub(crate) fn clear(&mut self) {
        *self = Self::with_tree_limit(self.tree_limit);
    }

    fn enforce_tree_limit(&mut self) {
        if self.current_page_trees.len() <= self.tree_limit {
            return;
        }
        self.current_page_trees.retain(|_, cached| {
            cached
                .fragment_if(|root| root.strong_count() != 0)
                .is_some()
        });
        while self.current_page_trees.len() > self.tree_limit {
            self.current_page_trees.pop_first();
        }
    }

    #[cfg(test)]
    pub(super) fn testing_with_tree_limit(tree_limit: usize) -> Self {
        Self::with_tree_limit(tree_limit)
    }

    #[cfg(test)]
    pub(super) fn testing_tree_entries(&self) -> usize {
        self.current_page_trees.len()
    }
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

impl PageBuilderState {
    pub(crate) fn state_hash_cursor(&self) -> PageStateHashCursor {
        PageStateHashCursor(self.clone())
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
        let current_page = project_page_nodes(cache, &self.current_page, &mut hash_nodes);
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
    cache: &mut PageHashCache,
    nodes: &PageNodeSequence,
    hash_nodes: &mut impl FnMut(&[Node], &mut StateHasher) -> usize,
) -> StateHashFragment {
    if nodes.len == 0 {
        cache.current_page_trees.clear();
    }
    let mut roots = Vec::with_capacity(nodes.forest.len());
    for root in nodes.forest.iter() {
        roots.push(project_page_tree(
            &mut cache.current_page_trees,
            root,
            hash_nodes,
        ));
    }
    cache.enforce_tree_limit();
    let tail = project_arc(
        &mut cache.current_page_tail,
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
    cache: &mut BTreeMap<usize, CachedProjection<Weak<PageNodeTree>>>,
    root: &Arc<PageNodeTree>,
    hash_nodes: &mut impl FnMut(&[Node], &mut StateHasher) -> usize,
) -> StateHashFragment {
    let key = Arc::as_ptr(root) as usize;
    if let Some(fragment) = cache.get(&key).and_then(|cached| {
        cached.fragment_if(|cached_root| {
            cached_root
                .upgrade()
                .is_some_and(|cached_root| Arc::ptr_eq(&cached_root, root))
        })
    }) {
        return fragment;
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
    cache.insert(key, CachedProjection::new(Arc::downgrade(root), fragment));
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
