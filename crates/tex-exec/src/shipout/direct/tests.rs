use std::cell::Cell;

use tex_state::token::OriginId;
use tex_state::world::ArtifactSourceRecipe;

use super::EmissionState;
use super::lower::pending_page_effects;
use crate::output_provenance::ArtifactSourceResolver;

struct Resolver(Cell<usize>);

impl ArtifactSourceResolver for Resolver {
    fn detach_artifact_source(&self, _origin: OriginId) -> Option<ArtifactSourceRecipe> {
        self.0.set(self.0.get() + 1);
        None
    }
}

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
    let resolver = Resolver(Cell::new(0));
    let mut batch = EmissionState::page(tex_state::ProvenanceDemand::DIAGNOSTICS, 0, &resolver, 0);
    batch.node([OriginId::UNKNOWN]);

    assert!(batch.render_origin_ends.is_none());
    assert!(batch.render_origins.is_none());
    assert_eq!(resolver.0.get(), 0);
}

#[test]
fn rendered_source_consumer_resolves_each_requested_origin() {
    let resolver = Resolver(Cell::new(0));
    let mut editor = EmissionState::page(
        tex_state::ProvenanceDemand::DIAGNOSTICS_AND_RENDERED_SOURCE,
        0,
        &resolver,
        0,
    );
    editor.node([OriginId::UNKNOWN]);

    assert_eq!(editor.render_origin_ends.as_deref(), Some(&[0, 1][..]));
    assert!(editor.render_origins.is_some());
    assert_eq!(resolver.0.get(), 1);
}
