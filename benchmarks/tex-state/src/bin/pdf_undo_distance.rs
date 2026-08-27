use tex_state::measurement::{HotCoreAllocator, retained_generation_census};
use tex_state::{PdfUndoDistanceMeasurement, PdfUndoDistancePhase, profile_pdf_undo_distance};

#[global_allocator]
static ALLOCATOR: HotCoreAllocator = HotCoreAllocator;

fn same_bounded_cost(short: PdfUndoDistancePhase, long: PdfUndoDistancePhase, phase: &str) {
    assert_eq!(
        short.allocations, long.allocations,
        "{phase} allocation count"
    );
    assert_eq!(
        short.requested_bytes, long.requested_bytes,
        "{phase} requested bytes"
    );
    assert_eq!(
        short.lifecycle_work, long.lifecycle_work,
        "{phase} CPU work"
    );
    assert_eq!(short.replay_work, 0, "{phase} short replay work");
    assert_eq!(long.replay_work, 0, "{phase} long replay work");
}

fn print_measurement(measurement: PdfUndoDistanceMeasurement) {
    for (phase, cost) in [
        ("open", measurement.open),
        ("first_mutation", measurement.first_mutation),
        ("reject", measurement.reject),
        ("accept", measurement.accept),
    ] {
        println!(
            "PDF_UNDO_DISTANCE distance={} phase={} elapsed_ns={} allocations={} requested_bytes={} lifecycle_work={} replay_work={} historical_lookup_probes={}",
            measurement.accepted_undo_distance,
            phase,
            cost.elapsed_ns,
            cost.allocations,
            cost.requested_bytes,
            cost.lifecycle_work,
            cost.replay_work,
            measurement.historical_lookup_probes,
        );
    }
}

fn main() {
    let short = profile_pdf_undo_distance(1_024);
    let long = profile_pdf_undo_distance(16_384);
    same_bounded_cost(short.open, long.open, "open");
    same_bounded_cost(short.first_mutation, long.first_mutation, "first_mutation");
    same_bounded_cost(short.reject, long.reject, "reject");
    same_bounded_cost(short.accept, long.accept, "accept");
    assert_eq!(
        short.historical_lookup_probes,
        long.historical_lookup_probes
    );
    print_measurement(short);
    print_measurement(long);
    std::hint::black_box(retained_generation_census());
}
