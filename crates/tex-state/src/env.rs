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
pub(crate) mod box_bank;
pub(crate) mod group;
pub(crate) mod overflow;
pub(crate) mod raw;

use self::banks::{
    BankSetContext, BoxWriteOutcome, DENSE_REGISTER_COUNT, DimenParam, FixedBank, FontIdCodec,
    GlueIdCodec, GlueParam, I32Codec, IntParam, OptionalTokenListIdCodec, PARAMETER_COUNT,
    ScaledCodec, TokParam, TokenListIdCodec,
};
use self::box_bank::{BoxBank, BoxWriteContext};
use self::overflow::{REGISTER_COUNT, SparseBank};
use crate::cell::{BankTag, CellId};
use crate::epoch::Epoch;
use crate::glue::GlueSpecRef;
use crate::ids::{FontId, GlueId, MacroDefinitionId, NodeListId, TokenListId};
use crate::interner::Symbol;
use crate::journal::{Journal, UndoRec};
use crate::macro_store::MacroDefinitionRef;
use crate::math::{MATH_FAMILY_COUNT, MathFontSize};
use crate::meaning::Meaning;
use crate::node_arena::NodeListRef;
use crate::scaled::Scaled;
use crate::token::Token;
use crate::token_store::TokenListRef;
#[cfg(feature = "shadow")]
use ahash::AHashMap;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

const SEGMENT_BITS: u32 = 16;
const SEGMENT_LEN: usize = 1 << SEGMENT_BITS;
const SEGMENT_MASK: u32 = (SEGMENT_LEN as u32) - 1;
const FONT_DIMEN_BITS: u32 = 17;
const MATH_FAMILY_FONT_COUNT: usize = 3 * MATH_FAMILY_COUNT as usize;

type MeaningSegment = Box<[Meaning]>;
type StampSegment = Box<[Epoch; SEGMENT_LEN]>;

/// Canonical semantic outcome of one barriered environment-cell operation.
///
/// The cell identity never carries assignment scope. The disposition is
/// deliberately independent of journal activity: an equal assignment can
/// still save or trace while remaining semantically unchanged.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CellMutationReceipt {
    cell: CellId,
    old_word: u64,
    new_word: u64,
    // Stores may project a memory-root delta before dropping an old owner.
    main_memory_roots_updated: bool,
    disposition: CellMutationDisposition,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CellMutationDisposition {
    Changed,
    Unchanged,
    Retained,
}

/// Opaque environment-journal boundary owned by the aggregate state layer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct JournalRegionMark {
    journal_pos: crate::journal::JournalPos,
    lineage: u64,
}

/// Fixed-size cursor used only to decide whether one direct operation added
/// retireable environment history. Unlike a checkpoint, this owns no rollback
/// root and cannot restore state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DirectJournalMark {
    journal_pos: crate::journal::JournalPos,
    lineage: u64,
}

/// The journal lineage changed while a tracked region was active.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct JournalRegionInvalidated;

/// Shared ownership census for one live environment-journal baseline.
///
/// Snapshot registration is explicit rather than inferred from `Arc` strong
/// counts because generation forks clone `Env` without inheriting another
/// store owner's rollback capabilities.
#[derive(Debug, Default)]
struct JournalRollbackRoots {
    snapshots: AtomicUsize,
}

impl JournalRollbackRoots {
    fn register(self: &Arc<Self>) -> Arc<Self> {
        self.snapshots.fetch_add(1, Ordering::Relaxed);
        Arc::clone(self)
    }

    fn unregister(&self) {
        let previous = self.snapshots.fetch_sub(1, Ordering::Relaxed);
        assert_ne!(previous, 0, "environment snapshot root count underflowed");
    }

    fn is_only(&self, snapshots: usize) -> bool {
        self.snapshots.load(Ordering::Relaxed) == snapshots
    }
}

impl CellMutationReceipt {
    pub(crate) fn write(cell: CellId, old: u64, new: u64) -> Self {
        Self {
            cell: cell.without_assignment_scope(),
            old_word: old,
            new_word: new,
            main_memory_roots_updated: false,
            disposition: if old == new {
                CellMutationDisposition::Unchanged
            } else {
                CellMutationDisposition::Changed
            },
        }
    }

    pub(crate) fn restore(cell: CellId, old: u64, new: u64, retained: bool) -> Self {
        Self {
            cell: cell.without_assignment_scope(),
            old_word: old,
            new_word: new,
            main_memory_roots_updated: false,
            disposition: if old != new {
                CellMutationDisposition::Changed
            } else if retained {
                CellMutationDisposition::Retained
            } else {
                CellMutationDisposition::Unchanged
            },
        }
    }

    pub(crate) const fn cell(self) -> CellId {
        self.cell
    }

    pub(crate) const fn changed(self) -> bool {
        matches!(self.disposition, CellMutationDisposition::Changed)
    }

    pub(crate) const fn words(self) -> (u64, u64) {
        (self.old_word, self.new_word)
    }

    pub(crate) const fn with_main_memory_roots_updated(mut self) -> Self {
        self.main_memory_roots_updated = true;
        self
    }

    pub(crate) const fn main_memory_roots_updated(self) -> bool {
        self.main_memory_roots_updated
    }

    #[cfg(test)]
    pub(crate) const fn disposition(self) -> CellMutationDisposition {
        self.disposition
    }
}

#[derive(Clone, Copy, Debug)]
struct WordStamp {
    word: u64,
    stamp: Epoch,
}

impl Default for WordStamp {
    fn default() -> Self {
        Self {
            word: 0,
            stamp: Epoch::ZERO,
        }
    }
}

pub(crate) use group::EnvSnapshot;

/// One validated cell in the immutable environment installed by a format.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FormatBaseCell {
    pub(crate) cell: CellId,
    pub(crate) word: u64,
    pub(crate) token_root: Option<TokenListRef>,
    pub(crate) macro_root: Option<MacroDefinitionRef>,
    pub(crate) glue_root: Option<GlueSpecRef>,
    pub(crate) box_root: Option<NodeListRef>,
}

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

        pub(crate) fn $set(&mut self, index: u16, value: $value) -> CellMutationReceipt {
            if is_dense_register(index) {
                self.$dense.set(
                    index,
                    value,
                    BankSetContext {
                        journal: &mut self.journal,
                        #[cfg(feature = "shadow")]
                        shadow: &mut self.shadow,
                        epoch: self.epoch,
                        bank: BankTag::$bank,
                        global: false,
                    },
                )
            } else {
                self.$sparse.set(
                    index,
                    value,
                    BankSetContext {
                        journal: &mut self.journal,
                        #[cfg(feature = "shadow")]
                        shadow: &mut self.shadow,
                        epoch: self.epoch,
                        bank: BankTag::$bank,
                        global: false,
                    },
                )
            }
        }

        pub(crate) fn $set_global(&mut self, index: u16, value: $value) -> CellMutationReceipt {
            if is_dense_register(index) {
                self.$dense.set(
                    index,
                    value,
                    BankSetContext {
                        journal: &mut self.journal,
                        #[cfg(feature = "shadow")]
                        shadow: &mut self.shadow,
                        epoch: self.epoch,
                        bank: BankTag::$bank,
                        global: true,
                    },
                )
            } else {
                self.$sparse.set(
                    index,
                    value,
                    BankSetContext {
                        journal: &mut self.journal,
                        #[cfg(feature = "shadow")]
                        shadow: &mut self.shadow,
                        epoch: self.epoch,
                        bank: BankTag::$bank,
                        global: true,
                    },
                )
            }
        }
    };
}

/// TeX environment cells plus the journal that makes mutation replayable.
#[derive(Clone, Debug)]
pub struct Env {
    format_base: Arc<[FormatBaseCell]>,
    empty_token_root: Option<TokenListRef>,
    token_roots: BTreeMap<CellId, TokenListRef>,
    macro_roots: BTreeMap<CellId, MacroDefinitionRef>,
    glue_roots: BTreeMap<CellId, GlueSpecRef>,
    meaning_cells: Vec<Option<MeaningSegment>>,
    meaning_stamps: Vec<Option<StampSegment>>,
    counts: FixedBank<I32Codec, DENSE_REGISTER_COUNT>,
    dimens: FixedBank<ScaledCodec, DENSE_REGISTER_COUNT>,
    skips: FixedBank<GlueIdCodec, DENSE_REGISTER_COUNT>,
    toks: FixedBank<TokenListIdCodec, DENSE_REGISTER_COUNT>,
    boxes: BoxBank,
    muskips: FixedBank<GlueIdCodec, DENSE_REGISTER_COUNT>,
    overflow_counts: SparseBank<I32Codec>,
    overflow_dimens: SparseBank<ScaledCodec>,
    overflow_skips: SparseBank<GlueIdCodec>,
    overflow_toks: SparseBank<TokenListIdCodec>,
    overflow_muskips: SparseBank<GlueIdCodec>,
    int_params: FixedBank<I32Codec, PARAMETER_COUNT>,
    dimen_params: FixedBank<ScaledCodec, PARAMETER_COUNT>,
    glue_params: FixedBank<GlueIdCodec, PARAMETER_COUNT>,
    tok_params: FixedBank<OptionalTokenListIdCodec, PARAMETER_COUNT>,
    font_dimens: BTreeMap<u32, WordStamp>,
    font_param_lens: BTreeMap<u32, WordStamp>,
    font_hyphen_chars: BTreeMap<u32, WordStamp>,
    font_skew_chars: BTreeMap<u32, WordStamp>,
    pdf_lp_codes: BTreeMap<u32, WordStamp>,
    pdf_rp_codes: BTreeMap<u32, WordStamp>,
    pdf_ef_codes: BTreeMap<u32, WordStamp>,
    pdf_tag_codes: BTreeMap<u32, WordStamp>,
    pdf_knbs_codes: BTreeMap<u32, WordStamp>,
    pdf_stbs_codes: BTreeMap<u32, WordStamp>,
    pdf_shbs_codes: BTreeMap<u32, WordStamp>,
    pdf_knbc_codes: BTreeMap<u32, WordStamp>,
    pdf_knac_codes: BTreeMap<u32, WordStamp>,
    pdf_no_ligatures: BTreeMap<u32, WordStamp>,
    current_font: WordStamp,
    math_family_fonts: FixedBank<FontIdCodec, MATH_FAMILY_FONT_COUNT>,
    journal: Journal,
    journal_rollback_roots: Arc<JournalRollbackRoots>,
    journal_baseline_serial: u64,
    group_boundaries: Vec<group::GroupBoundary>,
    aftergroup: Vec<crate::token::RootedTracedTokenWord>,
    afterassignment: Option<Token>,
    group_depth: u32,
    next_group_lineage: u64,
    /// Monotonic identity of the uncompacted dependency-journal timeline.
    /// Group entry changes write epochs but does not invalidate a region;
    /// checkpoint capture, group exit, and rollback do.
    journal_lineage: u64,
    epoch: Epoch,
    #[cfg(feature = "shadow")]
    shadow: AHashMap<CellId, u64>,
}

impl Env {
    /// Creates an empty environment in the first session epoch.
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            format_base: Arc::from([]),
            empty_token_root: None,
            token_roots: BTreeMap::new(),
            macro_roots: BTreeMap::new(),
            glue_roots: BTreeMap::new(),
            meaning_cells: Vec::new(),
            meaning_stamps: Vec::new(),
            counts: FixedBank::new(),
            dimens: FixedBank::new(),
            skips: FixedBank::new(),
            toks: FixedBank::new(),
            boxes: BoxBank::new(),
            muskips: FixedBank::new(),
            overflow_counts: SparseBank::new(),
            overflow_dimens: SparseBank::new(),
            overflow_skips: SparseBank::new(),
            overflow_toks: SparseBank::new(),
            overflow_muskips: SparseBank::new(),
            int_params: FixedBank::new(),
            dimen_params: FixedBank::new(),
            glue_params: FixedBank::new(),
            tok_params: FixedBank::new(),
            font_dimens: BTreeMap::new(),
            font_param_lens: BTreeMap::new(),
            font_hyphen_chars: BTreeMap::new(),
            font_skew_chars: BTreeMap::new(),
            pdf_lp_codes: BTreeMap::new(),
            pdf_rp_codes: BTreeMap::new(),
            pdf_ef_codes: BTreeMap::new(),
            pdf_tag_codes: BTreeMap::new(),
            pdf_knbs_codes: BTreeMap::new(),
            pdf_stbs_codes: BTreeMap::new(),
            pdf_shbs_codes: BTreeMap::new(),
            pdf_knbc_codes: BTreeMap::new(),
            pdf_knac_codes: BTreeMap::new(),
            pdf_no_ligatures: BTreeMap::new(),
            current_font: WordStamp::default(),
            math_family_fonts: FixedBank::new(),
            journal: Journal::new(),
            journal_rollback_roots: Arc::new(JournalRollbackRoots::default()),
            journal_baseline_serial: 1,
            group_boundaries: Vec::new(),
            aftergroup: Vec::new(),
            afterassignment: None,
            group_depth: 0,
            next_group_lineage: 1,
            journal_lineage: 1,
            epoch: Epoch::START,
            #[cfg(feature = "shadow")]
            shadow: AHashMap::new(),
        }
    }

    /// Installs the canonical empty token owner shared by default token cells.
    pub(crate) fn install_empty_token_root(&mut self, root: TokenListRef) {
        assert_eq!(root.id(), TokenListId::EMPTY);
        if let Some(existing) = &self.empty_token_root {
            assert_eq!(existing.id(), TokenListId::EMPTY);
        }
        self.empty_token_root = Some(root);
    }

    /// Installs a validated immutable format base into a fresh environment.
    ///
    /// The ordinary banks are the mutable job overlay. Seeding them here does
    /// not create assignment history; later writes use the normal barrier and
    /// can therefore restore these base values through groups and snapshots.
    pub(crate) fn install_format_base(&mut self, cells: Vec<FormatBaseCell>) {
        debug_assert_eq!(self.group_depth, 0);
        debug_assert!(self.format_base.is_empty());
        for entry in &cells {
            self.restore_raw_with_owners(
                entry.cell,
                entry.word,
                entry.token_root,
                entry.macro_root,
                entry.glue_root,
                entry.box_root.clone(),
            );
        }
        self.format_base = cells.into();
    }

    #[cfg(test)]
    pub(crate) fn testing_format_base(&self) -> &[FormatBaseCell] {
        &self.format_base
    }

    /// Returns the current epoch.
    #[must_use]
    pub const fn epoch(&self) -> Epoch {
        self.epoch
    }

    /// Advances to the next epoch.
    pub(crate) fn bump_epoch(&mut self) {
        self.epoch.bump();
    }

    /// Returns the current journal end position.
    #[must_use]
    pub(crate) fn journal_pos(&self) -> crate::journal::JournalPos {
        self.journal.pos()
    }

    /// Returns the meaning for `symbol`, defaulting to `Undefined`.
    #[must_use]
    pub fn get(&self, symbol: Symbol) -> Meaning {
        self.get_meaning_slot(symbol.raw())
    }

    /// Returns the TeX assignment level encoded by the live save stack.
    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn testing_meaning_level(&self, symbol: Symbol) -> u32 {
        let mut depth = 0_u32;
        let mut level = 1_u32;
        for index in 0..self.journal.len() {
            match self.journal.entry(index) {
                crate::journal::Entry::Marker(crate::journal::Marker::Group { .. }) => {
                    depth = depth.checked_add(1).expect("group depth exceeds u32");
                }
                crate::journal::Entry::Undo(rec)
                    if rec.cell().bank() == BankTag::Meaning
                        && rec.cell().index() == symbol.raw() =>
                {
                    level = if rec.cell().is_global() {
                        1
                    } else {
                        depth.checked_add(1).expect("meaning level exceeds u32")
                    };
                }
                crate::journal::Entry::Undo(_)
                | crate::journal::Entry::BoxUndo(_)
                | crate::journal::Entry::Marker(crate::journal::Marker::Aftergroup)
                | crate::journal::Entry::Marker(crate::journal::Marker::Checkpoint(_)) => {}
            }
        }
        level
    }

    /// Returns the meaning at a dense interner slot.
    #[must_use]
    pub(crate) fn get_meaning_slot(&self, slot: u32) -> Meaning {
        self.meaning_value(slot).unwrap_or(Meaning::Undefined)
    }

    /// Sets the local meaning for a symbol validated by the owning store.
    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn set(&mut self, symbol: Symbol, meaning: Meaning) {
        self.set_meaning_slot(symbol.raw(), meaning, false);
    }

    /// Sets the global meaning for a symbol validated by the owning store.
    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn set_global(&mut self, symbol: Symbol, meaning: Meaning) {
        self.set_meaning_slot(symbol.raw(), meaning, true);
    }

    /// Sets a meaning by dense interner slot after aggregate validation.
    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn set_meaning_slot(
        &mut self,
        slot: u32,
        meaning: Meaning,
        global: bool,
    ) -> CellMutationReceipt {
        self.set_meaning_slot_with_macro_root(slot, meaning, None, global)
    }

    pub(crate) fn set_meaning_slot_with_macro_root(
        &mut self,
        slot: u32,
        meaning: Meaning,
        macro_root: Option<MacroDefinitionRef>,
        global: bool,
    ) -> CellMutationReceipt {
        let cell = CellId::new(BankTag::Meaning, slot);
        assert_eq!(
            macro_root.as_ref().map(MacroDefinitionRef::raw),
            match meaning {
                Meaning::Macro { definition, .. } => Some(definition.raw()),
                _ => None,
            },
            "meaning word and macro owner diverged"
        );
        let old_root = self.macro_root(cell);
        let mark = self.journal.pos();
        let receipt = self.set_meaning_value(slot, meaning, global);
        if self.journal.pos() != mark {
            self.journal.attach_macro_undo_roots(
                mark,
                crate::journal::MacroUndoRoots::new(old_root, macro_root),
            );
        }
        self.set_macro_root(cell, macro_root);
        receipt
    }

    /// Test-only local meaning write for isolated `Env` barrier coverage.
    #[cfg(any(test, feature = "testing"))]
    pub fn testing_set_meaning(&mut self, symbol: Symbol, meaning: Meaning) {
        self.set(symbol, meaning);
    }

    /// Test-only global meaning write for isolated `Env` barrier coverage.
    #[cfg(any(test, feature = "testing"))]
    pub fn testing_set_meaning_global(&mut self, symbol: Symbol, meaning: Meaning) {
        self.set_global(symbol, meaning);
    }

    #[must_use]
    pub fn count(&self, index: u16) -> i32 {
        if is_dense_register(index) {
            self.counts.get(index)
        } else {
            self.overflow_counts.get(index)
        }
    }

    pub(crate) fn set_count(&mut self, index: u16, value: i32) -> CellMutationReceipt {
        self.set_count_with_scope(index, value, false)
    }

    pub(crate) fn set_count_global(&mut self, index: u16, value: i32) -> CellMutationReceipt {
        self.set_count_with_scope(index, value, true)
    }

    fn set_count_with_scope(
        &mut self,
        index: u16,
        value: i32,
        global: bool,
    ) -> CellMutationReceipt {
        let context = BankSetContext {
            journal: &mut self.journal,
            #[cfg(feature = "shadow")]
            shadow: &mut self.shadow,
            epoch: self.epoch,
            bank: BankTag::Count,
            global,
        };
        if is_dense_register(index) {
            self.counts.set(index, value, context)
        } else {
            self.overflow_counts.set(index, value, context)
        }
    }
    register_accessors!(
        dimen,
        set_dimen,
        set_dimen_global,
        Scaled,
        Dimen,
        dimens,
        overflow_dimens
    );
    #[must_use]
    pub fn skip(&self, index: u16) -> GlueId {
        self.glue_register(index, RegisterBank::Skip)
    }

    pub(crate) fn set_skip(&mut self, index: u16, root: GlueSpecRef) -> CellMutationReceipt {
        self.set_glue_register(index, root, RegisterBank::Skip, false)
    }

    pub(crate) fn set_skip_global(&mut self, index: u16, root: GlueSpecRef) -> CellMutationReceipt {
        self.set_glue_register(index, root, RegisterBank::Skip, true)
    }

    #[must_use]
    pub fn muskip(&self, index: u16) -> GlueId {
        self.glue_register(index, RegisterBank::Muskip)
    }

    pub(crate) fn set_muskip(&mut self, index: u16, root: GlueSpecRef) -> CellMutationReceipt {
        self.set_glue_register(index, root, RegisterBank::Muskip, false)
    }

    pub(crate) fn set_muskip_global(
        &mut self,
        index: u16,
        root: GlueSpecRef,
    ) -> CellMutationReceipt {
        self.set_glue_register(index, root, RegisterBank::Muskip, true)
    }

    fn glue_register(&self, index: u16, bank: RegisterBank) -> GlueId {
        let dense = match bank {
            RegisterBank::Skip => &self.skips,
            RegisterBank::Muskip => &self.muskips,
            _ => unreachable!("non-glue register bank"),
        };
        let sparse = match bank {
            RegisterBank::Skip => &self.overflow_skips,
            RegisterBank::Muskip => &self.overflow_muskips,
            _ => unreachable!("non-glue register bank"),
        };
        if is_dense_register(index) {
            dense.get(index)
        } else {
            sparse.get(index)
        }
    }

    fn set_glue_register(
        &mut self,
        index: u16,
        root: GlueSpecRef,
        bank: RegisterBank,
        global: bool,
    ) -> CellMutationReceipt {
        let tag = match bank {
            RegisterBank::Skip => BankTag::Skip,
            RegisterBank::Muskip => BankTag::Muskip,
            _ => unreachable!("non-glue register bank"),
        };
        let cell = CellId::new(tag, u32::from(index));
        let old_root = self.glue_root(cell);
        let mark = self.journal.pos();
        let value = root.id();
        let context = BankSetContext {
            journal: &mut self.journal,
            #[cfg(feature = "shadow")]
            shadow: &mut self.shadow,
            epoch: self.epoch,
            bank: tag,
            global,
        };
        let receipt = if is_dense_register(index) {
            match bank {
                RegisterBank::Skip => self.skips.set(index, value, context),
                RegisterBank::Muskip => self.muskips.set(index, value, context),
                _ => unreachable!(),
            }
        } else {
            match bank {
                RegisterBank::Skip => self.overflow_skips.set(index, value, context),
                RegisterBank::Muskip => self.overflow_muskips.set(index, value, context),
                _ => unreachable!(),
            }
        };
        self.finish_glue_write(mark, cell, old_root, Some(root));
        receipt
    }
    /// Returns a token register coordinate.
    #[must_use]
    pub fn toks(&self, index: u16) -> TokenListId {
        if is_dense_register(index) {
            self.toks.get(index)
        } else {
            self.overflow_toks.get(index)
        }
    }

    pub(crate) fn set_toks(&mut self, index: u16, root: TokenListRef) -> CellMutationReceipt {
        self.set_toks_with_scope(index, root, false)
    }

    pub(crate) fn set_toks_global(
        &mut self,
        index: u16,
        root: TokenListRef,
    ) -> CellMutationReceipt {
        self.set_toks_with_scope(index, root, true)
    }

    fn set_toks_with_scope(
        &mut self,
        index: u16,
        root: TokenListRef,
        global: bool,
    ) -> CellMutationReceipt {
        let cell = CellId::new(BankTag::Toks, u32::from(index));
        let old_root = self.token_root(cell);
        let mark = self.journal.pos();
        let value = root.id();
        let context = BankSetContext {
            journal: &mut self.journal,
            #[cfg(feature = "shadow")]
            shadow: &mut self.shadow,
            epoch: self.epoch,
            bank: BankTag::Toks,
            global,
        };
        let receipt = if is_dense_register(index) {
            self.toks.set(index, value, context)
        } else {
            self.overflow_toks.set(index, value, context)
        };
        self.finish_token_write(mark, cell, old_root, Some(root));
        receipt
    }
    /// Returns a box register value; `None` is TeX's void box.
    #[must_use]
    pub fn box_reg(&self, index: u16) -> Option<NodeListId> {
        NodeListId::decode_box_word(self.boxes.get(index).value())
    }

    /// Clones the structural owner of one nonvoid box register.
    #[must_use]
    pub(crate) fn box_reg_ref(&self, index: u16) -> Option<NodeListRef> {
        self.boxes.get(index).root()
    }

    /// Sets a local box register value validated by the owning store.
    pub(crate) fn set_box_reg(
        &mut self,
        index: u16,
        value: Option<NodeListRef>,
    ) -> (CellMutationReceipt, BoxWriteOutcome) {
        self.set_box_reg_local(index, value, true)
    }

    fn set_box_reg_local(
        &mut self,
        index: u16,
        value: Option<NodeListRef>,
        coalesce: bool,
    ) -> (CellMutationReceipt, BoxWriteOutcome) {
        let old_word = self.boxes.get(index).value();
        let new_word = NodeListId::encode_box_word(value.as_ref().map(NodeListRef::id));
        let outcome = self.boxes.write(
            index,
            value,
            BoxWriteContext {
                global: false,
                coalesce,
                journal: &mut self.journal,
                epoch: self.epoch,
                group_depth: self.group_depth,
            },
        );
        #[cfg(feature = "shadow")]
        shadow_set(
            &mut self.shadow,
            CellId::new(BankTag::Box, u32::from(index)),
            new_word,
        );
        (
            CellMutationReceipt::write(
                CellId::new(BankTag::Box, u32::from(index)),
                old_word,
                new_word,
            ),
            outcome,
        )
    }

    /// Sets a global box register value validated by the owning store.
    pub(crate) fn set_box_reg_global(
        &mut self,
        index: u16,
        value: Option<NodeListRef>,
    ) -> (CellMutationReceipt, BoxWriteOutcome) {
        let old_word = self.boxes.get(index).value();
        let new_word = NodeListId::encode_box_word(value.as_ref().map(NodeListRef::id));
        let outcome = self.boxes.write(
            index,
            value,
            BoxWriteContext {
                global: true,
                coalesce: false,
                journal: &mut self.journal,
                epoch: self.epoch,
                group_depth: self.group_depth,
            },
        );
        #[cfg(feature = "shadow")]
        shadow_set(
            &mut self.shadow,
            CellId::new(BankTag::Box, u32::from(index)),
            new_word,
        );
        (
            CellMutationReceipt::write(
                CellId::new(BankTag::Box, u32::from(index)),
                old_word,
                new_word,
            ),
            outcome,
        )
    }

    /// Sets a box register at TeX's current box level.
    pub(crate) fn set_box_reg_same_level(
        &mut self,
        index: u16,
        value: Option<NodeListRef>,
    ) -> (CellMutationReceipt, BoxWriteOutcome) {
        let owner_depth = self.boxes.get(index).owner_depth();
        if owner_depth == 0 {
            return self.set_box_reg_global(index, value);
        }
        let old_word = self.boxes.get(index).value();
        let new_word = NodeListId::encode_box_word(value.as_ref().map(NodeListRef::id));
        let outcome = self.boxes.write_same_level(index, value, &mut self.journal);
        #[cfg(feature = "shadow")]
        shadow_set(
            &mut self.shadow,
            CellId::new(BankTag::Box, u32::from(index)),
            new_word,
        );
        (
            CellMutationReceipt::write(
                CellId::new(BankTag::Box, u32::from(index)),
                old_word,
                new_word,
            ),
            outcome,
        )
    }

    /// Takes a box register at TeX's current box level.
    ///
    /// This matches `\box<n>`: if the visible box value was locally assigned
    /// in the current group, the voiding is local to that group; otherwise it
    /// must survive the current group while remaining rollback-visible.
    pub(crate) fn take_box_reg_same_level(
        &mut self,
        index: u16,
    ) -> (Option<NodeListRef>, CellMutationReceipt, BoxWriteOutcome) {
        let old = self.box_reg_ref(index);
        let owner_depth = self.boxes.get(index).owner_depth();
        let (receipt, rec) = if owner_depth == 0 {
            self.set_box_reg_global(index, None)
        } else {
            self.set_box_reg_same_level(index, None)
        };
        (old, receipt, rec)
    }

    /// Takes a local box while retaining its returned handle in a distinct
    /// undo record until the caller has consumed it.
    pub(crate) fn take_box_reg(
        &mut self,
        index: u16,
    ) -> (Option<NodeListRef>, CellMutationReceipt, BoxWriteOutcome) {
        let old = self.box_reg_ref(index);
        let (receipt, outcome) = self.set_box_reg_local(index, None, false);
        (old, receipt, outcome)
    }

    /// Returns an integer parameter value.
    #[must_use]
    pub fn int_param(&self, param: IntParam) -> i32 {
        self.int_params.get(param.raw())
    }

    /// Sets a local integer parameter value.
    pub(crate) fn set_int_param(&mut self, param: IntParam, value: i32) -> CellMutationReceipt {
        self.int_params.set(
            param.raw(),
            value,
            BankSetContext {
                journal: &mut self.journal,
                #[cfg(feature = "shadow")]
                shadow: &mut self.shadow,
                epoch: self.epoch,
                bank: BankTag::IntParam,
                global: false,
            },
        )
    }

    /// Sets a global integer parameter value.
    pub(crate) fn set_int_param_global(
        &mut self,
        param: IntParam,
        value: i32,
    ) -> CellMutationReceipt {
        self.int_params.set(
            param.raw(),
            value,
            BankSetContext {
                journal: &mut self.journal,
                #[cfg(feature = "shadow")]
                shadow: &mut self.shadow,
                epoch: self.epoch,
                bank: BankTag::IntParam,
                global: true,
            },
        )
    }

    /// Returns a dimension parameter value.
    #[must_use]
    pub fn dimen_param(&self, param: DimenParam) -> Scaled {
        self.dimen_params.get(param.raw())
    }

    /// Sets a local dimension parameter value.
    pub(crate) fn set_dimen_param(
        &mut self,
        param: DimenParam,
        value: Scaled,
    ) -> CellMutationReceipt {
        self.dimen_params.set(
            param.raw(),
            value,
            BankSetContext {
                journal: &mut self.journal,
                #[cfg(feature = "shadow")]
                shadow: &mut self.shadow,
                epoch: self.epoch,
                bank: BankTag::DimenParam,
                global: false,
            },
        )
    }

    /// Sets a global dimension parameter value.
    pub(crate) fn set_dimen_param_global(
        &mut self,
        param: DimenParam,
        value: Scaled,
    ) -> CellMutationReceipt {
        self.dimen_params.set(
            param.raw(),
            value,
            BankSetContext {
                journal: &mut self.journal,
                #[cfg(feature = "shadow")]
                shadow: &mut self.shadow,
                epoch: self.epoch,
                bank: BankTag::DimenParam,
                global: true,
            },
        )
    }

    /// Returns a glue parameter value.
    #[must_use]
    pub fn glue_param(&self, param: GlueParam) -> GlueId {
        self.glue_params.get(param.raw())
    }

    /// Sets a local glue parameter value.
    pub(crate) fn set_glue_param(
        &mut self,
        param: GlueParam,
        root: GlueSpecRef,
    ) -> CellMutationReceipt {
        self.set_glue_param_with_scope(param, root, false)
    }

    fn set_glue_param_with_scope(
        &mut self,
        param: GlueParam,
        root: GlueSpecRef,
        global: bool,
    ) -> CellMutationReceipt {
        let cell = CellId::new(BankTag::GlueParam, u32::from(param.raw()));
        let old_root = self.glue_root(cell);
        let mark = self.journal.pos();
        let receipt = self.glue_params.set(
            param.raw(),
            root.id(),
            BankSetContext {
                journal: &mut self.journal,
                #[cfg(feature = "shadow")]
                shadow: &mut self.shadow,
                epoch: self.epoch,
                bank: BankTag::GlueParam,
                global,
            },
        );
        self.finish_glue_write(mark, cell, old_root, Some(root));
        receipt
    }

    /// Sets a global glue parameter value.
    pub(crate) fn set_glue_param_global(
        &mut self,
        param: GlueParam,
        root: GlueSpecRef,
    ) -> CellMutationReceipt {
        self.set_glue_param_with_scope(param, root, true)
    }

    /// Returns a token-list parameter value.
    #[must_use]
    pub fn tok_param(&self, param: TokParam) -> TokenListId {
        self.tok_param_option(param).unwrap_or(TokenListId::EMPTY)
    }

    /// Returns a token-list parameter while preserving tex.web's null pointer.
    #[must_use]
    pub fn tok_param_option(&self, param: TokParam) -> Option<TokenListId> {
        self.tok_params.get(param.raw())
    }

    /// Sets a local token-list parameter, preserving TeX's null pointer.
    pub(crate) fn set_tok_param_option(
        &mut self,
        param: TokParam,
        value: Option<TokenListRef>,
    ) -> CellMutationReceipt {
        self.set_tok_param_option_with_scope(param, value, false)
    }

    /// Sets a global token-list parameter, preserving TeX's null pointer.
    pub(crate) fn set_tok_param_option_global(
        &mut self,
        param: TokParam,
        value: Option<TokenListRef>,
    ) -> CellMutationReceipt {
        self.set_tok_param_option_with_scope(param, value, true)
    }

    fn set_tok_param_option_with_scope(
        &mut self,
        param: TokParam,
        root: Option<TokenListRef>,
        global: bool,
    ) -> CellMutationReceipt {
        let cell = CellId::new(BankTag::TokParam, u32::from(param.raw()));
        let old_root = self.token_root(cell);
        let mark = self.journal.pos();
        let value = root.as_ref().map(TokenListRef::id);
        let receipt = self.tok_params.set(
            param.raw(),
            value,
            BankSetContext {
                journal: &mut self.journal,
                #[cfg(feature = "shadow")]
                shadow: &mut self.shadow,
                epoch: self.epoch,
                bank: BankTag::TokParam,
                global,
            },
        );
        self.finish_token_write(mark, cell, old_root, root);
        receipt
    }

    fn finish_token_write(
        &mut self,
        mark: crate::journal::JournalPos,
        cell: CellId,
        old_root: Option<TokenListRef>,
        new_root: Option<TokenListRef>,
    ) {
        if self.journal.pos() != mark {
            self.journal.attach_token_undo_roots(
                mark,
                crate::journal::TokenUndoRoots::new(old_root, new_root),
            );
        }
        self.set_token_root(cell, new_root);
    }

    fn finish_glue_write(
        &mut self,
        mark: crate::journal::JournalPos,
        cell: CellId,
        old_root: Option<GlueSpecRef>,
        new_root: Option<GlueSpecRef>,
    ) {
        if self.journal.pos() != mark {
            self.journal.attach_glue_undo_roots(
                mark,
                crate::journal::GlueUndoRoots::new(old_root, new_root),
            );
        }
        self.set_glue_root(cell, new_root);
    }

    pub(crate) fn glue_root(&self, cell: CellId) -> Option<GlueSpecRef> {
        let cell = cell.without_assignment_scope();
        debug_assert!(matches!(
            cell.bank(),
            BankTag::Skip | BankTag::Muskip | BankTag::GlueParam
        ));
        self.glue_roots.get(&cell).cloned()
    }

    pub(crate) fn set_glue_root(&mut self, cell: CellId, root: Option<GlueSpecRef>) {
        let cell = cell.without_assignment_scope();
        debug_assert!(matches!(
            cell.bank(),
            BankTag::Skip | BankTag::Muskip | BankTag::GlueParam
        ));
        match root {
            Some(root) if root.id() != GlueId::ZERO => {
                self.glue_roots.insert(cell, root);
            }
            Some(root) => {
                assert_eq!(root.id(), GlueId::ZERO);
                self.glue_roots.remove(&cell);
            }
            None => {
                self.glue_roots.remove(&cell);
            }
        }
    }

    pub(crate) fn token_root(&self, cell: CellId) -> Option<TokenListRef> {
        let cell = cell.without_assignment_scope();
        debug_assert!(matches!(cell.bank(), BankTag::Toks | BankTag::TokParam));
        self.token_roots.get(&cell).cloned().or_else(|| {
            let word = self.semantic_word(cell);
            let is_empty = match cell.bank() {
                BankTag::Toks => word == u64::from(TokenListId::EMPTY.raw()),
                BankTag::TokParam => word == u64::from(TokenListId::EMPTY.raw()) + 1,
                _ => false,
            };
            is_empty.then_some(self.empty_token_root).flatten()
        })
    }

    pub(crate) fn set_token_root(&mut self, cell: CellId, root: Option<TokenListRef>) {
        let cell = cell.without_assignment_scope();
        debug_assert!(matches!(cell.bank(), BankTag::Toks | BankTag::TokParam));
        match root {
            Some(root) if root.id() != TokenListId::EMPTY => {
                self.token_roots.insert(cell, root);
            }
            Some(root) => {
                assert_eq!(root.id(), TokenListId::EMPTY);
                self.token_roots.remove(&cell);
            }
            None => {
                self.token_roots.remove(&cell);
            }
        }
    }

    pub(crate) fn macro_root(&self, cell: CellId) -> Option<MacroDefinitionRef> {
        let cell = cell.without_assignment_scope();
        debug_assert_eq!(cell.bank(), BankTag::Meaning);
        self.macro_roots.get(&cell).cloned()
    }

    pub(crate) fn macro_root_id(&self, cell: CellId) -> Option<MacroDefinitionId> {
        let cell = cell.without_assignment_scope();
        debug_assert_eq!(cell.bank(), BankTag::Meaning);
        self.macro_roots.get(&cell).map(MacroDefinitionRef::id)
    }

    pub(crate) fn set_macro_root(&mut self, cell: CellId, root: Option<MacroDefinitionRef>) {
        let cell = cell.without_assignment_scope();
        debug_assert_eq!(cell.bank(), BankTag::Meaning);
        match root {
            Some(root) => {
                self.macro_roots.insert(cell, root);
            }
            None => {
                self.macro_roots.remove(&cell);
            }
        }
    }

    #[must_use]
    pub fn current_font(&self) -> FontId {
        FontId::new(self.current_font.word as u32)
    }

    #[must_use]
    pub fn current_font_symbol(&self) -> Option<Symbol> {
        let raw = self.current_font.word >> 32;
        if raw == 0 {
            None
        } else {
            Some(Symbol::new((raw - 1) as u32))
        }
    }

    pub(crate) fn set_current_font(&mut self, value: FontId) -> CellMutationReceipt {
        self.set_current_font_word(pack_current_font(self.current_font_symbol(), value), false)
    }

    pub(crate) fn set_current_font_global(&mut self, value: FontId) -> CellMutationReceipt {
        self.set_current_font_word(pack_current_font(self.current_font_symbol(), value), true)
    }

    pub(crate) fn set_current_font_selector(
        &mut self,
        symbol: Symbol,
        value: FontId,
    ) -> CellMutationReceipt {
        self.set_current_font_word(pack_current_font(Some(symbol), value), false)
    }

    pub(crate) fn set_current_font_selector_global(
        &mut self,
        symbol: Symbol,
        value: FontId,
    ) -> CellMutationReceipt {
        self.set_current_font_word(pack_current_font(Some(symbol), value), true)
    }

    /// Returns the font selected for a math family and size.
    #[must_use]
    pub fn math_family_font(&self, size: MathFontSize, family: u8) -> FontId {
        self.math_family_fonts
            .get(math_family_font_index(size, family))
    }

    /// Sets a local math-family font selector.
    pub(crate) fn set_math_family_font(
        &mut self,
        size: MathFontSize,
        family: u8,
        value: FontId,
    ) -> CellMutationReceipt {
        self.math_family_fonts.set(
            math_family_font_index(size, family),
            value,
            BankSetContext {
                journal: &mut self.journal,
                #[cfg(feature = "shadow")]
                shadow: &mut self.shadow,
                epoch: self.epoch,
                bank: BankTag::MathFamilyFont,
                global: false,
            },
        )
    }

    /// Sets a global math-family font selector.
    pub(crate) fn set_math_family_font_global(
        &mut self,
        size: MathFontSize,
        family: u8,
        value: FontId,
    ) -> CellMutationReceipt {
        self.math_family_fonts.set(
            math_family_font_index(size, family),
            value,
            BankSetContext {
                journal: &mut self.journal,
                #[cfg(feature = "shadow")]
                shadow: &mut self.shadow,
                epoch: self.epoch,
                bank: BankTag::MathFamilyFont,
                global: true,
            },
        )
    }

    #[must_use]
    pub fn font_dimen(&self, font: FontId, number: u32) -> Scaled {
        let Ok(index) = font_dimen_index(font, number) else {
            return Scaled::from_raw(0);
        };
        Scaled::from_raw(decode_i32(font_bank_word(&self.font_dimens, index)))
    }

    pub(crate) fn set_font_dimen_global(
        &mut self,
        index: u32,
        value: Scaled,
    ) -> CellMutationReceipt {
        set_font_bank_word(
            &mut self.font_dimens,
            &mut self.journal,
            #[cfg(feature = "shadow")]
            &mut self.shadow,
            self.epoch,
            BankTag::FontDimen,
            index,
            encode_i32(value.raw()),
            true,
        )
    }

    #[must_use]
    pub fn font_param_len(&self, font: FontId) -> u32 {
        decode_u32(font_bank_word(&self.font_param_lens, font.raw()))
    }

    pub(crate) fn set_font_param_len_global(
        &mut self,
        font: FontId,
        value: u32,
    ) -> CellMutationReceipt {
        set_font_bank_word(
            &mut self.font_param_lens,
            &mut self.journal,
            #[cfg(feature = "shadow")]
            &mut self.shadow,
            self.epoch,
            BankTag::FontParamLen,
            font.raw(),
            u64::from(value),
            true,
        )
    }

    #[must_use]
    pub fn font_hyphen_char(&self, font: FontId) -> i32 {
        decode_i32(font_bank_word(&self.font_hyphen_chars, font.raw()))
    }

    pub(crate) fn set_font_hyphen_char_global(
        &mut self,
        font: FontId,
        value: i32,
    ) -> CellMutationReceipt {
        self.set_font_int_bank(BankTag::FontHyphenChar, font, value, true)
    }

    #[must_use]
    pub fn font_skew_char(&self, font: FontId) -> i32 {
        decode_i32(font_bank_word(&self.font_skew_chars, font.raw()))
    }

    pub(crate) fn set_font_skew_char_global(
        &mut self,
        font: FontId,
        value: i32,
    ) -> CellMutationReceipt {
        self.set_font_int_bank(BankTag::FontSkewChar, font, value, true)
    }

    pub(crate) fn pdf_font_code(&self, bank: BankTag, font: FontId, code: u8) -> Option<i32> {
        let index = (font.raw() << 8) | u32::from(code);
        let map = match bank {
            BankTag::PdfLpCode => &self.pdf_lp_codes,
            BankTag::PdfRpCode => &self.pdf_rp_codes,
            BankTag::PdfEfCode => &self.pdf_ef_codes,
            BankTag::PdfTagCode => &self.pdf_tag_codes,
            BankTag::PdfKnbsCode => &self.pdf_knbs_codes,
            BankTag::PdfStbsCode => &self.pdf_stbs_codes,
            BankTag::PdfShbsCode => &self.pdf_shbs_codes,
            BankTag::PdfKnbcCode => &self.pdf_knbc_codes,
            BankTag::PdfKnacCode => &self.pdf_knac_codes,
            _ => unreachable!("caller restricts pdfTeX font-code banks"),
        };
        map.get(&index).map(|entry| decode_i32(entry.word))
    }

    pub(crate) fn set_pdf_font_code_global(
        &mut self,
        bank: BankTag,
        font: FontId,
        code: u8,
        value: i32,
    ) -> CellMutationReceipt {
        let index = (font.raw() << 8) | u32::from(code);
        let map = match bank {
            BankTag::PdfLpCode => &mut self.pdf_lp_codes,
            BankTag::PdfRpCode => &mut self.pdf_rp_codes,
            BankTag::PdfEfCode => &mut self.pdf_ef_codes,
            BankTag::PdfTagCode => &mut self.pdf_tag_codes,
            BankTag::PdfKnbsCode => &mut self.pdf_knbs_codes,
            BankTag::PdfStbsCode => &mut self.pdf_stbs_codes,
            BankTag::PdfShbsCode => &mut self.pdf_shbs_codes,
            BankTag::PdfKnbcCode => &mut self.pdf_knbc_codes,
            BankTag::PdfKnacCode => &mut self.pdf_knac_codes,
            _ => unreachable!("caller restricts pdfTeX font-code banks"),
        };
        set_font_bank_word(
            map,
            &mut self.journal,
            #[cfg(feature = "shadow")]
            &mut self.shadow,
            self.epoch,
            bank,
            index,
            encode_i32(value),
            true,
        )
    }

    pub(crate) fn pdf_no_ligatures(&self, font: FontId) -> bool {
        font_bank_word(&self.pdf_no_ligatures, font.raw()) != 0
    }

    pub(crate) fn set_pdf_no_ligatures_global(&mut self, font: FontId) -> CellMutationReceipt {
        set_font_bank_word(
            &mut self.pdf_no_ligatures,
            &mut self.journal,
            #[cfg(feature = "shadow")]
            &mut self.shadow,
            self.epoch,
            BankTag::PdfNoLigatures,
            font.raw(),
            1,
            true,
        )
    }

    fn set_current_font_word(&mut self, word: u64, global: bool) -> CellMutationReceipt {
        let cell = if global {
            CellId::new_global(BankTag::CurrentFont, 0)
        } else {
            CellId::new(BankTag::CurrentFont, 0)
        };
        barrier(
            &mut self.current_font.word,
            &mut self.current_font.stamp,
            &mut self.journal,
            #[cfg(feature = "shadow")]
            &mut self.shadow,
            self.epoch,
            cell,
            word,
        )
    }

    fn set_font_int_bank(
        &mut self,
        bank: BankTag,
        font: FontId,
        value: i32,
        global: bool,
    ) -> CellMutationReceipt {
        let map = match bank {
            BankTag::FontHyphenChar => &mut self.font_hyphen_chars,
            BankTag::FontSkewChar => &mut self.font_skew_chars,
            _ => unreachable!("caller restricts font integer banks"),
        };
        set_font_bank_word(
            map,
            &mut self.journal,
            #[cfg(feature = "shadow")]
            &mut self.shadow,
            self.epoch,
            bank,
            font.raw(),
            encode_i32(value),
            global,
        )
    }
}

#[inline]
pub(crate) fn barrier(
    cell_slot: &mut u64,
    stamp_slot: &mut Epoch,
    journal: &mut Journal,
    #[cfg(feature = "shadow")] shadow: &mut AHashMap<CellId, u64>,
    epoch: Epoch,
    cell_id: CellId,
    new_word: u64,
) -> CellMutationReceipt {
    let receipt = CellMutationReceipt::write(cell_id, *cell_slot, new_word);
    if *cell_slot == new_word {
        if cell_id.is_global() {
            journal.push_undo(UndoRec::new(cell_id, *cell_slot, new_word));
        } else if *stamp_slot < epoch {
            // TeX82 §§278/283 and §1194: `eq_word_define` saves the outer
            // value on the first assignment at a new group level even when
            // the replacement word is equal. The save-stack record remains
            // observable through `\tracingrestores` when that group ends.
            journal.push_undo(UndoRec::new(cell_id, *cell_slot, new_word));
            *stamp_slot = epoch;
        }
        return receipt;
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
    receipt
}

#[cfg(feature = "shadow")]
pub(crate) fn shadow_set(shadow: &mut AHashMap<CellId, u64>, cell: CellId, word: u64) {
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
    Muskip,
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

pub(crate) fn font_dimen_index(
    font: FontId,
    number: u32,
) -> Result<u32, crate::stores::FontParameterError> {
    use crate::font::{MAX_FONT_DIMEN, MAX_FONT_DIMEN_FONT_ID};
    use crate::stores::FontParameterError;

    if number == 0 {
        return Err(FontParameterError::Zero);
    }
    if number > MAX_FONT_DIMEN {
        return Err(FontParameterError::NumberOutOfRange {
            number,
            maximum: MAX_FONT_DIMEN,
        });
    }
    if font.raw() > MAX_FONT_DIMEN_FONT_ID {
        return Err(FontParameterError::FontOutOfRange {
            font,
            maximum: MAX_FONT_DIMEN_FONT_ID,
        });
    }
    Ok((font.raw() << FONT_DIMEN_BITS) | (number - 1))
}

fn math_family_font_index(size: MathFontSize, family: u8) -> u16 {
    assert!(family < MATH_FAMILY_COUNT, "math family index out of range");
    size.index() * u16::from(MATH_FAMILY_COUNT) + u16::from(family)
}

fn font_bank_word(map: &BTreeMap<u32, WordStamp>, index: u32) -> u64 {
    map.get(&index).map_or(0, |entry| entry.word)
}

fn pack_current_font(symbol: Option<Symbol>, font: FontId) -> u64 {
    let symbol = symbol.map_or(0, |symbol| u64::from(symbol.raw()) + 1);
    (symbol << 32) | u64::from(font.raw())
}

#[allow(clippy::too_many_arguments)]
fn set_font_bank_word(
    map: &mut BTreeMap<u32, WordStamp>,
    journal: &mut Journal,
    #[cfg(feature = "shadow")] shadow: &mut AHashMap<CellId, u64>,
    epoch: Epoch,
    bank: BankTag,
    index: u32,
    word: u64,
    global: bool,
) -> CellMutationReceipt {
    let entry = map.entry(index).or_default();
    let cell = if global {
        CellId::new_global(bank, index)
    } else {
        CellId::new(bank, index)
    };
    barrier(
        &mut entry.word,
        &mut entry.stamp,
        journal,
        #[cfg(feature = "shadow")]
        shadow,
        epoch,
        cell,
        word,
    )
}

fn encode_i32(value: i32) -> u64 {
    u64::from(value as u32)
}

fn decode_i32(word: u64) -> i32 {
    word as u32 as i32
}

fn decode_u32(word: u64) -> u32 {
    match u32::try_from(word) {
        Ok(value) => value,
        Err(_) => panic!("font parameter count exceeds u32"),
    }
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
