use tex_state::measurement::{
    HotCoreAllocationOwner, HotCoreAllocator, hot_core_allocation_scope,
    hot_core_thread_allocation_measurement,
};
use tex_state::node::Node;
use tex_state::node_region::NodePool;
use tex_state::page_node_arena::{
    PageMaterialActiveListBuilder, PageMaterialArena, PageMaterialRegion,
};

#[global_allocator]
static ALLOCATOR: HotCoreAllocator = HotCoreAllocator;

fn main() {
    assert_eq!(core::mem::size_of::<Node>(), 168);
    let mut pool = NodePool::with_chunk_bytes(4 * 1024);
    let mut region = PageMaterialRegion::new(&mut pool);
    let mut arena = PageMaterialArena::new(&mut pool, &mut region);
    let empty = arena.operation_mark();

    construct(&mut arena, 4_096);
    arena.restore_operation(empty).expect("restore warmup");

    for nodes in [1, 4_096] {
        let mark = arena.operation_mark();
        let counters_before = arena.counters();
        let owner = HotCoreAllocationOwner::SemanticApply;
        let allocation_before = hot_core_thread_allocation_measurement(owner);
        {
            let _scope = hot_core_allocation_scope(owner);
            construct(&mut arena, nodes);
        }
        let allocation_after = hot_core_thread_allocation_measurement(owner);
        let counters_after = arena.counters();
        let allocations = allocation_after.calls - allocation_before.calls;
        let requested_bytes = allocation_after.requested_bytes - allocation_before.requested_bytes;
        let moves = counters_after.whole_payload_moves - counters_before.whole_payload_moves;
        let copies = counters_after.whole_payload_copies - counters_before.whole_payload_copies;
        let constructed = counters_after.destination_values_constructed
            - counters_before.destination_values_constructed;
        let blocks =
            counters_after.direct_blocks_allocated - counters_before.direct_blocks_allocated;

        println!(
            "PAGE_NODE_DESTINATION_GATE nodes={nodes} allocations={allocations} requested_bytes={requested_bytes} whole_node_moves={moves} whole_node_copies={copies} destination_nodes={constructed} structural_blocks={blocks}"
        );
        assert_eq!((allocations, requested_bytes, moves, copies), (0, 0, 0, 0));
        assert_eq!(constructed, nodes as u64);
        assert_eq!(
            blocks,
            nodes.div_ceil(arena.payload_chunk_capacity()) as u64
        );
        arena.restore_operation(mark).expect("restore measured row");
    }
}

fn construct(arena: &mut PageMaterialArena<'_>, nodes: usize) {
    let mut builder = PageMaterialActiveListBuilder::vacant();
    arena.open_active_list(&mut builder).expect("open builder");
    for penalty in 0..nodes {
        arena
            .construct_active_list(&mut builder, |slot| {
                *slot = Some(Node::Penalty(penalty as i32));
            })
            .expect("construct resident node");
    }
    let list = arena
        .finalize_active_list(&mut builder)
        .expect("finish builder");
    assert_eq!(list.len(), nodes);
}
