use std::hint::black_box;

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

const ITERATIONS: usize = 10_000;

fn main() {
    let mut pool = NodePool::with_chunk_bytes(4 * 1024);
    let mut region = PageMaterialRegion::new(&mut pool);
    let mut arena = PageMaterialArena::new(&mut pool, &mut region);
    let source = arena
        .publish_owned((0..64).map(Node::Penalty))
        .expect("publish source");
    let span = arena.admit_span(source).expect("admit source span");
    let rollback = arena.operation_mark();

    exercise(&mut arena, span);
    arena
        .restore_operation(rollback)
        .expect("discard warm retained descriptor");

    let copies_before = arena.counters().source_nodes_copied;
    let owner = HotCoreAllocationOwner::SemanticApply;
    let before = hot_core_thread_allocation_measurement(owner);
    {
        let _scope = hot_core_allocation_scope(owner);
        for _ in 0..ITERATIONS {
            exercise(&mut arena, span);
            arena
                .restore_operation(rollback)
                .expect("discard retained descriptor");
        }
    }
    let after = hot_core_thread_allocation_measurement(owner);
    let allocations = after.calls.saturating_sub(before.calls);
    let bytes = after.requested_bytes.saturating_sub(before.requested_bytes);
    let copies = arena
        .counters()
        .source_nodes_copied
        .saturating_sub(copies_before);

    println!(
        "PAGE_SPAN_GATE iterations={ITERATIONS} allocations={allocations} requested_bytes={bytes} source_nodes_copied={copies}"
    );
    assert_eq!(allocations, 0, "warmed checked-span traversal allocates");
    assert_eq!(bytes, 0, "warmed checked-span traversal requests bytes");
    assert_eq!(copies, 0, "checked-span retention copies source nodes");
}

fn exercise(arena: &mut PageMaterialArena<'_>, span: tex_state::page_node_arena::PageListSpan) {
    let nodes = arena.span_node_cursor(span).expect("span remains live");
    black_box(nodes.first());
    black_box(nodes.last());
    let mut retained = PageMaterialActiveListBuilder::vacant();
    arena.open_active_list(&mut retained).expect("open list");
    arena
        .append_span_range_to_active_list(&mut retained, span, 7..57)
        .expect("retain span range");
    black_box(
        arena
            .finalize_active_list(&mut retained)
            .expect("finalize retained list"),
    );
}
