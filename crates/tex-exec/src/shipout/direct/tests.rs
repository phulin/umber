use super::EmissionState;

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
