use std::hint::black_box;

use tex_state::Universe;
use tex_state::interner::InternerBudget;
use tex_state::measurement::{
    HotCoreAllocationOwner, HotCoreAllocator, hot_core_allocation_scope,
    hot_core_thread_allocation_measurement,
};
use tex_state::token::{Token, TokenWord};

#[global_allocator]
static GLOBAL: HotCoreAllocator = HotCoreAllocator;

const DIRECTORY_PREFIX_CHUNKS: usize = 320;
const DEFINITION_WORD_CHUNK_CAPACITY: usize = 4_096;

fn main() {
    for words in [1_usize, 4_096, 8_193] {
        run_fixture(words);
    }
    println!("resident macro body gate: PASS");
}

fn run_fixture(words: usize) {
    let budget = InternerBudget::new(64, 64, 1 << 16).expect("benchmark interner budget");
    tex_state::with_universe(budget, |universe| run_in_universe(universe, words))
        .expect("benchmark universe");
}

fn run_in_universe<G>(universe: &mut Universe<G>, words: usize) {
    let relax = TokenWord::pack(Token::frozen_relax());
    let prefix = vec![relax; DIRECTORY_PREFIX_CHUNKS * DEFINITION_WORD_CHUNK_CAPACITY];
    universe
        .allocate_definition(&[], &prefix)
        .expect("resident directory prefix");
    let replacement = (0..words)
        .map(|_| relax)
        .collect::<Vec<_>>();
    let definition = universe
        .allocate_definition(&[], &replacement)
        .expect("resident definition fixture");
    tex_state::reset_resident_macro_body_read_counters();
    let context = universe.command_context().expect("command context");
    let (_, _, body) = context
        .admit_macro_body(definition)
        .expect("resident body admission");
    let owner_count = body.profile_region_owner_count();
    let definition_retains = tex_state::definition_retain_count();
    let allocation_owner = HotCoreAllocationOwner::DeliveryAndScan;
    let allocations_before = hot_core_thread_allocation_measurement(allocation_owner);
    let mut checksum = 0_u64;
    {
        let _scope = hot_core_allocation_scope(allocation_owner);
        for position in 0..words {
            checksum = checksum.wrapping_add(u64::from(
                body.word(black_box(position))
                    .expect("resident word")
                    .raw(),
            ));
        }
    }
    let allocations_after = hot_core_thread_allocation_measurement(allocation_owner);
    let counters = tex_state::resident_macro_body_read_counters();
    let expected_transitions = match words {
        1 | 4_096 => 0,
        8_193 => 2,
        _ => unreachable!("fixed gate shape"),
    };
    assert_eq!(counters.admission_chunk_lookups, 1);
    assert_eq!(counters.region_owner_acquisitions, 1);
    assert_eq!(counters.direct_chunk_slot_reads, words as u64);
    assert_eq!(
        counters.chunk_boundary_transitions,
        expected_transitions
    );
    assert_eq!(counters.whole_body_copies, 0);
    assert_eq!(body.profile_region_owner_count(), owner_count);
    assert_eq!(tex_state::definition_retain_count(), definition_retains);
    assert_eq!(allocations_after.calls - allocations_before.calls, 0);
    assert_eq!(
        allocations_after.requested_bytes - allocations_before.requested_bytes,
        0
    );
    println!(
        "resident_macro_body words={words} directory_prefix_chunks={DIRECTORY_PREFIX_CHUNKS} admission_chunk_lookups={} direct_chunk_slot_reads={} chunk_boundary_transitions={} region_owner_acquisitions={} extra_region_owner_acquisitions=0 definition_retains=0 whole_body_copies=0 allocations=0 requested_bytes=0 checksum={checksum}",
        counters.admission_chunk_lookups,
        counters.direct_chunk_slot_reads,
        counters.chunk_boundary_transitions,
        counters.region_owner_acquisitions,
    );
}
