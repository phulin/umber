use super::*;

#[test]
fn cloned_memory_world_shares_seeded_input_bytes() {
    let mut world = World::memory();
    world
        .set_memory_file("gentle.tex", vec![b'x'; 1024])
        .expect("seed memory input");

    let cloned = world.clone();
    let (WorldBackend::Memory(original), WorldBackend::Memory(cloned)) =
        (&world.backend, &cloned.backend)
    else {
        panic!("worlds should remain memory backed");
    };
    let original = original
        .files
        .get(Path::new("gentle.tex"))
        .expect("original input");
    let cloned = cloned
        .files
        .get(Path::new("gentle.tex"))
        .expect("cloned input");
    assert!(Arc::ptr_eq(original, cloned));
}
use crate::Universe;

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
fn artifact_identity_excludes_render_provenance() {
    let bytes = b"page artifact".to_vec();
    let hash = ContentHash::for_domain(ContentDomain::Artifact, &bytes);
    let first = CommittedArtifact::new(hash, bytes.clone(), vec![vec![OriginId::from_raw(1)]]);
    let second = CommittedArtifact::new(
        hash,
        bytes,
        vec![vec![OriginId::from_raw(2), OriginId::from_raw(3)]],
    );

    assert_eq!(first, second);
    assert_ne!(first.render_origins(), second.render_origins());
    assert!(second.render_provenance_bytes() > first.render_provenance_bytes());
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
    assert_eq!(
        world.input_record(content.record()),
        Some(&world.input_records()[0])
    );
}

#[test]
fn memory_file_modification_metadata_is_pinned_with_the_input_record() {
    let mut world = World::memory();
    let date = FileModificationDate::with_offset(
        JobClock {
            time: 23 * 60 + 5,
            second: 6,
            day: 2,
            month: 2,
            year: 2024,
        },
        -5 * 60,
    );
    world
        .set_memory_file("dated.tex", b"dated".to_vec())
        .expect("seed file");
    world
        .set_memory_file_modification_date("dated.tex", date)
        .expect("seed metadata");

    let content = world.read_file("dated.tex").expect("read dated file");
    assert_eq!(content.modification_date(), Some(date));
    assert_eq!(
        world
            .recorded_input_content(content.record())
            .expect("recorded content")
            .modification_date(),
        Some(date)
    );
}

#[test]
fn input_record_id_is_a_two_word_runtime_capability() {
    assert_eq!(core::mem::size_of::<InputRecordId>(), 16);
}

#[test]
fn rolled_back_input_record_never_revives_when_its_slot_is_reused() {
    let mut world = World::memory();
    world
        .set_memory_file("input.tex", b"old".to_vec())
        .expect("seed old input");
    let snapshot = world.snapshot();
    let old = world.read_file("input.tex").expect("read old input");

    world.rollback(&snapshot);
    assert!(world.input_record(old.record()).is_none());
    assert!(world.recorded_input_content(old.record()).is_none());

    world
        .set_memory_file("input.tex", b"new".to_vec())
        .expect("replace input");
    let new = world.read_file("input.tex").expect("read new input");

    assert_eq!(old.record().raw(), new.record().raw());
    assert_ne!(old.record(), new.record());
    assert!(world.input_record(old.record()).is_none());
    assert_eq!(
        world.input_record(new.record()).expect("new record").path(),
        Path::new("input.tex")
    );
    assert_eq!(
        world
            .recorded_input_content(new.record())
            .expect("new content")
            .bytes(),
        b"new"
    );
}

#[test]
fn rollback_retains_prefix_records_and_invalidates_only_the_suffix() {
    let mut world = World::memory();
    world
        .set_memory_file("first.tex", b"first".to_vec())
        .expect("seed first input");
    world
        .set_memory_file("second.tex", b"second".to_vec())
        .expect("seed second input");
    let first = world.read_file("first.tex").expect("read first input");
    let snapshot = world.snapshot();
    let discarded = world.read_file("second.tex").expect("read second input");

    world.rollback(&snapshot);

    assert_eq!(
        world
            .recorded_input_content(first.record())
            .expect("retained content")
            .bytes(),
        b"first"
    );
    assert!(world.input_record(discarded.record()).is_none());
    let replacement = world.read_file("second.tex").expect("reread second input");
    assert_ne!(discarded.record(), replacement.record());
}

#[test]
fn cloned_worlds_share_inherited_records_but_reject_each_others_new_records() {
    let mut left = World::memory();
    left.set_memory_file("inherited.tex", b"base".to_vec())
        .expect("seed inherited input");
    left.set_memory_file("branch.tex", b"left".to_vec())
        .expect("seed branch input");
    let inherited = left
        .read_file("inherited.tex")
        .expect("read inherited input");
    let mut right = left.clone();
    right
        .set_memory_file("branch.tex", b"right".to_vec())
        .expect("replace right branch input");

    let left_only = left.read_file("branch.tex").expect("read left branch");
    let right_only = right.read_file("branch.tex").expect("read right branch");

    assert!(left.input_record(inherited.record()).is_some());
    assert!(right.input_record(inherited.record()).is_some());
    assert_eq!(left_only.record().raw(), right_only.record().raw());
    assert_ne!(left_only.record(), right_only.record());
    assert!(left.input_record(right_only.record()).is_none());
    assert!(right.input_record(left_only.record()).is_none());
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

    assert_eq!(
        first,
        ContentHash::for_domain(ContentDomain::Artifact, bytes)
    );
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
fn real_world_rejects_non_file_artifact_destination_without_temporary_file() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let artifact_dir = temp_dir.path().join("artifacts");
    std::fs::create_dir_all(&artifact_dir).expect("create artifact dir");
    let bytes = b"committed page";
    let hash = ContentHash::for_domain(ContentDomain::Artifact, bytes);
    let final_path = artifact_dir.join(hash.hex());
    std::fs::create_dir(&final_path).expect("block final artifact path");
    let mut world = World::real_with_artifact_dir(&artifact_dir);

    let error = world
        .store_artifact(bytes)
        .expect_err("invalid destination is reported");

    assert_eq!(error.path.as_deref(), Some(final_path.as_path()));
    let entries = std::fs::read_dir(&artifact_dir)
        .expect("read artifact dir")
        .map(|entry| entry.expect("artifact entry").file_name())
        .collect::<Vec<_>>();
    assert_eq!(entries, vec![final_path.file_name().expect("final name")]);
}

#[test]
fn real_world_concurrent_identical_artifact_publication_is_idempotent() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let artifact_dir = temp_dir.path().join("artifacts");
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let mut threads = Vec::new();
    for _ in 0..2 {
        let artifact_dir = artifact_dir.clone();
        let barrier = std::sync::Arc::clone(&barrier);
        threads.push(std::thread::spawn(move || {
            let mut world = World::real_with_artifact_dir(artifact_dir);
            barrier.wait();
            world
                .store_artifact(b"shared committed page")
                .expect("publish shared artifact")
        }));
    }
    barrier.wait();

    let first = threads.remove(0).join().expect("first publisher");
    let second = threads.remove(0).join().expect("second publisher");

    assert_eq!(first, second);
    assert_eq!(
        std::fs::read(artifact_dir.join(first.hex())).expect("published artifact"),
        b"shared committed page"
    );
    assert_eq!(
        std::fs::read_dir(&artifact_dir)
            .expect("read artifact directory")
            .count(),
        1
    );
}

#[test]
fn real_world_rejects_corrupt_existing_artifact_during_publication() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let artifact_dir = temp_dir.path().join("artifacts");
    std::fs::create_dir_all(&artifact_dir).expect("create artifact dir");
    let bytes = b"committed page";
    let hash = ContentHash::for_domain(ContentDomain::Artifact, bytes);
    std::fs::write(artifact_dir.join(hash.hex()), b"corrupt page").expect("seed corruption");
    let mut world = World::real_with_artifact_dir(&artifact_dir);

    let error = world
        .store_artifact(bytes)
        .expect_err("corrupt existing artifact is rejected");

    assert!(error.to_string().contains("content identity mismatch"));
}

#[test]
fn artifact_reads_verify_requested_identity_before_returning_bytes() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let artifact_dir = temp_dir.path().join("artifacts");
    let mut world = World::real_with_artifact_dir(&artifact_dir);
    let hash = world
        .store_artifact(b"committed page")
        .expect("store artifact");
    std::fs::write(artifact_dir.join(hash.hex()), b"corrupt page").expect("corrupt artifact");

    let error = world
        .read_artifact(hash)
        .expect_err("corruption is rejected");
    assert!(error.to_string().contains("content identity mismatch"));
}

#[test]
fn artifact_reads_accept_explicit_legacy_identity_policy() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let artifact_dir = temp_dir.path().join("artifacts");
    std::fs::create_dir_all(&artifact_dir).expect("create artifact dir");
    let bytes = b"legacy page";
    let legacy = ContentHash::legacy(bytes);
    std::fs::write(artifact_dir.join(legacy.hex()), bytes).expect("write legacy artifact");
    let world = World::real_with_artifact_dir(&artifact_dir);

    assert_eq!(
        world.read_artifact(legacy).expect("read legacy artifact"),
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
    assert!(!world.input_stream_eof(slot));

    world.rollback(&snapshot);

    assert_eq!(
        world.read_stream_line(slot).expect("reread second line"),
        Some("two".to_owned())
    );
}

#[test]
fn input_stream_advances_an_incremental_byte_cursor() {
    let mut world = World::memory();
    let contents = "é\r\ntwo\n末";
    world
        .set_memory_file("large-stream.tex", contents.as_bytes().to_vec())
        .expect("seed memory file");
    let slot = StreamSlot::new(2);
    world.open_in(slot, "large-stream.tex").expect("open input");

    assert!(!world.input_stream_eof(slot));
    assert_eq!(
        world
            .read_stream_line(slot)
            .expect("first UTF-8 line should be readable")
            .as_deref(),
        Some("é")
    );
    assert_eq!(
        world
            .stream_bufs()
            .read_stream_target(slot)
            .expect("open stream should retain its target")
            .next_byte(),
        4
    );
    assert_eq!(
        world
            .read_stream_line(slot)
            .expect("CRLF line should be readable")
            .as_deref(),
        Some("two")
    );
    assert_eq!(
        world
            .stream_bufs()
            .read_stream_target(slot)
            .expect("open stream should retain its target")
            .next_byte(),
        8
    );
    assert!(!world.input_stream_eof(slot));
    assert_eq!(
        world
            .read_stream_line(slot)
            .expect("final UTF-8 line should be readable")
            .as_deref(),
        Some("末")
    );
    assert!(!world.input_stream_eof(slot));
    assert_eq!(
        world
            .stream_bufs()
            .read_stream_target(slot)
            .expect("open stream should retain its target")
            .next_byte(),
        contents.len()
    );
    assert_eq!(
        world
            .read_stream_line(slot)
            .expect("read past the final line"),
        Some(String::new())
    );
    assert!(world.input_stream_eof(slot));
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
fn pdftex_random_stream_matches_seeded_reference_sequence() {
    let mut world = World::memory();
    world.set_pdf_random_seed(1);
    assert_eq!(world.pdf_random_seed(), 1);
    assert_eq!(world.pdf_uniform_deviate(0), 0);
    assert_eq!(world.pdf_uniform_deviate(1), 0);
    assert_eq!(world.pdf_uniform_deviate(2), 1);
    assert_eq!(world.pdf_uniform_deviate(10), 6);
    assert_eq!(world.pdf_uniform_deviate(10), 5);
    assert_eq!(world.pdf_uniform_deviate(-10), -4);
    assert_eq!(world.pdf_normal_deviate(), 44_619);
    assert_eq!(world.pdf_normal_deviate(), 31_254);

    world.set_pdf_random_seed(-1);
    assert_eq!(world.pdf_random_seed(), 1);
    assert_eq!(world.pdf_uniform_deviate(10), 7);
}

#[test]
fn pdftex_utility_state_rolls_back_with_world_snapshot() {
    let mut world = World::memory();
    world.set_pdf_random_seed(1);
    world.set_pdf_time_micros(1_250_000);
    world.reset_pdf_timer();
    let snapshot = world.snapshot();

    let random = world.pdf_uniform_deviate(10);
    world.set_pdf_time_micros(2_250_000);
    assert_eq!(world.pdf_elapsed_time(), 65_536);
    world.set_shell_escape_policy(ShellEscapePolicy::Restricted);

    world.rollback(&snapshot);
    assert_eq!(world.pdf_uniform_deviate(10), random);
    assert_eq!(world.pdf_elapsed_time(), 0);
    assert_eq!(world.shell_escape_policy(), ShellEscapePolicy::Disabled);
}

#[test]
fn pdftex_session_inputs_are_supplied_at_world_construction() {
    let world = World::memory_with_pdftex_inputs(
        JobClock::DEFAULT,
        17,
        2_500_000,
        ShellEscapePolicy::Restricted,
    );
    assert_eq!(world.pdf_random_seed(), 17);
    assert_eq!(world.pdf_elapsed_time(), 0);
    assert_eq!(world.shell_escape_policy(), ShellEscapePolicy::Restricted);
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
fn unix_clock_conversion_uses_utc_time_and_date() {
    assert_eq!(
        unix_seconds_to_job_clock(1_783_604_197),
        JobClock {
            time: 816,
            second: 37,
            day: 9,
            month: 7,
            year: 2026,
        }
    );
}

#[test]
fn source_date_epoch_parser_accepts_unsigned_epoch_seconds() {
    assert_eq!(
        parse_source_date_epoch(Some("1783604160".into())),
        Some(1_783_604_160)
    );
    assert_eq!(parse_source_date_epoch(Some("not-an-epoch".into())), None);
    assert_eq!(parse_source_date_epoch(None), None);
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
fn buffered_memory_output_is_readable_as_same_job_input_and_rolls_back() {
    let mut world = World::memory();
    let slot = StreamSlot::new(10);
    let before = world.snapshot();

    world.open_out(slot, "same-job.tex");
    world.write_text(PrintSink::Stream(slot), "first\nsecond\n");
    world.close_out(slot);

    let content = world
        .read_file("same-job.tex")
        .expect("buffered output is readable before host commit");
    assert_eq!(content.bytes(), b"first\nsecond\n");
    assert_eq!(world.memory_output("same-job.tex"), None);
    world.rollback(&before);
    assert!(world.read_file("same-job.tex").is_err());

    world.open_out(slot, "same-job.tex");
    world.write_text(PrintSink::Stream(slot), "first\nsecond\n");
    world.close_out(slot);
    world
        .commit_effects(world.effect_pos())
        .expect("commit same-job output");

    let content = world
        .read_file("same-job.tex")
        .expect("committed output is readable");
    assert_eq!(content.bytes(), b"first\nsecond\n");
}

#[test]
fn buffered_real_output_is_readable_without_materializing_on_the_host() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let output = temp_dir.path().join("same-job.aux");
    let mut world = World::real();
    let slot = StreamSlot::new(10);

    world.open_out(slot, &output);
    world.write_text(PrintSink::Stream(slot), "auxiliary\n");
    world.close_out(slot);

    let content = world
        .read_file(&output)
        .expect("buffered real output is visible within the job");
    assert_eq!(content.bytes(), b"auxiliary\n");
    assert!(!output.exists());
}

#[test]
fn committed_memory_output_replaces_seeded_input_at_the_same_path() {
    let mut world = World::memory();
    let slot = StreamSlot::new(3);
    world
        .set_memory_file("replace.tex", b"old".to_vec())
        .expect("seed old file");

    world.open_out(slot, "replace.tex");
    world.write_text(PrintSink::Stream(slot), "new");
    world.close_out(slot);
    world
        .commit_effects(world.effect_pos())
        .expect("commit replacement");

    assert_eq!(
        world
            .read_file("replace.tex")
            .expect("read replacement")
            .bytes(),
        b"new"
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
fn memory_outputs_enumerate_only_materialized_outputs_in_path_order() {
    let mut world = World::memory();
    world
        .set_memory_file("seed.tex", b"input".to_vec())
        .expect("seed input");
    let slot = StreamSlot::new(1);

    world.open_out(slot, "zeta.aux");
    world.write_text(PrintSink::Stream(slot), "z");
    world.close_out(slot);
    world.open_out(slot, "alpha.aux");
    world.write_text(PrintSink::Stream(slot), "a");
    world.close_out(slot);
    world
        .commit_effects(world.effect_pos())
        .expect("commit outputs");

    let outputs = world
        .memory_outputs()
        .expect("memory output iterator")
        .map(|output| (output.path().to_owned(), output.bytes().to_vec()))
        .collect::<Vec<_>>();
    assert_eq!(
        outputs,
        vec![
            (PathBuf::from("alpha.aux"), b"a".to_vec()),
            (PathBuf::from("zeta.aux"), b"z".to_vec()),
        ]
    );
}

#[test]
fn supplied_input_bytes_are_recorded_and_pending_output_takes_precedence() {
    let mut world = World::memory();
    let supplied: Arc<[u8]> = Arc::from(&b"snapshot"[..]);
    let first = world
        .read_supplied_file(Path::new("same.aux"), Arc::clone(&supplied))
        .expect("read supplied input");
    assert_eq!(first.bytes(), b"snapshot");

    let slot = StreamSlot::new(1);
    world.open_out(slot, "same.aux");
    world.write_text(PrintSink::Stream(slot), "pending");
    world.close_out(slot);
    let reopened = world
        .read_supplied_file(Path::new("same.aux"), supplied)
        .expect("reopen pending output");
    assert_eq!(reopened.bytes(), b"pending");
    assert_eq!(world.input_records().len(), 2);
}

#[test]
fn supplied_memory_input_remains_available_for_retained_validation() {
    let mut world = World::memory();
    world
        .read_supplied_file(Path::new("/job/font.tfm"), Arc::from(&b"metrics"[..]))
        .expect("read supplied input");

    world
        .validate_recorded_inputs()
        .expect("supplied input remains available");
}

#[test]
fn real_world_has_no_memory_output_view() {
    assert!(World::real().memory_outputs().is_none());
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
fn retained_session_exports_once_in_order() {
    let mut universe = Universe::new();
    let slot = StreamSlot::new(2);
    universe
        .begin_retained_session()
        .expect("retained session starts");
    universe.world_mut().open_out(slot, "retained.log");
    universe
        .world_mut()
        .write_text(PrintSink::Stream(slot), "one");
    let prefix = universe.world().effect_pos();
    universe
        .world_mut()
        .write_text(PrintSink::Stream(slot), "two");

    universe
        .commit_effects(prefix)
        .expect("logical commit succeeds");
    assert_eq!(universe.world().memory_output("retained.log"), None);
    assert_eq!(universe.world().effect_records().len(), 3);

    universe
        .export_retained_effects()
        .expect("retained output exports");
    assert_eq!(
        universe.world().memory_output("retained.log"),
        Some(&b"onetwo"[..])
    );
    assert!(universe.export_retained_effects().is_err());
}

#[test]
fn retained_session_rejects_enabled_shell_escape() {
    let mut universe = Universe::new();
    universe
        .world_mut()
        .set_shell_escape_policy(ShellEscapePolicy::Enabled);
    assert!(universe.begin_retained_session().is_err());
    assert_eq!(universe.world().commit_mode(), WorldCommitMode::Eager);
}

#[test]
fn failure_before_effect_reports_prefix_and_retries_without_duplication() {
    let mut world = World::memory();
    let slot = StreamSlot::new(2);
    world.open_out(slot, "retry.log");
    world.write_text(PrintSink::Stream(slot), "once");
    let end = world.effect_pos();
    world.fail_effect_commit_before(end);

    let error = world.commit_effects(end).expect_err("injected failure");
    assert_eq!(error.committed_effects_through(), Some(EffectPos(1)));
    assert_eq!(error.retry_safety(), EffectRetrySafety::Safe);
    assert_eq!(world.memory_output("retry.log"), Some(&b""[..]));

    world.commit_effects(end).expect("safe retry succeeds");
    assert_eq!(world.memory_output("retry.log"), Some(&b"once"[..]));
    world.commit_effects(end).expect("recommit is idempotent");
    assert_eq!(world.memory_output("retry.log"), Some(&b"once"[..]));
}

#[test]
fn ambiguous_partial_effect_poisons_retries_without_duplicate_bytes() {
    let mut world = World::memory();
    world.write_text(PrintSink::Terminal, "abcdef");
    let end = world.effect_pos();
    world.fail_effect_commit_after_partial(end);

    let error = world
        .commit_effects(end)
        .expect_err("injected partial failure");
    assert_eq!(
        error.committed_effects_through(),
        Some(EffectPos::default())
    );
    assert_eq!(error.retry_safety(), EffectRetrySafety::Poisoned);
    assert_eq!(world.memory_terminal_output(), Some(&b"abc"[..]));

    let retry = world.commit_effects(end).expect_err("poison is terminal");
    assert_eq!(retry, error);
    assert_eq!(world.memory_terminal_output(), Some(&b"abc"[..]));
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
