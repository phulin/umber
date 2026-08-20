use super::{UniverseError, with_universe};
use crate::env::AssignmentScope;
use crate::interner::InternerBudget;
use crate::meaning::{Meaning, MeaningWord, ResolvedMeaning};

fn budget() -> InternerBudget {
    InternerBudget::new(32, 32, 1024).expect("budget")
}

#[test]
fn command_episode_admits_session_and_generation_once() {
    with_universe(budget(), |universe| {
        let symbol = universe.intern("alpha").expect("intern");
        universe
            .assign_meaning(
                symbol,
                MeaningWord::from_static(Meaning::Relax),
                AssignmentScope::Global,
            )
            .expect("assign");

        let context = universe.command_context().expect("admit episode");
        assert_eq!(context.resolve_symbol(symbol), Ok("alpha"));
        assert_eq!(
            context.meaning(symbol).expect("meaning"),
            ResolvedMeaning::Static(Meaning::Relax)
        );
    })
    .expect("universe allocation");
}

#[test]
fn rollback_never_recycles_an_interned_symbol() {
    with_universe(budget(), |universe| {
        let first = universe.intern("first").expect("intern first");
        let cursor = universe.journal_cursor().expect("cursor");
        let second = universe.intern("second").expect("intern second");
        universe.restore_state(cursor).expect("state rollback");

        assert_eq!(universe.resolve_symbol(first), Ok("first"));
        assert_eq!(universe.resolve_symbol(second), Ok("second"));
        assert_eq!(universe.intern("second"), Ok(second));
    })
    .expect("universe allocation");
}

#[test]
fn whole_session_retirement_rejects_future_admission() {
    with_universe(budget(), |universe| {
        universe.intern("retained").expect("intern");
        let retired = universe.retire().expect("retire");
        assert_eq!(retired.interner_usage().control_sequence_names(), 1);
        assert!(universe.is_retired());
        assert_eq!(
            universe.command_context().err(),
            Some(UniverseError::Retired)
        );
        assert_eq!(universe.intern("late"), Err(UniverseError::Retired));
    })
    .expect("universe allocation");
}

#[test]
fn foreign_session_symbols_are_rejected_before_dense_access() {
    let mut foreign = None;
    with_universe(budget(), |universe| {
        foreign = Some(universe.intern("foreign").expect("intern"));
    })
    .expect("first universe");

    with_universe(budget(), |universe| {
        let local = universe.intern("local").expect("intern local");
        let context = universe.command_context().expect("context");
        assert_eq!(context.resolve_symbol(local), Ok("local"));
        assert!(
            context
                .resolve_symbol(foreign.expect("foreign id"))
                .is_err()
        );
    })
    .expect("second universe");
}
