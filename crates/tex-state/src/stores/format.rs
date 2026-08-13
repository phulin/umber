use super::*;
use serde::{Deserialize, Serialize};

mod node;
use node::{FormatContentIds, FormatNode};

mod frozen_core;
mod frozen_env;
mod frozen_node;
mod frozen_non_node;

mod font_validation;
#[cfg(test)]
mod tests;
#[cfg(test)]
pub(crate) use font_validation::{TestingFontFormatCorruption, testing_corrupt_font_format};

pub(crate) use frozen_core::{
    FrozenCoreSections, GLUE_SECTION, MACROS_SECTION, NAMES_LOOKUP_SECTION, NAMES_SECTION,
    TOKEN_LISTS_SECTION,
};
pub(crate) use frozen_env::FROZEN_ENV_SECTION;
pub(crate) use frozen_node::{FROZEN_NODES_SECTION, FrozenNodeSection};
pub(crate) use frozen_non_node::{
    CODE_TABLES_SECTION, FONTS_SECTION, FrozenNonNodeSections, HYPHENATION_SECTION,
};

#[cfg(test)]
pub(crate) fn testing_frozen_environment_shape(payload: &[u8]) -> usize {
    frozen_env::decode(payload)
        .expect("test frozen environment payload")
        .len()
}

#[cfg(test)]
pub(crate) fn testing_corrupt_environment_macro_reference(payload: &[u8]) -> Vec<u8> {
    let mut entries = frozen_env::decode(payload).expect("test frozen environment payload");
    let entry = entries
        .iter_mut()
        .find(|entry| {
            crate::cell::CellId::from_raw(entry.cell)
                .is_some_and(|cell| cell.bank() == crate::cell::BankTag::Meaning)
        })
        .expect("test overlay has a meaning entry");
    entry.value = FormatEnvValue::Raw(
        crate::meaning::Meaning::Macro {
            flags: crate::meaning::MeaningFlags::EMPTY,
            definition: MacroDefinitionId::new(u32::MAX),
        }
        .encode(),
    );
    frozen_env::encode(&entries).expect("corrupted frozen environment serializes")
}

#[cfg(test)]
pub(crate) fn testing_corrupt_environment_global_cell(payload: &[u8]) -> Vec<u8> {
    let mut entries = frozen_env::decode(payload).expect("test frozen environment payload");
    entries[0].cell |= 1_u64 << 32;
    entries.sort_unstable_by_key(|entry| entry.cell);
    frozen_env::encode(&entries).expect("corrupted frozen environment serializes")
}

#[cfg(test)]
pub(crate) fn testing_corrupt_environment_box_reference(payload: &[u8]) -> Vec<u8> {
    let mut entries = frozen_env::decode(payload).expect("test frozen environment payload");
    let entry = entries
        .iter_mut()
        .find(|entry| matches!(entry.value, FormatEnvValue::Box(_)))
        .expect("test frozen environment has a box entry");
    entry.value = FormatEnvValue::Box(FormatListKey {
        survivor_root: None,
        start: u32::MAX,
        len: 1,
    });
    frozen_env::encode(&entries).expect("corrupted frozen environment serializes")
}

pub(crate) struct EncodedStoreFormat {
    pub env: Vec<u8>,
    pub names: Vec<u8>,
    pub names_lookup: Vec<u8>,
    pub token_lists: Vec<u8>,
    pub macros: Vec<u8>,
    pub glue: Vec<u8>,
    pub fonts: Vec<u8>,
    pub code_tables: Vec<u8>,
    pub hyphenation: Vec<u8>,
    pub nodes: Vec<u8>,
}

impl EncodedStoreFormat {
    pub(crate) fn payload_len(&self) -> usize {
        self.env
            .len()
            .saturating_add(self.names.len())
            .saturating_add(self.names_lookup.len())
            .saturating_add(self.token_lists.len())
            .saturating_add(self.macros.len())
            .saturating_add(self.glue.len())
            .saturating_add(self.fonts.len())
            .saturating_add(self.code_tables.len())
            .saturating_add(self.hyphenation.len())
            .saturating_add(self.nodes.len())
    }
}

#[derive(Debug)]
pub(crate) enum StoreFormatError {
    OpenGroups(u32),
    Codec(String),
    Invalid(&'static str),
    InvalidFontMetrics {
        font: usize,
        source: FontMetricsValidationError,
    },
}

#[derive(Clone, Debug)]
struct StoreFormat {
    names: Vec<FormatName>,
    token_lists: Vec<Vec<FormatToken>>,
    macros: Vec<FormatMacro>,
    glue: Vec<FormatGlue>,
    fonts: Vec<FormatFont>,
    node_lists: Vec<FormatNodeList>,
    env: Vec<FormatEnvEntry>,
    code_tables: Vec<FormatCodeTables>,
    hyphenation: HyphenationTable,
    prepared_mag: Option<i32>,
    last_loaded_font: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct MainMemoryUsage {
    pub(super) variable: usize,
    pub(super) dynamic: usize,
    pub(super) dynamic_extent: usize,
}

#[derive(Clone, Debug)]
pub(super) struct MainMemoryProjection {
    usage: MainMemoryUsage,
    macro_refs: Vec<u32>,
    token_refs: Vec<u32>,
    macro_token_refs: Vec<u32>,
    /// Cached TeX allocator words by physical token slot. Mutation receipts
    /// arrive after `Env` has transferred its typed root, so the removed
    /// coordinate may already be dead even though this projection still has
    /// to subtract its previously measured allocation exactly.
    token_words: Vec<usize>,
    live_node_lists: std::collections::BTreeSet<NodeListId>,
    live_survivor_roots: std::collections::BTreeSet<crate::ids::SurvivorRootId>,
    box_root_counts: std::collections::BTreeMap<NodeListId, u32>,
    box_graphs: std::collections::BTreeMap<NodeListId, BoxMemoryProjection>,
    detached_dynamic_extent: usize,
}

struct CapturedMemoryRoots {
    env: Vec<FormatEnvEntry>,
    node_lists: Vec<FormatNodeList>,
    live_node_lists: std::collections::BTreeSet<NodeListId>,
    box_roots: Vec<NodeListId>,
}

const TEX82_UNTYPED_ONE_WORD_SCRATCH_EXTENT: usize = 4;

/// TeX82 §§125--130/200/384's live main-memory use.
///
/// Umber's immutable token, macro, glue, and node stores retain unreachable
/// history, so their backing lengths are not WEB allocator coordinates. The
/// format closure is the single existing owner of semantic reachability
/// across meanings, registers, macro bodies, and node-held lists.
pub(super) fn main_memory_usage(
    stores: &Stores,
    extra_node: Option<&Node>,
) -> Result<MainMemoryUsage, StoreFormatError> {
    let extra_nodes = extra_node.map_or(&[][..], std::slice::from_ref);
    main_memory_usage_inner(stores, extra_nodes, true)
}

pub(super) fn main_memory_usage_with_extra_dynamic_words(
    base: MainMemoryUsage,
    extra_words: usize,
) -> MainMemoryUsage {
    let dynamic = base.dynamic.saturating_add(extra_words);
    MainMemoryUsage {
        variable: base.variable,
        dynamic,
        dynamic_extent: base.dynamic_extent.max(dynamic),
    }
}

pub(super) fn main_memory_usage_without_scratch(
    stores: &Stores,
) -> Result<MainMemoryProjection, StoreFormatError> {
    main_memory_projection_inner(stores, &[], false)
}

fn main_memory_usage_inner(
    stores: &Stores,
    extra_nodes: &[Node],
    include_scratch_extent: bool,
) -> Result<MainMemoryUsage, StoreFormatError> {
    main_memory_projection_inner(stores, extra_nodes, include_scratch_extent)
        .map(|projection| projection.usage)
}

fn main_memory_projection_inner(
    stores: &Stores,
    extra_nodes: &[Node],
    include_scratch_extent: bool,
) -> Result<MainMemoryProjection, StoreFormatError> {
    let CapturedMemoryRoots {
        env,
        mut node_lists,
        live_node_lists,
        box_roots,
    } = capture_memory_roots(stores, extra_nodes)?;
    let token_count = stores.tokens.slot_len() as usize;
    let macro_count = stores.macros.watermark().definitions as usize;
    let mut macro_refs = vec![0_u32; macro_count];
    let mut token_refs = vec![0_u32; token_count];
    if let Some(empty) = token_refs.first_mut() {
        *empty = 1;
    }
    for entry in &env {
        let cell = crate::cell::CellId::from_raw(entry.cell)
            .ok_or(StoreFormatError::Invalid("unknown environment cell"))?;
        let FormatEnvValue::Raw(raw) = entry.value else {
            continue;
        };
        match cell.bank() {
            crate::cell::BankTag::Meaning => {
                if let crate::meaning::Meaning::Macro { definition, .. } =
                    crate::meaning::Meaning::decode_stored(raw)
                {
                    let definition = stores
                        .macros
                        .resolve_stored(definition)
                        .ok_or(StoreFormatError::Invalid("environment macro"))?;
                    increment_reachable(&mut macro_refs, definition.raw(), "environment macro")?;
                }
            }
            crate::cell::BankTag::Toks => {
                let raw = u32::try_from(raw)
                    .map_err(|_| StoreFormatError::Invalid("environment token list"))?;
                let id = stores.resolve_stored_token_list(TokenListId::new(raw));
                increment_reachable(&mut token_refs, id.raw(), "environment token list")?;
            }
            crate::cell::BankTag::TokParam if raw != 0 => {
                let raw = u32::try_from(raw - 1)
                    .map_err(|_| StoreFormatError::Invalid("environment token list"))?;
                let id = stores.resolve_stored_token_list(TokenListId::new(raw));
                increment_reachable(&mut token_refs, id.raw(), "environment token list")?;
            }
            _ => {}
        }
    }

    let mut macro_token_refs = vec![0_u32; token_count];
    let mut token_words = vec![0_usize; token_count];
    if let Some(empty) = token_words.first_mut() {
        *empty = stores
            .tokens
            .get(TokenListId::EMPTY)
            .len()
            .saturating_add(1);
    }
    let mut macro_words = 0_usize;
    let mut live_macro_count = 0_usize;
    for (raw, &refs) in macro_refs.iter().enumerate() {
        if refs == 0 {
            continue;
        }
        live_macro_count = live_macro_count.saturating_add(1);
        let id = stores
            .macros
            .resolve_stored(MacroDefinitionId::new(raw as u32))
            .ok_or(StoreFormatError::Invalid("macro definition"))?;
        let definition = stores.macros.get(id);
        for list_id in [definition.parameter_text(), definition.replacement_text()] {
            let list_id = stores.resolve_stored_token_list(list_id);
            let index = list_id.raw() as usize;
            let list = stores.tokens.get(list_id);
            token_words[index] = list.len().saturating_add(1);
            macro_token_refs[index] = macro_token_refs[index].saturating_add(1);
            macro_words = macro_words.saturating_add(list.len());
        }
    }
    for list in &mut node_lists {
        for node in &mut list.nodes {
            let mut invalid = false;
            node.visit_token_list_refs(|raw| {
                let id = stores.resolve_stored_token_list(TokenListId::new(*raw));
                invalid |=
                    increment_reachable(&mut token_refs, id.raw(), "node token list").is_err();
            });
            if invalid {
                return Err(StoreFormatError::Invalid("node token-list reference"));
            }
        }
    }
    let ordinary_token_words = token_refs
        .iter()
        .enumerate()
        .filter(|(index, refs)| **refs != 0 && *index != 0 && macro_token_refs[*index] == 0)
        .map(|(index, _)| {
            let id = stores.resolve_stored_token_list(TokenListId::new(index as u32));
            let words = stores.tokens.get(id).len().saturating_add(1);
            token_words[index] = words;
            words
        })
        .sum::<usize>();
    // §§200/384 allocate one reference-count head and one `end_match` word
    // around each macro definition. Umber stores its parameter and
    // replacement sequences as separate hash-consed lists, so counting those
    // host list heads (or eqtb aliases of the definition) would report the
    // representation rather than TeX's one-word allocator.
    let macro_words = macro_words.saturating_add(live_macro_count.saturating_mul(2));
    // pdfTeX §130 reserves 15 static high-memory words. Its merged e-TeX
    // runtime keeps five additional one-word allocator positions outside the
    // typed token closure; these are profile coordinates, not host objects.
    // Diagnostic replacement children preserve §182 display after the live
    // list has been directly mutated; they are not a second allocation of the
    // same TeX nodes for §§125/127 live accounting. The largest detached
    // physical branch did occupy the allocator before direct mutation, so it
    // remains part of §1334's coordinate extent.
    let mut node_roots = env
        .iter()
        .filter_map(|entry| match entry.value {
            FormatEnvValue::Box(key) => Some(key),
            FormatEnvValue::Raw(_) => None,
        })
        .collect::<Vec<_>>();
    node_roots.extend(
        node_lists
            .iter()
            .map(|list| list.key)
            .filter(|key| key.start == u32::MAX),
    );
    let (node_low_words, node_high_words, detached_dynamic_extent) =
        node_memory_words(&node_lists, node_roots);
    // Section 133's five fixed glue specifications occupy the 21 static low
    // words. Every additional reachable §150 glue specification owns four
    // variable-size words independently of its two-word glue node.
    let variable = 21_usize
        .saturating_add(
            (stores.glue.watermark().specs as usize)
                .saturating_sub(5)
                .saturating_mul(4),
        )
        .saturating_add(node_low_words);
    let dynamic = 20_usize
        .saturating_add(ordinary_token_words)
        .saturating_add(macro_words)
        .saturating_add(node_high_words);
    // Section 125's coordinate survives the engine's four one-word scratch
    // allocations outside the retained typed closure. They may be returned to
    // `avail`, but §1334 reports the smallest coordinate ever reached, not
    // live occupancy. A construction-time extra list is itself a separate
    // allocator observation, so it must not charge those historical scratch
    // positions again.
    let scratch_extent = if include_scratch_extent {
        TEX82_UNTYPED_ONE_WORD_SCRATCH_EXTENT
    } else {
        0
    };
    let dynamic_extent = dynamic
        .saturating_add(detached_dynamic_extent)
        .saturating_add(scratch_extent);
    let mut box_root_counts = std::collections::BTreeMap::new();
    for root in box_roots {
        let count = box_root_counts.entry(root).or_insert(0_u32);
        *count = count.saturating_add(1);
    }
    Ok(MainMemoryProjection {
        usage: MainMemoryUsage {
            variable,
            dynamic,
            dynamic_extent,
        },
        macro_refs,
        token_refs,
        macro_token_refs,
        token_words,
        live_node_lists,
        live_survivor_roots: box_root_counts
            .keys()
            .filter_map(|id| match id.arena() {
                crate::ids::ArenaRef::Survivor(root) => Some(root),
                crate::ids::ArenaRef::Epoch => None,
            })
            .collect(),
        box_root_counts,
        box_graphs: std::collections::BTreeMap::new(),
        detached_dynamic_extent,
    })
}

fn node_memory_words(
    node_lists: &[FormatNodeList],
    roots: impl IntoIterator<Item = FormatListKey>,
) -> (usize, usize, usize) {
    let lists_by_key = node_lists
        .iter()
        .map(|list| (list.key, list))
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut semantic_lists = std::collections::BTreeSet::new();
    let mut stack = roots.into_iter().collect::<Vec<_>>();
    while let Some(key) = stack.pop() {
        if !semantic_lists.insert(key) {
            continue;
        }
        if let Some(list) = lists_by_key.get(&key) {
            stack.extend(
                list.nodes
                    .iter()
                    .flat_map(FormatNode::semantic_children)
                    .flatten(),
            );
        }
    }
    let (low_words, high_words) = node_lists
        .iter()
        .filter(|list| semantic_lists.contains(&list.key))
        .flat_map(|list| &list.nodes)
        .fold((0_usize, 0_usize), |(low, high), node| {
            let (node_low, node_high) = node.tex82_memory_words();
            (low.saturating_add(node_low), high.saturating_add(node_high))
        });
    let detached_extent = node_lists
        .iter()
        .flat_map(|list| &list.nodes)
        .filter_map(FormatNode::diagnostic_children)
        .map(|root| {
            let mut words = 0_usize;
            let mut seen = std::collections::BTreeSet::new();
            let mut stack = vec![root];
            while let Some(key) = stack.pop() {
                if semantic_lists.contains(&key) || !seen.insert(key) {
                    continue;
                }
                if let Some(list) = lists_by_key.get(&key) {
                    words = words.saturating_add(
                        list.nodes
                            .iter()
                            .map(|node| node.tex82_memory_words().1)
                            .sum::<usize>(),
                    );
                    stack.extend(
                        list.nodes
                            .iter()
                            .flat_map(FormatNode::semantic_children)
                            .flatten(),
                    );
                }
            }
            words
        })
        .max()
        .unwrap_or(0);
    (low_words, high_words, detached_extent)
}

impl MainMemoryProjection {
    pub(super) const fn usage(&self) -> MainMemoryUsage {
        self.usage
    }

    pub(super) fn update_glue_specs(&mut self, old_specs: u32, new_specs: u32) {
        let old_words = (old_specs as usize).saturating_sub(5).saturating_mul(4);
        let new_words = (new_specs as usize).saturating_sub(5).saturating_mul(4);
        self.usage.variable = self
            .usage
            .variable
            .saturating_sub(old_words)
            .saturating_add(new_words);
    }

    pub(super) fn usage_with_extra_nodes(
        &self,
        stores: &Stores,
        extra_nodes: &[Node],
    ) -> Result<MainMemoryUsage, StoreFormatError> {
        if extra_nodes.is_empty() {
            return Ok(self.usage);
        }
        let mut node_lists = capture_extra_memory_nodes(
            stores,
            extra_nodes,
            &self.live_node_lists,
            &self.live_survivor_roots,
        )?;
        let mut extra_tokens = std::collections::BTreeSet::new();
        for node in node_lists.iter_mut().flat_map(|list| &mut list.nodes) {
            node.visit_token_list_refs(|raw| {
                let id = stores.resolve_stored_token_list(TokenListId::new(*raw));
                let index = id.raw() as usize;
                if index != 0
                    && self.token_refs.get(index).copied().unwrap_or(0) == 0
                    && self.macro_token_refs.get(index).copied().unwrap_or(0) == 0
                {
                    extra_tokens.insert(id);
                }
            });
        }
        let extra_token_words = extra_tokens
            .into_iter()
            .map(|id| stores.tokens.get(id).len().saturating_add(1))
            .sum::<usize>();

        let roots = node_lists
            .iter()
            .map(|list| list.key)
            .filter(|key| key.start == u32::MAX);
        let (node_low_words, node_high_words, detached_dynamic_extent) =
            node_memory_words(&node_lists, roots);
        let variable = self.usage.variable.saturating_add(node_low_words);
        let dynamic = self
            .usage
            .dynamic
            .saturating_add(extra_token_words)
            .saturating_add(node_high_words);
        Ok(MainMemoryUsage {
            variable,
            dynamic,
            dynamic_extent: dynamic
                .saturating_add(self.detached_dynamic_extent.max(detached_dynamic_extent)),
        })
    }

    pub(super) fn update_cell(
        &mut self,
        stores: &Stores,
        cell: crate::cell::CellId,
        old_word: u64,
        new_word: u64,
    ) -> Result<bool, StoreFormatError> {
        self.macro_refs
            .resize(stores.macros.watermark().definitions as usize, 0);
        let token_count = stores.tokens.slot_len() as usize;
        self.token_refs.resize(token_count, 0);
        self.macro_token_refs.resize(token_count, 0);
        self.token_words.resize(token_count, 0);
        match cell.bank() {
            crate::cell::BankTag::Meaning => {
                self.adjust_meaning(stores, old_word, false)?;
                self.adjust_meaning(stores, new_word, true)?;
            }
            crate::cell::BankTag::Toks => {
                self.adjust_token(stores, old_word, false, false)?;
                self.adjust_token(stores, new_word, false, true)?;
            }
            crate::cell::BankTag::TokParam => {
                self.adjust_token(stores, old_word, true, false)?;
                self.adjust_token(stores, new_word, true, true)?;
            }
            crate::cell::BankTag::Box => {
                return self.update_box_root(
                    stores,
                    NodeListId::decode_box_word(old_word),
                    NodeListId::decode_box_word(new_word),
                    false,
                );
            }
            _ => return Ok(true),
        }
        self.usage.dynamic_extent = self
            .usage
            .dynamic
            .saturating_add(self.detached_dynamic_extent);
        Ok(true)
    }

    pub(super) fn update_box_root(
        &mut self,
        stores: &Stores,
        old: Option<NodeListId>,
        new: Option<NodeListId>,
        capture_missing: bool,
    ) -> Result<bool, StoreFormatError> {
        if old == new {
            return Ok(true);
        }

        // The projection already owns the exact box-root multiplicities from
        // the preceding environment state. Update those counts at the same
        // handoff as the graph contribution instead of rescanning unrelated
        // meaning and token roots for every box assignment.
        let remove_old = if let Some(old) = old {
            let Some(count) = self.box_root_counts.get_mut(&old) else {
                return Ok(false);
            };
            *count = count.saturating_sub(1);
            if *count == 0 {
                self.box_root_counts.remove(&old);
                true
            } else {
                false
            }
        } else {
            false
        };
        let add_new = if let Some(new) = new {
            let count = self.box_root_counts.entry(new).or_insert(0);
            let add = *count == 0;
            *count = count.saturating_add(1);
            add
        } else {
            false
        };
        let old_graph = if remove_old {
            let old = old.expect("checked old box root");
            match self.box_graphs.get(&old).cloned() {
                Some(graph) => Some(graph),
                None if capture_missing => {
                    let graph = box_memory_projection(stores, old)?;
                    self.box_graphs.insert(old, graph.clone());
                    Some(graph)
                }
                None => return Ok(false),
            }
        } else {
            None
        };
        let new_graph = if add_new {
            let new = new.expect("checked new box root");
            match self.box_graphs.get(&new).cloned() {
                Some(graph) => Some(graph),
                None if capture_missing => {
                    let graph = box_memory_projection(stores, new)?;
                    self.box_graphs.insert(new, graph.clone());
                    Some(graph)
                }
                None => return Ok(false),
            }
        } else {
            None
        };
        let old_low_words = old_graph.as_ref().map_or(0, |graph| graph.low_words);
        let old_high_words = old_graph.as_ref().map_or(0, |graph| graph.high_words);
        let new_low_words = new_graph.as_ref().map_or(0, |graph| graph.low_words);
        let new_high_words = new_graph.as_ref().map_or(0, |graph| graph.high_words);
        self.usage.variable = self
            .usage
            .variable
            .saturating_sub(old_low_words)
            .saturating_add(new_low_words);
        self.usage.dynamic = self
            .usage
            .dynamic
            .saturating_sub(old_high_words)
            .saturating_add(new_high_words);
        if let Some(old_graph) = old_graph {
            for raw in old_graph.token_refs.iter().copied() {
                self.adjust_token(stores, u64::from(raw), false, false)?;
            }
            if let Some(old) = old
                && let crate::ids::ArenaRef::Survivor(root) = old.arena()
            {
                self.live_survivor_roots.remove(&root);
            }
        }
        if let Some(new_graph) = new_graph {
            for raw in new_graph.token_refs.iter().copied() {
                self.adjust_token(stores, u64::from(raw), false, true)?;
            }
            if let Some(new) = new
                && let crate::ids::ArenaRef::Survivor(root) = new.arena()
            {
                self.live_survivor_roots.insert(root);
            }
            self.detached_dynamic_extent = self
                .detached_dynamic_extent
                .max(new_graph.detached_dynamic_extent);
        }
        self.usage.dynamic_extent = self
            .usage
            .dynamic
            .saturating_add(self.detached_dynamic_extent);
        Ok(true)
    }

    fn adjust_meaning(
        &mut self,
        stores: &Stores,
        word: u64,
        add: bool,
    ) -> Result<(), StoreFormatError> {
        let crate::meaning::Meaning::Macro { definition, .. } =
            crate::meaning::Meaning::decode_stored(word)
        else {
            return Ok(());
        };
        let definition = stores
            .macros
            .resolve_stored(definition)
            .ok_or(StoreFormatError::Invalid("environment macro"))?;
        let index = definition.raw() as usize;
        let refs = *self
            .macro_refs
            .get(index)
            .ok_or(StoreFormatError::Invalid("environment macro"))?;
        if add {
            if refs == 0 {
                self.adjust_macro_words(stores, definition, true)?;
            }
            self.macro_refs[index] = refs.saturating_add(1);
        } else {
            if refs == 0 {
                return Err(StoreFormatError::Invalid("environment macro refcount"));
            }
            let refs = refs.saturating_sub(1);
            self.macro_refs[index] = refs;
            if refs == 0 {
                self.adjust_macro_words(stores, definition, false)?;
            }
        }
        Ok(())
    }

    fn adjust_macro_words(
        &mut self,
        stores: &Stores,
        definition: MacroDefinitionId,
        add: bool,
    ) -> Result<(), StoreFormatError> {
        let definition = stores.macros.get(definition);
        let mut words = 2_usize;
        for list_id in [definition.parameter_text(), definition.replacement_text()] {
            let list_id = stores.resolve_stored_token_list(list_id);
            let index = list_id.raw() as usize;
            let list_words = stores.tokens.get(list_id).len();
            self.token_words[index] = list_words.saturating_add(1);
            words = words.saturating_add(list_words);
            let refs = self
                .macro_token_refs
                .get_mut(index)
                .ok_or(StoreFormatError::Invalid("macro token-list index"))?;
            if add {
                if *refs == 0 && index != 0 && self.token_refs[index] != 0 {
                    self.usage.dynamic = self
                        .usage
                        .dynamic
                        .saturating_sub(list_words.saturating_add(1));
                }
                *refs = refs.saturating_add(1);
            } else {
                if *refs == 0 {
                    return Err(StoreFormatError::Invalid("macro token-list refcount"));
                }
                *refs = refs.saturating_sub(1);
                if *refs == 0 && index != 0 && self.token_refs[index] != 0 {
                    self.usage.dynamic = self
                        .usage
                        .dynamic
                        .saturating_add(list_words.saturating_add(1));
                }
            }
        }
        if add {
            self.usage.dynamic = self.usage.dynamic.saturating_add(words);
        } else {
            self.usage.dynamic = self.usage.dynamic.saturating_sub(words);
        }
        Ok(())
    }

    fn adjust_token(
        &mut self,
        stores: &Stores,
        word: u64,
        optional: bool,
        add: bool,
    ) -> Result<(), StoreFormatError> {
        if optional && word == 0 {
            return Ok(());
        }
        let raw = if optional { word - 1 } else { word };
        let raw =
            u32::try_from(raw).map_err(|_| StoreFormatError::Invalid("environment token list"))?;
        let index = raw as usize;
        let refs = self
            .token_refs
            .get_mut(index)
            .ok_or(StoreFormatError::Invalid("environment token list"))?;
        let words = if add {
            let id = stores.resolve_stored_token_list(TokenListId::new(raw));
            let words = stores.tokens.get(id).len().saturating_add(1);
            self.token_words[index] = words;
            words
        } else {
            let words = self.token_words[index];
            if words == 0 {
                return Err(StoreFormatError::Invalid(
                    "environment token-list allocation",
                ));
            }
            words
        };
        if add {
            if *refs == 0 && index != 0 && self.macro_token_refs[index] == 0 {
                self.usage.dynamic = self.usage.dynamic.saturating_add(words);
            }
            *refs = refs.saturating_add(1);
        } else {
            if *refs == 0 {
                return Err(StoreFormatError::Invalid("environment token-list refcount"));
            }
            *refs = refs.saturating_sub(1);
            if *refs == 0 && index != 0 && self.macro_token_refs[index] == 0 {
                self.usage.dynamic = self.usage.dynamic.saturating_sub(words);
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
struct BoxMemoryProjection {
    token_refs: std::sync::Arc<[u32]>,
    low_words: usize,
    high_words: usize,
    detached_dynamic_extent: usize,
}

fn box_memory_projection(
    stores: &Stores,
    root: NodeListId,
) -> Result<BoxMemoryProjection, StoreFormatError> {
    let mut node_lists = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    let mut visiting = std::collections::BTreeSet::new();
    let mut survivor_roots = std::collections::BTreeMap::new();
    capture_node_list(
        stores,
        root,
        &mut seen,
        &mut visiting,
        &mut survivor_roots,
        &mut node_lists,
        None,
    )?;
    let root = FormatListKey::capture(stores, root, &mut survivor_roots);
    let mut token_refs = Vec::new();
    for node in node_lists.iter_mut().flat_map(|list| &mut list.nodes) {
        node.visit_token_list_refs(|raw| token_refs.push(*raw));
    }
    let (low_words, high_words, detached_dynamic_extent) = node_memory_words(&node_lists, [root]);
    Ok(BoxMemoryProjection {
        token_refs: token_refs.into(),
        low_words,
        high_words,
        detached_dynamic_extent,
    })
}

/// Captures only the live typed roots needed for TeX82 allocator accounting.
///
/// Unlike a format image, this diagnostic projection does not clone names,
/// fonts, code tables, hyphenation state, or unreachable immutable history.
fn capture_memory_roots(
    stores: &Stores,
    extra_nodes: &[Node],
) -> Result<CapturedMemoryRoots, StoreFormatError> {
    let mut env_words = Vec::new();
    stores.for_each_main_memory_root_word(|cell, word| {
        env_words.push(capture_env_word(stores, cell, word));
    });
    let box_roots = env_words
        .iter()
        .filter_map(|&(cell, word)| {
            (cell.bank() == crate::cell::BankTag::Box)
                .then(|| NodeListId::decode_box_word(word))
                .flatten()
        })
        .collect::<Vec<_>>();
    let mut seen = std::collections::BTreeSet::new();
    let mut visiting = std::collections::BTreeSet::new();
    let mut survivor_roots = std::collections::BTreeMap::new();
    let mut node_lists = Vec::new();
    for &root in &box_roots {
        capture_node_list(
            stores,
            root,
            &mut seen,
            &mut visiting,
            &mut survivor_roots,
            &mut node_lists,
            None,
        )?;
    }
    let live_node_lists = seen;
    let mut env = env_words
        .into_iter()
        .map(|(cell, word)| {
            let value = if cell.bank() == crate::cell::BankTag::Box {
                let id = NodeListId::decode_box_word(word)
                    .expect("non-default box memory entry should contain a list");
                FormatEnvValue::Box(FormatListKey::capture(stores, id, &mut survivor_roots))
            } else {
                FormatEnvValue::Raw(word)
            };
            FormatEnvEntry {
                cell: cell.raw(),
                value,
            }
        })
        .collect::<Vec<_>>();
    canonicalize_node_list_keys(&mut node_lists, &mut env);

    if !extra_nodes.is_empty() {
        let mut seen = std::collections::BTreeSet::new();
        let mut visiting = std::collections::BTreeSet::new();
        let mut survivor_roots = std::collections::BTreeMap::new();
        for node in extra_nodes {
            for child in crate::node_arena::NodeRef::from(node).physical_children() {
                capture_node_list(
                    stores,
                    child,
                    &mut seen,
                    &mut visiting,
                    &mut survivor_roots,
                    &mut node_lists,
                    None,
                )?;
            }
        }
        node_lists.push(FormatNodeList {
            key: FormatListKey {
                survivor_root: None,
                start: u32::MAX,
                len: u32::try_from(extra_nodes.len())
                    .map_err(|_| StoreFormatError::Invalid("extra node-list length"))?,
            },
            semantic_id: 0,
            nodes: extra_nodes
                .iter()
                .map(|node| {
                    FormatNode::capture(
                        stores,
                        crate::node_arena::NodeRef::from(node),
                        &mut survivor_roots,
                    )
                })
                .collect(),
        });
    }
    Ok(CapturedMemoryRoots {
        env,
        node_lists,
        live_node_lists,
        box_roots,
    })
}

fn capture_extra_memory_nodes(
    stores: &Stores,
    extra_nodes: &[Node],
    live_node_lists: &std::collections::BTreeSet<NodeListId>,
    live_survivor_roots: &std::collections::BTreeSet<crate::ids::SurvivorRootId>,
) -> Result<Vec<FormatNodeList>, StoreFormatError> {
    let mut seen = live_node_lists.clone();
    let mut visiting = std::collections::BTreeSet::new();
    let mut survivor_roots = std::collections::BTreeMap::new();
    let mut node_lists = Vec::new();
    for node in extra_nodes {
        for child in crate::node_arena::NodeRef::from(node).physical_children() {
            if matches!(
                child.arena(),
                crate::ids::ArenaRef::Survivor(root) if live_survivor_roots.contains(&root)
            ) {
                continue;
            }
            capture_node_list(
                stores,
                child,
                &mut seen,
                &mut visiting,
                &mut survivor_roots,
                &mut node_lists,
                None,
            )?;
        }
    }
    node_lists.push(FormatNodeList {
        key: FormatListKey {
            survivor_root: None,
            start: u32::MAX,
            len: u32::try_from(extra_nodes.len())
                .map_err(|_| StoreFormatError::Invalid("extra node-list length"))?,
        },
        semantic_id: 0,
        nodes: extra_nodes
            .iter()
            .map(|node| {
                FormatNode::capture(
                    stores,
                    crate::node_arena::NodeRef::from(node),
                    &mut survivor_roots,
                )
            })
            .collect(),
    });
    Ok(node_lists)
}

struct ImmutableStoreIdentity {
    names: Vec<FormatName>,
    token_lists: Vec<Vec<FormatToken>>,
    macros: Vec<FormatMacro>,
    glue: Vec<FormatGlue>,
    fonts: Vec<FormatFont>,
}

#[derive(Serialize)]
struct MutableStoreIdentity {
    node_lists: Vec<FormatNodeList>,
    env: Vec<FormatEnvEntry>,
    code_tables: Vec<FormatCodeTables>,
    hyphenation: HyphenationTable,
    prepared_mag: Option<i32>,
    last_loaded_font: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct FormatEnvEntry {
    cell: u64,
    value: FormatEnvValue,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
enum FormatEnvValue {
    Raw(u64),
    Box(FormatListKey),
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct FormatName {
    active: bool,
    hash_occupied: bool,
    text: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
enum FormatToken {
    Char { ch: char, cat: u8 },
    Cs(u32),
    Param(u8),
    Frozen(u16),
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct FormatMacro {
    flags: u8,
    parameter_text: u32,
    replacement_text: u32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct FormatGlue {
    width: i32,
    stretch: i32,
    stretch_order: u8,
    shrink: i32,
    shrink_order: u8,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct FormatFont {
    name: String,
    content_hash: [u8; 32],
    checksum: u32,
    design_size: i32,
    size: i32,
    parameters: Vec<i32>,
    source_parameters: Vec<i32>,
    font_info_words: u32,
    characters: Vec<Option<tex_fonts::CharMetrics>>,
    lig_kern_program: Vec<tex_fonts::LigKernInstruction>,
    right_boundary_char: Option<u8>,
    left_boundary_program: Option<u16>,
    extensible_recipes: Vec<tex_fonts::metrics::ExtensibleRecipe>,
    identifier: Option<u32>,
    expansion: Option<crate::font::FontExpansion>,
    construction: FormatFontConstruction,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
enum FormatFontConstruction {
    Loaded,
    Copied {
        source: [u8; 32],
    },
    Letterspaced {
        source: [u8; 32],
        amount: i16,
        no_ligatures: bool,
    },
    Expanded {
        source: [u8; 32],
        ratio: i16,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct FormatCodeTables {
    code: u32,
    catcode: u8,
    lccode: u32,
    uccode: u32,
    sfcode: u16,
    mathcode: u32,
    delcode: i32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
struct FormatListKey {
    survivor_root: Option<u32>,
    start: u32,
    len: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct FormatNodeList {
    key: FormatListKey,
    semantic_id: u64,
    nodes: Vec<FormatNode>,
}

#[derive(Deserialize, Serialize)]
struct MemoNodeBundle {
    names: Vec<FormatName>,
    token_lists: Vec<Vec<FormatToken>>,
    glue: Vec<FormatGlue>,
    fonts: Vec<FormatFont>,
    node_lists: Vec<FormatNodeList>,
    root: FormatListKey,
}

#[derive(Deserialize, Serialize)]
struct MemoFontBundle {
    font: FormatFont,
    identifier: Option<FormatName>,
}

fn capture_env_word(
    stores: &Stores,
    cell: crate::cell::CellId,
    word: u64,
) -> (crate::cell::CellId, u64) {
    let cell = crate::cell::CellId::new(cell.bank(), cell.index());
    let word = if cell.bank() == crate::cell::BankTag::CurrentFont {
        let symbol_plus_one = word >> 32;
        let symbol = if symbol_plus_one == 0 {
            0
        } else {
            u64::from(
                stores
                    .resolve_stored_symbol(Symbol::new((symbol_plus_one - 1) as u32))
                    .raw(),
            ) + 1
        };
        (symbol << 32) | u64::from(word as u32)
    } else {
        word
    };
    (cell, word)
}

fn restore_current_font_word(stores: &Stores, word: u64) -> Result<u64, StoreFormatError> {
    let symbol_plus_one = word >> 32;
    let symbol = if symbol_plus_one == 0 {
        0
    } else {
        let slot = u32::try_from(symbol_plus_one - 1)
            .map_err(|_| StoreFormatError::Invalid("current-font identifier is not live"))?;
        u64::from(
            stores
                .interner
                .symbol_at_slot(slot)
                .ok_or(StoreFormatError::Invalid(
                    "current-font identifier is not live",
                ))?
                .raw(),
        ) + 1
    };
    Ok((symbol << 32) | u64::from(word as u32))
}

impl Stores {
    /// Canonical reachable semantic store root for checkpoint verification.
    ///
    /// Environment, code-table, hyphenation, and font-selection roots resolve
    /// referenced immutable values to canonical content. Append-store
    /// watermarks and unreferenced entries are absent; survivor pins and
    /// identity caches are retention metadata, not reachability authority.
    pub(crate) fn semantic_identity(&mut self) -> Result<u64, StoreFormatError> {
        if self.env.group_depth() != 0 {
            return Err(StoreFormatError::OpenGroups(self.env.group_depth()));
        }
        let mutable = self.exact_mutable_identity();
        let mut composed = Vec::with_capacity(64);
        composed.extend_from_slice(b"umber-live-reachable-store-v1");
        composed.extend_from_slice(&mutable.to_le_bytes());
        Ok(crate::state_hash::exact_identity_bytes(
            b"umber-live-reachable-store-v2",
            &composed,
        ))
    }

    pub(crate) fn encode_frozen_format(&self) -> Result<EncodedStoreFormat, StoreFormatError> {
        if self.env.group_depth() != 0 {
            return Err(StoreFormatError::OpenGroups(self.env.group_depth()));
        }
        let format = StoreFormat::capture(self)?;
        let frozen = frozen_core::encode(&format)?;
        let non_node = frozen_non_node::encode(&format)?;
        let nodes = frozen_node::encode(&format, self)?;
        let env = frozen_env::encode(&format.env)?;
        Ok(EncodedStoreFormat {
            env,
            names: frozen.names,
            names_lookup: frozen.names_lookup,
            token_lists: frozen.token_lists,
            macros: frozen.macros,
            glue: frozen.glue,
            fonts: non_node.fonts,
            code_tables: non_node.code_tables,
            hyphenation: non_node.hyphenation,
            nodes,
        })
    }

    pub(crate) fn decode_frozen_format(
        env_section: &[u8],
        sections: FrozenCoreSections<'_>,
        non_node_sections: FrozenNonNodeSections<'_>,
        node_section: FrozenNodeSection<'_>,
    ) -> Result<Self, StoreFormatError> {
        let env = frozen_env::decode(env_section)?;
        let mut core = frozen_core::decode(sections)?;
        let mut non_node = frozen_non_node::decode(non_node_sections, &core.interner)?;
        let node_lists = frozen_node::decode(node_section)?;
        let format = StoreFormat {
            names: std::mem::take(&mut core.names),
            token_lists: std::mem::take(&mut core.token_lists),
            macros: std::mem::take(&mut core.macro_rows),
            glue: std::mem::take(&mut core.glue_rows),
            fonts: std::mem::take(&mut non_node.font_rows),
            node_lists: node_lists.lists,
            env,
            code_tables: std::mem::take(&mut non_node.code_rows),
            hyphenation: std::mem::take(&mut non_node.hyphenation),
            prepared_mag: non_node.prepared_mag,
            last_loaded_font: non_node.last_loaded_font.raw(),
        };
        format.validate_references()?;
        format.validate_font_state()?;
        install_frozen_sections(format, core, non_node, node_lists.semantic_ids)
    }

    pub(crate) fn encode_memo_node_list(
        &self,
        root: NodeListId,
    ) -> Result<Vec<u8>, StoreFormatError> {
        self.encode_memo_node_list_with_origins(root)
            .map(|(bytes, _)| bytes)
    }

    pub(crate) fn encode_memo_node_list_with_origins(
        &self,
        root: NodeListId,
    ) -> Result<(Vec<u8>, Vec<crate::token::OriginId>), StoreFormatError> {
        let names = (0..self.interner.len())
            .map(|raw| {
                let symbol = self
                    .interner
                    .symbol_at_slot(raw as u32)
                    .expect("captured interner slot should be live");
                FormatName {
                    active: self.interner.kind(symbol) == ControlSequenceKind::ActiveCharacter,
                    hash_occupied: self.interner.is_hash_entry(symbol),
                    text: self.interner.resolve(symbol).to_owned(),
                }
            })
            .collect();
        let token_lists = (0..self.tokens.slot_len())
            .map(|raw| {
                self.tokens
                    .get(self.resolve_stored_token_list(TokenListId::new(raw)))
                    .iter()
                    .copied()
                    .map(|token| FormatToken::capture(self, token))
                    .collect()
            })
            .collect();
        let glue_mark = self.glue.watermark();
        let glue = (0..glue_mark.specs)
            .map(|raw| FormatGlue::capture(self.glue.stored_slot(raw).spec()))
            .collect();
        let font_mark = self.fonts.watermark();
        let fonts = (0..font_mark.len)
            .map(|raw| FormatFont::capture(&self.fonts, self.resolve_stored_font(FontId::new(raw))))
            .collect();
        let mut seen = std::collections::BTreeSet::new();
        let mut visiting = std::collections::BTreeSet::new();
        let mut survivor_roots = std::collections::BTreeMap::new();
        let mut node_lists = Vec::new();
        let mut origins = Vec::new();
        capture_node_list(
            self,
            root,
            &mut seen,
            &mut visiting,
            &mut survivor_roots,
            &mut node_lists,
            Some(&mut origins),
        )?;
        let root = FormatListKey::capture(self, root, &mut survivor_roots);
        let bytes = bincode::serialize(&MemoNodeBundle {
            names,
            token_lists,
            glue,
            fonts,
            node_lists,
            root,
        })
        .map_err(|error| StoreFormatError::Codec(error.to_string()))?;
        Ok((bytes, origins))
    }

    pub(crate) fn import_memo_node_list(
        &mut self,
        bytes: &[u8],
        max_nodes: usize,
        max_tokens: usize,
        max_string_bytes: usize,
    ) -> Result<NodeListId, StoreFormatError> {
        self.import_memo_node_list_with_origins(bytes, max_nodes, max_tokens, max_string_bytes, &[])
    }

    pub(crate) fn import_memo_node_list_with_origins(
        &mut self,
        bytes: &[u8],
        max_nodes: usize,
        max_tokens: usize,
        max_string_bytes: usize,
        origins: &[crate::token::OriginId],
    ) -> Result<NodeListId, StoreFormatError> {
        let bundle: MemoNodeBundle = bincode::deserialize(bytes)
            .map_err(|error| StoreFormatError::Codec(error.to_string()))?;
        let node_count = bundle
            .node_lists
            .iter()
            .try_fold(0usize, |total, list| total.checked_add(list.nodes.len()));
        if node_count.is_none_or(|count| count > max_nodes) {
            return Err(StoreFormatError::Invalid("memo node budget exceeded"));
        }
        let token_count = bundle
            .token_lists
            .iter()
            .try_fold(0usize, |total, list| total.checked_add(list.len()));
        if token_count.is_none_or(|count| count > max_tokens) {
            return Err(StoreFormatError::Invalid("memo token budget exceeded"));
        }
        let string_bytes = bundle
            .names
            .iter()
            .map(|name| name.text.len())
            .chain(bundle.fonts.iter().map(|font| font.name.len()))
            .try_fold(0usize, usize::checked_add);
        if string_bytes.is_none_or(|count| count > max_string_bytes) {
            return Err(StoreFormatError::Invalid("memo string budget exceeded"));
        }

        let mut symbols = Vec::with_capacity(bundle.names.len());
        let mut symbol_ids = Vec::with_capacity(bundle.names.len());
        for name in bundle.names {
            let id = if name.active {
                let mut chars = name.text.chars();
                let ch = chars
                    .next()
                    .ok_or(StoreFormatError::Invalid("empty active name"))?;
                if chars.next().is_some() {
                    return Err(StoreFormatError::Invalid("multi-character active name"));
                }
                self.intern_active_character(ch)
            } else if name.hash_occupied {
                self.try_intern_hash(&name.text)
                    .map_err(|_| StoreFormatError::Invalid("memo control-sequence capacity"))?
            } else {
                self.intern(&name.text)
            };
            symbols.push(id.symbol());
            symbol_ids.push(id);
        }

        let mut token_ids = Vec::with_capacity(bundle.token_lists.len());
        for tokens in bundle.token_lists {
            let tokens = tokens
                .into_iter()
                .map(|token| token.restore_mapped(&symbols))
                .collect::<Result<Vec<_>, _>>()?;
            token_ids.push(self.intern_token_list_ref_in_domain(&tokens, None));
        }
        let mut glue_ids = Vec::with_capacity(bundle.glue.len());
        for glue in bundle.glue {
            glue_ids.push(self.intern_glue_in_domain(glue.restore()?, None));
        }
        let mut font_ids = Vec::with_capacity(bundle.fonts.len());
        for (raw, font) in bundle.fonts.into_iter().enumerate() {
            if raw == 0 {
                font_ids.push(NULL_FONT);
                continue;
            }
            let identifier = font.identifier;
            let id = match identifier {
                Some(symbol) => {
                    let symbol = symbol_ids
                        .get(symbol as usize)
                        .copied()
                        .ok_or(StoreFormatError::Invalid("font identifier symbol"))?;
                    self.try_intern_font_with_identifier(font.restore(), symbol)
                }
                None => self.try_intern_font(font.restore()),
            }
            .map_err(|_| StoreFormatError::Invalid("memo font capacity"))?;
            font_ids.push(id);
        }

        let content_ids = FormatContentIds {
            fonts: &font_ids,
            glue: &glue_ids,
            token_lists: &token_ids,
        };
        let mut node_ids = std::collections::BTreeMap::new();
        let mut origins = origins.iter().copied();
        for list in bundle.node_lists {
            let nodes = list
                .nodes
                .into_iter()
                .map(|node| node.restore_with_origins(&content_ids, &node_ids, &mut origins))
                .collect::<Result<Vec<_>, _>>()?;
            let id = self.freeze_node_list(&nodes);
            node_ids.insert(list.key, id);
        }
        node_ids
            .get(&bundle.root)
            .copied()
            .ok_or(StoreFormatError::Invalid("memo root is missing"))
    }

    pub(crate) fn encode_memo_font(&self, id: FontId) -> Result<Vec<u8>, StoreFormatError> {
        let id = self.resolve_stored_font(id);
        let mut font = FormatFont::capture(&self.fonts, id);
        let identifier = font.identifier.take().map(|raw| {
            let symbol = self
                .interner
                .symbol_at_slot(raw)
                .expect("font identifier symbol should be live");
            FormatName {
                active: self.interner.kind(symbol) == ControlSequenceKind::ActiveCharacter,
                hash_occupied: self.interner.is_hash_entry(symbol),
                text: self.interner.resolve(symbol).to_owned(),
            }
        });
        bincode::serialize(&MemoFontBundle { font, identifier })
            .map_err(|error| StoreFormatError::Codec(error.to_string()))
    }

    pub(crate) fn import_memo_font(&mut self, bytes: &[u8]) -> Result<FontId, StoreFormatError> {
        let bundle: MemoFontBundle = bincode::deserialize(bytes)
            .map_err(|error| StoreFormatError::Codec(error.to_string()))?;
        let font = bundle.font.restore();
        let result = match bundle.identifier {
            Some(name) => {
                let symbol = if name.active {
                    let mut chars = name.text.chars();
                    let ch = chars
                        .next()
                        .ok_or(StoreFormatError::Invalid("empty active font identifier"))?;
                    if chars.next().is_some() {
                        return Err(StoreFormatError::Invalid(
                            "multi-character active font identifier",
                        ));
                    }
                    self.intern_active_character(ch)
                } else if name.hash_occupied {
                    self.try_intern_hash(&name.text)
                        .map_err(|_| StoreFormatError::Invalid("memo control-sequence capacity"))?
                } else {
                    self.intern(&name.text)
                };
                self.try_intern_font_with_identifier(font, symbol)
            }
            None => self.try_intern_font(font),
        };
        result.map_err(|_| StoreFormatError::Invalid("memo font capacity"))
    }
}

impl StoreFormat {
    fn capture(stores: &Stores) -> Result<Self, StoreFormatError> {
        Self::capture_with_extra_nodes(stores, &[])
    }

    fn capture_with_extra_nodes(
        stores: &Stores,
        extra_nodes: &[Node],
    ) -> Result<Self, StoreFormatError> {
        let immutable = ImmutableStoreIdentity::capture(stores);
        let mut mutable = MutableStoreIdentity::capture(stores)?;
        if !extra_nodes.is_empty() {
            let mut seen = std::collections::BTreeSet::new();
            let mut visiting = std::collections::BTreeSet::new();
            let mut survivor_roots = std::collections::BTreeMap::new();
            for node in extra_nodes {
                for child in crate::node_arena::NodeRef::from(node).physical_children() {
                    capture_node_list(
                        stores,
                        child,
                        &mut seen,
                        &mut visiting,
                        &mut survivor_roots,
                        &mut mutable.node_lists,
                        None,
                    )?;
                }
            }
            mutable.node_lists.push(FormatNodeList {
                key: FormatListKey {
                    survivor_root: None,
                    start: u32::MAX,
                    len: u32::try_from(extra_nodes.len())
                        .map_err(|_| StoreFormatError::Invalid("extra node-list length"))?,
                },
                semantic_id: 0,
                nodes: extra_nodes
                    .iter()
                    .map(|node| {
                        FormatNode::capture(
                            stores,
                            crate::node_arena::NodeRef::from(node),
                            &mut survivor_roots,
                        )
                    })
                    .collect(),
            });
        }
        let mut format = Self {
            names: immutable.names,
            token_lists: immutable.token_lists,
            macros: immutable.macros,
            glue: immutable.glue,
            fonts: immutable.fonts,
            node_lists: mutable.node_lists,
            env: mutable.env,
            code_tables: mutable.code_tables,
            hyphenation: mutable.hyphenation,
            prepared_mag: mutable.prepared_mag,
            last_loaded_font: mutable.last_loaded_font,
        };
        format.retain_reachable_format_closure()?;
        Ok(format)
    }

    fn retain_reachable_format_closure(&mut self) -> Result<(), StoreFormatError> {
        use crate::cell::BankTag;

        let mut live_macros = vec![false; self.macros.len()];
        let mut live_tokens = vec![false; self.token_lists.len()];
        let mut live_glue = vec![false; self.glue.len()];
        if let Some(empty) = live_tokens.first_mut() {
            *empty = true;
        }
        if let Some(zero) = live_glue.first_mut() {
            *zero = true;
        }
        for entry in &self.env {
            let cell = crate::cell::CellId::from_raw(entry.cell)
                .ok_or(StoreFormatError::Invalid("unknown environment cell"))?;
            let FormatEnvValue::Raw(raw) = entry.value else {
                continue;
            };
            match cell.bank() {
                BankTag::Meaning => {
                    if let crate::meaning::Meaning::Macro { definition, .. } =
                        crate::meaning::Meaning::decode_stored(raw)
                    {
                        mark_reachable(&mut live_macros, definition.raw(), "meaning macro")?;
                    }
                }
                BankTag::Toks => {
                    let raw = u32::try_from(raw)
                        .map_err(|_| StoreFormatError::Invalid("environment token list"))?;
                    mark_reachable(&mut live_tokens, raw, "environment token list")?;
                }
                BankTag::TokParam => {
                    if raw != 0 {
                        let raw = u32::try_from(raw - 1)
                            .map_err(|_| StoreFormatError::Invalid("environment token list"))?;
                        mark_reachable(&mut live_tokens, raw, "environment token list")?;
                    }
                }
                BankTag::Skip | BankTag::Muskip | BankTag::GlueParam => {
                    let raw = u32::try_from(raw)
                        .map_err(|_| StoreFormatError::Invalid("environment glue"))?;
                    mark_reachable(&mut live_glue, raw, "environment glue")?;
                }
                _ => {}
            }
        }

        for (raw, definition) in self.macros.iter().enumerate() {
            if live_macros[raw] {
                mark_reachable(
                    &mut live_tokens,
                    definition.parameter_text,
                    "macro parameter token list",
                )?;
                mark_reachable(
                    &mut live_tokens,
                    definition.replacement_text,
                    "macro replacement token list",
                )?;
            }
        }
        for list in &mut self.node_lists {
            for node in &mut list.nodes {
                let mut invalid = false;
                node.visit_token_list_refs(|raw| {
                    invalid |= mark_reachable(&mut live_tokens, *raw, "node token list").is_err();
                });
                node.visit_glue_refs(|raw| {
                    invalid |= mark_reachable(&mut live_glue, *raw, "node glue").is_err();
                });
                if invalid {
                    return Err(StoreFormatError::Invalid("node immutable reference"));
                }
            }
        }

        let macro_map = dense_reachable_map(&live_macros)?;
        let token_map = dense_reachable_map(&live_tokens)?;
        let glue_map = dense_reachable_map(&live_glue)?;
        // TeX82 §256 never removes a control sequence after `id_lookup` has
        // entered it, even when its meaning remains `undefined_cs`; §1309
        // therefore dumps the complete occupied hash table, not merely names
        // reachable from non-default `eqtb` cells. Preserve that namespace
        // across a format round trip so `no_new_control_sequence` can
        // distinguish a known undefined name from §222's shared dummy slot.
        let mut live_names = vec![true; self.names.len()];
        for entry in &self.env {
            let cell = crate::cell::CellId::from_raw(entry.cell)
                .ok_or(StoreFormatError::Invalid("unknown environment cell"))?;
            if cell.bank() == BankTag::Meaning {
                mark_reachable(&mut live_names, cell.index(), "meaning symbol")?;
            }
            if cell.bank() == BankTag::CurrentFont
                && let FormatEnvValue::Raw(word) = entry.value
            {
                let symbol_plus_one = word >> 32;
                if symbol_plus_one != 0 {
                    mark_reachable(
                        &mut live_names,
                        u32::try_from(symbol_plus_one - 1)
                            .map_err(|_| StoreFormatError::Invalid("current-font identifier"))?,
                        "current-font identifier",
                    )?;
                }
            }
        }
        for (raw, list) in self.token_lists.iter().enumerate() {
            if live_tokens[raw] {
                for token in list {
                    if let FormatToken::Cs(symbol) = token {
                        mark_reachable(&mut live_names, *symbol, "token symbol")?;
                    }
                }
            }
        }
        for font in &self.fonts {
            if let Some(symbol) = font.identifier {
                mark_reachable(&mut live_names, symbol, "font identifier symbol")?;
            }
        }
        let name_map = dense_reachable_map(&live_names)?;
        for entry in &mut self.env {
            let cell = crate::cell::CellId::from_raw(entry.cell)
                .ok_or(StoreFormatError::Invalid("unknown environment cell"))?;
            if cell.bank() == BankTag::Meaning {
                entry.cell = crate::cell::CellId::new(
                    BankTag::Meaning,
                    remapped(&name_map, cell.index(), "meaning symbol")?,
                )
                .raw();
            }
            let FormatEnvValue::Raw(raw) = &mut entry.value else {
                continue;
            };
            match cell.bank() {
                BankTag::Meaning => {
                    if let crate::meaning::Meaning::Macro { flags, definition } =
                        crate::meaning::Meaning::decode_stored(*raw)
                    {
                        let definition = remapped(&macro_map, definition.raw(), "meaning macro")?;
                        *raw = crate::meaning::Meaning::Macro {
                            flags,
                            definition: MacroDefinitionId::new(definition),
                        }
                        .encode();
                    }
                }
                BankTag::Toks => {
                    let old = u32::try_from(*raw)
                        .map_err(|_| StoreFormatError::Invalid("environment token list"))?;
                    *raw = u64::from(remapped(&token_map, old, "environment token list")?);
                }
                BankTag::TokParam => {
                    if *raw != 0 {
                        let old = u32::try_from(*raw - 1)
                            .map_err(|_| StoreFormatError::Invalid("environment token list"))?;
                        *raw = u64::from(remapped(&token_map, old, "environment token list")?) + 1;
                    }
                }
                BankTag::Skip | BankTag::Muskip | BankTag::GlueParam => {
                    let old = u32::try_from(*raw)
                        .map_err(|_| StoreFormatError::Invalid("environment glue"))?;
                    *raw = u64::from(remapped(&glue_map, old, "environment glue")?);
                }
                BankTag::CurrentFont => {
                    let symbol_plus_one = *raw >> 32;
                    if symbol_plus_one != 0 {
                        let old = u32::try_from(symbol_plus_one - 1)
                            .map_err(|_| StoreFormatError::Invalid("current-font identifier"))?;
                        let symbol = remapped(&name_map, old, "current-font identifier")?;
                        *raw = (u64::from(symbol) + 1) << 32 | u64::from(*raw as u32);
                    }
                }
                _ => {}
            }
        }
        self.env.sort_unstable_by_key(|entry| entry.cell);
        for list in &mut self.node_lists {
            for node in &mut list.nodes {
                let mut invalid = false;
                node.visit_token_list_refs(|raw| {
                    match remapped(&token_map, *raw, "node token list") {
                        Ok(mapped) => *raw = mapped,
                        Err(_) => invalid = true,
                    }
                });
                node.visit_glue_refs(|raw| match remapped(&glue_map, *raw, "node glue") {
                    Ok(mapped) => *raw = mapped,
                    Err(_) => invalid = true,
                });
                if invalid {
                    return Err(StoreFormatError::Invalid("node immutable reference"));
                }
            }
        }

        self.macros = self
            .macros
            .drain(..)
            .enumerate()
            .filter_map(|(raw, mut definition)| {
                live_macros[raw].then(|| {
                    definition.parameter_text = token_map[definition.parameter_text as usize]
                        .expect("live macro parameter was marked reachable");
                    definition.replacement_text = token_map[definition.replacement_text as usize]
                        .expect("live macro replacement was marked reachable");
                    definition
                })
            })
            .collect();
        for (raw, list) in self.token_lists.iter_mut().enumerate() {
            if live_tokens[raw] {
                for token in list {
                    if let FormatToken::Cs(symbol) = token {
                        *symbol = remapped(&name_map, *symbol, "token symbol")?;
                    }
                }
            }
        }
        for font in &mut self.fonts {
            if let Some(symbol) = &mut font.identifier {
                *symbol = remapped(&name_map, *symbol, "font identifier symbol")?;
            }
        }
        self.token_lists = self
            .token_lists
            .drain(..)
            .enumerate()
            .filter_map(|(raw, tokens)| live_tokens[raw].then_some(tokens))
            .collect();
        self.glue = self
            .glue
            .drain(..)
            .enumerate()
            .filter_map(|(raw, glue)| live_glue[raw].then_some(glue))
            .collect();
        self.names = self
            .names
            .drain(..)
            .enumerate()
            .filter_map(|(raw, name)| live_names[raw].then_some(name))
            .collect();
        Ok(())
    }
}

fn mark_reachable(
    reachable: &mut [bool],
    raw: u32,
    message: &'static str,
) -> Result<(), StoreFormatError> {
    let slot = reachable
        .get_mut(raw as usize)
        .ok_or(StoreFormatError::Invalid(message))?;
    *slot = true;
    Ok(())
}

fn increment_reachable(
    reachable: &mut [u32],
    raw: u32,
    message: &'static str,
) -> Result<(), StoreFormatError> {
    let slot = reachable
        .get_mut(raw as usize)
        .ok_or(StoreFormatError::Invalid(message))?;
    *slot = slot.saturating_add(1);
    Ok(())
}

fn dense_reachable_map(reachable: &[bool]) -> Result<Vec<Option<u32>>, StoreFormatError> {
    let mut next = 0_u32;
    reachable
        .iter()
        .map(|&live| {
            if !live {
                return Ok(None);
            }
            let mapped = next;
            next = next
                .checked_add(1)
                .ok_or(StoreFormatError::Invalid("reachable store exceeds u32"))?;
            Ok(Some(mapped))
        })
        .collect()
}

fn remapped(
    mapping: &[Option<u32>],
    raw: u32,
    message: &'static str,
) -> Result<u32, StoreFormatError> {
    mapping
        .get(raw as usize)
        .copied()
        .flatten()
        .ok_or(StoreFormatError::Invalid(message))
}

impl ImmutableStoreIdentity {
    fn capture(stores: &Stores) -> Self {
        let names = (0..stores.interner.len())
            .map(|raw| {
                let symbol = stores
                    .interner
                    .symbol_at_slot(raw as u32)
                    .expect("captured interner slot should be live");
                FormatName {
                    active: stores.interner.kind(symbol) == ControlSequenceKind::ActiveCharacter,
                    hash_occupied: stores.interner.is_hash_entry(symbol),
                    text: stores.interner.resolve(symbol).to_owned(),
                }
            })
            .collect();
        let token_lists = (0..stores.tokens.slot_len())
            .map(|raw| {
                stores
                    .tokens
                    .stored_slot_tokens(raw)
                    .iter()
                    .copied()
                    .map(|token| FormatToken::capture(stores, token))
                    .collect()
            })
            .collect();
        let macro_mark = stores.macros.watermark();
        let macros = (0..macro_mark.definitions)
            .map(|raw| {
                let meaning = stores.macros.stored_slot(raw).map_or(
                    MacroMeaning::new(
                        crate::meaning::MeaningFlags::EMPTY,
                        TokenListId::EMPTY,
                        TokenListId::EMPTY,
                    ),
                    |root| root.meaning(),
                );
                FormatMacro {
                    flags: meaning.flags().bits(),
                    parameter_text: meaning.parameter_text().raw(),
                    replacement_text: meaning.replacement_text().raw(),
                }
            })
            .collect();
        let glue_mark = stores.glue.watermark();
        let glue = (0..glue_mark.specs)
            .map(|raw| FormatGlue::capture(stores.glue.stored_slot(raw).spec()))
            .collect();
        let font_mark = stores.fonts.watermark();
        let fonts = (0..font_mark.len)
            .map(|raw| {
                FormatFont::capture(&stores.fonts, stores.resolve_stored_font(FontId::new(raw)))
            })
            .collect();
        Self {
            names,
            token_lists,
            macros,
            glue,
            fonts,
        }
    }
}

impl MutableStoreIdentity {
    fn capture(stores: &Stores) -> Result<Self, StoreFormatError> {
        let mut env_words = Vec::new();
        stores.env.for_each_semantic_non_default_word(|cell, word| {
            // e-TeX change [50.1307] resets every optional e-TeX state
            // variable immediately before tex.web §1307 dumps `eqtb`.
            // `TeXXeTstate` is currently the sole member of that class; it
            // must therefore restore at its zero default even when INITEX
            // enabled it before requesting the format dump.
            if cell.bank() != crate::cell::BankTag::IntParam
                || cell.index() != u32::from(crate::env::banks::IntParam::TEX_XET_STATE.raw())
            {
                env_words.push(capture_env_word(stores, cell, word));
            }
        });
        let roots: Vec<_> = env_words
            .iter()
            .filter_map(|&(cell, word)| {
                (cell.bank() == crate::cell::BankTag::Box)
                    .then(|| NodeListId::decode_box_word(word))
                    .flatten()
            })
            .collect();
        let mut seen = std::collections::BTreeSet::new();
        let mut visiting = std::collections::BTreeSet::new();
        let mut survivor_roots = std::collections::BTreeMap::new();
        let mut node_lists = Vec::new();
        for root in roots {
            capture_node_list(
                stores,
                root,
                &mut seen,
                &mut visiting,
                &mut survivor_roots,
                &mut node_lists,
                None,
            )?;
        }
        let mut env: Vec<FormatEnvEntry> = env_words
            .into_iter()
            .map(|(cell, word)| {
                let value = if cell.bank() == crate::cell::BankTag::Box {
                    let id = NodeListId::decode_box_word(word)
                        .expect("non-default box format entry should contain a list");
                    FormatEnvValue::Box(FormatListKey::capture(stores, id, &mut survivor_roots))
                } else {
                    FormatEnvValue::Raw(word)
                };
                FormatEnvEntry {
                    cell: cell.raw(),
                    value,
                }
            })
            .collect();
        canonicalize_node_list_keys(&mut node_lists, &mut env);
        env.sort_unstable_by_key(|entry| entry.cell);
        let mut code_tables = Vec::new();
        stores.code_tables.for_each_non_default(|ch, values| {
            code_tables.push(FormatCodeTables {
                code: ch as u32,
                catcode: values.catcode as u8,
                lccode: values.lccode,
                uccode: values.uccode,
                sfcode: values.sfcode,
                mathcode: values.mathcode,
                delcode: values.delcode,
            });
        });
        Ok(Self {
            node_lists,
            env,
            code_tables,
            hyphenation: (*stores.hyphenation).clone(),
            prepared_mag: stores.prepared_mag,
            last_loaded_font: stores.last_loaded_font.raw(),
        })
    }
}

/// Publishes the already decoded and cross-section-validated schema-11 bases.
///
/// The production loader installs one frozen node root and one immutable
/// environment base; it never re-enters ordinary node sealing or Env writes.
fn install_frozen_sections(
    format: StoreFormat,
    frozen: frozen_core::DecodedFrozenCore,
    non_node: frozen_non_node::DecodedFrozenNonNode,
    semantic_ids: Vec<u64>,
) -> Result<Stores, StoreFormatError> {
    let font_count = format.fonts.len();
    let glue_count = format.glue.len();
    let token_list_count = format.token_lists.len();
    let mut stores = Stores::new();
    stores.interner = frozen.interner;
    stores.tokens = frozen.tokens;
    stores.env.install_empty_token_root(
        stores
            .tokens
            .owner(TokenListId::EMPTY)
            .expect("frozen token store owns canonical empty list"),
    );
    stores.macros = frozen.macros;
    stores.glue = frozen.glue;
    stores.fonts = non_node.fonts;
    stores.code_tables = non_node.code_tables;
    stores.hyphenation = format.hyphenation.into();
    stores.prepared_mag = non_node.prepared_mag;
    stores.last_loaded_font = non_node.last_loaded_font;
    let font_ids = (0..font_count)
        .map(|raw| stores.resolve_stored_font(FontId::new(raw as u32)))
        .collect::<Vec<_>>();
    let glue_ids = (0..glue_count)
        .map(|raw| stores.glue_ref(stores.resolve_stored_glue(GlueId::new(raw as u32))))
        .collect::<Vec<_>>();
    let token_ids = (0..token_list_count)
        .map(|raw| {
            let id = stores.resolve_stored_token_list(TokenListId::new(raw as u32));
            stores.token_list_ref(id)
        })
        .collect::<Vec<_>>();
    let content_ids = FormatContentIds {
        fonts: &font_ids,
        glue: &glue_ids,
        token_lists: &token_ids,
    };

    if semantic_ids.len() != format.node_lists.len() {
        return Err(StoreFormatError::Invalid("frozen node identity count"));
    }
    let root = stores.survivors.reserve_frozen_root();
    let mut next_start = 0_u32;
    let node_ids: std::collections::BTreeMap<_, _> = format
        .node_lists
        .iter()
        .map(|list| {
            let len = u32::try_from(list.nodes.len())
                .map_err(|_| StoreFormatError::Invalid("frozen node list exceeds u32"))?;
            let id = NodeListId::new_survivor(root, next_start, len);
            next_start = next_start
                .checked_add(len)
                .ok_or(StoreFormatError::Invalid("frozen node arena exceeds u32"))?;
            Ok((list.key, id))
        })
        .collect::<Result<_, StoreFormatError>>()?;
    let mut storage = crate::node_arena::NodeStorage::default();
    let mut spans = Vec::with_capacity(format.node_lists.len());
    let mut verified_ids = Vec::with_capacity(format.node_lists.len());
    for (list, expected_id) in format.node_lists.into_iter().zip(semantic_ids) {
        let id = node_ids
            .get(&list.key)
            .copied()
            .ok_or(StoreFormatError::Invalid("missing frozen node list"))?;
        let nodes = list
            .nodes
            .into_iter()
            .map(|node| node.restore(&content_ids, &node_ids))
            .collect::<Result<Vec<_>, _>>()?;
        let (start, len) = storage.append(&nodes);
        if start != id.start() || len != id.len() {
            return Err(StoreFormatError::Invalid("frozen node span metadata"));
        }
        spans.push((
            start,
            len,
            crate::node_arena::NodeSemanticId::unverified_frozen(expected_id),
        ));
        verified_ids.push((id, expected_id));
    }
    stores.survivors.publish_frozen_root(root, storage, spans);
    for (id, expected_fingerprint) in verified_ids {
        let nodes = stores.nodes(id).to_vec();
        let semantic_id = stores.compute_node_semantic_id(&nodes);
        if semantic_id.value() != expected_fingerprint {
            return Err(StoreFormatError::Invalid("frozen node semantic identity"));
        }
        if id.len() != 0 {
            stores.survivors.set_frozen_semantic_id(id, semantic_id);
        }
    }
    let mut base = Vec::with_capacity(format.env.len());
    for entry in format.env {
        let dto_cell = crate::cell::CellId::from_raw(entry.cell)
            .ok_or(StoreFormatError::Invalid("unknown environment cell"))?;
        let cell = crate::cell::CellId::new(dto_cell.bank(), dto_cell.index());
        let word = match (cell.bank(), entry.value) {
            (crate::cell::BankTag::Box, FormatEnvValue::Box(key)) => {
                let id = node_ids
                    .get(&key)
                    .copied()
                    .ok_or(StoreFormatError::Invalid("missing box node list"))?;
                NodeListId::encode_box_word(Some(stores.prepare_box_value(id)))
            }
            (crate::cell::BankTag::Box, FormatEnvValue::Raw(_)) => {
                return Err(StoreFormatError::Invalid("raw box environment value"));
            }
            (crate::cell::BankTag::CurrentFont, FormatEnvValue::Raw(word)) => {
                restore_current_font_word(&stores, word)?
            }
            (_, FormatEnvValue::Raw(word)) => word,
            (_, FormatEnvValue::Box(_)) => {
                return Err(StoreFormatError::Invalid("box value in non-box bank"));
            }
        };
        let token_root = match cell.bank() {
            crate::cell::BankTag::Toks => Some(
                stores
                    .tokens
                    .resolve_stored(TokenListId::new(word as u32))
                    .and_then(|id| stores.tokens.owner(id))
                    .ok_or(StoreFormatError::Invalid(
                        "frozen environment token-register owner",
                    ))?,
            ),
            crate::cell::BankTag::TokParam if word != 0 => Some(
                stores
                    .tokens
                    .resolve_stored(TokenListId::new((word - 1) as u32))
                    .and_then(|id| stores.tokens.owner(id))
                    .ok_or(StoreFormatError::Invalid(
                        "frozen environment token-parameter owner",
                    ))?,
            ),
            crate::cell::BankTag::TokParam => None,
            _ => None,
        };
        let macro_root = if cell.bank() == crate::cell::BankTag::Meaning {
            match crate::meaning::Meaning::decode_stored(word) {
                crate::meaning::Meaning::Macro { definition, .. } => Some(
                    stores
                        .macros
                        .resolve_stored(definition)
                        .and_then(|id| stores.macros.owner(id))
                        .ok_or(StoreFormatError::Invalid("frozen environment macro owner"))?,
                ),
                _ => None,
            }
        } else {
            None
        };
        let glue_root = if matches!(
            cell.bank(),
            crate::cell::BankTag::Skip
                | crate::cell::BankTag::Muskip
                | crate::cell::BankTag::GlueParam
        ) {
            let id = stores
                .glue
                .resolve_stored(GlueId::new(word as u32))
                .ok_or(StoreFormatError::Invalid("frozen environment glue owner"))?;
            Some(
                stores
                    .glue
                    .owner(id)
                    .ok_or(StoreFormatError::Invalid("frozen environment glue owner"))?,
            )
        } else {
            None
        };
        base.push(crate::env::FormatBaseCell {
            cell,
            word,
            token_root,
            macro_root,
            glue_root,
        });
    }
    stores.env.install_format_base(base);
    Ok(stores)
}

impl StoreFormat {
    fn validate_references(&self) -> Result<(), StoreFormatError> {
        if self
            .token_lists
            .first()
            .is_none_or(|tokens| !tokens.is_empty())
        {
            return Err(StoreFormatError::Invalid(
                "missing canonical empty token list",
            ));
        }
        if self.glue.is_empty() {
            return Err(StoreFormatError::Invalid("missing canonical zero glue"));
        }
        for tokens in &self.token_lists {
            for token in tokens {
                match token {
                    FormatToken::Cs(raw) if *raw as usize >= self.names.len() => {
                        return Err(StoreFormatError::Invalid("token symbol is not live"));
                    }
                    _ => {}
                }
            }
        }
        for definition in &self.macros {
            if definition.parameter_text as usize >= self.token_lists.len()
                || definition.replacement_text as usize >= self.token_lists.len()
            {
                return Err(StoreFormatError::Invalid("macro token-list reference"));
            }
        }

        let mut previous_code = None;
        for row in &self.code_tables {
            if char::from_u32(row.code).is_none() {
                return Err(StoreFormatError::Invalid("codepoint"));
            }
            if previous_code.is_some_and(|previous| previous >= row.code) {
                return Err(StoreFormatError::Invalid("non-canonical code-table order"));
            }
            previous_code = Some(row.code);
            catcode(row.catcode)?;
        }

        let mut seen_cells = std::collections::BTreeSet::new();
        for entry in &self.env {
            let cell = crate::cell::CellId::from_raw(entry.cell)
                .ok_or(StoreFormatError::Invalid("unknown environment cell"))?;
            if cell.is_global() {
                return Err(StoreFormatError::Invalid("global environment cell"));
            }
            if !seen_cells.insert((cell.bank() as u8, cell.index())) {
                return Err(StoreFormatError::Invalid("duplicate environment cell"));
            }
            let raw = match entry.value {
                FormatEnvValue::Raw(raw) => raw,
                FormatEnvValue::Box(_) if cell.bank() == crate::cell::BankTag::Box => continue,
                FormatEnvValue::Box(_) => {
                    return Err(StoreFormatError::Invalid("box value in non-box bank"));
                }
            };
            use crate::cell::BankTag;
            match cell.bank() {
                BankTag::Meaning => {
                    if cell.index() as usize >= self.names.len() {
                        return Err(StoreFormatError::Invalid("meaning symbol is not live"));
                    }
                    match crate::meaning::Meaning::decode_stored(raw) {
                        crate::meaning::Meaning::Macro { definition, .. }
                            if definition.raw() as usize >= self.macros.len() =>
                        {
                            return Err(StoreFormatError::Invalid("meaning macro is not live"));
                        }
                        crate::meaning::Meaning::Font(font)
                            if font.raw() as usize >= self.fonts.len() =>
                        {
                            return Err(StoreFormatError::Invalid("meaning font is not live"));
                        }
                        _ => {}
                    }
                }
                BankTag::Count
                | BankTag::Dimen
                | BankTag::Skip
                | BankTag::Toks
                | BankTag::Box
                | BankTag::Muskip => {
                    if cell.index() >= 32_768 {
                        return Err(StoreFormatError::Invalid("register index out of range"));
                    }
                    if matches!(cell.bank(), BankTag::Skip | BankTag::Muskip)
                        && (raw > u64::from(u32::MAX) || raw as u32 as usize >= self.glue.len())
                    {
                        return Err(StoreFormatError::Invalid("register glue is not live"));
                    }
                    if cell.bank() == BankTag::Toks
                        && (raw > u64::from(u32::MAX)
                            || raw as u32 as usize >= self.token_lists.len())
                    {
                        return Err(StoreFormatError::Invalid("register token list is not live"));
                    }
                    if cell.bank() == BankTag::Box {
                        return Err(StoreFormatError::Invalid("raw box environment value"));
                    }
                }
                BankTag::IntParam
                | BankTag::DimenParam
                | BankTag::GlueParam
                | BankTag::TokParam => {
                    if cell.index() >= crate::env::banks::PARAMETER_COUNT as u32 {
                        return Err(StoreFormatError::Invalid("parameter index out of range"));
                    }
                    if cell.bank() == BankTag::GlueParam
                        && (raw > u64::from(u32::MAX) || raw as u32 as usize >= self.glue.len())
                    {
                        return Err(StoreFormatError::Invalid("parameter glue is not live"));
                    }
                    if cell.bank() == BankTag::TokParam
                        && raw != 0
                        && (raw - 1 > u64::from(u32::MAX)
                            || (raw - 1) as u32 as usize >= self.token_lists.len())
                    {
                        return Err(StoreFormatError::Invalid(
                            "parameter token list is not live",
                        ));
                    }
                }
                BankTag::FontDimen
                | BankTag::FontParamLen
                | BankTag::FontHyphenChar
                | BankTag::FontSkewChar
                | BankTag::PdfLpCode
                | BankTag::PdfRpCode
                | BankTag::PdfEfCode
                | BankTag::PdfTagCode
                | BankTag::PdfKnbsCode
                | BankTag::PdfStbsCode
                | BankTag::PdfShbsCode
                | BankTag::PdfKnbcCode
                | BankTag::PdfKnacCode
                | BankTag::PdfNoLigatures
                | BankTag::CurrentFont
                | BankTag::MathFamilyFont => {}
            }
        }
        Ok(())
    }
}

fn canonicalize_node_list_keys(node_lists: &mut [FormatNodeList], env: &mut [FormatEnvEntry]) {
    let keys: std::collections::BTreeMap<_, _> = node_lists
        .iter()
        .enumerate()
        .map(|(index, list)| {
            (
                list.key,
                FormatListKey {
                    survivor_root: None,
                    start: u32::try_from(index).expect("format node-list count exceeds u32"),
                    len: u32::try_from(list.nodes.len()).expect("format node list exceeds u32"),
                },
            )
        })
        .collect();
    for list in node_lists {
        for node in &mut list.nodes {
            node.remap_list_keys(&keys);
        }
        list.key = keys[&list.key];
    }
    for entry in env {
        if let FormatEnvValue::Box(key) = &mut entry.value {
            *key = keys[key];
        }
    }
}

impl FormatListKey {
    fn capture(
        stores: &Stores,
        id: NodeListId,
        survivor_roots: &mut std::collections::BTreeMap<crate::ids::SurvivorRootId, u32>,
    ) -> Self {
        let (start, len) = match id.arena() {
            crate::ids::ArenaRef::Epoch => {
                let span = stores
                    .nodes
                    .span(id)
                    .expect("captured epoch node-list id must be live");
                (span.start, span.len)
            }
            crate::ids::ArenaRef::Survivor(_) => (id.start(), id.len()),
        };
        Self {
            survivor_root: match id.arena() {
                crate::ids::ArenaRef::Epoch => None,
                crate::ids::ArenaRef::Survivor(root) => Some(match survivor_roots.get(&root) {
                    Some(&detached) => detached,
                    None => {
                        let detached = u32::try_from(survivor_roots.len())
                            .expect("format survivor roots exceed u32");
                        survivor_roots.insert(root, detached);
                        detached
                    }
                }),
            },
            start,
            len,
        }
    }
}

fn capture_node_list(
    stores: &Stores,
    id: NodeListId,
    seen: &mut std::collections::BTreeSet<NodeListId>,
    visiting: &mut std::collections::BTreeSet<NodeListId>,
    survivor_roots: &mut std::collections::BTreeMap<crate::ids::SurvivorRootId, u32>,
    out: &mut Vec<FormatNodeList>,
    mut origins: Option<&mut Vec<crate::token::OriginId>>,
) -> Result<(), StoreFormatError> {
    enum Visit {
        Enter(NodeListId),
        Exit(NodeListId),
    }

    let mut stack = vec![Visit::Enter(id)];
    while let Some(visit) = stack.pop() {
        match visit {
            Visit::Enter(id) => {
                if seen.contains(&id) {
                    continue;
                }
                if !visiting.insert(id) {
                    return Err(StoreFormatError::Invalid("cyclic node-list graph"));
                }
                stack.push(Visit::Exit(id));
                let nodes = stores.nodes(id);
                for node in nodes.iter().rev() {
                    // TeX82 §§135 and 1307 dump the complete reachable
                    // memory graph behind every box list pointer. The frozen
                    // DTO likewise retains detached physical projections of
                    // §§115/162 replacement nodes for §182 diagnostics even
                    // though semantic traversal excludes them. Discovery must
                    // follow every edge capture serializes; §638 can observe
                    // this graph while accounting a shipped box.
                    for child in node.physical_children().rev() {
                        stack.push(Visit::Enter(child));
                    }
                }
            }
            Visit::Exit(id) => {
                visiting.remove(&id);
                if !seen.insert(id) {
                    continue;
                }
                let nodes = stores
                    .nodes(id)
                    .iter()
                    .map(|node| match origins.as_deref_mut() {
                        Some(origins) => {
                            FormatNode::capture_with_origins(stores, node, survivor_roots, origins)
                        }
                        None => FormatNode::capture(stores, node, survivor_roots),
                    })
                    .collect();
                out.push(FormatNodeList {
                    key: FormatListKey::capture(stores, id, survivor_roots),
                    semantic_id: stores.node_semantic_id(id).value(),
                    nodes,
                });
            }
        }
    }
    Ok(())
}

impl FormatToken {
    fn capture(stores: &Stores, token: Token) -> Self {
        match token {
            Token::Char { ch, cat } => Self::Char { ch, cat: cat as u8 },
            // Weak token slots may still be live through a detached builder
            // after its timeline-local symbol was rolled back. The reachable
            // closure below discards such a slot; use an invalid sentinel so
            // an actually reachable stale reference is rejected rather than
            // panicking while the unreachable physical table is projected.
            Token::Cs(symbol) => Self::Cs(
                stores
                    .try_resolve_stored_symbol(symbol)
                    .map_or(u32::MAX, |symbol| symbol.raw()),
            ),
            Token::Param(slot) => Self::Param(slot),
            Token::Frozen(frozen) => Self::Frozen(frozen.raw()),
        }
    }

    fn restore_mapped(self, symbols: &[Symbol]) -> Result<Token, StoreFormatError> {
        Ok(match self {
            Self::Char { ch, cat } => Token::Char {
                ch,
                cat: catcode(cat)?,
            },
            Self::Cs(raw) => Token::Cs(
                symbols
                    .get(raw as usize)
                    .copied()
                    .ok_or(StoreFormatError::Invalid("token symbol is not live"))?,
            ),
            Self::Param(slot) => Token::Param(slot),
            Self::Frozen(raw) => Token::Frozen(crate::token::FrozenToken::from_raw(raw)),
        })
    }
}

impl FormatGlue {
    fn capture(spec: GlueSpec) -> Self {
        Self {
            width: spec.width.raw(),
            stretch: spec.stretch.raw(),
            stretch_order: spec.stretch_order as u8,
            shrink: spec.shrink.raw(),
            shrink_order: spec.shrink_order as u8,
        }
    }

    fn restore(self) -> Result<GlueSpec, StoreFormatError> {
        Ok(GlueSpec {
            width: Scaled::from_raw(self.width),
            stretch: Scaled::from_raw(self.stretch),
            stretch_order: order(self.stretch_order)?,
            shrink: Scaled::from_raw(self.shrink),
            shrink_order: order(self.shrink_order)?,
        })
    }
}

impl FormatFont {
    fn capture(fonts: &FontStore, id: FontId) -> Self {
        let font = fonts.get(id);
        Self {
            name: font.name().to_owned(),
            content_hash: font.content_hash(),
            checksum: font.checksum(),
            design_size: font.design_size().raw(),
            size: font.size().raw(),
            parameters: font.parameters().iter().map(|v| v.raw()).collect(),
            source_parameters: font.source_parameters().iter().map(|v| v.raw()).collect(),
            font_info_words: font
                .font_info_words()
                .try_into()
                .expect("live font-info extent fits the TeX82 capacity"),
            characters: font.metrics().characters().to_vec(),
            lig_kern_program: font.metrics().lig_kern_program().to_vec(),
            right_boundary_char: font.metrics().right_boundary_char(),
            left_boundary_program: font.metrics().left_boundary_program(),
            extensible_recipes: font.metrics().extensible_recipes().to_vec(),
            identifier: fonts.identifier(id).map(crate::interner::SymbolId::raw),
            expansion: fonts.expansion(id),
            construction: match font.construction() {
                tex_fonts::FontConstruction::Loaded => FormatFontConstruction::Loaded,
                tex_fonts::FontConstruction::Copied { source } => FormatFontConstruction::Copied {
                    source: source.bytes(),
                },
                tex_fonts::FontConstruction::Letterspaced {
                    source,
                    amount,
                    no_ligatures,
                } => FormatFontConstruction::Letterspaced {
                    source: source.bytes(),
                    amount: *amount,
                    no_ligatures: *no_ligatures,
                },
                tex_fonts::FontConstruction::Expanded { source, ratio } => {
                    FormatFontConstruction::Expanded {
                        source: source.bytes(),
                        ratio: *ratio,
                    }
                }
            },
        }
    }

    fn restore(self) -> LoadedFont {
        let diagnostic_path = std::path::PathBuf::from(&self.name);
        let construction = match self.construction {
            FormatFontConstruction::Loaded => tex_fonts::FontConstruction::Loaded,
            FormatFontConstruction::Copied { source } => tex_fonts::FontConstruction::Copied {
                source: tex_fonts::FontSourceIdentity::from_bytes(source),
            },
            FormatFontConstruction::Letterspaced {
                source,
                amount,
                no_ligatures,
            } => tex_fonts::FontConstruction::Letterspaced {
                source: tex_fonts::FontSourceIdentity::from_bytes(source),
                amount,
                no_ligatures,
            },
            FormatFontConstruction::Expanded { source, ratio } => {
                tex_fonts::FontConstruction::Expanded {
                    source: tex_fonts::FontSourceIdentity::from_bytes(source),
                    ratio,
                }
            }
        };
        LoadedFont::new(
            self.name,
            diagnostic_path,
            self.content_hash,
            self.checksum,
            Scaled::from_raw(self.design_size),
            Scaled::from_raw(self.size),
            self.parameters.into_iter().map(Scaled::from_raw).collect(),
            FontMetrics::new(
                self.characters,
                self.lig_kern_program,
                self.right_boundary_char,
                self.left_boundary_program,
                self.extensible_recipes,
            ),
        )
        .with_font_info_words(self.font_info_words as usize)
        .with_source_parameters(
            self.source_parameters
                .into_iter()
                .map(Scaled::from_raw)
                .collect(),
        )
        .with_construction(construction)
    }

    fn metrics(&self) -> FontMetrics {
        FontMetrics::new(
            self.characters.clone(),
            self.lig_kern_program.clone(),
            self.right_boundary_char,
            self.left_boundary_program,
            self.extensible_recipes.clone(),
        )
    }
}

fn catcode(value: u8) -> Result<Catcode, StoreFormatError> {
    match value {
        0 => Ok(Catcode::Escape),
        1 => Ok(Catcode::BeginGroup),
        2 => Ok(Catcode::EndGroup),
        3 => Ok(Catcode::MathShift),
        4 => Ok(Catcode::AlignmentTab),
        5 => Ok(Catcode::EndLine),
        6 => Ok(Catcode::Parameter),
        7 => Ok(Catcode::Superscript),
        8 => Ok(Catcode::Subscript),
        9 => Ok(Catcode::Ignored),
        10 => Ok(Catcode::Space),
        11 => Ok(Catcode::Letter),
        12 => Ok(Catcode::Other),
        13 => Ok(Catcode::Active),
        14 => Ok(Catcode::Comment),
        15 => Ok(Catcode::Invalid),
        _ => Err(StoreFormatError::Invalid("catcode")),
    }
}

fn order(value: u8) -> Result<crate::glue::Order, StoreFormatError> {
    match value {
        0 => Ok(crate::glue::Order::Normal),
        1 => Ok(crate::glue::Order::Fil),
        2 => Ok(crate::glue::Order::Fill),
        3 => Ok(crate::glue::Order::Filll),
        _ => Err(StoreFormatError::Invalid("glue order")),
    }
}
