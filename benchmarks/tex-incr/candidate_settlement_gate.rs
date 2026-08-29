use std::path::Path;

use tex_exec::{
    Cancellation, ResourceFulfillment, ResourceHost, ResourceNeed, ResourceOutcome, ResourceWorld,
};
use tex_incr::{
    Edit, RevisionCandidate, RevisionCandidateResult, RevisionId, Session, new_reachability_store,
};
use tex_state::ContentHash;
use tex_state::measurement::{
    HotCoreAllocationMeasurement, HotCoreAllocationOwner, HotCoreAllocator,
    hot_core_thread_allocation_measurement,
};

#[global_allocator]
static ALLOCATOR: HotCoreAllocator = HotCoreAllocator;

const SOURCE: &str = "A\\par\nB\\par\nC\\par\\end";

struct DirectResourceHost;

impl ResourceHost for DirectResourceHost {
    fn fulfill(&mut self, world: &mut ResourceWorld<'_>, need: &ResourceNeed) -> ResourceOutcome {
        match need {
            ResourceNeed::Input { name, .. } => world.read_file(Path::new(name)).ok().map_or(
                ResourceOutcome::Unavailable,
                |content| {
                    ResourceOutcome::Fulfilled(ResourceFulfillment::world_input(name, content))
                },
            ),
            ResourceNeed::InputProbe { request } => world
                .read_file(Path::new(&request.name))
                .ok()
                .map_or(ResourceOutcome::Unavailable, |content| {
                    ResourceOutcome::Fulfilled(ResourceFulfillment::world_input_probe(
                        request.clone(),
                        content,
                    ))
                }),
            ResourceNeed::Font { .. } | ResourceNeed::PdfImage { .. } => {
                ResourceOutcome::Unavailable
            }
        }
    }
}

fn main() {
    // Initialize the thread-local measurement path before either production
    // transition enters its internally attributed allocation scope.
    let _ = measurement();

    let (mut rejected_session, rejected, rejected_page_before) = prepared_non_job_start();
    let rejected_allocations = measure(|| rejected.reject());
    let rejected_page_after = rejected_session
        .page_material_counters()
        .expect("returned accepted counters")
        .expect("returned accepted generation");
    assert_zero("reject", rejected_allocations);
    assert_eq!(
        rejected_page_after.source_nodes_copied, rejected_page_before.source_nodes_copied,
        "production reject copied page material"
    );

    let (mut accepted_session, mut accepted, _) = prepared_non_job_start();
    let accepted_page_before = accepted
        .page_material_counters()
        .expect("candidate counters");
    let (accepted_allocations, _output) = measure_with_output(|| {
        accepted_session
            .accept_revision(accepted)
            .expect("production acceptance")
    });
    let accepted_page_after = accepted_session
        .page_material_counters()
        .expect("accepted counters")
        .expect("accepted generation");
    assert_zero("accept", accepted_allocations);
    assert_eq!(
        accepted_page_after.source_nodes_copied, accepted_page_before.source_nodes_copied,
        "production accept copied page material"
    );

    println!(
        "CANDIDATE_SETTLEMENT_GATE accept_allocations=0 accept_bytes=0 \
         reject_allocations=0 reject_bytes=0 accept_page_copies=0 reject_page_copies=0"
    );
}

fn prepared_non_job_start() -> (
    Session<'static>,
    tex_incr::RevisionTransaction<'static>,
    tex_state::fork_arena::ForkArenaCounters,
) {
    let store = Box::leak(Box::new(new_reachability_store()));
    let mut session = Session::start(
        store,
        "candidate-settlement-gate",
        RevisionId::new(1),
        SOURCE,
        usize::MAX,
    )
    .expect("session starts");
    session.cold().expect("accepted prior");
    let accepted_page_before = session
        .page_material_counters()
        .expect("accepted counters")
        .expect("accepted generation");
    let edit_position = SOURCE.find('C').expect("third paragraph");
    let edit = Edit {
        base_revision: session.revision(),
        expected_hash: ContentHash::from_bytes(SOURCE.as_bytes()),
        range: edit_position..edit_position,
        replacement: "\\relax ".to_owned(),
    };
    let mut candidate = session
        .start_advance_candidate(RevisionId::new(2), edit)
        .expect("candidate starts");
    drive(&mut candidate);
    let transaction = session
        .prepare_revision_candidate(candidate)
        .expect("candidate prepares");
    assert!(
        transaction
            .reuse()
            .restart_boundary
            .is_some_and(|key| key.boundary != tex_exec::EngineBoundary::JobStart),
        "gate requires a real retained non-JobStart candidate"
    );
    (session, transaction, accepted_page_before)
}

fn drive(candidate: &mut RevisionCandidate<'_>) {
    match candidate
        .drive_with_resource_resolvers(&mut DirectResourceHost, &Cancellation::new())
        .expect("candidate drive")
    {
        RevisionCandidateResult::Complete => {}
        RevisionCandidateResult::AwaitingResources(need) => {
            panic!("gate candidate unexpectedly suspended: {need:?}")
        }
    }
}

fn measurement() -> HotCoreAllocationMeasurement {
    hot_core_thread_allocation_measurement(HotCoreAllocationOwner::GenerationBoundary)
}

fn measure(action: impl FnOnce()) -> HotCoreAllocationMeasurement {
    let before = measurement();
    action();
    allocation_delta(measurement(), before)
}

fn measure_with_output<T>(action: impl FnOnce() -> T) -> (HotCoreAllocationMeasurement, T) {
    let before = measurement();
    let output = action();
    (allocation_delta(measurement(), before), output)
}

fn allocation_delta(
    after: HotCoreAllocationMeasurement,
    before: HotCoreAllocationMeasurement,
) -> HotCoreAllocationMeasurement {
    HotCoreAllocationMeasurement {
        calls: after.calls.saturating_sub(before.calls),
        requested_bytes: after.requested_bytes.saturating_sub(before.requested_bytes),
    }
}

fn assert_zero(transition: &str, measurement: HotCoreAllocationMeasurement) {
    assert_eq!(
        measurement.calls, 0,
        "production {transition} allocated {} times",
        measurement.calls
    );
    assert_eq!(
        measurement.requested_bytes, 0,
        "production {transition} requested {} bytes",
        measurement.requested_bytes
    );
}
