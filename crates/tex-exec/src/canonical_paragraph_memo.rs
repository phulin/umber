//! Source-free canonical paragraph validation, mutation replay, and provenance recipes.

use tex_state::Universe;

#[inline]
pub(crate) const fn same_mutation_entry_class(
    recorded_in_group: bool,
    execution_group_depth: u32,
) -> bool {
    recorded_in_group == (execution_group_depth != 0)
}

pub(crate) fn validate_dependencies(
    stores: &Universe,
    observations: &[tex_state::ObservedDependency],
) -> bool {
    observations.iter().all(|observation| {
        stores.dependency_changed_at(observation.key) == observation.changed_at
            || stores.semantic_dependency_value(observation.key).as_ref()
                == Some(&observation.value)
    })
}

pub(crate) fn validate_mutations(
    stores: &Universe,
    mutations: &[tex_state::PureParagraphMutation],
) -> bool {
    let mut seen = ahash::AHashSet::new();
    mutations.iter().all(|mutation| {
        let key = match *mutation {
            tex_state::PureParagraphMutation::Count { index, .. } => (0_u8, index),
            tex_state::PureParagraphMutation::IntParam { param, .. } => (1_u8, param.raw()),
            tex_state::PureParagraphMutation::CurrentFont { .. } => (2_u8, 0),
        };
        if !seen.insert(key) {
            return true;
        }
        match *mutation {
            tex_state::PureParagraphMutation::Count {
                index, expected, ..
            } => stores.count(index) == expected,
            tex_state::PureParagraphMutation::IntParam {
                param, expected, ..
            } => stores.int_param(param) == expected,
            tex_state::PureParagraphMutation::CurrentFont {
                expected_font,
                expected_symbol,
                ..
            } => font_selector_matches(stores, expected_font, expected_symbol),
        }
    })
}

pub(crate) fn replay_mutations(
    stores: &mut Universe,
    mutations: &[tex_state::PureParagraphMutation],
) {
    for mutation in mutations {
        match *mutation {
            tex_state::PureParagraphMutation::Count {
                index,
                value,
                global,
                ..
            } => {
                if global {
                    stores.set_count_global(index, value);
                } else {
                    stores.set_count(index, value);
                }
            }
            tex_state::PureParagraphMutation::IntParam {
                param,
                value,
                global,
                ..
            } => {
                if global {
                    stores.set_int_param_global(param, value);
                } else {
                    stores.set_int_param(param, value);
                }
            }
            tex_state::PureParagraphMutation::CurrentFont {
                value_font,
                value_symbol,
                global,
                ..
            } => match (global, value_symbol) {
                (true, Some(symbol)) => {
                    stores.set_current_font_selector_global(symbol, value_font);
                }
                (false, Some(symbol)) => stores.set_current_font_selector(symbol, value_font),
                (true, None) => stores.set_current_font_global(value_font),
                (false, None) => stores.set_current_font(value_font),
            },
        }
    }
}

fn font_selector_matches(
    stores: &Universe,
    expected_font: tex_state::ids::FontId,
    expected_symbol: Option<tex_state::interner::Symbol>,
) -> bool {
    stores.semantic_font_dependency_value(stores.current_font())
        == stores.semantic_font_dependency_value(expected_font)
        && match (
            stores.current_font_symbol().map(|symbol| symbol.symbol()),
            expected_symbol,
        ) {
            (Some(current), Some(expected)) => {
                stores.control_sequence_kind(current) == stores.control_sequence_kind(expected)
                    && stores.resolve(current) == stores.resolve(expected)
            }
            (None, None) => true,
            (Some(_), None) | (None, Some(_)) => false,
        }
}

pub(crate) fn provenance_recipe_for_origins(
    stores: &Universe,
    origins: impl IntoIterator<Item = tex_state::token::OriginId>,
) -> tex_state::ParagraphProvenanceRecipe {
    let mut recipe = ParagraphProvenanceBuilder::default();
    for origin in origins {
        recipe.push_origin(stores, origin);
    }
    recipe.finish()
}

#[derive(Default)]
pub(crate) struct ParagraphProvenanceBuilder {
    piece_anchors: Vec<tex_state::RootSpanId>,
    root_spans: Vec<tex_state::ParagraphProvenanceSpan>,
    origin_slots: Vec<u32>,
    root_ordinals: ahash::AHashMap<tex_state::RootSpanId, u32>,
    piece_ordinals: ahash::AHashMap<tex_state::PieceId, u32>,
}

impl ParagraphProvenanceBuilder {
    pub(crate) fn push_origin(&mut self, stores: &Universe, origin: tex_state::token::OriginId) {
        let Some(span) = stores.root_span_for_origin(origin) else {
            self.origin_slots.push(u32::MAX);
            return;
        };
        let ordinal = if let Some(&ordinal) = self.root_ordinals.get(&span) {
            ordinal
        } else {
            let Ok(ordinal) = u32::try_from(self.root_spans.len()) else {
                self.origin_slots.push(u32::MAX);
                return;
            };
            let piece = span.piece();
            let piece_ordinal = if let Some(&piece_ordinal) = self.piece_ordinals.get(&piece) {
                piece_ordinal
            } else {
                let Ok(piece_ordinal) = u32::try_from(self.piece_anchors.len()) else {
                    self.origin_slots.push(u32::MAX);
                    return;
                };
                self.piece_anchors.push(span.start_anchor());
                self.piece_ordinals.insert(piece, piece_ordinal);
                piece_ordinal
            };
            self.root_spans.push(tex_state::ParagraphProvenanceSpan {
                piece: piece_ordinal,
                start: span.start(),
                end: span.end(),
            });
            self.root_ordinals.insert(span, ordinal);
            ordinal
        };
        self.origin_slots.push(ordinal);
    }

    pub(crate) fn finish(self) -> tex_state::ParagraphProvenanceRecipe {
        tex_state::ParagraphProvenanceRecipe {
            piece_anchors: self.piece_anchors.into(),
            root_spans: self.root_spans.into(),
            origin_slots: self.origin_slots.into(),
            node_slots: std::sync::Arc::from([]),
        }
    }
}
