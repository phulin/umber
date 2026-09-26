use super::*;
use crate::fork_arena::{ArenaListId, ForkArena, PageMaterialLane};
use crate::glue::Order;
use crate::node::{BoxLr, BoxNode, BoxNodeFields, Sign};
use crate::scaled::{GlueSetRatio, Scaled};

fn boxed(children: PageListId) -> Node<PageListId> {
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
    arena: &mut ForkArena<Node<PageListId>, PageMaterialLane>,
    pool: &mut ChunkPool<Node<PageListId>>,
    nodes: impl IntoIterator<Item = Node<PageListId>>,
) -> ArenaListId<PageMaterialLane> {
    let mut builder = arena.begin_builder(pool).expect("open list builder");
    for node in nodes {
        builder.push(node).expect("publish test node");
    }
    builder.finish()
}

fn resident_address<Role>(
    region: &NodeRegion<Role>,
    pool: &NodePool,
    list: PageListId,
) -> *const RegionNode {
    region
        .pub_arena
        .list(&pool.chunks, list.coordinate())
        .expect("resident list")
        .get(0)
        .map(std::ptr::from_ref)
        .expect("resident node")
}

fn resident_nodes<Role>(
    region: &NodeRegion<Role>,
    pool: &NodePool,
    list: PageListId,
) -> Vec<Node<PageListId>> {
    region
        .pub_arena
        .list(&pool.chunks, list.coordinate())
        .expect("resident list")
        .iter()
        .map(|record| {
            record
                .decode_owned(crate::node_record::NodeAnnexView::new(
                    &pool.annex_chunks,
                    &region.annex_arena,
                ))
                .expect("typed annex")
        })
        .collect()
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
    let child_address = resident_address(&source, &pool, child.list);
    let parent = source
        .publish_owned(&mut pool, [boxed(child.list)])
        .expect("parent");
    let mut annex_builder = source
        .annex_arena
        .begin_builder(&mut pool.annex_chunks)
        .expect("whole-region annex builder");
    annex_builder.push(137).expect("whole-region annex word");
    let annex = annex_builder.finish();
    let annex_address = source
        .annex_arena
        .list(&pool.annex_chunks, annex)
        .expect("source annex")
        .get(0)
        .map(std::ptr::from_ref)
        .expect("source annex address");
    let mut closure = source
        .into_closure(&pool, parent)
        .map_err(|(error, _)| error)
        .expect("owned closure");
    let mut destination = pool.start_region::<PageRole>().expect("destination");

    let moved = transfer_closure_into(&mut pool, &mut closure, &mut destination)
        .expect("whole closure transfer");
    let moved_parent = destination.list(&pool, moved).expect("moved parent");
    let crate::NodeView::HList(box_node) = moved_parent.get(0).expect("parent node") else {
        panic!("moved parent lost box shape");
    };
    let moved_child = destination
        .pub_arena
        .list(&pool.chunks, box_node.children.coordinate())
        .expect("rebranded nested child");

    assert_eq!(
        resident_nodes(&destination, &pool, box_node.children),
        [Node::Penalty(11)]
    );
    assert_eq!(
        moved_child.get(0).map(std::ptr::from_ref),
        Some(child_address),
        "whole-envelope movement preserves payload addresses"
    );
    assert_eq!(destination.counters().source_nodes_copied, 0);
    let moved_annex = destination
        .annex_arena
        .list(&pool.annex_chunks, annex)
        .expect("moved annex");
    assert_eq!(moved_annex.iter().copied().collect::<Vec<_>>(), [137]);
    assert_eq!(
        moved_annex.get(0).map(std::ptr::from_ref),
        Some(annex_address)
    );
}

#[test]
fn whole_closure_preflight_rejects_foreign_nested_child_atomically() {
    let mut pool = NodePool::with_chunk_bytes(64);
    let mut foreign = pool.start_region::<PageRole>().expect("foreign");
    let foreign_child = foreign
        .publish_owned(&mut pool, [Node::Penalty(17)])
        .expect("foreign child");
    let mut source = pool.start_region::<DurableRole>().expect("source");
    assert_eq!(
        source.publish_owned(&mut pool, [boxed(foreign_child.list)]),
        Err(ForkArenaError::InvalidRegion),
        "foreign child rejects at destination-directed publication"
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
fn void_closure_build_preserves_shared_successor_tail() {
    let mut pool = NodePool::with_chunk_bytes(64);
    let mut prior = pool.start_region::<PageRole>().expect("prior page region");
    let build = prior
        .begin_closure_build(&mut pool)
        .expect("retained prefix build");
    let retained = prior
        .publish_owned(&mut pool, [Node::Penalty(17)])
        .expect("retained page root");
    let address = resident_address(&prior, &pool, retained.list);
    let mut successor = prior
        .share_sealed_prefix(&mut pool, build, [retained.list])
        .expect("shared successor");

    let void_build = successor
        .begin_closure_build(&mut pool)
        .expect("void box construction");
    successor
        .cancel_closure_build(&mut pool, void_build)
        .expect("void construction leaves shared prefix untouched");

    assert_eq!(resident_address(&prior, &pool, retained.list), address);
    assert_eq!(resident_address(&successor, &pool, retained.list), address);
    assert_eq!(
        resident_nodes(&successor, &pool, retained.list),
        [Node::Penalty(17)]
    );
}

#[test]
fn typed_annex_bulk_publication_completes_dependency_metadata() {
    let mut pool = NodePool::with_chunk_bytes(64);
    let mut region = pool.start_region::<PageRole>().expect("page region");
    let build = region
        .begin_closure_build(&mut pool)
        .expect("annex publication boundary");
    {
        let mut writer = NodeAnnexWriter::new(&mut pool.annex_chunks, &mut region.annex_arena);
        writer.append_fixed::<()>(&[1, 2]);
        writer.append_span::<()>(&(0..80).collect::<Vec<_>>());
        writer.append_span_iter::<()>(80..160);
    }

    region
        .can_share_sealed_prefix(&pool, &build, [])
        .expect("every bulk-published annex block has complete child metadata");
}

#[test]
fn suffix_transfer_preflights_and_rebrands_the_whole_nested_closure() {
    let mut chunks = ChunkPool::<Node<PageListId>>::with_chunk_bytes(64);
    let mut source = ForkArena::<Node<PageListId>, PageMaterialLane>::new();
    let mut destination = ForkArena::<Node<PageListId>, PageMaterialLane>::new();
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
    let mut chunks = ChunkPool::<Node<PageListId>>::with_chunk_bytes(64);
    let mut source = ForkArena::<Node<PageListId>, PageMaterialLane>::new();
    let mut destination = ForkArena::<Node<PageListId>, PageMaterialLane>::new();
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
    let source_leaf_address = resident_address(&source, &pool, leaf.list);
    let closure = source
        .into_closure(&pool, root)
        .map_err(|(error, _)| error)
        .expect("source closure");
    let mut destination = pool.start_region::<PageRole>().expect("destination");
    let before = destination.counters();

    let copied =
        copy_closure_into(&mut pool, &closure, &mut destination, true).expect("recursive copy");
    let after = destination.counters();
    let copied_root = destination.list(&pool, copied).expect("copied root");
    let crate::NodeView::Disc { pre, .. } = copied_root.get(0).expect("disc node") else {
        panic!("copied root lost discretionary shape");
    };
    let copied_child = resident_nodes(&destination, &pool, pre);
    let Node::HList(box_node) = &copied_child[0] else {
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
        resident_nodes(&destination, &pool, box_node.children),
        [Node::Penalty(31)]
    );
    assert_eq!(
        closure
            .list(&pool)
            .expect("source closure remains live")
            .len(),
        1
    );
    assert!(pre.semantic_identity().is_some());
    assert!(box_node.children.semantic_identity().is_some());
    assert_eq!(after.source_nodes_copied - before.source_nodes_copied, 3);
    assert_eq!(after.whole_payload_copies - before.whole_payload_copies, 0);
    assert_eq!(
        after.resident_payload_clones - before.resident_payload_clones,
        0
    );
    assert_eq!(after.whole_payload_moves - before.whole_payload_moves, 0);
}

#[test]
fn mapped_region_node_copy_is_one_resident_clone_at_required_sizes() {
    assert_eq!(core::mem::size_of::<RegionNode>(), 32);

    for size in [1_usize, 64, 4_096] {
        let mut pool = NodePool::with_chunk_bytes(512);
        let mut source = pool.start_region::<DurableRole>().expect("source");
        let root = source
            .publish_owned(
                &mut pool,
                (0..size).map(|value| Node::Penalty(value as i32)),
            )
            .expect("source root");
        let source_address = resident_address(&source, &pool, root.list);
        let closure = source
            .into_closure(&pool, root)
            .map_err(|(error, _)| error)
            .expect("source closure");
        let mut destination = pool.start_region::<PageRole>().expect("destination");
        let before = destination.counters();

        let copied = copy_closure_into(&mut pool, &closure, &mut destination, true)
            .expect("mapped closure copy");
        let after = destination.counters();
        let copies = after.whole_payload_copies - before.whole_payload_copies;
        let resident_clones = after.resident_payload_clones - before.resident_payload_clones;

        assert_eq!(copied.len(), size);
        assert_eq!(
            after.source_nodes_copied - before.source_nodes_copied,
            size as u64
        );
        assert_eq!(
            copies, 0,
            "destination-directed encoding performs no resident Clone"
        );
        assert_eq!(resident_clones, 0);
        assert_eq!(
            after.whole_payload_moves - before.whole_payload_moves,
            0,
            "mapped copy does not move a staged whole RegionNode"
        );
        let copied_list = destination.list(&pool, copied).expect("copied list");
        let owned = copied_list
            .iter()
            .map(|node| node.to_owned())
            .collect::<Vec<_>>();
        let expected_identity = SemanticSequenceIdentity::from_nodes(owned.iter());
        assert_eq!(
            copied.page_list().semantic_identity(),
            Some(expected_identity.raw())
        );
        for (index, node) in copied_list.iter().enumerate() {
            assert_eq!(node, crate::NodeView::Penalty(index as i32));
        }
        assert_eq!(
            Some(resident_address(&closure.region, &pool, root.list)),
            Some(source_address)
        );
        eprintln!(
            "MAPPED_REGION_NODE_CLONE_SCALE bytes=32 nodes={size} required_clones={copies} resident_clones={resident_clones} second_payload_transfers=0"
        );
    }
}

#[test]
fn fixed_annex_copy_batches_across_blocks_and_remains_transferable() {
    let mut pool = NodePool::with_chunk_bytes(512);
    let mut source = pool.start_region::<DurableRole>().expect("source");
    let root = source
        .publish_owned(
            &mut pool,
            (0..128).map(|width| {
                let Node::HList(mut node) = boxed(PageListId::empty()) else {
                    unreachable!("boxed helper creates an hlist");
                };
                node.width = Scaled::from_raw(width);
                Node::HList(node)
            }),
        )
        .expect("source box list");
    let mut page = pool.start_region::<PageRole>().expect("page destination");
    let copied = copy_region_root_into(&mut pool, &source, root, &mut page, true)
        .expect("batch copy across annex blocks");
    let copied_list = page.list(&pool, copied).expect("copied box list");
    assert_eq!(copied_list.len(), 128);
    for (index, node) in copied_list.iter().enumerate() {
        let crate::NodeView::HList(node) = node else {
            panic!("copied box shape");
        };
        assert_eq!(node.width, Scaled::from_raw(index as i32));
    }
    let mut closure = page
        .into_closure(&pool, copied)
        .map_err(|(error, _)| error)
        .expect("copied boxes form a sealed closure");
    let mut durable = pool
        .start_region::<DurableRole>()
        .expect("transfer destination");
    let moved = transfer_closure_into(&mut pool, &mut closure, &mut durable)
        .expect("batched annex records satisfy paired transfer floors");
    assert_eq!(durable.list(&pool, moved).expect("moved boxes").len(), 128);
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
        .testing_node_address(0)
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
            .testing_node_address(0),
        Some(address)
    );
    assert_eq!(destination.counters().source_nodes_copied, 0);
    assert_eq!(destination.counters().resident_payload_clones, 0);
    assert_eq!(pool.closure_transition_counters().envelope_moves, 1);
    assert_eq!(pool.closure_transition_counters().rebrand_scan_nodes, 0);
}

#[test]
fn closure_transfer_moves_node_and_annex_suffix_together() {
    let mut pool = NodePool::with_chunk_bytes(64);
    let mut source = pool.start_region::<PageRole>().expect("source");
    let mark = source
        .begin_closure_build(&mut pool)
        .expect("aggregate closure boundary");
    let root = source
        .publish_owned(&mut pool, [Node::Penalty(83)])
        .expect("node suffix");
    let mut annex_builder = source
        .annex_arena
        .begin_builder(&mut pool.annex_chunks)
        .expect("annex suffix builder");
    annex_builder.push(101).expect("first annex word");
    annex_builder.push(103).expect("second annex word");
    let annex = annex_builder.finish();
    let annex_address = source
        .annex_arena
        .list(&pool.annex_chunks, annex)
        .expect("source annex")
        .get(0)
        .map(std::ptr::from_ref)
        .expect("annex address");
    let receipt = source
        .consumed_closure_roots_receipt(&mark)
        .expect("closure roots consumed");
    let closure = source
        .seal_closure(&mut pool, mark, root, receipt)
        .expect("sealed aggregate suffix");
    assert!(source.annex_arena.list(&pool.annex_chunks, annex).is_err());

    let mut destination = pool.start_region::<DurableRole>().expect("destination");
    transfer_sealed_closure_into(&mut pool, &mut source, closure, &mut destination)
        .map_err(|failure| failure.error)
        .expect("aggregate transfer");
    let moved_annex = destination
        .annex_arena
        .list(&pool.annex_chunks, annex)
        .expect("destination annex");
    assert_eq!(moved_annex.iter().copied().collect::<Vec<_>>(), [101, 103]);
    assert_eq!(
        moved_annex.get(0).map(std::ptr::from_ref),
        Some(annex_address),
        "annex transfer preserves the physical suffix"
    );
    assert_eq!(pool.closure_transition_counters().rebrand_scan_nodes, 0);
    assert_eq!(destination.counters().source_nodes_copied, 0);
}

#[test]
fn annex_preflight_failure_returns_the_whole_closure_for_exact_rollback() {
    let mut pool = NodePool::with_chunk_bytes(64);
    let mut source = pool.start_region::<PageRole>().expect("source");
    let mark = source
        .begin_closure_build(&mut pool)
        .expect("aggregate closure boundary");
    let root = source
        .publish_owned(&mut pool, [Node::Penalty(89)])
        .expect("node suffix");
    let mut annex_builder = source
        .annex_arena
        .begin_builder(&mut pool.annex_chunks)
        .expect("annex suffix builder");
    annex_builder.push(107).expect("annex word");
    let annex = annex_builder.finish();
    let receipt = source
        .consumed_closure_roots_receipt(&mark)
        .expect("closure roots consumed");
    let closure = source
        .seal_closure(&mut pool, mark, root, receipt)
        .expect("sealed aggregate suffix");
    let mut destination = pool.start_region::<DurableRole>().expect("destination");
    let mut open_destination_annex =
        crate::fork_arena::ActiveListBuilder::<u32, NodeAnnexLane>::vacant();
    destination
        .annex_arena
        .open_active_list(&pool.annex_chunks, &mut open_destination_annex)
        .expect("open destination annex builder");

    let failure = transfer_sealed_closure_into(&mut pool, &mut source, closure, &mut destination)
        .expect_err("open annex destination must reject before node mutation");
    assert_eq!(failure.error, ForkArenaError::InvalidRegion);
    source
        .rollback_closure(&mut pool, failure.closure)
        .map_err(|failure| failure.error)
        .expect("exact aggregate rollback");
    assert_eq!(source.list(&pool, root).expect("restored nodes").len(), 1);
    assert_eq!(
        source
            .annex_arena
            .list(&pool.annex_chunks, annex)
            .expect("restored annex")
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        [107]
    );
}

#[test]
fn closure_boundary_rotates_node_and_annex_tails() {
    let mut pool = NodePool::with_chunk_bytes(512);
    let mut source = pool.start_region::<PageRole>().expect("source");
    let prefix = source
        .publish_owned(&mut pool, [Node::Penalty(109)])
        .expect("node prefix");
    let mut prefix_annex_builder = source
        .annex_arena
        .begin_builder(&mut pool.annex_chunks)
        .expect("annex prefix builder");
    prefix_annex_builder.push(113).expect("annex prefix");
    let prefix_annex = prefix_annex_builder.finish();
    let prefix_node_address = resident_address(&source, &pool, prefix.list);
    let prefix_annex_address = source
        .annex_arena
        .list(&pool.annex_chunks, prefix_annex)
        .expect("prefix annex")
        .get(0)
        .map(std::ptr::from_ref)
        .expect("prefix annex address");

    let _mark = source
        .begin_closure_build(&mut pool)
        .expect("paired boundary");
    let suffix = source
        .publish_owned(&mut pool, [Node::Penalty(127)])
        .expect("node suffix");
    let mut suffix_annex_builder = source
        .annex_arena
        .begin_builder(&mut pool.annex_chunks)
        .expect("annex suffix builder");
    suffix_annex_builder.push(131).expect("annex suffix");
    let suffix_annex = suffix_annex_builder.finish();
    let suffix_node_address = resident_address(&source, &pool, suffix.list);
    let suffix_annex_address = source
        .annex_arena
        .list(&pool.annex_chunks, suffix_annex)
        .expect("suffix annex")
        .get(0)
        .map(std::ptr::from_ref)
        .expect("suffix annex address");

    assert_ne!(prefix_node_address, suffix_node_address);
    assert_ne!(prefix_annex_address, suffix_annex_address);
    assert_eq!(pool.annex_chunks.chunk_byte_budget(), 4_096);
    assert!(
        source
            .annex_arena
            .payload_chunk_capacity(&pool.annex_chunks)
            <= 1_024
    );
}

#[test]
fn checkpoint_fork_accepts_or_rejects_node_and_annex_as_one_pair() {
    let mut pool = NodePool::with_chunk_bytes(64);
    let mut region = pool.start_region::<PageRole>().expect("region");
    region
        .publish_owned(&mut pool, [Node::Penalty(139)])
        .expect("checkpoint prefix node");
    let mut prefix_annex_builder = region
        .annex_arena
        .begin_builder(&mut pool.annex_chunks)
        .expect("checkpoint prefix annex builder");
    prefix_annex_builder.push(149).expect("prefix annex word");
    prefix_annex_builder.finish();
    let checkpoint = region
        .seal_checkpoint_boundary(&mut pool)
        .and_then(|boundary| region.checkpoint_mark(boundary))
        .expect("aggregate checkpoint");

    let accepted = region
        .publish_owned(&mut pool, [Node::Penalty(151)])
        .expect("accepted suffix node");
    let mut accepted_annex_builder = region
        .annex_arena
        .begin_builder(&mut pool.annex_chunks)
        .expect("accepted suffix annex builder");
    accepted_annex_builder
        .push(157)
        .expect("accepted annex word");
    let accepted_annex = accepted_annex_builder.finish();
    let accepted_address = region
        .annex_arena
        .list(&pool.annex_chunks, accepted_annex)
        .expect("accepted annex view")
        .get(0)
        .map(std::ptr::from_ref)
        .expect("accepted annex address");

    region
        .begin_checkpoint_candidate(&mut pool, checkpoint)
        .expect("begin paired candidate");
    let rejected = region
        .publish_owned(&mut pool, [Node::Penalty(163)])
        .expect("rejected node");
    let mut rejected_annex_builder = region
        .annex_arena
        .begin_builder(&mut pool.annex_chunks)
        .expect("rejected annex builder");
    rejected_annex_builder
        .push(167)
        .expect("rejected annex word");
    let rejected_annex = rejected_annex_builder.finish();
    let rejection = region
        .seal_checkpoint_boundary(&mut pool)
        .expect("rejection boundary");
    region
        .reject_checkpoint_candidate(&mut pool, rejection)
        .expect("reject pair");
    assert!(region.list(&pool, rejected).is_err());
    assert!(
        region
            .annex_arena
            .list(&pool.annex_chunks, rejected_annex)
            .is_err()
    );
    assert_eq!(
        region.list(&pool, accepted).expect("accepted node").len(),
        1
    );
    assert_eq!(
        region
            .annex_arena
            .list(&pool.annex_chunks, accepted_annex)
            .expect("accepted annex restored")
            .get(0)
            .map(std::ptr::from_ref),
        Some(accepted_address)
    );

    region
        .begin_checkpoint_candidate(&mut pool, checkpoint)
        .expect("begin accepted candidate");
    let candidate = region
        .publish_owned(&mut pool, [Node::Penalty(173)])
        .expect("candidate node");
    let mut candidate_annex_builder = region
        .annex_arena
        .begin_builder(&mut pool.annex_chunks)
        .expect("candidate annex builder");
    candidate_annex_builder
        .push(179)
        .expect("candidate annex word");
    let candidate_annex = candidate_annex_builder.finish();
    let acceptance = region
        .seal_checkpoint_boundary(&mut pool)
        .expect("acceptance boundary");
    region
        .accept_checkpoint_candidate(&mut pool, acceptance)
        .expect("accept pair");
    assert!(region.list(&pool, accepted).is_err());
    assert!(
        region
            .annex_arena
            .list(&pool.annex_chunks, accepted_annex)
            .is_err()
    );
    assert_eq!(
        region.list(&pool, candidate).expect("candidate node").len(),
        1
    );
    assert_eq!(
        region
            .annex_arena
            .list(&pool.annex_chunks, candidate_annex)
            .expect("candidate annex")
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        [179]
    );
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
    let child_address = resident_address(&source, &pool, child.list);
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
    let crate::NodeView::HList(box_node) = parent.get(0).expect("parent node") else {
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
        Some(crate::NodeView::Penalty(53))
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
        .testing_node_address(0)
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
            .testing_node_address(0),
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
    let _mark = failure.mark;
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
fn consumed_interleaved_box_interval_moves_and_restores_both_lanes() {
    let mut pool = NodePool::with_chunk_bytes(64);
    let mut page = pool.start_region::<PageRole>().expect("page");
    let before = page
        .publish_owned(&mut pool, [Node::Penalty(61)])
        .expect("page prefix");
    let start = page.begin_closure_build(&mut pool).expect("box start");
    let child = page
        .publish_owned(&mut pool, [Node::Penalty(62)])
        .expect("box child");
    let selected = page
        .publish_owned(&mut pool, [boxed(child.list)])
        .expect("box wrapper");
    let end = page.begin_closure_build(&mut pool).expect("box end");
    let after = page
        .publish_owned(&mut pool, [Node::Penalty(63)])
        .expect("page source rewrite");
    let child_address = page
        .list(&pool, child)
        .expect("child before move")
        .testing_node_address(0);
    let mut durable = pool.start_region::<DurableRole>().expect("durable");

    let (moved, loan) = transfer_page_interior_closure(
        &mut pool,
        &mut page,
        selected,
        start.batch.payload_start()..end.batch.payload_start(),
        start.annex_batch.payload_start()..end.annex_batch.payload_start(),
        &mut durable,
    )
    .expect("consumed box moves");
    assert!(page.list(&pool, before).is_ok());
    assert!(page.list(&pool, after).is_ok());
    assert!(page.list(&pool, child).is_err());
    assert_eq!(
        durable.list(&pool, moved).expect("durable wrapper").len(),
        1
    );
    let moved_child = durable.root(&pool, child.list).expect("moved child");
    assert_eq!(
        durable
            .list(&pool, moved_child)
            .expect("durable child")
            .testing_node_address(0),
        child_address
    );

    rollback_page_interior_closure(&mut pool, &mut page, &mut durable, loan)
        .expect("operation loan rollback");
    assert_eq!(
        page.list(&pool, child)
            .expect("restored child")
            .testing_node_address(0),
        child_address
    );
    assert!(page.list(&pool, selected).is_ok());
    assert!(page.list(&pool, after).is_ok());
}

#[test]
fn paired_box_rollback_checks_annex_before_returning_node_chunks() {
    let mut pool = NodePool::with_chunk_bytes(64);
    let mut page = pool.start_region::<PageRole>().expect("page");
    let start = page.begin_closure_build(&mut pool).expect("box start");
    let child = page
        .publish_owned(&mut pool, [Node::Penalty(73)])
        .expect("child");
    let selected = page
        .publish_owned(&mut pool, [boxed(child.list)])
        .expect("wrapper");
    let end = page.begin_closure_build(&mut pool).expect("box end");
    let mut durable = pool.start_region::<DurableRole>().expect("durable");
    let empty_annex = durable.annex_arena.operation_mark(&pool.annex_chunks);
    let (moved, loan) = transfer_page_interior_closure(
        &mut pool,
        &mut page,
        selected,
        start.batch.payload_start()..end.batch.payload_start(),
        start.annex_batch.payload_start()..end.annex_batch.payload_start(),
        &mut durable,
    )
    .expect("paired box transfer");

    durable
        .annex_arena
        .restore_operation(&mut pool.annex_chunks, empty_annex)
        .expect("invalidate only annex destination");
    assert!(rollback_page_interior_closure(&mut pool, &mut page, &mut durable, loan).is_err());
    assert!(page.list(&pool, selected).is_err());
    assert!(
        durable
            .pub_arena
            .list(&pool.chunks, moved.list.coordinate())
            .is_ok()
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
    let _mark = failure.mark;
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

#[test]
fn copied_parent_chunks_keep_order_and_independent_children_after_source_retirement() {
    let mut pool = NodePool::with_chunk_bytes(64);
    let mut source = pool.start_region::<DurableRole>().expect("source");
    let leaf = source
        .publish_owned(&mut pool, [Node::Penalty(73)])
        .expect("leaf");
    let parent = source
        .publish_owned(&mut pool, (0..9).map(|_| boxed(leaf.list)))
        .expect("parent");
    let mut destination = pool.start_region::<PageRole>().expect("destination");
    let copy =
        copy_region_root_into(&mut pool, &source, parent, &mut destination, true).expect("copy");
    let mut child_roots = Vec::new();
    for node in destination.list(&pool, copy).expect("copied parent").iter() {
        let crate::NodeView::HList(node) = node else {
            panic!("box child");
        };
        assert_ne!(node.children, leaf.list);
        assert!(
            !child_roots.contains(&node.children),
            "each semantic occurrence owns its copy"
        );
        child_roots.push(node.children);
    }
    assert_eq!(child_roots.len(), 9);
    pool.retire_region(source)
        .map_err(|(error, _)| error)
        .expect("retire source");
    for child in child_roots {
        assert_eq!(
            resident_nodes(&destination, &pool, child),
            [Node::Penalty(73)]
        );
    }
}

#[test]
fn copied_sibling_boxes_stamp_before_multiple_annex_blocks_seal() {
    let mut pool = NodePool::with_chunk_bytes(512);
    let mut source = pool.start_region::<DurableRole>().expect("source");
    let leaf = source
        .publish_owned(&mut pool, [Node::Penalty(73)])
        .expect("shared source child");
    // Four hundred 44-word box annex records cross a 64 KiB physical block.
    // Earlier records are sealed before later parent chunks are published.
    let parent = source
        .publish_owned(&mut pool, (0..400).map(|_| boxed(leaf.list)))
        .expect("source siblings");
    let mut destination = pool.start_region::<PageRole>().expect("destination");
    let copied = copy_region_root_into(&mut pool, &source, parent, &mut destination, false)
        .expect("copy siblings");
    let records = destination
        .pub_arena
        .list(&pool.chunks, copied.list.coordinate())
        .expect("copied records");
    let annex =
        crate::node_record::NodeAnnexView::new(&pool.annex_chunks, &destination.annex_arena);
    let children = records
        .iter()
        .map(|record| {
            assert!(
                record.copied_box_body_stamp(annex).is_some(),
                "every copied box carries its own authenticated child envelope"
            );
            let Node::HList(box_node) = record.decode_owned(annex).expect("copied box") else {
                panic!("copied box shape");
            };
            box_node.children
        })
        .collect::<Vec<_>>();
    assert_eq!(children.len(), 400);
    pool.retire_region(source)
        .map_err(|(error, _)| error)
        .expect("retire source");
    for child in children {
        assert_eq!(
            resident_nodes(&destination, &pool, child),
            [Node::Penalty(73)]
        );
    }
}

#[test]
fn copied_boxes_with_inline_children_keep_annex_chunks_packed() {
    let mut pool = NodePool::new();
    let mut source = pool.start_region::<DurableRole>().expect("source");
    let child = source
        .publish_owned(&mut pool, [Node::Penalty(11)])
        .expect("inline source child");
    let roots = (0..400)
        .map(|_| {
            source
                .publish_owned(&mut pool, [boxed(child.list)])
                .expect("source box")
        })
        .collect::<Vec<_>>();
    let mut destination = pool.start_region::<PageRole>().expect("destination");
    for root in roots {
        copy_region_root_into(&mut pool, &source, root, &mut destination, false)
            .expect("independent box copy");
    }
    assert!(
        destination.annex_arena.payload_position_end() < 32,
        "inline children do not strand one 1,024-word annex chunk per box"
    );
}

#[test]
fn copied_box_with_empty_annex_body_survives_source_retirement_and_whole_move() {
    let mut pool = NodePool::new();
    let mut source = pool.start_region::<DurableRole>().expect("source");
    let child = source
        .publish_owned(&mut pool, [Node::Penalty(19)])
        .expect("inline child");
    let root = source
        .publish_owned(&mut pool, [boxed(child.list)])
        .expect("source box");
    let mut page = pool.start_region::<PageRole>().expect("copied page");
    let copied = copy_region_root_into(&mut pool, &source, root, &mut page, false)
        .expect("explicit copied box");
    pool.retire_region(source)
        .map_err(|(error, _)| error)
        .expect("retire source");
    let mut closure = page
        .into_closure(&pool, copied)
        .map_err(|(error, _)| error)
        .expect("copied closure");
    let mut durable = pool
        .start_region::<DurableRole>()
        .expect("moved destination");
    let moved =
        transfer_closure_into(&mut pool, &mut closure, &mut durable).expect("whole-region move");
    let wrapper = resident_nodes(&durable, &pool, moved.list);
    let [Node::HList(boxed)] = wrapper.as_slice() else {
        panic!("moved copied wrapper");
    };
    assert_eq!(
        resident_nodes(&durable, &pool, boxed.children),
        [Node::Penalty(19)]
    );
}

#[test]
fn long_flat_copy_uses_bounded_rust_stack() {
    std::thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(|| {
            let mut pool = NodePool::with_chunk_bytes(64);
            let mut source = pool.start_region::<DurableRole>().expect("source");
            let root = source
                .publish_owned(&mut pool, (0..16_384).map(Node::Penalty))
                .expect("source list");
            let mut destination = pool.start_region::<PageRole>().expect("destination");
            let copied = copy_region_root_into(&mut pool, &source, root, &mut destination, false)
                .expect("copy");
            let view = destination.list(&pool, copied).expect("copied list");
            assert_eq!(view.len(), 16_384);
            assert!(
                view.iter()
                    .enumerate()
                    .all(|(index, node)| node == crate::NodeView::Penalty(index as i32))
            );
        })
        .expect("small-stack worker")
        .join()
        .expect("bounded-stack copy");
}

#[test]
fn nested_fixed_copy_keeps_batch_scratch_off_recursive_stack() {
    std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(|| {
            let mut pool = NodePool::new();
            let mut source = pool.start_region::<DurableRole>().expect("source");
            let mut root = source
                .publish_owned(&mut pool, [Node::Penalty(73)])
                .expect("leaf");
            for _ in 0..64 {
                root = source
                    .publish_owned(&mut pool, [boxed(root.list)])
                    .expect("nested box");
            }
            let mut destination = pool.start_region::<PageRole>().expect("destination");
            let copied = copy_region_root_into(&mut pool, &source, root, &mut destination, false)
                .expect("deep copy");
            pool.retire_region(source)
                .map_err(|(error, _)| error)
                .expect("retire source");
            let mut list = copied.list;
            for _ in 0..64 {
                let admitted = destination.root(&pool, list).expect("copied box root");
                let view = destination.list(&pool, admitted).expect("copied box list");
                let crate::NodeView::HList(boxed) = view.get(0).expect("box") else {
                    panic!("copied node lost its box shape");
                };
                list = boxed.children;
            }
            let admitted = destination.root(&pool, list).expect("copied leaf root");
            assert_eq!(
                destination
                    .list(&pool, admitted)
                    .expect("copied leaf")
                    .get(0),
                Some(crate::NodeView::Penalty(73))
            );
        })
        .expect("small-stack worker")
        .join()
        .expect("bounded recursive fixed copy");
}

#[test]
fn copied_partial_parent_chunk_is_sealed_before_child_construction_continues() {
    let mut pool = NodePool::with_chunk_bytes(64);
    let mut source = pool.start_region::<PageRole>().expect("source");
    let leaf = source
        .publish_owned(&mut pool, [Node::Penalty(17)])
        .expect("leaf");
    let parent = source
        .publish_owned(
            &mut pool,
            [Node::Penalty(0), boxed(leaf.list), boxed(leaf.list)],
        )
        .expect("parent");
    let selected = source
        .pub_arena
        .slice_list(&mut pool.chunks, parent.list.coordinate(), 1..3)
        .expect("partial first chunk");
    let selected = RegionRoot {
        region: source.id,
        list: PageListId::from_parts(selected, None),
        _role: PhantomData,
    };
    let mut durable = pool.start_region::<DurableRole>().expect("durable");
    let copied =
        copy_region_root_into(&mut pool, &source, selected, &mut durable, false).expect("copy");
    let mut closure = durable
        .into_closure(&pool, copied)
        .map_err(|(error, _)| error)
        .expect("closure");
    let mut page = pool.start_region::<PageRole>().expect("page");
    let moved = transfer_closure_into(&mut pool, &mut closure, &mut page)
        .expect("all copied chunks sealed for transfer");
    let nodes = page.list(&pool, moved).expect("moved root");
    assert_eq!(nodes.len(), 2);
    let mut children = Vec::new();
    for node in nodes.iter() {
        let crate::NodeView::HList(node) = node else {
            panic!("box");
        };
        children.push(node.children.coordinate());
        assert_eq!(
            resident_nodes(&page, &pool, node.children),
            [Node::Penalty(17)]
        );
    }
    assert_ne!(
        children[0], children[1],
        "each child occurrence owns its copy"
    );
}

#[test]
fn consumed_cut_copies_only_selected_direct_records_and_rejects_atomically() {
    let mut pool = NodePool::with_chunk_bytes(512);
    let mut source = pool.start_region::<PageRole>().expect("source");
    let list = source
        .publish_owned(&mut pool, (0..12).map(Node::Penalty).collect::<Vec<_>>())
        .expect("one source chunk");
    source
        .seal_checkpoint_boundary(&mut pool)
        .expect("source chunk is immutable");
    let position = source
        .pub_arena
        .owner_relative_head_position(&pool.chunks, list.list.coordinate())
        .expect("source chunk position");
    let mut destination = pool.start_region::<DurableRole>().expect("destination");
    let copied = copy_consumed_direct_cut_into(
        &mut pool,
        &source,
        position,
        3..6,
        &mut destination,
        false,
        Some,
    )
    .expect("bounded shallow cut copy");
    assert_eq!(
        resident_nodes(&destination, &pool, copied),
        vec![Node::Penalty(3), Node::Penalty(4), Node::Penalty(5)]
    );
    assert_eq!(resident_nodes(&source, &pool, list.list).len(), 12);

    let child = source
        .publish_owned(&mut pool, [Node::Penalty(71)])
        .expect("source child");
    let parent = source
        .publish_owned(&mut pool, [boxed(child.list)])
        .expect("source parent");
    source
        .seal_checkpoint_boundary(&mut pool)
        .expect("parent chunk immutable");
    let parent_position = source
        .pub_arena
        .owner_relative_head_position(&pool.chunks, parent.list.coordinate())
        .expect("parent chunk position");
    let before = (
        destination.pub_arena.payload_position_end(),
        destination.annex_arena.payload_position_end(),
    );
    assert_eq!(
        copy_consumed_direct_cut_into(
            &mut pool,
            &source,
            parent_position,
            0..1,
            &mut destination,
            false,
            |_| None,
        ),
        Err(ForkArenaError::InvalidRegion),
    );
    assert_eq!(
        (
            destination.pub_arena.payload_position_end(),
            destination.annex_arena.payload_position_end()
        ),
        before
    );
    assert_eq!(
        resident_nodes(&source, &pool, parent.list),
        vec![boxed(child.list)]
    );
}

#[test]
fn rejected_mixed_generated_rollback_keeps_both_owners_unchanged() {
    let mut pool = NodePool::with_chunk_bytes(512);
    let mut source = pool.start_region::<PageRole>().expect("page source");
    let source_root = source
        .publish_owned(&mut pool, (0..40).map(Node::Penalty).collect::<Vec<_>>())
        .expect("three source chunks");
    source
        .seal_checkpoint_boundary(&mut pool)
        .expect("immutable direct chunks");
    let left = PageListId::from_parts(
        source
            .pub_arena
            .slice_list(&mut pool.chunks, source_root.list.coordinate(), 0..3)
            .expect("left sibling"),
        None,
    );
    let middle = PageListId::from_parts(
        source
            .pub_arena
            .slice_list(&mut pool.chunks, source_root.list.coordinate(), 16..32)
            .expect("whole middle chunk"),
        None,
    );
    let right = PageListId::from_parts(
        source
            .pub_arena
            .slice_list(&mut pool.chunks, source_root.list.coordinate(), 35..40)
            .expect("right sibling"),
        None,
    );
    let start = source
        .pub_arena
        .owner_relative_head_position(&pool.chunks, source_root.list.coordinate())
        .expect("first source chunk");
    let pieces = [
        GeneratedInlinePiece::Cut(crate::page_node_arena::PageBoxCutRange {
            chunk_position: start,
            local: 3..16,
        }),
        GeneratedInlinePiece::Full(middle),
        GeneratedInlinePiece::Cut(crate::page_node_arena::PageBoxCutRange {
            chunk_position: start + 2,
            local: 0..3,
        }),
    ];
    let mut destination = pool
        .start_region::<DurableRole>()
        .expect("durable destination");
    let (projected, mut loan) = transfer_page_generated_inline_selected(
        &mut pool,
        &mut source,
        &pieces,
        &[start + 1..start + 2],
        false,
        &mut destination,
    )
    .expect("mixed cut and complete-chunk projection");
    let projected = PageListId::from_parts(projected, None);
    let before = (
        source.pub_arena.payload_position_end(),
        destination.pub_arena.payload_position_end(),
    );
    let PageInteriorTransferredChunks::GeneratedSelected { heads, .. } = &mut loan.chunks else {
        panic!("mixed transfer returns a generated receipt")
    };
    assert!(heads[0].bound_prefix.is_some());
    heads[0].bound_prefix = None;
    assert_eq!(
        rollback_page_interior_closure(&mut pool, &mut source, &mut destination, loan),
        Err(ForkArenaError::InvalidRegion),
        "inverse preflight rejects a stale head binding before either owner mutates"
    );
    assert_eq!(
        (
            source.pub_arena.payload_position_end(),
            destination.pub_arena.payload_position_end(),
        ),
        before
    );
    assert_eq!(
        resident_nodes(&source, &pool, left),
        (0..3).map(Node::Penalty).collect::<Vec<_>>()
    );
    assert_eq!(
        resident_nodes(&source, &pool, right),
        (35..40).map(Node::Penalty).collect::<Vec<_>>()
    );
    assert_eq!(
        resident_nodes(&destination, &pool, projected),
        (3..35).map(Node::Penalty).collect::<Vec<_>>()
    );
}

#[test]
fn rejected_generated_hole_transfer_restores_source_head_and_floors() {
    let mut pool = NodePool::with_chunk_bytes(512);
    let mut source = pool.start_region::<PageRole>().expect("page source");
    let source_root = source
        .publish_owned(&mut pool, (0..32).map(Node::Penalty).collect::<Vec<_>>())
        .expect("two source chunks");
    source
        .seal_checkpoint_boundary(&mut pool)
        .expect("immutable direct chunks");
    let selected = PageListId::from_parts(
        source
            .pub_arena
            .slice_list(&mut pool.chunks, source_root.list.coordinate(), 16..32)
            .expect("complete second chunk"),
        None,
    );
    let start = source
        .pub_arena
        .owner_relative_head_position(&pool.chunks, source_root.list.coordinate())
        .expect("first source chunk");
    let mut destination = pool
        .start_region::<DurableRole>()
        .expect("durable destination");
    destination
        .pub_arena
        .reserve_vacant_prefix_until(&pool.chunks, start + 1)
        .expect("reserve source-relative prefix");
    let occupied = destination
        .publish_owned(&mut pool, [Node::Penalty(99)])
        .expect("occupy the selected hole");
    destination
        .seal_checkpoint_boundary(&mut pool)
        .expect("occupied hole is sealed");
    assert_eq!(
        transfer_page_generated_inline_selected(
            &mut pool,
            &mut source,
            &[GeneratedInlinePiece::Full(selected)],
            &[start + 1..start + 2],
            false,
            &mut destination,
        )
        .map(|_| ()),
        Err(ForkArenaError::InvalidRegion),
        "the occupied destination rejects after source head and floors detach"
    );
    assert_eq!(
        resident_nodes(&source, &pool, source_root.list),
        (0..32).map(Node::Penalty).collect::<Vec<_>>(),
        "forward rejection restores the complete source chain"
    );
    assert_eq!(
        resident_nodes(&destination, &pool, occupied.list),
        vec![Node::Penalty(99)]
    );
}
