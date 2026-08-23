use tex_state::glue::{GlueSpec, Order};
use tex_state::provenance::OriginRecord;
use tex_state::scaled::Scaled;
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use super::{CommandGroupError, CommandState};
use crate::input::OutParameterReplay;
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
fn root_census_can_reclaim_below_a_stale_operation_mark() {
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
        let stale = state.begin_attempt_operation();
        let durable = state
            .promote_attempt_definition(universe, definition)
            .expect("successful operation publishes its durable root");

        state
            .reclaim_unreachable_attempt_suffix()
            .expect("typed roots define an empty live cursor");
        assert!(matches!(
            state.reclaim_attempt_operation(stale),
            Err(AttemptError::InvalidCoordinate)
        ));
        assert!(matches!(
            state.attempt.arena_mut().truncate(stale.attempt_mark()),
            Err(AttemptError::InvalidCoordinate)
        ));
        state
            .reclaim_unreachable_attempt_suffix()
            .expect("successful commit does not require a stale opening mark");
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
fn live_pre_mark_macro_arguments_bound_suffix_reclamation() {
    crate::test_harness::with_universe(|universe| {
        let mut state = CommandState::default();
        let definition = universe
            .allocate_definition(&[], &[])
            .expect("macro definition");
        let name = universe.intern("outer").expect("macro name").symbol();
        let opening = state
            .macro_activation_opening_cursor()
            .expect("opening roots");
        let retained = state
            .attempt
            .arena_mut()
            .allocate_token_list([word('a')])
            .expect("retained argument");
        let mut arguments = MacroArgumentBuilder::default();
        arguments.complete(1, retained).expect("first argument");
        let arguments = arguments
            .finish(state.attempt.arena_mut(), opening)
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
        let opening = state
            .macro_activation_opening_cursor()
            .expect("opening roots");
        let retained = state
            .attempt
            .arena_mut()
            .allocate_token_list([word('x')])
            .expect("new argument");
        let mut arguments = MacroArgumentBuilder::default();
        arguments.complete(1, retained).expect("first argument");
        let arguments = arguments
            .finish(state.attempt.arena_mut(), opening)
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
fn scanner_rollback_preserves_post_mark_parameter_replay_roots() {
    crate::test_harness::with_universe(|universe| {
        let mut state = CommandState::default();
        let definition = universe
            .allocate_definition(&[], &[])
            .expect("macro definition");
        let name = universe.intern("inner").expect("macro name").symbol();
        let scanner_mark = state.attempt.arena().mark();
        let opening = state
            .macro_activation_opening_cursor()
            .expect("opening roots");
        let retained = state
            .attempt
            .arena_mut()
            .allocate_token_list([word('x')])
            .expect("new argument");
        let mut arguments = MacroArgumentBuilder::default();
        arguments.complete(1, retained).expect("first argument");
        let arguments = arguments
            .finish(state.attempt.arena_mut(), opening)
            .expect("argument record");
        let level = state.push_macro_activation(name, definition, arguments, OriginId::UNKNOWN, 0);
        state
            .replay_out_parameter(level, 1)
            .expect("parameter replay level");
        assert_ne!(
            state.input_argument_watermark,
            crate::attempt::AttemptInputWatermark::default()
        );
        let scratch = state
            .attempt
            .arena_mut()
            .allocate_token_list([word('z')])
            .expect("scanner-local scratch");

        state
            .rollback_attempt_scanner_scratch(scanner_mark)
            .expect("scanner rollback preserves nested command roots");

        assert_eq!(state.attempt_token_words(retained), Ok(&[word('x')][..]));
        assert!(state.attempt_token_words(scratch).is_err());
    });
}

#[test]
fn nested_macro_retirement_preserves_live_parameter_replay() {
    crate::test_harness::with_universe(|universe| {
        let mut state = CommandState::default();
        let definition = universe
            .allocate_definition(&[], &[])
            .expect("macro definition");
        let name = universe.intern("nested").expect("macro name").symbol();

        let outer_opening = state
            .macro_activation_opening_cursor()
            .expect("outer opening roots");
        let argument = state
            .attempt
            .arena_mut()
            .allocate_token_list([word('a')])
            .expect("outer argument");
        let mut arguments = MacroArgumentBuilder::default();
        arguments.complete(1, argument).expect("outer slot");
        let arguments = arguments
            .finish(state.attempt.arena_mut(), outer_opening)
            .expect("outer argument record");
        let outer = state.push_macro_activation(name, definition, arguments, OriginId::UNKNOWN, 0);
        let replay = state
            .replay_out_parameter(outer, 1)
            .expect("parameter replay");
        let OutParameterReplay::Pushed(replay) = replay else {
            panic!("outer parameter must push a replay level");
        };

        let inner_opening = state
            .macro_activation_opening_cursor()
            .expect("inner opening roots");
        let inner_arguments = MacroArgumentBuilder::default()
            .finish(state.attempt.arena_mut(), inner_opening)
            .expect("empty inner argument record");
        let inner =
            state.push_macro_activation(name, definition, inner_arguments, OriginId::UNKNOWN, 0);
        state
            .retire_exhausted_input(inner)
            .expect("inner body retires around outer replay");
        assert_eq!(state.attempt_token_words(argument), Ok(&[word('a')][..]));

        state
            .retire_exhausted_input(replay)
            .expect("parameter replay retires in LIFO order");
        assert_eq!(
            state.input_argument_watermark,
            crate::attempt::AttemptInputWatermark::default()
        );
        state
            .retire_exhausted_input(outer)
            .expect("outer body retires after replay");
        assert!(state.attempt.is_empty());
    });
}

#[test]
fn nested_macro_retirement_preserves_outer_arguments() {
    crate::test_harness::with_universe(|universe| {
        let mut state = CommandState::default();
        let definition = universe
            .allocate_definition(&[], &[])
            .expect("macro definition");
        let name = universe.intern("nested").expect("macro name").symbol();

        let outer_opening = state
            .macro_activation_opening_cursor()
            .expect("outer opening roots");
        let outer = state
            .attempt
            .arena_mut()
            .allocate_token_list([word('o')])
            .expect("outer argument");
        let mut outer_arguments = MacroArgumentBuilder::default();
        outer_arguments.complete(1, outer).expect("outer slot");
        let outer_arguments = outer_arguments
            .finish(state.attempt.arena_mut(), outer_opening)
            .expect("outer argument record");
        let outer_level =
            state.push_macro_activation(name, definition, outer_arguments, OriginId::UNKNOWN, 0);

        let inner_opening = state
            .macro_activation_opening_cursor()
            .expect("inner opening roots");
        let inner = state
            .attempt
            .arena_mut()
            .allocate_token_list([word('i')])
            .expect("inner argument");
        let mut inner_arguments = MacroArgumentBuilder::default();
        inner_arguments.complete(1, inner).expect("inner slot");
        let inner_arguments = inner_arguments
            .finish(state.attempt.arena_mut(), inner_opening)
            .expect("inner argument record");
        let inner_level =
            state.push_macro_activation(name, definition, inner_arguments, OriginId::UNKNOWN, 0);

        state
            .retire_exhausted_input(inner_level)
            .expect("inner body retires in LIFO order");
        assert_eq!(state.attempt_token_words(outer), Ok(&[word('o')][..]));
        assert!(state.attempt_token_words(inner).is_err());

        state
            .retire_exhausted_input(outer_level)
            .expect("outer body retires after its child");
        assert!(state.attempt.is_empty());
    });
}

#[test]
fn macro_retirement_preserves_a_live_scanner_builder() {
    crate::test_harness::with_universe(|universe| {
        let mut state = CommandState::default();
        let definition = universe
            .allocate_definition(&[], &[])
            .expect("macro definition");
        let name = universe.intern("builder").expect("macro name").symbol();
        let opening = state
            .macro_activation_opening_cursor()
            .expect("opening roots");
        let argument = state
            .attempt
            .arena_mut()
            .allocate_token_list([word('a')])
            .expect("macro argument");
        let mut arguments = MacroArgumentBuilder::default();
        arguments.complete(1, argument).expect("argument slot");
        let arguments = arguments
            .finish(state.attempt.arena_mut(), opening)
            .expect("argument record");
        let level = state.push_macro_activation(name, definition, arguments, OriginId::UNKNOWN, 0);

        let tokens = state
            .attempt
            .arena_mut()
            .allocate_token_buffer()
            .expect("live scanner builder");
        state.transient.builders.push(super::LiveTokenBuilder {
            identity: 7,
            tokens,
        });
        let discarded = state
            .attempt
            .arena_mut()
            .allocate_token_list([word('x')])
            .expect("retired scratch");

        state
            .retire_exhausted_input(level)
            .expect("macro body retires around live builder");
        assert!(state.attempt.arena().token_buffer(tokens).is_ok());
        assert!(state.attempt_token_words(discarded).is_err());
        assert!(state.attempt_token_words(argument).is_err());

        state.transient.builders.clear();
        state
            .reclaim_unreachable_attempt_suffix()
            .expect("builder retirement releases final attempt row");
        assert!(state.attempt.is_empty());
    });
}

#[test]
fn macro_retirement_preserves_completed_inner_arguments_before_activation() {
    crate::test_harness::with_universe(|universe| {
        let mut state = CommandState::default();
        let definition = universe
            .allocate_definition(&[], &[])
            .expect("macro definition");
        let name = universe.intern("pending").expect("macro name").symbol();

        let outer_opening = state
            .macro_activation_opening_cursor()
            .expect("outer opening roots");
        let outer_argument = state
            .attempt
            .arena_mut()
            .allocate_token_list([word('o')])
            .expect("outer argument");
        let mut outer_arguments = MacroArgumentBuilder::default();
        outer_arguments
            .complete(1, outer_argument)
            .expect("outer slot");
        let outer_arguments = outer_arguments
            .finish(state.attempt.arena_mut(), outer_opening)
            .expect("outer argument record");
        let outer_level =
            state.push_macro_activation(name, definition, outer_arguments, OriginId::UNKNOWN, 0);

        let inner_opening = state
            .macro_activation_opening_cursor()
            .expect("inner opening roots");
        let inner_argument = state
            .attempt
            .arena_mut()
            .allocate_token_list([word('i')])
            .expect("inner argument");
        let mut inner_arguments = MacroArgumentBuilder::default();
        inner_arguments
            .complete(1, inner_argument)
            .expect("inner slot");
        let inner_arguments = inner_arguments
            .finish(state.attempt.arena_mut(), inner_opening)
            .expect("inner argument record");
        state.pending_macro_arguments = Some(inner_arguments);

        state
            .retire_exhausted_input(outer_level)
            .expect("outer retirement while inner arguments await activation");
        assert_eq!(
            state.attempt_token_words(inner_argument),
            Ok(&[word('i')][..])
        );
        assert_eq!(
            state.attempt_token_words(outer_argument),
            Ok(&[word('o')][..])
        );

        let inner_arguments = state
            .pending_macro_arguments
            .take()
            .expect("pending arguments survive outer retirement");
        let inner_level =
            state.push_macro_activation(name, definition, inner_arguments, OriginId::UNKNOWN, 0);
        state
            .retire_exhausted_input(inner_level)
            .expect("inner retirement releases both physical prefixes");
        assert!(state.attempt.is_empty());
    });
}

#[cfg(feature = "profiling")]
#[test]
fn warmed_long_macro_operation_retires_each_argument_without_allocation() {
    crate::test_harness::with_universe(|universe| {
        let mut state = CommandState::default();
        let definition = universe
            .allocate_definition(&[], &[])
            .expect("macro definition");
        let name = universe.intern("long-op").expect("macro name").symbol();
        let run = |state: &mut CommandState<_>| {
            let opening = state
                .macro_activation_opening_cursor()
                .expect("opening roots");
            let argument = state
                .attempt
                .arena_mut()
                .allocate_token_list([word('x')])
                .expect("macro argument");
            let mut arguments = MacroArgumentBuilder::default();
            arguments.complete(1, argument).expect("argument slot");
            let arguments = arguments
                .finish(state.attempt.arena_mut(), opening)
                .expect("argument record");
            let level =
                state.push_macro_activation(name, definition, arguments, OriginId::UNKNOWN, 0);
            state
                .retire_exhausted_input(level)
                .expect("macro body retirement");
            assert!(state.attempt.is_empty());
        };

        for _ in 0..64 {
            run(&mut state);
        }
        let owners = [
            tex_state::measurement::HotCoreAllocationOwner::DeliveryAndScan,
            tex_state::measurement::HotCoreAllocationOwner::SemanticApply,
            tex_state::measurement::HotCoreAllocationOwner::EvidencePublication,
            tex_state::measurement::HotCoreAllocationOwner::InterpreterConstruction,
            tex_state::measurement::HotCoreAllocationOwner::InterpreterBorrow,
            tex_state::measurement::HotCoreAllocationOwner::ColdMaterialization,
            tex_state::measurement::HotCoreAllocationOwner::AttemptScratch,
            tex_state::measurement::HotCoreAllocationOwner::GenerationBoundary,
            tex_state::measurement::HotCoreAllocationOwner::ArenaGrowth,
        ];
        let before = owners.map(tex_state::measurement::hot_core_thread_allocation_measurement);
        {
            let _scope = tex_state::measurement::hot_core_allocation_scope(
                tex_state::measurement::HotCoreAllocationOwner::DeliveryAndScan,
            );
            for _ in 0..8_192 {
                run(&mut state);
            }
        }
        let after = owners.map(tex_state::measurement::hot_core_thread_allocation_measurement);
        let calls = after
            .iter()
            .zip(before.iter())
            .map(|(after, before)| after.calls - before.calls)
            .sum::<u64>();
        let bytes = after
            .iter()
            .zip(before.iter())
            .map(|(after, before)| after.requested_bytes - before.requested_bytes)
            .sum::<u64>();
        assert_eq!(calls, 0);
        assert_eq!(bytes, 0);
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
fn stale_resource_suspension_mark_is_typed_and_mutation_free() {
    crate::test_harness::with_universe(|universe| {
        let mut state = CommandState::default();
        let retained = state
            .attempt
            .arena_mut()
            .allocate_token_list([word('x')])
            .expect("retained attempt value");
        let live = state.begin_attempt_operation();
        state
            .attempt
            .arena_mut()
            .allocate_token_list([word('y')])
            .expect("discarded attempt value");
        let stale = state.begin_attempt_operation();
        state.discard_attempt_operation(live);

        let error = match state.suspend_attempt(
            universe,
            stale,
            crate::AttemptResumePoint::default(),
            "input request",
        ) {
            Ok(_) => panic!("truncated opening mark must be stale"),
            Err(error) => error,
        };
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
