//! Control-sequence name interning.
//!
//! Symbols are dense indexes into a span table. The arena can be truncated to
//! a watermark, but that rollback machinery is crate-private so the live
//! interner rolls back only as part of the aggregate `Universe` tuple.

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::str;

/// The TeX82 control-sequence namespace containing an interned symbol.
///
/// Active characters and escaped names have distinct meanings even when
/// their printed spelling is the same (for example active `~` and `\~`).
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ControlSequenceKind {
    /// A name scanned after an escape character or manufactured by `\csname`.
    Named,
    /// A character whose current category code is active.
    ActiveCharacter,
}

/// A dense interned-name identifier.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Symbol(u32);

/// Maximum number of symbols that can be represented in a packed token word.
pub const SYMBOL_CAPACITY: u32 = 1 << 30;

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

/// Failure to intern a new control-sequence name.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InternerError {
    /// The dense symbol space used by packed traced tokens is exhausted.
    TooManySymbols,
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
    kinds: Vec<ControlSequenceKind>,
    next_symbol: u32,
    index: HashMap<u64, Vec<Symbol>>,
    index_dirty: bool,
}

impl Interner {
    /// Creates an empty interner.
    #[must_use]
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Interns `name`, returning its stable dense symbol while it remains live.
    pub(crate) fn intern(&mut self, name: &str) -> Result<Symbol, InternerError> {
        self.intern_key(ControlSequenceKind::Named, name)
    }

    /// Interns an active-character control sequence.
    pub(crate) fn intern_active(&mut self, ch: char) -> Result<Symbol, InternerError> {
        let mut encoded = [0; 4];
        self.intern_key(
            ControlSequenceKind::ActiveCharacter,
            ch.encode_utf8(&mut encoded),
        )
    }

    fn intern_key(
        &mut self,
        kind: ControlSequenceKind,
        name: &str,
    ) -> Result<Symbol, InternerError> {
        if self.index_dirty {
            self.rebuild_index();
        }

        let hash = content_hash(kind, name);
        if let Some(candidates) = self.index.get(&hash) {
            for &symbol in candidates {
                if self.kind(symbol) == kind && self.resolve(symbol) == name {
                    return Ok(symbol);
                }
            }
        }

        if self.next_symbol >= SYMBOL_CAPACITY {
            return Err(InternerError::TooManySymbols);
        }

        let start = u32_len(self.arena.len(), "interner arena exceeds u32 bytes");
        let len = u32_len(name.len(), "interned string exceeds u32 bytes");
        let symbol = Symbol::new(self.next_symbol);

        self.arena.extend_from_slice(name.as_bytes());
        self.spans.push((start, len));
        self.kinds.push(kind);
        self.next_symbol += 1;
        self.index.entry(hash).or_default().push(symbol);

        Ok(symbol)
    }

    /// Returns the live symbol for `name` without mutating the interner.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<Symbol> {
        self.get_key(ControlSequenceKind::Named, name)
    }

    /// Returns the live symbol for an active character without mutating.
    #[must_use]
    pub fn get_active(&self, ch: char) -> Option<Symbol> {
        let mut encoded = [0; 4];
        self.get_key(
            ControlSequenceKind::ActiveCharacter,
            ch.encode_utf8(&mut encoded),
        )
    }

    fn get_key(&self, kind: ControlSequenceKind, name: &str) -> Option<Symbol> {
        let hash = content_hash(kind, name);
        self.index.get(&hash).and_then(|candidates| {
            candidates.iter().copied().find(|&symbol| {
                self.contains(symbol) && self.kind(symbol) == kind && self.resolve(symbol) == name
            })
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

    /// Returns the TeX control-sequence namespace of a live symbol.
    #[must_use]
    pub fn kind(&self, symbol: Symbol) -> ControlSequenceKind {
        let index = symbol.raw() as usize;
        assert!(index < self.kinds.len(), "symbol is not live");
        self.kinds[index]
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
        debug_assert_eq!(self.next_symbol as usize, self.spans.len());
        debug_assert_eq!(self.kinds.len(), self.spans.len());
        InternerMark {
            spans: self.next_symbol,
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
        self.kinds.truncate(spans);
        self.arena.truncate(bytes);
        self.next_symbol = mark.spans;
        debug_assert_eq!(self.kinds.len(), self.spans.len());
        self.index_dirty = true;
    }

    fn rebuild_index(&mut self) {
        self.index.clear();
        for raw in 0..self.spans.len() {
            let symbol = Symbol::new(u32_len(raw, "interner spans exceed u32 entries"));
            let hash = content_hash(self.kind(symbol), self.resolve(symbol));
            self.index.entry(hash).or_default().push(symbol);
        }
        self.index_dirty = false;
    }
}

fn content_hash(kind: ControlSequenceKind, name: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    // PERF: revisit hasher (fastpaths epic).
    kind.hash(&mut hasher);
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
mod tests;
