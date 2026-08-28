use super::{PageListId, PageMaterialActiveListBuilder, PageMaterialArena};
use crate::node::Node;
use crate::node_sequence::SemanticSequenceIdentity;

type PageMaterialNode = Node<PageListId>;

fn penalties(values: &[i32]) -> Vec<PageMaterialNode> {
    values.iter().copied().map(Node::Penalty).collect()
}

fn identity(nodes: &[PageMaterialNode]) -> SemanticSequenceIdentity {
    SemanticSequenceIdentity::from_nodes(nodes)
}

fn resolved(arena: &PageMaterialArena, list: PageListId) -> Vec<PageMaterialNode> {
    arena
        .list(list)
        .expect("live page list")
        .iter()
        .cloned()
        .collect()
}

#[test]
fn disabled_demand_performs_no_semantic_hash_work() {
    let mut arena = PageMaterialArena::with_chunk_bytes(32);
    let list = arena
        .publish_owned(penalties(&[10, 20, 30]))
        .expect("publish");

    assert_eq!(arena.semantic_hash_work(), 0);
    assert_eq!(arena.semantic_summary_work(), 0);
    assert_eq!(list.semantic_identity(), None);
    assert_eq!(arena.counters().source_nodes_copied, 0);
    assert_eq!(arena.counters().new_semantic_nodes, 3);
}

#[test]
fn parent_nodes_reject_foreign_region_children_without_partial_publication() {
    let mut local = PageMaterialArena::with_chunk_bytes(32);
    local.enable_semantic_identity();
    let before_hash_work = local.semantic_hash_work();
    let mut foreign = PageMaterialArena::with_chunk_bytes(32);
    let foreign_child = foreign
        .publish_owned(penalties(&[91]))
        .expect("foreign child");

    let result = local.publish_owned([
        Node::Penalty(1),
        Node::Disc {
            kind: crate::node::DiscKind::Discretionary,
            pre: foreign_child,
            post: PageListId::empty(),
            replace: PageListId::empty(),
            physical_replace_count: 0,
        },
    ]);

    assert_eq!(
        result,
        Err(crate::fork_arena::ForkArenaError::InvalidRegion)
    );
    assert_eq!(local.len(), 0);
    assert_eq!(local.counters().source_nodes_copied, 0);
    assert_eq!(local.semantic_hash_work(), before_hash_work);
}

#[test]
fn disabled_demand_keeps_range_and_composition_identity_work_at_zero() {
    let mut arena =
        PageMaterialArena::with_chunk_bytes(std::mem::size_of::<Option<PageMaterialNode>>() * 8);
    let whole = arena
        .publish_owned(penalties(&(0..128).collect::<Vec<_>>()))
        .expect("publish");
    let middle = arena
        .slice_sequence(whole, 3..125, &mut Vec::new())
        .expect("middle slice");
    let composed = arena.compose_sequences(&[middle, middle]).expect("compose");
    let mut builder = PageMaterialActiveListBuilder::vacant();
    arena.open_active_list(&mut builder).expect("open builder");
    arena
        .append_range_to_active_list(&mut builder, composed, 5..239)
        .expect("append range");
    let _ = arena
        .finalize_active_list(&mut builder)
        .expect("finalize builder");

    assert_eq!(arena.semantic_hash_work(), 0);
    assert_eq!(arena.semantic_summary_work(), 0);
    assert_eq!(arena.counters().identity_nodes_hashed, 0);
    assert_eq!(arena.counters().identity_summaries_combined, 0);
    assert_eq!(arena.counters().source_nodes_copied, 0);
}

#[test]
fn active_list_preserves_disabled_demand_and_appends_ranges_without_copying() {
    let mut arena = PageMaterialArena::with_chunk_bytes(32);
    let source = arena.publish_owned(penalties(&[10, 20])).expect("source");
    let source_address = arena
        .list(source)
        .expect("live source")
        .get(0)
        .map(std::ptr::from_ref)
        .expect("source node");
    let mut builder = PageMaterialActiveListBuilder::vacant();

    arena.open_active_list(&mut builder).expect("open builder");
    arena
        .append_to_active_list(&mut builder, source)
        .expect("append source coordinates");
    arena
        .push_active_list(&mut builder, Node::Penalty(30))
        .expect("append new semantic node");
    let composed = arena
        .finalize_active_list(&mut builder)
        .expect("finalize builder");

    assert_eq!(resolved(&arena, composed), penalties(&[10, 20, 30]));
    assert_eq!(composed.semantic_identity(), None);
    assert_eq!(arena.semantic_hash_work(), 0);
    assert_eq!(arena.counters().new_semantic_nodes, 3);
    assert_eq!(arena.counters().source_nodes_copied, 0);
    assert_eq!(
        arena
            .list(composed)
            .expect("live composed list")
            .get(0)
            .map(std::ptr::from_ref)
            .expect("composed source node"),
        source_address
    );
}

#[test]
fn source_copy_counter_has_a_real_negative_control() {
    let mut arena = PageMaterialArena::with_chunk_bytes(32);
    let source = arena.publish_owned(penalties(&[4, 5])).expect("source");
    let copied = arena
        .publish_source_copy(source)
        .expect("explicit compatibility copy");

    assert_eq!(resolved(&arena, copied), penalties(&[4, 5]));
    assert_eq!(arena.counters().source_nodes_copied, 2);
    assert_eq!(arena.counters().new_semantic_nodes, 4);
}

#[test]
fn generated_line_edges_preserve_the_selected_source_subrange_addresses() {
    let mut arena = PageMaterialArena::with_chunk_bytes(32);
    let source = arena
        .publish_owned(penalties(&[10, 20, 30]))
        .expect("source");
    let selected_addresses = [1, 2].map(|index| {
        arena
            .list(source)
            .expect("live source")
            .get(index)
            .map(std::ptr::from_ref)
            .expect("selected source node")
    });
    let mut line = PageMaterialActiveListBuilder::vacant();
    arena.open_active_list(&mut line).expect("open line");
    arena
        .push_active_list(&mut line, Node::Penalty(1))
        .expect("generated left edge");
    arena
        .append_range_to_active_list(&mut line, source, 1..3)
        .expect("borrowed line interior");
    arena
        .push_active_list(&mut line, Node::Penalty(2))
        .expect("generated right edge");
    let line = arena
        .finalize_active_list(&mut line)
        .expect("finalize line");

    assert_eq!(resolved(&arena, line), penalties(&[1, 20, 30, 2]));
    let line_view = arena.list(line).expect("live line");
    assert_eq!(
        [1, 2].map(|index| {
            line_view
                .get(index)
                .map(std::ptr::from_ref)
                .expect("borrowed line node")
        }),
        selected_addresses
    );
    assert_eq!(arena.counters().source_nodes_copied, 0);
    assert_eq!(arena.counters().new_semantic_nodes, 5);
}

#[test]
fn active_list_concatenates_maintained_semantic_identity() {
    let mut arena = PageMaterialArena::with_chunk_bytes(32);
    arena.enable_semantic_identity();
    let source_nodes = penalties(&[1, 2]);
    let source = arena
        .publish_owned(source_nodes.clone())
        .expect("source list");
    let mut builder = PageMaterialActiveListBuilder::vacant();

    arena.open_active_list(&mut builder).expect("open builder");
    arena
        .append_to_active_list(&mut builder, source)
        .expect("append source identity");
    arena
        .push_active_list(&mut builder, Node::Penalty(3))
        .expect("append new node identity");
    let result = arena
        .finalize_active_list(&mut builder)
        .expect("finalize builder");
    let expected_nodes = penalties(&[1, 2, 3]);

    assert_eq!(
        result.semantic_identity(),
        Some(identity(&expected_nodes).raw())
    );
    assert_eq!(arena.semantic_hash_work(), 3);
    assert_eq!(arena.counters().source_nodes_copied, 0);
}

#[test]
fn identity_is_preserved_across_build_split_and_compose() {
    let mut arena = PageMaterialArena::with_chunk_bytes(32);
    arena.enable_semantic_identity();
    let nodes = penalties(&[1, 2, 3, 4]);
    let whole = arena
        .publish_owned(nodes.clone())
        .expect("publish semantic list");
    let left_nodes = &nodes[..2];
    let right_nodes = &nodes[2..];
    let left = arena
        .slice_sequence(whole, 0..2, &mut Vec::new())
        .expect("split left");
    let right = arena
        .slice_sequence(whole, 2..4, &mut Vec::new())
        .expect("split right");
    let recomposed = arena.compose_sequences(&[left, right]).expect("compose");

    assert_eq!(whole.semantic_identity(), Some(identity(&nodes).raw()));
    assert_eq!(left.semantic_identity(), Some(identity(left_nodes).raw()));
    assert_eq!(right.semantic_identity(), Some(identity(right_nodes).raw()));
    assert_eq!(recomposed.semantic_identity(), whole.semantic_identity());
    assert_eq!(resolved(&arena, recomposed), nodes);
    assert_eq!(arena.semantic_hash_work(), 4);
    assert_eq!(arena.counters().source_nodes_copied, 0);
}

#[test]
fn long_middle_subrange_hashes_only_two_bounded_chunk_edges() {
    const CHUNK_VALUES: usize = 8;
    let mut arena = PageMaterialArena::with_chunk_bytes(
        std::mem::size_of::<Option<PageMaterialNode>>() * CHUNK_VALUES,
    );
    arena.enable_semantic_identity();
    let nodes = penalties(&(0..1024).collect::<Vec<_>>());
    let whole = arena
        .publish_owned(nodes.clone())
        .expect("publish long list");
    let hash_before = arena.semantic_hash_work();
    let summaries_before = arena.semantic_summary_work();

    let middle = arena
        .slice_sequence(whole, 3..1021, &mut Vec::new())
        .expect("slice long middle");

    let hashed = arena.semantic_hash_work() - hash_before;
    let summaries = arena.semantic_summary_work() - summaries_before;
    assert_eq!(
        middle.semantic_identity(),
        Some(identity(&nodes[3..1021]).raw())
    );
    assert!(
        hashed <= (2 * CHUNK_VALUES) as u64,
        "only two partial boundary chunks may hash payload: {hashed}"
    );
    assert!(hashed < middle.len() as u64);
    assert!(summaries > 100, "long interior must use chunk summaries");

    let append_hash_before = arena.semantic_hash_work();
    let append_summaries_before = arena.semantic_summary_work();
    let mut builder = PageMaterialActiveListBuilder::vacant();
    arena.open_active_list(&mut builder).expect("open builder");
    arena
        .append_range_to_active_list(&mut builder, whole, 3..1021)
        .expect("append long middle");
    let appended = arena
        .finalize_active_list(&mut builder)
        .expect("finalize retained range");
    assert_eq!(appended.semantic_identity(), middle.semantic_identity());
    assert!(
        arena.semantic_hash_work() - append_hash_before <= (2 * CHUNK_VALUES) as u64,
        "active range append may hash only its boundary chunks"
    );
    assert!(arena.semantic_summary_work() > append_summaries_before);

    let compose_hash_before = arena.semantic_hash_work();
    let doubled = arena
        .compose_sequences(&[middle, appended])
        .expect("compose summarized lists");
    let mut expected = nodes[3..1021].to_vec();
    expected.extend_from_slice(&nodes[3..1021]);
    assert_eq!(doubled.semantic_identity(), Some(identity(&expected).raw()));
    assert_eq!(arena.semantic_hash_work(), compose_hash_before);
    assert_eq!(arena.counters().source_nodes_copied, 0);
}

#[test]
fn multi_range_slice_identity_is_independent_of_descriptor_boundaries() {
    const CHUNK_VALUES: usize = 8;
    let mut arena = PageMaterialArena::with_chunk_bytes(
        std::mem::size_of::<Option<PageMaterialNode>>() * CHUNK_VALUES,
    );
    arena.enable_semantic_identity();
    let nodes = penalties(&(0..1024).collect::<Vec<_>>());
    let whole = arena.publish_owned(nodes.clone()).expect("publish");
    let left = arena
        .slice_sequence(whole, 0..300, &mut Vec::new())
        .expect("left");
    let right = arena
        .slice_sequence(whole, 700..1024, &mut Vec::new())
        .expect("right");
    let composite = arena.compose_sequences(&[left, right]).expect("composite");
    let hash_before = arena.semantic_hash_work();
    let selected = arena
        .slice_sequence(composite, 2..622, &mut Vec::new())
        .expect("cross-range selection");
    let mut expected = nodes[2..300].to_vec();
    expected.extend_from_slice(&nodes[700..1022]);

    assert_eq!(resolved(&arena, selected), expected);
    assert_eq!(
        selected.semantic_identity(),
        Some(identity(&expected).raw())
    );
    assert!(
        arena.semantic_hash_work() - hash_before <= (2 * CHUNK_VALUES) as u64,
        "only the two physical boundary chunks may be rehashed"
    );
    assert_eq!(arena.counters().source_nodes_copied, 0);
}

#[test]
fn partial_operation_restore_restores_payload_chunk_summary() {
    let mut arena =
        PageMaterialArena::with_chunk_bytes(std::mem::size_of::<Option<PageMaterialNode>>() * 8);
    arena.enable_semantic_identity();
    let retained_nodes = penalties(&[1, 2, 3]);
    let retained = arena
        .publish_owned(retained_nodes.clone())
        .expect("retained prefix");
    let operation = arena.operation_mark();
    let rejected = arena
        .publish_owned(penalties(&[8, 9]))
        .expect("operation suffix");
    assert!(arena.contains(rejected));

    arena
        .restore_operation(operation)
        .expect("restore partial payload tail");
    assert!(!arena.contains(rejected));
    let restored = arena
        .slice_sequence(retained, 1..3, &mut Vec::new())
        .expect("slice restored prefix");
    assert_eq!(
        restored.semantic_identity(),
        Some(identity(&retained_nodes[1..]).raw())
    );
    assert_eq!(arena.counters().source_nodes_copied, 0);
}

#[test]
fn accepted_identity_survives_reject_accept_and_prune() {
    let mut arena = PageMaterialArena::with_chunk_bytes(32);
    arena.enable_semantic_identity();
    let prefix = arena
        .publish_owned(penalties(&[7, 8]))
        .expect("accepted prefix");
    let checkpoint = arena
        .seal_boundary()
        .and_then(|boundary| arena.checkpoint_mark(boundary))
        .expect("checkpoint");
    let prior_nodes = penalties(&[30, 31]);
    let prior = arena
        .publish_owned(prior_nodes.clone())
        .expect("prior accepted suffix");
    let prior_identity = prior.semantic_identity();

    arena
        .begin_checkpoint_candidate(checkpoint)
        .expect("begin rejected candidate");
    let rejected = arena
        .publish_owned(penalties(&[90]))
        .expect("rejected list");
    assert_ne!(rejected.semantic_identity(), prior_identity);
    let boundary = arena.seal_boundary().expect("seal rejected candidate");
    arena
        .reject_checkpoint_candidate(boundary)
        .expect("reject candidate");
    assert_eq!(prior.semantic_identity(), prior_identity);
    assert_eq!(resolved(&arena, prior), prior_nodes);
    assert_eq!(resolved(&arena, prefix), penalties(&[7, 8]));

    arena
        .begin_checkpoint_candidate(checkpoint)
        .expect("begin accepted candidate");
    let replacement_nodes = penalties(&[11, 12, 13]);
    let replacement = arena
        .publish_owned(replacement_nodes.clone())
        .expect("replacement list");
    let replacement_identity = replacement.semantic_identity();
    let boundary = arena.seal_boundary().expect("seal accepted candidate");
    arena
        .accept_checkpoint_candidate(boundary)
        .expect("accept candidate");
    assert_eq!(replacement.semantic_identity(), replacement_identity);
    assert_eq!(resolved(&arena, replacement), replacement_nodes);
    assert_eq!(arena.counters().source_nodes_copied, 0);
    assert!(arena.counters().obsolete_chunks_pruned > 0);
}

#[test]
fn independent_root_overwrite_recycles_a_prefix_batch_and_rejects_stale_id() {
    let mut arena = PageMaterialArena::with_chunk_bytes(32);
    let stale = arena.publish_owned(penalties(&[1, 2])).expect("stale");
    let stale_root = arena.acquire_root(stale).expect("root stale batch");
    let survivor = arena.publish_owned(penalties(&[8, 9])).expect("survivor");
    let survivor_root = arena.acquire_root(survivor).expect("root survivor");

    arena.release_root(stale_root).expect("overwrite root");
    assert!(!arena.contains(stale));
    assert_eq!(resolved(&arena, survivor), penalties(&[8, 9]));

    let replacement = arena.publish_owned(penalties(&[3, 4])).expect("reuse");
    assert!(
        !arena.contains(stale),
        "recycled chunk generation rejects ABA"
    );
    assert_eq!(resolved(&arena, replacement), penalties(&[3, 4]));
    arena.release_root(survivor_root).expect("drop survivor");
}

#[test]
fn explicit_box_copy_keeps_one_coarse_batch_until_both_roots_drop() {
    let mut arena = PageMaterialArena::with_chunk_bytes(32);
    let list = arena.publish_owned(penalties(&[10])).expect("box");
    let first = arena.acquire_root(list).expect("box register root");
    let second = arena.copy_root(&first).expect("TeX copy root");

    arena.release_root(first).expect("drop first box");
    assert!(arena.contains(list));
    arena.release_root(second).expect("drop copied box");
    assert!(!arena.contains(list));
}

#[test]
fn enclosing_immutable_closure_keeps_nested_coordinates_live() {
    let mut arena = PageMaterialArena::with_chunk_bytes(32);
    let child = arena.publish_owned(penalties(&[17])).expect("child");
    let parent = arena
        .publish_owned([Node::Disc {
            kind: crate::node::DiscKind::Discretionary,
            pre: child,
            post: PageListId::empty(),
            replace: PageListId::empty(),
            physical_replace_count: 0,
        }])
        .expect("parent");
    let parent_root = arena.acquire_root(parent).expect("parent root");

    assert!(arena.contains(child));
    arena.release_root(parent_root).expect("drop closure");
    assert!(!arena.contains(parent));
    assert!(!arena.contains(child));
}

#[test]
fn retained_checkpoint_interval_outlives_current_root() {
    let mut arena = PageMaterialArena::with_chunk_bytes(32);
    let list = arena.publish_owned(penalties(&[21])).expect("list");
    let root = arena.acquire_root(list).expect("current root");
    let mark = arena
        .seal_boundary()
        .and_then(|boundary| arena.checkpoint_mark(boundary))
        .expect("checkpoint");
    let lease = arena
        .retain_checkpoint_interval(mark)
        .expect("retained interval");

    arena.release_root(root).expect("drop current root");
    assert!(arena.contains(list));
    arena
        .release_checkpoint_interval(lease)
        .expect("drop retained interval");
    assert!(!arena.contains(list));
}

#[test]
fn repeated_completed_pages_reuse_coarse_storage_without_scans_or_copies() {
    let mut arena = PageMaterialArena::with_chunk_bytes(32);
    let mut warmed_pages = 0;
    for page in 0..256 {
        let list = arena
            .publish_owned(penalties(&[page, page + 1]))
            .expect("page batch");
        let root = arena.acquire_root(list).expect("output handoff");
        arena.release_root(root).expect("shipout release");
        if page == 31 {
            warmed_pages = arena.pool.chunks.page_count();
        }
    }
    assert_eq!(arena.pool.chunks.page_count(), warmed_pages);
    assert_eq!(arena.counters().source_nodes_copied, 0);
}
