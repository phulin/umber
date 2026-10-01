#[cfg(feature = "profiling")]
use tex_state::measurement::HotCoreAllocationOwner;

#[test]
#[cfg(feature = "profiling")]
fn resident_source_delivery_skips_owner_validation_at_one_and_4096_operations() {
    fn run(operations: usize) -> ((u64, u64, u64, u64), u64, u64) {
        crate::test_harness::with_universe(|universe| {
            let mut state = crate::CommandState::default();
            let source = state
                .register_source(crate::SourceRegistration::new(
                    crate::RegisteredSourceKind::Generated,
                    std::sync::Arc::<[u8]>::from(vec![b'x'; operations + 1]),
                ))
                .expect("source delivery fixture registers");
            state
                .open_registered_source(source)
                .expect("source delivery fixture opens");
            state.profile_prepare_source_line(1);

            let mut context = universe.command_context().expect("command context");
            let mut capabilities = crate::CommandHostCapabilities::default();
            let mut fuel = crate::CommandFuelLedger::new(
                u64::try_from(operations).expect("operation count fits u64") + 16,
            )
            .expect("source delivery fixture fuel limit");
            let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
            let mut command_slot = None;

            state.profile_reset_input_source_context_counters();
            let owner = HotCoreAllocationOwner::DeliveryAndScan;
            let allocations_before =
                tex_state::measurement::hot_core_thread_allocation_measurement(owner);
            {
                let _scope = tex_state::measurement::hot_core_allocation_scope(owner);
                let mut processor = crate::test_harness::processor(
                    &mut state,
                    &mut context,
                    &mut capabilities,
                    &mut fuel,
                    &mut diagnostic_effects,
                );
                for _ in 0..operations {
                    assert!(matches!(
                        processor.get_next_into(&mut command_slot),
                        Ok(crate::DeliveryStatus::Command)
                    ));
                    command_slot = None;
                }
            }
            let allocations_after =
                tex_state::measurement::hot_core_thread_allocation_measurement(owner);
            (
                state.profile_input_source_context_counters(),
                allocations_after.calls - allocations_before.calls,
                allocations_after.requested_bytes - allocations_before.requested_bytes,
            )
        })
    }

    let one = run(1);
    let four_k = run(4_096);
    assert_eq!(one, ((0, 0, 0, 1), 0, 0));
    assert_eq!(four_k, ((0, 0, 0, 4_096), 0, 0));
}
