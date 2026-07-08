//! Top-level TeX state timeline.
//!
//! `Universe` is the only public checkpoint/rollback boundary. The older
//! `Stores` aggregate remains as private composition because its facade already
//! enforces handle liveness and couples Env/content/code-table rollback. The
//! public timeline tuple lives here so future World/effect/input state cannot
//! grow a partial rollback API beside the store tuple.

use crate::code_tables::{CodeTableGenerations, DelCode, LcCode, MathCode, SfCode, UcCode};
use crate::env::Env;
use crate::env::banks::{DimenParam, GlueParam, IntParam, TokParam};
use crate::epoch::Epoch;
use crate::font::{
    CharMetrics, ExtensibleRecipe, FontMetrics, LigKernChar, LigKernCommand, LigKernIter,
    LoadedFont, MissingCharacter,
};
use crate::glue::GlueSpec;
use crate::hyphenation::{ExceptionSpec, PatternSpec};
use crate::ids::{FontId, GlueId, MacroDefinitionId, NodeListId, TokenListId};
use crate::input::{
    ConditionKind, ConditionLimb, InputFrameSummary, InputSummary, LexerState, TokenListReplayKind,
};
use crate::interner::Symbol;
use crate::macro_store::MacroMeaning;
use crate::meaning::Meaning;
use crate::node::Node;
use crate::node_arena::NodeListBuilder;
use crate::scaled::Scaled;
use crate::state_hash::{INITIAL_STATE_HASH, StateHasher, combine};
use crate::stores::StoreStateHashCursor;
use crate::stores::{
    FontParameterError, GroupKind, GroupMismatch, PrepareMagDiagnostic, StoreSnapshot, Stores,
};
use crate::token::{Catcode, Token};
use crate::token_store::TokenListBuilder;
use crate::world::{
    EffectRecord, JobClock, PrintSink, ShellEscapePolicy, ShellEscapeRecord, StreamBufState,
    StreamSlot, World, WorldSnapshot, WorldStateHashCursor, install_job_clock_params,
};
use std::hash::BuildHasher;
#[cfg(any(test, feature = "testing", feature = "shadow"))]
use std::hash::{Hash, Hasher};

/// State operations available to TeX's lexer and expansion engine.
///
/// This is intentionally narrower than `Universe`: it permits immutable state
/// reads plus the content/interner mutations that the mouth and gullet are
/// semantically allowed to perform, but it does not expose Env, register, box,
/// code-table, font-parameter, grouping, snapshot, input-file reads, or World
/// mutation APIs.
pub trait ExpansionState {
    fn catcode(&self, ch: char) -> Catcode;
    fn lccode(&self, ch: char) -> LcCode;
    fn uccode(&self, ch: char) -> UcCode;
    fn sfcode(&self, ch: char) -> SfCode;
    fn mathcode(&self, ch: char) -> MathCode;
    fn delcode(&self, ch: char) -> DelCode;
    fn meaning(&self, symbol: Symbol) -> Meaning;
    fn macro_definition(&self, id: MacroDefinitionId) -> MacroMeaning;
    fn macro_meaning(&self, symbol: Symbol) -> Option<MacroMeaning>;
    fn intern_relaxed_control_sequence(&mut self, name: &str) -> Symbol;
    fn intern(&mut self, name: &str) -> Symbol;
    fn symbol(&self, name: &str) -> Option<Symbol>;
    fn resolve(&self, symbol: Symbol) -> &str;
    fn token_list_builder(&self) -> TokenListBuilder;
    fn intern_token_list(&mut self, tokens: &[Token]) -> TokenListId;
    fn finish_token_list(&mut self, builder: &mut TokenListBuilder) -> TokenListId;
    fn tokens(&self, id: TokenListId) -> &[Token];
    fn intern_glue(&mut self, spec: GlueSpec) -> GlueId;
    fn glue(&self, id: GlueId) -> GlueSpec;
    fn font_name(&self, id: FontId) -> String;
    fn font_parameter(&self, font: FontId, number: u16) -> Scaled;
    fn font_dimen(&self, font: FontId, number: u16) -> Scaled;
    fn font_hyphen_char(&self, font: FontId) -> i32;
    fn font_skew_char(&self, font: FontId) -> i32;
    fn current_font(&self) -> FontId;
    fn current_font_symbol(&self) -> Option<Symbol>;
    fn nodes(&self, id: NodeListId) -> &[Node];
    fn box_dimension(&self, index: u16, dimension: BoxDimension) -> Option<Scaled>;
    fn count(&self, index: u16) -> i32;
    fn dimen(&self, index: u16) -> Scaled;
    fn skip(&self, index: u16) -> GlueId;
    fn muskip(&self, index: u16) -> GlueId;
    fn toks(&self, index: u16) -> TokenListId;
    fn box_reg(&self, index: u16) -> Option<NodeListId>;
    fn int_param(&self, param: IntParam) -> i32;
    fn mag(&self) -> i32;
    fn prepared_mag(&self) -> Option<i32>;
    fn prepare_mag(&mut self) -> (i32, Option<PrepareMagDiagnostic>);
    fn endlinechar(&self) -> i32;
    fn dimen_param(&self, param: DimenParam) -> Scaled;
    fn glue_param(&self, param: GlueParam) -> GlueId;
    fn tok_param(&self, param: TokParam) -> TokenListId;
    fn input_stream_eof(&self, stream: StreamSlot) -> bool;
}

/// Input file reads available to driver-supplied `\input` hooks.
///
/// This is intentionally separate from [`ExpansionState`] so ordinary gullet
/// code cannot open files and input hooks cannot see expansion, Env/register,
/// code-table, snapshot, font-assignment, or general [`World`] mutation APIs.
pub trait InputReadState {
    fn read_input_file(
        &mut self,
        path: &std::path::Path,
    ) -> Result<crate::FileContent, crate::WorldError>;
}

/// State operations available only to the top-level `\input` dispatch path.
///
/// This is separate from [`ExpansionState`] so helper code that is generic over
/// ordinary expansion authority cannot derive input-file read access.
pub trait InputOpenState {
    type Input<'a>: InputReadState
    where
        Self: 'a;

    fn input_open_context(&mut self) -> Self::Input<'_>;
}

/// Production expansion capability over a [`Universe`].
///
/// Pass this wrapper to lexer/expansion code instead of `&mut Universe` when
/// the caller does not need the full top-level driver surface.
pub struct ExpansionContext<'a> {
    universe: &'a mut Universe,
}

impl<'a> ExpansionContext<'a> {
    #[must_use]
    pub fn new(universe: &'a mut Universe) -> Self {
        Self { universe }
    }
}

/// Production input-open capability over a [`Universe`].
pub struct InputOpenContext<'a> {
    universe: &'a mut Universe,
}

impl<'a> InputOpenContext<'a> {
    #[must_use]
    pub fn new(universe: &'a mut Universe) -> Self {
        Self { universe }
    }
}

/// A whole-Universe rollback snapshot.
///
/// Snapshot capture is O(1): the private store snapshot is a tuple of marks,
/// roots, and positions; the remaining fields are small scalar placeholders
/// for M3 World/input state.
#[derive(Clone, Debug)]
pub struct Snapshot {
    owner: SnapshotOwner,
    store: StoreSnapshot,
    epoch: Epoch,
    world: WorldSnapshot,
    input_summary: InputSummary,
    interaction_mode: InteractionMode,
    state_hash: u64,
    state_hash_base: StateHashBase,
}

impl Snapshot {
    /// Returns the epoch captured by this snapshot.
    ///
    /// Rollback does not restore this value; the live Universe always bumps
    /// forward from its current maximum epoch after restoring state.
    #[must_use]
    pub const fn epoch(&self) -> Epoch {
        self.epoch
    }

    /// Returns the semantic convergence hash captured by this snapshot.
    ///
    /// f26.4 replaces the placeholder value with the real hash computation.
    #[must_use]
    pub const fn state_hash(&self) -> u64 {
        self.state_hash
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SnapshotOwner {
    address: usize,
    nonce: u64,
}

#[derive(Debug)]
struct UniverseOwner(Box<UniverseOwnerToken>);

#[derive(Debug)]
struct UniverseOwnerToken {
    nonce: u64,
}

impl UniverseOwner {
    fn new() -> Self {
        Self(Box::new(UniverseOwnerToken {
            nonce: random_owner_nonce(),
        }))
    }

    fn snapshot_owner(&self) -> SnapshotOwner {
        SnapshotOwner {
            address: self.0.as_ref() as *const UniverseOwnerToken as usize,
            nonce: self.0.nonce,
        }
    }
}

fn random_owner_nonce() -> u64 {
    let state = std::collections::hash_map::RandomState::new();
    state.hash_one(0x756e_6976_6572_7365_u64)
}

/// Current engine interaction mode.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum InteractionMode {
    /// Stop at recoverable errors.
    Batch,
    /// Stop and report recoverable errors without terminal prompting.
    Nonstop,
    /// Scroll through recoverable errors.
    Scroll,
    /// TeX's ordinary interactive mode.
    #[default]
    ErrorStop,
}

#[derive(Clone, Debug)]
struct StateHashBase {
    store: StoreStateHashCursor,
    world: WorldStateHashCursor,
    input_summary: InputSummary,
    interaction_mode: InteractionMode,
    checkpoint_hash: u64,
}

/// One owned TeX state timeline.
#[derive(Debug)]
pub struct Universe {
    owner: UniverseOwner,
    stores: Stores,
    world: World,
    interaction_mode: InteractionMode,
    input_summary: InputSummary,
    state_hash_base: StateHashBase,
}

impl Clone for Universe {
    fn clone(&self) -> Self {
        let stores = self.stores.clone();
        let state_hash_base = StateHashBase {
            store: stores.retarget_state_hash_cursor(&self.state_hash_base.store),
            world: self.state_hash_base.world.clone(),
            input_summary: self.state_hash_base.input_summary.clone(),
            interaction_mode: self.state_hash_base.interaction_mode,
            checkpoint_hash: self.state_hash_base.checkpoint_hash,
        };
        Self {
            owner: UniverseOwner::new(),
            stores,
            world: self.world.clone(),
            interaction_mode: self.interaction_mode,
            input_summary: self.input_summary.clone(),
            state_hash_base,
        }
    }
}

impl Default for Universe {
    fn default() -> Self {
        Self::new()
    }
}

impl Universe {
    /// Creates an isolated TeX state timeline.
    #[must_use]
    pub fn new() -> Self {
        Self::with_world(World::default())
    }

    /// Creates an isolated TeX timeline backed by an explicit effect world.
    #[must_use]
    pub fn with_world(world: World) -> Self {
        let mut stores = Stores::new();
        let clock = world.job_clock();
        install_job_clock_params(
            &mut |param, value| stores.set_int_param(param, value),
            clock,
        );
        let state_hash_base = StateHashBase {
            store: stores.state_hash_cursor(),
            world: world.state_hash_cursor(),
            input_summary: InputSummary::default(),
            interaction_mode: InteractionMode::default(),
            checkpoint_hash: INITIAL_STATE_HASH,
        };
        Self {
            owner: UniverseOwner::new(),
            stores,
            world,
            interaction_mode: InteractionMode::default(),
            input_summary: InputSummary::default(),
            state_hash_base,
        }
    }

    /// Takes an O(1) snapshot of the whole timeline tuple.
    #[must_use]
    pub fn snapshot(&mut self) -> Snapshot {
        let hash_base = self.state_hash_base.clone();
        let world = self.world.snapshot();
        let store = self.stores.checkpoint();
        let store_cursor = Stores::state_hash_cursor_from_snapshot(&store);
        let world_cursor = World::state_hash_cursor_from_snapshot(&world);
        let state_hash = if hash_base.store == store_cursor
            && hash_base.world == world_cursor
            && hash_base.input_summary == self.input_summary
            && hash_base.interaction_mode == self.interaction_mode
        {
            hash_base.checkpoint_hash
        } else {
            let slice_hash = self.state_hash_slice(&hash_base, &store);
            combine(hash_base.checkpoint_hash, slice_hash)
        };
        let next_hash_base = StateHashBase {
            store: store_cursor,
            world: world_cursor,
            input_summary: self.input_summary.clone(),
            interaction_mode: self.interaction_mode,
            checkpoint_hash: state_hash,
        };
        self.state_hash_base = next_hash_base.clone();
        Snapshot {
            owner: self.owner.snapshot_owner(),
            epoch: store.epoch(),
            store,
            world,
            input_summary: self.input_summary.clone(),
            interaction_mode: self.interaction_mode,
            state_hash,
            state_hash_base: next_hash_base,
        }
    }

    /// Rolls the whole timeline back to `snapshot` atomically.
    pub fn rollback(&mut self, snapshot: &Snapshot) {
        self.assert_valid_snapshot(snapshot);
        self.stores.rollback(&snapshot.store);
        self.world.rollback(&snapshot.world);
        self.input_summary = snapshot.input_summary.clone();
        self.interaction_mode = snapshot.interaction_mode;
        self.state_hash_base = snapshot.state_hash_base.clone();
    }

    fn state_hash_slice(&self, hash_base: &StateHashBase, store: &StoreSnapshot) -> u64 {
        let mut hasher = StateHasher::new(0x756e_6976_6572_7365);
        hasher.u64(self.stores.state_hash_slice(&hash_base.store, store));
        self.hash_world_state_slice(&hash_base.world, &mut hasher);
        self.hash_input_summary(&mut hasher);
        hash_interaction_mode(self.interaction_mode, &mut hasher);
        hasher.finish()
    }

    fn hash_world_state_slice(&self, cursor: &WorldStateHashCursor, hasher: &mut StateHasher) {
        hasher.tag(0x80);
        let effects = self.world.effect_records_since(cursor);
        hasher.usize(effects.len());
        for effect in effects {
            self.hash_effect_record(effect, hasher);
        }

        hasher.tag(0x81);
        let inputs = self.world.input_records_since(cursor);
        hasher.usize(inputs.len());
        for input in inputs {
            hash_path(input.path(), hasher);
            hasher.bytes(&input.hash().bytes());
            hasher.usize(input.len());
        }

        hasher.tag(0x82);
        let shell_escapes = self.world.shell_escape_records_since(cursor);
        hasher.usize(shell_escapes.len());
        for record in shell_escapes {
            hash_shell_escape_record(record, hasher);
        }

        hash_stream_bufs(self.world.stream_bufs(), hasher);
        hash_rng_state(self.world.rng_state(), hasher);
        hash_job_clock(self.world.job_clock(), hasher);
        hash_shell_escape_policy(self.world.shell_escape_policy(), hasher);
    }

    fn hash_effect_record(&self, record: &EffectRecord, hasher: &mut StateHasher) {
        match record {
            EffectRecord::StreamOpen { slot, target } => {
                hasher.tag(0);
                hash_stream_slot(*slot, hasher);
                hash_path(target.path(), hasher);
            }
            EffectRecord::StreamClose { slot } => {
                hasher.tag(1);
                hash_stream_slot(*slot, hasher);
            }
            EffectRecord::StreamWrite { sink, text } => {
                hasher.tag(2);
                hash_print_sink(*sink, hasher);
                hasher.str(text);
            }
            EffectRecord::DeferredWrite { stream, tokens } => {
                hasher.tag(3);
                hash_stream_slot(*stream, hasher);
                self.stores.hash_token_list_semantic(*tokens, hasher);
            }
            EffectRecord::Special { class, payload } => {
                hasher.tag(4);
                hasher.str(class);
                hasher.bytes(payload);
            }
            EffectRecord::PdfObjectPlaceholder { label } => {
                hasher.tag(5);
                hasher.str(label);
            }
            EffectRecord::ShellEscape(record) => {
                hasher.tag(6);
                hash_shell_escape_record(record, hasher);
            }
        }
    }

    fn hash_input_summary(&self, hasher: &mut StateHasher) {
        hasher.tag(0x90);
        hash_input_summary_fields(&self.stores, &self.input_summary, hasher);
    }

    fn assert_valid_snapshot(&self, snapshot: &Snapshot) {
        assert_eq!(
            snapshot.owner,
            self.owner.snapshot_owner(),
            "Universe snapshot belongs to a different Universe instance"
        );
    }

    /// Reads the owned environment through the Universe boundary.
    #[must_use]
    pub fn env(&self) -> &Env {
        self.stores.env()
    }

    /// Reads the external-effect capability object.
    #[must_use]
    pub const fn world(&self) -> &World {
        &self.world
    }

    /// Mutates the external-effect capability object through the Universe boundary.
    pub fn world_mut(&mut self) -> &mut World {
        &mut self.world
    }

    /// Records the current lexer-owned input stack state for the next snapshot.
    pub fn set_input_summary(&mut self, summary: InputSummary) {
        self.input_summary = summary;
    }

    /// Returns the lexer-owned input stack state restored by the last rollback.
    #[must_use]
    pub const fn input_summary(&self) -> &InputSummary {
        &self.input_summary
    }

    /// Returns the current interaction mode.
    #[must_use]
    pub const fn interaction_mode(&self) -> InteractionMode {
        self.interaction_mode
    }

    /// Sets the current interaction mode.
    pub fn set_interaction_mode(&mut self, mode: InteractionMode) {
        self.interaction_mode = mode;
    }

    /// Returns the current code-table generation vector.
    #[must_use]
    pub fn code_table_generations(&self) -> CodeTableGenerations {
        self.stores.code_table_generations()
    }

    #[must_use]
    pub fn catcode(&self, ch: char) -> Catcode {
        self.stores.catcode(ch)
    }

    pub fn set_catcode(&mut self, ch: char, value: Catcode) {
        self.stores.set_catcode(ch, value);
    }

    #[must_use]
    pub fn lccode(&self, ch: char) -> LcCode {
        self.stores.lccode(ch)
    }

    pub fn set_lccode(&mut self, ch: char, value: LcCode) {
        self.stores.set_lccode(ch, value);
    }

    #[must_use]
    pub fn uccode(&self, ch: char) -> UcCode {
        self.stores.uccode(ch)
    }

    pub fn set_uccode(&mut self, ch: char, value: UcCode) {
        self.stores.set_uccode(ch, value);
    }

    #[must_use]
    pub fn sfcode(&self, ch: char) -> SfCode {
        self.stores.sfcode(ch)
    }

    pub fn set_sfcode(&mut self, ch: char, value: SfCode) {
        self.stores.set_sfcode(ch, value);
    }

    #[must_use]
    pub fn mathcode(&self, ch: char) -> MathCode {
        self.stores.mathcode(ch)
    }

    pub fn set_mathcode(&mut self, ch: char, value: MathCode) {
        self.stores.set_mathcode(ch, value);
    }

    #[must_use]
    pub fn delcode(&self, ch: char) -> DelCode {
        self.stores.delcode(ch)
    }

    pub fn set_delcode(&mut self, ch: char, value: DelCode) {
        self.stores.set_delcode(ch, value);
    }

    pub fn add_hyphenation_pattern(&mut self, pattern: PatternSpec) {
        self.stores.add_hyphenation_pattern(pattern);
    }

    pub fn add_hyphenation_exception(&mut self, exception: ExceptionSpec) {
        self.stores.add_hyphenation_exception(exception);
    }

    #[must_use]
    pub fn hyphen_positions(&self, word: &str, left_min: usize, right_min: usize) -> Vec<usize> {
        self.stores.hyphen_positions(word, left_min, right_min)
    }

    #[must_use]
    pub fn hyphenation_exception(&self, word: &str) -> Option<&[usize]> {
        self.stores.hyphenation_exception(word)
    }

    #[must_use]
    pub fn meaning(&self, symbol: Symbol) -> Meaning {
        self.stores.meaning(symbol)
    }

    pub fn set_meaning(&mut self, symbol: Symbol, meaning: Meaning) {
        self.stores.set_meaning(symbol, meaning);
    }

    pub fn intern_relaxed_control_sequence(&mut self, name: &str) -> Symbol {
        self.stores.intern_relaxed_control_sequence(name)
    }

    pub fn set_meaning_global(&mut self, symbol: Symbol, meaning: Meaning) {
        self.stores.set_meaning_global(symbol, meaning);
    }

    pub fn intern_macro(&mut self, macro_meaning: MacroMeaning) -> MacroDefinitionId {
        self.stores.intern_macro(macro_meaning)
    }

    #[must_use]
    pub fn macro_definition(&self, id: MacroDefinitionId) -> MacroMeaning {
        self.stores.macro_definition(id)
    }

    pub fn set_macro_meaning(&mut self, symbol: Symbol, macro_meaning: MacroMeaning) {
        self.stores.set_macro_meaning(symbol, macro_meaning);
    }

    pub fn set_macro_meaning_global(&mut self, symbol: Symbol, macro_meaning: MacroMeaning) {
        self.stores.set_macro_meaning_global(symbol, macro_meaning);
    }

    #[must_use]
    pub fn macro_meaning(&self, symbol: Symbol) -> Option<MacroMeaning> {
        self.stores.macro_meaning(symbol)
    }

    pub fn intern(&mut self, name: &str) -> Symbol {
        self.stores.intern(name)
    }

    #[must_use]
    pub fn symbol(&self, name: &str) -> Option<Symbol> {
        self.stores.symbol(name)
    }

    #[must_use]
    pub fn resolve(&self, symbol: Symbol) -> &str {
        self.stores.resolve(symbol)
    }

    #[must_use]
    pub fn token_list_builder(&self) -> TokenListBuilder {
        self.stores.token_list_builder()
    }

    pub fn intern_token_list(&mut self, tokens: &[Token]) -> TokenListId {
        self.stores.intern_token_list(tokens)
    }

    pub fn finish_token_list(&mut self, builder: &mut TokenListBuilder) -> TokenListId {
        self.stores.finish_token_list(builder)
    }

    #[must_use]
    pub fn tokens(&self, id: TokenListId) -> &[Token] {
        self.stores.tokens(id)
    }

    pub fn intern_glue(&mut self, spec: GlueSpec) -> GlueId {
        self.stores.intern_glue(spec)
    }

    #[must_use]
    pub fn glue(&self, id: GlueId) -> GlueSpec {
        self.stores.glue(id)
    }

    pub fn intern_font(&mut self, font: LoadedFont) -> FontId {
        self.stores.intern_font(font)
    }

    #[must_use]
    pub fn font(&self, id: FontId) -> &LoadedFont {
        self.stores.font(id)
    }

    #[must_use]
    pub fn font_name(&self, id: FontId) -> String {
        self.stores.font_name(id)
    }

    #[must_use]
    pub fn font_metrics(&self, font: FontId) -> &FontMetrics {
        self.stores.font_metrics(font)
    }

    #[must_use]
    pub fn font_char_exists(&self, font: FontId, code: u8) -> bool {
        self.stores.font_char_exists(font, code)
    }

    #[must_use]
    pub fn font_char_metrics(&self, font: FontId, code: u8) -> Option<CharMetrics> {
        self.stores.font_char_metrics(font, code)
    }

    #[must_use]
    pub fn missing_font_character(&self, font: FontId, code: u8) -> Option<MissingCharacter> {
        self.stores.missing_font_character(font, code)
    }

    #[must_use]
    pub fn lig_kern_iter(
        &self,
        font: FontId,
        left: LigKernChar,
        right: LigKernChar,
    ) -> LigKernIter<'_> {
        self.stores.lig_kern_iter(font, left, right)
    }

    #[must_use]
    pub fn lig_kern_command(
        &self,
        font: FontId,
        left: LigKernChar,
        right: LigKernChar,
    ) -> Option<LigKernCommand> {
        self.stores.lig_kern_command(font, left, right)
    }

    #[must_use]
    pub fn extensible_recipe(&self, font: FontId, code: u8) -> Option<ExtensibleRecipe> {
        self.stores.extensible_recipe(font, code)
    }

    #[must_use]
    pub fn font_parameter(&self, font: FontId, number: u16) -> Scaled {
        self.stores.font_parameter(font, number)
    }

    #[must_use]
    pub fn current_font(&self) -> FontId {
        self.stores.current_font()
    }

    #[must_use]
    pub fn current_font_symbol(&self) -> Option<Symbol> {
        self.stores.current_font_symbol()
    }

    pub fn set_current_font(&mut self, id: FontId) {
        self.stores.set_current_font(id);
    }

    pub fn set_current_font_global(&mut self, id: FontId) {
        self.stores.set_current_font_global(id);
    }

    pub fn set_current_font_selector(&mut self, symbol: Symbol, id: FontId) {
        self.stores.set_current_font_selector(symbol, id);
    }

    pub fn set_current_font_selector_global(&mut self, symbol: Symbol, id: FontId) {
        self.stores.set_current_font_selector_global(symbol, id);
    }

    #[must_use]
    pub fn font_dimen(&self, font: FontId, number: u16) -> Scaled {
        self.stores.font_dimen(font, number)
    }

    pub fn set_font_dimen(
        &mut self,
        font: FontId,
        number: u16,
        value: Scaled,
        global: bool,
    ) -> Result<(), FontParameterError> {
        self.stores.set_font_dimen(font, number, value, global)
    }

    #[must_use]
    pub fn font_hyphen_char(&self, font: FontId) -> i32 {
        self.stores.font_hyphen_char(font)
    }

    pub fn set_font_hyphen_char(&mut self, font: FontId, value: i32, global: bool) {
        self.stores.set_font_hyphen_char(font, value, global);
    }

    #[must_use]
    pub fn font_skew_char(&self, font: FontId) -> i32 {
        self.stores.font_skew_char(font)
    }

    pub fn set_font_skew_char(&mut self, font: FontId, value: i32, global: bool) {
        self.stores.set_font_skew_char(font, value, global);
    }

    #[must_use]
    pub fn node_list_builder(&self) -> NodeListBuilder {
        self.stores.node_list_builder()
    }

    pub fn freeze_node_list(&mut self, nodes: &[Node]) -> NodeListId {
        self.stores.freeze_node_list(nodes)
    }

    pub fn finish_node_list(&mut self, builder: &mut NodeListBuilder) -> NodeListId {
        self.stores.finish_node_list(builder)
    }

    #[must_use]
    pub fn nodes(&self, id: NodeListId) -> &[Node] {
        self.stores.nodes(id)
    }

    pub fn enter_group(&mut self) {
        self.stores.enter_group();
    }

    pub fn enter_group_with_kind(&mut self, kind: GroupKind) {
        self.stores.enter_group_with_kind(kind);
    }

    pub fn push_aftergroup(&mut self, payload: Token) {
        self.stores.push_aftergroup(payload);
    }

    #[must_use]
    pub fn leave_group(&mut self) -> Vec<Token> {
        self.stores.leave_group()
    }

    pub fn leave_group_with_kind(
        &mut self,
        expected: GroupKind,
    ) -> Result<Vec<Token>, GroupMismatch> {
        self.stores.leave_group_with_kind(expected)
    }

    pub fn set_afterassignment(&mut self, token: Token) {
        self.stores.set_afterassignment(token);
    }

    pub fn take_afterassignment(&mut self) -> Option<Token> {
        self.stores.take_afterassignment()
    }

    pub fn set_count(&mut self, index: u16, value: i32) {
        self.stores.set_count(index, value);
    }

    #[must_use]
    pub fn count(&self, index: u16) -> i32 {
        self.stores.count(index)
    }

    pub fn set_count_global(&mut self, index: u16, value: i32) {
        self.stores.set_count_global(index, value);
    }

    pub fn set_dimen(&mut self, index: u16, value: Scaled) {
        self.stores.set_dimen(index, value);
    }

    #[must_use]
    pub fn dimen(&self, index: u16) -> Scaled {
        self.stores.dimen(index)
    }

    pub fn set_dimen_global(&mut self, index: u16, value: Scaled) {
        self.stores.set_dimen_global(index, value);
    }

    pub fn set_skip(&mut self, index: u16, value: GlueId) {
        self.stores.set_skip(index, value);
    }

    #[must_use]
    pub fn skip(&self, index: u16) -> GlueId {
        self.stores.skip(index)
    }

    pub fn set_skip_global(&mut self, index: u16, value: GlueId) {
        self.stores.set_skip_global(index, value);
    }

    pub fn set_muskip(&mut self, index: u16, value: GlueId) {
        self.stores.set_muskip(index, value);
    }

    #[must_use]
    pub fn muskip(&self, index: u16) -> GlueId {
        self.stores.muskip(index)
    }

    pub fn set_muskip_global(&mut self, index: u16, value: GlueId) {
        self.stores.set_muskip_global(index, value);
    }

    pub fn set_toks(&mut self, index: u16, value: TokenListId) {
        self.stores.set_toks(index, value);
    }

    #[must_use]
    pub fn toks(&self, index: u16) -> TokenListId {
        self.stores.toks(index)
    }

    pub fn set_toks_global(&mut self, index: u16, value: TokenListId) {
        self.stores.set_toks_global(index, value);
    }

    pub fn set_box_reg(&mut self, index: u16, value: NodeListId) {
        self.stores.set_box_reg(index, value);
    }

    pub fn set_box_reg_global(&mut self, index: u16, value: NodeListId) {
        self.stores.set_box_reg_global(index, value);
    }

    #[must_use]
    pub fn box_reg(&self, index: u16) -> Option<NodeListId> {
        self.stores.box_reg(index)
    }

    pub fn take_box_reg(&mut self, index: u16) -> Option<NodeListId> {
        self.stores.take_box_reg(index)
    }

    #[must_use]
    pub fn box_dimension(&self, index: u16, dimension: BoxDimension) -> Option<Scaled> {
        let id = self.box_reg(index)?;
        box_dimension_from_nodes(self.nodes(id), dimension)
    }

    pub fn set_box_dimension(&mut self, index: u16, dimension: BoxDimension, value: Scaled) {
        let Some(id) = self.box_reg(index) else {
            return;
        };
        let epoch_id = self.clone_node_list_to_epoch(id);
        let mut nodes = self.nodes(epoch_id).to_vec();
        set_box_dimension_in_nodes(&mut nodes, dimension, value);
        let rewritten = self.freeze_node_list(&nodes);
        self.set_box_reg(index, rewritten);
    }

    pub fn clone_node_list_to_epoch(&mut self, id: NodeListId) -> NodeListId {
        let nodes = self.nodes(id).to_vec();
        let cloned: Vec<_> = nodes
            .into_iter()
            .map(|node| self.clone_node_to_epoch(node))
            .collect();
        self.freeze_node_list(&cloned)
    }

    pub fn clone_node_to_epoch(&mut self, node: Node) -> Node {
        match node {
            Node::HList(mut box_node) => {
                box_node.children = self.clone_node_list_to_epoch(box_node.children);
                Node::HList(box_node)
            }
            Node::VList(mut box_node) => {
                box_node.children = self.clone_node_list_to_epoch(box_node.children);
                Node::VList(box_node)
            }
            Node::Disc {
                kind,
                pre,
                post,
                replace,
            } => Node::Disc {
                kind,
                pre: self.clone_node_list_to_epoch(pre),
                post: self.clone_node_list_to_epoch(post),
                replace: self.clone_node_list_to_epoch(replace),
            },
            Node::Ins { class, content } => Node::Ins {
                class,
                content: self.clone_node_list_to_epoch(content),
            },
            Node::Adjust(content) => Node::Adjust(self.clone_node_list_to_epoch(content)),
            node => node,
        }
    }

    pub fn set_int_param(&mut self, param: IntParam, value: i32) {
        self.stores.set_int_param(param, value);
    }

    pub fn set_int_param_global(&mut self, param: IntParam, value: i32) {
        self.stores.set_int_param_global(param, value);
    }

    #[must_use]
    pub fn int_param(&self, param: IntParam) -> i32 {
        self.stores.int_param(param)
    }

    #[must_use]
    pub fn mag(&self) -> i32 {
        self.stores.mag()
    }

    pub fn set_mag(&mut self, value: i32) {
        self.stores.set_mag(value);
    }

    pub fn set_mag_global(&mut self, value: i32) {
        self.stores.set_mag_global(value);
    }

    #[must_use]
    pub fn prepared_mag(&self) -> Option<i32> {
        self.stores.prepared_mag()
    }

    pub fn prepare_mag(&mut self) -> (i32, Option<PrepareMagDiagnostic>) {
        self.stores.prepare_mag()
    }

    #[must_use]
    pub fn endlinechar(&self) -> i32 {
        self.stores.endlinechar()
    }

    pub fn set_dimen_param(&mut self, param: DimenParam, value: Scaled) {
        self.stores.set_dimen_param(param, value);
    }

    pub fn set_dimen_param_global(&mut self, param: DimenParam, value: Scaled) {
        self.stores.set_dimen_param_global(param, value);
    }

    #[must_use]
    pub fn dimen_param(&self, param: DimenParam) -> Scaled {
        self.stores.dimen_param(param)
    }

    pub fn set_glue_param(&mut self, param: GlueParam, value: GlueId) {
        self.stores.set_glue_param(param, value);
    }

    #[must_use]
    pub fn glue_param(&self, param: GlueParam) -> GlueId {
        self.stores.glue_param(param)
    }

    pub fn set_glue_param_global(&mut self, param: GlueParam, value: GlueId) {
        self.stores.set_glue_param_global(param, value);
    }

    pub fn set_tok_param(&mut self, param: TokParam, value: TokenListId) {
        self.stores.set_tok_param(param, value);
    }

    #[must_use]
    pub fn tok_param(&self, param: TokParam) -> TokenListId {
        self.stores.tok_param(param)
    }

    pub fn set_tok_param_global(&mut self, param: TokParam, value: TokenListId) {
        self.stores.set_tok_param_global(param, value);
    }

    #[must_use]
    pub fn env_journal_bytes_since(&self, snapshot: &Snapshot) -> usize {
        self.assert_valid_snapshot(snapshot);
        self.stores.env_journal_bytes_since(&snapshot.store)
    }

    #[cfg(feature = "shadow")]
    pub fn verify_shadow(&self) {
        self.stores.verify_shadow();
    }

    #[cfg(any(test, feature = "testing", feature = "shadow"))]
    #[must_use]
    pub fn testing_state_hash(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.stores.testing_state_hash().hash(&mut hasher);
        self.world.testing_state_hash().hash(&mut hasher);
        self.input_summary.hash(&mut hasher);
        self.interaction_mode.hash(&mut hasher);
        hasher.finish()
    }

    #[cfg(any(test, feature = "testing", feature = "shadow"))]
    pub fn testing_hash_node_list_content(&self, id: NodeListId, hasher: &mut impl Hasher) {
        self.stores.testing_hash_node_list_content(id, hasher);
    }

    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn testing_live_survivor_slot_count(&self) -> usize {
        self.stores.testing_live_survivor_slot_count()
    }

    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn testing_survivor_refcount(&self, id: NodeListId) -> u32 {
        self.stores.testing_survivor_refcount(id)
    }
}

/// A mutable dimension field of a box register's top-level box.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum BoxDimension {
    Width,
    Height,
    Depth,
}

fn box_dimension_from_nodes(nodes: &[Node], dimension: BoxDimension) -> Option<Scaled> {
    let box_node = match nodes {
        [Node::HList(box_node)] | [Node::VList(box_node)] => box_node,
        _ => return None,
    };
    Some(match dimension {
        BoxDimension::Width => box_node.width,
        BoxDimension::Height => box_node.height,
        BoxDimension::Depth => box_node.depth,
    })
}

fn set_box_dimension_in_nodes(nodes: &mut [Node], dimension: BoxDimension, value: Scaled) {
    let box_node = match nodes {
        [Node::HList(box_node)] | [Node::VList(box_node)] => box_node,
        _ => return,
    };
    match dimension {
        BoxDimension::Width => box_node.width = value,
        BoxDimension::Height => box_node.height = value,
        BoxDimension::Depth => box_node.depth = value,
    }
}

impl ExpansionState for Universe {
    fn catcode(&self, ch: char) -> Catcode {
        Self::catcode(self, ch)
    }

    fn lccode(&self, ch: char) -> LcCode {
        Self::lccode(self, ch)
    }

    fn uccode(&self, ch: char) -> UcCode {
        Self::uccode(self, ch)
    }

    fn sfcode(&self, ch: char) -> SfCode {
        Self::sfcode(self, ch)
    }

    fn mathcode(&self, ch: char) -> MathCode {
        Self::mathcode(self, ch)
    }

    fn delcode(&self, ch: char) -> DelCode {
        Self::delcode(self, ch)
    }

    fn meaning(&self, symbol: Symbol) -> Meaning {
        Self::meaning(self, symbol)
    }

    fn macro_definition(&self, id: MacroDefinitionId) -> MacroMeaning {
        Self::macro_definition(self, id)
    }

    fn macro_meaning(&self, symbol: Symbol) -> Option<MacroMeaning> {
        Self::macro_meaning(self, symbol)
    }

    fn intern_relaxed_control_sequence(&mut self, name: &str) -> Symbol {
        Self::intern_relaxed_control_sequence(self, name)
    }

    fn intern(&mut self, name: &str) -> Symbol {
        Self::intern(self, name)
    }

    fn symbol(&self, name: &str) -> Option<Symbol> {
        Self::symbol(self, name)
    }

    fn resolve(&self, symbol: Symbol) -> &str {
        Self::resolve(self, symbol)
    }

    fn token_list_builder(&self) -> TokenListBuilder {
        Self::token_list_builder(self)
    }

    fn intern_token_list(&mut self, tokens: &[Token]) -> TokenListId {
        Self::intern_token_list(self, tokens)
    }

    fn finish_token_list(&mut self, builder: &mut TokenListBuilder) -> TokenListId {
        Self::finish_token_list(self, builder)
    }

    fn tokens(&self, id: TokenListId) -> &[Token] {
        Self::tokens(self, id)
    }

    fn intern_glue(&mut self, spec: GlueSpec) -> GlueId {
        Self::intern_glue(self, spec)
    }

    fn glue(&self, id: GlueId) -> GlueSpec {
        Self::glue(self, id)
    }

    fn font_name(&self, id: FontId) -> String {
        Self::font_name(self, id)
    }

    fn font_parameter(&self, font: FontId, number: u16) -> Scaled {
        Self::font_parameter(self, font, number)
    }

    fn font_dimen(&self, font: FontId, number: u16) -> Scaled {
        Self::font_dimen(self, font, number)
    }

    fn font_hyphen_char(&self, font: FontId) -> i32 {
        Self::font_hyphen_char(self, font)
    }

    fn font_skew_char(&self, font: FontId) -> i32 {
        Self::font_skew_char(self, font)
    }

    fn current_font(&self) -> FontId {
        Self::current_font(self)
    }

    fn current_font_symbol(&self) -> Option<Symbol> {
        Self::current_font_symbol(self)
    }

    fn nodes(&self, id: NodeListId) -> &[Node] {
        Self::nodes(self, id)
    }

    fn count(&self, index: u16) -> i32 {
        Self::count(self, index)
    }

    fn dimen(&self, index: u16) -> Scaled {
        Self::dimen(self, index)
    }

    fn skip(&self, index: u16) -> GlueId {
        Self::skip(self, index)
    }

    fn muskip(&self, index: u16) -> GlueId {
        Self::muskip(self, index)
    }

    fn toks(&self, index: u16) -> TokenListId {
        Self::toks(self, index)
    }

    fn box_reg(&self, index: u16) -> Option<NodeListId> {
        Self::box_reg(self, index)
    }

    fn box_dimension(&self, index: u16, dimension: BoxDimension) -> Option<Scaled> {
        Self::box_dimension(self, index, dimension)
    }

    fn int_param(&self, param: IntParam) -> i32 {
        Self::int_param(self, param)
    }

    fn mag(&self) -> i32 {
        Self::mag(self)
    }

    fn prepared_mag(&self) -> Option<i32> {
        Self::prepared_mag(self)
    }

    fn prepare_mag(&mut self) -> (i32, Option<PrepareMagDiagnostic>) {
        Self::prepare_mag(self)
    }

    fn endlinechar(&self) -> i32 {
        Self::endlinechar(self)
    }

    fn dimen_param(&self, param: DimenParam) -> Scaled {
        Self::dimen_param(self, param)
    }

    fn glue_param(&self, param: GlueParam) -> GlueId {
        Self::glue_param(self, param)
    }

    fn tok_param(&self, param: TokParam) -> TokenListId {
        Self::tok_param(self, param)
    }

    fn input_stream_eof(&self, stream: StreamSlot) -> bool {
        self.world.input_stream_eof(stream)
    }
}

impl ExpansionState for ExpansionContext<'_> {
    fn catcode(&self, ch: char) -> Catcode {
        self.universe.catcode(ch)
    }

    fn lccode(&self, ch: char) -> LcCode {
        self.universe.lccode(ch)
    }

    fn uccode(&self, ch: char) -> UcCode {
        self.universe.uccode(ch)
    }

    fn sfcode(&self, ch: char) -> SfCode {
        self.universe.sfcode(ch)
    }

    fn mathcode(&self, ch: char) -> MathCode {
        self.universe.mathcode(ch)
    }

    fn delcode(&self, ch: char) -> DelCode {
        self.universe.delcode(ch)
    }

    fn meaning(&self, symbol: Symbol) -> Meaning {
        self.universe.meaning(symbol)
    }

    fn macro_definition(&self, id: MacroDefinitionId) -> MacroMeaning {
        self.universe.macro_definition(id)
    }

    fn macro_meaning(&self, symbol: Symbol) -> Option<MacroMeaning> {
        self.universe.macro_meaning(symbol)
    }

    fn intern_relaxed_control_sequence(&mut self, name: &str) -> Symbol {
        self.universe.intern_relaxed_control_sequence(name)
    }

    fn intern(&mut self, name: &str) -> Symbol {
        self.universe.intern(name)
    }

    fn symbol(&self, name: &str) -> Option<Symbol> {
        self.universe.symbol(name)
    }

    fn resolve(&self, symbol: Symbol) -> &str {
        self.universe.resolve(symbol)
    }

    fn token_list_builder(&self) -> TokenListBuilder {
        self.universe.token_list_builder()
    }

    fn intern_token_list(&mut self, tokens: &[Token]) -> TokenListId {
        self.universe.intern_token_list(tokens)
    }

    fn finish_token_list(&mut self, builder: &mut TokenListBuilder) -> TokenListId {
        self.universe.finish_token_list(builder)
    }

    fn tokens(&self, id: TokenListId) -> &[Token] {
        self.universe.tokens(id)
    }

    fn intern_glue(&mut self, spec: GlueSpec) -> GlueId {
        self.universe.intern_glue(spec)
    }

    fn glue(&self, id: GlueId) -> GlueSpec {
        self.universe.glue(id)
    }

    fn font_name(&self, id: FontId) -> String {
        self.universe.font_name(id)
    }

    fn font_parameter(&self, font: FontId, number: u16) -> Scaled {
        self.universe.font_parameter(font, number)
    }

    fn font_dimen(&self, font: FontId, number: u16) -> Scaled {
        self.universe.font_dimen(font, number)
    }

    fn font_hyphen_char(&self, font: FontId) -> i32 {
        self.universe.font_hyphen_char(font)
    }

    fn font_skew_char(&self, font: FontId) -> i32 {
        self.universe.font_skew_char(font)
    }

    fn current_font(&self) -> FontId {
        self.universe.current_font()
    }

    fn current_font_symbol(&self) -> Option<Symbol> {
        self.universe.current_font_symbol()
    }

    fn nodes(&self, id: NodeListId) -> &[Node] {
        self.universe.nodes(id)
    }

    fn count(&self, index: u16) -> i32 {
        self.universe.count(index)
    }

    fn dimen(&self, index: u16) -> Scaled {
        self.universe.dimen(index)
    }

    fn skip(&self, index: u16) -> GlueId {
        self.universe.skip(index)
    }

    fn muskip(&self, index: u16) -> GlueId {
        self.universe.muskip(index)
    }

    fn toks(&self, index: u16) -> TokenListId {
        self.universe.toks(index)
    }

    fn box_reg(&self, index: u16) -> Option<NodeListId> {
        self.universe.box_reg(index)
    }

    fn box_dimension(&self, index: u16, dimension: BoxDimension) -> Option<Scaled> {
        self.universe.box_dimension(index, dimension)
    }

    fn int_param(&self, param: IntParam) -> i32 {
        self.universe.int_param(param)
    }

    fn mag(&self) -> i32 {
        self.universe.mag()
    }

    fn prepared_mag(&self) -> Option<i32> {
        self.universe.prepared_mag()
    }

    fn prepare_mag(&mut self) -> (i32, Option<PrepareMagDiagnostic>) {
        self.universe.prepare_mag()
    }

    fn endlinechar(&self) -> i32 {
        self.universe.endlinechar()
    }

    fn dimen_param(&self, param: DimenParam) -> Scaled {
        self.universe.dimen_param(param)
    }

    fn glue_param(&self, param: GlueParam) -> GlueId {
        self.universe.glue_param(param)
    }

    fn tok_param(&self, param: TokParam) -> TokenListId {
        self.universe.tok_param(param)
    }

    fn input_stream_eof(&self, stream: StreamSlot) -> bool {
        self.universe.world.input_stream_eof(stream)
    }
}

impl InputReadState for InputOpenContext<'_> {
    fn read_input_file(
        &mut self,
        path: &std::path::Path,
    ) -> Result<crate::FileContent, crate::WorldError> {
        self.universe.world.read_file(path)
    }
}

impl InputOpenState for Universe {
    type Input<'a>
        = InputOpenContext<'a>
    where
        Self: 'a;

    fn input_open_context(&mut self) -> Self::Input<'_> {
        InputOpenContext::new(self)
    }
}

impl InputOpenState for ExpansionContext<'_> {
    type Input<'a>
        = InputOpenContext<'a>
    where
        Self: 'a;

    fn input_open_context(&mut self) -> Self::Input<'_> {
        InputOpenContext::new(self.universe)
    }
}

fn hash_stream_bufs(streams: &StreamBufState, hasher: &mut StateHasher) {
    hasher.tag(0x83);
    for raw in 0..crate::world::STREAM_SLOT_COUNT as u8 {
        let slot = StreamSlot::new(raw);
        match streams.read_stream_target(slot) {
            Some(target) => {
                hasher.bool(true);
                hash_path(target.path(), hasher);
                hasher.bytes(&target.hash().bytes());
                hasher.usize(target.next_line());
            }
            None => hasher.bool(false),
        }
        match streams.write_stream_target(slot) {
            Some(target) => {
                hasher.bool(true);
                hash_path(target.path(), hasher);
            }
            None => hasher.bool(false),
        }
        hasher.str(streams.partial_line(slot));
    }
    hasher.str(streams.log_partial_line());
    hasher.str(streams.terminal_partial_line());
}

fn hash_rng_state(rng: crate::world::RngState, hasher: &mut StateHasher) {
    hasher.tag(0x84);
    let text = format!("{rng:?}");
    hasher.str(&text);
}

fn hash_job_clock(clock: JobClock, hasher: &mut StateHasher) {
    hasher.tag(0x85);
    hasher.i32(clock.time);
    hasher.i32(clock.day);
    hasher.i32(clock.month);
    hasher.i32(clock.year);
}

fn hash_shell_escape_policy(policy: ShellEscapePolicy, hasher: &mut StateHasher) {
    hasher.tag(0x86);
    hasher.u8(match policy {
        ShellEscapePolicy::Disabled => 0,
        ShellEscapePolicy::Enabled => 1,
    });
}

fn hash_interaction_mode(mode: InteractionMode, hasher: &mut StateHasher) {
    hasher.tag(0x91);
    hasher.u8(match mode {
        InteractionMode::Batch => 0,
        InteractionMode::Nonstop => 1,
        InteractionMode::Scroll => 2,
        InteractionMode::ErrorStop => 3,
    });
}

fn hash_print_sink(sink: PrintSink, hasher: &mut StateHasher) {
    match sink {
        PrintSink::Terminal => hasher.tag(0),
        PrintSink::Log => hasher.tag(1),
        PrintSink::TerminalAndLog => hasher.tag(2),
        PrintSink::Stream(slot) => {
            hasher.tag(3);
            hash_stream_slot(slot, hasher);
        }
    }
}

fn hash_stream_slot(slot: StreamSlot, hasher: &mut StateHasher) {
    hasher.u8(slot.raw());
}

fn hash_shell_escape_record(record: &ShellEscapeRecord, hasher: &mut StateHasher) {
    hasher.str(record.command());
    hasher.bool(record.allowed());
}

fn hash_path(path: &std::path::Path, hasher: &mut StateHasher) {
    hasher.bytes(path.as_os_str().as_encoded_bytes());
}

fn hash_input_summary_fields(stores: &Stores, summary: &InputSummary, hasher: &mut StateHasher) {
    hasher.usize(summary.frames().len());
    for frame in summary.frames() {
        match frame {
            InputFrameSummary::Source { source_id, source } => {
                hasher.tag(0);
                hasher.u32(source_id.raw());
                hasher.usize(source.buffer_offset());
                hasher.usize(source.next_source_offset());
                hasher.usize(source.line_number());
                hasher.usize(source.column());
                hash_lexer_state(source.lexer_state(), hasher);
                hasher.str(source.normalized_line());
                hasher.usize(source.line_char_offset());
                hasher.usize(source.line_byte_offset());
                hasher.usize(source.pending().len());
                for token in source.pending() {
                    hash_token(stores, *token, hasher);
                }
                hasher.bool(source.end_after_current_line());
            }
            InputFrameSummary::TokenList {
                token_list,
                replay_kind,
                index,
                macro_arguments,
            } => {
                hasher.tag(1);
                stores.hash_token_list_semantic(*token_list, hasher);
                hash_token_list_replay_kind(*replay_kind, hasher);
                hasher.usize(*index);
                for slot in 1..=crate::input::MACRO_ARGUMENT_SLOTS as u8 {
                    match macro_arguments.get(slot) {
                        Some(token_list) => {
                            hasher.bool(true);
                            stores.hash_token_list_semantic(token_list, hasher);
                        }
                        None => hasher.bool(false),
                    }
                }
            }
            InputFrameSummary::Condition(condition) => {
                hasher.tag(2);
                hash_condition_kind(condition.kind(), hasher);
                hash_condition_limb(condition.limb(), hasher);
                hasher.bool(condition.current_limb_taken());
                hasher.bool(condition.any_limb_taken());
                hasher.u32(condition.ifcase_or_count());
                hasher.u32(condition.skip_nesting());
            }
        }
    }
    match summary.last_source_frame() {
        Some(source) => {
            hasher.bool(true);
            hasher.usize(source.buffer_offset());
            hasher.usize(source.next_source_offset());
            hasher.usize(source.line_number());
            hasher.usize(source.column());
            hash_lexer_state(source.lexer_state(), hasher);
            hasher.str(source.normalized_line());
            hasher.usize(source.line_char_offset());
            hasher.usize(source.line_byte_offset());
            hasher.usize(source.pending().len());
            for token in source.pending() {
                hash_token(stores, *token, hasher);
            }
            hasher.bool(source.end_after_current_line());
        }
        None => hasher.bool(false),
    }
}

fn hash_token(stores: &Stores, token: Token, hasher: &mut StateHasher) {
    match token {
        Token::Char { ch, cat } => {
            hasher.tag(0);
            hasher.u32(ch as u32);
            hasher.u8(cat as u8);
        }
        Token::Cs(symbol) => {
            hasher.tag(1);
            hasher.str(stores.resolve(symbol));
        }
        Token::Param(slot) => {
            hasher.tag(2);
            hasher.u8(slot);
        }
    }
}

fn hash_lexer_state(state: LexerState, hasher: &mut StateHasher) {
    hasher.u8(match state {
        LexerState::NewLine => 0,
        LexerState::MidLine => 1,
        LexerState::SkippingBlanks => 2,
    });
}

fn hash_token_list_replay_kind(kind: TokenListReplayKind, hasher: &mut StateHasher) {
    hasher.u8(match kind {
        TokenListReplayKind::MacroBody => 0,
        TokenListReplayKind::MacroArgument => 1,
        TokenListReplayKind::NoExpand => 2,
        TokenListReplayKind::EveryPar => 3,
        TokenListReplayKind::Mark => 4,
        TokenListReplayKind::OutputRoutine => 5,
        TokenListReplayKind::Inserted => 6,
    });
}

fn hash_condition_kind(kind: ConditionKind, hasher: &mut StateHasher) {
    hasher.u8(match kind {
        ConditionKind::If => 0,
        ConditionKind::IfCase => 1,
    });
}

fn hash_condition_limb(limb: ConditionLimb, hasher: &mut StateHasher) {
    hasher.u8(match limb {
        ConditionLimb::If => 0,
        ConditionLimb::Or => 1,
        ConditionLimb::Else => 2,
    });
}

#[cfg(test)]
mod tests {
    use super::Universe;
    use crate::font::NULL_FONT;
    use crate::glue::{GlueSpec, Order};
    use crate::macro_store::MacroMeaning;
    use crate::meaning::{Meaning, MeaningFlags};
    use crate::node::{BoxNode, BoxNodeFields, Node, Sign};
    use crate::scaled::Scaled;
    use crate::token::{Catcode, Token};
    use crate::world::{ContentHash, JobClock, PrintSink, StreamSlot, World};

    #[test]
    fn universe_is_send() {
        fn assert_send<T: Send>() {}

        assert_send::<Universe>();
    }

    #[test]
    #[should_panic(expected = "Universe snapshot belongs to a different Universe instance")]
    fn rollback_rejects_snapshot_from_different_universe() {
        let mut first = Universe::new();
        let mut second = Universe::new();
        let snapshot = first.snapshot();

        second.rollback(&snapshot);
    }

    #[test]
    fn rollback_restores_store_tuple_and_placeholder_scalars() {
        let mut universe = Universe::new();
        let symbol = universe.intern("x");
        let snapshot = universe.snapshot();

        universe.set_meaning(symbol, Meaning::Relax);
        universe.rollback(&snapshot);

        assert_eq!(universe.meaning(symbol), Meaning::Undefined);
    }

    #[test]
    fn rollback_bumps_epoch_past_previous_live_epoch() {
        let mut universe = Universe::new();
        let snapshot = universe.snapshot();
        let before_rollback = universe.env().epoch();

        universe.rollback(&snapshot);

        assert!(snapshot.epoch() < before_rollback);
        assert!(before_rollback < universe.env().epoch());
    }

    #[test]
    fn job_clock_initializes_tex_clock_parameters_once() {
        let clock = JobClock {
            time: 721,
            day: 8,
            month: 7,
            year: 2026,
        };
        let universe = Universe::with_world(World::memory_with_clock(clock));

        assert_eq!(universe.int_param(crate::env::banks::IntParam::TIME), 721);
        assert_eq!(universe.int_param(crate::env::banks::IntParam::DAY), 8);
        assert_eq!(universe.int_param(crate::env::banks::IntParam::MONTH), 7);
        assert_eq!(universe.int_param(crate::env::banks::IntParam::YEAR), 2026);
    }

    #[test]
    fn rollback_restores_world_inputs_stream_buffers_and_rng() {
        let mut universe = Universe::new();
        universe
            .world_mut()
            .set_memory_file("main.tex", b"abc".to_vec())
            .expect("seed memory file");
        let slot = StreamSlot::new(2);
        let snapshot = universe.snapshot();

        let read = universe
            .world_mut()
            .open_in(slot, "main.tex")
            .expect("read file through world");
        universe.world_mut().open_out(slot, "main.aux");
        universe
            .world_mut()
            .write_text(PrintSink::Stream(slot), "partial");
        let random = universe.world_mut().next_random_u64();
        assert_eq!(read.hash(), ContentHash::from_bytes(b"abc"));
        assert_eq!(universe.world().input_records().len(), 1);

        universe.rollback(&snapshot);

        assert!(universe.world().input_records().is_empty());
        assert_eq!(universe.world().stream_bufs().partial_line(slot), "");
        assert!(
            universe
                .world()
                .stream_bufs()
                .read_stream_path(slot)
                .is_none()
        );
        assert_eq!(universe.world_mut().next_random_u64(), random);
    }

    #[test]
    fn snapshot_state_hash_is_deterministic_for_same_program() {
        assert_eq!(
            checkpoint_hashes_for_program(),
            checkpoint_hashes_for_program()
        );
    }

    #[test]
    fn snapshot_state_hash_ignores_content_intern_order() {
        let mut first = Universe::new();
        let zed = first.intern("z");
        let alpha = first.intern("alpha");
        let macro_target = first.intern("macro_target");
        first.set_meaning(zed, Meaning::Relax);
        let filler_tokens = first.intern_token_list(&[Token::param(1)]);
        let target_tokens = first.intern_token_list(&[
            Token::Cs(alpha),
            Token::Char {
                ch: 'x',
                cat: Catcode::Letter,
            },
        ]);
        let filler_glue = first.intern_glue(glue(99));
        let target_glue = first.intern_glue(glue(7));
        let filler_macro = first.intern_macro(MacroMeaning::new(
            MeaningFlags::LONG,
            filler_tokens,
            filler_tokens,
        ));
        let target_macro = first.intern_macro(MacroMeaning::new(
            MeaningFlags::PROTECTED,
            target_tokens,
            target_tokens,
        ));
        first.set_toks(0, target_tokens);
        first.set_skip(0, target_glue);
        first.set_meaning(
            macro_target,
            Meaning::Macro {
                flags: MeaningFlags::PROTECTED,
                definition: target_macro,
            },
        );
        assert_ne!(filler_glue, target_glue);
        assert_ne!(filler_macro, target_macro);
        let first_hash = first.snapshot().state_hash();

        let mut second = Universe::new();
        let macro_target = second.intern("macro_target");
        let alpha = second.intern("alpha");
        let target_tokens = second.intern_token_list(&[
            Token::Cs(alpha),
            Token::Char {
                ch: 'x',
                cat: Catcode::Letter,
            },
        ]);
        let filler_tokens = second.intern_token_list(&[Token::param(1)]);
        let target_glue = second.intern_glue(glue(7));
        let filler_glue = second.intern_glue(glue(99));
        let target_macro = second.intern_macro(MacroMeaning::new(
            MeaningFlags::PROTECTED,
            target_tokens,
            target_tokens,
        ));
        let filler_macro = second.intern_macro(MacroMeaning::new(
            MeaningFlags::LONG,
            filler_tokens,
            filler_tokens,
        ));
        let zed = second.intern("z");
        second.set_meaning(zed, Meaning::Relax);
        second.set_toks(0, target_tokens);
        second.set_skip(0, target_glue);
        second.set_meaning(
            macro_target,
            Meaning::Macro {
                flags: MeaningFlags::PROTECTED,
                definition: target_macro,
            },
        );
        assert_ne!(filler_glue, target_glue);
        assert_ne!(filler_macro, target_macro);

        assert_eq!(first_hash, second.snapshot().state_hash());
    }

    #[test]
    fn snapshot_state_hash_changes_for_one_register_bit() {
        let mut unchanged = Universe::new();
        let mut changed = Universe::new();
        changed.set_count(0, 1);

        assert_ne!(
            unchanged.snapshot().state_hash(),
            changed.snapshot().state_hash()
        );
    }

    #[test]
    fn clone_preserves_pending_state_hash_slice() {
        let mut original = Universe::new();
        let _base = original.snapshot();
        original.set_count(0, 42);
        let mut fork = original.clone();

        assert_eq!(fork.count(0), 42);
        assert_eq!(
            original.snapshot().state_hash(),
            fork.snapshot().state_hash()
        );
    }

    #[test]
    fn snapshot_state_hash_changes_for_rng_only_change() {
        let mut unchanged = Universe::new();
        let mut changed = Universe::new();
        let _ = changed.world_mut().next_random_u64();

        assert_ne!(
            unchanged.snapshot().state_hash(),
            changed.snapshot().state_hash()
        );
    }

    #[test]
    fn snapshot_state_hash_distinguishes_font_content_identity() {
        let mut first = Universe::new();
        let mut second = Universe::new();
        let first_symbol = first.intern("font");
        let second_symbol = second.intern("font");

        let first_font = first.intern_font(test_font("cmr10", b"same"));
        let second_font = second.intern_font(test_font("cmr10", b"different"));
        assert_eq!(first_font.raw(), second_font.raw());

        first.set_meaning(first_symbol, Meaning::Font(first_font));
        second.set_meaning(second_symbol, Meaning::Font(second_font));

        assert_ne!(
            first.snapshot().state_hash(),
            second.snapshot().state_hash()
        );
    }

    #[test]
    fn rollback_restores_state_hash_cursor() {
        let mut universe = Universe::new();
        let base = universe.snapshot();
        universe.set_count(0, 10);
        let first = universe.snapshot();

        universe.rollback(&base);
        universe.set_count(0, 10);
        let second = universe.snapshot();

        assert_eq!(first.state_hash(), second.state_hash());
    }

    #[test]
    fn snapshot_state_hash_walks_deep_node_lists_iteratively() {
        let mut universe = Universe::new();
        let mut current = universe.freeze_node_list(&[Node::Char {
            font: NULL_FONT,
            ch: 'x',
        }]);

        for _ in 0..5000 {
            current = universe.freeze_node_list(&[Node::HList(BoxNode::new(BoxNodeFields {
                width: Scaled::from_raw(1),
                height: Scaled::from_raw(2),
                depth: Scaled::from_raw(3),
                shift: Scaled::from_raw(0),
                glue_set: 0.0,
                glue_sign: Sign::Normal,
                glue_order: Order::Normal,
                children: current,
            }))]);
        }

        universe.set_box_reg(0, current);
        assert_ne!(universe.snapshot().state_hash(), 0);
    }

    fn checkpoint_hashes_for_program() -> Vec<u64> {
        let mut universe = Universe::new();
        let mut hashes = Vec::new();
        hashes.push(universe.snapshot().state_hash());

        universe.set_count(0, 42);
        universe.set_catcode('@', Catcode::Letter);
        hashes.push(universe.snapshot().state_hash());

        let symbol = universe.intern("foo");
        let tokens = universe.intern_token_list(&[Token::Cs(symbol)]);
        universe.set_toks(2, tokens);
        universe
            .world_mut()
            .record_deferred_write(StreamSlot::new(1), tokens);
        hashes.push(universe.snapshot().state_hash());

        let _ = universe.world_mut().next_random_u64();
        hashes.push(universe.snapshot().state_hash());
        hashes
    }

    fn glue(width: i32) -> GlueSpec {
        GlueSpec {
            width: Scaled::from_raw(width),
            stretch: Scaled::from_raw(1),
            stretch_order: Order::Fil,
            shrink: Scaled::from_raw(2),
            shrink_order: Order::Normal,
        }
    }

    fn test_font(name: &str, bytes: &[u8]) -> crate::font::LoadedFont {
        crate::font::LoadedFont::new(
            name,
            format!("{name}.tfm"),
            ContentHash::from_bytes(bytes).bytes(),
            0,
            Scaled::from_raw(10 * Scaled::UNITY),
            Scaled::from_raw(10 * Scaled::UNITY),
            vec![Scaled::from_raw(0); 7],
            crate::font::FontMetrics::default(),
        )
    }
}
