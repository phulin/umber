//! Dense fixed-size environment banks.

use crate::cell::{BankTag, CellId};
use crate::env::barrier;
use crate::epoch::Epoch;
use crate::ids::{GlueId, NodeListId, TokenListId};
use crate::journal::Journal;
use crate::scaled::Scaled;
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

pub(crate) trait BankCodec {
    type Value: Copy;

    fn encode(value: Self::Value) -> u64;
    fn decode(word: u64) -> Self::Value;
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

    pub(crate) fn set(
        &mut self,
        index: u16,
        value: C::Value,
        journal: &mut Journal,
        #[cfg(feature = "shadow")] shadow: &mut std::collections::HashMap<CellId, u64>,
        epoch: Epoch,
        bank: BankTag,
        global: bool,
    ) {
        let offset = checked_index::<N>(index);
        let cell_id = cell_id(bank, index, global);
        barrier(
            &mut self.values[offset],
            &mut self.stamps[offset],
            journal,
            #[cfg(feature = "shadow")]
            shadow,
            epoch,
            cell_id,
            C::encode(value),
        );
    }

    #[allow(dead_code)]
    pub(crate) fn restore_word(&mut self, index: u16, word: u64) {
        self.values[checked_index::<N>(index)] = word;
    }

    #[cfg(any(test, feature = "testing", feature = "shadow"))]
    pub(crate) fn non_default_words(&self, bank: BankTag, out: &mut Vec<(CellId, u64)>) {
        for (index, &word) in self.values.iter().enumerate() {
            if word != 0 {
                out.push((CellId::new(bank, index as u32), word));
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
    type Value = NodeListId;

    fn encode(value: Self::Value) -> u64 {
        u64::from(value.raw())
    }

    fn decode(word: u64) -> Self::Value {
        NodeListId::new(decode_u32(word))
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
