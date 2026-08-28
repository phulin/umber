//! Direct-index dense and page/index dense mutable banks.

use core::array;

/// Number of dense classical register slots per bank.
pub const DENSE_REGISTER_COUNT: usize = 256;

/// Number of M1 parameter slots per parameter class.
pub const PARAMETER_COUNT: usize = 128;

/// Integer parameter index.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct IntParam(u16);

/// Dimension parameter index.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct DimenParam(u16);

/// Glue parameter index.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct GlueParam(u16);

/// Token-list parameter index.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TokParam(u16);

macro_rules! param_index {
    ($name:ident) => {
        impl $name {
            /// Creates a parameter index.
            #[must_use]
            pub const fn new(raw: u16) -> Self {
                assert!(
                    raw < PARAMETER_COUNT as u16,
                    "parameter index out of dense range"
                );
                Self(raw)
            }

            /// Returns the raw parameter index.
            #[must_use]
            pub const fn raw(self) -> u16 {
                self.0
            }
        }
    };
}

param_index!(IntParam);
param_index!(DimenParam);
param_index!(GlueParam);
param_index!(TokParam);

impl IntParam {
    /// TeX's first-pass paragraph badness cutoff.
    pub const PRETOLERANCE: Self = Self::new(0);

    /// TeX's paragraph badness cutoff.
    pub const TOLERANCE: Self = Self::new(1);

    /// TeX's per-line demerit parameter.
    pub const LINE_PENALTY: Self = Self::new(2);

    /// TeX's automatic hyphenation break penalty.
    pub const HYPHEN_PENALTY: Self = Self::new(3);

    /// TeX's explicit discretionary hyphen break penalty.
    pub const EX_HYPHEN_PENALTY: Self = Self::new(4);

    /// TeX's club-line penalty.
    pub const CLUB_PENALTY: Self = Self::new(5);

    /// TeX's widow-line penalty.
    pub const WIDOW_PENALTY: Self = Self::new(6);

    /// TeX's display-interrupted paragraph widow penalty.
    pub const DISPLAY_WIDOW_PENALTY: Self = Self::new(7);

    /// TeX's penalty inserted after binary operators in inline math.
    pub const BIN_OP_PENALTY: Self = Self::new(9);

    /// TeX's penalty inserted after relations in inline math.
    pub const REL_PENALTY: Self = Self::new(10);

    /// TeX's post-hyphenated-line penalty.
    pub const BROKEN_PENALTY: Self = Self::new(8);

    /// TeX's penalty before a display.
    pub const PRE_DISPLAY_PENALTY: Self = Self::new(11);

    /// TeX's penalty after a display.
    pub const POST_DISPLAY_PENALTY: Self = Self::new(12);

    /// TeX's inter-line penalty.
    pub const INTERLINE_PENALTY: Self = Self::new(13);

    /// TeX's demerits for consecutive hyphenated lines.
    pub const DOUBLE_HYPHEN_DEMERITS: Self = Self::new(14);

    /// TeX's demerits for a penultimate hyphenated line.
    pub const FINAL_HYPHEN_DEMERITS: Self = Self::new(15);

    /// TeX's demerits for adjacent incompatible line fitness.
    pub const ADJ_DEMERITS: Self = Self::new(16);

    /// TeX's `\mag` integer parameter.
    pub const MAG: Self = Self::new(17);

    /// TeX's variable delimiter scaling ratio.
    pub const DELIMITER_FACTOR: Self = Self::new(18);

    /// TeX's job-start minutes since midnight.
    pub const TIME: Self = Self::new(20);

    /// TeX's job-start day of month.
    pub const DAY: Self = Self::new(21);

    /// TeX's job-start month.
    pub const MONTH: Self = Self::new(22);

    /// TeX's job-start year.
    pub const YEAR: Self = Self::new(23);

    /// TeX's `\tracingonline` diagnostic-destination gate (tex.web §245).
    pub const TRACING_ONLINE: Self = Self::new(29);

    /// TeX's macro invocation and argument diagnostic level (tex.web
    /// §§389, 400).
    pub const TRACING_MACROS: Self = Self::new(30);

    /// TeX's end-of-job memory-usage diagnostic level (tex.web §1333).
    pub const TRACING_STATS: Self = Self::new(31);

    /// TeX's `\globaldefs` integer parameter.
    pub const GLOBAL_DEFS: Self = Self::new(32);

    /// TeX's `\tracingpages` page-cost diagnostic level (tex.web §§987, 1005).
    pub const TRACING_PAGES: Self = Self::new(34);

    /// TeX's line-breaking diagnostic level (tex.web §§826, 845, 855, 863).
    pub const TRACING_PARAGRAPHS: Self = Self::new(33);

    /// TeX's `\tracingoutput` shipped-page diagnostic level (tex.web §638).
    pub const TRACING_OUTPUT: Self = Self::new(35);

    /// TeX's missing-character diagnostic level.
    pub const TRACING_LOST_CHARS: Self = Self::new(36);

    /// TeX's main-control command diagnostic level (tex.web §1030).
    pub const TRACING_COMMANDS: Self = Self::new(37);

    /// TeX's save-stack restoration diagnostic level (tex.web §283).
    pub const TRACING_RESTORES: Self = Self::new(38);

    /// TeX's `\uchyph` uppercase-start hyphenation gate.
    pub const UC_HYPH: Self = Self::new(39);

    /// Plain TeX's `\escapechar` integer parameter.
    pub const ESCAPE_CHAR: Self = Self::new(40);

    /// Initial `\hyphenchar` value assigned to newly loaded fonts.
    pub const DEFAULT_HYPHEN_CHAR: Self = Self::new(41);

    /// Initial `\skewchar` value assigned to newly loaded fonts.
    pub const DEFAULT_SKEW_CHAR: Self = Self::new(42);

    /// TeX's `\pausing` interactive line-replacement parameter (tex.web
    /// §236). A positive value makes §363's `firm_up_the_line` display and
    /// offer to replace every line before it is tokenized.
    pub const PAUSING: Self = Self::new(28);

    /// Plain TeX's `\endlinechar` integer parameter.
    pub const END_LINE_CHAR: Self = Self::new(48);

    /// TeX's `\newlinechar` output-rendering integer parameter.
    pub const NEWLINE_CHAR: Self = Self::new(49);

    /// TeX's `\lefthyphenmin` paragraph-breaking parameter.
    pub const LEFT_HYPHEN_MIN: Self = Self::new(51);

    /// TeX's `\righthyphenmin` paragraph-breaking parameter.
    pub const RIGHT_HYPHEN_MIN: Self = Self::new(52);

    /// TeX's `\holdinginserts` output-routine parameter.
    pub const HOLDING_INSERTS: Self = Self::new(53);

    /// TeX's `\outputpenalty` parameter.
    pub const OUTPUT_PENALTY: Self = Self::new(55);

    /// TeX's `\maxdeadcycles` parameter.
    pub const MAX_DEAD_CYCLES: Self = Self::new(56);

    /// TeX's `\floatingpenalty` insertion parameter.
    pub const FLOATING_PENALTY: Self = Self::new(58);

    /// TeX's current math family parameter.
    pub const FAM: Self = Self::new(59);

    /// Hidden backing cell for TeX's read-only `\badness` internal integer.
    pub const LAST_BADNESS: Self = Self::new(60);

    /// e-TeX pseudo-file tracing switch.
    pub const TRACING_SCAN_TOKENS: Self = Self::new(61);
    /// e-TeX bidirectional typesetting enhancement switch.
    pub const TEX_XET_STATE: Self = Self::new(62);
    /// Direction preceding an e-TeX display.
    pub const PRE_DISPLAY_DIRECTION: Self = Self::new(63);
    /// e-TeX assignment tracing switch.
    pub const TRACING_ASSIGNS: Self = Self::new(64);
    /// e-TeX group entry/exit tracing switch.
    pub const TRACING_GROUPS: Self = Self::new(65);
    /// e-TeX conditional-branch tracing switch.
    pub const TRACING_IFS: Self = Self::new(66);
    /// e-TeX semantic-nesting tracing switch.
    pub const TRACING_NESTING: Self = Self::new(67);
    /// e-TeX switch retaining vertical material discarded at page tops.
    pub const SAVING_V_DISCARDS: Self = Self::new(68);
    /// e-TeX last-line paragraph fitting ratio.
    pub const LAST_LINE_FIT: Self = Self::new(69);
    /// e-TeX switch saving language-specific hyphenation codes at `\patterns`.
    pub const SAVING_HYPH_CODES: Self = Self::new(70);
    /// Hidden e-TeX extended-mode flag controlling compatibility-sensitive limits.
    pub const ETEX_EXTENDED_MODE: Self = Self::new(71);

    /// pdfTeX's DVI/PDF output selection.
    pub const PDF_OUTPUT: Self = Self::new(72);
    pub const PDF_COMPRESS_LEVEL: Self = Self::new(73);
    pub const PDF_OBJ_COMPRESS_LEVEL: Self = Self::new(74);
    pub const PDF_DECIMAL_DIGITS: Self = Self::new(75);
    pub const PDF_MOVE_CHARS: Self = Self::new(76);
    pub const PDF_IMAGE_RESOLUTION: Self = Self::new(77);
    pub const PDF_PK_RESOLUTION: Self = Self::new(78);
    pub const PDF_UNIQUE_RESNAME: Self = Self::new(79);
    pub const PDF_MINOR_VERSION: Self = Self::new(80);
    pub const PDF_FORCE_PAGE_BOX: Self = Self::new(81);
    pub const PDF_PAGE_BOX: Self = Self::new(82);
    pub const PDF_INCLUSION_ERROR_LEVEL: Self = Self::new(83);
    pub const PDF_MAJOR_VERSION: Self = Self::new(84);
    pub const PDF_GAMMA: Self = Self::new(85);
    pub const PDF_IMAGE_GAMMA: Self = Self::new(86);
    pub const PDF_IMAGE_HICOLOR: Self = Self::new(87);
    pub const PDF_IMAGE_APPLY_GAMMA: Self = Self::new(88);
    pub const PDF_ADJUST_SPACING: Self = Self::new(89);
    pub const PDF_PROTRUDE_CHARS: Self = Self::new(90);
    pub const PDF_TRACING_FONTS: Self = Self::new(91);
    pub const PDF_ADJUST_INTERWORD_GLUE: Self = Self::new(92);
    pub const PDF_PREPEND_KERN: Self = Self::new(93);
    pub const PDF_APPEND_KERN: Self = Self::new(94);
    pub const PDF_GEN_TO_UNICODE: Self = Self::new(95);
    pub const PDF_DRAFT_MODE: Self = Self::new(96);
    pub const PDF_INCLUSION_COPY_FONTS: Self = Self::new(97);
    pub const PDF_SUPPRESS_WARNING_DUP_DEST: Self = Self::new(98);
    pub const PDF_SUPPRESS_WARNING_DUP_MAP: Self = Self::new(99);
    pub const PDF_SUPPRESS_WARNING_PAGE_GROUP: Self = Self::new(100);
    pub const PDF_INFO_OMIT_DATE: Self = Self::new(101);
    pub const PDF_SUPPRESS_PTEX_INFO: Self = Self::new(102);
    pub const PDF_OMIT_CHARSET: Self = Self::new(103);
    pub const PDF_OMIT_INFO_DICT: Self = Self::new(104);
    pub const PDF_OMIT_PROCSET: Self = Self::new(105);
    pub const PDF_PTEX_USE_UNDERSCORE: Self = Self::new(106);
    /// Obsolete `\pdfoptionalwaysusepdfpagebox` compatibility cell.
    ///
    /// pdfTeX keeps this separate from `\pdfforcepagebox` and transfers it
    /// only while scanning an external image.
    pub const PDF_OPTION_ALWAYS_USE_PDF_PAGE_BOX: Self = Self::new(107);
    /// Obsolete `\pdfoptionpdfinclusionerrorlevel` compatibility cell.
    ///
    /// pdfTeX keeps this separate from `\pdfinclusionerrorlevel` and
    /// transfers it only while scanning an external image.
    pub const PDF_OPTION_INCLUSION_ERROR_LEVEL: Self = Self::new(108);
    /// pdfTeX/e-TeX bitmask controlling explicitly ignorable primitive errors.
    ///
    /// Bit 1 suppresses the ordinary error recovery for infinite shrinkage
    /// encountered by `\vsplit` while retaining a one-line diagnostic.
    pub const IGNORE_PRIMITIVE_ERROR: Self = Self::new(109);

    /// Web2C/pdfTeX's paragraph-token insertion context.
    ///
    /// Zero retains TeX82's direct paragraph completion. Positive values
    /// replay `\par` at vertical-box boundaries, and values greater than one
    /// extend that replay to insertion, output, alignment-item, and no-align
    /// boundaries.
    pub const PAR_TOKEN_CONTEXT: Self = Self::new(110);

    /// TeX Live Web2C [54/SyncTeX]'s synchronization integer parameter.
    pub const SYNCTEX: Self = Self::new(111);

    /// Current hyphenation language.
    pub const LANGUAGE: Self = Self::new(50);

    /// TeX's `\showboxbreadth` integer parameter.
    pub const SHOW_BOX_BREADTH: Self = Self::new(24);

    /// TeX's `\showboxdepth` integer parameter.
    pub const SHOW_BOX_DEPTH: Self = Self::new(25);

    /// TeX's `\hbadness` integer parameter.
    pub const HBADNESS: Self = Self::new(26);

    /// TeX's `\vbadness` integer parameter.
    pub const VBADNESS: Self = Self::new(27);

    /// TeX's `\looseness` paragraph-breaking parameter.
    pub const LOOSENESS: Self = Self::new(19);

    /// TeX's `\hangafter` paragraph-shape parameter.
    pub const HANG_AFTER: Self = Self::new(57);
}

impl DimenParam {
    /// TeX's `\parindent` dimension parameter.
    pub const PAR_INDENT: Self = Self::new(0);

    /// TeX's `\mathsurround` dimension parameter.
    pub const MATH_SURROUND: Self = Self::new(1);

    /// TeX's `\lineskiplimit` dimension parameter.
    pub const LINE_SKIP_LIMIT: Self = Self::new(2);

    /// TeX's `\boxmaxdepth` dimension parameter.
    pub const BOX_MAX_DEPTH: Self = Self::new(7);

    /// TeX's `\hfuzz` dimension parameter.
    pub const HFUZZ: Self = Self::new(8);

    /// TeX's `\vfuzz` dimension parameter.
    pub const VFUZZ: Self = Self::new(9);

    /// TeX's variable delimiter shortfall allowance.
    pub const DELIMITER_SHORTFALL: Self = Self::new(10);

    /// TeX's width for a null delimiter.
    pub const NULL_DELIMITER_SPACE: Self = Self::new(11);

    /// TeX's last-line width measure before a display.
    pub const PRE_DISPLAY_SIZE: Self = Self::new(13);

    /// TeX's display line width.
    pub const DISPLAY_WIDTH: Self = Self::new(14);

    /// TeX's display line indent.
    pub const DISPLAY_INDENT: Self = Self::new(15);

    /// TeX's `\overfullrule` dimension parameter.
    pub const OVERFULL_RULE: Self = Self::new(16);

    /// TeX's `\hangindent` paragraph-shape parameter.
    pub const HANG_INDENT: Self = Self::new(17);

    /// TeX's line width parameter.
    pub const H_SIZE: Self = Self::new(3);

    /// TeX's page height parameter.
    pub const V_SIZE: Self = Self::new(4);

    /// TeX's maximum page depth parameter.
    pub const MAX_DEPTH: Self = Self::new(5);

    /// TeX's maximum split depth parameter.
    pub const SPLIT_MAX_DEPTH: Self = Self::new(6);

    /// TeX's horizontal page offset used by `ship_out`.
    pub const H_OFFSET: Self = Self::new(18);

    /// TeX's vertical page offset used by `ship_out`.
    pub const V_OFFSET: Self = Self::new(19);

    /// TeX's final-pass paragraph emergency stretch.
    pub const EMERGENCY_STRETCH: Self = Self::new(20);

    pub const PDF_H_ORIGIN: Self = Self::new(21);
    pub const PDF_V_ORIGIN: Self = Self::new(22);
    pub const PDF_PAGE_WIDTH: Self = Self::new(23);
    pub const PDF_PAGE_HEIGHT: Self = Self::new(24);
    pub const PDF_LINK_MARGIN: Self = Self::new(25);
    pub const PDF_DEST_MARGIN: Self = Self::new(26);
    pub const PDF_THREAD_MARGIN: Self = Self::new(27);
    pub const PDF_FIRST_LINE_HEIGHT: Self = Self::new(28);
    pub const PDF_LAST_LINE_DEPTH: Self = Self::new(29);
    pub const PDF_EACH_LINE_HEIGHT: Self = Self::new(30);
    pub const PDF_EACH_LINE_DEPTH: Self = Self::new(31);
    pub const PDF_IGNORED_DIMEN: Self = Self::new(32);
    pub const PDF_PX_DIMEN: Self = Self::new(33);
}

impl GlueParam {
    /// TeX's `\lineskip` glue parameter.
    pub const LINE_SKIP: Self = Self::new(0);

    /// TeX's `\baselineskip` glue parameter.
    pub const BASELINE_SKIP: Self = Self::new(1);

    /// TeX's `\topskip` glue parameter.
    pub const TOP_SKIP: Self = Self::new(9);

    /// TeX's `\splittopskip` glue parameter.
    pub const SPLIT_TOP_SKIP: Self = Self::new(10);

    /// TeX's `\tabskip` glue parameter.
    pub const TAB_SKIP: Self = Self::new(11);

    /// TeX's `\spaceskip` glue parameter.
    pub const SPACE_SKIP: Self = Self::new(12);

    /// TeX's `\xspaceskip` glue parameter.
    pub const XSPACE_SKIP: Self = Self::new(13);

    /// TeX's `\parskip` glue parameter.
    pub const PAR_SKIP: Self = Self::new(2);

    /// TeX's `\leftskip` glue parameter.
    pub const LEFT_SKIP: Self = Self::new(7);

    /// TeX's `\rightskip` glue parameter.
    pub const RIGHT_SKIP: Self = Self::new(8);

    /// TeX's `\parfillskip` glue parameter.
    pub const PAR_FILL_SKIP: Self = Self::new(14);

    /// TeX's glue above a display.
    pub const ABOVE_DISPLAY_SKIP: Self = Self::new(3);

    /// TeX's glue below a display.
    pub const BELOW_DISPLAY_SKIP: Self = Self::new(4);

    /// TeX's short glue above a display.
    pub const ABOVE_DISPLAY_SHORT_SKIP: Self = Self::new(5);

    /// TeX's short glue below a display.
    pub const BELOW_DISPLAY_SHORT_SKIP: Self = Self::new(6);
}

impl TokParam {
    /// Internal immutable payloads backing e-TeX's scoped penalty arrays.
    /// These are not user-visible token-list parameters.
    pub(crate) const INTER_LINE_PENALTIES_INTERNAL: Self = Self::new(123);
    pub(crate) const CLUB_PENALTIES_INTERNAL: Self = Self::new(124);
    pub(crate) const WIDOW_PENALTIES_INTERNAL: Self = Self::new(125);
    pub(crate) const DISPLAY_WIDOW_PENALTIES_INTERNAL: Self = Self::new(126);

    /// Internal immutable payload backing TeX's scoped `\parshape` value.
    /// This is not a user-visible token-list parameter.
    pub(crate) const PAR_SHAPE_INTERNAL: Self = Self::new(127);

    /// TeX's `\output` token-list parameter.
    pub const OUTPUT: Self = Self::new(0);

    /// TeX's `\everypar` token-list parameter.
    pub const EVERY_PAR: Self = Self::new(1);

    /// TeX's `\everymath` token-list parameter.
    pub const EVERY_MATH: Self = Self::new(2);

    /// TeX's `\everydisplay` token-list parameter.
    pub const EVERY_DISPLAY: Self = Self::new(3);

    /// TeX's token list inserted at the start of every explicit hbox.
    pub const EVERY_HBOX: Self = Self::new(4);

    /// TeX's token list inserted at the start of every explicit vbox or vtop.
    pub const EVERY_VBOX: Self = Self::new(5);

    /// TeX's token list inserted at the start of a format-loaded job.
    pub const EVERY_JOB: Self = Self::new(6);

    /// TeX's `\everycr` token-list parameter.
    pub const EVERY_CR: Self = Self::new(7);

    /// TeX's supplementary-help token list used after an error prompt.
    pub const ERR_HELP: Self = Self::new(8);

    /// e-TeX's token list inserted at natural real or virtual EOF.
    ///
    /// Slot 8 remains TeX's `\errhelp`; e-TeX state must not alias it.
    pub const EVERY_EOF: Self = Self::new(13);

    pub const PDF_PAGES_ATTR: Self = Self::new(9);
    pub const PDF_PAGE_ATTR: Self = Self::new(10);
    pub const PDF_PAGE_RESOURCES: Self = Self::new(11);
    pub const PDF_PK_MODE: Self = Self::new(12);
}

/// TeX's level-zero undefined value and level-one global value.
pub(crate) const LEVEL_ZERO: u32 = 0;
pub(crate) const LEVEL_ONE: u32 = 1;

const PAGE_BITS: u32 = 8;
const PAGE_LEN: usize = 1 << PAGE_BITS;
const PAGE_MASK: u32 = PAGE_LEN as u32 - 1;

#[cfg(test)]
#[path = "banks/tests.rs"]
mod tests;

/// One current value and its TeX assignment level.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BankCell<T> {
    pub(crate) value: T,
    pub(crate) level: u32,
}

impl<T> BankCell<T> {
    pub(crate) const fn level_zero(value: T) -> Self {
        Self {
            value,
            level: LEVEL_ZERO,
        }
    }

    pub(crate) const fn level_one(value: T) -> Self {
        Self {
            value,
            level: LEVEL_ONE,
        }
    }
}

/// A rejected bank access.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BankError {
    IndexOutOfBounds,
    AllocationFailed,
}

/// Contiguous current-value storage. Reads are one bounds-checked index.
#[derive(Clone)]
pub(crate) struct DenseBank<T: Clone> {
    cells: Vec<BankCell<T>>,
    default: T,
}

impl<T: Clone> DenseBank<T> {
    pub(crate) fn fixed(len: usize, default: T, level: u32) -> Result<Self, BankError> {
        let mut cells = Vec::new();
        cells
            .try_reserve_exact(len)
            .map_err(|_| BankError::AllocationFailed)?;
        cells.resize(
            len,
            BankCell {
                value: default.clone(),
                level,
            },
        );
        Ok(Self { cells, default })
    }

    pub(crate) fn growing(default: T) -> Self {
        Self {
            cells: Vec::new(),
            default,
        }
    }

    pub(crate) fn admit_through(&mut self, index: u32) -> Result<(), BankError> {
        let required = index as usize + 1;
        if required <= self.cells.len() {
            return Ok(());
        }
        self.cells
            .try_reserve_exact(required - self.cells.len())
            .map_err(|_| BankError::AllocationFailed)?;
        self.cells
            .resize(required, BankCell::level_zero(self.default.clone()));
        Ok(())
    }

    #[inline(always)]
    pub(crate) fn get(&self, index: u32) -> Result<BankCell<T>, BankError> {
        self.cells
            .get(index as usize)
            .cloned()
            .ok_or(BankError::IndexOutOfBounds)
    }

    /// Borrows one direct-indexed row without cloning its stored value.
    #[inline(always)]
    pub(crate) fn get_ref(&self, index: u32) -> Result<&BankCell<T>, BankError> {
        self.cells
            .get(index as usize)
            .ok_or(BankError::IndexOutOfBounds)
    }

    #[inline(always)]
    pub(crate) fn get_mut(&mut self, index: u32) -> Result<&mut BankCell<T>, BankError> {
        self.cells
            .get_mut(index as usize)
            .ok_or(BankError::IndexOutOfBounds)
    }

    #[inline(always)]
    pub(crate) fn write(&mut self, index: u32, cell: BankCell<T>) -> Result<(), BankError> {
        *self
            .cells
            .get_mut(index as usize)
            .ok_or(BankError::IndexOutOfBounds)? = cell;
        Ok(())
    }

    pub(crate) fn values(&self) -> impl Iterator<Item = T> + '_ {
        self.cells.iter().map(|cell| cell.value.clone())
    }
}

type Page<T> = Box<[BankCell<T>; PAGE_LEN]>;

/// Sparse-allocation storage with direct page/index access.
///
/// The complete page directory is allocated once. A read performs no search
/// and absent pages evaluate their algorithmic default without allocation.
#[derive(Clone)]
pub(crate) struct PagedDenseBank<T: Clone> {
    pages: Vec<Option<Page<T>>>,
    len: u32,
    default: fn(u32) -> T,
    default_level: u32,
}

impl<T: Clone> PagedDenseBank<T> {
    pub(crate) fn new(
        len: u32,
        default: fn(u32) -> T,
        default_level: u32,
    ) -> Result<Self, BankError> {
        let page_count = len.div_ceil(PAGE_LEN as u32) as usize;
        let mut pages = Vec::new();
        pages
            .try_reserve_exact(page_count)
            .map_err(|_| BankError::AllocationFailed)?;
        pages.resize_with(page_count, || None);
        Ok(Self {
            pages,
            len,
            default,
            default_level,
        })
    }

    #[inline(always)]
    pub(crate) fn get(&self, index: u32) -> Result<BankCell<T>, BankError> {
        let (page, offset) = self.location(index)?;
        Ok(self.pages[page].as_ref().map_or_else(
            || BankCell {
                value: (self.default)(index),
                level: self.default_level,
            },
            |values| values[offset].clone(),
        ))
    }

    pub(crate) fn write(&mut self, index: u32, cell: BankCell<T>) -> Result<(), BankError> {
        let (page, offset) = self.location(index)?;
        if self.pages[page].is_none() {
            let base = page as u32 * PAGE_LEN as u32;
            let default = self.default;
            let default_level = self.default_level;
            let values = array::from_fn(|slot| BankCell {
                value: default(base + slot as u32),
                level: default_level,
            });
            self.pages[page] = Some(Box::new(values));
        }
        self.pages[page].as_mut().expect("page was installed")[offset] = cell;
        Ok(())
    }

    #[inline(always)]
    pub(crate) fn get_mut(&mut self, index: u32) -> Result<&mut BankCell<T>, BankError> {
        let (page, offset) = self.location(index)?;
        self.pages[page]
            .as_mut()
            .map(|values| &mut values[offset])
            .ok_or(BankError::IndexOutOfBounds)
    }

    #[must_use]
    pub(crate) fn allocated_pages(&self) -> usize {
        self.pages.iter().filter(|page| page.is_some()).count()
    }

    fn location(&self, index: u32) -> Result<(usize, usize), BankError> {
        if index >= self.len {
            return Err(BankError::IndexOutOfBounds);
        }
        Ok(((index >> PAGE_BITS) as usize, (index & PAGE_MASK) as usize))
    }
}

impl<T: Clone + PartialEq> PagedDenseBank<T> {
    pub(crate) fn nondefault_values(&self) -> impl Iterator<Item = (u32, T)> + '_ {
        self.pages
            .iter()
            .enumerate()
            .filter_map(|(page, values)| values.as_ref().map(|values| (page, values)))
            .flat_map(move |(page, values)| {
                values.iter().enumerate().filter_map(move |(offset, cell)| {
                    let index = page as u32 * PAGE_LEN as u32 + offset as u32;
                    (index < self.len && cell.value != (self.default)(index))
                        .then(|| (index, cell.value.clone()))
                })
            })
    }
}

/// TeX's dense 0--255 register prefix plus page/index dense e-TeX overflow.
#[derive(Clone)]
pub(crate) struct RegisterBank<T: Clone> {
    dense: [BankCell<T>; DENSE_REGISTER_COUNT],
    overflow: PagedDenseBank<T>,
}

impl<T: Clone> RegisterBank<T> {
    pub(crate) fn new(default: fn(u32) -> T) -> Result<Self, BankError> {
        Ok(Self {
            dense: array::from_fn(|_| BankCell::level_one(default(0))),
            overflow: PagedDenseBank::new(u16::MAX as u32 + 1, default, LEVEL_ONE)?,
        })
    }

    #[inline(always)]
    pub(crate) fn get(&self, index: u16) -> Result<BankCell<T>, BankError> {
        if usize::from(index) < DENSE_REGISTER_COUNT {
            Ok(self.dense[index as usize].clone())
        } else {
            self.overflow.get(u32::from(index))
        }
    }

    pub(crate) fn write(&mut self, index: u16, cell: BankCell<T>) -> Result<(), BankError> {
        if usize::from(index) < DENSE_REGISTER_COUNT {
            self.dense[index as usize] = cell;
            Ok(())
        } else {
            self.overflow.write(u32::from(index), cell)
        }
    }

    #[inline(always)]
    pub(crate) fn get_mut(&mut self, index: u16) -> Result<&mut BankCell<T>, BankError> {
        if usize::from(index) < DENSE_REGISTER_COUNT {
            Ok(&mut self.dense[index as usize])
        } else {
            self.overflow.get_mut(u32::from(index))
        }
    }

    #[must_use]
    pub(crate) fn allocated_overflow_pages(&self) -> usize {
        self.overflow.allocated_pages()
    }
}
