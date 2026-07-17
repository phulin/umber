//! Cheap paragraph mutation identity and direct root-transition recording.

use super::Env;
use crate::cell::{BankTag, CellId};
use crate::env::banks::IntParam;
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

#[derive(Clone, Copy, Debug)]
struct RecordedCell {
    cell: CellId,
    expected: u64,
    escapes: bool,
    global: bool,
}

#[derive(Clone, Debug)]
pub(super) struct ParagraphMutationRecorder {
    entry_fingerprint: u64,
    entry_group_depth: u32,
    write_observed: bool,
    cells: Vec<RecordedCell>,
    positions: AHashMap<CellId, usize, RandomState>,
}

/// Opaque proof that one environment paragraph recorder is active.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ParagraphMutationCheckpoint(());

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

    /// Captures paragraph entry identity without changing the rollback epoch.
    pub(crate) fn begin_paragraph_mutations(&mut self) -> ParagraphMutationCheckpoint {
        assert!(
            self.paragraph_mutations.is_none(),
            "paragraph mutation recorder already active"
        );
        let entry_fingerprint = self.count_int_fingerprint();
        self.paragraph_mutations = Some(ParagraphMutationRecorder {
            entry_fingerprint,
            entry_group_depth: self.group_depth,
            write_observed: false,
            cells: Vec::new(),
            positions: AHashMap::with_hasher(count_int_hash_state()),
        });
        ParagraphMutationCheckpoint(())
    }

    /// Records one count/int setter before its write barrier runs.
    pub(super) fn record_paragraph_mutation(&mut self, cell: CellId, expected: u64, global: bool) {
        let Some(recorder) = &mut self.paragraph_mutations else {
            return;
        };
        recorder.write_observed = true;
        if recorder.entry_group_depth != 0 {
            return;
        }

        let cell = CellId::new(cell.bank(), cell.index());
        let escapes = global || self.group_depth == 0;
        if let Some(&position) = recorder.positions.get(&cell) {
            let recorded = &mut recorder.cells[position];
            recorded.escapes |= escapes;
            recorded.global |= global;
            return;
        }
        recorder.positions.insert(cell, recorder.cells.len());
        recorder.cells.push(RecordedCell {
            cell,
            expected,
            escapes,
            global,
        });
    }

    /// Builds the compact final-value redo from directly observed setters.
    pub(crate) fn finish_paragraph_mutations(
        &mut self,
        _checkpoint: ParagraphMutationCheckpoint,
    ) -> PureParagraphMutationSummary {
        let recorder = self
            .paragraph_mutations
            .take()
            .expect("paragraph mutation recorder missing at finish");
        let entry_fingerprint = recorder.entry_fingerprint;
        let exit_fingerprint = self.count_int_fingerprint();
        let mutations = recorder
            .cells
            .into_iter()
            .filter(|recorded| {
                recorded.escapes && self.semantic_word(recorded.cell) != recorded.expected
            })
            .map(|recorded| match recorded.cell.bank() {
                BankTag::Count => PureParagraphMutation::Count {
                    index: u16::try_from(recorded.cell.index())
                        .expect("count register index fits u16"),
                    expected: recorded.expected as u32 as i32,
                    value: self.semantic_word(recorded.cell) as u32 as i32,
                    global: recorded.global,
                },
                BankTag::IntParam => PureParagraphMutation::IntParam {
                    param: IntParam::new(
                        u16::try_from(recorded.cell.index())
                            .expect("integer parameter index fits u16"),
                    ),
                    expected: recorded.expected as u32 as i32,
                    value: self.semantic_word(recorded.cell) as u32 as i32,
                    global: recorded.global,
                },
                _ => unreachable!("filtered count/int paragraph survivor"),
            })
            .collect();

        PureParagraphMutationSummary {
            entry_fingerprint,
            exit_fingerprint,
            unsupported_group_ownership: recorder.entry_group_depth != 0 && recorder.write_observed,
            mutations,
        }
    }

    pub(crate) fn abandon_paragraph_mutations(&mut self, _checkpoint: ParagraphMutationCheckpoint) {
        self.paragraph_mutations
            .take()
            .expect("paragraph mutation recorder missing at abandon");
    }
}
