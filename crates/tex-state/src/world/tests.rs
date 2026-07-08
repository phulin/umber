use super::*;

#[test]
fn content_hash_is_stable_for_same_bytes() {
    assert_eq!(
        ContentHash::from_bytes(b"abc").hex(),
        ContentHash::from_bytes(b"abc").hex()
    );
    assert_ne!(
        ContentHash::from_bytes(b"abc"),
        ContentHash::from_bytes(b"abd")
    );
}

#[test]
fn memory_world_reads_and_records_hashes() {
    let mut world = World::memory();
    world
        .set_memory_file("main.tex", b"hello".to_vec())
        .expect("memory world accepts files");

    let content = world.read_file("main.tex").expect("read memory file");

    assert_eq!(content.bytes(), b"hello");
    assert_eq!(content.hash(), ContentHash::from_bytes(b"hello"));
    assert_eq!(world.input_records()[0].hash(), content.hash());
}

#[test]
fn stream_partial_lines_snapshot_and_restore() {
    let mut world = World::memory();
    let slot = StreamSlot::new(3);
    world.open_out(slot, "out.log");
    world.write_text(PrintSink::Stream(slot), "partial");
    world.write_text(PrintSink::TerminalAndLog, "term");
    let snapshot = world.snapshot();

    world.write_text(PrintSink::Stream(slot), " line\nnext");
    world.write_text(PrintSink::TerminalAndLog, " done\nnext");
    world.rollback(&snapshot);

    assert_eq!(world.stream_bufs().partial_line(slot), "partial");
    assert_eq!(world.stream_bufs().terminal_partial_line(), "term");
    assert_eq!(world.stream_bufs().log_partial_line(), "term");
}

#[test]
fn rng_snapshot_restores_sequence() {
    let mut world = World::memory();
    let first = world.next_random_u64();
    let snapshot = world.snapshot();
    let second = world.next_random_u64();

    world.rollback(&snapshot);

    assert_ne!(first, second);
    assert_eq!(world.next_random_u64(), second);
}

#[test]
fn shell_escape_is_record_only_and_disabled_by_default() {
    let mut world = World::memory();

    assert!(!world.record_shell_escape("echo no"));
    assert_eq!(world.shell_escape_records()[0].command(), "echo no");
    assert!(!world.shell_escape_records()[0].allowed());
}

#[test]
fn unix_clock_conversion_matches_epoch() {
    assert_eq!(unix_seconds_to_job_clock(0), JobClock::DEFAULT);
}
