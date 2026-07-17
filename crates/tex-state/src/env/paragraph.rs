//! Cheap paragraph mutation identity and journal-derived root survivor redo.

use super::Env;
use crate::cell::{BankTag, CellId};
use crate::env::banks::IntParam;
use crate::journal::{Entry, JournalPos};
use crate::{PureParagraphMutation, PureParagraphMutationSummary};
use ahash::{AHashMap, RandomState};
use std::hash::{BuildHasher, Hasher};

const COUNT_INT_HASH_KEYS: [u64; 4] = [
    0x7061_7261_5f63_6f75,
    0x6e74_5f69_6e74_5f76,
    0x315f_6669_7865_645f,
    0x7365_6564_735f_3634,
];

fn count_int_hash_state() -> RandomState {
    RandomState::with_seeds(
        COUNT_INT_HASH_KEYS[0],
        COUNT_INT_HASH_KEYS[1],
        COUNT_INT_HASH_KEYS[2],
        COUNT_INT_HASH_KEYS[3],
    )
}

/// Opaque start position for one paragraph's surviving environment writes.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ParagraphMutationCheckpoint {
    journal_pos: JournalPos,
    entry_fingerprint: u64,
}

impl Env {
    /// Returns a cached identity for all count registers and integer parameters.
    pub(crate) fn count_int_fingerprint(&mut self) -> u64 {
        if let Some(fingerprint) = self.count_int_fingerprint {
            return fingerprint;
        }

        let mut hasher = count_int_hash_state().build_hasher();
        hasher.write(b"umber-paragraph-count-int-v1");
        let mut write_cell = |cell: CellId, word: u64| {
            hasher.write_u64(cell.raw());
            hasher.write_u64(word);
        };
        self.counts
            .for_each_non_default_word(BankTag::Count, &mut write_cell);
        self.overflow_counts
            .for_each_non_default_word(BankTag::Count, &mut write_cell);
        self.int_params
            .for_each_non_default_word(BankTag::IntParam, &mut write_cell);
        let fingerprint = hasher.finish();
        self.count_int_fingerprint = Some(fingerprint);
        fingerprint
    }

    /// Captures paragraph entry identity and opens a fresh journal epoch.
    pub(crate) fn begin_paragraph_mutations(&mut self) -> ParagraphMutationCheckpoint {
        let checkpoint = ParagraphMutationCheckpoint {
            journal_pos: self.current_journal_pos(),
            entry_fingerprint: self.count_int_fingerprint(),
        };
        // The barrier records only the first local write in an epoch. A fresh
        // epoch makes every root-surviving paragraph write visible after the
        // captured journal position without adding work to individual setters.
        self.epoch.bump();
        checkpoint
    }

    /// Derives the compact final-value redo for count/int writes still visible
    /// in the paragraph's journal suffix after group compaction.
    pub(crate) fn finish_paragraph_mutations(
        &mut self,
        checkpoint: ParagraphMutationCheckpoint,
    ) -> PureParagraphMutationSummary {
        let exit_fingerprint = self.count_int_fingerprint();
        if self.current_journal_pos() < checkpoint.journal_pos {
            return PureParagraphMutationSummary {
                entry_fingerprint: checkpoint.entry_fingerprint,
                exit_fingerprint,
                journal_rewound: true,
                mutations: Vec::new(),
            };
        }

        let mut cells: Vec<(CellId, u64, bool)> = Vec::new();
        let mut positions: AHashMap<CellId, usize, RandomState> =
            AHashMap::with_hasher(count_int_hash_state());
        for entry in self.journal_entries_since(checkpoint.journal_pos) {
            let Entry::Undo(rec) = entry else {
                continue;
            };
            let cell = rec.cell();
            if !matches!(cell.bank(), BankTag::Count | BankTag::IntParam) {
                continue;
            }
            let cell = CellId::new(cell.bank(), cell.index());
            if let Some(&position) = positions.get(&cell) {
                if rec.cell().is_global() {
                    cells[position].2 = true;
                }
            } else {
                positions.insert(cell, cells.len());
                cells.push((cell, rec.old(), rec.cell().is_global()));
            }
        }

        let mutations = cells
            .into_iter()
            .map(|(cell, expected, global)| match cell.bank() {
                BankTag::Count => PureParagraphMutation::Count {
                    index: u16::try_from(cell.index()).expect("count register index fits u16"),
                    expected: expected as u32 as i32,
                    value: self.semantic_word(cell) as u32 as i32,
                    global,
                },
                BankTag::IntParam => PureParagraphMutation::IntParam {
                    param: IntParam::new(
                        u16::try_from(cell.index()).expect("integer parameter index fits u16"),
                    ),
                    expected: expected as u32 as i32,
                    value: self.semantic_word(cell) as u32 as i32,
                    global,
                },
                _ => unreachable!("filtered count/int paragraph survivor"),
            })
            .collect();

        PureParagraphMutationSummary {
            entry_fingerprint: checkpoint.entry_fingerprint,
            exit_fingerprint,
            journal_rewound: false,
            mutations,
        }
    }
}
