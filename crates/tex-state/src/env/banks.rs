//! Dense fixed-size environment banks.

use crate::cell::{BankTag, CellId};
use crate::env::barrier;
use crate::epoch::Epoch;
use crate::ids::{GlueId, NodeListId, TokenListId};
use crate::journal::{Journal, UndoRec};
use crate::scaled::Scaled;
use core::marker::PhantomData;
#[cfg(feature = "shadow")]
use std::collections::HashMap;

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
    /// TeX's `\mag` integer parameter.
    pub const MAG: Self = Self::new(17);

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

    /// Plain TeX's `\escapechar` integer parameter.
    pub const ESCAPE_CHAR: Self = Self::new(40);

    /// Initial `\hyphenchar` value assigned to newly loaded fonts.
    pub const DEFAULT_HYPHEN_CHAR: Self = Self::new(41);

    /// Initial `\skewchar` value assigned to newly loaded fonts.
    pub const DEFAULT_SKEW_CHAR: Self = Self::new(42);

    /// Plain TeX's `\endlinechar` integer parameter.
    pub const END_LINE_CHAR: Self = Self::new(48);

    /// TeX's `\showboxbreadth` integer parameter.
    pub const SHOW_BOX_BREADTH: Self = Self::new(24);

    /// TeX's `\showboxdepth` integer parameter.
    pub const SHOW_BOX_DEPTH: Self = Self::new(25);

    /// TeX's `\hbadness` integer parameter.
    pub const HBADNESS: Self = Self::new(26);

    /// TeX's `\vbadness` integer parameter.
    pub const VBADNESS: Self = Self::new(27);
}

impl DimenParam {
    /// TeX's `\lineskiplimit` dimension parameter.
    pub const LINE_SKIP_LIMIT: Self = Self::new(2);

    /// TeX's `\boxmaxdepth` dimension parameter.
    pub const BOX_MAX_DEPTH: Self = Self::new(7);

    /// TeX's `\hfuzz` dimension parameter.
    pub const HFUZZ: Self = Self::new(8);

    /// TeX's `\vfuzz` dimension parameter.
    pub const VFUZZ: Self = Self::new(9);

    /// TeX's `\overfullrule` dimension parameter.
    pub const OVERFULL_RULE: Self = Self::new(16);
}

impl GlueParam {
    /// TeX's `\lineskip` glue parameter.
    pub const LINE_SKIP: Self = Self::new(0);

    /// TeX's `\baselineskip` glue parameter.
    pub const BASELINE_SKIP: Self = Self::new(1);
}

pub(crate) trait BankCodec {
    type Value: Copy;

    fn encode(value: Self::Value) -> u64;
    fn decode(word: u64) -> Self::Value;
}

pub(crate) struct BankSetContext<'a> {
    pub(crate) journal: &'a mut Journal,
    #[cfg(feature = "shadow")]
    pub(crate) shadow: &'a mut HashMap<CellId, u64>,
    pub(crate) epoch: Epoch,
    pub(crate) bank: BankTag,
    pub(crate) global: bool,
}

impl BankSetContext<'_> {
    fn cell_id(&self, index: u16) -> CellId {
        cell_id(self.bank, index, self.global)
    }
}

pub(crate) struct BankJournalContext<'a> {
    pub(crate) journal: &'a mut Journal,
    #[cfg(feature = "shadow")]
    pub(crate) shadow: &'a mut HashMap<CellId, u64>,
    pub(crate) bank: BankTag,
    pub(crate) global: bool,
}

impl BankJournalContext<'_> {
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

impl<C, const N: usize> FixedBank<C, N>
where
    C: BankCodec,
{
    pub(crate) const fn new() -> Self {
        Self {
            values: [0; N],
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

    pub(crate) fn set_always_journal(
        &mut self,
        index: u16,
        value: C::Value,
        ctx: BankJournalContext<'_>,
    ) -> Option<UndoRec> {
        let offset = checked_index::<N>(index);
        let cell_id = ctx.cell_id(index);
        let old = self.values[offset];
        let new = C::encode(value);
        if old == new && !ctx.global {
            return None;
        }
        let rec = UndoRec::new(cell_id, old, new);
        ctx.journal.push_undo(rec);
        self.values[offset] = new;
        #[cfg(feature = "shadow")]
        crate::env::shadow_set(ctx.shadow, CellId::new(ctx.bank, u32::from(index)), new);
        Some(rec)
    }

    #[allow(dead_code)]
    pub(crate) fn restore_word(&mut self, index: u16, word: u64) {
        self.values[checked_index::<N>(index)] = word;
    }

    #[cfg(any(test, feature = "testing", feature = "shadow"))]
    pub(crate) fn for_each_non_default_word(&self, bank: BankTag, mut f: impl FnMut(CellId, u64)) {
        for (index, &word) in self.values.iter().enumerate() {
            if word != 0 {
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

#[derive(Clone, Copy, Debug)]
pub(crate) struct NodeListIdCodec;

impl BankCodec for NodeListIdCodec {
    type Value = Option<NodeListId>;

    fn encode(value: Self::Value) -> u64 {
        NodeListId::encode_box_word(value)
    }

    fn decode(word: u64) -> Self::Value {
        NodeListId::decode_box_word(word)
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
