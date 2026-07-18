//! Control-sequence name interning.
//!
//! Symbols are dense indexes into a span table. The arena can be truncated to
//! a watermark, but that rollback machinery is crate-private so the live
//! interner rolls back only as part of the aggregate `Universe` tuple.

use crate::identity::{HandleIdentity, IdentityAllocator, IdentityMark};
use crate::state_hash::StateHasher;
use ahash::{AHashMap, AHasher};
use std::hash::{Hash, Hasher};
use std::sync::{OnceLock, RwLock};

static GLOBAL_SYMBOLS: OnceLock<RwLock<GlobalSymbols>> = OnceLock::new();

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

/// A permanent process-wide key for a semantic control-sequence name.
///
/// Keys fit the 30-bit token payload, are never reused, and resolve to dense
/// local interner slots only through the owning aggregate.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Symbol(u32);

/// A live generation-tagged capability for an interned control-sequence name.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SymbolId {
    identity: HandleIdentity,
    symbol: Symbol,
}

/// A live symbol capability or an explicitly compact token/Env symbol key.
pub trait SymbolReference: Copy {
    #[doc(hidden)]
    fn live_id(self) -> Option<SymbolId>;
    #[doc(hidden)]
    fn stored_key(self) -> Option<Symbol>;
}

impl SymbolReference for SymbolId {
    fn live_id(self) -> Option<SymbolId> {
        Some(self)
    }
    fn stored_key(self) -> Option<Symbol> {
        None
    }
}

impl SymbolReference for Symbol {
    fn live_id(self) -> Option<SymbolId> {
        None
    }
    fn stored_key(self) -> Option<Symbol> {
        Some(self)
    }
}

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

    /// Returns the compact permanent symbol key.
    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }

    /// Returns this compact token/Env key (parallel to `SymbolId::symbol`).
    #[must_use]
    pub const fn symbol(self) -> Self {
        self
    }
}

impl SymbolId {
    const fn from_identity(identity: HandleIdentity, symbol: Symbol) -> Self {
        Self { identity, symbol }
    }
    const fn identity(self) -> HandleIdentity {
        self.identity
    }
    #[must_use]
    pub const fn symbol(self) -> Symbol {
        self.symbol
    }
    #[must_use]
    pub const fn raw(self) -> u32 {
        self.identity.slot()
    }
}

/// Failure to intern a new control-sequence name.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InternerError {
    /// The process-wide compact key space used by packed tokens is exhausted.
    TooManySymbols,
}

/// A rollback watermark for the interner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct InternerMark {
    spans: u32,
    bytes: u32,
    identities: IdentityMark,
}

/// Interned UTF-8 string arena.
#[derive(Debug)]
pub struct Interner {
    arena: String,
    spans: Vec<(u32, u32)>,
    kinds: Vec<ControlSequenceKind>,
    semantic_atoms: Vec<u64>,
    symbols: Vec<Symbol>,
    symbol_slots: AHashMap<Symbol, u32>,
    index: AHashMap<u64, Vec<SymbolId>>,
    index_dirty: bool,
    identities: IdentityAllocator,
}

impl Clone for Interner {
    fn clone(&self) -> Self {
        Self {
            arena: self.arena.clone(),
            spans: self.spans.clone(),
            kinds: self.kinds.clone(),
            semantic_atoms: self.semantic_atoms.clone(),
            symbols: self.symbols.clone(),
            symbol_slots: self.symbol_slots.clone(),
            index: self.index.clone(),
            index_dirty: self.index_dirty,
            identities: self.identities.fork(),
        }
    }
}

impl Interner {
    /// Creates an empty interner.
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            arena: String::new(),
            spans: Vec::new(),
            kinds: Vec::new(),
            semantic_atoms: Vec::new(),
            symbols: Vec::new(),
            symbol_slots: AHashMap::new(),
            index: AHashMap::new(),
            index_dirty: false,
            identities: IdentityAllocator::new(0),
        }
    }

    /// Installs an already structurally validated frozen dense prefix without
    /// replaying ordinary interning. Process-wide compact symbols are resolved
    /// in one batch while local slots and lookup indexes are built directly.
    pub(crate) fn from_frozen(
        arena: String,
        spans: Vec<(u32, u32)>,
        kinds: Vec<ControlSequenceKind>,
        semantic_atoms: Vec<u64>,
    ) -> Result<Self, &'static str> {
        if spans.len() != kinds.len() || spans.len() != semantic_atoms.len() {
            return Err("frozen interner column length mismatch");
        }
        let count = u32::try_from(spans.len()).map_err(|_| "frozen interner capacity")?;
        let identities = IdentityAllocator::from_frozen_len(0, count);
        let mut symbols = Vec::with_capacity(spans.len());
        let mut symbol_slots = AHashMap::with_capacity(spans.len());
        let mut index: AHashMap<u64, Vec<SymbolId>> = AHashMap::with_capacity(spans.len());
        for slot in 0..spans.len() {
            let (start, len) = spans[slot];
            let start = start as usize;
            let end = start
                .checked_add(len as usize)
                .ok_or("frozen interner span overflow")?;
            let name = arena
                .get(start..end)
                .ok_or("frozen interner span is not UTF-8 aligned")?;
            let kind = kinds[slot];
            if semantic_atoms[slot] != semantic_atom(kind, name) {
                return Err("frozen interner semantic atom mismatch");
            }
            if kind == ControlSequenceKind::ActiveCharacter {
                let mut chars = name.chars();
                if chars.next().is_none() || chars.next().is_some() {
                    return Err("frozen active name is not one character");
                }
            }
            let stored = global_symbol(kind, name).map_err(|_| "frozen interner capacity")?;
            if symbol_slots.insert(stored, slot as u32).is_some() {
                return Err("duplicate frozen interner name");
            }
            let identity = identities
                .identity_at(slot as u32)
                .expect("validated frozen identity slot");
            let id = SymbolId::from_identity(identity, stored);
            index.entry(content_hash(kind, name)).or_default().push(id);
            symbols.push(stored);
        }
        Ok(Self {
            arena,
            spans,
            kinds,
            semantic_atoms,
            symbols,
            symbol_slots,
            index,
            index_dirty: false,
            identities,
        })
    }

    /// Interns `name`, returning its live capability and compact stored key.
    pub(crate) fn intern(&mut self, name: &str) -> Result<SymbolId, InternerError> {
        self.intern_key(ControlSequenceKind::Named, name)
    }

    /// Interns an active-character control sequence.
    pub(crate) fn intern_active(&mut self, ch: char) -> Result<SymbolId, InternerError> {
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
    ) -> Result<SymbolId, InternerError> {
        if self.index_dirty {
            self.rebuild_index();
        }

        let hash = content_hash(kind, name);
        if let Some(candidates) = self.index.get(&hash) {
            for &symbol in candidates {
                if self.kind_id(symbol) == kind && self.resolve_id(symbol) == name {
                    return Ok(symbol);
                }
            }
        }

        let start = u32_len(self.arena.len(), "interner arena exceeds u32 bytes");
        let len = u32_len(name.len(), "interned string exceeds u32 bytes");
        let stored = global_symbol(kind, name)?;
        let identity = self
            .identities
            .allocate()
            .map_err(|_| InternerError::TooManySymbols)?;
        let symbol = SymbolId::from_identity(identity, stored);
        debug_assert_eq!(identity.slot() as usize, self.spans.len());

        self.arena.push_str(name);
        self.spans.push((start, len));
        self.kinds.push(kind);
        self.semantic_atoms.push(semantic_atom(kind, name));
        self.symbols.push(stored);
        let old = self.symbol_slots.insert(stored, identity.slot());
        debug_assert!(old.is_none(), "symbol already mapped in local interner");
        self.index.entry(hash).or_default().push(symbol);

        Ok(symbol)
    }

    /// Returns the live symbol for `name` without mutating the interner.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<SymbolId> {
        self.get_key(ControlSequenceKind::Named, name)
    }

    /// Returns the live symbol for an active character without mutating.
    #[must_use]
    pub fn get_active(&self, ch: char) -> Option<SymbolId> {
        let mut encoded = [0; 4];
        self.get_key(
            ControlSequenceKind::ActiveCharacter,
            ch.encode_utf8(&mut encoded),
        )
    }

    fn get_key(&self, kind: ControlSequenceKind, name: &str) -> Option<SymbolId> {
        let hash = content_hash(kind, name);
        self.index.get(&hash).and_then(|candidates| {
            candidates.iter().copied().find(|&symbol| {
                self.contains_id(symbol)
                    && self.kind_id(symbol) == kind
                    && self.resolve_id(symbol) == name
            })
        })
    }

    /// Resolves a live symbol to its interned string.
    #[must_use]
    pub fn resolve_id(&self, symbol: SymbolId) -> &str {
        assert!(self.contains_id(symbol), "symbol is not live");
        let index = symbol.raw() as usize;
        assert!(index < self.spans.len(), "symbol is not live");
        let (start, len) = self.spans[index];
        let start = start as usize;
        let end = start + len as usize;
        assert!(end <= self.arena.len(), "symbol span exceeds arena");

        &self.arena[start..end]
    }

    /// Returns the TeX control-sequence namespace of a live symbol.
    #[must_use]
    pub fn kind_id(&self, symbol: SymbolId) -> ControlSequenceKind {
        assert!(self.contains_id(symbol), "symbol is not live");
        let index = symbol.raw() as usize;
        assert!(index < self.kinds.len(), "symbol is not live");
        self.kinds[index]
    }

    /// Returns whether `symbol` names a currently-live interner slot.
    #[must_use]
    pub fn contains_id(&self, symbol: SymbolId) -> bool {
        self.identities.contains(symbol.identity())
            && self.symbols.get(symbol.raw() as usize).copied() == Some(symbol.symbol())
    }

    /// Resolves a compact stored symbol key through the owning interner.
    #[must_use]
    pub fn resolve(&self, symbol: Symbol) -> &str {
        self.resolve_id(self.resolve_stored(symbol).expect("symbol is not live"))
    }

    #[must_use]
    pub fn kind(&self, symbol: Symbol) -> ControlSequenceKind {
        self.kind_id(self.resolve_stored(symbol).expect("symbol is not live"))
    }

    /// Returns the canonical semantic atom for a live compact symbol.
    pub(crate) fn semantic_atom(&self, symbol: Symbol) -> Option<u64> {
        let id = self.resolve_stored(symbol)?;
        self.semantic_atoms.get(id.raw() as usize).copied()
    }

    #[must_use]
    pub fn contains(&self, symbol: Symbol) -> bool {
        self.resolve_stored(symbol).is_some()
    }

    pub(crate) fn resolve_stored(&self, symbol: Symbol) -> Option<SymbolId> {
        let slot = *self.symbol_slots.get(&symbol)?;
        self.identities
            .identity_at(slot)
            .map(|identity| SymbolId::from_identity(identity, symbol))
    }

    /// Returns the compact nonreused key stored at a live dense interner slot.
    pub(crate) fn symbol_at_slot(&self, slot: u32) -> Option<Symbol> {
        self.symbols.get(slot as usize).copied()
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
        debug_assert_eq!(self.symbols.len(), self.spans.len());
        debug_assert_eq!(self.kinds.len(), self.spans.len());
        debug_assert_eq!(self.semantic_atoms.len(), self.spans.len());
        InternerMark {
            spans: u32_len(self.spans.len(), "interner spans exceed u32 entries"),
            bytes: u32_len(self.arena.len(), "interner arena exceeds u32 bytes"),
            identities: self.identities.watermark(),
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

        self.identities
            .rollback(mark.identities)
            .expect("interner mark is not an ancestor");
        self.spans.truncate(spans);
        self.kinds.truncate(spans);
        self.semantic_atoms.truncate(spans);
        for symbol in self.symbols.drain(spans..) {
            self.symbol_slots.remove(&symbol);
        }
        self.arena.truncate(bytes);
        debug_assert_eq!(self.kinds.len(), self.spans.len());
        debug_assert_eq!(self.semantic_atoms.len(), self.spans.len());
        debug_assert_eq!(self.symbols.len(), self.spans.len());
        self.index_dirty = true;
    }

    fn rebuild_index(&mut self) {
        self.index.clear();
        for raw in 0..self.spans.len() {
            let stored = self.symbols[raw];
            let identity = self
                .identities
                .identity_at(u32_len(raw, "interner spans exceed u32 entries"))
                .expect("live interner slot should have identity");
            let symbol = SymbolId::from_identity(identity, stored);
            let hash = content_hash(self.kind_id(symbol), self.resolve_id(symbol));
            self.index.entry(hash).or_default().push(symbol);
        }
        self.index_dirty = false;
    }
}

pub(crate) fn semantic_atom(kind: ControlSequenceKind, name: &str) -> u64 {
    let mut hasher = StateHasher::new(0x6373_5f61_746f_6d31);
    hasher.u8(match kind {
        ControlSequenceKind::Named => 0,
        ControlSequenceKind::ActiveCharacter => 1,
    });
    hasher.str(name);
    hasher.finish()
}

#[derive(Debug, Default)]
struct GlobalSymbols {
    names: AHashMap<u64, Vec<GlobalSymbolEntry>>,
    len: u32,
}

#[derive(Debug)]
struct GlobalSymbolEntry {
    kind: ControlSequenceKind,
    name: String,
    symbol: Symbol,
}

fn global_symbol(kind: ControlSequenceKind, name: &str) -> Result<Symbol, InternerError> {
    let symbols = GLOBAL_SYMBOLS.get_or_init(|| RwLock::new(GlobalSymbols::default()));
    let hash = content_hash(kind, name);
    if let Some(symbol) = symbols
        .read()
        .expect("global symbol registry lock poisoned")
        .names
        .get(&hash)
        .and_then(|entries| {
            entries
                .iter()
                .find(|entry| entry.kind == kind && entry.name == name)
                .map(|entry| entry.symbol)
        })
    {
        return Ok(symbol);
    }

    let mut symbols = symbols
        .write()
        .expect("global symbol registry lock poisoned");
    if let Some(symbol) = symbols.names.get(&hash).and_then(|entries| {
        entries
            .iter()
            .find(|entry| entry.kind == kind && entry.name == name)
            .map(|entry| entry.symbol)
    }) {
        return Ok(symbol);
    }
    let symbol = symbol_for_global_len(symbols.len)?;
    symbols.len += 1;
    symbols
        .names
        .entry(hash)
        .or_default()
        .push(GlobalSymbolEntry {
            kind,
            name: name.to_owned(),
            symbol,
        });
    Ok(symbol)
}

fn symbol_for_global_len(len: u32) -> Result<Symbol, InternerError> {
    (len < SYMBOL_CAPACITY)
        .then_some(Symbol(len))
        .ok_or(InternerError::TooManySymbols)
}

fn content_hash(kind: ControlSequenceKind, name: &str) -> u64 {
    let mut hasher = AHasher::default();
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
