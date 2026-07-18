use super::exact_collection::CanonicalCollectionIdentity;
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
std::thread_local! {
    static TRANSITIONAL_FORMAT_WORK: std::cell::Cell<TestingFormatLoadWork> =
        const { std::cell::Cell::new(TestingFormatLoadWork::ZERO) };
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TestingFormatLoadWork {
    pub(crate) graph_key_remaps: usize,
    pub(crate) semantic_reseals: usize,
    pub(crate) assignment_replays: usize,
}

#[cfg(test)]
impl TestingFormatLoadWork {
    const ZERO: Self = Self {
        graph_key_remaps: 0,
        semantic_reseals: 0,
        assignment_replays: 0,
    };
}

#[cfg(test)]
fn record_transitional_format_work(update: impl FnOnce(&mut TestingFormatLoadWork)) {
    TRANSITIONAL_FORMAT_WORK.with(|work| {
        let mut current = work.get();
        update(&mut current);
        work.set(current);
    });
}

#[cfg(test)]
pub(crate) fn testing_take_transitional_format_work() -> TestingFormatLoadWork {
    TRANSITIONAL_FORMAT_WORK.with(|work| work.replace(TestingFormatLoadWork::ZERO))
}

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
    entries[0].cell |= 1_u64 << 30;
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

#[derive(Clone, Debug, Deserialize, Serialize)]
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ImmutableStoreMarks {
    interner: InternerMark,
    tokens: TokenStoreMark,
    macros: MacroStoreMark,
    glue: GlueStoreMark,
    fonts: FontStoreMark,
}

#[derive(Debug, Default)]
pub(super) struct ExactIdentityCache {
    names: LineageIdentityCache<InternerMark>,
    tokens: LineageIdentityCache<TokenStoreMark>,
    macros: LineageIdentityCache<MacroStoreMark>,
    glue: LineageIdentityCache<GlueStoreMark>,
    fonts: LineageIdentityCache<FontStoreMark>,
    #[cfg(test)]
    immutable_encodes: usize,
    #[cfg(test)]
    immutable_leaves: usize,
}

const EXACT_IDENTITY_CACHE_BRANCHES: usize = 4;

#[derive(Debug)]
struct LineageIdentityCache<M> {
    branches: Vec<(M, AppendOnlyIdentityCache)>,
}

impl<M> Default for LineageIdentityCache<M> {
    fn default() -> Self {
        Self {
            branches: Vec::new(),
        }
    }
}

#[derive(Debug, Default)]
struct AppendOnlyIdentityCache {
    identity: CanonicalCollectionIdentity,
    logical_len: usize,
}

impl AppendOnlyIdentityCache {
    fn update(
        &mut self,
        len: usize,
        can_extend: bool,
        mut leaf: impl FnMut(usize) -> Result<u64, StoreFormatError>,
    ) -> Result<(), StoreFormatError> {
        if !can_extend || self.logical_len > len {
            self.identity = CanonicalCollectionIdentity::default();
            self.logical_len = 0;
        }
        self.identity.reserve(len.saturating_sub(self.logical_len));
        for raw in self.logical_len..len {
            let identity = leaf(raw)?;
            self.identity.insert(identity);
        }
        self.logical_len = len;
        Ok(())
    }

    fn identity(&mut self) -> u64 {
        self.identity.identity()
    }
}

impl<M: Copy + Eq> LineageIdentityCache<M> {
    #[cfg(any(test, feature = "profiling-stats"))]
    fn contains(&self, mark: M) -> bool {
        self.branches
            .iter()
            .any(|(cached_mark, _)| *cached_mark == mark)
    }

    fn update(
        &mut self,
        mark: M,
        len: usize,
        retains: impl Fn(M) -> bool,
        leaf: impl FnMut(usize) -> Result<u64, StoreFormatError>,
    ) -> Result<u64, StoreFormatError> {
        if let Some(index) = self
            .branches
            .iter()
            .position(|(cached_mark, _)| *cached_mark == mark)
        {
            return Ok(self.branches[index].1.identity());
        }
        let reusable = self
            .branches
            .iter()
            .enumerate()
            .filter(|(_, (cached_mark, _))| retains(*cached_mark))
            .max_by_key(|(_, (_, cache))| cache.logical_len)
            .map(|(index, _)| index);
        let mut cache = reusable
            .map(|index| self.branches.swap_remove(index).1)
            .unwrap_or_default();
        cache.update(len, reusable.is_some(), leaf)?;
        let identity = cache.identity();
        if reusable.is_some() || self.branches.len() < EXACT_IDENTITY_CACHE_BRANCHES {
            self.branches.push((mark, cache));
        } else {
            self.branches[0] = (mark, cache);
        }
        Ok(identity)
    }
}

fn exact_serialized_leaf<T: Serialize>(domain: &[u8], value: &T) -> Result<u64, StoreFormatError> {
    let encoded =
        bincode::serialize(value).map_err(|error| StoreFormatError::Codec(error.to_string()))?;
    let mut framed = Vec::with_capacity(domain.len() + encoded.len());
    framed.extend_from_slice(domain);
    framed.extend_from_slice(&encoded);
    Ok(crate::state_hash::exact_identity_bytes(domain, &framed))
}

fn exact_name_leaf(stores: &Stores, raw: usize) -> Result<u64, StoreFormatError> {
    let symbol = stores
        .interner
        .symbol_at_slot(raw as u32)
        .expect("exact name slot should be live");
    exact_serialized_leaf(
        b"umber-exact-name-v1",
        &FormatName {
            active: stores.interner.kind(symbol) == ControlSequenceKind::ActiveCharacter,
            text: stores.interner.resolve(symbol).to_owned(),
        },
    )
}

fn exact_token_leaf(stores: &Stores, raw: usize) -> Result<u64, StoreFormatError> {
    let id = stores.resolve_stored_token_list(TokenListId::new(raw as u32));
    let mut framed = Vec::new();
    framed.extend_from_slice(b"umber-exact-token-list-v1");
    framed.extend_from_slice(&(stores.tokens.get(id).len() as u64).to_le_bytes());
    for &token in stores.tokens.get(id) {
        match token {
            Token::Char { ch, cat } => {
                framed.push(0);
                framed.extend_from_slice(&(ch as u32).to_le_bytes());
                framed.push(cat as u8);
            }
            Token::Cs(symbol) => {
                framed.push(1);
                let symbol = stores.resolve_stored_symbol(symbol);
                framed.extend_from_slice(
                    &exact_name_leaf(stores, symbol.raw() as usize)?.to_le_bytes(),
                );
            }
            Token::Param(slot) => framed.extend_from_slice(&[2, slot]),
            Token::Frozen(frozen) => {
                framed.push(3);
                framed.extend_from_slice(&frozen.raw().to_le_bytes());
            }
        }
    }
    Ok(crate::state_hash::exact_identity_bytes(
        b"umber-exact-token-list-v2",
        &framed,
    ))
}

fn exact_macro_leaf(stores: &Stores, raw: usize) -> Result<u64, StoreFormatError> {
    let meaning = stores.macros.get(
        stores
            .macros
            .resolve_stored(MacroDefinitionId::new(raw as u32))
            .expect("exact macro slot should be live"),
    );
    let parameter = stores.resolve_stored_token_list(meaning.parameter_text());
    let replacement = stores.resolve_stored_token_list(meaning.replacement_text());
    let mut framed = Vec::with_capacity(96);
    framed.extend_from_slice(b"umber-exact-macro-v2");
    framed.push(meaning.flags().bits());
    framed.extend_from_slice(&exact_token_leaf(stores, parameter.raw() as usize)?.to_le_bytes());
    framed.extend_from_slice(&exact_token_leaf(stores, replacement.raw() as usize)?.to_le_bytes());
    Ok(crate::state_hash::exact_identity_bytes(
        b"umber-exact-macro-v3",
        &framed,
    ))
}

fn exact_glue_leaf(stores: &Stores, raw: usize) -> Result<u64, StoreFormatError> {
    exact_serialized_leaf(
        b"umber-exact-glue-v1",
        &FormatGlue::capture(
            stores
                .glue
                .get(stores.resolve_stored_glue(GlueId::new(raw as u32))),
        ),
    )
}

fn exact_font_leaf(stores: &Stores, raw: usize) -> Result<u64, StoreFormatError> {
    let id = stores.resolve_stored_font(FontId::new(raw as u32));
    let base = stores.fonts.immutable_exact_identity(id);
    let identifier = stores.fonts.identifier(id).map(|symbol| {
        let symbol = stores.resolve_stored_symbol(symbol.symbol());
        exact_name_leaf(stores, symbol.raw() as usize)
    });
    let identifier = identifier.transpose()?;
    let expansion = bincode::serialize(&stores.fonts.expansion(id))
        .map_err(|error| StoreFormatError::Codec(error.to_string()))?;
    let mut framed = Vec::with_capacity(96 + expansion.len());
    framed.extend_from_slice(b"umber-exact-font-v2");
    framed.extend_from_slice(&base.bytes());
    match identifier {
        Some(identifier) => {
            framed.push(1);
            framed.extend_from_slice(&identifier.to_le_bytes());
        }
        None => framed.push(0),
    }
    framed.extend_from_slice(&expansion);
    Ok(crate::state_hash::exact_identity_bytes(
        b"umber-exact-font-v3",
        &framed,
    ))
}

fn compose_immutable_store_root(
    names: u64,
    tokens: u64,
    macros: u64,
    glue: u64,
    fonts: u64,
) -> u64 {
    let mut framed = Vec::with_capacity(192);
    framed.extend_from_slice(b"umber-exact-immutable-store-v1");
    for identity in [names, tokens, macros, glue, fonts] {
        framed.extend_from_slice(&identity.to_le_bytes());
    }
    crate::state_hash::exact_identity_bytes(b"umber-exact-immutable-store-v2", &framed)
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
    #[cfg(test)]
    pub(crate) fn encode_format(&self) -> Result<Vec<u8>, StoreFormatError> {
        if self.env.group_depth() != 0 {
            return Err(StoreFormatError::OpenGroups(self.env.group_depth()));
        }
        // Survivor pins are allocation-lifetime bookkeeping, not TeX state.
        // A format captures the reachable box graph below and deliberately
        // drops transient mode/page material, just as TeX's `store_fmt_file`
        // does not serialize the current nest or contribution list.
        let format = StoreFormat::capture(self)?;
        bincode::serialize(&format).map_err(|error| StoreFormatError::Codec(error.to_string()))
    }

    /// Canonical semantic store root for checkpoint verification. Survivor
    /// pins are retention metadata, so unlike a restorable format dump they
    /// neither prevent nor participate in this identity.
    pub(crate) fn semantic_identity(&mut self) -> Result<u64, StoreFormatError> {
        if self.env.group_depth() != 0 {
            return Err(StoreFormatError::OpenGroups(self.env.group_depth()));
        }
        let current_marks = ImmutableStoreMarks::capture(self);
        let immutable = {
            let mut cache = self
                .exact_identity_cache
                .lock()
                .expect("exact store identity cache is not poisoned");
            #[cfg(any(test, feature = "profiling-stats"))]
            let root_hits = [
                cache.names.contains(current_marks.interner),
                cache.tokens.contains(current_marks.tokens),
                cache.macros.contains(current_marks.macros),
                cache.glue.contains(current_marks.glue),
                cache.fonts.contains(current_marks.fonts),
            ]
            .into_iter()
            .filter(|hit| *hit)
            .count();
            let mut leaves = 0;
            let names = cache.names.update(
                current_marks.interner,
                self.interner.len(),
                |mark| self.interner.retains_mark(mark),
                |raw| {
                    leaves += 1;
                    exact_name_leaf(self, raw)
                },
            )?;
            let tokens = cache.tokens.update(
                current_marks.tokens,
                current_marks.tokens.spans as usize,
                |mark| self.tokens.retains_mark(mark),
                |raw| {
                    leaves += 1;
                    exact_token_leaf(self, raw)
                },
            )?;
            let macros = cache.macros.update(
                current_marks.macros,
                current_marks.macros.definitions as usize,
                |mark| self.macros.retains_mark(mark),
                |raw| {
                    leaves += 1;
                    exact_macro_leaf(self, raw)
                },
            )?;
            let glue = cache.glue.update(
                current_marks.glue,
                current_marks.glue.specs as usize,
                |mark| self.glue.retains_mark(mark),
                |raw| {
                    leaves += 1;
                    exact_glue_leaf(self, raw)
                },
            )?;
            let fonts = cache.fonts.update(
                current_marks.fonts,
                current_marks.fonts.len as usize,
                |mark| {
                    mark.identifier_writes_len == current_marks.fonts.identifier_writes_len
                        && mark.expansion_writes_len == current_marks.fonts.expansion_writes_len
                        && self.fonts.retains_mark(mark)
                },
                |raw| {
                    leaves += 1;
                    exact_font_leaf(self, raw)
                },
            )?;
            #[cfg(test)]
            {
                cache.immutable_encodes += usize::from(root_hits != 5);
                cache.immutable_leaves += leaves;
            }
            #[cfg(feature = "profiling-stats")]
            crate::measurement::record_exact_root_cache(
                root_hits as u64,
                (5 - root_hits) as u64,
                leaves,
            );
            compose_immutable_store_root(names, tokens, macros, glue, fonts)
        };
        let mutable = self.exact_mutable_identity();
        let mut composed = Vec::with_capacity(96);
        composed.extend_from_slice(b"umber-exact-store-v1");
        composed.extend_from_slice(&immutable.to_le_bytes());
        composed.extend_from_slice(&mutable.to_le_bytes());
        Ok(crate::state_hash::exact_identity_bytes(
            b"umber-exact-store-v2",
            &composed,
        ))
    }

    #[cfg(test)]
    pub(crate) fn testing_exact_immutable_encodes(&self) -> usize {
        self.exact_identity_cache
            .lock()
            .expect("exact store identity cache is not poisoned")
            .immutable_encodes
    }

    #[cfg(test)]
    pub(crate) fn testing_exact_immutable_leaves(&self) -> usize {
        self.exact_identity_cache
            .lock()
            .expect("exact store identity cache is not poisoned")
            .immutable_leaves
    }

    #[cfg(test)]
    pub(crate) fn decode_format(bytes: &[u8]) -> Result<Self, StoreFormatError> {
        let format: StoreFormat = bincode::deserialize(bytes)
            .map_err(|error| StoreFormatError::Codec(error.to_string()))?;
        format.restore()
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
                    text: self.interner.resolve(symbol).to_owned(),
                }
            })
            .collect();
        let token_mark = self.tokens.watermark();
        let token_lists = (0..token_mark.spans)
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
            .map(|raw| {
                FormatGlue::capture(self.glue.get(self.resolve_stored_glue(GlueId::new(raw))))
            })
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
            token_ids.push(self.intern_token_list(&tokens));
        }
        let mut glue_ids = Vec::with_capacity(bundle.glue.len());
        for glue in bundle.glue {
            glue_ids.push(self.intern_glue(glue.restore()?));
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
        let immutable = ImmutableStoreIdentity::capture(stores);
        let mutable = MutableStoreIdentity::capture(stores)?;
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
        if let Some(empty) = live_tokens.first_mut() {
            *empty = true;
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
                BankTag::Toks | BankTag::TokParam => {
                    let raw = u32::try_from(raw)
                        .map_err(|_| StoreFormatError::Invalid("environment token list"))?;
                    mark_reachable(&mut live_tokens, raw, "environment token list")?;
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
                if invalid {
                    return Err(StoreFormatError::Invalid("node token-list reference"));
                }
            }
        }

        let macro_map = dense_reachable_map(&live_macros)?;
        let token_map = dense_reachable_map(&live_tokens)?;
        let mut live_names = vec![false; self.names.len()];
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
                BankTag::Toks | BankTag::TokParam => {
                    let old = u32::try_from(*raw)
                        .map_err(|_| StoreFormatError::Invalid("environment token list"))?;
                    *raw = u64::from(remapped(&token_map, old, "environment token list")?);
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
                if invalid {
                    return Err(StoreFormatError::Invalid("node token-list reference"));
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

impl ImmutableStoreMarks {
    fn capture(stores: &Stores) -> Self {
        Self {
            interner: stores.interner.watermark(),
            tokens: stores.tokens.watermark(),
            macros: stores.macros.watermark(),
            glue: stores.glue.watermark(),
            fonts: stores.fonts.watermark(),
        }
    }
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
                    text: stores.interner.resolve(symbol).to_owned(),
                }
            })
            .collect();
        let token_mark = stores.tokens.watermark();
        let token_lists = (0..token_mark.spans)
            .map(|raw| {
                stores
                    .tokens
                    .get(stores.resolve_stored_token_list(TokenListId::new(raw)))
                    .iter()
                    .copied()
                    .map(|token| FormatToken::capture(stores, token))
                    .collect()
            })
            .collect();
        let macro_mark = stores.macros.watermark();
        let macros = (0..macro_mark.definitions)
            .map(|raw| {
                let meaning = stores.macros.get(
                    stores
                        .macros
                        .resolve_stored(MacroDefinitionId::new(raw))
                        .expect("captured macro slot should be live"),
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
            .map(|raw| {
                FormatGlue::capture(
                    stores
                        .glue
                        .get(stores.resolve_stored_glue(GlueId::new(raw))),
                )
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
            env_words.push(capture_env_word(stores, cell, word));
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

impl StoreFormat {
    #[cfg(test)]
    fn restore(self) -> Result<Stores, StoreFormatError> {
        self.validate_references()?;
        self.validate_font_state()?;
        self.restore_with_core(None, None)
    }

    #[cfg(test)]
    fn restore_with_core(
        self,
        frozen: Option<frozen_core::DecodedFrozenCore>,
        non_node: Option<frozen_non_node::DecodedFrozenNonNode>,
    ) -> Result<Stores, StoreFormatError> {
        let mut stores = Stores::new();
        let has_frozen_core = frozen.is_some();
        if let Some(frozen) = frozen {
            stores.interner = frozen.interner;
            stores.tokens = frozen.tokens;
            stores.macros = frozen.macros;
            stores.glue = frozen.glue;
        }
        let symbol_ids = if has_frozen_core {
            (0..self.names.len())
                .map(|raw| {
                    stores
                        .interner
                        .symbol_at_slot(raw as u32)
                        .map(|symbol| symbol.symbol())
                        .ok_or(StoreFormatError::Invalid("frozen symbol mapping"))
                })
                .collect::<Result<Vec<_>, _>>()?
        } else {
            let mut symbol_ids = Vec::with_capacity(self.names.len());
            for (raw, name) in self.names.into_iter().enumerate() {
                let symbol = if name.active {
                    let mut chars = name.text.chars();
                    let ch = chars
                        .next()
                        .ok_or(StoreFormatError::Invalid("empty active name"))?;
                    if chars.next().is_some() {
                        return Err(StoreFormatError::Invalid("multi-character active name"));
                    }
                    stores.interner.intern_active(ch)
                } else {
                    stores.interner.intern(&name.text)
                }
                .map_err(|_| StoreFormatError::Invalid("symbol capacity"))?;
                if symbol.raw() as usize != raw {
                    return Err(StoreFormatError::Invalid("non-canonical symbol order"));
                }
                symbol_ids.push(symbol.symbol());
            }
            symbol_ids
        };
        let token_ids = if has_frozen_core {
            (0..self.token_lists.len())
                .map(|raw| stores.resolve_stored_token_list(TokenListId::new(raw as u32)))
                .collect::<Vec<_>>()
        } else {
            let mut token_ids = vec![TokenListId::EMPTY];
            for (raw, tokens) in self.token_lists.into_iter().enumerate().skip(1) {
                let tokens = tokens
                    .into_iter()
                    .map(|token| token.restore_mapped(&symbol_ids))
                    .collect::<Result<Vec<_>, _>>()?;
                let id = stores.intern_token_list(&tokens);
                if id.raw() as usize != raw {
                    return Err(StoreFormatError::Invalid("non-canonical token-list order"));
                }
                token_ids.push(id);
            }
            token_ids
        };
        if !has_frozen_core {
            for (raw, definition) in self.macros.into_iter().enumerate() {
                let meaning = MacroMeaning::new(
                    crate::meaning::MeaningFlags::from_bits(definition.flags),
                    stores.resolve_stored_token_list(TokenListId::new(definition.parameter_text)),
                    stores.resolve_stored_token_list(TokenListId::new(definition.replacement_text)),
                );
                if stores.intern_macro(meaning).raw() as usize != raw {
                    return Err(StoreFormatError::Invalid("macro order"));
                }
            }
        }
        let glue_ids = if has_frozen_core {
            (0..self.glue.len())
                .map(|raw| stores.resolve_stored_glue(GlueId::new(raw as u32)))
                .collect::<Vec<_>>()
        } else {
            let mut glue_ids = vec![GlueId::ZERO];
            for (raw, glue) in self.glue.into_iter().enumerate().skip(1) {
                let id = stores.glue.intern(glue.restore()?);
                if id.raw() as usize != raw {
                    return Err(StoreFormatError::Invalid("non-canonical glue order"));
                }
                glue_ids.push(id);
            }
            glue_ids
        };
        let has_frozen_non_node = non_node.is_some();
        let font_ids = if let Some(non_node) = non_node {
            stores.fonts = non_node.fonts;
            stores.code_tables = non_node.code_tables;
            stores.hyphenation = non_node.hyphenation.into();
            stores.prepared_mag = non_node.prepared_mag;
            stores.last_loaded_font = non_node.last_loaded_font;
            (0..self.fonts.len())
                .map(|raw| stores.resolve_stored_font(FontId::new(raw as u32)))
                .collect::<Vec<_>>()
        } else {
            let mut font_ids = Vec::with_capacity(self.fonts.len());
            for (raw, font) in self.fonts.into_iter().enumerate() {
                let identifier = font.identifier;
                let expansion = font.expansion;
                let id = if raw == 0 {
                    NULL_FONT
                } else {
                    let id = stores.fonts.intern(font.restore()).map_err(|_| {
                        StoreFormatError::Invalid("font count exceeds bank capacity")
                    })?;
                    if id.raw() as usize != raw {
                        return Err(StoreFormatError::Invalid("non-canonical font order"));
                    }
                    id
                };
                if let Some(symbol) = identifier {
                    let symbol = symbol_ids
                        .get(symbol as usize)
                        .copied()
                        .and_then(|symbol| stores.interner.resolve_stored(symbol))
                        .ok_or(StoreFormatError::Invalid("font identifier symbol"))?;
                    stores.set_resolved_font_identifier(id, symbol);
                }
                if let Some(expansion) = expansion {
                    stores
                        .fonts
                        .set_expansion(id, expansion)
                        .map_err(|_| StoreFormatError::Invalid("font expansion configuration"))?;
                }
                font_ids.push(id);
            }
            font_ids
        };
        let content_ids = FormatContentIds {
            fonts: &font_ids,
            glue: &glue_ids,
            token_lists: &token_ids,
        };
        let mut node_ids = std::collections::BTreeMap::new();
        for list in self.node_lists {
            let nodes = list
                .nodes
                .into_iter()
                .map(|node| node.restore(&content_ids, &node_ids))
                .collect::<Result<Vec<_>, _>>()?;
            let semantic_id = stores.compute_and_seal_node_semantic_id(&nodes);
            record_transitional_format_work(|work| work.semantic_reseals += 1);
            let id = stores.nodes.append_with_semantic_id(&nodes, semantic_id);
            node_ids.insert(list.key, id);
        }
        if !has_frozen_non_node {
            for entry in self.code_tables {
                entry.restore(&mut stores.code_tables)?;
            }
            stores.hyphenation = self.hyphenation.into();
            stores.prepared_mag = self.prepared_mag;
            stores.last_loaded_font =
                stores.resolve_stored_font(FontId::new(self.last_loaded_font));
        }
        for entry in self.env {
            let dto_cell = crate::cell::CellId::from_raw(entry.cell)
                .ok_or(StoreFormatError::Invalid("unknown environment cell"))?;
            let bank = dto_cell.bank();
            let dto_index = dto_cell.index();
            let cell = crate::cell::CellId::new(bank, dto_index);
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
            record_transitional_format_work(|work| work.assignment_replays += 1);
            stores.env.restore_raw(cell, word);
        }
        stores.initialize_exact_env_identity();
        Ok(stores)
    }
}

/// Publishes the already decoded and cross-section-validated schema-10 bases.
///
/// This is deliberately separate from the test-only transitional DTO restore
/// above. The production loader installs one frozen node root and one immutable
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
        .map(|raw| stores.resolve_stored_glue(GlueId::new(raw as u32)))
        .collect::<Vec<_>>();
    let token_ids = (0..token_list_count)
        .map(|raw| stores.resolve_stored_token_list(TokenListId::new(raw as u32)))
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
        base.push(crate::env::FormatBaseCell { cell, word });
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
                        && (raw > u64::from(u32::MAX)
                            || raw as u32 as usize >= self.token_lists.len())
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
            #[cfg(test)]
            record_transitional_format_work(|work| work.graph_key_remaps += 1);
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
                    let children = node_child_ids(&node.to_owned());
                    for child in children.into_iter().rev() {
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
                        Some(origins) => FormatNode::capture_with_origins(
                            stores,
                            node.to_owned(),
                            survivor_roots,
                            origins,
                        ),
                        None => FormatNode::capture(stores, node.to_owned(), survivor_roots),
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

fn node_child_ids(node: &Node) -> Vec<NodeListId> {
    let mut out = Vec::new();
    match node {
        Node::HList(box_node) | Node::VList(box_node) => out.push(box_node.children),
        Node::Glue {
            leader:
                Some(
                    crate::node::LeaderPayload::HList(box_node)
                    | crate::node::LeaderPayload::VList(box_node),
                ),
            ..
        } => out.push(box_node.children),
        Node::Unset(unset) => out.push(unset.children),
        Node::Disc {
            pre, post, replace, ..
        } => out.extend([*pre, *post, *replace]),
        Node::Ins { content, .. } | Node::Adjust(content) => out.push(*content),
        Node::MathNoad(noad) => {
            math_field_child(&noad.nucleus, &mut out);
            math_field_child(&noad.subscript, &mut out);
            math_field_child(&noad.superscript, &mut out);
        }
        Node::FractionNoad(fraction) => {
            out.extend([fraction.numerator, fraction.denominator]);
        }
        Node::MathChoice(choice) => out.extend([
            choice.display,
            choice.text,
            choice.script,
            choice.script_script,
        ]),
        Node::MathList(list) => out.push(list.content),
        _ => {}
    }
    out
}

fn math_field_child(field: &crate::math::MathField, out: &mut Vec<NodeListId>) {
    if let crate::math::MathField::SubBox(id) | crate::math::MathField::SubMlist(id) = field {
        out.push(*id);
    }
}

impl FormatToken {
    fn capture(stores: &Stores, token: Token) -> Self {
        match token {
            Token::Char { ch, cat } => Self::Char { ch, cat: cat as u8 },
            Token::Cs(symbol) => Self::Cs(stores.resolve_stored_symbol(symbol).raw()),
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

impl FormatCodeTables {
    #[cfg(test)]
    fn restore(self, tables: &mut CodeTables) -> Result<(), StoreFormatError> {
        let ch = char::from_u32(self.code).ok_or(StoreFormatError::Invalid("codepoint"))?;
        tables.set_catcode(ch, catcode(self.catcode)?);
        tables.set_lccode(ch, self.lccode);
        tables.set_uccode(ch, self.uccode);
        tables.set_sfcode(ch, self.sfcode);
        tables.set_mathcode(ch, self.mathcode);
        tables.set_delcode(ch, self.delcode);
        Ok(())
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
