//! Stable, owned source recipes for detached output artifacts.

use tex_state::ProvenanceDemand;
use tex_state::token::OriginId;
use tex_state::world::{ArtifactSourceRecipe, RenderProvenanceBuilder};

#[cfg(test)]
mod tests;

/// Cold capability for detaching one live node-origin coordinate.
///
/// The admitted command/execution adapter implements this after it has
/// validated the generation which owns `origin`. Shipout deliberately cannot
/// decode the coordinate or inspect source-map storage itself.
pub(crate) trait ArtifactSourceResolver {
    fn detach_artifact_source(&self, origin: OriginId) -> Option<ArtifactSourceRecipe>;
}

/// Demand-selected builder for the artifact-owned provenance column.
///
/// Every admitted row is an owned DTO. Unknown or over-budget origins retain
/// their slot without retaining any runtime coordinate or owner.
pub(crate) struct OutputProvenanceBuilder {
    output: RenderProvenanceBuilder,
    budget_bytes: usize,
    used_bytes: usize,
}

impl OutputProvenanceBuilder {
    #[must_use]
    pub(crate) fn for_demand(demand: ProvenanceDemand, budget_bytes: usize) -> Option<Self> {
        Some(Self {
            output: RenderProvenanceBuilder::for_demand(demand)?,
            budget_bytes,
            used_bytes: 0,
        })
    }

    pub(crate) fn push_origin(&mut self, resolver: &dyn ArtifactSourceResolver, origin: OriginId) {
        let Some(recipe) = resolver.detach_artifact_source(origin) else {
            self.output.push_unknown();
            return;
        };
        let charge =
            std::mem::size_of::<ArtifactSourceRecipe>().checked_add(recipe.logical_path.len());
        let Some(next) = charge.and_then(|charge| self.used_bytes.checked_add(charge)) else {
            self.output.push_unknown();
            return;
        };
        if next > self.budget_bytes {
            self.output.push_unknown();
            return;
        }
        self.used_bytes = next;
        self.output.push_source(recipe);
    }

    pub(crate) fn finish(self) -> RenderProvenanceBuilder {
        self.output
    }
}
