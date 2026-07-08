use super::{SnapshotOwner, StoreSnapshot, Stores};
use crate::cell::{BankTag, CellId};
use crate::glue::GlueSpec;
use crate::ids::{GlueId, MacroDefinitionId, NodeListId, TokenListId};
use crate::interner::Symbol;
use crate::journal::Entry;
use crate::meaning::{ExpandablePrimitive, Meaning, RawMeaning, UnexpandablePrimitive};
use crate::node::{BoxNode, GlueKind, KernKind, Node, Sign, Whatsit};
use crate::node_arena::NodeArenaMark;
use crate::state_hash::StateHasher;
use crate::token::{Catcode, Token};
use std::collections::BTreeMap;

const STORE_SLICE_DOMAIN: u64 = 0x7374_6f72_6573_6c63;
const CELL_VALUE_DOMAIN: u64 = 0x6365_6c6c_7661_6c75;
const TOKEN_LIST_MAX_ITEMS: usize = 1_000_000;
const NODE_LIST_MAX_ITEMS: usize = 1_000_000;

/// Cursor into store-owned state for semantic convergence hashing.
#[derive(Clone, Debug)]
pub(crate) struct StoreStateHashCursor {
    owner: SnapshotOwner,
    journal_pos: crate::journal::JournalPos,
    node_mark: NodeArenaMark,
}

impl Stores {
    #[must_use]
    pub(crate) fn state_hash_cursor(&self) -> StoreStateHashCursor {
        StoreStateHashCursor {
            owner: self.owner.snapshot_owner(),
            journal_pos: self.env.current_journal_pos(),
            node_mark: self.nodes.watermark(),
        }
    }

    #[must_use]
    pub(crate) fn state_hash_cursor_from_snapshot(
        snapshot: &StoreSnapshot,
    ) -> StoreStateHashCursor {
        StoreStateHashCursor {
            owner: snapshot.owner,
            journal_pos: snapshot.env_snapshot.journal_pos(),
            node_mark: snapshot.node_mark,
        }
    }

    #[must_use]
    pub(crate) fn state_hash_slice(
        &self,
        start: &StoreStateHashCursor,
        end: &StoreSnapshot,
    ) -> u64 {
        self.assert_valid_hash_cursor(start);
        self.assert_valid_snapshot(end);
        assert!(
            start.journal_pos <= end.env_snapshot.journal_pos(),
            "state hash cursor journal position is after snapshot"
        );

        let mut hasher = StateHasher::new(STORE_SLICE_DOMAIN);
        self.hash_journal_changed_cells(start, end, &mut hasher);
        self.hash_code_generations(&mut hasher);
        self.hash_epoch_node_slice(start.node_mark, &mut hasher);
        hash_prepared_mag(self.prepared_mag, &mut hasher);
        hasher.finish()
    }

    pub(crate) fn hash_token_list_semantic(&self, id: TokenListId, hasher: &mut StateHasher) {
        self.assert_live_token_list(id);
        let tokens = self.tokens.get(id);
        assert!(
            tokens.len() <= TOKEN_LIST_MAX_ITEMS,
            "state hash exceeded maximum token-list items"
        );
        hasher.tag(0x50);
        hasher.usize(tokens.len());
        for token in tokens {
            self.hash_token(*token, hasher);
        }
    }

    fn assert_valid_hash_cursor(&self, cursor: &StoreStateHashCursor) {
        assert_eq!(
            cursor.owner,
            self.owner.snapshot_owner(),
            "Stores state-hash cursor belongs to a different Stores instance"
        );
        assert!(
            cursor.journal_pos <= self.env.current_journal_pos(),
            "Stores state-hash cursor journal position is past the current journal"
        );
    }

    fn hash_journal_changed_cells(
        &self,
        start: &StoreStateHashCursor,
        end: &StoreSnapshot,
        hasher: &mut StateHasher,
    ) {
        let start_index = start.journal_pos.raw() as usize;
        let end_index = end.env_snapshot.journal_pos().raw() as usize;
        let mut first_old = BTreeMap::<SemanticCellKey, (CellId, u64)>::new();
        for entry in &self.env.journal_entries_since(start.journal_pos)
            [..end_index.saturating_sub(start_index)]
        {
            let Entry::Undo(rec) = entry else {
                continue;
            };
            let cell = canonical_cell(rec.cell());
            first_old
                .entry(self.semantic_cell_key(cell))
                .or_insert((cell, rec.old()));
        }

        let mut changed = Vec::new();
        for (key, (cell, old_word)) in first_old {
            let new_word = self.env.semantic_word(cell);
            if self.cell_value_hash(cell, old_word) != self.cell_value_hash(cell, new_word) {
                changed.push((key, cell, new_word));
            }
        }

        hasher.tag(0x10);
        hasher.usize(changed.len());
        for (key, cell, word) in changed {
            self.hash_cell_key(&key, hasher);
            self.hash_cell_value(cell, word, hasher);
        }
    }

    fn semantic_cell_key(&self, cell: CellId) -> SemanticCellKey {
        match cell.bank() {
            BankTag::Meaning => SemanticCellKey::Meaning(
                self.interner.resolve(Symbol::new(cell.index())).to_owned(),
            ),
            bank => SemanticCellKey::Bank {
                bank: bank_order(bank),
                index: cell.index(),
            },
        }
    }

    fn hash_cell_key(&self, key: &SemanticCellKey, hasher: &mut StateHasher) {
        match key {
            SemanticCellKey::Meaning(name) => {
                hasher.tag(0x01);
                hasher.str(name);
            }
            SemanticCellKey::Bank { bank, index } => {
                hasher.tag(0x02);
                hasher.u8(*bank);
                hasher.u32(*index);
            }
        }
    }

    fn cell_value_hash(&self, cell: CellId, word: u64) -> u64 {
        let mut hasher = StateHasher::new(CELL_VALUE_DOMAIN);
        self.hash_cell_value(cell, word, &mut hasher);
        hasher.finish()
    }

    fn hash_cell_value(&self, cell: CellId, word: u64, hasher: &mut StateHasher) {
        match cell.bank() {
            BankTag::Meaning => self.hash_meaning(Meaning::decode_stored(word), hasher),
            BankTag::Count | BankTag::IntParam => hasher.i32(word as u32 as i32),
            BankTag::Dimen | BankTag::DimenParam => hasher.i32(word as u32 as i32),
            BankTag::Skip | BankTag::Muskip | BankTag::GlueParam => {
                self.hash_glue(GlueId::new(decode_u32(word)), hasher);
            }
            BankTag::Toks | BankTag::TokParam => {
                self.hash_token_list_semantic(TokenListId::new(decode_u32(word)), hasher);
            }
            BankTag::Box => match NodeListId::decode_box_word(word) {
                Some(id) => self.hash_node_list(id, hasher),
                None => hasher.tag(0),
            },
        }
    }

    fn hash_meaning(&self, meaning: Meaning, hasher: &mut StateHasher) {
        match meaning {
            Meaning::Undefined => hasher.tag(0),
            Meaning::Relax => hasher.tag(1),
            Meaning::Macro { flags, definition } => {
                hasher.tag(2);
                hasher.u8(flags.bits());
                self.hash_macro_definition(definition, hasher);
            }
            Meaning::CharGiven(ch) => {
                hasher.tag(3);
                hasher.u32(ch as u32);
            }
            Meaning::MathCharGiven(value) => {
                hasher.tag(4);
                hasher.u16(value);
            }
            Meaning::CountRegister(index) => hash_register_alias(5, index, hasher),
            Meaning::DimenRegister(index) => hash_register_alias(6, index, hasher),
            Meaning::SkipRegister(index) => hash_register_alias(7, index, hasher),
            Meaning::MuskipRegister(index) => hash_register_alias(8, index, hasher),
            Meaning::ToksRegister(index) => hash_register_alias(9, index, hasher),
            Meaning::IntParam(index) => hash_register_alias(10, index, hasher),
            Meaning::DimenParam(index) => hash_register_alias(11, index, hasher),
            Meaning::GlueParam(index) => hash_register_alias(12, index, hasher),
            Meaning::TokParam(index) => hash_register_alias(13, index, hasher),
            Meaning::ExpandablePrimitive(primitive) => hash_expandable_primitive(primitive, hasher),
            Meaning::UnexpandablePrimitive(primitive) => {
                hash_unexpandable_primitive(primitive, hasher);
            }
            Meaning::Unknown(raw) => hash_unknown_meaning(raw, hasher),
        }
    }

    fn hash_macro_definition(&self, id: MacroDefinitionId, hasher: &mut StateHasher) {
        self.assert_live_macro_definition(id);
        let definition = self.macros.get(id);
        hasher.u8(definition.flags().bits());
        self.hash_token_list_semantic(definition.parameter_text(), hasher);
        self.hash_token_list_semantic(definition.replacement_text(), hasher);
    }

    fn hash_token(&self, token: Token, hasher: &mut StateHasher) {
        match token {
            Token::Char { ch, cat } => {
                hasher.tag(0);
                hasher.u32(ch as u32);
                hash_catcode(cat, hasher);
            }
            Token::Cs(symbol) => {
                self.assert_live_symbol(symbol);
                hasher.tag(1);
                hasher.str(self.interner.resolve(symbol));
            }
            Token::Param(slot) => {
                hasher.tag(2);
                hasher.u8(slot);
            }
        }
    }

    fn hash_glue(&self, id: GlueId, hasher: &mut StateHasher) {
        self.assert_live_glue(id);
        let GlueSpec {
            width,
            stretch,
            stretch_order,
            shrink,
            shrink_order,
        } = self.glue.get(id);
        hasher.tag(0x60);
        hasher.i32(width.raw());
        hasher.i32(stretch.raw());
        hasher.u8(stretch_order as u8);
        hasher.i32(shrink.raw());
        hasher.u8(shrink_order as u8);
    }

    fn hash_node_list(&self, id: NodeListId, hasher: &mut StateHasher) {
        self.assert_live_node_list(id);
        let mut stack = vec![NodeFrame::List(id)];
        let mut seen = 0_usize;
        while let Some(frame) = stack.pop() {
            seen += 1;
            assert!(
                seen <= NODE_LIST_MAX_ITEMS,
                "state hash exceeded maximum node traversal items"
            );
            match frame {
                NodeFrame::List(id) => {
                    let nodes = self.nodes(id);
                    hasher.tag(0x70);
                    hasher.usize(nodes.len());
                    stack.push(NodeFrame::ListEnd);
                    for node in nodes.iter().rev() {
                        stack.push(NodeFrame::Node(node.clone()));
                    }
                }
                NodeFrame::ListEnd => hasher.tag(0x71),
                NodeFrame::Node(node) => self.hash_node(node, hasher, &mut stack),
            }
        }
    }

    fn hash_node(&self, node: Node, hasher: &mut StateHasher, stack: &mut Vec<NodeFrame>) {
        match node {
            Node::Char { font, ch } => {
                hasher.tag(0);
                hash_font(font, hasher);
                hasher.u32(ch as u32);
            }
            Node::Lig { font, ch, orig } => {
                hasher.tag(1);
                hash_font(font, hasher);
                hasher.u32(ch as u32);
                hasher.u32(orig.0 as u32);
                hasher.u32(orig.1 as u32);
            }
            Node::Kern { amount, kind } => {
                hasher.tag(2);
                hasher.i32(amount.raw());
                hash_kern_kind(kind, hasher);
            }
            Node::Glue { spec, kind } => {
                hasher.tag(3);
                self.hash_glue(spec, hasher);
                hash_glue_kind(kind, hasher);
            }
            Node::Penalty(value) => {
                hasher.tag(4);
                hasher.i32(value);
            }
            Node::Rule {
                width,
                height,
                depth,
            } => {
                hasher.tag(5);
                hash_optional_scaled(width, hasher);
                hash_optional_scaled(height, hasher);
                hash_optional_scaled(depth, hasher);
            }
            Node::HList(box_node) => self.hash_box_node(6, box_node, hasher, stack),
            Node::VList(box_node) => self.hash_box_node(7, box_node, hasher, stack),
            Node::Unset => hasher.tag(8),
            Node::Disc { pre, post, replace } => {
                hasher.tag(9);
                stack.push(NodeFrame::List(replace));
                stack.push(NodeFrame::List(post));
                stack.push(NodeFrame::List(pre));
            }
            Node::Mark { class, tokens } => {
                hasher.tag(10);
                hasher.u16(class);
                self.hash_token_list_semantic(tokens, hasher);
            }
            Node::Ins { class, content } => {
                hasher.tag(11);
                hasher.u16(class);
                stack.push(NodeFrame::List(content));
            }
            Node::Whatsit(whatsit) => self.hash_whatsit(whatsit, hasher),
            Node::MathOn => hasher.tag(13),
            Node::MathOff => hasher.tag(14),
            Node::Adjust(content) => {
                hasher.tag(15);
                stack.push(NodeFrame::List(content));
            }
        }
    }

    fn hash_box_node(
        &self,
        tag: u8,
        box_node: BoxNode,
        hasher: &mut StateHasher,
        stack: &mut Vec<NodeFrame>,
    ) {
        hasher.tag(tag);
        hasher.i32(box_node.width.raw());
        hasher.i32(box_node.height.raw());
        hasher.i32(box_node.depth.raw());
        hasher.i32(box_node.shift.raw());
        hasher.u64(box_node.glue_set.to_bits());
        hash_sign(box_node.glue_sign, hasher);
        hasher.u8(box_node.glue_order as u8);
        stack.push(NodeFrame::List(box_node.children));
    }

    fn hash_whatsit(&self, whatsit: Whatsit, hasher: &mut StateHasher) {
        match whatsit {
            Whatsit::DeferredWrite { stream, tokens } => {
                hasher.tag(12);
                hasher.u8(stream);
                self.hash_token_list_semantic(tokens, hasher);
            }
        }
    }

    fn hash_code_generations(&self, hasher: &mut StateHasher) {
        let generations = self.code_tables.generations();
        hasher.tag(0x20);
        hasher.u32(generations.catcode);
        hasher.u32(generations.lccode);
        hasher.u32(generations.uccode);
        hasher.u32(generations.sfcode);
        hasher.u32(generations.mathcode);
        hasher.u32(generations.delcode);
    }

    fn hash_epoch_node_slice(&self, start: NodeArenaMark, hasher: &mut StateHasher) {
        hasher.tag(0x30);
        let nodes = self.nodes.nodes_since(start);
        hasher.usize(nodes.len());
        for node in nodes {
            self.hash_node_tree_from_node(node.clone(), hasher);
        }
    }

    fn hash_node_tree_from_node(&self, node: Node, hasher: &mut StateHasher) {
        let mut stack = Vec::new();
        self.hash_node(node, hasher, &mut stack);
        let mut seen = 0_usize;
        while let Some(frame) = stack.pop() {
            seen += 1;
            assert!(
                seen <= NODE_LIST_MAX_ITEMS,
                "state hash exceeded maximum node traversal items"
            );
            match frame {
                NodeFrame::List(id) => {
                    let nodes = self.nodes(id);
                    hasher.tag(0x70);
                    hasher.usize(nodes.len());
                    stack.push(NodeFrame::ListEnd);
                    for node in nodes.iter().rev() {
                        stack.push(NodeFrame::Node(node.clone()));
                    }
                }
                NodeFrame::ListEnd => hasher.tag(0x71),
                NodeFrame::Node(node) => self.hash_node(node, hasher, &mut stack),
            }
        }
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum SemanticCellKey {
    Meaning(String),
    Bank { bank: u8, index: u32 },
}

#[derive(Clone, Debug)]
enum NodeFrame {
    List(NodeListId),
    ListEnd,
    Node(Node),
}

fn canonical_cell(cell: CellId) -> CellId {
    CellId::new(cell.bank(), cell.index())
}

fn hash_prepared_mag(value: Option<i32>, hasher: &mut StateHasher) {
    hasher.tag(0x40);
    match value {
        Some(value) => {
            hasher.bool(true);
            hasher.i32(value);
        }
        None => hasher.bool(false),
    }
}

fn hash_register_alias(tag: u8, index: u16, hasher: &mut StateHasher) {
    hasher.tag(tag);
    hasher.u16(index);
}

fn hash_expandable_primitive(primitive: ExpandablePrimitive, hasher: &mut StateHasher) {
    hasher.tag(14);
    hasher.u64(primitive.operand());
}

fn hash_unexpandable_primitive(primitive: UnexpandablePrimitive, hasher: &mut StateHasher) {
    hasher.tag(15);
    hasher.u64(primitive.operand());
}

fn hash_unknown_meaning(raw: RawMeaning, hasher: &mut StateHasher) {
    hasher.tag(16);
    hasher.u8(raw.op());
    hasher.u64(raw.operand());
}

fn hash_catcode(cat: Catcode, hasher: &mut StateHasher) {
    hasher.u8(cat as u8);
}

fn hash_font(_font: crate::ids::FontId, hasher: &mut StateHasher) {
    // Font storage is not implemented yet. Until fonts have content-backed
    // handles, placeholder font slots are deliberately omitted instead of
    // feeding raw allocation-order ids into convergence hashing.
    hasher.tag(0);
}

fn hash_kern_kind(kind: KernKind, hasher: &mut StateHasher) {
    hasher.u8(match kind {
        KernKind::Explicit => 0,
        KernKind::Font => 1,
        KernKind::Accent => 2,
    });
}

fn hash_glue_kind(kind: GlueKind, hasher: &mut StateHasher) {
    hasher.u8(match kind {
        GlueKind::Normal => 0,
        GlueKind::Leaders => 1,
        GlueKind::Cleaders => 2,
        GlueKind::Xleaders => 3,
    });
}

fn hash_sign(sign: Sign, hasher: &mut StateHasher) {
    hasher.u8(match sign {
        Sign::Normal => 0,
        Sign::Stretching => 1,
        Sign::Shrinking => 2,
    });
}

fn hash_optional_scaled(value: Option<crate::scaled::Scaled>, hasher: &mut StateHasher) {
    match value {
        Some(value) => {
            hasher.bool(true);
            hasher.i32(value.raw());
        }
        None => hasher.bool(false),
    }
}

fn bank_order(bank: BankTag) -> u8 {
    match bank {
        BankTag::Meaning => 0,
        BankTag::Count => 1,
        BankTag::Dimen => 2,
        BankTag::Skip => 3,
        BankTag::Toks => 4,
        BankTag::Box => 5,
        BankTag::IntParam => 6,
        BankTag::DimenParam => 7,
        BankTag::GlueParam => 8,
        BankTag::TokParam => 9,
        BankTag::Muskip => 10,
    }
}

fn decode_u32(word: u64) -> u32 {
    match u32::try_from(word) {
        Ok(value) => value,
        Err(_) => panic!("opaque id word exceeds u32"),
    }
}
