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

#[test]
fn shipout_sources_never_use_graph_copy_or_token_materialization_helpers() {
    let sources = [
        include_str!("../direct.rs"),
        include_str!("normalize.rs"),
        include_str!("../transaction.rs"),
    ];
    for source in sources {
        for forbidden in [
            "copy_durable_page_nodes",
            "copy_box_to_page",
            "take_box_to_page",
            "publish_page_nodes",
            "node.clone()",
            ".nodes().to_vec()",
            "collect::<Vec<TokenWord",
            "tokens.iter().collect::<Vec",
        ] {
            assert!(
                !source.contains(forbidden),
                "shipout source contains forbidden lifetime crossing: {forbidden}"
            );
        }
    }
    let normalization = include_str!("normalize.rs");
    assert!(!normalization.contains("finish_math_list_node("));
    assert!(normalization.contains("finish_math_list_node_to_shipout_scratch("));

    let math = include_str!("../../math/lower.rs");
    let shipout_math = math
        .split_once("fn append_span_to_shipout")
        .expect("shipout math lowering exists")
        .1
        .split_once("fn take_root_nodes")
        .expect("shipout math lowering has a structural end")
        .0;
    for forbidden in [
        "publish_page_nodes",
        ".to_vec()",
        "scratch.push",
        "root_nodes",
    ] {
        assert!(
            !shipout_math.contains(forbidden),
            "shipout math lowering materializes outside its final scratch rows: {forbidden}"
        );
    }
}
