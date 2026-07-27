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
            state: FixtureState::Compared {
                divergences: divergences.len(),
                first_index: Some(1),
                budget_reached: false,
            },
        }],
        divergences,
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
        first_index: Some(1),
        budget_reached: true,
    };
    report.max_divergences = 5;
    let rendered = report.to_string();
    assert!(
        rendered.contains("BOUNDED: --max-divergences 5 was reached"),
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
        state: FixtureState::Compared {
            divergences: 0,
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

#[test]
fn the_long_index_list_wraps_without_dropping_an_occurrence() {
    let divergences: Vec<Divergence> = (0..200).map(|index| mismatch(index, 98)).collect();
    let report = ComparisonReport {
        fixtures: Vec::new(),
        divergences,
        grouped: true,
        max_divergences: 20,
    };
    let rendered = report.to_string();
    assert!(rendered.contains("[0] x200 "), "{rendered}");
    let list = rendered
        .split("Every occurrence, by oracle event index:")
        .nth(1)
        .expect("index list");
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
