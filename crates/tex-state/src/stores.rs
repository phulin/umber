//! Aggregate state stores and atomic rollback boundary.
//!
//! `Stores` is the M1 aggregate owner for state that must checkpoint and
//! roll back together. Later milestones extend the tuple with token, glue, and
//! node arenas; callers still use this boundary instead of rolling back `Env`
//! or any content store independently.

use crate::env::{Env, EnvSnapshot};
use crate::interner::{Interner, InternerMark, Symbol};
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

    /// Runs barriered environment writes against the owned environment.
    pub fn with_env_mut<R>(&mut self, f: impl FnOnce(&mut Env) -> R) -> R {
        f(&mut self.env)
    }

    /// Reads the owned environment.
    #[must_use]
    pub fn env(&self) -> &Env {
        &self.env
    }

    /// Interns a control-sequence name in the owned interner.
    pub fn intern(&mut self, name: &str) -> Symbol {
        self.interner.intern(name)
    }

    /// Resolves a live control-sequence symbol.
    #[must_use]
    pub fn resolve(&self, symbol: Symbol) -> &str {
        self.interner.resolve(symbol)
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
                .resolve(Symbol::testing_new(raw as u32))
                .hash(&mut hasher);
        }
        hasher.finish()
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
    use crate::interner::Symbol;
    use crate::meaning::Meaning;

    #[test]
    fn rollback_restores_env_and_interner_as_one_tuple() {
        let mut stores = Stores::new();
        let kept = stores.intern("kept");
        stores.with_env_mut(|env| env.set(kept, Meaning::Relax));
        let snapshot = stores.checkpoint();

        let temporary = stores.intern("temporary");
        stores.with_env_mut(|env| env.set(temporary, Meaning::CharGiven('x')));

        stores.rollback(snapshot);

        assert_eq!(stores.resolve(kept), "kept");
        assert_eq!(stores.env().get(kept), Meaning::Relax);
        let reused = stores.intern("temporary");
        assert_eq!(reused.raw(), temporary.raw());
        assert_eq!(
            stores.env().get(Symbol::testing_new(reused.raw())),
            Meaning::Undefined
        );
    }

    #[test]
    fn rollback_discards_aftergroup_payloads_pushed_after_snapshot() {
        let mut stores = Stores::new();
        stores.with_env_mut(|env| env.enter_group());
        let snapshot = stores.checkpoint();

        stores.with_env_mut(|env| env.push_aftergroup(99));
        stores.rollback(snapshot);

        assert_eq!(
            stores.with_env_mut(|env| env.leave_group()),
            Vec::<u64>::new()
        );
    }
}
