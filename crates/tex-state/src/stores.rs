//! Aggregate state stores and atomic rollback boundary.
//!
//! `Stores` is the M1 aggregate owner for state that must checkpoint and
//! roll back together. Later milestones extend the tuple with token, glue, and
//! node arenas; callers still use this boundary instead of rolling back `Env`
//! or any content store independently.

use crate::env::banks::{DimenParam, GlueParam, IntParam, TokParam};
use crate::env::{Env, EnvSnapshot};
use crate::ids::{GlueId, NodeListId, TokenListId};
use crate::interner::{Interner, InternerMark, Symbol};
use crate::meaning::Meaning;
use crate::scaled::Scaled;
#[cfg(any(test, feature = "testing", feature = "shadow"))]
use std::hash::{Hash, Hasher};
use std::mem;

/// A rollback snapshot for all currently implemented state stores.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Snapshot {
    env_snapshot: EnvSnapshot,
    interner_mark: InternerMark,
}

/// Top-level owner for rollback-coupled state stores.
#[derive(Clone, Debug)]
pub struct Stores {
    env: Env,
    interner: Interner,
}

impl Stores {
    /// Creates an empty state-store tuple.
    #[must_use]
    pub fn new() -> Self {
        Self {
            env: Env::new(),
            interner: Interner::new(),
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
        self.env.set(symbol, meaning);
    }

    /// Sets the global meaning for a live control-sequence symbol.
    pub fn set_meaning_global(&mut self, symbol: Symbol, meaning: Meaning) {
        self.assert_live_symbol(symbol);
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
        self.env.leave_group()
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
        self.env.set_skip(index, value);
    }

    pub fn set_skip_global(&mut self, index: u16, value: GlueId) {
        self.env.set_skip_global(index, value);
    }

    pub fn set_toks(&mut self, index: u16, value: TokenListId) {
        self.env.set_toks(index, value);
    }

    pub fn set_toks_global(&mut self, index: u16, value: TokenListId) {
        self.env.set_toks_global(index, value);
    }

    pub fn set_box_reg(&mut self, index: u16, value: NodeListId) {
        self.env.set_box_reg(index, value);
    }

    pub fn set_box_reg_global(&mut self, index: u16, value: NodeListId) {
        self.env.set_box_reg_global(index, value);
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
        self.env.set_glue_param(param, value);
    }

    pub fn set_glue_param_global(&mut self, param: GlueParam, value: GlueId) {
        self.env.set_glue_param_global(param, value);
    }

    pub fn set_tok_param(&mut self, param: TokParam, value: TokenListId) {
        self.env.set_tok_param(param, value);
    }

    pub fn set_tok_param_global(&mut self, param: TokParam, value: TokenListId) {
        self.env.set_tok_param_global(param, value);
    }

    /// Takes an O(1) checkpoint for the rollback-coupled store tuple.
    #[must_use]
    pub fn checkpoint(&mut self) -> Snapshot {
        Snapshot {
            env_snapshot: self.env.checkpoint(),
            interner_mark: self.interner.watermark(),
        }
    }

    /// Rolls all stores back to `snapshot` as one atomic tuple.
    pub fn rollback(&mut self, snapshot: Snapshot) {
        self.env.rollback_to(snapshot.env_snapshot);
        self.interner.truncate_to(snapshot.interner_mark);
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
        hasher.finish()
    }

    fn assert_live_symbol(&self, symbol: Symbol) {
        assert!(
            self.interner.contains(symbol),
            "symbol is not live in this Stores timeline"
        );
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
    use crate::meaning::Meaning;

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
}
