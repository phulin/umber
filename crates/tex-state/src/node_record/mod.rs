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
pub(crate) use node_codec::RelocatedFixedCopy;

/// Rewrites only a durable root box's scalar annex word. The caller owns the
/// exclusive region and journals the returned old value before any rollback.
pub(crate) fn set_root_box_dimension(
    record: NodeRecord<crate::fork_arena::PageMaterialLane>,
    pool: &mut crate::fork_arena::ChunkPool<u32>,
    arena: &mut crate::fork_arena::ForkArena<u32, crate::node_region::NodeAnnexLane>,
    dimension: crate::command_context::BoxDimension,
    value: Scaled,
) -> Result<Scaled, crate::fork_arena::ForkArenaError> {
    if !matches!(record.kind(), Some(NodeKind::HList | NodeKind::VList))
        || record.subtype() != 0
        || record.flags() != 0
    {
        return Err(crate::fork_arena::ForkArenaError::InvalidRange);
    }
    let offset = match dimension {
        crate::command_context::BoxDimension::Width => 1,
        crate::command_context::BoxDimension::Height => 2,
        crate::command_context::BoxDimension::Depth => 3,
    };
    annex::set_fixed_box_word(pool, arena, annex::key_from_record(record), offset, value)
}

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
