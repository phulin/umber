use super::*;

use tex_oracle::{
    CanonicalCommand, CanonicalValue, CommandDelivery, CommandEvent, Event, SourceLocation,
};

use crate::ObservedEvent;
use crate::compare::{MismatchSides, Repair, StreamMismatch, classify_mismatch_kind};

const IDENTITY: &str = "tex82/case manifest=abc";

fn letter(character: u32, line: u32) -> Event {
    Event::Command(CommandEvent {
        delivery: CommandDelivery::Raw,
        command: CanonicalCommand {
            command: "letter".into(),
            operand: CanonicalValue::Integer(i64::from(character)),
            control_sequence: None,
            location: Some(SourceLocation {
                source: "case.tex".into(),
                line,
                byte: 0,
            }),
        },
    })
}

fn mismatch(index: usize, actual: u32) -> Divergence {
    let line = index as u32 * 10;
    let sides = MismatchSides::Both {
        expected: Box::new(letter(97, line)),
        actual: Box::new(ObservedEvent::new(
            letter(actual, line),
            "source=case.tex".into(),
        )),
    };
    Divergence::Mismatch(Box::new(StreamMismatch {
        fixture: IDENTITY.into(),
        index,
        actual_index: index,
        kind: classify_mismatch_kind(&sides),
        repair: Repair::Changed,
        sides,
        suppressed_cascade: 0,
    }))
}

/// Three recurrences of one root site and two of another, interleaved.
fn report(grouped: bool) -> ComparisonReport {
    let divergences = vec![
        mismatch(1, 98),
        mismatch(2, 99),
        mismatch(3, 98),
        mismatch(4, 99),
        mismatch(5, 98),
    ];
    ComparisonReport {
        fixtures: vec![FixtureSummary {
            name: "tex82/case".into(),
            identity: IDENTITY.into(),
            advisory: false,
            state: FixtureState::Compared {
                divergences: divergences.len(),
                budgeted: divergences.len(),
                first_index: Some(1),
                budget_reached: false,
            },
        }],
        divergences,
        advisories: Vec::new(),
        grouped,
        max_divergences: 20,
    }
}

#[test]
fn the_grouped_header_labels_both_totals() {
    let rendered = report(true).to_string();
    let headline = rendered.lines().next().expect("headline");
    assert_eq!(headline, "5 ordered divergence(s) in 2 root site(s):");
    assert!(
        rendered.contains("Grouping does not change this"),
        "{rendered}"
    );
    assert!(rendered.contains("--ungrouped"), "{rendered}");
}

#[test]
fn the_ungrouped_header_keeps_the_historical_first_line_and_still_names_both_totals() {
    let rendered = report(false).to_string();
    let headline = rendered.lines().next().expect("headline");
    assert_eq!(headline, "5 ordered divergence(s):");
    assert!(
        rendered.contains("would report 2 root site(s)"),
        "{rendered}"
    );
}

#[test]
fn a_grouped_entry_names_its_count_and_every_occurrence() {
    let rendered = report(true).to_string();
    assert!(rendered.contains("[0] x3 fixture tex82/case"), "{rendered}");
    assert!(rendered.contains("[1] x2 fixture tex82/case"), "{rendered}");
    assert!(
        rendered.contains("3 exact occurrence(s) of this root site"),
        "{rendered}"
    );
    // Every collapsed occurrence is reachable from the entry that stands for
    // it: the index list is printed whole, never sampled.
    assert!(rendered.contains(" 1, 3, 5"), "{rendered}");
    assert!(rendered.contains(" 2, 4"), "{rendered}");
}

#[test]
fn a_single_occurrence_entry_carries_no_recurrence_block() {
    let mut report = report(true);
    report.divergences.truncate(1);
    let rendered = report.to_string();
    assert_eq!(
        rendered.lines().next().expect("headline"),
        "1 ordered divergence(s) in 1 root site(s):"
    );
    assert!(rendered.contains("[0] x1 fixture tex82/case"), "{rendered}");
    assert!(!rendered.contains("exact occurrence(s)"), "{rendered}");
}

#[test]
fn the_ungrouped_worklist_still_prints_one_entry_per_divergence() {
    let rendered = report(false).to_string();
    for position in 0..5 {
        assert!(
            rendered.contains(&format!("[{position}] fixture tex82/case")),
            "{rendered}"
        );
    }
    assert!(!rendered.contains(" x3 "), "{rendered}");
}

#[test]
fn a_reached_budget_is_announced_rather_than_left_to_be_inferred() {
    let mut report = report(true);
    report.fixtures[0].state = FixtureState::Compared {
        divergences: 5,
        budgeted: 5,
        first_index: Some(1),
        budget_reached: true,
    };
    report.max_divergences = 5;
    let rendered = report.to_string();
    assert!(
        rendered.contains("BOUNDED: --max-divergences 5 counts ordered divergences"),
        "{rendered}"
    );
}

#[test]
fn a_fixture_that_never_ran_does_not_read_like_a_clean_one() {
    let mut report = report(true);
    report
        .fixtures
        .push(FixtureSummary::not_generated("tex82/document-absent-v1"));
    let rendered = report.to_string();
    assert!(
        rendered.contains("tex82/document-absent-v1  not compared -- trace not generated"),
        "{rendered}"
    );
    assert!(!rendered.contains("tex82/document-absent-v1  0 divergence(s)"));
}

#[test]
fn a_clean_fixture_is_listed_with_zero_rather_than_omitted() {
    let mut report = report(true);
    report.fixtures.push(FixtureSummary {
        name: "tex82/document-clean-v1".into(),
        identity: "tex82/document-clean-v1 manifest=def".into(),
        advisory: false,
        state: FixtureState::Compared {
            divergences: 0,
            budgeted: 0,
            first_index: None,
            budget_reached: false,
        },
    });
    let rendered = report.to_string();
    assert!(
        rendered.contains("tex82/document-clean-v1  0 divergence(s)"),
        "{rendered}"
    );
}

/// The defect this contract exists for (`umber2-johp.168`): a run that never
/// compared three of four registered fixtures reported the same "no
/// divergence" success as a run that compared all four.
#[test]
fn an_uncompared_fixture_makes_the_run_partial_rather_than_clean() {
    let mut clean = report(true);
    clean.divergences.clear();
    clean.fixtures[0].state = FixtureState::Compared {
        divergences: 0,
        budgeted: 0,
        first_index: None,
        budget_reached: false,
    };
    assert_eq!(clean.outcome(), RunOutcome::Clean);
    assert_eq!(clean.outcome().exit_code(), 0);

    let mut partial = clean.clone();
    partial
        .fixtures
        .push(FixtureSummary::not_generated("tex82/document-absent-v1"));
    assert_eq!(partial.outcome(), RunOutcome::Partial);
    assert_eq!(partial.outcome().exit_code(), 2);
    assert_ne!(partial.outcome().exit_code(), clean.outcome().exit_code());
    assert_eq!(partial.divergence_count(), clean.divergence_count());

    let rendered = partial.to_string();
    assert!(rendered.contains("VERDICT: PARTIAL (exit 2)"), "{rendered}");
    assert!(rendered.contains("LOWER BOUND"), "{rendered}");
    assert!(
        rendered.contains("never compared (1): tex82/document-absent-v1"),
        "{rendered}"
    );
}

/// A budget that stopped a fixture short also makes the total a lower bound,
/// so it earns the same status as a fixture that never ran.
#[test]
fn a_reached_budget_makes_the_run_partial_too() {
    let mut report = report(true);
    assert_eq!(report.outcome(), RunOutcome::Diverged);
    assert_eq!(report.outcome().exit_code(), 1);
    assert!(report.to_string().contains("VERDICT: DIVERGED (exit 1)"));

    report.fixtures[0].state = FixtureState::Compared {
        divergences: 5,
        budgeted: 5,
        first_index: Some(1),
        budget_reached: true,
    };
    report.max_divergences = 5;
    assert_eq!(report.outcome(), RunOutcome::Partial);
    let rendered = report.to_string();
    assert!(
        rendered.contains("stopped at the --max-divergences 5 budget (1): tex82/case"),
        "{rendered}"
    );
}

#[test]
fn a_clean_run_still_prints_a_verdict_rather_than_nothing() {
    let report = ComparisonReport {
        fixtures: vec![FixtureSummary {
            name: "tex82/case".into(),
            identity: IDENTITY.into(),
            advisory: false,
            state: FixtureState::Compared {
                divergences: 0,
                budgeted: 0,
                first_index: None,
                budget_reached: false,
            },
        }],
        ..ComparisonReport::default()
    };
    let rendered = report.to_string();
    assert!(rendered.contains("VERDICT: CLEAN (exit 0)"), "{rendered}");
    assert!(
        rendered.contains("every registered fixture was compared"),
        "{rendered}"
    );
}

#[test]
fn geometry_only_difference_is_visible_but_does_not_change_the_verdict() {
    let mut report = report(true);
    let geometry_difference = report.divergences.remove(0);
    report.advisories.push(geometry_difference);
    report.divergences.clear();
    report.fixtures[0].advisory = true;
    report.fixtures[0].state = FixtureState::Compared {
        divergences: 1,
        budgeted: 1,
        first_index: Some(1),
        budget_reached: false,
    };

    assert_eq!(report.outcome(), RunOutcome::Clean);
    assert_eq!(report.outcome().exit_code(), 0);
    assert_eq!(report.divergence_count(), 0);
    assert_eq!(report.advisory_count(), 1);
    let rendered = report.to_string();
    assert!(rendered.contains("ADVISORY (non-gating)"), "{rendered}");
    assert!(
        rendered.contains("1 advisory geometry difference(s)"),
        "{rendered}"
    );
    assert!(rendered.contains("VERDICT: CLEAN (exit 0)"), "{rendered}");
}

#[test]
fn command_difference_remains_gating() {
    let report = report(true);
    assert_eq!(report.outcome(), RunOutcome::Diverged);
    assert_eq!(report.outcome().exit_code(), 1);
}

#[test]
fn the_long_index_list_wraps_without_dropping_an_occurrence() {
    let divergences: Vec<Divergence> = (0..200).map(|index| mismatch(index, 98)).collect();
    let report = ComparisonReport {
        fixtures: Vec::new(),
        divergences,
        advisories: Vec::new(),
        grouped: true,
        max_divergences: 20,
    };
    let rendered = report.to_string();
    assert!(rendered.contains("[0] x200 "), "{rendered}");
    let list = rendered
        .split("Every occurrence, by oracle event index:")
        .nth(1)
        .expect("index list")
        .split("\nADVISORY geometry diagnostics")
        .next()
        .expect("index list before the verdict");
    let printed: Vec<usize> = list
        .split(|character: char| !character.is_ascii_digit())
        .filter(|token| !token.is_empty())
        .map(|token| token.parse().expect("index"))
        .collect();
    assert_eq!(printed, (0..200).collect::<Vec<_>>());
    assert!(
        list.lines().all(|line| line.len() <= INDEX_LIST_WIDTH),
        "{list}"
    );
}

/// The exact header a complete run prints. Pinned byte for byte because every
/// concurrent measurement in this epic is taken from an exhaustive run, and
/// the lower-bound annotation added for bounded runs (`umber2-johp.207`) must
/// not reach this path.
const COMPLETE_GROUPED_HEADER: &str = "\
5 ordered divergence(s) in 2 root site(s):
  divergence(s): what the comparator found. Grouping does not change this
    number; it is the one to compare against historical totals.
  root site(s): the entries below, one per group of divergences that are
    identical after erasing source positions and nothing else. Every
    divergence is in exactly one group; none is dropped, sampled, or
    truncated. Pass --ungrouped for one entry per divergence.
";

/// Marks `report`'s single fixture as stopped by a budget of `budget`, with
/// `divergences` total of which `budgeted` were counted against it.
fn bounded_at(report: &mut ComparisonReport, budget: usize, budgeted: usize, divergences: usize) {
    report.fixtures[0].state = FixtureState::Compared {
        divergences,
        budgeted,
        first_index: Some(1),
        budget_reached: true,
    };
    report.max_divergences = budget;
}

#[test]
fn a_complete_run_header_carries_no_lower_bound_annotation() {
    let rendered = report(true).to_string();
    assert!(rendered.starts_with(COMPLETE_GROUPED_HEADER), "{rendered}");
    assert!(!rendered.contains("LOWER BOUND"), "{rendered}");
}

/// The header is where a reader takes a number from, so a bounded run must
/// withdraw the "compare this against historical totals" instruction there,
/// not only in the verdict at the bottom.
#[test]
fn a_bounded_run_marks_both_headline_totals_as_floors_in_the_header() {
    let mut report = report(true);
    bounded_at(&mut report, 5, 5, 5);
    let rendered = report.to_string();
    assert!(
        rendered.contains("LOWER BOUND: this run stopped short"),
        "{rendered}"
    );
    assert!(
        rendered.contains("Every total above is a floor, not a total, and"),
        "{rendered}"
    );
    assert!(
        !rendered.contains("it is the one to compare against historical totals"),
        "a bounded total must not be advertised as comparable: {rendered}"
    );
}

/// The same withdrawal applies to a run that never compared a fixture: the
/// annotation tracks completeness, not which bound caused it.
#[test]
fn an_uncompared_fixture_marks_the_headline_totals_as_floors_too() {
    let mut report = report(true);
    report
        .fixtures
        .push(FixtureSummary::not_generated("tex82/document-absent-v1"));
    let rendered = report.to_string();
    assert!(
        rendered.contains("LOWER BOUND: this run stopped short"),
        "{rendered}"
    );
    assert!(
        !rendered.contains("it is the one to compare against historical totals"),
        "{rendered}"
    );
}

/// The defect `umber2-johp.207` was filed for: a budget of 20 next to a
/// worklist of 13 entries, with nothing printed to say the two count
/// different things.
#[test]
fn a_bounded_fixture_says_the_budget_counts_divergences_not_the_entries_it_prints() {
    let mut report = report(true);
    bounded_at(&mut report, 5, 5, 5);
    let rendered = report.to_string();
    assert!(
        rendered.contains("BOUNDED: --max-divergences 5 counts ordered divergences; it"),
        "{rendered}"
    );
    assert!(
        rendered.contains("counts neither root sites nor printed entries."),
        "{rendered}"
    );
    // The fixture printed two entries under a budget of five; the notice has
    // to name both numbers so the smaller one does not read as slack.
    assert!(
        rendered.contains("stopped at 5 of them, so its 5 divergence(s) and"),
        "{rendered}"
    );
    assert!(
        rendered.contains("2 root site(s) above are both floors"),
        "{rendered}"
    );
}

/// A contained replay failure is reported outside the mismatch budget, so a
/// bounded fixture's divergence total can exceed the budget by one. Printed
/// without explanation that reads as the budget having been overrun.
#[test]
fn a_divergence_total_above_the_budget_names_the_unbudgeted_replay_failure() {
    let mut report = report(true);
    bounded_at(&mut report, 5, 5, 6);
    let rendered = report.to_string();
    assert!(
        rendered.contains("Its contained replay failure is reported outside the mismatch"),
        "{rendered}"
    );
    assert!(
        rendered.contains("budget, which is why 6 is more than the budget of 5."),
        "{rendered}"
    );

    // A fixture with no such failure must not carry the explanation.
    let mut without = report;
    bounded_at(&mut without, 5, 5, 5);
    assert!(
        !without.to_string().contains("contained replay failure"),
        "{without}"
    );
}
