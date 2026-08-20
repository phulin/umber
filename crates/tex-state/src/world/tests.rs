use super::*;
use crate::token::Token;

#[test]
fn deferred_write_values_survive_world_rollback_and_journal_detachment() {
    let mut universe = crate::Universe::new();
    let root = universe.intern_token_list_ref(&[Token::param(5)]);
    let id = root.id();
    let mut world = World::memory();
    world.record_deferred_write(StreamSlot::new(3), root);
    let snapshot = world.snapshot();
    world.record_special("suffix", b"discarded".to_vec());
    world.rollback(&snapshot);
    let detached = world.effect_journal();
    let [EffectRecord::DeferredWrite { tokens, .. }] = detached.records() else {
        panic!("expected one deferred write")
    };
    assert_eq!(tokens.id(), id);
    assert_eq!(universe.tokens(tokens.id()).as_ref(), &[Token::param(5)]);
}

#[test]
fn cloned_memory_world_preserves_seeded_input_bytes() {
    let mut world = World::memory();
    world
        .set_memory_file("gentle.tex", vec![b'x'; 1024])
        .expect("seed memory input");

    let mut cloned = world.clone();
    assert_eq!(
        world
            .read_file("gentle.tex")
            .expect("original input")
            .bytes(),
        &[b'x'; 1024]
    );
    assert_eq!(
        cloned
            .read_file("gentle.tex")
            .expect("cloned input")
            .bytes(),
        &[b'x'; 1024]
    );
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
    let first = CommittedArtifact::new(
        hash,
        bytes.clone(),
        ArtifactRenderProvenance::live(vec![1], vec![OriginId::from_raw(1)]),
        Vec::new(),
    );
    let second = CommittedArtifact::new(
        hash,
        bytes,
        ArtifactRenderProvenance::live(vec![2], vec![OriginId::from_raw(2), OriginId::from_raw(3)]),
        Vec::new(),
    );

    assert_eq!(first, second);
    assert_ne!(first.render_origins(), second.render_origins());
    assert!(second.render_provenance_bytes() > first.render_provenance_bytes());
}

#[test]
fn flat_artifact_render_provenance_preserves_empty_and_nonempty_spans() {
    let bytes = b"page artifact".to_vec();
    let artifact = CommittedArtifact::new(
        ContentHash::for_domain(ContentDomain::Artifact, &bytes),
        bytes,
        ArtifactRenderProvenance::live(
            vec![0, 2, 3],
            vec![
                OriginId::from_raw(1),
                OriginId::from_raw(2),
                OriginId::from_raw(3),
            ],
        ),
        Vec::new(),
    );

    let origins = artifact.render_origins().expect("eager provenance");
    assert_eq!(origins.len(), 3);
    assert_eq!(origins.get(0), Some([].as_slice()));
    assert_eq!(
        origins.get(1),
        Some([OriginId::from_raw(1), OriginId::from_raw(2)].as_slice())
    );
    assert_eq!(origins.get(2), Some([OriginId::from_raw(3)].as_slice()));
    assert_eq!(origins.get(3), None);
    assert_eq!(origins.iter().count(), 3);
    assert!(artifact.render_provenance_bytes() > 0);
}

#[test]
fn mixed_artifact_provenance_decodes_only_the_requested_source() {
    let mut fragments = crate::FragmentStore::new();
    let (_, registration) = fragments
        .append(Arc::from(&b"stable"[..]), 1)
        .expect("fragment registration");
    let span = fragments
        .registered_root_span_id(registration, 1..4)
        .expect("stable root span");
    let recipe = crate::OutputProvenanceRecipe {
        piece_anchors: Arc::from([span.start_anchor()]),
        root_spans: Arc::from([crate::OutputProvenanceSpan {
            piece: 0,
            start: span.start(),
            end: span.end(),
        }]),
        origin_slots: Arc::from([0]),
    };
    let first = OriginId::from_raw(11);
    let last = OriginId::from_raw(12);
    let mut provenance = RenderProvenanceBuilder::default();
    provenance.push_root(crate::provenance::OriginRef::direct(first));
    provenance.push_deferred(&recipe, 0..1);
    provenance.push_root(crate::provenance::OriginRef::direct(last));
    let verified = VerifiedArtifact::new(b"page artifact".to_vec())
        .with_built_render_origins(vec![1, 2, 3], provenance);
    let (bytes, render_provenance, open_out_occurrences) = verified.into_parts();
    let artifact = CommittedArtifact::new(
        ContentHash::for_domain(ContentDomain::Artifact, &bytes),
        bytes,
        render_provenance,
        open_out_occurrences,
    );

    assert_eq!(
        artifact.render_origin(0, 0),
        ArtifactOrigin::Rooted(crate::provenance::OriginRef::direct(first))
    );
    assert_eq!(artifact.render_origin(1, 0), ArtifactOrigin::Stable(span));
    assert_eq!(
        artifact.render_origin(2, 0),
        ArtifactOrigin::Rooted(crate::provenance::OriginRef::direct(last))
    );
    assert_eq!(artifact.render_origin(1, 1), ArtifactOrigin::Unknown);
    assert!(artifact.render_origins().is_none());
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
    assert_eq!(content.origin(), InputOrigin::External);
    assert_eq!(world.input_records()[0].hash(), content.hash());
    assert!(world.input_records()[0].is_external_dependency());
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

/// tex.web §58 increments `term_offset` and `file_offset` separately and
/// calls the corresponding newline primitive at exactly `max_print_line`.
#[test]
fn write_text_wraps_terminal_and_log_at_exact_limit() {
    let mut world = World::memory();

    world.write_text(PrintSink::TerminalAndLog, &"a".repeat(79));
    world.write_text(PrintSink::TerminalAndLog, "b");
    world.write_text(PrintSink::Terminal, &"t".repeat(78));
    world.write_text(PrintSink::Log, &"l".repeat(77));
    world.write_text(PrintSink::TerminalAndLog, "xy");
    world.write_text(PrintSink::TerminalAndLog, "\nnext");
    let end = world.effect_pos();
    world.commit_effects(end).expect("commit wrapped writes");

    assert_eq!(
        world.memory_terminal_output(),
        Some(format!("{}\nb{}\nxy\nnext", "a".repeat(79), "t".repeat(78)).as_bytes())
    );
    assert_eq!(
        world.memory_log_output(),
        Some(format!("{}\nb{}x\ny\nnext", "a".repeat(79), "l".repeat(77)).as_bytes())
    );
    assert_eq!(world.stream_bufs().terminal_partial_line(), "next");
    assert_eq!(world.stream_bufs().log_partial_line(), "next");
}

/// The TeX82 profile crosses the output boundary as raw bytes, but §58's
/// line meters still count each byte as one character and wrap each selected
/// sink from its own current offset.
#[test]
fn encoded_bytes_wrap_independent_print_sinks_without_utf8_projection() {
    let mut world = World::memory();
    world.write_encoded_bytes(PrintSink::Terminal, &[0xff; 78]);
    world.write_encoded_bytes(PrintSink::Log, &[0x80; 77]);
    world.write_encoded_bytes(PrintSink::TerminalAndLog, &[0x00, 0x81]);
    world.write_encoded_bytes(PrintSink::TerminalAndLog, &[b'\n', 0xfe]);
    let end = world.effect_pos();
    world.commit_effects(end).expect("commit exact-byte writes");

    assert_eq!(
        world.memory_terminal_output(),
        Some(
            [&[0xff; 78][..], &[0x00, b'\n', 0x81, b'\n', 0xfe]]
                .concat()
                .as_slice()
        )
    );
    assert_eq!(
        world.memory_log_output(),
        Some(
            [&[0x80; 77][..], &[0x00, 0x81, b'\n', b'\n', 0xfe]]
                .concat()
                .as_slice()
        )
    );
    assert_eq!(
        world.stream_bufs().terminal_partial_line().as_bytes(),
        [0xc3, 0xbe]
    );
    assert_eq!(
        world.stream_bufs().log_partial_line().as_bytes(),
        [0xc3, 0xbe]
    );
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
fn pdftex_uniform_deviate_matches_oracle_at_extreme_bounds() {
    // pdftex.web lines 2514-2873 and §1587 in the pinned 1.40.29 oracle.
    let mut world = World::memory();
    for (bound, expected) in [
        (0, 0),
        (1, 0),
        (i32::MAX, 1_516_446_631),
        (-i32::MAX, -1_516_446_631),
    ] {
        world.set_pdf_random_seed(1);
        assert_eq!(world.pdf_uniform_deviate(bound), expected, "bound {bound}");
    }
}

#[test]
fn pdftex_uniform_deviate_matches_oracle_across_repeated_refills() {
    // Exact pinned-pdfTeX checkpoints straddle the generator's first two
    // 55-value refresh boundaries. Index zero is the first value consumed.
    const ORACLE_CHECKPOINTS: &[(usize, i32)] = &[
        (0, 1_516_446_631),
        (1, 206_616_856),
        (52, 1_882_092_151),
        (53, 314_976_584),
        (54, 585_288_992),
        (55, 2_081_720_319),
        (56, 932_870_584),
        (107, 624_910_504),
        (108, 1_263_438_591),
        (109, 149_803_704),
        (110, 2_035_657_095),
        (111, 2_068_020_719),
    ];

    let sequence = |world: &mut World| {
        world.set_pdf_random_seed(1);
        (0..112)
            .map(|_| world.pdf_uniform_deviate(i32::MAX))
            .collect::<Vec<_>>()
    };
    let mut first_world = World::memory();
    let mut second_world = World::memory();
    let first = sequence(&mut first_world);
    let second = sequence(&mut second_world);

    assert_eq!(first, second, "equal seeds must reproduce every refill");
    for &(index, expected) in ORACLE_CHECKPOINTS {
        assert_eq!(first[index], expected, "oracle value at index {index}");
    }
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
fn pdftex_elapsed_timer_matches_source_quantization_saturation_and_rollback() {
    let mut world = World::memory_with_pdftex_inputs(
        JobClock::DEFAULT,
        0,
        1_000_000,
        ShellEscapePolicy::Disabled,
    );
    assert_eq!(world.pdf_elapsed_time(), 0);

    world.set_pdf_time_micros(1_000_100);
    assert_eq!(world.pdf_elapsed_time(), 6);
    world.set_pdf_time_micros(1_999_999);
    assert_eq!(world.pdf_elapsed_time(), 65_529);
    world.set_pdf_time_micros(2_000_000);
    assert_eq!(world.pdf_elapsed_time(), 65_536);

    let snapshot = world.snapshot();
    world.reset_pdf_timer();
    assert_eq!(world.pdf_elapsed_time(), 0);
    world.set_pdf_time_micros(32_770_000_000);
    assert_eq!(world.pdf_elapsed_time(), i32::MAX);

    world.rollback(&snapshot);
    assert_eq!(world.pdf_elapsed_time(), 65_536);
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
fn host_clock_conversion_uses_the_local_calendar_day() {
    use chrono::FixedOffset;

    let utc = chrono::DateTime::from_timestamp(1_735_689_300, 0).expect("valid UTC timestamp");
    let west = utc.with_timezone(&FixedOffset::west_opt(8 * 60 * 60).expect("valid offset"));
    let east =
        utc.with_timezone(&FixedOffset::east_opt(5 * 60 * 60 + 30 * 60).expect("valid offset"));

    assert_eq!(
        datetime_to_job_clock(&west),
        JobClock {
            time: 955,
            second: 0,
            day: 31,
            month: 12,
            year: 2024,
        }
    );
    assert_eq!(
        datetime_to_job_clock(&east),
        JobClock {
            time: 325,
            second: 0,
            day: 1,
            month: 1,
            year: 2025,
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
    world
        .set_memory_file("materialized.tex", b"host input".to_vec())
        .expect("seed ordinary input");
    assert!(
        world
            .read_pending_output_file(Path::new("materialized.tex"))
            .expect("pending-only lookup")
            .is_none(),
        "pending-output lookup must not bypass driver input policy"
    );
    assert!(
        world
            .read_same_run_output_file("materialized.tex")
            .expect("same-run lookup")
            .is_none(),
        "same-run lookup must not consume ordinary host input"
    );
    assert!(world.input_records().is_empty());
    let before = world.snapshot();

    world.open_out(slot, "same-job.tex");
    world.write_text(PrintSink::Stream(slot), "first\nsecond\n");
    world.close_out(slot);

    let generated = world
        .read_pending_output_file(Path::new("same-job.tex"))
        .expect("read pending output")
        .expect("generated path");
    assert_eq!(generated.bytes(), b"first\nsecond\n");
    assert_eq!(generated.origin(), InputOrigin::SameRunGenerated);
    assert!(world.external_input_records().next().is_none());

    let content = world
        .read_file("same-job.tex")
        .expect("buffered output is readable before host commit");
    assert_eq!(content.bytes(), b"first\nsecond\n");
    assert_eq!(content.origin(), InputOrigin::SameRunGenerated);
    assert!(world.external_input_records().next().is_none());
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
    assert_eq!(content.origin(), InputOrigin::SameRunGenerated);
    assert!(world.external_input_records().next().is_none());
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
    assert_eq!(first.origin(), InputOrigin::External);

    let slot = StreamSlot::new(1);
    world.open_out(slot, "same.aux");
    world.write_text(PrintSink::Stream(slot), "pending");
    world.close_out(slot);
    let reopened = world
        .read_supplied_file(Path::new("same.aux"), supplied)
        .expect("reopen pending output");
    assert_eq!(reopened.bytes(), b"pending");
    assert_eq!(reopened.origin(), InputOrigin::SameRunGenerated);
    assert_eq!(world.input_records().len(), 2);
    assert_eq!(world.external_input_records().count(), 1);
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
fn semantic_input_dependencies_reduce_duplicates_and_restore_on_rollback() {
    let mut world = World::memory();
    let path = PathBuf::from("/job/main.aux");
    let old = ContentHash::from_bytes(b"old");
    let new = ContentHash::from_bytes(b"new");

    world
        .record_input_dependency(
            path.clone(),
            InputDependencyOutcome::Present(old),
            InputDependencyAccess::AuthoritativeProbe,
        )
        .expect("record positive probe");
    let snapshot = world.snapshot();
    world
        .record_input_dependency(
            path.clone(),
            InputDependencyOutcome::Present(new),
            InputDependencyAccess::RequiredRead,
        )
        .expect("replace with required read");
    world
        .record_input_dependency(
            path.clone(),
            InputDependencyOutcome::Present(new),
            InputDependencyAccess::AuthoritativeProbe,
        )
        .expect("duplicate probe");

    assert_eq!(
        world.input_dependencies().collect::<Vec<_>>(),
        vec![&InputDependency {
            path: Arc::from(path.clone().into_boxed_path()),
            outcome: InputDependencyOutcome::Present(new),
            access: InputDependencyAccess::RequiredRead,
        }]
    );

    world.rollback(&snapshot);
    assert_eq!(
        world.input_dependencies().collect::<Vec<_>>(),
        vec![&InputDependency {
            path: Arc::from(path.into_boxed_path()),
            outcome: InputDependencyOutcome::Present(old),
            access: InputDependencyAccess::AuthoritativeProbe,
        }]
    );
}

#[test]
fn semantic_input_dependencies_are_bounded_and_accounted() {
    let mut world = World::memory();
    let before = world.generation_retained_bytes();
    for index in 0..MAX_INPUT_DEPENDENCIES {
        world
            .record_input_dependency(
                format!("/job/{index}.aux"),
                InputDependencyOutcome::Missing,
                InputDependencyAccess::AuthoritativeProbe,
            )
            .expect("dependency below limit");
    }
    assert!(world.generation_retained_bytes() > before);
    assert!(
        world
            .record_input_dependency(
                "/job/overflow.aux",
                InputDependencyOutcome::Missing,
                InputDependencyAccess::AuthoritativeProbe,
            )
            .is_err()
    );
}

#[test]
fn real_world_has_no_memory_output_view() {
    assert!(World::real().memory_outputs().is_none());
}

#[test]
fn failed_file_set_publish_restores_every_destination() {
    let temp = tempfile::tempdir().expect("temp dir");
    let first = temp.path().join("index.html");
    let second = temp.path().join("assets/font.woff2");
    let third = temp.path().join("assets/manifest.json");
    std::fs::create_dir_all(second.parent().expect("asset parent")).expect("asset directory");
    std::fs::write(&first, b"old html").expect("old html");
    std::fs::write(&third, b"old manifest").expect("old manifest");

    let mut world = World::real_with_artifact_dir(temp.path().join("artifacts"));
    world.fail_publish_rename_at(1);
    let error = world
        .publish_files(vec![
            (first.clone(), b"new html".to_vec()),
            (second.clone(), b"new font".to_vec()),
            (third.clone(), b"new manifest".to_vec()),
        ])
        .expect_err("injected rename failure");

    assert!(
        error
            .to_string()
            .contains("injected publish rename failure")
    );
    assert_eq!(std::fs::read(&first).expect("restored html"), b"old html");
    assert!(!second.exists(), "new asset must not remain published");
    assert_eq!(
        std::fs::read(&third).expect("restored manifest"),
        b"old manifest"
    );
    let entries = std::fs::read_dir(second.parent().expect("asset parent"))
        .expect("read asset directory")
        .map(|entry| entry.expect("directory entry").file_name())
        .collect::<Vec<_>>();
    assert_eq!(entries, vec![std::ffi::OsString::from("manifest.json")]);
}

#[test]
fn file_set_publish_replaces_all_destinations_after_staging() {
    let temp = tempfile::tempdir().expect("temp dir");
    let html = temp.path().join("index.html");
    let manifest = temp.path().join("assets/manifest.json");
    std::fs::create_dir_all(manifest.parent().expect("asset parent")).expect("asset directory");
    std::fs::write(&html, b"old html").expect("old html");
    std::fs::write(&manifest, b"old manifest").expect("old manifest");

    let mut world = World::real_with_artifact_dir(temp.path().join("artifacts"));
    world
        .publish_files(vec![
            (html.clone(), b"new html".to_vec()),
            (manifest.clone(), b"new manifest".to_vec()),
        ])
        .expect("publish complete file set");

    assert_eq!(std::fs::read(html).expect("new html"), b"new html");
    assert_eq!(
        std::fs::read(manifest).expect("new manifest"),
        b"new manifest"
    );
}

#[test]
fn invalid_later_destination_restores_earlier_backup() {
    let temp = tempfile::tempdir().expect("temp dir");
    let first = temp.path().join("index.html");
    let invalid = temp.path().join("assets");
    std::fs::write(&first, b"old html").expect("old html");
    std::fs::create_dir(&invalid).expect("conflicting directory");

    let mut world = World::real_with_artifact_dir(temp.path().join("artifacts"));
    world
        .publish_files(vec![
            (first.clone(), b"new html".to_vec()),
            (invalid.clone(), b"asset".to_vec()),
        ])
        .expect_err("directory destination must fail");

    assert_eq!(std::fs::read(first).expect("restored html"), b"old html");
    assert!(invalid.is_dir());
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
fn real_stream_open_failure_is_typed_and_retains_ordered_suffix_for_retry() {
    let temp = tempfile::tempdir().expect("temp dir");
    let prior = temp.path().join("prior.out");
    let unavailable = temp.path().join("missing").join("blocked.out");
    let replacement = temp.path().join("replacement.out");
    let slot = StreamSlot::new(2);
    let mut universe = Universe::with_world(World::real());
    universe
        .begin_retained_session()
        .expect("retained real session");
    universe.world_mut().open_out(slot, &prior);
    universe
        .world_mut()
        .write_text(PrintSink::Stream(slot), "prior");
    universe.world_mut().close_out(slot);
    universe.world_mut().open_out(slot, &unavailable);
    universe
        .world_mut()
        .write_text(PrintSink::Stream(slot), "suffix");

    let error = universe
        .export_retained_effects()
        .expect_err("authoritative open fails");
    let failed = error
        .stream_open_unavailable()
        .expect("typed unavailable open")
        .clone();
    assert_eq!(failed.path(), unavailable.as_path());
    assert_eq!(error.retry_safety(), EffectRetrySafety::Safe);
    assert_eq!(
        std::fs::read(&prior).expect("prior effects committed"),
        b"prior"
    );
    assert!(!unavailable.exists(), "failed open creates no file");
    assert!(matches!(
        universe.world().effect_records(),
        [EffectRecord::StreamOpen { target, .. }, EffectRecord::StreamWrite { .. }]
            if target.path() == unavailable
    ));

    universe
        .world_mut()
        .retarget_pending_stream_open(&failed, &replacement)
        .expect("retarget pending open");
    universe
        .export_retained_effects()
        .expect("ordered suffix retry succeeds");
    assert_eq!(
        std::fs::read(&replacement).expect("replacement output"),
        b"suffix"
    );
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
fn tex82_output_preserves_control_and_high_bytes() {
    let mut world = World::memory();
    let slot = StreamSlot::new(3);
    let expected = [0x00, 0x0f, 0x7f, 0xff];

    world.open_out(slot, "bytes.out");
    world.write_encoded_bytes(PrintSink::Stream(slot), &expected);
    world.close_out(slot);
    let end = world.effect_pos();
    world.commit_effects(end).expect("commit exact-byte write");

    assert_eq!(world.memory_output("bytes.out"), Some(expected.as_slice()));
}

#[test]
fn diagnostic_commits_preencoded_bytes_without_utf8_projection() {
    let mut universe = Universe::new();
    let expected = [0x00, 0x0f, 0x7f, 0xff];

    let mut diagnostic = universe.begin_diagnostic();
    diagnostic.print_encoded_bytes(&expected);
    diagnostic.end(false);
    let end = universe.world().effect_pos();
    universe
        .world_mut()
        .commit_effects(end)
        .expect("commit diagnostic bytes");

    assert_eq!(
        universe.world().memory_log_output(),
        Some([expected.as_slice(), b"\n"].concat().as_slice())
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

#[test]
fn real_output_open_outcome_does_not_create_or_probe_the_target() {
    let temp = tempfile::tempdir().expect("temp dir");
    let target = temp.path().join("not-created.tex");
    let world = World::real_with_artifact_dir(temp.path().join("artifacts"));

    assert_eq!(
        world.retained_output_open_outcome(&target),
        RetainedOutputOpenOutcome::DeferredToCommit
    );
    assert!(
        !target.exists(),
        "pre-commit availability must be effect-free"
    );
}

#[test]
fn page_output_receipt_preserves_ordered_multi_artifact_group() {
    let effect = EffectPublicationId::new(7);
    let first = ArtifactPublicationRecord::new(
        ArtifactPublicationId::new(20),
        PageOutputPublicationReceiptId::new(9),
        Some(effect),
        EffectSequence::new(11),
        EffectDomain::World(3),
        0,
    );
    let second = ArtifactPublicationRecord::new(
        ArtifactPublicationId::new(22),
        PageOutputPublicationReceiptId::new(9),
        Some(effect),
        EffectSequence::new(11),
        EffectDomain::World(3),
        1,
    );
    let mut receipt = PageOutputPublicationReceipt::committed(effect, second);
    receipt.extend(&PageOutputPublicationReceipt::committed(effect, first));

    assert_eq!(receipt.artifacts(), &[first, second]);
}

#[test]
fn failed_artifact_reservation_does_not_consume_effect_identity() {
    let mut world = World::memory();
    let failed =
        world.reserve_artifact_publication(EffectSequence::new(1), EffectDomain::World(1), None);
    let effect = world.reserve_effect_publication();
    let committed =
        world.reserve_artifact_publication(EffectSequence::new(2), EffectDomain::World(2), None);

    assert_eq!(failed.record().publication, ArtifactPublicationId::new(1));
    assert_eq!(
        committed.record().publication,
        ArtifactPublicationId::new(2)
    );
    assert_eq!(effect, EffectPublicationId::new(1));
}

#[test]
fn provisional_page_output_receipts_clone_rollback_and_continue_ordered_group() {
    fn commit(world: &mut World, reservation: ArtifactPublicationReservation, byte: u8) {
        let bytes = vec![byte];
        world.record_artifact_commit(
            ContentHash::for_domain(ContentDomain::Artifact, &bytes),
            bytes,
            ArtifactRenderProvenance::live(Vec::new(), Vec::new()),
            Vec::new(),
            reservation,
        );
    }

    let mut world = World::memory();
    let receipt = PageOutputPublicationReceiptId::new(44);
    let first = world.reserve_active_artifact_publication_at(0, Some(receipt));
    commit(&mut world, first, 1);
    let snapshot = world.snapshot();
    let mut fork = world.clone();

    let second = world.reserve_active_artifact_publication_at(0, Some(receipt));
    commit(&mut world, second, 2);
    assert_eq!(
        world
            .provisional_page_output_receipt(receipt)
            .expect("live receipt")
            .iter()
            .map(|record| record.intra_order())
            .collect::<Vec<_>>(),
        [0, 1]
    );

    world.rollback(&snapshot);
    assert_eq!(
        world
            .provisional_page_output_receipt(receipt)
            .expect("rolled-back receipt")
            .len(),
        1
    );
    let fork_second = fork.reserve_active_artifact_publication_at(0, Some(receipt));
    commit(&mut fork, fork_second, 3);
    assert_eq!(
        fork.provisional_page_output_receipt(receipt)
            .expect("fork receipt")[1]
            .intra_order(),
        1
    );

    fork.discard_provisional_page_output_receipt(receipt);
    assert!(fork.provisional_page_output_receipt(receipt).is_none());
    assert!(world.provisional_page_output_receipt(receipt).is_some());
}

#[test]
fn effect_semantic_record_ordinals_survive_clone_rollback_and_install() {
    let mut world = World::memory();
    let domain = EffectDomain::World(17);
    world.set_active_effect_domain(Some(domain));
    world.record_special("test", b"one".to_vec());
    world.record_special("test", b"two".to_vec());
    let snapshot = world.snapshot();
    let mut fork = world.clone();

    world.record_special("test", b"discarded".to_vec());
    world.rollback(&snapshot);
    world.record_special("test", b"replacement".to_vec());
    fork.record_special("test", b"fork".to_vec());

    let expected = [
        EffectSemanticRecordOrdinal::new(1),
        EffectSemanticRecordOrdinal::new(2),
        EffectSemanticRecordOrdinal::new(3),
    ];
    assert_eq!(world.effect_semantic_record_ordinals().as_slice(), expected);
    assert_eq!(fork.effect_semantic_record_ordinals().as_slice(), expected);

    let mut installed = World::memory();
    installed.record_special("test", b"one".to_vec());
    installed.record_special("test", b"two".to_vec());
    installed.record_special("test", b"three".to_vec());
    installed.install_effect_domains(&[domain; 3]);
    installed.install_effect_semantic_record_ordinals(&expected);
    assert_eq!(
        installed.effect_semantic_record_ordinals().as_slice(),
        expected
    );
}

#[test]
fn effect_placement_intra_orders_survive_clone_rollback_install_and_failed_gaps() {
    let mut world = World::memory();
    world.record_special("test", b"one".to_vec());
    let snapshot = world.snapshot();
    let mut fork = world.clone();

    world.record_special("test", b"discarded".to_vec());
    world.rollback(&snapshot);
    world.record_special("test", b"replacement".to_vec());
    fork.record_special("test", b"fork".to_vec());

    let expected = [
        EffectPlacementIntraOrder::new(1),
        EffectPlacementIntraOrder::new(2),
    ];
    assert_eq!(world.effect_placement_intra_orders().as_slice(), expected);
    assert_eq!(fork.effect_placement_intra_orders().as_slice(), expected);

    world.record_special("test", b"failed".to_vec());
    world.record_special("test", b"after gap".to_vec());
    assert_eq!(
        world.effect_placement_intra_orders().last(),
        Some(&EffectPlacementIntraOrder::new(4)),
        "each appended record must receive a fresh placement order"
    );

    let mut installed = World::memory();
    installed.record_special("test", b"one".to_vec());
    installed.record_special("test", b"two".to_vec());
    installed.install_effect_placement_intra_orders(&expected);
    assert_eq!(
        installed.effect_placement_intra_orders().as_slice(),
        expected
    );
}

#[test]
fn installed_publication_boundary_claim_restarts_its_local_ordinals() {
    let mut world = World::memory();
    let left = EffectPublicationId::new(7);
    let right = EffectPublicationId::new(11);
    world.record_special("test", b"left".to_vec());
    world.claim_effect_publication(0..1, left);
    world.record_special("test", b"first boundary record".to_vec());
    world.record_special("test", b"second boundary record".to_vec());
    world.record_special("test", b"right".to_vec());
    world.claim_effect_publication(3..4, right);
    let output_attempt = world.allocate_effect_output_attempt();
    world.claim_effect_publication_boundary(1..3, 3, right, output_attempt);

    let installed = world.effect_semantic_record_ordinals();
    let boundary_domain = EffectDomain::PublicationBoundary {
        left: Some(left),
        right: Some(right),
        output_attempt,
    };
    assert_eq!(world.effect_domains()[1..3], [boundary_domain; 2]);
    assert_eq!(
        installed[1..3],
        [
            EffectSemanticRecordOrdinal::new(1),
            EffectSemanticRecordOrdinal::new(2),
        ]
    );

    world.install_effect_semantic_record_ordinals(&installed);
    world.claim_effect_publication_boundary(1..3, 3, right, output_attempt);

    assert_eq!(
        world.effect_semantic_record_ordinals()[1..3],
        installed[1..3],
        "replaying one typed boundary claim preserves exact record identity"
    );
    assert_ne!(
        world.effect_semantic_record_ordinals()[1],
        world.effect_semantic_record_ordinals()[2],
        "legitimate records within one boundary claim remain distinct"
    );
}

#[test]
fn retry_suffix_reconciliation_preserves_prefix_and_new_tail() {
    let mut current = b"accepted marker marker tail".to_vec();
    assert!(super::deduplicate_retry_suffix(
        b"accepted marker",
        &mut current
    ));
    assert_eq!(current, b"accepted marker tail");

    let mut unrelated = b"accepted marker new".to_vec();
    assert!(!super::deduplicate_retry_suffix(
        b"accepted marker",
        &mut unrelated
    ));
    assert_eq!(unrelated, b"accepted marker new");
}

#[test]
fn memory_retry_reconciliation_deduplicates_every_output_channel_once() {
    let mut world = World::memory();
    let slot = StreamSlot::new(4);
    world.open_out(slot, "retry.aux");
    world.write_text(PrintSink::TerminalAndLog, "accepted marker");
    world.write_text(PrintSink::Stream(slot), "accepted marker");
    world
        .commit_effects(world.effect_pos())
        .expect("materialize suspended attempt");
    let checkpoint = world
        .memory_materialization_checkpoint()
        .expect("memory materialization checkpoint");

    world.write_text(PrintSink::TerminalAndLog, " marker tail");
    world.write_text(PrintSink::Stream(slot), " marker tail");
    world
        .commit_effects(world.effect_pos())
        .expect("materialize replay");

    assert!(world.reconcile_memory_retry_materialization(&checkpoint));
    assert_eq!(
        world.memory_terminal_output(),
        Some(&b"accepted marker tail"[..])
    );
    assert_eq!(
        world.memory_log_output(),
        Some(&b"accepted marker tail"[..])
    );
    assert_eq!(
        world.memory_output("retry.aux"),
        Some(&b"accepted marker tail"[..])
    );
    assert!(!world.reconcile_memory_retry_materialization(&checkpoint));
}
