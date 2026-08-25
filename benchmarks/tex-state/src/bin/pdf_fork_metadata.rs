use tex_state::measurement::{HotCoreAllocator, retained_generation_census};
use tex_state::{PdfForkProfileFamily, profile_pdf_fork_family};

#[global_allocator]
static ALLOCATOR: HotCoreAllocator = HotCoreAllocator;

fn main() {
    for rows in [1_000, 10_000] {
        let iterations = if rows == 1_000 { 100 } else { 20 };
        for family in PdfForkProfileFamily::ALL {
            let measurement = profile_pdf_fork_family(family, rows, iterations);
            println!(
                "PDF_FORK_METADATA family={} rows={} iterations={} ns_per_fork={} allocations_per_fork={} requested_bytes_per_fork={}",
                family.name(),
                rows,
                iterations,
                measurement.elapsed_ns / iterations as u128,
                measurement.allocations / iterations as u64,
                measurement.requested_bytes / iterations as u64,
            );
        }
    }
    std::hint::black_box(retained_generation_census());
}
