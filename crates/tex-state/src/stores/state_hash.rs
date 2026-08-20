use super::{SnapshotOwner, StoreSnapshot, Stores};
use crate::ContentHash;
use crate::cell::{BankTag, CellId};
use crate::glue::GlueSpec;
use crate::ids::{FontId, GlueId, MacroDefinitionId, NodeListId, TokenListId};
use crate::interner::{ControlSequenceKind, Symbol, SymbolId};
use crate::journal::Entry;
use crate::meaning::{
    ExpandablePrimitive, InternalInteger, Meaning, RawMeaning, UnexpandablePrimitive,
};
use crate::node::{GlueKind, KernKind, Node, Sign};
use crate::node_arena::NodeListRef;
use crate::state_hash::{StateHashComponent, StateHashFragment, StateHasher};
use crate::token::{Catcode, Token};
use ahash::AHashMap;
use std::collections::VecDeque;

const STORE_SLICE_DOMAIN: u64 = 0x7374_6f72_6573_6c63;
const JOURNAL_SLICE_DOMAIN: u64 = 0x6a6f_7572_6e61_6c73;
const CODE_TABLES_DOMAIN: u64 = 0x636f_6465_7461_626c;
const HYPHENATION_DOMAIN: u64 = 0x6879_7068_656e_6174;
const PREPARED_MAG_DOMAIN: u64 = 0x7072_6570_5f6d_6167;
const FONT_SELECTION_DOMAIN: u64 = 0x666f_6e74_5f73_656c;
const FONT_STATE_DOMAIN: u64 = 0x666f_6e74_5f73_7461;
const CELL_VALUE_DOMAIN: u64 = 0x6365_6c6c_7661_6c75;
const CELL_ORDER_DOMAIN: u64 = 0x6365_6c6c_5f6f_7264;
const EXACT_CELL_KEY_DOMAIN: u64 = 0x6578_6163_745f_6b79;
const EXACT_CELL_VALUE_DOMAIN: u64 = 0x6578_6163_745f_766c;
const FONT_DIMEN_BITS: u32 = 17;
const FONT_DIMEN_MASK: u32 = (1 << FONT_DIMEN_BITS) - 1;

/// Derived semantic fingerprints at the latest checkpoint boundary.
///
/// This is an accelerator, not rollback state. [`Stores::rollback`] clears it
/// so the next slice reconstructs any needed baseline from journal `old`
/// words. Keeping it out of [`StoreSnapshot`] preserves O(1) snapshots.
#[derive(Debug)]
pub(super) struct SemanticHashCache {
    cells: AHashMap<CellId, CachedCellHash>,
    retired_first_old: AHashMap<CellId, u64>,
    pub(super) projections: StoreProjectionCache,
    first_old: Vec<(CellId, usize, u64)>,
    changed_cells: Vec<(u64, CellId)>,
}

/// Fixed-size derived roots retained by snapshots.
///
/// Unlike the journal scratch in [`SemanticHashCache`], these projections are
/// keyed by immutable semantic roots and are safe to restore with a snapshot.
/// A mutation changes the key and therefore turns only that component into a
/// cache miss.
#[derive(Clone, Debug, Default)]
pub(super) struct StoreProjectionCache {
    code_tables: [Option<CachedProjection<crate::code_tables::CodeTablesSemanticCursor>>; 6],
    hyphenation: Option<CachedProjection<HyphenationSemanticCursor>>,
    last_loaded_font: Option<CachedProjection<FontSelectionCursor>>,
    #[cfg(test)]
    hyphenation_hash_calls: usize,
}

impl Default for SemanticHashCache {
    fn default() -> Self {
        // Cell ids are trusted dense engine keys, and canonical output is
        // sorted independently of this map. Fixed AHash keys avoid asking the
        // OS for fresh randomness whenever state_hash_slice temporarily moves
        // this discardable cache out with mem::take.
        let cell_hasher = ahash::RandomState::with_seeds(
            0x6365_6c6c_5f68_6173,
            0x685f_6361_6368_655f,
            0x756d_6265_725f_7631,
            0x7374_6174_655f_6964,
        );
        Self {
            cells: AHashMap::with_hasher(cell_hasher),
            retired_first_old: AHashMap::with_hasher(ahash::RandomState::with_seeds(
                0x7265_7469_7265_645f,
                0x656e_765f_6261_7365,
                0x6c69_6e65_5f76_315f,
                0x756d_6265_725f_7631,
            )),
            projections: StoreProjectionCache::default(),
            first_old: Vec::new(),
            changed_cells: Vec::new(),
        }
    }
}

impl Clone for SemanticHashCache {
    fn clone(&self) -> Self {
        Self {
            cells: self.cells.clone(),
            retired_first_old: self.retired_first_old.clone(),
            projections: self.projections.clone(),
            first_old: Vec::new(),
            changed_cells: Vec::new(),
        }
    }
}

impl SemanticHashCache {
    pub(super) fn clear(&mut self) {
        self.cells.clear();
        self.retired_first_old.clear();
        self.projections = StoreProjectionCache::default();
        self.first_old.clear();
        self.changed_cells.clear();
    }

    #[cfg(test)]
    pub(super) fn testing_scratch_capacities(&self) -> (usize, usize) {
        (self.first_old.capacity(), self.changed_cells.capacity())
    }

    #[cfg(test)]
    pub(super) const fn testing_hyphenation_hash_calls(&self) -> usize {
        self.projections.hyphenation_hash_calls
    }
}

#[derive(Clone, Debug)]
struct CachedCellHash {
    key: SemanticCellKey,
    order: u64,
    value_hash: u64,
}

#[derive(Clone, Debug)]
struct CachedProjection<K> {
    key: K,
    fragment: StateHashFragment,
}

fn cached_projection<K: Clone + Eq>(
    cached: &mut Option<CachedProjection<K>>,
    key: &K,
    domain: u64,
    component: StateHashComponent,
    build: impl FnOnce(&mut StateHasher) -> usize,
) -> StateHashFragment {
    if let Some(cached) = cached
        && cached.key == *key
    {
        return cached.fragment;
    }
    let fragment = StateHashFragment::from_measured_builder_counted(domain, component, build);
    *cached = Some(CachedProjection {
        key: key.clone(),
        fragment,
    });
    fragment
}

fn cached_code_table_projection(
    cached: &mut Option<CachedProjection<crate::code_tables::CodeTablesSemanticCursor>>,
    key: &crate::code_tables::CodeTablesSemanticCursor,
    table: usize,
    build: impl FnOnce(&mut StateHasher) -> usize,
) -> StateHashFragment {
    if let Some(cached) = cached
        && cached.key.shares_table_root(key, table)
    {
        return cached.fragment;
    }
    let fragment = StateHashFragment::from_measured_builder_counted(
        CODE_TABLES_DOMAIN ^ table as u64,
        StateHashComponent::CodeTables,
        build,
    );
    *cached = Some(CachedProjection {
        key: key.clone(),
        fragment,
    });
    fragment
}

/// Cursor into store-owned state for semantic convergence hashing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StoreStateHashCursor {
    owner: SnapshotOwner,
    journal_pos: crate::journal::JournalPos,
    journal_baseline_serial: u64,
    code_tables: crate::code_tables::CodeTablesSemanticCursor,
    hyphenation_root: HyphenationSemanticCursor,
    prepared_mag: Option<i32>,
    last_loaded_font: FontSelectionCursor,
}

#[derive(Clone, Debug)]
struct HyphenationSemanticCursor(std::sync::Arc<crate::hyphenation::HyphenationTable>);

impl PartialEq for HyphenationSemanticCursor {
    fn eq(&self, other: &Self) -> bool {
        std::sync::Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for HyphenationSemanticCursor {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FontSelectionCursor {
    font: FontId,
    identifier: Option<SymbolId>,
}

impl Stores {
    /// Preserves the compact semantic delta before an unneeded rollback
    /// journal is retired. Old value hashes are computed while strong journal
    /// sidecars are still live; later checkpoint hashing visits only the
    /// distinct cells changed since its prior semantic boundary.
    pub(crate) fn preserve_retired_env_journal_hash_delta(&mut self, start: &StoreStateHashCursor) {
        self.assert_valid_hash_cursor(start);
        let mut cache = std::mem::take(&mut self.semantic_hash_cache);
        let mut first_old = std::mem::take(&mut cache.first_old);
        let mut first_old_box_hashes = AHashMap::new();
        debug_assert!(first_old.is_empty());
        for (position, entry) in self
            .env
            .journal_entries_since(start.journal_pos)
            .iter()
            .enumerate()
        {
            match entry {
                Entry::Undo(rec) => {
                    first_old.push((rec.cell().without_assignment_scope(), position, rec.old()))
                }
                Entry::BoxUndo(id) => {
                    let rec = self.env.box_undo(*id);
                    let cell = CellId::new(BankTag::Box, u32::from(rec.index()));
                    let old = rec.old();
                    first_old_box_hashes.entry(cell).or_insert_with(|| {
                        self.cell_value_hash_with_box_root(cell, old.value(), old.root().as_ref())
                    });
                    first_old.push((cell, position, old.value()));
                }
                Entry::Marker(_) => {}
            }
        }
        first_old.sort_unstable_by_key(|&(cell, position, _)| (cell, position));
        first_old.dedup_by_key(|entry| entry.0);
        for &(cell, _, old_word) in &first_old {
            if cache.retired_first_old.contains_key(&cell) {
                continue;
            }
            let baseline_hash = cache.cells.get(&cell).map_or_else(
                || {
                    first_old_box_hashes
                        .get(&cell)
                        .copied()
                        .unwrap_or_else(|| self.cell_value_hash(cell, old_word))
                },
                |cached| cached.value_hash,
            );
            cache.retired_first_old.insert(cell, baseline_hash);
        }
        for &(cell, _, _) in &first_old {
            self.synchronize_exact_env_cell(cell, self.env.semantic_word(cell));
        }
        first_old.clear();
        cache.first_old = first_old;
        self.semantic_hash_cache = cache;
    }

    /// Canonical identity of the rollback-coupled mutable store roots.
    ///
    /// Environment cells already carry a canonical commutative accumulator. The other
    /// components reuse the same root-keyed canonical projections as the
    /// rolling checkpoint hash, so an exact comparison visits only roots that
    /// have changed since their last projection.
    pub(crate) fn exact_mutable_identity(&mut self) -> u64 {
        self.synchronize_exact_env_identity();
        let cursor = self.state_hash_cursor();
        let mut cache = std::mem::take(&mut self.semantic_hash_cache);
        let code_tables: [StateHashFragment; 6] = core::array::from_fn(|table| {
            cached_code_table_projection(
                &mut cache.projections.code_tables[table],
                &cursor.code_tables,
                table,
                |projection| self.hash_code_table(table, projection),
            )
        });
        let hyphenation = cached_projection(
            &mut cache.projections.hyphenation,
            &cursor.hyphenation_root,
            HYPHENATION_DOMAIN,
            StateHashComponent::Hyphenation,
            |projection| self.hyphenation.hash_semantic(projection),
        );
        let prepared_mag =
            StateHashFragment::from_exact_builder(PREPARED_MAG_DOMAIN, |projection| {
                hash_prepared_mag(self.prepared_mag, projection);
            });
        let last_loaded_font = cached_projection(
            &mut cache.projections.last_loaded_font,
            &cursor.last_loaded_font,
            FONT_SELECTION_DOMAIN,
            StateHashComponent::FontSelection,
            |projection| {
                self.hash_font(self.last_loaded_font, projection);
                1
            },
        );
        self.semantic_hash_cache = cache;

        let mut framed = Vec::with_capacity(32 + 8 * 9);
        framed.extend_from_slice(b"umber-exact-mutable-store-v3");
        framed.extend_from_slice(&self.exact_env_identity().to_le_bytes());
        for fragment in code_tables {
            framed.extend_from_slice(&fragment.exact_identity().to_le_bytes());
        }
        framed.extend_from_slice(&hyphenation.exact_identity().to_le_bytes());
        framed.extend_from_slice(&prepared_mag.exact_identity().to_le_bytes());
        framed.extend_from_slice(&last_loaded_font.exact_identity().to_le_bytes());
        crate::state_hash::exact_identity_bytes(b"umber-exact-mutable-store-v4", &framed)
    }

    #[must_use]
    pub(crate) fn state_hash_cursor(&self) -> StoreStateHashCursor {
        StoreStateHashCursor {
            owner: self.owner.snapshot_owner(),
            journal_pos: self.env.current_journal_pos(),
            journal_baseline_serial: self.env.journal_baseline_serial(),
            code_tables: self.code_tables.semantic_cursor(),
            hyphenation_root: HyphenationSemanticCursor(std::sync::Arc::clone(&self.hyphenation)),
            prepared_mag: self.prepared_mag,
            last_loaded_font: self.font_selection_cursor(self.last_loaded_font),
        }
    }

    #[must_use]
    pub(crate) fn state_hash_cursor_from_snapshot(
        &self,
        snapshot: &StoreSnapshot,
    ) -> StoreStateHashCursor {
        StoreStateHashCursor {
            owner: snapshot.owner,
            journal_pos: snapshot.env_snapshot.journal_pos(),
            journal_baseline_serial: snapshot.env_snapshot.journal_baseline_serial(),
            code_tables: crate::code_tables::CodeTables::semantic_cursor_from_snapshot(
                &snapshot.code_tables_snapshot,
            ),
            hyphenation_root: HyphenationSemanticCursor(std::sync::Arc::clone(
                &snapshot.hyphenation,
            )),
            prepared_mag: snapshot.prepared_mag,
            last_loaded_font: self.font_selection_cursor(snapshot.last_loaded_font),
        }
    }

    #[must_use]
    pub(crate) fn retarget_state_hash_cursor(
        &self,
        cursor: &StoreStateHashCursor,
    ) -> StoreStateHashCursor {
        assert!(
            cursor.journal_pos <= self.env.current_journal_pos(),
            "Stores state-hash cursor journal position is past the current journal"
        );
        StoreStateHashCursor {
            owner: self.owner.snapshot_owner(),
            journal_pos: cursor.journal_pos,
            journal_baseline_serial: cursor.journal_baseline_serial,
            code_tables: cursor.code_tables.clone(),
            hyphenation_root: cursor.hyphenation_root.clone(),
            prepared_mag: cursor.prepared_mag,
            last_loaded_font: cursor.last_loaded_font,
        }
    }

    #[must_use]
    pub(crate) fn retarget_state_hash_cursor_after_node_release(
        &self,
        cursor: &StoreStateHashCursor,
    ) -> StoreStateHashCursor {
        self.assert_valid_hash_cursor(cursor);
        StoreStateHashCursor {
            owner: self.owner.snapshot_owner(),
            journal_pos: cursor.journal_pos,
            journal_baseline_serial: cursor.journal_baseline_serial,
            code_tables: cursor.code_tables.clone(),
            hyphenation_root: cursor.hyphenation_root.clone(),
            prepared_mag: cursor.prepared_mag,
            last_loaded_font: cursor.last_loaded_font,
        }
    }

    #[must_use]
    pub(crate) fn retarget_state_hash_cursor_after_journal_compaction(
        &self,
        cursor: &StoreStateHashCursor,
    ) -> StoreStateHashCursor {
        assert_eq!(
            cursor.owner,
            self.owner.snapshot_owner(),
            "Stores state-hash cursor belongs to a different Stores instance"
        );
        let current_journal_pos = self.env.current_journal_pos();
        StoreStateHashCursor {
            owner: self.owner.snapshot_owner(),
            journal_pos: cursor.journal_pos.min(current_journal_pos),
            journal_baseline_serial: cursor.journal_baseline_serial,
            code_tables: cursor.code_tables.clone(),
            hyphenation_root: cursor.hyphenation_root.clone(),
            prepared_mag: cursor.prepared_mag,
            last_loaded_font: cursor.last_loaded_font,
        }
    }

    #[must_use]
    pub(crate) fn state_hash_slice(
        &mut self,
        start: &StoreStateHashCursor,
        end: &mut StoreSnapshot,
    ) -> u64 {
        self.assert_valid_hash_cursor(start);
        self.assert_valid_snapshot(end);
        assert!(
            start.journal_pos <= end.env_snapshot.journal_pos(),
            "state hash cursor journal position is after snapshot"
        );

        #[cfg(feature = "profiling")]
        crate::measurement::record_hash_call(
            end.env_snapshot
                .journal_pos()
                .raw()
                .saturating_sub(start.journal_pos.raw()) as usize,
        );

        let mut cache = std::mem::take(&mut self.semantic_hash_cache);
        let journal_entries = end
            .env_snapshot
            .journal_pos()
            .raw()
            .saturating_sub(start.journal_pos.raw()) as usize;
        let journal = StateHashFragment::from_measured_builder(
            JOURNAL_SLICE_DOMAIN,
            StateHashComponent::Journal,
            journal_entries,
            |projection| self.hash_journal_changed_cells(start, end, &mut cache, projection),
        );
        let end_cursor = self.state_hash_cursor_from_snapshot(end);
        let code_tables: [StateHashFragment; 6] = core::array::from_fn(|table| {
            cached_code_table_projection(
                &mut cache.projections.code_tables[table],
                &end_cursor.code_tables,
                table,
                |projection| self.hash_code_table(table, projection),
            )
        });
        #[cfg(test)]
        let rehash_hyphenation = cache
            .projections
            .hyphenation
            .as_ref()
            .is_none_or(|cached| cached.key != end_cursor.hyphenation_root);
        let hyphenation = cached_projection(
            &mut cache.projections.hyphenation,
            &end_cursor.hyphenation_root,
            HYPHENATION_DOMAIN,
            StateHashComponent::Hyphenation,
            |projection| self.hyphenation.hash_semantic(projection),
        );
        #[cfg(test)]
        if rehash_hyphenation {
            cache.projections.hyphenation_hash_calls += 1;
        }
        let prepared_mag = StateHashFragment::from_measured_builder(
            PREPARED_MAG_DOMAIN,
            StateHashComponent::PreparedMag,
            1,
            |projection| hash_prepared_mag(self.prepared_mag, projection),
        );
        let last_loaded_font = cached_projection(
            &mut cache.projections.last_loaded_font,
            &end_cursor.last_loaded_font,
            FONT_SELECTION_DOMAIN,
            StateHashComponent::FontSelection,
            |projection| {
                self.hash_font(self.last_loaded_font, projection);
                1
            },
        );
        end.exact_projection_cache = cache.projections.clone();
        self.semantic_hash_cache = cache;
        let mut hasher = StateHasher::new_exact(STORE_SLICE_DOMAIN);
        journal.apply(&mut hasher);
        for fragment in code_tables {
            fragment.apply(&mut hasher);
        }
        hyphenation.apply(&mut hasher);
        prepared_mag.apply(&mut hasher);
        last_loaded_font.apply(&mut hasher);
        end.exact_env_identity = self.exact_env_identity.snapshot();
        hasher.finish()
    }

    pub(crate) fn hash_token_list_semantic(&self, id: TokenListId, hasher: &mut StateHasher) {
        hasher.tag(0x50);
        self.tokens(id).semantic_id().apply(hasher);
    }

    pub(crate) fn hash_node_slice_semantic(
        &self,
        nodes: &[Node],
        hasher: &mut StateHasher,
    ) -> usize {
        self.hash_node_iter_semantic(nodes.len(), nodes.iter(), hasher)
    }

    pub(crate) fn hash_node_deque_semantic(
        &self,
        nodes: &VecDeque<Node>,
        hasher: &mut StateHasher,
    ) -> usize {
        self.hash_node_iter_semantic(nodes.len(), nodes.iter(), hasher)
    }

    fn hash_node_iter_semantic<'a>(
        &self,
        len: usize,
        nodes: impl Iterator<Item = &'a Node>,
        hasher: &mut StateHasher,
    ) -> usize {
        hasher.tag(0x72);
        hasher.usize(len);
        for node in nodes {
            self.hash_owned_node_semantic_identity(node, hasher);
        }
        len
    }

    pub(crate) fn hash_glue_semantic(&self, id: GlueId, hasher: &mut StateHasher) {
        self.hash_glue(id, hasher);
    }

    pub(crate) fn hash_font_semantic(&self, id: FontId, hasher: &mut StateHasher) {
        hasher.tag(0x68);
        let id = self.resolve_stored_font(id);
        self.fonts.hash_fragment(id).apply(hasher);
    }

    pub(crate) fn hash_meaning_semantic(&self, meaning: Meaning, hasher: &mut StateHasher) {
        self.hash_meaning(meaning, hasher);
    }

    #[cfg(test)]
    pub(crate) fn testing_font_semantic_fingerprint(&self, id: FontId) -> u64 {
        self.font_state_fragment(id).fingerprint()
    }

    fn assert_valid_hash_cursor(&self, cursor: &StoreStateHashCursor) {
        assert_eq!(
            cursor.owner,
            self.owner.snapshot_owner(),
            "Stores state-hash cursor belongs to a different Stores instance"
        );
        assert!(
            cursor.journal_pos <= self.env.current_journal_pos(),
            "Stores state-hash cursor journal position is past the current journal"
        );
    }

    fn hash_journal_changed_cells(
        &mut self,
        start: &StoreStateHashCursor,
        end: &StoreSnapshot,
        cache: &mut SemanticHashCache,
        hasher: &mut StateHasher,
    ) {
        let start_index = start.journal_pos.raw() as usize;
        let end_index = end.env_snapshot.journal_pos().raw() as usize;
        let mut first_old = std::mem::take(&mut cache.first_old);
        let mut changed_cells = std::mem::take(&mut cache.changed_cells);
        let mut first_old_box_hashes = AHashMap::new();
        debug_assert!(first_old.is_empty());
        debug_assert!(changed_cells.is_empty());
        for (position, entry) in self.env.journal_entries_since(start.journal_pos)
            [..end_index.saturating_sub(start_index)]
            .iter()
            .enumerate()
        {
            match entry {
                Entry::Undo(rec) => {
                    let cell = rec.cell().without_assignment_scope();
                    first_old.push((cell, position, rec.old()));
                }
                Entry::BoxUndo(id) => {
                    let rec = self.env.box_undo(*id);
                    let cell = CellId::new(crate::cell::BankTag::Box, u32::from(rec.index()));
                    let old = rec.old();
                    first_old_box_hashes.entry(cell).or_insert_with(|| {
                        self.cell_value_hash_with_box_root(cell, old.value(), old.root().as_ref())
                    });
                    first_old.push((cell, position, old.value()));
                }
                Entry::Marker(_) => {}
            }
        }
        first_old.sort_unstable_by_key(|&(cell, position, _)| (cell, position));
        first_old.dedup_by_key(|entry| entry.0);

        for &(cell, _, old_word) in &first_old {
            if cache.retired_first_old.contains_key(&cell) {
                continue;
            }
            let baseline_hash = cache.cells.get(&cell).map_or_else(
                || {
                    first_old_box_hashes
                        .get(&cell)
                        .copied()
                        .unwrap_or_else(|| self.cell_value_hash(cell, old_word))
                },
                |cached| cached.value_hash,
            );
            cache.retired_first_old.insert(cell, baseline_hash);
        }

        let mut retired_first_old = std::mem::take(&mut cache.retired_first_old);
        for (&cell, &baseline_hash) in &retired_first_old {
            let new_word = self.env.semantic_word(cell);
            self.synchronize_exact_env_cell(cell, new_word);
            let current_hash = self.cell_value_hash(cell, new_word);

            let order = match cache.cells.get_mut(&cell) {
                Some(cached) => {
                    cached.value_hash = current_hash;
                    cached.order
                }
                None => {
                    let key = self.semantic_cell_key(cell);
                    let order = self.cell_order(&key);
                    cache.cells.insert(
                        cell,
                        CachedCellHash {
                            order,
                            key,
                            value_hash: current_hash,
                        },
                    );
                    order
                }
            };
            if baseline_hash != current_hash {
                changed_cells.push((order, cell));
            }
        }

        changed_cells.sort_unstable_by(|(left_order, left), (right_order, right)| {
            left_order.cmp(right_order).then_with(|| {
                cache.cells[left]
                    .key
                    .cmp(&cache.cells[right].key)
                    .then_with(|| left.cmp(right))
            })
        });
        changed_cells
            .dedup_by(|(_, right), (_, left)| cache.cells[left].key == cache.cells[right].key);

        #[cfg(feature = "profiling")]
        crate::measurement::record_hash_changed_cells(
            changed_cells.len(),
            first_old.capacity() * core::mem::size_of::<(CellId, usize, u64)>()
                + changed_cells.capacity() * core::mem::size_of::<(u64, CellId)>()
                + retired_first_old.capacity() * core::mem::size_of::<(CellId, u64)>(),
        );

        hasher.tag(0x10);
        hasher.usize(changed_cells.len());
        for &(_, cell) in &changed_cells {
            let cached = &cache.cells[&cell];
            self.hash_cell_key(&cached.key, hasher);
            hasher.u64(cached.value_hash);
        }

        first_old.clear();
        changed_cells.clear();
        retired_first_old.clear();
        cache.first_old = first_old;
        cache.changed_cells = changed_cells;
        cache.retired_first_old = retired_first_old;
    }

    fn semantic_cell_key(&self, cell: CellId) -> SemanticCellKey {
        match cell.bank() {
            BankTag::Meaning => {
                let symbol = self
                    .interner
                    .symbol_at_slot(cell.index())
                    .expect("meaning slot should name a live symbol");
                SemanticCellKey::Meaning {
                    kind: self.interner.kind(symbol),
                    name: self.interner.resolve(symbol).to_owned(),
                }
            }
            BankTag::FontDimen => {
                let (font, slot) = unpack_font_dimen_index(cell.index());
                SemanticCellKey::FontBank {
                    bank: bank_order(cell.bank()),
                    font: self.font_semantic_key(self.resolve_stored_font(font)),
                    index: slot,
                }
            }
            BankTag::FontParamLen | BankTag::FontHyphenChar | BankTag::FontSkewChar => {
                SemanticCellKey::FontBank {
                    bank: bank_order(cell.bank()),
                    font: self
                        .font_semantic_key(self.resolve_stored_font(FontId::new(cell.index()))),
                    index: 0,
                }
            }
            bank @ (BankTag::PdfLpCode
            | BankTag::PdfRpCode
            | BankTag::PdfEfCode
            | BankTag::PdfTagCode
            | BankTag::PdfKnbsCode
            | BankTag::PdfStbsCode
            | BankTag::PdfShbsCode
            | BankTag::PdfKnbcCode
            | BankTag::PdfKnacCode) => SemanticCellKey::FontBank {
                bank: bank_order(bank),
                font: self
                    .font_semantic_key(self.resolve_stored_font(FontId::new(cell.index() >> 8))),
                index: cell.index() & 0xff,
            },
            BankTag::PdfNoLigatures => SemanticCellKey::FontBank {
                bank: bank_order(cell.bank()),
                font: self.font_semantic_key(self.resolve_stored_font(FontId::new(cell.index()))),
                index: 0,
            },
            bank => SemanticCellKey::Bank {
                bank: bank_order(bank),
                index: cell.index(),
            },
        }
    }

    fn cell_order(&self, key: &SemanticCellKey) -> u64 {
        let mut hasher = StateHasher::new(CELL_ORDER_DOMAIN);
        self.hash_cell_key(key, &mut hasher);
        hasher.finish()
    }

    fn hash_cell_key(&self, key: &SemanticCellKey, hasher: &mut StateHasher) {
        match key {
            SemanticCellKey::Meaning { kind, name } => {
                hasher.tag(0x01);
                hash_control_sequence_kind(*kind, hasher);
                hasher.str(name);
            }
            SemanticCellKey::Bank { bank, index } => {
                hasher.tag(0x02);
                hasher.u8(*bank);
                hasher.u32(*index);
            }
            SemanticCellKey::FontBank { bank, font, index } => {
                hasher.tag(0x03);
                hasher.u8(*bank);
                hash_font_semantic_key(font, hasher);
                hasher.u32(*index);
            }
        }
    }

    fn cell_value_hash(&self, cell: CellId, word: u64) -> u64 {
        let box_root = if cell.bank() == BankTag::Box {
            self.env.box_reg_ref(cell.index() as u16)
        } else {
            None
        };
        self.cell_value_hash_with_box_root(cell, word, box_root.as_ref())
    }

    fn cell_value_hash_with_box_root(
        &self,
        cell: CellId,
        word: u64,
        box_root: Option<&NodeListRef>,
    ) -> u64 {
        let mut hasher = StateHasher::new(CELL_VALUE_DOMAIN);
        self.hash_cell_value(cell, word, box_root, &mut hasher);
        hasher.finish()
    }

    fn exact_cell_key(&self, key: &SemanticCellKey) -> u64 {
        exact_identity_from_hashers(EXACT_CELL_KEY_DOMAIN, |hasher| {
            self.hash_cell_key(key, hasher);
        })
    }

    fn exact_cell_value(&self, cell: CellId, word: u64) -> u64 {
        let box_root = if cell.bank() == BankTag::Box {
            self.env.box_reg_ref(cell.index() as u16)
        } else {
            None
        };
        exact_identity_from_hashers(EXACT_CELL_VALUE_DOMAIN, |hasher| {
            self.hash_cell_value(cell, word, box_root.as_ref(), hasher);
        })
    }

    pub(crate) fn update_exact_env_cell(&mut self, cell: CellId, word: u64) {
        debug_assert_eq!(word, self.env.semantic_word(cell));
        let value = self
            .env
            .semantic_non_default_word(cell)
            .map(|word| self.exact_cell_value(cell, word));
        if value.is_none() {
            self.exact_env_identity.update(cell, 0, None);
            return;
        }
        let semantic_key = self.semantic_cell_key(cell);
        let key = self.exact_cell_key(&semantic_key);
        self.exact_env_identity.update(cell, key, value);
    }

    fn synchronize_exact_env_cell(&mut self, cell: CellId, word: u64) {
        debug_assert_eq!(word, self.env.semantic_word(cell));
        let value = self
            .env
            .semantic_non_default_word(cell)
            .map(|word| self.exact_cell_value(cell, word));
        if value.is_none() {
            if !self.exact_env_identity.contains(cell, 0, None) {
                self.exact_env_identity.update(cell, 0, None);
            }
            return;
        }
        let semantic_key = self.semantic_cell_key(cell);
        let key = self.exact_cell_key(&semantic_key);
        if !self.exact_env_identity.contains(cell, key, value) {
            self.exact_env_identity.update(cell, key, value);
        }
    }

    pub(crate) fn synchronize_exact_env_identity(&mut self) {
        let (journal_pos, baseline_serial) = self.exact_env_identity.journal_cursor();
        assert_eq!(
            baseline_serial,
            self.env.journal_baseline_serial(),
            "exact environment cursor belongs to a retired journal baseline"
        );
        // Scalar assignment episodes ordinarily advance only a handful of
        // journal cells. Keep that exact sort/dedup projection inline and
        // spill only for an unusually broad delta.
        let mut cells = self
            .env
            .journal_entries_since(journal_pos)
            .iter()
            .filter_map(|entry| match entry {
                Entry::Undo(rec) => Some(rec.cell().without_assignment_scope()),
                Entry::BoxUndo(id) => Some(CellId::new(
                    BankTag::Box,
                    u32::from(self.env.box_undo(*id).index()),
                )),
                Entry::Marker(_) => None,
            })
            .collect::<smallvec::SmallVec<[CellId; 8]>>();
        cells.sort_unstable();
        cells.dedup();
        for cell in cells {
            self.synchronize_exact_env_cell(cell, self.env.semantic_word(cell));
        }
        self.mark_exact_env_journal_current();
    }

    pub(super) fn mark_exact_env_journal_current(&mut self) {
        self.exact_env_identity.mark_journal(
            self.env.current_journal_pos(),
            self.env.journal_baseline_serial(),
        );
    }

    pub(crate) fn initialize_exact_env_identity(&mut self) {
        let recomputed = self.recomputed_exact_env_identity();
        self.exact_env_identity.reconcile(&recomputed);
        self.mark_exact_env_journal_current();
    }

    pub(crate) fn discard_exact_env_undo_history(&mut self) {
        self.exact_env_identity.discard_undo_history();
    }

    fn recomputed_exact_env_identity(&self) -> super::exact_identity::ExactEnvIdentity {
        let mut cells = Vec::new();
        self.env
            .for_each_semantic_non_default_word(|cell, word| cells.push((cell, word)));
        let mut identity = super::exact_identity::ExactEnvIdentity::default();
        for (cell, word) in cells {
            let semantic_key = self.semantic_cell_key(cell);
            identity.update(
                cell,
                self.exact_cell_key(&semantic_key),
                Some(self.exact_cell_value(cell, word)),
            );
        }
        identity.discard_undo_history();
        identity
    }

    pub(crate) fn exact_env_identity(&self) -> u64 {
        self.exact_env_identity.identity()
    }

    #[cfg(test)]
    pub(crate) const fn testing_exact_env_updates(&self) -> usize {
        self.exact_env_identity.testing_updates()
    }

    #[cfg(test)]
    pub(crate) const fn testing_exact_env_undo_entries(&self) -> usize {
        self.exact_env_identity.testing_undo_len()
    }

    #[cfg(test)]
    pub(crate) fn testing_recomputed_exact_env_identity(&self) -> u64 {
        self.recomputed_exact_env_identity().identity()
    }

    fn hash_cell_value(
        &self,
        cell: CellId,
        word: u64,
        box_root: Option<&NodeListRef>,
        hasher: &mut StateHasher,
    ) {
        match cell.bank() {
            BankTag::Meaning => self.hash_meaning(
                self.resolve_stored_meaning(Meaning::decode_stored(word)),
                hasher,
            ),
            BankTag::Count | BankTag::IntParam => hasher.i32(word as u32 as i32),
            BankTag::Dimen | BankTag::DimenParam => hasher.i32(word as u32 as i32),
            BankTag::Skip | BankTag::Muskip | BankTag::GlueParam => {
                self.hash_glue(
                    self.resolve_stored_glue(GlueId::new(decode_u32(word))),
                    hasher,
                );
            }
            BankTag::Toks => {
                self.hash_token_list_semantic(
                    self.resolve_stored_token_list(TokenListId::new(decode_u32(word))),
                    hasher,
                );
            }
            BankTag::TokParam => {
                if word == 0 {
                    hasher.tag(0);
                } else {
                    hasher.tag(1);
                    self.hash_token_list_semantic(
                        self.resolve_stored_token_list(TokenListId::new(decode_u32(word - 1))),
                        hasher,
                    );
                }
            }
            BankTag::Box => match NodeListId::decode_box_word(word) {
                Some(id) => {
                    let root = box_root.expect("nonvoid box word must carry a structural owner");
                    assert_eq!(id, root.id(), "box word and structural owner disagree");
                    hasher.tag(0x70);
                    root.semantic_id().apply(hasher);
                }
                None => {
                    assert!(
                        box_root.is_none(),
                        "void box word carried a structural owner"
                    );
                    hasher.tag(0);
                }
            },
            BankTag::FontDimen => hasher.i32(word as u32 as i32),
            BankTag::FontParamLen => hasher.u32(decode_u32(word)),
            BankTag::FontHyphenChar
            | BankTag::FontSkewChar
            | BankTag::PdfLpCode
            | BankTag::PdfRpCode
            | BankTag::PdfEfCode
            | BankTag::PdfTagCode
            | BankTag::PdfKnbsCode
            | BankTag::PdfStbsCode
            | BankTag::PdfShbsCode
            | BankTag::PdfKnbcCode
            | BankTag::PdfKnacCode => hasher.i32(word as u32 as i32),
            BankTag::PdfNoLigatures => hasher.bool(word != 0),
            BankTag::CurrentFont => self.hash_current_font_word(word, hasher),
            BankTag::MathFamilyFont => self.hash_font(
                self.resolve_stored_font(FontId::new(decode_u32(word))),
                hasher,
            ),
        }
    }

    fn hash_meaning(&self, meaning: Meaning, hasher: &mut StateHasher) {
        match meaning {
            Meaning::Undefined => hasher.tag(0),
            Meaning::Relax => hasher.tag(1),
            Meaning::Macro { flags, definition } => {
                hasher.tag(2);
                hasher.u8(flags.bits());
                self.hash_macro_definition(definition, hasher);
            }
            Meaning::CharGiven(ch) => {
                hasher.tag(3);
                hasher.u32(ch as u32);
            }
            Meaning::CharToken { ch, cat } => {
                hasher.tag(21);
                hasher.u32(ch as u32);
                hash_catcode(cat, hasher);
            }
            Meaning::MathCharGiven(value) => {
                hasher.tag(4);
                hasher.u16(value);
            }
            Meaning::CountRegister(index) => hash_register_alias(5, index, hasher),
            Meaning::DimenRegister(index) => hash_register_alias(6, index, hasher),
            Meaning::SkipRegister(index) => hash_register_alias(7, index, hasher),
            Meaning::MuskipRegister(index) => hash_register_alias(8, index, hasher),
            Meaning::ToksRegister(index) => hash_register_alias(9, index, hasher),
            Meaning::IntParam(index) => hash_register_alias(10, index, hasher),
            Meaning::DimenParam(index) => hash_register_alias(11, index, hasher),
            Meaning::GlueParam(index) => hash_register_alias(12, index, hasher),
            Meaning::TokParam(index) => hash_register_alias(13, index, hasher),
            Meaning::MuGlueParam(index) => hash_register_alias(20, index, hasher),
            Meaning::PageDimension(dimension) => {
                hasher.tag(18);
                hasher.u8(dimension.index());
            }
            Meaning::PageInteger(integer) => {
                hasher.tag(19);
                hasher.u8(integer.index());
            }
            Meaning::InternalInteger(integer) => {
                hasher.tag(22);
                hash_internal_integer(integer, hasher);
            }
            Meaning::Font(id) => {
                hasher.tag(17);
                self.hash_font(id, hasher);
            }
            Meaning::ExpandablePrimitive(primitive) => hash_expandable_primitive(primitive, hasher),
            Meaning::EndV => hasher.tag(23),
            Meaning::UnexpandablePrimitive(primitive) => {
                hash_unexpandable_primitive(primitive, hasher);
            }
            Meaning::Unknown(raw) => hash_unknown_meaning(raw, hasher),
        }
    }

    fn hash_macro_definition(&self, id: MacroDefinitionId, hasher: &mut StateHasher) {
        let definition = self.macro_definition(id).meaning();
        hasher.u8(definition.flags().bits());
        self.hash_portable_token_list(definition.parameter_text(), hasher);
        self.hash_portable_token_list(definition.replacement_text(), hasher);
    }

    fn hash_portable_token_list(&self, id: TokenListId, hasher: &mut StateHasher) {
        let tokens = self.tokens(id);
        hasher.tag(0x50);
        hasher.usize(tokens.len());
        for &token in tokens.iter() {
            match token {
                Token::Char { ch, cat } => {
                    hasher.tag(0);
                    hasher.u32(ch as u32);
                    hasher.u8(cat as u8);
                }
                Token::Cs(symbol) => {
                    let symbol = self.resolve_stored_symbol(symbol);
                    hasher.tag(1);
                    hasher.u8(match self.control_sequence_kind(symbol) {
                        ControlSequenceKind::ActiveCharacter => 1,
                        ControlSequenceKind::Null
                        | ControlSequenceKind::SingleCharacter
                        | ControlSequenceKind::Named => 0,
                        ControlSequenceKind::Internal => 2,
                    });
                    hasher.str(self.resolve(symbol));
                }
                Token::Param(slot) => {
                    hasher.tag(2);
                    hasher.u8(slot);
                }
                Token::Frozen(frozen) => {
                    hasher.tag(3);
                    hasher.u16(frozen.raw());
                }
            }
        }
    }

    fn hash_glue(&self, id: GlueId, hasher: &mut StateHasher) {
        let GlueSpec {
            width,
            stretch,
            stretch_order,
            shrink,
            shrink_order,
        } = self
            .runtime_values
            .glue(self.resolve_stored_glue(id))
            .expect("stored glue slot is not live")
            .spec();
        hasher.tag(0x60);
        hasher.i32(width.raw());
        hasher.i32(stretch.raw());
        hasher.u8(*stretch_order as u8);
        hasher.i32(shrink.raw());
        hasher.u8(*shrink_order as u8);
    }

    fn hash_font(&self, font: FontId, hasher: &mut StateHasher) {
        hasher.tag(0x68);
        self.font_state_fragment(font).apply(hasher);
    }

    fn font_state_fragment(&self, font: FontId) -> StateHashFragment {
        let font = self
            .fonts
            .resolve_stored(font)
            .expect("stored font slot is not live");
        StateHashFragment::from_exact_builder(FONT_STATE_DOMAIN, |fragment| {
            self.fonts.complete_hash_fragment(font).apply(fragment);
            match self.fonts.expansion(font) {
                Some(expansion) => {
                    fragment.bool(true);
                    fragment.i32(i32::from(expansion.stretch));
                    fragment.i32(i32::from(expansion.shrink));
                    fragment.i32(i32::from(expansion.step));
                    fragment.bool(expansion.auto_expand);
                }
                None => fragment.bool(false),
            }
        })
    }

    fn font_semantic_key(&self, font: FontId) -> FontSemanticKey {
        #[cfg(feature = "profiling")]
        crate::measurement::record_owned_font_key();
        self.assert_live_font(font);
        let identifier = self.fonts.identifier(font).map(|symbol| {
            self.assert_live_symbol(symbol);
            (
                self.interner.kind_id(symbol),
                self.interner.resolve_id(symbol).to_owned(),
            )
        });
        let complete_hash = self.font_state_fragment(font).identity();
        let font = self.fonts.get(font);
        FontSemanticKey {
            name: font.name().to_owned(),
            content_hash: font.content_hash(),
            checksum: font.checksum(),
            design_size: font.design_size().raw(),
            size: font.size().raw(),
            complete_hash,
            identifier,
        }
    }

    fn font_selection_cursor(&self, font: FontId) -> FontSelectionCursor {
        self.assert_live_font(font);
        let identifier = self.fonts.identifier(font);
        if let Some(symbol) = identifier {
            self.assert_live_symbol(symbol);
        }
        FontSelectionCursor { font, identifier }
    }

    fn hash_current_font_word(&self, word: u64, hasher: &mut StateHasher) {
        hasher.tag(0x69);
        let font = self.resolve_stored_font(FontId::new(word as u32));
        self.hash_font(font, hasher);
        let symbol = word >> 32;
        if symbol == 0 {
            hasher.bool(false);
        } else {
            let symbol = self.resolve_stored_symbol(Symbol::new((symbol - 1) as u32));
            hasher.bool(true);
            hash_control_sequence_kind(self.interner.kind_id(symbol), hasher);
            hasher.str(self.interner.resolve_id(symbol));
        }
    }

    fn hash_code_table(&self, table: usize, hasher: &mut StateHasher) -> usize {
        hasher.tag(0x20 + table as u8);
        let mut visits = 0;
        macro_rules! hash_values {
            ($method:ident, $hash:ident) => {{
                self.code_tables.$method(|ch, value| {
                    visits += 1;
                    hasher.u32(ch as u32);
                    hasher.$hash(value);
                });
            }};
        }
        match table {
            0 => self.code_tables.for_each_non_default_catcode(|ch, value| {
                visits += 1;
                hasher.u32(ch as u32);
                hasher.u8(value as u8);
            }),
            1 => hash_values!(for_each_non_default_lccode, u32),
            2 => hash_values!(for_each_non_default_uccode, u32),
            3 => hash_values!(for_each_non_default_sfcode, u16),
            4 => hash_values!(for_each_non_default_mathcode, u32),
            5 => hash_values!(for_each_non_default_delcode, i32),
            _ => panic!("code-table index out of range"),
        }
        visits
    }

    pub(crate) fn hash_dependency_code_table(
        &self,
        table: crate::DependencyCodeTable,
        hasher: &mut StateHasher,
    ) {
        let index = match table {
            crate::DependencyCodeTable::Catcode => 0,
            crate::DependencyCodeTable::Lccode => 1,
            crate::DependencyCodeTable::Uccode => 2,
            crate::DependencyCodeTable::Sfcode => 3,
            crate::DependencyCodeTable::Mathcode => 4,
            crate::DependencyCodeTable::Delcode => 5,
        };
        let _ = self.hash_code_table(index, hasher);
    }
}

pub(super) fn hash_print_sink(sink: crate::world::PrintSink, hasher: &mut StateHasher) {
    match sink {
        crate::world::PrintSink::Terminal => hasher.tag(0),
        crate::world::PrintSink::Log => hasher.tag(1),
        crate::world::PrintSink::TerminalAndLog => hasher.tag(2),
        crate::world::PrintSink::Stream(slot) => {
            hasher.tag(3);
            hasher.u8(slot.raw());
        }
    }
}

fn exact_identity_from_hashers(domain: u64, mut write: impl FnMut(&mut StateHasher)) -> u64 {
    let mut hasher = StateHasher::new_exact(domain);
    write(&mut hasher);
    hasher.finish_exact_identity()
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum SemanticCellKey {
    Meaning {
        kind: ControlSequenceKind,
        name: String,
    },
    Bank {
        bank: u8,
        index: u32,
    },
    FontBank {
        bank: u8,
        font: FontSemanticKey,
        index: u32,
    },
}

fn hash_control_sequence_kind(kind: ControlSequenceKind, hasher: &mut StateHasher) {
    hasher.u8(match kind {
        ControlSequenceKind::Null
        | ControlSequenceKind::SingleCharacter
        | ControlSequenceKind::Named => 0,
        ControlSequenceKind::ActiveCharacter => 1,
        ControlSequenceKind::Internal => 2,
    });
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct FontSemanticKey {
    name: String,
    content_hash: [u8; 32],
    checksum: u32,
    design_size: i32,
    size: i32,
    complete_hash: ContentHash,
    identifier: Option<(ControlSequenceKind, String)>,
}

fn hash_prepared_mag(value: Option<i32>, hasher: &mut StateHasher) {
    hasher.tag(0x40);
    match value {
        Some(value) => {
            hasher.bool(true);
            hasher.i32(value);
        }
        None => hasher.bool(false),
    }
}

fn hash_register_alias(tag: u8, index: u16, hasher: &mut StateHasher) {
    hasher.tag(tag);
    hasher.u16(index);
}

fn hash_expandable_primitive(primitive: ExpandablePrimitive, hasher: &mut StateHasher) {
    hasher.tag(14);
    hasher.u64(primitive.operand());
}

fn hash_unexpandable_primitive(primitive: UnexpandablePrimitive, hasher: &mut StateHasher) {
    hasher.tag(15);
    hasher.u64(primitive.operand());
}

fn hash_unknown_meaning(raw: RawMeaning, hasher: &mut StateHasher) {
    hasher.tag(16);
    hasher.u8(raw.op());
    hasher.u8(raw.flags().bits());
    hasher.u64(raw.operand());
}

fn hash_catcode(cat: Catcode, hasher: &mut StateHasher) {
    hasher.u8(cat as u8);
}

fn hash_font_semantic_key(font: &FontSemanticKey, hasher: &mut StateHasher) {
    hasher.tag(0x68);
    hasher.bytes(&font.complete_hash.bytes());
}

pub(super) fn hash_kern_kind(kind: KernKind, hasher: &mut StateHasher) {
    hasher.u8(match kind {
        KernKind::Explicit => 0,
        KernKind::Font => 1,
        KernKind::Accent => 2,
        KernKind::Mu => 3,
        KernKind::LeftMargin => 4,
        KernKind::RightMargin => 5,
        KernKind::Auto => 6,
    });
}

pub(super) fn hash_glue_kind(kind: GlueKind, hasher: &mut StateHasher) {
    hasher.u8(match kind {
        GlueKind::Normal => 0,
        GlueKind::BaselineSkip => 1,
        GlueKind::LineSkip => 2,
        GlueKind::TopSkip => 3,
        GlueKind::SplitTopSkip => 4,
        GlueKind::LeftSkip => 5,
        GlueKind::RightSkip => 6,
        GlueKind::ParFillSkip => 7,
        GlueKind::Leaders => 8,
        GlueKind::Cleaders => 9,
        GlueKind::Xleaders => 10,
        GlueKind::MuSkip => 11,
        GlueKind::NonScript => 12,
        GlueKind::AboveDisplaySkip => 13,
        GlueKind::BelowDisplaySkip => 14,
        GlueKind::AboveDisplayShortSkip => 15,
        GlueKind::BelowDisplayShortSkip => 16,
        GlueKind::ThinMuSkip => 17,
        GlueKind::MedMuSkip => 18,
        GlueKind::ThickMuSkip => 19,
        GlueKind::TabSkip => 20,
        GlueKind::ParSkip => 21,
        GlueKind::SpaceSkip => 22,
        GlueKind::XSpaceSkip => 23,
    });
}

pub(super) fn hash_math_char(ch: crate::math::MathChar, hasher: &mut StateHasher) {
    hasher.u8(ch.family);
    hasher.u32(ch.character as u32);
}

pub(super) fn hash_noad_kind(kind: &crate::math::NoadKind, hasher: &mut StateHasher) {
    match kind {
        crate::math::NoadKind::Normal(class) => {
            hasher.tag(0);
            hasher.u8(match class {
                crate::math::NoadClass::Ord => 0,
                crate::math::NoadClass::Op => 1,
                crate::math::NoadClass::Bin => 2,
                crate::math::NoadClass::Rel => 3,
                crate::math::NoadClass::Open => 4,
                crate::math::NoadClass::Close => 5,
                crate::math::NoadClass::Punct => 6,
                crate::math::NoadClass::Inner => 7,
            });
        }
        crate::math::NoadKind::Operator(limit_type) => {
            hasher.tag(1);
            hasher.u8(match limit_type {
                crate::math::LimitType::DisplayLimits => 0,
                crate::math::LimitType::Limits => 1,
                crate::math::LimitType::NoLimits => 2,
            });
        }
        crate::math::NoadKind::Radical { delimiter } => {
            hasher.tag(2);
            hasher.u32(*delimiter);
        }
        crate::math::NoadKind::Accent { accent } => {
            hasher.tag(3);
            hash_math_char(*accent, hasher);
        }
        crate::math::NoadKind::LeftDelimiter { delimiter } => {
            hasher.tag(4);
            hasher.u32(*delimiter);
        }
        crate::math::NoadKind::RightDelimiter { delimiter } => {
            hasher.tag(5);
            hasher.u32(*delimiter);
        }
        crate::math::NoadKind::MiddleDelimiter { delimiter } => {
            hasher.tag(9);
            hasher.u32(*delimiter);
        }
        crate::math::NoadKind::Underline => hasher.tag(6),
        crate::math::NoadKind::Overline => hasher.tag(7),
        crate::math::NoadKind::VCenter => hasher.tag(8),
    }
}

pub(super) fn hash_fraction_thickness(
    thickness: crate::math::FractionThickness,
    hasher: &mut StateHasher,
) {
    match thickness {
        crate::math::FractionThickness::Default => hasher.tag(0),
        crate::math::FractionThickness::Explicit(value) => {
            hasher.tag(1);
            hasher.i32(value.raw());
        }
    }
}

pub(super) fn hash_optional_delimiter(delimiter: Option<u32>, hasher: &mut StateHasher) {
    match delimiter {
        Some(delimiter) => {
            hasher.bool(true);
            hasher.u32(delimiter);
        }
        None => hasher.bool(false),
    }
}

fn hash_internal_integer(integer: InternalInteger, hasher: &mut StateHasher) {
    match integer {
        InternalInteger::Badness => hasher.tag(0),
        InternalInteger::InputLineNumber => hasher.tag(1),
        InternalInteger::ETeXVersion => hasher.tag(2),
        InternalInteger::PdfTeXVersion => hasher.tag(9),
        InternalInteger::PdfElapsedTime => hasher.tag(10),
        InternalInteger::PdfRandomSeed => hasher.tag(11),
        InternalInteger::PdfShellEscape => hasher.tag(12),
        InternalInteger::PdfLastObject => hasher.tag(13),
        InternalInteger::PdfLastAnnot => hasher.tag(17),
        InternalInteger::PdfLastLink => hasher.tag(18),
        InternalInteger::PdfLastXPos => hasher.tag(14),
        InternalInteger::PdfLastYPos => hasher.tag(15),
        InternalInteger::PdfLastXForm => hasher.tag(16),
        InternalInteger::PdfLastXImage => hasher.tag(21),
        InternalInteger::PdfReturnValue => hasher.tag(22),
        InternalInteger::PdfLastXImagePages => hasher.tag(23),
        InternalInteger::PdfLastXImageColorDepth => hasher.tag(24),
        InternalInteger::CurrentGroupLevel => hasher.tag(3),
        InternalInteger::CurrentGroupType => hasher.tag(4),
        InternalInteger::CurrentIfLevel => hasher.tag(5),
        InternalInteger::CurrentIfType => hasher.tag(6),
        InternalInteger::CurrentIfBranch => hasher.tag(7),
        InternalInteger::LastNodeType => hasher.tag(8),
    }
}

pub(super) fn hash_sign(sign: Sign, hasher: &mut StateHasher) {
    hasher.u8(match sign {
        Sign::Normal => 0,
        Sign::Stretching => 1,
        Sign::Shrinking => 2,
    });
}

pub(super) fn hash_optional_scaled(value: Option<crate::scaled::Scaled>, hasher: &mut StateHasher) {
    match value {
        Some(value) => {
            hasher.bool(true);
            hasher.i32(value.raw());
        }
        None => hasher.bool(false),
    }
}

fn bank_order(bank: BankTag) -> u8 {
    match bank {
        BankTag::Meaning => 0,
        BankTag::Count => 1,
        BankTag::Dimen => 2,
        BankTag::Skip => 3,
        BankTag::Toks => 4,
        BankTag::Box => 5,
        BankTag::IntParam => 6,
        BankTag::DimenParam => 7,
        BankTag::GlueParam => 8,
        BankTag::TokParam => 9,
        BankTag::Muskip => 10,
        BankTag::FontDimen => 11,
        BankTag::FontParamLen => 12,
        BankTag::FontHyphenChar => 13,
        BankTag::FontSkewChar => 14,
        BankTag::CurrentFont => 15,
        BankTag::MathFamilyFont => 16,
        BankTag::PdfLpCode => 17,
        BankTag::PdfRpCode => 18,
        BankTag::PdfEfCode => 19,
        BankTag::PdfTagCode => 20,
        BankTag::PdfKnbsCode => 21,
        BankTag::PdfStbsCode => 22,
        BankTag::PdfShbsCode => 23,
        BankTag::PdfKnbcCode => 24,
        BankTag::PdfKnacCode => 25,
        BankTag::PdfNoLigatures => 26,
    }
}

fn decode_u32(word: u64) -> u32 {
    match u32::try_from(word) {
        Ok(value) => value,
        Err(_) => panic!("opaque id word exceeds u32"),
    }
}

fn unpack_font_dimen_index(index: u32) -> (FontId, u32) {
    let font = FontId::new(index >> FONT_DIMEN_BITS);
    let slot = (index & FONT_DIMEN_MASK) + 1;
    (font, slot)
}

#[cfg(test)]
mod cell_tests {
    use super::*;

    #[test]
    fn canonical_hash_cells_preserve_full_symbol_index_and_drop_global_bit() {
        for index in [1 << 26, (1 << 30) - 1] {
            let canonical = CellId::new_global(BankTag::Meaning, index).without_assignment_scope();
            assert_eq!(canonical, CellId::new(BankTag::Meaning, index));
            assert_eq!(canonical.index(), index);
            assert!(!canonical.is_global());
        }
    }
}
