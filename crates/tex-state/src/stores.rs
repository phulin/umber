//! Internal aggregate state stores and atomic rollback machinery.
//!
//! `Stores` is the private composition owned by `Universe`. Public callers use
//! `Universe` for checkpointing and rollback so the whole timeline tuple is
//! restored atomically.

use crate::code_tables::{
    CodeTableGenerations, CodeTableRestoreRecord, CodeTables, CodeTablesSnapshot, DelCode, LcCode,
    MathCode, SfCode, UcCode,
};
use crate::env::banks::{DimenParam, GlueParam, IntParam, TokParam};
use crate::env::group::MutationReceipts;
use crate::env::{CellMutationReceipt, DirectJournalMark, Env, EnvSnapshot};
use crate::font::{
    CharMetrics, CharTag, ExtensibleRecipe, FontMetrics, FontMetricsValidationError,
    FontSourceIdentity, FontStore, FontStoreMark, LigKernChar, LigKernCommand, LigKernIter,
    LoadedFont, MissingCharacter, NULL_FONT, complete_font_hash_fragment,
};
use crate::font::{FontExpansion, FontExpansionConfigError, PdfFontCode};
use crate::state_hash::StateHashFragment;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

type GroupExitObservation = (
    Vec<crate::token::RootedTracedTokenWord>,
    crate::env::group::MutationReceipts,
    CodeTableGenerations,
    CodeTableGenerations,
    Vec<crate::env::group::RestoreRecord>,
    Vec<CodeTableRestoreRecord>,
);

fn pdf_font_code_bank(table: PdfFontCode) -> crate::cell::BankTag {
    use crate::cell::BankTag;
    match table {
        PdfFontCode::Lp => BankTag::PdfLpCode,
        PdfFontCode::Rp => BankTag::PdfRpCode,
        PdfFontCode::Ef => BankTag::PdfEfCode,
        PdfFontCode::Tag => BankTag::PdfTagCode,
        PdfFontCode::Knbs => BankTag::PdfKnbsCode,
        PdfFontCode::Stbs => BankTag::PdfStbsCode,
        PdfFontCode::Shbs => BankTag::PdfShbsCode,
        PdfFontCode::Knbc => BankTag::PdfKnbcCode,
        PdfFontCode::Knac => BankTag::PdfKnacCode,
    }
}
use crate::glue::{GlueSpec, GlueSpecRef, GlueStore, GlueStoreMark};
use crate::hyphenation::{ExceptionSpec, HyphenationTable, PatternSpec};
use crate::ids::{FontId, GlueId, MacroDefinitionId, NodeListId, OriginListId, TokenListId};
use crate::input::SourceId;
use crate::input::TracedTokenList;
use crate::interner::{
    ControlSequenceKind, Interner, InternerError, InternerMark, Symbol, SymbolId, SymbolReference,
};
use crate::macro_store::{
    MacroDefinitionProvenance, MacroDefinitionRef, MacroMeaning, MacroParameterPattern, MacroStore,
    MacroStoreMark,
};
use crate::math::MathFontSize;
use crate::meaning::{Meaning, MeaningFlags};
use crate::node::Node;
use crate::node_arena::{NodeListBuilder, NodeListRef, NodeListWeakIndex};
use crate::provenance::{
    ExpansionFrameRef, InsertedOrigin, InsertedOriginKind, MacroInvocationOrigin, OriginListRef,
    OriginRecord, OriginRef, ProvenanceStats, ProvenanceStore, ProvenanceStoreMark, SourceOrigin,
    SynthesizedOrigin, SynthesizedOriginKind, SyntheticOrigin, SyntheticOriginKind,
};
use crate::scaled::Scaled;
use crate::source_fragments::{FragmentStore, direct_fragment_span};
use crate::source_map::{
    GeneratedSource, SourceBacking, SourceDescriptor, SourceMap, SourceMapError, SourceMapMark,
    SourcePos, SourceRegion, SourceSpan,
};
use crate::token::{Catcode, OriginId, Token, TracedTokenWord};
use crate::token_store::{
    TokenListBuilder, TokenListRef, TokenSemanticId, TokenSemanticIdBuilder, TokenStore,
    TokenStoreMark,
};
use std::mem;
use std::sync::Arc;

mod exact_identity;
mod format;
mod handles;
mod low_memory;
mod node_semantic;
mod state_hash;

pub(crate) use format::{
    CODE_TABLES_SECTION, FONTS_SECTION, FROZEN_ENV_SECTION, FROZEN_NODES_SECTION,
    FrozenCoreSections, FrozenNodeSection, FrozenNonNodeSections, GLUE_SECTION,
    HYPHENATION_SECTION, MACROS_SECTION, NAMES_LOOKUP_SECTION, NAMES_SECTION, StoreFormatError,
    TOKEN_LISTS_SECTION,
};
#[cfg(test)]
pub(crate) use format::{
    TestingFontFormatCorruption, testing_corrupt_environment_box_reference,
    testing_corrupt_environment_global_cell, testing_corrupt_environment_macro_reference,
    testing_corrupt_font_format, testing_frozen_environment_shape,
};

pub use crate::env::group::{GroupFrame, GroupKind, GroupMismatch};
pub(crate) use state_hash::StoreStateHashCursor;

/// A rollback snapshot for all currently implemented state stores.
#[derive(Clone, Debug)]
pub(crate) struct StoreSnapshot {
    owner: SnapshotOwner,
    env_snapshot: EnvSnapshot,
    interner_mark: InternerMark,
    string_pool: StringPoolSnapshot,
    string_pool_recycled_mark: usize,
    token_mark: TokenStoreMark,
    provenance_mark: ProvenanceStoreMark,
    source_map_mark: SourceMapMark,
    macro_mark: MacroStoreMark,
    glue_mark: GlueStoreMark,
    font_mark: FontStoreMark,
    code_tables_snapshot: CodeTablesSnapshot,
    hyphenation: Arc<HyphenationTable>,
    prepared_mag: Option<i32>,
    last_loaded_font: FontId,
    exact_env_identity: exact_identity::ExactEnvSnapshot,
    exact_projection_cache: state_hash::StoreProjectionCache,
}

/// Fixed-size direct-operation cursor. This is not a rollback snapshot and
/// owns no aggregate state root.
#[derive(Debug)]
pub(crate) struct DirectStoreOperationMark {
    env: DirectJournalMark,
}

/// Fixed-size immutable-store suffix coordinates for a private operation.
/// These marks own no values and are used only when the allocation domain
/// rejects an unpublished suffix.
#[derive(Debug)]
pub(crate) struct StorePatchOperationMark {
    tokens: TokenStoreMark,
    macros: MacroStoreMark,
    glue: GlueStoreMark,
}

#[derive(Clone, Debug)]
struct PendingLowMemoryBreak {
    arena: low_memory::LowMemoryArena,
    owners: Vec<(crate::PureBreakMemoryOwner, low_memory::Allocation)>,
}

impl StoreSnapshot {
    #[must_use]
    pub(crate) const fn epoch(&self) -> crate::epoch::Epoch {
        self.env_snapshot.epoch()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SnapshotOwner {
    address: usize,
    nonce: u64,
}

#[derive(Debug)]
struct StoreOwner(Box<StoreOwnerToken>);

#[derive(Debug)]
struct StoreOwnerToken {
    nonce: u64,
}

impl StoreOwner {
    fn new() -> Self {
        Self(Box::new(StoreOwnerToken {
            nonce: random_owner_nonce(),
        }))
    }

    fn snapshot_owner(&self) -> SnapshotOwner {
        SnapshotOwner {
            address: self.0.as_ref() as *const StoreOwnerToken as usize,
            nonce: self.0.nonce,
        }
    }
}

fn random_owner_nonce() -> u64 {
    let state = ahash::RandomState::new();
    state.hash_one(0x7374_6f72_6573_u64)
}

/// Internal owner for rollback-coupled state stores.
#[derive(Debug)]
pub struct Stores {
    owner: StoreOwner,
    env: Env,
    interner: Interner,
    string_pool: StringPoolAccounting,
    string_pool_recycled_journal: Vec<Arc<str>>,
    tokens: TokenStore,
    provenance: ProvenanceStore,
    source_map: SourceMap,
    source_fragments: FragmentStore,
    macros: MacroStore,
    glue: GlueStore,
    fonts: FontStore,
    node_ref_index: NodeListWeakIndex,
    code_tables: CodeTables,
    hyphenation: Arc<HyphenationTable>,
    prepared_mag: Option<i32>,
    last_loaded_font: FontId,
    font_info_capacity: usize,
    semantic_hash_cache: state_hash::SemanticHashCache,
    exact_env_identity: exact_identity::ExactEnvIdentity,
    usage_high_water: EngineUsageStatistics,
    memory_low_extent: usize,
    memory_high_extent: usize,
    main_memory_profile: StringPoolProfile,
    low_memory_fragments: Vec<usize>,
    pending_low_memory_break: Option<PendingLowMemoryBreak>,
    transient_memory_base: Option<(u32, format::MainMemoryProjection)>,
    #[cfg(test)]
    transient_memory_base_projections: usize,
    #[cfg(test)]
    main_memory_root_traversals: std::sync::atomic::AtomicUsize,
}

/// TeX82-shaped projection of allocation use over Umber's typed stores.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EngineUsageStatistics {
    pub strings: usize,
    pub string_capacity: usize,
    pub string_characters: usize,
    pub string_character_capacity: usize,
    pub memory_words: usize,
    pub memory_word_capacity: usize,
    pub control_sequences: usize,
    pub font_info_words: usize,
    pub fonts: usize,
    pub hyphenation_exceptions: usize,
    pub hyphenation_exception_capacity: usize,
    pub input_stack: usize,
    pub nest_stack: usize,
    pub parameter_stack: usize,
    pub buffer_stack: usize,
    pub save_stack: usize,
}

/// Runtime-only TeX82 stack maxima supplied by their owning engine layers.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EngineStackUsage {
    pub input_stack: usize,
    pub nest_stack: usize,
    pub parameter_stack: usize,
    pub buffer_stack: usize,
    pub save_stack: usize,
}

/// Test-only census of one weak immutable-value pool.
///
/// Live objects and logical bytes are ownership authority. The remaining
/// fields describe bounded, non-owning lookup and slot metadata so long-run
/// tests can distinguish live-root growth from allocator retention.
#[cfg(any(test, feature = "testing"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TestingValuePoolCensus {
    pub live_objects: usize,
    pub logical_bytes: usize,
    pub slot_extent: usize,
    pub slot_capacity: usize,
    pub index_keys: usize,
    pub index_capacity: usize,
    pub max_bucket_capacity: usize,
    pub free_slots: usize,
}

#[cfg(any(test, feature = "testing"))]
impl TestingValuePoolCensus {
    fn new(live: (usize, usize), shape: (usize, usize, usize, usize, usize, usize)) -> Self {
        Self {
            live_objects: live.0,
            logical_bytes: live.1,
            slot_extent: shape.0,
            slot_capacity: shape.1,
            index_keys: shape.2,
            index_capacity: shape.3,
            max_bucket_capacity: shape.4,
            free_slots: shape.5,
        }
    }
}

/// Test-only logical-owner and bounded-metadata census for a live generation.
#[cfg(any(test, feature = "testing"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TestingOwnershipCensus {
    pub token_lists: TestingValuePoolCensus,
    pub macro_bodies: TestingValuePoolCensus,
    pub macro_definitions: TestingValuePoolCensus,
    pub glue_specs: TestingValuePoolCensus,
    pub node_weak_entries: usize,
    pub node_weak_capacity: usize,
    pub provenance_records: usize,
    pub provenance_lists: usize,
    pub provenance_entries: usize,
    pub provenance_retained_bytes: usize,
    pub source_regions: usize,
    pub source_bytes: usize,
    pub journal_entries: usize,
    pub journal_retained_bytes: usize,
}

/// Web2C TeX82's configured main-memory arena profile.
const TEX82_MEMORY_WORD_CAPACITY: usize = 250_000;
const TEX82_LOW_MEMORY_GROWTH: usize = 1_000;
const TEX82_STATIC_LOW_MEMORY_WORDS: usize = 21;
const TEX82_INITIAL_LOW_MEMORY_EXTENT: usize =
    TEX82_STATIC_LOW_MEMORY_WORDS + TEX82_LOW_MEMORY_GROWTH;
const TEX82_INITIAL_HIGH_MEMORY_EXTENT: usize = 20;

/// TeX82's string-pool counters and format-relative reporting profile.
///
/// The bytes themselves remain in their typed owners; this ledger models only
/// the `make_string` side effect shared by those owners. Control-sequence names
/// are deliberately one allocation class among several, not the pool owner.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StringPoolAccounting {
    profile_version: u8,
    strings: usize,
    characters: usize,
    init_str_ptr: usize,
    init_pool_ptr: usize,
    memory_low_extent: usize,
    memory_high_extent: usize,
    max_strings: usize,
    pool_size: usize,
    /// Strings available to Web2C's `search_string` recycling extension.
    #[serde(
        serialize_with = "serialize_recycled_strings",
        deserialize_with = "deserialize_recycled_strings"
    )]
    recycled: BTreeSet<Arc<str>>,
    /// INITEX's aggregate ledger predates the runtime recycling projection;
    /// restoration enables exact post-format `slow_make_string` behavior.
    #[serde(skip)]
    recycling_enabled: bool,
    /// Whether §38's single unfinished current-string byte is already present
    /// in the INITEX aggregate coordinate.
    #[serde(default)]
    unfinished_current_string_accounted: bool,
}

fn serialize_recycled_strings<S>(
    strings: &BTreeSet<Arc<str>>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    strings
        .iter()
        .map(AsRef::as_ref)
        .collect::<Vec<&str>>()
        .serialize(serializer)
}

fn deserialize_recycled_strings<'de, D>(deserializer: D) -> Result<BTreeSet<Arc<str>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Vec::<String>::deserialize(deserializer).map(|strings| {
        strings
            .into_iter()
            .map(Arc::<str>::from)
            .collect::<BTreeSet<_>>()
    })
}

/// Constant-size rollback coordinates for the canonical string-pool ledger.
///
/// Web2C tex.ch [29.517] keeps the retained strings in the append-only TeX
/// pool. The owning [`Stores`] journal removes only unique strings appended
/// after this mark when an aggregate operation rolls back.
#[derive(Clone, Copy, Debug)]
struct StringPoolSnapshot {
    profile_version: u8,
    strings: usize,
    characters: usize,
    init_str_ptr: usize,
    init_pool_ptr: usize,
    memory_low_extent: usize,
    memory_high_extent: usize,
    max_strings: usize,
    pool_size: usize,
    recycling_enabled: bool,
}

/// Engine-owned static string-pool vocabulary installed before INITEX input.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StringPoolProfile {
    Tex82,
    Etex26,
}

impl StringPoolProfile {
    const fn baseline(self) -> (usize, usize) {
        match self {
            // The typed primitive registry carries 33 more spelling bytes
            // than TeX82's §§47/50/226 static-pool-plus-primitive image. Its
            // profile origin compensates for that representation difference
            // so the completed engine-owned vocabulary lands on the WEB
            // `pool_ptr` coordinate rather than making host spellings usage.
            Self::Tex82 => (1_027, 106_808),
            // TeX82 §§47 and 50 load every multi-character WEB literal before
            // input. The merged e-TeX program adds literals that are not all
            // represented by its typed primitive names, while three repeated
            // spellings (including Web2C [54/SyncTeX]'s parameter) reuse
            // existing pool strings. The profile offset makes
            // the completed typed registry land exactly 119 strings and 1621
            // characters above the pinned TeX82 `init_prim` coordinate.
            Self::Etex26 => (1_079, 107_690),
        }
    }
}

impl Default for StringPoolAccounting {
    fn default() -> Self {
        Self {
            profile_version: 11,
            // TeX82's INITEX profile begins after `get_strings_started` has
            // installed the character strings and tex.pool vocabulary. These
            // are profile coordinates, not job usage or fixture totals.
            strings: 1_027,
            characters: 106_808,
            init_str_ptr: 1_027,
            init_pool_ptr: 106_808,
            memory_low_extent: TEX82_INITIAL_LOW_MEMORY_EXTENT,
            memory_high_extent: TEX82_INITIAL_HIGH_MEMORY_EXTENT,
            // Web2C's TeX82 profile used by the pinned canonical oracle.
            max_strings: 15_000,
            pool_size: 125_000,
            recycled: BTreeSet::new(),
            recycling_enabled: false,
            unfinished_current_string_accounted: false,
        }
    }
}

impl StringPoolAccounting {
    fn select_profile(&mut self, profile: StringPoolProfile) {
        let (strings, characters) = profile.baseline();
        let added_strings = strings.saturating_sub(self.init_str_ptr);
        let added_characters = characters.saturating_sub(self.init_pool_ptr);
        self.allocate(added_strings, added_characters);
        self.init_str_ptr = self.init_str_ptr.max(strings);
        self.init_pool_ptr = self.init_pool_ptr.max(characters);
    }

    fn allocate(&mut self, strings: usize, characters: usize) {
        self.strings = self.strings.saturating_add(strings);
        self.characters = self.characters.saturating_add(characters);
    }

    fn make_string(&mut self, value: &str) -> Option<Arc<str>> {
        self.allocate(1, value.len());
        self.remember_string(value)
    }

    fn slow_make_string(&mut self, value: &str) -> Option<Arc<str>> {
        let inserted = self.remember_string(value);
        if inserted.is_some() {
            self.allocate(1, value.len());
        }
        inserted
    }

    fn remember_string(&mut self, value: &str) -> Option<Arc<str>> {
        if self.recycled.contains(value) {
            return None;
        }
        let retained = Arc::<str>::from(value);
        self.recycled.insert(Arc::clone(&retained));
        Some(retained)
    }

    fn checkpoint(&self) -> StringPoolSnapshot {
        StringPoolSnapshot {
            profile_version: self.profile_version,
            strings: self.strings,
            characters: self.characters,
            init_str_ptr: self.init_str_ptr,
            init_pool_ptr: self.init_pool_ptr,
            memory_low_extent: self.memory_low_extent,
            memory_high_extent: self.memory_high_extent,
            max_strings: self.max_strings,
            pool_size: self.pool_size,
            recycling_enabled: self.recycling_enabled,
        }
    }

    fn rollback_to(&mut self, snapshot: StringPoolSnapshot) {
        self.profile_version = snapshot.profile_version;
        self.strings = snapshot.strings;
        self.characters = snapshot.characters;
        self.init_str_ptr = snapshot.init_str_ptr;
        self.init_pool_ptr = snapshot.init_pool_ptr;
        self.memory_low_extent = snapshot.memory_low_extent;
        self.memory_high_extent = snapshot.memory_high_extent;
        self.max_strings = snapshot.max_strings;
        self.pool_size = snapshot.pool_size;
        self.recycling_enabled = snapshot.recycling_enabled;
    }

    fn flush_last(&mut self, strings: usize, characters: usize) {
        self.strings = self.strings.saturating_sub(strings);
        self.characters = self.characters.saturating_sub(characters);
    }

    fn mark_format_baseline(&mut self, memory_low_extent: usize, memory_high_extent: usize) {
        self.init_str_ptr = self.strings;
        self.init_pool_ptr = self.characters;
        // A loaded format retains INITEX's serialized allocator coordinates
        // when it is re-dumped; job-local high-water observations are not a
        // new format-construction baseline.
        if !self.recycling_enabled {
            self.memory_low_extent = memory_low_extent;
            self.memory_high_extent = memory_high_extent;
        }
    }

    #[must_use]
    pub const fn used_strings(&self) -> usize {
        self.strings.saturating_sub(self.init_str_ptr)
    }

    #[must_use]
    pub const fn used_characters(&self) -> usize {
        self.characters.saturating_sub(self.init_pool_ptr)
    }

    #[must_use]
    pub const fn string_capacity(&self) -> usize {
        self.max_strings.saturating_sub(self.init_str_ptr)
    }

    #[must_use]
    pub const fn character_capacity(&self) -> usize {
        self.pool_size.saturating_sub(self.init_pool_ptr)
    }

    pub(crate) const fn memory_extents(&self) -> (usize, usize) {
        (self.memory_low_extent, self.memory_high_extent)
    }

    pub(crate) const fn has_current_profile(&self) -> bool {
        self.profile_version == 11
    }
}

impl EngineUsageStatistics {
    fn merge_max(self, other: Self) -> Self {
        Self {
            strings: self.strings.max(other.strings),
            string_capacity: other.string_capacity,
            string_characters: self.string_characters.max(other.string_characters),
            string_character_capacity: other.string_character_capacity,
            // Variable-size typed nodes report their terminating live extent;
            // TokenStore separately retains §125's one-word allocator extent.
            memory_words: other.memory_words,
            memory_word_capacity: other.memory_word_capacity,
            control_sequences: self.control_sequences.max(other.control_sequences),
            font_info_words: self.font_info_words.max(other.font_info_words),
            fonts: self.fonts.max(other.fonts),
            hyphenation_exceptions: self
                .hyphenation_exceptions
                .max(other.hyphenation_exceptions),
            hyphenation_exception_capacity: self
                .hyphenation_exception_capacity
                .max(other.hyphenation_exception_capacity),
            input_stack: self.input_stack.max(other.input_stack),
            nest_stack: self.nest_stack.max(other.nest_stack),
            parameter_stack: self.parameter_stack.max(other.parameter_stack),
            buffer_stack: self.buffer_stack.max(other.buffer_stack),
            save_stack: self.save_stack.max(other.save_stack),
        }
    }
}

/// Recoverable diagnostics from TeX's `prepare_mag` operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrepareMagDiagnostic {
    IllegalMagnification { attempted: i32 },
    IncompatibleMagnification { attempted: i32, retained: i32 },
}

/// Diagnostics for mutable font parameter assignments.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FontParameterError {
    /// TeX font parameter numbers start at 1.
    Zero,
    /// The parameter number exceeds the injective fontdimen slot domain.
    NumberOutOfRange { number: u32, maximum: u32 },
    /// The dense font id exceeds the fontdimen key's font field.
    FontOutOfRange { font: FontId, maximum: u32 },
    /// A loaded immutable font has more parameters than the cell key can name.
    ParameterCountOutOfRange { count: usize, maximum: u32 },
    /// Loading another distinct font would exceed the fontdimen font field.
    TooManyFonts { maximum: u32 },
    /// Growing the shared TeX82 `font_info` pool would exceed its capacity.
    FontInfoCapacity { capacity: usize },
    /// Only the most recently loaded font may grow its parameter table.
    CannotGrow {
        font: FontId,
        number: u32,
        current_len: u32,
        last_loaded_font: FontId,
    },
}

impl Clone for Stores {
    fn clone(&self) -> Self {
        let mut env = self.env.clone();
        env.reset_snapshot_roots_for_fork();
        Self {
            owner: StoreOwner::new(),
            env,
            interner: self.interner.clone(),
            string_pool: self.string_pool.clone(),
            string_pool_recycled_journal: self.string_pool_recycled_journal.clone(),
            tokens: self.tokens.clone(),
            provenance: self.provenance.clone(),
            source_map: self.source_map.clone(),
            source_fragments: self.source_fragments.clone(),
            macros: self.macros.clone(),
            glue: self.glue.clone(),
            fonts: self.fonts.clone(),
            node_ref_index: NodeListWeakIndex::new(),
            code_tables: self.code_tables.clone(),
            hyphenation: self.hyphenation.clone(),
            prepared_mag: self.prepared_mag,
            last_loaded_font: self.last_loaded_font,
            font_info_capacity: self.font_info_capacity,
            semantic_hash_cache: self.semantic_hash_cache.clone(),
            exact_env_identity: self.exact_env_identity.clone(),
            usage_high_water: self.usage_high_water,
            memory_low_extent: self.memory_low_extent,
            memory_high_extent: self.memory_high_extent,
            main_memory_profile: self.main_memory_profile,
            low_memory_fragments: self.low_memory_fragments.clone(),
            pending_low_memory_break: self.pending_low_memory_break.clone(),
            transient_memory_base: self.transient_memory_base.clone(),
            #[cfg(test)]
            transient_memory_base_projections: self.transient_memory_base_projections,
            #[cfg(test)]
            main_memory_root_traversals: std::sync::atomic::AtomicUsize::new(
                self.main_memory_root_traversals
                    .load(std::sync::atomic::Ordering::Relaxed),
            ),
        }
    }
}

impl Stores {
    pub(crate) fn configure_provenance_budgets(
        &mut self,
        budgets: crate::provenance::ProvenanceBudgets,
    ) {
        self.provenance.configure_budgets(budgets);
    }

    #[cfg(test)]
    pub(crate) fn testing_macro_store(&self) -> &MacroStore {
        &self.macros
    }

    #[cfg(test)]
    pub(crate) fn testing_macro_store_mut(&mut self) -> &mut MacroStore {
        &mut self.macros
    }

    /// TeX82 §§273/275's save depth immediately before the newest checked
    /// push, merged across the Env and CodeTables physical owners.
    pub(crate) fn checked_save_stack_words(&self, save_group_source_lines: bool) -> usize {
        let (env_words, env_latest) = self
            .env
            .canonical_save_stack_projection(save_group_source_lines);
        let (code_words, code_latest) = self.code_tables.canonical_save_stack_projection();
        let latest_words = match (env_latest, code_latest) {
            (Some(env), Some(code)) if code.0 >= env.0 => code.1,
            (Some(env), _) => env.1,
            (None, Some(code)) => code.1,
            (None, None) => 0,
        };
        env_words
            .saturating_add(code_words)
            .saturating_sub(latest_words)
    }

    pub(crate) fn record_engine_stack_usage(&mut self, usage: EngineStackUsage) {
        self.usage_high_water = self.usage_high_water.merge_max(EngineUsageStatistics {
            input_stack: usage.input_stack,
            nest_stack: usage.nest_stack,
            parameter_stack: usage.parameter_stack,
            buffer_stack: usage.buffer_stack,
            save_stack: usage.save_stack,
            ..EngineUsageStatistics::default()
        });
    }

    pub(crate) fn engine_usage_statistics(&mut self) -> EngineUsageStatistics {
        let font_mark = self.fonts.watermark();
        let fonts = font_mark.len as usize;
        let font_info_words = self.font_info_words();
        self.observe_main_memory();
        let current = EngineUsageStatistics {
            strings: self.string_pool.used_strings(),
            string_capacity: self.string_pool.string_capacity(),
            string_characters: self.string_pool.used_characters(),
            string_character_capacity: self.string_pool.character_capacity(),
            memory_words: self
                .memory_low_extent
                .saturating_add(self.memory_high_extent),
            memory_word_capacity: TEX82_MEMORY_WORD_CAPACITY,
            // TeX82 §1334 reports occupied §259 hash entries, not the whole
            // control-sequence `eqtb` namespace described by §222.
            control_sequences: self.interner.multiletter_len(),
            font_info_words,
            fonts: fonts.saturating_sub(1),
            hyphenation_exceptions: self.hyphenation.exception_usage().occupied,
            hyphenation_exception_capacity: self.hyphenation.exception_usage().capacity,
            ..EngineUsageStatistics::default()
        };
        let high_water = self.usage_high_water.merge_max(current);
        self.usage_high_water = high_water;
        high_water
    }

    fn cached_main_memory_usage(
        &mut self,
        extra_nodes: &[Node],
        include_scratch_extent: bool,
    ) -> Result<format::MainMemoryUsage, format::StoreFormatError> {
        let projection = self.take_transient_memory_base()?;
        let usage = projection.usage_with_extra_nodes(self, extra_nodes);
        self.transient_memory_base = Some((self.glue.watermark().specs, projection));
        usage.map(|usage| {
            if include_scratch_extent {
                format::main_memory_usage_with_scratch_extent(usage)
            } else {
                usage
            }
        })
    }

    fn observe_main_memory(&mut self) -> usize {
        let usage = self.cached_main_memory_usage(&[], true);
        self.record_main_memory_usage(usage)
    }

    pub(crate) fn observe_main_memory_nodes(&mut self, extra_nodes: &[Node]) -> usize {
        let projection = self.take_transient_memory_base();
        let Ok(projection) = projection else {
            return self.record_main_memory_usage(projection.map(|value| value.usage()));
        };
        if let Ok(requests) = projection.low_node_requests(self, extra_nodes) {
            self.observe_low_memory_requests(projection.usage().variable, &requests);
        }
        let usage = projection.usage_with_extra_nodes(self, extra_nodes);
        self.transient_memory_base = Some((self.glue.watermark().specs, projection));
        self.record_main_memory_usage(usage)
    }

    pub(crate) fn observe_line_break_memory_search(&mut self, memory: &crate::PureBreakMemoryPlan) {
        let live_words = self
            .cached_main_memory_usage(&[], false)
            .map_or(TEX82_STATIC_LOW_MEMORY_WORDS, |usage| usage.variable);
        let mut pending = PendingLowMemoryBreak {
            arena: low_memory::LowMemoryArena::from_live_and_fragments(
                self.memory_low_extent,
                TEX82_LOW_MEMORY_GROWTH,
                live_words,
                &self.low_memory_fragments,
            ),
            owners: Vec::new(),
        };
        Self::replay_low_memory_events(&mut pending, &memory.search);
        self.memory_low_extent = self.memory_low_extent.max(pending.arena.extent());
        self.pending_low_memory_break = Some(pending);
    }

    pub(crate) fn observe_line_break_memory_cleanup(
        &mut self,
        memory: &crate::PureBreakMemoryPlan,
    ) {
        let Some(mut pending) = self.pending_low_memory_break.take() else {
            return;
        };
        Self::replay_low_memory_events(&mut pending, &memory.cleanup);
        self.memory_low_extent = self.memory_low_extent.max(pending.arena.extent());
        self.low_memory_fragments = pending.arena.detached_free_sizes();
    }

    fn replay_low_memory_events(
        pending: &mut PendingLowMemoryBreak,
        events: &[crate::PureBreakMemoryEvent],
    ) {
        for &event in events {
            match event {
                crate::PureBreakMemoryEvent::Allocate { owner, words } => {
                    let allocation = pending.arena.allocate(usize::from(words));
                    pending.owners.push((owner, allocation));
                }
                crate::PureBreakMemoryEvent::Free(owner) => {
                    let index = pending
                        .owners
                        .iter()
                        .position(|(candidate, _)| *candidate == owner)
                        .expect("line-break allocator owner is live");
                    let (_, allocation) = pending.owners.remove(index);
                    pending.arena.free(allocation);
                }
            }
        }
    }

    fn observe_low_memory_requests(&mut self, live_words: usize, requests: &[usize]) {
        if requests.is_empty() {
            return;
        }
        if let Some(pending) = &mut self.pending_low_memory_break {
            for &request in requests {
                let _ = pending.arena.allocate(request);
            }
            self.memory_low_extent = self.memory_low_extent.max(pending.arena.extent());
            return;
        }
        if self.low_memory_fragments.is_empty() {
            return;
        }
        let mut arena = low_memory::LowMemoryArena::from_live_and_fragments(
            self.memory_low_extent,
            TEX82_LOW_MEMORY_GROWTH,
            live_words,
            &self.low_memory_fragments,
        );
        for &request in requests {
            let _ = arena.allocate(request);
        }
        self.memory_low_extent = self.memory_low_extent.max(arena.extent());
        self.low_memory_fragments = arena.detached_free_sizes();
    }

    pub(crate) fn observe_main_memory_dynamic_words(&mut self, extra_words: usize) -> usize {
        // TeX82 §§125--130 retain the allocator base between actual owner
        // changes; §1334 only observes that allocator state. Scanner-owned
        // transient words therefore extend one reusable live-root base rather
        // than requiring the macro/token closure to be rebuilt per sample.
        #[cfg(feature = "profiling")]
        crate::measurement::record_main_memory_dynamic_observation();
        let projection = self.take_transient_memory_base();
        let Ok(projection) = projection else {
            return self.record_main_memory_usage(projection.map(|value| value.usage()));
        };
        let base = projection.usage();
        self.transient_memory_base = Some((self.glue.watermark().specs, projection));
        self.record_main_memory_usage(Ok(format::main_memory_usage_with_extra_dynamic_words(
            base,
            extra_words,
        )))
    }

    pub(crate) fn observe_main_memory_box_copy(
        &mut self,
        root: &NodeListRef,
        live_dynamic_words: usize,
    ) {
        // Root capture/update precomputes the ordered §204 summary. The hot
        // copy path composes lifetimes without revisiting the frozen graph.
        let projection = self.take_transient_memory_base();
        let Ok(projection) = projection else {
            self.record_main_memory_usage(projection.map(|value| value.usage()));
            return;
        };
        let usage = projection
            .usage_with_box_copy(root.id(), live_dynamic_words)
            .expect("copied box root belongs to the allocator projection");
        self.transient_memory_base = Some((self.glue.watermark().specs, projection));
        self.record_main_memory_usage(Ok(usage));
    }

    fn take_transient_memory_base(
        &mut self,
    ) -> Result<format::MainMemoryProjection, format::StoreFormatError> {
        let glue_specs = self.glue.watermark().specs;
        if let Some((cached_glue_specs, mut projection)) = self.transient_memory_base.take() {
            #[cfg(feature = "profiling")]
            crate::measurement::record_main_memory_base_request(true);
            projection.update_glue_specs(cached_glue_specs, glue_specs);
            return Ok(projection);
        }
        #[cfg(feature = "profiling")]
        crate::measurement::record_main_memory_base_request(false);
        #[cfg(test)]
        {
            self.transient_memory_base_projections =
                self.transient_memory_base_projections.saturating_add(1);
        }
        format::main_memory_usage_without_scratch(self)
    }

    pub(crate) fn for_each_main_memory_root_word(&self, f: impl FnMut(crate::cell::CellId, u64)) {
        #[cfg(test)]
        self.main_memory_root_traversals
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.env.for_each_main_memory_root_word(f);
    }

    /// Updates the cached TeX82 allocator base after a canonical root changes.
    /// Immutable store appends are deliberately not roots.
    pub(crate) fn update_main_memory_roots(
        &mut self,
        receipt: crate::env::CellMutationReceipt,
    ) -> bool {
        if receipt.main_memory_roots_updated() {
            return true;
        }
        let cell = receipt.cell();
        if !matches!(
            cell.bank(),
            crate::cell::BankTag::Meaning
                | crate::cell::BankTag::Toks
                | crate::cell::BankTag::TokParam
                | crate::cell::BankTag::Box
        ) {
            return false;
        }
        let (old_word, new_word) = receipt.words();
        let Some((_, mut projection)) = self.transient_memory_base.take() else {
            return false;
        };
        let retained = projection
            .update_cell(self, cell, old_word, new_word)
            .is_ok_and(|updated| updated);
        #[cfg(feature = "profiling")]
        crate::measurement::record_main_memory_cell_root_update(retained);
        if retained {
            self.transient_memory_base = Some((self.glue.watermark().specs, projection));
        } else {
            #[cfg(feature = "profiling")]
            crate::measurement::record_main_memory_cache_loss(
                crate::measurement::MainMemoryProjectionLossOwner::CellRootUpdate,
            );
        }
        retained
    }

    /// Updates one box-register root while its structural graph is still live.
    /// TeX82 §§125--130 mutate allocator ownership at the root;
    /// replacing or restoring Umber's immutable graph must not force the next
    /// allocation event to reconstruct every unrelated root before §1334.
    fn update_main_memory_box_root(
        &mut self,
        old: Option<NodeListId>,
        new: Option<NodeListId>,
    ) -> bool {
        let Some((_, mut projection)) = self.transient_memory_base.take() else {
            return false;
        };
        // If this projection has not seen the graph, discard the cache and let
        // the next full root capture borrow Env's owner.
        let update = projection.update_box_root(self, old, new, false);
        let retained = update.is_ok_and(|updated| updated);
        #[cfg(feature = "profiling")]
        crate::measurement::record_main_memory_box_root_update(retained);
        if retained {
            self.transient_memory_base = Some((self.glue.watermark().specs, projection));
            true
        } else {
            #[cfg(feature = "profiling")]
            crate::measurement::record_main_memory_cache_loss(
                crate::measurement::MainMemoryProjectionLossOwner::BoxRootUpdate,
            );
            false
        }
    }

    #[cfg(test)]
    pub(crate) const fn testing_transient_memory_base_projections(&self) -> usize {
        self.transient_memory_base_projections
    }

    #[cfg(test)]
    pub(crate) fn testing_main_memory_root_traversals(&self) -> usize {
        self.main_memory_root_traversals
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    fn record_main_memory_usage(
        &mut self,
        usage: Result<format::MainMemoryUsage, format::StoreFormatError>,
    ) -> usize {
        let variable_usage = usage
            .as_ref()
            .map_or(TEX82_STATIC_LOW_MEMORY_WORDS, |usage| usage.variable);
        let low_extent = variable_usage
            .saturating_sub(TEX82_STATIC_LOW_MEMORY_WORDS)
            .max(1)
            .div_ceil(TEX82_LOW_MEMORY_GROWTH)
            .saturating_mul(TEX82_LOW_MEMORY_GROWTH)
            .saturating_add(TEX82_STATIC_LOW_MEMORY_WORDS);
        self.memory_low_extent = self.memory_low_extent.max(low_extent);
        let (dynamic_usage, dynamic_extent) = usage
            .map(|usage| (usage.dynamic, usage.dynamic_extent))
            .unwrap_or((self.memory_high_extent, self.memory_high_extent));
        self.memory_high_extent = self.memory_high_extent.max(dynamic_extent);
        dynamic_usage
    }

    /// TeX82 §638's live `var_used`/`dyn_used` projection.
    ///
    /// TeX's variable-size arena owns glue specifications and multiword
    /// nodes; its one-word dynamic arena owns token nodes. Unlike
    /// [`Self::engine_usage_statistics`], this is a live observation rather
    /// than a high-water mark, because `ship_out` compares the values before
    /// and after releasing the shipped box.
    pub(crate) fn shipout_memory_usage(&mut self, shipped_node: Option<&Node>) -> (usize, usize) {
        let extra_nodes = shipped_node.map_or(&[][..], std::slice::from_ref);
        let usage = self.cached_main_memory_usage(extra_nodes, true);
        let variable_usage = usage
            .as_ref()
            .map_or(TEX82_STATIC_LOW_MEMORY_WORDS, |usage| usage.variable);
        let dynamic_usage = self.record_main_memory_usage(usage);
        (variable_usage, dynamic_usage)
    }
    pub(crate) fn loaded_fonts(&self) -> impl Iterator<Item = &LoadedFont> {
        self.fonts.iter()
    }

    pub(crate) fn install_source_fragments(&mut self, fragments: FragmentStore) {
        self.source_fragments = fragments;
    }

    pub(crate) fn bind_rebound_root_registration(&mut self, source: SourceId) {
        let Some(registration) = self.source_map.registered_source(source) else {
            return;
        };
        self.source_fragments
            .bind_rebound_root_registration(registration);
    }

    pub(crate) fn direct_root_span_id(
        &self,
        origin: crate::token::OriginId,
    ) -> Option<crate::RootSpanId> {
        self.source_fragments.direct_root_span_id(origin)
    }

    pub(crate) fn source_origin_root_span_id(
        &self,
        source: crate::provenance::SourceOrigin,
    ) -> Option<crate::RootSpanId> {
        let position = self
            .source_position(source.source(), source.byte_offset())
            .ok()?;
        self.source_fragments
            .direct_root_span_id(OriginId::direct_source(position)?)
    }

    pub(crate) fn source_span_root_span_id(
        &self,
        span: crate::source_map::SourceSpan,
    ) -> Option<crate::RootSpanId> {
        self.source_fragments.root_span_for_source_span(span)
    }

    pub(crate) fn source_span_for_root(
        &self,
        span: crate::RootSpanId,
    ) -> Option<crate::source_map::SourceSpan> {
        self.source_fragments.source_span_for_root(span)
    }

    pub(crate) fn can_restore_snapshot(&self, snapshot: &StoreSnapshot) -> bool {
        snapshot.owner == self.owner.snapshot_owner()
            && self.env.can_rollback_to(&snapshot.env_snapshot)
            && snapshot.env_snapshot.journal_pos() <= self.env.current_journal_pos()
            && snapshot.string_pool_recycled_mark <= self.string_pool_recycled_journal.len()
    }

    /// Retargets an already-validated inherited snapshot to this fork's exact owner.
    pub(crate) fn retarget_inherited_snapshot(&self, snapshot: &StoreSnapshot) -> StoreSnapshot {
        let mut snapshot = snapshot.clone();
        snapshot.owner = self.owner.snapshot_owner();
        snapshot.env_snapshot = self.env.retarget_snapshot(&snapshot.env_snapshot);
        snapshot
    }

    pub(crate) fn env_group_depth(&self) -> u32 {
        self.env.group_depth()
    }

    pub(crate) fn innermost_group_kind(&self) -> Option<GroupKind> {
        self.env.innermost_group_kind()
    }

    pub(crate) fn group_kinds(&self) -> impl DoubleEndedIterator<Item = GroupKind> + '_ {
        self.env.group_kinds()
    }

    pub(crate) fn group_frames(&self) -> impl DoubleEndedIterator<Item = GroupFrame> + '_ {
        self.env.group_frames()
    }
    /// Creates an empty state-store tuple.
    #[must_use]
    pub fn new() -> Self {
        let mut stores = Self {
            owner: StoreOwner::new(),
            env: Env::new(),
            interner: Interner::new(),
            string_pool: StringPoolAccounting::default(),
            string_pool_recycled_journal: Vec::new(),
            tokens: TokenStore::new(),
            provenance: ProvenanceStore::new(),
            source_map: SourceMap::default(),
            source_fragments: FragmentStore::new(),
            macros: MacroStore::new(),
            glue: GlueStore::new(),
            fonts: FontStore::new(),
            node_ref_index: NodeListWeakIndex::new(),
            code_tables: CodeTables::new(),
            hyphenation: Arc::new(HyphenationTable::new()),
            prepared_mag: None,
            last_loaded_font: NULL_FONT,
            font_info_capacity: crate::font::FONT_INFO_CAPACITY,
            semantic_hash_cache: state_hash::SemanticHashCache::default(),
            exact_env_identity: exact_identity::ExactEnvIdentity::default(),
            usage_high_water: EngineUsageStatistics::default(),
            memory_low_extent: TEX82_INITIAL_LOW_MEMORY_EXTENT,
            memory_high_extent: TEX82_INITIAL_HIGH_MEMORY_EXTENT,
            main_memory_profile: StringPoolProfile::Tex82,
            low_memory_fragments: Vec::new(),
            pending_low_memory_break: None,
            transient_memory_base: None,
            #[cfg(test)]
            transient_memory_base_projections: 0,
            #[cfg(test)]
            main_memory_root_traversals: std::sync::atomic::AtomicUsize::new(0),
        };
        stores.env.install_empty_token_root(
            stores
                .tokens
                .owner(TokenListId::EMPTY)
                .expect("token store owns canonical empty list"),
        );
        stores.set_int_param(IntParam::MAG, 1000);
        stores.set_int_param(IntParam::TOLERANCE, 10_000);
        stores.set_int_param(IntParam::HANG_AFTER, 1);
        stores.set_int_param(IntParam::MAX_DEAD_CYCLES, 25);
        stores.set_int_param(IntParam::ESCAPE_CHAR, b'\\'.into());
        stores.set_int_param(IntParam::END_LINE_CHAR, 13);
        stores.initialize_font_banks(NULL_FONT, 7, &[]);
        // TeX.web §§552--556 define these directly on `nullfont`; they do not
        // come from the mutable defaults used for subsequently loaded fonts.
        stores.set_font_hyphen_char(NULL_FONT, i32::from(b'-'));
        stores.set_font_skew_char(NULL_FONT, -1);
        stores.initialize_exact_env_identity();
        stores.discard_exact_env_undo_history();
        stores
    }

    /// Reads the owned environment.
    #[must_use]
    #[cfg(test)]
    pub fn env(&self) -> &Env {
        &self.env
    }

    pub(crate) fn effective_restored_env_word(&self, cell: crate::cell::CellId) -> u64 {
        let raw = self.env.semantic_word(cell);
        self.env.restored_semantic_word(cell, raw).word
    }

    pub(crate) fn begin_dependency_journal_region(&mut self) -> crate::env::JournalRegionMark {
        self.env.begin_journal_region()
    }

    pub(crate) fn dependency_journal_region_cells(
        &self,
        mark: crate::env::JournalRegionMark,
    ) -> Result<Vec<crate::cell::CellId>, crate::env::JournalRegionInvalidated> {
        self.env.journal_region_cells(mark)
    }

    pub(crate) fn semantic_env_word(&self, cell: crate::cell::CellId) -> u64 {
        self.env.semantic_word(cell)
    }

    /// Rewrites one semantic environment cell outside TeX assignment policy.
    ///
    /// Env owns the raw typed storage, while Stores owns exact identity because
    /// canonical values resolve aggregate token, macro, glue, font, and node
    /// handles. Every raw semantic write therefore crosses this seam exactly
    /// once. Env records a save-stack-neutral global undo so group refiling and
    /// aggregate rollback preserve ordering; Stores updates the accumulator now, and
    /// the later journal hash observes that it is already synchronized rather
    /// than folding it twice.
    fn restore_env_word_with_exact_identity(
        &mut self,
        cell: crate::cell::CellId,
        word: u64,
    ) -> CellMutationReceipt {
        let token_root = match cell.bank() {
            crate::cell::BankTag::Toks => self.tokens.owner(TokenListId::new(word as u32)),
            crate::cell::BankTag::TokParam if word != 0 => {
                self.tokens.owner(TokenListId::new((word - 1) as u32))
            }
            crate::cell::BankTag::TokParam => None,
            _ => None,
        };
        if matches!(
            cell.bank(),
            crate::cell::BankTag::Toks | crate::cell::BankTag::TokParam
        ) && word != 0
        {
            assert!(token_root.is_some(), "raw token word has no live owner");
        }
        let macro_root = if cell.bank() == crate::cell::BankTag::Meaning {
            match Meaning::decode_stored(word) {
                Meaning::Macro { definition, .. } => Some(self.macro_definition_ref(definition)),
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
            self.glue
                .resolve_stored(GlueId::new(word as u32))
                .and_then(|id| self.glue.owner(id))
        } else {
            None
        };
        if matches!(
            cell.bank(),
            crate::cell::BankTag::Skip
                | crate::cell::BankTag::Muskip
                | crate::cell::BankTag::GlueParam
        ) && word != 0
        {
            assert!(glue_root.is_some(), "raw glue word has no live owner");
        }
        let receipt = self
            .env
            .restore_raw_global(cell, word, token_root, macro_root, glue_root, None);
        if receipt.changed() {
            self.synchronize_exact_env_identity();
        }
        receipt
    }

    pub(crate) fn rewrite_null_parshape_representation(
        &mut self,
        value: TokenListId,
    ) -> CellMutationReceipt {
        let cell = crate::cell::CellId::new(
            crate::cell::BankTag::TokParam,
            u32::from(TokParam::PAR_SHAPE_INTERNAL.raw()),
        );
        self.restore_env_word_with_exact_identity(cell, u64::from(value.raw()) + 1)
    }

    #[cfg(test)]
    pub(crate) fn testing_restore_env_word(
        &mut self,
        cell: crate::cell::CellId,
        word: u64,
    ) -> CellMutationReceipt {
        self.restore_env_word_with_exact_identity(cell, word)
    }

    /// Returns the current code-table generation vector.
    #[must_use]
    pub fn code_table_generations(&self) -> CodeTableGenerations {
        self.code_tables.generations()
    }

    #[must_use]
    pub fn catcode(&self, ch: char) -> Catcode {
        self.code_tables.catcode(ch)
    }

    pub fn set_catcode(&mut self, ch: char, value: Catcode) {
        self.code_tables
            .set_catcode_at(self.env.journal_pos().raw() as usize, ch, value);
    }

    pub fn set_catcode_global(&mut self, ch: char, value: Catcode) {
        self.code_tables.set_catcode_global(ch, value);
    }

    #[must_use]
    pub fn lccode(&self, ch: char) -> LcCode {
        self.code_tables.lccode(ch)
    }

    pub fn set_lccode(&mut self, ch: char, value: LcCode) {
        self.code_tables
            .set_lccode_at(self.env.journal_pos().raw() as usize, ch, value);
    }

    pub fn set_lccode_global(&mut self, ch: char, value: LcCode) {
        self.code_tables.set_lccode_global(ch, value);
    }

    #[must_use]
    pub fn uccode(&self, ch: char) -> UcCode {
        self.code_tables.uccode(ch)
    }

    pub fn set_uccode(&mut self, ch: char, value: UcCode) {
        self.code_tables
            .set_uccode_at(self.env.journal_pos().raw() as usize, ch, value);
    }

    pub fn set_uccode_global(&mut self, ch: char, value: UcCode) {
        self.code_tables.set_uccode_global(ch, value);
    }

    #[must_use]
    pub fn sfcode(&self, ch: char) -> SfCode {
        self.code_tables.sfcode(ch)
    }

    pub fn set_sfcode(&mut self, ch: char, value: SfCode) {
        self.code_tables
            .set_sfcode_at(self.env.journal_pos().raw() as usize, ch, value);
    }

    pub fn set_sfcode_global(&mut self, ch: char, value: SfCode) {
        self.code_tables.set_sfcode_global(ch, value);
    }

    #[must_use]
    pub fn mathcode(&self, ch: char) -> MathCode {
        self.code_tables.mathcode(ch)
    }

    pub fn set_mathcode(&mut self, ch: char, value: MathCode) {
        self.code_tables
            .set_mathcode_at(self.env.journal_pos().raw() as usize, ch, value);
    }

    pub fn set_mathcode_global(&mut self, ch: char, value: MathCode) {
        self.code_tables.set_mathcode_global(ch, value);
    }

    #[must_use]
    pub fn delcode(&self, ch: char) -> DelCode {
        self.code_tables.delcode(ch)
    }

    pub fn set_delcode(&mut self, ch: char, value: DelCode) {
        self.code_tables
            .set_delcode_at(self.env.journal_pos().raw() as usize, ch, value);
    }

    pub fn set_delcode_global(&mut self, ch: char, value: DelCode) {
        self.code_tables.set_delcode_global(ch, value);
    }

    pub fn add_hyphenation_pattern(
        &mut self,
        pattern: PatternSpec,
    ) -> Result<(), crate::hyphenation::HyphenationCapacityError> {
        self.add_hyphenation_pattern_for_language(0, pattern)
            .map(|_| ())
    }

    pub fn add_hyphenation_pattern_for_language(
        &mut self,
        language: u8,
        pattern: PatternSpec,
    ) -> Result<bool, crate::hyphenation::HyphenationCapacityError> {
        Arc::make_mut(&mut self.hyphenation).add_pattern_for_language(language, pattern)
    }

    pub fn set_hyphenation_trie_capacity(&mut self, capacity: usize) {
        Arc::make_mut(&mut self.hyphenation).set_trie_capacity(capacity);
    }

    pub fn set_hyphenation_exception_capacity(&mut self, capacity: usize) {
        Arc::make_mut(&mut self.hyphenation).set_exception_capacity(capacity);
    }

    #[must_use]
    pub(crate) fn contains_hyphenation_pattern_for_language(
        &self,
        language: u8,
        letters: &[char],
    ) -> bool {
        self.hyphenation
            .contains_pattern_for_language(language, letters)
    }

    #[must_use]
    pub fn hyphenation_patterns_open(&self) -> bool {
        self.hyphenation.patterns_open()
    }

    pub fn close_hyphenation_patterns(&mut self) {
        Arc::make_mut(&mut self.hyphenation).close_patterns();
    }

    pub fn add_hyphenation_exception(&mut self, exception: ExceptionSpec) {
        self.add_hyphenation_exception_for_language(0, exception);
    }

    pub fn add_hyphenation_exception_for_language(
        &mut self,
        language: u8,
        exception: ExceptionSpec,
    ) {
        // TeX82 §934 retains the normalized word plus its language byte.
        // Web2C tex.ch [42.941] flushes that just-made string when the same
        // language/word entry is replaced.
        let word = exception.word.clone();
        let insertion =
            Arc::make_mut(&mut self.hyphenation).add_exception_for_language(language, exception);
        if matches!(insertion, crate::hyphenation::ExceptionInsertion::Allocated) {
            self.string_pool.allocate(1, word.len().saturating_add(1));
            if !self.string_pool.recycling_enabled
                && !self.string_pool.unfinished_current_string_accounted
            {
                // TeX82 §38 has one current string between str_start[str_ptr]
                // and pool_ptr. INITEX's aggregate projection includes that
                // one unfinished byte, not one byte per §934 exception.
                self.string_pool.allocate(0, 1);
                self.string_pool.unfinished_current_string_accounted = true;
            }
        }
    }

    pub fn save_hyphenation_codes(
        &mut self,
        language: u8,
        codes: impl IntoIterator<Item = (char, char)>,
    ) {
        Arc::make_mut(&mut self.hyphenation).save_hyphen_codes(language, codes);
    }

    #[must_use]
    pub fn saved_hyphenation_code(&self, language: u8, ch: char) -> Option<Option<char>> {
        self.hyphenation.saved_hyphen_code(language, ch)
    }

    pub(crate) fn hyphenation_dependency_fingerprint(&self, language: u8, kind: u8) -> u64 {
        self.hyphenation.dependency_fingerprint(language, kind)
    }

    #[must_use]
    pub fn hyphen_positions(&self, word: &str, left_min: usize, right_min: usize) -> Vec<usize> {
        self.hyphen_positions_for_language(0, word, left_min, right_min)
    }

    #[must_use]
    pub fn hyphen_positions_for_language(
        &self,
        language: u8,
        word: &str,
        left_min: usize,
        right_min: usize,
    ) -> Vec<usize> {
        self.hyphenation
            .hyphen_positions_for_language(language, word, left_min, right_min)
    }

    #[must_use]
    pub fn hyphenation_exception(&self, word: &str) -> Option<&[usize]> {
        self.hyphenation.exception(word)
    }

    /// Returns the meaning for a live control-sequence symbol.
    #[must_use]
    pub fn meaning(&self, symbol: impl SymbolReference) -> Meaning {
        let symbol = self.resolve_symbol_reference(symbol);
        let stored = self.env.get_meaning_slot(symbol.raw());
        match stored {
            Meaning::Macro { definition, flags } => Meaning::Macro {
                definition: self
                    .env
                    .macro_root_id(crate::cell::CellId::new(
                        crate::cell::BankTag::Meaning,
                        symbol.raw(),
                    ))
                    .filter(|live| live.raw() == definition.raw())
                    .expect("macro meaning cell has no matching live root"),
                flags,
            },
            other => self.resolve_stored_meaning(other),
        }
    }

    pub(crate) fn symbol_at_slot(&self, slot: u32) -> Option<Symbol> {
        self.interner.symbol_at_slot(slot)
    }

    pub(crate) fn first_symbol_with_meaning(&self, meaning: Meaning) -> Option<Symbol> {
        (0..self.interner.len()).find_map(|slot| {
            let symbol = self.interner.symbol_at_slot(slot as u32)?;
            (self.meaning(symbol) == meaning).then_some(symbol)
        })
    }

    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn testing_meaning_level(&self, symbol: impl SymbolReference) -> u32 {
        let symbol = self.resolve_symbol_reference(symbol);
        self.env.testing_meaning_level(symbol.symbol())
    }

    /// Sets the local meaning for a live control-sequence symbol.
    pub fn set_meaning(
        &mut self,
        symbol: impl SymbolReference,
        meaning: Meaning,
    ) -> crate::env::CellMutationReceipt {
        let symbol = self.resolve_symbol_reference(symbol);
        self.assert_live_macro_definition_in_meaning(meaning);
        self.assert_live_font_in_meaning(meaning);
        let macro_root = match meaning {
            Meaning::Macro { definition, .. } => Some(self.macro_definition_ref(definition)),
            _ => None,
        };
        self.env
            .set_meaning_slot_with_macro_root(symbol.raw(), meaning, macro_root, false)
    }

    /// Interns a control-sequence name and gives a previously undefined name
    /// TeX's `\csname`-created `\relax` meaning.
    pub(crate) fn intern_relaxed_control_sequence_with_receipt(
        &mut self,
        name: &str,
    ) -> (SymbolId, Option<CellMutationReceipt>) {
        let symbol = self
            .try_intern_hash(name)
            .expect("control-sequence symbol capacity exceeded");
        let receipt = (self.meaning(symbol) == Meaning::Undefined)
            .then(|| self.set_meaning(symbol, Meaning::Relax));
        (symbol, receipt)
    }

    /// Sets the global meaning for a live control-sequence symbol.
    pub fn set_meaning_global(
        &mut self,
        symbol: impl SymbolReference,
        meaning: Meaning,
    ) -> crate::env::CellMutationReceipt {
        let symbol = self.resolve_symbol_reference(symbol);
        self.assert_live_macro_definition_in_meaning(meaning);
        self.assert_live_font_in_meaning(meaning);
        let macro_root = match meaning {
            Meaning::Macro { definition, .. } => Some(self.macro_definition_ref(definition)),
            _ => None,
        };
        self.env
            .set_meaning_slot_with_macro_root(symbol.raw(), meaning, macro_root, true)
    }

    /// Interns a frozen macro definition in the owned macro-definition store.
    pub fn intern_macro(&mut self, macro_meaning: MacroMeaning) -> MacroDefinitionRef {
        self.intern_macro_with_provenance_in_domain(macro_meaning, None, None)
    }

    /// Interns a frozen macro definition with optional diagnostic provenance.
    pub fn intern_macro_with_provenance(
        &mut self,
        macro_meaning: MacroMeaning,
        provenance: Option<MacroDefinitionProvenance>,
    ) -> MacroDefinitionRef {
        self.intern_macro_with_provenance_in_domain(macro_meaning, provenance, None)
    }

    pub(crate) fn intern_macro_with_provenance_in_domain(
        &mut self,
        macro_meaning: MacroMeaning,
        provenance: Option<MacroDefinitionProvenance>,
        domain: Option<&mut crate::patch_domain::PatchAllocationDomain>,
    ) -> MacroDefinitionRef {
        self.assert_live_token_list(macro_meaning.parameter_text());
        self.assert_live_token_list(macro_meaning.replacement_text());
        if let Some(provenance) = &provenance {
            self.assert_live_origin(provenance.definition_origin());
            self.assert_origin_list_len_matches(
                macro_meaning.parameter_text(),
                provenance.parameter_ref(),
            );
            self.assert_origin_list_len_matches(
                macro_meaning.replacement_text(),
                provenance.replacement_ref(),
            );
        }
        let parameter_root = self
            .tokens
            .resolved_owner(macro_meaning.parameter_text())
            .expect("macro parameter tokens have a live owner");
        let replacement_root = self
            .tokens
            .resolved_owner(macro_meaning.replacement_text())
            .expect("macro replacement tokens have a live owner");
        let parameter_pattern = MacroParameterPattern::from_tokens(parameter_root.tokens());
        let observation_width =
            u32::try_from(1_usize + parameter_root.len() + replacement_root.len())
                .expect("macro token list length exceeds u32");
        let parameter_semantic_id = parameter_root.semantic_id();
        let replacement_semantic_id = replacement_root.semantic_id();
        self.macros.intern_with_provenance(
            macro_meaning,
            parameter_root,
            replacement_root,
            parameter_pattern,
            parameter_semantic_id,
            replacement_semantic_id,
            provenance,
            observation_width,
            domain,
        )
    }

    pub(crate) fn macro_definition_ref(&self, id: MacroDefinitionId) -> MacroDefinitionRef {
        self.macros
            .resolved_owner(id)
            .expect("macro definition id is not live")
    }

    pub(crate) fn packed_macro_owner(
        &self,
        id: MacroDefinitionId,
    ) -> crate::macro_store::PackedMacroChunkOwner {
        self.macros
            .packed_owner(id)
            .expect("macro definition has no packed chunk")
    }

    pub(crate) fn packed_macro_meaning(&self, id: MacroDefinitionId) -> Option<MacroMeaning> {
        self.macros.packed_meaning(id)
    }

    /// Reads a live frozen macro definition.
    #[must_use]
    pub fn macro_definition(&self, id: MacroDefinitionId) -> MacroMeaning {
        self.macros.get(id)
    }

    /// Returns TeX82's definition-head identity for command observation.
    #[must_use]
    pub fn macro_definition_observation_operand(&self, id: MacroDefinitionId) -> i64 {
        self.macros.observation_operand(id)
    }

    pub(crate) fn packed_macro_observation_operand(&self, id: MacroDefinitionId) -> Option<i64> {
        self.macros.packed_observation_operand(id)
    }

    /// Reads the pre-parsed parameter structure for a live macro definition.
    #[must_use]
    pub fn macro_definition_parameter_pattern(
        &self,
        id: MacroDefinitionId,
    ) -> MacroParameterPattern {
        self.macros.parameter_pattern(id)
    }

    /// Reads structurally owned diagnostic provenance for a macro definition,
    /// degrading to unknown when a loaded definition has none.
    #[must_use]
    pub fn macro_definition_provenance(&self, id: MacroDefinitionId) -> MacroDefinitionProvenance {
        self.macros
            .provenance(id)
            .unwrap_or_else(MacroDefinitionProvenance::unknown)
    }

    pub(crate) fn macro_definition_provenance_roots(
        &self,
        id: MacroDefinitionId,
    ) -> Option<(OriginRef, OriginListRef, OriginListRef)> {
        let provenance = self.macros.provenance(id)?;
        Some((
            provenance.definition_ref().clone(),
            provenance.parameter_ref().clone(),
            provenance.replacement_ref().clone(),
        ))
    }

    pub(crate) fn set_macro_definition_provenance(
        &mut self,
        id: MacroDefinitionId,
        provenance: MacroDefinitionProvenance,
    ) {
        self.macros.set_provenance(id, provenance);
    }

    /// Sets a local macro meaning by freezing its public aggregate first.
    pub fn set_macro_meaning(
        &mut self,
        symbol: impl SymbolReference,
        macro_meaning: MacroMeaning,
    ) -> CellMutationReceipt {
        let definition = self.intern_macro(macro_meaning);
        self.install_macro_meaning(symbol, macro_meaning.flags(), definition, false)
    }

    /// Sets a local macro meaning with diagnostic definition provenance.
    pub fn set_macro_meaning_with_provenance(
        &mut self,
        symbol: impl SymbolReference,
        macro_meaning: MacroMeaning,
        provenance: MacroDefinitionProvenance,
    ) -> CellMutationReceipt {
        let definition = self.intern_macro_with_provenance(macro_meaning, Some(provenance));
        self.install_macro_meaning(symbol, macro_meaning.flags(), definition, false)
    }

    /// Sets a global macro meaning by freezing its public aggregate first.
    pub fn set_macro_meaning_global(
        &mut self,
        symbol: impl SymbolReference,
        macro_meaning: MacroMeaning,
    ) -> CellMutationReceipt {
        let definition = self.intern_macro(macro_meaning);
        self.install_macro_meaning(symbol, macro_meaning.flags(), definition, true)
    }

    /// Sets a global macro meaning with diagnostic definition provenance.
    pub fn set_macro_meaning_global_with_provenance(
        &mut self,
        symbol: impl SymbolReference,
        macro_meaning: MacroMeaning,
        provenance: MacroDefinitionProvenance,
    ) -> CellMutationReceipt {
        let definition = self.intern_macro_with_provenance(macro_meaning, Some(provenance));
        self.install_macro_meaning(symbol, macro_meaning.flags(), definition, true)
    }

    /// Installs an ordinary runtime definition from the scanner's existing
    /// strong token owners, without weak resolution or exact-body interning.
    pub fn set_macro_meaning_from_traced(
        &mut self,
        symbol: impl SymbolReference,
        flags: MeaningFlags,
        parameter_text: &TracedTokenList,
        replacement_text: &TracedTokenList,
        provenance: MacroDefinitionProvenance,
        global: bool,
    ) -> CellMutationReceipt {
        let parameter_root = parameter_text.token_ref();
        let replacement_root = replacement_text.token_ref();
        assert!(
            self.tokens.accepts_owner(parameter_root),
            "macro parameter tokens belong to a foreign or stale timeline"
        );
        assert!(
            self.tokens.accepts_owner(replacement_root),
            "macro replacement tokens belong to a foreign or stale timeline"
        );
        self.assert_live_origin(provenance.definition_origin());
        assert_eq!(
            parameter_root.len(),
            provenance.parameter_ref().origins().len(),
            "macro parameter token and origin lengths differ"
        );
        assert_eq!(
            replacement_root.len(),
            provenance.replacement_ref().origins().len(),
            "macro replacement token and origin lengths differ"
        );
        let meaning = MacroMeaning::new(flags, parameter_root.id(), replacement_root.id());
        let parameter_pattern = MacroParameterPattern::from_tokens(parameter_root.tokens());
        let observation_width =
            u32::try_from(1_usize + parameter_root.len() + replacement_root.len())
                .expect("macro token list length exceeds u32");
        let definition = self.macros.allocate_with_provenance(
            meaning,
            parameter_root.clone(),
            replacement_root.clone(),
            parameter_pattern,
            Some(provenance),
            observation_width,
            None,
        );
        self.install_macro_meaning(symbol, flags, definition, global)
    }

    /// Installs one scanner-completed macro directly into the dense token and
    /// macro arenas. The scanner buffers already own and validate their traced
    /// words, so ordinary publication does not need weak objects or exact
    /// candidate indexes.
    pub fn set_macro_meaning_from_buffers(
        &mut self,
        symbol: impl SymbolReference,
        flags: MeaningFlags,
        parameter_text: &crate::token::RootedTracedTokenBuffer,
        replacement_text: &crate::token::RootedTracedTokenBuffer,
        definition_origin: crate::provenance::OriginRef,
        global: bool,
    ) -> CellMutationReceipt {
        self.assert_live_origin(definition_origin.id());
        self.macros.prepare_runtime_allocation();
        self.tokens.prepare_runtime_allocation();
        let parameter_semantic_id = self.traced_token_list_semantic_id(parameter_text.words());
        let replacement_semantic_id =
            self.traced_token_list_semantic_id(replacement_text.words());
        let (parameter_root, replacement_root) = self.tokens.allocate_traced_pair(
            parameter_text.words(),
            replacement_text.words(),
            [parameter_semantic_id, replacement_semantic_id],
        );
        let parameter_pattern =
            MacroParameterPattern::from_traced_words(parameter_text.words());
        let observation_width = u32::try_from(
            1_usize + parameter_text.len() + replacement_text.len(),
        )
        .expect("macro token list length exceeds u32");
        let definition = self.macros.allocate_packed_with_provenance(
            flags,
            parameter_root,
            replacement_root,
            parameter_pattern,
            definition_origin,
            parameter_text.roots(),
            replacement_text.roots(),
            parameter_text.words(),
            replacement_text.words(),
            observation_width,
        );
        self.install_macro_meaning(symbol, flags, definition, global)
    }

    fn install_macro_meaning(
        &mut self,
        symbol: impl SymbolReference,
        flags: MeaningFlags,
        definition: MacroDefinitionRef,
        global: bool,
    ) -> CellMutationReceipt {
        let symbol = self.resolve_symbol_reference(symbol);
        let meaning = Meaning::Macro {
            flags,
            definition: definition.id(),
        };
        self.env
            .set_meaning_slot_with_macro_root(symbol.raw(), meaning, Some(definition), global)
    }

    /// Decodes a symbol's meaning as a public macro aggregate when applicable.
    #[must_use]
    pub fn macro_meaning(&self, symbol: impl SymbolReference) -> Option<MacroMeaning> {
        match self.meaning(symbol) {
            Meaning::Macro { definition, .. } => Some(self.macro_definition(definition)),
            _ => None,
        }
    }

    /// Interns a control-sequence name in the owned interner.
    pub fn intern(&mut self, name: &str) -> SymbolId {
        self.try_intern(name)
            .expect("control-sequence symbol capacity exceeded")
    }

    /// Interns an active-character control sequence in its TeX82 namespace.
    pub fn intern_active_character(&mut self, ch: char) -> SymbolId {
        // TeX82 §§341/372 use the direct one-character control-sequence
        // namespace without calling `id_lookup` or `make_string`.
        self.interner
            .intern_active(ch)
            .expect("control-sequence symbol capacity exceeded")
    }

    /// Interns an inaccessible engine-owned fixed `eqtb` control sequence.
    pub fn intern_internal_control_sequence(&mut self, name: &str) -> SymbolId {
        let prior_kind = self
            .interner
            .get(name)
            .map(|symbol| self.interner.kind_id(symbol));
        let symbol = self
            .interner
            .intern_internal(name)
            .expect("control-sequence symbol capacity exceeded");
        if prior_kind.is_some_and(|kind| kind != self.interner.kind_id(symbol)) {
            // Canonicalizing an already-live name into the inaccessible
            // namespace changes the semantic key of its Meaning cell and any
            // font identities that use it as `font_id_text`.
            self.initialize_exact_env_identity();
            self.semantic_hash_cache.clear();
        }
        symbol
    }

    pub(crate) fn intern_retained_pool_string(&mut self, value: &str) -> SymbolId {
        let symbol = self
            .interner
            .intern_retained_string(value)
            .expect("control-sequence symbol capacity exceeded");
        self.make_pool_string(value);
        symbol
    }

    pub(crate) fn record_pool_strings(&mut self, strings: usize, characters: usize) {
        self.string_pool.allocate(strings, characters);
    }

    pub(crate) fn make_pool_string(&mut self, value: &str) {
        let retained = self.string_pool.make_string(value);
        self.record_recycled_string(retained);
    }

    pub(crate) fn slow_make_pool_string(&mut self, value: &str) {
        let retained = self.string_pool.slow_make_string(value);
        self.record_recycled_string(retained);
    }

    pub(crate) fn remember_pool_string(&mut self, value: &str) {
        let retained = self.string_pool.remember_string(value);
        self.record_recycled_string(retained);
    }

    fn record_recycled_string(&mut self, retained: Option<Arc<str>>) {
        if let Some(retained) = retained {
            self.string_pool_recycled_journal.push(retained);
        }
    }

    pub(crate) fn flush_pool_strings(&mut self, strings: usize, characters: usize) {
        self.string_pool.flush_last(strings, characters);
    }

    pub(crate) fn string_pool_accounting(&self) -> StringPoolAccounting {
        self.string_pool.clone()
    }

    pub(crate) fn mark_string_pool_format_baseline(&mut self) -> Result<(), StoreFormatError> {
        let usage = format::main_memory_usage(self, None)?;
        let variable_usage = usage.variable;
        let low_extent = variable_usage
            .saturating_sub(TEX82_STATIC_LOW_MEMORY_WORDS)
            .max(1)
            .div_ceil(TEX82_LOW_MEMORY_GROWTH)
            .saturating_mul(TEX82_LOW_MEMORY_GROWTH)
            .saturating_add(TEX82_STATIC_LOW_MEMORY_WORDS);
        self.memory_low_extent = self.memory_low_extent.max(low_extent);
        self.memory_high_extent = self.memory_high_extent.max(usage.dynamic_extent);
        self.string_pool
            .mark_format_baseline(self.memory_low_extent, self.memory_high_extent);
        Ok(())
    }

    pub(crate) fn restore_string_pool_accounting(&mut self, mut accounting: StringPoolAccounting) {
        accounting.recycling_enabled = true;
        (self.memory_low_extent, self.memory_high_extent) = accounting.memory_extents();
        self.string_pool = accounting;
        self.string_pool_recycled_journal.clear();
    }

    pub(crate) fn select_string_pool_profile(&mut self, profile: StringPoolProfile) {
        self.string_pool.select_profile(profile);
        self.main_memory_profile = profile;
        let projection_was_live = self.transient_memory_base.take().is_some();
        #[cfg(feature = "profiling")]
        if projection_was_live {
            crate::measurement::record_main_memory_cache_loss(
                crate::measurement::MainMemoryProjectionLossOwner::ProfileChange,
            );
        }
        #[cfg(not(feature = "profiling"))]
        let _ = projection_was_live;
    }

    /// Interns a control-sequence name, reporting packed-token capacity exhaustion.
    pub(crate) fn try_intern(&mut self, name: &str) -> Result<SymbolId, InternerError> {
        let before = self.interner.len();
        let symbol = self.interner.intern(name)?;
        // TeX82 §§341/372 select the direct single-character namespace
        // without constructing a pool string.
        if self.interner.len() != before && name.chars().nth(1).is_some() {
            self.make_pool_string(name);
        }
        Ok(symbol)
    }

    /// Interns a spelling through TeX82 §259's hash-table path.
    pub(crate) fn try_intern_hash(&mut self, name: &str) -> Result<SymbolId, InternerError> {
        let before = self.interner.len();
        let symbol = self.interner.intern_hash(name)?;
        // §§356/372 use the preloaded one-character string and its fixed
        // `eqtb` slot; multiletter names allocate through §259's hash.
        if self.interner.len() != before && name.chars().nth(1).is_some() {
            self.make_pool_string(name);
        }
        Ok(symbol)
    }

    /// Returns the live symbol for an already-interned control-sequence name.
    #[must_use]
    pub fn symbol(&self, name: &str) -> Option<SymbolId> {
        self.interner.get(name)
    }

    /// Returns the live symbol for an already-interned active character.
    #[must_use]
    pub fn active_character_symbol(&self, ch: char) -> Option<SymbolId> {
        self.interner.get_active(ch)
    }

    /// Resolves a live control-sequence symbol.
    #[must_use]
    pub fn resolve(&self, symbol: impl SymbolReference) -> &str {
        let symbol = self.resolve_symbol_reference(symbol);
        self.interner.resolve_id(symbol)
    }

    /// Returns the TeX control-sequence namespace of a live symbol.
    #[must_use]
    pub fn control_sequence_kind(&self, symbol: impl SymbolReference) -> ControlSequenceKind {
        let symbol = self.resolve_symbol_reference(symbol);
        self.interner.kind_id(symbol)
    }

    /// Creates a fresh owned scratch token-list builder.
    #[must_use]
    pub fn token_list_builder(&self) -> TokenListBuilder {
        TokenStore::builder()
    }

    /// Interns a frozen token-list value in the owned token store.
    #[cfg(test)]
    pub fn intern_token_list(&mut self, tokens: &[Token]) -> TokenListId {
        self.intern_token_list_in_domain(tokens, None)
    }

    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn intern_token_list_in_domain(
        &mut self,
        tokens: &[Token],
        domain: Option<&mut crate::patch_domain::PatchAllocationDomain>,
    ) -> TokenListId {
        let semantic_id = self.token_list_semantic_id(tokens.iter().copied());
        let frozen_hash = self
            .tokens
            .has_frozen_lists()
            .then(|| self.frozen_token_lookup_hash(tokens.iter().copied()));
        let legacy_key = self
            .tokens
            .requires_legacy_frozen_key()
            .then(|| self.legacy_frozen_token_lookup_key(tokens.iter().copied()));
        self.tokens.testing_intern_with_semantic_id(
            tokens,
            semantic_id,
            frozen_hash.unwrap_or(0),
            legacy_key.as_deref(),
            domain,
        )
    }

    pub(crate) fn intern_token_list_ref_in_domain(
        &mut self,
        tokens: &[Token],
        domain: Option<&mut crate::patch_domain::PatchAllocationDomain>,
    ) -> TokenListRef {
        let semantic_id = self.token_list_semantic_id(tokens.iter().copied());
        let frozen_hash = self
            .tokens
            .has_frozen_lists()
            .then(|| self.frozen_token_lookup_hash(tokens.iter().copied()));
        let legacy_key = self
            .tokens
            .requires_legacy_frozen_key()
            .then(|| self.legacy_frozen_token_lookup_key(tokens.iter().copied()));
        self.tokens.intern_owned_with_semantic_identity(
            tokens,
            semantic_id,
            frozen_hash.unwrap_or(0),
            legacy_key.as_deref(),
            domain,
        )
    }

    /// Interns the current token-list builder value and clears it for reuse.
    #[cfg(test)]
    pub fn finish_token_list(&mut self, builder: &mut TokenListBuilder) -> TokenListId {
        self.finish_token_list_in_domain(builder, None)
    }

    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn finish_token_list_in_domain(
        &mut self,
        builder: &mut TokenListBuilder,
        domain: Option<&mut crate::patch_domain::PatchAllocationDomain>,
    ) -> TokenListId {
        let semantic_id = self.token_list_semantic_id(builder.as_slice().iter().copied());
        let frozen_hash = self
            .tokens
            .has_frozen_lists()
            .then(|| self.frozen_token_lookup_hash(builder.as_slice().iter().copied()));
        let legacy_key = self
            .tokens
            .requires_legacy_frozen_key()
            .then(|| self.legacy_frozen_token_lookup_key(builder.as_slice().iter().copied()));
        let id = self.tokens.testing_intern_with_semantic_id(
            builder.as_slice(),
            semantic_id,
            frozen_hash.unwrap_or(0),
            legacy_key.as_deref(),
            domain,
        );
        builder.clear();
        id
    }

    pub(crate) fn finish_traced_token_list_in_domain(
        &mut self,
        traced: &[TracedTokenWord],
        domain: Option<&mut crate::patch_domain::PatchAllocationDomain>,
    ) -> TracedTokenList {
        let semantic_id = self.traced_token_list_semantic_id(traced);
        let token_list =
            self.tokens
                .allocate_traced_owned_with_semantic_id(traced, semantic_id, domain);
        #[cfg(feature = "profiling")]
        crate::measurement::record_traced_list_finish(traced.len(), 0, 0);
        let origin_list = self
            .provenance
            .allocate_unrooted_origin_ids(traced.iter().map(|word| word.origin()));
        TracedTokenList::new(token_list, origin_list)
    }

    pub(crate) fn finish_rooted_traced_token_list_in_domain(
        &mut self,
        traced: &crate::token::RootedTracedTokenBuffer,
        domain: Option<&mut crate::patch_domain::PatchAllocationDomain>,
    ) -> TracedTokenList {
        let words = traced.words();
        let semantic_id = self.traced_token_list_semantic_id(words);
        let token_list =
            self.tokens
                .allocate_traced_owned_with_semantic_id(words, semantic_id, domain);
        #[cfg(feature = "profiling")]
        crate::measurement::record_traced_list_finish(words.len(), 0, 0);
        let origin_list = self
            .provenance
            .allocate_rooted_origin_words(words.iter().map(|word| word.origin()), traced.roots());
        TracedTokenList::new(token_list, origin_list)
    }

    pub(crate) fn token_list_ref(&self, id: TokenListId) -> TokenListRef {
        self.tokens
            .resolved_owner(id)
            .expect("token list id is not live")
    }

    /// Reads a live frozen token list.
    #[must_use]
    pub fn tokens(&self, id: TokenListId) -> TokenListRef {
        self.tokens.get(id)
    }

    pub(crate) fn token_list_semantic_id_value(&self, id: TokenListId) -> u64 {
        self.tokens.semantic_id(id).value()
    }

    pub(crate) fn token_list_semantic_fragment(&self, id: TokenListId) -> StateHashFragment {
        self.tokens.semantic_id(id).fragment()
    }

    #[cfg(test)]
    pub(crate) fn testing_token_semantic_id(&self, id: TokenListId) -> TokenSemanticId {
        self.tokens.semantic_id(id)
    }

    pub(crate) fn selected_patch_roots(
        &self,
        domain: &crate::patch_domain::PatchAllocationDomain,
    ) -> Vec<crate::patch_domain::PatchRoot> {
        let mut roots = self.tokens.selected_patch_roots(domain);
        roots.extend(self.macros.selected_patch_roots(domain));
        roots.extend(self.glue.selected_patch_roots(domain));
        roots
    }

    pub(crate) fn patch_allocation_count(&self) -> usize {
        self.tokens
            .patch_allocation_count()
            .saturating_add(self.macros.patch_allocation_count())
            .saturating_add(self.glue.patch_allocation_count())
    }

    pub(crate) fn clear_patch_allocations(&mut self) {
        self.tokens.clear_patch_allocations();
        self.macros.clear_patch_allocations();
        self.glue.clear_patch_allocations();
    }

    pub(crate) fn begin_patch_operation(&self) -> StorePatchOperationMark {
        StorePatchOperationMark {
            tokens: self.tokens.watermark(),
            macros: self.macros.watermark(),
            glue: self.glue.watermark(),
        }
    }

    pub(crate) fn discard_patch_operation_allocations(&mut self, mark: StorePatchOperationMark) {
        self.tokens.truncate_to(mark.tokens);
        self.macros.truncate_to(mark.macros);
        self.glue.truncate_to(mark.glue);
    }

    fn token_list_semantic_id(&self, tokens: impl IntoIterator<Item = Token>) -> TokenSemanticId {
        let mut identity = TokenSemanticIdBuilder::new();
        let mut cached_symbol = None;
        for token in tokens {
            let atom = match token {
                Token::Cs(symbol) => {
                    let atom = cached_symbol
                        .filter(|(cached, _)| *cached == symbol)
                        .map_or_else(
                            || {
                                let atom = self
                                    .interner
                                    .semantic_atom_identity(symbol)
                                    .expect("symbol is not live in this Universe timeline");
                                cached_symbol = Some((symbol, atom));
                                atom
                            },
                            |(_, atom)| atom,
                        );
                    Some(atom)
                }
                _ => None,
            };
            identity.push(token, atom);
        }
        identity.finish()
    }

    fn traced_token_list_semantic_id(&self, traced: &[TracedTokenWord]) -> TokenSemanticId {
        let mut identity = TokenSemanticIdBuilder::new();
        let mut cached_symbol = None;
        let mut validated_origin = None;
        for &word in traced {
            let token = word
                .token()
                .expect("traced token list contains an invalid semantic token");
            let origin = word.origin();
            if validated_origin != Some(origin) {
                self.assert_live_origin(origin);
                validated_origin = Some(origin);
            }
            let atom = match token {
                Token::Cs(symbol) => {
                    let atom = cached_symbol
                        .filter(|(cached, _)| *cached == symbol)
                        .map_or_else(
                            || {
                                let atom = self
                                    .interner
                                    .semantic_atom_identity(symbol)
                                    .expect("symbol is not live in this Universe timeline");
                                cached_symbol = Some((symbol, atom));
                                atom
                            },
                            |(_, atom)| atom,
                        );
                    Some(atom)
                }
                _ => None,
            };
            identity.push(token, atom);
        }
        identity.finish()
    }

    fn frozen_token_lookup_hash(&self, tokens: impl IntoIterator<Item = Token>) -> u64 {
        let mut hash = crate::frozen_lookup::FrozenWordHasher::new();
        for token in tokens {
            hash.push_u32(self.frozen_token_word(token));
        }
        hash.finish()
    }

    fn frozen_token_word(&self, token: Token) -> u32 {
        const CS_TAG: u32 = 1 << 30;
        const PARAM_TAG: u32 = 2 << 30;
        const FROZEN_TAG: u32 = 3 << 30;
        match token {
            Token::Char { ch, cat } => u32::from(cat as u8) << 21 | ch as u32,
            Token::Cs(symbol) => {
                let slot = self
                    .interner
                    .resolve_stored(symbol)
                    .expect("token symbol is live")
                    .raw();
                assert!(slot < CS_TAG, "frozen token symbol exceeds 30 bits");
                CS_TAG | slot
            }
            Token::Param(slot) => PARAM_TAG | u32::from(slot),
            Token::Frozen(frozen) => FROZEN_TAG | u32::from(frozen.raw()),
        }
    }

    fn legacy_frozen_token_lookup_key(&self, tokens: impl IntoIterator<Item = Token>) -> Vec<u8> {
        let mut key = Vec::new();
        for token in tokens {
            let word = match token {
                Token::Char { ch, cat } => u64::from(ch as u32) | (u64::from(cat as u8) << 32),
                Token::Cs(symbol) => {
                    let slot = self
                        .interner
                        .resolve_stored(symbol)
                        .expect("token symbol is live")
                        .raw();
                    (1_u64 << 56) | u64::from(slot)
                }
                Token::Param(slot) => (2_u64 << 56) | u64::from(slot),
                Token::Frozen(frozen) => (3_u64 << 56) | u64::from(frozen.raw()),
            };
            key.extend_from_slice(&word.to_le_bytes());
        }
        key
    }

    /// Returns the reserved unknown/bootstrap provenance origin.
    #[must_use]
    pub fn bootstrap_origin(&self) -> OriginId {
        ProvenanceStore::unknown_id()
    }

    /// Allocates a source-coordinate origin.
    pub fn source_origin(
        &mut self,
        source: SourceId,
        byte_offset: u64,
        line: u32,
        column: u32,
    ) -> OriginId {
        self.provenance
            .allocate(OriginRecord::Source(SourceOrigin::new(
                source,
                byte_offset,
                line,
                column,
            )))
    }

    pub fn source_origin_with_input_record(
        &mut self,
        source: SourceId,
        input_record: Option<crate::InputRecordId>,
        byte_offset: u64,
        line: u32,
        column: u32,
    ) -> OriginId {
        let mut origin = SourceOrigin::new(source, byte_offset, line, column);
        if let Some(input_record) = input_record {
            origin = origin.with_input_record(input_record);
        }
        self.provenance.allocate(OriginRecord::Source(origin))
    }

    /// Encodes an ordinary one-scalar source delivery directly when possible,
    /// falling back to a validated arena span outside the direct payload.
    pub fn source_token_origin(
        &mut self,
        source: SourceId,
        byte_offset: u64,
        byte_end: u64,
    ) -> OriginId {
        let Ok(span) = self
            .source_map
            .span_for_source_offsets(source, byte_offset, byte_end)
        else {
            return OriginId::UNKNOWN;
        };
        if span.is_empty() {
            return OriginId::UNKNOWN;
        }
        OriginId::direct_source(span.lo())
            .unwrap_or_else(|| self.provenance.allocate(OriginRecord::SourceSpan(span)))
    }

    /// Allocates an exact validated half-open range for a nontrivial physical
    /// spelling. Unlike `source_token_origin`, this always records both ends.
    pub fn source_range_origin(
        &mut self,
        source: SourceId,
        byte_offset: u64,
        byte_end: u64,
    ) -> OriginId {
        let Ok(span) = self
            .source_map
            .span_for_source_offsets(source, byte_offset, byte_end)
        else {
            return OriginId::UNKNOWN;
        };
        self.provenance.allocate(OriginRecord::SourceSpan(span))
    }

    /// Allocates an exact range already validated by a registered-source
    /// capability, avoiding another source-map lookup on the hot path.
    pub fn source_span_origin(&mut self, span: SourceSpan) -> OriginId {
        self.provenance.allocate(OriginRecord::SourceSpan(span))
    }

    pub fn source_span_origin_ref(&mut self, span: SourceSpan) -> OriginRef {
        let registration = self.source_map.registration_for_span(span);
        self.provenance.allocate_rooted_with_registration(
            OriginRecord::SourceSpan(span),
            [],
            registration,
        )
    }

    pub fn source_token_origin_ref(
        &mut self,
        source: SourceId,
        byte_offset: u64,
        byte_end: u64,
    ) -> OriginRef {
        let Ok(span) = self
            .source_map
            .span_for_source_offsets(source, byte_offset, byte_end)
        else {
            return OriginRef::unknown();
        };
        if span.is_empty() {
            return OriginRef::unknown();
        }
        let registration = self.source_map.registration_for_span(span);
        OriginId::direct_source(span.lo()).map_or_else(
            || self.source_span_origin_ref(span),
            |id| {
                registration.map_or_else(
                    || OriginRef::direct(id),
                    |registration| OriginRef::direct_registered(id, registration),
                )
            },
        )
    }

    pub fn source_range_origin_ref(
        &mut self,
        source: SourceId,
        byte_offset: u64,
        byte_end: u64,
    ) -> OriginRef {
        let Ok(span) = self
            .source_map
            .span_for_source_offsets(source, byte_offset, byte_end)
        else {
            return OriginRef::unknown();
        };
        self.source_span_origin_ref(span)
    }

    /// Allocates a macro-invocation origin.
    pub fn macro_invocation_origin(
        &mut self,
        definition: MacroDefinitionId,
        invocation: OriginId,
        definition_origin: OriginId,
        parent_invocation: OriginId,
    ) -> OriginId {
        self.assert_live_macro_definition(definition);
        self.assert_live_origin(invocation);
        self.assert_live_origin(definition_origin);
        self.assert_live_origin(parent_invocation);
        let definition_operand = self.macros.observation_operand(definition) as u64;
        self.provenance
            .allocate_unique(OriginRecord::MacroInvocation(
                MacroInvocationOrigin::from_nonowning_operand(
                    definition_operand,
                    invocation,
                    definition_origin,
                    parent_invocation,
                ),
            ))
    }

    pub fn macro_invocation_origin_from_nonowning_operand(
        &mut self,
        definition_operand: u64,
        invocation: OriginId,
        definition_origin: OriginId,
        parent_invocation: OriginId,
    ) -> OriginId {
        self.assert_live_origin(invocation);
        self.assert_live_origin(definition_origin);
        self.assert_live_origin(parent_invocation);
        self.provenance
            .allocate_unique(OriginRecord::MacroInvocation(
                MacroInvocationOrigin::from_nonowning_operand(
                    definition_operand,
                    invocation,
                    definition_origin,
                    parent_invocation,
                ),
            ))
    }

    pub fn macro_invocation_frame_from_nonowning_operand(
        &mut self,
        definition_operand: u64,
        invocation: OriginRef,
        definition_origin: OriginRef,
        parent_invocation: OriginRef,
    ) -> ExpansionFrameRef {
        let record = OriginRecord::MacroInvocation(MacroInvocationOrigin::from_nonowning_operand(
            definition_operand,
            invocation.id(),
            definition_origin.id(),
            parent_invocation.id(),
        ));
        ExpansionFrameRef(
            self.provenance
                .allocate_rooted(record, [invocation, definition_origin, parent_invocation]),
        )
    }

    pub fn macro_invocation_frame(
        &mut self,
        definition: MacroDefinitionId,
        invocation: OriginRef,
        definition_origin: OriginRef,
        parent_invocation: OriginRef,
    ) -> ExpansionFrameRef {
        self.assert_live_macro_definition(definition);
        let definition_operand = self.macros.observation_operand(definition) as u64;
        self.macro_invocation_frame_from_nonowning_operand(
            definition_operand,
            invocation,
            definition_origin,
            parent_invocation,
        )
    }

    /// Allocates an inserted-token origin.
    pub fn inserted_origin(
        &mut self,
        kind: InsertedOriginKind,
        token: Token,
        parent: OriginId,
    ) -> OriginId {
        self.assert_live_token(token);
        self.assert_live_origin(parent);
        self.provenance
            .allocate(OriginRecord::Inserted(InsertedOrigin::new(
                kind, token, parent,
            )))
    }

    pub fn inserted_origin_ref(
        &mut self,
        kind: InsertedOriginKind,
        token: Token,
        parent: OriginRef,
    ) -> OriginRef {
        self.assert_live_token(token);
        self.provenance.allocate_rooted(
            OriginRecord::Inserted(InsertedOrigin::new(kind, token, parent.id())),
            [parent],
        )
    }

    /// Allocates a synthesized-token origin.
    pub fn synthesized_origin(
        &mut self,
        kind: SynthesizedOriginKind,
        parent: OriginId,
    ) -> OriginId {
        self.assert_live_origin(parent);
        self.provenance
            .allocate(OriginRecord::Synthesized(SynthesizedOrigin::new(
                kind, parent,
            )))
    }

    pub fn synthesized_origin_ref(
        &mut self,
        kind: SynthesizedOriginKind,
        parent: OriginRef,
    ) -> OriginRef {
        self.provenance.allocate_rooted(
            OriginRecord::Synthesized(SynthesizedOrigin::new(kind, parent.id())),
            [parent],
        )
    }

    /// Allocates a synthetic/bootstrap origin.
    pub fn synthetic_origin(&mut self, kind: SyntheticOriginKind) -> OriginId {
        match kind {
            SyntheticOriginKind::Bootstrap => ProvenanceStore::unknown_id(),
            _ => self
                .provenance
                .allocate(OriginRecord::Synthetic(SyntheticOrigin::new(kind))),
        }
    }

    pub fn synthetic_origin_ref(&mut self, kind: SyntheticOriginKind) -> OriginRef {
        match kind {
            SyntheticOriginKind::Bootstrap => OriginRef::unknown(),
            _ => self
                .provenance
                .allocate_rooted(OriginRecord::Synthetic(SyntheticOrigin::new(kind)), []),
        }
    }

    pub fn origin_ref(&self, id: OriginId) -> Option<OriginRef> {
        self.provenance.origin_ref(id)
    }

    pub fn materialize_origin_ref(&mut self, id: OriginId) -> Option<OriginRef> {
        self.provenance.materialize_origin_ref(id, &self.source_map)
    }

    /// Reads a live origin record.
    #[cfg(test)]
    #[must_use]
    pub fn origin(&self, id: OriginId) -> OriginRecord {
        self.assert_live_origin(id);
        match id.decode() {
            crate::token::OriginEncoding::DirectSource(position) => {
                OriginRecord::SourceSpan(self.direct_source_span(position))
            }
            crate::token::OriginEncoding::NoExpandFallback => OriginRecord::UnknownBootstrap,
            crate::token::OriginEncoding::Unknown | crate::token::OriginEncoding::Arena(_) => {
                self.provenance.get(id)
            }
        }
    }

    /// Reads an origin record if it is still live on this timeline.
    #[must_use]
    pub fn origin_if_live(&self, id: OriginId) -> Option<OriginRecord> {
        match id.decode() {
            crate::token::OriginEncoding::DirectSource(position) => self
                .source_map
                .region_for_backed_position(position)
                .map(|_| OriginRecord::SourceSpan(self.direct_source_span(position))),
            crate::token::OriginEncoding::NoExpandFallback => Some(OriginRecord::UnknownBootstrap),
            crate::token::OriginEncoding::Unknown | crate::token::OriginEncoding::Arena(_) => self
                .provenance
                .contains_origin(id)
                .then(|| self.provenance.get(id)),
        }
    }

    pub fn allocate_origin_list_ref(&mut self, origins: &[OriginRef]) -> OriginListRef {
        self.provenance.allocate_rooted_list(origins)
    }

    /// Returns live provenance arena length counters.
    #[must_use]
    pub fn provenance_stats(&self) -> ProvenanceStats {
        self.provenance
            .stats()
            .with_source_map(self.source_map.stats())
    }

    pub(crate) fn macro_invocation_provenance_stats(
        &self,
    ) -> crate::provenance::MacroInvocationProvenanceStats {
        self.provenance.macro_invocation_stats()
    }

    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn macro_invocation_origins(&self) -> Vec<OriginId> {
        self.provenance.macro_invocation_origins()
    }

    /// Registers immutable source backing on this aggregate timeline.
    pub(crate) fn register_source(
        &mut self,
        source: SourceId,
        descriptor: SourceDescriptor,
        line_starts: std::sync::Arc<[usize]>,
    ) -> Result<SourcePos, SourceMapError> {
        let generated = match &descriptor {
            SourceDescriptor::Generated(generated) => Some(generated.clone()),
            SourceDescriptor::World { .. } => None,
        };
        let byte_len = descriptor.byte_len();
        let start = self
            .source_map
            .register_with_line_starts(source, descriptor, line_starts)?;
        if let Some(generated) = generated {
            self.source_fragments.bind_generated_root_registration(
                crate::source_map::RegisteredSource::new(start, byte_len),
                &generated,
            );
        }
        Ok(start)
    }

    pub(crate) fn existing_source_registration(
        &self,
        source: SourceId,
        descriptor: &SourceDescriptor,
    ) -> Result<Option<SourcePos>, SourceMapError> {
        self.source_map.existing_registration(source, descriptor)
    }

    /// Assigns one local byte offset in a live source to logical source space.
    pub(crate) fn source_position(
        &self,
        source: SourceId,
        byte_offset: u64,
    ) -> Result<SourcePos, SourceMapError> {
        self.source_map.position(source, byte_offset)
    }

    /// Validates a half-open span against the region containing its low endpoint.
    pub(crate) fn source_span(
        &self,
        lo: SourcePos,
        hi: SourcePos,
    ) -> Result<SourceSpan, SourceMapError> {
        self.source_map.span(lo, hi)
    }

    pub(crate) fn source_region(&self, source: SourceId) -> Option<SourceRegion> {
        self.source_map.region_for_source(source)
    }

    pub(crate) fn source_region_at_position(&self, position: SourcePos) -> Option<SourceRegion> {
        self.source_map.region_for_position(position)
    }

    pub(crate) fn source_descriptor(&self, source: SourceId) -> Option<SourceDescriptor> {
        self.source_map.descriptor_for_source(source)
    }

    pub(crate) fn source_line_starts(&self, region: SourceRegion) -> Option<&[usize]> {
        self.source_map.line_starts(region)
    }

    pub(crate) fn direct_source_origin(&self, id: OriginId) -> Option<SourceOrigin> {
        let crate::token::OriginEncoding::DirectSource(position) = id.decode() else {
            return None;
        };
        self.source_origin_at_position(position)
    }

    pub(crate) fn source_origin_at_position(&self, position: SourcePos) -> Option<SourceOrigin> {
        let region = self.source_map.region_for_backed_position(position)?;
        let byte_offset = position.raw().checked_sub(region.start.raw())?;
        let mut source = SourceOrigin::new(region.source, byte_offset, 0, 0);
        if let SourceBacking::World(record) = region.backing {
            source = source.with_input_record(record);
        }
        Some(source)
    }

    /// Resolves a direct origin minted by the installed editor fragment store.
    pub(crate) fn direct_fragment_origin_span(&self, origin: OriginId) -> Option<SourceSpan> {
        direct_fragment_span(origin, &self.source_fragments)
    }

    fn direct_source_span(&self, position: SourcePos) -> SourceSpan {
        let hi = SourcePos::from_raw_for_store(position.raw() + 1);
        self.source_map
            .span(position, hi)
            .expect("live direct source position must admit one backed byte")
    }

    pub(crate) fn generated_source(&self, backing: SourceBacking) -> Option<&GeneratedSource> {
        match backing {
            SourceBacking::Generated(id) => self.source_map.generated(id),
            SourceBacking::World(_) => None,
        }
    }

    pub(crate) fn root_generated_content_hash(
        &self,
        summary: &crate::input::InputSummary,
    ) -> Option<crate::world::ContentHash> {
        let source_id = summary.frames().iter().find_map(|frame| match frame {
            crate::input::InputFrameSummary::Source { source_id, .. } => Some(*source_id),
            crate::input::InputFrameSummary::TokenList { .. }
            | crate::input::InputFrameSummary::TransientTokenList { .. }
            | crate::input::InputFrameSummary::Condition { .. } => None,
        })?;
        let region = self.source_region(source_id)?;
        self.generated_source(region.backing)
            .map(GeneratedSource::hash)
    }

    fn assert_origin_list_len_matches(&self, token_list: TokenListId, origins: &OriginListRef) {
        if origins.id() == OriginListId::EMPTY {
            return;
        }
        assert_eq!(
            self.tokens(token_list).len(),
            origins.origins().len(),
            "origin-list length does not match token-list length"
        );
    }

    /// Interns a glue specification and returns its strong exact-content owner.
    pub fn intern_glue_in_domain(
        &mut self,
        spec: GlueSpec,
        domain: Option<&mut crate::patch_domain::PatchAllocationDomain>,
    ) -> GlueSpecRef {
        self.glue.intern_owned(spec, domain)
    }

    #[cfg(any(test, feature = "testing"))]
    #[allow(dead_code)]
    pub(crate) fn intern_glue(&mut self, spec: GlueSpec) -> GlueId {
        self.glue.testing_intern(spec)
    }

    #[cfg(test)]
    pub(crate) fn testing_glue_live_totals(&self) -> (usize, usize) {
        self.glue.testing_live_totals()
    }

    pub(crate) fn glue_ref(&self, id: GlueId) -> GlueSpecRef {
        let id = self.resolve_stored_glue(id);
        self.glue.owner(id).expect("glue id is not live")
    }

    /// Reads a live frozen glue specification.
    #[must_use]
    pub fn glue(&self, id: GlueId) -> GlueSpec {
        self.glue
            .resolve_get(id)
            .expect("stored glue slot is not live")
    }

    /// Interns a loaded immutable font and initializes its Env-side banks.
    pub fn try_intern_font(&mut self, font: LoadedFont) -> Result<FontId, FontParameterError> {
        let parameter_len = font.parameters().len();
        let parameter_count = u32::try_from(parameter_len)
            .ok()
            .filter(|&count| count <= crate::font::MAX_FONT_DIMEN)
            .ok_or(FontParameterError::ParameterCountOutOfRange {
                count: parameter_len,
                maximum: crate::font::MAX_FONT_DIMEN,
            })?;
        if self.fonts.would_allocate(&font)
            && font.font_info_words()
                > self
                    .font_info_capacity
                    .saturating_sub(self.font_info_words())
        {
            return Err(FontParameterError::FontInfoCapacity {
                capacity: self.font_info_capacity,
            });
        }
        let parameters = font.parameters().to_vec();
        let id = self
            .fonts
            .intern(font)
            .map_err(|_| FontParameterError::TooManyFonts {
                maximum: crate::font::MAX_FONT_DIMEN_FONT_ID,
            })?;
        if self.env.font_param_len(id) == 0 && id != NULL_FONT {
            self.initialize_font_banks(id, parameter_count, &parameters);
        }
        self.last_loaded_font = id;
        Ok(id)
    }

    pub(crate) fn font_would_allocate(&self, font: &LoadedFont) -> bool {
        self.fonts.would_allocate(font)
    }

    /// Interns a font for callers that construct bounded in-memory fonts.
    /// Runtime loading should use [`Self::try_intern_font`] for recovery.
    pub fn intern_font(&mut self, font: LoadedFont) -> FontId {
        self.try_intern_font(font)
            .expect("loaded font exceeds the fontdimen cell domain")
    }

    /// Interns a font and records the control sequence TeX uses for its
    /// identifier token (the `font_id_text` associated with the font).
    pub fn try_intern_font_with_identifier(
        &mut self,
        font: LoadedFont,
        symbol: impl SymbolReference,
    ) -> Result<FontId, FontParameterError> {
        let symbol = self.resolve_symbol_reference(symbol);
        let id = self.try_intern_font(font)?;
        self.set_resolved_font_identifier(id, symbol);
        Ok(id)
    }

    /// Creates a distinct pdfTeX copied-font instance and initializes its
    /// mutable banks from the source font's current values.
    pub fn try_copy_font_with_identifier(
        &mut self,
        source: FontId,
        symbol: impl SymbolReference,
    ) -> Result<FontId, FontParameterError> {
        self.assert_live_font(source);
        let parameter_count = self.font_parameter_count(source);
        let parameters = (1..=parameter_count)
            .map(|number| self.font_parameter(source, number))
            .collect();
        let font = self.font(source).copied(parameters);
        let hyphen_char = self.font_hyphen_char(source);
        let skew_char = self.font_skew_char(source);
        let id = self.try_intern_font_with_identifier(font, symbol)?;
        self.env.set_font_hyphen_char_global(id, hyphen_char);
        self.env.set_font_skew_char_global(id, skew_char);
        Ok(id)
    }

    /// Creates a distinct host-neutral letterspaced font instance.
    pub fn try_letterspace_font_with_identifier(
        &mut self,
        source: FontId,
        symbol: impl SymbolReference,
        amount: i16,
        no_ligatures: bool,
    ) -> Result<FontId, FontParameterError> {
        self.assert_live_font(source);
        let current_quad = self.font_parameter(source, 6);
        let font = self
            .font(source)
            .letterspaced(current_quad, amount, no_ligatures)
            .expect("bounded live TeX font widths support letterspacing");
        let id = self.try_intern_font_with_identifier(font, symbol)?;
        if no_ligatures {
            self.env.set_pdf_no_ligatures_global(id);
        }
        Ok(id)
    }

    pub fn configure_font_expansion(
        &mut self,
        font: FontId,
        expansion: FontExpansion,
    ) -> Result<(), FontExpansionConfigError> {
        self.assert_live_font(font);
        if self.fonts.set_expansion(font, expansion)? {
            self.initialize_exact_env_identity();
            self.semantic_hash_cache.clear();
        }
        Ok(())
    }

    #[must_use]
    pub fn font_expansion(&self, font: FontId) -> Option<FontExpansion> {
        let font = self.resolve_stored_font(font);
        self.fonts.expansion(font)
    }

    pub fn try_expanded_font(
        &mut self,
        source: FontId,
        ratio: i16,
    ) -> Result<FontId, FontParameterError> {
        self.assert_live_font(source);
        if ratio == 0 {
            return Ok(source);
        }
        let generated = self.font(source).expanded(ratio);
        if let Some(existing) = self.font_by_source_identity(generated.source_identity()) {
            return Ok(existing);
        }
        let hyphen_char = self.font_hyphen_char(source);
        let skew_char = self.font_skew_char(source);
        let mut codes = Vec::with_capacity(9 * 256);
        for table in [
            PdfFontCode::Lp,
            PdfFontCode::Rp,
            PdfFontCode::Ef,
            PdfFontCode::Tag,
            PdfFontCode::Knbs,
            PdfFontCode::Stbs,
            PdfFontCode::Shbs,
            PdfFontCode::Knbc,
            PdfFontCode::Knac,
        ] {
            for code in u8::MIN..=u8::MAX {
                codes.push((table, code, self.pdf_font_code(table, source, code)));
            }
        }
        let id = self.try_intern_font(generated)?;
        self.env.set_font_hyphen_char_global(id, hyphen_char);
        self.env.set_font_skew_char_global(id, skew_char);
        for (table, code, value) in codes {
            self.env
                .set_pdf_font_code_global(pdf_font_code_bank(table), id, code, value);
        }
        Ok(id)
    }

    pub fn intern_font_with_identifier(
        &mut self,
        font: LoadedFont,
        symbol: impl SymbolReference,
    ) -> FontId {
        self.try_intern_font_with_identifier(font, symbol)
            .expect("loaded font exceeds the fontdimen cell domain")
    }

    /// Reads a live immutable font record.
    #[must_use]
    pub fn font(&self, id: FontId) -> &LoadedFont {
        let id = self.resolve_stored_font(id);
        self.fonts.get(id)
    }

    #[must_use]
    pub fn font_by_source_identity(&self, identity: FontSourceIdentity) -> Option<FontId> {
        self.fonts.by_source_identity(identity)
    }

    #[must_use]
    pub fn font_name(&self, id: FontId) -> String {
        self.font(id).fontname_text()
    }

    #[must_use]
    pub fn font_identifier_symbol(&self, id: FontId) -> Option<SymbolId> {
        let id = self.resolve_stored_font(id);
        let symbol = self.fonts.identifier(id)?;
        self.assert_live_symbol(symbol);
        Some(symbol)
    }

    pub fn set_font_identifier_symbol(&mut self, id: FontId, symbol: impl SymbolReference) {
        self.assert_live_font(id);
        let symbol = self.resolve_symbol_reference(symbol);
        self.set_resolved_font_identifier(id, symbol);
    }

    fn set_resolved_font_identifier(&mut self, id: FontId, symbol: SymbolId) {
        self.assert_live_font(id);
        self.assert_live_symbol(symbol);
        let immutable = *self.fonts.hash_fragment(id);
        let complete = complete_font_hash_fragment(
            immutable,
            Some((
                self.interner.kind_id(symbol),
                self.interner.resolve_id(symbol),
            )),
        );
        if self.fonts.set_identifier(id, symbol, complete) {
            // Font-bank cells are keyed by the font's allocation-independent
            // complete identity. TeX82 §1257 may assign or replace
            // `font_id_text` without writing any of those Env words, so no
            // journal receipt can remove their former semantic keys. The
            // identifier mutation owns that rekey: rebuild the persistent Env
            // projection from live words and discard derived cell-key caches.
            self.initialize_exact_env_identity();
            self.semantic_hash_cache.clear();
        }
    }

    #[must_use]
    pub fn font_metrics(&self, font: FontId) -> &FontMetrics {
        self.font(font).metrics()
    }

    #[must_use]
    pub fn font_char_exists(&self, font: FontId, code: u8) -> bool {
        self.font(font).character_exists(char::from(code))
    }

    #[must_use]
    pub fn font_char_metrics(&self, font: FontId, code: u8) -> Option<CharMetrics> {
        self.font(font).character_metrics(char::from(code))
    }

    #[must_use]
    pub fn font_character_exists(&self, font: FontId, ch: char) -> bool {
        self.font(font).character_exists(ch)
    }

    #[must_use]
    pub fn font_character_metrics(&self, font: FontId, ch: char) -> Option<CharMetrics> {
        self.font(font).character_metrics(ch)
    }

    #[must_use]
    pub fn font_uses_tfm_metrics(&self, font: FontId) -> bool {
        self.font(font).uses_tfm_metrics()
    }

    #[must_use]
    pub fn font_widths(&self, font: FontId) -> &[Scaled; 256] {
        self.font(font).metrics().widths()
    }

    #[must_use]
    pub fn font_characters(&self, font: FontId) -> &[Option<CharMetrics>] {
        self.font(font).metrics().characters()
    }

    #[must_use]
    pub fn font_next_larger(&self, font: FontId, code: u8) -> Option<u8> {
        if self.pdf_font_code(PdfFontCode::Tag, font, code) & 2 == 0 {
            return None;
        }
        self.font(font).metrics().next_larger(code)
    }

    #[must_use]
    pub fn missing_font_character(&self, font: FontId, code: u8) -> Option<MissingCharacter> {
        (!self.font_char_exists(font, code)).then_some(MissingCharacter { font, code })
    }

    #[must_use]
    pub fn lig_kern_iter(
        &self,
        font: FontId,
        left: LigKernChar,
        right: LigKernChar,
    ) -> LigKernIter<'_> {
        self.font(font).metrics().lig_kern_iter(left, right)
    }

    #[must_use]
    pub fn lig_kern_command(
        &self,
        font: FontId,
        left: LigKernChar,
        right: LigKernChar,
    ) -> Option<LigKernCommand> {
        let loaded = self.font(font);
        self.lig_kern_command_with_loaded(font, loaded, left, right)
    }

    #[must_use]
    pub fn tfm_lig_kern_command(
        &self,
        font: FontId,
        left: LigKernChar,
        right: LigKernChar,
    ) -> Option<LigKernCommand> {
        let loaded = self.font(font);
        if !loaded.uses_tfm_metrics() {
            return None;
        }
        self.lig_kern_command_with_loaded(font, loaded, left, right)
    }

    #[must_use]
    pub fn font_false_boundary_char(&self, font: FontId) -> Option<u8> {
        self.font(font).metrics().false_boundary_char()
    }

    fn lig_kern_command_with_loaded(
        &self,
        font: FontId,
        loaded: &LoadedFont,
        left: LigKernChar,
        right: LigKernChar,
    ) -> Option<LigKernCommand> {
        let metrics = loaded.metrics();
        let start = metrics.lig_kern_start(left)?;
        if let LigKernChar::Char(code) = left {
            let tag = self
                .env
                .pdf_font_code(pdf_font_code_bank(PdfFontCode::Tag), font, code)
                .unwrap_or(1);
            if tag & 1 == 0 {
                return None;
            }
        }
        let command = metrics.lig_kern_command_from_start(start, right);
        if self.env.pdf_no_ligatures(font) {
            return command.filter(|command| matches!(command, LigKernCommand::Kern(_)));
        }
        command
    }

    #[must_use]
    pub fn pdf_font_code(&self, table: PdfFontCode, font: FontId, code: u8) -> i32 {
        self.pdf_font_code_with_loaded(table, font, code, self.font(font))
    }

    fn pdf_font_code_with_loaded(
        &self,
        table: PdfFontCode,
        font: FontId,
        code: u8,
        loaded: &LoadedFont,
    ) -> i32 {
        let bank = pdf_font_code_bank(table);
        self.env
            .pdf_font_code(bank, font, code)
            .unwrap_or_else(|| match table {
                PdfFontCode::Ef => 1000,
                PdfFontCode::Tag => {
                    loaded
                        .character_metrics(char::from(code))
                        .map_or(0, |metrics| match metrics.tag {
                            CharTag::None => 0,
                            CharTag::LigKern { .. } => 1,
                            CharTag::NextLarger(_) => 2,
                            CharTag::Extensible(_) => 4,
                        })
                }
                _ => 0,
            })
    }

    pub fn set_pdf_font_code(
        &mut self,
        table: PdfFontCode,
        font: FontId,
        code: u8,
        value: i32,
    ) -> crate::env::CellMutationReceipt {
        self.assert_live_font(font);
        let value = match table {
            PdfFontCode::Lp
            | PdfFontCode::Rp
            | PdfFontCode::Knbs
            | PdfFontCode::Stbs
            | PdfFontCode::Shbs
            | PdfFontCode::Knbc
            | PdfFontCode::Knac => value.clamp(-1000, 1000),
            PdfFontCode::Ef => value.clamp(0, 1000),
            PdfFontCode::Tag => {
                let current = self.pdf_font_code(table, font, code);
                if value >= 0 {
                    current
                } else {
                    current & !(-value).min(7)
                }
            }
        };
        self.env
            .set_pdf_font_code_global(pdf_font_code_bank(table), font, code, value)
    }

    pub fn disable_pdf_font_ligatures(&mut self, font: FontId) -> crate::env::CellMutationReceipt {
        self.assert_live_font(font);
        self.env.set_pdf_no_ligatures_global(font)
    }

    #[must_use]
    pub fn pdf_font_ligatures_disabled(&self, font: FontId) -> bool {
        self.assert_live_font(font);
        self.env.pdf_no_ligatures(font)
    }

    #[must_use]
    pub fn extensible_recipe(&self, font: FontId, code: u8) -> Option<ExtensibleRecipe> {
        if self.pdf_font_code(PdfFontCode::Tag, font, code) & 4 == 0 {
            return None;
        }
        self.font(font).metrics().extensible_recipe(code)
    }

    #[must_use]
    pub fn font_parameter(&self, font: FontId, number: u32) -> Scaled {
        self.font_dimen(font, number)
    }

    #[must_use]
    pub fn current_font(&self) -> FontId {
        self.resolve_stored_font(self.env.current_font())
    }

    #[must_use]
    pub fn current_font_symbol(&self) -> Option<SymbolId> {
        self.interner
            .resolve_stored(self.env.current_font_symbol()?)
    }

    pub fn set_current_font(&mut self, id: FontId) -> crate::env::CellMutationReceipt {
        self.assert_live_font(id);
        self.env.set_current_font(id)
    }

    pub fn set_current_font_global(&mut self, id: FontId) -> crate::env::CellMutationReceipt {
        self.assert_live_font(id);
        self.env.set_current_font_global(id)
    }

    pub fn set_current_font_selector(
        &mut self,
        symbol: impl SymbolReference,
        id: FontId,
    ) -> crate::env::CellMutationReceipt {
        let symbol = self.resolve_symbol_reference(symbol);
        self.assert_live_font(id);
        self.env.set_current_font_selector(symbol.symbol(), id)
    }

    pub fn set_current_font_selector_global(
        &mut self,
        symbol: impl SymbolReference,
        id: FontId,
    ) -> crate::env::CellMutationReceipt {
        let symbol = self.resolve_symbol_reference(symbol);
        self.assert_live_font(id);
        self.env
            .set_current_font_selector_global(symbol.symbol(), id)
    }

    #[must_use]
    pub fn math_family_font(&self, size: MathFontSize, family: u8) -> FontId {
        self.resolve_stored_font(self.env.math_family_font(size, family))
    }

    pub fn set_math_family_font(
        &mut self,
        size: MathFontSize,
        family: u8,
        id: FontId,
        global: bool,
    ) -> crate::env::CellMutationReceipt {
        self.assert_live_font(id);
        if global {
            self.env.set_math_family_font_global(size, family, id)
        } else {
            self.env.set_math_family_font(size, family, id)
        }
    }

    #[must_use]
    pub fn font_dimen(&self, font: FontId, number: u32) -> Scaled {
        self.assert_live_font(font);
        self.env.font_dimen(font, number)
    }

    #[must_use]
    pub fn font_parameter_count(&self, font: FontId) -> u32 {
        self.assert_live_font(font);
        self.env.font_param_len(font)
    }

    pub fn set_font_dimen(
        &mut self,
        font: FontId,
        number: u32,
        value: Scaled,
    ) -> Result<smallvec::SmallVec<[crate::env::CellMutationReceipt; 2]>, FontParameterError> {
        let (index, length_receipt) = self.prepare_font_dimen_write(font, number)?;
        let mut receipts = smallvec::SmallVec::new();
        if let Some(receipt) = length_receipt {
            receipts.push(receipt);
        }
        receipts.push(self.env.set_font_dimen_global(index, value));
        Ok(receipts)
    }

    /// Selects the process-owned Web2C `font_mem_size` limit.
    ///
    /// Canonical formats record their occupied extent, not this runtime
    /// configuration. Drivers therefore reapply the engine configuration
    /// after constructing or loading a universe.
    pub(crate) fn configure_font_info_capacity(&mut self, capacity: usize) {
        assert!(capacity <= crate::font::WEB2C_FONT_INFO_CAPACITY);
        // Web2C's `undump_size` raises a smaller runtime setting to the
        // occupied extent recorded by the loaded format.
        self.font_info_capacity = capacity.max(self.font_info_words());
    }

    #[must_use]
    pub fn font_hyphen_char(&self, font: FontId) -> i32 {
        self.assert_live_font(font);
        self.env.font_hyphen_char(font)
    }

    pub fn set_font_hyphen_char(
        &mut self,
        font: FontId,
        value: i32,
    ) -> crate::env::CellMutationReceipt {
        self.assert_live_font(font);
        self.env.set_font_hyphen_char_global(font, value)
    }

    #[must_use]
    pub fn font_skew_char(&self, font: FontId) -> i32 {
        self.assert_live_font(font);
        self.env.font_skew_char(font)
    }

    pub fn set_font_skew_char(
        &mut self,
        font: FontId,
        value: i32,
    ) -> crate::env::CellMutationReceipt {
        self.assert_live_font(font);
        self.env.set_font_skew_char_global(font, value)
    }

    fn initialize_font_banks(&mut self, font: FontId, parameter_count: u32, parameters: &[Scaled]) {
        self.env.set_font_param_len_global(font, parameter_count);
        for (index, value) in parameters.iter().copied().enumerate() {
            let number = u32::try_from(index + 1).expect("font parameter index exceeds u32");
            let index = crate::env::font_dimen_index(font, number)
                .expect("validated loaded font parameters fit the fontdimen key");
            self.env.set_font_dimen_global(index, value);
        }
        self.env
            .set_font_hyphen_char_global(font, self.env.int_param(IntParam::DEFAULT_HYPHEN_CHAR));
        self.env
            .set_font_skew_char_global(font, self.env.int_param(IntParam::DEFAULT_SKEW_CHAR));
    }

    /// TeX82 §578's `find_font_dimen` decision, without §580's growth.
    ///
    /// False is exactly the `cur_val=fmem_ptr` case §579 reports on: a number
    /// outside the addressable range, or one past a font that is not the last
    /// one loaded and so cannot be grown. §578 makes this decision before
    /// §1253 scans `=<dimen>`, so a caller that has to report §579 with the
    /// context of the moment must ask here rather than infer it from a failed
    /// [`Self::set_font_dimen`] afterwards.
    #[must_use]
    pub fn font_dimen_writable(&self, font: FontId, number: u32) -> bool {
        self.assert_live_font(font);
        crate::env::font_dimen_index(font, number).is_ok()
            && (number <= self.env.font_param_len(font) || font == self.last_loaded_font)
    }

    fn prepare_font_dimen_write(
        &mut self,
        font: FontId,
        number: u32,
    ) -> Result<(u32, Option<crate::env::CellMutationReceipt>), FontParameterError> {
        self.assert_live_font(font);
        let index = crate::env::font_dimen_index(font, number)?;
        let current_len = self.env.font_param_len(font);
        let length_receipt = if number > current_len {
            if font != self.last_loaded_font {
                return Err(FontParameterError::CannotGrow {
                    font,
                    number,
                    current_len,
                    last_loaded_font: self.last_loaded_font,
                });
            }
            let growth = (number - current_len) as usize;
            let used = self.font_info_words();
            if growth > self.font_info_capacity.saturating_sub(used) {
                return Err(FontParameterError::FontInfoCapacity {
                    capacity: self.font_info_capacity,
                });
            }
            Some(self.env.set_font_param_len_global(font, number))
        } else {
            None
        };
        Ok((index, length_receipt))
    }

    fn font_info_words(&self) -> usize {
        self.fonts
            .iter()
            .enumerate()
            .map(|(raw, font)| {
                let id = FontId::new(raw as u32);
                font.font_info_words().saturating_add(
                    (self.env.font_param_len(id) as usize).saturating_sub(font.parameters().len()),
                )
            })
            .sum()
    }

    /// Creates a fresh mutable scratch node-list builder.
    #[must_use]
    pub fn node_list_builder(&self) -> NodeListBuilder {
        NodeListBuilder::new_compact()
    }

    /// Freezes a node list as one directly owned immutable compact graph.
    pub fn freeze_node_list(&mut self, nodes: &[Node]) -> NodeListRef {
        self.observe_main_memory_nodes(nodes);
        let mut builder = NodeListBuilder::new();
        builder.reserve(nodes.len());
        for node in nodes {
            builder.push(node.clone());
        }
        self.freeze_node_list_ref(builder)
    }

    /// Freezes an owned decoded node vector and clears it for allocation reuse.
    pub fn freeze_node_list_owned(&mut self, nodes: &mut Vec<Node>) -> NodeListRef {
        self.observe_main_memory_nodes(nodes);
        let mut builder = NodeListBuilder::new();
        builder.reserve(nodes.len());
        for node in nodes.drain(..) {
            builder.push(node);
        }
        self.freeze_node_list_ref(builder)
    }

    /// Freezes the current node-list builder value and clears it for reuse.
    pub fn finish_node_list(&mut self, builder: &mut NodeListBuilder) -> NodeListRef {
        let owned = core::mem::take(builder);
        self.freeze_node_list_ref(owned)
    }

    /// Consumes one operation-local builder and publishes a directly owned,
    /// immutable compact graph. No aggregate destination is changed if
    /// validation fails.
    pub fn freeze_node_list_ref(&mut self, builder: NodeListBuilder) -> NodeListRef {
        let children = builder.direct_children();
        let (semantic_id, needs) = self.validate_and_plan_direct_node_list(&builder, &children);
        let frozen = match builder.into_compact_rows() {
            Ok(rows) => {
                debug_assert!(children.is_empty());
                debug_assert_eq!(needs, crate::node_arena::SidecarNeeds::default());
                NodeListRef::freeze_compact_builder(rows, semantic_id)
            }
            Err(nodes) => NodeListRef::freeze_builder(nodes, children, semantic_id, needs),
        };
        self.node_ref_index.intern(frozen)
    }

    /// Enters a TeX group.
    pub fn enter_group(&mut self) {
        self.code_tables.enter_group();
        self.env.enter_group();
    }

    /// Enters a TeX group with a boundary kind used for mismatch diagnostics.
    pub fn enter_group_with_kind(&mut self, kind: GroupKind) {
        self.code_tables.enter_group();
        self.env.enter_group_with_kind(kind);
    }

    pub fn enter_group_with_kind_at_line(&mut self, kind: GroupKind, entered_line: u32) {
        self.code_tables.enter_group();
        self.env.enter_group_with_kind_at_line(kind, entered_line);
    }

    /// Pushes an `\aftergroup` token for the current group.
    pub fn push_aftergroup(&mut self, payload: Token) {
        self.assert_live_token(payload);
        self.env.push_aftergroup(payload);
    }

    pub fn push_aftergroup_traced(&mut self, payload: crate::token::RootedTracedTokenWord) {
        self.assert_live_token(payload.word().semantic_token());
        self.env.push_aftergroup_traced(payload);
    }

    /// Leaves the innermost TeX group and returns its `\aftergroup` payloads.
    #[must_use]
    #[cfg(test)]
    pub fn leave_group(&mut self) -> Vec<Token> {
        self.leave_group_observing_dependencies()
            .0
            .into_iter()
            .map(|word| word.word().semantic_token())
            .collect()
    }

    pub(crate) fn leave_group_observing_dependencies(&mut self) -> GroupExitObservation {
        let (payloads, _meaning_changed, changed_cells, mut restores) =
            self.env.leave_group_observing_meanings();
        for receipt in &changed_cells {
            let cell = receipt.cell();
            self.update_exact_env_cell(cell, self.env.semantic_word(cell));
        }
        self.mark_exact_env_journal_current();
        self.retire_unrooted_region_values();
        self.capture_box_restore_texts(&mut restores);
        let code_before = self.code_tables.generations();
        let code_restores = self.code_tables.leave_group();
        let code_after = self.code_tables.generations();
        (
            payloads,
            changed_cells,
            code_before,
            code_after,
            restores,
            code_restores,
        )
    }

    pub(crate) fn leave_group_with_kind_observing_dependencies(
        &mut self,
        expected: GroupKind,
    ) -> Result<GroupExitObservation, GroupMismatch> {
        let Some(actual) = self.env.innermost_group_kind() else {
            return Err(GroupMismatch::new_no_group(expected));
        };
        if actual != expected {
            return Err(GroupMismatch::new(expected, actual));
        }
        let (payloads, _meaning_changed, changed_cells, mut restores) = self
            .env
            .leave_group_with_kind_observing_meanings(expected)?;
        for receipt in &changed_cells {
            let cell = receipt.cell();
            self.update_exact_env_cell(cell, self.env.semantic_word(cell));
        }
        self.mark_exact_env_journal_current();
        self.retire_unrooted_region_values();
        self.capture_box_restore_texts(&mut restores);
        let code_before = self.code_tables.generations();
        let code_restores = self.code_tables.leave_group();
        let code_after = self.code_tables.generations();
        Ok((
            payloads,
            changed_cells,
            code_before,
            code_after,
            restores,
            code_restores,
        ))
    }

    /// Stores the token to insert after the next assignment.
    pub fn set_afterassignment(&mut self, token: Token) {
        self.assert_live_token(token);
        self.env.set_afterassignment(token);
    }

    /// Takes and clears the token to insert after the current assignment.
    pub fn take_afterassignment(&mut self) -> Option<Token> {
        self.env.take_afterassignment()
    }

    pub fn set_count(&mut self, index: u16, value: i32) -> crate::env::CellMutationReceipt {
        self.env.set_count(index, value)
    }

    #[must_use]
    pub fn count(&self, index: u16) -> i32 {
        self.env.count(index)
    }

    pub fn set_count_global(&mut self, index: u16, value: i32) -> crate::env::CellMutationReceipt {
        self.env.set_count_global(index, value)
    }

    pub fn set_dimen(&mut self, index: u16, value: Scaled) -> crate::env::CellMutationReceipt {
        self.env.set_dimen(index, value)
    }

    #[must_use]
    pub fn dimen(&self, index: u16) -> Scaled {
        self.env.dimen(index)
    }

    pub fn set_dimen_global(
        &mut self,
        index: u16,
        value: Scaled,
    ) -> crate::env::CellMutationReceipt {
        self.env.set_dimen_global(index, value)
    }

    pub fn set_skip(
        &mut self,
        index: u16,
        value: impl crate::glue::GlueHandle,
    ) -> crate::env::CellMutationReceipt {
        let value = value.glue_id();
        self.assert_live_glue(value);
        let value = self.resolve_stored_glue(value);
        self.env.set_skip(index, self.glue_ref(value))
    }

    #[must_use]
    pub fn skip(&self, index: u16) -> GlueId {
        self.resolve_stored_glue(self.env.skip(index))
    }

    pub fn set_skip_global(
        &mut self,
        index: u16,
        value: impl crate::glue::GlueHandle,
    ) -> crate::env::CellMutationReceipt {
        let value = value.glue_id();
        self.assert_live_glue(value);
        let value = self.resolve_stored_glue(value);
        self.env.set_skip_global(index, self.glue_ref(value))
    }

    pub fn set_muskip(
        &mut self,
        index: u16,
        value: impl crate::glue::GlueHandle,
    ) -> crate::env::CellMutationReceipt {
        let value = value.glue_id();
        self.assert_live_glue(value);
        let value = self.resolve_stored_glue(value);
        self.env.set_muskip(index, self.glue_ref(value))
    }

    #[must_use]
    pub fn muskip(&self, index: u16) -> GlueId {
        self.resolve_stored_glue(self.env.muskip(index))
    }

    pub fn set_muskip_global(
        &mut self,
        index: u16,
        value: impl crate::glue::GlueHandle,
    ) -> crate::env::CellMutationReceipt {
        let value = value.glue_id();
        self.assert_live_glue(value);
        let value = self.resolve_stored_glue(value);
        self.env.set_muskip_global(index, self.glue_ref(value))
    }

    pub fn set_toks(&mut self, index: u16, value: TokenListId) -> crate::env::CellMutationReceipt {
        self.assert_live_token_list(value);
        let value = self.resolve_stored_token_list(value);
        self.env.set_toks(
            index,
            self.tokens
                .owner(value)
                .expect("validated token list has a live owner"),
        )
    }

    #[must_use]
    pub fn toks(&self, index: u16) -> TokenListId {
        self.resolve_stored_token_list(self.env.toks(index))
    }

    pub fn set_toks_global(
        &mut self,
        index: u16,
        value: TokenListId,
    ) -> crate::env::CellMutationReceipt {
        self.assert_live_token_list(value);
        let value = self.resolve_stored_token_list(value);
        self.env.set_toks_global(
            index,
            self.tokens
                .owner(value)
                .expect("validated token list has a live owner"),
        )
    }

    pub fn clear_box_reg(&mut self, index: u16) -> crate::env::CellMutationReceipt {
        self.write_box_reg_ref(index, None, false)
    }

    pub fn clear_box_reg_global(&mut self, index: u16) -> crate::env::CellMutationReceipt {
        self.write_box_reg_ref(index, None, true)
    }

    pub fn clear_box_reg_same_level(&mut self, index: u16) -> crate::env::CellMutationReceipt {
        self.write_box_reg_ref_same_level(index, None)
    }

    pub(crate) fn box_reg_ref(&self, index: u16) -> Option<NodeListRef> {
        self.env.box_reg_ref(index)
    }

    pub(crate) fn take_box_reg_ref_with_receipt(
        &mut self,
        index: u16,
    ) -> (Option<NodeListRef>, crate::env::CellMutationReceipt) {
        let (old, receipt, rec) = self.env.take_box_reg(index);
        let old_id = old.as_ref().map(NodeListRef::id);
        let receipt = if receipt.changed() && self.update_main_memory_box_root(old_id, None) {
            receipt.with_main_memory_roots_updated()
        } else {
            receipt
        };
        let _ = rec;
        (old, receipt)
    }

    pub(crate) fn take_box_reg_ref_same_level_with_receipt(
        &mut self,
        index: u16,
    ) -> (Option<NodeListRef>, crate::env::CellMutationReceipt) {
        let (old, receipt, rec) = self.env.take_box_reg_same_level(index);
        let old_id = old.as_ref().map(NodeListRef::id);
        let receipt = if receipt.changed() && self.update_main_memory_box_root(old_id, None) {
            receipt.with_main_memory_roots_updated()
        } else {
            receipt
        };
        let _ = rec;
        (old, receipt)
    }

    pub fn set_int_param(
        &mut self,
        param: IntParam,
        value: i32,
    ) -> crate::env::CellMutationReceipt {
        self.env.set_int_param(param, value)
    }

    pub fn set_int_param_global(
        &mut self,
        param: IntParam,
        value: i32,
    ) -> crate::env::CellMutationReceipt {
        self.env.set_int_param_global(param, value)
    }

    #[must_use]
    pub fn int_param(&self, param: IntParam) -> i32 {
        self.env.int_param(param)
    }

    /// Reads TeX's most recent glue-setting badness.
    #[must_use]
    pub fn last_badness(&self) -> i32 {
        self.int_param(IntParam::LAST_BADNESS)
    }

    /// Records TeX's most recent glue-setting badness as global engine state.
    pub fn set_last_badness(&mut self, value: i32) -> CellMutationReceipt {
        self.set_int_param_global(IntParam::LAST_BADNESS, value)
    }

    /// Reads TeX's current `\mag` parameter.
    #[must_use]
    pub fn mag(&self) -> i32 {
        self.int_param(IntParam::MAG)
    }

    /// Sets TeX's local `\mag` parameter.
    pub fn set_mag(&mut self, value: i32) -> CellMutationReceipt {
        self.set_int_param(IntParam::MAG, value)
    }

    /// Sets TeX's global `\mag` parameter.
    pub fn set_mag_global(&mut self, value: i32) -> CellMutationReceipt {
        self.set_int_param_global(IntParam::MAG, value)
    }

    /// Returns the job-level magnification frozen by `prepare_mag`, if any.
    #[must_use]
    pub fn prepared_mag(&self) -> Option<i32> {
        self.prepared_mag
    }

    /// Validates and freezes TeX's job-level magnification.
    ///
    /// This mirrors tex.web's `prepare_mag`: illegal `\mag` values are
    /// globally coerced to 1000, and once any magnification has been prepared
    /// the same effective value is retained for the rest of the job.
    #[cfg(test)]
    pub fn prepare_mag(&mut self) -> (i32, Option<PrepareMagDiagnostic>) {
        self.prepare_mag_with_receipts().0
    }

    pub(crate) fn prepare_mag_with_receipts(
        &mut self,
    ) -> (
        (i32, Option<PrepareMagDiagnostic>),
        smallvec::SmallVec<[CellMutationReceipt; 1]>,
    ) {
        let attempted = self.mag();
        let mut receipts = smallvec::SmallVec::new();
        let (effective, diagnostic) = if !(1..=32_768).contains(&attempted) {
            receipts.push(self.set_int_param_global(IntParam::MAG, 1000));
            (
                1000,
                Some(PrepareMagDiagnostic::IllegalMagnification { attempted }),
            )
        } else if attempted != 1000 {
            match self.prepared_mag {
                Some(retained) if retained != attempted => {
                    receipts.push(self.set_int_param_global(IntParam::MAG, retained));
                    (
                        retained,
                        Some(PrepareMagDiagnostic::IncompatibleMagnification {
                            attempted,
                            retained,
                        }),
                    )
                }
                _ => (attempted, None),
            }
        } else {
            (attempted, None)
        };
        self.prepared_mag = Some(effective);
        ((effective, diagnostic), receipts)
    }

    /// Reads TeX's current `\endlinechar` parameter.
    #[must_use]
    pub fn endlinechar(&self) -> i32 {
        self.int_param(IntParam::END_LINE_CHAR)
    }

    pub fn set_dimen_param(
        &mut self,
        param: DimenParam,
        value: Scaled,
    ) -> crate::env::CellMutationReceipt {
        self.env.set_dimen_param(param, value)
    }

    pub fn set_dimen_param_global(
        &mut self,
        param: DimenParam,
        value: Scaled,
    ) -> crate::env::CellMutationReceipt {
        self.env.set_dimen_param_global(param, value)
    }

    #[must_use]
    pub fn dimen_param(&self, param: DimenParam) -> Scaled {
        self.env.dimen_param(param)
    }

    pub fn set_glue_param(
        &mut self,
        param: GlueParam,
        value: impl crate::glue::GlueHandle,
    ) -> crate::env::CellMutationReceipt {
        let value = value.glue_id();
        self.assert_live_glue(value);
        let value = self.resolve_stored_glue(value);
        self.env.set_glue_param(param, self.glue_ref(value))
    }

    #[must_use]
    pub fn glue_param(&self, param: GlueParam) -> GlueId {
        self.resolve_stored_glue(self.env.glue_param(param))
    }

    pub fn set_glue_param_global(
        &mut self,
        param: GlueParam,
        value: impl crate::glue::GlueHandle,
    ) -> crate::env::CellMutationReceipt {
        let value = value.glue_id();
        self.assert_live_glue(value);
        let value = self.resolve_stored_glue(value);
        self.env.set_glue_param_global(param, self.glue_ref(value))
    }

    pub fn set_tok_param_option(
        &mut self,
        param: TokParam,
        value: Option<TokenListId>,
    ) -> crate::env::CellMutationReceipt {
        let root = value.map(|value| {
            self.assert_live_token_list(value);
            let value = self.resolve_stored_token_list(value);
            self.tokens
                .owner(value)
                .expect("validated token list has a live owner")
        });
        self.env.set_tok_param_option(param, root)
    }

    #[must_use]
    pub fn tok_param(&self, param: TokParam) -> TokenListId {
        self.resolve_stored_token_list(self.env.tok_param(param))
    }

    /// Returns a token-list parameter while preserving an unassigned null cell.
    #[must_use]
    pub fn tok_param_option(&self, param: TokParam) -> Option<TokenListId> {
        self.env
            .tok_param_option(param)
            .map(|value| self.resolve_stored_token_list(value))
    }

    pub fn set_tok_param_option_global(
        &mut self,
        param: TokParam,
        value: Option<TokenListId>,
    ) -> crate::env::CellMutationReceipt {
        let root = value.map(|value| {
            self.assert_live_token_list(value);
            let value = self.resolve_stored_token_list(value);
            self.tokens
                .owner(value)
                .expect("validated token list has a live owner")
        });
        self.env.set_tok_param_option_global(param, root)
    }

    /// Takes a checkpoint for the rollback-coupled store tuple.
    ///
    /// Most fields remain O(1) marks/roots. The hyphenation table is cloned in
    /// v1 because pattern loading is rare and rollback soundness is more
    /// important than a premature journal for this INITEX-style state.
    #[must_use]
    pub(crate) fn checkpoint(&mut self) -> StoreSnapshot {
        self.synchronize_exact_env_identity();
        StoreSnapshot {
            owner: self.owner.snapshot_owner(),
            env_snapshot: self.env.checkpoint(),
            interner_mark: self.interner.watermark(),
            string_pool: self.string_pool.checkpoint(),
            string_pool_recycled_mark: self.string_pool_recycled_journal.len(),
            token_mark: self.tokens.watermark(),
            provenance_mark: self.provenance.watermark(),
            source_map_mark: self.source_map.watermark(),
            macro_mark: self.macros.watermark(),
            glue_mark: self.glue.watermark(),
            font_mark: self.fonts.watermark(),
            code_tables_snapshot: self.code_tables.checkpoint(),
            hyphenation: self.hyphenation.clone(),
            prepared_mag: self.prepared_mag,
            last_loaded_font: self.last_loaded_font,
            exact_env_identity: self.exact_env_identity.snapshot(),
            exact_projection_cache: self.semantic_hash_cache.projections.clone(),
        }
    }

    /// Retires direct-operation history when no group, checkpoint, or fork
    /// still owns the current journal baseline.
    pub(crate) fn commit_direct_operation(
        &mut self,
        mark: DirectStoreOperationMark,
        hash_base: &StoreStateHashCursor,
    ) -> bool {
        let discard_exact_history = self.env.can_discard_direct_derived_history();
        if !self.env.direct_operation_changed(mark.env) || !self.env.can_retire_direct_operation() {
            if discard_exact_history {
                self.discard_exact_env_undo_history();
                self.retire_unrooted_region_values();
            }
            self.finish_node_operation();
            return false;
        }
        self.preserve_retired_env_journal_hash_delta(hash_base);
        self.env
            .retire_direct_operation()
            .expect("direct retirement eligibility was checked above");
        self.discard_exact_env_undo_history();
        self.mark_exact_env_journal_current();
        self.retire_unrooted_region_values();
        self.finish_node_operation();
        true
    }

    fn retire_unrooted_region_values(&mut self) {
        self.macros.retire_unrooted_region_values();
        self.tokens.retire_unrooted_region_values();
        self.glue.retire_unrooted_region_values();
    }

    /// Finishes one executor operation without discarding its live-root projection.
    ///
    /// Transient node and box-copy observations compose usage without adding
    /// operation-local roots. Canonical Env roots are updated at their write
    /// barriers, so an unchanged operation boundary has no projection delta.
    pub(crate) fn finish_node_operation(&mut self) {
        #[cfg(feature = "profiling")]
        crate::measurement::record_main_memory_operation_boundary(
            self.transient_memory_base.is_some(),
        );
    }

    /// Opens one executor operation that owns no retry root. The TeX save
    /// stack keeps open-group history; advancing the compact write epoch
    /// prevents first-write coalescing from crossing command boundaries.
    pub(crate) fn begin_direct_operation(&mut self) -> DirectStoreOperationMark {
        DirectStoreOperationMark {
            env: self.env.begin_direct_operation(),
        }
    }

    /// Rolls all stores back to `snapshot` as one atomic tuple.
    pub(crate) fn rollback(&mut self, snapshot: &StoreSnapshot) -> MutationReceipts {
        self.rollback_inner(snapshot)
    }

    fn rollback_inner(&mut self, snapshot: &StoreSnapshot) -> MutationReceipts {
        self.assert_valid_snapshot(snapshot);
        let _ = self.engine_usage_statistics();
        let mut receipts = self.env.rollback_to(snapshot.env_snapshot.clone());
        // Env restoration still owns every destination root here, before the
        // rejected immutable-store suffix is truncated. Apply the O(delta)
        // receipt walk now and mark successful updates so Universe's semantic
        // mutation fanout does not apply the allocator delta twice.
        for receipt in &mut receipts {
            if receipt.changed() && self.update_main_memory_roots(*receipt) {
                *receipt = receipt.with_main_memory_roots_updated();
            }
        }
        self.interner.truncate_to(snapshot.interner_mark);
        while self.string_pool_recycled_journal.len() > snapshot.string_pool_recycled_mark {
            let retained = self
                .string_pool_recycled_journal
                .pop()
                .expect("checked string-pool journal length");
            assert!(
                self.string_pool.recycled.remove(retained.as_ref()),
                "string-pool journal entry is absent from the recycling index"
            );
        }
        self.string_pool.rollback_to(snapshot.string_pool);
        self.tokens.truncate_to(snapshot.token_mark);
        self.provenance.truncate_to(snapshot.provenance_mark);
        self.source_map.truncate_to(snapshot.source_map_mark);
        self.macros.truncate_to(snapshot.macro_mark);
        self.glue.truncate_to(snapshot.glue_mark);
        self.fonts.truncate_to(snapshot.font_mark);
        self.code_tables
            .rollback_to(snapshot.code_tables_snapshot.clone());
        self.hyphenation = snapshot.hyphenation.clone();
        self.prepared_mag = snapshot.prepared_mag;
        self.last_loaded_font = snapshot.last_loaded_font;
        for receipt in &receipts {
            let cell = receipt.cell();
            self.update_exact_env_cell(cell, self.env.semantic_word(cell));
        }
        self.mark_exact_env_journal_current();
        self.exact_env_identity.restore(snapshot.exact_env_identity);
        debug_assert_eq!(
            self.exact_env_identity.snapshot(),
            snapshot.exact_env_identity,
            "exact environment deltas did not restore the checkpoint accumulator"
        );
        // The cache is derived from the checkpoint timeline rather than part
        // of semantic state. Rebuild baselines lazily from the restored
        // journal slice instead of adding it to the O(1) snapshot tuple.
        self.semantic_hash_cache.clear();
        self.semantic_hash_cache.projections = snapshot.exact_projection_cache.clone();
        receipts
    }

    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn testing_clear_semantic_hash_cache(&mut self) {
        self.semantic_hash_cache.clear();
    }

    #[cfg(test)]
    pub(crate) fn testing_hyphenation_projection_hash_calls(&self) -> usize {
        self.semantic_hash_cache.testing_hyphenation_hash_calls()
    }

    /// Returns the number of journal bytes appended since `snapshot`.
    #[must_use]
    pub(crate) fn env_journal_bytes_since(&self, snapshot: &StoreSnapshot) -> usize {
        self.assert_valid_snapshot(snapshot);
        mem::size_of_val(
            self.env
                .journal_entries_since(snapshot.env_snapshot.journal_pos()),
        )
    }

    /// Current live environment-journal storage used by grouping and rollback.
    #[must_use]
    pub(crate) fn env_journal_bytes(&self) -> usize {
        self.env.journal_retained_bytes()
    }

    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn env_journal_entry_count(&self) -> usize {
        self.env.journal_entry_count()
    }

    pub(crate) fn generation_retained_bytes(&self) -> usize {
        let serialized = self
            .encode_frozen_format()
            .map_or(0, |format| format.payload_len());
        let provenance = self.provenance_stats().retained_bytes();
        let source_map = self.source_map.stats().retained_bytes;
        let source_fragment_metadata = self.source_fragments.metadata_retained_bytes();
        std::mem::size_of::<Self>()
            .saturating_add(serialized)
            .saturating_add(self.env.journal_retained_bytes())
            .saturating_add(provenance)
            .saturating_add(source_map)
            .saturating_add(source_fragment_metadata)
    }

    /// Verifies the shadow mirror against real environment storage.
    #[cfg(feature = "shadow")]
    pub fn verify_shadow(&self) {
        self.env.verify_shadow();
    }

    fn assert_valid_snapshot(&self, snapshot: &StoreSnapshot) {
        assert_eq!(
            snapshot.owner,
            self.owner.snapshot_owner(),
            "Stores snapshot belongs to a different Stores instance"
        );
        assert!(
            self.env.can_rollback_to(&snapshot.env_snapshot),
            "Stores snapshots are invalidated by exiting a group that encloses them"
        );
        assert!(
            snapshot.env_snapshot.journal_pos() <= self.env.current_journal_pos(),
            "Stores snapshots are invalidated by journal truncation before their checkpoint position"
        );
    }

    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn testing_ownership_census(&self) -> TestingOwnershipCensus {
        let macro_live = self.macros.testing_live_totals();
        let macro_shapes = self.macros.testing_pool_shapes();
        let provenance = self.provenance_stats();
        let (node_weak_entries, node_weak_capacity) = self.node_ref_index.shape();
        TestingOwnershipCensus {
            token_lists: TestingValuePoolCensus::new(
                self.tokens.testing_live_totals(),
                self.tokens.testing_pool_shape(),
            ),
            macro_bodies: TestingValuePoolCensus::new((macro_live.0, macro_live.1), macro_shapes.0),
            macro_definitions: TestingValuePoolCensus::new(
                (macro_live.2, macro_live.3),
                macro_shapes.1,
            ),
            glue_specs: TestingValuePoolCensus::new(
                self.glue.testing_live_totals(),
                self.glue.testing_pool_shape(),
            ),
            node_weak_entries,
            node_weak_capacity,
            provenance_records: provenance.origin_records(),
            provenance_lists: provenance.origin_list_spans(),
            provenance_entries: provenance.origin_list_entries(),
            provenance_retained_bytes: provenance.retained_bytes(),
            source_regions: provenance.source_regions(),
            source_bytes: provenance.source_map_bytes(),
            journal_entries: self.env.journal_entry_count(),
            journal_retained_bytes: self.env.journal_retained_bytes(),
        }
    }
}

impl Default for Stores {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
