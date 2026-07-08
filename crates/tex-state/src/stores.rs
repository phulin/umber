//! Aggregate state stores and atomic rollback boundary.
//!
//! `Stores` is hidden M3 scaffolding for state that must checkpoint and roll
//! back together until `Universe` subsumes this aggregate boundary. Callers use
//! this boundary instead of rolling back `Env` or content stores independently.

use crate::code_tables::{
    CodeTableGenerations, CodeTables, CodeTablesSnapshot, DelCode, LcCode, MathCode, SfCode, UcCode,
};
use crate::env::banks::{DimenParam, GlueParam, IntParam, TokParam};
use crate::env::{Env, EnvSnapshot};
use crate::glue::{GlueSpec, GlueStore, GlueStoreMark};
use crate::ids::{GlueId, MacroDefinitionId, NodeListId, TokenListId};
use crate::interner::{Interner, InternerMark, Symbol};
use crate::macro_store::{MacroMeaning, MacroStore, MacroStoreMark};
use crate::meaning::Meaning;
use crate::node::Node;
use crate::node_arena::{NodeArena, NodeArenaMark, NodeListBuilder};
use crate::scaled::Scaled;
use crate::survivor::SurvivorArena;
use crate::token::Catcode;
use crate::token::Token;
use crate::token_store::{TokenListBuilder, TokenStore, TokenStoreMark};
#[cfg(any(test, feature = "testing", feature = "shadow"))]
use std::hash::{Hash, Hasher};
use std::mem;

mod handles;

#[cfg(any(test, feature = "testing", feature = "shadow"))]
const TESTING_NODE_HASH_MAX_DEPTH: usize = 4096;

/// A rollback snapshot for all currently implemented state stores.
#[derive(Clone, Debug)]
pub struct Snapshot {
    owner: SnapshotOwner,
    env_snapshot: EnvSnapshot,
    interner_mark: InternerMark,
    token_mark: TokenStoreMark,
    macro_mark: MacroStoreMark,
    glue_mark: GlueStoreMark,
    node_mark: NodeArenaMark,
    code_tables_snapshot: CodeTablesSnapshot,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SnapshotOwner(usize);

#[derive(Debug)]
struct StoreOwner(Box<StoreOwnerToken>);

#[derive(Debug)]
struct StoreOwnerToken {
    _private: u8,
}

impl StoreOwner {
    fn new() -> Self {
        Self(Box::new(StoreOwnerToken { _private: 0 }))
    }

    fn snapshot_owner(&self) -> SnapshotOwner {
        SnapshotOwner(self.0.as_ref() as *const StoreOwnerToken as usize)
    }
}

/// Top-level owner for rollback-coupled state stores.
#[derive(Debug)]
pub struct Stores {
    owner: StoreOwner,
    env: Env,
    interner: Interner,
    tokens: TokenStore,
    macros: MacroStore,
    glue: GlueStore,
    nodes: NodeArena,
    survivors: SurvivorArena,
    code_tables: CodeTables,
}

impl Clone for Stores {
    fn clone(&self) -> Self {
        Self {
            owner: StoreOwner::new(),
            env: self.env.clone(),
            interner: self.interner.clone(),
            tokens: self.tokens.clone(),
            macros: self.macros.clone(),
            glue: self.glue.clone(),
            nodes: self.nodes.clone(),
            survivors: self.survivors.clone(),
            code_tables: self.code_tables.clone(),
        }
    }
}

impl Stores {
    /// Creates an empty state-store tuple.
    #[must_use]
    pub fn new() -> Self {
        let mut stores = Self {
            owner: StoreOwner::new(),
            env: Env::new(),
            interner: Interner::new(),
            tokens: TokenStore::new(),
            macros: MacroStore::new(),
            glue: GlueStore::new(),
            nodes: NodeArena::new(),
            survivors: SurvivorArena::new(),
            code_tables: CodeTables::new(),
        };
        stores.set_int_param(IntParam::MAG, 1000);
        stores.set_int_param(IntParam::ESCAPE_CHAR, b'\\'.into());
        stores
    }

    /// Reads the owned environment.
    #[must_use]
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

    #[must_use]
    pub fn lccode(&self, ch: char) -> LcCode {
        self.code_tables.lccode(ch)
    }

    pub fn set_lccode(&mut self, ch: char, value: LcCode) {
        self.code_tables.set_lccode(ch, value);
    }

    #[must_use]
    pub fn uccode(&self, ch: char) -> UcCode {
        self.code_tables.uccode(ch)
    }

    pub fn set_uccode(&mut self, ch: char, value: UcCode) {
        self.code_tables.set_uccode(ch, value);
    }

    #[must_use]
    pub fn sfcode(&self, ch: char) -> SfCode {
        self.code_tables.sfcode(ch)
    }

    pub fn set_sfcode(&mut self, ch: char, value: SfCode) {
        self.code_tables.set_sfcode(ch, value);
    }

    #[must_use]
    pub fn mathcode(&self, ch: char) -> MathCode {
        self.code_tables.mathcode(ch)
    }

    pub fn set_mathcode(&mut self, ch: char, value: MathCode) {
        self.code_tables.set_mathcode(ch, value);
    }

    #[must_use]
    pub fn delcode(&self, ch: char) -> DelCode {
        self.code_tables.delcode(ch)
    }

    pub fn set_delcode(&mut self, ch: char, value: DelCode) {
        self.code_tables.set_delcode(ch, value);
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
        self.env.set_global(symbol, meaning);
    }

    /// Interns a frozen macro definition in the owned macro-definition store.
    pub fn intern_macro(&mut self, macro_meaning: MacroMeaning) -> MacroDefinitionId {
        self.assert_live_token_list(macro_meaning.parameter_text());
        self.assert_live_token_list(macro_meaning.replacement_text());
        self.macros.intern(macro_meaning)
    }

    /// Reads a live frozen macro definition.
    #[must_use]
    pub fn macro_definition(&self, id: MacroDefinitionId) -> MacroMeaning {
        self.assert_live_macro_definition(id);
        self.macros.get(id)
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
        self.interner.intern(name)
    }

    /// Returns the live symbol for an already-interned control-sequence name.
    #[must_use]
    pub fn symbol(&self, name: &str) -> Option<Symbol> {
        self.interner.get(name)
    }

    /// Resolves a live control-sequence symbol.
    #[must_use]
    pub fn resolve(&self, symbol: Symbol) -> &str {
        self.assert_live_symbol(symbol);
        self.interner.resolve(symbol)
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
    pub fn nodes(&self, id: NodeListId) -> &[Node] {
        self.assert_live_node_list(id);
        self.nodes.get(id, &self.survivors)
    }

    /// Enters a TeX group.
    pub fn enter_group(&mut self) {
        self.env.enter_group();
    }

    /// Pushes an opaque `\aftergroup` payload for the current group.
    pub fn push_aftergroup(&mut self, payload: u64) {
        self.env.push_aftergroup(payload);
    }

    /// Leaves the innermost TeX group and returns its `\aftergroup` payloads.
    #[must_use]
    pub fn leave_group(&mut self) -> Vec<u64> {
        self.account_current_group_box_refs();
        self.env.leave_group()
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

    #[must_use]
    pub fn box_reg(&self, index: u16) -> Option<NodeListId> {
        self.env.box_reg(index)
    }

    pub fn take_box_reg(&mut self, index: u16) -> Option<NodeListId> {
        let old = self.env.box_reg(index);
        let rec = self.env.set_box_reg(index, None);
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

    /// Takes an O(1) checkpoint for the rollback-coupled store tuple.
    ///
    /// A `Snapshot` belongs only to the `Stores` instance that created it.
    /// Leaving a TeX group invalidates snapshots taken inside that group:
    /// rollback may only target a snapshot whose captured group depth still
    /// matches the current group depth.
    #[must_use]
    pub fn checkpoint(&mut self) -> Snapshot {
        Snapshot {
            owner: self.owner.snapshot_owner(),
            env_snapshot: self.env.checkpoint(),
            interner_mark: self.interner.watermark(),
            token_mark: self.tokens.watermark(),
            macro_mark: self.macros.watermark(),
            glue_mark: self.glue.watermark(),
            node_mark: self.nodes.watermark(),
            code_tables_snapshot: self.code_tables.checkpoint(),
        }
    }

    /// Rolls all stores back to `snapshot` as one atomic tuple.
    pub fn rollback(&mut self, snapshot: Snapshot) {
        self.assert_valid_snapshot(&snapshot);
        self.account_rollback_box_refs(snapshot.env_snapshot);
        self.env.rollback_to(snapshot.env_snapshot);
        self.interner.truncate_to(snapshot.interner_mark);
        self.tokens.truncate_to(snapshot.token_mark);
        self.macros.truncate_to(snapshot.macro_mark);
        self.glue.truncate_to(snapshot.glue_mark);
        self.nodes.truncate_to(snapshot.node_mark);
        self.code_tables.rollback_to(snapshot.code_tables_snapshot);
    }

    /// Returns the number of journal bytes appended since `snapshot`.
    #[must_use]
    pub fn env_journal_bytes_since(&self, snapshot: Snapshot) -> usize {
        self.assert_valid_snapshot(&snapshot);
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
            self.interner
                .resolve(Symbol::new(raw as u32))
                .hash(&mut hasher);
        }
        self.tokens.testing_state_hash().hash(&mut hasher);
        self.glue.testing_state_hash().hash(&mut hasher);
        self.testing_hash_all_epoch_nodes(&mut hasher);
        self.code_tables.testing_hash_content(&mut hasher);
        hasher.finish()
    }

    #[cfg(any(test, feature = "testing", feature = "shadow"))]
    fn testing_hash_env_by_content(&self, hasher: &mut impl Hasher) {
        let mut pairs = self.env.semantic_non_default_words();
        pairs.sort_by_key(|(cell, _)| *cell);
        for (cell, word) in pairs {
            cell.hash(hasher);
            if cell.bank() == BankTag::Box {
                self.testing_hash_box_word(word, hasher);
            } else {
                word.hash(hasher);
            }
        }
        self.env.testing_aftergroup_payloads().hash(hasher);
    }

    fn assert_valid_snapshot(&self, snapshot: &Snapshot) {
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
            self.testing_hash_node_content_bounded(node, hasher, 0);
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
            self.testing_hash_node_content_bounded(node, hasher, depth);
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
            Node::Glue { spec, kind } => {
                self.glue(*spec).hash(hasher);
                kind.hash(hasher);
            }
            Node::Penalty(value) => value.hash(hasher),
            Node::HList(box_node) | Node::VList(box_node) => {
                box_node.width.raw().hash(hasher);
                box_node.height.raw().hash(hasher);
                box_node.depth.raw().hash(hasher);
                box_node.shift.raw().hash(hasher);
                box_node.glue_set.to_bits().hash(hasher);
                box_node.glue_sign.hash(hasher);
                box_node.glue_order.hash(hasher);
                self.testing_hash_node_list_content_bounded(box_node.children, hasher, depth + 1);
            }
            Node::MathOn
            | Node::MathOff
            | Node::Lig { .. }
            | Node::Rule { .. }
            | Node::Unset
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
    pub fn testing_survivor_refcount(&self, id: NodeListId) -> u32 {
        self.survivors.testing_refcount(id)
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
