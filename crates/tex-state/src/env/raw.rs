#[cfg(feature = "shadow")]
use super::shadow_set;
use super::{
    Env, RegisterBank, SEGMENT_LEN, barrier, is_dense_register, register_index, segment_index,
    segment_offset, u16_index,
};
use crate::cell::{BankTag, CellId};
use crate::env::banks::{DimenParam, GlueParam, IntParam, TokParam};
use crate::epoch::Epoch;
use crate::ids::NodeListId;
use crate::interner::Symbol;
#[cfg(any(test, feature = "testing", feature = "shadow"))]
use std::hash::{Hash as _, Hasher};

impl Env {
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
            BankTag::Muskip => self.restore_register(cell.index(), word, RegisterBank::Muskip),
            BankTag::IntParam => self.int_params.restore_word(u16_index(cell.index()), word),
            BankTag::DimenParam => self
                .dimen_params
                .restore_word(u16_index(cell.index()), word),
            BankTag::GlueParam => self.glue_params.restore_word(u16_index(cell.index()), word),
            BankTag::TokParam => self.tok_params.restore_word(u16_index(cell.index()), word),
            BankTag::FontDimen => restore_font_bank_word(&mut self.font_dimens, cell.index(), word),
            BankTag::FontParamLen => {
                restore_font_bank_word(&mut self.font_param_lens, cell.index(), word);
            }
            BankTag::FontHyphenChar => {
                restore_font_bank_word(&mut self.font_hyphen_chars, cell.index(), word);
            }
            BankTag::FontSkewChar => {
                restore_font_bank_word(&mut self.font_skew_chars, cell.index(), word);
            }
            BankTag::CurrentFont => self.current_font.word = word,
            BankTag::MathFamilyFont => self
                .math_family_fonts
                .restore_word(u16_index(cell.index()), word),
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
        self.for_each_semantic_non_default_word(|cell, real_word| {
            let shadow_word = self.shadow.get(&cell).copied().unwrap_or(0);
            assert_eq!(
                shadow_word, real_word,
                "shadow mismatch at {cell:?}: shadow={shadow_word} real={real_word}"
            );
        });
        for (&cell, &shadow_word) in &self.shadow {
            let real_word = self.semantic_word(cell);
            assert_eq!(
                shadow_word, real_word,
                "shadow mismatch at {cell:?}: shadow={shadow_word} real={real_word}"
            );
        }
    }

    pub(crate) fn semantic_word(&self, cell: CellId) -> u64 {
        let index = cell.index();
        match cell.bank() {
            BankTag::Meaning => self.get(Symbol::new(index)).encode(),
            BankTag::Count => u64::from(self.count(u16_index(index)) as u32),
            BankTag::Dimen => u64::from(self.dimen(u16_index(index)).raw() as u32),
            BankTag::Skip => u64::from(self.skip(u16_index(index)).raw()),
            BankTag::Toks => u64::from(self.toks(u16_index(index)).raw()),
            BankTag::Box => NodeListId::encode_box_word(self.box_reg(u16_index(index))),
            BankTag::Muskip => u64::from(self.muskip(u16_index(index)).raw()),
            BankTag::IntParam => u64::from(self.int_param(IntParam::new(u16_index(index))) as u32),
            BankTag::DimenParam => {
                u64::from(self.dimen_param(DimenParam::new(u16_index(index))).raw() as u32)
            }
            BankTag::GlueParam => {
                u64::from(self.glue_param(GlueParam::new(u16_index(index))).raw())
            }
            BankTag::TokParam => u64::from(self.tok_param(TokParam::new(u16_index(index))).raw()),
            BankTag::FontDimen => self.font_dimens.get(&index).map_or(0, |entry| entry.word),
            BankTag::FontParamLen => self
                .font_param_lens
                .get(&index)
                .map_or(0, |entry| entry.word),
            BankTag::FontHyphenChar => self
                .font_hyphen_chars
                .get(&index)
                .map_or(0, |entry| entry.word),
            BankTag::FontSkewChar => self
                .font_skew_chars
                .get(&index)
                .map_or(0, |entry| entry.word),
            BankTag::CurrentFont => self.current_font.word,
            BankTag::MathFamilyFont => {
                u64::from(self.math_family_fonts.get(u16_index(index)).raw())
            }
        }
    }

    /// Returns a content-only hash of environment semantic state.
    ///
    /// The hash intentionally excludes allocation lengths, capacities, and
    /// epoch stamps; replay identity is about semantic state.
    #[cfg(any(test, feature = "testing", feature = "shadow"))]
    #[must_use]
    pub fn testing_state_hash(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.for_each_semantic_non_default_word(|cell, word| {
            cell.hash(&mut hasher);
            word.hash(&mut hasher);
        });
        self.aftergroup.hash(&mut hasher);
        self.afterassignment.hash(&mut hasher);
        hasher.finish()
    }

    pub(crate) fn for_each_semantic_non_default_word(&self, mut f: impl FnMut(CellId, u64)) {
        for (segment_index, segment) in self.meaning_cells.iter().enumerate() {
            let Some(segment) = segment else {
                continue;
            };
            for (offset, &word) in segment.iter().enumerate() {
                if word != 0 {
                    let index = ((segment_index as u32) << super::SEGMENT_BITS) | offset as u32;
                    f(CellId::new(BankTag::Meaning, index), word);
                }
            }
        }
        self.counts
            .for_each_non_default_word(BankTag::Count, &mut f);
        self.dimens
            .for_each_non_default_word(BankTag::Dimen, &mut f);
        self.skips.for_each_non_default_word(BankTag::Skip, &mut f);
        self.toks.for_each_non_default_word(BankTag::Toks, &mut f);
        self.boxes.for_each_non_default_word(BankTag::Box, &mut f);
        self.muskips
            .for_each_non_default_word(BankTag::Muskip, &mut f);
        self.overflow_counts
            .for_each_non_default_word(BankTag::Count, &mut f);
        self.overflow_dimens
            .for_each_non_default_word(BankTag::Dimen, &mut f);
        self.overflow_skips
            .for_each_non_default_word(BankTag::Skip, &mut f);
        self.overflow_toks
            .for_each_non_default_word(BankTag::Toks, &mut f);
        self.overflow_boxes
            .for_each_non_default_word(BankTag::Box, &mut f);
        self.overflow_muskips
            .for_each_non_default_word(BankTag::Muskip, &mut f);
        self.int_params
            .for_each_non_default_word(BankTag::IntParam, &mut f);
        self.dimen_params
            .for_each_non_default_word(BankTag::DimenParam, &mut f);
        self.glue_params
            .for_each_non_default_word(BankTag::GlueParam, &mut f);
        self.tok_params
            .for_each_non_default_word(BankTag::TokParam, &mut f);
        self.math_family_fonts
            .for_each_non_default_word(BankTag::MathFamilyFont, &mut f);
        for_each_font_bank_word(BankTag::FontDimen, &self.font_dimens, &mut f);
        for_each_font_bank_word(BankTag::FontParamLen, &self.font_param_lens, &mut f);
        for_each_font_bank_word(BankTag::FontHyphenChar, &self.font_hyphen_chars, &mut f);
        for_each_font_bank_word(BankTag::FontSkewChar, &self.font_skew_chars, &mut f);
        if self.current_font.word != 0 {
            f(CellId::new(BankTag::CurrentFont, 0), self.current_font.word);
        }
    }

    #[cfg(any(test, feature = "testing", feature = "shadow"))]
    pub(crate) fn testing_aftergroup_payloads(&self) -> &[crate::token::Token] {
        &self.aftergroup
    }

    #[cfg(any(test, feature = "testing", feature = "shadow"))]
    pub(crate) const fn testing_afterassignment(&self) -> Option<crate::token::Token> {
        self.afterassignment
    }

    pub(super) fn meaning_word(&self, index: u32) -> Option<u64> {
        let segment = segment_index(index);
        let offset = segment_offset(index);
        self.meaning_cells
            .get(segment)
            .and_then(Option::as_ref)
            .map(|cells| cells[offset])
    }

    pub(super) fn set_meaning_word(&mut self, symbol: Symbol, word: u64, global: bool) {
        let index = symbol.raw();
        self.ensure_meaning_segment(index);
        let segment = segment_index(index);
        let offset = segment_offset(index);
        let cells = self.meaning_cells[segment]
            .as_mut()
            .expect("ensured meaning segment");
        let stamps = self.meaning_stamps[segment]
            .as_mut()
            .expect("ensured meaning stamp segment");
        let cell = if global {
            CellId::new_global(BankTag::Meaning, index)
        } else {
            CellId::new(BankTag::Meaning, index)
        };

        barrier(
            &mut cells[offset],
            &mut stamps[offset],
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
        self.meaning_cells.resize_with(required_len, || None);
        self.meaning_stamps.resize_with(required_len, || None);
        let segment = required_len - 1;
        if self.meaning_cells[segment].is_none() {
            self.meaning_cells[segment] = Some(Box::new([0; SEGMENT_LEN]));
            self.meaning_stamps[segment] = Some(Box::new([Epoch::ZERO; SEGMENT_LEN]));
        }
    }

    #[allow(dead_code)]
    fn restore_meaning_word(&mut self, index: u32, word: u64) {
        self.ensure_meaning_segment(index);
        let segment = segment_index(index);
        let offset = segment_offset(index);
        self.meaning_cells[segment]
            .as_mut()
            .expect("ensured meaning segment")[offset] = word;
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
                RegisterBank::Muskip => self.muskips.restore_word(index, word),
            }
        } else {
            match bank {
                RegisterBank::Count => self.overflow_counts.restore_word(index, word),
                RegisterBank::Dimen => self.overflow_dimens.restore_word(index, word),
                RegisterBank::Skip => self.overflow_skips.restore_word(index, word),
                RegisterBank::Toks => self.overflow_toks.restore_word(index, word),
                RegisterBank::Box => self.overflow_boxes.restore_word(index, word),
                RegisterBank::Muskip => self.overflow_muskips.restore_word(index, word),
            }
        }
    }
}

fn restore_font_bank_word(
    map: &mut std::collections::BTreeMap<u32, super::WordStamp>,
    index: u32,
    word: u64,
) {
    if word == 0 {
        map.remove(&index);
    } else {
        map.entry(index).or_default().word = word;
    }
}

fn for_each_font_bank_word(
    bank: BankTag,
    map: &std::collections::BTreeMap<u32, super::WordStamp>,
    f: &mut impl FnMut(CellId, u64),
) {
    for (&index, entry) in map {
        if entry.word != 0 {
            f(CellId::new(bank, index), entry.word);
        }
    }
}
