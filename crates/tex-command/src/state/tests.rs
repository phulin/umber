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
            state.rollback_attempt_operation(operation),
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
fn committed_macro_scope_survives_until_a_later_lifo_retirement() {
    crate::test_harness::with_universe(|universe| {
        let mut state = CommandState::default();
        let definition = universe
            .allocate_definition(&[], &[])
            .expect("macro definition");
        let name = universe.intern("inner").expect("macro name").symbol();
        let operation = state.begin_attempt_operation();
        let scope = state.begin_attempt_macro_scope().expect("macro scope");
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
        let level =
            state.push_macro_activation(name, definition, arguments, OriginId::UNKNOWN, 0, scope);

        state
            .commit_attempt_operation(operation)
            .expect("operation commits around persistent macro scope");
        assert_eq!(state.attempt_token_words(retained), Ok(&[word('x')][..]));
        assert!(state.retained_attempt_operation().is_err());

        let retirement = state.begin_attempt_operation();
        state
            .retire_exhausted_input(level)
            .expect("empty macro body retires");
        state
            .commit_attempt_operation(retirement)
            .expect("retired suffix reclaims");
        assert!(state.attempt.is_empty());
        assert!(state.attempt_token_words(retained).is_err());
    });
}

#[test]
fn repeated_same_depth_macro_replacement_hands_operation_to_the_latest_owner() {
    crate::test_harness::with_universe(|universe| {
        let mut state = CommandState::default();
        let definition = universe
            .allocate_definition(&[], &[])
            .expect("macro definition");
        let name = universe.intern("replacement").expect("macro name").symbol();

        let opening = state.begin_attempt_operation();
        let outer_scope = state.begin_attempt_macro_scope().expect("outer scope");
        let outer_arguments = MacroArgumentBuilder::default()
            .finish(state.attempt.arena_mut())
            .expect("outer arguments");
        let outer = state.push_macro_activation(
            name,
            definition,
            outer_arguments,
            OriginId::UNKNOWN,
            0,
            outer_scope,
        );
        state
            .commit_attempt_operation(opening)
            .expect("outer activation commits");

        let replacement_operation = state.begin_attempt_operation();
        let mut replacement_scope = state
            .begin_attempt_macro_scope()
            .expect("replacement scope");
        state
            .retire_exhausted_input_around_local_child(outer, &mut replacement_scope)
            .expect("outer retires into the unpublished replacement owner");
        let retained = state
            .attempt
            .arena_mut()
            .allocate_token_list([word('r')])
            .expect("replacement argument");
        let mut arguments = MacroArgumentBuilder::default();
        arguments.complete(1, retained).expect("replacement slot");
        let arguments = arguments
            .finish(state.attempt.arena_mut())
            .expect("replacement arguments");
        let replacement = state.push_macro_activation(
            name,
            definition,
            arguments,
            OriginId::UNKNOWN,
            0,
            replacement_scope,
        );
        let mut final_scope = state
            .begin_attempt_macro_scope()
            .expect("second replacement scope");
        state
            .retire_exhausted_input_around_local_child(replacement, &mut final_scope)
            .expect("first replacement retires into its unpublished successor");
        let final_retained = state
            .attempt
            .arena_mut()
            .allocate_token_list([word('f')])
            .expect("final replacement argument");
        let mut final_arguments = MacroArgumentBuilder::default();
        final_arguments
            .complete(1, final_retained)
            .expect("final replacement slot");
        let final_arguments = final_arguments
            .finish(state.attempt.arena_mut())
            .expect("final replacement arguments");
        let final_replacement = state.push_macro_activation(
            name,
            definition,
            final_arguments,
            OriginId::UNKNOWN,
            0,
            final_scope,
        );
        state
            .commit_attempt_operation(replacement_operation)
            .expect("latest same-depth replacement owns the committed suffix");
        assert_eq!(
            state.attempt_token_words(final_retained),
            Ok(&[word('f')][..])
        );

        let retirement = state.begin_attempt_operation();
        state
            .retire_exhausted_input(final_replacement)
            .expect("latest replacement retires");
        state
            .commit_attempt_operation(retirement)
            .expect("replacement suffix closes");
        assert!(state.attempt.is_empty());
        assert!(state.attempt_token_words(retained).is_err());
        assert!(state.attempt_token_words(final_retained).is_err());
    });
}

#[test]
fn failed_unpublished_macro_child_returns_ownership_before_operation_rollback() {
    crate::test_harness::with_universe(|universe| {
        let mut state = CommandState::default();
        let definition = universe
            .allocate_definition(&[], &[])
            .expect("macro definition");
        let name = universe.intern("outer").expect("macro name").symbol();

        let opening = state.begin_attempt_operation();
        let outer_scope = state.begin_attempt_macro_scope().expect("outer scope");
        let retained = state
            .attempt
            .arena_mut()
            .allocate_token_list([word('o')])
            .expect("outer argument");
        let mut outer_arguments = MacroArgumentBuilder::default();
        outer_arguments.complete(1, retained).expect("outer slot");
        let outer_arguments = outer_arguments
            .finish(state.attempt.arena_mut())
            .expect("outer arguments");
        state.push_macro_activation(
            name,
            definition,
            outer_arguments,
            OriginId::UNKNOWN,
            0,
            outer_scope,
        );
        state
            .commit_attempt_operation(opening)
            .expect("outer activation commits");

        let rejected = state.begin_attempt_operation();
        let unpublished = state.begin_attempt_macro_scope().expect("local child");
        let scratch = state
            .attempt
            .arena_mut()
            .allocate_token_list([word('x')])
            .expect("rejected scratch");
        state
            .discard_attempt_scope_suffix(unpublished)
            .expect("failed local child returns the parent capability");
        state
            .rollback_attempt_operation(rejected)
            .expect("operation rollback remains exact");

        assert_eq!(state.attempt_token_words(retained), Ok(&[word('o')][..]));
        assert!(state.attempt_token_words(scratch).is_err());
    });
}

#[test]
fn nested_completed_scanners_defer_only_the_outer_owner_to_the_operation() {
    let mut state = CommandState::<()>::default();
    let operation = state.begin_attempt_operation();
    let parent_sink = state
        .attempt
        .arena_mut()
        .allocate_token_list([word('p')])
        .expect("parent-owned scanner sink");
    let parent = state
        .begin_attempt_scanner_scope()
        .expect("parent scanner scope");
    let child_sink = state
        .attempt
        .arena_mut()
        .allocate_token_list([word('c')])
        .expect("nested parent-owned scanner sink");
    let child = state
        .begin_attempt_scanner_scope()
        .expect("nested scanner scope");

    state
        .defer_attempt_scope_retirement(child)
        .expect("nested scanner closes to the synchronous parent");
    assert_eq!(state.attempt_token_words(parent_sink), Ok(&[word('p')][..]));
    assert_eq!(state.attempt_token_words(child_sink), Ok(&[word('c')][..]));
    state
        .defer_attempt_scope_retirement(parent)
        .expect("parent scanner closes to the operation");
    state
        .commit_attempt_operation(operation)
        .expect("operation closes after both synchronous scanners");
    assert!(state.attempt.is_empty());
}

#[test]
fn completed_scanner_hands_ownership_to_its_live_macro_child() {
    crate::test_harness::with_universe(|universe| {
        let mut state = CommandState::default();
        let definition = universe
            .allocate_definition(&[], &[])
            .expect("macro definition");
        let name = universe.intern("child").expect("macro name").symbol();
        let operation = state.begin_attempt_operation();
        let scanner = state.begin_attempt_scanner_scope().expect("scanner scope");
        let child_scope = state.begin_attempt_macro_scope().expect("macro child");
        let retained = state
            .attempt
            .arena_mut()
            .allocate_token_list([word('m')])
            .expect("macro argument");
        let mut arguments = MacroArgumentBuilder::default();
        arguments.complete(1, retained).expect("macro slot");
        let arguments = arguments
            .finish(state.attempt.arena_mut())
            .expect("macro arguments");
        let child = state.push_macro_activation(
            name,
            definition,
            arguments,
            OriginId::UNKNOWN,
            0,
            child_scope,
        );

        state
            .defer_attempt_scope_retirement(scanner)
            .expect("scanner moves its close-through into the exact live child");
        state
            .commit_attempt_operation(operation)
            .expect("operation commits around the child owner");
        assert_eq!(state.attempt_token_words(retained), Ok(&[word('m')][..]));

        let retirement = state.begin_attempt_operation();
        state.retire_exhausted_input(child).expect("child retires");
        state
            .commit_attempt_operation(retirement)
            .expect("child closes the scanner and operation chain");
        assert!(state.attempt.is_empty());
    });
}

#[test]
fn non_top_direct_macro_retirement_clears_the_consumed_child_link() {
    crate::test_harness::with_universe(|universe| {
        let mut state = CommandState::default();
        let definition = universe
            .allocate_definition(&[], &[])
            .expect("macro definition");
        let name = universe
            .intern("retired-parent")
            .expect("macro name")
            .symbol();

        let operation = state.begin_attempt_operation();
        let macro_scope = state.begin_attempt_macro_scope().expect("macro scope");
        let arguments = MacroArgumentBuilder::default()
            .finish(state.attempt.arena_mut())
            .expect("macro arguments");
        let macro_level = state.push_macro_activation(
            name,
            definition,
            arguments,
            OriginId::UNKNOWN,
            0,
            macro_scope,
        );
        let scanner_scope = state
            .begin_attempt_scanner_scope()
            .expect("nested scanner scope");
        state
            .retire_exhausted_input(macro_level)
            .expect("loaned macro parent retires while the scanner owns the chain");
        state
            .defer_attempt_scope_retirement(scanner_scope)
            .expect("scanner becomes the operation owner");
        state
            .commit_attempt_operation(operation)
            .expect("consumed direct-child link does not outlive the macro owner");
        assert!(state.attempt.is_empty());
    });
}

#[test]
fn stale_direct_macro_child_index_rejects_commit_before_mutation() {
    crate::test_harness::with_universe(|universe| {
        let mut state = CommandState::default();
        let definition = universe
            .allocate_definition(&[], &[])
            .expect("macro definition");
        let name = universe.intern("stale-child").expect("macro name").symbol();
        let operation = state.begin_attempt_operation();
        let scope = state.begin_attempt_macro_scope().expect("macro scope");
        let retained = state
            .attempt
            .arena_mut()
            .allocate_token_list([word('x')])
            .expect("macro argument");
        let mut arguments = MacroArgumentBuilder::default();
        arguments.complete(1, retained).expect("argument slot");
        let arguments = arguments
            .finish(state.attempt.arena_mut())
            .expect("arguments");
        state.push_macro_activation(name, definition, arguments, OriginId::UNKNOWN, 0, scope);
        let before = state.attempt.arena().mark();
        state
            .attempt
            .replace_operation_macro_child_index_for_test(u32::MAX);
        assert_eq!(
            state.commit_attempt_operation(operation),
            Err(crate::AttemptError::InvalidCoordinate)
        );
        assert_eq!(state.attempt.arena().mark(), before);
        assert_eq!(state.attempt_token_words(retained), Ok(&[word('x')][..]));
        state
            .rollback_attempt_operation(operation)
            .expect("failed preflight left operation rollbackable");
        assert!(state.attempt.is_empty());
    });
}

#[test]
fn nested_macro_child_keeps_the_direct_parent_link_on_the_outer_frame() {
    crate::test_harness::with_universe(|universe| {
        let mut state = CommandState::default();
        let definition = universe
            .allocate_definition(&[], &[])
            .expect("macro definition");
        let name = universe
            .intern("nested-child")
            .expect("macro name")
            .symbol();
        let operation = state.begin_attempt_operation();

        let outer_scope = state.begin_attempt_macro_scope().expect("outer scope");
        let outer_arguments = MacroArgumentBuilder::default()
            .finish(state.attempt.arena_mut())
            .expect("outer arguments");
        let outer = state.push_macro_activation(
            name,
            definition,
            outer_arguments,
            OriginId::UNKNOWN,
            0,
            outer_scope,
        );
        let inner_scope = state.begin_attempt_macro_scope().expect("inner scope");
        let inner_arguments = MacroArgumentBuilder::default()
            .finish(state.attempt.arena_mut())
            .expect("inner arguments");
        let inner = state.push_macro_activation(
            name,
            definition,
            inner_arguments,
            OriginId::UNKNOWN,
            0,
            inner_scope,
        );
        state
            .commit_attempt_operation(operation)
            .expect("operation hands ownership to the direct outer child");

        let inner_retirement = state.begin_attempt_operation();
        state.retire_exhausted_input(inner).expect("inner retires");
        state
            .commit_attempt_operation(inner_retirement)
            .expect("inner retirement closes to outer");
        let outer_retirement = state.begin_attempt_operation();
        state.retire_exhausted_input(outer).expect("outer retires");
        state
            .commit_attempt_operation(outer_retirement)
            .expect("outer retirement closes the operation chain");
        assert!(state.attempt.is_empty());
    });
}

#[test]
fn operation_rollback_discards_children_and_preserves_the_prior_macro_scope() {
    crate::test_harness::with_universe(|universe| {
        let mut state = CommandState::default();
        let definition = universe
            .allocate_definition(&[], &[])
            .expect("macro definition");
        let name = universe.intern("inner").expect("macro name").symbol();
        let activation = state.begin_attempt_operation();
        let scope = state.begin_attempt_macro_scope().expect("macro scope");
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
        let level =
            state.push_macro_activation(name, definition, arguments, OriginId::UNKNOWN, 0, scope);
        state
            .commit_attempt_operation(activation)
            .expect("commit macro");

        let rejected_operation = state.begin_attempt_operation();
        let rejected_scope = state.begin_attempt_macro_scope().expect("rejected child");
        let scratch = state
            .attempt
            .arena_mut()
            .allocate_token_list([word('z')])
            .expect("child scratch");

        state
            .discard_attempt_scope_suffix(rejected_scope)
            .expect("rejected child returns ownership to the live parent");
        state
            .rollback_attempt_operation(rejected_operation)
            .expect("rejected child rolls back");

        assert_eq!(state.attempt_token_words(retained), Ok(&[word('x')][..]));
        assert!(state.attempt_token_words(scratch).is_err());

        let retirement = state.begin_attempt_operation();
        state.retire_exhausted_input(level).expect("retire macro");
        state
            .commit_attempt_operation(retirement)
            .expect("commit retirement");
    });
}

#[test]
fn nested_scope_retirement_preserves_outer_parameter_replay() {
    crate::test_harness::with_universe(|universe| {
        let mut state = CommandState::default();
        let definition = universe
            .allocate_definition(&[], &[])
            .expect("macro definition");
        let name = universe.intern("nested").expect("macro name").symbol();

        let operation = state.begin_attempt_operation();
        let outer_scope = state.begin_attempt_macro_scope().expect("outer scope");
        let argument = state
            .attempt
            .arena_mut()
            .allocate_token_list([word('a')])
            .expect("outer argument");
        let mut arguments = MacroArgumentBuilder::default();
        arguments.complete(1, argument).expect("outer slot");
        let arguments = arguments
            .finish(state.attempt.arena_mut())
            .expect("outer argument record");
        let outer = state.push_macro_activation(
            name,
            definition,
            arguments,
            OriginId::UNKNOWN,
            0,
            outer_scope,
        );
        let replay = state
            .replay_out_parameter(outer, 1)
            .expect("parameter replay");
        let OutParameterReplay::Pushed(replay) = replay else {
            panic!("outer parameter must push a replay level");
        };

        let inner_scope = state.begin_attempt_macro_scope().expect("inner scope");
        let inner_arguments = MacroArgumentBuilder::default()
            .finish(state.attempt.arena_mut())
            .expect("empty inner argument record");
        let inner = state.push_macro_activation(
            name,
            definition,
            inner_arguments,
            OriginId::UNKNOWN,
            0,
            inner_scope,
        );
        state
            .retire_exhausted_input(inner)
            .expect("inner body retires around outer replay");
        assert_eq!(state.attempt_token_words(argument), Ok(&[word('a')][..]));

        state
            .retire_exhausted_input(replay)
            .expect("parameter replay retires in LIFO order");
        state
            .retire_exhausted_input(outer)
            .expect("outer body retires after replay");
        state
            .commit_attempt_operation(operation)
            .expect("commit nested retirements");
        assert!(state.attempt.is_empty());
    });
}
#[cfg(feature = "profiling")]
#[test]
fn warmed_scope_retirement_is_allocation_free_for_8192_activations() {
    crate::test_harness::with_universe(|universe| {
        let mut state = CommandState::default();
        let definition = universe
            .allocate_definition(&[], &[])
            .expect("macro definition");
        let name = universe.intern("long-op").expect("macro name").symbol();
        let run = |state: &mut CommandState<_>| {
            let activation = state.begin_attempt_operation();
            let scope = state.begin_attempt_macro_scope().expect("macro scope");
            let argument = state
                .attempt
                .arena_mut()
                .allocate_token_list([word('x')])
                .expect("macro argument");
            let mut arguments = MacroArgumentBuilder::default();
            arguments.complete(1, argument).expect("argument slot");
            let arguments = arguments
                .finish(state.attempt.arena_mut())
                .expect("argument record");
            let level = state.push_macro_activation(
                name,
                definition,
                arguments,
                OriginId::UNKNOWN,
                0,
                scope,
            );
            state
                .commit_attempt_operation(activation)
                .expect("commit activation");
            let retirement = state.begin_attempt_operation();
            state
                .retire_exhausted_input(level)
                .expect("macro body retirement");
            state
                .commit_attempt_operation(retirement)
                .expect("commit retirement");
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
        assert!(
            matches!(
                state.commit_attempt_operation(opening),
                Err(AttemptError::InvalidCoordinate | AttemptError::ForeignAttempt)
            ),
            "an operation moved into a continuation cannot commit from empty command state"
        );
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
        state
            .rollback_attempt_operation(restored_opening)
            .expect("resumed operation rolls back");
        assert_eq!(state.attempt_token_words(retained), Ok(&[word('a')][..]));
        assert!(state.attempt_token_words(rejected).is_err());
    });
}

#[test]
fn nested_owned_scopes_survive_resource_suspension_and_resume_once() {
    crate::test_harness::with_universe(|universe| {
        let mut state = CommandState::default();
        let operation = state.begin_attempt_operation();
        let scanner = state.begin_attempt_scanner_scope().expect("scanner scope");
        let macro_child = state.begin_attempt_macro_scope().expect("macro child");
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
                (scanner, macro_child),
            )
            .expect("nested scopes suspend with their arena owner");

        let (resumed, _, (scanner, macro_child)) = state
            .resume_attempt(universe, pending)
            .ok()
            .expect("nested scopes resume into the same state");
        assert_eq!(resumed, operation);
        assert_eq!(state.attempt_token_words(child_value), Ok(&[word('x')][..]));
        state
            .discard_attempt_scope_suffix(macro_child)
            .expect("top macro child retires");
        state
            .defer_attempt_scope_retirement(scanner)
            .expect("scanner defers until commit");
        state
            .commit_attempt_operation(operation)
            .expect("commit consumes each owner exactly once");
        assert!(state.attempt.is_empty());
        assert!(matches!(
            state.commit_attempt_operation(operation),
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
        state
            .rollback_attempt_operation(live)
            .expect("operation rolls back");

        let error = match state.suspend_attempt(
            universe,
            live,
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
