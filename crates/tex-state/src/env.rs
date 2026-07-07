//! Barriered environment storage.
//!
//! # Freeze theorem
//!
//! `Env` owns all mutable meaning-cell storage and its journal. All fields are
//! private, reads return decoded `Copy` values, and the API exposes no mutable
//! references into the backing arrays. Therefore `&Env` implies frozen state:
//! safe crate consumers cannot change environment cells without obtaining
//! `&mut Env` and passing through the write barrier.

pub mod banks;
pub(crate) mod overflow;

use self::banks::{
    DENSE_REGISTER_COUNT, DimenParam, FixedBank, GlueIdCodec, GlueParam, I32Codec, IntParam,
    NodeListIdCodec, PARAMETER_COUNT, ScaledCodec, TokParam, TokenListIdCodec,
};
use self::overflow::{REGISTER_COUNT, SparseBank};
use crate::cell::{BankTag, CellId};
use crate::epoch::Epoch;
use crate::ids::{GlueId, NodeListId, TokenListId};
use crate::interner::Symbol;
use crate::journal::{Entry, Journal, JournalPos, Marker, UndoRec};
use crate::meaning::Meaning;
use crate::scaled::Scaled;
use std::collections::{HashMap, HashSet};
#[cfg(any(test, feature = "testing", feature = "shadow"))]
use std::hash::{Hash, Hasher};

const SEGMENT_BITS: u32 = 16;
const SEGMENT_LEN: usize = 1 << SEGMENT_BITS;
const SEGMENT_MASK: u32 = (SEGMENT_LEN as u32) - 1;

type MeaningSegment = Box<[u64; SEGMENT_LEN]>;
type StampSegment = Box<[Epoch; SEGMENT_LEN]>;

macro_rules! register_accessors {
    ($get:ident, $set:ident, $set_global:ident, $value:ty, $bank:ident, $dense:ident, $sparse:ident) => {
        #[must_use]
        pub fn $get(&self, index: u16) -> $value {
            if is_dense_register(index) {
                self.$dense.get(index)
            } else {
                self.$sparse.get(index)
            }
        }

        pub fn $set(&mut self, index: u16, value: $value) {
            if is_dense_register(index) {
                self.$dense.set(
                    index,
                    value,
                    &mut self.journal,
                    #[cfg(feature = "shadow")]
                    &mut self.shadow,
                    self.epoch,
                    BankTag::$bank,
                    false,
                );
            } else {
                self.$sparse.set(
                    index,
                    value,
                    &mut self.journal,
                    #[cfg(feature = "shadow")]
                    &mut self.shadow,
                    self.epoch,
                    BankTag::$bank,
                    false,
                );
            }
        }

        pub fn $set_global(&mut self, index: u16, value: $value) {
            if is_dense_register(index) {
                self.$dense.set(
                    index,
                    value,
                    &mut self.journal,
                    #[cfg(feature = "shadow")]
                    &mut self.shadow,
                    self.epoch,
                    BankTag::$bank,
                    true,
                );
            } else {
                self.$sparse.set(
                    index,
                    value,
                    &mut self.journal,
                    #[cfg(feature = "shadow")]
                    &mut self.shadow,
                    self.epoch,
                    BankTag::$bank,
                    true,
                );
            }
        }
    };
}

/// TeX environment cells plus the journal that makes mutation replayable.
#[derive(Clone, Debug)]
pub struct Env {
    meaning_cells: Vec<MeaningSegment>,
    meaning_stamps: Vec<StampSegment>,
    counts: FixedBank<I32Codec, DENSE_REGISTER_COUNT>,
    dimens: FixedBank<ScaledCodec, DENSE_REGISTER_COUNT>,
    skips: FixedBank<GlueIdCodec, DENSE_REGISTER_COUNT>,
    toks: FixedBank<TokenListIdCodec, DENSE_REGISTER_COUNT>,
    boxes: FixedBank<NodeListIdCodec, DENSE_REGISTER_COUNT>,
    overflow_counts: SparseBank<I32Codec>,
    overflow_dimens: SparseBank<ScaledCodec>,
    overflow_skips: SparseBank<GlueIdCodec>,
    overflow_toks: SparseBank<TokenListIdCodec>,
    overflow_boxes: SparseBank<NodeListIdCodec>,
    int_params: FixedBank<I32Codec, PARAMETER_COUNT>,
    dimen_params: FixedBank<ScaledCodec, PARAMETER_COUNT>,
    glue_params: FixedBank<GlueIdCodec, PARAMETER_COUNT>,
    tok_params: FixedBank<TokenListIdCodec, PARAMETER_COUNT>,
    journal: Journal,
    aftergroup: Vec<u64>,
    epoch: Epoch,
    #[cfg(feature = "shadow")]
    shadow: HashMap<CellId, u64>,
}

impl Env {
    /// Creates an empty environment in the first session epoch.
    #[must_use]
    pub fn new() -> Self {
        Self {
            meaning_cells: Vec::new(),
            meaning_stamps: Vec::new(),
            counts: FixedBank::new(),
            dimens: FixedBank::new(),
            skips: FixedBank::new(),
            toks: FixedBank::new(),
            boxes: FixedBank::new(),
            overflow_counts: SparseBank::new(),
            overflow_dimens: SparseBank::new(),
            overflow_skips: SparseBank::new(),
            overflow_toks: SparseBank::new(),
            overflow_boxes: SparseBank::new(),
            int_params: FixedBank::new(),
            dimen_params: FixedBank::new(),
            glue_params: FixedBank::new(),
            tok_params: FixedBank::new(),
            journal: Journal::new(),
            aftergroup: Vec::new(),
            epoch: Epoch::START,
            #[cfg(feature = "shadow")]
            shadow: HashMap::new(),
        }
    }

    /// Returns the current epoch.
    #[must_use]
    pub const fn epoch(&self) -> Epoch {
        self.epoch
    }

    /// Advances to the next epoch.
    pub fn bump_epoch(&mut self) {
        self.epoch.bump();
    }

    /// Returns the current journal end position.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn journal_pos(&self) -> JournalPos {
        self.journal.pos()
    }

    /// Records a checkpoint position and starts a fresh epoch for later writes.
    #[must_use]
    pub(crate) fn checkpoint(&mut self) -> JournalPos {
        let pos = self.journal.pos();
        self.epoch.bump();
        pos
    }

    /// Returns journal entries appended since `pos`.
    #[must_use]
    pub(crate) fn journal_entries_since(&self, pos: JournalPos) -> &[Entry] {
        self.journal.entries_since(pos)
    }

    /// Enters a TeX group.
    pub fn enter_group(&mut self) {
        let aftergroup_start = u32_len(
            self.aftergroup.len(),
            "aftergroup payload list exceeds u32 entries",
        );
        self.journal.push_marker(Marker::Group { aftergroup_start });
        self.epoch.bump();
    }

    /// Pushes an opaque `\aftergroup` payload for the current group.
    pub fn push_aftergroup(&mut self, payload: u64) {
        self.aftergroup.push(payload);
    }

    /// Leaves the innermost TeX group and returns its `\aftergroup` payloads.
    ///
    /// Payloads are returned FIFO. Global assignments in the group survive by
    /// being compacted into the enclosing journal slice.
    #[must_use]
    pub fn leave_group(&mut self) -> Vec<u64> {
        let Some((marker_pos, aftergroup_start)) = self.journal.find_last_group_marker() else {
            panic!("leave_group without matching group marker");
        };
        let entries = self.journal.entries_since(marker_pos).to_vec();
        let mut globals = Vec::new();
        let mut globally_reassigned = HashSet::new();
        let mut first_old = HashMap::new();

        for entry in &entries {
            if let Entry::Undo(rec) = *entry {
                first_old
                    .entry(cell_key(rec.cell()))
                    .or_insert_with(|| rec.old());
            }
        }

        for entry in entries.iter().rev() {
            match *entry {
                Entry::Undo(rec) if rec.cell().is_global() => {
                    globally_reassigned.insert(cell_key(rec.cell()));
                    globals.push(rec);
                }
                Entry::Undo(rec) if globally_reassigned.contains(&cell_key(rec.cell())) => {}
                Entry::Undo(rec) => self.restore_raw(rec.cell(), rec.old()),
                Entry::Marker(Marker::Group { .. }) => break,
                Entry::Marker(Marker::Checkpoint(_)) => {}
            }
        }

        self.journal.truncate_to(marker_pos);
        let mut refiled_globals = HashSet::new();
        for rec in globals.into_iter().rev() {
            self.restore_raw(rec.cell(), rec.new_value());
            let key = cell_key(rec.cell());
            let old = if refiled_globals.insert(key) {
                first_old[&key]
            } else {
                rec.old()
            };
            self.journal
                .push_undo(UndoRec::new(rec.cell(), old, rec.new_value()));
        }

        let aftergroup_start = checked_aftergroup_start(aftergroup_start, self.aftergroup.len());
        let payloads = self.aftergroup.drain(aftergroup_start..).collect();

        // core_state.md §6 / 97a3c1d: restore leaves stamps high, so group
        // exit must start a fresh epoch or the enclosing undo slice can be
        // corrupted by a later write to the same restored cell.
        self.epoch.bump();
        payloads
    }

    /// Rolls back all journaled environment writes after `pos`.
    pub(crate) fn rollback_to(&mut self, pos: JournalPos) {
        let entries = self.journal.entries_since(pos).to_vec();
        for entry in entries.iter().rev() {
            if let Entry::Undo(rec) = *entry {
                self.restore_raw(rec.cell(), rec.old());
            }
        }
        self.journal.truncate_to(pos);
        self.epoch.bump();
    }

    /// Returns the meaning for `symbol`, defaulting to `Undefined`.
    #[must_use]
    pub fn get(&self, symbol: Symbol) -> Meaning {
        let Some(word) = self.meaning_word(symbol.raw()) else {
            return Meaning::Undefined;
        };
        Meaning::decode(word)
    }

    /// Sets the local meaning for `symbol`.
    pub fn set(&mut self, symbol: Symbol, meaning: Meaning) {
        self.set_meaning_word(symbol, meaning.encode(), false);
    }

    /// Sets the global meaning for `symbol`.
    pub fn set_global(&mut self, symbol: Symbol, meaning: Meaning) {
        self.set_meaning_word(symbol, meaning.encode(), true);
    }

    register_accessors!(
        count,
        set_count,
        set_count_global,
        i32,
        Count,
        counts,
        overflow_counts
    );
    register_accessors!(
        dimen,
        set_dimen,
        set_dimen_global,
        Scaled,
        Dimen,
        dimens,
        overflow_dimens
    );
    register_accessors!(
        skip,
        set_skip,
        set_skip_global,
        GlueId,
        Skip,
        skips,
        overflow_skips
    );
    register_accessors!(
        toks,
        set_toks,
        set_toks_global,
        TokenListId,
        Toks,
        toks,
        overflow_toks
    );
    register_accessors!(
        box_reg,
        set_box_reg,
        set_box_reg_global,
        NodeListId,
        Box,
        boxes,
        overflow_boxes
    );

    /// Returns an integer parameter value.
    #[must_use]
    pub fn int_param(&self, param: IntParam) -> i32 {
        self.int_params.get(param.raw())
    }

    /// Sets a local integer parameter value.
    pub fn set_int_param(&mut self, param: IntParam, value: i32) {
        self.int_params.set(
            param.raw(),
            value,
            &mut self.journal,
            #[cfg(feature = "shadow")]
            &mut self.shadow,
            self.epoch,
            BankTag::IntParam,
            false,
        );
    }

    /// Sets a global integer parameter value.
    pub fn set_int_param_global(&mut self, param: IntParam, value: i32) {
        self.int_params.set(
            param.raw(),
            value,
            &mut self.journal,
            #[cfg(feature = "shadow")]
            &mut self.shadow,
            self.epoch,
            BankTag::IntParam,
            true,
        );
    }

    /// Returns a dimension parameter value.
    #[must_use]
    pub fn dimen_param(&self, param: DimenParam) -> Scaled {
        self.dimen_params.get(param.raw())
    }

    /// Sets a local dimension parameter value.
    pub fn set_dimen_param(&mut self, param: DimenParam, value: Scaled) {
        self.dimen_params.set(
            param.raw(),
            value,
            &mut self.journal,
            #[cfg(feature = "shadow")]
            &mut self.shadow,
            self.epoch,
            BankTag::DimenParam,
            false,
        );
    }

    /// Sets a global dimension parameter value.
    pub fn set_dimen_param_global(&mut self, param: DimenParam, value: Scaled) {
        self.dimen_params.set(
            param.raw(),
            value,
            &mut self.journal,
            #[cfg(feature = "shadow")]
            &mut self.shadow,
            self.epoch,
            BankTag::DimenParam,
            true,
        );
    }

    /// Returns a glue parameter value.
    #[must_use]
    pub fn glue_param(&self, param: GlueParam) -> GlueId {
        self.glue_params.get(param.raw())
    }

    /// Sets a local glue parameter value.
    pub fn set_glue_param(&mut self, param: GlueParam, value: GlueId) {
        self.glue_params.set(
            param.raw(),
            value,
            &mut self.journal,
            #[cfg(feature = "shadow")]
            &mut self.shadow,
            self.epoch,
            BankTag::GlueParam,
            false,
        );
    }

    /// Sets a global glue parameter value.
    pub fn set_glue_param_global(&mut self, param: GlueParam, value: GlueId) {
        self.glue_params.set(
            param.raw(),
            value,
            &mut self.journal,
            #[cfg(feature = "shadow")]
            &mut self.shadow,
            self.epoch,
            BankTag::GlueParam,
            true,
        );
    }

    /// Returns a token-list parameter value.
    #[must_use]
    pub fn tok_param(&self, param: TokParam) -> TokenListId {
        self.tok_params.get(param.raw())
    }

    /// Sets a local token-list parameter value.
    pub fn set_tok_param(&mut self, param: TokParam, value: TokenListId) {
        self.tok_params.set(
            param.raw(),
            value,
            &mut self.journal,
            #[cfg(feature = "shadow")]
            &mut self.shadow,
            self.epoch,
            BankTag::TokParam,
            false,
        );
    }

    /// Sets a global token-list parameter value.
    pub fn set_tok_param_global(&mut self, param: TokParam, value: TokenListId) {
        self.tok_params.set(
            param.raw(),
            value,
            &mut self.journal,
            #[cfg(feature = "shadow")]
            &mut self.shadow,
            self.epoch,
            BankTag::TokParam,
            true,
        );
    }

    /// Restore-only raw write primitive for journal rollback and group walks.
    ///
    /// This deliberately bypasses the write barrier and does not append to the
    /// journal. It must only be used while replaying existing journal records;
    /// semantic assignments must go through the typed `set*` APIs so the
    /// single write path records history.
    #[allow(dead_code)]
    pub(crate) fn restore_raw(&mut self, cell: CellId, word: u64) {
        match cell.bank() {
            BankTag::Meaning => self.restore_meaning_word(cell.index(), word),
            BankTag::Count => self.restore_register(cell.index(), word, RegisterBank::Count),
            BankTag::Dimen => self.restore_register(cell.index(), word, RegisterBank::Dimen),
            BankTag::Skip => self.restore_register(cell.index(), word, RegisterBank::Skip),
            BankTag::Toks => self.restore_register(cell.index(), word, RegisterBank::Toks),
            BankTag::Box => self.restore_register(cell.index(), word, RegisterBank::Box),
            BankTag::IntParam => self.int_params.restore_word(u16_index(cell.index()), word),
            BankTag::DimenParam => self
                .dimen_params
                .restore_word(u16_index(cell.index()), word),
            BankTag::GlueParam => self.glue_params.restore_word(u16_index(cell.index()), word),
            BankTag::TokParam => self.tok_params.restore_word(u16_index(cell.index()), word),
        }
        #[cfg(feature = "shadow")]
        shadow_set(
            &mut self.shadow,
            CellId::new(cell.bank(), cell.index()),
            word,
        );
    }

    /// Verifies the shadow mirror against real environment storage.
    #[cfg(feature = "shadow")]
    pub fn verify_shadow(&self) {
        let real = self.non_default_words();
        for (cell, real_word) in &real {
            let shadow_word = self.shadow.get(cell).copied().unwrap_or(0);
            assert_eq!(
                shadow_word, *real_word,
                "shadow mismatch at {cell:?}: shadow={shadow_word} real={real_word}"
            );
        }
        for (&cell, &shadow_word) in &self.shadow {
            let real_word = real
                .iter()
                .find_map(|(real_cell, real_word)| (*real_cell == cell).then_some(*real_word))
                .unwrap_or(0);
            assert_eq!(
                shadow_word, real_word,
                "shadow mismatch at {cell:?}: shadow={shadow_word} real={real_word}"
            );
        }
    }

    /// Returns a content-only hash of all non-default environment cells.
    ///
    /// The hash intentionally excludes allocation lengths, capacities, and
    /// epoch stamps; replay identity is about semantic state.
    #[cfg(any(test, feature = "testing", feature = "shadow"))]
    #[must_use]
    pub fn testing_state_hash(&self) -> u64 {
        let mut pairs = self.non_default_words();
        pairs.sort_by_key(|(cell, _)| *cell);

        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for (cell, word) in pairs {
            cell.hash(&mut hasher);
            word.hash(&mut hasher);
        }
        hasher.finish()
    }

    #[cfg(any(test, feature = "testing", feature = "shadow"))]
    fn non_default_words(&self) -> Vec<(CellId, u64)> {
        let mut out = Vec::new();
        for (segment_index, segment) in self.meaning_cells.iter().enumerate() {
            for (offset, &word) in segment.iter().enumerate() {
                if word != 0 {
                    let index = ((segment_index as u32) << SEGMENT_BITS) | offset as u32;
                    out.push((CellId::new(BankTag::Meaning, index), word));
                }
            }
        }
        self.counts.non_default_words(BankTag::Count, &mut out);
        self.dimens.non_default_words(BankTag::Dimen, &mut out);
        self.skips.non_default_words(BankTag::Skip, &mut out);
        self.toks.non_default_words(BankTag::Toks, &mut out);
        self.boxes.non_default_words(BankTag::Box, &mut out);
        self.overflow_counts
            .non_default_words(BankTag::Count, &mut out);
        self.overflow_dimens
            .non_default_words(BankTag::Dimen, &mut out);
        self.overflow_skips
            .non_default_words(BankTag::Skip, &mut out);
        self.overflow_toks
            .non_default_words(BankTag::Toks, &mut out);
        self.overflow_boxes
            .non_default_words(BankTag::Box, &mut out);
        self.int_params
            .non_default_words(BankTag::IntParam, &mut out);
        self.dimen_params
            .non_default_words(BankTag::DimenParam, &mut out);
        self.glue_params
            .non_default_words(BankTag::GlueParam, &mut out);
        self.tok_params
            .non_default_words(BankTag::TokParam, &mut out);
        out
    }

    fn meaning_word(&self, index: u32) -> Option<u64> {
        let segment = segment_index(index);
        let offset = segment_offset(index);
        self.meaning_cells.get(segment).map(|cells| cells[offset])
    }

    fn set_meaning_word(&mut self, symbol: Symbol, word: u64, global: bool) {
        let index = symbol.raw();
        self.ensure_meaning_segment(index);
        let segment = segment_index(index);
        let offset = segment_offset(index);
        let cell = if global {
            CellId::new_global(BankTag::Meaning, index)
        } else {
            CellId::new(BankTag::Meaning, index)
        };

        barrier(
            &mut self.meaning_cells[segment][offset],
            &mut self.meaning_stamps[segment][offset],
            &mut self.journal,
            #[cfg(feature = "shadow")]
            &mut self.shadow,
            self.epoch,
            cell,
            word,
        );
    }

    fn ensure_meaning_segment(&mut self, index: u32) {
        let required_len = segment_index(index) + 1;
        while self.meaning_cells.len() < required_len {
            self.meaning_cells.push(Box::new([0; SEGMENT_LEN]));
            self.meaning_stamps
                .push(Box::new([Epoch::ZERO; SEGMENT_LEN]));
        }
    }

    #[allow(dead_code)]
    fn restore_meaning_word(&mut self, index: u32, word: u64) {
        self.ensure_meaning_segment(index);
        let segment = segment_index(index);
        let offset = segment_offset(index);
        self.meaning_cells[segment][offset] = word;
    }

    #[allow(dead_code)]
    fn restore_register(&mut self, index: u32, word: u64, bank: RegisterBank) {
        let index = register_index(index);
        if is_dense_register(index) {
            match bank {
                RegisterBank::Count => self.counts.restore_word(index, word),
                RegisterBank::Dimen => self.dimens.restore_word(index, word),
                RegisterBank::Skip => self.skips.restore_word(index, word),
                RegisterBank::Toks => self.toks.restore_word(index, word),
                RegisterBank::Box => self.boxes.restore_word(index, word),
            }
        } else {
            match bank {
                RegisterBank::Count => self.overflow_counts.restore_word(index, word),
                RegisterBank::Dimen => self.overflow_dimens.restore_word(index, word),
                RegisterBank::Skip => self.overflow_skips.restore_word(index, word),
                RegisterBank::Toks => self.overflow_toks.restore_word(index, word),
                RegisterBank::Box => self.overflow_boxes.restore_word(index, word),
            }
        }
    }
}

impl Default for Env {
    fn default() -> Self {
        Self::new()
    }
}

#[inline]
pub(crate) fn barrier(
    cell_slot: &mut u64,
    stamp_slot: &mut Epoch,
    journal: &mut Journal,
    #[cfg(feature = "shadow")] shadow: &mut HashMap<CellId, u64>,
    epoch: Epoch,
    cell_id: CellId,
    new_word: u64,
) {
    if *cell_slot == new_word {
        if cell_id.is_global() {
            journal.push_undo(UndoRec::new(cell_id, *cell_slot, new_word));
        }
        return;
    }

    if *stamp_slot < epoch {
        journal.push_undo(UndoRec::new(cell_id, *cell_slot, new_word));
        *stamp_slot = epoch;
    } else if cell_id.is_global() {
        journal.push_undo(UndoRec::new(cell_id, *cell_slot, new_word));
    }
    *cell_slot = new_word;
    #[cfg(feature = "shadow")]
    shadow_set(
        shadow,
        CellId::new(cell_id.bank(), cell_id.index()),
        new_word,
    );
}

#[cfg(feature = "shadow")]
fn shadow_set(shadow: &mut HashMap<CellId, u64>, cell: CellId, word: u64) {
    if word == 0 {
        shadow.remove(&cell);
    } else {
        shadow.insert(cell, word);
    }
}

fn segment_index(index: u32) -> usize {
    (index >> SEGMENT_BITS) as usize
}

fn segment_offset(index: u32) -> usize {
    (index & SEGMENT_MASK) as usize
}

#[derive(Clone, Copy, Debug)]
enum RegisterBank {
    Count,
    Dimen,
    Skip,
    Toks,
    Box,
}

fn is_dense_register(index: u16) -> bool {
    assert!(index < REGISTER_COUNT, "register index out of range");
    usize::from(index) < DENSE_REGISTER_COUNT
}

#[allow(dead_code)]
fn register_index(index: u32) -> u16 {
    match u16::try_from(index) {
        Ok(index) if index < REGISTER_COUNT => index,
        _ => panic!("register cell index out of range"),
    }
}

#[allow(dead_code)]
fn u16_index(index: u32) -> u16 {
    match u16::try_from(index) {
        Ok(index) => index,
        Err(_) => panic!("cell index exceeds u16 range"),
    }
}

fn checked_aftergroup_start(start: u32, len: usize) -> usize {
    let start = start as usize;
    assert!(start <= len, "aftergroup start is past the end");
    start
}

fn u32_len(value: usize, message: &str) -> u32 {
    match u32::try_from(value) {
        Ok(value) => value,
        Err(_) => panic!("{message}"),
    }
}

fn cell_key(cell: CellId) -> (BankTag, u32) {
    (cell.bank(), cell.index())
}

#[cfg(test)]
mod tests;
