use std::collections::BTreeMap;

use tex_expand::{
    ExpansionHooks, NoopExpansionHooks, ReadRecorder, get_x_token_with_recorder_and_hooks,
    token_text,
};
use tex_lex::{InputSource, InputStack, MemoryInput, TokenListReplayKind};
use tex_out::{
    BoxNode as PageBoxNode, ContentHash as PageContentHash, EffectSink, FontResource,
    GlueKind as PageGlueKind, GlueOrder as PageGlueOrder, GlueSetRatio, GlueSign,
    GlueSpec as PageGlueSpec, KernKind as PageKernKind, PageArtifact, PageEffect, PageNode,
};
use tex_state::glue::Order;
use tex_state::ids::{FontId, NodeListId, TokenListId};
use tex_state::node::{
    BoxNode as StateBoxNode, GlueKind as StateGlueKind, KernKind as StateKernKind, Node, Sign,
    Whatsit,
};
use tex_state::{ContentHash, EffectRecord, PrintSink, Universe};

use super::scan_required_box_node;
use crate::ExecError;

pub(super) fn execute_shipout<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<ContentHash, ExecError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    let node = scan_required_box_node(input, stores, hooks)?;
    let pending_effects = pending_page_effects(stores.world().effect_records());
    let counts = page_counts(stores);
    let (root, fonts, effects) = {
        let mut lowerer = ShipoutLowerer {
            stores,
            recorder,
            fonts: Vec::new(),
            font_map: BTreeMap::new(),
            effects: pending_effects,
        };
        let root = lowerer.lower_node(node)?;
        (root, lowerer.fonts, lowerer.effects)
    };

    let artifact = PageArtifact {
        fonts,
        counts,
        root,
        effects,
    };
    let bytes = artifact.to_bytes();
    let hash = stores.world_mut().store_artifact(&bytes)?;
    let effect_pos = stores.world().effect_pos();
    stores.commit_effects(effect_pos)?;
    let _checkpoint = stores.snapshot();
    Ok(hash)
}

struct ShipoutLowerer<'a, R> {
    stores: &'a mut Universe,
    recorder: &'a mut R,
    fonts: Vec<FontResource>,
    font_map: BTreeMap<FontId, u32>,
    effects: Vec<PageEffect>,
}

impl<R> ShipoutLowerer<'_, R>
where
    R: ReadRecorder,
{
    fn lower_node(&mut self, node: Node) -> Result<PageNode, ExecError> {
        Ok(match node {
            Node::Char { font, ch } => PageNode::Char {
                font_id: self.font_resource_id(font),
                ch: ch as u32,
            },
            Node::Lig { font, ch, orig } => PageNode::Lig {
                font_id: self.font_resource_id(font),
                ch: ch as u32,
                left: orig.0 as u32,
                right: orig.1 as u32,
            },
            Node::Kern { amount, kind } => PageNode::Kern {
                amount,
                kind: lower_kern_kind(kind),
            },
            Node::Glue { spec, kind } => PageNode::Glue {
                spec: lower_glue(self.stores.glue(spec)),
                kind: lower_glue_kind(kind),
            },
            Node::Penalty(value) => PageNode::Penalty(value),
            Node::Rule {
                width,
                height,
                depth,
            } => PageNode::Rule {
                width,
                height,
                depth,
            },
            Node::HList(box_node) => PageNode::HList(self.lower_box(box_node)?),
            Node::VList(box_node) => PageNode::VList(self.lower_box(box_node)?),
            Node::Unset => PageNode::Unset,
            Node::Whatsit(whatsit) => self.lower_whatsit(whatsit)?,
            Node::MathOn => PageNode::MathOn,
            Node::MathOff => PageNode::MathOff,
            Node::Disc { .. } => {
                return Err(ExecError::UnsupportedShipoutNode { node: "disc" });
            }
            Node::Mark { .. } => {
                return Err(ExecError::UnsupportedShipoutNode { node: "mark" });
            }
            Node::Ins { .. } => {
                return Err(ExecError::UnsupportedShipoutNode { node: "insert" });
            }
            Node::Adjust(_) => {
                return Err(ExecError::UnsupportedShipoutNode { node: "adjust" });
            }
        })
    }

    fn lower_box(&mut self, box_node: StateBoxNode) -> Result<PageBoxNode, ExecError> {
        Ok(PageBoxNode {
            width: box_node.width,
            height: box_node.height,
            depth: box_node.depth,
            shift: box_node.shift,
            glue_set: lower_glue_set(box_node.glue_set),
            glue_sign: lower_glue_sign(box_node.glue_sign),
            glue_order: lower_order(box_node.glue_order),
            children: self.lower_node_list(box_node.children)?,
        })
    }

    fn lower_node_list(&mut self, list: NodeListId) -> Result<Vec<PageNode>, ExecError> {
        let nodes = self.stores.nodes(list).to_vec();
        nodes
            .into_iter()
            .map(|node| self.lower_node(node))
            .collect()
    }

    fn lower_whatsit(&mut self, whatsit: Whatsit) -> Result<PageNode, ExecError> {
        match whatsit {
            Whatsit::DeferredWrite { sink, tokens } => {
                let text = expand_write_tokens(self.stores, self.recorder, tokens)?;
                let effect_index = self.effects.len();
                self.stores.world_mut().write_text(sink, &text);
                self.effects.push(PageEffect::Write {
                    sink: lower_sink(sink),
                    text,
                });
                Ok(PageNode::WhatsitAnchor {
                    effect_index: u32::try_from(effect_index)
                        .map_err(|_| ExecError::ArithmeticOverflow)?,
                })
            }
        }
    }

    fn font_resource_id(&mut self, font: FontId) -> u32 {
        if let Some(id) = self.font_map.get(&font) {
            return *id;
        }
        let id = u32::try_from(self.fonts.len()).expect("page font count exceeds u32");
        let loaded = self.stores.font(font);
        self.fonts.push(FontResource {
            font_id: id,
            name: loaded.name().to_owned(),
            tfm_content_hash: PageContentHash::new(loaded.content_hash()),
            tfm_checksum: loaded.checksum(),
            design_size: loaded.design_size(),
            at_size: loaded.size(),
        });
        self.font_map.insert(font, id);
        id
    }
}

fn expand_write_tokens<R>(
    stores: &mut Universe,
    recorder: &mut R,
    tokens: TokenListId,
) -> Result<String, ExecError>
where
    R: ReadRecorder,
{
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(tokens, TokenListReplayKind::Inserted);
    let mut hooks = NoopExpansionHooks;
    let mut text = String::new();
    while let Some(token) =
        get_x_token_with_recorder_and_hooks(&mut input, stores, recorder, &mut hooks)?
    {
        text.push_str(&token_text(stores, token));
    }
    Ok(text)
}

fn pending_page_effects(records: &[EffectRecord]) -> Vec<PageEffect> {
    records.iter().filter_map(lower_effect_record).collect()
}

fn lower_effect_record(record: &EffectRecord) -> Option<PageEffect> {
    match record {
        EffectRecord::StreamOpen { slot, target } => Some(PageEffect::OpenOut {
            stream: slot.raw(),
            path: target.path().to_string_lossy().into_owned(),
        }),
        EffectRecord::StreamClose { slot } => Some(PageEffect::CloseOut { stream: slot.raw() }),
        EffectRecord::StreamWrite { sink, text } => Some(PageEffect::Write {
            sink: lower_sink(*sink),
            text: text.clone(),
        }),
        EffectRecord::Special { class, payload } => Some(PageEffect::Special {
            class: class.clone(),
            payload: payload.clone(),
        }),
        EffectRecord::DeferredWrite { .. }
        | EffectRecord::PdfObjectPlaceholder { .. }
        | EffectRecord::ShellEscape(_) => None,
    }
}

fn page_counts(stores: &Universe) -> [i32; 10] {
    let mut counts = [0; 10];
    for (index, value) in counts.iter_mut().enumerate() {
        *value = stores.count(index as u16);
    }
    counts
}

fn lower_sink(sink: PrintSink) -> EffectSink {
    match sink {
        PrintSink::Terminal => EffectSink::Terminal,
        PrintSink::Log => EffectSink::Log,
        PrintSink::TerminalAndLog => EffectSink::TerminalAndLog,
        PrintSink::Stream(slot) => EffectSink::Stream(slot.raw()),
    }
}

fn lower_glue(spec: tex_state::glue::GlueSpec) -> PageGlueSpec {
    PageGlueSpec {
        width: spec.width,
        stretch: spec.stretch,
        stretch_order: lower_order(spec.stretch_order),
        shrink: spec.shrink,
        shrink_order: lower_order(spec.shrink_order),
    }
}

fn lower_order(order: Order) -> PageGlueOrder {
    match order {
        Order::Normal => PageGlueOrder::Normal,
        Order::Fil => PageGlueOrder::Fil,
        Order::Fill => PageGlueOrder::Fill,
        Order::Filll => PageGlueOrder::Filll,
    }
}

fn lower_glue_sign(sign: Sign) -> GlueSign {
    match sign {
        Sign::Normal => GlueSign::Normal,
        Sign::Stretching => GlueSign::Stretching,
        Sign::Shrinking => GlueSign::Shrinking,
    }
}

fn lower_glue_set(value: f64) -> GlueSetRatio {
    const SCALE: f64 = 1_000_000.0;
    let raw = if value.is_finite() {
        (value * SCALE).round()
    } else {
        0.0
    };
    GlueSetRatio {
        raw: raw.clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32,
    }
}

fn lower_kern_kind(kind: StateKernKind) -> PageKernKind {
    match kind {
        StateKernKind::Explicit => PageKernKind::Explicit,
        StateKernKind::Font => PageKernKind::Font,
        StateKernKind::Accent => PageKernKind::Accent,
    }
}

fn lower_glue_kind(kind: StateGlueKind) -> PageGlueKind {
    match kind {
        StateGlueKind::Normal => PageGlueKind::Normal,
        StateGlueKind::BaselineSkip => PageGlueKind::BaselineSkip,
        StateGlueKind::LineSkip => PageGlueKind::LineSkip,
        StateGlueKind::LeftSkip => PageGlueKind::LeftSkip,
        StateGlueKind::RightSkip => PageGlueKind::RightSkip,
        StateGlueKind::ParFillSkip => PageGlueKind::ParFillSkip,
        StateGlueKind::Leaders => PageGlueKind::Leaders,
        StateGlueKind::Cleaders => PageGlueKind::Cleaders,
        StateGlueKind::Xleaders => PageGlueKind::Xleaders,
    }
}
