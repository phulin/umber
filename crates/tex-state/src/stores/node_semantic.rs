//! Canonical semantic identities for immutable node-list aggregates.

use super::Stores;
use super::state_hash::{
    hash_fraction_thickness, hash_glue_kind, hash_kern_kind, hash_math_char, hash_noad_kind,
    hash_optional_delimiter, hash_optional_scaled, hash_print_sink, hash_sign,
};
use crate::ids::NodeListId;
use crate::math::MathField;
use crate::node::{BoxNode, LeaderPayload, Node, Whatsit};
use crate::node_arena::{NodeSemanticId, NodeSemanticIdBuilder};
use crate::state_hash::StateHasher;

impl Stores {
    pub(super) fn compute_node_semantic_id(&self, nodes: &[Node]) -> NodeSemanticId {
        let mut identity = NodeSemanticIdBuilder::new();
        for node in nodes {
            identity.push(|hasher| self.hash_node_semantic_identity(node, hasher));
        }
        identity.finish()
    }

    pub(crate) fn node_semantic_id(&self, id: NodeListId) -> NodeSemanticId {
        self.assert_live_node_list(id);
        self.nodes.semantic_id(id, &self.survivors)
    }

    pub(super) fn hash_node_semantic_identity(&self, node: &Node, hasher: &mut StateHasher) {
        match node {
            Node::Char { font, ch } => {
                hasher.tag(0);
                self.hash_font_semantic(*font, hasher);
                hasher.u32(*ch as u32);
            }
            Node::Lig { font, ch, orig } => {
                hasher.tag(1);
                self.hash_font_semantic(*font, hasher);
                hasher.u32(*ch as u32);
                hasher.u32(orig.0 as u32);
                hasher.u32(orig.1 as u32);
            }
            Node::Kern { amount, kind } => {
                hasher.tag(2);
                hasher.i32(amount.raw());
                hash_kern_kind(*kind, hasher);
            }
            Node::Glue { spec, kind, leader } => {
                hasher.tag(3);
                self.hash_glue_semantic(*spec, hasher);
                hash_glue_kind(*kind, hasher);
                self.hash_leader_identity(leader.as_ref(), hasher);
            }
            Node::Penalty(value) => {
                hasher.tag(4);
                hasher.i32(*value);
            }
            Node::Rule {
                width,
                height,
                depth,
            } => {
                hasher.tag(5);
                hash_optional_scaled(*width, hasher);
                hash_optional_scaled(*height, hasher);
                hash_optional_scaled(*depth, hasher);
            }
            Node::HList(box_node) => self.hash_box_identity(6, box_node, hasher),
            Node::VList(box_node) => self.hash_box_identity(7, box_node, hasher),
            Node::Unset(unset) => {
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
                self.hash_child_identity(unset.children, hasher);
            }
            Node::Disc {
                kind,
                pre,
                post,
                replace,
            } => {
                hasher.tag(9);
                hasher.u8(match kind {
                    crate::node::DiscKind::Discretionary => 0,
                    crate::node::DiscKind::ExplicitHyphen => 1,
                    crate::node::DiscKind::AutomaticHyphen => 2,
                });
                self.hash_child_identity(*pre, hasher);
                self.hash_child_identity(*post, hasher);
                self.hash_child_identity(*replace, hasher);
            }
            Node::Mark { class, tokens } => {
                hasher.tag(10);
                hasher.u16(*class);
                self.hash_token_list_semantic(*tokens, hasher);
            }
            Node::Ins {
                class,
                size,
                split_top_skip,
                split_max_depth,
                floating_penalty,
                content,
            } => {
                hasher.tag(11);
                hasher.u16(*class);
                hasher.i32(size.raw());
                self.hash_glue_semantic(*split_top_skip, hasher);
                hasher.i32(split_max_depth.raw());
                hasher.i32(*floating_penalty);
                self.hash_child_identity(*content, hasher);
            }
            Node::Whatsit(whatsit) => self.hash_whatsit_identity(whatsit, hasher),
            Node::MathOn(width) => {
                hasher.tag(13);
                hasher.i32(width.raw());
            }
            Node::MathOff(width) => {
                hasher.tag(14);
                hasher.i32(width.raw());
            }
            Node::Adjust(content) => {
                hasher.tag(15);
                self.hash_child_identity(*content, hasher);
            }
            Node::MathNoad(noad) => {
                hasher.tag(16);
                hash_noad_kind(&noad.kind, hasher);
                self.hash_math_field_identity(&noad.nucleus, hasher);
                self.hash_math_field_identity(&noad.subscript, hasher);
                self.hash_math_field_identity(&noad.superscript, hasher);
            }
            Node::FractionNoad(fraction) => {
                hasher.tag(17);
                self.hash_child_identity(fraction.numerator, hasher);
                self.hash_child_identity(fraction.denominator, hasher);
                hash_fraction_thickness(fraction.thickness, hasher);
                hash_optional_delimiter(fraction.left_delimiter, hasher);
                hash_optional_delimiter(fraction.right_delimiter, hasher);
            }
            Node::MathStyle(style) => {
                hasher.tag(18);
                hasher.u8(match style {
                    crate::math::MathStyle::Display => 0,
                    crate::math::MathStyle::Text => 1,
                    crate::math::MathStyle::Script => 2,
                    crate::math::MathStyle::ScriptScript => 3,
                });
            }
            Node::MathChoice(choice) => {
                hasher.tag(19);
                self.hash_child_identity(choice.display, hasher);
                self.hash_child_identity(choice.text, hasher);
                self.hash_child_identity(choice.script, hasher);
                self.hash_child_identity(choice.script_script, hasher);
            }
            Node::MathList(list) => {
                hasher.tag(20);
                hasher.bool(list.display);
                self.hash_child_identity(list.content, hasher);
            }
            Node::Nonscript => hasher.tag(21),
            Node::Direction(direction) => {
                hasher.tag(22);
                hasher.u8(*direction as u8);
            }
        }
    }

    fn hash_child_identity(&self, child: NodeListId, hasher: &mut StateHasher) {
        self.hash_node_list_identity(child, hasher);
    }

    pub(super) fn hash_node_list_identity(&self, id: NodeListId, hasher: &mut StateHasher) {
        let semantic_id = self.node_semantic_id(id);
        hasher.tag(0x70);
        hasher.u64(semantic_id.value());
    }

    fn hash_box_identity(&self, tag: u8, box_node: &BoxNode, hasher: &mut StateHasher) {
        hasher.tag(tag);
        hasher.i32(box_node.width.raw());
        hasher.i32(box_node.height.raw());
        hasher.i32(box_node.depth.raw());
        hasher.i32(box_node.shift.raw());
        hasher.bool(box_node.display);
        hasher.i32(box_node.glue_set.numerator());
        hasher.i32(box_node.glue_set.denominator());
        hash_sign(box_node.glue_sign, hasher);
        hasher.u8(box_node.glue_order as u8);
        self.hash_child_identity(box_node.children, hasher);
    }

    fn hash_leader_identity(&self, payload: Option<&LeaderPayload>, hasher: &mut StateHasher) {
        match payload {
            None => hasher.tag(0),
            Some(LeaderPayload::HList(box_node)) => self.hash_box_identity(1, box_node, hasher),
            Some(LeaderPayload::VList(box_node)) => self.hash_box_identity(2, box_node, hasher),
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

    fn hash_math_field_identity(&self, field: &MathField, hasher: &mut StateHasher) {
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
                self.hash_child_identity(*list, hasher);
            }
            MathField::SubMlist(list) => {
                hasher.tag(4);
                self.hash_child_identity(*list, hasher);
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
                hasher.u8(slot.raw());
            }
            Whatsit::DeferredWrite { sink, tokens } => {
                hasher.tag(14);
                hash_print_sink(*sink, hasher);
                self.hash_token_list_semantic(*tokens, hasher);
            }
            Whatsit::Special { class, payload } => {
                hasher.tag(15);
                hasher.bytes(class.as_bytes());
                hasher.bytes(payload);
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
        }
    }
}
