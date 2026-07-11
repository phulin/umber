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
#[cfg(any(test, feature = "testing", feature = "shadow"))]
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
const VARIABLE_MATH_CLASS: u32 = 7 << 12;
const LETTER_MATH_FAMILY: u32 = 1 << 8;

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
        }
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
        };
        Arc::make_mut(&mut self.group_roots).push(roots);
    }

    pub(crate) fn leave_group(&mut self) {
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
        if self.group_roots.is_empty() {
            self.global_writes = GlobalWriteHistory::default();
        }
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

    pub(crate) fn set_catcode(&mut self, ch: char, value: Catcode) {
        self.catcodes.set(ch, value);
    }

    pub(crate) fn set_catcode_global(&mut self, ch: char, value: Catcode) {
        self.catcodes.set(ch, value);
        self.record_global(GlobalCodeTableWrite::Catcode(ch, value));
    }

    #[must_use]
    pub fn lccode(&self, ch: char) -> LcCode {
        self.lccodes.get(ch)
    }

    pub(crate) fn set_lccode(&mut self, ch: char, value: LcCode) {
        assert_unicode_code(value, "lccode");
        self.lccodes.set(ch, value);
    }

    pub(crate) fn set_lccode_global(&mut self, ch: char, value: LcCode) {
        assert_unicode_code(value, "lccode");
        self.lccodes.set(ch, value);
        self.record_global(GlobalCodeTableWrite::LcCode(ch, value));
    }

    #[must_use]
    pub fn uccode(&self, ch: char) -> UcCode {
        self.uccodes.get(ch)
    }

    pub(crate) fn set_uccode(&mut self, ch: char, value: UcCode) {
        assert_unicode_code(value, "uccode");
        self.uccodes.set(ch, value);
    }

    pub(crate) fn set_uccode_global(&mut self, ch: char, value: UcCode) {
        assert_unicode_code(value, "uccode");
        self.uccodes.set(ch, value);
        self.record_global(GlobalCodeTableWrite::UcCode(ch, value));
    }

    #[must_use]
    pub fn sfcode(&self, ch: char) -> SfCode {
        self.sfcodes.get(ch)
    }

    pub(crate) fn set_sfcode(&mut self, ch: char, value: SfCode) {
        self.sfcodes.set(ch, value);
    }

    pub(crate) fn set_sfcode_global(&mut self, ch: char, value: SfCode) {
        self.sfcodes.set(ch, value);
        self.record_global(GlobalCodeTableWrite::SfCode(ch, value));
    }

    #[must_use]
    pub fn mathcode(&self, ch: char) -> MathCode {
        self.mathcodes.get(ch)
    }

    pub(crate) fn set_mathcode(&mut self, ch: char, value: MathCode) {
        self.mathcodes.set(ch, value);
    }

    pub(crate) fn set_mathcode_global(&mut self, ch: char, value: MathCode) {
        self.mathcodes.set(ch, value);
        self.record_global(GlobalCodeTableWrite::MathCode(ch, value));
    }

    #[must_use]
    pub fn delcode(&self, ch: char) -> DelCode {
        self.delcodes.get(ch)
    }

    pub(crate) fn set_delcode(&mut self, ch: char, value: DelCode) {
        self.delcodes.set(ch, value);
    }

    pub(crate) fn set_delcode_global(&mut self, ch: char, value: DelCode) {
        self.delcodes.set(ch, value);
        self.record_global(GlobalCodeTableWrite::DelCode(ch, value));
    }

    fn record_global(&mut self, write: GlobalCodeTableWrite) {
        if !self.group_roots.is_empty() {
            self.global_writes.push(write);
        }
    }

    #[cfg(any(test, feature = "testing", feature = "shadow"))]
    pub(crate) fn testing_hash_content(&self, hasher: &mut impl Hasher) {
        self.catcodes.hash_content(hasher);
        self.lccodes.hash_content(hasher);
        self.uccodes.hash_content(hasher);
        self.sfcodes.hash_content(hasher);
        self.mathcodes.hash_content(hasher);
        self.delcodes.hash_content(hasher);
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

    #[cfg(any(test, feature = "testing", feature = "shadow"))]
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

    #[cfg(test)]
    fn materialized_page_count(&self) -> usize {
        self.chunks
            .iter()
            .filter_map(Option::as_deref)
            .map(|chunk| chunk.pages.iter().filter(|page| page.is_some()).count())
            .sum()
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
    fn default_for(code: u32) -> Catcode {
        match code {
            0 => Catcode::Ignored,
            13 => Catcode::EndLine,
            32 => Catcode::Space,
            92 => Catcode::Escape,
            123 => Catcode::BeginGroup,
            125 => Catcode::EndGroup,
            36 => Catcode::MathShift,
            38 => Catcode::AlignmentTab,
            35 => Catcode::Parameter,
            94 => Catcode::Superscript,
            95 => Catcode::Subscript,
            37 => Catcode::Comment,
            127 => Catcode::Invalid,
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
    fn default_for(_: u32) -> DelCode {
        DELCODE_DEFAULT
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
