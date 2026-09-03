use super::{
    LigatureWorkItem, LigatureWorkList, OpenTypeShapingScratch, flush_pending_hchar_run_with_fuel,
};

#[test]
fn shaping_scratch_clear_preserves_warm_capacity_without_logical_contents() {
    let mut scratch = OpenTypeShapingScratch::default();
    scratch.text.push_str("mapped text");
    scratch.byte_starts.extend([0, 2, 5]);
    scratch.break_bytes.extend([2, 5]);
    scratch.cluster_advances.extend([(0, 10), (1, 20)]);
    scratch
        .adjustments
        .extend([tex_state::scaled::Scaled::from_raw(1); 3]);
    let capacities = (
        scratch.text.capacity(),
        scratch.byte_starts.capacity(),
        scratch.break_bytes.capacity(),
        scratch.cluster_advances.capacity(),
        scratch.adjustments.capacity(),
    );

    scratch.clear();

    assert!(scratch.text.is_empty());
    assert!(scratch.byte_starts.is_empty());
    assert!(scratch.break_bytes.is_empty());
    assert!(scratch.cluster_advances.is_empty());
    assert!(scratch.adjustments.is_empty());
    assert_eq!(
        capacities,
        (
            scratch.text.capacity(),
            scratch.byte_starts.capacity(),
            scratch.break_bytes.capacity(),
            scratch.cluster_advances.capacity(),
            scratch.adjustments.capacity(),
        )
    );
}

#[test]
fn ligature_work_clear_preserves_capacity_without_stale_links() {
    let mut work = LigatureWorkList::with_capacity(8);
    work.push_back(LigatureWorkItem::Boundary(None));
    work.push_back(LigatureWorkItem::Boundary(Some(b'f')));
    let capacity = work.nodes.capacity();

    work.clear();

    assert!(work.nodes.is_empty());
    assert!(work.head.is_none());
    assert!(work.tail.is_none());
    assert_eq!(work.nodes.capacity(), capacity);
}

#[test]
fn direct_tfm_failure_rolls_back_output_and_clears_reusable_work() {
    crate::test_harness::with_nonstop_plain_universe(|universe| {
        let mut stores = universe.command_context().expect("test state is admitted");
        let mut nest = crate::mode::ModeNest::new();
        nest.push(crate::mode::Mode::Horizontal)
            .expect("horizontal mode");
        nest.current_list_mutation().begin_pending_hchars(
            tex_state::font::NULL_FONT,
            'a',
            tex_state::token::OriginId::UNKNOWN,
        );

        let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut fuel = tex_command::CommandFuelLedger::new(1).expect("bounded fuel");
        let before = stores.page_material_counters();
        let error = flush_pending_hchar_run_with_fuel(
            &mut nest,
            &mut stores,
            &mut effects,
            false,
            false,
            fuel.fuel_mut(),
        )
        .expect_err("second TFM transition exhausts the one-unit budget");
        assert!(matches!(error, crate::ExecError::Command(_)));
        assert!(nest.current_list().pending_hchars().is_some());
        assert!(nest.current_list().nodes(&stores).is_empty());
        let after = stores.page_material_counters();
        assert_eq!(after.new_semantic_nodes, before.new_semantic_nodes);
        assert_eq!(
            after.destination_values_constructed,
            before.destination_values_constructed
        );
        assert_eq!(after.identity_nodes_hashed, before.identity_nodes_hashed);

        let mut retry_fuel = tex_command::CommandFuelLedger::default();
        flush_pending_hchar_run_with_fuel(
            &mut nest,
            &mut stores,
            &mut effects,
            false,
            false,
            retry_fuel.fuel_mut(),
        )
        .expect("retry after rollback uses the cleared work list");
        assert!(nest.current_list().pending_hchars().is_none());
        assert_eq!(nest.current_list().nodes(&stores).len(), 1);
    });
}
