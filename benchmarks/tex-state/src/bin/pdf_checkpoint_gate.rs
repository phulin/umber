use std::time::Instant;

use tex_state::measurement::{
    HotCoreAllocationOwner, HotCoreAllocator, hot_core_allocation_scope,
    hot_core_thread_allocation_measurement,
};
use tex_state::scaled::Scaled;
use tex_state::{
    ContentHash, PdfExternalImageDimensions, PdfExternalImageMetadata, PdfExternalImageSource,
    PdfRasterColorSpace, PdfRasterFormat, PdfRasterImageMetadata, with_universe,
};
use tex_state_benchmarks::engine_budget;

#[global_allocator]
static ALLOCATOR: HotCoreAllocator = HotCoreAllocator;

const ITERATIONS: usize = 1_000_000;

fn main() {
    for payload_bytes in [1, 64 * 1024 * 1024] {
        let result = with_universe(engine_budget(), |universe| {
            universe
                .command_context()
                .expect("PDF benchmark context")
                .allocate_pdf_external_image(
                    PdfExternalImageSource {
                        identity: ContentHash::new([7; 32]),
                        metadata: PdfExternalImageMetadata::Raster(PdfRasterImageMetadata {
                            format: PdfRasterFormat::Png,
                            width: 1,
                            height: 1,
                            bits_per_component: 8,
                            color_space: PdfRasterColorSpace::Gray,
                            alpha: false,
                            png_color_type: Some(0),
                        }),
                        natural_width: Scaled::from_raw(1),
                        natural_height: Scaled::from_raw(1),
                        bytes: vec![3; payload_bytes],
                    },
                    PdfExternalImageDimensions {
                        width: Scaled::from_raw(1),
                        height: Scaled::from_raw(1),
                        depth: Scaled::from_raw(0),
                    },
                    0,
                )
                .expect("PDF benchmark image");

            let owner = HotCoreAllocationOwner::GenerationBoundary;
            let before = hot_core_thread_allocation_measurement(owner);
            let capture_start = Instant::now();
            let capture_checksum = {
                let _scope = hot_core_allocation_scope(owner);
                universe.profile_pdf_checkpoint_capture(ITERATIONS)
            };
            let capture_elapsed = capture_start.elapsed();
            let after_capture = hot_core_thread_allocation_measurement(owner);
            let restore_start = Instant::now();
            let restore_checksum = {
                let _scope = hot_core_allocation_scope(owner);
                universe.profile_pdf_checkpoint_restore(ITERATIONS)
            };
            let restore_elapsed = restore_start.elapsed();
            let after_restore = hot_core_thread_allocation_measurement(owner);
            (
                capture_checksum ^ restore_checksum,
                capture_elapsed,
                restore_elapsed,
                after_capture.calls - before.calls,
                after_capture.requested_bytes - before.requested_bytes,
                after_restore.calls - after_capture.calls,
                after_restore.requested_bytes - after_capture.requested_bytes,
                universe.profile_pdf_payload_bytes(),
            )
        })
        .expect("PDF benchmark universe");

        println!(
            "PDF_CHECKPOINT_GATE payload_bytes={} iterations={} capture_ns_per_op={} restore_ns_per_op={} capture_allocations={} capture_requested_bytes={} restore_allocations={} restore_requested_bytes={} retained_payload_bytes={} checksum={}",
            payload_bytes,
            ITERATIONS,
            result.1.as_nanos() / ITERATIONS as u128,
            result.2.as_nanos() / ITERATIONS as u128,
            result.3,
            result.4,
            result.5,
            result.6,
            result.7,
            result.0,
        );
    }
}
