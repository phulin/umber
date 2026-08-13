use super::Stores;
use crate::env::EnvSnapshot;
use crate::ids::{
    ArenaRef, FontId, GlueId, MacroDefinitionId, NodeListId, OriginListId, TokenListId,
};
use crate::input::{InputFrameSummary, InputSummary, SourceFrameSummary};
use crate::interner::{Symbol, SymbolId, SymbolReference};
use crate::meaning::Meaning;
use crate::node::Node;
use crate::node_arena::{
    NodeDescriptor, NodeHandle, NodeHandleEvent, NodeHandlePolicy, NodeRef, NodeSchemaVisitor,
};
use crate::token::{OriginId, Token};
use crate::world::World;

impl Stores {
    /// Detaches TeX82 §§252/283's display from the effective value selected by
    /// `unsave`, while group ownership still keeps that value live. Journal
    /// redo words are replay metadata, not TeX save-stack values: in
    /// particular, a global assignment's discarded `old` word may already be
    /// retired by the time the group ends.
    pub(super) fn capture_box_restore_texts(
        &self,
        restores: &mut [crate::env::group::RestoreRecord],
    ) {
        for record in restores {
            if record.cell().bank() == crate::cell::BankTag::Box {
                let text = NodeListId::decode_box_word(record.old())
                    .map_or_else(|| "void".to_owned(), |id| self.box_restore_trace_text(id));
                record.capture_box_trace_text(text);
            }
        }
    }

    pub(crate) fn box_restore_trace_text(&self, id: NodeListId) -> String {
        let Some(node) = self.nodes(id).first() else {
            return "void".to_owned();
        };
        let (name, box_node) = match node {
            crate::node_arena::NodeRef::HList(box_node) => ("hbox", box_node),
            crate::node_arena::NodeRef::VList(box_node) => ("vbox", box_node),
            _ => return "[]".to_owned(),
        };
        let scaled = |value: crate::scaled::Scaled| {
            let raw = i64::from(value.raw());
            let sign = if raw < 0 { "-" } else { "" };
            let magnitude = raw.abs();
            let whole = magnitude / 65_536;
            let fraction = magnitude % 65_536;
            if fraction == 0 {
                format!("{sign}{whole}.0")
            } else {
                let mut digits = format!("{:05}", (fraction * 100_000 + 32_768) / 65_536);
                while digits.ends_with('0') {
                    digits.pop();
                }
                format!("{sign}{whole}.{digits}")
            }
        };
        let mut text = format!(
            "\\{name}({}+{})x{}",
            scaled(box_node.height),
            scaled(box_node.depth),
            scaled(box_node.width)
        );
        if box_node.glue_sign != crate::node::Sign::Normal && !box_node.glue_set.is_zero() {
            use std::fmt::Write as _;
            let sign = if box_node.glue_sign == crate::node::Sign::Shrinking {
                "-"
            } else {
                ""
            };
            let numerator = i64::from(box_node.glue_set.numerator()) * 65_536;
            let denominator = i64::from(box_node.glue_set.denominator());
            let raw = if numerator >= 0 {
                (numerator + denominator / 2) / denominator
            } else {
                -((-numerator + denominator / 2) / denominator)
            };
            let ratio = crate::scaled::Scaled::from_raw(i32::try_from(raw).unwrap_or(i32::MAX));
            let _ = write!(text, ", glue set {sign}{}", scaled(ratio));
        }
        if !self.nodes(box_node.children).is_empty() {
            text.push_str(" []");
        }
        text
    }

    pub(crate) fn assert_live_input_summary(&self, world: &World, summary: &InputSummary) {
        let mut max_source_id = None;
        for frame in summary.frames() {
            match frame {
                InputFrameSummary::Source {
                    source_id,
                    input_record,
                    source,
                } => {
                    max_source_id = Some(
                        max_source_id.map_or(source_id.raw(), |old: u32| old.max(source_id.raw())),
                    );
                    self.assert_live_source_frame(world, *source_id, *input_record, source);
                }
                InputFrameSummary::TokenList {
                    token_list,
                    origin_list,
                    index,
                    macro_arguments,
                    macro_invocation,
                    parent_macro_invocation,
                    ..
                } => {
                    self.assert_live_token_list(token_list.id());
                    self.assert_live_origin_list(*origin_list);
                    assert!(
                        *index <= token_list.tokens().len(),
                        "input token-list frame index exceeds its live token list"
                    );
                    for &word in macro_arguments.tokens().iter() {
                        self.assert_live_traced_token_word(word);
                    }
                    self.assert_live_origin(*macro_invocation);
                    self.assert_live_origin(*parent_macro_invocation);
                }
                InputFrameSummary::TransientTokenList {
                    tokens,
                    macro_invocation,
                    parent_macro_invocation,
                    ..
                } => {
                    for &word in tokens.iter() {
                        self.assert_live_traced_token_word(word);
                    }
                    self.assert_live_origin(*macro_invocation);
                    self.assert_live_origin(*parent_macro_invocation);
                }
                InputFrameSummary::Condition { condition, .. } => {
                    self.assert_live_traced_token_word(condition.context());
                }
            }
        }

        match (
            summary.last_source_id(),
            summary.last_source_record(),
            summary.last_source_frame(),
        ) {
            (Some(source_id), input_record, Some(source)) => {
                max_source_id = Some(
                    max_source_id.map_or(source_id.raw(), |old: u32| old.max(source_id.raw())),
                );
                self.assert_live_source_frame(world, source_id, input_record, source);
            }
            (None, None, None) => {}
            _ => panic!("last input source frame metadata is incomplete"),
        }
        if let Some(max_source_id) = max_source_id {
            assert!(
                summary.next_source_id() > max_source_id,
                "input source id frontier would reuse a live source id"
            );
        }
    }

    fn assert_live_source_frame(
        &self,
        world: &World,
        source_id: crate::input::SourceId,
        input_record: Option<crate::world::InputRecordId>,
        source: &SourceFrameSummary,
    ) {
        assert!(
            source.is_resume_complete(),
            "input source frame is not resume-complete"
        );
        let registration = source
            .registration()
            .expect("input source frame has no registered source capability");
        if self.source_fragments.contains_registration(registration) {
            assert!(
                input_record.is_none(),
                "fragment-backed editor source frame carries a World input record"
            );
            for &word in source.pending() {
                self.assert_live_traced_token_word(word);
            }
            return;
        }
        let region = self
            .source_map
            .region_for_source(source_id)
            .expect("input source id is not live in this Universe timeline");
        assert!(
            self.source_map
                .contains_registration(source_id, registration),
            "input source registration is not live in this Universe timeline"
        );
        let byte_len = usize::try_from(region.byte_len)
            .expect("input source backing length exceeds resume address space");
        assert!(
            source.buffer_offset() <= byte_len && source.next_source_offset() <= byte_len,
            "input source frame offset exceeds its live backing"
        );
        match region.backing {
            crate::source_map::SourceBacking::World(expected) => {
                assert_eq!(
                    input_record,
                    Some(expected),
                    "input source frame record does not match its registered source"
                );
                let record = world
                    .input_record(expected)
                    .expect("input record is not live in this World timeline");
                assert_eq!(
                    record.len(),
                    byte_len,
                    "input source frame record length does not match its registered source"
                );
            }
            crate::source_map::SourceBacking::Generated(backing) => {
                assert!(
                    input_record.is_none(),
                    "generated input source frame carries a World input record"
                );
                assert!(
                    self.source_map.generated(backing).is_some(),
                    "generated input source backing is not live"
                );
            }
        }
        for &word in source.pending() {
            self.assert_live_traced_token_word(word);
        }
    }

    fn assert_live_traced_token_word(&self, word: crate::token::TracedTokenWord) {
        let token = word
            .token()
            .expect("input summary contains an invalid traced token");
        self.assert_live_token(token);
        self.assert_live_origin(word.origin());
    }

    pub(crate) fn resolve_stored_symbol(&self, symbol: Symbol) -> SymbolId {
        self.interner
            .resolve_stored(symbol)
            .expect("stored symbol slot is not live")
    }

    pub(crate) fn try_resolve_stored_symbol(&self, symbol: Symbol) -> Option<SymbolId> {
        self.interner.resolve_stored(symbol)
    }

    pub(crate) fn resolve_symbol_reference(&self, symbol: impl SymbolReference) -> SymbolId {
        if let Some(id) = symbol.live_id() {
            self.assert_live_symbol(id);
            id
        } else {
            self.resolve_stored_symbol(symbol.stored_key().expect("symbol reference kind"))
        }
    }
    pub(super) fn resolve_stored_token_list(&self, id: TokenListId) -> TokenListId {
        self.tokens
            .resolve_stored(id)
            .expect("stored token-list slot is not live")
    }

    pub(super) fn resolve_stored_glue(&self, id: GlueId) -> GlueId {
        self.glue
            .resolve_stored(id)
            .expect("stored glue slot is not live")
    }

    pub(crate) fn resolve_stored_font(&self, id: FontId) -> FontId {
        self.fonts
            .resolve_stored(id)
            .expect("stored font slot is not live")
    }

    pub(crate) fn resolve_stored_meaning(&self, meaning: Meaning) -> Meaning {
        match meaning {
            Meaning::Macro { definition, flags } => Meaning::Macro {
                definition: self
                    .macros
                    .resolve_stored(definition)
                    .expect("stored macro-definition slot is not live"),
                flags,
            },
            Meaning::Font(id) => Meaning::Font(self.resolve_stored_font(id)),
            other => other,
        }
    }

    pub(super) fn assert_live_symbol(&self, symbol: SymbolId) {
        assert!(
            self.interner.contains_id(symbol),
            "symbol is not live in this Universe timeline"
        );
    }

    pub(crate) fn assert_live_token_list(&self, id: TokenListId) {
        assert!(
            self.tokens.resolve_stored(id).is_some(),
            "token list is not live in this Universe timeline"
        );
    }

    pub(super) fn assert_live_glue(&self, id: GlueId) {
        assert!(
            self.glue.resolve_stored(id).is_some(),
            "glue id is not live in this Universe timeline"
        );
    }

    pub(super) fn assert_live_font(&self, id: FontId) {
        assert!(
            self.fonts.resolve_stored(id).is_some(),
            "font id is not live in this Universe timeline"
        );
    }

    pub(super) fn assert_live_macro_definition(&self, id: MacroDefinitionId) {
        assert!(
            self.macros.contains(id),
            "macro definition id is not live in this Universe timeline: {id:?}"
        );
    }

    pub(super) fn assert_live_origin(&self, id: OriginId) {
        let live = match id.decode() {
            crate::token::OriginEncoding::DirectSource(position) => {
                self.source_map
                    .region_for_backed_position(position)
                    .is_some()
                    || self.source_fragments.contains_position(position)
            }
            crate::token::OriginEncoding::NoExpandFallback => true,
            crate::token::OriginEncoding::Unknown | crate::token::OriginEncoding::Arena(_) => {
                self.provenance.contains_origin(id)
            }
        };
        assert!(live, "origin id is not live in this Universe timeline");
    }

    pub(super) fn assert_live_origin_list(&self, id: OriginListId) {
        assert!(
            self.provenance.resolve_stored_list(id).is_some(),
            "origin list id is not live in this Universe timeline"
        );
    }

    pub(super) fn assert_live_token(&self, token: Token) {
        if let Token::Cs(symbol) = token {
            assert!(
                self.interner.resolve_stored(symbol).is_some(),
                "symbol is not live in this Universe timeline"
            );
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

    pub(crate) fn assert_live_handles_in_nodes(&self, nodes: &[Node]) {
        for node in nodes {
            self.assert_live_handles_in_node(node);
        }
    }

    pub(crate) fn assert_live_handles_in_node(&self, node: &Node) {
        NodeRef::from(node).visit_schema(&mut LiveHandleValidator(self));
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

    pub(super) fn write_box_reg(
        &mut self,
        index: u16,
        value: Option<NodeListId>,
        global: bool,
    ) -> crate::env::CellMutationReceipt {
        let old = self.env.box_reg(index);
        let value = match value {
            Some(value) if Some(value) == old => Some(value),
            Some(value) => Some(self.prepare_box_value(value)),
            None => None,
        };
        let (receipt, rec) = if global {
            self.env.set_box_reg_global(index, value)
        } else {
            self.env.set_box_reg(index, value)
        };
        let receipt = if receipt.changed() && self.update_main_memory_box_root(old, value) {
            receipt.with_main_memory_roots_updated()
        } else {
            receipt
        };
        self.account_box_write(old, rec);
        receipt
    }

    pub(super) fn write_box_reg_same_level(
        &mut self,
        index: u16,
        value: Option<NodeListId>,
    ) -> crate::env::CellMutationReceipt {
        let old = self.env.box_reg(index);
        let value = match value {
            Some(value) if Some(value) == old => Some(value),
            Some(value) => Some(self.prepare_box_value(value)),
            None => None,
        };
        let (receipt, rec) = self.env.set_box_reg_same_level(index, value);
        let receipt = if receipt.changed() && self.update_main_memory_box_root(old, value) {
            receipt.with_main_memory_roots_updated()
        } else {
            receipt
        };
        self.account_box_write(old, rec);
        receipt
    }

    pub(super) fn account_box_write(
        &mut self,
        old: Option<NodeListId>,
        outcome: crate::env::banks::BoxWriteOutcome,
    ) {
        match outcome {
            crate::env::banks::BoxWriteOutcome::Unchanged => {}
            crate::env::banks::BoxWriteOutcome::SameLevel => {}
            crate::env::banks::BoxWriteOutcome::Journaled { rec, .. } => {
                if rec.old().value() == rec.new_value().value() {
                    self.inc_survivor_ref(NodeListId::decode_box_word(rec.old().value()));
                }
                if rec.old().value() == 0 {
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
                crate::journal::Entry::BoxUndo(id) => {
                    Some(self.env.box_undo(*id).new_value().value())
                }
                _ => None,
            })
            .collect();
        for word in dropped {
            self.dec_survivor_ref_opt(NodeListId::decode_box_word(word));
        }
    }

    pub(super) fn current_group_box_refs_to_release(&self) -> Vec<NodeListId> {
        let Some(pos) = self.env.last_group_marker_pos() else {
            return Vec::new();
        };
        let leaving_depth = self.env.group_depth();
        self.env
            .journal_entries_since(pos)
            .iter()
            .rev()
            .filter_map(|entry| match entry {
                crate::journal::Entry::BoxUndo(id)
                    if !self.env.box_undo(*id).survives_group(leaving_depth) =>
                {
                    NodeListId::decode_box_word(self.env.box_undo(*id).new_value().value())
                }
                _ => None,
            })
            .collect()
    }

    pub(super) fn release_box_refs(&mut self, dropped: Vec<NodeListId>) {
        for id in dropped {
            self.dec_survivor_ref(id);
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

struct LiveHandleValidator<'a>(&'a Stores);

impl NodeSchemaVisitor for LiveHandleValidator<'_> {
    fn descriptor(&mut self, _descriptor: &'static NodeDescriptor) {}

    fn handle(&mut self, event: NodeHandleEvent<'_>) {
        if event.policy == NodeHandlePolicy::Diagnostic {
            return;
        }
        match event.handle {
            NodeHandle::Font(id) => self.0.assert_live_font(id),
            NodeHandle::Glue(id) => self.0.assert_live_glue(id),
            NodeHandle::TokenList(id) => self.0.assert_live_token_list(id),
            NodeHandle::NodeList(id) => self.0.assert_live_child_node_list(id),
            NodeHandle::Origin(_) | NodeHandle::Origins(_) => {
                unreachable!("semantic node handles cannot contain origins")
            }
        }
    }
}
