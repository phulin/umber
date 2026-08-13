//! Stable source-provenance recipes for detached output artifacts.

use tex_state::Universe;

#[cfg(test)]
mod tests;

pub(crate) fn provenance_recipe_for_origins(
    stores: &Universe,
    origins: impl IntoIterator<Item = tex_state::token::OriginId>,
) -> Option<tex_state::OutputProvenanceRecipe> {
    let mut recipe =
        OutputProvenanceBuilder::new(stores.provenance_budgets().detached_artifact_recipe_bytes);
    for origin in origins {
        recipe.push_origin(stores, origin)?;
    }
    Some(recipe.finish())
}

pub(crate) struct OutputProvenanceBuilder {
    piece_anchors: Vec<tex_state::RootSpanId>,
    root_spans: Vec<tex_state::OutputProvenanceSpan>,
    origin_slots: Vec<u32>,
    root_ordinals: ahash::AHashMap<tex_state::RootSpanId, u32>,
    piece_ordinals: ahash::AHashMap<tex_state::PieceId, u32>,
    budget_bytes: usize,
}

impl OutputProvenanceBuilder {
    pub(crate) fn new(budget_bytes: usize) -> Self {
        Self {
            piece_anchors: Vec::new(),
            root_spans: Vec::new(),
            origin_slots: Vec::new(),
            root_ordinals: ahash::AHashMap::new(),
            piece_ordinals: ahash::AHashMap::new(),
            budget_bytes,
        }
    }

    pub(crate) fn push_origin(
        &mut self,
        stores: &Universe,
        origin: tex_state::token::OriginId,
    ) -> Option<()> {
        self.admit(
            self.piece_anchors.len(),
            self.root_spans.len(),
            self.origin_slots.len().checked_add(1)?,
        )?;
        let Some(span) = stores.root_span_for_origin(origin) else {
            self.origin_slots.push(u32::MAX);
            return Some(());
        };
        let ordinal = if let Some(&ordinal) = self.root_ordinals.get(&span) {
            ordinal
        } else {
            let Ok(ordinal) = u32::try_from(self.root_spans.len()) else {
                self.origin_slots.push(u32::MAX);
                return Some(());
            };
            let piece = span.piece();
            let piece_ordinal = if let Some(&piece_ordinal) = self.piece_ordinals.get(&piece) {
                piece_ordinal
            } else {
                let Ok(piece_ordinal) = u32::try_from(self.piece_anchors.len()) else {
                    self.origin_slots.push(u32::MAX);
                    return Some(());
                };
                self.admit(
                    self.piece_anchors.len().checked_add(1)?,
                    self.root_spans.len().checked_add(1)?,
                    self.origin_slots.len().checked_add(1)?,
                )?;
                self.piece_anchors.push(span.start_anchor());
                self.piece_ordinals.insert(piece, piece_ordinal);
                piece_ordinal
            };
            self.admit(
                self.piece_anchors.len(),
                self.root_spans.len().checked_add(1)?,
                self.origin_slots.len().checked_add(1)?,
            )?;
            self.root_spans.push(tex_state::OutputProvenanceSpan {
                piece: piece_ordinal,
                start: span.start(),
                end: span.end(),
            });
            self.root_ordinals.insert(span, ordinal);
            ordinal
        };
        self.origin_slots.push(ordinal);
        Some(())
    }

    fn admit(&self, pieces: usize, spans: usize, slots: usize) -> Option<()> {
        let bytes = pieces
            .checked_mul(std::mem::size_of::<tex_state::RootSpanId>())?
            .checked_add(
                spans.checked_mul(std::mem::size_of::<tex_state::OutputProvenanceSpan>())?,
            )?
            .checked_add(slots.checked_mul(std::mem::size_of::<u32>())?)?;
        (bytes <= self.budget_bytes).then_some(())
    }

    pub(crate) fn finish(self) -> tex_state::OutputProvenanceRecipe {
        tex_state::OutputProvenanceRecipe {
            piece_anchors: self.piece_anchors.into(),
            root_spans: self.root_spans.into(),
            origin_slots: self.origin_slots.into(),
        }
    }
}
