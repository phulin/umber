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
    GlueIdCodec, GlueParam, I32Codec, IntParam, PARAMETER_COUNT, ScaledCodec, TokParam,
    TokenListIdCodec,
};
use self::box_bank::{BoxBank, BoxWriteContext};
use self::overflow::{REGISTER_COUNT, SparseBank};
use crate::cell::{BankTag, CellId};
use crate::epoch::Epoch;
use crate::ids::{FontId, GlueId, NodeListId, TokenListId};
use crate::interner::Symbol;
use crate::journal::{Journal, UndoRec};
use crate::math::{MATH_FAMILY_COUNT, MathFontSize};
use crate::meaning::Meaning;
use crate::scaled::Scaled;
use crate::token::Token;
#[cfg(feature = "shadow")]
use ahash::AHashMap;
use std::collections::BTreeMap;

const SEGMENT_BITS: u32 = 16;
const SEGMENT_LEN: usize = 1 << SEGMENT_BITS;
const SEGMENT_MASK: u32 = (SEGMENT_LEN as u32) - 1;
const FONT_DIMEN_BITS: u32 = 17;
const MATH_FAMILY_FONT_COUNT: usize = 3 * MATH_FAMILY_COUNT as usize;

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
    tok_params: FixedBank<TokenListIdCodec, PARAMETER_COUNT>,
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
    group_boundaries: Vec<group::GroupBoundary>,
    aftergroup: Vec<Token>,
    afterassignment: Option<Token>,
    group_depth: u32,
    epoch: Epoch,
    #[cfg(feature = "shadow")]
    shadow: AHashMap<CellId, u64>,
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
            group_boundaries: Vec::new(),
            aftergroup: Vec::new(),
            afterassignment: None,
            group_depth: 0,
            epoch: Epoch::START,
            #[cfg(feature = "shadow")]
            shadow: AHashMap::new(),
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
    pub(crate) fn journal_pos(&self) -> crate::journal::JournalPos {
        self.journal.pos()
    }

    /// Returns the meaning for `symbol`, defaulting to `Undefined`.
    #[must_use]
    pub fn get(&self, symbol: Symbol) -> Meaning {
        self.get_meaning_slot(symbol.raw())
    }

    /// Returns the meaning at a dense interner slot.
    #[must_use]
    pub(crate) fn get_meaning_slot(&self, slot: u32) -> Meaning {
        let Some(word) = self.meaning_word(slot) else {
            return Meaning::Undefined;
        };
        Meaning::decode_stored(word)
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
    pub(crate) fn set_meaning_slot(&mut self, slot: u32, meaning: Meaning, global: bool) {
        self.set_meaning_word(slot, meaning.encode(), global);
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
        NodeListId::decode_box_word(self.boxes.get(index).value())
    }

    /// Sets a local box register value validated by the owning store.
    pub(crate) fn set_box_reg(&mut self, index: u16, value: Option<NodeListId>) -> BoxWriteOutcome {
        self.set_box_reg_local(index, value, true)
    }

    fn set_box_reg_local(
        &mut self,
        index: u16,
        value: Option<NodeListId>,
        coalesce: bool,
    ) -> BoxWriteOutcome {
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
            NodeListId::encode_box_word(value),
        );
        outcome
    }

    /// Sets a global box register value validated by the owning store.
    pub(crate) fn set_box_reg_global(
        &mut self,
        index: u16,
        value: Option<NodeListId>,
    ) -> BoxWriteOutcome {
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
            NodeListId::encode_box_word(value),
        );
        outcome
    }

    /// Sets a box register at TeX's current box level.
    pub(crate) fn set_box_reg_same_level(
        &mut self,
        index: u16,
        value: Option<NodeListId>,
    ) -> BoxWriteOutcome {
        let owner_depth = self.boxes.get(index).owner_depth();
        if owner_depth == 0 {
            return self.set_box_reg_global(index, value);
        }
        if owner_depth == self.group_depth {
            return self.set_box_reg(index, value);
        }
        let outcome = self.boxes.write_same_level(index, value, &mut self.journal);
        #[cfg(feature = "shadow")]
        shadow_set(
            &mut self.shadow,
            CellId::new(BankTag::Box, u32::from(index)),
            NodeListId::encode_box_word(value),
        );
        outcome
    }

    /// Takes a box register at TeX's current box level.
    ///
    /// This matches `\box<n>`: if the visible box value was locally assigned
    /// in the current group, the voiding is local to that group; otherwise it
    /// must survive the current group while remaining rollback-visible.
    pub(crate) fn take_box_reg_same_level(
        &mut self,
        index: u16,
    ) -> (Option<NodeListId>, BoxWriteOutcome) {
        let old = self.box_reg(index);
        let owner_depth = self.boxes.get(index).owner_depth();
        let rec = if owner_depth == 0 {
            self.set_box_reg_global(index, None)
        } else if owner_depth == self.group_depth {
            self.set_box_reg_local(index, None, false)
        } else {
            self.set_box_reg_same_level(index, None)
        };
        (old, rec)
    }

    /// Takes a local box while retaining its returned handle in a distinct
    /// undo record until the caller has consumed it.
    pub(crate) fn take_box_reg(&mut self, index: u16) -> (Option<NodeListId>, BoxWriteOutcome) {
        let old = self.box_reg(index);
        let outcome = self.set_box_reg_local(index, None, false);
        (old, outcome)
    }

    #[cfg(test)]
    fn box_reg_is_local_to_current_group(&self, index: u16) -> bool {
        self.group_depth != 0 && self.boxes.get(index).is_owned_by(self.group_depth)
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

    /// Returns the font selected for a math family and size.
    #[must_use]
    pub fn math_family_font(&self, size: MathFontSize, family: u8) -> FontId {
        self.math_family_fonts
            .get(math_family_font_index(size, family))
    }

    /// Sets a local math-family font selector.
    pub(crate) fn set_math_family_font(&mut self, size: MathFontSize, family: u8, value: FontId) {
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
        );
    }

    /// Sets a global math-family font selector.
    pub(crate) fn set_math_family_font_global(
        &mut self,
        size: MathFontSize,
        family: u8,
        value: FontId,
    ) {
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
        );
    }

    #[must_use]
    pub fn font_dimen(&self, font: FontId, number: u32) -> Scaled {
        let Ok(index) = font_dimen_index(font, number) else {
            return Scaled::from_raw(0);
        };
        Scaled::from_raw(decode_i32(font_bank_word(&self.font_dimens, index)))
    }

    pub(crate) fn set_font_dimen_global(&mut self, index: u32, value: Scaled) {
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
        );
    }

    #[must_use]
    pub fn font_param_len(&self, font: FontId) -> u32 {
        decode_u32(font_bank_word(&self.font_param_lens, font.raw()))
    }

    pub(crate) fn set_font_param_len_global(&mut self, font: FontId, value: u32) {
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

    pub(crate) fn set_font_hyphen_char_global(&mut self, font: FontId, value: i32) {
        self.set_font_int_bank(BankTag::FontHyphenChar, font, value, true);
    }

    #[must_use]
    pub fn font_skew_char(&self, font: FontId) -> i32 {
        decode_i32(font_bank_word(&self.font_skew_chars, font.raw()))
    }

    pub(crate) fn set_font_skew_char_global(&mut self, font: FontId, value: i32) {
        self.set_font_int_bank(BankTag::FontSkewChar, font, value, true);
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
    ) {
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
        );
    }

    pub(crate) fn pdf_no_ligatures(&self, font: FontId) -> bool {
        font_bank_word(&self.pdf_no_ligatures, font.raw()) != 0
    }

    pub(crate) fn set_pdf_no_ligatures_global(&mut self, font: FontId) {
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
        );
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

#[inline]
pub(crate) fn barrier(
    cell_slot: &mut u64,
    stamp_slot: &mut Epoch,
    journal: &mut Journal,
    #[cfg(feature = "shadow")] shadow: &mut AHashMap<CellId, u64>,
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
