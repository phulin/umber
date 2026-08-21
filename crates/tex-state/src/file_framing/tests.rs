//! Direct tests for tex.web §§537/362/1335's file bracketing.

use super::*;
use crate::interner::InternerBudget;
use crate::world::{EffectRecord, PrintSink};

fn with_test_universe<R>(
    use_universe: impl for<'id> FnOnce(&mut crate::Universe<crate::GenerationBrand<'id>>) -> R,
) -> R {
    let budget = InternerBudget::new(16, 16, 256).expect("budget");
    crate::with_universe(budget, use_universe).expect("fresh universe")
}

fn channel_text<G>(
    universe: &crate::Universe<G>,
    matches_sink: impl Fn(PrintSink) -> bool,
) -> String {
    universe
        .world()
        .effect_records()
        .iter()
        .filter_map(|record| match record {
            EffectRecord::StreamWrite { sink, text } if matches_sink(*sink) => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn terminal_text<G>(universe: &crate::Universe<G>) -> String {
    channel_text(universe, |sink| {
        matches!(sink, PrintSink::Terminal | PrintSink::TerminalAndLog)
    })
}

fn log_text<G>(universe: &crate::Universe<G>) -> String {
    channel_text(universe, |sink| {
        matches!(sink, PrintSink::Log | PrintSink::TerminalAndLog)
    })
}

#[test]
fn open_and_close_print_exact_brackets_and_update_the_count() {
    with_test_universe(|universe| {
        print_file_open(universe, "a.tex");
        assert_eq!(terminal_text(universe), "(a.tex");
        assert_eq!(universe.world().file_framing().open_parens(), 1);

        print_file_close(universe);
        assert_eq!(terminal_text(universe), "(a.tex)");
        assert_eq!(universe.world().file_framing().open_parens(), 0);
    });
}

#[test]
fn adjacent_file_names_are_separated_and_long_names_wrap() {
    with_test_universe(|universe| {
        print_file_open(universe, "a");
        print_file_close(universe);
        print_file_open(universe, "b");
        assert_eq!(terminal_text(universe), "(a) (b");
    });

    with_test_universe(|universe| {
        print_file_open(universe, &"x".repeat(80));
        assert_eq!(
            terminal_text(universe),
            format!("\n({}\n{}", "x".repeat(78), "x".repeat(2))
        );
    });
}

#[test]
fn final_cleanup_closes_every_open_file() {
    with_test_universe(|universe| {
        print_file_open(universe, "a");
        print_file_open(universe, "b");
        let opened = terminal_text(universe);
        print_remaining_file_closes(universe);

        assert_eq!(terminal_text(universe).strip_prefix(&opened), Some(" ) )"));
        assert_eq!(universe.world().file_framing().open_parens(), 0);
    });
}

#[test]
fn startup_open_is_terminal_only_but_cleanup_is_logged() {
    with_test_universe(|universe| {
        print_startup_file_open(universe, "./trip.tex");
        print_remaining_file_closes(universe);

        assert_eq!(terminal_text(universe), "(./trip.tex )");
        assert_eq!(log_text(universe), " )");
    });
}

#[test]
fn world_checkpoint_restores_file_framing_and_print_effects_together() {
    with_test_universe(|universe| {
        let checkpoint = universe.world().snapshot();
        print_file_open(universe, "a.tex");
        assert_eq!(universe.world().file_framing().open_parens(), 1);

        universe.world_mut().rollback(&checkpoint);

        assert_eq!(universe.world().file_framing().open_parens(), 0);
        assert!(terminal_text(universe).is_empty());
    });
}
