use super::EmissionState;
use super::lower::pending_page_effects;

#[test]
fn detached_open_out_sidecars_use_artifact_local_effect_ordinals() {
    let mut world = tex_state::World::memory();
    world.open_out(tex_state::StreamSlot::new(2), "first.out");
    world.open_out(tex_state::StreamSlot::new(3), "second.out");

    let pending = pending_page_effects(&world, world.effect_records().len());

    assert_eq!(pending.open_out_occurrences.len(), 2);
    assert_eq!(pending.open_out_occurrences[0].0, 0);
    assert_eq!(pending.open_out_occurrences[0].1.index(), 1);
    assert_eq!(pending.open_out_occurrences[1].0, 1);
    assert_eq!(pending.open_out_occurrences[1].1.index(), 2);
}

#[test]
fn no_rendered_source_consumer_builds_no_artifact_origin_column() {
    let mut batch = EmissionState::page(false, 0);
    let stores = tex_state::Universe::new();
    batch.node(&stores, [tex_state::provenance::OriginRef::unknown()]);
    assert!(batch.render_origin_ends.is_none());
    assert!(batch.render_origins.is_empty());

    let mut editor = EmissionState::page(true, 0);
    editor.node(&stores, [tex_state::provenance::OriginRef::unknown()]);
    assert_eq!(editor.render_origin_ends.as_deref(), Some(&[0, 1][..]));
    assert!(!editor.render_origins.is_empty());
}
