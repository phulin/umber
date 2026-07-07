//! Aggregate state stores and atomic rollback boundary.
//!
//! `Stores` is hidden M3 scaffolding for state that must checkpoint and roll
//! back together until `Universe` subsumes this aggregate boundary. Callers use
//! this boundary instead of rolling back `Env` or content stores independently.

use crate::cell::BankTag;
use crate::env::banks::{DimenParam, GlueParam, IntParam, TokParam};
use crate::env::{Env, EnvSnapshot};
use crate::glue::{GlueSpec, GlueStore, GlueStoreMark};
use crate::ids::{ArenaRef, GlueId, NodeListId, TokenListId};
use crate::interner::{Interner, InternerMark, Symbol};
use crate::meaning::Meaning;
use crate::node::Node;
use crate::node_arena::{NodeArena, NodeArenaMark, NodeListBuilder};
use crate::scaled::Scaled;
use crate::survivor::SurvivorArena;
use crate::token::Token;
use crate::token_store::{TokenListBuilder, TokenStore, TokenStoreMark};
#[cfg(any(test, feature = "testing", feature = "shadow"))]
use std::hash::{Hash, Hasher};
use std::mem;

/// A rollback snapshot for all currently implemented state stores.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Snapshot {
    env_snapshot: EnvSnapshot,
    interner_mark: InternerMark,
    token_mark: TokenStoreMark,
    glue_mark: GlueStoreMark,
    node_mark: NodeArenaMark,
}

/// Top-level owner for rollback-coupled state stores.
#[derive(Clone, Debug)]
pub struct Stores {
    env: Env,
    interner: Interner,
    tokens: TokenStore,
    glue: GlueStore,
    nodes: NodeArena,
    survivors: SurvivorArena,
}

impl Stores {
    /// Creates an empty state-store tuple.
    #[must_use]
    pub fn new() -> Self {
        Self {
            env: Env::new(),
            interner: Interner::new(),
            tokens: TokenStore::new(),
            glue: GlueStore::new(),
            nodes: NodeArena::new(),
            survivors: SurvivorArena::new(),
        }
    }

    /// Reads the owned environment.
    #[must_use]
    pub fn env(&self) -> &Env {
        &self.env
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
        self.assert_live_token_list_in_meaning(meaning);
        self.env.set(symbol, meaning);
    }

    /// Sets the global meaning for a live control-sequence symbol.
    pub fn set_meaning_global(&mut self, symbol: Symbol, meaning: Meaning) {
        self.assert_live_symbol(symbol);
        self.assert_live_token_list_in_meaning(meaning);
        self.env.set_global(symbol, meaning);
    }

    /// Interns a control-sequence name in the owned interner.
    pub fn intern(&mut self, name: &str) -> Symbol {
        self.interner.intern(name)
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
        self.tokens.builder()
    }

    /// Interns a frozen token-list value in the owned token store.
    pub fn intern_token_list(&mut self, tokens: &[Token]) -> TokenListId {
        self.tokens.intern(tokens)
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
        self.nodes.builder()
    }

    /// Appends and freezes a node list in the owned epoch arena.
    pub fn freeze_node_list(&mut self, nodes: &[Node]) -> NodeListId {
        let mut builder = NodeListBuilder::new();
        for node in nodes {
            builder.push(node.clone());
        }
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
        let entries = self.env.current_group_entries().to_vec();
        let payloads = self.env.leave_group();
        self.adjust_box_refs_for_group_exit(&entries);
        payloads
    }

    pub fn set_count(&mut self, index: u16, value: i32) {
        self.env.set_count(index, value);
    }

    pub fn set_count_global(&mut self, index: u16, value: i32) {
        self.env.set_count_global(index, value);
    }

    pub fn set_dimen(&mut self, index: u16, value: Scaled) {
        self.env.set_dimen(index, value);
    }

    pub fn set_dimen_global(&mut self, index: u16, value: Scaled) {
        self.env.set_dimen_global(index, value);
    }

    pub fn set_skip(&mut self, index: u16, value: GlueId) {
        self.assert_live_glue(value);
        self.env.set_skip(index, value);
    }

    pub fn set_skip_global(&mut self, index: u16, value: GlueId) {
        self.assert_live_glue(value);
        self.env.set_skip_global(index, value);
    }

    pub fn set_toks(&mut self, index: u16, value: TokenListId) {
        self.assert_live_token_list(value);
        self.env.set_toks(index, value);
    }

    pub fn set_toks_global(&mut self, index: u16, value: TokenListId) {
        self.assert_live_token_list(value);
        self.env.set_toks_global(index, value);
    }

    pub fn set_box_reg(&mut self, index: u16, value: NodeListId) {
        let value = self.prepare_box_value(value);
        let old = self.env.box_reg(index);
        let start = self.env.journal_pos();
        self.env.set_box_reg(index, Some(value));
        self.hold_new_box_journal_refs(start);
        self.dec_if_survivor(old);
    }

    pub fn set_box_reg_global(&mut self, index: u16, value: NodeListId) {
        let value = self.prepare_box_value(value);
        let old = self.env.box_reg(index);
        let start = self.env.journal_pos();
        self.env.set_box_reg_global(index, Some(value));
        self.hold_new_box_journal_refs(start);
        self.dec_if_survivor(old);
    }

    #[must_use]
    pub fn box_reg(&self, index: u16) -> Option<NodeListId> {
        self.env.box_reg(index)
    }

    pub fn take_box_reg(&mut self, index: u16) -> Option<NodeListId> {
        let start = self.env.journal_pos();
        let old = self.env.take_box_reg(index);
        self.hold_new_box_journal_refs(start);
        self.dec_if_survivor(old);
        old
    }

    pub fn set_int_param(&mut self, param: IntParam, value: i32) {
        self.env.set_int_param(param, value);
    }

    pub fn set_int_param_global(&mut self, param: IntParam, value: i32) {
        self.env.set_int_param_global(param, value);
    }

    pub fn set_dimen_param(&mut self, param: DimenParam, value: Scaled) {
        self.env.set_dimen_param(param, value);
    }

    pub fn set_dimen_param_global(&mut self, param: DimenParam, value: Scaled) {
        self.env.set_dimen_param_global(param, value);
    }

    pub fn set_glue_param(&mut self, param: GlueParam, value: GlueId) {
        self.assert_live_glue(value);
        self.env.set_glue_param(param, value);
    }

    pub fn set_glue_param_global(&mut self, param: GlueParam, value: GlueId) {
        self.assert_live_glue(value);
        self.env.set_glue_param_global(param, value);
    }

    pub fn set_tok_param(&mut self, param: TokParam, value: TokenListId) {
        self.assert_live_token_list(value);
        self.env.set_tok_param(param, value);
    }

    pub fn set_tok_param_global(&mut self, param: TokParam, value: TokenListId) {
        self.assert_live_token_list(value);
        self.env.set_tok_param_global(param, value);
    }

    /// Takes an O(1) checkpoint for the rollback-coupled store tuple.
    #[must_use]
    pub fn checkpoint(&mut self) -> Snapshot {
        Snapshot {
            env_snapshot: self.env.checkpoint(),
            interner_mark: self.interner.watermark(),
            token_mark: self.tokens.watermark(),
            glue_mark: self.glue.watermark(),
            node_mark: self.nodes.watermark(),
        }
    }

    /// Rolls all stores back to `snapshot` as one atomic tuple.
    pub fn rollback(&mut self, snapshot: Snapshot) {
        let entries = self
            .env
            .journal_entries_since(snapshot.env_snapshot.journal_pos())
            .to_vec();
        self.env.rollback_to(snapshot.env_snapshot);
        self.adjust_box_refs_for_restore(&entries);
        self.interner.truncate_to(snapshot.interner_mark);
        self.tokens.truncate_to(snapshot.token_mark);
        self.glue.truncate_to(snapshot.glue_mark);
        self.nodes.truncate_to(snapshot.node_mark);
    }

    /// Returns the number of journal bytes appended since `snapshot`.
    #[must_use]
    pub fn env_journal_bytes_since(&self, snapshot: Snapshot) -> usize {
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
        self.env.testing_state_hash().hash(&mut hasher);
        self.interner.len().hash(&mut hasher);
        for raw in 0..self.interner.len() {
            self.interner
                .resolve(Symbol::new(raw as u32))
                .hash(&mut hasher);
        }
        self.tokens.testing_state_hash().hash(&mut hasher);
        self.glue.testing_state_hash().hash(&mut hasher);
        hasher.finish()
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

    fn assert_live_symbol(&self, symbol: Symbol) {
        assert!(
            self.interner.contains(symbol),
            "symbol is not live in this Stores timeline"
        );
    }

    fn assert_live_token_list(&self, id: TokenListId) {
        assert!(
            self.tokens.contains(id),
            "token list is not live in this Stores timeline"
        );
    }

    fn assert_live_glue(&self, id: GlueId) {
        assert!(
            self.glue.contains(id),
            "glue id is not live in this Stores timeline"
        );
    }

    fn assert_live_node_list(&self, id: NodeListId) {
        let live = match id.arena() {
            ArenaRef::Epoch => self.nodes.contains(id),
            ArenaRef::Survivor(_) => self.survivors.contains(id),
        };
        assert!(live, "node list is not live in this Stores timeline");
    }

    fn assert_live_token_list_in_meaning(&self, meaning: Meaning) {
        if let Meaning::Macro { token_list, .. } = meaning {
            self.assert_live_token_list(token_list);
        }
    }

    fn prepare_box_value(&mut self, value: NodeListId) -> NodeListId {
        self.assert_live_node_list(value);
        match value.arena() {
            ArenaRef::Epoch => self.survivors.promote(value, &self.nodes),
            ArenaRef::Survivor(_) => {
                self.survivors.inc_ref(value);
                value
            }
        }
    }

    fn adjust_box_refs_for_restore(&mut self, entries: &[crate::journal::Entry]) {
        for entry in entries.iter().rev() {
            let crate::journal::Entry::Undo(rec) = entry else {
                continue;
            };
            if rec.cell().bank() != BankTag::Box {
                continue;
            }
            self.inc_if_survivor(NodeListId::decode_box_word(rec.old()));
            self.dec_if_survivor(NodeListId::decode_box_word(rec.new_value()));
            self.dec_if_survivor(NodeListId::decode_box_word(rec.old()));
        }
    }

    fn adjust_box_refs_for_group_exit(&mut self, entries: &[crate::journal::Entry]) {
        for entry in entries.iter().rev() {
            let crate::journal::Entry::Undo(rec) = entry else {
                continue;
            };
            if rec.cell().bank() != BankTag::Box || rec.cell().is_global() {
                continue;
            }
            self.inc_if_survivor(NodeListId::decode_box_word(rec.old()));
            self.dec_if_survivor(NodeListId::decode_box_word(rec.new_value()));
            self.dec_if_survivor(NodeListId::decode_box_word(rec.old()));
        }
    }

    fn hold_new_box_journal_refs(&mut self, start: crate::journal::JournalPos) {
        let entries = self.env.journal_entries_since(start).to_vec();
        for entry in entries {
            let crate::journal::Entry::Undo(rec) = entry else {
                continue;
            };
            if rec.cell().bank() == BankTag::Box {
                self.inc_if_survivor(NodeListId::decode_box_word(rec.old()));
            }
        }
    }

    fn inc_if_survivor(&mut self, value: Option<NodeListId>) {
        if let Some(id) = value
            && matches!(id.arena(), ArenaRef::Survivor(_))
        {
            self.survivors.inc_ref(id);
        }
    }

    fn dec_if_survivor(&mut self, value: Option<NodeListId>) {
        if let Some(id) = value
            && matches!(id.arena(), ArenaRef::Survivor(_))
        {
            self.survivors.dec_ref(id);
        }
    }
}

impl Default for Stores {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::Stores;
    use crate::glue::{GlueSpec, Order};
    use crate::ids::{ArenaRef, FontId, NodeListId};
    use crate::meaning::Meaning;
    use crate::node::{BoxNode, BoxNodeFields, Node, Sign};
    use crate::scaled::Scaled;

    #[test]
    fn rollback_restores_env_and_interner_as_one_tuple() {
        let mut stores = Stores::new();
        let kept = stores.intern("kept");
        stores.set_meaning(kept, Meaning::Relax);
        let snapshot = stores.checkpoint();

        let temporary = stores.intern("temporary");
        stores.set_meaning(temporary, Meaning::CharGiven('x'));

        stores.rollback(snapshot);

        assert_eq!(stores.resolve(kept), "kept");
        assert_eq!(stores.meaning(kept), Meaning::Relax);
        let reused = stores.intern("temporary");
        assert_eq!(reused.raw(), temporary.raw());
        assert_eq!(stores.meaning(reused), Meaning::Undefined);
    }

    #[test]
    fn rollback_restores_token_store_as_part_of_snapshot_tuple() {
        let mut stores = Stores::new();
        let snapshot = stores.checkpoint();
        let stale = stores.intern_token_list(&[crate::token::Token::param(1)]);

        stores.rollback(snapshot);
        let reused = stores.intern_token_list(&[crate::token::Token::param(2)]);

        assert_eq!(reused.raw(), stale.raw());
        assert_eq!(stores.tokens(reused), &[crate::token::Token::param(2)]);
    }

    #[test]
    fn rollback_restores_glue_store_as_part_of_snapshot_tuple() {
        let mut stores = Stores::new();
        let snapshot = stores.checkpoint();
        let stale = stores.intern_glue(glue_spec(1));

        stores.rollback(snapshot);
        let reused = stores.intern_glue(glue_spec(2));

        assert_eq!(reused.raw(), stale.raw());
        assert_eq!(stores.glue(reused), glue_spec(2));
        assert_eq!(stores.glue(crate::ids::GlueId::ZERO), GlueSpec::ZERO);
    }

    #[test]
    #[should_panic(expected = "token list is not live in this Stores timeline")]
    fn stale_rolled_back_token_list_cannot_mutate_toks_register() {
        let mut stores = Stores::new();
        let snapshot = stores.checkpoint();
        let stale = stores.intern_token_list(&[crate::token::Token::param(1)]);

        stores.rollback(snapshot);
        stores.set_toks(0, stale);
    }

    #[test]
    #[should_panic(expected = "glue id is not live in this Stores timeline")]
    fn stale_rolled_back_glue_cannot_mutate_skip_register() {
        let mut stores = Stores::new();
        let snapshot = stores.checkpoint();
        let stale = stores.intern_glue(glue_spec(1));

        stores.rollback(snapshot);
        stores.set_skip(0, stale);
    }

    #[test]
    fn rollback_discards_aftergroup_payloads_pushed_after_snapshot() {
        let mut stores = Stores::new();
        stores.enter_group();
        let snapshot = stores.checkpoint();

        stores.push_aftergroup(99);
        stores.rollback(snapshot);

        assert_eq!(stores.leave_group(), Vec::<u64>::new());
    }

    #[test]
    #[should_panic(expected = "symbol is not live in this Stores timeline")]
    fn stale_rolled_back_symbol_cannot_write_reused_meaning_cell() {
        let mut stores = Stores::new();
        let snapshot = stores.checkpoint();
        let stale = stores.intern("rolled-back");

        stores.rollback(snapshot);
        stores.set_meaning(stale, Meaning::Relax);
    }

    #[test]
    fn same_epoch_list_stored_twice_promotes_to_independent_roots() {
        let mut stores = Stores::new();
        let list = one_char(&mut stores, 'a');

        stores.set_box_reg(0, list);
        stores.set_box_reg(1, list);

        let first = stores.box_reg(0).expect("box 0 should be non-void");
        let second = stores.box_reg(1).expect("box 1 should be non-void");
        assert_ne!(first.arena(), second.arena());
        assert_eq!(stores.testing_live_survivor_slot_count(), 2);
        assert_eq!(stores.testing_survivor_refcount(first), 1);
        assert_eq!(stores.testing_survivor_refcount(second), 1);
    }

    #[test]
    fn storing_survivor_in_second_register_shares_refcount_until_release() {
        let mut stores = Stores::new();
        let list = one_char(&mut stores, 'a');

        stores.set_box_reg(0, list);
        let survivor = stores.box_reg(0).expect("box should be non-void");
        stores.set_box_reg(1, survivor);

        assert_eq!(stores.testing_live_survivor_slot_count(), 1);
        assert_eq!(stores.testing_survivor_refcount(survivor), 2);

        assert_eq!(stores.take_box_reg(0), Some(survivor));
        assert_eq!(stores.testing_survivor_refcount(survivor), 1);

        let replacement = one_char(&mut stores, 'b');
        stores.set_box_reg(1, replacement);
        assert_eq!(stores.testing_live_survivor_slot_count(), 1);
    }

    #[test]
    fn group_exit_and_rollback_restore_box_refs_once() {
        let mut stores = Stores::new();
        let outer = one_char(&mut stores, 'o');
        stores.set_box_reg(0, outer);
        let baseline = stores.box_reg(0).expect("outer box should be stored");
        let snapshot = stores.checkpoint();

        stores.enter_group();
        let inner = one_char(&mut stores, 'i');
        stores.set_box_reg(0, inner);
        assert_eq!(stores.testing_live_survivor_slot_count(), 2);

        assert_eq!(stores.leave_group(), Vec::<u64>::new());
        assert_eq!(stores.box_reg(0), Some(baseline));
        assert_eq!(stores.testing_live_survivor_slot_count(), 1);
        assert_eq!(stores.testing_survivor_refcount(baseline), 1);

        stores.rollback(snapshot);
        assert_eq!(stores.box_reg(0), Some(baseline));
        assert_eq!(stores.testing_live_survivor_slot_count(), 1);
        assert_eq!(stores.testing_survivor_refcount(baseline), 1);
    }

    #[test]
    fn promoted_nested_box_remaps_children_to_same_survivor_root() {
        let mut stores = Stores::new();
        let inner = one_char(&mut stores, 'x');
        let middle = stores.freeze_node_list(&[Node::HList(BoxNode::new(BoxNodeFields {
            width: scaled(10),
            height: scaled(7),
            depth: scaled(3),
            shift: scaled(0),
            glue_set: 0.0,
            glue_sign: Sign::Normal,
            glue_order: Order::Normal,
            children: inner,
        }))]);
        let outer = stores.freeze_node_list(&[Node::VList(BoxNode::new(BoxNodeFields {
            width: scaled(20),
            height: scaled(9),
            depth: scaled(4),
            shift: scaled(0),
            glue_set: 0.0,
            glue_sign: Sign::Normal,
            glue_order: Order::Normal,
            children: middle,
        }))]);

        stores.set_box_reg(0, outer);
        let promoted_outer = stores.box_reg(0).expect("box should be promoted");
        let [Node::VList(outer_box)] = stores.nodes(promoted_outer) else {
            panic!("outer survivor list should contain one vlist");
        };
        assert_same_root(promoted_outer, outer_box.children);
        let [Node::HList(middle_box)] = stores.nodes(outer_box.children) else {
            panic!("middle survivor list should contain one hlist");
        };
        assert_same_root(promoted_outer, middle_box.children);
        assert_eq!(
            stores.nodes(middle_box.children),
            &[Node::Char {
                font: FontId::testing_new(1),
                ch: 'x'
            }]
        );
    }

    fn glue_spec(width: i32) -> GlueSpec {
        GlueSpec {
            width: Scaled::from_raw(width),
            stretch: Scaled::from_raw(2),
            stretch_order: Order::Fil,
            shrink: Scaled::from_raw(3),
            shrink_order: Order::Fill,
        }
    }

    fn one_char(stores: &mut Stores, ch: char) -> NodeListId {
        stores.freeze_node_list(&[Node::Char {
            font: FontId::testing_new(1),
            ch,
        }])
    }

    fn assert_same_root(a: NodeListId, b: NodeListId) {
        let (ArenaRef::Survivor(a), ArenaRef::Survivor(b)) = (a.arena(), b.arena()) else {
            panic!("expected survivor ids");
        };
        assert_eq!(a, b);
    }

    fn scaled(raw: i32) -> Scaled {
        Scaled::from_raw(raw)
    }
}
