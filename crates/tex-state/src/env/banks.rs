//! Dense fixed-size environment banks.

use crate::cell::{BankTag, CellId};
use crate::env::barrier;
use crate::epoch::Epoch;
use crate::ids::{FontId, GlueId, TokenListId};
use crate::journal::{Journal, JournalPos};
use crate::scaled::Scaled;
#[cfg(feature = "shadow")]
use ahash::AHashMap;
use core::marker::PhantomData;

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

    /// TeX's `\globaldefs` integer parameter.
    pub const GLOBAL_DEFS: Self = Self::new(32);

    /// TeX's `\uchyph` uppercase-start hyphenation gate.
    pub const UC_HYPH: Self = Self::new(39);

    /// Plain TeX's `\escapechar` integer parameter.
    pub const ESCAPE_CHAR: Self = Self::new(40);

    /// Initial `\hyphenchar` value assigned to newly loaded fonts.
    pub const DEFAULT_HYPHEN_CHAR: Self = Self::new(41);

    /// Initial `\skewchar` value assigned to newly loaded fonts.
    pub const DEFAULT_SKEW_CHAR: Self = Self::new(42);

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

    /// TeX's `\everycr` token-list parameter.
    pub const EVERY_CR: Self = Self::new(7);

    /// e-TeX's token list inserted at natural real or virtual EOF.
    pub const EVERY_EOF: Self = Self::new(8);
}

pub(crate) trait BankCodec {
    type Value: Copy;

    const DEFAULT_WORD: u64 = 0;

    fn encode(value: Self::Value) -> u64;
    fn decode(word: u64) -> Self::Value;
}

pub(crate) struct BankSetContext<'a> {
    pub(crate) journal: &'a mut Journal,
    #[cfg(feature = "shadow")]
    pub(crate) shadow: &'a mut AHashMap<CellId, u64>,
    pub(crate) epoch: Epoch,
    pub(crate) bank: BankTag,
    pub(crate) global: bool,
}

impl BankSetContext<'_> {
    fn cell_id(&self, index: u16) -> CellId {
        cell_id(self.bank, index, self.global)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct FixedBank<C, const N: usize> {
    values: [u64; N],
    stamps: [Epoch; N],
    _codec: PhantomData<C>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BoxWriteOutcome {
    Unchanged,
    Journaled {
        rec: crate::journal::BoxUndoRec,
        pos: JournalPos,
    },
    Coalesced {
        displaced: u64,
    },
}

impl<C, const N: usize> FixedBank<C, N>
where
    C: BankCodec,
{
    pub(crate) const fn new() -> Self {
        Self {
            values: [C::DEFAULT_WORD; N],
            stamps: [Epoch::ZERO; N],
            _codec: PhantomData,
        }
    }

    pub(crate) fn get(&self, index: u16) -> C::Value {
        C::decode(self.values[checked_index::<N>(index)])
    }

    pub(crate) fn set(&mut self, index: u16, value: C::Value, ctx: BankSetContext<'_>) {
        let offset = checked_index::<N>(index);
        let cell_id = ctx.cell_id(index);
        barrier(
            &mut self.values[offset],
            &mut self.stamps[offset],
            ctx.journal,
            #[cfg(feature = "shadow")]
            ctx.shadow,
            ctx.epoch,
            cell_id,
            C::encode(value),
        );
    }

    #[allow(dead_code)]
    pub(crate) fn restore_word(&mut self, index: u16, word: u64) {
        self.values[checked_index::<N>(index)] = word;
    }

    pub(crate) fn for_each_non_default_word(&self, bank: BankTag, mut f: impl FnMut(CellId, u64)) {
        for (index, &word) in self.values.iter().enumerate() {
            if word != C::DEFAULT_WORD {
                f(CellId::new(bank, index as u32), word);
            }
        }
    }
}

impl<C, const N: usize> Default for FixedBank<C, N>
where
    C: BankCodec,
{
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct I32Codec;

impl BankCodec for I32Codec {
    type Value = i32;

    fn encode(value: Self::Value) -> u64 {
        value as u32 as u64
    }

    fn decode(word: u64) -> Self::Value {
        word as u32 as i32
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ScaledCodec;

impl BankCodec for ScaledCodec {
    type Value = Scaled;

    fn encode(value: Self::Value) -> u64 {
        I32Codec::encode(value.raw())
    }

    fn decode(word: u64) -> Self::Value {
        Scaled::from_raw(I32Codec::decode(word))
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct GlueIdCodec;

impl BankCodec for GlueIdCodec {
    type Value = GlueId;

    fn encode(value: Self::Value) -> u64 {
        u64::from(value.raw())
    }

    fn decode(word: u64) -> Self::Value {
        GlueId::new(decode_u32(word))
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct FontIdCodec;

impl BankCodec for FontIdCodec {
    type Value = FontId;

    fn encode(value: Self::Value) -> u64 {
        u64::from(value.raw())
    }

    fn decode(word: u64) -> Self::Value {
        FontId::new(decode_u32(word))
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct TokenListIdCodec;

impl BankCodec for TokenListIdCodec {
    type Value = TokenListId;

    fn encode(value: Self::Value) -> u64 {
        u64::from(value.raw())
    }

    fn decode(word: u64) -> Self::Value {
        TokenListId::new(decode_u32(word))
    }
}

fn checked_index<const N: usize>(index: u16) -> usize {
    let index = usize::from(index);
    assert!(index < N, "index out of dense bank range");
    index
}

fn cell_id(bank: BankTag, index: u16, global: bool) -> CellId {
    if global {
        CellId::new_global(bank, u32::from(index))
    } else {
        CellId::new(bank, u32::from(index))
    }
}

fn decode_u32(word: u64) -> u32 {
    match u32::try_from(word) {
        Ok(value) => value,
        Err(_) => panic!("opaque id word exceeds u32"),
    }
}
