#[cfg(feature = "shadow")]
use super::shadow_set;
use super::{
    Env, RegisterBank, SEGMENT_LEN, font_bank_word, is_dense_register, register_index,
    segment_index, segment_offset, u16_index,
};
use crate::cell::{BankTag, CellId};
use crate::env::banks::{
    BankCodec, DimenParam, GlueParam, IntParam, OptionalTokenListIdCodec, TokParam,
};
use crate::epoch::Epoch;
use crate::glue::GlueSpecRef;
use crate::ids::NodeListId;
use crate::macro_store::MacroDefinitionRef;
use crate::node_arena::NodeListRef;
use crate::token_store::TokenListRef;

impl Env {
    /// Applies one hidden semantic write that persists across group exit while
    /// remaining rollback-visible to aggregate checkpoints.
    ///
    /// This is not a TeX assignment and creates no save-stack word. Encoding
    /// it as a global undo record reuses the journal's established ordering,
    /// group-refiling, and snapshot rollback semantics without manufacturing a
    /// local restoration edge. Stores must atomically fold the returned value
    /// into its aggregate exact-identity owner.
    pub(crate) fn restore_raw_global(
        &mut self,
        cell: CellId,
        word: u64,
        token_root: Option<TokenListRef>,
        macro_root: Option<MacroDefinitionRef>,
        glue_root: Option<GlueSpecRef>,
        box_root: Option<NodeListRef>,
    ) -> super::CellMutationReceipt {
        let cell = cell.without_assignment_scope();
        let old = self.semantic_word(cell);
        let receipt = super::CellMutationReceipt::restore(cell, old, word, false);
        if old == word {
            return receipt;
        }
        let pos = self.journal.push_undo(crate::journal::UndoRec::new(
            CellId::new_global(cell.bank(), cell.index()),
            old,
            word,
        ));
        if matches!(cell.bank(), BankTag::Toks | BankTag::TokParam) {
            self.journal.attach_token_undo_roots(
                pos,
                crate::journal::TokenUndoRoots::new(self.token_root(cell), token_root),
            );
        } else if cell.bank() == BankTag::Meaning {
            self.journal.attach_macro_undo_roots(
                pos,
                crate::journal::MacroUndoRoots::new(self.macro_root(cell), macro_root),
            );
        } else if matches!(
            cell.bank(),
            BankTag::Skip | BankTag::Muskip | BankTag::GlueParam
        ) {
            self.journal.attach_glue_undo_roots(
                pos,
                crate::journal::GlueUndoRoots::new(self.glue_root(cell), glue_root),
            );
        } else {
            assert!(
                token_root.is_none(),
                "non-token raw write carried token owner"
            );
            assert!(
                macro_root.is_none(),
                "non-meaning raw write carried macro owner"
            );
            assert!(glue_root.is_none(), "non-glue raw write carried glue owner");
            assert!(box_root.is_none(), "non-box raw write carried box owner");
        }
        self.restore_raw_with_owners(cell, word, token_root, macro_root, glue_root, box_root);
        receipt
    }

    /// Restore-only raw write primitive for journal rollback and group walks.
    ///
    /// This deliberately bypasses the write barrier and does not append to the
    /// journal. It must only be used while replaying existing journal records;
    /// semantic assignments must go through the typed `set*` APIs so the
    /// single write path records history.
    #[allow(dead_code)]
    pub(crate) fn restore_raw(&mut self, cell: CellId, word: u64) {
        self.restore_raw_with_owners(cell, word, None, None, None, None);
    }

    pub(crate) fn restore_raw_with_owners(
        &mut self,
        cell: CellId,
        word: u64,
        token_root: Option<TokenListRef>,
        macro_root: Option<MacroDefinitionRef>,
        glue_root: Option<GlueSpecRef>,
        box_root: Option<NodeListRef>,
    ) {
        match cell.bank() {
            BankTag::Meaning => self.restore_meaning_word(cell.index(), word),
            BankTag::Count => self.restore_register(cell.index(), word, RegisterBank::Count),
            BankTag::Dimen => self.restore_register(cell.index(), word, RegisterBank::Dimen),
            BankTag::Skip => self.restore_register(cell.index(), word, RegisterBank::Skip),
            BankTag::Toks => self.restore_register(cell.index(), word, RegisterBank::Toks),
            BankTag::Box => {
                self.boxes
                    .restore_value(u16_index(cell.index()), word, box_root.clone())
            }
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
            BankTag::PdfLpCode => {
                restore_font_bank_word(&mut self.pdf_lp_codes, cell.index(), word)
            }
            BankTag::PdfRpCode => {
                restore_font_bank_word(&mut self.pdf_rp_codes, cell.index(), word)
            }
            BankTag::PdfEfCode => {
                restore_font_bank_word(&mut self.pdf_ef_codes, cell.index(), word)
            }
            BankTag::PdfTagCode => {
                restore_font_bank_word(&mut self.pdf_tag_codes, cell.index(), word)
            }
            BankTag::PdfKnbsCode => {
                restore_font_bank_word(&mut self.pdf_knbs_codes, cell.index(), word)
            }
            BankTag::PdfStbsCode => {
                restore_font_bank_word(&mut self.pdf_stbs_codes, cell.index(), word)
            }
            BankTag::PdfShbsCode => {
                restore_font_bank_word(&mut self.pdf_shbs_codes, cell.index(), word)
            }
            BankTag::PdfKnbcCode => {
                restore_font_bank_word(&mut self.pdf_knbc_codes, cell.index(), word)
            }
            BankTag::PdfKnacCode => {
                restore_font_bank_word(&mut self.pdf_knac_codes, cell.index(), word)
            }
            BankTag::PdfNoLigatures => {
                restore_font_bank_word(&mut self.pdf_no_ligatures, cell.index(), word)
            }
            BankTag::CurrentFont => self.current_font.word = word,
            BankTag::MathFamilyFont => self
                .math_family_fonts
                .restore_word(u16_index(cell.index()), word),
        }
        if matches!(cell.bank(), BankTag::Toks | BankTag::TokParam) {
            let expected = match cell.bank() {
                BankTag::Toks => Some(word as u32),
                BankTag::TokParam if word != 0 => Some((word - 1) as u32),
                BankTag::TokParam => None,
                _ => unreachable!(),
            };
            assert_eq!(
                token_root.as_ref().map(|root| root.id().raw()),
                expected,
                "raw token word and strong owner diverged"
            );
            self.set_token_root(cell, token_root);
            assert!(macro_root.is_none(), "token write carried macro owner");
            assert!(glue_root.is_none(), "token write carried glue owner");
            assert!(box_root.is_none(), "token write carried box owner");
        } else if cell.bank() == BankTag::Meaning {
            let expected = match crate::meaning::Meaning::decode_stored(word) {
                crate::meaning::Meaning::Macro { definition, .. } => Some(definition),
                _ => None,
            };
            assert_eq!(
                macro_root.as_ref().map(MacroDefinitionRef::raw),
                expected.map(|definition| definition.raw()),
                "raw meaning word and strong macro owner diverged"
            );
            self.set_macro_root(cell, macro_root);
            assert!(token_root.is_none(), "meaning write carried token owner");
            assert!(glue_root.is_none(), "meaning write carried glue owner");
            assert!(box_root.is_none(), "meaning write carried box owner");
        } else if matches!(
            cell.bank(),
            BankTag::Skip | BankTag::Muskip | BankTag::GlueParam
        ) {
            assert_eq!(
                glue_root.as_ref().map(|root| root.id().raw()).unwrap_or(0),
                word as u32,
                "raw glue word and strong owner diverged"
            );
            self.set_glue_root(cell, glue_root);
            assert!(token_root.is_none(), "glue write carried token owner");
            assert!(macro_root.is_none(), "glue write carried macro owner");
            assert!(box_root.is_none(), "glue write carried box owner");
        } else if cell.bank() == BankTag::Box {
            assert_eq!(
                box_root.as_ref().map(NodeListRef::id),
                NodeListId::decode_box_word(word),
                "raw box word and strong owner diverged"
            );
            assert!(token_root.is_none(), "box write carried token owner");
            assert!(macro_root.is_none(), "box write carried macro owner");
            assert!(glue_root.is_none(), "box write carried glue owner");
        } else {
            assert!(
                token_root.is_none(),
                "non-token raw write carried token owner"
            );
            assert!(
                macro_root.is_none(),
                "non-meaning raw write carried macro owner"
            );
            assert!(glue_root.is_none(), "non-glue raw write carried glue owner");
            assert!(box_root.is_none(), "non-box raw write carried box owner");
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
            BankTag::Meaning => self.get_meaning_slot(index).encode(),
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
            BankTag::TokParam => OptionalTokenListIdCodec::encode(
                self.tok_param_option(TokParam::new(u16_index(index))),
            ),
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
            BankTag::PdfLpCode => font_bank_word(&self.pdf_lp_codes, index),
            BankTag::PdfRpCode => font_bank_word(&self.pdf_rp_codes, index),
            BankTag::PdfEfCode => font_bank_word(&self.pdf_ef_codes, index),
            BankTag::PdfTagCode => font_bank_word(&self.pdf_tag_codes, index),
            BankTag::PdfKnbsCode => font_bank_word(&self.pdf_knbs_codes, index),
            BankTag::PdfStbsCode => font_bank_word(&self.pdf_stbs_codes, index),
            BankTag::PdfShbsCode => font_bank_word(&self.pdf_shbs_codes, index),
            BankTag::PdfKnbcCode => font_bank_word(&self.pdf_knbc_codes, index),
            BankTag::PdfKnacCode => font_bank_word(&self.pdf_knac_codes, index),
            BankTag::PdfNoLigatures => font_bank_word(&self.pdf_no_ligatures, index),
            BankTag::CurrentFont => self.current_font.word,
            BankTag::MathFamilyFont => {
                u64::from(self.math_family_fonts.get(u16_index(index)).raw())
            }
        }
    }

    /// Returns the cell's effective semantic word when it differs from its
    /// virtual default.
    ///
    /// Meaning and box banks use nonzero sentinels for their absent values;
    /// every other Env bank uses zero. Exact live-state identity must apply
    /// the same sparse predicate as `for_each_semantic_non_default_word` when
    /// a typed mutation removes one current cell.
    pub(crate) fn semantic_non_default_word(&self, cell: CellId) -> Option<u64> {
        let word = self.semantic_word(cell);
        let non_default = match cell.bank() {
            BankTag::Meaning => {
                crate::meaning::Meaning::decode_stored(word) != crate::meaning::Meaning::Undefined
            }
            BankTag::Box => word != NodeListId::encode_box_word(None),
            _ => word != 0,
        };
        non_default.then_some(word)
    }

    /// Returns the effective value visible after a save-stack restoration.
    ///
    /// A loaded-format journal may use the default word to remove a mutable
    /// overlay. In that representation the restored value lives only in the
    /// immutable format base, so TeX82 §283's following `show_eqtb` must not
    /// interpret the undo word itself as the restored semantic value.
    pub(crate) fn restored_semantic_word(
        &self,
        cell: CellId,
        journal_word: u64,
    ) -> RestoredSemanticWord {
        if journal_word != 0 {
            return RestoredSemanticWord {
                word: journal_word,
                trace_eligible: true,
            };
        }
        let cell = CellId::new(cell.bank(), cell.index());
        let format_word = self
            .format_base
            .binary_search_by_key(&cell.raw(), |entry| entry.cell.raw())
            .ok()
            .map(|index| self.format_base[index].word);
        RestoredSemanticWord {
            word: format_word.unwrap_or(journal_word),
            // `par_shape_loc` is represented by a private token-list cell in
            // Umber.  Its absent state has no TeX eqtb entry/save-stack word
            // to show. A frozen format-base cell, however, is TeX's
            // level-one value even when the mutable overlay's undo word is
            // zero, and §283 must trace that restored value through §252.
            trace_eligible: cell.bank() != BankTag::TokParam
                || cell.index() != u32::from(super::banks::TokParam::PAR_SHAPE_INTERNAL.raw())
                || format_word.is_some(),
        }
    }

    pub(crate) fn for_each_semantic_non_default_word(&self, mut f: impl FnMut(CellId, u64)) {
        for (segment_index, segment) in self.meaning_cells.iter().enumerate() {
            let Some(segment) = segment else {
                continue;
            };
            for (offset, &meaning) in segment.iter().enumerate() {
                if meaning != crate::meaning::Meaning::Undefined {
                    let index = ((segment_index as u32) << super::SEGMENT_BITS) | offset as u32;
                    f(CellId::new(BankTag::Meaning, index), meaning.encode());
                }
            }
        }
        self.counts
            .for_each_non_default_word(BankTag::Count, &mut f);
        self.dimens
            .for_each_non_default_word(BankTag::Dimen, &mut f);
        self.skips.for_each_non_default_word(BankTag::Skip, &mut f);
        self.toks.for_each_non_default_word(BankTag::Toks, &mut f);
        self.boxes.for_each_non_default_word(|index, word| {
            f(CellId::new(BankTag::Box, u32::from(index)), word)
        });
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
        for_each_font_bank_word(BankTag::PdfLpCode, &self.pdf_lp_codes, &mut f);
        for_each_font_bank_word(BankTag::PdfRpCode, &self.pdf_rp_codes, &mut f);
        for_each_font_bank_word(BankTag::PdfEfCode, &self.pdf_ef_codes, &mut f);
        for_each_font_bank_word(BankTag::PdfTagCode, &self.pdf_tag_codes, &mut f);
        for_each_font_bank_word(BankTag::PdfKnbsCode, &self.pdf_knbs_codes, &mut f);
        for_each_font_bank_word(BankTag::PdfStbsCode, &self.pdf_stbs_codes, &mut f);
        for_each_font_bank_word(BankTag::PdfShbsCode, &self.pdf_shbs_codes, &mut f);
        for_each_font_bank_word(BankTag::PdfKnbcCode, &self.pdf_knbc_codes, &mut f);
        for_each_font_bank_word(BankTag::PdfKnacCode, &self.pdf_knac_codes, &mut f);
        for_each_font_bank_word(BankTag::PdfNoLigatures, &self.pdf_no_ligatures, &mut f);
        if self.current_font.word != 0 {
            f(CellId::new(BankTag::CurrentFont, 0), self.current_font.word);
        }
    }

    /// Visits only environment cells that root TeX82 main-memory objects.
    ///
    /// Numeric, glue, font, and pdfTeX font-code banks do not own token,
    /// macro, or node-list reachability. Keeping this diagnostic traversal
    /// separate avoids scanning those large typed banks at every transient
    /// node allocation.
    pub(crate) fn for_each_main_memory_root_word(&self, mut f: impl FnMut(CellId, u64)) {
        for (segment_index, segment) in self.meaning_cells.iter().enumerate() {
            let Some(segment) = segment else {
                continue;
            };
            for (offset, &meaning) in segment.iter().enumerate() {
                if meaning != crate::meaning::Meaning::Undefined {
                    let index = ((segment_index as u32) << super::SEGMENT_BITS) | offset as u32;
                    f(CellId::new(BankTag::Meaning, index), meaning.encode());
                }
            }
        }
        self.toks.for_each_non_default_word(BankTag::Toks, &mut f);
        self.boxes.for_each_non_default_word(|index, word| {
            f(CellId::new(BankTag::Box, u32::from(index)), word)
        });
        self.overflow_toks
            .for_each_non_default_word(BankTag::Toks, &mut f);
        self.tok_params
            .for_each_non_default_word(BankTag::TokParam, &mut f);
    }

    pub(super) fn meaning_value(&self, index: u32) -> Option<crate::meaning::Meaning> {
        let segment = segment_index(index);
        let offset = segment_offset(index);
        self.meaning_cells
            .get(segment)
            .and_then(Option::as_ref)
            .map(|cells| cells[offset])
    }

    pub(super) fn set_meaning_value(
        &mut self,
        index: u32,
        meaning: crate::meaning::Meaning,
        global: bool,
    ) -> super::CellMutationReceipt {
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

        let old = cells[offset];
        let old_word = old.encode();
        let new_word = meaning.encode();
        let receipt = super::CellMutationReceipt::write(cell, old_word, new_word);
        if old == meaning {
            if cell.is_global() {
                self.journal
                    .push_undo(crate::journal::UndoRec::new(cell, old_word, new_word));
            }
            return receipt;
        }
        if stamps[offset] < self.epoch {
            self.journal
                .push_undo(crate::journal::UndoRec::new(cell, old_word, new_word));
            stamps[offset] = self.epoch;
        } else if cell.is_global() {
            self.journal
                .push_undo(crate::journal::UndoRec::new(cell, old_word, new_word));
        }
        cells[offset] = meaning;
        #[cfg(feature = "shadow")]
        shadow_set(
            &mut self.shadow,
            CellId::new(cell.bank(), cell.index()),
            new_word,
        );
        receipt
    }

    fn ensure_meaning_segment(&mut self, index: u32) {
        let required_len = segment_index(index) + 1;
        if self.meaning_cells.len() < required_len {
            self.meaning_cells.resize_with(required_len, || None);
            self.meaning_stamps.resize_with(required_len, || None);
        }
        let segment = required_len - 1;
        if self.meaning_cells[segment].is_none() {
            self.meaning_cells[segment] =
                Some(vec![crate::meaning::Meaning::Undefined; SEGMENT_LEN].into_boxed_slice());
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
            .expect("ensured meaning segment")[offset] =
            crate::meaning::Meaning::decode_stored(word);
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
                RegisterBank::Muskip => self.muskips.restore_word(index, word),
            }
        } else {
            match bank {
                RegisterBank::Count => self.overflow_counts.restore_word(index, word),
                RegisterBank::Dimen => self.overflow_dimens.restore_word(index, word),
                RegisterBank::Skip => self.overflow_skips.restore_word(index, word),
                RegisterBank::Toks => self.overflow_toks.restore_word(index, word),
                RegisterBank::Muskip => self.overflow_muskips.restore_word(index, word),
            }
        }
    }
}

pub(crate) struct RestoredSemanticWord {
    pub(crate) word: u64,
    pub(crate) trace_eligible: bool,
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
