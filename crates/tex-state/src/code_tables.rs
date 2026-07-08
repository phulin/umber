//! TeX code tables over Unicode scalar values.
//!
//! Code-table writes are sparse, so each table is represented as a 256-way
//! root of 256-entry pages. Snapshot history is structural: snapshots keep old
//! roots and writes copy a touched shared page before replacing the value.
//! Generations track write events, including same-value assignments, so lexer
//! classifiers can invalidate on assignment activity rather than value changes.

use crate::token::Catcode;
use core::array;
#[cfg(any(test, feature = "testing", feature = "shadow"))]
use std::hash::{Hash, Hasher};
use std::sync::{Arc, OnceLock};

const PAGE_BITS: u32 = 8;
const PAGE_LEN: usize = 1 << PAGE_BITS;
const PAGE_MASK: u32 = PAGE_LEN as u32 - 1;
const UNICODE_SCALAR_COUNT: usize = 0x11_0000;
const ROOT_LEN: usize = UNICODE_SCALAR_COUNT / PAGE_LEN;
const DELCODE_DEFAULT: i32 = -1;
const ASCII_A: u32 = b'A' as u32;
const ASCII_Z: u32 = b'Z' as u32;
const ASCII_LOWER_A: u32 = b'a' as u32;
const ASCII_LOWER_Z: u32 = b'z' as u32;

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
        }
    }

    pub(crate) fn rollback_to(&mut self, snapshot: CodeTablesSnapshot) {
        self.catcodes.rollback_to(snapshot.catcodes);
        self.lccodes.rollback_to(snapshot.lccodes);
        self.uccodes.rollback_to(snapshot.uccodes);
        self.sfcodes.rollback_to(snapshot.sfcodes);
        self.mathcodes.rollback_to(snapshot.mathcodes);
        self.delcodes.rollback_to(snapshot.delcodes);
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

    #[must_use]
    pub fn lccode(&self, ch: char) -> LcCode {
        self.lccodes.get(ch)
    }

    pub(crate) fn set_lccode(&mut self, ch: char, value: LcCode) {
        assert_unicode_code(value, "lccode");
        self.lccodes.set(ch, value);
    }

    #[must_use]
    pub fn uccode(&self, ch: char) -> UcCode {
        self.uccodes.get(ch)
    }

    pub(crate) fn set_uccode(&mut self, ch: char, value: UcCode) {
        assert_unicode_code(value, "uccode");
        self.uccodes.set(ch, value);
    }

    #[must_use]
    pub fn sfcode(&self, ch: char) -> SfCode {
        self.sfcodes.get(ch)
    }

    pub(crate) fn set_sfcode(&mut self, ch: char, value: SfCode) {
        self.sfcodes.set(ch, value);
    }

    #[must_use]
    pub fn mathcode(&self, ch: char) -> MathCode {
        self.mathcodes.get(ch)
    }

    pub(crate) fn set_mathcode(&mut self, ch: char, value: MathCode) {
        self.mathcodes.set(ch, value);
    }

    #[must_use]
    pub fn delcode(&self, ch: char) -> DelCode {
        self.delcodes.get(ch)
    }

    pub(crate) fn set_delcode(&mut self, ch: char, value: DelCode) {
        self.delcodes.set(ch, value);
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
        let (page, offset) = location(ch);
        self.root.pages[page].values[offset]
    }

    fn set(&mut self, ch: char, value: T) {
        let (page_index, offset) = location(ch);
        self.generation = self
            .generation
            .checked_add(1)
            .expect("code-table generation overflow");

        if self.root.pages[page_index].values[offset] == value {
            return;
        }

        let root = Arc::make_mut(&mut self.root);
        let page = Arc::make_mut(&mut root.pages[page_index]);
        page.values[offset] = value;
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
        for page in self.root.pages.iter() {
            page.values.hash(hasher);
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
    pages: Box<[Arc<Page<T>>]>,
}

#[derive(Clone, Debug)]
struct Page<T> {
    values: [T; PAGE_LEN],
}

impl<T> Page<T>
where
    T: Copy,
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
}

trait Defaults<T> {
    fn default_for(code: u32) -> T;
}

trait StaticDefaultRoot<T> {
    fn default_root() -> Arc<Root<T>>;
}

fn build_default_root<T, D>() -> Arc<Root<T>>
where
    T: Copy,
    D: Defaults<T>,
{
    let pages: Vec<_> = (0..ROOT_LEN)
        .map(|page| Arc::new(Page::default_for::<D>(page)))
        .collect();
    Arc::new(Root {
        pages: pages.into_boxed_slice(),
    })
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
        Arc::clone(ROOT.get_or_init(build_default_root::<Catcode, CatcodeDefaults>))
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
        Arc::clone(ROOT.get_or_init(build_default_root::<LcCode, LcCodeDefaults>))
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
        Arc::clone(ROOT.get_or_init(build_default_root::<UcCode, UcCodeDefaults>))
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
        Arc::clone(ROOT.get_or_init(build_default_root::<SfCode, SfCodeDefaults>))
    }
}

#[derive(Clone, Debug)]
struct MathCodeDefaults;

impl Defaults<MathCode> for MathCodeDefaults {
    fn default_for(code: u32) -> MathCode {
        code
    }
}

impl StaticDefaultRoot<MathCode> for MathCodeDefaults {
    fn default_root() -> Arc<Root<MathCode>> {
        static ROOT: OnceLock<Arc<Root<MathCode>>> = OnceLock::new();
        Arc::clone(ROOT.get_or_init(build_default_root::<MathCode, MathCodeDefaults>))
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
        Arc::clone(ROOT.get_or_init(build_default_root::<DelCode, DelCodeDefaults>))
    }
}

fn location(ch: char) -> (usize, usize) {
    let code = ch as u32;
    ((code >> PAGE_BITS) as usize, (code & PAGE_MASK) as usize)
}

fn assert_unicode_code(value: u32, table: &str) {
    assert!(
        value < UNICODE_SCALAR_COUNT as u32,
        "{table} value exceeds Unicode scalar range"
    );
}

#[cfg(test)]
mod tests {
    use super::{CodeTableGenerations, CodeTables, location};
    use crate::token::Catcode;
    use proptest::prelude::*;
    use std::sync::Arc;

    #[test]
    fn initex_catcode_defaults_match_tex82_ascii() {
        let tables = CodeTables::new();

        assert_eq!(tables.catcode('\0'), Catcode::Ignored);
        assert_eq!(tables.catcode('\r'), Catcode::EndLine);
        assert_eq!(tables.catcode(' '), Catcode::Space);
        assert_eq!(tables.catcode('\\'), Catcode::Escape);
        assert_eq!(tables.catcode('{'), Catcode::BeginGroup);
        assert_eq!(tables.catcode('}'), Catcode::EndGroup);
        assert_eq!(tables.catcode('$'), Catcode::MathShift);
        assert_eq!(tables.catcode('&'), Catcode::AlignmentTab);
        assert_eq!(tables.catcode('#'), Catcode::Parameter);
        assert_eq!(tables.catcode('^'), Catcode::Superscript);
        assert_eq!(tables.catcode('_'), Catcode::Subscript);
        assert_eq!(tables.catcode('%'), Catcode::Comment);
        assert_eq!(tables.catcode('\u{7f}'), Catcode::Invalid);
        assert_eq!(tables.catcode('A'), Catcode::Letter);
        assert_eq!(tables.catcode('z'), Catcode::Letter);
        assert_eq!(tables.catcode('@'), Catcode::Other);
        assert_eq!(tables.catcode('é'), Catcode::Other);
    }

    #[test]
    fn initex_case_space_math_and_delimiter_defaults() {
        let tables = CodeTables::new();

        assert_eq!(tables.lccode('A'), u32::from('a'));
        assert_eq!(tables.lccode('a'), u32::from('a'));
        assert_eq!(tables.lccode('@'), 0);
        assert_eq!(tables.uccode('A'), u32::from('A'));
        assert_eq!(tables.uccode('a'), u32::from('A'));
        assert_eq!(tables.uccode('@'), 0);
        assert_eq!(tables.sfcode('A'), 999);
        assert_eq!(tables.sfcode('a'), 1000);
        assert_eq!(tables.sfcode('é'), 1000);
        assert_eq!(tables.mathcode('A'), u32::from('A'));
        assert_eq!(tables.mathcode('é'), u32::from('é'));
        assert_eq!(tables.delcode('A'), -1);
    }

    #[test]
    fn snapshot_restores_roots_and_generations() {
        let mut tables = CodeTables::new();
        let snapshot = tables.checkpoint();
        let generation = tables.generations();

        tables.set_catcode('@', Catcode::Letter);
        tables.set_lccode('@', u32::from('a'));
        tables.set_uccode('@', u32::from('A'));
        tables.set_sfcode('A', 1000);
        tables.set_mathcode('∑', 0x1350);
        tables.set_delcode('[', 0x45);

        assert_ne!(tables.generations(), generation);
        tables.rollback_to(snapshot);

        assert_eq!(tables.generations(), generation);
        assert_eq!(tables.catcode('@'), Catcode::Other);
        assert_eq!(tables.lccode('@'), 0);
        assert_eq!(tables.uccode('@'), 0);
        assert_eq!(tables.sfcode('A'), 999);
        assert_eq!(tables.mathcode('∑'), u32::from('∑'));
        assert_eq!(tables.delcode('['), -1);
    }

    #[test]
    fn snapshots_keep_old_shared_pages_after_copy_on_write() {
        let mut tables = CodeTables::new();
        let snapshot = tables.checkpoint();

        tables.set_catcode('@', Catcode::Letter);
        assert_eq!(tables.catcode('@'), Catcode::Letter);
        let (page, offset) = location('@');
        assert_eq!(
            snapshot.catcodes.root.pages[page].values[offset],
            Catcode::Other
        );
    }

    #[test]
    fn new_tables_share_canonical_default_roots_and_pages() {
        let first = CodeTables::new();
        let second = CodeTables::new();

        assert!(Arc::ptr_eq(&first.catcodes.root, &second.catcodes.root));
        assert!(Arc::ptr_eq(
            &first.catcodes.root.pages[0],
            &second.catcodes.root.pages[0]
        ));
        assert!(Arc::ptr_eq(&first.lccodes.root, &second.lccodes.root));
        assert!(Arc::ptr_eq(&first.uccodes.root, &second.uccodes.root));
        assert!(Arc::ptr_eq(&first.sfcodes.root, &second.sfcodes.root));
        assert!(Arc::ptr_eq(&first.mathcodes.root, &second.mathcodes.root));
        assert!(Arc::ptr_eq(&first.delcodes.root, &second.delcodes.root));
    }

    #[test]
    fn checkpoint_captures_root_pointers_without_cloning_root_arrays() {
        let mut tables = CodeTables::new();
        tables.set_catcode('@', Catcode::Letter);
        let snapshot = tables.checkpoint();

        assert!(Arc::ptr_eq(&tables.catcodes.root, &snapshot.catcodes.root));
        let old_root = Arc::clone(&snapshot.catcodes.root);

        tables.set_catcode('!', Catcode::Letter);

        assert!(!Arc::ptr_eq(&tables.catcodes.root, &old_root));
        assert_eq!(tables.catcode('!'), Catcode::Letter);
        let (page, offset) = location('!');
        assert_eq!(
            snapshot.catcodes.root.pages[page].values[offset],
            Catcode::Other
        );
    }

    #[test]
    fn no_op_write_bumps_generation_without_copying_root() {
        let mut tables = CodeTables::new();
        let generation = tables.generations();
        let snapshot = tables.checkpoint();

        tables.set_catcode('@', Catcode::Other);

        assert_eq!(tables.generations().catcode, generation.catcode + 1);
        assert_eq!(tables.catcode('@'), Catcode::Other);
        assert!(Arc::ptr_eq(&tables.catcodes.root, &snapshot.catcodes.root));
    }

    proptest! {
        #[test]
        fn structural_persistence_restores_catcode_roots(
            ch in any::<char>(),
            replacement in 0_u8..=15,
        ) {
            let replacement = catcode_from_u8(replacement);
            let mut tables = CodeTables::new();
            let before = tables.catcode(ch);
            let generation = tables.generations();
            let snapshot = tables.checkpoint();

            tables.set_catcode(ch, replacement);
            prop_assert_eq!(
                tables.generations().catcode,
                generation.catcode + 1
            );

            tables.rollback_to(snapshot);
            prop_assert_eq!(tables.catcode(ch), before);
            prop_assert_eq!(tables.generations(), generation);
        }

        #[test]
        fn generation_bumps_once_per_code_table_write(
            ch in any::<char>(),
            lc in 0_u32..0x11_0000,
            uc in 0_u32..0x11_0000,
            sf in any::<u16>(),
            math in 0_u32..0x80_0000,
            del in -1_i32..0x80_0000,
        ) {
            let mut tables = CodeTables::new();
            let before = tables.generations();
            let expected = CodeTableGenerations {
                catcode: before.catcode,
                lccode: before.lccode + 1,
                uccode: before.uccode + 1,
                sfcode: before.sfcode + 1,
                mathcode: before.mathcode + 1,
                delcode: before.delcode + 1,
            };

            tables.set_lccode(ch, lc);
            tables.set_uccode(ch, uc);
            tables.set_sfcode(ch, sf);
            tables.set_mathcode(ch, math);
            tables.set_delcode(ch, del);

            prop_assert_eq!(tables.generations(), expected);
        }
    }

    fn catcode_from_u8(value: u8) -> Catcode {
        match value {
            0 => Catcode::Escape,
            1 => Catcode::BeginGroup,
            2 => Catcode::EndGroup,
            3 => Catcode::MathShift,
            4 => Catcode::AlignmentTab,
            5 => Catcode::EndLine,
            6 => Catcode::Parameter,
            7 => Catcode::Superscript,
            8 => Catcode::Subscript,
            9 => Catcode::Ignored,
            10 => Catcode::Space,
            11 => Catcode::Letter,
            12 => Catcode::Other,
            13 => Catcode::Active,
            14 => Catcode::Comment,
            15 => Catcode::Invalid,
            _ => unreachable!("strategy bounds catcodes"),
        }
    }
}
