//! Private compact resident node and typed word-annex substrate.

use core::marker::PhantomData;
use core::num::NonZeroU32;

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
mod semantic;
mod whatsit_codec;

pub(crate) use annex::{AnnexKey, NodeAnnexView, NodeAnnexWriter};
pub(crate) use layout::NodeRecord;

pub(crate) trait NodeRecordEncoder {
    fn encode_node(&mut self, node: Node) -> NodeRecord;

    fn encode_char(&mut self, font: FontId, ch: char, origin: OriginId) -> NodeRecord;

    #[allow(clippy::too_many_arguments)] // Direct encoding keeps all fixed payload fields at the destination boundary.
    fn encode_ligature(
        &mut self,
        font: FontId,
        ch: char,
        source_len: usize,
        origins_empty: bool,
        left_hit: bool,
        right_hit: bool,
        source: &mut dyn ExactSizeIterator<Item = (char, OriginId)>,
    ) -> NodeRecord;

    fn encode_kern(&mut self, amount: Scaled, kind: KernKind) -> NodeRecord;
}

impl NodeRecordEncoder for NodeAnnexWriter<'_> {
    fn encode_node(&mut self, node: Node) -> NodeRecord {
        NodeRecord::encode_owned(node, self)
    }

    fn encode_char(&mut self, font: FontId, ch: char, origin: OriginId) -> NodeRecord {
        NodeRecord::encode_char(font, ch, origin)
    }

    #[allow(clippy::too_many_arguments)] // Direct encoding keeps all fixed payload fields at the destination boundary.
    fn encode_ligature(
        &mut self,
        font: FontId,
        ch: char,
        source_len: usize,
        origins_empty: bool,
        left_hit: bool,
        right_hit: bool,
        source: &mut dyn ExactSizeIterator<Item = (char, OriginId)>,
    ) -> NodeRecord {
        NodeRecord::encode_ligature(
            font,
            ch,
            source_len,
            origins_empty,
            left_hit,
            right_hit,
            source,
            self,
        )
    }

    fn encode_kern(&mut self, amount: Scaled, kind: KernKind) -> NodeRecord {
        NodeRecord::encode_kern(amount, kind)
    }
}

use annex::*;
use layout::*;
use whatsit_codec::*;

#[cfg(test)]
mod tests;
