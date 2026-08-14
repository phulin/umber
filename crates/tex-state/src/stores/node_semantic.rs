//! Canonical semantic identities for immutable node-list aggregates.

use super::Stores;
use super::state_hash::{
    hash_fraction_thickness, hash_glue_kind, hash_kern_kind, hash_math_char, hash_noad_kind,
    hash_optional_delimiter, hash_optional_scaled, hash_print_sink, hash_sign,
};
use crate::ids::{FontId, NodeListId};
use crate::math::MathField;
use crate::node::{BoxNode, LeaderPayload, Node, Whatsit};
use crate::node_arena::{NodeRef, NodeSemanticId, NodeSemanticIdBuilder, SidecarNeeds};
use crate::state_hash::StateHasher;

impl Stores {
    pub(super) fn compute_node_semantic_id(&self, nodes: &[Node]) -> NodeSemanticId {
        let mut identity = NodeSemanticIdBuilder::new();
        let mut index = 0;
        while index < nodes.len() {
            if let Node::Char { font, .. } = nodes[index] {
                let end = same_font_char_run_end(nodes, index, font);
                self.push_char_run_identity(&mut identity, font, &nodes[index..end]);
                index = end;
            } else {
                identity.push(|hasher| {
                    self.hash_node_semantic_identity(NodeRef::from(&nodes[index]), hasher);
                });
                index += 1;
            }
        }
        identity.finish()
    }

    pub(super) fn validate_and_plan_node_list(
        &mut self,
        nodes: &[Node],
    ) -> (NodeSemanticId, SidecarNeeds) {
        let mut identity = NodeSemanticIdBuilder::new();
        let mut needs = SidecarNeeds::default();
        let mut index = 0;
        while index < nodes.len() {
            if let Node::Char { font, .. } = nodes[index] {
                let end = same_font_char_run_end(nodes, index, font);
                self.assert_live_font(font);
                self.push_char_run_identity(&mut identity, font, &nodes[index..end]);
                index = end;
            } else {
                let node = &nodes[index];
                needs.preflight_and_count(node);
                self.assert_live_handles_in_node(node);
                identity.push(|hasher| {
                    self.hash_node_semantic_identity(NodeRef::from(node), hasher);
                });
                index += 1;
            }
        }
        (identity.finish(), needs)
    }

    pub(super) fn validate_and_plan_direct_node_list(
        &mut self,
        builder: &crate::node_arena::NodeListBuilder,
    ) -> (NodeSemanticId, SidecarNeeds) {
        let nodes = builder.as_slice();
        let mut identity = NodeSemanticIdBuilder::new();
        let mut needs = SidecarNeeds::default();
        let mut index = 0;
        while index < nodes.len() {
            if let Node::Char { font, .. } = nodes[index] {
                let end = same_font_char_run_end(nodes, index, font);
                self.assert_live_font(font);
                self.push_char_run_identity(&mut identity, font, &nodes[index..end]);
                index = end;
            } else {
                let node = &nodes[index];
                needs.preflight_and_count(node);
                self.assert_live_handles_in_direct_node(node, |id| builder.owns_direct_child(id));
                identity.push(|hasher| {
                    self.hash_node_semantic_identity_with(NodeRef::from(node), hasher, &|child| {
                        builder
                            .direct_child_semantic_id(child)
                            .expect("direct node-list child coordinate is stale or unowned")
                    });
                });
                index += 1;
            }
        }
        (identity.finish(), needs)
    }

    fn push_char_run_identity(
        &self,
        identity: &mut NodeSemanticIdBuilder,
        font: FontId,
        nodes: &[Node],
    ) {
        identity.push_run(nodes.len(), |hasher| {
            // Tag 24 is reserved for the v3 node-list stream's maximal
            // same-font character-run encoding. Origins are non-semantic.
            hasher.tag(24);
            self.hash_font_semantic(font, hasher);
            hasher.usize(nodes.len());
            for node in nodes {
                let Node::Char { ch, .. } = node else {
                    unreachable!("character run contains a non-character node")
                };
                hasher.u32(*ch as u32);
            }
        });
    }

    pub(crate) fn node_semantic_id(&self, id: NodeListId) -> NodeSemanticId {
        self.assert_live_node_list(id);
        self.nodes.semantic_id(id, &self.survivors)
    }

    pub(super) fn hash_node_semantic_identity(&self, node: NodeRef<'_>, hasher: &mut StateHasher) {
        self.hash_node_semantic_identity_with(node, hasher, &|child| self.node_semantic_id(child));
    }

    fn hash_node_semantic_identity_with(
        &self,
        node: NodeRef<'_>,
        hasher: &mut StateHasher,
        child_semantic_id: &impl Fn(NodeListId) -> NodeSemanticId,
    ) {
        match node {
            NodeRef::Char { font, ch, .. } => {
                hasher.tag(0);
                self.hash_font_semantic(font, hasher);
                hasher.u32(ch as u32);
            }
            NodeRef::Lig {
                font,
                ch,
                orig,
                left_hit,
                right_hit,
                ..
            } => {
                hasher.tag(1);
                self.hash_font_semantic(font, hasher);
                hasher.u32(ch as u32);
                hasher.usize(orig.len());
                for source in orig {
                    hasher.u32(*source as u32);
                }
                hasher.bool(left_hit);
                hasher.bool(right_hit);
            }
            NodeRef::Kern { amount, kind } => {
                hasher.tag(2);
                hasher.i32(amount.raw());
                hash_kern_kind(kind, hasher);
            }
            NodeRef::MarginKern {
                amount,
                side,
                font,
                ch,
            } => {
                hasher.tag(22);
                hasher.i32(amount.raw());
                hasher.u8(side as u8);
                self.hash_font_semantic(font, hasher);
                hasher.u8(ch);
            }
            NodeRef::Glue { spec, kind, leader } => {
                hasher.tag(3);
                self.hash_glue_semantic(spec.id(), hasher);
                hash_glue_kind(kind, hasher);
                self.hash_leader_identity(leader, hasher, child_semantic_id);
            }
            NodeRef::Penalty(value) => {
                hasher.tag(4);
                hasher.i32(value);
            }
            NodeRef::Rule {
                width,
                height,
                depth,
            } => {
                hasher.tag(5);
                hash_optional_scaled(width, hasher);
                hash_optional_scaled(height, hasher);
                hash_optional_scaled(depth, hasher);
            }
            NodeRef::HList(box_node) => {
                self.hash_box_identity(6, &box_node, hasher, child_semantic_id)
            }
            NodeRef::VList(box_node) => {
                self.hash_box_identity(7, &box_node, hasher, child_semantic_id)
            }
            NodeRef::Unset(unset) => {
                hasher.tag(8);
                hasher.u8(match unset.kind {
                    crate::node::UnsetKind::HBox => 0,
                    crate::node::UnsetKind::VBox => 1,
                });
                hasher.i32(unset.width.raw());
                hasher.i32(unset.height.raw());
                hasher.i32(unset.depth.raw());
                hasher.u16(unset.span_count);
                hasher.i32(unset.stretch.raw());
                hasher.u8(unset.stretch_order as u8);
                hasher.i32(unset.shrink.raw());
                hasher.u8(unset.shrink_order as u8);
                hash_child_identity(unset.children, hasher, child_semantic_id);
            }
            NodeRef::Disc {
                kind,
                pre,
                post,
                replace,
                ..
            } => {
                hasher.tag(9);
                hasher.u8(match kind {
                    crate::node::DiscKind::Discretionary => 0,
                    crate::node::DiscKind::ExplicitHyphen => 1,
                    crate::node::DiscKind::AutomaticHyphen => 2,
                });
                hash_child_identity(pre, hasher, child_semantic_id);
                hash_child_identity(post, hasher, child_semantic_id);
                hash_child_identity(replace, hasher, child_semantic_id);
            }
            NodeRef::Mark { class, tokens } => {
                hasher.tag(10);
                hasher.u16(class);
                self.hash_token_list_semantic(tokens.id(), hasher);
            }
            NodeRef::Ins {
                class,
                size,
                split_top_skip,
                split_max_depth,
                floating_penalty,
                content,
            } => {
                hasher.tag(11);
                hasher.u16(class);
                hasher.i32(size.raw());
                self.hash_glue_semantic(split_top_skip.id(), hasher);
                hasher.i32(split_max_depth.raw());
                hasher.i32(floating_penalty);
                hash_child_identity(content, hasher, child_semantic_id);
            }
            NodeRef::Whatsit(whatsit) => self.hash_whatsit_identity(whatsit, hasher),
            NodeRef::MathOn(width) => {
                hasher.tag(13);
                hasher.i32(width.raw());
            }
            NodeRef::MathOff(width) => {
                hasher.tag(14);
                hasher.i32(width.raw());
            }
            NodeRef::Adjust(adjust) => {
                hasher.tag(15);
                hasher.bool(adjust.pre);
                hash_child_identity(adjust.content, hasher, child_semantic_id);
            }
            NodeRef::MathNoad(noad) => {
                hasher.tag(16);
                hash_noad_kind(&noad.kind, hasher);
                self.hash_math_field_identity(&noad.nucleus, hasher, child_semantic_id);
                self.hash_math_field_identity(&noad.subscript, hasher, child_semantic_id);
                self.hash_math_field_identity(&noad.superscript, hasher, child_semantic_id);
            }
            NodeRef::FractionNoad(fraction) => {
                hasher.tag(17);
                hash_child_identity(fraction.numerator, hasher, child_semantic_id);
                hash_child_identity(fraction.denominator, hasher, child_semantic_id);
                hash_fraction_thickness(fraction.thickness, hasher);
                hash_optional_delimiter(fraction.left_delimiter, hasher);
                hash_optional_delimiter(fraction.right_delimiter, hasher);
            }
            NodeRef::MathStyle(style) => {
                hasher.tag(18);
                hasher.u8(match style {
                    crate::math::MathStyle::Display => 0,
                    crate::math::MathStyle::Text => 1,
                    crate::math::MathStyle::Script => 2,
                    crate::math::MathStyle::ScriptScript => 3,
                });
            }
            NodeRef::MathChoice(choice) => {
                hasher.tag(19);
                hash_child_identity(choice.display, hasher, child_semantic_id);
                hash_child_identity(choice.text, hasher, child_semantic_id);
                hash_child_identity(choice.script, hasher, child_semantic_id);
                hash_child_identity(choice.script_script, hasher, child_semantic_id);
            }
            NodeRef::MathList(list) => {
                hasher.tag(20);
                hasher.bool(list.display);
                hash_child_identity(list.content, hasher, child_semantic_id);
            }
            NodeRef::Nonscript => hasher.tag(21),
            NodeRef::Direction(direction) => {
                hasher.tag(22);
                hasher.u8(direction as u8);
            }
        }
    }

    pub(super) fn hash_node_list_identity(&self, id: NodeListId, hasher: &mut StateHasher) {
        hash_child_identity(id, hasher, &|child| self.node_semantic_id(child));
    }

    fn hash_box_identity(
        &self,
        tag: u8,
        box_node: &BoxNode,
        hasher: &mut StateHasher,
        child_semantic_id: &impl Fn(NodeListId) -> NodeSemanticId,
    ) {
        hasher.tag(tag);
        hasher.i32(box_node.width.raw());
        hasher.i32(box_node.height.raw());
        hasher.i32(box_node.depth.raw());
        hasher.i32(box_node.shift.raw());
        hasher.u8(box_node.box_lr as u8);
        hasher.i32(box_node.glue_set.numerator());
        hasher.i32(box_node.glue_set.denominator());
        hash_sign(box_node.glue_sign, hasher);
        hasher.u8(box_node.glue_order as u8);
        hash_child_identity(box_node.children, hasher, child_semantic_id);
    }

    fn hash_leader_identity(
        &self,
        payload: Option<&LeaderPayload>,
        hasher: &mut StateHasher,
        child_semantic_id: &impl Fn(NodeListId) -> NodeSemanticId,
    ) {
        match payload {
            None => hasher.tag(0),
            Some(LeaderPayload::HList(box_node)) => {
                self.hash_box_identity(1, box_node, hasher, child_semantic_id)
            }
            Some(LeaderPayload::VList(box_node)) => {
                self.hash_box_identity(2, box_node, hasher, child_semantic_id)
            }
            Some(LeaderPayload::Rule {
                width,
                height,
                depth,
            }) => {
                hasher.tag(3);
                hash_optional_scaled(*width, hasher);
                hash_optional_scaled(*height, hasher);
                hash_optional_scaled(*depth, hasher);
            }
        }
    }

    fn hash_math_field_identity(
        &self,
        field: &MathField,
        hasher: &mut StateHasher,
        child_semantic_id: &impl Fn(NodeListId) -> NodeSemanticId,
    ) {
        match field {
            MathField::Empty => hasher.tag(0),
            MathField::MathChar(ch) => {
                hasher.tag(1);
                hash_math_char(*ch, hasher);
            }
            MathField::MathTextChar(ch) => {
                hasher.tag(2);
                hash_math_char(*ch, hasher);
            }
            MathField::SubBox(list) => {
                hasher.tag(3);
                hash_child_identity(*list, hasher, child_semantic_id);
            }
            MathField::SubMlist(list) => {
                hasher.tag(4);
                hash_child_identity(*list, hasher, child_semantic_id);
            }
        }
    }

    fn hash_whatsit_identity(&self, whatsit: &Whatsit, hasher: &mut StateHasher) {
        match whatsit {
            Whatsit::OpenOut { slot, path } => {
                hasher.tag(12);
                hasher.u8(slot.raw());
                hasher.str(path);
            }
            Whatsit::CloseOut { slot } => {
                hasher.tag(13);
                hasher.u8(slot.map_or(16, |slot| slot.raw()));
            }
            Whatsit::DeferredWrite { sink, tokens } => {
                hasher.tag(14);
                hash_print_sink(*sink, hasher);
                self.hash_token_list_semantic(tokens.id(), hasher);
            }
            Whatsit::Special { class, payload } => {
                hasher.tag(15);
                hasher.bytes(class.as_bytes());
                hasher.bytes(payload);
            }
            Whatsit::DeferredSpecial { class, tokens } => {
                hasher.tag(16);
                hasher.bytes(class.as_bytes());
                self.hash_token_list_semantic(tokens.id(), hasher);
            }
            Whatsit::PdfLiteral { mode, payload } => {
                hasher.tag(17);
                hasher.u8(*mode as u8);
                hasher.bytes(payload);
            }
            Whatsit::DeferredPdfLiteral { mode, tokens } => {
                hasher.tag(18);
                hasher.u8(*mode as u8);
                self.hash_token_list_semantic(tokens.id(), hasher);
            }
            Whatsit::PdfSetMatrix { payload } => {
                hasher.tag(19);
                hasher.bytes(payload);
            }
            Whatsit::PdfSave => hasher.tag(20),
            Whatsit::PdfRestore => hasher.tag(21),
            Whatsit::PdfColorStack { id, action } => {
                hasher.tag(22);
                hasher.u32(*id);
                match action {
                    crate::PdfColorStackAction::Set(payload) => {
                        hasher.u8(0);
                        hasher.bytes(payload);
                    }
                    crate::PdfColorStackAction::Push(payload) => {
                        hasher.u8(1);
                        hasher.bytes(payload);
                    }
                    crate::PdfColorStackAction::Pop => hasher.u8(2),
                    crate::PdfColorStackAction::Current => hasher.u8(3),
                }
            }
            Whatsit::PdfSavePos => hasher.tag(23),
            Whatsit::PdfSnapRefPoint => hasher.tag(24),
            Whatsit::PdfSnapY { glue } => {
                hasher.tag(25);
                self.hash_glue_semantic(glue.id(), hasher);
            }
            Whatsit::PdfSnapYComp { ratio } => {
                hasher.tag(26);
                hasher.u16(*ratio);
            }
            Whatsit::Language {
                language,
                left_hyphen_min,
                right_hyphen_min,
            } => {
                hasher.tag(16);
                hasher.u8(*language);
                hasher.u8(*left_hyphen_min);
                hasher.u8(*right_hyphen_min);
            }
            Whatsit::PdfReferenceObject { object } => {
                hasher.tag(17);
                hasher.u32(*object);
            }
            Whatsit::PdfRefXImage {
                object,
                width,
                height,
                depth,
            } => {
                hasher.tag(28);
                hasher.u32(*object);
                hasher.i32(width.raw());
                hasher.i32(height.raw());
                hasher.i32(depth.raw());
            }
            Whatsit::PdfAccessibility(control) => {
                hasher.tag(18);
                hasher.u8(match control {
                    crate::node::PdfAccessibilityControl::InterwordSpaceOn => 0,
                    crate::node::PdfAccessibilityControl::InterwordSpaceOff => 1,
                    crate::node::PdfAccessibilityControl::FakeSpace => 2,
                });
            }
            Whatsit::PdfAnnotation { object } => {
                hasher.tag(19);
                hasher.u32(*object);
            }
            Whatsit::PdfLinkStart { object } => {
                hasher.tag(20);
                hasher.u32(*object);
            }
            Whatsit::PdfLinkEnd { object } => {
                hasher.tag(21);
                hasher.u32(*object);
            }
            Whatsit::PdfRunningLink(enabled) => {
                hasher.tag(22);
                hasher.bool(*enabled);
            }
            Whatsit::PdfRefXForm {
                object,
                width,
                height,
                depth,
            } => {
                hasher.tag(27);
                hasher.u32(*object);
                hasher.i32(width.raw());
                hasher.i32(height.raw());
                hasher.i32(depth.raw());
            }
            Whatsit::PdfDestination(destination) => {
                let crate::node::PdfDestinationNode {
                    identifier,
                    structure,
                    kind,
                } = destination.as_ref();
                hasher.tag(23);
                match identifier {
                    crate::PdfActionIdentifier::Name(tokens) => {
                        hasher.u8(0);
                        hasher.u64(self.token_list_semantic_id_value(tokens.id()));
                    }
                    crate::PdfActionIdentifier::Number(number) => {
                        hasher.u8(1);
                        hasher.u32(*number);
                    }
                    crate::PdfActionIdentifier::Raw(_) => {
                        unreachable!("destinations use typed identifiers")
                    }
                }
                hasher.bool(structure.is_some());
                if let Some(structure) = structure {
                    hasher.u32(*structure);
                }
                hash_pdf_destination_kind(hasher, *kind);
            }
            Whatsit::PdfThread(thread) => {
                let crate::node::PdfThreadNode {
                    identifier,
                    dimensions,
                    attributes,
                    running,
                } = thread.as_ref();
                hasher.tag(24);
                match identifier {
                    crate::PdfActionIdentifier::Name(tokens) => {
                        hasher.u8(0);
                        hasher.u64(self.token_list_semantic_id_value(tokens.id()));
                    }
                    crate::PdfActionIdentifier::Number(number) => {
                        hasher.u8(1);
                        hasher.u32(*number);
                    }
                    crate::PdfActionIdentifier::Raw(_) => {
                        unreachable!("threads use typed identifiers")
                    }
                }
                for value in [dimensions.width, dimensions.height, dimensions.depth] {
                    hasher.bool(value.is_some());
                    if let Some(value) = value {
                        hasher.i32(value.raw());
                    }
                }
                hasher.u64(self.token_list_semantic_id_value(attributes.id()));
                hasher.bool(*running);
            }
            Whatsit::PdfEndThread => hasher.tag(25),
        }
    }
}

fn hash_child_identity(
    child: NodeListId,
    hasher: &mut StateHasher,
    child_semantic_id: &impl Fn(NodeListId) -> NodeSemanticId,
) {
    hasher.tag(0x70);
    child_semantic_id(child).apply(hasher);
}

fn same_font_char_run_end(nodes: &[Node], start: usize, font: FontId) -> usize {
    let mut end = start + 1;
    while matches!(nodes.get(end), Some(Node::Char { font: next, .. }) if *next == font) {
        end += 1;
    }
    end
}

fn hash_pdf_destination_kind(
    hasher: &mut crate::state_hash::StateHasher,
    kind: crate::node::PdfDestinationKind,
) {
    use crate::node::PdfDestinationKind::*;
    match kind {
        Xyz { zoom } => {
            hasher.u8(0);
            hasher.bool(zoom.is_some());
            if let Some(zoom) = zoom {
                hasher.i32(zoom);
            }
        }
        FitBoundingBoxHorizontal => hasher.u8(1),
        FitBoundingBoxVertical => hasher.u8(2),
        FitBoundingBox => hasher.u8(3),
        FitHorizontal => hasher.u8(4),
        FitVertical => hasher.u8(5),
        FitRectangle(dimensions) => {
            hasher.u8(6);
            for value in [dimensions.width, dimensions.height, dimensions.depth] {
                hasher.bool(value.is_some());
                if let Some(value) = value {
                    hasher.i32(value.raw());
                }
            }
        }
        Fit => hasher.u8(7),
    }
}
