use std::sync::Arc;

mod support;

use tex_command::{RegisteredSourceKind, SourceRegistration};
use tex_exec::{MainControl, MainControlStep, RootCompletionPolicy};
use tex_state::{EffectRecord, PrintSink, Universe};

const MAX_MAIN_CONTROL_STEPS: usize = 2_048;
const MAX_COMMAND_FUEL: u64 = 200_000;

const CASES: &[Case] = &[
    Case::new("after", &["A B"]),
    Case::new("box_brace_aliases", &["B:7.0pt"]),
    Case::new("box_dimensions", &["B:12.0pt,3.0pt,2.0pt"]),
    Case::new("box_movement", &["M:void,void"]),
    Case::new(
        "box_uncopy_badness",
        &[
            "Underfull \\hbox (badness 10000) detected at line 1",
            "B:10000 H:kept V:kept",
        ],
    ),
    Case::new(
        "every_box_hooks",
        &[
            "Underfull \\hbox (badness 10000) detected at line 6",
            "H:3,10.0pt;V:2",
        ],
    ),
    Case::new("grouping", &["G:0,2"]),
    Case::new(
        "hskip_penalty_recovery",
        &[
            "! Missing number, treated as zero.",
            "! Illegal unit of measure (pt inserted).",
            "R:recovered",
        ],
    ),
    Case::new(
        "illegal_mag",
        &[
            "! Illegal magnification has been changed to 1000 (40000).",
            "> 1.0pt.",
        ],
    ),
    Case::new(
        "incompatible_mag",
        &["! Incompatible magnification (2000);", "> 0.83333pt."],
    ),
    Case::new("insert_brace_aliases", &["I:1,3"]),
    Case::new("internal_dimension_params", &["D:11.0pt,7.0pt"]),
    Case::new(
        "last_box",
        &[
            "! You can't use `\\lastbox' in vertical mode.",
            "! You can't use `\\lastbox' in math mode.",
            "L:0.0pt,7.0pt;0.0pt,8.0pt;void;3.0pt,0.0pt;11.0pt;12.0pt;void,void",
        ],
    ),
    Case::new(
        "lccode_selector_recovery",
        &["! Bad character code (256).", "L:3:2"],
    ),
    Case::new("prefixed_macro", &["P:7"]),
    Case::new("too_many", &["! Too many }'s."]),
    Case::new(
        "wrong_close",
        &[
            "! Extra }, or forgotten \\endgroup.",
            "(\\end occurred inside a group at level 1)",
        ],
    ),
];

struct Case {
    name: &'static str,
    projection: &'static [&'static str],
}

impl Case {
    const fn new(name: &'static str, projection: &'static [&'static str]) -> Self {
        Self { name, projection }
    }
}

#[test]
fn tex82_reference_observation_fixtures_match_canonical_execution() {
    for case in CASES {
        let name = case.name;
        let source =
            test_support::read_repository_asset(format!("tests/corpus/tex_exec/{name}/{name}.tex"))
                .unwrap_or_else(|error| panic!("read tex_exec/{name} source: {error:#}"));
        let reference = test_support::read_fixture("tex_exec", name, "ref");
        let expected = ReferenceObservation::parse(&reference)
            .unwrap_or_else(|error| panic!("parse tex_exec/{name}/expected.ref: {error}"));
        let actual =
            execute(&source).unwrap_or_else(|error| panic!("execute tex_exec/{name}: {error}"));

        for (channel, reference, actual) in [
            ("terminal", expected.terminal, actual.terminal.as_str()),
            ("log", expected.log, actual.log.as_str()),
        ] {
            assert_projection(channel, reference, case.projection).unwrap_or_else(|error| {
                panic!("tex_exec/{name} reference projection failed: {error}")
            });
            assert_projection(channel, actual, case.projection)
                .unwrap_or_else(|error| panic!("tex_exec/{name} Umber projection failed: {error}"));
        }
    }
}

fn assert_projection(channel: &str, text: &str, projection: &[&str]) -> Result<(), String> {
    let lines = text
        .lines()
        .map(normalize_source_suffix)
        .collect::<Vec<_>>();
    let mut next_line = 0;
    for expected in projection {
        let Some(relative_index) = lines[next_line..].iter().position(|line| line == expected)
        else {
            return Err(format!(
                "{channel} lacks exact ordered line {expected:?}:\n{text}"
            ));
        };
        next_line += relative_index + 1;
    }
    Ok(())
}

fn normalize_source_suffix(line: &str) -> &str {
    let Some((message, suffix)) = line.rsplit_once(" [") else {
        return line;
    };
    let Some(source_id) = suffix.strip_suffix(']') else {
        return line;
    };
    if source_id.split('.').all(|component| {
        !component.is_empty() && component.bytes().all(|byte| byte.is_ascii_digit())
    }) {
        message
    } else {
        line
    }
}

struct ReferenceObservation<'a> {
    terminal: &'a str,
    log: &'a str,
}

impl<'a> ReferenceObservation<'a> {
    fn parse(reference: &'a str) -> Result<Self, &'static str> {
        let (_, channels) = reference
            .split_once("\nstdout:\n")
            .ok_or("missing stdout header")?;
        let (terminal, log) = channels.split_once("log:\n").ok_or("missing log header")?;
        Ok(Self { terminal, log })
    }
}

#[derive(Debug)]
struct ActualObservation {
    terminal: String,
    log: String,
}

fn execute(source: &[u8]) -> Result<ActualObservation, String> {
    execute_with_limits(source, MAX_MAIN_CONTROL_STEPS, MAX_COMMAND_FUEL)
}

fn execute_with_limits(
    source: &[u8],
    step_limit: usize,
    fuel_limit: u64,
) -> Result<ActualObservation, String> {
    execute_with_policy_and_limits(
        source,
        RootCompletionPolicy::RequireTeXEnd,
        step_limit,
        fuel_limit,
    )
}

fn execute_with_policy_and_limits(
    source: &[u8],
    completion: RootCompletionPolicy,
    step_limit: usize,
    fuel_limit: u64,
) -> Result<ActualObservation, String> {
    support::with_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        control.set_root_completion_policy(completion);
        control
            .set_fuel_limit(fuel_limit)
            .map_err(|error| format!("invalid command-fuel limit {fuel_limit}: {error:?}"))?;
        control
            .register_root_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(source),
            ))
            .map_err(|error| format!("fixture source registration failed: {error:?}"))?;

        for step in 1..=step_limit {
            match control.step(stores) {
                Ok(MainControlStep::Continue) => {}
                Ok(MainControlStep::End) => {
                    if let Some(fatal) = control.fatal_error() {
                        return Err(format!(
                            "fatal main-control termination after {step} steps and {}/{} fuel: {fatal:?}",
                            control.fuel_burned(),
                            control.fuel_limit()
                        ));
                    }
                    return Ok(ActualObservation {
                        terminal: channel_text(stores, PrintSink::Terminal),
                        log: channel_text(stores, PrintSink::Log),
                    });
                }
                Ok(MainControlStep::EndOfInput) => {
                    return Err(format!(
                        "physical input exhaustion after {step} steps (fatal={:?}, fuel={}/{})",
                        control.fatal_error(),
                        control.fuel_burned(),
                        control.fuel_limit()
                    ));
                }
                Err(error) => {
                    return Err(format!(
                        "main-control step {step} failed after {}/{} fuel: {error:?}",
                        control.fuel_burned(),
                        control.fuel_limit()
                    ));
                }
            }
        }

        Err(format!(
            "exceeded {step_limit} main-control steps after {}/{} fuel",
            control.fuel_burned(),
            control.fuel_limit()
        ))
    })
}

#[test]
fn fixture_runner_rejects_prefix_matches_and_noncompletion() {
    let prefix_only = "prefix has an unreviewed suffix";
    assert!(
        assert_projection("terminal", prefix_only, &["prefix"]).is_err(),
        "a projected line prefix must not hide a changed observation"
    );
    assert!(
        assert_projection("terminal", "prefix [unreviewed]", &["prefix"]).is_err(),
        "only a numeric source-id suffix may be normalized"
    );

    let exhaustion = execute_with_policy_and_limits(
        br"\message{prefix}",
        RootCompletionPolicy::StopAtRootEof,
        64,
        1_000,
    )
    .expect_err("fragment EOF must not count as complete-job completion");
    assert!(
        exhaustion.contains("physical input exhaustion"),
        "unexpected EOF failure: {exhaustion}"
    );

    let execution_error = execute_with_limits(br"\input missing\end", 64, 1_000)
        .expect_err("a main-control error must not count as completion");
    assert!(
        execution_error.contains("main-control step") && execution_error.contains("MissingToken"),
        "unexpected execution failure: {execution_error}"
    );
}

#[test]
fn fixture_runner_distinguishes_clean_end_from_fatal_end() {
    execute_with_limits(br"\end", 8, 100).expect("an explicit clean end must complete");

    let mut fatal_source = br"\nonstopmode ".to_vec();
    for _ in 0..100 {
        fatal_source.extend_from_slice(br"\badness ");
    }
    fatal_source.extend_from_slice(br"\count0=23\end");
    let fatal = execute_with_limits(&fatal_source, 256, 10_000)
        .expect_err("TeX82's hundredth-error succumb must not count as clean completion");
    assert!(
        fatal.contains("fatal main-control termination") && fatal.contains("TooManyErrors"),
        "unexpected fatal-End failure: {fatal}"
    );
}

#[test]
fn fixture_runner_enforces_step_and_fuel_limits() {
    let step_error = execute_with_limits(br"\relax\end", 1, 1_000)
        .expect_err("one step cannot finish two commands");
    assert!(
        step_error.contains("exceeded 1 main-control steps"),
        "unexpected step-limit failure: {step_error}"
    );

    let fuel_error = execute_with_limits(br"\def\again{\again}\again\end", 64, 16)
        .expect_err("recursive expansion must exhaust finite command fuel");
    assert!(
        fuel_error.contains("FuelExhausted"),
        "unexpected fuel-limit failure: {fuel_error}"
    );
}

fn channel_text<G>(stores: &Universe<G>, channel: PrintSink) -> String {
    let committed = match channel {
        PrintSink::Terminal => stores.world().memory_terminal_output(),
        PrintSink::Log => stores.world().memory_log_output(),
        _ => unreachable!("only terminal and log are comparable fixture channels"),
    }
    .unwrap_or_default();
    let mut bytes = committed.to_vec();
    for effect in stores.world().effect_records() {
        if let EffectRecord::StreamWrite { sink, text } = effect
            && (*sink == channel || *sink == PrintSink::TerminalAndLog)
        {
            bytes.extend_from_slice(text.as_bytes());
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}
