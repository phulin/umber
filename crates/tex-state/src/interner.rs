//! Control-sequence name interning.
//!
//! Symbols are dense indexes into a span table. The arena can be truncated to
//! a watermark, but that rollback machinery is crate-private so the live
//! interner rolls back only as part of the aggregate `Universe` tuple.

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::str;

/// A dense interned-name identifier.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Symbol(u32);

impl Symbol {
    pub(crate) const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// Creates a symbol for tests that need direct cell-level state coverage.
    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub const fn testing_new(raw: u32) -> Self {
        Self(raw)
    }

    /// Returns the dense symbol index.
    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }
}

/// A rollback watermark for the interner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct InternerMark {
    spans: u32,
    bytes: u32,
}

/// Interned UTF-8 string arena.
#[derive(Clone, Debug, Default)]
pub struct Interner {
    arena: Vec<u8>,
    spans: Vec<(u32, u32)>,
    index: HashMap<u64, Vec<Symbol>>,
    index_dirty: bool,
}

impl Interner {
    /// Creates an empty interner.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Interns `name`, returning its stable dense symbol while it remains live.
    pub fn intern(&mut self, name: &str) -> Symbol {
        if self.index_dirty {
            self.rebuild_index();
        }

        let hash = content_hash(name);
        if let Some(candidates) = self.index.get(&hash) {
            for &symbol in candidates {
                if self.resolve(symbol) == name {
                    return symbol;
                }
            }
        }

        let start = u32_len(self.arena.len(), "interner arena exceeds u32 bytes");
        let len = u32_len(name.len(), "interned string exceeds u32 bytes");
        let symbol = Symbol::new(u32_len(
            self.spans.len(),
            "interner spans exceed u32 entries",
        ));

        self.arena.extend_from_slice(name.as_bytes());
        self.spans.push((start, len));
        self.index.entry(hash).or_default().push(symbol);

        symbol
    }

    /// Returns the live symbol for `name` without mutating the interner.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<Symbol> {
        let hash = content_hash(name);
        self.index.get(&hash).and_then(|candidates| {
            candidates
                .iter()
                .copied()
                .find(|&symbol| self.resolve(symbol) == name)
        })
    }

    /// Resolves a live symbol to its interned string.
    #[must_use]
    pub fn resolve(&self, symbol: Symbol) -> &str {
        let index = symbol.raw() as usize;
        assert!(index < self.spans.len(), "symbol is not live");
        let (start, len) = self.spans[index];
        let start = start as usize;
        let end = start + len as usize;
        assert!(end <= self.arena.len(), "symbol span exceeds arena");

        match str::from_utf8(&self.arena[start..end]) {
            Ok(name) => name,
            Err(_) => panic!("interner arena contains invalid UTF-8"),
        }
    }

    /// Returns whether `symbol` names a currently-live interner slot.
    #[must_use]
    pub fn contains(&self, symbol: Symbol) -> bool {
        (symbol.raw() as usize) < self.spans.len()
    }

    /// Returns the number of live interned names.
    #[must_use]
    pub fn len(&self) -> usize {
        self.spans.len()
    }

    /// Returns whether there are no live interned names.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.spans.is_empty()
    }

    /// Takes a rollback watermark for `Universe`-owned aggregate snapshots.
    #[must_use]
    pub(crate) fn watermark(&self) -> InternerMark {
        InternerMark {
            spans: u32_len(self.spans.len(), "interner spans exceed u32 entries"),
            bytes: u32_len(self.arena.len(), "interner arena exceeds u32 bytes"),
        }
    }

    /// Truncates to a previously-taken aggregate snapshot watermark.
    pub(crate) fn truncate_to(&mut self, mark: InternerMark) {
        let spans = mark.spans as usize;
        let bytes = mark.bytes as usize;
        assert!(
            spans <= self.spans.len(),
            "interner mark has too many spans"
        );
        assert!(
            bytes <= self.arena.len(),
            "interner mark has too many bytes"
        );
        assert!(
            self.spans[..spans]
                .last()
                .map_or(mark.bytes == 0, |&(start, len)| start + len == mark.bytes),
            "interner mark does not point to a span boundary"
        );

        self.spans.truncate(spans);
        self.arena.truncate(bytes);
        self.index_dirty = true;
    }

    fn rebuild_index(&mut self) {
        self.index.clear();
        for raw in 0..self.spans.len() {
            let symbol = Symbol::new(u32_len(raw, "interner spans exceed u32 entries"));
            let hash = content_hash(self.resolve(symbol));
            self.index.entry(hash).or_default().push(symbol);
        }
        self.index_dirty = false;
    }
}

fn content_hash(name: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    // PERF: revisit hasher (fastpaths epic).
    name.hash(&mut hasher);
    hasher.finish()
}

fn u32_len(value: usize, message: &str) -> u32 {
    match u32::try_from(value) {
        Ok(value) => value,
        Err(_) => panic!("{message}"),
    }
}

#[cfg(test)]
mod tests {
    use super::{Interner, InternerMark};
    use proptest::prelude::*;

    #[test]
    fn intern_is_idempotent() {
        let mut interner = Interner::new();

        let first = interner.intern("count");
        let second = interner.intern("count");

        assert_eq!(first, second);
        assert_eq!(first.raw(), 0);
        assert_eq!(interner.len(), 1);
    }

    #[test]
    fn resolve_round_trips_ascii_and_non_ascii() {
        let mut interner = Interner::new();

        let ascii = interner.intern("par");
        let non_ascii = interner.intern("é漢字🙂");

        assert_eq!(interner.resolve(ascii), "par");
        assert_eq!(interner.resolve(non_ascii), "é漢字🙂");
    }

    #[test]
    fn truncate_then_reintern_reuses_dense_symbol_id() {
        let mut interner = Interner::new();

        let kept = interner.intern("kept");
        let mark = interner.watermark();
        let truncated = interner.intern("temporary");
        assert_eq!(truncated.raw(), 1);

        interner.truncate_to(mark);
        assert_eq!(interner.len(), 1);
        assert_eq!(interner.resolve(kept), "kept");

        let reinserted = interner.intern("temporary");
        assert_eq!(reinserted.raw(), truncated.raw());
        assert_eq!(interner.resolve(reinserted), "temporary");
    }

    #[test]
    #[should_panic(expected = "symbol is not live")]
    fn stale_symbol_panics_after_truncation() {
        let mut interner = Interner::new();
        let mark = interner.watermark();
        let stale = interner.intern("rolled-back");

        interner.truncate_to(mark);

        let _ = interner.resolve(stale);
    }

    #[derive(Clone, Debug)]
    enum Op {
        Intern(String),
        Mark,
        TruncateToMark(usize),
    }

    prop_compose! {
        fn intern_name()(name in "\\PC{0,8}") -> String {
            name
        }
    }

    fn op_strategy() -> impl Strategy<Value = Op> {
        prop_oneof![
            intern_name().prop_map(Op::Intern),
            Just(Op::Mark),
            any::<usize>().prop_map(Op::TruncateToMark),
        ]
    }

    proptest! {
        #[test]
        fn arbitrary_intern_and_truncate_sequences_match_naive_model(
            ops in prop::collection::vec(op_strategy(), 0..256)
        ) {
            let mut interner = Interner::new();
            let mut model: Vec<String> = Vec::new();
            let mut marks: Vec<(InternerMark, usize)> = vec![(interner.watermark(), 0)];

            for op in ops {
                match op {
                    Op::Intern(name) => {
                        let symbol = interner.intern(&name);
                        let model_index = match model.iter().position(|existing| existing == &name) {
                            Some(index) => index,
                            None => {
                                model.push(name.clone());
                                model.len() - 1
                            }
                        };

                        prop_assert_eq!(symbol.raw() as usize, model_index);
                        prop_assert_eq!(interner.resolve(symbol), name.as_str());
                    }
                    Op::Mark => {
                        marks.push((interner.watermark(), model.len()));
                    }
                    Op::TruncateToMark(raw_index) => {
                        let index = raw_index % marks.len();
                        let (mark, model_len) = marks[index];
                        interner.truncate_to(mark);
                        model.truncate(model_len);
                        marks.retain(|&(_, len)| len <= model_len);
                    }
                }

                prop_assert_eq!(interner.len(), model.len());
                for (raw, expected) in model.iter().enumerate() {
                    let symbol = super::Symbol::new(raw as u32);
                    prop_assert_eq!(interner.resolve(symbol), expected.as_str());
                    prop_assert_eq!(interner.intern(expected).raw() as usize, raw);
                }
            }
        }
    }
}
