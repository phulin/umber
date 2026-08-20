use super::*;

const ROUTINE_CYCLES: usize = 8;
const STRESS_WARMUP_CYCLES: usize = 64;
const STRESS_CYCLES: usize = 2_048;
const STRESS_MILESTONE_CYCLES: usize = 128;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct WorkReceipt {
    accepted_patches: u64,
    rejected_patches: u64,
    resource_retries: u64,
    accepted_checkpoints: u64,
    delivered_commands: u64,
    cumulative_fuel: u64,
}

impl WorkReceipt {
    fn add_telemetry(&mut self, telemetry: tex_exec::ExecutionTelemetry) {
        self.resource_retries = self
            .resource_retries
            .saturating_add(telemetry.local_step_retries);
        self.delivered_commands = self
            .delivered_commands
            .saturating_add(telemetry.advance_calls);
        self.cumulative_fuel = self
            .cumulative_fuel
            .saturating_add(telemetry.cumulative_fuel);
    }
}

fn long_session_source(value: u8) -> String {
    assert!(matches!(value, b'1' | b'2'));
    format!(
        "\\def\\live#1{{#1}}\\count0={}\\begingroup\\def\\live#1{{#1#1}}\\skip0=1pt plus 1fil\\message{{group}}\\endgroup\\shipout\\vbox{{\\hrule height1pt width10pt}}\n\\end",
        char::from(value)
    )
}

fn value_offset(source: &str) -> usize {
    source.find("\\count0=").expect("count assignment") + "\\count0=".len()
}

fn complete_accepted_patch(
    session: &mut Session,
    next_revision: RevisionId,
    replacement: u8,
    receipt: &mut WorkReceipt,
) -> AcceptedOutput {
    let source = session.source.clone();
    let offset = value_offset(&source);
    let mut candidate = session
        .start_advance_candidate(
            next_revision,
            Edit {
                base_revision: session.revision(),
                expected_hash: session.content_hash(),
                range: offset..offset + 1,
                replacement: char::from(replacement).to_string(),
            },
        )
        .expect("accepted patch candidate starts");
    assert!(matches!(
        candidate
            .drive_with_resource_resolvers(&mut StagedResourceHost::default(), &Cancellation::new())
            .expect("accepted patch executes"),
        RevisionCandidateResult::Complete
    ));
    receipt.add_telemetry(candidate.execution_telemetry());
    let transaction = session
        .prepare_revision_candidate(candidate)
        .expect("accepted patch prepares");
    let output = session
        .accept_revision(transaction)
        .expect("accepted patch commits");
    receipt.accepted_patches += 1;
    receipt.accepted_checkpoints = receipt
        .accepted_checkpoints
        .saturating_add(session.history().len() as u64);
    output
}

fn complete_retried_rejected_patch(session: &Session, receipt: &mut WorkReceipt) {
    let before_retention = session.retention_metrics().expect("accepted retention");
    let before_output = session.output(ReuseMetrics::default(), before_retention);
    let before_state_hash = session
        .history()
        .last()
        .expect("accepted checkpoint")
        .state_hash();
    let source = session.source.clone();
    let end = source.rfind("\\end").expect("terminal end");
    let mut candidate = session
        .start_advance_candidate(
            RevisionId::new(session.revision().raw() + 1),
            Edit {
                base_revision: session.revision(),
                expected_hash: session.content_hash(),
                range: end..source.len(),
                replacement: "\\input missing \\end".to_owned(),
            },
        )
        .expect("rejected patch candidate starts");
    assert!(matches!(
        &candidate.kind,
        RevisionCandidateKind::Incremental { setup, .. }
            if setup.execution_path == RevisionExecutionPath::SlowEdit
    ));
    let mut inputs = StagedInputResolver::default();
    assert!(matches!(
        candidate
            .drive_with_resource_resolvers(
                &mut DecliningStagedResourceHost::new(&mut inputs),
                &Cancellation::new(),
            )
            .expect("missing resource suspends"),
        RevisionCandidateResult::AwaitingResources(ResourceNeed::Input { ref name, .. })
            if name == "missing.tex"
    ));
    inputs
        .files
        .insert("missing".to_owned(), "\\relax".to_owned());
    assert!(matches!(
        candidate
            .drive_with_resource_resolvers(
                &mut DecliningStagedResourceHost::new(&mut inputs),
                &Cancellation::new(),
            )
            .expect("resource retry completes"),
        RevisionCandidateResult::Complete
    ));
    let telemetry = candidate.execution_telemetry();
    assert_eq!(telemetry.suspensions, 1, "receipt records one suspension");
    assert_eq!(telemetry.local_step_retries, 1);
    assert_eq!(telemetry.replayed_delivered_tokens, 0);
    assert_eq!(telemetry.replayed_dispatches, 0);
    receipt.add_telemetry(telemetry);
    receipt.rejected_patches += 1;
    drop(candidate);
    assert_eq!(
        session
            .history()
            .last()
            .expect("accepted checkpoint")
            .state_hash(),
        before_state_hash
    );
    let after_output = session.output(ReuseMetrics::default(), before_retention);
    assert_output_eq(
        &after_output,
        &before_output,
        "rejected resource-retried patch",
    );
}

fn assert_output_eq(actual: &AcceptedOutput, expected: &AcceptedOutput, label: &str) {
    assert_eq!(actual.effects, expected.effects, "{label}: effects");
    assert_eq!(actual.artifacts, expected.artifacts, "{label}: artifacts");
    assert_eq!(actual.dvi_pages, expected.dvi_pages, "{label}: DVI plans");
    assert_eq!(
        actual.dvi_bytes().expect("actual DVI"),
        expected.dvi_bytes().expect("expected DVI"),
        "{label}: DVI bytes"
    );
}

fn assert_clean_rebuild_equivalence(session: &Session, output: &AcceptedOutput) {
    let mut cold = Session::start(
        template(),
        "long-session-cold",
        session.revision(),
        session.source.clone(),
        0,
    )
    .expect("clean comparison starts");
    let cold_output = cold.cold().expect("clean comparison executes");
    assert_output_eq(output, &cold_output, "long-session clean rebuild");
    let actual = session.history().last().expect("accepted checkpoint");
    let expected = cold.history().last().expect("clean checkpoint");
    assert_eq!(actual.key().boundary, expected.key().boundary);
    assert_eq!(actual.state_hash(), expected.state_hash());
    assert!(
        actual
            .checkpoint()
            .exact_future_state_matches(expected.checkpoint()),
        "reachable future state differs from clean rebuild"
    );
}

fn run_long_session(cycles: usize, warmup: usize, milestone_cycles: usize) -> WorkReceipt {
    let initial = long_session_source(b'1');
    let mut session = Session::start(template(), "long-session", RevisionId::new(1), initial, 0)
        .expect("long session starts");
    let mut output = session.cold().expect("initial revision accepts");
    let mut receipt = WorkReceipt::default();
    for cycle in 1..=warmup {
        let replacement = if cycle % 2 == 0 { b'1' } else { b'2' };
        let next_revision = RevisionId::new(session.revision().raw() + 1);
        output = complete_accepted_patch(&mut session, next_revision, replacement, &mut receipt);
        complete_retried_rejected_patch(&session, &mut receipt);
    }
    if warmup % 2 == 1 {
        let next_revision = RevisionId::new(session.revision().raw() + 1);
        output = complete_accepted_patch(&mut session, next_revision, b'1', &mut receipt);
        complete_retried_rejected_patch(&session, &mut receipt);
    }
    assert_clean_rebuild_equivalence(&session, &output);
    let mut expected_interval = None;
    receipt = WorkReceipt::default();

    for cycle in 1..=cycles {
        let replacement = if cycle % 2 == 0 { b'1' } else { b'2' };
        let next_revision = RevisionId::new(session.revision().raw() + 1);
        output = complete_accepted_patch(&mut session, next_revision, replacement, &mut receipt);
        complete_retried_rejected_patch(&session, &mut receipt);
        if cycle % milestone_cycles != 0 {
            continue;
        }
        assert_eq!(replacement, b'1', "milestones must have equal live work");
        assert_eq!(
            session.history().len(),
            2,
            "history budget stays protected-only"
        );
        assert_clean_rebuild_equivalence(&session, &output);
        if let Some(expected) = expected_interval {
            assert_eq!(receipt, expected, "equal-work interval receipt drifted");
        } else {
            expected_interval = Some(receipt);
        }
        receipt = WorkReceipt::default();
    }
    expected_interval.expect("at least one equal-work milestone")
}

#[test]
fn long_session_revisions_and_retries_match_clean_semantics() {
    let _ = run_long_session(ROUTINE_CYCLES, 2, 2);
}

#[test]
#[ignore = "explicit 2048 accepted/rejected patch semantic parity tier"]
fn long_session_thousands_match_clean_at_equal_work_milestones() {
    let receipt = run_long_session(STRESS_CYCLES, STRESS_WARMUP_CYCLES, STRESS_MILESTONE_CYCLES);
    eprintln!("long-session equal_work={receipt:?}");
}
