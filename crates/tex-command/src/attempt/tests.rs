use tex_state::glue::{GlueSpec, Order};
use tex_state::interner::InternerBudget;
use tex_state::scaled::Scaled;
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use super::{
    AttemptArena, AttemptError, AttemptEscapeRoots, AttemptResumePoint, AttemptTokenStorage,
    CommandAttempt, PendingCommandAttempt,
};

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

fn budget() -> InternerBudget {
    InternerBudget::new(64, 64, 4096).expect("test budget")
}

#[test]
fn mark_truncates_every_suffix_without_inspecting_values() {
    tex_state::with_universe(budget(), |universe| {
        let mut attempt = AttemptArena::default();
        let retained = attempt
            .allocate_token_list([word('a')])
            .expect("test fixture is valid");
        let mark = attempt.mark();
        let rejected = attempt
            .allocate_token_list([word('b'), word('c')])
            .expect("test fixture is valid");
        let rejected_glue = attempt
            .allocate_glue(glue(17))
            .expect("test fixture is valid");
        let rejected_name = attempt
            .allocate_name("discarded")
            .expect("test fixture is valid");

        attempt.truncate(mark).expect("test fixture is valid");

        assert_eq!(
            attempt
                .token_words(retained)
                .expect("test fixture is valid"),
            &[word('a')]
        );
        assert_eq!(
            attempt.token_words(rejected),
            Err(AttemptError::InvalidCoordinate)
        );
        assert_eq!(
            attempt.glue(rejected_glue),
            Err(AttemptError::InvalidCoordinate)
        );
        assert_eq!(
            attempt.name(rejected_name),
            Err(AttemptError::InvalidCoordinate)
        );
        let promoted = attempt
            .promote(
                universe,
                AttemptEscapeRoots {
                    token_lists: &[retained],
                    ..AttemptEscapeRoots::default()
                },
            )
            .expect("test fixture is valid");
        assert_eq!(
            universe
                .command_context()
                .expect("test fixture is valid")
                .token_list(promoted.token_lists[0]),
            &[word('a').token_word()]
        );
    })
    .expect("test fixture is valid");
}

#[test]
fn foreign_marks_and_offsets_are_rejected() {
    tex_state::with_universe(budget(), |_universe| {
        let first = AttemptArena::<()>::default();
        let mark = first.mark();
        let mut second = AttemptArena::<()>::default();
        assert_eq!(second.truncate(mark), Err(AttemptError::ForeignAttempt));
    })
    .expect("test fixture is valid");
}

#[test]
fn promotion_follows_only_declared_roots_and_definition_children() {
    tex_state::with_universe(budget(), |universe| {
        let mut attempt = AttemptArena::default();
        let parameter = attempt
            .allocate_token_list([word('#')])
            .expect("test fixture is valid");
        let replacement = attempt
            .allocate_token_list([word('x')])
            .expect("test fixture is valid");
        let unrelated = attempt
            .allocate_token_list([word('z')])
            .expect("test fixture is valid");
        let definition = attempt
            .allocate_definition(parameter, replacement)
            .expect("test fixture is valid");
        let promoted_glue = attempt
            .allocate_glue(glue(42))
            .expect("test fixture is valid");
        let unrelated_glue = attempt
            .allocate_glue(glue(99))
            .expect("test fixture is valid");

        let promoted = attempt
            .promote(
                universe,
                AttemptEscapeRoots {
                    token_lists: &[replacement],
                    glue: &[promoted_glue],
                    definitions: &[definition],
                    ..AttemptEscapeRoots::default()
                },
            )
            .expect("test fixture is valid");

        assert_eq!(promoted.token_lists.len(), 1);
        assert_eq!(promoted.glue.len(), 1);
        assert_eq!(promoted.definitions.len(), 1);
        assert_eq!(
            universe
                .command_context()
                .expect("test fixture is valid")
                .glue(promoted.glue[0]),
            glue(42)
        );
        assert_eq!(
            universe
                .command_context()
                .expect("test fixture is valid")
                .definition(promoted.definitions[0])
                .replacement_text(),
            &[word('x').token_word()]
        );
        assert_eq!(
            universe
                .retire()
                .expect("test fixture is valid")
                .token_list_rows(),
            1,
            "the unrelated list and definition children were not independently promoted"
        );
        let _ = (unrelated, unrelated_glue);
    })
    .expect("test fixture is valid");
}

#[test]
fn promotion_copies_only_declared_provenance_roots() {
    tex_state::with_universe(budget(), |universe| {
        let mut attempt = AttemptArena::default();
        let retained = attempt
            .allocate_provenance(tex_state::provenance::OriginRecord::UnknownBootstrap)
            .expect("test fixture is valid");
        let discarded = attempt
            .allocate_provenance(tex_state::provenance::OriginRecord::UnknownBootstrap)
            .expect("test fixture is valid");

        let promoted = attempt
            .promote(
                universe,
                AttemptEscapeRoots {
                    provenance: &[retained],
                    ..AttemptEscapeRoots::default()
                },
            )
            .expect("test fixture is valid");

        assert_eq!(promoted.provenance.len(), 1);
        assert_eq!(
            universe
                .command_context()
                .expect("test fixture is valid")
                .provenance(promoted.provenance[0]),
            tex_state::provenance::OriginRecord::UnknownBootstrap
        );
        let _ = discarded;
    })
    .expect("test fixture is valid");
}

#[test]
fn nested_builders_keep_outer_and_inner_scratch_disjoint() {
    tex_state::with_universe(budget(), |_universe| {
        let mut attempt = AttemptArena::<()>::default();
        let outer = attempt.begin_token_list().expect("test fixture is valid");
        attempt
            .push_token(outer, word('a'))
            .expect("test fixture is valid");
        let inner = attempt.begin_token_list().expect("test fixture is valid");
        attempt
            .push_token(inner, word('x'))
            .expect("test fixture is valid");
        let inner = attempt
            .finish_token_list(inner)
            .expect("test fixture is valid");
        attempt
            .push_token(outer, word('b'))
            .expect("test fixture is valid");
        let outer = attempt
            .finish_token_list(outer)
            .expect("test fixture is valid");

        assert_eq!(
            attempt.token_words(inner).expect("test fixture is valid"),
            &[word('x')]
        );
        assert_eq!(
            attempt.token_words(outer).expect("test fixture is valid"),
            &[word('a'), word('b')]
        );
    })
    .expect("test fixture is valid");
}

#[test]
fn mutable_scanner_buffers_are_attempt_owned_and_mark_bounded() {
    tex_state::with_universe(budget(), |_universe| {
        let mut attempt = AttemptArena::<()>::default();
        let mark = attempt.mark();
        let buffer = attempt
            .allocate_token_buffer()
            .expect("test fixture is valid");
        attempt
            .push_buffer_token(buffer, word('a'))
            .expect("test fixture is valid");
        attempt
            .push_buffer_token(buffer, word('b'))
            .expect("test fixture is valid");
        assert_eq!(
            attempt.token_buffer(buffer).expect("test fixture is valid"),
            &[word('a'), word('b')]
        );
        let buffer_storage = attempt.token_buffers[buffer.index()].value.words.as_ptr();
        let frozen = attempt
            .finish_token_buffer(buffer)
            .expect("test fixture is valid");
        assert_eq!(
            attempt.token_words(frozen).expect("test fixture is valid"),
            &[word('a'), word('b')]
        );
        let AttemptTokenStorage::Buffer(frozen_buffer) = attempt.token_lists[frozen.index()].value
        else {
            panic!("finished scanner result addresses its parent sink")
        };
        assert_eq!(frozen_buffer, buffer);
        assert_eq!(
            attempt.token_words(frozen).expect("frozen words").as_ptr(),
            buffer_storage
        );

        attempt.truncate(mark).expect("test fixture is valid");
        let recycled = attempt
            .allocate_token_buffer()
            .expect("test fixture is valid");
        assert_eq!(
            attempt.token_buffers[recycled.index()].value.words.as_ptr(),
            buffer_storage,
            "retiring the scanner result returns its backing to the attempt pool"
        );
        assert_eq!(
            attempt.token_buffer(buffer),
            Err(AttemptError::InvalidCoordinate)
        );
        assert_eq!(attempt.token_buffer(recycled), Ok(&[][..]));
        assert_eq!(
            attempt.token_words(frozen),
            Err(AttemptError::InvalidCoordinate)
        );
        attempt.truncate(mark).expect("recycled buffer retires");
        assert_eq!(
            attempt.token_buffer(recycled),
            Err(AttemptError::InvalidCoordinate)
        );
    })
    .expect("test fixture is valid");
}

#[cfg(feature = "profiling")]
#[test]
fn warmed_parent_owned_scanner_results_allocate_zero_heap() {
    let mut attempt = AttemptArena::<()>::default();
    let run = |attempt: &mut AttemptArena<()>| {
        let mark = attempt.mark();
        let buffer = attempt.allocate_token_buffer().expect("scanner buffer");
        attempt
            .push_buffer_token(buffer, word('x'))
            .expect("scanner word");
        let result = attempt.finish_token_buffer(buffer).expect("scanner result");
        assert_eq!(attempt.token_words(result), Ok(&[word('x')][..]));
        attempt.truncate(mark).expect("scanner scope retires");
    };
    for _ in 0..64 {
        run(&mut attempt);
    }
    let owner = tex_state::measurement::HotCoreAllocationOwner::AttemptScratch;
    let before = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
    for _ in 0..8_192 {
        run(&mut attempt);
    }
    let after = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
    assert_eq!(after.calls - before.calls, 0);
    assert_eq!(after.requested_bytes - before.requested_bytes, 0);
}

#[test]
fn argument_records_reject_more_than_texs_nine_slots() {
    tex_state::with_universe(budget(), |_universe| {
        let mut attempt = AttemptArena::<()>::default();
        let empty = attempt
            .allocate_token_list([])
            .expect("test fixture is valid");
        assert_eq!(
            attempt.allocate_arguments(&[empty; 10]),
            Err(AttemptError::InvalidCoordinate)
        );
    })
    .expect("test fixture is valid");
}

#[test]
fn attempt_local_provenance_stays_aligned_through_nested_builders() {
    tex_state::with_universe(budget(), |_universe| {
        let mut attempt = AttemptArena::<()>::default();
        let origin = attempt
            .allocate_provenance(tex_state::provenance::OriginRecord::UnknownBootstrap)
            .expect("test fixture is valid");
        let outer = attempt.begin_token_list().expect("test fixture is valid");
        attempt
            .push_token_with_local_origin(outer, word('a').token_word(), origin)
            .expect("test fixture is valid");
        let inner = attempt.begin_token_list().expect("test fixture is valid");
        attempt
            .push_token(inner, word('x'))
            .expect("test fixture is valid");
        let inner = attempt
            .finish_token_list(inner)
            .expect("test fixture is valid");
        let outer = attempt
            .finish_token_list(outer)
            .expect("test fixture is valid");

        assert_eq!(
            attempt
                .token_origin(outer, 0)
                .expect("test fixture is valid"),
            super::AttemptOrigin::Local(origin)
        );
        assert_eq!(
            attempt
                .token_origin(inner, 0)
                .expect("test fixture is valid"),
            super::AttemptOrigin::Admitted(OriginId::UNKNOWN)
        );
    })
    .expect("test fixture is valid");
}

#[test]
fn truncated_row_cannot_alias_a_reallocated_coordinate() {
    tex_state::with_universe(budget(), |_universe| {
        let mut attempt = AttemptArena::<()>::default();
        let mark = attempt.mark();
        let stale = attempt
            .allocate_token_list([word('a')])
            .expect("test fixture is valid");
        attempt.truncate(mark).expect("test fixture is valid");
        let replacement = attempt
            .allocate_token_list([word('b')])
            .expect("test fixture is valid");

        assert_eq!(
            attempt.token_words(stale),
            Err(AttemptError::InvalidCoordinate)
        );
        assert_eq!(
            attempt
                .token_words(replacement)
                .expect("test fixture is valid"),
            &[word('b')]
        );
    })
    .expect("test fixture is valid");
}

#[test]
fn macro_arguments_are_attempt_local_ranges_and_rollback_with_their_mark() {
    tex_state::with_universe(budget(), |_universe| {
        let mut attempt = AttemptArena::<()>::default();
        let first = attempt
            .allocate_token_list([word('a')])
            .expect("test fixture is valid");
        let second = attempt
            .allocate_token_list([word('b')])
            .expect("test fixture is valid");
        let mark = attempt.mark();
        let arguments = attempt
            .allocate_arguments(&[first, second])
            .expect("test fixture is valid");
        assert_eq!(
            attempt.arguments(arguments).expect("test fixture is valid"),
            &[first, second]
        );

        attempt.truncate(mark).expect("test fixture is valid");
        assert_eq!(
            attempt.arguments(arguments),
            Err(AttemptError::InvalidCoordinate)
        );
        assert_eq!(
            attempt.token_words(first).expect("test fixture is valid"),
            &[word('a')]
        );
    })
    .expect("test fixture is valid");
}

#[test]
fn pending_attempt_owns_generation_and_resumes_without_a_borrow() {
    tex_state::with_universe(budget(), |universe| {
        let generation = universe.generation_owner().expect("test fixture is valid");
        let pending = PendingCommandAttempt::new(
            CommandAttempt::default(),
            generation,
            AttemptResumePoint {
                command: 7,
                scanner: 11,
                expansion: 13,
                subordinate: 17,
            },
            "font request",
        );
        let allocated_while_pinned = universe
            .allocate_token_list(&[])
            .expect("a coarse owner pins retirement, not append-only allocation");
        assert!(
            universe
                .command_context()
                .expect("context")
                .token_list(allocated_while_pinned)
                .is_empty()
        );
        assert_eq!(
            universe.retire(),
            Err(tex_state::UniverseError::State(
                tex_state::StateError::GenerationInUse
            ))
        );

        let (attempt, opening, resume, request) = pending
            .resume(universe)
            .ok()
            .expect("test fixture is valid");
        assert!(
            attempt
                .arena()
                .validate_mark(opening.attempt_mark())
                .is_ok()
        );
        assert!(
            attempt
                .arena()
                .validate_scope_coordinate(opening.operation_scope())
                .is_ok()
        );
        assert_eq!(resume.command, 7);
        assert_eq!(request, "font request");
        assert!(attempt.arena().mark().traced_words == 0);
        universe
            .allocate_token_list(&[])
            .expect("test fixture is valid");
    })
    .expect("test fixture is valid");
}

#[test]
fn pending_owner_rejects_retirement_without_partially_retiring_universe() {
    tex_state::with_universe(budget(), |universe| {
        let owner = universe.generation_owner().expect("test fixture is valid");
        assert_eq!(
            universe.retire(),
            Err(tex_state::UniverseError::State(
                tex_state::StateError::GenerationInUse
            ))
        );
        assert!(!universe.is_retired());
        universe
            .intern("still-live")
            .expect("test fixture is valid");
        drop(owner);
        universe.retire().expect("test fixture is valid");
    })
    .expect("test fixture is valid");
}

#[test]
fn owned_scopes_close_exact_lifo_and_reject_stale_coordinates() {
    let mut attempt = AttemptArena::<()>::default();
    let retained = attempt
        .allocate_token_list([word('p')])
        .expect("parent value");
    let parent = attempt.begin_owned_scope().expect("parent scope");
    let parent_value = attempt
        .allocate_token_list([word('a')])
        .expect("parent-scope value");
    let child = attempt.begin_owned_scope().expect("child scope");
    let child_value = attempt
        .allocate_token_list([word('b')])
        .expect("child-scope value");

    assert_eq!(
        attempt.close_owned_scope(parent),
        Err(AttemptError::InvalidCoordinate)
    );
    assert_eq!(attempt.token_words(child_value), Ok(&[word('b')][..]));
    attempt
        .close_owned_scope(child)
        .expect("child closes first");
    assert_eq!(
        attempt.token_words(child_value),
        Err(AttemptError::InvalidCoordinate)
    );
    assert_eq!(attempt.token_words(parent_value), Ok(&[word('a')][..]));
    attempt
        .close_owned_scope(parent)
        .expect("parent closes second");
    assert_eq!(
        attempt.close_owned_scope(parent),
        Err(AttemptError::InvalidCoordinate)
    );
    assert_eq!(attempt.token_words(retained), Ok(&[word('p')][..]));
}

#[test]
fn lexical_scope_truncates_its_branded_child_id() {
    let mut attempt = AttemptArena::<()>::default();
    attempt
        .with_child_scope(|scope| {
            let child = scope
                .allocate_token_list([word('x')])
                .expect("child allocation");
            assert_eq!(scope.token_words(&child), Ok(&[word('x')][..]));
        })
        .expect("lexical scope");
    assert!(attempt.mark().is_empty());
}

#[test]
fn long_parent_scope_reclaims_8192_retired_children_at_constant_depth() {
    let mut attempt = AttemptArena::<()>::default();
    let operation = attempt.begin_owned_scope().expect("operation scope");
    let scanner = attempt.begin_owned_scope().expect("scanner scope");
    let output = attempt
        .allocate_token_buffer()
        .expect("parent-owned scanner sink");

    for _ in 0..8_192 {
        let child = attempt.begin_owned_scope().expect("macro child");
        let retired = attempt
            .allocate_token_list([word('x')])
            .expect("child scratch");
        attempt
            .retire_owned_scope(child, operation)
            .expect("top child retires immediately");
        assert_eq!(attempt.scopes.len(), 2);
        assert!(attempt.token_buffer(output).is_ok());
        assert_eq!(
            attempt.token_words(retired),
            Err(AttemptError::InvalidCoordinate)
        );
    }

    attempt
        .defer_owned_scope_retirement(scanner, operation)
        .expect("scanner output stays live until operation commit");
    assert!(attempt.token_buffer(output).is_ok());
    attempt
        .commit_owned_operation(operation)
        .expect("operation consumes scanner then itself");
    assert!(attempt.mark().is_empty());
}

#[test]
fn deferred_scanner_result_survives_a_younger_immediate_retirement() {
    let mut attempt = AttemptArena::<()>::default();
    let operation = attempt.begin_owned_scope().expect("operation scope");
    let scanner = attempt.begin_owned_scope().expect("scanner scope");
    let result = attempt
        .allocate_token_list([word('s')])
        .expect("scanner result");
    attempt
        .defer_owned_scope_retirement(scanner, operation)
        .expect("scanner defers until its result is consumed");

    let macro_child = attempt.begin_owned_scope().expect("macro child");
    let scratch = attempt
        .allocate_token_list([word('x')])
        .expect("macro scratch");
    attempt
        .retire_owned_scope(macro_child, operation)
        .expect("younger macro retires immediately");

    assert_eq!(attempt.token_words(result), Ok(&[word('s')][..]));
    assert_eq!(
        attempt.token_words(scratch),
        Err(AttemptError::InvalidCoordinate)
    );
    assert_eq!(attempt.scopes.len(), 2);
    attempt
        .commit_owned_operation(operation)
        .expect("commit releases scanner result and operation");
    assert!(attempt.mark().is_empty());
}

#[test]
fn preallocated_scanner_sink_survives_a_younger_operation_rollback() {
    let mut attempt = AttemptArena::<()>::default();
    let opening_operation = attempt.begin_owned_scope().expect("opening operation");
    let scanner = attempt.begin_owned_scope().expect("scanner scope");
    let output = attempt
        .allocate_token_buffer()
        .expect("scanner reserves its parent sink");
    attempt
        .commit_owned_operation(opening_operation)
        .expect("scanner keeps the opening operation below it");

    let rejected_retry = attempt.begin_owned_scope().expect("rejected retry");
    let rejected = attempt
        .allocate_token_list([word('x')])
        .expect("retry-local scratch");
    attempt
        .rollback_owned_operation(rejected_retry)
        .expect("retry suffix rolls back");
    assert_eq!(attempt.token_buffer(output), Ok(&[][..]));
    assert_eq!(
        attempt.token_words(rejected),
        Err(AttemptError::InvalidCoordinate)
    );

    let completed_retry = attempt.begin_owned_scope().expect("completed retry");
    attempt
        .push_buffer_token(output, word('r'))
        .expect("retry writes through the scanner-owned sink");
    let result = attempt
        .finish_token_buffer(output)
        .expect("parent sink finalizes after retry");
    attempt
        .defer_owned_scope_retirement(scanner, completed_retry)
        .expect("result survives through retry commit");
    assert_eq!(attempt.token_words(result), Ok(&[word('r')][..]));
    attempt
        .commit_owned_operation(completed_retry)
        .expect("completed retry closes the whole retired suffix");
    assert!(attempt.mark().is_empty());
}
