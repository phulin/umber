use super::*;

#[test]
fn print_nl_publication_uses_post_effect_selected_line_state() {
    let mut world = World::memory();
    world.publish_print_text(PrintSink::Terminal, "term", 79);
    world.publish_print_nl_text(PrintSink::TerminalAndLog, "next\n", 79);

    let writes: Vec<_> = world
        .effect_records()
        .iter()
        .filter_map(|effect| match effect {
            EffectRecord::StreamWrite { sink, text } => Some((*sink, text.as_str())),
            _ => None,
        })
        .collect();
    assert_eq!(
        writes,
        [
            (PrintSink::Terminal, "term"),
            (PrintSink::TerminalAndLog, "\nnext\n"),
        ]
    );
}

#[test]
fn encoded_print_nl_preserves_high_bytes_and_shared_line_start() {
    let mut world = World::memory();
    world.publish_print_text(PrintSink::Terminal, "term", 79);
    world.publish_print_nl_encoded_bytes(PrintSink::TerminalAndLog, &[0xc3, 0xa3, b'\n'], 79);
    let end = world.effect_pos();
    world.commit_effects(end).expect("commit encoded print");

    assert_eq!(
        world.memory_terminal_output(),
        Some(&b"term\n\xc3\xa3\n"[..])
    );
    assert_eq!(world.memory_log_output(), Some(&b"\n\xc3\xa3\n"[..]));
}

fn source(path: &str, start: u64, end: u64) -> ArtifactSourceRecipe {
    ArtifactSourceRecipe {
        content: ContentHash::for_domain(ContentDomain::Input, path.as_bytes()),
        logical_path: path.into(),
        start,
        end,
    }
}

#[test]
fn artifact_identity_excludes_owned_render_presentation() {
    let bytes = b"page artifact".to_vec();
    let hash = ContentHash::for_domain(ContentDomain::Artifact, &bytes);
    let first = CommittedArtifact::new(
        hash,
        bytes.clone(),
        ArtifactRenderProvenance::live(vec![1], vec![source("one.tex", 0, 1)]),
        Vec::new(),
    );
    let second = CommittedArtifact::new(
        hash,
        bytes,
        ArtifactRenderProvenance::live(vec![1], vec![source("two.tex", 2, 3)]),
        Vec::new(),
    );
    assert_eq!(first, second);
    assert_ne!(first.render_origins(), second.render_origins());
}

#[test]
fn reachable_future_state_identity_excludes_committed_artifact_history() {
    let mut first = World::memory();
    first.enable_reachable_state_identity();
    let mut second = first.clone();
    for (world, bytes) in [
        (&mut first, b"alpha".as_slice()),
        (&mut second, b"omega".as_slice()),
    ] {
        let hash = ContentHash::for_domain(ContentDomain::Artifact, bytes);
        let reservation = world.reserve_artifact_publication_at(0);
        world.record_artifact_commit(
            hash,
            bytes.to_vec(),
            ArtifactRenderProvenance::live(Vec::new(), Vec::new()),
            Vec::new(),
            reservation,
        );
    }
    assert_ne!(first.artifact_commits(), second.artifact_commits());
    assert_eq!(
        first.reachable_state_identity_root(),
        second.reachable_state_identity_root(),
        "detached output history is reconciled by artifact prefixes, not future engine state",
    );
}

#[test]
fn cold_render_builder_records_only_detached_sources_or_unknowns() {
    assert!(RenderProvenanceBuilder::for_demand(crate::ProvenanceDemand::DIAGNOSTICS).is_none());
    let recipe = source("chapter.tex", 7, 11);
    let mut builder = RenderProvenanceBuilder::for_demand(
        crate::ProvenanceDemand::DIAGNOSTICS_AND_RENDERED_SOURCE,
    )
    .expect("test fixture is valid");
    builder.push_source(recipe.clone());
    builder.push_unknown();
    let artifact =
        VerifiedArtifact::new(b"pdf".to_vec()).with_built_render_origins(vec![2], builder);
    let (bytes, provenance, occurrences) = artifact.into_parts();
    let committed = CommittedArtifact::new(
        ContentHash::for_domain(ContentDomain::Artifact, &bytes),
        bytes,
        provenance,
        occurrences,
    );
    assert_eq!(
        committed.render_origin(0, 0),
        ArtifactOrigin::Detached(recipe)
    );
    assert_eq!(committed.render_origin(0, 1), ArtifactOrigin::Unknown);
    assert!(committed.has_deferred_render_origins());
}

#[test]
fn flat_render_ranges_preserve_empty_and_nonempty_nodes() {
    let first = source("main.tex", 0, 1);
    let second = source("main.tex", 1, 2);
    let artifact = VerifiedArtifact::new(b"pdf".to_vec())
        .with_flat_render_origins(vec![0, 2], vec![first.clone(), second.clone()]);
    let origins = artifact.render_origins_for_memo();
    assert_eq!(origins.get(0), Some([].as_slice()));
    assert_eq!(origins.get(1), Some([Some(first), Some(second)].as_slice()));
    assert_eq!(origins.get(2), None);
}

#[test]
fn cloned_world_preserves_seeded_input_without_aliasing_artifact_dtos() {
    let mut world = World::memory();
    world
        .set_memory_file("gentle.tex", vec![b'x'; 1024])
        .expect("test fixture is valid");
    let mut cloned = world.clone();
    assert_eq!(
        world
            .read_file("gentle.tex")
            .expect("test fixture is valid")
            .bytes(),
        &[b'x'; 1024]
    );
    assert_eq!(
        cloned
            .read_file("gentle.tex")
            .expect("test fixture is valid")
            .bytes(),
        &[b'x'; 1024]
    );
}

#[test]
fn rollback_restores_effects_and_value_root_identity() {
    let mut world = World::memory();
    world.record_special("prefix", vec![1]);
    let checkpoint = world.snapshot();
    let root = world.effect_root_identity();
    assert!(root.is_mounted_in(&world));
    world.record_special("suffix", vec![2]);
    world.rollback(&checkpoint);
    assert_eq!(world.effect_records().len(), 1);
    assert!(root.is_mounted_in(&world));
    let cloned = world.clone();
    assert!(root.is_mounted_in(&cloned));
}

#[test]
fn page_effect_interval_and_stream_open_context_survive_detachment_and_rollback() {
    let mut world = World::memory();
    world.open_out(StreamSlot::new(3), "first.out");
    world.set_last_stream_open_context("\nLEAF-CONTEXT");
    world.finish_page_effect_interval();
    let checkpoint = world.snapshot();
    world.open_out(StreamSlot::new(3), "second.out");
    world.set_last_stream_open_context("\nSTALE-CONTEXT");
    assert_eq!(world.pending_page_effect_range(2), 1..2);

    let (records, contexts) = world.detached_effect_records();
    assert_eq!(records.len(), 2);
    assert_eq!(contexts[0].as_deref(), Some("\nLEAF-CONTEXT"));
    assert_eq!(contexts[1].as_deref(), Some("\nSTALE-CONTEXT"));

    world.finish_page_effect_interval();
    assert_eq!(world.pending_page_effect_range(2), 2..2);
    world.rollback(&checkpoint);
    world.open_out(StreamSlot::new(3), "second.out");
    assert_eq!(world.pending_page_effect_range(2), 1..2);
    let (_, contexts) = world.detached_effect_records();
    assert_eq!(contexts[1], None, "rollback must discard stale context");
    world
        .commit_effects(world.effect_pos())
        .expect("memory publication succeeds");
    assert!(
        world.stream_open_contexts.is_empty(),
        "committed contexts must not become retained World history"
    );
}

#[test]
fn repeated_checkpoint_forks_share_accepted_effect_blocks_and_isolate_suffixes() {
    let mut source = World::memory();
    source
        .begin_retained_session()
        .expect("test World becomes rollback-capable");
    source.record_special("accepted-0", vec![0]);
    let first = source.snapshot();
    source.record_special("source-only", vec![9]);

    let mut candidate = source.fork_checkpoint(&first);
    assert_eq!(source.effect_records().len(), 2);
    assert_eq!(candidate.page_effect_prefix_len(), 1);
    assert!(candidate.effect_records().is_empty());
    candidate.record_special("candidate-1", vec![1]);

    let mut labels = Vec::new();
    candidate.visit_pending_page_effects(candidate.effect_records().len(), |_, effect| {
        if let EffectRecord::Special { class, .. } = effect {
            labels.push(class.clone());
        }
    });
    assert_eq!(labels, ["accepted-0", "candidate-1"]);
    assert_eq!(
        source.effect_records().len(),
        2,
        "candidate mutation cannot alter the retained source suffix"
    );

    let second = candidate.snapshot();
    let next = candidate.fork_checkpoint(&second);
    assert_eq!(next.page_effect_prefix_len(), 2);
    assert!(next.effect_records().is_empty());
    let mut labels = Vec::new();
    next.visit_pending_page_effects(0, |_, effect| {
        if let EffectRecord::Special { class, .. } = effect {
            labels.push(class.clone());
        }
    });
    assert_eq!(labels, ["accepted-0", "candidate-1"]);
}

#[test]
fn pending_page_effect_visit_does_not_revisit_committed_prefixes() {
    for prefix_len in [1_usize, 64, 4_096] {
        let mut live = World::memory();
        live.begin_retained_session()
            .expect("test World becomes rollback-capable");
        for value in 0..prefix_len {
            live.record_special("prior", value.to_le_bytes().to_vec());
        }
        live.finish_page_effect_interval();
        live.record_special("pending", vec![1]);

        let pending = live.pending_page_effect_range(live.effect_records().len());
        let mut visited = Vec::new();
        let inspected = live.visit_page_effect_range(pending, |index, effect| {
            visited.push((index, effect.clone()));
        });
        assert_eq!(inspected, 1, "live prefix length {prefix_len}");
        assert_eq!(visited.len(), 1, "live prefix length {prefix_len}");
        assert_eq!(visited[0].0, prefix_len, "live prefix length {prefix_len}");

        live.finish_page_effect_interval();
        let checkpoint = live.snapshot();
        let mut fork = live.fork_checkpoint(&checkpoint);
        fork.record_special("fork-pending", vec![2]);
        let pending = fork.pending_page_effect_range(fork.effect_records().len());
        let mut visited = Vec::new();
        let inspected = fork.visit_page_effect_range(pending, |index, effect| {
            visited.push((index, effect.clone()));
        });
        assert_eq!(inspected, 1, "accepted prefix length {prefix_len}");
        assert_eq!(visited.len(), 1, "accepted prefix length {prefix_len}");
        assert_eq!(
            visited[0].0,
            prefix_len + 1,
            "accepted prefix length {prefix_len}"
        );
    }
}

#[test]
fn checkpoint_candidate_rejects_or_promotes_one_flat_world_suffix() {
    let mut world = World::memory();
    world
        .begin_retained_session()
        .expect("test World becomes rollback-capable");
    world.record_special("root", vec![0]);
    let mark = world.snapshot();
    world.record_special("accepted", vec![1]);

    let tail = world.begin_checkpoint_candidate(&mark);
    world.record_special("rejected", vec![2]);
    world.reject_checkpoint_candidate(&mark, tail);
    let labels = world
        .effect_records()
        .iter()
        .filter_map(|effect| match effect {
            EffectRecord::Special { class, .. } => Some(class.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(labels, ["root", "accepted"]);

    let tail = world.begin_checkpoint_candidate(&mark);
    world.record_special("promoted", vec![3]);
    world.accept_checkpoint_candidate(tail);
    let labels = world
        .effect_records()
        .iter()
        .filter_map(|effect| match effect {
            EffectRecord::Special { class, .. } => Some(class.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(labels, ["root", "promoted"]);
}

#[test]
fn stream_checkpoints_are_fixed_marks_over_session_owned_paths() {
    let mut world = World::memory();
    world
        .begin_retained_session()
        .expect("test World becomes rollback-capable");
    for raw in 0..STREAM_SLOT_COUNT as u8 {
        let input = format!("input-{raw}.tex");
        world
            .set_memory_file(&input, format!("first-{raw}\nsecond-{raw}\n").into_bytes())
            .expect("stream input is seeded");
        world
            .open_in(StreamSlot::new(raw), &input)
            .expect("stream input opens");
        assert_eq!(
            world
                .read_stream_line(StreamSlot::new(raw))
                .expect("stream read succeeds"),
            Some(format!("first-{raw}"))
        );
        world.open_out(StreamSlot::new(raw), format!("output-{raw}.tex"));
    }
    world.write_text(PrintSink::Terminal, "terminal-prefix");
    world.write_text(PrintSink::Log, "log-prefix");

    let input_path = Arc::clone(
        &world.stream_bufs.read_streams[0]
            .as_ref()
            .expect("input stream is open")
            .path,
    );
    let output_path = Arc::clone(
        &world.stream_bufs.write_streams[0]
            .as_ref()
            .expect("output stream is open")
            .path,
    );
    let input_owners = Arc::strong_count(&input_path);
    let output_owners = Arc::strong_count(&output_path);
    let mark = world.snapshot();
    let cloned = mark.clone();
    assert_eq!(mark, cloned);
    assert_eq!(Arc::strong_count(&input_path), input_owners);
    assert_eq!(Arc::strong_count(&output_path), output_owners);

    for raw in 0..STREAM_SLOT_COUNT as u8 {
        assert_eq!(
            world
                .read_stream_line(StreamSlot::new(raw))
                .expect("stream read succeeds"),
            Some(format!("second-{raw}"))
        );
        assert!(world.close_out(StreamSlot::new(raw)));
    }
    world.write_text(PrintSink::TerminalAndLog, "\nmutated");
    world.rollback(&mark);

    assert_eq!(world.stream_bufs.terminal_offset, "terminal-prefix".len());
    assert_eq!(world.stream_bufs.log_offset, "log-prefix".len());
    assert!(Arc::ptr_eq(
        &world.stream_bufs.read_streams[0]
            .as_ref()
            .expect("input stream is restored")
            .path,
        &input_path
    ));
    assert!(Arc::ptr_eq(
        &world.stream_bufs.write_streams[0]
            .as_ref()
            .expect("output stream is restored")
            .path,
        &output_path
    ));
    assert_eq!(
        world
            .read_stream_line(StreamSlot::new(0))
            .expect("restored stream read succeeds"),
        Some("second-0".to_owned())
    );

    world.rollback(&mark);
    let candidate = world.fork_checkpoint(&mark);
    assert_eq!(
        candidate.stream_bufs.terminal_offset,
        "terminal-prefix".len()
    );
    assert!(Arc::ptr_eq(
        &candidate.stream_bufs.read_streams[0]
            .as_ref()
            .expect("forked input stream is restored")
            .path,
        &input_path
    ));
}

#[test]
fn repeated_stream_candidate_accept_and_reject_restore_positions_and_scalars() {
    let mut world = World::memory_with_pdftex_inputs(
        JobClock {
            time: 123,
            second: 45,
            day: 6,
            month: 7,
            year: 2026,
        },
        17,
        1_000,
        ShellEscapePolicy::Restricted,
    );
    world
        .begin_retained_session()
        .expect("restricted shell policy is rollback-capable");
    world
        .set_memory_file("stream.tex", b"one\ntwo\nthree\n".to_vec())
        .expect("stream input is seeded");
    world
        .open_in(StreamSlot::new(0), "stream.tex")
        .expect("stream opens");
    assert_eq!(
        world.read_stream_line(StreamSlot::new(0)).expect("read"),
        Some("one".to_owned())
    );
    world.open_out(StreamSlot::new(0), "accepted.out");
    world.write_text(PrintSink::TerminalAndLog, "accepted");
    let root = world.snapshot();

    let accepted_random = world.next_random_u64();
    world.set_pdf_random_seed(91);
    world.set_pdf_time_micros(9_000);
    assert_eq!(
        world.read_stream_line(StreamSlot::new(0)).expect("read"),
        Some("two".to_owned())
    );
    let accepted_head = world.snapshot();

    let tail = world.begin_checkpoint_candidate(&root);
    assert_eq!(world.next_random_u64(), accepted_random);
    assert_eq!(world.pdf_random_seed(), 17);
    assert_eq!(
        world.read_stream_line(StreamSlot::new(0)).expect("read"),
        Some("two".to_owned())
    );
    world.open_out(StreamSlot::new(0), "candidate-rejected.out");
    world.reject_checkpoint_candidate(&root, tail);
    assert_eq!(world.snapshot(), accepted_head);

    let tail = world.begin_checkpoint_candidate(&root);
    assert_eq!(
        world.read_stream_line(StreamSlot::new(0)).expect("read"),
        Some("two".to_owned())
    );
    world.open_out(StreamSlot::new(0), "candidate-accepted.out");
    world.set_pdf_random_seed(314);
    world.set_pdf_time_micros(27_000);
    world.accept_checkpoint_candidate(tail);
    let promoted = world.snapshot();
    assert_eq!(world.pdf_random_seed(), 314);

    let tail = world.begin_checkpoint_candidate(&promoted);
    assert_eq!(
        world.read_stream_line(StreamSlot::new(0)).expect("read"),
        Some("three".to_owned())
    );
    world.reject_checkpoint_candidate(&promoted, tail);
    assert_eq!(world.snapshot(), promoted);
}

#[test]
fn checkpoint_candidate_reuses_detached_storage_and_moves_owned_payloads() {
    let mut world = World::memory();
    world
        .begin_retained_session()
        .expect("test World becomes rollback-capable");
    let root = world.snapshot();

    for slot in 0..16 {
        let path = format!("stream-{slot}.tex");
        world
            .set_memory_file(&path, format!("line-{slot}\n").into_bytes())
            .expect("stream input is seeded");
        world
            .open_in(StreamSlot::new(slot), &path)
            .expect("stream input opens");
        world.open_out(StreamSlot::new(slot), format!("stream-{slot}.out"));
    }
    for index in 0..64 {
        world.record_special(format!("effect-{index}"), vec![index as u8; 32]);
        world.record_shell_escape(format!("command-{index}"));
        world
            .record_input_dependency(
                format!("dependency-{index}.tex"),
                InputDependencyOutcome::Missing,
                InputDependencyAccess::AuthoritativeProbe,
            )
            .expect("dependency is recorded");
    }
    for slot in 0..16 {
        assert_eq!(
            world
                .read_stream_line(StreamSlot::new(slot))
                .expect("stream input reads"),
            Some(format!("line-{slot}"))
        );
    }
    let publication = world.reserve_effect_publication();
    world.claim_effect_publication(0..world.effects.len(), publication);
    let output_attempt = world.allocate_effect_output_attempt();
    world.commit_effect_publication_winner(None, publication, output_attempt, None);
    for index in 0..8 {
        let bytes = vec![index as u8; 128];
        let hash = ContentHash::for_domain(ContentDomain::Artifact, &bytes);
        let reservation = world.reserve_artifact_publication_at(0);
        world.record_artifact_commit(
            hash,
            bytes,
            ArtifactRenderProvenance::live(Vec::new(), Vec::new()),
            Vec::new(),
            reservation,
        );
    }

    let accepted = world.snapshot();
    let warmed_capacities = world.detached.capacities();
    let effect_payload = world
        .effects
        .iter()
        .find_map(|record| match record {
            EffectRecord::Special { payload, .. } => Some(payload.as_ptr()),
            _ => None,
        })
        .expect("special payload exists");
    let input_path = Arc::as_ptr(&world.inputs[0].path);
    let artifact_bytes = world.committed_artifacts[0].bytes().as_ptr();

    let tail = world.begin_checkpoint_candidate(&root);
    assert_eq!(world.detached.capacities(), warmed_capacities);
    assert_eq!(world.detached.effects.len(), accepted.effect_len);
    assert_eq!(world.detached.inputs.len(), accepted.input_len);
    assert_eq!(world.detached.committed_artifacts.len(), 8);
    assert_eq!(
        world
            .detached
            .effects
            .iter()
            .find_map(|record| match record {
                EffectRecord::Special { payload, .. } => Some(payload.as_ptr()),
                _ => None,
            })
            .expect("detached special payload exists"),
        effect_payload
    );
    assert_eq!(Arc::as_ptr(&world.detached.inputs[0].path), input_path);
    assert_eq!(
        world.detached.committed_artifacts[0].bytes().as_ptr(),
        artifact_bytes
    );

    world.record_special("rejected", vec![255; 32]);
    world.reject_checkpoint_candidate(&root, tail);
    assert_eq!(world.snapshot(), accepted);
    assert!(world.detached.is_empty());
    assert_eq!(world.detached.capacities(), warmed_capacities);
    assert_eq!(
        world
            .effects
            .iter()
            .find_map(|record| match record {
                EffectRecord::Special { payload, .. } => Some(payload.as_ptr()),
                _ => None,
            })
            .expect("restored special payload exists"),
        effect_payload
    );
    assert_eq!(Arc::as_ptr(&world.inputs[0].path), input_path);
    assert_eq!(
        world.committed_artifacts[0].bytes().as_ptr(),
        artifact_bytes
    );

    let tail = world.begin_checkpoint_candidate(&root);
    world.record_special("accepted", vec![127; 32]);
    world.accept_checkpoint_candidate(tail);
    assert!(world.detached.is_empty());
    assert_eq!(world.detached.capacities(), warmed_capacities);
}

#[test]
fn effect_counter_marks_restore_exactly_and_continue_across_a_fork() {
    let mut source = World::memory();
    source
        .begin_retained_session()
        .expect("test World becomes rollback-capable");
    source.record_special("accepted-0", vec![0]);
    source.record_special("accepted-1", vec![1]);
    let publication = source.reserve_effect_publication();
    source.claim_effect_publication(0..2, publication);
    let checkpoint = source.snapshot();

    source.record_special("abandoned", vec![9]);
    source.claim_effect_publication(2..3, publication);
    let abandoned_publication = source.effect_publication_record_ordinals()[2];
    let abandoned_semantic = source.effect_semantic_record_ordinals()[2];
    source.rollback(&checkpoint);
    source.record_special("replayed", vec![2]);
    source.claim_effect_publication(2..3, publication);
    assert_eq!(
        source.effect_publication_record_ordinals()[2],
        abandoned_publication,
        "rollback must restore the publication counter, not retain the abandoned value"
    );
    assert_eq!(
        source.effect_semantic_record_ordinals()[2],
        abandoned_semantic,
        "rollback must restore the semantic counter, not retain the abandoned value"
    );

    let mut candidate = source.fork_checkpoint(&checkpoint);
    candidate.record_special("candidate", vec![3]);
    candidate.claim_effect_publication(0..1, publication);
    assert_eq!(
        candidate.effect_publication_record_ordinals()[0],
        abandoned_publication,
        "a fork must continue accepted publication numbering without copying its map"
    );
    assert_eq!(
        candidate.effect_semantic_record_ordinals()[0],
        abandoned_semantic,
        "a fork must continue accepted semantic numbering without copying its map"
    );
}

#[test]
fn checkpoint_fork_shares_only_retained_input_records_and_opens_a_private_suffix() {
    let mut source = World::memory();
    source
        .begin_retained_session()
        .expect("test World becomes rollback-capable");
    source
        .set_memory_file("accepted.tex", b"accepted".to_vec())
        .expect("accepted input is seeded");
    source
        .read_file("accepted.tex")
        .expect("accepted input is read");
    let checkpoint = source.snapshot();
    source
        .set_memory_file("later.tex", b"later".to_vec())
        .expect("later input is seeded");
    source.read_file("later.tex").expect("later input is read");

    let mut candidate = source.fork_checkpoint(&checkpoint);
    assert_eq!(candidate.input_records().len(), 1);
    assert_eq!(
        candidate.input_records()[0].path(),
        Path::new("accepted.tex")
    );
    assert_eq!(
        candidate.input_content(candidate.input_records()[0].hash()),
        Some(&b"accepted"[..])
    );

    candidate
        .set_memory_file("candidate.tex", b"candidate".to_vec())
        .expect("candidate input is seeded");
    candidate
        .read_file("candidate.tex")
        .expect("candidate input is read");
    assert_eq!(candidate.input_records().len(), 2);
    assert_eq!(source.input_records().len(), 2);
    assert_eq!(source.input_records()[1].path(), Path::new("later.tex"));
}

#[test]
fn input_dependency_mark_restores_exactly_and_fork_uses_a_private_delta() {
    let mut source = World::memory();
    source
        .begin_retained_session()
        .expect("test World becomes rollback-capable");
    source
        .record_input_dependency(
            "accepted.tex",
            InputDependencyOutcome::Missing,
            InputDependencyAccess::AuthoritativeProbe,
        )
        .expect("accepted dependency");
    let checkpoint = source.snapshot();
    source
        .record_input_dependency(
            "accepted.tex",
            InputDependencyOutcome::Present(ContentHash::from_bytes(b"later")),
            InputDependencyAccess::RequiredRead,
        )
        .expect("source override");
    source
        .record_input_dependency(
            "source-only.tex",
            InputDependencyOutcome::Missing,
            InputDependencyAccess::AuthoritativeProbe,
        )
        .expect("source-only dependency");

    let mut candidate = source.fork_checkpoint(&checkpoint);
    let accepted = candidate.input_dependencies().collect::<Vec<_>>();
    assert_eq!(accepted.len(), 1);
    assert_eq!(accepted[0].path(), Path::new("accepted.tex"));
    assert_eq!(accepted[0].outcome(), InputDependencyOutcome::Missing);
    candidate
        .record_input_dependency(
            "accepted.tex",
            InputDependencyOutcome::Present(ContentHash::from_bytes(b"candidate")),
            InputDependencyAccess::RequiredRead,
        )
        .expect("candidate override");
    assert_eq!(candidate.input_dependencies().count(), 1);
    assert_eq!(source.input_dependencies().count(), 2);

    source.rollback(&checkpoint);
    let restored = source.input_dependencies().collect::<Vec<_>>();
    assert_eq!(restored.len(), 1);
    assert_eq!(restored[0].path(), Path::new("accepted.tex"));
    assert_eq!(restored[0].outcome(), InputDependencyOutcome::Missing);
    assert_eq!(
        restored[0].access(),
        InputDependencyAccess::AuthoritativeProbe
    );
}

#[test]
fn committed_artifact_bytes_are_owned_and_rehash_on_preparation() {
    let original = VerifiedArtifact::new(vec![1, 2, 3]);
    let original_hash = original.hash();
    let (bytes, provenance, occurrences) = original.into_parts();
    let committed = CommittedArtifact::new(original_hash, bytes, provenance, occurrences)
        .with_prepared_bytes(vec![4, 5, 6]);
    assert_eq!(committed.bytes(), &[4, 5, 6]);
    assert_ne!(committed.hash(), original_hash);
}

#[test]
fn terminal_input_positions_reject_foreign_and_stale_values_before_mutation() {
    let mut first = World::memory();
    first
        .push_memory_terminal_line("first")
        .expect("memory terminal input");
    let foreign = first.terminal_input_position();

    let mut second = World::memory();
    second
        .push_memory_terminal_line("second")
        .expect("memory terminal input");
    assert!(second.restore_terminal_input_position(foreign).is_err());
    assert_eq!(
        second.read_terminal_line().expect("terminal read"),
        Some("second".to_owned())
    );

    let live = second.terminal_input_position();
    let stale = TerminalInputPosition {
        owner: live.owner,
        next: usize::MAX,
    };
    assert!(second.restore_terminal_input_position(stale).is_err());
    assert_eq!(second.terminal_input_position(), live);
}
