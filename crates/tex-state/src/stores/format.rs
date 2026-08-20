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
        payload_root: None,
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
    macro_words: Vec<Option<MacroMemoryProjection>>,
    token_refs: Vec<u32>,
    macro_token_refs: Vec<u32>,
    /// Cached TeX allocator words by physical token slot. Mutation receipts
    /// arrive after `Env` has transferred its typed root, so the removed
    /// coordinate may already be dead even though this projection still has
    /// to subtract its previously measured allocation exactly.
    token_words: Vec<usize>,
    box_root_counts: std::collections::BTreeMap<NodeListId, u32>,
    box_copy_projections: std::collections::BTreeMap<NodeListId, CopyNodeListProjection>,
    detached_dynamic_extent: usize,
}

struct CapturedMemoryRoots {
    env: Vec<FormatEnvEntry>,
    node_lists: Vec<FormatNodeList>,
    box_roots: Vec<(NodeListId, FormatListKey)>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CopyNodeListProjection {
    /// Variable-size words retained by the completed duplicate.
    low_words: usize,
    /// One-word cells retained by the completed duplicate.
    high_words: usize,
    /// Maximum concurrent one-word cells, including §204's active heads.
    high_peak: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MacroMemoryProjection {
    children: [(usize, usize); 2],
    words: usize,
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

pub(super) fn main_memory_usage_with_scratch_extent(mut usage: MainMemoryUsage) -> MainMemoryUsage {
    usage.dynamic_extent = usage
        .dynamic_extent
        .saturating_add(TEX82_UNTYPED_ONE_WORD_SCRATCH_EXTENT);
    usage
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
        box_roots,
    } = capture_memory_roots(stores, extra_nodes)?;
    let token_count = stores.runtime_values.token_len() as usize;
    let macro_count = stores.runtime_values.macro_len() as usize;
    let mut macro_refs = vec![0_u32; macro_count];
    let mut macro_words_by_definition = vec![None; macro_count];
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
                        .runtime_values
                        .macro_id_at(definition.raw())
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
        *empty = stores.tokens(TokenListId::EMPTY).len().saturating_add(1);
    }
    let mut macro_words = 0_usize;
    let mut live_macro_count = 0_usize;
    for (raw, &refs) in macro_refs.iter().enumerate() {
        if refs == 0 {
            continue;
        }
        live_macro_count = live_macro_count.saturating_add(1);
        let definition = stores
            .macro_definition(
                stores
                    .runtime_values
                    .macro_id_at(raw as u32)
                    .ok_or(StoreFormatError::Invalid("environment macro allocation"))?,
            )
            .meaning();
        let mut children = [(0, 0); 2];
        for (child, list_id) in children
            .iter_mut()
            .zip([definition.parameter_text(), definition.replacement_text()])
        {
            let list = stores.tokens(list_id);
            let index = list_id.raw() as usize;
            *child = (index, list.len());
            token_words[index] = list.len().saturating_add(1);
            macro_token_refs[index] = macro_token_refs[index].saturating_add(1);
            macro_words = macro_words.saturating_add(list.len());
        }
        macro_words_by_definition[raw] = Some(MacroMemoryProjection {
            children,
            words: children
                .iter()
                .map(|(_, words)| *words)
                .sum::<usize>()
                .saturating_add(2),
        });
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
            let words = stores
                .tokens(stores.resolve_stored_token_list(TokenListId::new(index as u32)))
                .len()
                .saturating_add(1);
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
        node_memory_words(&node_lists, node_roots, stores.main_memory_profile);
    // Section 133's five fixed glue specifications occupy the 21 static low
    // words. Every additional reachable §150 glue specification owns four
    // variable-size words independently of its two-word glue node.
    let variable = 21_usize
        .saturating_add(
            (stores.runtime_values.glue_len() as usize)
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
    let mut box_copy_projections = std::collections::BTreeMap::new();
    let mut copy_cache = std::collections::BTreeMap::new();
    let lists_by_key = node_lists
        .iter()
        .map(|list| (list.key, list))
        .collect::<std::collections::BTreeMap<_, _>>();
    for (root, key) in box_roots {
        let count = box_root_counts.entry(root).or_insert(0_u32);
        *count = count.saturating_add(1);
        if *count == 1 {
            box_copy_projections.insert(
                root,
                copy_node_list_projection(
                    &lists_by_key,
                    key,
                    &mut copy_cache,
                    true,
                    stores.main_memory_profile,
                )?,
            );
        }
    }
    Ok(MainMemoryProjection {
        usage: MainMemoryUsage {
            variable,
            dynamic,
            dynamic_extent,
        },
        macro_refs,
        macro_words: macro_words_by_definition,
        token_refs,
        macro_token_refs,
        token_words,
        box_root_counts,
        box_copy_projections,
        detached_dynamic_extent,
    })
}

fn copy_node_list_projection(
    lists_by_key: &std::collections::BTreeMap<FormatListKey, &FormatNodeList>,
    root: FormatListKey,
    cache: &mut std::collections::BTreeMap<(FormatListKey, bool), CopyNodeListProjection>,
    owns_head: bool,
    profile: super::StringPoolProfile,
) -> Result<CopyNodeListProjection, StoreFormatError> {
    if let Some(projection) = cache.get(&(root, owns_head)).copied() {
        return Ok(projection);
    }
    lists_by_key
        .get(&root)
        .ok_or(StoreFormatError::Invalid("copy node-list root"))?;

    enum Frame {
        List {
            key: FormatListKey,
            owns_head: bool,
            next_node: usize,
            projection: CopyNodeListProjection,
        },
        Node {
            children: [Option<(FormatListKey, bool)>; 4],
            next_child: usize,
            projection: CopyNodeListProjection,
        },
    }

    let list_frame = |key, owns_head| Frame::List {
        key,
        owns_head,
        next_node: 0,
        projection: CopyNodeListProjection {
            low_words: 0,
            high_words: 0,
            high_peak: usize::from(owns_head),
        },
    };
    let mut frames = vec![list_frame(root, owns_head)];
    let mut completed = None;
    loop {
        if let Some(child) = completed.take() {
            let Some(parent) = frames.pop() else {
                return Ok(child);
            };
            frames.push(match parent {
                Frame::List {
                    key,
                    owns_head,
                    next_node,
                    mut projection,
                } => {
                    compose_copy_projection(&mut projection, child, usize::from(owns_head));
                    Frame::List {
                        key,
                        owns_head,
                        next_node,
                        projection,
                    }
                }
                Frame::Node {
                    children,
                    next_child,
                    mut projection,
                } => {
                    compose_copy_projection(&mut projection, child, 0);
                    Frame::Node {
                        children,
                        next_child,
                        projection,
                    }
                }
            });
            continue;
        }

        let frame = frames.pop().expect("copy projection has a root frame");
        match frame {
            Frame::List {
                key,
                owns_head,
                mut next_node,
                projection,
            } => {
                let list = lists_by_key
                    .get(&key)
                    .copied()
                    .ok_or(StoreFormatError::Invalid("copy node-list root"))?;
                if let Some(node) = list.nodes.get(next_node) {
                    next_node = next_node.saturating_add(1);
                    frames.push(Frame::List {
                        key,
                        owns_head,
                        next_node,
                        projection,
                    });
                    let (projection, children) = copy_node_projection(node, profile);
                    frames.push(Frame::Node {
                        children,
                        next_child: 0,
                        projection,
                    });
                } else {
                    cache.insert((key, owns_head), projection);
                    completed = Some(projection);
                }
            }
            Frame::Node {
                children,
                mut next_child,
                projection,
            } => {
                let child = children[next_child..]
                    .iter()
                    .enumerate()
                    .find_map(|(offset, child)| child.map(|child| (offset, child)));
                if let Some((offset, (key, owns_head))) = child {
                    next_child = next_child.saturating_add(offset).saturating_add(1);
                    frames.push(Frame::Node {
                        children,
                        next_child,
                        projection,
                    });
                    if let Some(child) = cache.get(&(key, owns_head)).copied() {
                        completed = Some(child);
                    } else {
                        frames.push(list_frame(key, owns_head));
                    }
                } else {
                    completed = Some(projection);
                }
            }
        }
    }
}

fn copy_node_projection(
    node: &FormatNode,
    profile: super::StringPoolProfile,
) -> (CopyNodeListProjection, [Option<(FormatListKey, bool)>; 4]) {
    let (low_words, high_words) = node.memory_words(profile);
    if matches!(node, FormatNode::Char { .. }) {
        return (
            CopyNodeListProjection {
                low_words: 0,
                high_words: 1,
                high_peak: 1,
            },
            [None; 4],
        );
    }
    if let FormatNode::Lig { orig, .. } = node {
        return (
            CopyNodeListProjection {
                low_words,
                high_words: orig.len(),
                high_peak: orig.len().saturating_add(1),
            },
            [None; 4],
        );
    }

    let mut children = node
        .semantic_children()
        .map(|child| child.map(|child| (child, true)));
    if let FormatNode::Disc { replace, .. } = node {
        // Section 204 copies the replacement nodes later in the same
        // enclosing list, not through a recursive temporary head.
        children[2] = Some((*replace, false));
    }
    (
        CopyNodeListProjection {
            low_words,
            high_words,
            high_peak: high_words,
        },
        children,
    )
}

fn compose_copy_projection(
    projection: &mut CopyNodeListProjection,
    child: CopyNodeListProjection,
    live_head_words: usize,
) {
    projection.low_words = projection.low_words.saturating_add(child.low_words);
    projection.high_peak = projection.high_peak.max(
        live_head_words
            .saturating_add(projection.high_words)
            .saturating_add(child.high_peak),
    );
    projection.high_words = projection.high_words.saturating_add(child.high_words);
}

fn node_memory_words(
    node_lists: &[FormatNodeList],
    roots: impl IntoIterator<Item = FormatListKey>,
    profile: super::StringPoolProfile,
) -> (usize, usize, usize) {
    let roots = roots.into_iter().collect::<Vec<_>>();
    // Exact immutable payloads may be shared, but every semantic root and
    // child edge still represents an independently allocated TeX list. Cache
    // each bottom-up subtree total, then charge it once per occurrence rather
    // than once per host coordinate.
    let mut subtree_words = std::collections::BTreeMap::new();
    for list in node_lists {
        let mut words = list
            .nodes
            .iter()
            .fold((0_usize, 0_usize), |(low, high), node| {
                let (node_low, node_high) = node.memory_words(profile);
                (low.saturating_add(node_low), high.saturating_add(node_high))
            });
        for child in list
            .nodes
            .iter()
            .flat_map(FormatNode::semantic_children)
            .flatten()
        {
            if let Some(&(child_low, child_high)) = subtree_words.get(&child) {
                words.0 = words.0.saturating_add(child_low);
                words.1 = words.1.saturating_add(child_high);
            }
        }
        subtree_words.insert(list.key, words);
    }
    let (low_words, high_words) = roots
        .iter()
        .filter_map(|root| subtree_words.get(root))
        .fold((0_usize, 0_usize), |(low, high), &(root_low, root_high)| {
            (low.saturating_add(root_low), high.saturating_add(root_high))
        });
    let detached_extent = node_lists
        .iter()
        .flat_map(|list| &list.nodes)
        .filter_map(|node| {
            node.diagnostic_children()
                .map(|root| (root, node.allocator_high_cell_overlap()))
        })
        .map(|(root, allocator_high_cell_overlap)| {
            // The diagnostic branch records a detached TeX allocation, even
            // when immutable host interning gives it the same coordinate as
            // a live semantic branch. Its complete subtree therefore has an
            // independent lifetime at the historical high-water mark.
            subtree_words.get(&root).map_or(0, |&(_, high)| {
                high.saturating_sub(allocator_high_cell_overlap as usize)
            })
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
        let mut node_lists = capture_extra_memory_nodes(stores, extra_nodes)?;
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
            .map(|id| stores.tokens(id).len().saturating_add(1))
            .sum::<usize>();

        let roots = node_lists
            .iter()
            .map(|list| list.key)
            .filter(|key| key.start == u32::MAX);
        let (node_low_words, node_high_words, detached_dynamic_extent) =
            node_memory_words(&node_lists, roots, stores.main_memory_profile);
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

    pub(super) fn low_node_requests(
        &self,
        stores: &Stores,
        extra_nodes: &[Node],
    ) -> Result<Vec<usize>, StoreFormatError> {
        let node_lists = capture_extra_memory_nodes(stores, extra_nodes)?;
        Ok(node_lists
            .iter()
            .flat_map(|list| &list.nodes)
            .filter_map(|node| {
                let words = node.memory_words(stores.main_memory_profile).0;
                (words > 1).then_some(words)
            })
            .collect())
    }

    pub(super) fn usage_with_box_copy(
        &self,
        root: NodeListId,
        live_dynamic_words: usize,
    ) -> Option<MainMemoryUsage> {
        self.box_root_counts.contains_key(&root).then_some(())?;
        let copy = self.box_copy_projections.get(&root)?;
        Some(MainMemoryUsage {
            variable: self.usage.variable.saturating_add(copy.low_words),
            dynamic: self
                .usage
                .dynamic
                .saturating_add(live_dynamic_words)
                .saturating_add(copy.high_words),
            dynamic_extent: self
                .usage
                .dynamic_extent
                // The cached coordinate and command-owned cells remain live
                // until §204 has returned the complete duplicate. Compose
                // their lifetimes; an independent max loses this overlap.
                .saturating_add(live_dynamic_words)
                .saturating_add(copy.high_peak),
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
            .resize(stores.runtime_values.macro_len() as usize, 0);
        self.macro_words
            .resize(stores.runtime_values.macro_len() as usize, None);
        let token_count = stores.runtime_values.token_len() as usize;
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
        _stores: &Stores,
        old: Option<NodeListId>,
        new: Option<NodeListId>,
        _capture_missing: bool,
    ) -> Result<bool, StoreFormatError> {
        if old == new {
            return Ok(true);
        }

        // Structural box ownership moves atomically with the Env mutation.
        // Retire this derived projection and rebuild it lazily instead of
        // retaining a second graph-lifetime index for incremental accounting.
        Ok(false)
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
        let index = definition.raw() as usize;
        let refs = *self
            .macro_refs
            .get(index)
            .ok_or(StoreFormatError::Invalid("environment macro"))?;
        if add {
            if refs == 0 {
                let definition = stores
                    .runtime_values
                    .macro_id_at(definition.raw())
                    .ok_or(StoreFormatError::Invalid("environment macro"))?;
                let projection = Self::capture_macro_words(stores, definition)?;
                self.adjust_macro_words(projection, true)?;
                self.macro_words[index] = Some(projection);
            }
            self.macro_refs[index] = refs.saturating_add(1);
        } else {
            if refs == 0 {
                return Err(StoreFormatError::Invalid("environment macro refcount"));
            }
            let refs = refs.saturating_sub(1);
            self.macro_refs[index] = refs;
            if refs == 0 {
                let projection = self.macro_words[index]
                    .take()
                    .ok_or(StoreFormatError::Invalid("environment macro allocation"))?;
                self.adjust_macro_words(projection, false)?;
            }
        }
        Ok(())
    }

    fn capture_macro_words(
        stores: &Stores,
        definition: MacroDefinitionId,
    ) -> Result<MacroMemoryProjection, StoreFormatError> {
        let definition = stores.macro_definition(definition).meaning();
        let mut children = [(0, 0); 2];
        for (child, list_id) in children
            .iter_mut()
            .zip([definition.parameter_text(), definition.replacement_text()])
        {
            let list = stores.tokens(list_id);
            let index = list_id.raw() as usize;
            let list_words = list.len();
            *child = (index, list_words);
        }
        Ok(MacroMemoryProjection {
            children,
            words: children
                .iter()
                .map(|(_, words)| *words)
                .sum::<usize>()
                .saturating_add(2),
        })
    }

    fn adjust_macro_words(
        &mut self,
        projection: MacroMemoryProjection,
        add: bool,
    ) -> Result<(), StoreFormatError> {
        for (index, list_words) in projection.children {
            self.token_words[index] = list_words.saturating_add(1);
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
            self.usage.dynamic = self.usage.dynamic.saturating_add(projection.words);
        } else {
            self.usage.dynamic = self.usage.dynamic.saturating_sub(projection.words);
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
            let words = stores
                .tokens(stores.resolve_stored_token_list(TokenListId::new(raw)))
                .len()
                .saturating_add(1);
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
        .filter(|&&(cell, _)| cell.bank() == crate::cell::BankTag::Box)
        .map(|&(cell, word)| {
            let id = NodeListId::decode_box_word(word)
                .expect("non-default box memory entry should contain a list");
            let owner = stores
                .box_reg_ref(cell.index() as u16)
                .expect("non-default box memory entry should own its list");
            (id, owner)
        })
        .collect::<Vec<_>>();
    let mut seen = std::collections::BTreeSet::new();
    let mut visiting = std::collections::BTreeSet::new();
    let mut payload_roots = std::collections::BTreeMap::new();
    let mut node_lists = Vec::new();
    for (_, root) in &box_roots {
        capture_owned_node_list(
            stores,
            root.clone(),
            &mut seen,
            &mut visiting,
            &mut payload_roots,
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
                FormatEnvValue::Box(FormatListKey::capture(stores, id, &mut payload_roots))
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
    let box_roots = box_roots
        .into_iter()
        .map(|(id, _)| id)
        .zip(env.iter().filter_map(|entry| match entry.value {
            FormatEnvValue::Box(key) => Some(key),
            FormatEnvValue::Raw(_) => None,
        }))
        .collect();

    if !extra_nodes.is_empty() {
        let mut seen = live_node_lists.clone();
        let mut visiting = std::collections::BTreeSet::new();
        let mut payload_roots = std::collections::BTreeMap::new();
        for node in extra_nodes {
            let mut children = Vec::new();
            node.visit_node_lists(|child| children.push(child.clone()));
            for child in children {
                capture_owned_node_list(
                    stores,
                    child,
                    &mut seen,
                    &mut visiting,
                    &mut payload_roots,
                    &mut node_lists,
                    None,
                )?;
            }
        }
        node_lists.push(FormatNodeList {
            key: FormatListKey {
                payload_root: None,
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
                        &mut payload_roots,
                    )
                })
                .collect(),
        });
    }
    Ok(CapturedMemoryRoots {
        env,
        node_lists,
        box_roots,
    })
}

fn capture_extra_memory_nodes(
    stores: &Stores,
    extra_nodes: &[Node],
) -> Result<Vec<FormatNodeList>, StoreFormatError> {
    // TeX82 §§125--130 allocate each child edge in a newly constructed node
    // graph even when Umber can share the child's immutable host payload with
    // an already-live root. Deduplicate only within this transient graph; a
    // process-local NodeListId shared with the cached live projection is not a
    // TeX allocator identity.
    let mut seen = std::collections::BTreeSet::new();
    let mut visiting = std::collections::BTreeSet::new();
    let mut payload_roots = std::collections::BTreeMap::new();
    let mut node_lists = Vec::new();
    for node in extra_nodes {
        let mut children = Vec::new();
        node.visit_node_lists(|child| children.push(child.clone()));
        for child in children {
            capture_owned_node_list(
                stores,
                child,
                &mut seen,
                &mut visiting,
                &mut payload_roots,
                &mut node_lists,
                None,
            )?;
        }
    }
    node_lists.push(FormatNodeList {
        key: FormatListKey {
            payload_root: None,
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
                    &mut payload_roots,
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
    payload_root: Option<u32>,
    start: u32,
    len: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct FormatNodeList {
    key: FormatListKey,
    semantic_id: u64,
    nodes: Vec<FormatNode>,
}

#[derive(Clone, Deserialize, Serialize)]
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
    /// watermarks and unreferenced entries are absent; structural roots are
    /// the sole node reachability authority.
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
        let core = frozen_core::decode(sections)?;
        let non_node = frozen_non_node::decode(non_node_sections, &core.interner)?;
        let node_lists = frozen_node::decode(node_section)?;
        validate_loaded_references(
            &env,
            core.interner.len(),
            core.tokens.len(),
            core.macros.len(),
            core.glue.len(),
            non_node.font_rows.len(),
        )?;
        #[cfg(feature = "profiling")]
        crate::measurement::record_format_restore_work(1, 0, 0);
        font_validation::validate_loaded_font_state(
            &non_node.font_rows,
            core.interner.len(),
            &env,
            non_node.last_loaded_font.raw(),
        )?;
        #[cfg(feature = "profiling")]
        crate::measurement::record_format_restore_work(2, 0, 0);
        install_frozen_sections(env, node_lists, core, non_node)
    }

    pub(crate) fn encode_memo_node_list(
        &self,
        root: &crate::node_arena::NodeListRef,
    ) -> Result<Vec<u8>, StoreFormatError> {
        self.encode_memo_node_list_with_origins(root)
            .map(|(bytes, _)| bytes)
    }

    /// Detaches a graph through its structural owner.
    pub(crate) fn encode_memo_node_list_ref(
        &self,
        root: &crate::node_arena::NodeListRef,
    ) -> Result<Vec<u8>, StoreFormatError> {
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
        let token_lists = (0..self.runtime_values.token_len())
            .map(|raw| {
                self.tokens(self.resolve_stored_token_list(TokenListId::new(raw)))
                    .iter()
                    .copied()
                    .map(|token| FormatToken::capture(self, token))
                    .collect()
            })
            .collect();
        let glue = (0..self.runtime_values.glue_len())
            .map(|raw| {
                let id = self.resolve_stored_glue(GlueId::new(raw));
                FormatGlue::capture(self.glue(id))
            })
            .collect();
        let font_mark = self.fonts.watermark();
        let fonts = (0..font_mark.len)
            .map(|raw| FormatFont::capture(&self.fonts, self.resolve_stored_font(FontId::new(raw))))
            .collect();
        let mut seen = std::collections::BTreeSet::new();
        let mut visiting = std::collections::BTreeSet::new();
        let mut payload_roots = std::collections::BTreeMap::new();
        let mut node_lists = Vec::new();
        capture_owned_node_list(
            self,
            root.clone(),
            &mut seen,
            &mut visiting,
            &mut payload_roots,
            &mut node_lists,
            None,
        )?;
        let mut root = FormatListKey::capture(self, root.id(), &mut payload_roots);
        canonicalize_memo_node_list_keys(&mut node_lists, &mut root);
        bincode::serialize(&MemoNodeBundle {
            names,
            token_lists,
            glue,
            fonts,
            node_lists,
            root,
        })
        .map_err(|error| StoreFormatError::Codec(error.to_string()))
    }

    pub(crate) fn encode_memo_node_list_with_origins(
        &self,
        root: &crate::node_arena::NodeListRef,
    ) -> Result<(Vec<u8>, Vec<crate::provenance::OriginRef>), StoreFormatError> {
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
        let token_lists = (0..self.runtime_values.token_len())
            .map(|raw| {
                self.tokens(self.resolve_stored_token_list(TokenListId::new(raw)))
                    .iter()
                    .copied()
                    .map(|token| FormatToken::capture(self, token))
                    .collect()
            })
            .collect();
        let glue = (0..self.runtime_values.glue_len())
            .map(|raw| {
                let id = self.resolve_stored_glue(GlueId::new(raw));
                FormatGlue::capture(self.glue(id))
            })
            .collect();
        let font_mark = self.fonts.watermark();
        let fonts = (0..font_mark.len)
            .map(|raw| FormatFont::capture(&self.fonts, self.resolve_stored_font(FontId::new(raw))))
            .collect();
        let mut seen = std::collections::BTreeSet::new();
        let mut visiting = std::collections::BTreeSet::new();
        let mut payload_roots = std::collections::BTreeMap::new();
        let mut node_lists = Vec::new();
        let mut origins = Vec::new();
        capture_owned_node_list(
            self,
            root.clone(),
            &mut seen,
            &mut visiting,
            &mut payload_roots,
            &mut node_lists,
            Some(&mut origins),
        )?;
        let mut root = FormatListKey::capture(self, root.id(), &mut payload_roots);
        canonicalize_memo_node_list_keys(&mut node_lists, &mut root);
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
    ) -> Result<crate::node_arena::NodeListRef, StoreFormatError> {
        self.import_memo_node_list_with_origins(bytes, max_nodes, max_tokens, max_string_bytes, &[])
    }

    pub(crate) fn import_memo_node_list_with_origins(
        &mut self,
        bytes: &[u8],
        max_nodes: usize,
        max_tokens: usize,
        max_string_bytes: usize,
        origins: &[crate::provenance::OriginRef],
    ) -> Result<crate::node_arena::NodeListRef, StoreFormatError> {
        let bundle: MemoNodeBundle = bincode::deserialize(bytes)
            .map_err(|error| StoreFormatError::Codec(error.to_string()))?;
        validate_memo_node_bundle_limits(&bundle, max_nodes, max_tokens, max_string_bytes)?;
        validate_dense_memo_node_graph(&bundle)?;

        // Validate content references and semantic fingerprints without
        // touching the destination stores. The second materialization can
        // therefore publish only a graph whose complete detached closure has
        // already succeeded. Target-specific capacity failures remain under
        // the aggregate scoped rollback owned by `Universe`.
        let mut validator = Stores::new();
        drop(validator.materialize_memo_node_bundle(bundle.clone(), &[])?);
        self.materialize_memo_node_bundle(bundle, origins)
    }

    fn materialize_memo_node_bundle(
        &mut self,
        bundle: MemoNodeBundle,
        origins: &[crate::provenance::OriginRef],
    ) -> Result<crate::node_arena::NodeListRef, StoreFormatError> {
        let MemoNodeBundle {
            names,
            token_lists,
            glue,
            fonts,
            node_lists,
            root,
        } = bundle;

        let mut symbols = Vec::with_capacity(names.len());
        let mut symbol_ids = Vec::with_capacity(names.len());
        for name in names {
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

        let mut token_ids = Vec::with_capacity(token_lists.len());
        for tokens in token_lists {
            let tokens = tokens
                .into_iter()
                .map(|token| token.restore_mapped(&symbols))
                .collect::<Result<Vec<_>, _>>()?;
            token_ids.push(self.intern_token_list_ref_in_domain(&tokens, None));
        }
        let mut glue_ids = Vec::with_capacity(glue.len());
        for glue in glue {
            glue_ids.push(self.intern_glue_in_domain(glue.restore()?, None));
        }
        let mut font_ids = Vec::with_capacity(fonts.len());
        for (raw, font) in fonts.into_iter().enumerate() {
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
        let mut node_owners = std::collections::BTreeMap::new();
        let mut owners_by_id = std::collections::BTreeMap::new();
        let mut origins = origins.iter().cloned();
        for list in node_lists {
            let expected_semantic_id = list.semantic_id;
            let nodes = list
                .nodes
                .into_iter()
                .map(|node| node.restore_with_origins(&content_ids, &node_ids, &mut origins))
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .map(|node| {
                    node.map_lists(|child| {
                        owners_by_id
                            .get(&child)
                            .cloned()
                            .expect("memo child must precede its parent")
                    })
                })
                .collect::<Vec<_>>();
            let owner = self.freeze_node_list(&nodes);
            if owner.semantic_fingerprint() != expected_semantic_id {
                return Err(StoreFormatError::Invalid("memo node semantic identity"));
            }
            node_ids.insert(list.key, owner.id());
            owners_by_id.insert(owner.id(), owner.clone());
            node_owners.insert(list.key, owner);
        }
        node_owners
            .get(&root)
            .cloned()
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
            let mut payload_roots = std::collections::BTreeMap::new();
            for node in extra_nodes {
                let mut children = Vec::new();
                node.visit_node_lists(|child| children.push(child.clone()));
                for child in children {
                    capture_owned_node_list(
                        stores,
                        child,
                        &mut seen,
                        &mut visiting,
                        &mut payload_roots,
                        &mut mutable.node_lists,
                        None,
                    )?;
                }
            }
            mutable.node_lists.push(FormatNodeList {
                key: FormatListKey {
                    payload_root: None,
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
                            &mut payload_roots,
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
        let token_lists = (0..stores.runtime_values.token_len())
            .map(|raw| {
                stores
                    .tokens(stores.resolve_stored_token_list(TokenListId::new(raw)))
                    .iter()
                    .copied()
                    .map(|token| FormatToken::capture(stores, token))
                    .collect()
            })
            .collect();
        let macros = (0..stores.runtime_values.macro_len())
            .map(|raw| {
                let meaning = stores
                    .macro_definition(
                        stores
                            .runtime_values
                            .macro_id_at(raw)
                            .expect("captured macro slot should be live"),
                    )
                    .meaning();
                FormatMacro {
                    flags: meaning.flags().bits(),
                    parameter_text: meaning.parameter_text().raw(),
                    replacement_text: meaning.replacement_text().raw(),
                }
            })
            .collect();
        let glue = (0..stores.runtime_values.glue_len())
            .map(|raw| {
                let id = stores.resolve_stored_glue(GlueId::new(raw));
                FormatGlue::capture(stores.glue(id))
            })
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
            .filter(|&&(cell, _)| cell.bank() == crate::cell::BankTag::Box)
            .map(|&(cell, word)| {
                let id = NodeListId::decode_box_word(word)
                    .expect("non-default box format entry should contain a list");
                let owner = stores
                    .box_reg_ref(cell.index() as u16)
                    .expect("non-default box format entry should own its list");
                (id, owner)
            })
            .collect();
        let mut seen = std::collections::BTreeSet::new();
        let mut visiting = std::collections::BTreeSet::new();
        let mut payload_roots = std::collections::BTreeMap::new();
        let mut node_lists = Vec::new();
        for (_, root) in roots {
            capture_owned_node_list(
                stores,
                root,
                &mut seen,
                &mut visiting,
                &mut payload_roots,
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
                    FormatEnvValue::Box(FormatListKey::capture(stores, id, &mut payload_roots))
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
    env: Vec<FormatEnvEntry>,
    node_lists: frozen_node::DecodedFrozenNodes,
    frozen: frozen_core::DecodedFrozenCore,
    non_node: frozen_non_node::DecodedFrozenNonNode,
) -> Result<Stores, StoreFormatError> {
    let font_count = non_node.font_rows.len();
    let glue_count = frozen.glue.len();
    let token_list_count = frozen.tokens.len();
    let mut stores = Stores::new();
    stores.interner = frozen.interner;
    for (raw, value) in frozen.tokens.iter().enumerate() {
        stores
            .runtime_values
            .install_frozen_token_list(
                raw as u32,
                crate::hot_core::arena::store::registry::RuntimeTokenValueInput {
                    semantic_id: value.semantic_id,
                    tokens: &value.tokens,
                    provenance: &[],
                },
            )
            .map_err(|_| StoreFormatError::Invalid("frozen token registry install"))?;
    }
    stores
        .env
        .install_empty_token_root(crate::token_store::TokenListRef::new(TokenListId::EMPTY));
    for (raw, value) in frozen.macros.iter().enumerate() {
        let parameter_text = stores
            .runtime_values
            .token_id_at(value.meaning.parameter_text().raw())
            .ok_or(StoreFormatError::Invalid("frozen macro parameter slot"))?;
        let replacement_text = stores
            .runtime_values
            .token_id_at(value.meaning.replacement_text().raw())
            .ok_or(StoreFormatError::Invalid("frozen macro replacement slot"))?;
        stores
            .runtime_values
            .install_frozen_macro(
                raw as u32,
                crate::hot_core::arena::store::registry::RuntimeMacroValueInput {
                    flags: value.meaning.flags(),
                    parameter_pattern: value.pattern,
                    parameter_text,
                    replacement_text,
                    definition_origin: crate::token::OriginId::UNKNOWN,
                    parameter_origins: &[],
                    replacement_origins: &[],
                    observation_width: value.observation_width,
                },
            )
            .map_err(|_| StoreFormatError::Invalid("frozen macro registry install"))?;
    }
    for (raw, spec) in frozen.glue.iter().copied().enumerate() {
        stores
            .runtime_values
            .install_frozen_glue(raw as u32, spec)
            .map_err(|_| StoreFormatError::Invalid("frozen glue registry install"))?;
    }
    stores
        .runtime_values
        .publish_into(&mut stores.runtime_value_roots)
        .map_err(|_| StoreFormatError::Invalid("frozen runtime registry publication"))?;
    stores.fonts = non_node.fonts;
    stores.code_tables = non_node.code_tables;
    stores.hyphenation = non_node.hyphenation.into();
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

    if node_lists.semantic_ids.len() != node_lists.lists.len() {
        return Err(StoreFormatError::Invalid("frozen node identity count"));
    }
    let root = crate::node_arena::allocate_node_payload_root().ok_or(StoreFormatError::Invalid(
        "frozen node root identity space exhausted",
    ))?;
    let mut next_start = 0_u32;
    let node_ids: std::collections::BTreeMap<_, _> = node_lists
        .lists
        .iter()
        .map(|list| {
            let len = u32::try_from(list.nodes.len())
                .map_err(|_| StoreFormatError::Invalid("frozen node list exceeds u32"))?;
            let id = NodeListId::new_owned(root, next_start, len);
            next_start = next_start
                .checked_add(len)
                .ok_or(StoreFormatError::Invalid("frozen node arena exceeds u32"))?;
            Ok((list.key, id))
        })
        .collect::<Result<_, StoreFormatError>>()?;
    let mut storage = crate::node_arena::NodeStorage::default();
    let mut runtime_value_roots = stores.runtime_values.empty_root_set();
    let mut verified_semantics = std::collections::BTreeMap::new();
    let mut spans = Vec::with_capacity(node_lists.lists.len());
    for (list, expected_id) in node_lists.lists.into_iter().zip(node_lists.semantic_ids) {
        let id = node_ids
            .get(&list.key)
            .copied()
            .ok_or(StoreFormatError::Invalid("missing frozen node list"))?;
        let nodes = list
            .nodes
            .into_iter()
            .map(|node| node.restore(&content_ids, &node_ids))
            .collect::<Result<Vec<_>, _>>()?;
        let (start, len) = storage.append_compact_nodes(&nodes);
        stores.retain_runtime_value_roots_in_frozen_nodes(
            &mut runtime_value_roots,
            storage.view(start, len),
        );
        if start != id.start() || len != id.len() {
            return Err(StoreFormatError::Invalid("frozen node span metadata"));
        }
        let invalid_child = std::cell::Cell::new(false);
        let semantic_id =
            stores.compute_frozen_node_semantic_id(storage.view(start, len), |child| {
                if child.is_empty() {
                    crate::node_arena::NodeSemanticId::empty()
                } else if let Some(semantic_id) = verified_semantics.get(&child).copied() {
                    semantic_id
                } else {
                    invalid_child.set(true);
                    crate::node_arena::NodeSemanticId::empty()
                }
            });
        if invalid_child.get() {
            return Err(StoreFormatError::Invalid(
                "node child does not precede dependent list",
            ));
        }
        if semantic_id.value() != expected_id {
            return Err(StoreFormatError::Invalid("frozen node semantic identity"));
        }
        if len != 0 {
            spans.push(crate::node_arena::OwnedSemanticSpan {
                start,
                len,
                semantic_id,
            });
        }
        verified_semantics.insert(id, semantic_id);
    }
    let payload = std::sync::Arc::new(crate::node_arena::NodeListPayload::new(
        root,
        storage,
        spans,
        Vec::new(),
        Some(runtime_value_roots),
    ));
    #[cfg(feature = "profiling")]
    crate::measurement::record_format_restore_work(1, 0, 0);
    let mut base = Vec::with_capacity(env.len());
    for entry in env {
        let dto_cell = crate::cell::CellId::from_raw(entry.cell)
            .ok_or(StoreFormatError::Invalid("unknown environment cell"))?;
        let cell = crate::cell::CellId::new(dto_cell.bank(), dto_cell.index());
        let (word, box_root) = match (cell.bank(), entry.value) {
            (crate::cell::BankTag::Box, FormatEnvValue::Box(key)) => {
                let id = node_ids
                    .get(&key)
                    .copied()
                    .ok_or(StoreFormatError::Invalid("missing box node list"))?;
                let root = NodeListRef::from_shared(id, std::sync::Arc::clone(&payload));
                (NodeListId::encode_box_word(Some(root.id())), Some(root))
            }
            (crate::cell::BankTag::Box, FormatEnvValue::Raw(_)) => {
                return Err(StoreFormatError::Invalid("raw box environment value"));
            }
            (crate::cell::BankTag::CurrentFont, FormatEnvValue::Raw(word)) => {
                (restore_current_font_word(&stores, word)?, None)
            }
            (_, FormatEnvValue::Raw(word)) => (word, None),
            (_, FormatEnvValue::Box(_)) => {
                return Err(StoreFormatError::Invalid("box value in non-box bank"));
            }
        };
        let token_root =
            match cell.bank() {
                crate::cell::BankTag::Toks => Some(stores.token_list_ref(
                    stores.resolve_stored_token_list(TokenListId::new(word as u32)),
                )),
                crate::cell::BankTag::TokParam if word != 0 => Some(stores.token_list_ref(
                    stores.resolve_stored_token_list(TokenListId::new((word - 1) as u32)),
                )),
                crate::cell::BankTag::TokParam => None,
                _ => None,
            };
        let macro_root = if cell.bank() == crate::cell::BankTag::Meaning {
            match crate::meaning::Meaning::decode_stored(word) {
                crate::meaning::Meaning::Macro { definition, .. } => {
                    let definition = stores
                        .runtime_values
                        .macro_id_at(definition.raw())
                        .ok_or(StoreFormatError::Invalid("frozen environment macro owner"))?;
                    Some(stores.macro_definition_ref(definition))
                }
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
            Some(stores.glue_ref(stores.resolve_stored_glue(GlueId::new(word as u32))))
        } else {
            None
        };
        base.push(crate::env::FormatBaseCell {
            cell,
            word,
            token_root,
            macro_root,
            glue_root,
            box_root,
        });
    }
    stores.env.install_format_base(base);
    Ok(stores)
}

fn validate_loaded_references(
    env: &[FormatEnvEntry],
    name_count: usize,
    token_list_count: usize,
    macro_count: usize,
    glue_count: usize,
    font_count: usize,
) -> Result<(), StoreFormatError> {
    for entry in env {
        let cell = crate::cell::CellId::from_raw(entry.cell)
            .ok_or(StoreFormatError::Invalid("unknown environment cell"))?;
        if cell.is_global() {
            return Err(StoreFormatError::Invalid("global environment cell"));
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
                if cell.index() as usize >= name_count {
                    return Err(StoreFormatError::Invalid("meaning symbol is not live"));
                }
                match crate::meaning::Meaning::decode_stored(raw) {
                    crate::meaning::Meaning::Macro { definition, .. }
                        if definition.raw() as usize >= macro_count =>
                    {
                        return Err(StoreFormatError::Invalid("meaning macro is not live"));
                    }
                    crate::meaning::Meaning::Font(font) if font.raw() as usize >= font_count => {
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
                    && (raw > u64::from(u32::MAX) || raw as u32 as usize >= glue_count)
                {
                    return Err(StoreFormatError::Invalid("register glue is not live"));
                }
                if cell.bank() == BankTag::Toks
                    && (raw > u64::from(u32::MAX) || raw as u32 as usize >= token_list_count)
                {
                    return Err(StoreFormatError::Invalid("register token list is not live"));
                }
                if cell.bank() == BankTag::Box {
                    return Err(StoreFormatError::Invalid("raw box environment value"));
                }
            }
            BankTag::IntParam | BankTag::DimenParam | BankTag::GlueParam | BankTag::TokParam => {
                if cell.index() >= crate::env::banks::PARAMETER_COUNT as u32 {
                    return Err(StoreFormatError::Invalid("parameter index out of range"));
                }
                if cell.bank() == BankTag::GlueParam
                    && (raw > u64::from(u32::MAX) || raw as u32 as usize >= glue_count)
                {
                    return Err(StoreFormatError::Invalid("parameter glue is not live"));
                }
                if cell.bank() == BankTag::TokParam
                    && raw != 0
                    && (raw - 1 > u64::from(u32::MAX)
                        || (raw - 1) as u32 as usize >= token_list_count)
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

fn canonicalize_node_list_keys(node_lists: &mut [FormatNodeList], env: &mut [FormatEnvEntry]) {
    let keys = dense_node_list_keys(node_lists);
    remap_node_list_keys(node_lists, &keys);
    for entry in env {
        if let FormatEnvValue::Box(key) = &mut entry.value {
            *key = keys[key];
        }
    }
}

fn canonicalize_memo_node_list_keys(node_lists: &mut [FormatNodeList], root: &mut FormatListKey) {
    let keys = dense_node_list_keys(node_lists);
    *root = keys[root];
    remap_node_list_keys(node_lists, &keys);
}

fn dense_node_list_keys(
    node_lists: &[FormatNodeList],
) -> std::collections::BTreeMap<FormatListKey, FormatListKey> {
    node_lists
        .iter()
        .enumerate()
        .map(|(index, list)| {
            (
                list.key,
                FormatListKey {
                    payload_root: None,
                    start: u32::try_from(index).expect("format node-list count exceeds u32"),
                    len: u32::try_from(list.nodes.len()).expect("format node list exceeds u32"),
                },
            )
        })
        .collect()
}

fn remap_node_list_keys(
    node_lists: &mut [FormatNodeList],
    keys: &std::collections::BTreeMap<FormatListKey, FormatListKey>,
) {
    for list in node_lists {
        for node in &mut list.nodes {
            node.remap_list_keys(keys);
        }
        list.key = keys[&list.key];
    }
}

fn validate_memo_node_bundle_limits(
    bundle: &MemoNodeBundle,
    max_nodes: usize,
    max_tokens: usize,
    max_string_bytes: usize,
) -> Result<(), StoreFormatError> {
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
    Ok(())
}

fn validate_dense_memo_node_graph(bundle: &MemoNodeBundle) -> Result<(), StoreFormatError> {
    let Some(last) = bundle.node_lists.last() else {
        return Err(StoreFormatError::Invalid("memo node graph is empty"));
    };
    if bundle.root != last.key {
        return Err(StoreFormatError::Invalid("memo root is not canonical"));
    }
    for (index, list) in bundle.node_lists.iter().enumerate() {
        let expected = FormatListKey {
            payload_root: None,
            start: u32::try_from(index)
                .map_err(|_| StoreFormatError::Invalid("memo node-list count exceeds u32"))?,
            len: u32::try_from(list.nodes.len())
                .map_err(|_| StoreFormatError::Invalid("memo node list exceeds u32"))?,
        };
        if list.key != expected {
            return Err(StoreFormatError::Invalid("noncanonical memo node-list key"));
        }
        for child in list.nodes.iter().flat_map(|node| {
            node.semantic_children()
                .into_iter()
                .flatten()
                .chain(node.diagnostic_children())
        }) {
            let child_index = child.start as usize;
            if child.payload_root.is_some()
                || child_index >= index
                || bundle
                    .node_lists
                    .get(child_index)
                    .is_none_or(|dependency| dependency.key != child)
            {
                return Err(StoreFormatError::Invalid(
                    "memo node dependency is not canonical",
                ));
            }
        }
    }
    Ok(())
}

impl FormatListKey {
    fn capture(
        _stores: &Stores,
        mut id: NodeListId,
        payload_roots: &mut std::collections::BTreeMap<crate::ids::NodePayloadId, u32>,
    ) -> Self {
        // Zero-length child coordinates are semantically the one canonical
        // empty list even when compact copying projected them into the
        // enclosing payload. Detached schemas must not retain that private
        // payload coordinate.
        if id.is_empty() {
            id = crate::node_arena::NodeListRef::empty().id();
        }
        let (start, len) = (id.start(), id.len());
        Self {
            payload_root: match id.arena() {
                crate::ids::ArenaRef::Epoch => None,
                crate::ids::ArenaRef::Owned(root) => Some(match payload_roots.get(&root) {
                    Some(&detached) => detached,
                    None => {
                        let detached = u32::try_from(payload_roots.len())
                            .expect("format payload roots exceed u32");
                        payload_roots.insert(root, detached);
                        detached
                    }
                }),
            },
            start,
            len,
        }
    }
}

fn capture_owned_node_list(
    stores: &Stores,
    root: crate::node_arena::NodeListRef,
    seen: &mut std::collections::BTreeSet<NodeListId>,
    visiting: &mut std::collections::BTreeSet<NodeListId>,
    payload_roots: &mut std::collections::BTreeMap<crate::ids::NodePayloadId, u32>,
    out: &mut Vec<FormatNodeList>,
    mut origins: Option<&mut Vec<crate::provenance::OriginRef>>,
) -> Result<(), StoreFormatError> {
    enum Visit {
        Enter(crate::node_arena::NodeListRef),
        Exit(crate::node_arena::NodeListRef),
    }

    let mut stack = vec![Visit::Enter(root)];
    while let Some(visit) = stack.pop() {
        match visit {
            Visit::Enter(owner) => {
                let id = owner.id();
                if seen.contains(&id) {
                    continue;
                }
                if !visiting.insert(id) {
                    return Err(StoreFormatError::Invalid("cyclic owned node-list graph"));
                }
                stack.push(Visit::Exit(owner.clone()));
                for node in owner.nodes().iter().rev() {
                    for child in node.physical_children().rev() {
                        let child = owner.resolve(child).ok_or(StoreFormatError::Invalid(
                            "owned node child is outside its payload",
                        ))?;
                        stack.push(Visit::Enter(child));
                    }
                }
            }
            Visit::Exit(owner) => {
                let id = owner.id();
                visiting.remove(&id);
                if !seen.insert(id) {
                    continue;
                }
                let nodes = owner
                    .nodes()
                    .iter()
                    .map(|node| match origins.as_deref_mut() {
                        Some(origins) => {
                            FormatNode::capture_with_origins(stores, node, payload_roots, origins)
                        }
                        None => FormatNode::capture(stores, node, payload_roots),
                    })
                    .collect();
                out.push(FormatNodeList {
                    key: FormatListKey::capture(stores, id, payload_roots),
                    semantic_id: owner.semantic_id().value(),
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
            // A detached cold builder may still contain a token whose
            // timeline-local symbol was rolled back. The reachable closure
            // below discards such a value; use an invalid sentinel so an
            // actually reachable stale reference is rejected rather than
            // panicking while unreachable cold content is projected.
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
