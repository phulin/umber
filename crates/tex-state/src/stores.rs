//! Internal aggregate state stores and atomic rollback machinery.
//!
//! `Stores` is the private composition owned by `Universe`. Public callers use
//! `Universe` for checkpointing and rollback so the whole timeline tuple is
//! restored atomically.

use crate::code_tables::{
    CodeTableGenerations, CodeTables, CodeTablesSnapshot, DelCode, LcCode, MathCode, SfCode, UcCode,
};
use crate::env::banks::{DimenParam, GlueParam, IntParam, TokParam};
use crate::env::{Env, EnvSnapshot};
use crate::font::{
    CharMetrics, CharTag, ExtensibleRecipe, FontMetrics, FontMetricsValidationError,
    FontSourceIdentity, FontStore, FontStoreMark, LigKernChar, LigKernCommand, LigKernIter,
    LoadedFont, MissingCharacter, NULL_FONT, complete_font_hash_fragment,
};
use crate::font::{FontExpansion, FontExpansionConfigError, PdfFontCode};

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
use crate::glue::{GlueSpec, GlueStore, GlueStoreMark};
use crate::hyphenation::{ExceptionSpec, HyphenationTable, PatternSpec};
use crate::ids::{FontId, GlueId, MacroDefinitionId, NodeListId, OriginListId, TokenListId};
use crate::input::SourceId;
use crate::input::TracedTokenList;
use crate::interner::{
    ControlSequenceKind, Interner, InternerError, InternerMark, Symbol, SymbolId, SymbolReference,
};
use crate::macro_store::{
    MacroDefinitionProvenance, MacroMeaning, MacroParameterPattern, MacroStore, MacroStoreMark,
};
use crate::math::MathFontSize;
use crate::meaning::Meaning;
use crate::node::Node;
#[cfg(feature = "profiling-stats")]
use crate::node_arena::NodeMemoryColumn;
use crate::node_arena::{NodeArena, NodeArenaMark, NodeList, NodeListBuilder};
use crate::provenance::{
    InsertedOrigin, InsertedOriginKind, MacroInvocationOrigin, OriginListBuilder, OriginRecord,
    ProvenanceStats, ProvenanceStore, ProvenanceStoreMark, SourceOrigin, SynthesizedOrigin,
    SynthesizedOriginKind, SyntheticOrigin, SyntheticOriginKind,
};
use crate::scaled::Scaled;
use crate::source_fragments::FragmentStore;
use crate::source_map::{
    GeneratedSource, SourceBacking, SourceDescriptor, SourceMap, SourceMapError, SourceMapMark,
    SourcePos, SourceRegion, SourceSpan,
};
use crate::survivor::SurvivorArena;
use crate::token::{Catcode, OriginId, Token, TracedTokenWord};
use crate::token_store::{
    TokenListBuilder, TokenSemanticId, TokenSemanticIdBuilder, TokenStore, TokenStoreMark,
};
#[cfg(any(test, feature = "testing", feature = "shadow"))]
use std::hash::{Hash, Hasher};
use std::mem;
use std::sync::Arc;

mod format;
mod handles;
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
    testing_take_legacy_restore_count,
};

pub use crate::env::group::{GroupKind, GroupMismatch};
pub(crate) use state_hash::StoreStateHashCursor;

#[cfg(any(test, feature = "testing", feature = "shadow"))]
const TESTING_NODE_HASH_MAX_DEPTH: usize = 4096;

/// A rollback snapshot for all currently implemented state stores.
#[derive(Clone, Debug)]
pub(crate) struct StoreSnapshot {
    owner: SnapshotOwner,
    env_snapshot: EnvSnapshot,
    interner_mark: InternerMark,
    token_mark: TokenStoreMark,
    provenance_mark: ProvenanceStoreMark,
    source_map_mark: SourceMapMark,
    macro_mark: MacroStoreMark,
    glue_mark: GlueStoreMark,
    font_mark: FontStoreMark,
    node_mark: NodeArenaMark,
    survivor_pin_mark: usize,
    code_tables_snapshot: CodeTablesSnapshot,
    hyphenation: Arc<HyphenationTable>,
    prepared_mag: Option<i32>,
    last_loaded_font: FontId,
}

impl StoreSnapshot {
    #[must_use]
    pub(crate) const fn epoch(&self) -> crate::epoch::Epoch {
        self.env_snapshot.epoch()
    }
}

/// Opaque node-allocation mark for one in-progress shipout.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ShipoutNodeMark {
    owner: SnapshotOwner,
    node_mark: NodeArenaMark,
    survivor_pin_mark: usize,
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
    tokens: TokenStore,
    provenance: ProvenanceStore,
    source_map: SourceMap,
    source_fragments: FragmentStore,
    macros: MacroStore,
    glue: GlueStore,
    fonts: FontStore,
    nodes: NodeArena,
    survivors: SurvivorArena,
    survivor_pins: Vec<NodeListId>,
    code_tables: CodeTables,
    hyphenation: Arc<HyphenationTable>,
    prepared_mag: Option<i32>,
    last_loaded_font: FontId,
    /// Runtime-only guard for derived control-sequence meaning caches.
    /// This never rewinds across group restoration or snapshot rollback.
    /// Live meanings can change only through the aggregate setters, a
    /// group-exit path that reports meaning journal activity, or aggregate
    /// rollback; each such boundary advances this generation. Format
    /// reconstruction writes raw Env words only while
    /// constructing a fresh Stores owner, and cloning likewise mints a fresh
    /// owner identity, so neither can validate a cache owned by the source.
    meaning_generation: u64,
    semantic_hash_cache: state_hash::SemanticHashCache,
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
        Self {
            owner: StoreOwner::new(),
            env: self.env.clone(),
            interner: self.interner.clone(),
            tokens: self.tokens.clone(),
            provenance: self.provenance.clone(),
            source_map: self.source_map.clone(),
            source_fragments: self.source_fragments.clone(),
            macros: self.macros.clone(),
            glue: self.glue.clone(),
            fonts: self.fonts.clone(),
            nodes: self.nodes.clone(),
            survivors: self.survivors.clone(),
            survivor_pins: self.survivor_pins.clone(),
            code_tables: self.code_tables.clone(),
            hyphenation: self.hyphenation.clone(),
            prepared_mag: self.prepared_mag,
            last_loaded_font: self.last_loaded_font,
            meaning_generation: self.meaning_generation,
            semantic_hash_cache: self.semantic_hash_cache.clone(),
        }
    }
}

impl Stores {
    pub(crate) fn retain_diagnostic_origins_from(&mut self, fork: &Self, roots: &[OriginId]) {
        self.provenance
            .retain_origin_graph_from(&fork.provenance, roots);
    }

    pub(crate) fn install_source_fragments(&mut self, fragments: FragmentStore) {
        self.source_fragments = fragments;
    }

    pub(crate) fn can_restore_snapshot(&self, snapshot: &StoreSnapshot) -> bool {
        snapshot.owner == self.owner.snapshot_owner()
            && snapshot.env_snapshot.group_depth() == self.env.group_depth()
            && snapshot.env_snapshot.journal_pos() <= self.env.current_journal_pos()
            && snapshot.survivor_pin_mark <= self.survivor_pins.len()
    }

    /// Retargets an already-validated inherited snapshot to this fork's exact owner.
    pub(crate) fn retarget_inherited_snapshot(&self, snapshot: &StoreSnapshot) -> StoreSnapshot {
        let mut snapshot = snapshot.clone();
        snapshot.owner = self.owner.snapshot_owner();
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
    /// Creates an empty state-store tuple.
    #[must_use]
    pub fn new() -> Self {
        let mut stores = Self {
            owner: StoreOwner::new(),
            env: Env::new(),
            interner: Interner::new(),
            tokens: TokenStore::new(),
            provenance: ProvenanceStore::new(),
            source_map: SourceMap::default(),
            source_fragments: FragmentStore::new(),
            macros: MacroStore::new(),
            glue: GlueStore::new(),
            fonts: FontStore::new(),
            nodes: NodeArena::new(),
            survivors: SurvivorArena::new(),
            survivor_pins: Vec::new(),
            code_tables: CodeTables::new(),
            hyphenation: Arc::new(HyphenationTable::new()),
            prepared_mag: None,
            last_loaded_font: NULL_FONT,
            meaning_generation: 1,
            semantic_hash_cache: state_hash::SemanticHashCache::default(),
        };
        stores.set_int_param(IntParam::MAG, 1000);
        stores.set_int_param(IntParam::TOLERANCE, 10_000);
        stores.set_int_param(IntParam::HANG_AFTER, 1);
        stores.set_int_param(IntParam::MAX_DEAD_CYCLES, 25);
        stores.set_int_param(IntParam::ESCAPE_CHAR, b'\\'.into());
        stores.set_int_param(IntParam::END_LINE_CHAR, 13);
        stores.initialize_font_banks(NULL_FONT, 7, &[]);
        stores
    }

    /// Reads the owned environment.
    #[must_use]
    #[cfg(test)]
    pub fn env(&self) -> &Env {
        &self.env
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
        self.code_tables.set_catcode(ch, value);
    }

    pub fn set_catcode_global(&mut self, ch: char, value: Catcode) {
        self.code_tables.set_catcode_global(ch, value);
    }

    #[must_use]
    pub fn lccode(&self, ch: char) -> LcCode {
        self.code_tables.lccode(ch)
    }

    pub fn set_lccode(&mut self, ch: char, value: LcCode) {
        self.code_tables.set_lccode(ch, value);
    }

    pub fn set_lccode_global(&mut self, ch: char, value: LcCode) {
        self.code_tables.set_lccode_global(ch, value);
    }

    #[must_use]
    pub fn uccode(&self, ch: char) -> UcCode {
        self.code_tables.uccode(ch)
    }

    pub fn set_uccode(&mut self, ch: char, value: UcCode) {
        self.code_tables.set_uccode(ch, value);
    }

    pub fn set_uccode_global(&mut self, ch: char, value: UcCode) {
        self.code_tables.set_uccode_global(ch, value);
    }

    #[must_use]
    pub fn sfcode(&self, ch: char) -> SfCode {
        self.code_tables.sfcode(ch)
    }

    pub fn set_sfcode(&mut self, ch: char, value: SfCode) {
        self.code_tables.set_sfcode(ch, value);
    }

    pub fn set_sfcode_global(&mut self, ch: char, value: SfCode) {
        self.code_tables.set_sfcode_global(ch, value);
    }

    #[must_use]
    pub fn mathcode(&self, ch: char) -> MathCode {
        self.code_tables.mathcode(ch)
    }

    pub fn set_mathcode(&mut self, ch: char, value: MathCode) {
        self.code_tables.set_mathcode(ch, value);
    }

    pub fn set_mathcode_global(&mut self, ch: char, value: MathCode) {
        self.code_tables.set_mathcode_global(ch, value);
    }

    #[must_use]
    pub fn delcode(&self, ch: char) -> DelCode {
        self.code_tables.delcode(ch)
    }

    pub fn set_delcode(&mut self, ch: char, value: DelCode) {
        self.code_tables.set_delcode(ch, value);
    }

    pub fn set_delcode_global(&mut self, ch: char, value: DelCode) {
        self.code_tables.set_delcode_global(ch, value);
    }

    pub fn add_hyphenation_pattern(&mut self, pattern: PatternSpec) {
        self.add_hyphenation_pattern_for_language(0, pattern);
    }

    pub fn add_hyphenation_pattern_for_language(&mut self, language: u8, pattern: PatternSpec) {
        Arc::make_mut(&mut self.hyphenation).add_pattern_for_language(language, pattern);
    }

    pub fn add_hyphenation_exception(&mut self, exception: ExceptionSpec) {
        self.add_hyphenation_exception_for_language(0, exception);
    }

    pub fn add_hyphenation_exception_for_language(
        &mut self,
        language: u8,
        exception: ExceptionSpec,
    ) {
        Arc::make_mut(&mut self.hyphenation).add_exception_for_language(language, exception);
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
        self.resolve_stored_meaning(self.env.get_meaning_slot(symbol.raw()))
    }

    /// Returns the nonzero, monotonically increasing meaning-write guard.
    #[must_use]
    pub(crate) fn meaning_cache_guard(&self) -> crate::universe::MeaningCacheGuard {
        let owner = self.owner.snapshot_owner();
        crate::universe::MeaningCacheGuard::new(owner.address, owner.nonce, self.meaning_generation)
    }

    fn bump_meaning_generation(&mut self) {
        self.meaning_generation = self
            .meaning_generation
            .checked_add(1)
            .expect("meaning generation exhausted");
    }

    /// Sets the local meaning for a live control-sequence symbol.
    pub fn set_meaning(&mut self, symbol: impl SymbolReference, meaning: Meaning) {
        let symbol = self.resolve_symbol_reference(symbol);
        self.assert_live_macro_definition_in_meaning(meaning);
        self.assert_live_font_in_meaning(meaning);
        self.env.set_meaning_slot(symbol.raw(), meaning, false);
        self.bump_meaning_generation();
        #[cfg(feature = "profiling-stats")]
        crate::measurement::record_meaning_cache_invalidation(
            crate::measurement::MeaningCacheInvalidation::LocalWrite,
        );
    }

    /// Interns a control-sequence name and gives a previously undefined name
    /// TeX's `\csname`-created `\relax` meaning.
    pub fn intern_relaxed_control_sequence(&mut self, name: &str) -> SymbolId {
        let symbol = self.intern(name);
        if self.meaning(symbol) == Meaning::Undefined {
            self.set_meaning(symbol, Meaning::Relax);
        }
        symbol
    }

    /// Sets the global meaning for a live control-sequence symbol.
    pub fn set_meaning_global(&mut self, symbol: impl SymbolReference, meaning: Meaning) {
        let symbol = self.resolve_symbol_reference(symbol);
        self.assert_live_macro_definition_in_meaning(meaning);
        self.assert_live_font_in_meaning(meaning);
        self.env.set_meaning_slot(symbol.raw(), meaning, true);
        self.bump_meaning_generation();
        #[cfg(feature = "profiling-stats")]
        crate::measurement::record_meaning_cache_invalidation(
            crate::measurement::MeaningCacheInvalidation::GlobalWrite,
        );
    }

    /// Interns a frozen macro definition in the owned macro-definition store.
    pub fn intern_macro(&mut self, macro_meaning: MacroMeaning) -> MacroDefinitionId {
        self.intern_macro_with_provenance(macro_meaning, None)
    }

    /// Interns a frozen macro definition with optional diagnostic provenance.
    pub fn intern_macro_with_provenance(
        &mut self,
        macro_meaning: MacroMeaning,
        provenance: Option<MacroDefinitionProvenance>,
    ) -> MacroDefinitionId {
        self.assert_live_token_list(macro_meaning.parameter_text());
        self.assert_live_token_list(macro_meaning.replacement_text());
        if let Some(provenance) = provenance {
            self.assert_live_origin(provenance.definition_origin());
            self.assert_live_origin_list(provenance.parameter_origins());
            self.assert_live_origin_list(provenance.replacement_origins());
            self.assert_origin_list_len_matches(
                macro_meaning.parameter_text(),
                provenance.parameter_origins(),
            );
            self.assert_origin_list_len_matches(
                macro_meaning.replacement_text(),
                provenance.replacement_origins(),
            );
        }
        let parameter_pattern =
            MacroParameterPattern::from_tokens(self.tokens(macro_meaning.parameter_text()));
        self.macros
            .intern_with_provenance(macro_meaning, parameter_pattern, provenance)
    }

    /// Reads a live frozen macro definition.
    #[must_use]
    pub fn macro_definition(&self, id: MacroDefinitionId) -> MacroMeaning {
        self.assert_live_macro_definition(id);
        self.macros.get(id)
    }

    /// Reads the pre-parsed parameter structure for a live macro definition.
    #[must_use]
    pub fn macro_definition_parameter_pattern(
        &self,
        id: MacroDefinitionId,
    ) -> MacroParameterPattern {
        self.assert_live_macro_definition(id);
        self.macros.parameter_pattern(id)
    }

    /// Reads diagnostic provenance for a macro definition, degrading to
    /// unknown when the optional side-table entry is absent or stale.
    #[must_use]
    pub fn macro_definition_provenance(&self, id: MacroDefinitionId) -> MacroDefinitionProvenance {
        let Some(provenance) = self.macros.provenance(id) else {
            return MacroDefinitionProvenance::unknown();
        };
        if self
            .provenance
            .contains_origin(provenance.definition_origin())
            && self
                .provenance
                .contains_list(provenance.parameter_origins())
            && self
                .provenance
                .contains_list(provenance.replacement_origins())
        {
            provenance
        } else {
            MacroDefinitionProvenance::unknown()
        }
    }

    /// Sets a local macro meaning by freezing its public aggregate first.
    pub fn set_macro_meaning(&mut self, symbol: impl SymbolReference, macro_meaning: MacroMeaning) {
        let definition = self.intern_macro(macro_meaning);
        self.set_meaning(
            symbol,
            Meaning::Macro {
                flags: macro_meaning.flags(),
                definition,
            },
        );
    }

    /// Sets a local macro meaning with diagnostic definition provenance.
    pub fn set_macro_meaning_with_provenance(
        &mut self,
        symbol: impl SymbolReference,
        macro_meaning: MacroMeaning,
        provenance: MacroDefinitionProvenance,
    ) {
        let definition = self.intern_macro_with_provenance(macro_meaning, Some(provenance));
        self.set_meaning(
            symbol,
            Meaning::Macro {
                flags: macro_meaning.flags(),
                definition,
            },
        );
    }

    /// Sets a global macro meaning by freezing its public aggregate first.
    pub fn set_macro_meaning_global(
        &mut self,
        symbol: impl SymbolReference,
        macro_meaning: MacroMeaning,
    ) {
        let definition = self.intern_macro(macro_meaning);
        self.set_meaning_global(
            symbol,
            Meaning::Macro {
                flags: macro_meaning.flags(),
                definition,
            },
        );
    }

    /// Sets a global macro meaning with diagnostic definition provenance.
    pub fn set_macro_meaning_global_with_provenance(
        &mut self,
        symbol: impl SymbolReference,
        macro_meaning: MacroMeaning,
        provenance: MacroDefinitionProvenance,
    ) {
        let definition = self.intern_macro_with_provenance(macro_meaning, Some(provenance));
        self.set_meaning_global(
            symbol,
            Meaning::Macro {
                flags: macro_meaning.flags(),
                definition,
            },
        );
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
        self.interner
            .intern_active(ch)
            .expect("control-sequence symbol capacity exceeded")
    }

    /// Interns a control-sequence name, reporting packed-token capacity exhaustion.
    pub(crate) fn try_intern(&mut self, name: &str) -> Result<SymbolId, InternerError> {
        self.interner.intern(name)
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
    pub fn intern_token_list(&mut self, tokens: &[Token]) -> TokenListId {
        let semantic_id = self.token_list_semantic_id(tokens.iter().copied());
        let frozen_key = self.frozen_token_lookup_key(tokens.iter().copied());
        self.tokens
            .intern_with_semantic_id(tokens, semantic_id, &frozen_key)
    }

    /// Interns the current token-list builder value and clears it for reuse.
    pub fn finish_token_list(&mut self, builder: &mut TokenListBuilder) -> TokenListId {
        let semantic_id = self.token_list_semantic_id(builder.as_slice().iter().copied());
        let frozen_key = self.frozen_token_lookup_key(builder.as_slice().iter().copied());
        let id = self
            .tokens
            .intern_with_semantic_id(builder.as_slice(), semantic_id, &frozen_key);
        builder.clear();
        id
    }

    /// Freezes semantic tokens and per-instance origins directly from their
    /// packed traced representation.
    pub fn finish_traced_token_list(&mut self, traced: &[TracedTokenWord]) -> TracedTokenList {
        for &word in traced {
            let token = word
                .token()
                .expect("traced token list contains an invalid semantic token");
            self.assert_live_token(token);
            self.assert_live_origin(word.origin());
        }

        let semantic_id = self.token_list_semantic_id(traced.iter().map(|word| {
            word.token()
                .expect("validated traced token became invalid during semantic hashing")
        }));
        let frozen_key = self.frozen_token_lookup_key(traced.iter().map(|word| {
            word.token()
                .expect("validated traced token became invalid during lookup encoding")
        }));

        #[cfg(feature = "profiling-stats")]
        crate::measurement::record_traced_list_finish(traced.len(), 0, 0);
        let token_list =
            self.tokens
                .intern_traced_with_semantic_id(traced, semantic_id, &frozen_key);
        let origin_list = self.provenance.allocate_traced_list(traced);
        TracedTokenList::new(token_list, origin_list)
    }

    /// Reads a live frozen token list.
    #[must_use]
    pub fn tokens(&self, id: TokenListId) -> &[Token] {
        let id = self.resolve_stored_token_list(id);
        self.tokens.get(id)
    }

    pub(crate) fn token_list_semantic_id_value(&self, id: TokenListId) -> u64 {
        let id = self.resolve_stored_token_list(id);
        self.tokens.semantic_id(id).value()
    }

    #[cfg(test)]
    pub(crate) fn testing_token_semantic_id(&self, id: TokenListId) -> TokenSemanticId {
        let id = self.resolve_stored_token_list(id);
        self.tokens.semantic_id(id)
    }

    fn token_list_semantic_id(&self, tokens: impl IntoIterator<Item = Token>) -> TokenSemanticId {
        let mut identity = TokenSemanticIdBuilder::new();
        for token in tokens {
            let atom = match token {
                Token::Cs(symbol) => Some(
                    self.interner
                        .semantic_atom(symbol)
                        .expect("symbol is not live in this Universe timeline"),
                ),
                _ => None,
            };
            identity.push(token, atom);
        }
        identity.finish()
    }

    fn frozen_token_lookup_key(&self, tokens: impl IntoIterator<Item = Token>) -> Vec<u8> {
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
                Token::Frozen(crate::token::FrozenToken::END_TEMPLATE) => 3_u64 << 56,
                Token::Frozen(crate::token::FrozenToken::END_V) => (3_u64 << 56) | 1,
                Token::Frozen(_) => unreachable!("invalid frozen token payload"),
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
        self.provenance
            .allocate(OriginRecord::MacroInvocation(MacroInvocationOrigin::new(
                definition,
                invocation,
                definition_origin,
                parent_invocation,
            )))
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

    /// Allocates a synthetic/bootstrap origin.
    pub fn synthetic_origin(&mut self, kind: SyntheticOriginKind) -> OriginId {
        match kind {
            SyntheticOriginKind::Bootstrap => ProvenanceStore::unknown_id(),
            _ => self
                .provenance
                .allocate(OriginRecord::Synthetic(SyntheticOrigin::new(kind))),
        }
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
            crate::token::OriginEncoding::Unknown | crate::token::OriginEncoding::Arena(_) => self
                .provenance
                .contains_origin(id)
                .then(|| self.provenance.get(id)),
        }
    }

    /// Allocates an origin-list span.
    pub fn allocate_origin_list(&mut self, origins: &[OriginId]) -> OriginListId {
        for &origin in origins {
            self.assert_live_origin(origin);
        }
        self.provenance.allocate_list(origins)
    }

    /// Allocates an origin-list span by repeating one live origin.
    pub fn allocate_repeated_origin_list(&mut self, origin: OriginId, len: usize) -> OriginListId {
        self.assert_live_origin(origin);
        self.provenance.allocate_repeated_list(origin, len)
    }

    /// Creates a fresh owned scratch origin-list builder.
    #[must_use]
    pub fn origin_list_builder(&self) -> OriginListBuilder {
        ProvenanceStore::builder()
    }

    /// Allocates the current origin-list builder value and clears it for reuse.
    pub fn finish_origin_list(&mut self, builder: &mut OriginListBuilder) -> OriginListId {
        for &origin in builder.as_slice() {
            self.assert_live_origin(origin);
        }
        builder.finish(&mut self.provenance)
    }

    /// Reads a live origin-list span.
    #[must_use]
    pub fn origin_list(&self, id: OriginListId) -> &[OriginId] {
        self.provenance
            .resolve_list(id)
            .expect("origin list id is not live in this Universe timeline")
    }

    /// Reads an origin-list span if it is still live on this timeline.
    #[must_use]
    pub fn origin_list_if_live(&self, id: OriginListId) -> Option<&[OriginId]> {
        self.provenance.resolve_list(id)
    }

    /// Returns live provenance arena length counters.
    #[must_use]
    pub fn provenance_stats(&self) -> ProvenanceStats {
        self.provenance
            .stats()
            .with_source_map(self.source_map.stats())
    }

    /// Registers immutable source backing on this aggregate timeline.
    pub(crate) fn register_source(
        &mut self,
        source: SourceId,
        descriptor: SourceDescriptor,
        line_starts: std::sync::Arc<[usize]>,
    ) -> Result<SourcePos, SourceMapError> {
        self.source_map
            .register_with_line_starts(source, descriptor, line_starts)
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

    fn assert_origin_list_len_matches(&self, token_list: TokenListId, origin_list: OriginListId) {
        if origin_list == OriginListId::EMPTY {
            return;
        }
        assert_eq!(
            self.tokens(token_list).len(),
            self.origin_list(origin_list).len(),
            "origin-list length does not match token-list length"
        );
    }

    /// Interns a frozen glue specification in the owned glue store.
    pub fn intern_glue(&mut self, spec: GlueSpec) -> GlueId {
        self.glue.intern(spec)
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
        self.fonts.set_expansion(font, expansion)
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
        self.fonts.set_identifier(id, symbol, complete);
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
        if let LigKernChar::Char(code) = left
            && self.pdf_font_code(PdfFontCode::Tag, font, code) & 1 == 0
        {
            return None;
        }
        if self.env.pdf_no_ligatures(font) {
            return self
                .font(font)
                .metrics()
                .lig_kern_command(left, right)
                .filter(|command| matches!(command, LigKernCommand::Kern(_)));
        }
        self.font(font).metrics().lig_kern_command(left, right)
    }

    #[must_use]
    pub fn pdf_font_code(&self, table: PdfFontCode, font: FontId, code: u8) -> i32 {
        self.assert_live_font(font);
        let bank = pdf_font_code_bank(table);
        self.env
            .pdf_font_code(bank, font, code)
            .unwrap_or_else(|| match table {
                PdfFontCode::Ef => 1000,
                PdfFontCode::Tag => {
                    self.font_char_metrics(font, code)
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

    pub fn set_pdf_font_code(&mut self, table: PdfFontCode, font: FontId, code: u8, value: i32) {
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
            .set_pdf_font_code_global(pdf_font_code_bank(table), font, code, value);
    }

    pub fn disable_pdf_font_ligatures(&mut self, font: FontId) {
        self.assert_live_font(font);
        self.env.set_pdf_no_ligatures_global(font);
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

    pub fn set_current_font(&mut self, id: FontId) {
        self.assert_live_font(id);
        self.env.set_current_font(id);
    }

    pub fn set_current_font_global(&mut self, id: FontId) {
        self.assert_live_font(id);
        self.env.set_current_font_global(id);
    }

    pub fn set_current_font_selector(&mut self, symbol: impl SymbolReference, id: FontId) {
        let symbol = self.resolve_symbol_reference(symbol);
        self.assert_live_font(id);
        self.env.set_current_font_selector(symbol.symbol(), id);
    }

    pub fn set_current_font_selector_global(&mut self, symbol: impl SymbolReference, id: FontId) {
        let symbol = self.resolve_symbol_reference(symbol);
        self.assert_live_font(id);
        self.env
            .set_current_font_selector_global(symbol.symbol(), id);
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
    ) {
        self.assert_live_font(id);
        if global {
            self.env.set_math_family_font_global(size, family, id);
        } else {
            self.env.set_math_family_font(size, family, id);
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
    ) -> Result<(), FontParameterError> {
        let index = self.prepare_font_dimen_write(font, number)?;
        self.env.set_font_dimen_global(index, value);
        Ok(())
    }

    #[must_use]
    pub fn font_hyphen_char(&self, font: FontId) -> i32 {
        self.assert_live_font(font);
        self.env.font_hyphen_char(font)
    }

    pub fn set_font_hyphen_char(&mut self, font: FontId, value: i32) {
        self.assert_live_font(font);
        self.env.set_font_hyphen_char_global(font, value);
    }

    #[must_use]
    pub fn font_skew_char(&self, font: FontId) -> i32 {
        self.assert_live_font(font);
        self.env.font_skew_char(font)
    }

    pub fn set_font_skew_char(&mut self, font: FontId, value: i32) {
        self.assert_live_font(font);
        self.env.set_font_skew_char_global(font, value);
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

    fn prepare_font_dimen_write(
        &mut self,
        font: FontId,
        number: u32,
    ) -> Result<u32, FontParameterError> {
        self.assert_live_font(font);
        let index = crate::env::font_dimen_index(font, number)?;
        let current_len = self.env.font_param_len(font);
        if number > current_len {
            if font != self.last_loaded_font {
                return Err(FontParameterError::CannotGrow {
                    font,
                    number,
                    current_len,
                    last_loaded_font: self.last_loaded_font,
                });
            }
            self.env.set_font_param_len_global(font, number);
        }
        Ok(index)
    }

    /// Creates a fresh owned scratch node-list builder.
    #[must_use]
    pub fn node_list_builder(&self) -> NodeListBuilder {
        NodeArena::builder()
    }

    /// Appends and freezes a node list in the owned epoch arena.
    pub fn freeze_node_list(&mut self, nodes: &[Node]) -> NodeListId {
        let semantic_id = self.validate_and_compute_node_semantic_id(nodes);
        self.nodes.append_with_semantic_id(nodes, semantic_id)
    }

    /// Freezes an owned decoded node vector and clears it for allocation reuse.
    pub fn freeze_node_list_owned(&mut self, nodes: &mut Vec<Node>) -> NodeListId {
        let semantic_id = self.validate_and_compute_node_semantic_id(nodes);
        let id = self.nodes.append_with_semantic_id(nodes, semantic_id);
        nodes.clear();
        id
    }

    /// Freezes the current node-list builder value and clears it for reuse.
    pub fn finish_node_list(&mut self, builder: &mut NodeListBuilder) -> NodeListId {
        self.assert_live_handles_in_nodes(builder.as_slice());
        let semantic_id = self.compute_and_seal_node_semantic_id(builder.as_slice());
        let id = self
            .nodes
            .append_with_semantic_id(builder.as_slice(), semantic_id);
        builder.clear();
        id
    }

    /// Reads a live frozen node list.
    #[must_use]
    pub fn nodes(&self, id: NodeListId) -> NodeList<'_> {
        self.assert_live_node_list(id);
        self.nodes.get(id, &self.survivors)
    }

    pub(crate) fn node_list_semantic_id_value(&self, id: NodeListId) -> u64 {
        self.assert_live_node_list(id);
        self.nodes.semantic_id(id, &self.survivors).value()
    }

    /// Keeps a survivor root alive until its enclosing allocation scope ends.
    pub fn pin_survivor(&mut self, id: NodeListId) {
        self.assert_live_node_list(id);
        self.survivors.inc_ref(id);
        self.survivor_pins.push(id);
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

    /// Pushes an `\aftergroup` token for the current group.
    pub fn push_aftergroup(&mut self, payload: Token) {
        self.assert_live_token(payload);
        self.env.push_aftergroup(payload);
    }

    /// Leaves the innermost TeX group and returns its `\aftergroup` payloads.
    #[must_use]
    pub fn leave_group(&mut self) -> Vec<Token> {
        self.account_current_group_box_refs();
        let (payloads, meaning_changed) = self.env.leave_group_observing_meanings();
        self.code_tables.leave_group();
        if meaning_changed {
            self.bump_meaning_generation();
            #[cfg(feature = "profiling-stats")]
            crate::measurement::record_meaning_cache_invalidation(
                crate::measurement::MeaningCacheInvalidation::GroupExit,
            );
        }
        payloads
    }

    /// Leaves the innermost TeX group after checking its boundary kind.
    pub fn leave_group_with_kind(
        &mut self,
        expected: GroupKind,
    ) -> Result<Vec<Token>, GroupMismatch> {
        let Some(actual) = self.env.innermost_group_kind() else {
            return Err(GroupMismatch::new_no_group(expected));
        };
        if actual != expected {
            return Err(GroupMismatch::new(expected, actual));
        }
        self.account_current_group_box_refs();
        let (payloads, meaning_changed) = self
            .env
            .leave_group_with_kind_observing_meanings(expected)?;
        self.code_tables.leave_group();
        if meaning_changed {
            self.bump_meaning_generation();
            #[cfg(feature = "profiling-stats")]
            crate::measurement::record_meaning_cache_invalidation(
                crate::measurement::MeaningCacheInvalidation::GroupExit,
            );
        }
        Ok(payloads)
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

    pub fn set_count(&mut self, index: u16, value: i32) {
        self.env.set_count(index, value);
    }

    #[must_use]
    pub fn count(&self, index: u16) -> i32 {
        self.env.count(index)
    }

    pub fn set_count_global(&mut self, index: u16, value: i32) {
        self.env.set_count_global(index, value);
    }

    pub fn set_dimen(&mut self, index: u16, value: Scaled) {
        self.env.set_dimen(index, value);
    }

    #[must_use]
    pub fn dimen(&self, index: u16) -> Scaled {
        self.env.dimen(index)
    }

    pub fn set_dimen_global(&mut self, index: u16, value: Scaled) {
        self.env.set_dimen_global(index, value);
    }

    pub fn set_skip(&mut self, index: u16, value: GlueId) {
        self.assert_live_glue(value);
        self.env.set_skip(index, value);
    }

    #[must_use]
    pub fn skip(&self, index: u16) -> GlueId {
        self.resolve_stored_glue(self.env.skip(index))
    }

    pub fn set_skip_global(&mut self, index: u16, value: GlueId) {
        self.assert_live_glue(value);
        self.env.set_skip_global(index, value);
    }

    pub fn set_muskip(&mut self, index: u16, value: GlueId) {
        self.assert_live_glue(value);
        self.env.set_muskip(index, value);
    }

    #[must_use]
    pub fn muskip(&self, index: u16) -> GlueId {
        self.resolve_stored_glue(self.env.muskip(index))
    }

    pub fn set_muskip_global(&mut self, index: u16, value: GlueId) {
        self.assert_live_glue(value);
        self.env.set_muskip_global(index, value);
    }

    pub fn set_toks(&mut self, index: u16, value: TokenListId) {
        self.assert_live_token_list(value);
        self.env.set_toks(index, value);
    }

    #[must_use]
    pub fn toks(&self, index: u16) -> TokenListId {
        self.resolve_stored_token_list(self.env.toks(index))
    }

    pub fn set_toks_global(&mut self, index: u16, value: TokenListId) {
        self.assert_live_token_list(value);
        self.env.set_toks_global(index, value);
    }

    pub fn set_box_reg(&mut self, index: u16, value: NodeListId) {
        self.write_box_reg(index, Some(value), false);
    }

    pub fn set_box_reg_global(&mut self, index: u16, value: NodeListId) {
        self.write_box_reg(index, Some(value), true);
    }

    pub fn set_box_reg_same_level(&mut self, index: u16, value: NodeListId) {
        self.write_box_reg_same_level(index, Some(value));
    }

    pub fn clear_box_reg(&mut self, index: u16) {
        self.write_box_reg(index, None, false);
    }

    pub fn clear_box_reg_global(&mut self, index: u16) {
        self.write_box_reg(index, None, true);
    }

    pub fn clear_box_reg_same_level(&mut self, index: u16) {
        self.write_box_reg_same_level(index, None);
    }

    #[must_use]
    pub fn box_reg(&self, index: u16) -> Option<NodeListId> {
        self.env.box_reg(index)
    }

    pub fn take_box_reg(&mut self, index: u16) -> Option<NodeListId> {
        let (old, rec) = self.env.take_box_reg(index);
        self.account_box_write(old, rec);
        old
    }

    pub fn take_box_reg_same_level(&mut self, index: u16) -> Option<NodeListId> {
        let (old, rec) = self.env.take_box_reg_same_level(index);
        self.account_box_write(old, rec);
        old
    }

    pub fn set_int_param(&mut self, param: IntParam, value: i32) {
        self.env.set_int_param(param, value);
    }

    pub fn set_int_param_global(&mut self, param: IntParam, value: i32) {
        self.env.set_int_param_global(param, value);
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
    pub fn set_last_badness(&mut self, value: i32) {
        self.set_int_param_global(IntParam::LAST_BADNESS, value);
    }

    /// Reads TeX's current `\mag` parameter.
    #[must_use]
    pub fn mag(&self) -> i32 {
        self.int_param(IntParam::MAG)
    }

    /// Sets TeX's local `\mag` parameter.
    pub fn set_mag(&mut self, value: i32) {
        self.set_int_param(IntParam::MAG, value);
    }

    /// Sets TeX's global `\mag` parameter.
    pub fn set_mag_global(&mut self, value: i32) {
        self.set_int_param_global(IntParam::MAG, value);
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
    pub fn prepare_mag(&mut self) -> (i32, Option<PrepareMagDiagnostic>) {
        let attempted = self.mag();
        let (effective, diagnostic) = if !(1..=32_768).contains(&attempted) {
            self.set_mag_global(1000);
            (
                1000,
                Some(PrepareMagDiagnostic::IllegalMagnification { attempted }),
            )
        } else if attempted != 1000 {
            match self.prepared_mag {
                Some(retained) if retained != attempted => {
                    self.set_mag_global(retained);
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
        (effective, diagnostic)
    }

    /// Reads TeX's current `\endlinechar` parameter.
    #[must_use]
    pub fn endlinechar(&self) -> i32 {
        self.int_param(IntParam::END_LINE_CHAR)
    }

    pub fn set_dimen_param(&mut self, param: DimenParam, value: Scaled) {
        self.env.set_dimen_param(param, value);
    }

    pub fn set_dimen_param_global(&mut self, param: DimenParam, value: Scaled) {
        self.env.set_dimen_param_global(param, value);
    }

    #[must_use]
    pub fn dimen_param(&self, param: DimenParam) -> Scaled {
        self.env.dimen_param(param)
    }

    pub fn set_glue_param(&mut self, param: GlueParam, value: GlueId) {
        self.assert_live_glue(value);
        self.env.set_glue_param(param, value);
    }

    #[must_use]
    pub fn glue_param(&self, param: GlueParam) -> GlueId {
        self.resolve_stored_glue(self.env.glue_param(param))
    }

    pub fn set_glue_param_global(&mut self, param: GlueParam, value: GlueId) {
        self.assert_live_glue(value);
        self.env.set_glue_param_global(param, value);
    }

    pub fn set_tok_param(&mut self, param: TokParam, value: TokenListId) {
        self.assert_live_token_list(value);
        self.env.set_tok_param(param, value);
    }

    #[must_use]
    pub fn tok_param(&self, param: TokParam) -> TokenListId {
        self.resolve_stored_token_list(self.env.tok_param(param))
    }

    pub fn set_tok_param_global(&mut self, param: TokParam, value: TokenListId) {
        self.assert_live_token_list(value);
        self.env.set_tok_param_global(param, value);
    }

    /// Takes a checkpoint for the rollback-coupled store tuple.
    ///
    /// Most fields remain O(1) marks/roots. The hyphenation table is cloned in
    /// v1 because pattern loading is rare and rollback soundness is more
    /// important than a premature journal for this INITEX-style state.
    #[must_use]
    pub(crate) fn checkpoint(&mut self) -> StoreSnapshot {
        StoreSnapshot {
            owner: self.owner.snapshot_owner(),
            env_snapshot: self.env.checkpoint(),
            interner_mark: self.interner.watermark(),
            token_mark: self.tokens.watermark(),
            provenance_mark: self.provenance.watermark(),
            source_map_mark: self.source_map.watermark(),
            macro_mark: self.macros.watermark(),
            glue_mark: self.glue.watermark(),
            font_mark: self.fonts.watermark(),
            node_mark: self.nodes.watermark(),
            survivor_pin_mark: self.survivor_pins.len(),
            code_tables_snapshot: self.code_tables.checkpoint(),
            hyphenation: self.hyphenation.clone(),
            prepared_mag: self.prepared_mag,
            last_loaded_font: self.last_loaded_font,
        }
    }

    /// Marks the start of node allocations owned by one shipout operation.
    #[must_use]
    pub(crate) fn shipout_node_mark(&self) -> ShipoutNodeMark {
        ShipoutNodeMark {
            owner: self.owner.snapshot_owner(),
            node_mark: self.nodes.watermark(),
            survivor_pin_mark: self.survivor_pins.len(),
        }
    }

    /// Releases epoch nodes allocated for a completed shipout page.
    pub(crate) fn release_shipout_nodes(&mut self, mark: ShipoutNodeMark) {
        assert_eq!(
            mark.owner,
            self.owner.snapshot_owner(),
            "shipout node mark belongs to a different Stores instance"
        );
        assert!(
            mark.survivor_pin_mark <= self.survivor_pins.len(),
            "shipout node mark is invalidated by an enclosing survivor-pin release"
        );
        self.release_survivor_pins_to(mark.survivor_pin_mark);
        self.nodes.truncate_to(mark.node_mark);
    }

    /// Rolls all stores back to `snapshot` as one atomic tuple.
    pub(crate) fn rollback(&mut self, snapshot: &StoreSnapshot) {
        self.assert_valid_snapshot(snapshot);
        self.release_survivor_pins_to(snapshot.survivor_pin_mark);
        self.account_rollback_box_refs(snapshot.env_snapshot);
        self.env.rollback_to(snapshot.env_snapshot);
        self.interner.truncate_to(snapshot.interner_mark);
        self.tokens.truncate_to(snapshot.token_mark);
        self.provenance.truncate_to(snapshot.provenance_mark);
        self.source_map.truncate_to(snapshot.source_map_mark);
        self.macros.truncate_to(snapshot.macro_mark);
        self.glue.truncate_to(snapshot.glue_mark);
        self.fonts.truncate_to(snapshot.font_mark);
        self.nodes.truncate_to(snapshot.node_mark);
        self.code_tables
            .rollback_to(snapshot.code_tables_snapshot.clone());
        self.hyphenation = snapshot.hyphenation.clone();
        self.prepared_mag = snapshot.prepared_mag;
        self.last_loaded_font = snapshot.last_loaded_font;
        self.bump_meaning_generation();
        #[cfg(feature = "profiling-stats")]
        crate::measurement::record_meaning_cache_invalidation(
            crate::measurement::MeaningCacheInvalidation::Rollback,
        );
        // The cache is derived from the checkpoint timeline rather than part
        // of semantic state. Rebuild baselines lazily from the restored
        // journal slice instead of adding it to the O(1) snapshot tuple.
        self.semantic_hash_cache.clear();
    }

    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn testing_clear_semantic_hash_cache(&mut self) {
        self.semantic_hash_cache.clear();
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

    pub(crate) fn generation_retained_bytes(&self) -> usize {
        // A live accepted generation may legitimately retain survivor pins;
        // format capture forbids them because formats have a stricter job-start
        // contract. Use the serialized-size proxy only when that contract is
        // satisfied instead of turning retention accounting into a panic.
        let serialized = if self.survivor_pins.is_empty() {
            self.encode_frozen_format()
                .map_or(0, |format| format.payload_len())
        } else {
            0
        };
        let provenance = self.provenance_stats().retained_bytes();
        let source_map = self.source_map.stats().retained_bytes;
        let source_fragment_metadata = self.source_fragments.metadata_retained_bytes();
        let nodes = self
            .nodes
            .retained_payload_bytes()
            .saturating_add(self.survivors.retained_payload_bytes())
            .saturating_add(
                self.survivor_pins
                    .capacity()
                    .saturating_mul(mem::size_of::<NodeListId>()),
            );
        std::mem::size_of::<Self>()
            .saturating_add(serialized)
            .saturating_add(self.env.journal_retained_bytes())
            .saturating_add(provenance)
            .saturating_add(source_map)
            .saturating_add(source_fragment_metadata)
            .saturating_add(nodes)
    }

    /// Verifies the shadow mirror against real environment storage.
    #[cfg(feature = "shadow")]
    pub fn verify_shadow(&self) {
        self.env.verify_shadow();
    }

    /// Returns a content-only hash of all semantic state currently in Stores.
    #[cfg(any(test, feature = "testing", feature = "shadow"))]
    #[must_use]
    pub fn testing_state_hash(&self) -> u64 {
        let mut hasher = ahash::AHasher::default();
        self.testing_hash_env_by_content(&mut hasher);
        self.interner.len().hash(&mut hasher);
        for raw in 0..self.interner.len() {
            let symbol = self
                .interner
                .symbol_at_slot(raw as u32)
                .expect("live interner slot should have a compact key");
            self.interner.kind(symbol).hash(&mut hasher);
            self.interner.resolve(symbol).hash(&mut hasher);
        }
        let token_mark = self.tokens.watermark();
        token_mark.spans.hash(&mut hasher);
        for raw in 0..token_mark.spans {
            let id = self.resolve_stored_token_list(TokenListId::new(raw));
            let tokens = self.tokens.get(id);
            tokens.len().hash(&mut hasher);
            for &token in tokens {
                self.testing_hash_token(token, &mut hasher);
            }
        }
        self.glue.testing_state_hash().hash(&mut hasher);
        self.fonts.testing_state_hash(&mut hasher);
        self.testing_hash_all_epoch_nodes(&mut hasher);
        self.code_tables.testing_hash_content(&mut hasher);
        self.prepared_mag.hash(&mut hasher);
        self.last_loaded_font.hash(&mut hasher);
        hasher.finish()
    }

    #[cfg(any(test, feature = "testing", feature = "shadow"))]
    fn testing_hash_env_by_content(&self, hasher: &mut impl Hasher) {
        self.env.for_each_semantic_non_default_word(|cell, word| {
            cell.bank().hash(hasher);
            match cell.bank() {
                BankTag::Meaning => {
                    let symbol = self
                        .interner
                        .symbol_at_slot(cell.index())
                        .and_then(|symbol| self.interner.resolve_stored(symbol))
                        .expect("meaning slot should name a live symbol");
                    self.interner.kind_id(symbol).hash(hasher);
                    self.interner.resolve_id(symbol).hash(hasher);
                    word.hash(hasher);
                }
                BankTag::Box => self.testing_hash_box_word(word, hasher),
                BankTag::CurrentFont => {
                    (word as u32).hash(hasher);
                    match self.env.current_font_symbol() {
                        Some(symbol) => {
                            1_u8.hash(hasher);
                            let symbol = self.resolve_stored_symbol(symbol);
                            self.interner.kind_id(symbol).hash(hasher);
                            self.interner.resolve_id(symbol).hash(hasher);
                        }
                        None => 0_u8.hash(hasher),
                    }
                }
                _ => {
                    cell.index().hash(hasher);
                    word.hash(hasher);
                }
            }
        });
        for &token in self.env.testing_aftergroup_payloads() {
            self.testing_hash_token(token, hasher);
        }
        match self.env.testing_afterassignment() {
            Some(token) => {
                1_u8.hash(hasher);
                self.testing_hash_token(token, hasher);
            }
            None => 0_u8.hash(hasher),
        }
    }

    #[cfg(any(test, feature = "testing", feature = "shadow"))]
    fn testing_hash_token(&self, token: Token, hasher: &mut impl Hasher) {
        core::mem::discriminant(&token).hash(hasher);
        match token {
            Token::Char { ch, cat } => {
                ch.hash(hasher);
                cat.hash(hasher);
            }
            Token::Cs(symbol) => {
                let symbol = self.resolve_stored_symbol(symbol);
                self.interner.kind_id(symbol).hash(hasher);
                self.interner.resolve_id(symbol).hash(hasher);
            }
            Token::Param(slot) => slot.hash(hasher),
            Token::Frozen(kind) => kind.hash(hasher),
        }
    }

    fn assert_valid_snapshot(&self, snapshot: &StoreSnapshot) {
        assert_eq!(
            snapshot.owner,
            self.owner.snapshot_owner(),
            "Stores snapshot belongs to a different Stores instance"
        );
        assert_eq!(
            snapshot.env_snapshot.group_depth(),
            self.env.group_depth(),
            "Stores snapshots are invalidated by exiting a group that encloses them"
        );
        assert!(
            snapshot.env_snapshot.journal_pos() <= self.env.current_journal_pos(),
            "Stores snapshots are invalidated by journal truncation before their checkpoint position"
        );
        assert!(
            snapshot.survivor_pin_mark <= self.survivor_pins.len(),
            "Stores snapshots are invalidated by an enclosing survivor-pin release"
        );
    }

    fn release_survivor_pins_to(&mut self, mark: usize) {
        while self.survivor_pins.len() > mark {
            let id = self
                .survivor_pins
                .pop()
                .expect("survivor pin length was checked");
            self.survivors.dec_ref(id);
        }
    }

    #[cfg(any(test, feature = "testing", feature = "shadow"))]
    fn testing_hash_box_word(&self, word: u64, hasher: &mut impl Hasher) {
        match NodeListId::decode_box_word(word) {
            Some(id) => self.testing_hash_node_list_content_bounded(id, hasher, 0),
            None => 0_u8.hash(hasher),
        }
    }

    #[cfg(any(test, feature = "testing", feature = "shadow"))]
    fn testing_hash_all_epoch_nodes(&self, hasher: &mut impl Hasher) {
        for node in self.nodes.testing_all_nodes() {
            self.testing_hash_node_content_bounded(&node.to_owned(), hasher, 0);
        }
    }

    #[cfg(any(test, feature = "testing", feature = "shadow"))]
    pub fn testing_hash_node_list_content(&self, id: NodeListId, hasher: &mut impl Hasher) {
        self.testing_hash_node_list_content_bounded(id, hasher, 0);
    }

    #[cfg(any(test, feature = "testing", feature = "shadow"))]
    fn testing_hash_node_list_content_bounded(
        &self,
        id: NodeListId,
        hasher: &mut impl Hasher,
        depth: usize,
    ) {
        assert!(
            depth <= TESTING_NODE_HASH_MAX_DEPTH,
            "testing node hash exceeded maximum node-list nesting depth"
        );
        1_u8.hash(hasher);
        for node in self.nodes(id) {
            self.testing_hash_node_content_bounded(&node.to_owned(), hasher, depth);
        }
    }

    #[cfg(any(test, feature = "testing", feature = "shadow"))]
    fn testing_hash_node_content_bounded(
        &self,
        node: &Node,
        hasher: &mut impl Hasher,
        depth: usize,
    ) {
        std::mem::discriminant(node).hash(hasher);
        match node {
            Node::Char { font, ch, .. } => {
                font.raw().hash(hasher);
                ch.hash(hasher);
            }
            Node::Kern { amount, kind } => {
                amount.raw().hash(hasher);
                kind.hash(hasher);
            }
            Node::Glue { spec, kind, leader } => {
                self.glue(*spec).hash(hasher);
                kind.hash(hasher);
                match leader {
                    Some(leader) => format!("{leader:?}").hash(hasher),
                    None => 0_u8.hash(hasher),
                }
            }
            Node::Penalty(value) => value.hash(hasher),
            Node::HList(box_node) | Node::VList(box_node) => {
                box_node.width.raw().hash(hasher);
                box_node.height.raw().hash(hasher);
                box_node.depth.raw().hash(hasher);
                box_node.shift.raw().hash(hasher);
                box_node.glue_set.numerator().hash(hasher);
                box_node.glue_set.denominator().hash(hasher);
                box_node.glue_sign.hash(hasher);
                box_node.glue_order.hash(hasher);
                self.testing_hash_node_list_content_bounded(box_node.children, hasher, depth + 1);
            }
            Node::MathOn(_)
            | Node::MathOff(_)
            | Node::Direction(_)
            | Node::MathNoad(_)
            | Node::FractionNoad(_)
            | Node::MathStyle(_)
            | Node::MathChoice(_)
            | Node::MathList(_)
            | Node::Nonscript
            | Node::Lig { .. }
            | Node::Rule { .. }
            | Node::Unset(_)
            | Node::Disc { .. }
            | Node::Mark { .. }
            | Node::Ins { .. }
            | Node::Whatsit(_)
            | Node::Adjust(_) => {
                // TODO(M3): replace this test/shadow fallback before using
                // node content hashes for convergence. Debug formatting
                // includes child NodeListId spans for some variants, which is
                // deterministic under replay but not semantic content.
                format!("{node:?}").hash(hasher);
            }
        }
    }

    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn testing_live_survivor_slot_count(&self) -> usize {
        self.survivors.testing_live_slot_count()
    }

    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn testing_epoch_node_count(&self) -> usize {
        self.nodes.testing_node_count()
    }

    /// The epoch-clone facility has been removed; register-read paths must
    /// keep this structural regression counter at zero.
    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub const fn testing_epoch_clone_counts(&self) -> (u64, u64) {
        (0, 0)
    }

    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn testing_survivor_refcount(&self, id: NodeListId) -> u32 {
        self.survivors.testing_refcount(id)
    }

    #[cfg(any(test, feature = "testing"))]
    #[allow(dead_code)] // Exposed through Universe when the retention budget lands.
    #[must_use]
    pub fn testing_survivor_pin_count(&self) -> usize {
        self.survivor_pins.len()
    }

    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn testing_survivor_pin_retained_bytes(&self) -> usize {
        self.survivor_pins.capacity() * mem::size_of::<NodeListId>()
    }

    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn testing_survivor_recycled_buffer_uses(&self) -> usize {
        self.survivors.testing_recycled_buffer_uses()
    }

    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn testing_survivor_root_slot_count(&self) -> usize {
        self.survivors.testing_root_slot_count()
    }

    #[cfg(feature = "profiling-stats")]
    pub(crate) fn node_memory_columns(&self) -> Vec<NodeMemoryColumn> {
        let mut columns = self.nodes.memory_columns();
        columns.extend(self.survivors.memory_columns());
        columns
    }
}

impl Default for Stores {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
#[cfg(any(test, feature = "testing", feature = "shadow"))]
use crate::cell::BankTag;
