use super::*;

use tex_oracle::{CanonicalValue, InputEvent, MutationEvent, ScannerEvent, SourceLocation};

fn scanner(name: &str, value: &str) -> Event {
    Event::Scanner(ScannerEvent {
        scanner: name.into(),
        result: CanonicalValue::Name(value.into()),
    })
}

fn input(name: &str) -> Event {
    Event::Input(InputEvent {
        transition: InputTransition::Push,
        reason: InputReason::Source,
        name: name.into(),
    })
}

fn command(line: u32) -> Event {
    letter(line, 97)
}

/// A command whose alignment key is fixed by `line` and whose payload is
/// fixed by `character`, so two streams can be made to mismatch on payload
/// alone at a position the aligner still matches.
fn letter(line: u32, character: i64) -> Event {
    Event::Command(CommandEvent {
        delivery: CommandDelivery::Raw,
        command: CanonicalCommand {
            command: "letter".into(),
            operand: CanonicalValue::Integer(character),
            control_sequence: None,
            location: Some(SourceLocation {
                source: "case.tex".into(),
                line,
                byte: 0,
            }),
        },
    })
}

/// A run of distinct-key events, so that alignment is unambiguous.
fn run(prefix: &str, count: usize) -> Vec<Event> {
    (0..count)
        .map(|index| scanner(&format!("{prefix}{index}"), "value"))
        .collect()
}

fn oracle(events: &[Event]) -> Vec<NormalizedEvent> {
    events
        .iter()
        .enumerate()
        .map(|(sequence, event)| NormalizedEvent {
            sequence: sequence as u64,
            semantic: event.clone(),
        })
        .collect()
}

fn observed(events: &[Event]) -> Vec<ObservedEvent> {
    events
        .iter()
        .map(|event| ObservedEvent::new(event.clone(), "source=case.tex".into()))
        .collect()
}

fn align(expected: &[Event], actual: &[Event], tuning: AlignmentTuning) -> Vec<StreamMismatch> {
    find_divergences(
        "tex82/case",
        &oracle(expected),
        &observed(actual),
        64,
        tuning,
    )
    .entries
}

fn default_align(expected: &[Event], actual: &[Event]) -> Vec<StreamMismatch> {
    align(expected, actual, AlignmentTuning::default())
}

#[test]
fn identical_streams_report_nothing() {
    let stream = run("k", 20);
    assert!(default_align(&stream, &stream).is_empty());
}

#[test]
fn payload_difference_is_reported_once_and_leaves_the_streams_aligned() {
    let mut expected = run("k", 20);
    let mut actual = expected.clone();
    actual[5] = scanner("k5", "other");
    // A second, independent payload defect must still be reported: repairing
    // the first one must not consume it.
    expected[12] = scanner("k12", "left");
    actual[12] = scanner("k12", "right");

    let entries = default_align(&expected, &actual);
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].index, 5);
    assert_eq!(entries[0].repair, Repair::Changed);
    assert_eq!(entries[0].kind, "scanner_result_mismatch");
    assert_eq!(entries[0].suppressed_cascade, 0);
    assert_eq!(entries[1].index, 12);
    assert_eq!(entries[1].repair, Repair::Changed);
}

#[test]
fn a_dropped_oracle_event_is_one_entry_carrying_its_suppressed_cascade() {
    let expected = run("k", 20);
    let mut actual = expected.clone();
    actual.remove(5);

    let entries = default_align(&expected, &actual);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].index, 5);
    assert_eq!(entries[0].repair, Repair::Dropped { count: 1 });
    // Index-aligned comparison would have reported every following index.
    assert_eq!(entries[0].suppressed_cascade, 14);
    let report = entries[0].to_string();
    assert!(
        report.contains("1 oracle event(s) dropped by Umber"),
        "{report}"
    );
    assert!(
        report.contains("suppressed 14 cascade event(s)"),
        "{report}"
    );
}

#[test]
fn an_extra_observed_event_is_classified_as_extra() {
    let expected = run("k", 20);
    let mut actual = expected.clone();
    actual.insert(5, scanner("intruder", "value"));

    let entries = default_align(&expected, &actual);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].index, 5);
    assert_eq!(entries[0].actual_index, 5);
    assert_eq!(entries[0].repair, Repair::Extra { count: 1 });
}

#[test]
fn a_substituted_event_is_a_short_replaced_run() {
    let expected = run("k", 20);
    let mut actual = expected.clone();
    actual[5] = scanner("substitute", "value");

    let entries = default_align(&expected, &actual);
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].repair,
        Repair::Replaced {
            expected: 1,
            actual: 1
        }
    );
}

#[test]
fn a_repair_does_not_hide_an_independent_later_divergence() {
    let expected = run("k", 40);
    let mut actual = expected.clone();
    actual.remove(5);
    actual[24] = scanner("k25", "other");

    let entries = default_align(&expected, &actual);
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].repair, Repair::Dropped { count: 1 });
    assert_eq!(entries[1].index, 25);
    assert_eq!(entries[1].actual_index, 24);
    assert_eq!(entries[1].repair, Repair::Changed);
}

#[test]
fn a_realignment_is_only_accepted_once_the_confirmation_run_agrees() {
    let mut expected = vec![scanner("a", "value")];
    expected.extend(run("shared", 3));
    expected.extend(run("oracle-tail", 20));
    let mut actual = run("shared", 3);
    actual.extend(run("umber-tail", 20));

    // Three agreeing events look like a repair but are coincidence under the
    // default confirmation run, so the comparator refuses to guess.
    let entries = default_align(&expected, &actual);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].repair, Repair::Abandoned);

    let permissive = align(
        &expected,
        &actual,
        AlignmentTuning {
            confirmation: 2,
            ..AlignmentTuning::default()
        },
    );
    assert_eq!(permissive[0].repair, Repair::Dropped { count: 1 });
}

#[test]
fn a_repair_wider_than_the_window_is_not_searched_for() {
    let mut expected = run("gap", 100);
    expected.extend(run("shared", 20));
    let actual = run("shared", 20);

    let entries = default_align(&expected, &actual);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].repair, Repair::Abandoned);

    let wide = align(
        &expected,
        &actual,
        AlignmentTuning {
            window: 128,
            ..AlignmentTuning::default()
        },
    );
    assert_eq!(wide.len(), 1);
    assert_eq!(wide[0].repair, Repair::Dropped { count: 100 });
}

#[test]
fn the_anchor_fallback_rejoins_at_a_shared_input_boundary() {
    let mut expected = run("gap", 100);
    expected.push(input("child.tex"));
    expected.extend(run("shared", 20));
    let mut actual = vec![input("child.tex")];
    actual.extend(run("shared", 20));

    let entries = align(
        &expected,
        &actual,
        AlignmentTuning {
            window: 8,
            ..AlignmentTuning::default()
        },
    );
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].repair,
        Repair::AnchorResync {
            expected_skipped: 100,
            actual_skipped: 0,
            anchor: ResyncAnchor::Input {
                transition: InputTransition::Push,
                reason: InputReason::Source,
                name: "child.tex".into(),
            },
        }
    );
}

#[test]
fn the_anchor_fallback_rejoins_at_a_shared_source_line() {
    let mut expected = run("gap", 100);
    expected.push(command(42));
    expected.extend(run("shared", 20));
    let mut actual = vec![command(42)];
    actual.extend(run("shared", 20));

    let entries = align(
        &expected,
        &actual,
        AlignmentTuning {
            window: 8,
            ..AlignmentTuning::default()
        },
    );
    assert_eq!(entries.len(), 1);
    assert!(
        matches!(
            &entries[0].repair,
            Repair::AnchorResync { anchor, .. }
                if *anchor
                    == ResyncAnchor::Line {
                        source: "case.tex".into(),
                        line: 42,
                    }
        ),
        "{:?}",
        entries[0].repair
    );
}

#[test]
fn an_anchor_outside_the_scan_bound_is_not_used() {
    let mut expected = run("gap", 100);
    expected.push(input("child.tex"));
    expected.extend(run("shared", 20));
    let mut actual = vec![input("child.tex")];
    actual.extend(run("shared", 20));

    let entries = align(
        &expected,
        &actual,
        AlignmentTuning {
            window: 8,
            anchor_scan: 50,
            ..AlignmentTuning::default()
        },
    );
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].repair, Repair::Abandoned);
}

/// The scan bound is the only bound: an anchor inside it is a candidate however
/// many other anchors precede it. A dense oracle region reaches dozens of
/// input-stack boundaries in a few hundred events, so any secondary cap on the
/// anchor count silently shortens `--anchor-scan` to a fraction of its stated
/// reach and turns a repairable structural divergence into an abandonment.
#[test]
fn every_anchor_inside_the_scan_bound_is_a_candidate() {
    let mut expected: Vec<Event> = (0..80).map(|index| input(&format!("f{index}"))).collect();
    expected.push(input("child.tex"));
    expected.extend(run("shared", 20));
    let mut actual = vec![input("child.tex")];
    actual.extend(run("shared", 20));

    let entries = align(
        &expected,
        &actual,
        AlignmentTuning {
            window: 8,
            ..AlignmentTuning::default()
        },
    );
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].repair,
        Repair::AnchorResync {
            expected_skipped: 80,
            actual_skipped: 0,
            anchor: ResyncAnchor::Input {
                transition: InputTransition::Push,
                reason: InputReason::Source,
                name: "child.tex".into(),
            },
        }
    );
}

/// A rejoin's job is to put both streams back at the same point in the
/// *document*, and only a source line says where that is. An input push names
/// nothing but the shape of a boundary -- every macro activation in a run
/// carries the identical `Push/Macro macro` key, and so does every backup --
/// so ranking the two classes together by skip alone lets an anonymous
/// boundary a few events away outbid the shared source line that actually
/// locates the streams. Here the anonymous rejoin is less than half the cost
/// and confirms just as well, and it is still the wrong one: taking it leaves
/// the oracle inside material Umber never produced.
#[test]
fn a_shared_source_line_outranks_a_cheaper_anonymous_input_anchor() {
    // The oracle runs 100 events Umber never produced (an output-routine
    // episode has exactly this shape: no source location anywhere in it),
    // pushes a backup, runs 99 more, and only then returns to case.tex line
    // 42. Umber pushes the same backup immediately and reaches line 42 nine
    // events later.
    let mut expected = run("gap", 100);
    expected.push(input("backup"));
    expected.extend(run("decoy", 8));
    expected.extend(run("interior", 91));
    expected.push(command(42));
    expected.extend(run("shared", 20));

    let mut actual = vec![input("backup")];
    actual.extend(run("decoy", 8));
    actual.push(command(42));
    actual.extend(run("shared", 20));

    let entries = align(
        &expected,
        &actual,
        AlignmentTuning {
            window: 8,
            ..AlignmentTuning::default()
        },
    );
    assert_eq!(
        entries[0].repair,
        Repair::AnchorResync {
            expected_skipped: 200,
            actual_skipped: 9,
            anchor: ResyncAnchor::Line {
                source: "case.tex".into(),
                line: 42,
            },
        },
        "the backup rejoin costs 100 against the line rejoin's 209"
    );
    assert_eq!(
        entries.len(),
        1,
        "rejoining at the shared line leaves nothing else to report"
    );
}

/// Rejoining at anything but the least-total-skip shared anchor leaves the
/// streams on a boundary they agree at only locally, and the next real key
/// mismatch then finds no shared anchor in reach at all. The cheapest candidate
/// is not the first one visited: anchors are enumerated by oracle offset, so a
/// distant oracle anchor paired with an immediate observed one can undercut a
/// nearby oracle anchor paired with a far observed one.
#[test]
fn the_cheapest_shared_anchor_wins_over_the_first_one_found() {
    let mut expected = vec![input("alpha")];
    expected.extend(run("e", 99));
    expected.push(input("beta"));
    expected.extend(run("shared", 20));

    let mut actual = vec![input("beta")];
    actual.extend(run("shared", 20));
    actual.extend(run("pad", 179));
    actual.push(input("alpha"));
    actual.extend(run("e", 20));

    let entries = align(
        &expected,
        &actual,
        AlignmentTuning {
            window: 8,
            ..AlignmentTuning::default()
        },
    );
    assert_eq!(
        entries[0].repair,
        Repair::AnchorResync {
            expected_skipped: 100,
            actual_skipped: 0,
            anchor: ResyncAnchor::Input {
                transition: InputTransition::Push,
                reason: InputReason::Source,
                name: "beta".into(),
            },
        },
        "the alpha rejoin costs 200 and is visited first"
    );
}

#[test]
fn abandoning_a_fixture_stops_its_comparison_rather_than_guessing() {
    let expected = run("oracle", 40);
    let actual = run("umber", 40);

    let entries = default_align(&expected, &actual);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].repair, Repair::Abandoned);
    let report = entries[0].to_string();
    assert!(
        report.contains("comparison of this fixture stopped here"),
        "{report}"
    );
}

#[test]
fn a_shorter_observed_stream_reports_one_truncation_entry() {
    let expected = run("k", 20);
    let actual = expected[..12].to_vec();

    let entries = default_align(&expected, &actual);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].index, 12);
    assert_eq!(entries[0].kind, "stream_truncated_early");
    assert_eq!(entries[0].repair, Repair::Truncated { remaining: 8 });
}

#[test]
fn a_longer_observed_stream_reports_one_trailing_entry() {
    let expected = run("k", 12);
    let mut actual = expected.clone();
    actual.extend(run("trailing", 8));

    let entries = default_align(&expected, &actual);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].kind, "stream_has_unexpected_trailing_events");
    assert_eq!(entries[0].repair, Repair::Truncated { remaining: 8 });
}

#[test]
fn structural_transitions_are_identity_and_values_are_payload() {
    let push = input("child.tex");
    let retire = Event::Input(InputEvent {
        transition: InputTransition::Retire,
        reason: InputReason::Source,
        name: "child.tex".into(),
    });
    assert_ne!(alignment_key(&push), alignment_key(&retire));

    let mutation = |value: i64| {
        Event::Mutation(MutationEvent {
            target: StateTarget::Parameter,
            key: CanonicalValue::Name("integer_parameter:35".into()),
            value: CanonicalValue::Integer(value),
            scope: "local".into(),
        })
    };
    assert_eq!(alignment_key(&mutation(1)), alignment_key(&mutation(2)));
}

#[test]
fn a_command_source_position_is_part_of_its_identity() {
    assert_ne!(alignment_key(&command(1)), alignment_key(&command(2)));
    assert_eq!(alignment_key(&command(1)), alignment_key(&command(1)));
}

#[test]
fn the_divergence_budget_still_caps_one_fixture() {
    let expected = run("k", 40);
    let actual: Vec<Event> = expected
        .iter()
        .enumerate()
        .map(|(index, event)| {
            if index % 2 == 0 {
                scanner(&format!("k{index}"), "other")
            } else {
                event.clone()
            }
        })
        .collect();

    let comparison = find_divergences(
        "tex82/case",
        &oracle(&expected),
        &observed(&actual),
        3,
        AlignmentTuning::default(),
    );
    assert_eq!(comparison.entries.len(), 3);
    // The budget cut this comparison short, and the runner must be able to say
    // so rather than let a bounded worklist read like a complete one.
    assert!(comparison.budget_reached);
}

/// The `--max-divergences` budget's unit is ordered divergences, not the root
/// sites the grouped worklist prints one entry per (`umber2-johp.207`).
///
/// This is the case a root-site budget could not bound: forty mismatches that
/// differ only in source position are one root site, so a budget counting root
/// sites would let the comparison run to exhaustion however small it was set.
/// The budget counting divergences stops it where it says it will.
#[test]
fn the_budget_counts_divergences_not_the_root_sites_they_collapse_to() {
    let expected: Vec<Event> = (0..40).map(|line| letter(line, 97)).collect();
    let actual: Vec<Event> = (0..40).map(|line| letter(line, 98)).collect();

    let unbounded = find_divergences(
        "tex82/case",
        &oracle(&expected),
        &observed(&actual),
        40,
        AlignmentTuning::default(),
    );
    assert_eq!(unbounded.entries.len(), 40);
    assert!(!unbounded.budget_reached);
    let divergences: Vec<crate::Divergence> = unbounded
        .entries
        .iter()
        .cloned()
        .map(Box::new)
        .map(crate::Divergence::Mismatch)
        .collect();
    assert_eq!(
        crate::group::group(&divergences).len(),
        1,
        "all forty differ only in source position, so they are one root site"
    );

    let bounded = find_divergences(
        "tex82/case",
        &oracle(&expected),
        &observed(&actual),
        5,
        AlignmentTuning::default(),
    );
    assert_eq!(bounded.entries.len(), 5, "five divergences, not five sites");
    assert!(bounded.budget_reached);
}
