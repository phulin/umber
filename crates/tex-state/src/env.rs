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
pub(crate) mod group;
pub(crate) mod overflow;
pub(crate) mod raw;

use self::banks::{
    BankJournalContext, BankSetContext, DENSE_REGISTER_COUNT, DimenParam, FixedBank, GlueIdCodec,
    GlueParam, I32Codec, IntParam, NodeListIdCodec, PARAMETER_COUNT, ScaledCodec, TokParam,
    TokenListIdCodec,
};
use self::overflow::{REGISTER_COUNT, SparseBank};
use crate::cell::{BankTag, CellId};
use crate::epoch::Epoch;
use crate::ids::{FontId, GlueId, NodeListId, TokenListId};
use crate::interner::Symbol;
#[cfg(test)]
use crate::journal::JournalPos;
use crate::journal::{Journal, UndoRec};
use crate::meaning::Meaning;
use crate::scaled::Scaled;
use crate::token::Token;
use std::collections::BTreeMap;
#[cfg(feature = "shadow")]
use std::collections::HashMap;

const SEGMENT_BITS: u32 = 16;
const SEGMENT_LEN: usize = 1 << SEGMENT_BITS;
const SEGMENT_MASK: u32 = (SEGMENT_LEN as u32) - 1;
const FONT_DIMEN_BITS: u32 = 15;
const FONT_DIMEN_MASK: u32 = (1 << FONT_DIMEN_BITS) - 1;

type MeaningSegment = Box<[u64; SEGMENT_LEN]>;
type StampSegment = Box<[Epoch; SEGMENT_LEN]>;

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

        pub(crate) fn $set(&mut self, index: u16, value: $value) {
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
                );
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
                );
            }
        }

        pub(crate) fn $set_global(&mut self, index: u16, value: $value) {
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
                );
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
    muskips: FixedBank<GlueIdCodec, DENSE_REGISTER_COUNT>,
    overflow_counts: SparseBank<I32Codec>,
    overflow_dimens: SparseBank<ScaledCodec>,
    overflow_skips: SparseBank<GlueIdCodec>,
    overflow_toks: SparseBank<TokenListIdCodec>,
    overflow_boxes: SparseBank<NodeListIdCodec>,
    overflow_muskips: SparseBank<GlueIdCodec>,
    int_params: FixedBank<I32Codec, PARAMETER_COUNT>,
    dimen_params: FixedBank<ScaledCodec, PARAMETER_COUNT>,
    glue_params: FixedBank<GlueIdCodec, PARAMETER_COUNT>,
    tok_params: FixedBank<TokenListIdCodec, PARAMETER_COUNT>,
    font_dimens: BTreeMap<u32, WordStamp>,
    font_param_lens: BTreeMap<u32, WordStamp>,
    font_hyphen_chars: BTreeMap<u32, WordStamp>,
    font_skew_chars: BTreeMap<u32, WordStamp>,
    current_font: WordStamp,
    journal: Journal,
    aftergroup: Vec<Token>,
    afterassignment: Option<Token>,
    group_depth: u32,
    epoch: Epoch,
    #[cfg(feature = "shadow")]
    shadow: HashMap<CellId, u64>,
}

impl Env {
    /// Creates an empty environment in the first session epoch.
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            meaning_cells: Vec::new(),
            meaning_stamps: Vec::new(),
            counts: FixedBank::new(),
            dimens: FixedBank::new(),
            skips: FixedBank::new(),
            toks: FixedBank::new(),
            boxes: FixedBank::new(),
            muskips: FixedBank::new(),
            overflow_counts: SparseBank::new(),
            overflow_dimens: SparseBank::new(),
            overflow_skips: SparseBank::new(),
            overflow_toks: SparseBank::new(),
            overflow_boxes: SparseBank::new(),
            overflow_muskips: SparseBank::new(),
            int_params: FixedBank::new(),
            dimen_params: FixedBank::new(),
            glue_params: FixedBank::new(),
            tok_params: FixedBank::new(),
            font_dimens: BTreeMap::new(),
            font_param_lens: BTreeMap::new(),
            font_hyphen_chars: BTreeMap::new(),
            font_skew_chars: BTreeMap::new(),
            current_font: WordStamp::default(),
            journal: Journal::new(),
            aftergroup: Vec::new(),
            afterassignment: None,
            group_depth: 0,
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
    #[cfg(test)]
    pub(crate) fn bump_epoch(&mut self) {
        self.epoch.bump();
    }

    /// Returns the current journal end position.
    #[must_use]
    #[cfg(test)]
    pub(crate) fn journal_pos(&self) -> JournalPos {
        self.journal.pos()
    }

    /// Returns the meaning for `symbol`, defaulting to `Undefined`.
    #[must_use]
    pub fn get(&self, symbol: Symbol) -> Meaning {
        let Some(word) = self.meaning_word(symbol.raw()) else {
            return Meaning::Undefined;
        };
        Meaning::decode_stored(word)
    }

    /// Sets the local meaning for a symbol validated by the owning store.
    pub(crate) fn set(&mut self, symbol: Symbol, meaning: Meaning) {
        self.set_meaning_word(symbol, meaning.encode(), false);
    }

    /// Sets the global meaning for a symbol validated by the owning store.
    pub(crate) fn set_global(&mut self, symbol: Symbol, meaning: Meaning) {
        self.set_meaning_word(symbol, meaning.encode(), true);
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
        muskip,
        set_muskip,
        set_muskip_global,
        GlueId,
        Muskip,
        muskips,
        overflow_muskips
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
    /// Returns a box register value; `None` is TeX's void box.
    #[must_use]
    pub fn box_reg(&self, index: u16) -> Option<NodeListId> {
        if is_dense_register(index) {
            self.boxes.get(index)
        } else {
            self.overflow_boxes.get(index)
        }
    }

    /// Sets a local box register value validated by the owning store.
    pub(crate) fn set_box_reg(&mut self, index: u16, value: Option<NodeListId>) -> Option<UndoRec> {
        if is_dense_register(index) {
            self.boxes.set_always_journal(
                index,
                value,
                BankJournalContext {
                    journal: &mut self.journal,
                    #[cfg(feature = "shadow")]
                    shadow: &mut self.shadow,
                    bank: BankTag::Box,
                    global: false,
                },
            )
        } else {
            self.overflow_boxes.set_always_journal(
                index,
                value,
                BankJournalContext {
                    journal: &mut self.journal,
                    #[cfg(feature = "shadow")]
                    shadow: &mut self.shadow,
                    bank: BankTag::Box,
                    global: false,
                },
            )
        }
    }

    /// Sets a global box register value validated by the owning store.
    pub(crate) fn set_box_reg_global(
        &mut self,
        index: u16,
        value: Option<NodeListId>,
    ) -> Option<UndoRec> {
        if is_dense_register(index) {
            self.boxes.set_always_journal(
                index,
                value,
                BankJournalContext {
                    journal: &mut self.journal,
                    #[cfg(feature = "shadow")]
                    shadow: &mut self.shadow,
                    bank: BankTag::Box,
                    global: true,
                },
            )
        } else {
            self.overflow_boxes.set_always_journal(
                index,
                value,
                BankJournalContext {
                    journal: &mut self.journal,
                    #[cfg(feature = "shadow")]
                    shadow: &mut self.shadow,
                    bank: BankTag::Box,
                    global: true,
                },
            )
        }
    }

    /// Returns an integer parameter value.
    #[must_use]
    pub fn int_param(&self, param: IntParam) -> i32 {
        self.int_params.get(param.raw())
    }

    /// Sets a local integer parameter value.
    pub(crate) fn set_int_param(&mut self, param: IntParam, value: i32) {
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
        );
    }

    /// Sets a global integer parameter value.
    pub(crate) fn set_int_param_global(&mut self, param: IntParam, value: i32) {
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
        );
    }

    /// Returns a dimension parameter value.
    #[must_use]
    pub fn dimen_param(&self, param: DimenParam) -> Scaled {
        self.dimen_params.get(param.raw())
    }

    /// Sets a local dimension parameter value.
    pub(crate) fn set_dimen_param(&mut self, param: DimenParam, value: Scaled) {
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
        );
    }

    /// Sets a global dimension parameter value.
    pub(crate) fn set_dimen_param_global(&mut self, param: DimenParam, value: Scaled) {
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
        );
    }

    /// Returns a glue parameter value.
    #[must_use]
    pub fn glue_param(&self, param: GlueParam) -> GlueId {
        self.glue_params.get(param.raw())
    }

    /// Sets a local glue parameter value.
    pub(crate) fn set_glue_param(&mut self, param: GlueParam, value: GlueId) {
        self.glue_params.set(
            param.raw(),
            value,
            BankSetContext {
                journal: &mut self.journal,
                #[cfg(feature = "shadow")]
                shadow: &mut self.shadow,
                epoch: self.epoch,
                bank: BankTag::GlueParam,
                global: false,
            },
        );
    }

    /// Sets a global glue parameter value.
    pub(crate) fn set_glue_param_global(&mut self, param: GlueParam, value: GlueId) {
        self.glue_params.set(
            param.raw(),
            value,
            BankSetContext {
                journal: &mut self.journal,
                #[cfg(feature = "shadow")]
                shadow: &mut self.shadow,
                epoch: self.epoch,
                bank: BankTag::GlueParam,
                global: true,
            },
        );
    }

    /// Returns a token-list parameter value.
    #[must_use]
    pub fn tok_param(&self, param: TokParam) -> TokenListId {
        self.tok_params.get(param.raw())
    }

    /// Sets a local token-list parameter value.
    pub(crate) fn set_tok_param(&mut self, param: TokParam, value: TokenListId) {
        self.tok_params.set(
            param.raw(),
            value,
            BankSetContext {
                journal: &mut self.journal,
                #[cfg(feature = "shadow")]
                shadow: &mut self.shadow,
                epoch: self.epoch,
                bank: BankTag::TokParam,
                global: false,
            },
        );
    }

    /// Sets a global token-list parameter value.
    pub(crate) fn set_tok_param_global(&mut self, param: TokParam, value: TokenListId) {
        self.tok_params.set(
            param.raw(),
            value,
            BankSetContext {
                journal: &mut self.journal,
                #[cfg(feature = "shadow")]
                shadow: &mut self.shadow,
                epoch: self.epoch,
                bank: BankTag::TokParam,
                global: true,
            },
        );
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

    pub(crate) fn set_current_font(&mut self, value: FontId) {
        self.set_current_font_word(pack_current_font(self.current_font_symbol(), value), false);
    }

    pub(crate) fn set_current_font_global(&mut self, value: FontId) {
        self.set_current_font_word(pack_current_font(self.current_font_symbol(), value), true);
    }

    pub(crate) fn set_current_font_selector(&mut self, symbol: Symbol, value: FontId) {
        self.set_current_font_word(pack_current_font(Some(symbol), value), false);
    }

    pub(crate) fn set_current_font_selector_global(&mut self, symbol: Symbol, value: FontId) {
        self.set_current_font_word(pack_current_font(Some(symbol), value), true);
    }

    #[must_use]
    pub fn font_dimen(&self, font: FontId, number: u16) -> Scaled {
        Scaled::from_raw(decode_i32(font_bank_word(
            &self.font_dimens,
            font_dimen_index(font, number),
        )))
    }

    pub(crate) fn set_font_dimen(&mut self, font: FontId, number: u16, value: Scaled) {
        set_font_bank_word(
            &mut self.font_dimens,
            &mut self.journal,
            #[cfg(feature = "shadow")]
            &mut self.shadow,
            self.epoch,
            BankTag::FontDimen,
            font_dimen_index(font, number),
            encode_i32(value.raw()),
            false,
        );
    }

    pub(crate) fn set_font_dimen_global(&mut self, font: FontId, number: u16, value: Scaled) {
        set_font_bank_word(
            &mut self.font_dimens,
            &mut self.journal,
            #[cfg(feature = "shadow")]
            &mut self.shadow,
            self.epoch,
            BankTag::FontDimen,
            font_dimen_index(font, number),
            encode_i32(value.raw()),
            true,
        );
    }

    #[must_use]
    pub fn font_param_len(&self, font: FontId) -> u16 {
        decode_u16(font_bank_word(&self.font_param_lens, font.raw()))
    }

    pub(crate) fn set_font_param_len(&mut self, font: FontId, value: u16) {
        set_font_bank_word(
            &mut self.font_param_lens,
            &mut self.journal,
            #[cfg(feature = "shadow")]
            &mut self.shadow,
            self.epoch,
            BankTag::FontParamLen,
            font.raw(),
            u64::from(value),
            false,
        );
    }

    pub(crate) fn set_font_param_len_global(&mut self, font: FontId, value: u16) {
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
        );
    }

    #[must_use]
    pub fn font_hyphen_char(&self, font: FontId) -> i32 {
        decode_i32(font_bank_word(&self.font_hyphen_chars, font.raw()))
    }

    pub(crate) fn set_font_hyphen_char(&mut self, font: FontId, value: i32) {
        self.set_font_int_bank(BankTag::FontHyphenChar, font, value, false);
    }

    pub(crate) fn set_font_hyphen_char_global(&mut self, font: FontId, value: i32) {
        self.set_font_int_bank(BankTag::FontHyphenChar, font, value, true);
    }

    #[must_use]
    pub fn font_skew_char(&self, font: FontId) -> i32 {
        decode_i32(font_bank_word(&self.font_skew_chars, font.raw()))
    }

    pub(crate) fn set_font_skew_char(&mut self, font: FontId, value: i32) {
        self.set_font_int_bank(BankTag::FontSkewChar, font, value, false);
    }

    pub(crate) fn set_font_skew_char_global(&mut self, font: FontId, value: i32) {
        self.set_font_int_bank(BankTag::FontSkewChar, font, value, true);
    }

    fn set_current_font_word(&mut self, word: u64, global: bool) {
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
        );
    }

    fn set_font_int_bank(&mut self, bank: BankTag, font: FontId, value: i32, global: bool) {
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
        );
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
pub(crate) fn shadow_set(shadow: &mut HashMap<CellId, u64>, cell: CellId, word: u64) {
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

fn font_dimen_index(font: FontId, number: u16) -> u32 {
    assert!(number != 0, "fontdimen number must be positive");
    let font = font.raw();
    assert!(
        font < (1 << (27 - FONT_DIMEN_BITS)),
        "font id exceeds fontdimen cell range"
    );
    (font << FONT_DIMEN_BITS) | (u32::from(number - 1) & FONT_DIMEN_MASK)
}

fn font_bank_word(map: &BTreeMap<u32, WordStamp>, index: u32) -> u64 {
    map.get(&index).map_or(0, |entry| entry.word)
}

fn pack_current_font(symbol: Option<Symbol>, font: FontId) -> u64 {
    let symbol = symbol.map_or(0, |symbol| u64::from(symbol.raw()) + 1);
    (symbol << 32) | u64::from(font.raw())
}

fn set_font_bank_word(
    map: &mut BTreeMap<u32, WordStamp>,
    journal: &mut Journal,
    #[cfg(feature = "shadow")] shadow: &mut HashMap<CellId, u64>,
    epoch: Epoch,
    bank: BankTag,
    index: u32,
    word: u64,
    global: bool,
) {
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
    );
}

fn encode_i32(value: i32) -> u64 {
    u64::from(value as u32)
}

fn decode_i32(word: u64) -> i32 {
    word as u32 as i32
}

fn decode_u16(word: u64) -> u16 {
    match u16::try_from(word) {
        Ok(value) => value,
        Err(_) => panic!("font parameter count exceeds u16"),
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
