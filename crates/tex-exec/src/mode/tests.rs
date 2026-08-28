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
    list.nodes(context).iter().cloned().collect()
}

fn nest_nodes<G>(nest: &ModeNest, context: &tex_state::CommandContext<'_, G>) -> Vec<Node> {
    let list = nest.current_list();
    list_nodes(&list, context)
}

#[cfg(feature = "profiling")]
fn semantic_apply_allocations() -> tex_state::measurement::HotCoreAllocationMeasurement {
    tex_state::measurement::hot_core_thread_allocation_measurement(
        tex_state::measurement::HotCoreAllocationOwner::SemanticApply,
    )
}

#[cfg(feature = "profiling")]
#[test]
fn warmed_mirrored_node_sequence_append_allocates_no_duplicate_channel_or_lineage_rows() {
    let _serial = ALLOCATION_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut sequence = tex_state::node_sequence::NodeSequence::default();
    for _ in 0..16_384 {
        sequence.push_mirrored(Node::Char {
            font: tex_state::font::NULL_FONT,
            ch: 'a',
            origin: tex_state::token::OriginId::UNKNOWN,
        });
    }
    sequence.truncate(0, 0);

    let before = semantic_apply_allocations();
    {
        let _scope = tex_state::measurement::hot_core_allocation_scope(
            tex_state::measurement::HotCoreAllocationOwner::SemanticApply,
        );
        for _ in 0..16_384 {
            sequence.push_mirrored(Node::Char {
                font: tex_state::font::NULL_FONT,
                ch: 'a',
                origin: tex_state::token::OriginId::UNKNOWN,
            });
        }
    }
    let after = semantic_apply_allocations();
    assert_eq!(after.calls - before.calls, 0);
    assert_eq!(after.requested_bytes - before.requested_bytes, 0);
    assert!(std::ptr::eq(
        sequence.semantic().as_ptr(),
        sequence.physical().as_ptr()
    ));
}

#[cfg(feature = "profiling")]
#[test]
fn warmed_mode_journal_begin_and_commit_allocate_nothing() {
    let _serial = ALLOCATION_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut nest = ModeNest::new();
    let warm = nest.begin_journal();
    nest.commit_journal(warm).expect("warm journal commit");
    drop(tex_state::measurement::hot_core_allocation_scope(
        tex_state::measurement::HotCoreAllocationOwner::SemanticApply,
    ));

    let before = semantic_apply_allocations();
    {
        let _scope = tex_state::measurement::hot_core_allocation_scope(
            tex_state::measurement::HotCoreAllocationOwner::SemanticApply,
        );
        for _ in 0..16_384 {
            let cursor = nest.begin_journal();
            nest.commit_journal(cursor).expect("journal commit");
        }
    }
    let after = semantic_apply_allocations();
    assert_eq!(after.calls - before.calls, 0);
    assert_eq!(after.requested_bytes - before.requested_bytes, 0);
}

#[cfg(feature = "profiling")]
#[test]
fn warmed_long_pending_run_mutation_and_rollback_allocate_nothing() {
    let _serial = ALLOCATION_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut nest = ModeNest::new();
    nest.current_list_mutation().begin_pending_hchars(
        FontId::testing_new(2),
        'a',
        tex_state::token::OriginId::UNKNOWN,
    );
    nest.current_list_mutation()
        .with_pending_hchars_mut(|pending| {
            pending.source.reserve(8_192);
            for _ in 0..4_096 {
                pending.source.push(super::PendingHChar {
                    font: FontId::testing_new(2),
                    ch: 'a',
                    origin: tex_state::token::OriginId::UNKNOWN,
                });
            }
        })
        .expect("pending run");
    let original_len = nest
        .current_list()
        .pending_hchars()
        .expect("pending run")
        .source
        .len();
    drop(tex_state::measurement::hot_core_allocation_scope(
        tex_state::measurement::HotCoreAllocationOwner::SemanticApply,
    ));

    let before = semantic_apply_allocations();
    {
        let _scope = tex_state::measurement::hot_core_allocation_scope(
            tex_state::measurement::HotCoreAllocationOwner::SemanticApply,
        );
        for _ in 0..16_384 {
            let cursor = nest.begin_journal();
            nest.current_list_mutation()
                .with_pending_hchars_mut(|pending| {
                    pending.source.push(super::PendingHChar {
                        font: FontId::testing_new(2),
                        ch: 'b',
                        origin: tex_state::token::OriginId::UNKNOWN,
                    });
                    pending.current = super::PendingHRunChar::new(
                        FontId::testing_new(2),
                        'b',
                        tex_state::token::OriginId::UNKNOWN,
                    );
                })
                .expect("pending run");
            nest.rollback_journal(cursor).expect("journal rollback");
        }
    }
    let after = semantic_apply_allocations();
    assert_eq!(after.calls - before.calls, 0);
    assert_eq!(after.requested_bytes - before.requested_bytes, 0);
    assert_eq!(
        nest.current_list()
            .pending_hchars()
            .expect("pending run")
            .source
            .len(),
        original_len
    );
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
fn semantic_nest_capacity_rejection_does_not_record_a_journal_push() {
    let mut nest = ModeNest::new();
    for _ in 0..ModeNest::TEX82_NEST_SIZE {
        nest.push(Mode::Math).expect("push within TeX82 limit");
    }
    let full = nest.summary();
    nest.reset_journal_for_test();
    let cursor = nest.begin_journal();

    let error = nest.push(Mode::Horizontal).expect_err("nest_size overflow");
    assert_eq!(
        error.as_fatal(),
        Some(FatalError::CapacityExceeded {
            resource: "semantic nest size",
            amount: 40,
        })
    );
    assert_eq!(nest.journal_inverse_len_for_test(), 0);
    nest.rollback_journal(cursor).expect("empty rollback");
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
        vec![tex_state::glue::GlueSpec::ZERO],
        tex_state::glue::GlueSpec::ZERO,
        None,
    )
}

#[test]
fn journal_append_watermarks_restore_scalars_without_append_inverses() {
    with_context(|context| {
        let mut nest = ModeNest::new();
        nest.current_list_mutation().push(context, kern(1));
        let before = nest.summary();
        nest.reset_journal_for_test();
        let cursor = nest.begin_journal();

        {
            let mut list = nest.current_list_mutation();
            list.push(context, kern(2));
            list.append(context, [kern(3), kern(4)]);
            list.push_reconstituted(context, None, kern(5), Some(kern(6)), Some(kern(7)));
            list.set_space_factor(777);
            list.set_no_boundary(true);
            list.set_hyphen_language(9);
            list.set_prev_depth(Scaled::from_raw(11));
            list.set_prev_graf(12);
            list.begin_pending_hchars(
                FontId::testing_new(2),
                'x',
                tex_state::token::OriginId::UNKNOWN,
            );
            list.set_align_state(align_state());
            list.set_incomplete_fraction(IncompleteFraction {
                numerator: tex_state::node_arena::PageListId::empty(),
                thickness: FractionThickness::Explicit(Scaled::from_raw(4)),
                left_delimiter: Some(5),
                right_delimiter: Some(6),
            });
            list.set_display_interrupt(DisplayInterrupt {
                active_directions: Vec::new(),
                prototype: None,
            });
            list.set_display_eq_no(DisplayEqNo {
                side: EqNoSide::Right,
                display: tex_state::node_arena::PageListId::empty(),
            });

            // Later writes in the same frame must preserve the first inverse for
            // each field rather than append another tagged record.
            list.set_space_factor(778);
            list.set_no_boundary(false);
            list.set_hyphen_language(10);
            list.set_prev_depth(Scaled::from_raw(13));
            list.set_prev_graf(14);
            list.set_align_state(align_state());
            list.set_incomplete_fraction(IncompleteFraction {
                numerator: tex_state::node_arena::PageListId::empty(),
                thickness: FractionThickness::Default,
                left_delimiter: None,
                right_delimiter: None,
            });
            list.set_display_interrupt(DisplayInterrupt {
                active_directions: vec![tex_state::node::Direction::BeginR],
                prototype: None,
            });
            list.set_display_eq_no(DisplayEqNo {
                side: EqNoSide::Left,
                display: tex_state::node_arena::PageListId::empty(),
            });
        }

        assert_eq!(
            nest.journal_inverse_len_for_test(),
            11,
            "the detached PageListId replacement records one root inverse, and every later node write deduplicates against it"
        );
        nest.rollback_journal(cursor).expect("rollback");
        assert_eq!(nest.summary(), before);
    });
}

#[test]
fn journal_destructive_node_reconstitution_alignment_and_transfers_restore() {
    with_context(|context| {
        let mut nest = ModeNest::new();
        nest.current_list_mutation()
            .append(context, [kern(10), kern(20), kern(30)]);
        nest.current_list_mutation().set_align_state(align_state());
        let before = nest.summary();
        nest.reset_journal_for_test();
        let cursor = nest.begin_journal();

        nest.current_list_mutation()
            .with_node_mut(context, 0, |node| *node = kern(11));
        nest.current_list_mutation()
            .with_last_node_mut(context, |node| *node = kern(31));
        nest.current_list_mutation()
            .with_reconstitution_target(context, |nodes| {
                nodes.remove(1);
                nodes.insert(0, kern(99));
            });
        nest.current_list_mutation().push_reconstituted(
            context,
            Some((1, kern(88))),
            kern(77),
            None,
            None,
        );
        nest.current_list_mutation().with_align_state_mut(|state| {
            state.start_cell(4, 3);
            state.finish_row();
        });
        let _ = nest.current_list_mutation().take_align_state();
        let _ = nest.current_list_mutation().pop_last_node(context);
        let _ = nest.current_list_mutation().take_nodes();

        assert_eq!(nest.journal_inverse_len_for_test(), 2);
        nest.rollback_journal(cursor).expect("rollback");
        assert_eq!(nest.summary(), before);
    });
}

#[test]
fn alignment_template_coordinates_survive_destructive_journal_rollback() {
    let u_template = tex_state::node::NodeTokenList::new([tex_state::token::TokenWord::pack(
        tex_state::token::Token::Char {
            ch: 'u',
            cat: tex_state::token::Catcode::Other,
        },
    )]);
    let v_template = tex_state::node::NodeTokenList::new([tex_state::token::TokenWord::pack(
        tex_state::token::Token::Char {
            ch: 'v',
            cat: tex_state::token::Catcode::Other,
        },
    )]);
    let mut nest = ModeNest::new();
    nest.current_list_mutation()
        .set_align_state(AlignState::new(
            AlignmentKind::HAlign,
            AlignmentPackSpec::Natural,
            vec![super::AlignColumn {
                u_template: u_template.clone(),
                v_template: v_template.clone(),
            }],
            vec![tex_state::glue::GlueSpec::ZERO],
            tex_state::glue::GlueSpec::ZERO,
            None,
        ));
    nest.reset_journal_for_test();
    let cursor = nest.begin_journal();

    let _ = nest.current_list_mutation().take_align_state();
    nest.rollback_journal(cursor).expect("alignment rollback");

    let current_list = nest.current_list();
    let column = &current_list
        .align_state()
        .expect("alignment restored")
        .columns()[0];
    assert_eq!(column.u_template, u_template);
    assert_eq!(column.v_template, v_template);
}

#[test]
fn journal_math_and_display_ownership_transfers_restore() {
    with_context(|context| {
        let mut nest = ModeNest::new();
        {
            let mut list = nest.current_list_mutation();
            list.set_incomplete_fraction(IncompleteFraction {
                numerator: tex_state::node_arena::PageListId::empty(),
                thickness: FractionThickness::Default,
                left_delimiter: None,
                right_delimiter: Some(9),
            });
            list.set_display_interrupt(DisplayInterrupt {
                active_directions: vec![tex_state::node::Direction::BeginR],
                prototype: None,
            });
            list.set_display_eq_no(DisplayEqNo {
                side: EqNoSide::Left,
                display: tex_state::node_arena::PageListId::empty(),
            });
        }
        let before = nest.summary();
        nest.reset_journal_for_test();
        let cursor = nest.begin_journal();
        {
            let mut list = nest.current_list_mutation();
            assert!(list.take_incomplete_fraction().is_some());
            assert!(list.take_display_interrupt().is_some());
            assert!(list.take_display_eq_no().is_some());
        }
        nest.rollback_journal(cursor).expect("rollback");
        assert_eq!(nest.summary(), before);

        let mut display = ModeNest::new();
        let display_nodes = context.publish_page_nodes(vec![kern(7), kern(8)]);
        display
            .current_list_mutation()
            .set_display_alignment(display_nodes, Some(Scaled::from_raw(9)));
        let before = display.summary();
        display.reset_journal_for_test();
        let cursor = display.begin_journal();
        let restored_nodes = display
            .current_list_mutation()
            .take_display_alignment()
            .expect("display alignment")
            .0;
        assert_eq!(
            context
                .page_node_list(restored_nodes)
                .expect("display alignment remains live")
                .nodes()
                .iter()
                .cloned()
                .collect::<Vec<_>>(),
            [kern(7), kern(8)]
        );
        display.rollback_journal(cursor).expect("display rollback");
        assert_eq!(display.summary(), before);
    });
}

#[test]
fn journal_nested_commit_and_rollback_compose() {
    with_context(|context| {
        let mut outer_rollback = ModeNest::new();
        outer_rollback.reset_journal_for_test();
        let outer = outer_rollback.begin_journal();
        outer_rollback
            .current_list_mutation()
            .push(context, kern(1));
        let inner = outer_rollback.begin_journal();
        outer_rollback
            .current_list_mutation()
            .push(context, kern(2));
        outer_rollback.commit_journal(inner).expect("inner commit");
        outer_rollback
            .rollback_journal(outer)
            .expect("outer rollback");
        assert!(outer_rollback.current_list().is_empty());

        let mut outer_commit = ModeNest::new();
        outer_commit.reset_journal_for_test();
        let outer = outer_commit.begin_journal();
        outer_commit.current_list_mutation().push(context, kern(1));
        let inner = outer_commit.begin_journal();
        outer_commit.current_list_mutation().push(context, kern(2));
        outer_commit
            .rollback_journal(inner)
            .expect("inner rollback");
        outer_commit.commit_journal(outer).expect("outer commit");
        assert_eq!(nest_nodes(&outer_commit, context), [kern(1)]);
    });
}

#[test]
fn journal_level_identity_handles_push_pop_replacement_and_nested_edits() {
    with_context(|context| {
        let mut nest = ModeNest::new();
        nest.push(Mode::Horizontal).expect("test mode push");
        nest.current_list_mutation().push(context, kern(1));
        let before = nest.summary();
        nest.reset_journal_for_test();
        let outer = nest.begin_journal();

        let removed = nest.pop().expect("pop horizontal");
        nest.push(Mode::Math).expect("test mode push");
        nest.current_list_mutation().push(context, kern(2));
        let inner = nest.begin_journal();
        nest.push(Mode::InternalVertical).expect("test mode push");
        nest.current_list_mutation().push(context, kern(3));
        nest.rollback_journal(inner).expect("inner rollback");
        assert_eq!(nest.current_mode(), Mode::Math);
        drop(removed);
        nest.rollback_journal(outer).expect("outer rollback");
        assert_eq!(nest.summary(), before);
    });
}

#[test]
fn mode_entry_lines_survive_summary_restore_and_journal_rollback() {
    let mut nest = ModeNest::new();
    nest.push_at_line(Mode::Horizontal, 7)
        .expect("horizontal mode push");
    let saved = nest.summary();

    let mut restored = ModeNest::from_summary(saved.clone()).expect("restore mode summary");
    assert_eq!(restored.summary().levels()[1].entry_line(), 7);

    restored.reset_journal_for_test();
    let cursor = restored.begin_journal();
    restored.pop().expect("pop restored horizontal mode");
    restored
        .push_at_line(Mode::Math, 11)
        .expect("replacement math mode push");
    restored.rollback_journal(cursor).expect("rollback");

    assert_eq!(restored.summary(), saved);
    assert_eq!(restored.summary().levels()[1].entry_line(), 7);
}

#[test]
fn journal_rejects_a_non_innermost_cursor_without_state_drift() {
    use super::journal::CursorError;

    let mut nest = ModeNest::new();
    nest.reset_journal_for_test();
    let outer = nest.begin_journal();
    let inner = nest.begin_journal();
    assert_eq!(nest.commit_journal(outer), Err(CursorError::NotInnermost));
    nest.rollback_journal(inner).expect("inner rollback");
    nest.commit_journal(outer).expect("outer commit");
    assert_eq!(nest.summary(), ModeNest::new().summary());
}

#[test]
fn journal_fatal_commit_model_and_operational_invisibility_hold() {
    with_context(|context| {
        let mut nest = ModeNest::new();
        nest.reset_journal_for_test();
        let cursor = nest.begin_journal();
        let semantic_before = nest.clone();
        let debug_before = format!("{nest:?}");
        nest.current_list_mutation().push(context, kern(42));
        nest.commit_journal(cursor)
            .expect("fatal path commits partial semantic state");

        assert_ne!(nest, semantic_before);
        assert_eq!(format!("{semantic_before:?}"), debug_before);
        assert_eq!(nest_nodes(&nest, context), [kern(42)]);
        assert_eq!(nest.journal_inverse_len_for_test(), 0);
    });
}

#[test]
fn rooted_candidate_rewinds_the_direct_owner_and_rejects_symmetrically() {
    with_context(|context| {
        let mut source = ModeNest::new();
        source.current_list_mutation().push(context, kern(-1));
        let checkpoint = source.checkpoint();
        for index in 0..4_096 {
            source.current_list_mutation().push(context, kern(index));
        }

        {
            let mut candidate = ModeNest::fork_checkpoint(&checkpoint).expect("rooted fork");
            assert_eq!(nest_nodes(&candidate, context), [kern(-1)]);
            candidate
                .current_list_mutation()
                .with_node_mut(context, 0, |node| *node = kern(-2));
            assert_eq!(nest_nodes(&candidate, context), [kern(-2)]);
            candidate.current_list_mutation().push(context, kern(9_001));
            assert_eq!(nest_nodes(&candidate, context).len(), 2);
            assert_eq!(
                candidate.current_list_mutation().pop_last_node(context),
                Some(kern(9_001))
            );
            assert_eq!(
                candidate.current_list_mutation().pop_last_node(context),
                Some(kern(-2))
            );
            assert!(candidate.current_list().is_empty());
        }

        let source_nodes = nest_nodes(&source, context);
        assert_eq!(source_nodes.len(), 4_097);
        assert_eq!(source_nodes.first(), Some(&kern(-1)));
        assert_eq!(source_nodes.last(), Some(&kern(4_095)));
    });
}

#[test]
fn rooted_candidate_accepts_direct_topology_and_keeps_the_mark_seedable() {
    with_context(|context| {
        let mut source = ModeNest::new();
        source.current_list_mutation().push(context, kern(1));
        let checkpoint = source.checkpoint();
        source.push(Mode::Horizontal).expect("accepted push");
        source.current_list_mutation().push(context, kern(2));

        let mut candidate = ModeNest::fork_checkpoint(&checkpoint).expect("rooted fork");
        assert_eq!(candidate.depth(), 1);
        assert_eq!(nest_nodes(&candidate, context), [kern(1)]);
        candidate.push(Mode::Vertical).expect("candidate push");
        candidate.current_list_mutation().push(context, kern(3));
        candidate.accept_checkpoint_candidate();
        assert_eq!(candidate.depth(), 2);
        assert_eq!(nest_nodes(&candidate, context), [kern(3)]);

        {
            let sibling = ModeNest::fork_checkpoint(&checkpoint).expect("sibling fork");
            assert_eq!(sibling.depth(), 1);
            assert_eq!(nest_nodes(&sibling, context), [kern(1)]);
        }
        assert_eq!(candidate.depth(), 2);
        assert_eq!(nest_nodes(&candidate, context), [kern(3)]);
    });
}

#[test]
fn accepted_candidate_keeps_its_published_mark_seedable() {
    with_context(|context| {
        let mut source = ModeNest::new();
        let root = source.checkpoint();
        let mut candidate = ModeNest::fork_checkpoint(&root).expect("rooted fork");
        candidate.current_list_mutation().push(context, kern(1));
        let published = candidate.checkpoint();
        candidate.current_list_mutation().push(context, kern(2));
        candidate.accept_checkpoint_candidate();

        let restarted = ModeNest::fork_checkpoint(&published).expect("published fork");
        assert_eq!(nest_nodes(&restarted, context), [kern(1)]);
    });
}

#[test]
fn rooted_candidate_take_excludes_the_accepted_later_suffix() {
    with_context(|context| {
        let mut source = ModeNest::new();
        source.current_list_mutation().push(context, kern(-1));
        let checkpoint = source.checkpoint();
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
                [kern(-1), kern(-2)]
            );
            assert!(candidate.current_list().is_empty());
        }

        let source_nodes = nest_nodes(&source, context);
        assert_eq!(source_nodes.len(), 4_097);
        assert_eq!(source_nodes.last(), Some(&kern(4_095)));
    });
}

#[test]
fn maintained_mode_identity_tracks_mutations_and_restores_exactly() {
    with_context(|context| {
        let mut nest = ModeNest::new();
        nest.enable_reachable_state_identity();
        let initial = nest
            .checkpoint()
            .reachable_state_identity_root()
            .expect("mode root is available");
        nest.current_list_mutation().set_prev_graf(7);
        let scalar = nest
            .checkpoint()
            .reachable_state_identity_root()
            .expect("mode root is available");
        assert_ne!(scalar, initial);
        nest.current_list_mutation().push(context, kern(11));
        let rooted = nest.checkpoint();
        let expected = rooted
            .reachable_state_identity_root()
            .expect("mode root is available");
        for index in 0..4_096 {
            nest.current_list_mutation().push(context, kern(index));
        }
        assert_ne!(
            nest.checkpoint().reachable_state_identity_root(),
            Some(expected)
        );
        nest.restore_checkpoint(&rooted).expect("root restores");
        assert_eq!(
            nest.checkpoint().reachable_state_identity_root(),
            Some(expected)
        );
    });
}

#[test]
fn rooted_mode_candidate_identity_rejects_without_layout_dependence() {
    with_context(|context| {
        let mut source = ModeNest::new();
        source.enable_reachable_state_identity();
        source.current_list_mutation().push(context, kern(1));
        let root = source.checkpoint();
        let expected = root.reachable_state_identity_root();
        for index in 0..4_096 {
            source.current_list_mutation().push(context, kern(index));
        }
        {
            let mut candidate = ModeNest::fork_checkpoint(&root).expect("candidate fork");
            assert_eq!(candidate.reachable_state_identity_root(), expected);
            candidate.current_list_mutation().push(context, kern(9_001));
            assert_ne!(candidate.reachable_state_identity_root(), expected);
        }
        assert_ne!(
            source.checkpoint().reachable_state_identity_root(),
            expected
        );
    });
}
