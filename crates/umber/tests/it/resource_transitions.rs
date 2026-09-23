use std::collections::BTreeSet;

use serde::Deserialize;
use umber::{
    CompileAttemptResult, FileKind, FileRequest, FileRequestKey, NeedResources, ResolvedFile,
    ResourceDomain, ResourceRequest, ResourceResponse, RevisionId, SessionOptions, SourcePatch,
    VirtualCompileSession,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    schema: u32,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Case {
    name: String,
    #[serde(default)]
    initial_accepted: Option<InitialAccepted>,
    source: String,
    initial_hints: Vec<String>,
    steps: Vec<Step>,
    terminal: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InitialAccepted {
    source: String,
    terminal: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Step {
    need: ExpectedNeed,
    responses: Vec<ExpectedResponse>,
    #[serde(default)]
    cancel_before_responses: bool,
    #[serde(default)]
    reject_late_conflict: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExpectedNeed {
    required: Vec<String>,
    probes: Vec<String>,
    prefetch_hints: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpectedResponse {
    name: String,
    outcome: String,
    bytes: Option<String>,
}

#[test]
fn native_resource_transitions_follow_shared_cases() {
    let fixture: Fixture = serde_json::from_str(include_str!(
        "../../../../tests/resource-transition-cases.json"
    ))
    .expect("shared resource transitions are valid JSON");
    assert_eq!(fixture.schema, 1);
    let names: BTreeSet<_> = fixture
        .cases
        .iter()
        .map(|case| case.name.as_str())
        .collect();
    assert_eq!(names.len(), fixture.cases.len(), "duplicate case name");
    assert_eq!(
        names,
        BTreeSet::from([
            "required-positive-retry",
            "authoritative-missing-probe",
            "empty-speculation-then-demand",
            "cancel-pending-resource-patch",
        ])
    );

    for case in fixture.cases {
        assert_eq!(
            case.steps
                .iter()
                .filter(|step| step.cancel_before_responses)
                .count(),
            usize::from(case.initial_accepted.is_some()),
            "{}: cancellation requires exactly one accepted baseline",
            case.name
        );
        let hints = case
            .initial_hints
            .iter()
            .map(|name| file_request(name))
            .collect::<Vec<_>>();
        let mut session = VirtualCompileSession::new_standalone(SessionOptions {
            initial_prefetch_hints: (!hints.is_empty()).then(|| hints.into_boxed_slice()),
            ..SessionOptions::default()
        })
        .expect("native compile session");
        let initial_source = case
            .initial_accepted
            .as_ref()
            .map_or(case.source.as_str(), |accepted| accepted.source.as_str());
        session
            .add_user_file("main.tex", initial_source.as_bytes().to_vec())
            .expect("authored source");
        let baseline = case.initial_accepted.as_ref().map(|accepted| {
            let CompileAttemptResult::Complete(output) = session.compile_attempt() else {
                panic!("{}: baseline must be accepted", case.name);
            };
            assert!(String::from_utf8_lossy(&output.terminal).contains(&accepted.terminal));
            let revision = session.revision().expect("accepted baseline revision");
            let hash = session.content_hash().expect("accepted baseline hash");
            let observations = session
                .accepted_input_observations()
                .expect("accepted baseline observations");
            (output, revision, hash, observations)
        });
        if let (Some(accepted), Some((_, revision, hash, _))) = (&case.initial_accepted, &baseline)
        {
            session
                .apply_patch(SourcePatch {
                    next_revision: RevisionId::new(revision.raw() + 1),
                    base_revision: *revision,
                    expected_hash: *hash,
                    range: 0..accepted.source.len(),
                    replacement: case.source.clone(),
                })
                .expect("resource-waiting patch");
        }

        for (index, step) in case.steps.iter().enumerate() {
            let CompileAttemptResult::NeedResources(mut need) = session.compile_attempt() else {
                panic!("{} step {index}: expected resource need", case.name);
            };
            assert_need(&need, &step.need, &case.name, index);
            assert_eq!(
                session.accepted_input_observations(),
                baseline
                    .as_ref()
                    .map(|(_, _, _, observations)| observations.clone()),
                "{} step {index}: candidate was published",
                case.name
            );
            if step.cancel_before_responses {
                let (accepted_output, revision, hash, observations) = baseline
                    .as_ref()
                    .expect("cancellation needs accepted baseline");
                assert!(session.cancel_pending_patch(), "{}", case.name);
                assert!(!session.cancel_pending_patch(), "{}", case.name);
                assert_eq!(session.revision(), Some(*revision));
                assert_eq!(session.content_hash(), Some(*hash));
                assert_eq!(
                    session.accepted_input_observations(),
                    Some(observations.clone())
                );
                assert_eq!(session.resolved_file_count(), 0);
                assert_eq!(session.attempts(), 0);
                assert_eq!(
                    session.compile_attempt(),
                    CompileAttemptResult::Complete(accepted_output.clone()),
                    "{}: cancellation left a stale resource wait",
                    case.name
                );
                let accepted = case.initial_accepted.as_ref().expect("accepted source");
                session
                    .apply_patch(SourcePatch {
                        next_revision: RevisionId::new(revision.raw() + 1),
                        base_revision: *revision,
                        expected_hash: *hash,
                        range: 0..accepted.source.len(),
                        replacement: case.source.clone(),
                    })
                    .expect("retry same revision after cancellation");
                let CompileAttemptResult::NeedResources(retried) = session.compile_attempt() else {
                    panic!("{}: retried revision must request its resource", case.name);
                };
                assert_need(&retried, &step.need, &case.name, index);
                assert_eq!(
                    session.accepted_input_observations(),
                    Some(observations.clone())
                );
                need = retried;
            }
            let responses = step
                .responses
                .iter()
                .map(|response| make_response(response, &need))
                .collect::<Vec<_>>();
            if step.reject_late_conflict {
                let [ResourceResponse::File(first)] = responses.as_slice() else {
                    panic!("{}: conflict step needs one positive file", case.name);
                };
                let mut conflicting = first.clone();
                conflicting.bytes = b"conflicting-late-payload".to_vec().into();
                assert!(
                    session
                        .provide_resources(vec![
                            responses[0].clone(),
                            ResourceResponse::File(conflicting)
                        ])
                        .is_err(),
                    "{}: conflicting late response must reject the batch",
                    case.name
                );
                assert_eq!(session.resolved_file_count(), 0, "{}", case.name);
                assert!(session.accepted_input_observations().is_none());
            }
            session
                .provide_resources(responses)
                .unwrap_or_else(|error| panic!("{} step {index}: {error:?}", case.name));
        }

        let CompileAttemptResult::Complete(output) = session.compile_attempt() else {
            panic!("{}: expected accepted completion", case.name);
        };
        assert!(
            String::from_utf8_lossy(&output.terminal).contains(&case.terminal),
            "{}: missing accepted marker",
            case.name
        );
        assert!(
            session.accepted_input_observations().is_some(),
            "{}",
            case.name
        );
        if let Some((_, revision, _, _)) = baseline {
            assert_eq!(
                session.revision(),
                Some(RevisionId::new(revision.raw() + 1))
            );
        }
    }
}

fn file_request(name: &str) -> ResourceRequest {
    ResourceRequest::File(FileRequest::new(
        FileRequestKey::new(FileKind::TexInput, name).expect("file request key"),
        name,
    ))
}

fn names(requests: &[ResourceRequest]) -> Vec<String> {
    requests
        .iter()
        .map(|request| match request {
            ResourceRequest::File(file) => {
                assert_eq!(file.key().domain(), ResourceDomain::Tex);
                assert_eq!(file.key().kind(), FileKind::TexInput);
                file.key().name().to_owned()
            }
            other => panic!("shared case expected a TeX input, got {other:?}"),
        })
        .collect()
}

fn assert_need(actual: &NeedResources, expected: &ExpectedNeed, case: &str, index: usize) {
    assert_eq!(
        names(&actual.required),
        expected.required,
        "{case} step {index} required"
    );
    assert_eq!(
        names(&actual.probes),
        expected.probes,
        "{case} step {index} probes"
    );
    assert_eq!(
        names(&actual.prefetch_hints),
        expected.prefetch_hints,
        "{case} step {index} hints"
    );
}

fn make_response(expected: &ExpectedResponse, need: &NeedResources) -> ResourceResponse {
    let requested = need
        .required
        .iter()
        .chain(&need.probes)
        .chain(&need.prefetch_hints)
        .find_map(|request| match request {
            ResourceRequest::File(file) if file.key().name() == expected.name => Some(file.key()),
            _ => None,
        })
        .expect("response must match a request");
    match (expected.outcome.as_str(), expected.bytes.as_deref()) {
        ("file", Some(bytes)) => ResourceResponse::File(ResolvedFile {
            request: requested.clone(),
            virtual_path: format!("/texlive/tex/browser/{}", expected.name),
            bytes: bytes.as_bytes().to_vec().into(),
            expected_digest: None,
        }),
        ("unavailable", None) => ResourceResponse::FileUnavailable(requested.clone()),
        _ => panic!("invalid shared response outcome"),
    }
}
