use super::*;

const ROUTINE_CYCLES: usize = 8;
const STRESS_WARMUP_CYCLES: usize = 64;
const STRESS_CYCLES: usize = 2_048;
const STRESS_MILESTONE_CYCLES: usize = 128;
const RSS_TOLERANCE_BYTES: usize = 64 * 1024 * 1024;

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ActiveOwners {
    token_objects: usize,
    token_bytes: usize,
    macro_bodies: usize,
    macro_body_bytes: usize,
    macro_definitions: usize,
    macro_definition_bytes: usize,
    glue_objects: usize,
    glue_bytes: usize,
    provenance_records: usize,
    provenance_lists: usize,
    provenance_entries: usize,
    source_regions: usize,
    source_bytes: usize,
    journal_entries: usize,
}

impl From<tex_state::TestingOwnershipCensus> for ActiveOwners {
    fn from(census: tex_state::TestingOwnershipCensus) -> Self {
        Self {
            token_objects: census.token_lists.live_objects,
            token_bytes: census.token_lists.logical_bytes,
            macro_bodies: census.macro_bodies.live_objects,
            macro_body_bytes: census.macro_bodies.logical_bytes,
            macro_definitions: census.macro_definitions.live_objects,
            macro_definition_bytes: census.macro_definitions.logical_bytes,
            glue_objects: census.glue_specs.live_objects,
            glue_bytes: census.glue_specs.logical_bytes,
            provenance_records: census.provenance_records,
            provenance_lists: census.provenance_lists,
            provenance_entries: census.provenance_entries,
            source_regions: census.source_regions,
            source_bytes: census.source_bytes,
            journal_entries: census.journal_entries,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct PlateauMilestone {
    active: ActiveOwners,
    physical: tex_state::TestingOwnershipCensus,
    retention: RetentionMetrics,
    rss_bytes: Option<usize>,
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
    let before_owners = session
        .substrate
        .as_ref()
        .expect("accepted substrate")
        .testing_ownership_census();
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
    let suspended = candidate
        .universe
        .testing_private_revision_domain_stats()
        .expect("rejected candidate retains one private domain");
    assert!(
        !suspended.3,
        "resource suspension closes the operation mark"
    );
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
    assert_eq!(session.retention_metrics(), Some(before_retention));
    assert_eq!(
        session
            .history()
            .last()
            .expect("accepted checkpoint")
            .state_hash(),
        before_state_hash
    );
    assert_eq!(
        session
            .substrate
            .as_ref()
            .expect("accepted substrate")
            .testing_ownership_census(),
        before_owners
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

fn milestone(session: &Session, rss: bool) -> PlateauMilestone {
    let physical = session
        .substrate
        .as_ref()
        .expect("accepted substrate")
        .testing_ownership_census();
    PlateauMilestone {
        active: physical.into(),
        physical,
        retention: session.retention_metrics().expect("accepted retention"),
        rss_bytes: rss.then(process_rss_bytes).flatten(),
    }
}

fn assert_budgeted_plateau(baseline: PlateauMilestone, current: PlateauMilestone) {
    assert_eq!(
        current.active, baseline.active,
        "live owners must plateau exactly"
    );
    assert_eq!(
        current.physical, baseline.physical,
        "weak indexes, reusable slots, journals, and provenance must plateau exactly"
    );
    assert_eq!(
        current.retention.checkpoint_root_bytes, baseline.retention.checkpoint_root_bytes,
        "generation charge must plateau at live roots plus the fragment-history budget"
    );
    assert_eq!(
        current.retention.diagnostic_bytes, baseline.retention.diagnostic_bytes,
        "diagnostic ownership must plateau at the live layout plus the fragment-history budget"
    );
    assert_eq!(
        current.retention.output_bytes,
        baseline.retention.output_bytes
    );
    assert!(current.physical.token_lists.index_keys <= 1_024);
    assert!(current.physical.macro_bodies.index_keys <= 1_024);
    assert!(current.physical.macro_definitions.index_keys <= 1_024);
    assert!(current.physical.glue_specs.index_keys <= 1_024);
    assert!(current.physical.node_weak_entries <= 64);
    assert!(current.physical.node_weak_capacity <= 64);
    assert!(current.physical.token_lists.max_bucket_capacity <= 64);
    assert!(current.physical.macro_bodies.max_bucket_capacity <= 64);
    assert!(current.physical.macro_definitions.max_bucket_capacity <= 64);
    assert!(current.physical.glue_specs.max_bucket_capacity <= 64);
    if let (Some(baseline_rss), Some(current_rss)) = (baseline.rss_bytes, current.rss_bytes) {
        assert!(
            current_rss <= baseline_rss.saturating_add(RSS_TOLERANCE_BYTES),
            "process RSS exceeded diagnostic tolerance: baseline={baseline_rss}, current={current_rss}"
        );
    }
}

#[cfg(target_os = "linux")]
#[allow(clippy::disallowed_methods)] // Linux-only explicit RSS stress telemetry owns this host read.
fn process_rss_bytes() -> Option<usize> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let kib = status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))?
        .split_ascii_whitespace()
        .next()?
        .parse::<usize>()
        .ok()?;
    kib.checked_mul(1024)
}

#[cfg(not(target_os = "linux"))]
fn process_rss_bytes() -> Option<usize> {
    None
}

fn run_long_session(
    cycles: usize,
    warmup: usize,
    milestone_cycles: usize,
    rss: bool,
) -> (PlateauMilestone, PlateauMilestone, WorkReceipt) {
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
    let baseline = milestone(&session, rss);
    let mut final_milestone = baseline;
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
        let current = milestone(&session, rss);
        assert_budgeted_plateau(baseline, current);
        final_milestone = current;
        if let Some(expected) = expected_interval {
            assert_eq!(receipt, expected, "equal-work interval receipt drifted");
        } else {
            expected_interval = Some(receipt);
        }
        receipt = WorkReceipt::default();
    }
    (
        baseline,
        final_milestone,
        expected_interval.expect("at least one equal-work milestone"),
    )
}

#[test]
fn long_session_ownership_smoke_matches_clean_and_plateaus() {
    let _ = run_long_session(ROUTINE_CYCLES, 2, 2, false);
}

#[test]
#[ignore = "explicit 2048 accepted/rejected patch ownership and RSS tier"]
fn long_session_thousands_plateau_at_equal_work_milestones() {
    let (baseline, final_milestone, receipt) = run_long_session(
        STRESS_CYCLES,
        STRESS_WARMUP_CYCLES,
        STRESS_MILESTONE_CYCLES,
        true,
    );
    eprintln!(
        "long-session plateau baseline={baseline:?} final={final_milestone:?} equal_work={receipt:?}"
    );
}
