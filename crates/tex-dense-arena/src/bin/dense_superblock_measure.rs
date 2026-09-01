//! Focused construction, checkpoint, fork-tail, and acceptance measurement.

use std::hint::black_box;

use tex_dense_arena::GenerationArena;
use tex_dense_prefix::{SUPERBLOCK_BYTES, SubstrateMetrics};

#[derive(Clone, Copy)]
struct Payload {
    bytes: [u8; 168],
}

impl Payload {
    fn new(seed: usize) -> Self {
        let mut bytes = [0; 168];
        bytes[0] = seed as u8;
        bytes[167] = !seed as u8;
        Self { bytes }
    }
}

fn main() {
    let substrate_before = SubstrateMetrics::snapshot();
    let mut arena = GenerationArena::<Payload>::default();
    let capacity = SUPERBLOCK_BYTES / core::mem::size_of::<Payload>();
    let checkpoint_index = capacity + 137;
    let mut checkpoint = None;
    for index in 0..4_096 {
        if index == checkpoint_index {
            checkpoint = Some(arena.cursor());
        }
        arena
            .push_with(|slot| slot.insert(Payload::new(black_box(index))))
            .expect("measurement construction");
    }
    let before_captures = arena.metrics();
    for _ in 0..4_096 {
        black_box(arena.cursor());
    }
    let after_captures = arena.metrics();
    let mut fork = arena
        .fork(checkpoint.expect("checkpoint captured"))
        .expect("measurement fork");
    let after_fork = fork.metrics();
    fork.candidate_push(Payload::new(99))
        .expect("candidate append");
    let checksum = fork
        .candidate_get(checkpoint_index)
        .expect("candidate")
        .bytes[0];
    let settled = fork.accept().expect("measurement acceptance");
    let after_accept = settled.metrics();
    let substrate = SubstrateMetrics::snapshot() - substrate_before;
    black_box(checksum);
    println!(
        "DENSE_SUPERBLOCK_MEASUREMENT capacity={capacity} allocation_attempts={} requested_bytes={} constructed={} checkpoint_captures={} checkpoint_value_copies={} fork_tail_values={} fork_tail_bytes={} table_entries={} table_bytes={} acceptance_value_copies={} descriptor_visits={}",
        substrate.allocation_attempts,
        substrate.requested_bytes,
        substrate.values_constructed,
        after_captures.cursor_captures - before_captures.cursor_captures,
        after_captures.fork_tail_values_copied - before_captures.fork_tail_values_copied,
        after_fork.fork_tail_values_copied,
        after_fork.fork_tail_bytes_copied,
        after_fork.table_entries_copied,
        after_fork.table_bytes_copied,
        after_accept.accepted_payload_copies,
        after_accept.descriptor_visits,
    );
}
