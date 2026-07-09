use super::*;
use crate::Universe;
use crate::token::{Catcode, Token};

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
fn memory_world_write_file_materializes_bytes_through_boundary() {
    let mut world = World::memory();

    world
        .write_file("out.dvi", b"dvi bytes")
        .expect("memory world writes file");
    let content = world.read_file("out.dvi").expect("read written file");

    assert_eq!(content.bytes(), b"dvi bytes");
}

#[test]
fn memory_world_stores_artifacts_by_content_hash() {
    let mut world = World::memory();
    let bytes = b"page artifact bytes";

    let first = world.store_artifact(bytes).expect("store artifact");
    let second = world.store_artifact(bytes).expect("store same artifact");

    assert_eq!(first, ContentHash::from_bytes(bytes));
    assert_eq!(first, second);
    assert_eq!(
        world.read_artifact(first).expect("read artifact"),
        Some(bytes.to_vec())
    );
}

#[test]
fn real_world_stores_artifacts_in_configured_directory() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let artifact_dir = temp_dir.path().join("artifacts");
    let mut world = World::real_with_artifact_dir(&artifact_dir);
    let bytes = b"committed page";

    let hash = world.store_artifact(bytes).expect("store artifact");
    let path = artifact_dir.join(hash.hex());

    assert_eq!(std::fs::read(&path).expect("artifact file"), bytes);
    assert_eq!(
        world.read_artifact(hash).expect("read artifact"),
        Some(bytes.to_vec())
    );
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
    assert_eq!(world.effect_records().len(), 3);
}

#[test]
fn input_stream_reads_are_pinned_and_snapshot_cursor_restores() {
    let mut world = World::memory();
    world
        .set_memory_file("stream.tex", b"one\ntwo\n".to_vec())
        .expect("seed memory file");
    let slot = StreamSlot::new(1);

    let opened = world.open_in(slot, "stream.tex").expect("open input");
    world
        .set_memory_file("stream.tex", b"changed\n".to_vec())
        .expect("mutate memory file after open");

    assert_eq!(opened.hash(), ContentHash::from_bytes(b"one\ntwo\n"));
    assert!(!world.input_stream_eof(slot));
    assert_eq!(
        world.read_stream_line(slot).expect("read first line"),
        Some("one".to_owned())
    );
    let snapshot = world.snapshot();
    assert_eq!(
        world.read_stream_line(slot).expect("read second line"),
        Some("two".to_owned())
    );
    assert!(world.input_stream_eof(slot));

    world.rollback(&snapshot);

    assert_eq!(
        world.read_stream_line(slot).expect("reread second line"),
        Some("two".to_owned())
    );
}

#[test]
fn terminal_input_cursor_is_snapshot_state() {
    let mut world = World::memory();
    world
        .push_memory_terminal_line("one")
        .expect("seed first terminal line");
    world
        .push_memory_terminal_line("two")
        .expect("seed second terminal line");

    assert_eq!(
        world
            .read_terminal_line()
            .expect("read first terminal line"),
        Some("one".to_owned())
    );
    let snapshot = world.snapshot();
    assert_eq!(
        world
            .read_terminal_line()
            .expect("read second terminal line"),
        Some("two".to_owned())
    );

    world.rollback(&snapshot);

    assert_eq!(
        world
            .read_terminal_line()
            .expect("reread second terminal line"),
        Some("two".to_owned())
    );
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
    assert!(matches!(
        world.effect_records(),
        [EffectRecord::ShellEscape(record)] if record.command() == "echo no" && !record.allowed()
    ));
}

#[test]
fn shell_escape_policy_is_snapshot_state() {
    let mut world = World::memory();
    let snapshot = world.snapshot();

    world.set_shell_escape_policy(ShellEscapePolicy::Enabled);
    assert!(world.record_shell_escape("echo yes"));

    world.rollback(&snapshot);

    assert_eq!(world.shell_escape_policy(), ShellEscapePolicy::Disabled);
    assert!(world.shell_escape_records().is_empty());
    assert!(!world.record_shell_escape("echo no"));
}

#[test]
fn unix_clock_conversion_matches_epoch() {
    assert_eq!(unix_seconds_to_job_clock(0), JobClock::DEFAULT);
}

#[test]
fn real_output_does_not_materialize_before_commit() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let output = temp_dir.path().join("job.aux");
    let mut world = World::real();
    let slot = StreamSlot::new(1);

    world.open_out(slot, &output);
    world.write_text(PrintSink::Stream(slot), "delayed");

    assert!(!output.exists());

    world
        .commit_effects(world.effect_pos())
        .expect("commit output");

    assert_eq!(
        std::fs::read(&output).expect("committed output"),
        b"delayed"
    );
}

#[test]
fn open_close_without_write_materializes_empty_output_only_at_commit() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let output = temp_dir.path().join("empty.aux");
    let mut world = World::real();
    let slot = StreamSlot::new(1);

    world.open_out(slot, &output);
    world.close_out(slot);

    assert!(!output.exists());

    world
        .commit_effects(world.effect_pos())
        .expect("commit open close");

    assert_eq!(std::fs::read(&output).expect("committed output"), b"");
}

#[test]
fn memory_open_close_without_write_materializes_empty_output() {
    let mut world = World::memory();
    let slot = StreamSlot::new(1);

    world.open_out(slot, "empty.aux");
    world.close_out(slot);
    world
        .commit_effects(world.effect_pos())
        .expect("commit open close");

    assert_eq!(world.memory_output("empty.aux"), Some(&b""[..]));
}

#[test]
fn commit_flushes_prefix_once_and_drops_history() {
    let mut world = World::memory();
    let slot = StreamSlot::new(2);

    world.open_out(slot, "out.log");
    world.write_text(PrintSink::Stream(slot), "one");
    let first_prefix = world.effect_pos();
    world.write_text(PrintSink::Stream(slot), "two");
    let second_prefix = world.effect_pos();

    world.commit_effects(first_prefix).expect("first commit");
    assert_eq!(world.memory_output("out.log"), Some(&b"one"[..]));
    assert_eq!(world.effect_records().len(), 1);

    world
        .commit_effects(first_prefix)
        .expect("idempotent recommit");
    assert_eq!(world.memory_output("out.log"), Some(&b"one"[..]));

    world.commit_effects(second_prefix).expect("second commit");
    assert_eq!(world.memory_output("out.log"), Some(&b"onetwo"[..]));
    assert!(world.effect_records().is_empty());
}

#[test]
fn rollback_discards_effect_suffix_and_restores_partial_line_bytes() {
    let mut universe = Universe::new();
    let slot = StreamSlot::new(4);

    universe.world_mut().open_out(slot, "interleaved.aux");
    universe
        .world_mut()
        .write_text(PrintSink::Stream(slot), "alpha");
    let snapshot = universe.snapshot();

    universe
        .world_mut()
        .write_text(PrintSink::Stream(slot), " beta");
    universe.world_mut().close_out(slot);
    assert_eq!(
        universe.world().stream_bufs().partial_line(slot),
        "",
        "close clears the live partial line before rollback"
    );

    universe.rollback(&snapshot);

    assert_eq!(universe.world().stream_bufs().partial_line(slot), "alpha");
    assert_eq!(universe.world().effect_records().len(), 2);

    let commit_pos = universe.world().effect_pos();
    universe
        .world_mut()
        .commit_effects(commit_pos)
        .expect("commit restored prefix");

    assert_eq!(
        universe.world().memory_output("interleaved.aux"),
        Some(&b"alpha"[..])
    );
}

#[test]
fn deferred_write_record_keeps_unexpanded_token_list_id() {
    let mut universe = Universe::new();
    let escape = universe.intern("the");
    let tokens = universe.intern_token_list(&[
        Token::Cs(escape),
        Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        },
    ]);
    let slot = StreamSlot::new(5);

    universe.world_mut().record_deferred_write(slot, tokens);

    assert!(matches!(
        universe.world().effect_records(),
        [EffectRecord::DeferredWrite { stream, tokens: recorded }]
            if *stream == slot && *recorded == tokens
    ));
}

#[test]
fn effect_log_accepts_non_stream_effect_record_kinds() {
    let mut world = World::memory();

    world.record_special("pdf:literal", b"q 1 0 0 1 0 0 cm".to_vec());
    world.record_pdf_object_placeholder("page-resource");
    world.record_shell_escape("kpsewhich foo.tfm");

    assert!(matches!(
        world.effect_records(),
        [
            EffectRecord::Special { class, payload },
            EffectRecord::PdfObjectPlaceholder { label },
            EffectRecord::ShellEscape(_)
        ] if class == "pdf:literal"
            && payload == b"q 1 0 0 1 0 0 cm"
            && label == "page-resource"
    ));
}
