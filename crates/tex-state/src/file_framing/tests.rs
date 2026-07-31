//! Direct tests for tex.web §537/§362/§1335's `(name`/`)` bracketing and the
//! §54 `open_parens` count that ties the three together.

use super::*;

use crate::world::{EffectRecord, PrintSink};

fn channel_text(universe: &Universe, matches_sink: impl Fn(PrintSink) -> bool) -> String {
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

fn terminal_text(universe: &Universe) -> String {
    channel_text(universe, |sink| {
        matches!(sink, PrintSink::Terminal | PrintSink::TerminalAndLog)
    })
}

fn log_text(universe: &Universe) -> String {
    channel_text(universe, |sink| {
        matches!(sink, PrintSink::Log | PrintSink::TerminalAndLog)
    })
}

fn open_parens(universe: &Universe) -> u32 {
    universe.world().file_framing().open_parens()
}

#[test]
fn open_paren_prints_a_bare_name_when_the_line_is_empty() {
    let mut universe = Universe::new();

    print_file_open(&mut universe, "a.tex");

    assert_eq!(terminal_text(&universe), "(a.tex");
    assert_eq!(open_parens(&universe), 1);
}

#[test]
fn open_paren_prints_a_leading_space_when_the_line_already_has_content() {
    let mut universe = Universe::new();

    print_file_open(&mut universe, "a");
    print_file_close(&mut universe);
    print_file_open(&mut universe, "b");

    assert_eq!(terminal_text(&universe), "(a) (b");
    assert_eq!(open_parens(&universe), 1);
}

#[test]
fn open_paren_breaks_the_line_when_the_name_would_overflow_max_print_line() {
    let mut universe = Universe::new();
    let long_name = "x".repeat(80);

    print_file_open(&mut universe, &long_name);

    // §537 breaks *before* the name because it cannot fit on the rest of the
    // line, and §58 then breaks again the instant `(` plus the name reaches
    // `max_print_line`: 78 of the 80 characters share the line with the open
    // paren and the last two start the next.
    assert_eq!(
        terminal_text(&universe),
        format!("\n({}\n{}", "x".repeat(78), "x".repeat(2))
    );
}

#[test]
fn close_paren_prints_a_bare_close_and_decrements_open_parens() {
    let mut universe = Universe::new();

    print_file_open(&mut universe, "a.tex");
    print_file_close(&mut universe);

    assert_eq!(terminal_text(&universe), "(a.tex)");
    assert_eq!(open_parens(&universe), 0);
}

#[test]
fn remaining_closes_print_a_leading_space_before_each_close() {
    // §1335's `final_cleanup`: a file still open at `\end` closes with
    // `print("␣)")`, once per still-open paren.
    let mut universe = Universe::new();
    print_file_open(&mut universe, "a");
    print_file_open(&mut universe, "b");
    let opened = terminal_text(&universe);

    print_remaining_file_closes(&mut universe);

    assert_eq!(terminal_text(&universe).strip_prefix(&opened), Some(" ) )"));
    assert_eq!(open_parens(&universe), 0);
}

#[test]
fn remaining_closes_are_a_no_op_when_nothing_is_open() {
    let mut universe = Universe::new();

    print_remaining_file_closes(&mut universe);

    assert!(terminal_text(&universe).is_empty());
}

#[test]
fn startup_open_is_terminal_only_but_still_counted_by_final_cleanup() {
    // TeX82 §§537/1335: a retained driver opens the root before the log
    // exists, so the opening reaches the terminal alone -- but `open_parens`
    // must still make an abandoned root close.
    let mut universe = Universe::new();

    print_startup_file_open(&mut universe, "./trip.tex");
    print_remaining_file_closes(&mut universe);

    assert_eq!(terminal_text(&universe), "(./trip.tex )");
    assert_eq!(log_text(&universe), " )");
    assert_eq!(open_parens(&universe), 0);
}

#[test]
fn open_parens_rolls_back_with_the_universe_snapshot() {
    // §54's count is print-adjacent state precisely so that a step which
    // prints `(name` and is then abandoned takes the count back with the
    // print, instead of leaving §1335 to close a paren nobody opened.
    let mut universe = Universe::new();
    let snapshot = universe.snapshot();

    print_file_open(&mut universe, "a.tex");
    assert_eq!(open_parens(&universe), 1);

    universe.rollback(&snapshot);

    assert_eq!(open_parens(&universe), 0);
    assert!(terminal_text(&universe).is_empty());
}
