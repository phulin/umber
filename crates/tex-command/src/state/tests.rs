use tex_state::glue::{GlueSpec, Order};
use tex_state::provenance::OriginRecord;
use tex_state::scaled::Scaled;
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use super::{CommandGroupError, CommandSemanticDiagnostic, CommandState};
use crate::processor::AlignmentIdentity;
use crate::{AttemptError, AttemptPromotionRoots};

fn word(ch: char) -> TracedTokenWord {
    TracedTokenWord::pack(
        Token::Char {
            ch,
            cat: Catcode::Other,
        },
        OriginId::UNKNOWN,
    )
}

#[test]
fn semantic_diagnostic_transfer_moves_the_ordered_allocation_without_allocating() {
    let mut state = CommandState::<()>::default();
    state
        .semantic_diagnostics
        .push(CommandSemanticDiagnostic::Trace {
            text: "first".to_owned(),
            force_newline: false,
        });
    state
        .semantic_diagnostics
        .push(CommandSemanticDiagnostic::MissingNumber {
            context: "second".to_owned(),
        });
    state
        .semantic_diagnostics
        .push(CommandSemanticDiagnostic::PdfExpansionMessage {
            text: "third".to_owned(),
        });
    let allocation = state.semantic_diagnostics.as_ptr();
    let capacity = state.semantic_diagnostics.capacity();

    #[cfg(feature = "profiling")]
    let owner = tex_state::measurement::HotCoreAllocationOwner::DeliveryAndScan;
    #[cfg(feature = "profiling")]
    let before = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
    let diagnostics;
    {
        #[cfg(feature = "profiling")]
        let _scope = tex_state::measurement::hot_core_allocation_scope(owner);
        diagnostics = state.take_semantic_diagnostics();
    }
    #[cfg(feature = "profiling")]
    let after = tex_state::measurement::hot_core_thread_allocation_measurement(owner);

    #[cfg(feature = "profiling")]
    assert_eq!(after.calls - before.calls, 0);
    #[cfg(feature = "profiling")]
    assert_eq!(after.requested_bytes - before.requested_bytes, 0);
    assert_eq!(diagnostics.as_ptr(), allocation);
    assert_eq!(diagnostics.capacity(), capacity);
    assert!(state.semantic_diagnostics.is_empty());
    assert_eq!(state.semantic_diagnostics.capacity(), 0);
    assert!(matches!(
        &diagnostics[..],
        [
            CommandSemanticDiagnostic::Trace {
                text,
                force_newline: false,
            },
            CommandSemanticDiagnostic::MissingNumber { context },
            CommandSemanticDiagnostic::PdfExpansionMessage { text: pdf_text },
        ] if text == "first" && context == "second" && pdf_text == "third"
    ));
}

#[test]
fn synchronous_attempt_child_scope_reclaims_only_its_exact_suffix() {
    let mut state = CommandState::<()>::default();
    let operation = state.begin_attempt_operation();
    let child = state
        .begin_attempt_child_scope()
        .expect("active operation admits one synchronous child");
    let scratch = state
        .attempt
        .arena_mut()
        .allocate_token_list([word('x')])
        .expect("child scratch");
    assert_eq!(state.attempt_token_words(scratch), Ok(&[word('x')][..]));

    state
        .close_attempt_child_scope(child)
        .expect("move-only receipt closes its exact child");
    assert_eq!(
        state.attempt_token_words(scratch),
        Err(AttemptError::InvalidCoordinate)
    );
    state
        .commit_attempt_operation(operation)
        .expect("child close left the parent owner intact");
}

#[test]
fn synchronous_attempt_child_scope_requires_an_active_operation() {
    let mut state = CommandState::<()>::default();
    assert_eq!(
        state
            .begin_attempt_child_scope()
            .expect_err("a synchronous child requires an active operation"),
        AttemptError::InvalidCoordinate
    );
    let operation = state.begin_attempt_operation();
    let child = state
        .begin_attempt_child_scope()
        .expect("active operation admits a child");
    state
        .close_attempt_child_scope(child)
        .expect("child closes normally");
    state
        .commit_attempt_operation(operation)
        .expect("parent closes normally");
}

fn glue(width: i32) -> GlueSpec {
    GlueSpec {
        width: Scaled::from_raw(width),
        stretch: Scaled::from_raw(0),
        stretch_order: Order::Normal,
        shrink: Scaled::from_raw(0),
        shrink_order: Order::Normal,
    }
}

#[test]
fn attempt_promotion_preserves_multiple_root_order_and_duplicates() {
    crate::test_harness::with_universe(|universe| {
        let mut state = CommandState::default();
        let first = state
            .attempt
            .arena_mut()
            .allocate_token_list([word('a')])
            .expect("first list");
        let second = state
            .attempt
            .arena_mut()
            .allocate_token_list([word('b')])
            .expect("second list");

        let receipt = state
            .promote_attempt_roots(
                universe,
                AttemptPromotionRoots::new(&[second, first, second], &[], &[], &[]),
            )
            .expect("promotion");

        assert_eq!(receipt.token_lists.len(), 3);
        assert_eq!(receipt.token_lists[0], receipt.token_lists[2]);
        let admitted = universe.command_context().expect("admission");
        assert_eq!(
            admitted
                .token_list(receipt.token_lists[0].clone())
                .iter()
                .collect::<Vec<_>>(),
            &[word('b').token_word()]
        );
        assert_eq!(
            admitted
                .token_list(receipt.token_lists[1].clone())
                .iter()
                .collect::<Vec<_>>(),
            &[word('a').token_word()]
        );
    });
}

#[test]
fn attempt_promotion_returns_mixed_roots_in_declared_order() {
    crate::test_harness::with_universe(|universe| {
        let mut state = CommandState::default();
        let parameter = state
            .attempt
            .arena_mut()
            .allocate_token_list([word('#')])
            .expect("parameter text");
        let replacement = state
            .attempt
            .arena_mut()
            .allocate_token_list([word('x')])
            .expect("replacement text");
        let definition = state
            .attempt
            .arena_mut()
            .allocate_definition(parameter, replacement)
            .expect("definition");
        let glue_root = state
            .attempt
            .arena_mut()
            .allocate_glue(glue(42))
            .expect("glue");
        let provenance = state
            .attempt
            .arena_mut()
            .allocate_provenance(OriginRecord::UnknownBootstrap)
            .expect("provenance");

        let receipt = state
            .promote_attempt_roots(
                universe,
                AttemptPromotionRoots::new(
                    &[replacement],
                    &[glue_root],
                    &[definition],
                    &[provenance],
                ),
            )
            .expect("mixed promotion");

        let admitted = universe.command_context().expect("admission");
        assert_eq!(
            admitted
                .token_list(receipt.token_lists[0].clone())
                .iter()
                .collect::<Vec<_>>(),
            &[word('x').token_word()]
        );
        assert_eq!(admitted.glue(receipt.glue[0]), glue(42));
        assert_eq!(
            admitted
                .definition(receipt.definitions[0].clone())
                .replacement_text(),
            &[word('x').token_word()]
        );
        assert_eq!(
            admitted.provenance(receipt.provenance[0]),
            OriginRecord::UnknownBootstrap
        );
    });
}

#[cfg(feature = "profiling")]
#[test]
fn warmed_single_definition_promotion_ignores_the_large_live_attempt_arena() {
    crate::test_harness::with_universe(|universe| {
        let mut state = CommandState::default();
        for _ in 0..16_384 {
            state
                .attempt
                .arena_mut()
                .allocate_token_list([word('z')])
                .expect("large unrelated attempt row");
        }
        let parameter = state
            .attempt
            .arena_mut()
            .allocate_token_list([word('#')])
            .expect("parameter text");
        let replacement = state
            .attempt
            .arena_mut()
            .allocate_token_list([word('x')])
            .expect("replacement text");
        let definition = state
            .attempt
            .arena_mut()
            .allocate_definition(parameter, replacement)
            .expect("definition");

        // Grow the two generation-owned destination vectors before measuring;
        // ordinary repeated promotions then reuse their retained capacity.
        for _ in 0..17 {
            state
                .promote_attempt_definition(universe, definition)
                .expect("warm definition promotion");
        }
        let owner = tex_state::measurement::HotCoreAllocationOwner::SemanticApply;
        let before = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
        {
            let _scope = tex_state::measurement::hot_core_allocation_scope(owner);
            for _ in 0..8 {
                state
                    .promote_attempt_definition(universe, definition)
                    .expect("measured definition promotion");
            }
        }
        let after = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
        assert_eq!(after.calls - before.calls, 0);
        assert_eq!(after.requested_bytes - before.requested_bytes, 0);
    });
}

#[test]
fn foreign_attempt_root_rejection_is_mutation_free() {
    crate::test_harness::with_universe(|universe| {
        let state = CommandState::<_>::default();
        let mut foreign = CommandState::<()>::default();
        let foreign_root = foreign
            .attempt
            .arena_mut()
            .allocate_token_list([word('x')])
            .expect("foreign root");

        assert!(matches!(
            state.promote_attempt_roots(
                universe,
                AttemptPromotionRoots::new(&[foreign_root], &[], &[], &[]),
            ),
            Err(AttemptError::ForeignAttempt)
        ));
        let retirement = universe.retire().expect("retirement");
        assert_eq!(retirement.token_list_rows(), 0);
        assert_eq!(retirement.definition_rows(), 0);
        assert_eq!(retirement.glue_rows(), 0);
        assert_eq!(retirement.provenance_rows(), 0);
    });
}

#[test]
fn stale_root_rejection_validates_complete_batch_before_mutation() {
    crate::test_harness::with_universe(|universe| {
        let mut state = CommandState::default();
        let valid = state
            .attempt
            .arena_mut()
            .allocate_token_list([word('a')])
            .expect("valid root");
        let mark = state.begin_attempt_operation();
        let stale = state
            .attempt
            .arena_mut()
            .allocate_token_list([word('b')])
            .expect("stale root");
        state
            .rollback_attempt_operation(mark)
            .expect("operation rolls back");

        assert!(matches!(
            state.promote_attempt_roots(
                universe,
                AttemptPromotionRoots::new(&[valid, stale], &[], &[], &[]),
            ),
            Err(AttemptError::InvalidCoordinate)
        ));
        let retirement = universe.retire().expect("retirement");
        assert_eq!(retirement.token_list_rows(), 0);
        assert_eq!(retirement.definition_rows(), 0);
        assert_eq!(retirement.glue_rows(), 0);
        assert_eq!(retirement.provenance_rows(), 0);
    });
}

#[test]
fn operation_discard_truncates_only_the_attempt_suffix() {
    crate::test_harness::with_universe(|_universe| {
        let mut state = CommandState::<()>::default();
        let retained = state
            .attempt
            .arena_mut()
            .allocate_token_list([word('a')])
            .expect("retained list");
        let mark = state.begin_attempt_operation();
        let rejected = state
            .attempt
            .arena_mut()
            .allocate_token_list([word('b')])
            .expect("candidate list");

        state
            .rollback_attempt_operation(mark)
            .expect("operation rolls back");
        assert_eq!(state.attempt_token_words(retained), Ok(&[word('a')][..]));
        assert!(state.attempt_token_words(rejected).is_err());
    });
}

#[test]
fn successful_scope_commit_reclaims_promoted_operation_rows() {
    crate::test_harness::with_universe(|universe| {
        let mut state = CommandState::default();
        let operation = state.begin_attempt_operation();
        let coordinate = operation.coordinate();
        let parameter = state
            .attempt
            .arena_mut()
            .allocate_token_list([word('#')])
            .expect("parameter text");
        let replacement = state
            .attempt
            .arena_mut()
            .allocate_token_list([word('x')])
            .expect("replacement text");
        let definition = state
            .attempt
            .arena_mut()
            .allocate_definition(parameter, replacement)
            .expect("definition");
        let durable = state
            .promote_attempt_definition(universe, definition)
            .expect("successful operation publishes its durable root");

        state
            .commit_attempt_operation(operation)
            .expect("operation scope commits");
        assert!(matches!(
            state.rollback_attempt_operation(crate::CommandAttemptOperation::new(coordinate)),
            Err(AttemptError::InvalidCoordinate)
        ));
        assert!(state.attempt.is_empty());
        assert_eq!(
            universe
                .command_context()
                .expect("admission")
                .definition(durable)
                .replacement_text(),
            &[word('x').token_word()]
        );
    });
}

#[test]
fn macro_scratch_descriptor_survives_attempt_suspension_without_an_arena_owner() {
    crate::test_harness::with_universe(|universe| {
        let definition = universe
            .allocate_definition(&[], &[])
            .expect("empty macro definition");
        let name = universe
            .intern("suspendedmacro")
            .expect("macro name")
            .symbol();
        let mut state = CommandState::default();
        let operation = state.begin_attempt_operation();
        let coordinate = operation.coordinate();
        let matching = state.scratch.begin_macro_match().expect("macro match");
        let frame = state
            .scratch
            .commit_macro_match(matching)
            .expect("sealed empty frame");
        let level = state.push_macro_activation(
            name,
            definition,
            crate::macro_call::MacroArguments::new(frame),
            OriginId::UNKNOWN,
            0,
        );

        let pending = state
            .suspend_attempt(
                universe,
                operation,
                crate::AttemptResumePoint::default(),
                "resource",
            )
            .expect("attempt suspension");
        assert!(state.attempt.is_empty());
        assert_eq!(state.scratch.frame_len(), 1);
        let (resumed, _, request) = state
            .resume_attempt(universe, pending)
            .ok()
            .expect("attempt resumption");
        assert_eq!(resumed.coordinate(), coordinate);
        assert_eq!(request, "resource");
        state
            .retire_exhausted_input(level)
            .expect("macro body retirement");
        assert!(state.scratch.is_quiescent());
        state
            .commit_attempt_operation(resumed)
            .expect("operation commit");
    });
}

#[test]
fn resource_suspension_moves_the_arena_and_restores_its_opening_cursor() {
    crate::test_harness::with_universe(|universe| {
        let mut state = CommandState::default();
        let retained = state
            .attempt
            .arena_mut()
            .allocate_token_list([word('a')])
            .expect("pre-operation attempt value");
        let opening = state.begin_attempt_operation();
        let opening_coordinate = opening.coordinate();
        let rejected = state
            .attempt
            .arena_mut()
            .allocate_token_list([word('b')])
            .expect("operation-local attempt value");
        let resume = crate::AttemptResumePoint {
            command: 3,
            scanner: 5,
            expansion: 7,
            subordinate: 11,
        };

        let pending = state
            .suspend_attempt(universe, opening, resume, "font request")
            .expect("live generation owner");
        assert!(state.attempt.is_empty());
        assert_eq!(
            universe.retire(),
            Err(tex_state::UniverseError::State(
                tex_state::StateError::GenerationInUse
            ))
        );

        let (restored_opening, restored_resume, request) = state
            .resume_attempt(universe, pending)
            .ok()
            .expect("same admitted generation");
        assert_eq!(restored_opening.coordinate(), opening_coordinate);
        assert_eq!(restored_resume, resume);
        assert_eq!(request, "font request");
        state
            .rollback_attempt_operation(restored_opening)
            .expect("resumed operation rolls back");
        assert_eq!(state.attempt_token_words(retained), Ok(&[word('a')][..]));
        assert!(state.attempt_token_words(rejected).is_err());
    });
}

#[test]
fn nested_scanner_scopes_survive_resource_suspension_and_resume_once() {
    crate::test_harness::with_universe(|universe| {
        let mut state = CommandState::default();
        let operation = state.begin_attempt_operation();
        let coordinate = operation.coordinate();
        let scanner = state.begin_attempt_scanner_scope().expect("scanner scope");
        let scanner_child = state.begin_attempt_scanner_scope().expect("scanner child");
        let child_value = state
            .attempt
            .arena_mut()
            .allocate_token_list([word('x')])
            .expect("nested child value");
        let pending = state
            .suspend_attempt(
                universe,
                operation,
                crate::AttemptResumePoint::default(),
                (scanner, scanner_child),
            )
            .expect("nested scopes suspend with their arena owner");

        let (resumed, _, (scanner, scanner_child)) = state
            .resume_attempt(universe, pending)
            .ok()
            .expect("nested scopes resume into the same state");
        assert_eq!(resumed.coordinate(), coordinate);
        assert_eq!(state.attempt_token_words(child_value), Ok(&[word('x')][..]));
        state
            .discard_attempt_scope_suffix(scanner_child)
            .expect("top scanner child retires");
        state
            .defer_attempt_scope_retirement(scanner)
            .expect("scanner defers until commit");
        state
            .commit_attempt_operation(resumed)
            .expect("commit consumes each owner exactly once");
        assert!(state.attempt.is_empty());
        assert!(matches!(
            state.commit_attempt_operation(crate::CommandAttemptOperation::new(coordinate)),
            Err(AttemptError::InvalidCoordinate)
        ));
    });
}

#[test]
fn failed_resource_suspension_keeps_the_live_attempt_installed() {
    crate::test_harness::with_universe(|universe| {
        let mut state = CommandState::default();
        let retained = state
            .attempt
            .arena_mut()
            .allocate_token_list([word('x')])
            .expect("attempt value");
        let opening = state.begin_attempt_operation();
        universe.retire().expect("unowned generation retires");

        let failure = match state.suspend_attempt(
            universe,
            opening,
            crate::AttemptResumePoint::default(),
            "input request",
        ) {
            Ok(_) => panic!("retired generation must reject suspension"),
            Err(failure) => failure,
        };
        let (opening, error) = failure.into_parts();
        assert!(matches!(error, crate::AttemptSuspendError::Generation(_)));
        assert_eq!(state.attempt_token_words(retained), Ok(&[word('x')][..]));
        state
            .commit_attempt_operation(opening)
            .expect("rejected suspension returns the live operation owner");
    });
}

#[test]
fn stale_resource_suspension_mark_is_typed_and_mutation_free() {
    crate::test_harness::with_universe(|universe| {
        let mut state = CommandState::default();
        let retained = state
            .attempt
            .arena_mut()
            .allocate_token_list([word('x')])
            .expect("retained attempt value");
        let live = state.begin_attempt_operation();
        let live_coordinate = live.coordinate();
        state
            .attempt
            .arena_mut()
            .allocate_token_list([word('y')])
            .expect("discarded attempt value");
        state
            .rollback_attempt_operation(live)
            .expect("operation rolls back");

        let failure = match state.suspend_attempt(
            universe,
            crate::CommandAttemptOperation::new(live_coordinate),
            crate::AttemptResumePoint::default(),
            "input request",
        ) {
            Ok(_) => panic!("truncated opening mark must be stale"),
            Err(failure) => failure,
        };
        let (_, error) = failure.into_parts();
        assert!(matches!(
            error,
            crate::AttemptSuspendError::StaleMark(crate::AttemptError::InvalidCoordinate)
        ));
        assert_eq!(state.attempt_token_words(retained), Ok(&[word('x')][..]));
    });
}

#[test]
fn resource_resume_rejects_a_nonempty_live_attempt_without_mutation() {
    crate::test_harness::with_universe(|universe| {
        let mut state = CommandState::default();
        let opening = state.begin_attempt_operation();
        let pending = state
            .suspend_attempt(
                universe,
                opening,
                crate::AttemptResumePoint::default(),
                "font request",
            )
            .expect("attempt suspends");
        let live = state
            .attempt
            .arena_mut()
            .allocate_token_list([word('z')])
            .expect("new live attempt value");

        let pending = state
            .resume_attempt(universe, pending)
            .expect_err("a pending arena cannot overwrite live attempt state");
        assert_eq!(state.attempt_token_words(live), Ok(&[word('z')][..]));

        state.attempt = crate::CommandAttempt::default();
        let (_, _, request) = state
            .resume_attempt(universe, pending)
            .ok()
            .expect("unchanged pending attempt remains resumable");
        assert_eq!(request, "font request");
    });
}

#[test]
fn alignment_state_restores_outer_running_depth_after_nested_lifecycle() {
    crate::test_harness::with_universe(|_universe| {
        let mut state = CommandState::<()>::default();
        let outer = AlignmentIdentity::new(1);
        let inner = AlignmentIdentity::new(2);
        state.begin_alignment(outer);
        state.suspend_alignment(outer).expect("suspend outer");
        state.begin_alignment(inner);
        state.finish_alignment(inner).expect("finish inner");
        state.resume_alignment(outer).expect("resume outer");
        state.finish_alignment(outer).expect("finish outer");
        assert_eq!(state.alignment.align_state, 1_000_000);
    });
}

#[test]
fn default_command_state_is_quiescent_at_a_cold_summary_boundary() {
    crate::test_harness::with_universe(|_universe| {
        let state = CommandState::<()>::default();
        assert!(state.scanner.is_quiescent());
        assert!(state.input.levels.is_empty());
    });
}

#[test]
fn nested_group_payloads_restore_exact_save_order() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        let mut state = universe.command_context().expect("admitted state");
        command
            .begin_group(&mut state, tex_state::GroupKind::Simple, 1)
            .expect("outer group");
        command.save_aftergroup(&state, word('a')).expect("outer a");
        command.save_aftergroup(&state, word('b')).expect("outer b");
        let outer_projection = command.aftergroup_save_stack_projection();
        assert_eq!(outer_projection.0, 2);
        assert!(outer_projection.1.is_some());
        command
            .begin_group(&mut state, tex_state::GroupKind::Math, 2)
            .expect("inner group");
        command.save_aftergroup(&state, word('c')).expect("inner c");
        command.save_aftergroup(&state, word('d')).expect("inner d");
        let nested_projection = command.aftergroup_save_stack_projection();
        assert_eq!(nested_projection.0, 4);
        assert!(nested_projection.1 > outer_projection.1);

        assert_eq!(
            command
                .end_group(&mut state, tex_state::GroupKind::Math)
                .expect("inner closes")
                .into_aftergroup(),
            vec![word('c'), word('d')]
        );
        assert_eq!(
            command.aftergroup_save_stack_projection(),
            outer_projection,
            "closing the inner level restores the outer ordering owner"
        );
        assert_eq!(
            command
                .end_group(&mut state, tex_state::GroupKind::Simple)
                .expect("outer closes")
                .into_aftergroup(),
            vec![word('a'), word('b')]
        );
    });
}

#[test]
fn stale_state_group_rejection_precedes_payload_mutation() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        let mut state = universe.command_context().expect("admitted state");
        state
            .begin_group(tex_state::GroupKind::Simple, 1)
            .expect("bypass creates stale state");

        assert_eq!(
            command.set_afterassignment(&state, word('x')),
            Err(CommandGroupError::StaleGroupState)
        );
        assert!(!command.has_afterassignment());
    });
}
