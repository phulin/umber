use std::collections::BTreeSet;

use serde::Deserialize;
use umber::{
    CompileAttemptResult, FileKind, FileRequest, FileRequestKey, NeedResources, ResolvedFile,
    ResourceDomain, ResourceRequest, ResourceResponse, SessionOptions, VirtualCompileSession,
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
    source: String,
    initial_hints: Vec<String>,
    steps: Vec<Step>,
    terminal: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Step {
    need: ExpectedNeed,
    responses: Vec<ExpectedResponse>,
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
        ])
    );

    for case in fixture.cases {
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
        session
            .add_user_file("main.tex", case.source.into_bytes())
            .expect("authored source");

        for (index, step) in case.steps.iter().enumerate() {
            let CompileAttemptResult::NeedResources(need) = session.compile_attempt() else {
                panic!("{} step {index}: expected resource need", case.name);
            };
            assert_need(&need, &step.need, &case.name, index);
            assert!(
                session.accepted_input_observations().is_none(),
                "{} step {index}: candidate was published",
                case.name
            );
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
