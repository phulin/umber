use super::{
    AlignState, AlignmentKind, AlignmentPackSpec, DisplayEqNo, DisplayInterrupt, EqNoSide,
    ExecError, IncompleteFraction, Mode, ModeLevelSummary, ModeNest,
};
use tex_command::{ConditionalMode, FatalError};
use tex_state::ids::FontId;
use tex_state::math::FractionThickness;
use tex_state::node::{KernKind, Node};
use tex_state::scaled::Scaled;

#[cfg(feature = "profiling")]
#[global_allocator]
static ALLOCATOR: tex_state::measurement::HotCoreAllocator =
    tex_state::measurement::HotCoreAllocator;

#[cfg(feature = "profiling")]
static ALLOCATION_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn kern(value: i32) -> Node {
    Node::Kern {
        amount: Scaled::from_raw(value),
        kind: KernKind::Explicit,
    }
}

fn with_context<R>(
    test: impl for<'id> FnOnce(&mut tex_state::CommandContext<'_, tex_state::GenerationBrand<'id>>) -> R,
) -> R {
    crate::test_harness::with_nonstop_universe(|universe| {
        crate::test_harness::with_admitted(universe, test)
    })
}

fn list_nodes<G>(list: &super::ModeList, context: &tex_state::CommandContext<'_, G>) -> Vec<Node> {
    list.nodes(context)
        .iter()
        .map(|node| node.to_owned())
        .collect()
}

fn nest_nodes<G>(nest: &ModeNest, context: &tex_state::CommandContext<'_, G>) -> Vec<Node> {
    let list = nest.current_list();
    list_nodes(list, context)
}

#[cfg(feature = "profiling")]
fn semantic_apply_allocations() -> tex_state::measurement::HotCoreAllocationMeasurement {
    tex_state::measurement::hot_core_thread_allocation_measurement(
        tex_state::measurement::HotCoreAllocationOwner::SemanticApply,
    )
}

#[test]
fn pending_source_capacity_recycles_on_flush() {
    let mut nest = ModeNest::new();
    nest.push(Mode::Horizontal).expect("horizontal mode");

    {
        let mut list = nest.current_list_mutation();
        list.begin_pending_hchars(
            FontId::testing_new(2),
            'a',
            tex_state::token::OriginId::UNKNOWN,
            tex_fonts::Script::Common,
        );
        for ch in ['b', 'c', 'd', 'e', 'f', 'g'] {
            list.append_pending_hchar(
                FontId::testing_new(2),
                ch,
                tex_state::token::OriginId::UNKNOWN,
                None,
            );
        }
        let pending_capacity = list
            .pending_hchars()
            .expect("pending run")
            .source
            .capacity();
        assert!(pending_capacity > 0);
    }
    assert!(nest.current_list_mutation().clear_pending_hchars());
    let recycled_capacity = nest.storage.scratch.pending_source.capacity();
    assert!(recycled_capacity > 0);
    assert!(nest.storage.scratch.pending_source.is_empty());

    nest.current_list_mutation().begin_pending_hchars(
        FontId::testing_new(3),
        'x',
        tex_state::token::OriginId::UNKNOWN,
        tex_fonts::Script::Common,
    );
    assert_eq!(
        nest.current_list()
            .pending_hchars()
            .expect("recycled pending run")
            .source
            .capacity(),
        recycled_capacity
    );
    assert_eq!(nest.storage.scratch.pending_source.capacity(), 0);
}

#[test]
fn pending_run_admission_reserves_before_first_append_and_stops_at_boundary() {
    let mut nest = ModeNest::new();
    nest.push(Mode::Horizontal).expect("horizontal mode");

    let mut calls = 0;
    let accepted = nest.current_list_mutation().append_pending_hchars(
        FontId::testing_new(2),
        16,
        tex_fonts::Script::Common,
        |index| {
            calls += 1;
            (index < 3).then_some((
                char::from(b'a' + index as u8),
                tex_state::token::OriginId::UNKNOWN,
                None,
            ))
        },
    );
    assert_eq!(accepted, 3);
    assert_eq!(
        calls, 4,
        "the first rejected candidate ends the borrowed walk"
    );
    let pending = nest.current_list().pending_hchars().expect("pending run");
    assert_eq!(pending.source.len(), 3);
    assert!(
        pending.source.capacity() >= 16,
        "the fresh pending vector reserves before begin_pending_hchars pushes"
    );
}

#[test]
fn pending_run_admission_with_zero_acceptance_leaves_list_and_scratch_untouched() {
    let mut nest = ModeNest::new();
    nest.push(Mode::Horizontal).expect("horizontal mode");
    let before = nest.summary();

    let mut calls = 0;
    let accepted = nest.current_list_mutation().append_pending_hchars(
        FontId::testing_new(2),
        8,
        tex_fonts::Script::Common,
        |_| {
            calls += 1;
            None
        },
    );
    assert_eq!(accepted, 0);
    assert_eq!(calls, 1);
    assert_eq!(nest.summary(), before);
    assert_eq!(nest.storage.scratch.pending_source.capacity(), 0);
}

#[test]
fn pending_accumulation_does_not_capture_mode_identity() {
    let mut nest = ModeNest::new();
    nest.push(Mode::Horizontal).expect("horizontal mode");
    super::reset_mode_identity_capture_calls_for_test();

    let appended = nest.current_list_mutation().append_pending_hchars(
        FontId::testing_new(2),
        8,
        tex_fonts::Script::Common,
        |index| {
            (index < 4).then_some((
                char::from(b'a' + index as u8),
                tex_state::token::OriginId::UNKNOWN,
                None,
            ))
        },
    );
    assert_eq!(appended, 4);
    assert_eq!(super::mode_identity_capture_calls_for_test(), 0);
    assert!(nest.current_list_mutation().clear_pending_hchars());
}

#[cfg(feature = "profiling")]
#[test]
fn warmed_pending_source_flushes_allocate_nothing() {
    let _serial = ALLOCATION_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut nest = ModeNest::new();
    nest.push(Mode::Horizontal).expect("horizontal mode");
    for _ in 0..2 {
        let mut list = nest.current_list_mutation();
        list.begin_pending_hchars(
            FontId::testing_new(2),
            'a',
            tex_state::token::OriginId::UNKNOWN,
            tex_fonts::Script::Common,
        );
        for ch in ['b', 'c', 'd', 'e'] {
            list.append_pending_hchar(
                FontId::testing_new(2),
                ch,
                tex_state::token::OriginId::UNKNOWN,
                None,
            );
        }
        assert!(list.clear_pending_hchars());
    }

    let before = semantic_apply_allocations();
    {
        let _scope = tex_state::measurement::hot_core_allocation_scope(
            tex_state::measurement::HotCoreAllocationOwner::SemanticApply,
        );
        for _ in 0..16_384 {
            let mut list = nest.current_list_mutation();
            list.begin_pending_hchars(
                FontId::testing_new(2),
                'a',
                tex_state::token::OriginId::UNKNOWN,
                tex_fonts::Script::Common,
            );
            list.append_pending_hchar(
                FontId::testing_new(2),
                'b',
                tex_state::token::OriginId::UNKNOWN,
                None,
            );
            assert!(list.clear_pending_hchars());
        }
    }
    let after = semantic_apply_allocations();
    assert_eq!(after.calls - before.calls, 0);
    assert_eq!(after.requested_bytes - before.requested_bytes, 0);
}

#[test]
fn nest_usage_records_tex82_pre_push_depth_and_survives_pop() {
    let mut nest = ModeNest::new();
    nest.push(Mode::Horizontal).expect("horizontal push");
    nest.push(Mode::Math).expect("math push");
    assert_eq!(nest.maximum_saved_depth(), 1);

    nest.pop().expect("math pop");
    nest.pop().expect("horizontal pop");
    assert_eq!(nest.maximum_saved_depth(), 1);
}

#[test]
fn independent_mode_summary_materialization_resets_usage_high_water() {
    let mut nest = ModeNest::new();
    nest.push(Mode::Horizontal).expect("horizontal push");
    nest.push(Mode::Math).expect("math push");
    assert_eq!(nest.maximum_saved_depth(), 1);

    let restored = ModeNest::from_summary(nest.summary()).expect("summary materializes");
    assert_eq!(restored.depth(), 3);
    assert_eq!(
        restored.maximum_saved_depth(),
        0,
        "a fresh format/session does not inherit construction-job maxima"
    );
}

#[test]
fn mode_summary_restores_an_independent_semantic_builder() {
    with_context(|context| {
        let mut nest = ModeNest::new();
        nest.push(Mode::Horizontal).expect("test mode push");
        nest.current_list_mutation().push(context, kern(1));
        let summary = nest.summary();

        let snapshot_nodes = list_nodes(
            &summary.levels.last().expect("horizontal level").list,
            context,
        );

        let mut restored = ModeNest::from_summary(summary.clone()).expect("restore mode nest");
        restored.current_list_mutation().push(context, kern(2));

        assert_eq!(snapshot_nodes.len(), 1);
        assert_eq!(
            list_nodes(
                &summary.levels.last().expect("horizontal level").list,
                context,
            )
            .len(),
            1
        );
        assert_eq!(nest_nodes(&restored, context).len(), 2);
    });
}

#[test]
fn preexisting_node_write_barriers_apply_scoped_mutations() {
    with_context(|context| {
        let mut nest = ModeNest::new();
        nest.current_list_mutation().push(context, kern(11));
        nest.current_list_mutation()
            .with_node_mut(context, 0, |node| {
                let Node::Kern { amount, .. } = node else {
                    panic!("fixture node must be a kern");
                };
                *amount = Scaled::from_raw(17);
            })
            .expect("fixture node");
        nest.current_list_mutation()
            .with_last_node_mut(context, |node| {
                let Node::Kern { amount, .. } = node else {
                    panic!("fixture tail must be a kern");
                };
                *amount = Scaled::from_raw(23);
            })
            .expect("fixture tail");

        let nodes = nest_nodes(&nest, context);
        let Node::Kern { amount, .. } = &nodes[0] else {
            panic!("fixture node must remain a kern");
        };
        assert_eq!(*amount, Scaled::from_raw(23));
    });
}

#[test]
fn pushing_a_mode_nest_preserves_the_prior_summary() {
    let mut nest = ModeNest::new();
    let summary = nest.summary();

    nest.push(Mode::Horizontal).expect("test mode push");

    assert_eq!(summary.levels.len(), 1);
    assert_eq!(nest.depth(), 2);
    assert_eq!(nest.current_mode(), Mode::Horizontal);
}

#[test]
fn mode_projection_is_canonical_and_content_sensitive() {
    crate::test_harness::with_nonstop_universe(|universe| {
        let mut first = ModeNest::new();
        first.push(Mode::Horizontal).expect("test mode push");
        let mut equal = ModeNest::new();
        equal.push(Mode::Horizontal).expect("test mode push");
        let mut changed = ModeNest::new();
        changed.push(Mode::Horizontal).expect("test mode push");
        crate::test_harness::with_admitted(universe, |context| {
            first.current_list_mutation().push(context, kern(11));
            equal.current_list_mutation().push(context, kern(11));
            changed.current_list_mutation().push(context, kern(12));
        });
        let first_hash = first.summary().semantic_fingerprint(universe);
        assert_eq!(equal.summary().semantic_fingerprint(universe), first_hash);
        assert_ne!(changed.summary().semantic_fingerprint(universe), first_hash);
    });
}

#[test]
fn semantic_nest_six_modes_and_fields_initialize_canonically() {
    for (mode, family, inner, horizontal_space_factor) in [
        (Mode::Vertical, ConditionalMode::Vertical, false, false),
        (
            Mode::InternalVertical,
            ConditionalMode::Vertical,
            true,
            false,
        ),
        (Mode::Horizontal, ConditionalMode::Horizontal, false, true),
        (
            Mode::RestrictedHorizontal,
            ConditionalMode::Horizontal,
            true,
            true,
        ),
        (Mode::Math, ConditionalMode::Math, true, false),
        (Mode::DisplayMath, ConditionalMode::Math, false, false),
    ] {
        let mut nest = ModeNest::new();
        nest.push(mode).expect("test mode push");
        let list = nest.current_list();
        let conditional_state = nest.conditional_state();

        assert_eq!(nest.current_mode(), mode);
        assert_eq!(conditional_state.mode(), family);
        assert_eq!(conditional_state.is_inner(), inner);
        assert!(list.is_empty());
        assert_eq!(
            list.raw_space_factor(),
            if horizontal_space_factor { 1000 } else { 0 }
        );
        assert_eq!(list.prev_depth(), None);
        assert_eq!(list.prev_graf(), 0);
        assert!(!list.no_boundary());
        assert_eq!(list.hyphen_language(), 0);
        assert!(list.align_state().is_none());
        assert!(list.incomplete_fraction().is_none());
        assert!(list.display_interrupt().is_none());
        assert!(list.display_eq_no().is_none());
    }
}

#[test]
fn semantic_nest_push_and_pop_preserve_fields_and_start_empty_list() {
    with_context(|context| {
        let mut nest = ModeNest::new();
        nest.current_list_mutation().set_prev_graf(7);
        nest.current_list_mutation().push(context, kern(11));

        for mode in [Mode::Horizontal, Mode::Math, Mode::InternalVertical] {
            nest.push(mode).expect("test mode push");
            assert_eq!(nest.current_mode(), mode);
            assert!(nest.current_list().is_empty());
        }
        nest.current_list_mutation()
            .set_prev_depth(Scaled::from_raw(23));
        nest.current_list_mutation().push(context, kern(29));

        let inner = nest.pop().expect("nested mode pops");
        assert_eq!(inner.mode(), Mode::InternalVertical);
        assert_eq!(inner.list().prev_depth(), Some(Scaled::from_raw(23)));
        assert_eq!(list_nodes(inner.list(), context), [kern(29)]);
        assert_eq!(nest.current_mode(), Mode::Math);
        assert!(nest.current_list().is_empty());

        nest.pop().expect("math mode pops");
        nest.pop().expect("horizontal mode pops");
        assert_eq!(nest.current_mode(), Mode::Vertical);
        assert_eq!(nest.current_list().prev_graf(), 7);
        assert_eq!(nest_nodes(&nest, context), [kern(11)]);
        assert!(nest.pop().is_err());
    });
}

#[test]
fn semantic_nest_capacity_and_bottom_pop_recovery_match_tex82() {
    with_context(|context| {
        let mut nest = ModeNest::new();
        nest.current_list_mutation().set_prev_graf(7);
        nest.current_list_mutation().push(context, kern(11));

        for _ in 0..ModeNest::TEX82_NEST_SIZE {
            nest.push(Mode::Horizontal)
                .expect("TeX82 permits nest_size saved semantic levels");
        }
        nest.current_list_mutation().push(context, kern(29));
        let full = nest.summary();

        let error = nest.push(Mode::Math).expect_err("nest_size overflow");
        assert!(matches!(
            &error,
            ExecError::Fatal(FatalError::CapacityExceeded {
                resource: "semantic nest size",
                amount: 40
            })
        ));
        assert_eq!(
            error.as_fatal(),
            Some(FatalError::CapacityExceeded {
                resource: "semantic nest size",
                amount: 40,
            })
        );
        assert_eq!(nest.summary(), full);

        while nest.depth() > 1 {
            nest.pop().expect("saved semantic level pops");
        }
        assert_eq!(nest.current_mode(), Mode::Vertical);
        assert_eq!(nest.current_list().prev_graf(), 7);
        assert_eq!(nest_nodes(&nest, context), [kern(11)]);
        assert!(matches!(nest.pop(), Err(ExecError::CannotPopBaseMode)));

        nest.push(Mode::Math)
            .expect("bottom-pop rejection leaves the nest reusable");
        assert_eq!(nest.current_mode(), Mode::Math);
    });
}

#[test]
fn semantic_nest_capacity_rejection_leaves_the_nest_unchanged() {
    let mut nest = ModeNest::new();
    for _ in 0..ModeNest::TEX82_NEST_SIZE {
        nest.push(Mode::Math).expect("push within TeX82 limit");
    }
    let full = nest.summary();

    let error = nest.push(Mode::Horizontal).expect_err("nest_size overflow");
    assert_eq!(
        error.as_fatal(),
        Some(FatalError::CapacityExceeded {
            resource: "semantic nest size",
            amount: 40,
        })
    );
    assert_eq!(nest.summary(), full);
}

#[test]
fn semantic_nest_summary_cannot_bypass_tex82_capacity() {
    let mut nest = ModeNest::new();
    for _ in 0..ModeNest::TEX82_NEST_SIZE {
        nest.push(Mode::Math).expect("push within TeX82 limit");
    }
    let full = nest.summary();
    assert_eq!(
        ModeNest::from_summary(full.clone())
            .expect("maximum TeX82 nest summary restores")
            .summary(),
        full
    );

    let mut oversized = full.clone();
    oversized.levels.push(ModeLevelSummary::new(Mode::Math));
    let error = ModeNest::from_summary(oversized).expect_err("oversized mode summary");
    assert!(matches!(
        &error,
        ExecError::Fatal(FatalError::CapacityExceeded {
            resource: "semantic nest size",
            amount: 40
        })
    ));
    assert_eq!(
        error.as_fatal(),
        Some(FatalError::CapacityExceeded {
            resource: "semantic nest size",
            amount: 40,
        })
    );
    assert_eq!(nest.summary(), full);
}

fn align_state() -> AlignState {
    AlignState::new(
        AlignmentKind::HAlign,
        AlignmentPackSpec::Natural,
        Vec::new(),
        vec![tex_state::node::GlueValue::owned(
            tex_state::glue::GlueSpec::ZERO,
        )],
        tex_state::node::GlueValue::owned(tex_state::glue::GlueSpec::ZERO),
        None,
    )
}

#[test]
fn mode_entry_lines_survive_summary_restore() {
    let mut nest = ModeNest::new();
    nest.push_at_line(Mode::Horizontal, 7)
        .expect("horizontal mode push");
    let saved = nest.summary();

    let restored = ModeNest::from_summary(saved.clone()).expect("restore mode summary");
    assert_eq!(restored.summary(), saved);
    assert_eq!(restored.summary().levels()[1].entry_line(), 7);
}

#[test]
fn rooted_candidate_rewinds_the_direct_owner_and_rejects_symmetrically() {
    with_context(|context| {
        let mut source = ModeNest::new();
        let checkpoint = source.checkpoint_with_identity(false);
        for index in 0..4_096 {
            source.current_list_mutation().push(context, kern(index));
        }

        {
            let mut candidate = ModeNest::fork_checkpoint(&checkpoint).expect("rooted fork");
            candidate.current_list_mutation().push(context, kern(9_001));
            assert_eq!(nest_nodes(&candidate, context), [kern(9_001)]);
            assert_eq!(
                candidate.current_list_mutation().pop_last_node(context),
                Some(kern(9_001))
            );
            assert!(candidate.current_list().is_empty());
            candidate.reject_checkpoint_candidate();
        }

        let source_nodes = nest_nodes(&source, context);
        assert_eq!(source_nodes.len(), 4_096);
        assert_eq!(source_nodes.first(), Some(&kern(0)));
        assert_eq!(source_nodes.last(), Some(&kern(4_095)));
    });
}

#[test]
fn rooted_candidate_accepts_direct_topology_and_keeps_the_mark_seedable() {
    with_context(|context| {
        let mut source = ModeNest::new();
        let checkpoint = source.checkpoint_with_identity(false);
        source.push(Mode::Horizontal).expect("accepted push");
        source.current_list_mutation().push(context, kern(2));

        let mut candidate = ModeNest::fork_checkpoint(&checkpoint).expect("rooted fork");
        assert_eq!(candidate.depth(), 1);
        assert!(nest_nodes(&candidate, context).is_empty());
        candidate.push(Mode::Vertical).expect("candidate push");
        candidate.current_list_mutation().push(context, kern(3));
        candidate = candidate.accept_checkpoint_candidate();
        assert_eq!(candidate.depth(), 2);
        assert_eq!(nest_nodes(&candidate, context), [kern(3)]);

        {
            let sibling = ModeNest::fork_checkpoint(&checkpoint).expect("sibling fork");
            assert_eq!(sibling.depth(), 1);
            assert!(nest_nodes(&sibling, context).is_empty());
            sibling.reject_checkpoint_candidate();
        }
        assert_eq!(candidate.depth(), 2);
        assert_eq!(nest_nodes(&candidate, context), [kern(3)]);
    });
}

#[test]
fn accepted_candidate_keeps_its_published_mark_seedable() {
    with_context(|context| {
        let mut source = ModeNest::new();
        let root = source.checkpoint_with_identity(false);
        let mut candidate = ModeNest::fork_checkpoint(&root).expect("rooted fork");
        candidate.current_list_mutation().set_prev_graf(1);
        let published = candidate.checkpoint_with_identity(false);
        candidate.current_list_mutation().set_prev_graf(2);
        drop(candidate.accept_checkpoint_candidate());

        let restarted = ModeNest::fork_checkpoint(&published).expect("published fork");
        assert_eq!(restarted.current_list().prev_graf(), 1);
        assert!(nest_nodes(&restarted, context).is_empty());
        restarted.reject_checkpoint_candidate();
    });
}

fn populate_nested_candidate<G>(
    candidate: &mut ModeNest,
    context: &mut tex_state::CommandContext<'_, G>,
) {
    candidate
        .push_at_line(Mode::Horizontal, 17)
        .expect("horizontal candidate level");
    {
        let mut list = candidate.current_list_mutation();
        list.push(context, kern(31));
        list.begin_pending_hchars(
            FontId::testing_new(2),
            'a',
            tex_state::token::OriginId::UNKNOWN,
            tex_fonts::Script::Common,
        );
        list.set_align_state(align_state());
        list.set_space_factor(777);
        list.set_no_boundary(true);
        list.set_hyphen_context(9, 2, 3);
        list.set_prev_depth(Scaled::from_raw(41));
        list.set_prev_graf(5);
    }
    candidate
        .push_at_line(Mode::Math, 23)
        .expect("math candidate level");
    {
        let mut list = candidate.current_list_mutation();
        list.set_incomplete_fraction(
            context,
            IncompleteFraction {
                numerator: tex_state::page_node_arena::PageListSpan::empty(),
                thickness: FractionThickness::Default,
                left_delimiter: Some(7),
                right_delimiter: Some(11),
            },
        );
        list.set_display_interrupt(
            context,
            DisplayInterrupt {
                active_directions: vec![tex_state::node::Direction::BeginR],
                prototype: None,
            },
        );
        list.set_display_eq_no(
            context,
            DisplayEqNo {
                side: EqNoSide::Right,
                display: tex_state::page_node_arena::PageListSpan::empty(),
            },
        );
    }
    candidate
        .push_at_line(Mode::DisplayMath, 29)
        .expect("display candidate level");
    let display = context.publish_page_nodes(vec![kern(43), kern(47)]);
    candidate.current_list_mutation().set_display_alignment(
        context,
        display,
        Some(Scaled::from_raw(53)),
    );
}

#[test]
fn rooted_candidate_consuming_transitions_preserve_all_mode_families_and_sibling_marks() {
    with_context(|context| {
        let mut source = ModeNest::new();
        {
            let mut outer = source.current_list_mutation();
            outer.set_space_factor(901);
            outer.set_no_boundary(true);
            outer.set_hyphen_context(6, 3, 4);
            outer.set_prev_depth(Scaled::from_raw(59));
            outer.set_prev_graf(13);
        }
        let checkpoint = source.checkpoint_with_identity(false);
        let rooted = source.summary();

        let mut accepted = ModeNest::fork_checkpoint(&checkpoint).expect("accepted fork");
        assert_eq!(
            accepted.summary(),
            rooted,
            "fork restores every retained scalar"
        );
        populate_nested_candidate(&mut accepted, context);
        let expected = accepted.summary();
        #[cfg(feature = "profiling")]
        let counters_before_accept = context.page_material_counters();
        accepted = accepted.accept_checkpoint_candidate();
        assert_eq!(
            accepted.summary(),
            expected,
            "accept preserves exact topology"
        );
        #[cfg(feature = "profiling")]
        assert_eq!(
            context.page_material_counters(),
            counters_before_accept,
            "acceptance neither copies nor republishes candidate roots"
        );
        drop(accepted);

        let sibling = ModeNest::fork_checkpoint(&checkpoint).expect("sibling after accept");
        assert_eq!(
            sibling.summary(),
            rooted,
            "accepted work does not rewrite the mark"
        );
        sibling.reject_checkpoint_candidate();

        let mut rejected = ModeNest::fork_checkpoint(&checkpoint).expect("rejected fork");
        populate_nested_candidate(&mut rejected, context);
        #[cfg(feature = "profiling")]
        let counters_before_reject = context.page_material_counters();
        rejected.reject_checkpoint_candidate();
        #[cfg(feature = "profiling")]
        assert_eq!(
            context.page_material_counters(),
            counters_before_reject,
            "mode rejection settles no accepted prefix or replacement copy"
        );

        let sibling = ModeNest::fork_checkpoint(&checkpoint).expect("sibling after reject");
        assert_eq!(
            sibling.summary(),
            rooted,
            "rejection leaves the mark reusable"
        );
        sibling.reject_checkpoint_candidate();
    });
}

#[test]
#[should_panic(expected = "checkpoint candidate mode owner requires explicit accept or reject")]
fn unresolved_mode_candidate_cannot_hide_disposition_in_drop() {
    let mut source = ModeNest::new();
    let checkpoint = source.checkpoint_with_identity(false);
    let _candidate = ModeNest::fork_checkpoint(&checkpoint).expect("rooted candidate");
}

#[test]
fn rooted_candidate_take_excludes_the_accepted_later_suffix() {
    with_context(|context| {
        let mut source = ModeNest::new();
        let checkpoint = source.checkpoint_with_identity(false);
        for index in 0..4_096 {
            source.current_list_mutation().push(context, kern(index));
        }

        {
            let mut candidate = ModeNest::fork_checkpoint(&checkpoint).expect("rooted fork");
            candidate.current_list_mutation().push(context, kern(-2));
            let taken = candidate.current_list_mutation().take_nodes();
            assert_eq!(
                context
                    .page_node_list(taken)
                    .expect("taken candidate list remains live")
                    .nodes()
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>(),
                [kern(-2)]
            );
            assert!(candidate.current_list().is_empty());
            candidate.reject_checkpoint_candidate();
        }

        let source_nodes = nest_nodes(&source, context);
        assert_eq!(source_nodes.len(), 4_096);
        assert_eq!(source_nodes.last(), Some(&kern(4_095)));
    });
}

#[test]
fn boundary_mode_identity_is_demanded_once_and_restores_exactly() {
    with_context(|_| {
        let mut nest = ModeNest::new();
        super::reset_mode_identity_capture_calls_for_test();
        assert_eq!(
            nest.checkpoint_with_identity(false)
                .reachable_state_identity_root(),
            None,
            "ordinary mode checkpoints do not compute convergence identity"
        );
        assert_eq!(super::mode_identity_capture_calls_for_test(), 0);
        let initial = nest
            .checkpoint_with_identity(true)
            .reachable_state_identity_root()
            .expect("mode root is available");
        assert_eq!(super::mode_identity_capture_calls_for_test(), 1);
        nest.current_list_mutation().set_prev_graf(7);
        let scalar = nest
            .checkpoint_with_identity(true)
            .reachable_state_identity_root()
            .expect("mode root is available");
        assert_ne!(scalar, initial);
        let rooted = nest.checkpoint_with_identity(true);
        let expected = rooted
            .reachable_state_identity_root()
            .expect("mode root is available");
        nest.current_list_mutation().set_prev_graf(8);
        assert_ne!(
            nest.checkpoint_with_identity(true)
                .reachable_state_identity_root(),
            Some(expected)
        );
        nest.restore_checkpoint(&rooted).expect("root restores");
        assert_eq!(
            nest.checkpoint_with_identity(true)
                .reachable_state_identity_root(),
            Some(expected)
        );
    });
}

#[test]
#[should_panic(expected = "restart checkpoint requires one quiescent empty outer vertical mode")]
fn transient_mode_material_cannot_publish_a_retained_checkpoint() {
    with_context(|context| {
        let mut source = ModeNest::new();
        source.current_list_mutation().push(context, kern(1));
        let _ = source.checkpoint_with_identity(false);
    });
}

#[test]
#[should_panic(expected = "restart checkpoint requires one quiescent empty outer vertical mode")]
fn pending_horizontal_run_cannot_publish_a_retained_checkpoint() {
    let mut source = ModeNest::new();
    source.current_list_mutation().begin_pending_hchars(
        FontId::testing_new(2),
        'a',
        tex_state::token::OriginId::UNKNOWN,
        tex_fonts::Script::Common,
    );
    let _ = source.checkpoint_with_identity(false);
}

#[test]
#[should_panic(expected = "restart checkpoint requires one quiescent empty outer vertical mode")]
fn nested_mode_cannot_publish_a_retained_checkpoint() {
    let mut source = ModeNest::new();
    source.push(Mode::Horizontal).expect("nested mode");
    let _ = source.checkpoint_with_identity(false);
}

#[test]
fn nested_mode_lifecycle_keeps_one_page_region_and_stable_source_addresses() {
    with_context(|context| {
        let mut nest = ModeNest::new();
        nest.current_list_mutation().push(context, kern(11));
        let owner = context.page_node_region_id();
        let address = context
            .page_node_address_for_test(nest.current_list().nodes.list(), 0)
            .expect("outer node address") as usize;
        #[cfg(feature = "profiling")]
        let counters = context.page_material_counters();

        nest.push(Mode::Horizontal).expect("nested horizontal mode");
        nest.current_list_mutation().push(context, kern(17));
        assert_eq!(context.page_node_region_id(), owner);
        assert!(
            context
                .page_node_span(nest.storage.current.list.nodes.span())
                .is_ok()
        );
        let mut nested = nest.pop().expect("pop nested mode");
        let nested_span = nested.list_mutation().take_span();
        let nested_nodes = context.reclaim_unique_page_nodes(nested_span);
        nest.current_list_mutation()
            .append_unique_list(context, nested_nodes);

        assert_eq!(nest_nodes(&nest, context), [kern(11), kern(17)]);
        assert_eq!(
            context
                .page_node_address_for_test(nest.current_list().nodes.list(), 0)
                .expect("unchanged outer address") as usize,
            address
        );
        #[cfg(feature = "profiling")]
        assert_eq!(
            context.page_material_counters().source_nodes_copied,
            counters.source_nodes_copied,
            "mode nesting moves exact same-region ranges without copying payload"
        );
    });
}

#[test]
fn foreign_cross_generation_mode_span_is_rejected_before_succession() {
    let foreign_root = with_context(|context| {
        let root = context.publish_page_nodes(vec![kern(31)]);
        context
            .admit_page_node_span(root)
            .expect("foreign context admits its own root")
    });
    with_context(|context| {
        let mut nest = ModeNest::new();
        nest.storage
            .current
            .list
            .nodes
            .replace_retained(foreign_root);

        assert!(
            nest.preflight_page_region_succession(context).is_none(),
            "foreign owner-relative roots fail before any region transition"
        );
    });
}

#[test]
fn eligible_checkpoint_has_no_independent_live_mode_list_owner() {
    let mut nest = ModeNest::new();
    nest.current_list_mutation().set_prev_graf(7);
    let checkpoint = nest.checkpoint_with_identity(false);

    assert!(checkpoint.outer.list.nodes.is_empty());
    assert!(checkpoint.outer.list.is_checkpoint_rootless());
    assert!(!checkpoint.retains_page_node_handles());
}

#[test]
fn page_region_succession_preflight_succeeds_after_mode_roots_are_consumed() {
    crate::test_harness::with_nonstop_universe(|universe| {
        let mut nest = ModeNest::new();
        let held_over = {
            let mut context = universe.command_context().expect("live generation");
            nest.current_list_mutation().push(&mut context, kern(37));
            let held_over = nest.current_list_mutation().take_nodes();
            context.prepend_page_contributions(held_over);
            nest.preflight_page_region_succession(&context)
                .expect("consumed mode roots no longer block succession")
        };

        universe
            .prepare_page_region_after_output(held_over)
            .expect("mode-list/page-region combined preflight");
        universe.cancel_page_region_after_output();
    });
}

#[test]
fn generated_source_consumes_the_live_mode_root() {
    with_context(|context| {
        let mut nest = ModeNest::new();
        nest.current_list_mutation().push(context, kern(47));

        let (root, authority) = nest.current_list_mutation().take_generated_source(context);
        assert!(!root.is_empty());
        assert!(authority.is_some(), "the live mode list is the sole owner");
        assert!(nest.current_list().is_empty());
    });
}
