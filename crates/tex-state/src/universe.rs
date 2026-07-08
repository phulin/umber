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
use crate::glue::GlueSpec;
use crate::ids::{GlueId, MacroDefinitionId, NodeListId, TokenListId};
use crate::interner::Symbol;
use crate::macro_store::MacroMeaning;
use crate::meaning::Meaning;
use crate::node::Node;
use crate::node_arena::NodeListBuilder;
use crate::scaled::Scaled;
use crate::stores::{GroupKind, GroupMismatch, PrepareMagDiagnostic, StoreSnapshot, Stores};
use crate::token::{Catcode, Token};
use crate::token_store::TokenListBuilder;
use crate::world::{World, WorldSnapshot, install_job_clock_params};
#[cfg(any(test, feature = "testing", feature = "shadow"))]
use std::hash::Hasher;

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
struct SnapshotOwner(usize);

#[derive(Debug)]
struct UniverseOwner(Box<UniverseOwnerToken>);

#[derive(Debug)]
struct UniverseOwnerToken {
    _private: u8,
}

impl UniverseOwner {
    fn new() -> Self {
        Self(Box::new(UniverseOwnerToken { _private: 0 }))
    }

    fn snapshot_owner(&self) -> SnapshotOwner {
        SnapshotOwner(self.0.as_ref() as *const UniverseOwnerToken as usize)
    }
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

/// Placeholder for replay-complete lexer/input-stack state.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct InputSummary {
    _private: (),
}

/// One owned TeX state timeline.
#[derive(Debug)]
pub struct Universe {
    owner: UniverseOwner,
    stores: Stores,
    world: World,
    interaction_mode: InteractionMode,
    input_summary: InputSummary,
}

impl Clone for Universe {
    fn clone(&self) -> Self {
        Self {
            owner: UniverseOwner::new(),
            stores: self.stores.clone(),
            world: self.world.clone(),
            interaction_mode: self.interaction_mode,
            input_summary: self.input_summary.clone(),
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
        Self {
            owner: UniverseOwner::new(),
            stores,
            world,
            interaction_mode: InteractionMode::default(),
            input_summary: InputSummary::default(),
        }
    }

    /// Takes an O(1) snapshot of the whole timeline tuple.
    #[must_use]
    pub fn snapshot(&mut self) -> Snapshot {
        let store = self.stores.checkpoint();
        Snapshot {
            owner: self.owner.snapshot_owner(),
            epoch: store.epoch(),
            store,
            world: self.world.snapshot(),
            input_summary: self.input_summary.clone(),
            interaction_mode: self.interaction_mode,
            state_hash: 0,
        }
    }

    /// Rolls the whole timeline back to `snapshot` atomically.
    pub fn rollback(&mut self, snapshot: &Snapshot) {
        self.assert_valid_snapshot(snapshot);
        self.stores.rollback(&snapshot.store);
        self.world.rollback(&snapshot.world);
        self.input_summary = snapshot.input_summary.clone();
        self.interaction_mode = snapshot.interaction_mode;
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
        self.stores.testing_state_hash()
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

#[cfg(test)]
mod tests {
    use super::Universe;
    use crate::meaning::Meaning;
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
}
