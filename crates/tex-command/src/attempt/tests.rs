use tex_state::glue::{GlueSpec, Order};
use tex_state::interner::InternerBudget;
use tex_state::scaled::Scaled;
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use super::{
    AttemptArena, AttemptError, AttemptEscapeRoots, AttemptResumePoint, CommandAttempt,
    PendingCommandAttempt,
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
        width: Scaled(width),
        stretch: Scaled::ZERO,
        stretch_order: Order::Normal,
        shrink: Scaled::ZERO,
        shrink_order: Order::Normal,
    }
}

#[test]
fn mark_truncates_every_suffix_without_inspecting_values() {
    tex_state::with_universe(InternerBudget::default(), |universe| {
        let mut attempt = AttemptArena::default();
        let retained = attempt.allocate_token_list([word('a')]).unwrap();
        let mark = attempt.mark();
        let rejected = attempt.allocate_token_list([word('b'), word('c')]).unwrap();
        let rejected_glue = attempt.allocate_glue(glue(17)).unwrap();
        let rejected_name = attempt.allocate_name("discarded").unwrap();

        attempt.truncate(mark).unwrap();

        assert_eq!(attempt.token_words(retained).unwrap(), &[word('a')]);
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
            .unwrap();
        assert_eq!(
            universe
                .command_context()
                .unwrap()
                .token_list(promoted.token_lists[0]),
            &[word('a').token_word()]
        );
    })
    .unwrap();
}

#[test]
fn foreign_marks_and_offsets_are_rejected() {
    tex_state::with_universe(InternerBudget::default(), |_universe| {
        let first = AttemptArena::<_>::default();
        let mark = first.mark();
        let mut second = AttemptArena::<_>::default();
        assert_eq!(second.truncate(mark), Err(AttemptError::ForeignAttempt));
    })
    .unwrap();
}

#[test]
fn promotion_follows_only_declared_roots_and_definition_children() {
    tex_state::with_universe(InternerBudget::default(), |universe| {
        let mut attempt = AttemptArena::default();
        let parameter = attempt.allocate_token_list([word('#')]).unwrap();
        let replacement = attempt.allocate_token_list([word('x')]).unwrap();
        let unrelated = attempt.allocate_token_list([word('z')]).unwrap();
        let definition = attempt.allocate_definition(parameter, replacement).unwrap();
        let promoted_glue = attempt.allocate_glue(glue(42)).unwrap();
        let unrelated_glue = attempt.allocate_glue(glue(99)).unwrap();

        let promoted = attempt
            .promote(
                universe,
                AttemptEscapeRoots {
                    token_lists: &[replacement],
                    glue: &[promoted_glue],
                    definitions: &[definition],
                },
            )
            .unwrap();

        assert_eq!(promoted.token_lists.len(), 1);
        assert_eq!(promoted.glue.len(), 1);
        assert_eq!(promoted.definitions.len(), 1);
        assert_eq!(
            universe.command_context().unwrap().glue(promoted.glue[0]),
            glue(42)
        );
        assert_eq!(
            universe
                .command_context()
                .unwrap()
                .definition(promoted.definitions[0])
                .replacement_text(),
            &[word('x').token_word()]
        );
        assert_eq!(
            universe.retire().unwrap().token_list_rows(),
            1,
            "the unrelated list and definition children were not independently promoted"
        );
        let _ = (unrelated, unrelated_glue);
    })
    .unwrap();
}

#[test]
fn nested_builders_keep_outer_and_inner_scratch_disjoint() {
    tex_state::with_universe(InternerBudget::default(), |_universe| {
        let mut attempt = AttemptArena::<_>::default();
        let outer = attempt.begin_token_list().unwrap();
        attempt.push_token(outer, word('a')).unwrap();
        let inner = attempt.begin_token_list().unwrap();
        attempt.push_token(inner, word('x')).unwrap();
        let inner = attempt.finish_token_list(inner).unwrap();
        attempt.push_token(outer, word('b')).unwrap();
        let outer = attempt.finish_token_list(outer).unwrap();

        assert_eq!(attempt.token_words(inner).unwrap(), &[word('x')]);
        assert_eq!(attempt.token_words(outer).unwrap(), &[word('a'), word('b')]);
    })
    .unwrap();
}

#[test]
fn truncated_row_cannot_alias_a_reallocated_coordinate() {
    tex_state::with_universe(InternerBudget::default(), |_universe| {
        let mut attempt = AttemptArena::<_>::default();
        let mark = attempt.mark();
        let stale = attempt.allocate_token_list([word('a')]).unwrap();
        attempt.truncate(mark).unwrap();
        let replacement = attempt.allocate_token_list([word('b')]).unwrap();

        assert_eq!(
            attempt.token_words(stale),
            Err(AttemptError::InvalidCoordinate)
        );
        assert_eq!(attempt.token_words(replacement).unwrap(), &[word('b')]);
    })
    .unwrap();
}

#[test]
fn pending_attempt_owns_generation_and_resumes_without_a_borrow() {
    tex_state::with_universe(InternerBudget::default(), |universe| {
        let generation = universe.generation_owner().unwrap();
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
        assert_eq!(
            universe.allocate_token_list(&[]),
            Err(tex_state::UniverseError::State(
                tex_state::StateError::GenerationInUse
            ))
        );

        let (attempt, resume, request) = pending.resume(universe).ok().unwrap();
        assert_eq!(resume.command, 7);
        assert_eq!(request, "font request");
        assert!(attempt.arena().mark().traced_words == 0);
        universe.allocate_token_list(&[]).unwrap();
    })
    .unwrap();
}
