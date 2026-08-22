use tex_state::glue::{GlueSpec, Order};
use tex_state::provenance::OriginRecord;
use tex_state::scaled::Scaled;
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use super::{CommandGroupError, CommandState};
use crate::macro_call::MacroArgumentBuilder;
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
            admitted.token_list(receipt.token_lists[0]),
            &[word('b').token_word()]
        );
        assert_eq!(
            admitted.token_list(receipt.token_lists[1]),
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
            admitted.token_list(receipt.token_lists[0]),
            &[word('x').token_word()]
        );
        assert_eq!(admitted.glue(receipt.glue[0]), glue(42));
        assert_eq!(
            admitted
                .definition(receipt.definitions[0])
                .replacement_text(),
            &[word('x').token_word()]
        );
        assert_eq!(
            admitted.provenance(receipt.provenance[0]),
            OriginRecord::UnknownBootstrap
        );
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
        state.discard_attempt_operation(mark);

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

        state.discard_attempt_operation(mark);
        assert_eq!(state.attempt_token_words(retained), Ok(&[word('a')][..]));
        assert!(state.attempt_token_words(rejected).is_err());
    });
}

#[test]
fn live_pre_mark_macro_arguments_bound_suffix_reclamation() {
    crate::test_harness::with_universe(|universe| {
        let mut state = CommandState::default();
        let definition = universe
            .allocate_definition(&[], &[])
            .expect("macro definition");
        let name = universe.intern("outer").expect("macro name").symbol();
        let retained = state
            .attempt
            .arena_mut()
            .allocate_token_list([word('a')])
            .expect("retained argument");
        let mut arguments = MacroArgumentBuilder::default();
        arguments.complete(1, retained).expect("first argument");
        let arguments = arguments
            .finish(state.attempt.arena_mut())
            .expect("argument record");
        let level = state.push_macro_activation(name, definition, arguments, OriginId::UNKNOWN, 0);

        let mark = state.begin_attempt_operation();
        let discarded = state
            .attempt
            .arena_mut()
            .allocate_token_list([word('b')])
            .expect("dead operation scratch");
        state
            .reclaim_attempt_operation(mark)
            .expect("live owner census");
        assert_eq!(state.attempt_token_words(retained), Ok(&[word('a')][..]));
        assert!(state.attempt_token_words(discarded).is_err());

        state
            .retire_exhausted_input(level)
            .expect("empty macro body retires");
        state
            .reclaim_unreachable_attempt_suffix()
            .expect("retired arguments reclaim");
        assert!(state.attempt.is_empty());
    });
}

#[test]
fn post_mark_macro_arguments_survive_commit_until_activation_retirement() {
    crate::test_harness::with_universe(|universe| {
        let mut state = CommandState::default();
        let definition = universe
            .allocate_definition(&[], &[])
            .expect("macro definition");
        let name = universe.intern("inner").expect("macro name").symbol();
        let mark = state.begin_attempt_operation();
        let retained = state
            .attempt
            .arena_mut()
            .allocate_token_list([word('x')])
            .expect("new argument");
        let mut arguments = MacroArgumentBuilder::default();
        arguments.complete(1, retained).expect("first argument");
        let arguments = arguments
            .finish(state.attempt.arena_mut())
            .expect("argument record");
        let level = state.push_macro_activation(name, definition, arguments, OriginId::UNKNOWN, 0);

        state
            .reclaim_attempt_operation(mark)
            .expect("post-mark command root survives");
        assert_eq!(state.attempt_token_words(retained), Ok(&[word('x')][..]));

        state
            .retire_exhausted_input(level)
            .expect("empty macro body retires");
        state
            .reclaim_unreachable_attempt_suffix()
            .expect("retired suffix reclaims");
        assert!(state.attempt.is_empty());
        assert!(state.attempt_token_words(retained).is_err());
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
        assert_eq!(restored_opening, opening);
        assert_eq!(restored_resume, resume);
        assert_eq!(request, "font request");
        state.discard_attempt_operation(restored_opening);
        assert_eq!(state.attempt_token_words(retained), Ok(&[word('a')][..]));
        assert!(state.attempt_token_words(rejected).is_err());
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

        assert!(
            state
                .suspend_attempt(
                    universe,
                    opening,
                    crate::AttemptResumePoint::default(),
                    "input request",
                )
                .is_err()
        );
        assert_eq!(state.attempt_token_words(retained), Ok(&[word('x')][..]));
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
        assert!(state.pending_expansions.is_empty());
        assert!(state.pending_scan_toks.is_empty());
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
        command
            .begin_group(&mut state, tex_state::GroupKind::Math, 2)
            .expect("inner group");
        command.save_aftergroup(&state, word('c')).expect("inner c");
        command.save_aftergroup(&state, word('d')).expect("inner d");

        assert_eq!(
            command
                .end_group(&mut state, tex_state::GroupKind::Math)
                .expect("inner closes")
                .into_aftergroup(),
            vec![word('c'), word('d')]
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
