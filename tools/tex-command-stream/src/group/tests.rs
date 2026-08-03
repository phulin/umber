use super::*;

use tex_oracle::{
    CommandDelivery, GeometryEvent, GeometryLocation, InputReason, InputTransition, RecoveryKind,
    ScannerEvent, SourceLocation, StateTarget,
};

use crate::ObservedEvent;
use crate::compare::StreamMismatch;

const FIXTURE: &str = "tex82/case manifest=abc";

fn location(line: u32, byte: u32) -> Option<SourceLocation> {
    Some(SourceLocation {
        source: "case.tex".into(),
        line,
        byte,
    })
}

fn hpack(line: u32, height_sp: i64) -> Event {
    Event::Geometry(GeometryEvent::Hpack {
        width_sp: 100,
        height_sp,
        depth_sp: 5,
        location: Some(GeometryLocation {
            source: "case.tex".into(),
            line,
        }),
    })
}

fn letter(character: u32, line: u32) -> Event {
    Event::Command(CommandEvent {
        delivery: CommandDelivery::Raw,
        command: CanonicalCommand {
            command: "letter".into(),
            operand: CanonicalValue::Integer(i64::from(character)),
            control_sequence: None,
            location: location(line, 0),
        },
    })
}

fn token(line: u32) -> OracleToken {
    OracleToken {
        character: 97,
        catcode: "letter".into(),
        control_sequence: None,
        location: location(line, 3),
    }
}

fn mismatch(index: usize, expected: Event, actual: Event, repair: Repair) -> Divergence {
    let sides = MismatchSides::Both {
        expected: Box::new(expected),
        actual: Box::new(ObservedEvent::new(
            actual,
            format!("source=case.tex; position={index}"),
        )),
    };
    Divergence::Mismatch(Box::new(StreamMismatch {
        fixture: FIXTURE.into(),
        index,
        actual_index: index,
        kind: crate::compare::classify_mismatch_kind(&sides),
        repair,
        sides,
        suppressed_cascade: index,
    }))
}

fn counts(sites: &[RootSite<'_>]) -> Vec<usize> {
    sites.iter().map(RootSite::count).collect()
}

#[test]
fn recurrences_that_differ_only_in_position_collapse_into_one_site() {
    let divergences = vec![
        mismatch(4, letter(97, 10), letter(98, 10), Repair::Changed),
        mismatch(9, letter(97, 40), letter(98, 40), Repair::Changed),
        mismatch(21, letter(97, 400), letter(98, 400), Repair::Changed),
    ];
    let sites = group(&divergences);
    assert_eq!(counts(&sites), vec![3]);
    assert_eq!(sites[0].representative().index(), 4);
    assert_eq!(
        sites[0]
            .occurrences()
            .iter()
            .map(|divergence| divergence.index())
            .collect::<Vec<_>>(),
        vec![4, 9, 21],
    );
    // The cascade a group stands in for is the sum over its members, so
    // collapsing recurrences never loses the scale of what they suppress.
    assert_eq!(sites[0].suppressed_cascade(), 4 + 9 + 21);
}

#[test]
fn geometry_recurrences_group_by_root_while_representative_retains_attribution() {
    let divergences = vec![
        mismatch(4, hpack(10, 20), hpack(10, 21), Repair::Changed),
        mismatch(9, hpack(40, 20), hpack(40, 21), Repair::Changed),
    ];
    let sites = group(&divergences);
    assert_eq!(counts(&sites), vec![2]);
    let Divergence::Mismatch(representative) = sites[0].representative() else {
        panic!("expected mismatch");
    };
    let MismatchSides::Both { expected, .. } = &representative.sides else {
        panic!("expected both sides");
    };
    assert_eq!(expected, &Box::new(hpack(10, 20)));
}

#[test]
fn a_differing_operand_keeps_two_entries_apart() {
    let divergences = vec![
        mismatch(4, letter(97, 10), letter(98, 10), Repair::Changed),
        mismatch(9, letter(97, 40), letter(99, 40), Repair::Changed),
    ];
    assert_eq!(counts(&group(&divergences)), vec![1, 1]);
}

#[test]
fn a_differing_repair_shape_keeps_two_entries_apart() {
    let divergences = vec![
        mismatch(
            4,
            letter(97, 10),
            letter(98, 10),
            Repair::Dropped { count: 1 },
        ),
        mismatch(
            9,
            letter(97, 40),
            letter(98, 40),
            Repair::Dropped { count: 3 },
        ),
    ];
    assert_eq!(counts(&group(&divergences)), vec![1, 1]);
}

#[test]
fn an_anchor_resync_ignores_only_the_anchor_line() {
    let line = |line: u32| Repair::AnchorResync {
        expected_skipped: 2,
        actual_skipped: 5,
        anchor: ResyncAnchor::Line {
            source: "case.tex".into(),
            line,
        },
    };
    let divergences = vec![
        mismatch(4, letter(97, 10), letter(98, 10), line(11)),
        mismatch(9, letter(97, 40), letter(98, 40), line(41)),
        mismatch(
            14,
            letter(97, 70),
            letter(98, 70),
            Repair::AnchorResync {
                expected_skipped: 2,
                actual_skipped: 5,
                anchor: ResyncAnchor::Input {
                    transition: InputTransition::Push,
                    reason: InputReason::Backup,
                    name: "backup".into(),
                },
            },
        ),
    ];
    assert_eq!(counts(&group(&divergences)), vec![2, 1]);
}

#[test]
fn positions_are_erased_everywhere_a_token_can_reach() {
    let recovery = |line: u32| {
        Event::Recovery(RecoveryEvent {
            kind: RecoveryKind::Backup,
            tokens: vec![token(line)],
        })
    };
    let scanner = |line: u32| {
        Event::Scanner(ScannerEvent {
            scanner: "token".into(),
            result: CanonicalValue::Tokens(vec![token(line)]),
        })
    };
    let mutation = |line: u32| {
        Event::Mutation(MutationEvent {
            target: StateTarget::Register,
            key: CanonicalValue::Token(token(line)),
            value: CanonicalValue::Tokens(vec![token(line + 1)]),
            scope: "local".into(),
        })
    };
    let macro_argument = |line: u32| {
        Event::Macro(MacroEvent::Argument {
            parameter: 1,
            tokens: vec![token(line)],
        })
    };
    let token_list = |line: u32| {
        Event::TokenList(TokenListEvent {
            transition: tex_oracle::TokenListTransition::Splice,
            purpose: "every_par".into(),
            tokens: vec![token(line)],
        })
    };
    let diagnostic = |line: u32| {
        Event::Diagnostic(DiagnosticEvent {
            severity: tex_oracle::DiagnosticSeverity::Warning,
            diagnostic: "undefined".into(),
            arguments: vec![CanonicalValue::Token(token(line))],
        })
    };
    let effect = |line: u32| {
        Event::Effect(EffectEvent {
            kind: tex_oracle::EffectKind::Write,
            channel: "stream:0".into(),
            value: CanonicalValue::Tokens(vec![token(line)]),
        })
    };
    let command_operand = |line: u32| {
        Event::Command(CommandEvent {
            delivery: CommandDelivery::Raw,
            command: CanonicalCommand {
                command: "the".into(),
                operand: CanonicalValue::Token(token(line)),
                control_sequence: Some("the".into()),
                location: location(line, 0),
            },
        })
    };

    for build in [
        &recovery as &dyn Fn(u32) -> Event,
        &scanner,
        &mutation,
        &macro_argument,
        &token_list,
        &diagnostic,
        &effect,
        &command_operand,
    ] {
        let divergences = vec![
            mismatch(4, build(10), letter(98, 10), Repair::Changed),
            mismatch(9, build(40), letter(98, 40), Repair::Changed),
        ];
        assert_eq!(
            counts(&group(&divergences)),
            vec![2],
            "{:?} still carries a position into the grouping key",
            build(10),
        );
    }
}

#[test]
fn a_payload_difference_beside_an_erased_position_still_separates() {
    // Two events whose *only* difference is a token payload, at different
    // positions: erasing positions must not erase the payload with them.
    let with = |line: u32, catcode: &str| {
        Event::Recovery(RecoveryEvent {
            kind: RecoveryKind::Backup,
            tokens: vec![OracleToken {
                character: 97,
                catcode: catcode.into(),
                control_sequence: None,
                location: location(line, 3),
            }],
        })
    };
    let divergences = vec![
        mismatch(4, with(10, "letter"), letter(98, 10), Repair::Changed),
        mismatch(9, with(40, "other_char"), letter(98, 40), Repair::Changed),
    ];
    assert_eq!(counts(&group(&divergences)), vec![1, 1]);
}

#[test]
fn contained_replay_failures_group_by_their_message() {
    let failure = |index: usize, message: &str| Divergence::Failure {
        fixture: FIXTURE.into(),
        index,
        failure: ReplayFailure::Error(message.into()),
    };
    let divergences = vec![
        failure(10, "missing font"),
        failure(20, "missing font"),
        failure(30, "bad register"),
    ];
    assert_eq!(counts(&group(&divergences)), vec![2, 1]);
}

#[test]
fn different_fixtures_never_merge() {
    let mut second = mismatch(9, letter(97, 40), letter(98, 40), Repair::Changed);
    if let Divergence::Mismatch(mismatch) = &mut second {
        mismatch.fixture = "tex82/other manifest=def".into();
    }
    let divergences = vec![
        mismatch(4, letter(97, 10), letter(98, 10), Repair::Changed),
        second,
    ];
    assert_eq!(counts(&group(&divergences)), vec![1, 1]);
}

#[test]
fn every_divergence_lands_in_exactly_one_group_in_stream_order() {
    let divergences = vec![
        mismatch(1, letter(97, 10), letter(98, 10), Repair::Changed),
        mismatch(2, letter(97, 20), letter(99, 20), Repair::Changed),
        mismatch(3, letter(97, 30), letter(98, 30), Repair::Changed),
        mismatch(4, letter(97, 40), letter(99, 40), Repair::Changed),
        mismatch(5, letter(97, 50), letter(98, 50), Repair::Changed),
    ];
    let sites = group(&divergences);
    assert_eq!(counts(&sites), vec![3, 2]);
    let mut seen: Vec<usize> = sites
        .iter()
        .flat_map(|site| site.occurrences().iter().map(|entry| entry.index()))
        .collect();
    assert_eq!(seen.len(), divergences.len());
    seen.sort_unstable();
    assert_eq!(seen, vec![1, 2, 3, 4, 5]);
    // Groups appear in the order their representatives do.
    assert_eq!(sites[0].representative().index(), 1);
    assert_eq!(sites[1].representative().index(), 2);
}
