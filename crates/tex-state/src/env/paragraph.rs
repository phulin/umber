//! Cheap paragraph mutation identity and direct root-transition recording.

use super::Env;
use crate::cell::{BankTag, CellId};
use crate::env::banks::IntParam;
use crate::ids::FontId;
use crate::interner::Symbol;
use crate::{PureParagraphMutation, PureParagraphMutationSummary};
use ahash::{AHashMap, RandomState};

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
    entry_group_depth: u32,
    group_frame_mutated: bool,
    cells: Vec<RecordedCell>,
    live_group_mutations: Vec<PureParagraphMutation>,
    live_group_entry_values: AHashMap<CellId, u64, RandomState>,
    positions: AHashMap<CellId, usize, RandomState>,
}

/// Opaque proof that one environment paragraph recorder is active.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ParagraphMutationCheckpoint(());

impl Env {
    /// Captures paragraph entry identity without changing the rollback epoch.
    pub(crate) fn begin_paragraph_mutations(&mut self) -> ParagraphMutationCheckpoint {
        assert!(
            self.paragraph_mutations.is_none(),
            "paragraph mutation recorder already active"
        );
        self.paragraph_mutations = Some(ParagraphMutationRecorder {
            entry_group_depth: self.group_depth,
            group_frame_mutated: false,
            cells: Vec::new(),
            live_group_mutations: Vec::new(),
            live_group_entry_values: AHashMap::with_hasher(count_int_hash_state()),
            positions: AHashMap::with_hasher(count_int_hash_state()),
        });
        ParagraphMutationCheckpoint(())
    }

    /// Records one count/int setter before its write barrier runs.
    pub(super) fn record_paragraph_mutation(
        &mut self,
        cell: CellId,
        expected: u64,
        value: u64,
        global: bool,
    ) {
        let Some(entry_group_depth) = self
            .paragraph_mutations
            .as_ref()
            .map(|recorder| recorder.entry_group_depth)
        else {
            return;
        };
        let cell = CellId::new(cell.bank(), cell.index());
        if entry_group_depth != 0 {
            let entry_value = *self
                .paragraph_mutations
                .as_mut()
                .expect("checked paragraph recorder")
                .live_group_entry_values
                .entry(cell)
                .or_insert(expected);
            if !global && self.group_depth > entry_group_depth {
                return;
            }
            let mutation = match cell.bank() {
                BankTag::Count => PureParagraphMutation::Count {
                    index: u16::try_from(cell.index()).expect("count register index fits u16"),
                    expected: entry_value as u32 as i32,
                    value: value as u32 as i32,
                    global,
                },
                BankTag::IntParam => PureParagraphMutation::IntParam {
                    param: IntParam::new(
                        u16::try_from(cell.index()).expect("integer parameter index fits u16"),
                    ),
                    expected: entry_value as u32 as i32,
                    value: value as u32 as i32,
                    global,
                },
                BankTag::CurrentFont => current_font_mutation(entry_value, value, global),
                _ => unreachable!("unsupported paragraph mutation cell"),
            };
            let recorder = self
                .paragraph_mutations
                .as_mut()
                .expect("checked paragraph recorder");
            recorder.live_group_mutations.push(mutation);
            return;
        }

        let recorder = self
            .paragraph_mutations
            .as_mut()
            .expect("checked paragraph recorder");
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

    /// Marks a structural or payload mutation of the live group stack.
    pub(super) fn record_paragraph_group_frame_mutation(&mut self) {
        let current_depth = self.group_depth;
        if let Some(recorder) = &mut self.paragraph_mutations {
            recorder.group_frame_mutated |= current_depth <= recorder.entry_group_depth;
        }
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
        let mutations = if recorder.entry_group_depth != 0 {
            recorder.live_group_mutations
        } else {
            recorder
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
                    BankTag::CurrentFont => current_font_mutation(
                        recorded.expected,
                        self.semantic_word(recorded.cell),
                        recorded.global,
                    ),
                    _ => unreachable!("unsupported paragraph mutation survivor"),
                })
                .collect()
        };

        PureParagraphMutationSummary {
            entry_in_group: recorder.entry_group_depth != 0,
            unsupported_group_ownership: recorder.entry_group_depth != 0
                && recorder.group_frame_mutated,
            mutations,
        }
    }

    pub(crate) fn abandon_paragraph_mutations(&mut self, _checkpoint: ParagraphMutationCheckpoint) {
        self.paragraph_mutations
            .take()
            .expect("paragraph mutation recorder missing at abandon");
    }
}

fn current_font_mutation(expected: u64, value: u64, global: bool) -> PureParagraphMutation {
    let decode = |word: u64| {
        let symbol = word >> 32;
        (
            FontId::new(word as u32),
            (symbol != 0).then(|| Symbol::new((symbol - 1) as u32)),
        )
    };
    let (expected_font, expected_symbol) = decode(expected);
    let (value_font, value_symbol) = decode(value);
    PureParagraphMutation::CurrentFont {
        expected_font,
        expected_symbol,
        value_font,
        value_symbol,
        global,
    }
}
