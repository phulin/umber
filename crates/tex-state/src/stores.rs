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
    CharMetrics, ExtensibleRecipe, FontMetrics, FontStore, FontStoreMark, LigKernChar,
    LigKernCommand, LigKernIter, LoadedFont, MissingCharacter, NULL_FONT,
};
use crate::glue::{GlueSpec, GlueStore, GlueStoreMark};
use crate::hyphenation::{ExceptionSpec, HyphenationTable, PatternSpec};
use crate::ids::{FontId, GlueId, MacroDefinitionId, NodeListId, OriginListId, TokenListId};
use crate::input::SourceId;
use crate::interner::{ControlSequenceKind, Interner, InternerError, InternerMark, Symbol};
use crate::macro_store::{MacroDefinitionProvenance, MacroMeaning, MacroStore, MacroStoreMark};
use crate::math::MathFontSize;
use crate::meaning::Meaning;
use crate::node::Node;
use crate::node_arena::{NodeArena, NodeArenaMark, NodeList, NodeListBuilder};
use crate::provenance::{
    InsertedOrigin, InsertedOriginKind, MacroInvocationOrigin, OriginListBuilder, OriginRecord,
    ProvenanceStats, ProvenanceStore, ProvenanceStoreMark, SourceOrigin, SynthesizedOrigin,
    SynthesizedOriginKind, SyntheticOrigin, SyntheticOriginKind,
};
use crate::scaled::Scaled;
use crate::source_map::{
    GeneratedSource, SourceBacking, SourceDescriptor, SourceMap, SourceMapError, SourceMapMark,
    SourcePos, SourceRegion, SourceSpan,
};
use crate::survivor::SurvivorArena;
use crate::token::{Catcode, OriginId, Token};
use crate::token_store::{TokenListBuilder, TokenStore, TokenStoreMark};
use std::hash::BuildHasher;
#[cfg(any(test, feature = "testing", feature = "shadow"))]
use std::hash::{Hash, Hasher};
use std::mem;

mod handles;
mod state_hash;

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
    code_tables_snapshot: CodeTablesSnapshot,
    hyphenation: HyphenationTable,
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
    let state = std::collections::hash_map::RandomState::new();
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
    macros: MacroStore,
    glue: GlueStore,
    fonts: FontStore,
    nodes: NodeArena,
    survivors: SurvivorArena,
    code_tables: CodeTables,
    hyphenation: HyphenationTable,
    prepared_mag: Option<i32>,
    last_loaded_font: FontId,
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
    /// Only the most recently loaded font may grow its parameter table.
    CannotGrow {
        font: FontId,
        number: u16,
        current_len: u16,
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
            macros: self.macros.clone(),
            glue: self.glue.clone(),
            fonts: self.fonts.clone(),
            nodes: self.nodes.clone(),
            survivors: self.survivors.clone(),
            code_tables: self.code_tables.clone(),
            hyphenation: self.hyphenation.clone(),
            prepared_mag: self.prepared_mag,
            last_loaded_font: self.last_loaded_font,
            semantic_hash_cache: self.semantic_hash_cache.clone(),
        }
    }
}

impl Stores {
    pub(crate) fn env_group_depth(&self) -> u32 {
        self.env.group_depth()
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
            macros: MacroStore::new(),
            glue: GlueStore::new(),
            fonts: FontStore::new(),
            nodes: NodeArena::new(),
            survivors: SurvivorArena::new(),
            code_tables: CodeTables::new(),
            hyphenation: HyphenationTable::new(),
            prepared_mag: None,
            last_loaded_font: NULL_FONT,
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
        self.hyphenation.add_pattern(pattern);
    }

    pub fn add_hyphenation_exception(&mut self, exception: ExceptionSpec) {
        self.hyphenation.add_exception(exception);
    }

    #[must_use]
    pub fn hyphen_positions(&self, word: &str, left_min: usize, right_min: usize) -> Vec<usize> {
        self.hyphenation.hyphen_positions(word, left_min, right_min)
    }

    #[must_use]
    pub fn hyphenation_exception(&self, word: &str) -> Option<&[usize]> {
        self.hyphenation.exception(word)
    }

    /// Returns the meaning for a live control-sequence symbol.
    #[must_use]
    pub fn meaning(&self, symbol: Symbol) -> Meaning {
        self.assert_live_symbol(symbol);
        self.env.get(symbol)
    }

    /// Sets the local meaning for a live control-sequence symbol.
    pub fn set_meaning(&mut self, symbol: Symbol, meaning: Meaning) {
        self.assert_live_symbol(symbol);
        self.assert_live_macro_definition_in_meaning(meaning);
        self.assert_live_font_in_meaning(meaning);
        self.env.set(symbol, meaning);
    }

    /// Interns a control-sequence name and gives a previously undefined name
    /// TeX's `\csname`-created `\relax` meaning.
    pub fn intern_relaxed_control_sequence(&mut self, name: &str) -> Symbol {
        let symbol = self.intern(name);
        if self.meaning(symbol) == Meaning::Undefined {
            self.set_meaning(symbol, Meaning::Relax);
        }
        symbol
    }

    /// Sets the global meaning for a live control-sequence symbol.
    pub fn set_meaning_global(&mut self, symbol: Symbol, meaning: Meaning) {
        self.assert_live_symbol(symbol);
        self.assert_live_macro_definition_in_meaning(meaning);
        self.assert_live_font_in_meaning(meaning);
        self.env.set_global(symbol, meaning);
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
        self.macros
            .intern_with_provenance(macro_meaning, provenance)
    }

    /// Reads a live frozen macro definition.
    #[must_use]
    pub fn macro_definition(&self, id: MacroDefinitionId) -> MacroMeaning {
        self.assert_live_macro_definition(id);
        self.macros.get(id)
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
    pub fn set_macro_meaning(&mut self, symbol: Symbol, macro_meaning: MacroMeaning) {
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
        symbol: Symbol,
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
    pub fn set_macro_meaning_global(&mut self, symbol: Symbol, macro_meaning: MacroMeaning) {
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
        symbol: Symbol,
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
    pub fn macro_meaning(&self, symbol: Symbol) -> Option<MacroMeaning> {
        match self.meaning(symbol) {
            Meaning::Macro { definition, .. } => Some(self.macro_definition(definition)),
            _ => None,
        }
    }

    /// Interns a control-sequence name in the owned interner.
    pub fn intern(&mut self, name: &str) -> Symbol {
        self.try_intern(name)
            .expect("control-sequence symbol capacity exceeded")
    }

    /// Interns an active-character control sequence in its TeX82 namespace.
    pub fn intern_active_character(&mut self, ch: char) -> Symbol {
        self.interner
            .intern_active(ch)
            .expect("control-sequence symbol capacity exceeded")
    }

    /// Interns a control-sequence name, reporting packed-token capacity exhaustion.
    pub(crate) fn try_intern(&mut self, name: &str) -> Result<Symbol, InternerError> {
        self.interner.intern(name)
    }

    /// Returns the live symbol for an already-interned control-sequence name.
    #[must_use]
    pub fn symbol(&self, name: &str) -> Option<Symbol> {
        self.interner.get(name)
    }

    /// Returns the live symbol for an already-interned active character.
    #[must_use]
    pub fn active_character_symbol(&self, ch: char) -> Option<Symbol> {
        self.interner.get_active(ch)
    }

    /// Resolves a live control-sequence symbol.
    #[must_use]
    pub fn resolve(&self, symbol: Symbol) -> &str {
        self.assert_live_symbol(symbol);
        self.interner.resolve(symbol)
    }

    /// Returns the TeX control-sequence namespace of a live symbol.
    #[must_use]
    pub fn control_sequence_kind(&self, symbol: Symbol) -> ControlSequenceKind {
        self.assert_live_symbol(symbol);
        self.interner.kind(symbol)
    }

    /// Creates a fresh owned scratch token-list builder.
    #[must_use]
    pub fn token_list_builder(&self) -> TokenListBuilder {
        TokenStore::builder()
    }

    /// Interns a frozen token-list value in the owned token store.
    pub fn intern_token_list(&mut self, tokens: &[Token]) -> TokenListId {
        self.tokens.intern(tokens)
    }

    /// Interns the current token-list builder value and clears it for reuse.
    pub fn finish_token_list(&mut self, builder: &mut TokenListBuilder) -> TokenListId {
        builder.finish(&mut self.tokens)
    }

    /// Reads a live frozen token list.
    #[must_use]
    pub fn tokens(&self, id: TokenListId) -> &[Token] {
        self.assert_live_token_list(id);
        self.tokens.get(id)
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

    /// Allocates a macro-invocation origin.
    pub fn macro_invocation_origin(
        &mut self,
        definition: MacroDefinitionId,
        invocation: OriginId,
        definition_origin: OriginId,
    ) -> OriginId {
        self.assert_live_macro_definition(definition);
        self.assert_live_origin(invocation);
        self.assert_live_origin(definition_origin);
        self.provenance
            .allocate(OriginRecord::MacroInvocation(MacroInvocationOrigin::new(
                definition,
                invocation,
                definition_origin,
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
        self.assert_live_origin_list(id);
        self.provenance.list(id)
    }

    /// Reads an origin-list span if it is still live on this timeline.
    #[must_use]
    pub fn origin_list_if_live(&self, id: OriginListId) -> Option<&[OriginId]> {
        self.provenance
            .contains_list(id)
            .then(|| self.provenance.list(id))
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
    ) -> Result<SourcePos, SourceMapError> {
        self.source_map.register(source, descriptor)
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
        self.assert_live_glue(id);
        self.glue.get(id)
    }

    /// Interns a loaded immutable font and initializes its Env-side banks.
    pub fn intern_font(&mut self, font: LoadedFont) -> FontId {
        let parameter_count = u16::try_from(font.parameters().len())
            .expect("loaded font has more than u16::MAX parameters");
        let parameters = font.parameters().to_vec();
        let id = self.fonts.intern(font);
        if self.env.font_param_len(id) == 0 && id != NULL_FONT {
            self.initialize_font_banks(id, parameter_count, &parameters);
        }
        self.last_loaded_font = id;
        id
    }

    /// Interns a font and records the control sequence TeX uses for its
    /// identifier token (the `font_id_text` associated with the font).
    pub fn intern_font_with_identifier(&mut self, font: LoadedFont, symbol: Symbol) -> FontId {
        self.assert_live_symbol(symbol);
        let id = self.intern_font(font);
        self.fonts.set_identifier(id, symbol);
        id
    }

    /// Reads a live immutable font record.
    #[must_use]
    pub fn font(&self, id: FontId) -> &LoadedFont {
        self.assert_live_font(id);
        self.fonts.get(id)
    }

    #[must_use]
    pub fn font_name(&self, id: FontId) -> String {
        self.font(id).fontname_text()
    }

    #[must_use]
    pub fn font_identifier_symbol(&self, id: FontId) -> Option<Symbol> {
        self.assert_live_font(id);
        let symbol = self.fonts.identifier(id)?;
        self.assert_live_symbol(symbol);
        Some(symbol)
    }

    pub fn set_font_identifier_symbol(&mut self, id: FontId, symbol: Symbol) {
        self.assert_live_font(id);
        self.assert_live_symbol(symbol);
        self.fonts.set_identifier(id, symbol);
    }

    #[must_use]
    pub fn font_metrics(&self, font: FontId) -> &FontMetrics {
        self.font(font).metrics()
    }

    #[must_use]
    pub fn font_char_exists(&self, font: FontId, code: u8) -> bool {
        self.font(font).metrics().char_exists(code)
    }

    #[must_use]
    pub fn font_char_metrics(&self, font: FontId, code: u8) -> Option<CharMetrics> {
        self.font(font).metrics().character(code)
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
        self.font(font).metrics().lig_kern_command(left, right)
    }

    #[must_use]
    pub fn extensible_recipe(&self, font: FontId, code: u8) -> Option<ExtensibleRecipe> {
        self.font(font).metrics().extensible_recipe(code)
    }

    #[must_use]
    pub fn font_parameter(&self, font: FontId, number: u16) -> Scaled {
        self.font_dimen(font, number)
    }

    #[must_use]
    pub fn current_font(&self) -> FontId {
        let id = self.env.current_font();
        self.assert_live_font(id);
        id
    }

    #[must_use]
    pub fn current_font_symbol(&self) -> Option<Symbol> {
        let symbol = self.env.current_font_symbol()?;
        self.assert_live_symbol(symbol);
        Some(symbol)
    }

    pub fn set_current_font(&mut self, id: FontId) {
        self.assert_live_font(id);
        self.env.set_current_font(id);
    }

    pub fn set_current_font_global(&mut self, id: FontId) {
        self.assert_live_font(id);
        self.env.set_current_font_global(id);
    }

    pub fn set_current_font_selector(&mut self, symbol: Symbol, id: FontId) {
        self.assert_live_symbol(symbol);
        self.assert_live_font(id);
        self.env.set_current_font_selector(symbol, id);
    }

    pub fn set_current_font_selector_global(&mut self, symbol: Symbol, id: FontId) {
        self.assert_live_symbol(symbol);
        self.assert_live_font(id);
        self.env.set_current_font_selector_global(symbol, id);
    }

    #[must_use]
    pub fn math_family_font(&self, size: MathFontSize, family: u8) -> FontId {
        let id = self.env.math_family_font(size, family);
        self.assert_live_font(id);
        id
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
    pub fn font_dimen(&self, font: FontId, number: u16) -> Scaled {
        self.assert_live_font(font);
        self.env.font_dimen(font, number)
    }

    #[must_use]
    pub fn font_parameter_count(&self, font: FontId) -> u16 {
        self.assert_live_font(font);
        self.env.font_param_len(font)
    }

    pub fn set_font_dimen(
        &mut self,
        font: FontId,
        number: u16,
        value: Scaled,
        global: bool,
    ) -> Result<(), FontParameterError> {
        self.prepare_font_dimen_write(font, number, global)?;
        if global {
            self.env.set_font_dimen_global(font, number, value);
        } else {
            self.env.set_font_dimen(font, number, value);
        }
        Ok(())
    }

    #[must_use]
    pub fn font_hyphen_char(&self, font: FontId) -> i32 {
        self.assert_live_font(font);
        self.env.font_hyphen_char(font)
    }

    pub fn set_font_hyphen_char(&mut self, font: FontId, value: i32, global: bool) {
        self.assert_live_font(font);
        if global {
            self.env.set_font_hyphen_char_global(font, value);
        } else {
            self.env.set_font_hyphen_char(font, value);
        }
    }

    #[must_use]
    pub fn font_skew_char(&self, font: FontId) -> i32 {
        self.assert_live_font(font);
        self.env.font_skew_char(font)
    }

    pub fn set_font_skew_char(&mut self, font: FontId, value: i32, global: bool) {
        self.assert_live_font(font);
        if global {
            self.env.set_font_skew_char_global(font, value);
        } else {
            self.env.set_font_skew_char(font, value);
        }
    }

    fn initialize_font_banks(&mut self, font: FontId, parameter_count: u16, parameters: &[Scaled]) {
        self.env.set_font_param_len_global(font, parameter_count);
        for (index, value) in parameters.iter().copied().enumerate() {
            let number = u16::try_from(index + 1).expect("font parameter index exceeds u16");
            self.env.set_font_dimen_global(font, number, value);
        }
        self.env
            .set_font_hyphen_char_global(font, self.env.int_param(IntParam::DEFAULT_HYPHEN_CHAR));
        self.env
            .set_font_skew_char_global(font, self.env.int_param(IntParam::DEFAULT_SKEW_CHAR));
    }

    fn prepare_font_dimen_write(
        &mut self,
        font: FontId,
        number: u16,
        global: bool,
    ) -> Result<(), FontParameterError> {
        self.assert_live_font(font);
        if number == 0 {
            return Err(FontParameterError::Zero);
        }
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
            if global {
                self.env.set_font_param_len_global(font, number);
            } else {
                self.env.set_font_param_len(font, number);
            }
        }
        Ok(())
    }

    /// Creates a fresh owned scratch node-list builder.
    #[must_use]
    pub fn node_list_builder(&self) -> NodeListBuilder {
        NodeArena::builder()
    }

    /// Appends and freezes a node list in the owned epoch arena.
    pub fn freeze_node_list(&mut self, nodes: &[Node]) -> NodeListId {
        self.assert_live_handles_in_nodes(nodes);
        self.nodes.append(nodes)
    }

    /// Freezes the current node-list builder value and clears it for reuse.
    pub fn finish_node_list(&mut self, builder: &mut NodeListBuilder) -> NodeListId {
        self.assert_live_handles_in_nodes(builder.as_slice());
        builder.finish(&mut self.nodes)
    }

    /// Reads a live frozen node list.
    #[must_use]
    pub fn nodes(&self, id: NodeListId) -> NodeList<'_> {
        self.assert_live_node_list(id);
        self.nodes.get(id, &self.survivors)
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
        self.env.push_aftergroup(payload);
    }

    /// Leaves the innermost TeX group and returns its `\aftergroup` payloads.
    #[must_use]
    pub fn leave_group(&mut self) -> Vec<Token> {
        self.account_current_group_box_refs();
        let payloads = self.env.leave_group();
        self.code_tables.leave_group();
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
        let payloads = self.env.leave_group_with_kind(expected)?;
        self.code_tables.leave_group();
        Ok(payloads)
    }

    /// Stores the token to insert after the next assignment.
    pub fn set_afterassignment(&mut self, token: Token) {
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
        let value = self.env.skip(index);
        self.assert_live_glue(value);
        value
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
        let value = self.env.muskip(index);
        self.assert_live_glue(value);
        value
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
        let value = self.env.toks(index);
        self.assert_live_token_list(value);
        value
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
        let value = self.env.glue_param(param);
        self.assert_live_glue(value);
        value
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
        let value = self.env.tok_param(param);
        self.assert_live_token_list(value);
        value
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
        }
    }

    /// Releases epoch nodes allocated for a completed shipout page.
    pub(crate) fn release_shipout_nodes(&mut self, mark: ShipoutNodeMark) {
        assert_eq!(
            mark.owner,
            self.owner.snapshot_owner(),
            "shipout node mark belongs to a different Stores instance"
        );
        self.nodes.truncate_to(mark.node_mark);
    }

    /// Rolls all stores back to `snapshot` as one atomic tuple.
    pub(crate) fn rollback(&mut self, snapshot: &StoreSnapshot) {
        self.assert_valid_snapshot(snapshot);
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
        // The cache is derived from the checkpoint timeline rather than part
        // of semantic state. Rebuild baselines lazily from the restored
        // journal slice instead of adding it to the O(1) snapshot tuple.
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

    /// Verifies the shadow mirror against real environment storage.
    #[cfg(feature = "shadow")]
    pub fn verify_shadow(&self) {
        self.env.verify_shadow();
    }

    /// Returns a content-only hash of all semantic state currently in Stores.
    #[cfg(any(test, feature = "testing", feature = "shadow"))]
    #[must_use]
    pub fn testing_state_hash(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.testing_hash_env_by_content(&mut hasher);
        self.interner.len().hash(&mut hasher);
        for raw in 0..self.interner.len() {
            let symbol = Symbol::new(raw as u32);
            self.interner.kind(symbol).hash(&mut hasher);
            self.interner.resolve(symbol).hash(&mut hasher);
        }
        self.tokens.testing_state_hash().hash(&mut hasher);
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
            cell.hash(hasher);
            if cell.bank() == BankTag::Box {
                self.testing_hash_box_word(word, hasher);
            } else {
                word.hash(hasher);
            }
        });
        self.env.testing_aftergroup_payloads().hash(hasher);
        self.env.testing_afterassignment().hash(hasher);
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
        let len = u32::try_from(self.nodes.testing_node_count())
            .expect("node arena test hash cannot cover more than u32 entries");
        for node in self.nodes.get_epoch(NodeListId::new_epoch(0, len)) {
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
            Node::Char { font, ch } => {
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

    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn testing_survivor_refcount(&self, id: NodeListId) -> u32 {
        self.survivors.testing_refcount(id)
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
