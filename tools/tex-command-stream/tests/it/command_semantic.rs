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
        let run = fs::read(declared.domain_dir.join(&declared.case.source))
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

fn compare_declared_channels(declared: &DeclaredCase, run: &SemanticRun) -> Vec<ChannelFailure> {
    let contract = declared
        .case
        .channels
        .as_ref()
        .expect("load_suite requires every case to declare a channel contract");
    let committed = |channel: StreamChannel| {
        fs::read_to_string(channel_file(
            &declared.domain_dir,
            &declared.case.id,
            channel,
        ))
        .ok()
    };
    compare(&CapturedChannels::capture(run), contract, &committed)
}

/// The set of cases exempt from the channel contract is exactly the set whose
/// engine run does not complete, and it is pinned at its measured size.
///
/// Without the count this invariant would be satisfiable by pinning more cases
/// as `xfail` -- the exemption would become the escape hatch instead of the
/// ledger. Lowering the number is the only edit this test accepts silently.
#[test]
fn only_unrunnable_xfail_cases_are_exempt_from_the_channel_contract() {
    let cases =
        load_suite().unwrap_or_else(|error| panic!("invalid command-semantic corpus: {error}"));
    let mut exempt = Vec::new();
    for declared in &cases {
        if declared.case.channels.is_some() {
            continue;
        }
        let source = fs::read(declared.domain_dir.join(&declared.case.source))
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
    assert_eq!(
        exempt,
        [
            "input-expansion/expansion-conversions",
            "input-expansion/input-start-file",
            "main-control/read-to-definition",
        ],
        "the exempt set moved"
    );
    assert_eq!(cases.len(), 130, "the corpus changed size");
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
