use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use criterion::{
    BatchSize, BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main,
};
use tex_state::FragmentStore;

const FRAGMENT_COUNTS: [usize; 4] = [128, 512, 2_048, 8_192];

struct CountingAllocator;

static COUNT_ALLOCATIONS: AtomicBool = AtomicBool::new(false);
static ALLOCATION_COUNT: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNT_ALLOCATIONS.load(Ordering::Relaxed) {
            ALLOCATION_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        if COUNT_ALLOCATIONS.load(Ordering::Relaxed) {
            ALLOCATION_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.realloc(pointer, layout, size) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn counted_allocations<T>(operation: impl FnOnce() -> T) -> (T, usize) {
    ALLOCATION_COUNT.store(0, Ordering::Relaxed);
    COUNT_ALLOCATIONS.store(true, Ordering::Release);
    let result = operation();
    COUNT_ALLOCATIONS.store(false, Ordering::Release);
    (result, ALLOCATION_COUNT.load(Ordering::Relaxed))
}

fn accepted_fragment_edit_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("accepted_fragment_edit_scaling");
    for count in FRAGMENT_COUNTS {
        group.throughput(Throughput::Elements(1));
        group.bench_with_input(
            BenchmarkId::new("append_and_snapshot", count),
            &count,
            |b, &count| {
                let base = fragment_metadata_case(count);
                let retired = RefCell::new(None);
                b.iter_batched(
                    || {
                        drop(retired.borrow_mut().take());
                        base.clone()
                    },
                    |mut fragments| {
                        fragments
                            .append(Arc::from(&b"replacement\n"[..]), count as u64 + 1)
                            .expect("benchmark fragment appends");
                        black_box(fragments.testing_metadata_snapshot());
                        *retired.borrow_mut() = Some(fragments);
                    },
                    BatchSize::SmallInput,
                );
                drop(retired.into_inner());
            },
        );
    }
    group.finish();

    if std::env::var_os("UMBER_ACCEPTED_EDIT_REPORT").is_some() {
        for count in FRAGMENT_COUNTS {
            let base = fragment_metadata_case(count);
            let mut fragments = base.clone();
            let (_, allocations) = counted_allocations(|| {
                fragments
                    .append(Arc::from(&b"replacement\n"[..]), count as u64 + 1)
                    .expect("benchmark fragment appends");
                black_box(fragments.testing_metadata_snapshot());
            });
            eprintln!("accepted-edit fragments={count} allocations={allocations}");
        }
    }
}

fn fragment_metadata_case(count: usize) -> FragmentStore {
    let mut fragments = FragmentStore::new();
    for revision in 0..count {
        fragments
            .append(Arc::from(&b"line\n"[..]), revision as u64)
            .expect("benchmark fragment appends");
    }
    fragments.testing_metadata_snapshot()
}

criterion_group!(benches, accepted_fragment_edit_scaling);
criterion_main!(benches);
