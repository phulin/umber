use super::Stores;
use crate::ids::{FontId, GlueId, MacroDefinitionId, NodeListId, TokenListId};
use crate::input::{InputFrameSummary, InputSummary, SourceFrameSummary};
use crate::interner::{Symbol, SymbolId, SymbolReference};
use crate::meaning::Meaning;
use crate::node::Node;
use crate::node_arena::{
    NodeDescriptor, NodeHandle, NodeHandleEvent, NodeHandlePolicy, NodeListRef, NodeRef,
    NodeSchemaVisitor,
};
use crate::token::{OriginId, Token};
use crate::world::World;

impl Stores {
    pub(crate) fn retain_runtime_value_roots_in_nodes(
        &self,
        destination: &mut crate::hot_core::arena::store::RuntimeValueRootSet,
        nodes: &[Node],
    ) {
        let mut collector = RuntimeValueRootCollector {
            registry: &self.runtime_values,
            source: &self.runtime_values.roots,
            destination,
        };
        for node in nodes {
            NodeRef::from(node).visit_schema(&mut collector);
        }
    }

    pub(crate) fn retain_runtime_value_roots_in_frozen_nodes(
        &self,
        destination: &mut crate::hot_core::arena::store::RuntimeValueRootSet,
        nodes: crate::node_arena::NodeList<'_>,
    ) {
        let mut collector = RuntimeValueRootCollector {
            registry: &self.runtime_values,
            source: &self.runtime_values.roots,
            destination,
        };
        for node in nodes.iter() {
            node.visit_schema(&mut collector);
        }
    }

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
                let text = record.box_root().map_or_else(
                    || "void".to_owned(),
                    |root| self.box_restore_trace_text_ref(root),
                );
                record.capture_box_trace_text(text);
            }
        }
    }

    pub(crate) fn box_restore_trace_text_ref(&self, root: &NodeListRef) -> String {
        Self::format_box_restore_trace(root.nodes().first(), |child| {
            root.resolve(child).is_some_and(|child| !child.is_empty())
        })
    }

    fn format_box_restore_trace(
        node: Option<NodeRef<'_>>,
        has_children: impl FnOnce(NodeListId) -> bool,
    ) -> String {
        let Some(node) = node else {
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
        if has_children(box_node.children) {
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
                    let token_view = self.tokens(token_list.id());
                    if origin_list.id() != crate::ids::OriginListId::EMPTY {
                        assert_eq!(
                            self.origin_list_len(*origin_list),
                            token_view.len(),
                            "input origin-list length does not match token list"
                        );
                    }
                    assert!(
                        *index <= token_view.len(),
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
    pub(crate) fn resolve_stored_token_list(&self, id: TokenListId) -> TokenListId {
        self.runtime_values
            .token_id_at(id.raw())
            .expect("stored token-list slot is not live")
    }

    pub(crate) fn resolve_stored_glue(&self, id: GlueId) -> GlueId {
        self.runtime_values
            .glue_id_at(id.raw())
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
                    .runtime_values
                    .macro_id_at(definition.raw())
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
            self.runtime_values.contains_token(id),
            "token list is not live in this Universe timeline"
        );
    }

    pub(super) fn assert_live_glue(&self, id: GlueId) {
        assert!(
            self.runtime_values.contains_glue(id),
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
            self.runtime_values.contains_macro(id),
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

    pub(super) fn assert_live_token(&self, token: Token) {
        if let Token::Cs(symbol) = token {
            assert!(
                self.interner.resolve_stored(symbol).is_some(),
                "symbol is not live in this Universe timeline"
            );
        }
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

    pub(crate) fn assert_live_handles_in_direct_node(
        &self,
        node: &Node,
        owns_child: impl Fn(NodeListId) -> bool,
    ) {
        NodeRef::from(node).visit_schema(&mut DirectHandleValidator {
            stores: self,
            owns_child: &owns_child,
        });
    }

    pub(crate) fn write_box_reg_ref(
        &mut self,
        index: u16,
        value: Option<NodeListRef>,
        global: bool,
    ) -> crate::env::CellMutationReceipt {
        let old = self.env.box_reg(index);
        let new = value.as_ref().map(NodeListRef::id);
        let (receipt, rec) = if global {
            self.env.set_box_reg_global(index, value)
        } else {
            self.env.set_box_reg(index, value)
        };
        let receipt = if receipt.changed() && self.update_main_memory_box_root(old, new) {
            receipt.with_main_memory_roots_updated()
        } else {
            receipt
        };
        let _ = rec;
        receipt
    }

    pub(crate) fn write_box_reg_ref_same_level(
        &mut self,
        index: u16,
        value: Option<NodeListRef>,
    ) -> crate::env::CellMutationReceipt {
        let old = self.env.box_reg(index);
        let new = value.as_ref().map(NodeListRef::id);
        let (receipt, rec) = self.env.set_box_reg_same_level(index, value);
        let receipt = if receipt.changed() && self.update_main_memory_box_root(old, new) {
            receipt.with_main_memory_roots_updated()
        } else {
            receipt
        };
        let _ = rec;
        receipt
    }
}

struct RuntimeValueRootCollector<'a> {
    registry: &'a crate::hot_core::arena::store::registry::RuntimeValueRegistry,
    source: &'a crate::hot_core::arena::store::RuntimeValueRootSet,
    destination: &'a mut crate::hot_core::arena::store::RuntimeValueRootSet,
}

impl NodeSchemaVisitor for RuntimeValueRootCollector<'_> {
    fn descriptor(&mut self, _descriptor: &'static NodeDescriptor) {}

    fn handle(&mut self, event: NodeHandleEvent<'_>) {
        match event.handle {
            NodeHandle::Glue(id) => self
                .registry
                .retain_glue_into(self.source, self.destination, id)
                .expect("validated node glue must retain its sealed region"),
            NodeHandle::TokenList(id) => self
                .registry
                .retain_token_list_into(self.source, self.destination, id)
                .expect("validated node tokens must retain their sealed region"),
            NodeHandle::Font(_)
            | NodeHandle::NodeList(_)
            | NodeHandle::Origin(_)
            | NodeHandle::Origins(_)
            | NodeHandle::OriginRefs(_) => {}
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
            NodeHandle::NodeList(_) => {}
            NodeHandle::Origin(_) | NodeHandle::Origins(_) | NodeHandle::OriginRefs(_) => {
                unreachable!("semantic node handles cannot contain origins")
            }
        }
    }
}

struct DirectHandleValidator<'a, F> {
    stores: &'a Stores,
    owns_child: &'a F,
}

impl<F> NodeSchemaVisitor for DirectHandleValidator<'_, F>
where
    F: Fn(NodeListId) -> bool,
{
    fn descriptor(&mut self, _descriptor: &'static NodeDescriptor) {}

    fn handle(&mut self, event: NodeHandleEvent<'_>) {
        if let NodeHandle::NodeList(id) = event.handle {
            assert!(
                (self.owns_child)(id),
                "direct node-list child coordinate is stale or unowned"
            );
            return;
        }
        if event.policy == NodeHandlePolicy::Diagnostic {
            return;
        }
        match event.handle {
            NodeHandle::Font(id) => self.stores.assert_live_font(id),
            NodeHandle::Glue(id) => self.stores.assert_live_glue(id),
            NodeHandle::TokenList(id) => self.stores.assert_live_token_list(id),
            NodeHandle::NodeList(_) => unreachable!(),
            NodeHandle::Origin(_) | NodeHandle::Origins(_) | NodeHandle::OriginRefs(_) => {
                unreachable!("semantic node handles cannot contain origins")
            }
        }
    }
}
