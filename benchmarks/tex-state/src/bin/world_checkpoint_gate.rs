use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicU64, Ordering};

use tex_state::{SourceFontCheckpointHarness, World};
use tex_state::world::{PrintSink, StreamSlot};

const SMALL_UNITS: usize = 1;
const LARGE_UNITS: usize = 64;

struct CountingAllocator;

static ALLOCATION_CALLS: AtomicU64 = AtomicU64::new(0);
static REQUESTED_BYTES: AtomicU64 = AtomicU64::new(0);

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

// SAFETY: every allocator operation delegates unchanged to `System`; the
// relaxed counters are observation-only and cannot affect allocation results.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: delegated with the caller-provided valid layout.
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            ALLOCATION_CALLS.fetch_add(1, Ordering::Relaxed);
            REQUESTED_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: delegated with the caller-provided valid layout.
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            ALLOCATION_CALLS.fetch_add(1, Ordering::Relaxed);
            REQUESTED_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: delegated with the allocation's original pointer and layout.
        unsafe { System.dealloc(pointer, layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: delegated with the allocation's original pointer and layout.
        let new_pointer = unsafe { System.realloc(pointer, layout, new_size) };
        if !new_pointer.is_null() && new_size > layout.size() {
            ALLOCATION_CALLS.fetch_add(1, Ordering::Relaxed);
            REQUESTED_BYTES.fetch_add((new_size - layout.size()) as u64, Ordering::Relaxed);
        }
        new_pointer
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AllocationStats {
    calls: u64,
    bytes: u64,
}

fn main() {
    let small = run_case(SMALL_UNITS);
    let large = run_case(LARGE_UNITS);
    let small_source_font = run_source_font_case(SMALL_UNITS);
    let large_source_font = run_source_font_case(LARGE_UNITS);
    let one_boundary = run_boundary_case(1);
    let many_boundaries = run_boundary_case(32);

    assert_eq!(small.capture, AllocationStats { calls: 0, bytes: 0 });
    assert_eq!(small.clone, AllocationStats { calls: 0, bytes: 0 });
    assert_eq!(small.restore, AllocationStats { calls: 0, bytes: 0 });
    assert_eq!(large.capture, small.capture, "capture scales with effects");
    assert_eq!(large.clone, small.clone, "mark clone scales with effects");
    assert_eq!(large.restore, small.restore, "restore scales with effects");
    assert_eq!(large.fork, small.fork, "fork scales with accepted effects");
    assert_eq!(large.mutate, small.mutate, "candidate mutation copies no accepted stream/effect prefix");
    assert_eq!(small_source_font.capture, AllocationStats { calls: 0, bytes: 0 });
    assert_eq!(small_source_font.clone, AllocationStats { calls: 0, bytes: 0 });
    assert_eq!(large_source_font.capture, small_source_font.capture);
    assert_eq!(large_source_font.clone, small_source_font.clone);
    assert_eq!(large_source_font.fork, small_source_font.fork);
    assert_eq!(large_source_font.mutate, small_source_font.mutate);
    assert_eq!(many_boundaries.0, one_boundary.0, "World fork scales with retained boundaries");
    assert_eq!(many_boundaries.1, one_boundary.1, "source/font fork scales with retained boundaries");

    println!("world checkpoint allocation gate");
    println!(
        "small_units={SMALL_UNITS} capture={:?} clone={:?} fork={:?} mutate={:?} restore={:?} retained_payload_bytes={}",
        small.capture, small.clone, small.fork, small.mutate, small.restore, small.retained_payload_bytes
    );
    println!(
        "retained_boundaries one world_fork={:?} source_font_fork={:?}; many=32 world_fork={:?} source_font_fork={:?}",
        one_boundary.0, one_boundary.1, many_boundaries.0, many_boundaries.1
    );
    println!(
        "large_units={LARGE_UNITS} capture={:?} clone={:?} fork={:?} mutate={:?} restore={:?} retained_payload_bytes={}",
        large.capture, large.clone, large.fork, large.mutate, large.restore, large.retained_payload_bytes
    );
    println!(
        "source_font small_units={SMALL_UNITS} capture={:?} clone={:?} fork={:?} mutate={:?} retained_payload_bytes={}",
        small_source_font.capture,
        small_source_font.clone,
        small_source_font.fork,
        small_source_font.mutate,
        small_source_font.retained_payload_bytes,
    );
    println!(
        "source_font large_units={LARGE_UNITS} capture={:?} clone={:?} fork={:?} mutate={:?} retained_payload_bytes={}",
        large_source_font.capture,
        large_source_font.clone,
        large_source_font.fork,
        large_source_font.mutate,
        large_source_font.retained_payload_bytes,
    );
}

#[derive(Clone, Copy)]
struct SourceFontCaseStats {
    capture: AllocationStats,
    clone: AllocationStats,
    fork: AllocationStats,
    mutate: AllocationStats,
    retained_payload_bytes: usize,
}

fn run_source_font_case(units: usize) -> SourceFontCaseStats {
    let source = SourceFontCheckpointHarness::with_units(units);
    let retained_payload_bytes = source.retained_payload_bytes();
    let (checkpoint, capture) = measure(|| source.checkpoint());
    let (cloned, clone) = measure(|| checkpoint.clone());
    let (mut candidate, fork) = measure(|| source.fork(cloned));
    let ((), mutate) = measure(|| candidate.append_unit(units));
    black_box((source, candidate));
    SourceFontCaseStats {
        capture,
        clone,
        fork,
        mutate,
        retained_payload_bytes,
    }
}

fn run_boundary_case(boundaries: usize) -> (AllocationStats, AllocationStats) {
    let mut world = World::memory();
    world.profile_begin_retained_session();
    let mut source_font = SourceFontCheckpointHarness::with_units(0);
    for boundary in 0..boundaries {
        world.record_special("boundary", vec![boundary as u8; 16]);
        let world_mark = world.profile_checkpoint_capture();
        world = world.profile_checkpoint_fork(&world_mark);
        source_font.append_unit(boundary);
        let source_font_mark = source_font.checkpoint();
        source_font = source_font.fork(source_font_mark);
    }
    let world_mark = world.profile_checkpoint_capture();
    let source_font_mark = source_font.checkpoint();
    let (_, world_fork) = measure(|| world.profile_checkpoint_fork(&world_mark));
    let (_, source_font_fork) = measure(|| source_font.fork(source_font_mark));
    (world_fork, source_font_fork)
}

#[derive(Clone, Copy)]
struct CaseStats {
    capture: AllocationStats,
    clone: AllocationStats,
    fork: AllocationStats,
    mutate: AllocationStats,
    restore: AllocationStats,
    retained_payload_bytes: usize,
}

fn run_case(units: usize) -> CaseStats {
    let mut world = World::memory();
    world.profile_begin_retained_session();
    world.open_out(StreamSlot::new(0), "world-checkpoint.out");
    for index in 0..units {
        let path = format!("input-{index:04}.tex");
        world
            .set_memory_file(&path, vec![index as u8; 96])
            .expect("profiling input is seeded");
        black_box(world.read_file(&path).expect("profiling input is read"));
        world
            .record_input_dependency(
                &path,
                tex_state::InputDependencyOutcome::Missing,
                tex_state::InputDependencyAccess::AuthoritativeProbe,
            )
            .expect("profiling dependency is recorded");
        world.write_text(PrintSink::Stream(StreamSlot::new(0)), "bounded-line");
        world.record_special("checkpoint", vec![index as u8; 128]);
    }
    world.profile_publish_artifact(vec![0x5a; units * 256]);

    let (checkpoint, capture) = measure(|| world.profile_checkpoint_capture());
    let (cloned, clone) = measure(|| checkpoint.clone());
    let (mut candidate, fork) = measure(|| world.profile_checkpoint_fork(&cloned));
    let ((), mutate) = measure(|| {
        candidate.write_text(PrintSink::Stream(StreamSlot::new(0)), "candidate-line");
        candidate
            .record_input_dependency(
                "candidate.tex",
                tex_state::InputDependencyOutcome::Missing,
                tex_state::InputDependencyAccess::AuthoritativeProbe,
            )
            .expect("candidate dependency is recorded");
    });
    let ((), restore) = measure(|| world.profile_checkpoint_restore(&checkpoint));
    let retained_payload_bytes = world.profile_retained_checkpoint_bytes();

    black_box((world, checkpoint, cloned, candidate));
    CaseStats {
        capture,
        clone,
        fork,
        mutate,
        restore,
        retained_payload_bytes,
    }
}

fn measure<T>(operation: impl FnOnce() -> T) -> (T, AllocationStats) {
    let calls = ALLOCATION_CALLS.load(Ordering::Relaxed);
    let bytes = REQUESTED_BYTES.load(Ordering::Relaxed);
    let value = operation();
    black_box(&value);
    (
        value,
        AllocationStats {
            calls: ALLOCATION_CALLS.load(Ordering::Relaxed) - calls,
            bytes: REQUESTED_BYTES.load(Ordering::Relaxed) - bytes,
        },
    )
}
