#[cfg(feature = "shadow")]
use super::shadow_set;
use super::{
    Env, RegisterBank, SEGMENT_LEN, barrier, is_dense_register, register_index, segment_index,
    segment_offset, u16_index,
};
use crate::cell::{BankTag, CellId};
use crate::epoch::Epoch;
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
        let real = self.semantic_non_default_words();
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

    /// Returns a content-only hash of environment semantic state.
    ///
    /// The hash intentionally excludes allocation lengths, capacities, and
    /// epoch stamps; replay identity is about semantic state.
    #[cfg(any(test, feature = "testing", feature = "shadow"))]
    #[must_use]
    pub fn testing_state_hash(&self) -> u64 {
        let mut pairs = self.semantic_non_default_words();
        pairs.sort_by_key(|(cell, _)| *cell);

        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for (cell, word) in pairs {
            cell.hash(&mut hasher);
            word.hash(&mut hasher);
        }
        self.aftergroup.hash(&mut hasher);
        self.afterassignment.hash(&mut hasher);
        hasher.finish()
    }

    #[cfg(any(test, feature = "testing", feature = "shadow"))]
    pub(crate) fn semantic_non_default_words(&self) -> Vec<(CellId, u64)> {
        let mut out = Vec::new();
        for (segment_index, segment) in self.meaning_cells.iter().enumerate() {
            for (offset, &word) in segment.iter().enumerate() {
                if word != 0 {
                    let index = ((segment_index as u32) << super::SEGMENT_BITS) | offset as u32;
                    out.push((CellId::new(BankTag::Meaning, index), word));
                }
            }
        }
        self.counts.non_default_words(BankTag::Count, &mut out);
        self.dimens.non_default_words(BankTag::Dimen, &mut out);
        self.skips.non_default_words(BankTag::Skip, &mut out);
        self.toks.non_default_words(BankTag::Toks, &mut out);
        self.boxes.non_default_words(BankTag::Box, &mut out);
        self.muskips.non_default_words(BankTag::Muskip, &mut out);
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
        self.overflow_muskips
            .non_default_words(BankTag::Muskip, &mut out);
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
        self.meaning_cells.get(segment).map(|cells| cells[offset])
    }

    pub(super) fn set_meaning_word(&mut self, symbol: Symbol, word: u64, global: bool) {
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
