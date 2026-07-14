use super::{SnapshotOwner, StoreSnapshot, Stores};
use crate::cell::{BankTag, CellId};
use crate::glue::GlueSpec;
use crate::ids::{FontId, GlueId, MacroDefinitionId, NodeListId, TokenListId};
use crate::interner::{ControlSequenceKind, Symbol, SymbolId};
use crate::journal::Entry;
use crate::meaning::{
    ExpandablePrimitive, InternalInteger, Meaning, RawMeaning, UnexpandablePrimitive,
};
#[cfg(test)]
use crate::node::{BoxNode, LeaderPayload, Whatsit};
use crate::node::{GlueKind, KernKind, Node, Sign};
#[cfg(test)]
use crate::node_arena::NodeRef;
use crate::state_hash::{StateHashComponent, StateHashFragment, StateHasher};
use crate::token::Catcode;
use ahash::AHashMap;
use std::collections::VecDeque;

const STORE_SLICE_DOMAIN: u64 = 0x7374_6f72_6573_6c63;
const JOURNAL_SLICE_DOMAIN: u64 = 0x6a6f_7572_6e61_6c73;
const CODE_TABLES_DOMAIN: u64 = 0x636f_6465_7461_626c;
const HYPHENATION_DOMAIN: u64 = 0x6879_7068_656e_6174;
const PREPARED_MAG_DOMAIN: u64 = 0x7072_6570_5f6d_6167;
const FONT_SELECTION_DOMAIN: u64 = 0x666f_6e74_5f73_656c;
const CELL_VALUE_DOMAIN: u64 = 0x6365_6c6c_7661_6c75;
const CELL_ORDER_DOMAIN: u64 = 0x6365_6c6c_5f6f_7264;
#[cfg(test)]
const NODE_LIST_MAX_ITEMS: usize = 1_000_000;
const FONT_DIMEN_BITS: u32 = 15;
const FONT_DIMEN_MASK: u32 = (1 << FONT_DIMEN_BITS) - 1;

/// Derived semantic fingerprints at the latest checkpoint boundary.
///
/// This is an accelerator, not rollback state. [`Stores::rollback`] clears it
/// so the next slice reconstructs any needed baseline from journal `old`
/// words. Keeping it out of [`StoreSnapshot`] preserves O(1) snapshots.
#[derive(Debug)]
pub(super) struct SemanticHashCache {
    cells: AHashMap<CellId, CachedCellHash>,
    code_tables: [Option<CachedProjection<crate::code_tables::CodeTablesSemanticCursor>>; 6],
    hyphenation: Option<CachedProjection<HyphenationSemanticCursor>>,
    last_loaded_font: Option<CachedProjection<FontSelectionCursor>>,
    first_old: Vec<(CellId, usize, u64)>,
    changed_cells: Vec<(u64, CellId)>,
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
            code_tables: core::array::from_fn(|_| None),
            hyphenation: None,
            last_loaded_font: None,
            first_old: Vec::new(),
            changed_cells: Vec::new(),
            #[cfg(test)]
            hyphenation_hash_calls: 0,
        }
    }
}

impl Clone for SemanticHashCache {
    fn clone(&self) -> Self {
        Self {
            cells: self.cells.clone(),
            code_tables: self.code_tables.clone(),
            hyphenation: self.hyphenation.clone(),
            last_loaded_font: self.last_loaded_font.clone(),
            first_old: Vec::new(),
            changed_cells: Vec::new(),
            #[cfg(test)]
            hyphenation_hash_calls: 0,
        }
    }
}

impl SemanticHashCache {
    pub(super) fn clear(&mut self) {
        self.cells.clear();
        self.code_tables = core::array::from_fn(|_| None);
        self.hyphenation = None;
        self.last_loaded_font = None;
        self.first_old.clear();
        self.changed_cells.clear();
    }

    #[cfg(test)]
    pub(super) fn testing_scratch_capacities(&self) -> (usize, usize) {
        (self.first_old.capacity(), self.changed_cells.capacity())
    }

    #[cfg(test)]
    pub(super) const fn testing_hyphenation_hash_calls(&self) -> usize {
        self.hyphenation_hash_calls
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
    #[must_use]
    pub(crate) fn state_hash_cursor(&self) -> StoreStateHashCursor {
        StoreStateHashCursor {
            owner: self.owner.snapshot_owner(),
            journal_pos: self.env.current_journal_pos(),
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
        end: &StoreSnapshot,
    ) -> u64 {
        self.assert_valid_hash_cursor(start);
        self.assert_valid_snapshot(end);
        assert!(
            start.journal_pos <= end.env_snapshot.journal_pos(),
            "state hash cursor journal position is after snapshot"
        );

        #[cfg(feature = "profiling-stats")]
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
                &mut cache.code_tables[table],
                &end_cursor.code_tables,
                table,
                |projection| self.hash_code_table(table, projection),
            )
        });
        #[cfg(test)]
        let rehash_hyphenation = cache
            .hyphenation
            .as_ref()
            .is_none_or(|cached| cached.key != end_cursor.hyphenation_root);
        let hyphenation = cached_projection(
            &mut cache.hyphenation,
            &end_cursor.hyphenation_root,
            HYPHENATION_DOMAIN,
            StateHashComponent::Hyphenation,
            |projection| self.hyphenation.hash_semantic(projection),
        );
        #[cfg(test)]
        if rehash_hyphenation {
            cache.hyphenation_hash_calls += 1;
        }
        let prepared_mag = StateHashFragment::from_measured_builder(
            PREPARED_MAG_DOMAIN,
            StateHashComponent::PreparedMag,
            1,
            |projection| hash_prepared_mag(self.prepared_mag, projection),
        );
        let last_loaded_font = cached_projection(
            &mut cache.last_loaded_font,
            &end_cursor.last_loaded_font,
            FONT_SELECTION_DOMAIN,
            StateHashComponent::FontSelection,
            |projection| {
                self.hash_font(self.last_loaded_font, projection);
                1
            },
        );
        self.semantic_hash_cache = cache;
        let mut hasher = StateHasher::new(STORE_SLICE_DOMAIN);
        journal.apply(&mut hasher);
        for fragment in code_tables {
            fragment.apply(&mut hasher);
        }
        hyphenation.apply(&mut hasher);
        prepared_mag.apply(&mut hasher);
        last_loaded_font.apply(&mut hasher);
        hasher.finish()
    }

    pub(crate) fn hash_token_list_semantic(&self, id: TokenListId, hasher: &mut StateHasher) {
        let id = self.resolve_stored_token_list(id);
        hasher.tag(0x50);
        hasher.u64(self.tokens.semantic_id(id).value());
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
            self.hash_node_semantic_identity(node, hasher);
        }
        len
    }

    pub(crate) fn hash_glue_semantic(&self, id: GlueId, hasher: &mut StateHasher) {
        self.hash_glue(id, hasher);
    }

    pub(crate) fn hash_node_list_semantic(&self, id: NodeListId, hasher: &mut StateHasher) {
        self.hash_node_list_identity(id, hasher);
    }

    pub(crate) fn hash_font_semantic(&self, id: FontId, hasher: &mut StateHasher) {
        self.hash_font(id, hasher);
    }

    #[cfg(test)]
    pub(crate) fn testing_font_semantic_fingerprint(&self, id: FontId) -> u64 {
        self.fonts
            .resolve_complete_hash_fragment(id)
            .expect("stored font slot is not live")
            .fingerprint()
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
        &self,
        start: &StoreStateHashCursor,
        end: &StoreSnapshot,
        cache: &mut SemanticHashCache,
        hasher: &mut StateHasher,
    ) {
        let start_index = start.journal_pos.raw() as usize;
        let end_index = end.env_snapshot.journal_pos().raw() as usize;
        let mut first_old = std::mem::take(&mut cache.first_old);
        let mut changed_cells = std::mem::take(&mut cache.changed_cells);
        debug_assert!(first_old.is_empty());
        debug_assert!(changed_cells.is_empty());
        for (position, entry) in self.env.journal_entries_since(start.journal_pos)
            [..end_index.saturating_sub(start_index)]
            .iter()
            .enumerate()
        {
            match entry {
                Entry::Undo(rec) => {
                    let cell = canonical_cell(rec.cell());
                    first_old.push((cell, position, rec.old()));
                }
                Entry::BoxUndo(id) => {
                    let rec = self.env.box_undo(*id);
                    let cell = CellId::new(crate::cell::BankTag::Box, u32::from(rec.index()));
                    first_old.push((cell, position, rec.old().value()));
                }
                Entry::Marker(_) => {}
            }
        }
        first_old.sort_unstable_by_key(|&(cell, position, _)| (cell, position));
        first_old.dedup_by_key(|entry| entry.0);

        for &(cell, _, old_word) in &first_old {
            let new_word = self.env.semantic_word(cell);
            let current_hash = self.cell_value_hash(cell, new_word);
            let baseline_hash = cache.cells.get(&cell).map_or_else(
                || self.cell_value_hash(cell, old_word),
                |cached| cached.value_hash,
            );

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

        #[cfg(feature = "profiling-stats")]
        crate::measurement::record_hash_changed_cells(
            changed_cells.len(),
            first_old.capacity() * core::mem::size_of::<(CellId, usize, u64)>()
                + changed_cells.capacity() * core::mem::size_of::<(u64, CellId)>(),
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
        cache.first_old = first_old;
        cache.changed_cells = changed_cells;
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
                    index: u32::from(slot),
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
        let mut hasher = StateHasher::new(CELL_VALUE_DOMAIN);
        self.hash_cell_value(cell, word, &mut hasher);
        hasher.finish()
    }

    fn hash_cell_value(&self, cell: CellId, word: u64, hasher: &mut StateHasher) {
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
            BankTag::Toks | BankTag::TokParam => {
                self.hash_token_list_semantic(
                    self.resolve_stored_token_list(TokenListId::new(decode_u32(word))),
                    hasher,
                );
            }
            BankTag::Box => match NodeListId::decode_box_word(word) {
                Some(id) => self.hash_node_list_identity(id, hasher),
                None => hasher.tag(0),
            },
            BankTag::FontDimen => hasher.i32(word as u32 as i32),
            BankTag::FontParamLen => hasher.u16(decode_u16(word)),
            BankTag::FontHyphenChar | BankTag::FontSkewChar => hasher.i32(word as u32 as i32),
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
            Meaning::UnexpandablePrimitive(primitive) => {
                hash_unexpandable_primitive(primitive, hasher);
            }
            Meaning::Unknown(raw) => hash_unknown_meaning(raw, hasher),
        }
    }

    fn hash_macro_definition(&self, id: MacroDefinitionId, hasher: &mut StateHasher) {
        self.assert_live_macro_definition(id);
        let definition = self.macros.get(id);
        hasher.u8(definition.flags().bits());
        self.hash_token_list_semantic(definition.parameter_text(), hasher);
        self.hash_token_list_semantic(definition.replacement_text(), hasher);
    }

    fn hash_glue(&self, id: GlueId, hasher: &mut StateHasher) {
        let GlueSpec {
            width,
            stretch,
            stretch_order,
            shrink,
            shrink_order,
        } = self
            .glue
            .resolve_get(id)
            .expect("stored glue slot is not live");
        hasher.tag(0x60);
        hasher.i32(width.raw());
        hasher.i32(stretch.raw());
        hasher.u8(stretch_order as u8);
        hasher.i32(shrink.raw());
        hasher.u8(shrink_order as u8);
    }

    #[cfg(test)]
    fn hash_node_ref(
        &self,
        node: NodeRef<'_>,
        hasher: &mut StateHasher,
        stack: &mut Vec<NodeFrame>,
    ) {
        match node {
            NodeRef::Char { font, ch } => {
                hasher.tag(0);
                self.hash_font(font, hasher);
                hasher.u32(ch as u32);
            }
            NodeRef::Lig { font, ch, orig } => {
                hasher.tag(1);
                self.hash_font(font, hasher);
                hasher.u32(ch as u32);
                hasher.u32(orig.0 as u32);
                hasher.u32(orig.1 as u32);
            }
            NodeRef::Kern { amount, kind } => {
                hasher.tag(2);
                hasher.i32(amount.raw());
                hash_kern_kind(kind, hasher);
            }
            NodeRef::Glue { spec, kind, leader } => {
                hasher.tag(3);
                self.hash_glue(spec, hasher);
                hash_glue_kind(kind, hasher);
                self.hash_leader_payload_ref(leader, hasher, stack);
            }
            NodeRef::Penalty(value) => {
                hasher.tag(4);
                hasher.i32(value);
            }
            NodeRef::Rule {
                width,
                height,
                depth,
            } => {
                hasher.tag(5);
                hash_optional_scaled(width, hasher);
                hash_optional_scaled(height, hasher);
                hash_optional_scaled(depth, hasher);
            }
            NodeRef::HList(box_node) => self.hash_box_node(6, box_node, hasher, stack),
            NodeRef::VList(box_node) => self.hash_box_node(7, box_node, hasher, stack),
            NodeRef::Unset(unset) => {
                hasher.tag(8);
                hasher.u8(match unset.kind {
                    crate::node::UnsetKind::HBox => 0,
                    crate::node::UnsetKind::VBox => 1,
                });
                hasher.i32(unset.width.raw());
                hasher.i32(unset.height.raw());
                hasher.i32(unset.depth.raw());
                hasher.u16(unset.span_count);
                hasher.i32(unset.stretch.raw());
                hasher.u8(unset.stretch_order as u8);
                hasher.i32(unset.shrink.raw());
                hasher.u8(unset.shrink_order as u8);
                stack.push(NodeFrame::List(unset.children));
            }
            NodeRef::Disc {
                kind,
                pre,
                post,
                replace,
            } => {
                hasher.tag(9);
                hasher.u8(match kind {
                    crate::node::DiscKind::Discretionary => 0,
                    crate::node::DiscKind::ExplicitHyphen => 1,
                    crate::node::DiscKind::AutomaticHyphen => 2,
                });
                stack.push(NodeFrame::List(replace));
                stack.push(NodeFrame::List(post));
                stack.push(NodeFrame::List(pre));
            }
            NodeRef::Mark { class, tokens } => {
                hasher.tag(10);
                hasher.u16(class);
                self.hash_token_list_semantic(tokens, hasher);
            }
            NodeRef::Ins {
                class,
                size,
                split_top_skip,
                split_max_depth,
                floating_penalty,
                content,
            } => {
                hasher.tag(11);
                hasher.u16(class);
                hasher.i32(size.raw());
                self.hash_glue_semantic(split_top_skip, hasher);
                hasher.i32(split_max_depth.raw());
                hasher.i32(floating_penalty);
                stack.push(NodeFrame::List(content));
            }
            NodeRef::Whatsit(whatsit) => self.hash_whatsit_ref(whatsit, hasher),
            NodeRef::MathOn(width) => {
                hasher.tag(13);
                hasher.i32(width.raw());
            }
            NodeRef::MathOff(width) => {
                hasher.tag(14);
                hasher.i32(width.raw());
            }
            NodeRef::Direction(direction) => {
                hasher.tag(22);
                hasher.u8(direction as u8);
            }
            NodeRef::Adjust(content) => {
                hasher.tag(15);
                stack.push(NodeFrame::List(content));
            }
            NodeRef::MathNoad(noad) => {
                hasher.tag(16);
                hash_noad_kind(&noad.kind, hasher);
                self.hash_math_field(noad.nucleus, hasher, stack);
                self.hash_math_field(noad.subscript, hasher, stack);
                self.hash_math_field(noad.superscript, hasher, stack);
            }
            NodeRef::FractionNoad(fraction) => {
                hasher.tag(17);
                stack.push(NodeFrame::List(fraction.denominator));
                stack.push(NodeFrame::List(fraction.numerator));
                hash_fraction_thickness(fraction.thickness, hasher);
                hash_optional_delimiter(fraction.left_delimiter, hasher);
                hash_optional_delimiter(fraction.right_delimiter, hasher);
            }
            NodeRef::MathStyle(style) => {
                hasher.tag(18);
                hasher.u8(match style {
                    crate::math::MathStyle::Display => 0,
                    crate::math::MathStyle::Text => 1,
                    crate::math::MathStyle::Script => 2,
                    crate::math::MathStyle::ScriptScript => 3,
                });
            }
            NodeRef::MathChoice(choice) => {
                hasher.tag(19);
                stack.push(NodeFrame::List(choice.script_script));
                stack.push(NodeFrame::List(choice.script));
                stack.push(NodeFrame::List(choice.text));
                stack.push(NodeFrame::List(choice.display));
            }
            NodeRef::MathList(list) => {
                hasher.tag(20);
                hasher.u8(u8::from(list.display));
                stack.push(NodeFrame::List(list.content));
            }
            NodeRef::Nonscript => hasher.tag(21),
        }
    }

    #[cfg(test)]
    fn hash_node(&self, node: Node, hasher: &mut StateHasher, stack: &mut Vec<NodeFrame>) {
        match node {
            Node::Char { font, ch } => {
                hasher.tag(0);
                self.hash_font(font, hasher);
                hasher.u32(ch as u32);
            }
            Node::Lig { font, ch, orig } => {
                hasher.tag(1);
                self.hash_font(font, hasher);
                hasher.u32(ch as u32);
                hasher.u32(orig.0 as u32);
                hasher.u32(orig.1 as u32);
            }
            Node::Kern { amount, kind } => {
                hasher.tag(2);
                hasher.i32(amount.raw());
                hash_kern_kind(kind, hasher);
            }
            Node::Glue { spec, kind, leader } => {
                hasher.tag(3);
                self.hash_glue(spec, hasher);
                hash_glue_kind(kind, hasher);
                self.hash_leader_payload(leader, hasher, stack);
            }
            Node::Penalty(value) => {
                hasher.tag(4);
                hasher.i32(value);
            }
            Node::Rule {
                width,
                height,
                depth,
            } => {
                hasher.tag(5);
                hash_optional_scaled(width, hasher);
                hash_optional_scaled(height, hasher);
                hash_optional_scaled(depth, hasher);
            }
            Node::HList(box_node) => self.hash_box_node(6, box_node, hasher, stack),
            Node::VList(box_node) => self.hash_box_node(7, box_node, hasher, stack),
            Node::Unset(unset) => {
                hasher.tag(8);
                hasher.u8(match unset.kind {
                    crate::node::UnsetKind::HBox => 0,
                    crate::node::UnsetKind::VBox => 1,
                });
                hasher.i32(unset.width.raw());
                hasher.i32(unset.height.raw());
                hasher.i32(unset.depth.raw());
                hasher.u16(unset.span_count);
                hasher.i32(unset.stretch.raw());
                hasher.u8(unset.stretch_order as u8);
                hasher.i32(unset.shrink.raw());
                hasher.u8(unset.shrink_order as u8);
                stack.push(NodeFrame::List(unset.children));
            }
            Node::Disc {
                kind,
                pre,
                post,
                replace,
            } => {
                hasher.tag(9);
                hasher.u8(match kind {
                    crate::node::DiscKind::Discretionary => 0,
                    crate::node::DiscKind::ExplicitHyphen => 1,
                    crate::node::DiscKind::AutomaticHyphen => 2,
                });
                stack.push(NodeFrame::List(replace));
                stack.push(NodeFrame::List(post));
                stack.push(NodeFrame::List(pre));
            }
            Node::Mark { class, tokens } => {
                hasher.tag(10);
                hasher.u16(class);
                self.hash_token_list_semantic(tokens, hasher);
            }
            Node::Ins {
                class,
                size,
                split_top_skip,
                split_max_depth,
                floating_penalty,
                content,
            } => {
                hasher.tag(11);
                hasher.u16(class);
                hasher.i32(size.raw());
                self.hash_glue_semantic(split_top_skip, hasher);
                hasher.i32(split_max_depth.raw());
                hasher.i32(floating_penalty);
                stack.push(NodeFrame::List(content));
            }
            Node::Whatsit(whatsit) => self.hash_whatsit_ref(&whatsit, hasher),
            Node::MathOn(width) => {
                hasher.tag(13);
                hasher.i32(width.raw());
            }
            Node::MathOff(width) => {
                hasher.tag(14);
                hasher.i32(width.raw());
            }
            Node::Direction(direction) => {
                hasher.tag(22);
                hasher.u8(direction as u8);
            }
            Node::Adjust(content) => {
                hasher.tag(15);
                stack.push(NodeFrame::List(content));
            }
            Node::MathNoad(noad) => {
                hasher.tag(16);
                hash_noad_kind(&noad.kind, hasher);
                self.hash_math_field(noad.nucleus, hasher, stack);
                self.hash_math_field(noad.subscript, hasher, stack);
                self.hash_math_field(noad.superscript, hasher, stack);
            }
            Node::FractionNoad(fraction) => {
                hasher.tag(17);
                stack.push(NodeFrame::List(fraction.denominator));
                stack.push(NodeFrame::List(fraction.numerator));
                hash_fraction_thickness(fraction.thickness, hasher);
                hash_optional_delimiter(fraction.left_delimiter, hasher);
                hash_optional_delimiter(fraction.right_delimiter, hasher);
            }
            Node::MathStyle(style) => {
                hasher.tag(18);
                hasher.u8(match style {
                    crate::math::MathStyle::Display => 0,
                    crate::math::MathStyle::Text => 1,
                    crate::math::MathStyle::Script => 2,
                    crate::math::MathStyle::ScriptScript => 3,
                });
            }
            Node::MathChoice(choice) => {
                hasher.tag(19);
                stack.push(NodeFrame::List(choice.script_script));
                stack.push(NodeFrame::List(choice.script));
                stack.push(NodeFrame::List(choice.text));
                stack.push(NodeFrame::List(choice.display));
            }
            Node::MathList(list) => {
                hasher.tag(20);
                hasher.u8(u8::from(list.display));
                stack.push(NodeFrame::List(list.content));
            }
            Node::Nonscript => hasher.tag(21),
        }
    }

    #[cfg(test)]
    fn hash_math_field(
        &self,
        field: crate::math::MathField,
        hasher: &mut StateHasher,
        stack: &mut Vec<NodeFrame>,
    ) {
        match field {
            crate::math::MathField::Empty => hasher.tag(0),
            crate::math::MathField::MathChar(ch) => {
                hasher.tag(1);
                hash_math_char(ch, hasher);
            }
            crate::math::MathField::MathTextChar(ch) => {
                hasher.tag(2);
                hash_math_char(ch, hasher);
            }
            crate::math::MathField::SubBox(list) => {
                hasher.tag(3);
                stack.push(NodeFrame::List(list));
            }
            crate::math::MathField::SubMlist(list) => {
                hasher.tag(4);
                stack.push(NodeFrame::List(list));
            }
        }
    }

    #[cfg(test)]
    fn hash_box_node(
        &self,
        tag: u8,
        box_node: BoxNode,
        hasher: &mut StateHasher,
        stack: &mut Vec<NodeFrame>,
    ) {
        hasher.tag(tag);
        hasher.i32(box_node.width.raw());
        hasher.i32(box_node.height.raw());
        hasher.i32(box_node.depth.raw());
        hasher.i32(box_node.shift.raw());
        hasher.i32(box_node.glue_set.numerator());
        hasher.i32(box_node.glue_set.denominator());
        hash_sign(box_node.glue_sign, hasher);
        hasher.u8(box_node.glue_order as u8);
        stack.push(NodeFrame::List(box_node.children));
    }

    #[cfg(test)]
    fn hash_leader_payload(
        &self,
        payload: Option<LeaderPayload>,
        hasher: &mut StateHasher,
        stack: &mut Vec<NodeFrame>,
    ) {
        match payload {
            None => hasher.tag(0),
            Some(LeaderPayload::HList(box_node)) => self.hash_box_node(1, box_node, hasher, stack),
            Some(LeaderPayload::VList(box_node)) => self.hash_box_node(2, box_node, hasher, stack),
            Some(LeaderPayload::Rule {
                width,
                height,
                depth,
            }) => {
                hasher.tag(3);
                hash_optional_scaled(width, hasher);
                hash_optional_scaled(height, hasher);
                hash_optional_scaled(depth, hasher);
            }
        }
    }

    #[cfg(test)]
    fn hash_leader_payload_ref(
        &self,
        payload: Option<&LeaderPayload>,
        hasher: &mut StateHasher,
        stack: &mut Vec<NodeFrame>,
    ) {
        match payload {
            None => hasher.tag(0),
            Some(LeaderPayload::HList(box_node)) => self.hash_box_node(1, *box_node, hasher, stack),
            Some(LeaderPayload::VList(box_node)) => self.hash_box_node(2, *box_node, hasher, stack),
            Some(LeaderPayload::Rule {
                width,
                height,
                depth,
            }) => {
                hasher.tag(3);
                hash_optional_scaled(*width, hasher);
                hash_optional_scaled(*height, hasher);
                hash_optional_scaled(*depth, hasher);
            }
        }
    }

    #[cfg(test)]
    fn hash_whatsit_ref(&self, whatsit: &Whatsit, hasher: &mut StateHasher) {
        match whatsit {
            Whatsit::OpenOut { slot, path } => {
                hasher.tag(13);
                hasher.u8(slot.raw());
                hasher.str(path);
            }
            Whatsit::CloseOut { slot } => {
                hasher.tag(14);
                hasher.u8(slot.raw());
            }
            Whatsit::DeferredWrite { sink, tokens } => {
                hasher.tag(12);
                hash_print_sink(*sink, hasher);
                self.hash_token_list_semantic(*tokens, hasher);
            }
            Whatsit::Special { class, payload } => {
                hasher.tag(16);
                hasher.bytes(class.as_bytes());
                hasher.bytes(payload);
            }
            Whatsit::Language {
                language,
                left_hyphen_min,
                right_hyphen_min,
            } => {
                hasher.tag(17);
                hasher.u8(*language);
                hasher.u8(*left_hyphen_min);
                hasher.u8(*right_hyphen_min);
            }
        }
    }

    fn hash_font(&self, font: FontId, hasher: &mut StateHasher) {
        hasher.tag(0x68);
        self.fonts
            .resolve_complete_hash_fragment(font)
            .expect("stored font slot is not live")
            .apply(hasher);
    }

    fn font_semantic_key(&self, font: FontId) -> FontSemanticKey {
        #[cfg(feature = "profiling-stats")]
        crate::measurement::record_owned_font_key();
        self.assert_live_font(font);
        let identifier = self.fonts.identifier(font).map(|symbol| {
            self.assert_live_symbol(symbol);
            (
                self.interner.kind_id(symbol),
                self.interner.resolve_id(symbol).to_owned(),
            )
        });
        let complete_hash = self.fonts.complete_hash_fragment(font).fingerprint();
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

    #[cfg(test)]
    fn hash_node_tree_from_node(&self, node: Node, hasher: &mut StateHasher) -> usize {
        let mut stack = Vec::new();
        self.hash_node(node, hasher, &mut stack);
        let mut seen = 0_usize;
        while let Some(frame) = stack.pop() {
            seen += 1;
            assert!(
                seen <= NODE_LIST_MAX_ITEMS,
                "state hash exceeded maximum node traversal items"
            );
            match frame {
                NodeFrame::List(id) => {
                    let nodes = self.nodes(id);
                    hasher.tag(0x70);
                    hasher.usize(nodes.len());
                    stack.push(NodeFrame::ListEnd);
                    for index in (0..nodes.len()).rev() {
                        stack.push(NodeFrame::NodeAt(id, index));
                    }
                }
                NodeFrame::ListEnd => hasher.tag(0x71),
                NodeFrame::NodeAt(id, index) => {
                    let node = self
                        .nodes(id)
                        .get(index)
                        .expect("state-hash node frame is live");
                    self.hash_node_ref(node, hasher, &mut stack);
                }
            }
        }
        seen + 1
    }

    #[cfg(test)]
    pub(super) fn testing_assert_owned_borrowed_node_hashes_equal(&self, id: NodeListId) {
        let nodes = self.nodes(id);
        for index in 0..nodes.len() {
            let owned = nodes
                .get(index)
                .expect("test node index is live")
                .to_owned();
            let mut owned_hasher = StateHasher::new(0x6e6f_6465_5f65_7175);
            self.hash_node_tree_from_node(owned, &mut owned_hasher);

            let mut borrowed_hasher = StateHasher::new(0x6e6f_6465_5f65_7175);
            let mut stack = Vec::new();
            self.hash_node_ref(
                nodes.get(index).expect("test node index is live"),
                &mut borrowed_hasher,
                &mut stack,
            );
            let mut seen = 0_usize;
            while let Some(frame) = stack.pop() {
                seen += 1;
                assert!(seen <= NODE_LIST_MAX_ITEMS);
                match frame {
                    NodeFrame::List(id) => {
                        let list = self.nodes(id);
                        borrowed_hasher.tag(0x70);
                        borrowed_hasher.usize(list.len());
                        stack.push(NodeFrame::ListEnd);
                        for child_index in (0..list.len()).rev() {
                            stack.push(NodeFrame::NodeAt(id, child_index));
                        }
                    }
                    NodeFrame::ListEnd => borrowed_hasher.tag(0x71),
                    NodeFrame::NodeAt(id, child_index) => self.hash_node_ref(
                        self.nodes(id)
                            .get(child_index)
                            .expect("test child index is live"),
                        &mut borrowed_hasher,
                        &mut stack,
                    ),
                }
            }
            assert_eq!(owned_hasher.finish(), borrowed_hasher.finish());
        }
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
        ControlSequenceKind::Named => 0,
        ControlSequenceKind::ActiveCharacter => 1,
    });
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct FontSemanticKey {
    name: String,
    content_hash: [u8; 32],
    checksum: u32,
    design_size: i32,
    size: i32,
    complete_hash: u64,
    identifier: Option<(ControlSequenceKind, String)>,
}

#[cfg(test)]
#[derive(Clone, Debug)]
enum NodeFrame {
    List(NodeListId),
    ListEnd,
    NodeAt(NodeListId, usize),
}

fn canonical_cell(cell: CellId) -> CellId {
    CellId::new(cell.bank(), cell.index())
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
    hasher.u64(font.complete_hash);
}

pub(super) fn hash_kern_kind(kind: KernKind, hasher: &mut StateHasher) {
    hasher.u8(match kind {
        KernKind::Explicit => 0,
        KernKind::Font => 1,
        KernKind::Accent => 2,
        KernKind::Mu => 3,
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
    }
}

fn decode_u32(word: u64) -> u32 {
    match u32::try_from(word) {
        Ok(value) => value,
        Err(_) => panic!("opaque id word exceeds u32"),
    }
}

fn unpack_font_dimen_index(index: u32) -> (FontId, u16) {
    let font = FontId::new(index >> FONT_DIMEN_BITS);
    let slot = ((index & FONT_DIMEN_MASK) + 1) as u16;
    (font, slot)
}

fn decode_u16(word: u64) -> u16 {
    match u16::try_from(word) {
        Ok(value) => value,
        Err(_) => panic!("font parameter count exceeds u16"),
    }
}

#[cfg(test)]
mod cell_tests {
    use super::*;

    #[test]
    fn canonical_hash_cells_preserve_full_symbol_index_and_drop_global_bit() {
        for index in [1 << 26, (1 << 30) - 1] {
            let canonical = canonical_cell(CellId::new_global(BankTag::Meaning, index));
            assert_eq!(canonical, CellId::new(BankTag::Meaning, index));
            assert_eq!(canonical.index(), index);
            assert!(!canonical.is_global());
        }
    }
}
