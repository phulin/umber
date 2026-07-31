//! Gate for the declarative command-semantic minifixture corpus.
//!
//! The corpus contract itself lives in `tex_command_stream::semantic`; this
//! test only asserts it.

#![allow(
    clippy::disallowed_methods,
    reason = "this host-only fixture test reads its committed corpus"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::process::Command;

use sha2::{Digest, Sha256};
use tex_command::FatalError;
use tex_state::Universe;

use tex_command_stream::semantic::channels::compare;
use tex_command_stream::semantic::*;

#[test]
fn declared_command_semantic_cases_match() {
    let cases =
        load_suite().unwrap_or_else(|error| panic!("invalid command-semantic corpus: {error}"));
    let mut failures = Vec::new();
    for declared in &cases {
        let label = format!("{}/{}", declared.domain, declared.case.id);
        let run = fs::read(declared.fixture_dir.join(&declared.case.source))
            .map_err(|error| format!("source read: {error}"))
            .and_then(|source| execute(&source, &declared.case));
        let actual = run
            .as_ref()
            .map(|run| project(run, &declared.case.projection))
            .map_err(Clone::clone);
        if let Err(error) =
            evaluate_expectation(&declared.case.expected, &actual, &declared.case.expectation)
        {
            failures.push(format!("{label}: {error:?}"));
        }
        // The projection is a focused property claim about one observable.
        // The channel contract is the completeness claim about the rest of
        // the same run, and both have to hold.
        if let Ok(run) = &run {
            for failure in compare_declared_channels(declared, run) {
                failures.push(format!("{label}: {failure:?}"));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} declared cases failed:\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
    assert_eq!(raw_tex82_format_initializations(), 1);
    assert_eq!(raw_etex26_format_initializations(), 1);
}

#[test]
fn loaded_projection_distinguishes_explicit_end_from_nested_source_exhaustion() {
    let cases = load_suite().expect("valid command-semantic corpus");
    let run = |name: &str| {
        let declared = cases
            .iter()
            .find(|declared| declared.case.id == name)
            .unwrap_or_else(|| panic!("missing focused case {name}"));
        let source = fs::read(declared.fixture_dir.join(&declared.case.source))
            .unwrap_or_else(|error| panic!("{name} source: {error}"));
        execute(&source, &declared.case).unwrap_or_else(|error| panic!("{name}: {error}"))
    };
    let explicit_end = run("etex-loaded-ifcsname");
    assert_eq!(
        explicit_end.mode_transitions.last(),
        Some(&tex_exec::Mode::Vertical)
    );
    assert!(explicit_end.observations.iter().any(|observation| {
        matches!(
            observation,
            tex_command::CommandObservation::Effect(effect)
                if effect.kind == "terminate" && effect.detail == "engine\0"
        )
    }));

    let exhausted = run("etex-loaded-glue-component-enquiries");
    assert_ne!(
        exhausted.mode_transitions.last(),
        Some(&tex_exec::Mode::Vertical)
    );
    assert!(matches!(
        exhausted.observations.as_slice(),
        [.., tex_command::CommandObservation::Input(input), tex_command::CommandObservation::Effect(effect)]
            if input.transition == tex_command::InputTransition::Stop
                && input.reason == tex_command::InputReason::Source
                && effect.kind == "terminate"
                && effect.detail == "engine\0"
    ));
}

#[test]
fn ordinary_raw_tex82_batch_declares_exact_loaded_route_and_job_contracts() {
    const PRIOR_EXPECTED: &[&str] = &[
        "conditionals/box-register-selector-recovery",
        "conditionals/branch-delimiters",
        "conditionals/classification",
        "conditionals/odd-integer",
        "conditionals/ordered-relations",
        "conditionals/predicate-dispatch",
        "conditionals/skipped-text",
        "conditionals/stack-lifecycle",
        "conditionals/token-predicates",
        "input-expansion/command-code-boundaries",
        "input-expansion/edef-noexpand-the-interaction",
        "input-expansion/expansion-conversions",
        "input-expansion/expansion-delivery",
        "input-expansion/input-control-sequences",
        "input-expansion/input-level-lifecycle",
        "input-expansion/input-outer-recovery",
        "input-expansion/input-raw-delivery",
        "input-expansion/input-read-toks",
        "input-expansion/input-start-file",
        "input-expansion/input-tokenization-lifecycle",
        "input-expansion/mode-activities",
        "input-expansion/stored-token-replay",
        "scanners-internal-quantities/coercion-ownership",
        "scanners-internal-quantities/dimension-fraction",
        "scanners-internal-quantities/infinite-glue-case",
        "scanners-internal-quantities/input-stream-four-bit-recovery",
        "scanners-internal-quantities/integer-radix-forms",
        "scanners-internal-quantities/integer-sign-chain-and-units",
        "scanners-internal-quantities/internal-unit-probe",
        "scanners-internal-quantities/missing-left-brace-recovery",
        "scanners-internal-quantities/missing-number-error-context",
        "scanners-internal-quantities/numeric-token-categories",
        "scanners-internal-quantities/register-sources",
        "scanners-internal-quantities/scaled-division",
        "scanners-internal-quantities/vacuous-dimension-units",
    ];
    const ETEX_LOADED: &[&str] = &[
        "conditionals/etex-loaded-ifcsname",
        "conditionals/etex-loaded-ifdefined",
        "conditionals/etex-loaded-iffontchar",
        "conditionals/etex-loaded-unless-frame",
        "etex-diagnostics/etex-loaded-code-reassignment",
        "etex-diagnostics/etex-loaded-glue-component-enquiries",
        "etex-diagnostics/etex-loaded-macro-call",
        "etex-diagnostics/etex-loaded-meaning-reassignment",
        "etex-diagnostics/etex-loaded-state-reset",
    ];
    const EXCLUDED: &[(&str, SessionProfile)] = &[
        (
            "input-expansion/etex-noexpand-undefined",
            SessionProfile::EtexInitex,
        ),
        (
            "input-expansion/etex-outer-validity-eof",
            SessionProfile::EtexInitex,
        ),
        (
            "input-expansion/etex-readline-terminal",
            SessionProfile::EtexInitex,
        ),
        (
            "input-expansion/etex-unexpanded-delivery",
            SessionProfile::EtexInitex,
        ),
    ];
    const MAIN_CONTROL_EXCLUDED: &[(&str, SessionProfile)] = &[
        (
            "main-control/final-cleanup-end-or-dump",
            SessionProfile::Production,
        ),
        (
            "main-control/hyphenation-data",
            SessionProfile::RawTex82Loaded,
        ),
        ("main-control/hyphenation-errors", SessionProfile::Initex),
    ];

    let cases = load_suite().expect("valid command-semantic corpus");
    let by_name: BTreeMap<_, _> = cases
        .iter()
        .map(|declared| {
            (
                format!("{}/{}", declared.domain, declared.case.id),
                &declared.case,
            )
        })
        .collect();
    let prior_loaded: BTreeSet<_> = by_name
        .iter()
        .filter_map(|(name, case)| {
            (case.profile.execution_route() == ExecutionRoute::RawTex82Loaded)
                .then_some(name.as_str())
        })
        .filter(|name| {
            name.starts_with("conditionals/")
                || name.starts_with("input-expansion/")
                || name.starts_with("scanners-internal-quantities/")
        })
        .collect();
    assert_eq!(prior_loaded, PRIOR_EXPECTED.iter().copied().collect());
    let main_control_excluded: BTreeSet<_> = MAIN_CONTROL_EXCLUDED
        .iter()
        .map(|(name, _)| *name)
        .collect();
    let main_control: BTreeSet<_> = by_name
        .iter()
        .filter_map(|(name, case)| {
            (name.starts_with("main-control/")
                && !main_control_excluded.contains(name.as_str())
                && case.profile.execution_route() == ExecutionRoute::RawTex82Loaded)
                .then_some(name.as_str())
        })
        .collect();
    assert_eq!(main_control.len(), 55);
    let alignments: BTreeSet<_> = by_name
        .iter()
        .filter_map(|(name, case)| {
            (name.starts_with("alignments/")
                && case.profile.execution_route() == ExecutionRoute::RawTex82Loaded)
                .then_some(name.as_str())
        })
        .collect();
    assert_eq!(alignments.len(), 18);
    let math: BTreeSet<_> = by_name
        .iter()
        .filter_map(|(name, case)| {
            (name.starts_with("math/")
                && case.profile.execution_route() == ExecutionRoute::RawTex82Loaded)
                .then_some(name.as_str())
        })
        .collect();
    assert_eq!(math.len(), 34);
    let page_output: BTreeSet<_> = by_name
        .iter()
        .filter_map(|(name, case)| {
            (name.starts_with("page-output/")
                && case.profile.execution_route() == ExecutionRoute::RawTex82Loaded)
                .then_some(name.as_str())
        })
        .collect();
    assert_eq!(page_output.len(), 30);
    let allowlist = std::fs::read_to_string(
        repository_root().join("tests/command-semantic-oracle-profiles/raw-tex82-loaded.cases"),
    )
    .expect("loaded oracle allowlist");
    let allowlisted: BTreeSet<_> = allowlist
        .lines()
        .map(|line| line.split('#').next().unwrap_or_default().trim())
        .filter(|line| !line.is_empty())
        .collect();
    let expected_allowlist: BTreeSet<_> = prior_loaded
        .iter()
        .copied()
        .chain(main_control.iter().copied())
        .chain(alignments.iter().copied())
        .chain(math.iter().copied())
        .chain(page_output.iter().copied())
        .collect();
    assert_eq!(allowlisted, expected_allowlist);
    assert_eq!(allowlisted.len(), 172);
    let etex_loaded: BTreeSet<_> = by_name
        .iter()
        .filter_map(|(name, case)| {
            (case.profile.execution_route() == ExecutionRoute::RawEtex26Loaded)
                .then_some(name.as_str())
        })
        .collect();
    assert_eq!(etex_loaded, ETEX_LOADED.iter().copied().collect());
    let etex_loaded_cases: Vec<_> = etex_loaded.iter().map(|name| by_name[*name]).collect();
    assert_eq!(
        etex_loaded_cases
            .iter()
            .map(|case| case.font_inputs.len())
            .sum::<usize>(),
        1
    );
    assert_eq!(
        by_name["conditionals/etex-loaded-iffontchar"]
            .font_inputs
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["cmr10.tfm"]
    );
    assert!(etex_loaded_cases.iter().all(|case| {
        matches!(
            case.channels.as_ref().expect("validated channels").dvi,
            StreamDisposition::Empty
        )
    }));
    for (name, profile) in EXCLUDED {
        assert_eq!(by_name[*name].profile, *profile, "{name}");
        assert_eq!(
            by_name[*name].profile.execution_route(),
            ExecutionRoute::Fresh,
            "{name}"
        );
    }
    for (name, profile) in MAIN_CONTROL_EXCLUDED {
        assert_eq!(by_name[*name].profile, *profile, "{name}");
        assert!(!allowlisted.contains(name), "{name}");
    }

    let selected: Vec<_> = PRIOR_EXPECTED.iter().map(|name| by_name[*name]).collect();
    assert_eq!(
        selected.iter().map(|case| case.inputs.len()).sum::<usize>(),
        3
    );
    assert_eq!(
        selected
            .iter()
            .filter(|case| !case.terminal_lines.is_empty())
            .count(),
        3
    );
    assert_eq!(
        selected
            .iter()
            .filter(|case| matches!(
                case.channels.as_ref().expect("validated channels").dvi,
                StreamDisposition::File
            ))
            .count(),
        5
    );
    assert_eq!(
        selected
            .iter()
            .filter(|case| matches!(
                case.channels.as_ref().expect("validated channels").dvi,
                StreamDisposition::Empty
            ))
            .count(),
        30
    );

    let main_control_selected: Vec<_> = main_control.iter().map(|name| by_name[*name]).collect();
    assert_eq!(
        main_control_selected
            .iter()
            .map(|case| case.inputs.len())
            .sum::<usize>(),
        1
    );
    assert_eq!(
        main_control_selected
            .iter()
            .map(|case| case.font_inputs.len())
            .sum::<usize>(),
        3
    );
    assert_eq!(
        main_control_selected
            .iter()
            .filter(|case| !case.terminal_lines.is_empty())
            .count(),
        6
    );
    assert_eq!(
        main_control_selected
            .iter()
            .filter(|case| {
                matches!(
                    case.channels.as_ref().expect("validated channels").dvi,
                    StreamDisposition::File
                )
            })
            .count(),
        4
    );
    assert_eq!(
        main_control_selected
            .iter()
            .filter(|case| {
                matches!(
                    case.channels.as_ref().expect("validated channels").dvi,
                    StreamDisposition::Empty
                )
            })
            .count(),
        51
    );

    let alignment_selected: Vec<_> = alignments.iter().map(|name| by_name[*name]).collect();
    assert_eq!(
        alignment_selected
            .iter()
            .map(|case| case.inputs.len())
            .sum::<usize>(),
        0
    );
    assert_eq!(
        alignment_selected
            .iter()
            .map(|case| case.font_inputs.len())
            .sum::<usize>(),
        2
    );
    assert_eq!(
        alignment_selected
            .iter()
            .filter(|case| {
                matches!(
                    case.channels.as_ref().expect("validated channels").dvi,
                    StreamDisposition::File
                )
            })
            .count(),
        8
    );
    assert_eq!(
        alignment_selected
            .iter()
            .filter(|case| {
                matches!(
                    case.channels.as_ref().expect("validated channels").dvi,
                    StreamDisposition::Empty
                )
            })
            .count(),
        10
    );
    assert_eq!(
        alignment_selected
            .iter()
            .filter(|case| {
                case.channels.as_ref().expect("validated channels").status
                    == "fatal:confusion(256 spans)"
            })
            .count(),
        1
    );
    assert_eq!(
        alignment_selected
            .iter()
            .filter(|case| {
                case.channels.as_ref().expect("validated channels").status == "clean"
            })
            .count(),
        17
    );

    let math_selected: Vec<_> = math.iter().map(|name| by_name[*name]).collect();
    assert_eq!(
        math_selected
            .iter()
            .map(|case| case.inputs.len())
            .sum::<usize>(),
        0
    );
    assert_eq!(
        math_selected
            .iter()
            .map(|case| case.font_inputs.len())
            .sum::<usize>(),
        17
    );
    assert_eq!(
        math_selected
            .iter()
            .filter(|case| !case.terminal_lines.is_empty())
            .count(),
        1
    );
    assert_eq!(
        math_selected
            .iter()
            .filter(|case| {
                matches!(
                    case.channels.as_ref().expect("validated channels").dvi,
                    StreamDisposition::File
                )
            })
            .count(),
        22
    );
    assert_eq!(
        math_selected
            .iter()
            .filter(|case| {
                matches!(
                    case.channels.as_ref().expect("validated channels").dvi,
                    StreamDisposition::Empty
                )
            })
            .count(),
        12
    );
    assert!(
        math_selected
            .iter()
            .all(|case| { case.channels.as_ref().expect("validated channels").status == "clean" })
    );

    let page_output_selected: Vec<_> = page_output.iter().map(|name| by_name[*name]).collect();
    assert_eq!(
        page_output_selected
            .iter()
            .map(|case| case.inputs.len())
            .sum::<usize>(),
        0
    );
    assert_eq!(
        page_output_selected
            .iter()
            .map(|case| case.font_inputs.len())
            .sum::<usize>(),
        13
    );
    assert_eq!(
        page_output_selected
            .iter()
            .filter(|case| {
                matches!(
                    case.channels.as_ref().expect("validated channels").dvi,
                    StreamDisposition::File
                )
            })
            .count(),
        26
    );
    assert_eq!(
        page_output_selected
            .iter()
            .filter(|case| {
                matches!(
                    case.channels.as_ref().expect("validated channels").dvi,
                    StreamDisposition::Empty
                )
            })
            .count(),
        3
    );
    let dvi_xfails: Vec<_> = page_output
        .iter()
        .filter(|name| {
            matches!(
                by_name[**name]
                    .channels
                    .as_ref()
                    .expect("validated channels")
                    .dvi,
                StreamDisposition::Xfail { .. }
            )
        })
        .copied()
        .collect();
    assert_eq!(dvi_xfails, ["page-output/special-in-shipped-hbox"]);
    assert!(
        page_output_selected
            .iter()
            .all(|case| { case.channels.as_ref().expect("validated channels").status == "clean" })
    );
}

#[test]
fn raw_tex82_loaded_supplies_the_oracle_default_terminal_line() {
    let case: Case = serde_json::from_value(serde_json::json!({
        "id": "raw-loaded-empty-terminal-read",
        "property_id": "tex82.assignment.read-to-definition",
        "profile": "raw-tex82-loaded",
        "source": "raw-loaded-empty-terminal-read.tex",
        "provenance": {
            "authority": "tex.web",
            "manifest": "tests/tex82-oracle-manifest.txt",
            "sections": [360, 484, 1225]
        },
        "projection": {
            "kind": "observations",
            "kinds": ["input", "recovery"]
        },
        "expected": [],
        "expectation": {"kind": "pass"}
    }))
    .expect("bounded regression case is valid");
    let run = execute(br"\read-1 to\line\end", &case)
        .expect("the oracle's implicit empty terminal line satisfies the terminal read");
    let channels = CapturedChannels::capture(&run);
    assert_eq!(channels.events, 25);
    assert_eq!(channels.status, "clean");
    assert_eq!(
        channels.stream(StreamChannel::Terminal),
        concat!(
            "This is pdfTeX, Version 3.141592653-2.6-1.40.27 (TeX Live 2025) ",
            "(preloaded format=production)\n",
            "(./raw-loaded-empty-terminal-read.tex )\n",
            "No pages of output.\n",
            "Transcript written on raw-loaded-empty-terminal-read.log.\n"
        )
        .as_bytes()
    );
    assert_eq!(
        channels.stream(StreamChannel::Log),
        concat!(
            "This is pdfTeX, Version 3.141592653-2.6-1.40.27 (TeX Live 2025) ",
            "(preloaded format=production 2026.3.1)  1 JAN 1970 00:00\n",
            "**raw-loaded-empty-terminal-read.tex\n",
            "(./raw-loaded-empty-terminal-read.tex\n",
            " )\n",
            "No pages of output.\n"
        )
        .as_bytes()
    );
    assert_eq!(
        project(&run, &case.projection),
        [
            "input:push:terminal",
            "input:retire:terminal",
            "input:retire:file",
            "input:stop:terminal",
        ]
    );
}

#[test]
fn raw_tex82_loaded_reapplies_declared_job_input_with_resolved_name() {
    let case: Case = serde_json::from_value(serde_json::json!({
        "id": "raw-loaded-declared-input",
        "property_id": "tex82.input.loaded-job-resource",
        "profile": "raw-tex82-loaded",
        "source": "raw-loaded-declared-input.tex",
        "provenance": {
            "authority": "tex.web",
            "manifest": "tests/tex82-oracle-manifest.txt",
            "sections": [24, 534, 536, 537]
        },
        "projection": {"kind": "observations", "kinds": ["input"]},
        "expected": [],
        "expectation": {"kind": "pass"},
        "inputs": {"child.tex": "\\count0=37 "}
    }))
    .expect("bounded loaded-input regression case is valid");
    let run = execute(br"\input child\end", &case).expect("declared loaded-job input is available");
    let channels = CapturedChannels::capture(&run);

    assert_eq!(run.counts[0], 37);
    assert_eq!(channels.events, 48);
    assert_eq!(channels.status, "clean");
    assert_eq!(
        channels.stream(StreamChannel::Terminal),
        concat!(
            "This is pdfTeX, Version 3.141592653-2.6-1.40.27 (TeX Live 2025) ",
            "(preloaded format=production)\n",
            "(./raw-loaded-declared-input.tex (./child.tex) )\n",
            "No pages of output.\n",
            "Transcript written on raw-loaded-declared-input.log.\n"
        )
        .as_bytes()
    );
    assert_eq!(
        channels.stream(StreamChannel::Log),
        concat!(
            "This is pdfTeX, Version 3.141592653-2.6-1.40.27 (TeX Live 2025) ",
            "(preloaded format=production 2026.3.1)  1 JAN 1970 00:00\n",
            "**raw-loaded-declared-input.tex\n",
            "(./raw-loaded-declared-input.tex (./child.tex) )\n",
            "No pages of output.\n"
        )
        .as_bytes()
    );
    assert!(channels.stream(StreamChannel::Dvi).is_empty());
    assert!(channels.stream(StreamChannel::Effects).is_empty());
}

#[test]
fn raw_tex82_loaded_reapplies_declared_job_tfm() {
    let case: Case = serde_json::from_value(serde_json::json!({
        "id": "raw-loaded-declared-tfm",
        "property_id": "tex82.font.loaded-job-resource",
        "profile": "raw-tex82-loaded",
        "source": "raw-loaded-declared-tfm.tex",
        "provenance": {
            "authority": "tex.web",
            "manifest": "tests/tex82-oracle-manifest.txt",
            "sections": [560, 561, 565, 618]
        },
        "projection": {
            "kind": "execution-boundaries",
            "command_names": ["leader_ship"],
            "include_artifact_hashes": true
        },
        "expected": [],
        "expectation": {"kind": "pass"},
        "font_inputs": {
            "cmr10.tfm": "crates/tex-fonts/tests/fixtures/cm/cmr10.tfm"
        }
    }))
    .expect("bounded loaded-font regression case is valid");
    let run = execute(br"\font\ten=cmr10 \ten\shipout\hbox{A}\end", &case)
        .expect("declared loaded-job TFM is available");
    let channels = CapturedChannels::capture(&run);

    assert_eq!(channels.events, 75);
    assert_eq!(channels.status, "clean");
    assert_eq!(
        (
            format!(
                "{:x}",
                Sha256::digest(channels.stream(StreamChannel::Terminal))
            ),
            format!("{:x}", Sha256::digest(channels.stream(StreamChannel::Log))),
            format!("{:x}", Sha256::digest(channels.stream(StreamChannel::Dvi))),
        ),
        (
            "018a58ed865382e7c2cf187b0e91dfad7bf8078f453fe0b81a322165b3cae721".to_owned(),
            "c522687d11c774bd068e414620a35459eef0528f3679839c681de5bcac52c681".to_owned(),
            "07c3e696d0a55c9e9beec4c55efb22417ecffa8d3381d696608d87f41b3cf7bc".to_owned(),
        )
    );
    assert!(channels.stream(StreamChannel::Effects).is_empty());
}

#[test]
fn raw_tex82_loaded_preserves_nontrivial_mode_transitions() {
    let case: Case = serde_json::from_value(serde_json::json!({
        "id": "raw-loaded-mode-transitions",
        "property_id": "tex82.main-control.loaded-job-outcomes",
        "profile": "raw-tex82-loaded",
        "source": "raw-loaded-mode-transitions.tex",
        "provenance": {
            "authority": "tex.web",
            "manifest": "tests/tex82-oracle-manifest.txt",
            "sections": [1027, 1090, 1138]
        },
        "projection": {
            "kind": "execution-boundaries",
            "include_mode_transitions": true
        },
        "expected": [],
        "expectation": {"kind": "pass"}
    }))
    .expect("bounded loaded-mode regression case is valid");

    let run = execute(br"a\par b\par\end", &case).expect("loaded mode sequence completes");

    assert_eq!(
        run.mode_transitions,
        [
            tex_exec::Mode::Vertical,
            tex_exec::Mode::Horizontal,
            tex_exec::Mode::Vertical,
            tex_exec::Mode::Horizontal,
            tex_exec::Mode::Vertical,
        ]
    );
}

#[test]
fn raw_tex82_loaded_preserves_fatal_completion_and_channel_status() {
    let case: Case = serde_json::from_value(serde_json::json!({
        "id": "raw-loaded-fatal",
        "property_id": "tex82.main-control.loaded-job-outcomes",
        "profile": "raw-tex82-loaded",
        "source": "raw-loaded-fatal.tex",
        "interaction_mode": "nonstopmode",
        "provenance": {
            "authority": "tex.web",
            "manifest": "tests/tex82-oracle-manifest.txt",
            "sections": [81, 93, 360]
        },
        "projection": {"kind": "state", "count_registers": [0]},
        "expected": [],
        "expectation": {"kind": "pass"}
    }))
    .expect("bounded loaded-fatal regression case is valid");

    let run = execute(br"\input unavailable", &case)
        .expect("TeX fatal completion remains a completed loaded run");

    assert!(run.fatal.is_some());
    assert_eq!(
        CapturedChannels::capture(&run).status,
        format!("fatal:{}", run.fatal.expect("fatal state").label())
    );
}

fn compare_declared_channels(declared: &DeclaredCase, run: &SemanticRun) -> Vec<ChannelFailure> {
    let contract = declared
        .case
        .channels
        .as_ref()
        .expect("load_suite requires every case to declare a channel contract");
    let committed =
        |channel: StreamChannel| fs::read(channel_file(&declared.fixture_dir, channel)).ok();
    compare(&CapturedChannels::capture(run), contract, &committed)
}

/// The set of cases exempt from the channel contract is exactly the set whose
/// engine run does not complete -- and it is empty.
///
/// This used to also pin the corpus size, from when the exempt set was
/// non-empty and a count was the only thing stopping someone from growing the
/// exemptions instead of fixing a case. With the set asserted empty the count
/// guarded nothing: a new case either declares a channel contract or lands in
/// `exempt` and fails here. It only ever obstructed legitimate additions, so
/// it is gone.
#[test]
fn only_unrunnable_xfail_cases_are_exempt_from_the_channel_contract() {
    let cases =
        load_suite().unwrap_or_else(|error| panic!("invalid command-semantic corpus: {error}"));
    let mut exempt = Vec::new();
    for declared in &cases {
        if declared.case.channels.is_some() {
            continue;
        }
        let source = fs::read(declared.fixture_dir.join(&declared.case.source))
            .expect("an exempt case still has a readable source");
        assert!(
            execute(&source, &declared.case).is_err(),
            "{}/{} runs and must therefore declare a channel contract",
            declared.domain,
            declared.case.id
        );
        exempt.push(format!("{}/{}", declared.domain, declared.case.id));
    }
    exempt.sort();
    // Empty, and that is the point of the ledger: the three cases that used to
    // sit here -- `input-expansion/expansion-conversions`,
    // `input-expansion/input-start-file`, and `main-control/read-to-definition`
    // -- all reach the end of their run now, so every case in the corpus
    // declares a channel contract. Growing this list again is a regression to
    // argue for, not a convenience.
    assert_eq!(exempt, [] as [String; 0], "the exempt set moved");
}

#[test]
fn every_minifixture_file_is_local_and_tracked() {
    let root = repository_root();
    let cases =
        load_suite().unwrap_or_else(|error| panic!("invalid command-semantic corpus: {error}"));
    let mut fixture_dirs = BTreeSet::new();
    for declared in &cases {
        assert!(
            fixture_dirs.insert(declared.fixture_dir.clone()),
            "duplicate fixture directory {}",
            declared.fixture_dir.display()
        );
        let relative = declared
            .fixture_dir
            .strip_prefix(&root)
            .expect("fixture is beneath the repository");
        let output = Command::new("git")
            .args(["ls-files", "--error-unmatch", "--"])
            .arg(relative)
            .current_dir(&root)
            .output()
            .expect("git is available for the repository fixture gate");
        assert!(
            output.status.success(),
            "{} contains an untracked fixture file:\n{}",
            relative.display(),
            String::from_utf8_lossy(&output.stderr)
        );
        let tracked = String::from_utf8(output.stdout).expect("Git paths are UTF-8");
        assert_eq!(
            tracked.lines().count(),
            fs::read_dir(&declared.fixture_dir)
                .expect("fixture directory is readable")
                .count(),
            "{} has a file not represented in Git",
            relative.display()
        );
        for channel in STREAM_CHANNELS {
            assert_eq!(
                channel_file(&declared.fixture_dir, channel),
                declared
                    .fixture_dir
                    .join(format!("expected.{}", channel.name())),
                "the generator must emit channels inside their fixture directory"
            );
        }
    }
}

#[test]
fn validator_rejects_duplicate_and_unowned_cases() {
    let mut case_ids = BTreeSet::new();
    let mut sources = BTreeSet::new();
    let mut declared_sources = BTreeSet::new();
    assert!(
        claim_case_identity(
            &mut case_ids,
            &mut sources,
            &mut declared_sources,
            "conditionals",
            "case-a",
            "case-a.tex",
        )
        .is_ok()
    );
    assert!(
        claim_case_identity(
            &mut case_ids,
            &mut sources,
            &mut declared_sources,
            "conditionals",
            "case-a",
            "case-b.tex",
        )
        .expect_err("duplicate case identity must be rejected")
        .contains("duplicate case")
    );

    let unowned: Case = serde_json::from_slice(
        br#"{
            "id":"case-b",
            "property_id":"tex82.conditionals.not-owned",
            "source":"case-b.tex",
            "provenance":{"authority":"tex.web","manifest":"tests/tex82-oracle-manifest.txt","sections":[505]},
            "projection":{"kind":"predicate-outcomes"},
            "expected":["predicate:iftrue:-:true"],
            "expectation":{"kind":"pass"}
        }"#,
    )
    .expect("negative case has a valid manifest shape");
    assert!(
        validate_case(
            &unowned,
            "conditionals",
            Path::new("."),
            Path::new("."),
            &BTreeMap::new(),
            ChannelPolicy::Deriving,
        )
        .expect_err("unowned property must be rejected")
        .contains("unowned property")
    );
}
#[test]
fn xfail_manifest_validation_rejects_malformed_and_missing_bug_links() {
    let missing_bug = br#"{
        "kind":"xfail",
        "mismatch":{"index":0,"kind":"observation","expected":"a","actual":"b"}
    }"#;
    assert!(serde_json::from_slice::<Expectation>(missing_bug).is_err());

    let malformed: Expectation = serde_json::from_slice(
        br#"{
        "kind":"xfail",
        "bug":"not-a-bead",
        "mismatch":{"index":0,"kind":"observation","expected":"a","actual":"b"}
    }"#,
    )
    .expect("shape is parseable before semantic validation");
    assert!(validate_expectation(&malformed).is_err());

    let opaque: Expectation = serde_json::from_slice(
        br#"{
        "kind":"xfail",
        "bug":"umber2-o96f",
        "mismatch":{"index":0,"kind":"observation","expected":"a","actual":"b"}
    }"#,
    )
    .expect("opaque Beads id has the manifest shape");
    assert!(validate_expectation(&opaque).is_ok());
}

#[test]
fn state_projection_emits_only_requested_final_counts() {
    let mut counts = [0; COUNT_SLOTS];
    counts[2] = 7;
    let run = SemanticRun {
        observations: Vec::new(),
        counts,
        universe: Universe::new(),
        mode_transitions: Vec::new(),
        artifacts: Vec::new(),
        dvi: Vec::new(),
        fatal: None,
    };
    let projection = Projection {
        kind: ProjectionKind::State,
        count_registers: vec![2],
        include_count_mutations: false,
        kinds: Vec::new(),
        commands: Vec::new(),
        command_names: Vec::new(),
        alignment_transitions: Vec::new(),
        box_registers: Vec::new(),
        node_depth: None,
        include_mode_transitions: false,
        include_artifact_hashes: false,
        terminal_checks: Vec::new(),
    };

    assert_eq!(project(&run, &projection), ["count:2=7"]);
}

#[test]
fn fatal_termination_precedes_every_projection_kinds_own_output() {
    let mut counts = [0; COUNT_SLOTS];
    counts[2] = 7;
    let run = SemanticRun {
        observations: Vec::new(),
        counts,
        universe: Universe::new(),
        mode_transitions: Vec::new(),
        artifacts: Vec::new(),
        dvi: Vec::new(),
        fatal: Some(FatalError::confusion("256 spans")),
    };
    let projection = Projection {
        kind: ProjectionKind::State,
        count_registers: vec![2],
        include_count_mutations: false,
        kinds: Vec::new(),
        commands: Vec::new(),
        command_names: Vec::new(),
        alignment_transitions: Vec::new(),
        box_registers: Vec::new(),
        node_depth: None,
        include_mode_transitions: false,
        include_artifact_hashes: false,
        terminal_checks: Vec::new(),
    };

    assert_eq!(
        project(&run, &projection),
        ["execution:error:confusion(256 spans)", "count:2=7"]
    );
}

#[test]
fn terminal_checks_report_presence_and_absence_in_declaration_order() {
    let checks = vec!["alpha beta".into(), "gamma".into()];

    assert_eq!(
        terminal_check_results("alpha beta", &checks),
        [
            "terminal-check:alpha beta=true",
            "terminal-check:gamma=false"
        ]
    );
}

#[test]
fn strict_xfail_accepts_only_the_pinned_failure_and_rejects_xpass() {
    let expectation = Expectation::Xfail {
        bug: "umber2-o96f".into(),
        mismatch: MismatchFingerprint {
            index: 0,
            kind: "observation".into(),
            expected: "expected".into(),
            actual: "known-bug".into(),
        },
    };
    let expected = vec!["expected".into()];
    assert_eq!(
        evaluate_expectation(&expected, &Ok(vec!["expected".into()]), &expectation),
        Err(ExpectationError::Xpass)
    );
    assert!(matches!(
        evaluate_expectation(&expected, &Ok(vec!["new-failure".into()]), &expectation),
        Err(ExpectationError::ChangedFailure { .. })
    ));
    assert_eq!(
        evaluate_expectation(&expected, &Ok(vec!["known-bug".into()]), &expectation),
        Ok(())
    );
}
