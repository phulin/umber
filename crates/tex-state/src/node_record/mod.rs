//! Private compact resident node and typed word-annex substrate.
#![allow(dead_code)] // Isolated codec proof remains nonresident until the atomic cutover.

use core::marker::PhantomData;
use core::num::NonZeroU32;
use core::sync::atomic::{AtomicU32, Ordering};

use crate::fork_arena::PageMaterialLane;
use crate::glue::{GlueSpec, Order};
use crate::ids::FontId;
use crate::math::{
    FractionThickness, LimitType, MathChar, MathChoice, MathField, MathFraction, MathListNode,
    MathNoad, MathStyle, NoadClass, NoadKind,
};
use crate::node::{
    AdjustNode, BoxLr, BoxNode, BoxNodeFields, DiscKind, GlueKind, KernKind, LeaderPayload,
    MarginKernSide, Node, NodeKind, NodePdfActionIdentifier, NodeTokenKey, PdfAccessibilityControl,
    PdfDestinationKind, PdfDestinationNode, PdfLiteralMode, PdfThreadNode, Sign, UnsetKind,
    UnsetNode, UnsetNodeFields, Whatsit,
};
use crate::page_node_arena::PageListId;
use crate::scaled::{GlueSetRatio, Scaled};
use crate::token::OriginId;
use crate::world::{PrintSink, StreamSlot};

mod annex;
mod layout;
mod node_codec;
mod whatsit_codec;

pub(crate) use annex::{AnnexKey, NodeAnnexArena};
#[allow(unused_imports)]
pub(crate) use annex::{NodeAnnexMark, NodeAnnexMetrics};
pub(crate) use layout::NodeRecord;

use annex::*;
use layout::*;
use whatsit_codec::*;

#[cfg(test)]
mod tests;
