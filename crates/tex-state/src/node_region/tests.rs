use super::*;
use crate::fork_arena::{ArenaListId, ForkArena, PageMaterialLane};
use crate::glue::Order;
use crate::node::{BoxLr, BoxNode, BoxNodeFields, Sign};
use crate::scaled::{GlueSetRatio, Scaled};

fn boxed(children: PageListId) -> RegionNode {
    Node::HList(BoxNode::new(BoxNodeFields {
        width: Scaled::from_raw(10),
        height: Scaled::from_raw(20),
        depth: Scaled::from_raw(3),
        shift: Scaled::from_raw(0),
        box_lr: BoxLr::Normal,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children,
    }))
}

fn publish_raw(
    arena: &mut ForkArena<RegionNode, PageMaterialLane>,
    pool: &mut ChunkPool<RegionNode>,
    nodes: impl IntoIterator<Item = RegionNode>,
) -> ArenaListId<PageMaterialLane> {
    let mut builder = arena.begin_builder(pool).expect("open list builder");
    for node in nodes {
        builder.push(node).expect("publish test node");
    }
    builder.seal().expect("seal test list")
}

#[test]
fn region_ids_reject_reused_slots_and_owner_relative_roots() {
    let mut pool = NodePool::with_chunk_bytes(64);
    let mut first = pool.start_region::<PageRole>().expect("first region");
    let root = first
        .publish_owned(&mut pool, [Node::Penalty(7)])
        .expect("first root");
    let stale_id = first.id();

    assert!(pool.retire_region(first).is_ok(), "retire first region");
    assert!(!pool.validates_id(stale_id));

    let replacement = pool.start_region::<PageRole>().expect("replacement region");
    assert_ne!(replacement.id(), stale_id);
    assert!(pool.validates_id(replacement.id()));
    assert!(matches!(
        replacement.list(&pool, root),
        Err(ForkArenaError::InvalidRegion)
    ));
}

#[test]
fn borrowed_region_list_is_admitted_by_the_matching_owner() {
    let mut pool = NodePool::with_chunk_bytes(64);
    let mut region = pool.start_region::<PageRole>().expect("region");
    let root = region
        .publish_owned(&mut pool, [Node::Penalty(3), Node::Penalty(5)])
        .expect("root");

    let borrowed = region.list(&pool, root).expect("matching region borrow");
    assert_eq!(borrowed.len(), 2);
    assert_eq!(
        borrowed.iter().cloned().collect::<Vec<_>>(),
        [Node::Penalty(3), Node::Penalty(5),]
    );
}

#[test]
fn whole_closure_transfer_rebrands_nested_children_without_copying() {
    let mut pool = NodePool::with_chunk_bytes(64);
    let mut source = pool.start_region::<DurableRole>().expect("source");
    let child = source
        .publish_owned(&mut pool, [Node::Penalty(11)])
        .expect("child");
    let child_address = source
        .list(&pool, child)
        .expect("source child")
        .get(0)
        .map(std::ptr::from_ref)
        .expect("child address");
    let parent = source
        .publish_owned(&mut pool, [boxed(child.list)])
        .expect("parent");
    let closure = source
        .into_closure(&pool, parent)
        .map_err(|(error, _)| error)
        .expect("owned closure");
    let mut destination = pool.start_region::<PageRole>().expect("destination");

    let moved = transfer_closure_into(&mut pool, closure, &mut destination)
        .map_err(|failure| failure.error)
        .expect("whole closure transfer");
    let moved_parent = destination.list(&pool, moved).expect("moved parent");
    let Node::HList(box_node) = moved_parent.get(0).expect("parent node") else {
        panic!("moved parent lost box shape");
    };
    let moved_child = destination
        .pub_arena
        .list(&pool.chunks, box_node.children.coordinate())
        .expect("rebranded nested child");

    assert_eq!(
        moved_child.iter().cloned().collect::<Vec<_>>(),
        [Node::Penalty(11),]
    );
    assert_eq!(
        moved_child.get(0).map(std::ptr::from_ref),
        Some(child_address),
        "whole-envelope movement preserves payload addresses"
    );
    assert_eq!(destination.counters().source_nodes_copied, 0);
}

#[test]
fn whole_closure_preflight_rejects_foreign_nested_child_atomically() {
    let mut pool = NodePool::with_chunk_bytes(64);
    let mut foreign = pool.start_region::<PageRole>().expect("foreign");
    let foreign_child = foreign
        .publish_owned(&mut pool, [Node::Penalty(17)])
        .expect("foreign child");
    let mut source = pool.start_region::<DurableRole>().expect("source");
    let parent = source
        .publish_owned(&mut pool, [boxed(foreign_child.list)])
        .expect("foreign-bearing parent");
    let closure = source
        .into_closure(&pool, parent)
        .map_err(|(error, _)| error)
        .expect("owned closure");
    let mut destination = pool.start_region::<PageRole>().expect("destination");
    let source_before = closure.region.counters();
    let destination_before = destination.counters();

    let failure = transfer_closure_into(&mut pool, closure, &mut destination)
        .expect_err("foreign nested child must reject");
    assert_eq!(failure.error, ForkArenaError::InvalidRegion);
    assert_eq!(failure.closure.region.counters(), source_before);
    assert_eq!(destination.counters(), destination_before);
    assert_eq!(
        failure
            .closure
            .list(&pool)
            .expect("source root remains admitted")
            .len(),
        1
    );
    assert_eq!(
        foreign
            .list(&pool, foreign_child)
            .expect("foreign child remains admitted")
            .iter()
            .cloned()
            .collect::<Vec<_>>(),
        [Node::Penalty(17)]
    );
}

#[test]
fn suffix_transfer_preflights_and_rebrands_the_whole_nested_closure() {
    let mut chunks = ChunkPool::<RegionNode>::with_chunk_bytes(64);
    let mut source = ForkArena::<RegionNode, PageMaterialLane>::new();
    let mut destination = ForkArena::<RegionNode, PageMaterialLane>::new();
    let mark = source.begin_batch(&mut chunks).expect("suffix start");
    let child = publish_raw(&mut source, &mut chunks, [Node::Penalty(23)]);
    let parent = publish_raw(
        &mut source,
        &mut chunks,
        [boxed(PageListId::from_parts(child, None))],
    );
    let child_address = source
        .list(&chunks, child)
        .expect("source child")
        .get(0)
        .map(std::ptr::from_ref)
        .expect("source child address");
    let batch = source
        .seal_batch(&mut chunks, mark, vec![parent])
        .expect("sealed suffix");

    let [moved_parent]: [_; 1] = source
        .promote_batch_into(&mut chunks, &mut destination, batch)
        .expect("self-contained suffix transfer")
        .try_into()
        .expect("one root");
    let moved_parent = destination
        .list(&chunks, moved_parent)
        .expect("moved parent");
    let Node::HList(box_node) = moved_parent.get(0).expect("parent node") else {
        panic!("moved parent lost box shape");
    };
    let moved_child = destination
        .list(&chunks, box_node.children.coordinate())
        .expect("rebranded child");
    assert_eq!(
        moved_child.get(0).map(std::ptr::from_ref),
        Some(child_address)
    );
}

#[test]
fn suffix_transfer_failure_returns_the_unchanged_batch_authority() {
    let mut chunks = ChunkPool::<RegionNode>::with_chunk_bytes(64);
    let mut source = ForkArena::<RegionNode, PageMaterialLane>::new();
    let mut destination = ForkArena::<RegionNode, PageMaterialLane>::new();
    let prefix_child = publish_raw(&mut source, &mut chunks, [Node::Penalty(29)]);
    let mark = source.begin_batch(&mut chunks).expect("suffix start");
    let parent = publish_raw(
        &mut source,
        &mut chunks,
        [boxed(PageListId::from_parts(prefix_child, None))],
    );
    let batch = source
        .seal_batch(&mut chunks, mark, vec![parent])
        .expect("sealed suffix");
    let source_before = source.counters();
    let destination_before = destination.counters();

    let failure = source
        .promote_batch_into(&mut chunks, &mut destination, batch)
        .expect_err("suffix may not retain a prefix coordinate");
    assert_eq!(failure.error, ForkArenaError::InvalidRegion);
    assert_eq!(source.counters(), source_before);
    assert_eq!(destination.counters(), destination_before);
    assert_eq!(
        source
            .list(&chunks, parent)
            .expect("source parent remains live")
            .len(),
        1
    );
    source
        .cancel_batch(failure.batch)
        .expect("returned batch authority cancels cleanly");
}

#[test]
fn explicit_copy_deep_copies_recursive_nodes_and_preserves_source() {
    let mut pool = NodePool::with_chunk_bytes(64);
    let mut source = pool.start_region::<DurableRole>().expect("source");
    let leaf = source
        .publish_owned(&mut pool, [Node::Penalty(31)])
        .expect("leaf");
    let child = source
        .publish_owned(&mut pool, [boxed(leaf.list)])
        .expect("child");
    let root = source
        .publish_owned(
            &mut pool,
            [Node::Disc {
                kind: crate::node::DiscKind::Discretionary,
                pre: child.list,
                post: PageListId::empty(),
                replace: PageListId::empty(),
                physical_replace_count: 0,
            }],
        )
        .expect("root");
    let source_leaf_address = source
        .list(&pool, leaf)
        .expect("source leaf")
        .get(0)
        .map(std::ptr::from_ref)
        .expect("source leaf address");
    let closure = source
        .into_closure(&pool, root)
        .map_err(|(error, _)| error)
        .expect("source closure");
    let mut destination = pool.start_region::<PageRole>().expect("destination");

    let copied =
        copy_closure_into(&mut pool, &closure, &mut destination, false).expect("recursive copy");
    let copied_root = destination.list(&pool, copied).expect("copied root");
    let Node::Disc { pre, .. } = copied_root.get(0).expect("disc node") else {
        panic!("copied root lost discretionary shape");
    };
    let copied_child = destination
        .pub_arena
        .list(&pool.chunks, pre.coordinate())
        .expect("copied child");
    let Node::HList(box_node) = copied_child.get(0).expect("box node") else {
        panic!("copied child lost box shape");
    };
    let copied_leaf = destination
        .pub_arena
        .list(&pool.chunks, box_node.children.coordinate())
        .expect("copied leaf");

    assert_ne!(
        copied_leaf.get(0).map(std::ptr::from_ref),
        Some(source_leaf_address)
    );
    assert_eq!(
        copied_leaf.iter().cloned().collect::<Vec<_>>(),
        [Node::Penalty(31),]
    );
    assert_eq!(
        closure
            .list(&pool)
            .expect("source closure remains live")
            .len(),
        1
    );
    assert_eq!(destination.counters().source_nodes_copied, 3);
}

#[test]
fn closure_build_transfer_is_zero_copy_and_address_stable() {
    let mut pool = NodePool::with_chunk_bytes(64);
    let mut source = pool.start_region::<PageRole>().expect("source");
    let mark = source
        .begin_closure_build(&mut pool)
        .expect("closure boundary");
    let root = source
        .publish_owned(&mut pool, [Node::Penalty(41), Node::Penalty(43)])
        .expect("closure root");
    let address = source
        .list(&pool, root)
        .expect("source list")
        .get(0)
        .map(std::ptr::from_ref)
        .expect("source address");
    let receipt = source
        .consumed_closure_roots_receipt(&mark)
        .expect("owner-local roots consumed");
    let closure = source
        .seal_closure(&mut pool, mark, root, receipt)
        .expect("self-contained closure");
    assert!(
        source.list(&pool, root).is_err(),
        "detached suffix is unavailable through the source owner"
    );
    let mut destination = pool.start_region::<DurableRole>().expect("destination");
    let moved = transfer_sealed_closure_into(&mut pool, &mut source, closure, &mut destination)
        .map_err(|failure| failure.error)
        .expect("transfer closure");

    assert_eq!(
        destination
            .list(&pool, moved)
            .expect("moved list")
            .get(0)
            .map(std::ptr::from_ref),
        Some(address)
    );
    assert_eq!(destination.counters().source_nodes_copied, 0);
    assert_eq!(pool.closure_transition_counters().envelope_moves, 1);
    assert_eq!(pool.closure_transition_counters().rebrand_scan_nodes, 2);
}

#[test]
fn closure_build_transfer_rebrands_nested_suffix_children() {
    let mut pool = NodePool::with_chunk_bytes(64);
    let mut source = pool.start_region::<PageRole>().expect("source");
    let mark = source
        .begin_closure_build(&mut pool)
        .expect("closure boundary");
    let child = source
        .publish_owned(&mut pool, [Node::Penalty(47)])
        .expect("child");
    let child_address = source
        .list(&pool, child)
        .expect("source child")
        .get(0)
        .map(std::ptr::from_ref)
        .expect("child address");
    let root = source
        .publish_owned(&mut pool, [boxed(child.list)])
        .expect("parent");
    let receipt = source
        .consumed_closure_roots_receipt(&mark)
        .expect("owner-local roots consumed");
    let closure = source
        .seal_closure(&mut pool, mark, root, receipt)
        .expect("nested suffix");
    let mut destination = pool.start_region::<DurableRole>().expect("destination");
    let moved = transfer_sealed_closure_into(&mut pool, &mut source, closure, &mut destination)
        .map_err(|failure| failure.error)
        .expect("nested transfer");
    let parent = destination.list(&pool, moved).expect("moved parent");
    let Node::HList(box_node) = parent.get(0).expect("parent node") else {
        panic!("moved parent lost box shape");
    };
    let moved_child = destination
        .pub_arena
        .list(&pool.chunks, box_node.children.coordinate())
        .expect("moved child");
    assert_eq!(
        moved_child.get(0).map(std::ptr::from_ref),
        Some(child_address)
    );
}

#[test]
fn checkpoint_before_closure_build_does_not_force_a_copy() {
    let mut pool = NodePool::with_chunk_bytes(64);
    let mut source = pool.start_region::<PageRole>().expect("source");
    let prefix = source
        .publish_owned(&mut pool, [Node::Penalty(53)])
        .expect("checkpoint prefix");
    let boundary = source
        .pub_arena
        .seal_boundary(&mut pool.chunks)
        .expect("checkpoint boundary");
    let checkpoint = source
        .pub_arena
        .checkpoint_mark(boundary)
        .expect("checkpoint mark");
    let mark = source
        .begin_closure_build(&mut pool)
        .expect("closure boundary");
    let root = source
        .publish_owned(&mut pool, [Node::Penalty(59)])
        .expect("closure root");
    let receipt = source
        .consumed_closure_roots_receipt(&mark)
        .expect("owner-local roots consumed");
    let closure = source
        .seal_closure(&mut pool, mark, root, receipt)
        .expect("closure after checkpoint");
    let mut destination = pool.start_region::<DurableRole>().expect("destination");
    transfer_sealed_closure_into(&mut pool, &mut source, closure, &mut destination)
        .map_err(|failure| failure.error)
        .expect("transfer after checkpoint");

    assert!(source.pub_arena.validates_checkpoint(checkpoint));
    assert_eq!(
        source
            .list(&pool, prefix)
            .expect("checkpoint prefix remains live")
            .get(0),
        Some(&Node::Penalty(53))
    );
    assert_eq!(pool.closure_transition_counters().structural_fallbacks, 0);
}

#[test]
fn transient_closure_loan_rolls_back_without_copying() {
    let mut pool = NodePool::with_chunk_bytes(64);
    let mut source = pool.start_region::<PageRole>().expect("source");
    let mark = source
        .begin_closure_build(&mut pool)
        .expect("closure boundary");
    let root = source
        .publish_owned(&mut pool, [Node::Penalty(61)])
        .expect("closure root");
    let address = source
        .list(&pool, root)
        .expect("source list")
        .get(0)
        .map(std::ptr::from_ref)
        .expect("source address");
    let receipt = source
        .consumed_closure_roots_receipt(&mark)
        .expect("owner-local roots consumed");
    let closure = source
        .seal_closure(&mut pool, mark, root, receipt)
        .expect("closure loan");
    source
        .rollback_closure(&mut pool, closure)
        .map_err(|failure| failure.error)
        .expect("rollback loan");

    assert_eq!(
        source
            .list(&pool, root)
            .expect("reattached source")
            .get(0)
            .map(std::ptr::from_ref),
        Some(address)
    );
    assert_eq!(pool.closure_transition_counters().transient_rollbacks, 1);
}

#[test]
fn prefix_child_rejects_without_mutation_and_fallback_is_counted() {
    let mut pool = NodePool::with_chunk_bytes(64);
    let mut source = pool.start_region::<PageRole>().expect("source");
    let child = source
        .publish_owned(&mut pool, [Node::Penalty(67)])
        .expect("prefix child");
    let mark = source
        .begin_closure_build(&mut pool)
        .expect("closure boundary");
    let root = source
        .publish_owned(&mut pool, [boxed(child.list)])
        .expect("interleaved parent");
    let before = source.counters();
    let receipt = source
        .consumed_closure_roots_receipt(&mark)
        .expect("owner-local roots consumed");
    let failure = match source.seal_closure(&mut pool, mark, root, receipt) {
        Ok(_) => panic!("prefix child is outside the suffix"),
        Err(failure) => failure,
    };
    assert_eq!(failure.error, ForkArenaError::InvalidRegion);
    source
        .compatibility_closure_build_receipt(failure.mark)
        .expect("failed seal returns the original build authority");
    assert_eq!(source.counters(), before);
    assert!(source.list(&pool, child).is_ok());
    assert!(source.list(&pool, root).is_ok());

    let mut destination = pool.start_region::<DurableRole>().expect("destination");
    let copied = structural_copy_fallback(
        &mut pool,
        &source,
        root,
        &mut destination,
        StructuralCopyReason::InterleavedPrefixChild,
    )
    .expect("bounded fallback copy");
    assert_eq!(destination.list(&pool, copied).expect("copy").len(), 1);
    assert_eq!(pool.closure_transition_counters().structural_fallbacks, 1);
    assert_eq!(
        pool.closure_transition_counters()
            .interleaved_prefix_fallbacks,
        1
    );
}

#[test]
fn foreign_root_receipt_rejects_without_detaching_suffix() {
    let mut pool = NodePool::with_chunk_bytes(64);
    let mut source = pool.start_region::<PageRole>().expect("source");
    let mark = source
        .begin_closure_build(&mut pool)
        .expect("closure boundary");
    let local = source
        .publish_owned(&mut pool, [Node::Penalty(71)])
        .expect("local suffix");
    let foreign = pool.start_region::<PageRole>().expect("foreign");
    let foreign_root = RegionRoot {
        region: foreign.id(),
        list: local.list,
        _role: PhantomData,
    };
    let before = source.counters();
    let receipt = source
        .consumed_closure_roots_receipt(&mark)
        .expect("owner-local roots consumed");
    let failure = match source.seal_closure(&mut pool, mark, foreign_root, receipt) {
        Ok(_) => panic!("foreign root cannot name the suffix"),
        Err(failure) => failure,
    };
    assert_eq!(failure.error, ForkArenaError::InvalidRegion);
    source
        .compatibility_closure_build_receipt(failure.mark)
        .expect("foreign rejection returns the original build authority");
    assert_eq!(source.counters(), before);
    assert!(source.list(&pool, local).is_ok());
}

#[test]
fn shared_pool_retires_transferred_source_and_rejects_stale_id() {
    let mut pool = NodePool::with_chunk_bytes(64);
    let mut source = pool.start_region::<PageRole>().expect("source");
    let stale = source.id();
    let mark = source
        .begin_closure_build(&mut pool)
        .expect("closure boundary");
    let root = source
        .publish_owned(&mut pool, [Node::Penalty(73)])
        .expect("closure root");
    let receipt = source
        .consumed_closure_roots_receipt(&mark)
        .expect("owner-local roots consumed");
    let closure = source
        .seal_closure(&mut pool, mark, root, receipt)
        .expect("sealed closure");
    let mut destination = pool.start_region::<DurableRole>().expect("destination");
    transfer_sealed_closure_into(&mut pool, &mut source, closure, &mut destination)
        .map_err(|failure| failure.error)
        .expect("transfer");
    assert!(pool.retire_region(source).is_ok(), "empty source retires");
    assert!(!pool.validates_id(stale));
    let replacement = pool.start_region::<PageRole>().expect("replacement");
    assert_ne!(replacement.id(), stale);
}
