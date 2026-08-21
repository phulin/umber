use super::*;
use crate::interner::InternerBudget;
use crate::token::{Token, TokenWord};
use crate::universe::with_universe;

fn source(path: &str, start: u64, end: u64) -> ArtifactSourceRecipe {
    ArtifactSourceRecipe {
        content: ContentHash::for_domain(ContentDomain::Input, path.as_bytes()),
        logical_path: path.into(),
        start,
        end,
    }
}

#[test]
fn deferred_write_journal_owns_a_handle_free_memo_value() {
    let detached = with_universe(
        InternerBudget::new(64, 128, 4 * 1024).expect("test fixture is valid"),
        |universe| {
            let id = universe
                .allocate_token_list(&[TokenWord::pack(Token::param(5))])
                .expect("test fixture is valid");
            universe
                .detach_token_list(id)
                .expect("test fixture is valid")
        },
    )
    .expect("test fixture is valid");
    let expected = detached.clone();
    let mut world = World::memory();
    world.record_deferred_write(StreamSlot::new(3), detached);
    let journal = world.effect_journal();
    let [EffectRecord::DeferredWrite { tokens, .. }] = journal.records() else {
        panic!("expected one deferred write")
    };
    assert_eq!(tokens, &expected);
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
fn committed_artifact_bytes_are_owned_and_rehash_on_preparation() {
    let original = VerifiedArtifact::new(vec![1, 2, 3]);
    let original_hash = original.hash();
    let (bytes, provenance, occurrences) = original.into_parts();
    let committed = CommittedArtifact::new(original_hash, bytes, provenance, occurrences)
        .with_prepared_bytes(vec![4, 5, 6]);
    assert_eq!(committed.bytes(), &[4, 5, 6]);
    assert_ne!(committed.hash(), original_hash);
}
