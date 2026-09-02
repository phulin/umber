//! Variant-directed construction into one final resident node slot.

use crate::glue::GlueSpec;
use crate::ids::FontId;
use crate::math::{MathChoice, MathFraction, MathListNode, MathNoad, MathStyle};
use crate::node::{
    AdjustNode, BoxNode, Direction, DiscKind, GlueKind, KernKind, LeaderPayload, MarginKernSide,
    Node, NodeTokenKey, PdfAccessibilityControl, PdfDestinationNode, PdfLiteralMode, PdfThreadNode,
    UnsetNode, Whatsit,
};
use crate::node_arena::PageListId;
use crate::scaled::Scaled;
use crate::token::OriginId;
use crate::world::{PrintSink, StreamSlot};

/// A one-use capability for initializing a reserved resident node slot.
///
/// This is not a draft or a stored representation. Each method writes one
/// variant directly into the arena-owned vacancy and consumes the capability.
pub struct NodeDestination<'a> {
    slot: NodeDestinationSlot<'a>,
}

enum NodeDestinationSlot<'a> {
    #[cfg(test)]
    Owned(&'a mut Option<Node>),
    Record {
        slot: &'a mut Option<crate::node_record::NodeRecord>,
        encoder: &'a mut dyn crate::node_record::NodeRecordEncoder,
    },
}

impl<'a> NodeDestination<'a> {
    #[cfg(test)]
    pub(crate) fn new(slot: &'a mut Option<Node>) -> Self {
        assert!(slot.is_none(), "node destination is vacant");
        Self {
            slot: NodeDestinationSlot::Owned(slot),
        }
    }

    pub(crate) fn new_record(
        slot: &'a mut Option<crate::node_record::NodeRecord>,
        encoder: &'a mut dyn crate::node_record::NodeRecordEncoder,
    ) -> Self {
        assert!(slot.is_none(), "node-record destination is vacant");
        Self {
            slot: NodeDestinationSlot::Record { slot, encoder },
        }
    }

    fn store(self, node: Node) {
        match self.slot {
            #[cfg(test)]
            NodeDestinationSlot::Owned(slot) => *slot = Some(node),
            NodeDestinationSlot::Record { slot, encoder } => {
                *slot = Some(encoder.encode_node(node));
            }
        }
    }

    pub fn char(self, font: FontId, ch: char, origin: OriginId) {
        self.store(Node::Char { font, ch, origin });
    }

    pub fn ligature(
        self,
        font: FontId,
        ch: char,
        orig: Vec<char>,
        origins: Vec<OriginId>,
        left_hit: bool,
        right_hit: bool,
    ) {
        self.store(Node::Lig {
            font,
            ch,
            orig,
            left_hit,
            right_hit,
            origins,
        });
    }

    pub fn kern(self, amount: Scaled, kind: KernKind) {
        self.store(Node::Kern { amount, kind });
    }

    pub fn margin_kern(self, amount: Scaled, side: MarginKernSide, font: FontId, ch: u8) {
        self.store(Node::MarginKern {
            amount,
            side,
            font,
            ch,
        });
    }

    pub fn glue(self, spec: GlueSpec, kind: GlueKind, leader: Option<LeaderPayload<PageListId>>) {
        self.store(Node::Glue { spec, kind, leader });
    }

    pub fn penalty(self, value: i32) {
        self.store(Node::Penalty(value));
    }

    pub fn rule(self, width: Option<Scaled>, height: Option<Scaled>, depth: Option<Scaled>) {
        self.store(Node::Rule {
            width,
            height,
            depth,
        });
    }

    pub fn hlist(self, value: BoxNode<PageListId>) {
        self.store(Node::HList(value));
    }

    pub fn vlist(self, value: BoxNode<PageListId>) {
        self.store(Node::VList(value));
    }

    pub fn unset(self, value: UnsetNode<PageListId>) {
        self.store(Node::Unset(value));
    }

    pub fn discretionary(
        self,
        kind: DiscKind,
        pre: PageListId,
        post: PageListId,
        replace: PageListId,
        physical_replace_count: u8,
    ) {
        self.store(Node::Disc {
            kind,
            pre,
            post,
            replace,
            physical_replace_count,
        });
    }

    pub fn mark(self, class: u16, tokens: NodeTokenKey) {
        self.store(Node::Mark { class, tokens });
    }

    pub fn insertion(
        self,
        class: u16,
        size: Scaled,
        split_top_skip: GlueSpec,
        split_max_depth: Scaled,
        floating_penalty: i32,
        content: PageListId,
    ) {
        self.store(Node::Ins {
            class,
            size,
            split_top_skip,
            split_max_depth,
            floating_penalty,
            content,
        });
    }

    pub fn math_on(self, surround: Scaled) {
        self.store(Node::MathOn(surround));
    }

    pub fn math_off(self, surround: Scaled) {
        self.store(Node::MathOff(surround));
    }

    pub fn direction(self, direction: Direction) {
        self.store(Node::Direction(direction));
    }

    pub fn math_noad(self, noad: MathNoad<PageListId>) {
        self.store(Node::MathNoad(noad));
    }

    pub fn fraction_noad(self, fraction: MathFraction<PageListId>) {
        self.store(Node::FractionNoad(fraction));
    }

    pub fn math_style(self, style: MathStyle) {
        self.store(Node::MathStyle(style));
    }

    pub fn math_choice(self, choice: MathChoice<PageListId>) {
        self.store(Node::MathChoice(choice));
    }

    pub fn math_list(self, list: MathListNode<PageListId>) {
        self.store(Node::MathList(list));
    }

    pub fn nonscript(self) {
        self.store(Node::Nonscript);
    }

    pub fn adjustment(self, adjustment: AdjustNode<PageListId>) {
        self.store(Node::Adjust(adjustment));
    }

    fn whatsit(self, whatsit: Whatsit) {
        self.store(Node::Whatsit(whatsit));
    }

    pub fn open_out(self, slot: StreamSlot, path: String) {
        self.whatsit(Whatsit::OpenOut { slot, path });
    }

    pub fn close_out(self, slot: Option<StreamSlot>) {
        self.whatsit(Whatsit::CloseOut { slot });
    }

    pub fn deferred_write(self, sink: PrintSink, tokens: NodeTokenKey) {
        self.whatsit(Whatsit::DeferredWrite { sink, tokens });
    }

    pub fn special(self, class: String, payload: Vec<u8>) {
        self.whatsit(Whatsit::Special { class, payload });
    }

    pub fn deferred_special(self, class: String, tokens: NodeTokenKey) {
        self.whatsit(Whatsit::DeferredSpecial { class, tokens });
    }

    pub fn pdf_reference_object(self, object: u32) {
        self.whatsit(Whatsit::PdfReferenceObject { object });
    }

    pub fn pdf_accessibility(self, value: PdfAccessibilityControl) {
        self.whatsit(Whatsit::PdfAccessibility(value));
    }

    pub fn pdf_annotation(self, object: u32) {
        self.whatsit(Whatsit::PdfAnnotation { object });
    }

    pub fn pdf_link_start(self, object: u32) {
        self.whatsit(Whatsit::PdfLinkStart { object });
    }

    pub fn pdf_link_end(self, object: u32) {
        self.whatsit(Whatsit::PdfLinkEnd { object });
    }

    pub fn pdf_running_link(self, running: bool) {
        self.whatsit(Whatsit::PdfRunningLink(running));
    }

    pub fn pdf_literal(self, mode: PdfLiteralMode, payload: Vec<u8>) {
        self.whatsit(Whatsit::PdfLiteral { mode, payload });
    }

    pub fn deferred_pdf_literal(self, mode: PdfLiteralMode, tokens: NodeTokenKey) {
        self.whatsit(Whatsit::DeferredPdfLiteral { mode, tokens });
    }

    pub fn pdf_set_matrix(self, payload: Vec<u8>) {
        self.whatsit(Whatsit::PdfSetMatrix { payload });
    }

    pub fn pdf_save(self) {
        self.whatsit(Whatsit::PdfSave);
    }

    pub fn pdf_restore(self) {
        self.whatsit(Whatsit::PdfRestore);
    }

    pub fn pdf_color_stack(self, id: u32, action: crate::PdfColorStackAction) {
        self.whatsit(Whatsit::PdfColorStack { id, action });
    }

    pub fn pdf_save_pos(self) {
        self.whatsit(Whatsit::PdfSavePos);
    }

    pub fn pdf_snap_ref_point(self) {
        self.whatsit(Whatsit::PdfSnapRefPoint);
    }

    pub fn pdf_snap_y(self, glue: GlueSpec) {
        self.whatsit(Whatsit::PdfSnapY { glue });
    }

    pub fn pdf_snap_y_comp(self, ratio: u16) {
        self.whatsit(Whatsit::PdfSnapYComp { ratio });
    }

    pub fn pdf_ref_xform(self, object: u32, width: Scaled, height: Scaled, depth: Scaled) {
        self.whatsit(Whatsit::PdfRefXForm {
            object,
            width,
            height,
            depth,
        });
    }

    pub fn pdf_ref_ximage(self, object: u32, width: Scaled, height: Scaled, depth: Scaled) {
        self.whatsit(Whatsit::PdfRefXImage {
            object,
            width,
            height,
            depth,
        });
    }

    pub fn pdf_destination(self, destination: PdfDestinationNode) {
        self.whatsit(Whatsit::PdfDestination(Box::new(destination)));
    }

    pub fn pdf_thread(self, thread: PdfThreadNode) {
        self.whatsit(Whatsit::PdfThread(Box::new(thread)));
    }

    pub fn pdf_end_thread(self) {
        self.whatsit(Whatsit::PdfEndThread);
    }

    pub fn language(self, language: u8, left_hyphen_min: u8, right_hyphen_min: u8) {
        self.whatsit(Whatsit::Language {
            language,
            left_hyphen_min,
            right_hyphen_min,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node_arena::NodeView;

    #[test]
    fn variant_builder_initializes_the_final_vacancy() {
        let mut slot = None;
        NodeDestination::new(&mut slot).kern(Scaled::from_raw(23), KernKind::Explicit);
        assert!(matches!(
            slot.as_ref().map(NodeView::from),
            Some(NodeView::Kern { amount, kind: KernKind::Explicit })
                if amount == Scaled::from_raw(23)
        ));
    }

    #[test]
    fn ligature_view_borrows_builder_owned_annex_storage() {
        let mut slot = None;
        NodeDestination::new(&mut slot).ligature(
            crate::font::NULL_FONT,
            'f',
            vec!['f', 'i'],
            vec![OriginId::UNKNOWN; 2],
            false,
            true,
        );
        let Some(NodeView::Lig {
            orig,
            origins,
            right_hit,
            ..
        }) = slot.as_ref().map(NodeView::from)
        else {
            panic!("ligature destination wrote its requested variant");
        };
        assert_eq!(orig.as_ref(), ['f', 'i']);
        assert_eq!(orig.len(), origins.len());
        assert!(right_hit);
    }
}
