//! Sparse e-TeX register overflow banks.

use crate::cell::CellId;
use crate::env::banks::{
    BankCodec, BankJournalContext, BankSetContext, BoxWriteOutcome, DENSE_REGISTER_COUNT,
};
use crate::env::barrier;
use crate::epoch::Epoch;
use crate::journal::UndoRec;
use core::array;
use core::marker::PhantomData;

pub(crate) const REGISTER_COUNT: u16 = 32_768;
const PAGE_BITS: u16 = 8;
const PAGE_LEN: usize = 1 << PAGE_BITS;
const PAGE_COUNT: usize = 128;
const PAGE_MASK: u16 = (PAGE_LEN as u16) - 1;

#[derive(Clone, Debug)]
pub(crate) struct SparseBank<C> {
    pages: [Option<Box<Page>>; PAGE_COUNT],
    _codec: PhantomData<C>,
}

#[derive(Clone, Debug)]
struct Page {
    values: [u64; PAGE_LEN],
    stamps: [Epoch; PAGE_LEN],
}

impl<C> SparseBank<C>
where
    C: BankCodec,
{
    pub(crate) fn new() -> Self {
        Self {
            pages: array::from_fn(|_| None),
            _codec: PhantomData,
        }
    }

    pub(crate) fn get(&self, index: u16) -> C::Value {
        let (page, offset) = sparse_location(index);
        let word = self.pages[page]
            .as_ref()
            .map_or(C::DEFAULT_WORD, |page| page.values[offset]);
        C::decode(word)
    }

    pub(crate) fn set(&mut self, index: u16, value: C::Value, ctx: BankSetContext<'_>) {
        let (page, offset) = sparse_location(index);
        let page = self.pages[page].get_or_insert_with(|| Box::new(Page::new(C::DEFAULT_WORD)));
        let cell_id = if ctx.global {
            CellId::new_global(ctx.bank, u32::from(index))
        } else {
            CellId::new(ctx.bank, u32::from(index))
        };
        barrier(
            &mut page.values[offset],
            &mut page.stamps[offset],
            ctx.journal,
            #[cfg(feature = "shadow")]
            ctx.shadow,
            ctx.epoch,
            cell_id,
            C::encode(value),
        );
    }

    pub(crate) fn set_always_journal(
        &mut self,
        index: u16,
        value: C::Value,
        ctx: BankJournalContext<'_>,
    ) -> BoxWriteOutcome {
        let (page, offset) = sparse_location(index);
        let page = self.pages[page].get_or_insert_with(|| Box::new(Page::new(C::DEFAULT_WORD)));
        let cell_id = if ctx.global {
            CellId::new_global(ctx.bank, u32::from(index))
        } else {
            CellId::new(ctx.bank, u32::from(index))
        };
        let old = page.values[offset];
        let new = C::encode(value);
        if old == new && !ctx.global {
            return BoxWriteOutcome::Unchanged;
        }
        let rec = UndoRec::new(cell_id, old, new);
        let outcome = if ctx.global {
            let pos = ctx.journal.push_undo(rec);
            BoxWriteOutcome::Journaled { rec, pos }
        } else if let Some(pos) = ctx.coalesce_pos {
            ctx.journal.replace_undo_new_value(pos, new);
            BoxWriteOutcome::Coalesced { displaced: old }
        } else {
            let pos = ctx.journal.push_undo(rec);
            BoxWriteOutcome::Journaled { rec, pos }
        };
        page.values[offset] = new;
        #[cfg(feature = "shadow")]
        crate::env::shadow_set(ctx.shadow, CellId::new(ctx.bank, u32::from(index)), new);
        outcome
    }

    #[allow(dead_code)]
    pub(crate) fn restore_word(&mut self, index: u16, word: u64) {
        let (page, offset) = sparse_location(index);
        let Some(sparse_page) = self.pages[page].as_mut() else {
            if word != C::DEFAULT_WORD {
                let mut sparse_page = Box::new(Page::new(C::DEFAULT_WORD));
                sparse_page.values[offset] = word;
                self.pages[page] = Some(sparse_page);
            }
            return;
        };

        sparse_page.values[offset] = word;
        if sparse_page.is_all_default(C::DEFAULT_WORD) {
            self.pages[page] = None;
        }
    }

    pub(crate) fn for_each_non_default_word(
        &self,
        bank: crate::cell::BankTag,
        mut f: impl FnMut(CellId, u64),
    ) {
        for (page_index, page) in self.pages.iter().enumerate() {
            let Some(page) = page else {
                continue;
            };
            for (offset, &word) in page.values.iter().enumerate() {
                if word != C::DEFAULT_WORD {
                    let index = DENSE_REGISTER_COUNT as u32
                        + (page_index as u32 * PAGE_LEN as u32)
                        + offset as u32;
                    f(CellId::new(bank, index), word);
                }
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn has_page_for(&self, index: u16) -> bool {
        let (page, _) = sparse_location(index);
        self.pages[page].is_some()
    }
}

impl<C> Default for SparseBank<C>
where
    C: BankCodec,
{
    fn default() -> Self {
        Self::new()
    }
}

impl Page {
    fn new(default_word: u64) -> Self {
        Self {
            values: [default_word; PAGE_LEN],
            stamps: [Epoch::ZERO; PAGE_LEN],
        }
    }

    fn is_all_default(&self, default_word: u64) -> bool {
        self.values.iter().all(|&word| word == default_word)
    }
}

fn sparse_location(index: u16) -> (usize, usize) {
    assert!(
        (DENSE_REGISTER_COUNT as u16..REGISTER_COUNT).contains(&index),
        "register index out of sparse overflow range"
    );
    let sparse = index - DENSE_REGISTER_COUNT as u16;
    (
        (sparse >> PAGE_BITS) as usize,
        (sparse & PAGE_MASK) as usize,
    )
}
