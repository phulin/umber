use std::sync::Arc;

use tex_command::{RegisteredSourceKind, SourceRegistration};
use tex_exec::{MainControl, MainControlStep};
use tex_state::{EffectRecord, InteractionMode, PrintSink, Universe};

const CASES: &[Case] = &[
    Case::new("after", &["A B"]),
    Case::new("box_brace_aliases", &["B:7.0pt"]),
    Case::new("box_dimensions", &["B:12.0pt,3.0pt,2.0pt"]),
    Case::new("box_movement", &["M:void,void"]),
    Case::new(
        "box_uncopy_badness",
        &["Underfull \\hbox (badness 10000)", "B:10000 H:kept V:kept"],
    ),
    Case::new(
        "every_box_hooks",
        &["Underfull \\hbox (badness 10000)", "H:3,10.0pt;V:2"],
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
        let stores = execute(&source);
        let actual = ActualObservation {
            terminal: channel_text(&stores, PrintSink::Terminal),
            log: channel_text(&stores, PrintSink::Log),
        };

        for (channel, reference, actual) in [
            ("terminal", expected.terminal, actual.terminal.as_str()),
            ("log", expected.log, actual.log.as_str()),
        ] {
            assert_projection(name, channel, reference, case.projection, "reference");
            assert_projection(name, channel, actual, case.projection, "Umber");
        }
    }
}

fn assert_projection(case: &str, channel: &str, text: &str, projection: &[&str], producer: &str) {
    let mut remainder = text;
    for expected in projection {
        let Some(index) = remainder.find(expected) else {
            panic!(
                "tex_exec/{case} {producer} {channel} lacks projected observation {expected:?}:\n{text}"
            );
        };
        remainder = &remainder[index + expected.len()..];
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

struct ActualObservation {
    terminal: String,
    log: String,
}

fn execute(source: &[u8]) -> Universe {
    let mut stores = Universe::new_with_plain_catcodes();
    stores.set_interaction_mode(InteractionMode::Nonstop);
    let mut control = MainControl::tex82_initex(&mut stores);
    control
        .register_root_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(source),
        ))
        .expect("fixture source registers");

    while let Ok(MainControlStep::Continue) = control.step(&mut stores) {}

    stores
}

fn channel_text(stores: &Universe, channel: PrintSink) -> String {
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
