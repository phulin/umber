use super::{SnapshotOwner, StoreSnapshot, Stores};
use crate::cell::{BankTag, CellId};
use crate::glue::GlueSpec;
use crate::ids::{FontId, GlueId, MacroDefinitionId, NodeListId, TokenListId};
use crate::interner::{ControlSequenceKind, Symbol};
use crate::journal::Entry;
use crate::meaning::{
    ExpandablePrimitive, InternalInteger, Meaning, RawMeaning, UnexpandablePrimitive,
};
use crate::node::{BoxNode, GlueKind, KernKind, LeaderPayload, Node, Sign, Whatsit};
use crate::node_arena::NodeArenaMark;
use crate::state_hash::StateHasher;
use crate::token::{Catcode, Token};
use std::collections::BTreeMap;

const STORE_SLICE_DOMAIN: u64 = 0x7374_6f72_6573_6c63;
const CELL_VALUE_DOMAIN: u64 = 0x6365_6c6c_7661_6c75;
const TOKEN_LIST_MAX_ITEMS: usize = 1_000_000;
const NODE_LIST_MAX_ITEMS: usize = 1_000_000;
const FONT_DIMEN_BITS: u32 = 15;
const FONT_DIMEN_MASK: u32 = (1 << FONT_DIMEN_BITS) - 1;

/// Cursor into store-owned state for semantic convergence hashing.
#[derive(Clone, Debug, Eq, PartialEq)]
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
    pub(crate) fn retarget_state_hash_cursor(
        &self,
        cursor: &StoreStateHashCursor,
    ) -> StoreStateHashCursor {
        assert!(
            cursor.journal_pos <= self.env.current_journal_pos(),
            "Stores state-hash cursor journal position is past the current journal"
        );
        StoreStateHashCursor {
            owner: self.owner.snapshot_owner(),
            journal_pos: cursor.journal_pos,
            node_mark: cursor.node_mark,
        }
    }

    #[must_use]
    pub(crate) fn retarget_state_hash_cursor_after_node_release(
        &self,
        cursor: &StoreStateHashCursor,
    ) -> StoreStateHashCursor {
        self.assert_valid_hash_cursor(cursor);
        let current_mark = self.nodes.watermark();
        StoreStateHashCursor {
            owner: self.owner.snapshot_owner(),
            journal_pos: cursor.journal_pos,
            node_mark: cursor.node_mark.min(current_mark),
        }
    }

    #[must_use]
    pub(crate) fn retarget_state_hash_cursor_after_journal_compaction(
        &self,
        cursor: &StoreStateHashCursor,
    ) -> StoreStateHashCursor {
        assert_eq!(
            cursor.owner,
            self.owner.snapshot_owner(),
            "Stores state-hash cursor belongs to a different Stores instance"
        );
        let current_journal_pos = self.env.current_journal_pos();
        StoreStateHashCursor {
            owner: self.owner.snapshot_owner(),
            journal_pos: cursor.journal_pos.min(current_journal_pos),
            node_mark: cursor.node_mark,
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
        self.hyphenation.hash_semantic(&mut hasher);
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

    pub(crate) fn hash_node_slice_semantic(&self, nodes: &[Node], hasher: &mut StateHasher) {
        hasher.tag(0x72);
        hasher.usize(nodes.len());
        for node in nodes {
            self.hash_node_tree_from_node(node.clone(), hasher);
        }
    }

    pub(crate) fn hash_glue_semantic(&self, id: GlueId, hasher: &mut StateHasher) {
        self.hash_glue(id, hasher);
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
            BankTag::Meaning => {
                let symbol = Symbol::new(cell.index());
                SemanticCellKey::Meaning {
                    kind: self.interner.kind(symbol),
                    name: self.interner.resolve(symbol).to_owned(),
                }
            }
            BankTag::FontDimen => {
                let (font, slot) = unpack_font_dimen_index(cell.index());
                SemanticCellKey::FontBank {
                    bank: bank_order(cell.bank()),
                    font: self.font_semantic_key(font),
                    index: u32::from(slot),
                }
            }
            BankTag::FontParamLen | BankTag::FontHyphenChar | BankTag::FontSkewChar => {
                SemanticCellKey::FontBank {
                    bank: bank_order(cell.bank()),
                    font: self.font_semantic_key(FontId::new(cell.index())),
                    index: 0,
                }
            }
            bank => SemanticCellKey::Bank {
                bank: bank_order(bank),
                index: cell.index(),
            },
        }
    }

    fn hash_cell_key(&self, key: &SemanticCellKey, hasher: &mut StateHasher) {
        match key {
            SemanticCellKey::Meaning { kind, name } => {
                hasher.tag(0x01);
                hash_control_sequence_kind(*kind, hasher);
                hasher.str(name);
            }
            SemanticCellKey::Bank { bank, index } => {
                hasher.tag(0x02);
                hasher.u8(*bank);
                hasher.u32(*index);
            }
            SemanticCellKey::FontBank { bank, font, index } => {
                hasher.tag(0x03);
                hasher.u8(*bank);
                hash_font_semantic_key(font, hasher);
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
            BankTag::FontDimen => hasher.i32(word as u32 as i32),
            BankTag::FontParamLen => hasher.u16(decode_u16(word)),
            BankTag::FontHyphenChar | BankTag::FontSkewChar => hasher.i32(word as u32 as i32),
            BankTag::CurrentFont => self.hash_current_font_word(word, hasher),
            BankTag::MathFamilyFont => self.hash_font(FontId::new(decode_u32(word)), hasher),
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
            Meaning::CharToken { ch, cat } => {
                hasher.tag(21);
                hasher.u32(ch as u32);
                hash_catcode(cat, hasher);
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
            Meaning::MuGlueParam(index) => hash_register_alias(20, index, hasher),
            Meaning::PageDimension(dimension) => {
                hasher.tag(18);
                hasher.u8(dimension.index());
            }
            Meaning::PageInteger(integer) => {
                hasher.tag(19);
                hasher.u8(integer.index());
            }
            Meaning::InternalInteger(integer) => {
                hasher.tag(22);
                hash_internal_integer(integer, hasher);
            }
            Meaning::Font(id) => {
                hasher.tag(17);
                self.hash_font(id, hasher);
            }
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
                hash_control_sequence_kind(self.interner.kind(symbol), hasher);
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
                self.hash_font(font, hasher);
                hasher.u32(ch as u32);
            }
            Node::Lig { font, ch, orig } => {
                hasher.tag(1);
                self.hash_font(font, hasher);
                hasher.u32(ch as u32);
                hasher.u32(orig.0 as u32);
                hasher.u32(orig.1 as u32);
            }
            Node::Kern { amount, kind } => {
                hasher.tag(2);
                hasher.i32(amount.raw());
                hash_kern_kind(kind, hasher);
            }
            Node::Glue { spec, kind, leader } => {
                hasher.tag(3);
                self.hash_glue(spec, hasher);
                hash_glue_kind(kind, hasher);
                self.hash_leader_payload(leader, hasher, stack);
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
                stack.push(NodeFrame::List(unset.children));
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
                stack.push(NodeFrame::List(replace));
                stack.push(NodeFrame::List(post));
                stack.push(NodeFrame::List(pre));
            }
            Node::Mark { class, tokens } => {
                hasher.tag(10);
                hasher.u16(class);
                self.hash_token_list_semantic(tokens, hasher);
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
                hasher.u16(class);
                hasher.i32(size.raw());
                self.hash_glue_semantic(split_top_skip, hasher);
                hasher.i32(split_max_depth.raw());
                hasher.i32(floating_penalty);
                stack.push(NodeFrame::List(content));
            }
            Node::Whatsit(whatsit) => self.hash_whatsit(whatsit, hasher),
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
                stack.push(NodeFrame::List(content));
            }
            Node::MathNoad(noad) => {
                hasher.tag(16);
                hash_noad_kind(&noad.kind, hasher);
                self.hash_math_field(noad.nucleus, hasher, stack);
                self.hash_math_field(noad.subscript, hasher, stack);
                self.hash_math_field(noad.superscript, hasher, stack);
            }
            Node::FractionNoad(fraction) => {
                hasher.tag(17);
                stack.push(NodeFrame::List(fraction.denominator));
                stack.push(NodeFrame::List(fraction.numerator));
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
                stack.push(NodeFrame::List(choice.script_script));
                stack.push(NodeFrame::List(choice.script));
                stack.push(NodeFrame::List(choice.text));
                stack.push(NodeFrame::List(choice.display));
            }
            Node::MathList(list) => {
                hasher.tag(20);
                hasher.u8(u8::from(list.display));
                stack.push(NodeFrame::List(list.content));
            }
            Node::Nonscript => hasher.tag(21),
        }
    }

    fn hash_math_field(
        &self,
        field: crate::math::MathField,
        hasher: &mut StateHasher,
        stack: &mut Vec<NodeFrame>,
    ) {
        match field {
            crate::math::MathField::Empty => hasher.tag(0),
            crate::math::MathField::MathChar(ch) => {
                hasher.tag(1);
                hash_math_char(ch, hasher);
            }
            crate::math::MathField::MathTextChar(ch) => {
                hasher.tag(2);
                hash_math_char(ch, hasher);
            }
            crate::math::MathField::SubBox(list) => {
                hasher.tag(3);
                stack.push(NodeFrame::List(list));
            }
            crate::math::MathField::SubMlist(list) => {
                hasher.tag(4);
                stack.push(NodeFrame::List(list));
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
        hasher.i32(box_node.glue_set.raw());
        hash_sign(box_node.glue_sign, hasher);
        hasher.u8(box_node.glue_order as u8);
        stack.push(NodeFrame::List(box_node.children));
    }

    fn hash_leader_payload(
        &self,
        payload: Option<LeaderPayload>,
        hasher: &mut StateHasher,
        stack: &mut Vec<NodeFrame>,
    ) {
        match payload {
            None => hasher.tag(0),
            Some(LeaderPayload::HList(box_node)) => self.hash_box_node(1, box_node, hasher, stack),
            Some(LeaderPayload::VList(box_node)) => self.hash_box_node(2, box_node, hasher, stack),
            Some(LeaderPayload::Rule {
                width,
                height,
                depth,
            }) => {
                hasher.tag(3);
                hash_optional_scaled(width, hasher);
                hash_optional_scaled(height, hasher);
                hash_optional_scaled(depth, hasher);
            }
        }
    }

    fn hash_whatsit(&self, whatsit: Whatsit, hasher: &mut StateHasher) {
        match whatsit {
            Whatsit::OpenOut { slot, path } => {
                hasher.tag(13);
                hasher.u8(slot.raw());
                hasher.str(&path);
            }
            Whatsit::CloseOut { slot } => {
                hasher.tag(14);
                hasher.u8(slot.raw());
            }
            Whatsit::DeferredWrite { sink, tokens } => {
                hasher.tag(12);
                hash_print_sink(sink, hasher);
                self.hash_token_list_semantic(tokens, hasher);
            }
            Whatsit::Special { class, payload } => {
                hasher.tag(16);
                hasher.bytes(class.as_bytes());
                hasher.bytes(&payload);
            }
            Whatsit::Language {
                language,
                left_hyphen_min,
                right_hyphen_min,
            } => {
                hasher.tag(17);
                hasher.u8(language);
                hasher.u8(left_hyphen_min);
                hasher.u8(right_hyphen_min);
            }
        }
    }

    fn hash_font(&self, font: FontId, hasher: &mut StateHasher) {
        self.assert_live_font(font);
        hash_font_semantic_key(&self.font_semantic_key(font), hasher);
    }

    fn font_semantic_key(&self, font: FontId) -> FontSemanticKey {
        self.assert_live_font(font);
        let font = self.fonts.get(font);
        FontSemanticKey {
            name: font.name().to_owned(),
            path: font.path().to_string_lossy().into_owned(),
            content_hash: font.content_hash(),
            checksum: font.checksum(),
            design_size: font.design_size().raw(),
            size: font.size().raw(),
        }
    }

    fn hash_current_font_word(&self, word: u64, hasher: &mut StateHasher) {
        hasher.tag(0x69);
        let font = FontId::new(word as u32);
        self.hash_font(font, hasher);
        let symbol = word >> 32;
        if symbol == 0 {
            hasher.bool(false);
        } else {
            let symbol = Symbol::new((symbol - 1) as u32);
            self.assert_live_symbol(symbol);
            hasher.bool(true);
            hash_control_sequence_kind(self.interner.kind(symbol), hasher);
            hasher.str(self.interner.resolve(symbol));
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

fn hash_print_sink(sink: crate::world::PrintSink, hasher: &mut StateHasher) {
    match sink {
        crate::world::PrintSink::Terminal => hasher.tag(0),
        crate::world::PrintSink::Log => hasher.tag(1),
        crate::world::PrintSink::TerminalAndLog => hasher.tag(2),
        crate::world::PrintSink::Stream(slot) => {
            hasher.tag(3);
            hasher.u8(slot.raw());
        }
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum SemanticCellKey {
    Meaning {
        kind: ControlSequenceKind,
        name: String,
    },
    Bank {
        bank: u8,
        index: u32,
    },
    FontBank {
        bank: u8,
        font: FontSemanticKey,
        index: u32,
    },
}

fn hash_control_sequence_kind(kind: ControlSequenceKind, hasher: &mut StateHasher) {
    hasher.u8(match kind {
        ControlSequenceKind::Named => 0,
        ControlSequenceKind::ActiveCharacter => 1,
    });
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct FontSemanticKey {
    name: String,
    path: String,
    content_hash: [u8; 32],
    checksum: u32,
    design_size: i32,
    size: i32,
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

fn hash_font_semantic_key(font: &FontSemanticKey, hasher: &mut StateHasher) {
    hasher.tag(0x68);
    hasher.str(&font.name);
    hasher.str(&font.path);
    hasher.bytes(&font.content_hash);
    hasher.u32(font.checksum);
    hasher.i32(font.design_size);
    hasher.i32(font.size);
}

fn hash_kern_kind(kind: KernKind, hasher: &mut StateHasher) {
    hasher.u8(match kind {
        KernKind::Explicit => 0,
        KernKind::Font => 1,
        KernKind::Accent => 2,
        KernKind::Mu => 3,
    });
}

fn hash_glue_kind(kind: GlueKind, hasher: &mut StateHasher) {
    hasher.u8(match kind {
        GlueKind::Normal => 0,
        GlueKind::BaselineSkip => 1,
        GlueKind::LineSkip => 2,
        GlueKind::TopSkip => 3,
        GlueKind::SplitTopSkip => 4,
        GlueKind::LeftSkip => 5,
        GlueKind::RightSkip => 6,
        GlueKind::ParFillSkip => 7,
        GlueKind::Leaders => 8,
        GlueKind::Cleaders => 9,
        GlueKind::Xleaders => 10,
        GlueKind::MuSkip => 11,
        GlueKind::NonScript => 12,
        GlueKind::AboveDisplaySkip => 13,
        GlueKind::BelowDisplaySkip => 14,
        GlueKind::AboveDisplayShortSkip => 15,
        GlueKind::BelowDisplayShortSkip => 16,
        GlueKind::ThinMuSkip => 17,
        GlueKind::MedMuSkip => 18,
        GlueKind::ThickMuSkip => 19,
        GlueKind::TabSkip => 20,
    });
}

fn hash_math_char(ch: crate::math::MathChar, hasher: &mut StateHasher) {
    hasher.u8(ch.family);
    hasher.u32(ch.character as u32);
}

fn hash_noad_kind(kind: &crate::math::NoadKind, hasher: &mut StateHasher) {
    match kind {
        crate::math::NoadKind::Normal(class) => {
            hasher.tag(0);
            hasher.u8(match class {
                crate::math::NoadClass::Ord => 0,
                crate::math::NoadClass::Op => 1,
                crate::math::NoadClass::Bin => 2,
                crate::math::NoadClass::Rel => 3,
                crate::math::NoadClass::Open => 4,
                crate::math::NoadClass::Close => 5,
                crate::math::NoadClass::Punct => 6,
                crate::math::NoadClass::Inner => 7,
            });
        }
        crate::math::NoadKind::Operator(limit_type) => {
            hasher.tag(1);
            hasher.u8(match limit_type {
                crate::math::LimitType::DisplayLimits => 0,
                crate::math::LimitType::Limits => 1,
                crate::math::LimitType::NoLimits => 2,
            });
        }
        crate::math::NoadKind::Radical { delimiter } => {
            hasher.tag(2);
            hasher.u32(*delimiter);
        }
        crate::math::NoadKind::Accent { accent } => {
            hasher.tag(3);
            hash_math_char(*accent, hasher);
        }
        crate::math::NoadKind::LeftDelimiter { delimiter } => {
            hasher.tag(4);
            hasher.u32(*delimiter);
        }
        crate::math::NoadKind::RightDelimiter { delimiter } => {
            hasher.tag(5);
            hasher.u32(*delimiter);
        }
        crate::math::NoadKind::Underline => hasher.tag(6),
        crate::math::NoadKind::Overline => hasher.tag(7),
        crate::math::NoadKind::VCenter => hasher.tag(8),
    }
}

fn hash_fraction_thickness(thickness: crate::math::FractionThickness, hasher: &mut StateHasher) {
    match thickness {
        crate::math::FractionThickness::Default => hasher.tag(0),
        crate::math::FractionThickness::Explicit(value) => {
            hasher.tag(1);
            hasher.i32(value.raw());
        }
    }
}

fn hash_optional_delimiter(delimiter: Option<u32>, hasher: &mut StateHasher) {
    match delimiter {
        Some(delimiter) => {
            hasher.bool(true);
            hasher.u32(delimiter);
        }
        None => hasher.bool(false),
    }
}

fn hash_internal_integer(integer: InternalInteger, hasher: &mut StateHasher) {
    match integer {
        InternalInteger::Badness => hasher.tag(0),
        InternalInteger::InputLineNumber => hasher.tag(1),
    }
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
        BankTag::FontDimen => 11,
        BankTag::FontParamLen => 12,
        BankTag::FontHyphenChar => 13,
        BankTag::FontSkewChar => 14,
        BankTag::CurrentFont => 15,
        BankTag::MathFamilyFont => 16,
    }
}

fn decode_u32(word: u64) -> u32 {
    match u32::try_from(word) {
        Ok(value) => value,
        Err(_) => panic!("opaque id word exceeds u32"),
    }
}

fn unpack_font_dimen_index(index: u32) -> (FontId, u16) {
    let font = FontId::new(index >> FONT_DIMEN_BITS);
    let slot = ((index & FONT_DIMEN_MASK) + 1) as u16;
    (font, slot)
}

fn decode_u16(word: u64) -> u16 {
    match u16::try_from(word) {
        Ok(value) => value,
        Err(_) => panic!("font parameter count exceeds u16"),
    }
}
