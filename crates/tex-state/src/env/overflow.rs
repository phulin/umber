//! Sparse e-TeX register overflow banks.

use crate::cell::{BankTag, CellId};
use crate::env::banks::{BankCodec, DENSE_REGISTER_COUNT};
use crate::env::barrier;
use crate::epoch::Epoch;
use crate::journal::{Journal, UndoRec};
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
            .map_or(0, |page| page.values[offset]);
        C::decode(word)
    }

    pub(crate) fn set(
        &mut self,
        index: u16,
        value: C::Value,
        journal: &mut Journal,
        #[cfg(feature = "shadow")] shadow: &mut std::collections::HashMap<CellId, u64>,
        epoch: Epoch,
        bank: BankTag,
        global: bool,
    ) {
        let (page, offset) = sparse_location(index);
        let page = self.pages[page].get_or_insert_with(|| Box::new(Page::default()));
        let cell_id = if global {
            CellId::new_global(bank, u32::from(index))
        } else {
            CellId::new(bank, u32::from(index))
        };
        barrier(
            &mut page.values[offset],
            &mut page.stamps[offset],
            journal,
            #[cfg(feature = "shadow")]
            shadow,
            epoch,
            cell_id,
            C::encode(value),
        );
    }

    pub(crate) fn set_always_journal(
        &mut self,
        index: u16,
        value: C::Value,
        journal: &mut Journal,
        #[cfg(feature = "shadow")] shadow: &mut std::collections::HashMap<CellId, u64>,
        bank: BankTag,
        global: bool,
    ) -> Option<UndoRec> {
        let (page, offset) = sparse_location(index);
        let page = self.pages[page].get_or_insert_with(|| Box::new(Page::default()));
        let cell_id = if global {
            CellId::new_global(bank, u32::from(index))
        } else {
            CellId::new(bank, u32::from(index))
        };
        let old = page.values[offset];
        let new = C::encode(value);
        if old == new && !global {
            return None;
        }
        let rec = UndoRec::new(cell_id, old, new);
        journal.push_undo(rec);
        page.values[offset] = new;
        #[cfg(feature = "shadow")]
        crate::env::shadow_set(shadow, CellId::new(bank, u32::from(index)), new);
        Some(rec)
    }

    #[allow(dead_code)]
    pub(crate) fn restore_word(&mut self, index: u16, word: u64) {
        let (page, offset) = sparse_location(index);
        let Some(sparse_page) = self.pages[page].as_mut() else {
            if word != 0 {
                let mut sparse_page = Box::new(Page::default());
                sparse_page.values[offset] = word;
                self.pages[page] = Some(sparse_page);
            }
            return;
        };

        sparse_page.values[offset] = word;
        if sparse_page.is_all_default() {
            self.pages[page] = None;
        }
    }

    #[cfg(any(test, feature = "testing", feature = "shadow"))]
    pub(crate) fn non_default_words(&self, bank: BankTag, out: &mut Vec<(CellId, u64)>) {
        for (page_index, page) in self.pages.iter().enumerate() {
            let Some(page) = page else {
                continue;
            };
            for (offset, &word) in page.values.iter().enumerate() {
                if word != 0 {
                    let index = DENSE_REGISTER_COUNT as u32
                        + (page_index as u32 * PAGE_LEN as u32)
                        + offset as u32;
                    out.push((CellId::new(bank, index), word));
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

impl Default for Page {
    fn default() -> Self {
        Self {
            values: [0; PAGE_LEN],
            stamps: [Epoch::ZERO; PAGE_LEN],
        }
    }
}

impl Page {
    fn is_all_default(&self) -> bool {
        self.values.iter().all(|&word| word == 0)
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
