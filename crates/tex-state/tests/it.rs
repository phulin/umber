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
fn callback_scoped_command_admission_is_allocation_free_and_stationary() {
    use tex_state::measurement::{
        HotCoreAllocationOwner, hot_core_allocation_scope, hot_core_thread_allocation_measurement,
    };

    let budget = tex_state::interner::InternerBudget::new(32, 32, 1024).expect("budget");
    tex_state::with_universe(budget, |universe| {
        let owner = HotCoreAllocationOwner::SemanticApply;
        let before = hot_core_thread_allocation_measurement(owner);
        let mut admitted_slot = None;
        {
            let _scope = hot_core_allocation_scope(owner);
            for _ in 0..4_096 {
                universe
                    .with_command_context(|context| {
                        let address = std::ptr::from_ref(context).cast::<()>();
                        assert_eq!(*admitted_slot.get_or_insert(address), address);
                        std::hint::black_box(context.execution_group_depth());
                    })
                    .expect("callback admission");
            }
        }
        let after = hot_core_thread_allocation_measurement(owner);
        assert_eq!(after.calls - before.calls, 0);
        assert_eq!(after.requested_bytes - before.requested_bytes, 0);
    })
    .expect("universe allocation");
}

#[cfg(feature = "profiling")]
#[test]
fn repeated_shipout_marks_allocate_and_copy_no_string_pool_storage() {
    use tex_state::measurement::{
        HotCoreAllocationOwner, hot_core_allocation_scope, hot_core_thread_allocation_measurement,
    };

    let measure = |retained_strings: bool| {
        let budget = tex_state::interner::InternerBudget::new(32, 32, 1024).expect("budget");
        tex_state::with_universe(budget, |universe| {
            if retained_strings {
                let mut context = universe.command_context().expect("context");
                for index in 0..1_024 {
                    context.slow_make_string_pool_string(&format!("retained-{index:04}"));
                }
            }
            let owner = HotCoreAllocationOwner::GenerationBoundary;
            drop(universe.begin_shipout());
            let before = hot_core_thread_allocation_measurement(owner);
            {
                let _scope = hot_core_allocation_scope(owner);
                for _ in 0..4_096 {
                    drop(std::hint::black_box(universe.begin_shipout()));
                }
            }
            let after = hot_core_thread_allocation_measurement(owner);
            (
                after.calls - before.calls,
                after.requested_bytes - before.requested_bytes,
            )
        })
        .expect("universe allocation")
    };
    assert_eq!(
        measure(true),
        measure(false),
        "shipout capture allocation and copy work is independent of retained strings"
    );

    let budget = tex_state::interner::InternerBudget::new(32, 32, 1024).expect("budget");
    tex_state::with_universe(budget, |universe| {
        {
            let mut context = universe.command_context().expect("context");
            for index in 0..1_024 {
                context.slow_make_string_pool_string(&format!("retained-{index:04}"));
            }
        }
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

#[cfg(feature = "profiling")]
#[test]
fn warmed_definition_group_nesting_and_retired_history_make_no_allocator_calls() {
    use tex_state::measurement::{
        HotCoreAllocationOwner, hot_core_allocation_scope, hot_core_thread_allocation_measurement,
    };

    for depth in [1_usize, 64, 4_096] {
        let budget = tex_state::interner::InternerBudget::new(32, 32, 1024).expect("budget");
        tex_state::with_universe(budget, |universe| {
            universe
                .profile_definition_region_group_cycle(depth, 4_096)
                .expect("warm definition region slots");

            let owner = HotCoreAllocationOwner::ArenaGrowth;
            let before = hot_core_thread_allocation_measurement(owner);
            {
                let _scope = hot_core_allocation_scope(owner);
                universe
                    .profile_definition_region_group_cycle(depth, 4_096)
                    .expect("measured definition region slots");
            }
            let after = hot_core_thread_allocation_measurement(owner);
            assert_eq!(after.calls - before.calls, 0, "warmed depth {depth}");
            assert_eq!(
                after.requested_bytes - before.requested_bytes,
                0,
                "warmed depth {depth}"
            );
        })
        .expect("universe allocation");
    }
}
