use super::{UniverseError, with_universe};
use crate::env::AssignmentScope;
use crate::interner::InternerBudget;
use crate::meaning::{Meaning, MeaningWord, ResolvedMeaning};
use crate::node::Node;
use crate::node_arena::NodeArenaError;

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
            context.meaning(symbol.symbol()),
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

#[test]
fn retained_state_checkpoint_restores_dense_roots_before_arena_suffixes() {
    with_universe(budget(), |universe| {
        universe
            .assign_count(0, 10, AssignmentScope::Global)
            .expect("baseline count");
        let checkpoint = universe.state_checkpoint().expect("checkpoint");
        let rejected = universe.publish_page_nodes(&[Node::Penalty(99)]);
        universe
            .assign_count(0, 20, AssignmentScope::Global)
            .expect("candidate count");

        universe
            .restore_state_checkpoint(&checkpoint)
            .expect("restore checkpoint");

        assert_eq!(universe.command_context().unwrap().count(0).unwrap(), 10);
        assert_eq!(
            universe.page_node_list(rejected).unwrap_err(),
            NodeArenaError::InvalidList
        );
        assert_eq!(
            universe.retire(),
            Err(UniverseError::State(crate::StateError::GenerationInUse))
        );
        drop(checkpoint);
        universe.retire().expect("last coarse owner released");
    })
    .expect("universe allocation");
}

#[test]
fn malformed_aggregate_restore_does_not_touch_dense_state() {
    with_universe(budget(), |universe| {
        let before_page = universe.page_node_cursor();
        let _ = universe.publish_page_nodes(&[Node::Penalty(7)]);
        let malformed = universe.state_checkpoint().expect("future page cursor");
        universe
            .assign_count(0, 41, AssignmentScope::Global)
            .expect("candidate count");
        universe
            .truncate_page_nodes(before_page)
            .expect("discard page suffix before restore");

        assert_eq!(
            universe.restore_state_checkpoint(&malformed),
            Err(UniverseError::NodeArena(NodeArenaError::CursorBeyondEnd))
        );
        assert_eq!(
            universe.command_context().unwrap().count(0).unwrap(),
            41,
            "page-cursor rejection must precede dense-state mutation"
        );
    })
    .expect("universe allocation");
}
