use std::collections::BTreeMap;

use tex_expand::{
    ExpansionHooks, NoopExpansionHooks, ReadRecorder, get_x_token_with_recorder_and_hooks,
    scan_dimen::DimensionDiagnostic, token_text,
};
use tex_lex::{InputSource, InputStack, MemoryInput, TokenListReplayKind};
use tex_out::{
    BoxNode as PageBoxNode, ContentHash as PageContentHash, DEFAULT_BANNER,
    DiscKind as PageDiscKind, EffectSink, FontResource, GlueKind as PageGlueKind,
    GlueOrder as PageGlueOrder, GlueSign, GlueSpec as PageGlueSpec, JobInfo,
    KernKind as PageKernKind, LeaderPayload as PageLeaderPayload, PageArtifact, PageEffect,
    PageNode, PageToken, TokenCatcode,
};
use tex_state::glue::Order;
use tex_state::ids::{FontId, NodeListId, TokenListId};
use tex_state::node::{
    BoxNode as StateBoxNode, DiscKind as StateDiscKind, GlueKind as StateGlueKind,
    KernKind as StateKernKind, LeaderPayload as StateLeaderPayload, Node, Sign, Whatsit,
};
use tex_state::page::PageInteger;
use tex_state::token::{Catcode, Token, TracedTokenWord};
use tex_state::{ContentHash, EffectRecord, PrintSink, Universe};

use super::scan_required_box_node;
use crate::ExecError;
use crate::diagnostics;

pub(super) fn execute_shipout<S, R, H>(
    context: TracedTokenWord,
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
    let node = scan_required_box_node(input, stores, hooks, context)?;
    shipout_node(node, stores, recorder)
}

pub(crate) fn shipout_node<R>(
    node: Node,
    stores: &mut Universe,
    recorder: &mut R,
) -> Result<ContentHash, ExecError>
where
    R: ReadRecorder,
{
    let boundary = stores.begin_shipout();
    let pending_effects = pending_page_effects(stores.world().effect_records());
    let counts = page_counts(stores);
    let (mag, diagnostic) = stores.prepare_mag();
    if let Some(diagnostic) = diagnostic {
        diagnostics::report_dimension_diagnostic(stores, DimensionDiagnostic::from(diagnostic));
    }
    let (root, fonts, effects) = {
        let mut lowerer = ShipoutLowerer {
            stores,
            recorder,
            fonts: Vec::new(),
            font_map: BTreeMap::new(),
            effects: pending_effects,
            suppress_deferred_streams: false,
        };
        let root = lowerer.lower_root_node(node)?;
        (root, lowerer.fonts, lowerer.effects)
    };

    let artifact = PageArtifact {
        job: JobInfo {
            mag,
            banner: DEFAULT_BANNER.to_owned(),
        },
        fonts,
        counts,
        root,
        effects,
    };
    let bytes = artifact.to_bytes();
    let effect_pos = stores.world().effect_pos();
    let hash = stores.commit_shipout(boundary, &bytes, effect_pos)?;
    stores.set_page_integer(PageInteger::DeadCycles, 0);
    Ok(hash)
}

struct ShipoutLowerer<'a, R> {
    stores: &'a mut Universe,
    recorder: &'a mut R,
    fonts: Vec<FontResource>,
    font_map: BTreeMap<FontId, u32>,
    effects: Vec<PageEffect>,
    suppress_deferred_streams: bool,
}

impl<R> ShipoutLowerer<'_, R>
where
    R: ReadRecorder,
{
    fn lower_root_node(&mut self, node: Node) -> Result<PageNode, ExecError> {
        self.lower_node(node)?
            .ok_or(ExecError::UnsupportedShipoutNode {
                node: "suppressed shipout root",
            })
    }

    fn lower_node(&mut self, node: Node) -> Result<Option<PageNode>, ExecError> {
        Ok(Some(match node {
            Node::Char { font, ch } => PageNode::Char {
                font_id: self.font_resource_id(font),
                ch: ch as u32,
                width: self.glyph_width(font, ch)?,
            },
            Node::Lig { font, ch, orig } => PageNode::Lig {
                font_id: self.font_resource_id(font),
                ch: ch as u32,
                left: orig.0 as u32,
                right: orig.1 as u32,
                width: self.glyph_width(font, ch)?,
            },
            Node::Kern { amount, kind } => PageNode::Kern {
                amount,
                kind: lower_kern_kind(kind),
            },
            Node::Glue { spec, kind, leader } => PageNode::Glue {
                spec: lower_glue(self.stores.glue(spec)),
                kind: lower_glue_kind(kind),
                leader: self.lower_leader_payload(leader)?,
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
            Node::Unset(_) => {
                return Err(ExecError::UnsupportedShipoutNode {
                    node: "unset alignment",
                });
            }
            Node::Whatsit(whatsit) => return self.lower_whatsit(whatsit),
            Node::MathOn(width) => PageNode::MathOn(width),
            Node::MathOff(width) => PageNode::MathOff(width),
            Node::Disc {
                kind,
                pre,
                post,
                replace,
            } => PageNode::Disc {
                kind: lower_disc_kind(kind),
                pre: self.lower_node_list(pre)?,
                post: self.lower_node_list(post)?,
                replace: self.lower_node_list(replace)?,
            },
            Node::Mark { class, tokens } => PageNode::Mark {
                class,
                tokens: self.lower_tokens(tokens),
            },
            Node::Ins { class, content, .. } => PageNode::Insert {
                class,
                content: self.lower_node_list(content)?,
            },
            Node::Adjust(content) => PageNode::Adjust(self.lower_node_list(content)?),
            Node::MathNoad(_)
            | Node::FractionNoad(_)
            | Node::MathStyle(_)
            | Node::MathChoice(_)
            | Node::MathList(_)
            | Node::Nonscript => {
                return Err(ExecError::UnsupportedShipoutNode { node: "math" });
            }
        }))
    }

    fn lower_box(&mut self, box_node: StateBoxNode) -> Result<PageBoxNode, ExecError> {
        Ok(PageBoxNode {
            width: box_node.width,
            height: box_node.height,
            depth: box_node.depth,
            shift: box_node.shift,
            glue_set: box_node.glue_set,
            glue_sign: lower_glue_sign(box_node.glue_sign),
            glue_order: lower_order(box_node.glue_order),
            children: self.lower_node_list(box_node.children)?,
        })
    }

    fn lower_leader_payload(
        &mut self,
        leader: Option<StateLeaderPayload>,
    ) -> Result<Option<PageLeaderPayload>, ExecError> {
        Ok(match leader {
            None => None,
            Some(StateLeaderPayload::HList(box_node)) => Some(PageLeaderPayload::HList(
                self.lower_leader_payload_box(box_node)?,
            )),
            Some(StateLeaderPayload::VList(box_node)) => Some(PageLeaderPayload::VList(
                self.lower_leader_payload_box(box_node)?,
            )),
            Some(StateLeaderPayload::Rule {
                width,
                height,
                depth,
            }) => Some(PageLeaderPayload::Rule {
                width,
                height,
                depth,
            }),
        })
    }

    fn lower_leader_payload_box(
        &mut self,
        box_node: StateBoxNode,
    ) -> Result<PageBoxNode, ExecError> {
        let outer = self.suppress_deferred_streams;
        self.suppress_deferred_streams = true;
        let lowered = self.lower_box(box_node);
        self.suppress_deferred_streams = outer;
        lowered
    }

    fn lower_node_list(&mut self, list: NodeListId) -> Result<Vec<PageNode>, ExecError> {
        let nodes = self.stores.nodes(list).to_vec();
        let mut lowered = Vec::with_capacity(nodes.len());
        for node in nodes {
            if let Some(node) = self.lower_node(node)? {
                lowered.push(node);
            }
        }
        Ok(lowered)
    }

    fn lower_tokens(&self, list: TokenListId) -> Vec<PageToken> {
        self.stores
            .tokens(list)
            .iter()
            .map(|token| self.lower_token(*token))
            .collect()
    }

    fn lower_token(&self, token: Token) -> PageToken {
        match token {
            Token::Char { ch, cat } => PageToken::Char {
                ch: ch as u32,
                cat: lower_token_catcode(cat),
            },
            Token::Cs(symbol) => PageToken::ControlSequence(self.stores.resolve(symbol).to_owned()),
            Token::Param(slot) => PageToken::Param(slot),
        }
    }

    fn lower_whatsit(&mut self, whatsit: Whatsit) -> Result<Option<PageNode>, ExecError> {
        match whatsit {
            Whatsit::OpenOut { slot, path } => {
                if self.suppress_deferred_streams {
                    return Ok(None);
                }
                let effect_index = self.effects.len();
                self.stores.world_mut().open_out(slot, path.clone());
                self.effects.push(PageEffect::OpenOut {
                    stream: slot.raw(),
                    path,
                });
                Ok(Some(PageNode::WhatsitAnchor {
                    effect_index: u32::try_from(effect_index)
                        .map_err(|_| ExecError::ArithmeticOverflow)?,
                }))
            }
            Whatsit::CloseOut { slot } => {
                if self.suppress_deferred_streams {
                    return Ok(None);
                }
                let effect_index = self.effects.len();
                self.stores.world_mut().close_out(slot);
                self.effects
                    .push(PageEffect::CloseOut { stream: slot.raw() });
                Ok(Some(PageNode::WhatsitAnchor {
                    effect_index: u32::try_from(effect_index)
                        .map_err(|_| ExecError::ArithmeticOverflow)?,
                }))
            }
            Whatsit::DeferredWrite { sink, tokens } => {
                if self.suppress_deferred_streams {
                    return Ok(None);
                }
                let text = expand_write_tokens(self.stores, self.recorder, tokens)?;
                let effect_index = self.effects.len();
                self.stores.world_mut().write_text(sink, &text);
                self.effects.push(PageEffect::Write {
                    sink: lower_sink(sink),
                    text,
                });
                Ok(Some(PageNode::WhatsitAnchor {
                    effect_index: u32::try_from(effect_index)
                        .map_err(|_| ExecError::ArithmeticOverflow)?,
                }))
            }
            Whatsit::Special { class, payload } => {
                let effect_index = self.effects.len();
                self.effects.push(PageEffect::Special { class, payload });
                Ok(Some(PageNode::WhatsitAnchor {
                    effect_index: u32::try_from(effect_index)
                        .map_err(|_| ExecError::ArithmeticOverflow)?,
                }))
            }
        }
    }

    fn font_resource_id(&mut self, font: FontId) -> u32 {
        if let Some(id) = self.font_map.get(&font) {
            return *id;
        }
        let id = font.raw().saturating_sub(1);
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

    fn glyph_width(&self, font: FontId, ch: char) -> Result<tex_state::scaled::Scaled, ExecError> {
        let Ok(code) = u8::try_from(ch as u32) else {
            return Err(ExecError::UnsupportedShipoutNode {
                node: "non-TeX82 character",
            });
        };
        self.stores
            .font_char_metrics(font, code)
            .map(|metrics| metrics.width)
            .ok_or(ExecError::UnsupportedShipoutNode {
                node: "missing character metrics",
            })
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
            .map(tex_expand::semantic_token)
    {
        text.push_str(&token_text(stores, token));
    }
    Ok(crate::diagnostics::print_text_with_newlinechar(
        stores, &text,
    ))
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

fn lower_kern_kind(kind: StateKernKind) -> PageKernKind {
    match kind {
        StateKernKind::Explicit => PageKernKind::Explicit,
        StateKernKind::Font => PageKernKind::Font,
        StateKernKind::Accent => PageKernKind::Accent,
        StateKernKind::Mu => PageKernKind::Explicit,
    }
}

fn lower_disc_kind(kind: StateDiscKind) -> PageDiscKind {
    match kind {
        StateDiscKind::Discretionary => PageDiscKind::Discretionary,
        StateDiscKind::ExplicitHyphen => PageDiscKind::ExplicitHyphen,
        StateDiscKind::AutomaticHyphen => PageDiscKind::AutomaticHyphen,
    }
}

fn lower_glue_kind(kind: StateGlueKind) -> PageGlueKind {
    match kind {
        StateGlueKind::Normal | StateGlueKind::TabSkip => PageGlueKind::Normal,
        StateGlueKind::BaselineSkip => PageGlueKind::BaselineSkip,
        StateGlueKind::LineSkip => PageGlueKind::LineSkip,
        StateGlueKind::TopSkip
        | StateGlueKind::SplitTopSkip
        | StateGlueKind::AboveDisplaySkip
        | StateGlueKind::BelowDisplaySkip
        | StateGlueKind::AboveDisplayShortSkip
        | StateGlueKind::BelowDisplayShortSkip => PageGlueKind::Normal,
        StateGlueKind::LeftSkip => PageGlueKind::LeftSkip,
        StateGlueKind::RightSkip => PageGlueKind::RightSkip,
        StateGlueKind::ParFillSkip => PageGlueKind::ParFillSkip,
        StateGlueKind::Leaders => PageGlueKind::Leaders,
        StateGlueKind::Cleaders => PageGlueKind::Cleaders,
        StateGlueKind::Xleaders => PageGlueKind::Xleaders,
        StateGlueKind::MuSkip
        | StateGlueKind::ThinMuSkip
        | StateGlueKind::MedMuSkip
        | StateGlueKind::ThickMuSkip
        | StateGlueKind::NonScript => PageGlueKind::Normal,
    }
}

fn lower_token_catcode(cat: Catcode) -> TokenCatcode {
    match cat {
        Catcode::Escape => TokenCatcode::Escape,
        Catcode::BeginGroup => TokenCatcode::BeginGroup,
        Catcode::EndGroup => TokenCatcode::EndGroup,
        Catcode::MathShift => TokenCatcode::MathShift,
        Catcode::AlignmentTab => TokenCatcode::AlignmentTab,
        Catcode::EndLine => TokenCatcode::EndLine,
        Catcode::Parameter => TokenCatcode::Parameter,
        Catcode::Superscript => TokenCatcode::Superscript,
        Catcode::Subscript => TokenCatcode::Subscript,
        Catcode::Ignored => TokenCatcode::Ignored,
        Catcode::Space => TokenCatcode::Space,
        Catcode::Letter => TokenCatcode::Letter,
        Catcode::Other => TokenCatcode::Other,
        Catcode::Active => TokenCatcode::Active,
        Catcode::Comment => TokenCatcode::Comment,
        Catcode::Invalid => TokenCatcode::Invalid,
    }
}
