//! Tiny TeX82 conditionals semantic minifixtures.

use std::collections::BTreeMap;
use std::sync::Arc;

use tex_command::{
    CommandObservation, CommandObserver, ObservedToken, RecoveryKind, RegisteredSourceKind,
    SourceRegistration,
};
use tex_exec::{CanonicalMainControl, MainControlStep};
use tex_state::Universe;

const CLASSIFICATION: &[u8] =
    include_bytes!("../../../../tests/corpus/command-semantic/conditionals/classification.tex");
const STACK_LIFECYCLE: &[u8] =
    include_bytes!("../../../../tests/corpus/command-semantic/conditionals/stack-lifecycle.tex");
const SKIPPED_TEXT: &[u8] =
    include_bytes!("../../../../tests/corpus/command-semantic/conditionals/skipped-text.tex");
const BRANCH_DELIMITERS: &[u8] =
    include_bytes!("../../../../tests/corpus/command-semantic/conditionals/branch-delimiters.tex");
const PREDICATE_DISPATCH: &[u8] =
    include_bytes!("../../../../tests/corpus/command-semantic/conditionals/predicate-dispatch.tex");
const ORDERED_RELATIONS: &[u8] =
    include_bytes!("../../../../tests/corpus/command-semantic/conditionals/ordered-relations.tex");
const ODD_INTEGER: &[u8] =
    include_bytes!("../../../../tests/corpus/command-semantic/conditionals/odd-integer.tex");
const TOKEN_PREDICATES: &[u8] =
    include_bytes!("../../../../tests/corpus/command-semantic/conditionals/token-predicates.tex");

const MAX_STEPS: usize = 512;
const COUNT_SLOTS: usize = 16;

#[derive(Default)]
struct Recorder(Vec<CommandObservation>);

impl CommandObserver for Recorder {
    fn committed(&mut self, observation: CommandObservation) {
        self.0.push(observation);
    }
}

struct SemanticRun {
    observations: Vec<CommandObservation>,
    counts: [i32; COUNT_SLOTS],
}

fn execute(source: &[u8]) -> SemanticRun {
    let mut universe = Universe::new();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    let source = control
        .command_mut()
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(source),
        ))
        .expect("semantic minifixture registers");
    control
        .command_mut()
        .open_registered_source(source)
        .expect("semantic minifixture opens");
    let mut recorder = Recorder::default();
    let mut ended = false;
    for _ in 0..MAX_STEPS {
        match control
            .step_with_observer(&mut universe, &mut recorder)
            .expect("semantic minifixture executes")
        {
            MainControlStep::Continue => {}
            MainControlStep::End | MainControlStep::EndOfInput => {
                ended = true;
                break;
            }
        }
    }
    assert!(ended, "semantic minifixture exceeded {MAX_STEPS} steps");
    let counts = std::array::from_fn(|slot| {
        universe.count(u16::try_from(slot).expect("count slot fits in TeX82 register index"))
    });
    SemanticRun {
        observations: recorder.0,
        counts,
    }
}

fn unique_command_operands(run: &SemanticRun, command: &str) -> Vec<i64> {
    let mut operands = Vec::new();
    for observation in &run.observations {
        let CommandObservation::Command(record) = observation else {
            continue;
        };
        if record.command != command {
            continue;
        }
        let operand = record
            .command_operand
            .expect("conditional command has a canonical operand");
        if !operands.contains(&operand) {
            operands.push(operand);
        }
    }
    operands
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ConditionStep {
    transition: &'static str,
    condition: &'static str,
    limit: &'static str,
    branch: Option<String>,
}

fn condition_steps(run: &SemanticRun) -> Vec<ConditionStep> {
    run.observations
        .iter()
        .filter_map(|observation| {
            let CommandObservation::Condition(record) = observation else {
                return None;
            };
            Some(ConditionStep {
                transition: record.transition,
                condition: record.condition,
                limit: record.limit,
                branch: record.branch.clone(),
            })
        })
        .collect()
}

fn step(
    transition: &'static str,
    condition: &'static str,
    limit: &'static str,
    branch: Option<&str>,
) -> ConditionStep {
    ConditionStep {
        transition,
        condition,
        limit,
        branch: branch.map(str::to_owned),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Scalar {
    kind: &'static str,
    value: String,
}

fn scalar(kind: &'static str, value: &str) -> Scalar {
    Scalar {
        kind,
        value: value.into(),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PredicateOutcome {
    condition: &'static str,
    scalars: Vec<Scalar>,
    truth: bool,
}

fn outcome(condition: &'static str, scalars: Vec<Scalar>, truth: bool) -> PredicateOutcome {
    PredicateOutcome {
        condition,
        scalars,
        truth,
    }
}

fn predicate_outcomes(run: &SemanticRun) -> Vec<PredicateOutcome> {
    let mut stack = Vec::new();
    let mut active = BTreeMap::<u64, (&'static str, Vec<Scalar>)>::new();
    let mut outcomes = Vec::new();
    for observation in &run.observations {
        match observation {
            CommandObservation::Condition(record) if record.transition == "push" => {
                stack.push(record.identity);
                active.insert(record.identity, (record.condition, Vec::new()));
            }
            CommandObservation::Scanner(record) => {
                if let Some(identity) = stack.last()
                    && let Some((_, scalars)) = active.get_mut(identity)
                {
                    scalars.push(Scalar {
                        kind: record.kind,
                        value: record.value.clone(),
                    });
                }
            }
            CommandObservation::Condition(record) if record.transition == "branch" => {
                let truth = match record.branch.as_deref() {
                    Some("true") => true,
                    Some("false") => false,
                    _ => continue,
                };
                let (condition, scalars) = active
                    .get(&record.identity)
                    .expect("boolean branch belongs to a pushed condition");
                outcomes.push(PredicateOutcome {
                    condition,
                    scalars: scalars.clone(),
                    truth,
                });
            }
            CommandObservation::Condition(record) if record.transition == "pop" => {
                assert_eq!(stack.pop(), Some(record.identity));
                active.remove(&record.identity);
            }
            _ => {}
        }
    }
    outcomes
}

fn skipping_status(run: &SemanticRun) -> Vec<(&'static str, &'static str)> {
    run.observations
        .iter()
        .filter_map(|observation| {
            let CommandObservation::ScannerStatus(record) = observation else {
                return None;
            };
            (record.from == "skipping" || record.to == "skipping")
                .then_some((record.from, record.to))
        })
        .collect()
}

fn count_mutations(run: &SemanticRun) -> Vec<String> {
    run.observations
        .iter()
        .filter_map(|observation| {
            let CommandObservation::Mutation(record) = observation else {
                return None;
            };
            record
                .value
                .starts_with("count:")
                .then(|| record.value.clone())
        })
        .collect()
}

fn branch_selections(run: &SemanticRun) -> Vec<(&'static str, &'static str, String)> {
    run.observations
        .iter()
        .filter_map(|observation| {
            let CommandObservation::Condition(record) = observation else {
                return None;
            };
            record
                .branch
                .as_ref()
                .map(|branch| (record.condition, record.limit, branch.clone()))
        })
        .collect()
}

#[test]
fn classification_minifixture_matches_tex82_sections_210_487_491() {
    let run = execute(CLASSIFICATION);
    assert_eq!(
        unique_command_operands(&run, "if_test"),
        (0..=16).collect::<Vec<_>>()
    );
    assert_eq!(unique_command_operands(&run, "fi_or_else"), vec![2, 3, 4]);

    let inserted_fi = run
        .observations
        .iter()
        .filter(|observation| {
            matches!(
                observation,
                CommandObservation::Recovery(record)
                    if record.kind == RecoveryKind::InsertedToken
                        && record.tokens == [ObservedToken::ControlSequence("fi".into())]
            )
        })
        .count();
    assert_eq!(
        inserted_fi, 1,
        "EOF recovery must use frozen fi after public fi was redefined"
    );
}

#[test]
fn stack_lifecycle_minifixture_matches_tex82_sections_489_495_497() {
    let run = execute(STACK_LIFECYCLE);
    assert_eq!(run.counts[0], 1);
    assert_eq!(
        condition_steps(&run),
        vec![
            step("push", "ifnum", "evaluating", None),
            step("push", "iftrue", "evaluating", None),
            step("branch", "iftrue", "evaluating", Some("true")),
            step("limit", "iftrue", "else", None),
            step("pop", "iftrue", "else", None),
            step("branch", "ifnum", "evaluating", Some("true")),
            step("limit", "ifnum", "else", None),
            step("pop", "ifnum", "else", None),
        ]
    );
}

#[test]
fn skipped_text_minifixture_matches_tex82_sections_493_494() {
    let run = execute(SKIPPED_TEXT);
    assert_eq!(run.counts[0], 7);
    assert_eq!(count_mutations(&run), ["count:0=7"]);
    assert_eq!(
        skipping_status(&run),
        [("normal", "skipping"), ("skipping", "normal")]
    );
    assert_eq!(
        condition_steps(&run),
        vec![
            step("push", "iffalse", "evaluating", None),
            step("branch", "iffalse", "evaluating", Some("false")),
            step("branch", "iffalse", "evaluating", Some("else")),
            step("limit", "iffalse", "fi", None),
            step("pop", "iffalse", "fi", None),
        ]
    );
}

#[test]
fn branch_delimiter_minifixture_matches_tex82_sections_498_500_509_510() {
    let run = execute(BRANCH_DELIMITERS);
    assert_eq!(&run.counts[..3], &[7, 1, 1]);
    assert_eq!(
        branch_selections(&run),
        [
            ("ifcase", "evaluating", "or".into()),
            ("ifcase", "evaluating", "or".into()),
            ("ifcase", "or", "case".into()),
            ("ifcase", "or", "else".into()),
            ("ifcase", "or", "fi".into()),
            ("ifcase", "evaluating", "or".into()),
            ("ifcase", "evaluating", "else".into()),
            ("iffalse", "evaluating", "false".into()),
            ("iffalse", "evaluating", "else".into()),
        ]
    );
}

#[test]
fn predicate_dispatch_minifixture_matches_tex82_section_501() {
    let run = execute(PREDICATE_DISPATCH);
    assert_eq!(
        predicate_outcomes(&run),
        [
            outcome("ifvmode", vec![], true),
            outcome("ifhmode", vec![], false),
            outcome("ifmmode", vec![], false),
            outcome("ifinner", vec![], false),
            outcome("ifhmode", vec![], true),
            outcome("ifinner", vec![], true),
            outcome("ifmmode", vec![], true),
            outcome("ifinner", vec![], true),
            outcome("ifeof", vec![scalar("integer", "0")], true),
            outcome("iftrue", vec![], true),
            outcome("iffalse", vec![], false),
        ]
    );
    assert_eq!(&run.counts[..9], &[1, 1, 1, 1, 0, 0, 1, 1, 1]);
}

#[test]
fn ordered_relations_minifixture_matches_tex82_section_503() {
    let run = execute(ORDERED_RELATIONS);
    assert_eq!(&run.counts[..7], &[1; 7]);
    assert_eq!(
        predicate_outcomes(&run),
        [
            outcome(
                "ifnum",
                vec![scalar("integer", "1"), scalar("integer", "2")],
                true
            ),
            outcome(
                "ifnum",
                vec![scalar("integer", "2"), scalar("integer", "2")],
                true
            ),
            outcome(
                "ifnum",
                vec![scalar("integer", "3"), scalar("integer", "2")],
                true
            ),
            outcome(
                "ifdim",
                vec![
                    scalar("integer", "1"),
                    scalar("dimension", "65536"),
                    scalar("integer", "2"),
                    scalar("dimension", "131072"),
                ],
                true,
            ),
            outcome(
                "ifdim",
                vec![
                    scalar("integer", "2"),
                    scalar("dimension", "131072"),
                    scalar("integer", "2"),
                    scalar("dimension", "131072"),
                ],
                true,
            ),
            outcome(
                "ifdim",
                vec![
                    scalar("integer", "3"),
                    scalar("dimension", "196608"),
                    scalar("integer", "2"),
                    scalar("dimension", "131072"),
                ],
                true,
            ),
            outcome(
                "ifnum",
                vec![scalar("integer", "1"), scalar("integer", "1")],
                true
            ),
        ]
    );
}

#[test]
fn odd_integer_minifixture_matches_tex82_section_504() {
    let run = execute(ODD_INTEGER);
    assert_eq!(&run.counts[..5], &[1; 5]);
    assert_eq!(
        predicate_outcomes(&run),
        [
            outcome("ifodd", vec![scalar("integer", "0")], false),
            outcome("ifodd", vec![scalar("integer", "2")], false),
            outcome("ifodd", vec![scalar("integer", "-2")], false),
            outcome("ifodd", vec![scalar("integer", "1")], true),
            outcome("ifodd", vec![scalar("integer", "-1")], true),
        ]
    );
}

#[test]
fn token_predicates_minifixture_matches_tex82_sections_506_508() {
    let run = execute(TOKEN_PREDICATES);
    assert_eq!(&run.counts[..10], &[1; 10]);
    assert_eq!(
        predicate_outcomes(&run),
        [
            outcome("if", vec![], true),
            outcome("if", vec![], false),
            outcome("ifcat", vec![], true),
            outcome("ifcat", vec![], false),
            outcome("if", vec![], true),
            outcome("if", vec![], true),
            outcome("ifx", vec![], true),
            outcome("ifx", vec![], false),
            outcome("ifx", vec![], true),
            outcome("ifx", vec![], false),
        ]
    );
}
