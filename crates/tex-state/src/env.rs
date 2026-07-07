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

use self::banks::{
    DENSE_REGISTER_COUNT, DimenParam, FixedBank, GlueIdCodec, GlueParam, I32Codec, IntParam,
    NodeListIdCodec, PARAMETER_COUNT, ScaledCodec, TokParam, TokenListIdCodec,
};
use crate::cell::{BankTag, CellId};
use crate::epoch::Epoch;
use crate::ids::{GlueId, NodeListId, TokenListId};
use crate::interner::Symbol;
use crate::journal::{Entry, Journal, JournalPos, UndoRec};
use crate::meaning::Meaning;
use crate::scaled::Scaled;

const SEGMENT_BITS: u32 = 16;
const SEGMENT_LEN: usize = 1 << SEGMENT_BITS;
const SEGMENT_MASK: u32 = (SEGMENT_LEN as u32) - 1;

type MeaningSegment = Box<[u64; SEGMENT_LEN]>;
type StampSegment = Box<[Epoch; SEGMENT_LEN]>;

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
    int_params: FixedBank<I32Codec, PARAMETER_COUNT>,
    dimen_params: FixedBank<ScaledCodec, PARAMETER_COUNT>,
    glue_params: FixedBank<GlueIdCodec, PARAMETER_COUNT>,
    tok_params: FixedBank<TokenListIdCodec, PARAMETER_COUNT>,
    journal: Journal,
    epoch: Epoch,
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
            int_params: FixedBank::new(),
            dimen_params: FixedBank::new(),
            glue_params: FixedBank::new(),
            tok_params: FixedBank::new(),
            journal: Journal::new(),
            epoch: Epoch::START,
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
    #[must_use]
    pub fn journal_pos(&self) -> JournalPos {
        self.journal.pos()
    }

    /// Returns journal entries appended since `pos`.
    #[must_use]
    pub fn journal_entries_since(&self, pos: JournalPos) -> &[Entry] {
        self.journal.entries_since(pos)
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

    /// Returns the dense count register value at `index`.
    #[must_use]
    pub fn count(&self, index: u16) -> i32 {
        self.counts.get(index)
    }

    /// Sets the local dense count register value at `index`.
    pub fn set_count(&mut self, index: u16, value: i32) {
        self.counts.set(
            index,
            value,
            &mut self.journal,
            self.epoch,
            BankTag::Count,
            false,
        );
    }

    /// Sets the global dense count register value at `index`.
    pub fn set_count_global(&mut self, index: u16, value: i32) {
        self.counts.set(
            index,
            value,
            &mut self.journal,
            self.epoch,
            BankTag::Count,
            true,
        );
    }

    /// Returns the dense dimension register value at `index`.
    #[must_use]
    pub fn dimen(&self, index: u16) -> Scaled {
        self.dimens.get(index)
    }

    /// Sets the local dense dimension register value at `index`.
    pub fn set_dimen(&mut self, index: u16, value: Scaled) {
        self.dimens.set(
            index,
            value,
            &mut self.journal,
            self.epoch,
            BankTag::Dimen,
            false,
        );
    }

    /// Sets the global dense dimension register value at `index`.
    pub fn set_dimen_global(&mut self, index: u16, value: Scaled) {
        self.dimens.set(
            index,
            value,
            &mut self.journal,
            self.epoch,
            BankTag::Dimen,
            true,
        );
    }

    /// Returns the dense skip register value at `index`.
    #[must_use]
    pub fn skip(&self, index: u16) -> GlueId {
        self.skips.get(index)
    }

    /// Sets the local dense skip register value at `index`.
    pub fn set_skip(&mut self, index: u16, value: GlueId) {
        self.skips.set(
            index,
            value,
            &mut self.journal,
            self.epoch,
            BankTag::Skip,
            false,
        );
    }

    /// Sets the global dense skip register value at `index`.
    pub fn set_skip_global(&mut self, index: u16, value: GlueId) {
        self.skips.set(
            index,
            value,
            &mut self.journal,
            self.epoch,
            BankTag::Skip,
            true,
        );
    }

    /// Returns the dense token-list register value at `index`.
    #[must_use]
    pub fn toks(&self, index: u16) -> TokenListId {
        self.toks.get(index)
    }

    /// Sets the local dense token-list register value at `index`.
    pub fn set_toks(&mut self, index: u16, value: TokenListId) {
        self.toks.set(
            index,
            value,
            &mut self.journal,
            self.epoch,
            BankTag::Toks,
            false,
        );
    }

    /// Sets the global dense token-list register value at `index`.
    pub fn set_toks_global(&mut self, index: u16, value: TokenListId) {
        self.toks.set(
            index,
            value,
            &mut self.journal,
            self.epoch,
            BankTag::Toks,
            true,
        );
    }

    /// Returns the dense box register value at `index`.
    #[must_use]
    pub fn box_reg(&self, index: u16) -> NodeListId {
        self.boxes.get(index)
    }

    /// Sets the local dense box register value at `index`.
    pub fn set_box_reg(&mut self, index: u16, value: NodeListId) {
        self.boxes.set(
            index,
            value,
            &mut self.journal,
            self.epoch,
            BankTag::Box,
            false,
        );
    }

    /// Sets the global dense box register value at `index`.
    pub fn set_box_reg_global(&mut self, index: u16, value: NodeListId) {
        self.boxes.set(
            index,
            value,
            &mut self.journal,
            self.epoch,
            BankTag::Box,
            true,
        );
    }

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
            self.epoch,
            BankTag::TokParam,
            true,
        );
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
    epoch: Epoch,
    cell_id: CellId,
    new_word: u64,
) {
    if *stamp_slot < epoch {
        journal.push_undo(UndoRec::new(cell_id, *cell_slot, new_word));
        *stamp_slot = epoch;
    }
    *cell_slot = new_word;
}

fn segment_index(index: u32) -> usize {
    (index >> SEGMENT_BITS) as usize
}

fn segment_offset(index: u32) -> usize {
    (index & SEGMENT_MASK) as usize
}

#[cfg(test)]
mod tests {
    use super::{Env, SEGMENT_LEN};
    use crate::cell::{BankTag, CellId};
    use crate::env::banks::{DimenParam, GlueParam, IntParam, TokParam};
    use crate::ids::{GlueId, NodeListId, TokenListId};
    use crate::interner::Symbol;
    use crate::journal::{Entry, UndoRec};
    use crate::meaning::Meaning;
    use crate::scaled::Scaled;

    #[test]
    fn default_get_before_any_set_is_undefined() {
        let env = Env::new();

        assert_eq!(env.get(Symbol::new(10)), Meaning::Undefined);
    }

    #[test]
    fn first_write_per_epoch_coalesces_and_keeps_first_new_value() {
        let mut env = Env::new();
        let symbol = Symbol::new(3);
        let start = env.journal_pos();

        env.set(symbol, Meaning::Relax);
        env.set(symbol, Meaning::CharGiven('x'));

        assert_eq!(env.get(symbol), Meaning::CharGiven('x'));
        assert_eq!(
            env.journal_entries_since(start),
            &[Entry::Undo(UndoRec::new(
                CellId::new(BankTag::Meaning, 3),
                Meaning::Undefined.encode(),
                Meaning::Relax.encode(),
            ))]
        );
    }

    #[test]
    fn write_in_later_epoch_records_again() {
        let mut env = Env::new();
        let symbol = Symbol::new(8);
        let start = env.journal_pos();

        env.set(symbol, Meaning::Relax);
        env.bump_epoch();
        env.set(symbol, Meaning::CharGiven('y'));

        assert_eq!(
            env.journal_entries_since(start),
            &[
                Entry::Undo(UndoRec::new(
                    CellId::new(BankTag::Meaning, 8),
                    Meaning::Undefined.encode(),
                    Meaning::Relax.encode(),
                )),
                Entry::Undo(UndoRec::new(
                    CellId::new(BankTag::Meaning, 8),
                    Meaning::Relax.encode(),
                    Meaning::CharGiven('y').encode(),
                )),
            ]
        );
    }

    #[test]
    fn global_set_tags_cell_id_in_journal() {
        let mut env = Env::new();
        let symbol = Symbol::new(4);
        let start = env.journal_pos();

        env.set_global(symbol, Meaning::Relax);

        assert_eq!(
            env.journal_entries_since(start),
            &[Entry::Undo(UndoRec::new(
                CellId::new_global(BankTag::Meaning, 4),
                Meaning::Undefined.encode(),
                Meaning::Relax.encode(),
            ))]
        );
    }

    #[test]
    fn segment_growth_keeps_earlier_segment_addresses_stable() {
        let mut env = Env::new();
        let first = Symbol::new(0);
        let second_segment = Symbol::new(SEGMENT_LEN as u32);

        env.set(first, Meaning::Relax);
        let cells_ptr = env.meaning_cells[0].as_ptr();
        let stamps_ptr = env.meaning_stamps[0].as_ptr();

        env.set(second_segment, Meaning::CharGiven('z'));

        assert_eq!(env.meaning_cells[0].as_ptr(), cells_ptr);
        assert_eq!(env.meaning_stamps[0].as_ptr(), stamps_ptr);
        assert_eq!(env.get(first), Meaning::Relax);
        assert_eq!(env.get(second_segment), Meaning::CharGiven('z'));
    }

    #[test]
    fn dense_register_typed_api_round_trips_boundary_and_signed_values() {
        let mut env = Env::new();

        env.set_count(255, i32::MIN);
        env.set_dimen(255, Scaled::MIN);
        env.set_skip(255, GlueId::new(u32::MAX));
        env.set_toks(255, TokenListId::new(u32::MAX - 1));
        env.set_box_reg(255, NodeListId::new(u32::MAX - 2));

        assert_eq!(env.count(255), i32::MIN);
        assert_eq!(env.dimen(255), Scaled::MIN);
        assert_eq!(env.skip(255), GlueId::new(u32::MAX));
        assert_eq!(env.toks(255), TokenListId::new(u32::MAX - 1));
        assert_eq!(env.box_reg(255), NodeListId::new(u32::MAX - 2));
    }

    #[test]
    fn dense_register_journal_records_use_bank_tags_and_encoded_words() {
        let mut env = Env::new();
        let start = env.journal_pos();

        env.set_count(1, -1);
        env.set_dimen(2, Scaled::from_raw(-2));
        env.set_skip(3, GlueId::new(33));
        env.set_toks(4, TokenListId::new(44));
        env.set_box_reg(5, NodeListId::new(55));

        assert_eq!(
            env.journal_entries_since(start),
            &[
                undo(BankTag::Count, 1, 0, u64::from((-1_i32) as u32)),
                undo(BankTag::Dimen, 2, 0, u64::from((-2_i32) as u32)),
                undo(BankTag::Skip, 3, 0, 33),
                undo(BankTag::Toks, 4, 0, 44),
                undo(BankTag::Box, 5, 0, 55),
            ]
        );
    }

    #[test]
    fn dense_register_global_sets_tag_journal_records() {
        let mut env = Env::new();
        let start = env.journal_pos();

        env.set_count_global(255, 7);

        assert_eq!(
            env.journal_entries_since(start),
            &[Entry::Undo(UndoRec::new(
                CellId::new_global(BankTag::Count, 255),
                0,
                7,
            ))]
        );
    }

    #[test]
    fn parameter_typed_api_round_trips_values() {
        let mut env = Env::new();

        env.set_int_param(IntParam::new(127), i32::MIN);
        env.set_dimen_param(DimenParam::new(127), Scaled::MIN);
        env.set_glue_param(GlueParam::new(127), GlueId::new(77));
        env.set_tok_param(TokParam::new(127), TokenListId::new(88));

        assert_eq!(env.int_param(IntParam::new(127)), i32::MIN);
        assert_eq!(env.dimen_param(DimenParam::new(127)), Scaled::MIN);
        assert_eq!(env.glue_param(GlueParam::new(127)), GlueId::new(77));
        assert_eq!(env.tok_param(TokParam::new(127)), TokenListId::new(88));
    }

    #[test]
    fn parameter_journal_records_use_parameter_bank_tags() {
        let mut env = Env::new();
        let start = env.journal_pos();

        env.set_int_param(IntParam::new(1), -9);
        env.set_dimen_param(DimenParam::new(2), Scaled::from_raw(-10));
        env.set_glue_param(GlueParam::new(3), GlueId::new(90));
        env.set_tok_param(TokParam::new(4), TokenListId::new(100));

        assert_eq!(
            env.journal_entries_since(start),
            &[
                undo(BankTag::IntParam, 1, 0, u64::from((-9_i32) as u32)),
                undo(BankTag::DimenParam, 2, 0, u64::from((-10_i32) as u32)),
                undo(BankTag::GlueParam, 3, 0, 90),
                undo(BankTag::TokParam, 4, 0, 100),
            ]
        );
    }

    #[test]
    fn parameter_global_sets_tag_journal_records() {
        let mut env = Env::new();
        let start = env.journal_pos();

        env.set_tok_param_global(TokParam::new(7), TokenListId::new(11));

        assert_eq!(
            env.journal_entries_since(start),
            &[Entry::Undo(UndoRec::new(
                CellId::new_global(BankTag::TokParam, 7),
                0,
                11,
            ))]
        );
    }

    fn undo(bank: BankTag, index: u32, old: u64, new: u64) -> Entry {
        Entry::Undo(UndoRec::new(CellId::new(bank, index), old, new))
    }
}
