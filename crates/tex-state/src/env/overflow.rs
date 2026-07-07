//! Sparse e-TeX register overflow banks.

use crate::cell::{BankTag, CellId};
use crate::env::banks::{BankCodec, DENSE_REGISTER_COUNT};
use crate::env::barrier;
use crate::epoch::Epoch;
use crate::journal::Journal;
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
            epoch,
            cell_id,
            C::encode(value),
        );
    }

    #[allow(dead_code)]
    pub(crate) fn restore_word(&mut self, index: u16, word: u64) {
        let (page, offset) = sparse_location(index);
        let page = self.pages[page].get_or_insert_with(|| Box::new(Page::default()));
        page.values[offset] = word;
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
