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
