//! Barriered environment storage.
//!
//! # Freeze theorem
//!
//! `Env` owns all mutable meaning-cell storage and its journal. All fields are
//! private, reads return decoded `Copy` values, and the API exposes no mutable
//! references into the backing arrays. Therefore `&Env` implies frozen state:
//! safe crate consumers cannot change environment cells without obtaining
//! `&mut Env` and passing through the write barrier.

use crate::cell::{BankTag, CellId};
use crate::epoch::Epoch;
use crate::interner::Symbol;
use crate::journal::{Entry, Journal, JournalPos, UndoRec};
use crate::meaning::Meaning;

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
    use crate::interner::Symbol;
    use crate::journal::{Entry, UndoRec};
    use crate::meaning::Meaning;

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
}
