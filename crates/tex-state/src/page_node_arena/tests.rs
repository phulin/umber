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
    assert_eq!(list.semantic_identity(), None);
    assert_eq!(arena.counters().source_nodes_copied, 0);
    assert_eq!(arena.counters().new_semantic_nodes, 3);
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
        .slice_with_identity(whole, 0..2, Some(identity(left_nodes)))
        .expect("split left");
    let right = arena
        .slice_with_identity(whole, 2..4, Some(identity(right_nodes)))
        .expect("split right");
    let recomposed = arena
        .compose_with_identity(&[left, right], Some(identity(&nodes)))
        .expect("compose");

    assert_eq!(whole.semantic_identity(), Some(identity(&nodes).raw()));
    assert_eq!(left.semantic_identity(), Some(identity(left_nodes).raw()));
    assert_eq!(right.semantic_identity(), Some(identity(right_nodes).raw()));
    assert_eq!(recomposed.semantic_identity(), whole.semantic_identity());
    assert_eq!(resolved(&arena, recomposed), nodes);
    assert_eq!(arena.semantic_hash_work(), 4);
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
