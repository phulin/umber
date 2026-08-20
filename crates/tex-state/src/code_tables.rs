//! TeX code tables over Unicode scalar values.
//!
//! Code-table writes are sparse, so each table is represented as a two-level
//! persistent radix whose absent pages mean the algorithmic INITEX defaults.
//! Snapshot history is structural: snapshots keep old roots and writes copy a
//! bounded root/chunk path plus the touched 256-entry page.
//! Generations track write events, including same-value assignments, so lexer
//! classifiers can invalidate on assignment activity rather than value changes.

use crate::token::Catcode;
use core::array;
use std::collections::HashSet;
#[cfg(test)]
use std::hash::{Hash, Hasher};
use std::sync::{Arc, OnceLock};

use self::global::{GlobalCodeTableWrite, GlobalWriteHistory};

const PAGE_BITS: u32 = 8;
const PAGE_LEN: usize = 1 << PAGE_BITS;
const PAGE_MASK: u32 = PAGE_LEN as u32 - 1;
const UNICODE_SCALAR_COUNT: usize = 0x11_0000;
const ROOT_LEN: usize = UNICODE_SCALAR_COUNT / PAGE_LEN;
const ROOT_CHUNK_LEN: usize = PAGE_LEN;
const ROOT_CHUNK_COUNT: usize = ROOT_LEN.div_ceil(ROOT_CHUNK_LEN);
const DELCODE_DEFAULT: i32 = -1;
const ASCII_A: u32 = b'A' as u32;
const ASCII_Z: u32 = b'Z' as u32;
const ASCII_LOWER_A: u32 = b'a' as u32;
const ASCII_LOWER_Z: u32 = b'z' as u32;
const ASCII_ZERO: u32 = b'0' as u32;
const ASCII_NINE: u32 = b'9' as u32;
const ASCII_PERIOD: u32 = b'.' as u32;
const VARIABLE_MATH_CLASS: u32 = 7 << 12;
const LETTER_MATH_FAMILY: u32 = 1 << 8;
/// tex.web §22 `null_code`: the ASCII code INITEX makes `\catcode` 9.
const NULL_CODE: u32 = 0;
/// tex.web §22 `carriage_return`: the ASCII code INITEX makes `\catcode` 5.
const CARRIAGE_RETURN: u32 = 0o15;
/// tex.web §22 `invalid_code`: the ASCII code INITEX makes `\catcode` 15.
const INVALID_CODE: u32 = 0o177;
const ASCII_SPACE: u32 = b' ' as u32;
const ASCII_PERCENT: u32 = b'%' as u32;
const ASCII_BACKSLASH: u32 = b'\\' as u32;

/// A TeX `\lccode` value.
pub type LcCode = u32;
/// A TeX `\uccode` value.
pub type UcCode = u32;
/// A TeX `\sfcode` value.
pub type SfCode = u16;
/// A TeX `\mathcode` value.
pub type MathCode = u32;
/// A TeX `\delcode` value.
pub type DelCode = i32;

/// Per-code-table generation stamps used by lexer classifiers.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct CodeTableGenerations {
    pub catcode: u32,
    pub lccode: u32,
    pub uccode: u32,
    pub sfcode: u32,
    pub mathcode: u32,
    pub delcode: u32,
}

/// One complete non-default Unicode code-table row for format capture.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CodeTableValues {
    pub(crate) catcode: Catcode,
    pub(crate) lccode: LcCode,
    pub(crate) uccode: UcCode,
    pub(crate) sfcode: SfCode,
    pub(crate) mathcode: MathCode,
    pub(crate) delcode: DelCode,
}

/// Root snapshot for all code tables.
#[derive(Clone, Debug)]
pub(crate) struct CodeTablesSnapshot {
    catcodes: PagedTableSnapshot<Catcode>,
    lccodes: PagedTableSnapshot<LcCode>,
    uccodes: PagedTableSnapshot<UcCode>,
    sfcodes: PagedTableSnapshot<SfCode>,
    mathcodes: PagedTableSnapshot<MathCode>,
    delcodes: PagedTableSnapshot<DelCode>,
    group_roots: Arc<Vec<CodeTableRoots>>,
    global_writes: GlobalWriteHistory,
    save_stack_words: usize,
    latest_save: Option<(usize, usize)>,
}

#[derive(Clone, Debug)]
pub(crate) struct CodeTablesSemanticCursor {
    catcodes: Arc<Root<Catcode>>,
    lccodes: Arc<Root<LcCode>>,
    uccodes: Arc<Root<UcCode>>,
    sfcodes: Arc<Root<SfCode>>,
    mathcodes: Arc<Root<MathCode>>,
    delcodes: Arc<Root<DelCode>>,
}

impl PartialEq for CodeTablesSemanticCursor {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.catcodes, &other.catcodes)
            && Arc::ptr_eq(&self.lccodes, &other.lccodes)
            && Arc::ptr_eq(&self.uccodes, &other.uccodes)
            && Arc::ptr_eq(&self.sfcodes, &other.sfcodes)
            && Arc::ptr_eq(&self.mathcodes, &other.mathcodes)
            && Arc::ptr_eq(&self.delcodes, &other.delcodes)
    }
}

impl Eq for CodeTablesSemanticCursor {}

impl CodeTablesSemanticCursor {
    pub(crate) fn shares_table_root(&self, other: &Self, table: usize) -> bool {
        match table {
            0 => Arc::ptr_eq(&self.catcodes, &other.catcodes),
            1 => Arc::ptr_eq(&self.lccodes, &other.lccodes),
            2 => Arc::ptr_eq(&self.uccodes, &other.uccodes),
            3 => Arc::ptr_eq(&self.sfcodes, &other.sfcodes),
            4 => Arc::ptr_eq(&self.mathcodes, &other.mathcodes),
            5 => Arc::ptr_eq(&self.delcodes, &other.delcodes),
            _ => panic!("code-table index out of range"),
        }
    }
}

/// Structurally shared code-table roots saved at TeX group boundaries.
#[derive(Clone, Debug)]
struct CodeTableRoots {
    catcodes: Arc<Root<Catcode>>,
    lccodes: Arc<Root<LcCode>>,
    uccodes: Arc<Root<UcCode>>,
    sfcodes: Arc<Root<SfCode>>,
    mathcodes: Arc<Root<MathCode>>,
    delcodes: Arc<Root<DelCode>>,
    global_writes: GlobalWriteHistory,
    saved: Vec<CodeTableRestoreRecord>,
    local_runs: HashSet<CodeTableKey>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum CodeTableKind {
    Catcode,
    Lccode,
    Uccode,
    Sfcode,
    Mathcode,
    Delcode,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct CodeTableKey {
    kind: CodeTableKind,
    ch: char,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CodeTableRestoreRecord {
    pub(crate) save_position: usize,
    pub(crate) kind: CodeTableKind,
    pub(crate) ch: char,
    pub(crate) value: i64,
    pub(crate) retaining: bool,
}

impl CodeTableRoots {
    fn apply_global_writes(&mut self, writes: &[GlobalCodeTableWrite]) {
        for write in writes {
            match *write {
                GlobalCodeTableWrite::Catcode(ch, value) => {
                    self.catcodes = PagedTable::<Catcode, CatcodeDefaults>::root_with_value(
                        &self.catcodes,
                        ch,
                        value,
                    );
                }
                GlobalCodeTableWrite::LcCode(ch, value) => {
                    self.lccodes = PagedTable::<LcCode, LcCodeDefaults>::root_with_value(
                        &self.lccodes,
                        ch,
                        value,
                    );
                }
                GlobalCodeTableWrite::UcCode(ch, value) => {
                    self.uccodes = PagedTable::<UcCode, UcCodeDefaults>::root_with_value(
                        &self.uccodes,
                        ch,
                        value,
                    );
                }
                GlobalCodeTableWrite::SfCode(ch, value) => {
                    self.sfcodes = PagedTable::<SfCode, SfCodeDefaults>::root_with_value(
                        &self.sfcodes,
                        ch,
                        value,
                    );
                }
                GlobalCodeTableWrite::MathCode(ch, value) => {
                    self.mathcodes = PagedTable::<MathCode, MathCodeDefaults>::root_with_value(
                        &self.mathcodes,
                        ch,
                        value,
                    );
                }
                GlobalCodeTableWrite::DelCode(ch, value) => {
                    self.delcodes = PagedTable::<DelCode, DelCodeDefaults>::root_with_value(
                        &self.delcodes,
                        ch,
                        value,
                    );
                }
            }
        }
    }
}

/// The six mutable TeX code tables.
#[derive(Clone, Debug)]
pub struct CodeTables {
    catcodes: PagedTable<Catcode, CatcodeDefaults>,
    lccodes: PagedTable<LcCode, LcCodeDefaults>,
    uccodes: PagedTable<UcCode, UcCodeDefaults>,
    sfcodes: PagedTable<SfCode, SfCodeDefaults>,
    mathcodes: PagedTable<MathCode, MathCodeDefaults>,
    delcodes: PagedTable<DelCode, DelCodeDefaults>,
    group_roots: Arc<Vec<CodeTableRoots>>,
    global_writes: GlobalWriteHistory,
    /// Incremental TeX82 §275 physical save-stack projection.
    save_stack_words: usize,
    latest_save: Option<(usize, usize)>,
}

impl CodeTables {
    /// Creates code tables initialized to INITEX defaults.
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            catcodes: PagedTable::new(),
            lccodes: PagedTable::new(),
            uccodes: PagedTable::new(),
            sfcodes: PagedTable::new(),
            mathcodes: PagedTable::new(),
            delcodes: PagedTable::new(),
            group_roots: Arc::new(Vec::new()),
            global_writes: GlobalWriteHistory::default(),
            save_stack_words: 0,
            latest_save: None,
        }
    }

    pub(crate) fn from_frozen(rows: &[(char, CodeTableValues)]) -> Result<Self, &'static str> {
        let mut tables = Self::new();
        let mut previous = None;
        for &(ch, values) in rows {
            if previous.is_some_and(|prior| prior >= ch) {
                return Err("non-canonical frozen code-table order");
            }
            previous = Some(ch);
            let code = ch as u32;
            if values
                == (CodeTableValues {
                    catcode: CatcodeDefaults::default_for(code),
                    lccode: LcCodeDefaults::default_for(code),
                    uccode: UcCodeDefaults::default_for(code),
                    sfcode: SfCodeDefaults::default_for(code),
                    mathcode: MathCodeDefaults::default_for(code),
                    delcode: DelCodeDefaults::default_for(code),
                })
            {
                return Err("default frozen code-table row");
            }
            tables.catcodes.root = PagedTable::<Catcode, CatcodeDefaults>::root_with_value(
                &tables.catcodes.root,
                ch,
                values.catcode,
            );
            tables.lccodes.root = PagedTable::<LcCode, LcCodeDefaults>::root_with_value(
                &tables.lccodes.root,
                ch,
                values.lccode,
            );
            tables.uccodes.root = PagedTable::<UcCode, UcCodeDefaults>::root_with_value(
                &tables.uccodes.root,
                ch,
                values.uccode,
            );
            tables.sfcodes.root = PagedTable::<SfCode, SfCodeDefaults>::root_with_value(
                &tables.sfcodes.root,
                ch,
                values.sfcode,
            );
            tables.mathcodes.root = PagedTable::<MathCode, MathCodeDefaults>::root_with_value(
                &tables.mathcodes.root,
                ch,
                values.mathcode,
            );
            tables.delcodes.root = PagedTable::<DelCode, DelCodeDefaults>::root_with_value(
                &tables.delcodes.root,
                ch,
                values.delcode,
            );
        }
        Ok(tables)
    }

    pub(crate) fn checkpoint(&self) -> CodeTablesSnapshot {
        CodeTablesSnapshot {
            catcodes: self.catcodes.checkpoint(),
            lccodes: self.lccodes.checkpoint(),
            uccodes: self.uccodes.checkpoint(),
            sfcodes: self.sfcodes.checkpoint(),
            mathcodes: self.mathcodes.checkpoint(),
            delcodes: self.delcodes.checkpoint(),
            group_roots: Arc::clone(&self.group_roots),
            global_writes: self.global_writes.clone(),
            save_stack_words: self.save_stack_words,
            latest_save: self.latest_save,
        }
    }

    pub(crate) fn semantic_cursor(&self) -> CodeTablesSemanticCursor {
        CodeTablesSemanticCursor {
            catcodes: Arc::clone(&self.catcodes.root),
            lccodes: Arc::clone(&self.lccodes.root),
            uccodes: Arc::clone(&self.uccodes.root),
            sfcodes: Arc::clone(&self.sfcodes.root),
            mathcodes: Arc::clone(&self.mathcodes.root),
            delcodes: Arc::clone(&self.delcodes.root),
        }
    }

    pub(crate) fn semantic_cursor_from_snapshot(
        snapshot: &CodeTablesSnapshot,
    ) -> CodeTablesSemanticCursor {
        CodeTablesSemanticCursor {
            catcodes: Arc::clone(&snapshot.catcodes.root),
            lccodes: Arc::clone(&snapshot.lccodes.root),
            uccodes: Arc::clone(&snapshot.uccodes.root),
            sfcodes: Arc::clone(&snapshot.sfcodes.root),
            mathcodes: Arc::clone(&snapshot.mathcodes.root),
            delcodes: Arc::clone(&snapshot.delcodes.root),
        }
    }

    pub(crate) fn rollback_to(&mut self, snapshot: CodeTablesSnapshot) {
        self.catcodes.rollback_to(snapshot.catcodes);
        self.lccodes.rollback_to(snapshot.lccodes);
        self.uccodes.rollback_to(snapshot.uccodes);
        self.sfcodes.rollback_to(snapshot.sfcodes);
        self.mathcodes.rollback_to(snapshot.mathcodes);
        self.delcodes.rollback_to(snapshot.delcodes);
        self.group_roots = snapshot.group_roots;
        self.global_writes = snapshot.global_writes;
        self.save_stack_words = snapshot.save_stack_words;
        self.latest_save = snapshot.latest_save;
    }

    pub(crate) fn enter_group(&mut self) {
        let roots = CodeTableRoots {
            catcodes: self.catcodes.root(),
            lccodes: self.lccodes.root(),
            uccodes: self.uccodes.root(),
            sfcodes: self.sfcodes.root(),
            mathcodes: self.mathcodes.root(),
            delcodes: self.delcodes.root(),
            global_writes: self.global_writes.clone(),
            saved: Vec::new(),
            local_runs: HashSet::new(),
        };
        Arc::make_mut(&mut self.group_roots).push(roots);
    }

    pub(crate) fn leave_group(&mut self) -> Vec<CodeTableRestoreRecord> {
        let mut roots = Arc::make_mut(&mut self.group_roots)
            .pop()
            .expect("leave_group without matching code-table roots");
        let writes = self.global_writes.writes_since(&roots.global_writes);
        roots.apply_global_writes(&writes);
        self.catcodes.restore_group_root(roots.catcodes);
        self.lccodes.restore_group_root(roots.lccodes);
        self.uccodes.restore_group_root(roots.uccodes);
        self.sfcodes.restore_group_root(roots.sfcodes);
        self.mathcodes.restore_group_root(roots.mathcodes);
        self.delcodes.restore_group_root(roots.delcodes);
        for record in &mut roots.saved {
            if record.retaining {
                record.value = self.current_value(record.kind, record.ch);
            }
        }
        if self.group_roots.is_empty() {
            self.global_writes = GlobalWriteHistory::default();
        }
        self.save_stack_words = self
            .save_stack_words
            .saturating_sub(roots.saved.len().saturating_mul(2));
        self.latest_save = self
            .group_roots
            .iter()
            .rev()
            .find_map(|frame| frame.saved.last())
            .map(|record| (record.save_position, 2));
        roots.saved.reverse();
        roots.saved
    }

    /// TeX82 §275 save-stack words owned by live code-table assignments.
    #[cfg(test)]
    pub(crate) fn canonical_save_stack_words(&self) -> usize {
        self.canonical_save_stack_projection().0
    }

    pub(crate) fn canonical_save_stack_projection(&self) -> (usize, Option<(usize, usize)>) {
        (self.save_stack_words, self.latest_save)
    }

    /// Returns the generation vector for all code tables.
    #[must_use]
    pub fn generations(&self) -> CodeTableGenerations {
        CodeTableGenerations {
            catcode: self.catcodes.generation(),
            lccode: self.lccodes.generation(),
            uccode: self.uccodes.generation(),
            sfcode: self.sfcodes.generation(),
            mathcode: self.mathcodes.generation(),
            delcode: self.delcodes.generation(),
        }
    }

    #[must_use]
    pub fn catcode(&self, ch: char) -> Catcode {
        self.catcodes.get(ch)
    }

    // Kept as an intentionally private boundary exercised by compile-fail tests.
    #[allow(dead_code)]
    pub(crate) fn set_catcode(&mut self, ch: char, value: Catcode) {
        self.set_catcode_at(0, ch, value);
    }

    pub(crate) fn set_catcode_at(&mut self, save_position: usize, ch: char, value: Catcode) {
        self.record_local(
            CodeTableKind::Catcode,
            ch,
            i64::from(self.catcodes.get(ch) as u8),
            save_position,
        );
        self.catcodes.set(ch, value);
    }

    pub(crate) fn set_catcode_global(&mut self, ch: char, value: Catcode) {
        self.record_global_trace(CodeTableKind::Catcode, ch);
        self.catcodes.set(ch, value);
        self.record_global(GlobalCodeTableWrite::Catcode(ch, value));
    }

    #[must_use]
    pub fn lccode(&self, ch: char) -> LcCode {
        self.lccodes.get(ch)
    }

    #[allow(dead_code)]
    pub(crate) fn set_lccode(&mut self, ch: char, value: LcCode) {
        self.set_lccode_at(0, ch, value);
    }

    pub(crate) fn set_lccode_at(&mut self, save_position: usize, ch: char, value: LcCode) {
        assert_unicode_code(value, "lccode");
        self.record_local(
            CodeTableKind::Lccode,
            ch,
            i64::from(self.lccodes.get(ch)),
            save_position,
        );
        self.lccodes.set(ch, value);
    }

    pub(crate) fn set_lccode_global(&mut self, ch: char, value: LcCode) {
        assert_unicode_code(value, "lccode");
        self.record_global_trace(CodeTableKind::Lccode, ch);
        self.lccodes.set(ch, value);
        self.record_global(GlobalCodeTableWrite::LcCode(ch, value));
    }

    #[must_use]
    pub fn uccode(&self, ch: char) -> UcCode {
        self.uccodes.get(ch)
    }

    #[allow(dead_code)]
    pub(crate) fn set_uccode(&mut self, ch: char, value: UcCode) {
        self.set_uccode_at(0, ch, value);
    }

    pub(crate) fn set_uccode_at(&mut self, save_position: usize, ch: char, value: UcCode) {
        assert_unicode_code(value, "uccode");
        self.record_local(
            CodeTableKind::Uccode,
            ch,
            i64::from(self.uccodes.get(ch)),
            save_position,
        );
        self.uccodes.set(ch, value);
    }

    pub(crate) fn set_uccode_global(&mut self, ch: char, value: UcCode) {
        assert_unicode_code(value, "uccode");
        self.record_global_trace(CodeTableKind::Uccode, ch);
        self.uccodes.set(ch, value);
        self.record_global(GlobalCodeTableWrite::UcCode(ch, value));
    }

    #[must_use]
    pub fn sfcode(&self, ch: char) -> SfCode {
        self.sfcodes.get(ch)
    }

    #[allow(dead_code)]
    pub(crate) fn set_sfcode(&mut self, ch: char, value: SfCode) {
        self.set_sfcode_at(0, ch, value);
    }

    pub(crate) fn set_sfcode_at(&mut self, save_position: usize, ch: char, value: SfCode) {
        self.record_local(
            CodeTableKind::Sfcode,
            ch,
            i64::from(self.sfcodes.get(ch)),
            save_position,
        );
        self.sfcodes.set(ch, value);
    }

    pub(crate) fn set_sfcode_global(&mut self, ch: char, value: SfCode) {
        self.record_global_trace(CodeTableKind::Sfcode, ch);
        self.sfcodes.set(ch, value);
        self.record_global(GlobalCodeTableWrite::SfCode(ch, value));
    }

    #[must_use]
    pub fn mathcode(&self, ch: char) -> MathCode {
        self.mathcodes.get(ch)
    }

    #[allow(dead_code)]
    pub(crate) fn set_mathcode(&mut self, ch: char, value: MathCode) {
        self.set_mathcode_at(0, ch, value);
    }

    pub(crate) fn set_mathcode_at(&mut self, save_position: usize, ch: char, value: MathCode) {
        self.record_local(
            CodeTableKind::Mathcode,
            ch,
            i64::from(self.mathcodes.get(ch)),
            save_position,
        );
        self.mathcodes.set(ch, value);
    }

    pub(crate) fn set_mathcode_global(&mut self, ch: char, value: MathCode) {
        self.record_global_trace(CodeTableKind::Mathcode, ch);
        self.mathcodes.set(ch, value);
        self.record_global(GlobalCodeTableWrite::MathCode(ch, value));
    }

    #[must_use]
    pub fn delcode(&self, ch: char) -> DelCode {
        self.delcodes.get(ch)
    }

    #[allow(dead_code)]
    pub(crate) fn set_delcode(&mut self, ch: char, value: DelCode) {
        self.set_delcode_at(0, ch, value);
    }

    pub(crate) fn set_delcode_at(&mut self, save_position: usize, ch: char, value: DelCode) {
        self.record_local(
            CodeTableKind::Delcode,
            ch,
            i64::from(self.delcodes.get(ch)),
            save_position,
        );
        self.delcodes.set(ch, value);
    }

    pub(crate) fn set_delcode_global(&mut self, ch: char, value: DelCode) {
        self.record_global_trace(CodeTableKind::Delcode, ch);
        self.delcodes.set(ch, value);
        self.record_global(GlobalCodeTableWrite::DelCode(ch, value));
    }

    fn record_global(&mut self, write: GlobalCodeTableWrite) {
        if !self.group_roots.is_empty() {
            self.global_writes.push(write);
        }
    }

    fn record_local(&mut self, kind: CodeTableKind, ch: char, old: i64, save_position: usize) {
        let Some(frame) = Arc::make_mut(&mut self.group_roots).last_mut() else {
            return;
        };
        let key = CodeTableKey { kind, ch };
        if frame.local_runs.insert(key) {
            frame.saved.push(CodeTableRestoreRecord {
                save_position,
                kind,
                ch,
                value: old,
                retaining: false,
            });
            self.save_stack_words = self.save_stack_words.saturating_add(2);
            self.latest_save = Some((save_position, 2));
        }
    }

    fn record_global_trace(&mut self, kind: CodeTableKind, ch: char) {
        let key = CodeTableKey { kind, ch };
        if !self
            .group_roots
            .iter()
            .any(|frame| frame.local_runs.contains(&key))
        {
            return;
        }
        for frame in Arc::make_mut(&mut self.group_roots) {
            if frame.local_runs.remove(&key)
                && let Some(saved) = frame
                    .saved
                    .iter_mut()
                    .rev()
                    .find(|saved| saved.kind == kind && saved.ch == ch)
            {
                saved.retaining = true;
            }
        }
    }

    fn current_value(&self, kind: CodeTableKind, ch: char) -> i64 {
        match kind {
            CodeTableKind::Catcode => i64::from(self.catcodes.get(ch) as u8),
            CodeTableKind::Lccode => i64::from(self.lccodes.get(ch)),
            CodeTableKind::Uccode => i64::from(self.uccodes.get(ch)),
            CodeTableKind::Sfcode => i64::from(self.sfcodes.get(ch)),
            CodeTableKind::Mathcode => i64::from(self.mathcodes.get(ch)),
            CodeTableKind::Delcode => i64::from(self.delcodes.get(ch)),
        }
    }

    #[cfg(test)]
    pub(crate) fn testing_hash_content(&self, hasher: &mut impl Hasher) {
        self.catcodes.hash_content(hasher);
        self.lccodes.hash_content(hasher);
        self.uccodes.hash_content(hasher);
        self.sfcodes.hash_content(hasher);
        self.mathcodes.hash_content(hasher);
        self.delcodes.hash_content(hasher);
    }

    pub(crate) fn for_each_non_default(&self, mut visit: impl FnMut(char, CodeTableValues)) {
        let mut catcodes = self.catcodes.allocated_page_indices().peekable();
        let mut lccodes = self.lccodes.allocated_page_indices().peekable();
        let mut uccodes = self.uccodes.allocated_page_indices().peekable();
        let mut sfcodes = self.sfcodes.allocated_page_indices().peekable();
        let mut mathcodes = self.mathcodes.allocated_page_indices().peekable();
        let mut delcodes = self.delcodes.allocated_page_indices().peekable();

        loop {
            let Some(page_index) = [
                catcodes.peek().copied(),
                lccodes.peek().copied(),
                uccodes.peek().copied(),
                sfcodes.peek().copied(),
                mathcodes.peek().copied(),
                delcodes.peek().copied(),
            ]
            .into_iter()
            .flatten()
            .min() else {
                break;
            };
            if catcodes.peek() == Some(&page_index) {
                catcodes.next();
            }
            if lccodes.peek() == Some(&page_index) {
                lccodes.next();
            }
            if uccodes.peek() == Some(&page_index) {
                uccodes.next();
            }
            if sfcodes.peek() == Some(&page_index) {
                sfcodes.next();
            }
            if mathcodes.peek() == Some(&page_index) {
                mathcodes.next();
            }
            if delcodes.peek() == Some(&page_index) {
                delcodes.next();
            }

            let start = (page_index * PAGE_LEN) as u32;
            for offset in 0..PAGE_LEN as u32 {
                let code = start + offset;
                let Some(ch) = char::from_u32(code) else {
                    continue;
                };
                let values = CodeTableValues {
                    catcode: self.catcode(ch),
                    lccode: self.lccode(ch),
                    uccode: self.uccode(ch),
                    sfcode: self.sfcode(ch),
                    mathcode: self.mathcode(ch),
                    delcode: self.delcode(ch),
                };
                if values
                    != (CodeTableValues {
                        catcode: CatcodeDefaults::default_for(code),
                        lccode: LcCodeDefaults::default_for(code),
                        uccode: UcCodeDefaults::default_for(code),
                        sfcode: SfCodeDefaults::default_for(code),
                        mathcode: MathCodeDefaults::default_for(code),
                        delcode: DelCodeDefaults::default_for(code),
                    })
                {
                    visit(ch, values);
                }
            }
        }
    }

    pub(crate) fn for_each_non_default_catcode(&self, visit: impl FnMut(char, Catcode)) {
        self.catcodes.for_each_non_default(visit);
    }

    pub(crate) fn for_each_non_default_lccode(&self, visit: impl FnMut(char, LcCode)) {
        self.lccodes.for_each_non_default(visit);
    }

    pub(crate) fn for_each_non_default_uccode(&self, visit: impl FnMut(char, UcCode)) {
        self.uccodes.for_each_non_default(visit);
    }

    pub(crate) fn for_each_non_default_sfcode(&self, visit: impl FnMut(char, SfCode)) {
        self.sfcodes.for_each_non_default(visit);
    }

    pub(crate) fn for_each_non_default_mathcode(&self, visit: impl FnMut(char, MathCode)) {
        self.mathcodes.for_each_non_default(visit);
    }

    pub(crate) fn for_each_non_default_delcode(&self, visit: impl FnMut(char, DelCode)) {
        self.delcodes.for_each_non_default(visit);
    }
}

#[derive(Clone, Debug)]
struct PagedTable<T, D>
where
    T: Copy + Eq,
    D: Defaults<T> + StaticDefaultRoot<T>,
{
    root: Arc<Root<T>>,
    generation: u32,
    _defaults: core::marker::PhantomData<D>,
}

impl<T, D> PagedTable<T, D>
where
    T: Copy + Eq,
    D: Defaults<T> + StaticDefaultRoot<T>,
{
    fn new() -> Self {
        Self {
            root: D::default_root(),
            generation: 0,
            _defaults: core::marker::PhantomData,
        }
    }

    fn generation(&self) -> u32 {
        self.generation
    }

    fn get(&self, ch: char) -> T {
        Self::value_in_root(&self.root, ch)
    }

    fn set(&mut self, ch: char, value: T) {
        let (page_index, offset) = location(ch);
        self.generation = self
            .generation
            .checked_add(1)
            .expect("code-table generation overflow");

        if Self::value_in_root(&self.root, ch) == value {
            return;
        }

        Self::write_value(&mut self.root, page_index, offset, value);
        if self.root.is_empty() {
            self.root = D::default_root();
        }
    }

    fn root(&self) -> Arc<Root<T>> {
        Arc::clone(&self.root)
    }

    fn root_with_value(root: &Arc<Root<T>>, ch: char, value: T) -> Arc<Root<T>> {
        let (page_index, offset) = location(ch);
        if Self::value_in_root(root, ch) == value {
            return Arc::clone(root);
        }

        let mut updated = Arc::clone(root);
        Self::write_value(&mut updated, page_index, offset, value);
        if updated.is_empty() {
            D::default_root()
        } else {
            updated
        }
    }

    fn restore_group_root(&mut self, root: Arc<Root<T>>) {
        if Arc::ptr_eq(&self.root, &root) {
            return;
        }
        self.root = root;
        self.generation = self
            .generation
            .checked_add(1)
            .expect("code-table generation overflow");
    }

    fn checkpoint(&self) -> PagedTableSnapshot<T> {
        PagedTableSnapshot {
            root: Arc::clone(&self.root),
            generation: self.generation,
        }
    }

    fn rollback_to(&mut self, snapshot: PagedTableSnapshot<T>) {
        self.root = snapshot.root;
        self.generation = snapshot.generation;
    }

    #[cfg(test)]
    fn hash_content(&self, hasher: &mut impl Hasher)
    where
        T: Hash,
    {
        self.generation.hash(hasher);
        for page_index in 0..ROOT_LEN {
            if let Some(page) = self.root.page(page_index) {
                page.values.hash(hasher);
            } else {
                Page::default_for::<D>(page_index).values.hash(hasher);
            }
        }
    }

    fn value_in_root(root: &Root<T>, ch: char) -> T {
        let (page_index, offset) = location(ch);
        root.page(page_index)
            .map_or_else(|| D::default_for(ch as u32), |page| page.values[offset])
    }

    fn allocated_page_indices(&self) -> impl Iterator<Item = usize> + '_ {
        self.root
            .chunks
            .iter()
            .enumerate()
            .flat_map(|(chunk_index, chunk)| {
                chunk.iter().flat_map(move |chunk| {
                    chunk
                        .pages
                        .iter()
                        .enumerate()
                        .filter_map(move |(page_offset, page)| {
                            page.as_ref()
                                .map(|_| chunk_index * ROOT_CHUNK_LEN + page_offset)
                        })
                })
            })
    }

    fn for_each_non_default(&self, mut visit: impl FnMut(char, T)) {
        for page_index in self.allocated_page_indices() {
            let page = self
                .root
                .page(page_index)
                .expect("allocated page iterator must yield live pages");
            let start = page_index as u32 * PAGE_LEN as u32;
            for (offset, &value) in page.values.iter().enumerate() {
                let code = start + offset as u32;
                if value != D::default_for(code)
                    && let Some(ch) = char::from_u32(code)
                {
                    visit(ch, value);
                }
            }
        }
    }

    fn write_value(root: &mut Arc<Root<T>>, page_index: usize, offset: usize, value: T) {
        let (chunk_index, page_slot) = page_location(page_index);
        let root = Arc::make_mut(root);
        let chunk = root.chunks[chunk_index].get_or_insert_with(|| Arc::new(PageChunk::empty()));
        let chunk = Arc::make_mut(chunk);
        let page = chunk.pages[page_slot]
            .get_or_insert_with(|| Arc::new(Page::default_for::<D>(page_index)));
        Arc::make_mut(page).values[offset] = value;

        if page.is_default_for::<D>(page_index) {
            chunk.pages[page_slot] = None;
        }
        if chunk.is_empty() {
            root.chunks[chunk_index] = None;
        }
    }
}

#[derive(Clone, Debug)]
struct PagedTableSnapshot<T> {
    root: Arc<Root<T>>,
    generation: u32,
}

#[derive(Clone, Debug)]
struct Root<T> {
    chunks: [Option<Arc<PageChunk<T>>>; ROOT_CHUNK_COUNT],
}

impl<T> Root<T> {
    fn empty() -> Self {
        Self {
            chunks: array::from_fn(|_| None),
        }
    }

    fn page(&self, page_index: usize) -> Option<&Page<T>> {
        let (chunk_index, page_slot) = page_location(page_index);
        self.chunks[chunk_index]
            .as_deref()
            .and_then(|chunk| chunk.pages[page_slot].as_deref())
    }

    fn is_empty(&self) -> bool {
        self.chunks.iter().all(Option::is_none)
    }
}

#[derive(Clone, Debug)]
struct PageChunk<T> {
    pages: [Option<Arc<Page<T>>>; ROOT_CHUNK_LEN],
}

impl<T> PageChunk<T> {
    fn empty() -> Self {
        Self {
            pages: array::from_fn(|_| None),
        }
    }

    fn is_empty(&self) -> bool {
        self.pages.iter().all(Option::is_none)
    }
}

#[derive(Clone, Debug)]
struct Page<T> {
    values: [T; PAGE_LEN],
}

impl<T> Page<T>
where
    T: Copy + Eq,
{
    fn default_for<D>(page: usize) -> Self
    where
        D: Defaults<T>,
    {
        let base = page as u32 * PAGE_LEN as u32;
        Self {
            values: array::from_fn(|offset| D::default_for(base + offset as u32)),
        }
    }

    fn is_default_for<D>(&self, page: usize) -> bool
    where
        D: Defaults<T>,
    {
        let base = page as u32 * PAGE_LEN as u32;
        self.values
            .iter()
            .enumerate()
            .all(|(offset, value)| *value == D::default_for(base + offset as u32))
    }
}

trait Defaults<T> {
    fn default_for(code: u32) -> T;
}

trait StaticDefaultRoot<T> {
    fn default_root() -> Arc<Root<T>>;
}

fn build_default_root<T>() -> Arc<Root<T>> {
    Arc::new(Root::empty())
}

#[derive(Clone, Debug)]
struct CatcodeDefaults;

impl Defaults<Catcode> for CatcodeDefaults {
    /// tex.web §232 initializes `cat_code(k)` to `other_char` for every `k`,
    /// then overrides exactly six single characters -- `^^@` (ignore), `^^M`
    /// (car_ret), space (spacer), `%` (comment), `\` (escape), and `^^?`
    /// (invalid_char) -- plus the ASCII letters.
    ///
    /// `{ } $ & # ^ _` are deliberately absent: they are `other_char` in
    /// INITEX and only become grouping, math-shift, alignment, parameter, and
    /// script characters when a format assigns them (plain.tex lines 11-17).
    fn default_for(code: u32) -> Catcode {
        match code {
            NULL_CODE => Catcode::Ignored,
            CARRIAGE_RETURN => Catcode::EndLine,
            ASCII_SPACE => Catcode::Space,
            ASCII_PERCENT => Catcode::Comment,
            ASCII_BACKSLASH => Catcode::Escape,
            INVALID_CODE => Catcode::Invalid,
            ASCII_A..=ASCII_Z | ASCII_LOWER_A..=ASCII_LOWER_Z => Catcode::Letter,
            _ => Catcode::Other,
        }
    }
}

impl StaticDefaultRoot<Catcode> for CatcodeDefaults {
    fn default_root() -> Arc<Root<Catcode>> {
        static ROOT: OnceLock<Arc<Root<Catcode>>> = OnceLock::new();
        Arc::clone(ROOT.get_or_init(build_default_root::<Catcode>))
    }
}

#[derive(Clone, Debug)]
struct LcCodeDefaults;

impl Defaults<LcCode> for LcCodeDefaults {
    fn default_for(code: u32) -> LcCode {
        match code {
            ASCII_A..=ASCII_Z => code + 32,
            ASCII_LOWER_A..=ASCII_LOWER_Z => code,
            _ => 0,
        }
    }
}

impl StaticDefaultRoot<LcCode> for LcCodeDefaults {
    fn default_root() -> Arc<Root<LcCode>> {
        static ROOT: OnceLock<Arc<Root<LcCode>>> = OnceLock::new();
        Arc::clone(ROOT.get_or_init(build_default_root::<LcCode>))
    }
}

#[derive(Clone, Debug)]
struct UcCodeDefaults;

impl Defaults<UcCode> for UcCodeDefaults {
    fn default_for(code: u32) -> UcCode {
        match code {
            ASCII_A..=ASCII_Z => code,
            ASCII_LOWER_A..=ASCII_LOWER_Z => code - 32,
            _ => 0,
        }
    }
}

impl StaticDefaultRoot<UcCode> for UcCodeDefaults {
    fn default_root() -> Arc<Root<UcCode>> {
        static ROOT: OnceLock<Arc<Root<UcCode>>> = OnceLock::new();
        Arc::clone(ROOT.get_or_init(build_default_root::<UcCode>))
    }
}

#[derive(Clone, Debug)]
struct SfCodeDefaults;

impl Defaults<SfCode> for SfCodeDefaults {
    fn default_for(code: u32) -> SfCode {
        match code {
            ASCII_A..=ASCII_Z => 999,
            _ => 1000,
        }
    }
}

impl StaticDefaultRoot<SfCode> for SfCodeDefaults {
    fn default_root() -> Arc<Root<SfCode>> {
        static ROOT: OnceLock<Arc<Root<SfCode>>> = OnceLock::new();
        Arc::clone(ROOT.get_or_init(build_default_root::<SfCode>))
    }
}

#[derive(Clone, Debug)]
struct MathCodeDefaults;

impl Defaults<MathCode> for MathCodeDefaults {
    fn default_for(code: u32) -> MathCode {
        match code {
            ASCII_ZERO..=ASCII_NINE => VARIABLE_MATH_CLASS | code,
            ASCII_A..=ASCII_Z | ASCII_LOWER_A..=ASCII_LOWER_Z => {
                VARIABLE_MATH_CLASS | LETTER_MATH_FAMILY | code
            }
            _ => code,
        }
    }
}

impl StaticDefaultRoot<MathCode> for MathCodeDefaults {
    fn default_root() -> Arc<Root<MathCode>> {
        static ROOT: OnceLock<Arc<Root<MathCode>>> = OnceLock::new();
        Arc::clone(ROOT.get_or_init(build_default_root::<MathCode>))
    }
}

#[derive(Clone, Debug)]
struct DelCodeDefaults;

impl Defaults<DelCode> for DelCodeDefaults {
    /// tex.web §240: INITEX sets every `del_code` to `-1` except
    /// `del_code(".")`, the null delimiter used in error recovery, which is
    /// `0`. Formats do not restore it (plain.tex line 121 records the same
    /// convention), so it belongs in the defaults.
    fn default_for(code: u32) -> DelCode {
        if code == ASCII_PERIOD {
            0
        } else {
            DELCODE_DEFAULT
        }
    }
}

impl StaticDefaultRoot<DelCode> for DelCodeDefaults {
    fn default_root() -> Arc<Root<DelCode>> {
        static ROOT: OnceLock<Arc<Root<DelCode>>> = OnceLock::new();
        Arc::clone(ROOT.get_or_init(build_default_root::<DelCode>))
    }
}

fn location(ch: char) -> (usize, usize) {
    let code = ch as u32;
    ((code >> PAGE_BITS) as usize, (code & PAGE_MASK) as usize)
}

fn page_location(page_index: usize) -> (usize, usize) {
    (page_index / ROOT_CHUNK_LEN, page_index % ROOT_CHUNK_LEN)
}

fn assert_unicode_code(value: u32, table: &str) {
    assert!(
        value < UNICODE_SCALAR_COUNT as u32,
        "{table} value exceeds Unicode scalar range"
    );
}

mod global;

#[cfg(test)]
mod tests;
