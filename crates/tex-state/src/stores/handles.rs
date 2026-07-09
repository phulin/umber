use super::Stores;
use crate::cell::BankTag;
use crate::env::EnvSnapshot;
use crate::ids::{
    ArenaRef, FontId, GlueId, MacroDefinitionId, NodeListId, OriginListId, TokenListId,
};
use crate::interner::Symbol;
use crate::math::MathField;
use crate::meaning::Meaning;
use crate::node::{LeaderPayload, Node};
use crate::token::{OriginId, Token};

impl Stores {
    pub(super) fn assert_live_symbol(&self, symbol: Symbol) {
        assert!(
            self.interner.contains(symbol),
            "symbol is not live in this Universe timeline"
        );
    }

    pub(super) fn assert_live_token_list(&self, id: TokenListId) {
        assert!(
            self.tokens.contains(id),
            "token list is not live in this Universe timeline"
        );
    }

    pub(super) fn assert_live_glue(&self, id: GlueId) {
        assert!(
            self.glue.contains(id),
            "glue id is not live in this Universe timeline"
        );
    }

    pub(super) fn assert_live_font(&self, id: FontId) {
        assert!(
            self.fonts.contains(id),
            "font id is not live in this Universe timeline"
        );
    }

    pub(super) fn assert_live_macro_definition(&self, id: MacroDefinitionId) {
        assert!(
            self.macros.contains(id),
            "macro definition id is not live in this Universe timeline"
        );
    }

    pub(super) fn assert_live_origin(&self, id: OriginId) {
        assert!(
            self.provenance.contains_origin(id),
            "origin id is not live in this Universe timeline"
        );
    }

    pub(super) fn assert_live_origin_list(&self, id: OriginListId) {
        assert!(
            self.provenance.contains_list(id),
            "origin list id is not live in this Universe timeline"
        );
    }

    pub(super) fn assert_live_token(&self, token: Token) {
        if let Token::Cs(symbol) = token {
            self.assert_live_symbol(symbol);
        }
    }

    pub(super) fn assert_live_node_list(&self, id: NodeListId) {
        let live = match id.arena() {
            ArenaRef::Epoch => self.nodes.contains(id),
            ArenaRef::Survivor(_) => self.survivors.contains(id),
        };
        assert!(live, "node list is not live in this Universe timeline");
    }

    pub(super) fn assert_live_macro_definition_in_meaning(&self, meaning: Meaning) {
        if let Meaning::Macro { definition, .. } = meaning {
            self.assert_live_macro_definition(definition);
        }
    }

    pub(super) fn assert_live_font_in_meaning(&self, meaning: Meaning) {
        if let Meaning::Font(id) = meaning {
            self.assert_live_font(id);
        }
    }

    pub(super) fn assert_live_handles_in_nodes(&self, nodes: &[Node]) {
        for node in nodes {
            self.assert_live_handles_in_node(node);
        }
    }

    fn assert_live_handles_in_node(&self, node: &Node) {
        match node {
            Node::Glue { spec, leader, .. } => {
                self.assert_live_glue(*spec);
                if let Some(leader) = leader {
                    self.assert_live_handles_in_leader_payload(leader);
                }
            }
            Node::Char { font, .. } | Node::Lig { font, .. } => self.assert_live_font(*font),
            Node::HList(box_node) | Node::VList(box_node) => {
                self.assert_live_child_node_list(box_node.children);
            }
            Node::Unset(unset) => {
                self.assert_live_child_node_list(unset.children);
            }
            Node::Disc {
                pre, post, replace, ..
            } => {
                self.assert_live_child_node_list(*pre);
                self.assert_live_child_node_list(*post);
                self.assert_live_child_node_list(*replace);
            }
            Node::Mark { tokens, .. } => self.assert_live_token_list(*tokens),
            Node::Ins {
                split_top_skip,
                content,
                ..
            } => {
                self.assert_live_glue(*split_top_skip);
                self.assert_live_child_node_list(*content);
            }
            Node::Adjust(content) => {
                self.assert_live_child_node_list(*content);
            }
            Node::MathNoad(noad) => {
                self.assert_live_handles_in_math_field(&noad.nucleus);
                self.assert_live_handles_in_math_field(&noad.subscript);
                self.assert_live_handles_in_math_field(&noad.superscript);
            }
            Node::FractionNoad(fraction) => {
                self.assert_live_child_node_list(fraction.numerator);
                self.assert_live_child_node_list(fraction.denominator);
            }
            Node::MathChoice(choice) => {
                self.assert_live_child_node_list(choice.display);
                self.assert_live_child_node_list(choice.text);
                self.assert_live_child_node_list(choice.script);
                self.assert_live_child_node_list(choice.script_script);
            }
            Node::MathList(list) => self.assert_live_child_node_list(list.content),
            Node::Whatsit(crate::node::Whatsit::DeferredWrite { tokens, .. }) => {
                self.assert_live_token_list(*tokens);
            }
            Node::Whatsit(
                crate::node::Whatsit::OpenOut { .. }
                | crate::node::Whatsit::CloseOut { .. }
                | crate::node::Whatsit::Special { .. },
            ) => {}
            Node::Kern { .. }
            | Node::Penalty(_)
            | Node::Rule { .. }
            | Node::MathOn(_)
            | Node::MathOff(_)
            | Node::MathStyle(_)
            | Node::Nonscript => {}
        }
    }

    fn assert_live_handles_in_math_field(&self, field: &MathField) {
        match field {
            MathField::SubBox(list) | MathField::SubMlist(list) => {
                self.assert_live_child_node_list(*list);
            }
            MathField::Empty | MathField::MathChar(_) | MathField::MathTextChar(_) => {}
        }
    }

    fn assert_live_handles_in_leader_payload(&self, payload: &LeaderPayload) {
        match payload {
            LeaderPayload::HList(box_node) | LeaderPayload::VList(box_node) => {
                self.assert_live_child_node_list(box_node.children);
            }
            LeaderPayload::Rule { .. } => {}
        }
    }

    fn assert_live_child_node_list(&self, id: NodeListId) {
        match id.arena() {
            ArenaRef::Epoch => {
                assert!(
                    self.nodes.contains(id),
                    "child node-list id is not live in this Universe timeline"
                );
            }
            ArenaRef::Survivor(_) => {
                assert!(
                    self.survivors.contains(id),
                    "child node-list id is not live in this Universe timeline"
                );
            }
        }
    }

    pub(super) fn prepare_box_value(&mut self, value: NodeListId) -> NodeListId {
        self.assert_live_node_list(value);
        match value.arena() {
            ArenaRef::Epoch => self.survivors.promote(value, &self.nodes),
            ArenaRef::Survivor(_) => {
                self.survivors.inc_ref(value);
                value
            }
        }
    }

    pub(super) fn write_box_reg(&mut self, index: u16, value: Option<NodeListId>, global: bool) {
        let old = self.env.box_reg(index);
        let value = match value {
            Some(value) if Some(value) == old => Some(value),
            Some(value) => Some(self.prepare_box_value(value)),
            None => None,
        };
        let rec = if global {
            self.env.set_box_reg_global(index, value)
        } else {
            self.env.set_box_reg(index, value)
        };
        self.account_box_write(old, rec);
    }

    pub(super) fn write_box_reg_same_level(&mut self, index: u16, value: Option<NodeListId>) {
        let old = self.env.box_reg(index);
        let value = match value {
            Some(value) if Some(value) == old => Some(value),
            Some(value) => Some(self.prepare_box_value(value)),
            None => None,
        };
        let rec = self.env.set_box_reg_same_level(index, value);
        self.account_box_write(old, rec);
    }

    pub(super) fn account_box_write(
        &mut self,
        old: Option<NodeListId>,
        outcome: crate::env::banks::BoxWriteOutcome,
    ) {
        match outcome {
            crate::env::banks::BoxWriteOutcome::Unchanged => {}
            crate::env::banks::BoxWriteOutcome::Journaled { rec, .. } => {
                if rec.old() == rec.new_value() {
                    self.inc_survivor_ref(NodeListId::decode_box_word(rec.old()));
                }
                if rec.old() == 0 {
                    self.dec_survivor_ref_opt(old);
                }
            }
            crate::env::banks::BoxWriteOutcome::Coalesced { displaced } => {
                self.dec_survivor_ref_opt(NodeListId::decode_box_word(displaced));
            }
        }
    }

    pub(super) fn account_rollback_box_refs(&mut self, snapshot: EnvSnapshot) {
        let dropped: Vec<_> = self
            .env
            .journal_entries_since(snapshot.journal_pos())
            .iter()
            .rev()
            .filter_map(|entry| match entry {
                crate::journal::Entry::Undo(rec) if rec.cell().bank() == BankTag::Box => {
                    Some(rec.new_value())
                }
                _ => None,
            })
            .collect();
        for word in dropped {
            self.dec_survivor_ref_opt(NodeListId::decode_box_word(word));
        }
    }

    pub(super) fn account_current_group_box_refs(&mut self) {
        let Some(pos) = self.env.last_group_marker_pos() else {
            return;
        };
        let dropped: Vec<_> = self
            .env
            .journal_entries_since(pos)
            .iter()
            .rev()
            .filter_map(|entry| match entry {
                crate::journal::Entry::Undo(rec)
                    if rec.cell().bank() == BankTag::Box && !rec.cell().is_global() =>
                {
                    Some(rec.new_value())
                }
                _ => None,
            })
            .collect();
        for word in dropped {
            self.dec_survivor_ref_opt(NodeListId::decode_box_word(word));
        }
    }

    pub(super) fn inc_survivor_ref(&mut self, value: Option<NodeListId>) {
        if let Some(id) = value
            && matches!(id.arena(), ArenaRef::Survivor(_))
        {
            self.survivors.inc_ref(id);
        }
    }

    pub(super) fn dec_survivor_ref_opt(&mut self, value: Option<NodeListId>) {
        if let Some(id) = value {
            self.dec_survivor_ref(id);
        }
    }

    pub(super) fn dec_survivor_ref(&mut self, id: NodeListId) {
        if matches!(id.arena(), ArenaRef::Survivor(_)) {
            self.survivors.dec_ref(id);
        }
    }
}
