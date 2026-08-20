//! Append-only journal storage for barriered environment writes.
//!
//! The journal records undo+redo words and structural markers. `Env` owns the
//! group-exit and rollback walks; this module owns positions, append, slicing,
//! truncation, and marker lookup.

use crate::cell::{BankTag, CellId};
use crate::env::box_bank::BoxSlot;
use crate::env::group::GroupKind;
use crate::glue::GlueSpecRef;
use crate::ids::SnapshotId;
use crate::macro_store::MacroDefinitionRef;
use crate::meaning::Meaning;
use crate::token_store::TokenListRef;
use ahash::AHashMap;

/// A journal entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Entry {
    Undo(UndoRec),
    BoxUndo(BoxUndoId),
    Marker(Marker),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BoxUndoId(u32);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BoxUndoRec {
    index: u16,
    global: bool,
    restore_depth: u32,
    old: BoxSlot,
    new: BoxSlot,
}

impl BoxUndoRec {
    pub(crate) fn new(index: u16, global: bool, old: BoxSlot, new: BoxSlot) -> Self {
        Self {
            index,
            global,
            restore_depth: if global { 0 } else { new.owner_depth() },
            old,
            new,
        }
    }
    pub(crate) fn new_at_depth(index: u16, restore_depth: u32, old: BoxSlot, new: BoxSlot) -> Self {
        Self {
            index,
            global: false,
            restore_depth,
            old,
            new,
        }
    }
    pub(crate) const fn index(&self) -> u16 {
        self.index
    }
    pub(crate) const fn is_global(&self) -> bool {
        self.global
    }
    pub(crate) const fn survives_group(&self, leaving_depth: u32) -> bool {
        self.global || self.restore_depth < leaving_depth
    }
    pub(crate) const fn restore_depth(&self) -> u32 {
        self.restore_depth
    }
    pub(crate) fn old(&self) -> BoxSlot {
        self.old.clone()
    }
    pub(crate) fn new_value(&self) -> BoxSlot {
        self.new.clone()
    }
    pub(crate) fn replace_new_value(&mut self, new: BoxSlot) {
        self.new = new;
    }
}

/// A barrier undo+redo record for one environment cell.
///
/// The write barrier records only the first write to a cell in each epoch.
/// With undo+redo records, that means `new` is the value from the first
/// barrier hit and can be stale if the same cell is written again before the
/// epoch advances. M1 accepts that behavior: rollback uses `old`, while later
/// forward-replay consumers must re-derive final values from live cells.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct UndoRec {
    cell: CellId,
    old: u64,
    new: u64,
}

/// Strong token-list owners aligned with one token-valued undo record.
///
/// Both values are retained because group compaction can replay `new` after
/// the current cell has moved on, then refile `old` into an enclosing slice.
#[derive(Clone, Debug)]
pub(crate) struct TokenUndoRoots {
    old: Option<TokenListRef>,
    new: Option<TokenListRef>,
}

/// Strong macro-definition owners aligned with one meaning-valued undo.
#[derive(Clone, Debug)]
pub(crate) struct MacroUndoRoots {
    old: Option<MacroDefinitionRef>,
    new: Option<MacroDefinitionRef>,
}

/// Strong glue owners aligned with one glue-valued undo record.
#[derive(Clone, Debug)]
pub(crate) struct GlueUndoRoots {
    old: Option<GlueSpecRef>,
    new: Option<GlueSpecRef>,
}

impl GlueUndoRoots {
    #[must_use]
    pub(crate) fn new(old: Option<GlueSpecRef>, new: Option<GlueSpecRef>) -> Self {
        Self { old, new }
    }

    #[must_use]
    pub(crate) fn old(&self) -> Option<GlueSpecRef> {
        self.old
    }

    #[must_use]
    pub(crate) fn new_value(&self) -> Option<GlueSpecRef> {
        self.new
    }
}

impl MacroUndoRoots {
    #[must_use]
    pub(crate) fn new(old: Option<MacroDefinitionRef>, new: Option<MacroDefinitionRef>) -> Self {
        Self { old, new }
    }

    #[must_use]
    pub(crate) fn old(&self) -> Option<MacroDefinitionRef> {
        self.old
    }

    #[must_use]
    pub(crate) fn new_value(&self) -> Option<MacroDefinitionRef> {
        self.new
    }
}

impl TokenUndoRoots {
    #[must_use]
    pub(crate) fn new(old: Option<TokenListRef>, new: Option<TokenListRef>) -> Self {
        Self { old, new }
    }

    #[must_use]
    pub(crate) fn old(&self) -> Option<TokenListRef> {
        self.old
    }

    #[must_use]
    pub(crate) fn new_value(&self) -> Option<TokenListRef> {
        self.new
    }
}

impl UndoRec {
    /// Creates a journal record for `cell`, replacing `old` with `new`.
    #[must_use]
    pub(crate) const fn new(cell: CellId, old: u64, new: u64) -> Self {
        Self { cell, old, new }
    }

    /// Returns the recorded cell id.
    #[must_use]
    pub(crate) const fn cell(self) -> CellId {
        self.cell
    }

    /// Returns the value to restore when walking the journal backward.
    #[must_use]
    pub(crate) const fn old(self) -> u64 {
        self.old
    }

    /// Returns the value written by the barrier.
    #[must_use]
    pub(crate) const fn new_value(self) -> u64 {
        self.new
    }
}

/// Structural journal markers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Marker {
    Group {
        aftergroup_start: u32,
        kind: GroupKind,
    },
    /// Ordering-only marker for §276's separately stored one-word payload.
    Aftergroup,
    #[allow(dead_code)]
    Checkpoint(SnapshotId),
}

/// A stable position between journal entries.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct JournalPos(u32);

impl JournalPos {
    /// Creates a journal position from a previously validated entry offset.
    #[must_use]
    pub(crate) fn from_raw(raw: usize) -> Self {
        JournalPos(u32_len(raw, "journal exceeds u32 entries"))
    }

    /// Returns the raw entry offset.
    #[must_use]
    pub(crate) const fn raw(self) -> u32 {
        self.0
    }
}

/// Append/truncate journal storage.
#[derive(Clone, Debug, Default)]
pub(crate) struct Journal {
    entries: Vec<Entry>,
    token_undo_roots: Vec<Option<TokenUndoRoots>>,
    macro_undo_roots: Vec<Option<MacroUndoRoots>>,
    glue_undo_roots: Vec<Option<GlueUndoRoots>>,
    box_undos: Vec<BoxUndoRec>,
    save_stack: SaveStackProjection,
}

/// Incremental projection of TeX82 §§273--280's physical save stack.
///
/// The typed journal is the semantic owner. This derived, rollback-coupled
/// view exists only for §1334's diagnostic high-water accounting.
#[derive(Clone, Debug, Default)]
struct SaveStackProjection {
    words: usize,
    groups: Vec<AHashMap<CellId, usize>>,
    entries: usize,
    latest_push: Option<(usize, usize)>,
    undos: Vec<SaveStackProjectionUndo>,
    #[cfg(test)]
    rolled_back_entries: usize,
}

#[derive(Clone, Copy, Debug)]
struct SaveStackProjectionUndo {
    previous_latest_push: Option<(usize, usize)>,
    mutation: SaveStackProjectionMutation,
}

#[derive(Clone, Copy, Debug)]
enum SaveStackProjectionMutation {
    None,
    InsertedSaved { cell: CellId, words: usize },
    RemovedSaved { cell: CellId, words: usize },
    PushedGroup,
}

impl SaveStackProjection {
    fn push_undo(&mut self, rec: UndoRec) {
        let mut pushed_words = 0;
        let mut mutation = SaveStackProjectionMutation::None;
        let Some(saved) = self.groups.last_mut() else {
            self.finish_entry(0, mutation);
            return;
        };
        let cell = rec.cell().without_assignment_scope();
        if rec.cell().is_global() {
            // TeX82 §§275/283 retain an already-pushed restore record after
            // a global definition; only the current local-run eligibility is
            // reset so a later local definition can push another record.
            if let Some(words) = saved.remove(&cell) {
                mutation = SaveStackProjectionMutation::RemovedSaved { cell, words };
            }
        } else if !saved.contains_key(&cell) {
            // TeX82 §§275--276 represents `restore_zero` in one word,
            // while `restore_old_value` occupies two.
            let words = if is_canonical_restore_zero(cell, rec.old()) {
                1
            } else {
                2
            };
            saved.insert(cell, words);
            self.words = self.words.saturating_add(words);
            pushed_words = words;
            mutation = SaveStackProjectionMutation::InsertedSaved { cell, words };
        }
        self.finish_entry(pushed_words, mutation);
    }

    fn push_box_undo(&mut self, rec: &BoxUndoRec) {
        let mut pushed_words = 0;
        let mut mutation = SaveStackProjectionMutation::None;
        let Some(saved) = self.groups.last_mut() else {
            self.finish_entry(0, mutation);
            return;
        };
        let cell = CellId::new(BankTag::Box, u32::from(rec.index()));
        if rec.is_global() {
            if let Some(words) = saved.remove(&cell) {
                mutation = SaveStackProjectionMutation::RemovedSaved { cell, words };
            }
        } else if !saved.contains_key(&cell) {
            saved.insert(cell, 2);
            self.words = self.words.saturating_add(2);
            pushed_words = 2;
            mutation = SaveStackProjectionMutation::InsertedSaved { cell, words: 2 };
        }
        self.finish_entry(pushed_words, mutation);
    }

    fn push_marker(&mut self, marker: Marker) {
        let (pushed_words, mutation) = match marker {
            Marker::Group { .. } => {
                self.words = self.words.saturating_add(1);
                self.groups.push(AHashMap::new());
                (1, SaveStackProjectionMutation::PushedGroup)
            }
            // The separately stored aftergroup payload owns this word count,
            // but its journal marker still orders the most recent §276 push.
            Marker::Aftergroup => (1, SaveStackProjectionMutation::None),
            Marker::Checkpoint(_) => (0, SaveStackProjectionMutation::None),
        };
        self.finish_entry(pushed_words, mutation);
    }

    fn finish_entry(&mut self, pushed_words: usize, mutation: SaveStackProjectionMutation) {
        self.undos.push(SaveStackProjectionUndo {
            previous_latest_push: self.latest_push,
            mutation,
        });
        self.entries = self.entries.saturating_add(1);
        if pushed_words != 0 {
            self.latest_push = Some((self.entries, pushed_words));
        }
    }

    fn truncate_to(&mut self, len: usize) {
        assert!(len <= self.entries, "save-stack position is past the end");
        while self.entries > len {
            let undo = self
                .undos
                .pop()
                .expect("save-stack projection undo is aligned with the journal");
            match undo.mutation {
                SaveStackProjectionMutation::None => {}
                SaveStackProjectionMutation::InsertedSaved { cell, words } => {
                    let removed = self
                        .groups
                        .last_mut()
                        .expect("saved cell belongs to a live group")
                        .remove(&cell);
                    assert_eq!(
                        removed,
                        Some(words),
                        "saved-cell projection must round trip"
                    );
                    self.words = self
                        .words
                        .checked_sub(words)
                        .expect("save-stack word count must cover saved cell");
                }
                SaveStackProjectionMutation::RemovedSaved { cell, words } => {
                    let previous = self
                        .groups
                        .last_mut()
                        .expect("saved cell belongs to a live group")
                        .insert(cell, words);
                    assert!(previous.is_none(), "removed saved cell must remain absent");
                }
                SaveStackProjectionMutation::PushedGroup => {
                    let group = self
                        .groups
                        .pop()
                        .expect("group marker belongs to a live save-stack group");
                    assert!(
                        group.is_empty(),
                        "group entries must roll back before its marker"
                    );
                    self.words = self
                        .words
                        .checked_sub(1)
                        .expect("save-stack word count must cover group marker");
                }
            }
            self.latest_push = undo.previous_latest_push;
            self.entries -= 1;
            #[cfg(test)]
            {
                self.rolled_back_entries = self.rolled_back_entries.saturating_add(1);
            }
        }
    }
}

pub(crate) fn is_canonical_restore_zero(cell: CellId, old: u64) -> bool {
    match cell.bank() {
        BankTag::Meaning => old == Meaning::Undefined.encode(),
        // TeX82 §240 initializes token-list parameters to
        // `undefined_control_sequence` at `level_zero`; the typed optional
        // codec preserves that null value as zero, distinct from a defined
        // empty list. Sections 275--276 save the null case in one word.
        BankTag::TokParam => old == 0,
        _ => false,
    }
}

impl Journal {
    /// Creates an empty journal.
    #[must_use]
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn retained_bytes(&self) -> usize {
        self.entries
            .capacity()
            .saturating_mul(std::mem::size_of::<Entry>())
            .saturating_add(
                self.token_undo_roots
                    .capacity()
                    .saturating_mul(std::mem::size_of::<Option<TokenUndoRoots>>()),
            )
            .saturating_add(
                self.macro_undo_roots
                    .capacity()
                    .saturating_mul(std::mem::size_of::<Option<MacroUndoRoots>>()),
            )
            .saturating_add(
                self.glue_undo_roots
                    .capacity()
                    .saturating_mul(std::mem::size_of::<Option<GlueUndoRoots>>()),
            )
            .saturating_add(
                self.box_undos
                    .capacity()
                    .saturating_mul(std::mem::size_of::<BoxUndoRec>()),
            )
            .saturating_add(
                self.save_stack
                    .groups
                    .iter()
                    .map(|group| {
                        group.capacity().saturating_mul(
                            std::mem::size_of::<CellId>() + std::mem::size_of::<usize>(),
                        )
                    })
                    .sum::<usize>(),
            )
            .saturating_add(
                self.save_stack
                    .undos
                    .capacity()
                    .saturating_mul(std::mem::size_of::<SaveStackProjectionUndo>()),
            )
    }

    /// Appends an undo+redo record.
    pub(crate) fn push_undo(&mut self, rec: UndoRec) -> JournalPos {
        let pos = self.pos();
        self.save_stack.push_undo(rec);
        self.entries.push(Entry::Undo(rec));
        self.token_undo_roots.push(None);
        self.macro_undo_roots.push(None);
        self.glue_undo_roots.push(None);
        pos
    }

    /// Attaches the strong roots for the token-valued undo at `pos`.
    pub(crate) fn attach_token_undo_roots(&mut self, pos: JournalPos, roots: TokenUndoRoots) {
        let index = checked_pos(pos, self.entries.len());
        assert!(
            index < self.entries.len(),
            "undo position names the journal end"
        );
        let Entry::Undo(rec) = self.entries[index] else {
            panic!("token roots require an undo entry");
        };
        assert!(
            matches!(rec.cell().bank(), BankTag::Toks | BankTag::TokParam),
            "token roots require a token-valued undo"
        );
        let previous = self.token_undo_roots[index].replace(roots);
        assert!(previous.is_none(), "token undo roots already attached");
    }

    #[must_use]
    pub(crate) fn token_undo_roots(&self, index: usize) -> Option<&TokenUndoRoots> {
        self.token_undo_roots.get(index).and_then(Option::as_ref)
    }

    pub(crate) fn attach_macro_undo_roots(&mut self, pos: JournalPos, roots: MacroUndoRoots) {
        let index = checked_pos(pos, self.entries.len());
        let Entry::Undo(rec) = self.entries[index] else {
            panic!("macro roots require an undo entry");
        };
        assert_eq!(rec.cell().bank(), BankTag::Meaning);
        let previous = self.macro_undo_roots[index].replace(roots);
        assert!(previous.is_none(), "macro undo roots already attached");
    }

    #[must_use]
    pub(crate) fn macro_undo_roots(&self, index: usize) -> Option<&MacroUndoRoots> {
        self.macro_undo_roots.get(index).and_then(Option::as_ref)
    }

    pub(crate) fn attach_glue_undo_roots(&mut self, pos: JournalPos, roots: GlueUndoRoots) {
        let index = checked_pos(pos, self.entries.len());
        let Entry::Undo(rec) = self.entries[index] else {
            panic!("glue roots require an undo entry");
        };
        assert!(matches!(
            rec.cell().bank(),
            BankTag::Skip | BankTag::Muskip | BankTag::GlueParam
        ));
        let previous = self.glue_undo_roots[index].replace(roots);
        assert!(previous.is_none(), "glue undo roots already attached");
    }

    #[must_use]
    pub(crate) fn glue_undo_roots(&self, index: usize) -> Option<&GlueUndoRoots> {
        self.glue_undo_roots.get(index).and_then(Option::as_ref)
    }

    pub(crate) fn push_box_undo(&mut self, rec: BoxUndoRec) -> JournalPos {
        let pos = self.pos();
        self.save_stack.push_box_undo(&rec);
        let id = BoxUndoId(u32_len(
            self.box_undos.len(),
            "box undo arena exceeds u32 entries",
        ));
        self.box_undos.push(rec);
        self.entries.push(Entry::BoxUndo(id));
        self.token_undo_roots.push(None);
        self.macro_undo_roots.push(None);
        self.glue_undo_roots.push(None);
        pos
    }

    pub(crate) fn box_undo(&self, id: BoxUndoId) -> BoxUndoRec {
        self.box_undos[id.0 as usize].clone()
    }

    pub(crate) fn replace_box_new(&mut self, pos: JournalPos, new: BoxSlot) {
        let index = checked_pos(pos, self.entries.len());
        let Entry::BoxUndo(id) = self.entries[index] else {
            panic!("journal position does not name a box undo entry");
        };
        let rec = &mut self.box_undos[id.0 as usize];
        rec.replace_new_value(new);
    }

    pub(crate) fn box_undo_len(&self) -> u32 {
        u32_len(self.box_undos.len(), "box undo arena exceeds u32 entries")
    }

    pub(crate) fn truncate_box_undos(&mut self, len: u32) {
        self.box_undos.truncate(len as usize);
    }

    /// Appends a structural marker.
    pub(crate) fn push_marker(&mut self, marker: Marker) {
        self.save_stack.push_marker(marker);
        self.entries.push(Entry::Marker(marker));
        self.token_undo_roots.push(None);
        self.macro_undo_roots.push(None);
        self.glue_undo_roots.push(None);
    }

    /// Returns the live TeX82 save-stack words and latest physical push
    /// represented by this journal.
    #[must_use]
    pub(crate) const fn canonical_save_stack_projection(&self) -> (usize, Option<(usize, usize)>) {
        (self.save_stack.words, self.save_stack.latest_push)
    }

    /// Returns the live TeX82 save-stack words represented by this journal.
    #[must_use]
    #[cfg(test)]
    pub(crate) const fn canonical_save_stack_words(&self) -> usize {
        self.canonical_save_stack_projection().0
    }

    /// Returns the current end position.
    #[must_use]
    pub(crate) fn pos(&self) -> JournalPos {
        JournalPos(u32_len(self.entries.len(), "journal exceeds u32 entries"))
    }

    /// Returns the number of entries currently held by the journal.
    #[must_use]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns one journal entry by absolute entry offset.
    #[must_use]
    pub(crate) fn entry(&self, index: usize) -> Entry {
        self.entries[index]
    }

    /// Returns entries appended since `pos`.
    #[must_use]
    pub(crate) fn entries_since(&self, pos: JournalPos) -> &[Entry] {
        let start = checked_pos(pos, self.entries.len());
        &self.entries[start..]
    }

    /// Truncates the journal to `pos`.
    pub(crate) fn truncate_to(&mut self, pos: JournalPos) {
        let len = checked_pos(pos, self.entries.len());
        if len == self.entries.len() {
            return;
        }
        self.save_stack.truncate_to(len);
        self.entries.truncate(len);
        self.token_undo_roots.truncate(len);
        self.macro_undo_roots.truncate(len);
        self.glue_undo_roots.truncate(len);
    }

    /// Retires every record below a committed level-zero baseline.
    ///
    /// `Env` proves that no snapshot or open group can name these positions
    /// before calling this method. Keep the allocations as bounded scratch so
    /// repeated operations plateau at their largest live journal slice.
    pub(crate) fn clear_committed(&mut self) {
        debug_assert!(self.save_stack.groups.is_empty());
        self.entries.clear();
        self.token_undo_roots.clear();
        self.macro_undo_roots.clear();
        self.glue_undo_roots.clear();
        self.box_undos.clear();
        self.save_stack.words = 0;
        self.save_stack.entries = 0;
        self.save_stack.latest_push = None;
        self.save_stack.undos.clear();
    }

    #[cfg(test)]
    pub(crate) const fn testing_save_stack_projection_rolled_back_entries(&self) -> usize {
        self.save_stack.rolled_back_entries
    }
}

fn checked_pos(pos: JournalPos, len: usize) -> usize {
    let index = pos.raw() as usize;
    assert!(index <= len, "journal position is past the end");
    index
}

fn u32_len(value: usize, message: &str) -> u32 {
    match u32::try_from(value) {
        Ok(value) => value,
        Err(_) => panic!("{message}"),
    }
}

#[cfg(test)]
mod tests;
