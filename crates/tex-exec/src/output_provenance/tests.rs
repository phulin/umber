use std::cell::Cell;

use tex_state::token::OriginId;
use tex_state::world::{ArtifactSourceRecipe, ContentHash};

use super::{ArtifactSourceResolver, OutputProvenanceBuilder};

struct Resolver {
    calls: Cell<usize>,
    recipe: ArtifactSourceRecipe,
}

impl ArtifactSourceResolver for Resolver {
    fn detach_artifact_source(&self, _origin: OriginId) -> Option<ArtifactSourceRecipe> {
        self.calls.set(self.calls.get() + 1);
        Some(self.recipe.clone())
    }
}

fn resolver() -> Resolver {
    Resolver {
        calls: Cell::new(0),
        recipe: ArtifactSourceRecipe {
            content: ContentHash::from_bytes(b"source"),
            logical_path: "main.tex".to_owned(),
            start: 2,
            end: 3,
        },
    }
}

#[test]
fn output_provenance_is_built_only_for_explicit_rendered_source_demand() {
    assert!(
        OutputProvenanceBuilder::for_demand(tex_state::ProvenanceDemand::DIAGNOSTICS, usize::MAX)
            .is_none()
    );
    assert!(
        OutputProvenanceBuilder::for_demand(
            tex_state::ProvenanceDemand::DIAGNOSTICS_AND_RENDERED_SOURCE,
            usize::MAX,
        )
        .is_some()
    );
}

#[test]
fn output_provenance_admits_only_owned_recipes_within_budget() {
    let resolver = resolver();
    let mut builder = OutputProvenanceBuilder::for_demand(
        tex_state::ProvenanceDemand::DIAGNOSTICS_AND_RENDERED_SOURCE,
        usize::MAX,
    )
    .expect("rendered-source demand opens the cold builder");
    builder.push_origin(&resolver, OriginId::UNKNOWN);
    assert_eq!(resolver.calls.get(), 1);

    let verified = tex_state::VerifiedArtifact::new(Vec::new())
        .with_built_render_origins(vec![1], builder.finish());
    let source = verified.render_origins_for_memo().get(0).unwrap()[0]
        .as_ref()
        .expect("detached source recipe");
    assert_eq!(source, &resolver.recipe);
    assert_ne!(
        source.logical_path.as_ptr(),
        resolver.recipe.logical_path.as_ptr()
    );
}

#[test]
fn over_budget_provenance_preserves_an_unknown_slot() {
    let resolver = resolver();
    let mut builder = OutputProvenanceBuilder::for_demand(
        tex_state::ProvenanceDemand::DIAGNOSTICS_AND_RENDERED_SOURCE,
        0,
    )
    .expect("rendered-source demand opens the cold builder");
    builder.push_origin(&resolver, OriginId::UNKNOWN);

    let verified = tex_state::VerifiedArtifact::new(Vec::new())
        .with_built_render_origins(vec![1], builder.finish());
    assert_eq!(verified.render_origins_for_memo().get(0), Some(&[None][..]));
}
