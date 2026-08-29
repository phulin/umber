#[path = "it/capability_boundaries.rs"]
mod capability_boundaries;
#[path = "it/handle_serialization.rs"]
mod handle_serialization;
#[path = "it/live_boundary.rs"]
mod live_boundary;

#[cfg(feature = "profiling")]
#[global_allocator]
static SHIPOUT_MARK_ALLOCATOR: tex_state::measurement::HotCoreAllocator =
    tex_state::measurement::HotCoreAllocator;

#[cfg(feature = "profiling")]
#[test]
fn repeated_shipout_marks_allocate_and_copy_no_string_pool_storage() {
    use tex_state::measurement::{
        HotCoreAllocationOwner, hot_core_allocation_scope, hot_core_thread_allocation_measurement,
    };

    let budget = tex_state::interner::InternerBudget::new(32, 32, 1024).expect("budget");
    tex_state::with_universe(budget, |universe| {
        let owner = HotCoreAllocationOwner::GenerationBoundary;
        drop(universe.begin_shipout());
        let empty_before = hot_core_thread_allocation_measurement(owner);
        {
            let _scope = hot_core_allocation_scope(owner);
            for _ in 0..4_096 {
                drop(std::hint::black_box(universe.begin_shipout()));
            }
        }
        let empty_after = hot_core_thread_allocation_measurement(owner);
        let empty = (
            empty_after.calls - empty_before.calls,
            empty_after.requested_bytes - empty_before.requested_bytes,
        );

        {
            let mut context = universe.command_context().expect("context");
            for index in 0..1_024 {
                context.slow_make_string_pool_string(&format!("retained-{index:04}"));
            }
        }
        drop(universe.begin_shipout());

        let before = hot_core_thread_allocation_measurement(owner);
        {
            let _scope = hot_core_allocation_scope(owner);
            for _ in 0..4_096 {
                drop(std::hint::black_box(universe.begin_shipout()));
            }
        }
        let after = hot_core_thread_allocation_measurement(owner);
        assert_eq!(
            (
                after.calls - before.calls,
                after.requested_bytes - before.requested_bytes
            ),
            empty,
            "shipout capture allocation and copy work is independent of retained strings"
        );

        let mut context = universe.command_context().expect("post-gate context");
        let before_hit = context.detach_engine_usage_statistics();
        context.slow_make_string_pool_string("retained-1023");
        assert_eq!(
            context.detach_engine_usage_statistics(),
            before_hit,
            "zero-copy marks retain the original membership owner"
        );
    })
    .expect("universe allocation");
}
