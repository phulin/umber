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
fn checkpoint_fork_shares_only_retained_input_records_and_opens_a_private_suffix() {
    let mut source = World::memory();
    source
        .begin_retained_session()
        .expect("test World becomes rollback-capable");
    source
        .set_memory_file("accepted.tex", b"accepted".to_vec())
        .expect("accepted input is seeded");
    source.read_file("accepted.tex").expect("accepted input is read");
    let checkpoint = source.snapshot();
    source
        .set_memory_file("later.tex", b"later".to_vec())
        .expect("later input is seeded");
    source.read_file("later.tex").expect("later input is read");

    let mut candidate = source.fork_checkpoint(&checkpoint);
    assert_eq!(candidate.input_records().len(), 1);
    assert_eq!(candidate.input_records()[0].path(), Path::new("accepted.tex"));
    assert_eq!(candidate.input_content(candidate.input_records()[0].hash()), Some(&b"accepted"[..]));

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
