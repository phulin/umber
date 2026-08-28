//! Reusable TeX node-list diagnostic dumping.

use std::fmt::Write as _;

use tex_command::CommandProfile;
use tex_state::CommandContext;
use tex_state::env::banks::IntParam;
use tex_state::glue::{GlueSpec, Order};
use tex_state::math::{
    FractionThickness, LimitType, MathChar, MathChoice, MathField, MathFraction, MathListNode,
    MathNoad, MathStyle, NoadClass, NoadKind,
};
use tex_state::node::{
    BoxNode, GlueKind, KernKind, LeaderPayload, Node, Sign, UnsetKind, UnsetNode, Whatsit,
};
use tex_state::node_arena::PageListId;
use tex_state::scaled::{GlueSetRatio, Scaled};
use tex_state::token::Token;
use tex_state::token_show::append_tex_print_char;
use tex_state::token_show::{append_token_show_text, token_text};

#[derive(Clone, Copy)]
pub(crate) struct DumpConfig {
    pub(crate) breadth: i32,
    pub(crate) depth: i32,
    pub(crate) profile: CommandProfile,
}

impl DumpConfig {
    /// TeX82 §198's `show_box`: `depth_threshold:=show_box_depth;
    /// breadth_max:=show_box_breadth` (§236), then `if breadth_max<=0 then
    /// breadth_max:=5`. INITEX leaves both `\showboxbreadth` and
    /// `\showboxdepth` at their default of 0 (tex.web §240's `int_par` table
    /// initialization), so every level's item count must fall back to 5
    /// rather than truncating to zero items and printing `etc.` immediately;
    /// `depth_threshold` has no such fallback and is used as read.
    pub(crate) fn read<G>(stores: &CommandContext<'_, G>) -> Self {
        let breadth = stores.int_param(IntParam::SHOW_BOX_BREADTH);
        Self {
            breadth: if breadth <= 0 { 5 } else { breadth },
            depth: stores.int_param(IntParam::SHOW_BOX_DEPTH),
            profile: CommandProfile::TEX82,
        }
    }

    pub(crate) const fn for_profile(mut self, profile: CommandProfile) -> Self {
        self.profile = profile;
        self
    }
}

pub(crate) fn dump_page_list<G>(
    stores: &CommandContext<'_, G>,
    owner: PageListId,
    config: DumpConfig,
) -> String {
    let mut out = String::new();
    let list = stores
        .page_node_list(owner)
        .expect("diagnostic root belongs to the live page arena");
    let nodes = list.nodes();
    dump_nodes::<_, _, _, _, PageDumpStorage>(
        stores,
        &nodes,
        &config,
        -1,
        ListContext::Neutral,
        false,
        &mut out,
    );
    out
}

pub(crate) fn dump_durable_list<G>(
    stores: &CommandContext<'_, G>,
    owner: tex_state::node_arena::DurableListId<G>,
    config: DumpConfig,
) -> String {
    let mut out = String::new();
    let list = stores
        .node_list(owner)
        .expect("diagnostic root belongs to the live durable generation");
    let nodes = list.nodes();
    dump_nodes::<_, _, _, _, PageDumpStorage>(
        stores,
        &nodes,
        &config,
        -1,
        ListContext::Neutral,
        false,
        &mut out,
    );
    out
}

pub(crate) fn dump_node_slice<G>(
    stores: &CommandContext<'_, G>,
    nodes: &[Node],
    config: DumpConfig,
) -> String {
    let mut out = String::new();
    dump_nodes::<_, _, _, _, PageDumpStorage>(
        stores,
        &nodes,
        &config,
        -1,
        ListContext::Neutral,
        false,
        &mut out,
    );
    out
}

pub(crate) fn dump_node_sequence_view<G>(
    stores: &CommandContext<'_, G>,
    nodes: tex_state::node_arena::NodeCursor<'_>,
    config: DumpConfig,
) -> String {
    let mut out = String::new();
    dump_nodes::<_, _, _, _, PageDumpStorage>(
        stores,
        &nodes,
        &config,
        -1,
        ListContext::Neutral,
        false,
        &mut out,
    );
    out
}

pub(crate) fn dump_incomplete_fraction<G>(
    stores: &CommandContext<'_, G>,
    fraction: &crate::mode::IncompleteFraction,
    config: DumpConfig,
) -> String {
    let mut out = String::new();
    dump_fraction_header(
        stores,
        fraction.thickness,
        fraction.left_delimiter,
        fraction.right_delimiter,
        &mut out,
    );
    dump_fraction_part(stores, &fraction.numerator, &config, 0, "\\", &mut out);
    out
}

#[derive(Clone, Copy)]
enum ListContext {
    Neutral,
    HList,
    VList,
}

struct PageDumpStorage;
struct DurableDumpStorage;

trait DumpListProjection<G, Storage> {
    fn is_empty(&self, stores: &CommandContext<'_, G>) -> bool;
    fn dump(
        &self,
        stores: &CommandContext<'_, G>,
        config: &DumpConfig,
        depth: i32,
        context: ListContext,
        physical_replacement_spans: bool,
        out: &mut String,
    );
}

impl<G> DumpListProjection<G, PageDumpStorage> for PageListId {
    fn is_empty(&self, _stores: &CommandContext<'_, G>) -> bool {
        PageListId::is_empty(*self)
    }

    fn dump(
        &self,
        stores: &CommandContext<'_, G>,
        config: &DumpConfig,
        depth: i32,
        context: ListContext,
        physical_replacement_spans: bool,
        out: &mut String,
    ) {
        let list = stores
            .page_node_list(*self)
            .expect("diagnostic child belongs to the live page arena");
        let nodes = list.nodes();
        dump_nodes::<_, _, _, _, PageDumpStorage>(
            stores,
            &nodes,
            config,
            depth,
            context,
            physical_replacement_spans,
            out,
        );
    }
}

impl<G> DumpListProjection<G, DurableDumpStorage> for tex_state::node_arena::DurableListId<G> {
    fn is_empty(&self, _stores: &CommandContext<'_, G>) -> bool {
        tex_state::node_arena::DurableListId::is_empty(*self)
    }

    fn dump(
        &self,
        stores: &CommandContext<'_, G>,
        config: &DumpConfig,
        depth: i32,
        context: ListContext,
        physical_replacement_spans: bool,
        out: &mut String,
    ) {
        let list = stores
            .node_list(*self)
            .expect("diagnostic child belongs to the live durable generation");
        let nodes = list.nodes();
        dump_nodes::<_, _, _, _, PageDumpStorage>(
            stores,
            &nodes,
            config,
            depth,
            context,
            physical_replacement_spans,
            out,
        );
    }
}

trait DumpGlueProjection<G>: Copy {
    fn resolve(self, stores: &CommandContext<'_, G>) -> GlueSpec;
}

impl<G> DumpGlueProjection<G> for GlueSpec {
    fn resolve(self, _stores: &CommandContext<'_, G>) -> GlueSpec {
        self
    }
}

impl<G> DumpGlueProjection<G> for tex_state::GlueId<G> {
    fn resolve(self, stores: &CommandContext<'_, G>) -> GlueSpec {
        stores.glue(self)
    }
}

trait DumpTokensProjection<G> {
    fn visit(&self, stores: &CommandContext<'_, G>, visit: impl FnMut(tex_state::token::TokenWord));
}

impl<G> DumpTokensProjection<G> for tex_state::node::NodeTokenList {
    fn visit(
        &self,
        _stores: &CommandContext<'_, G>,
        visit: impl FnMut(tex_state::token::TokenWord),
    ) {
        self.words().iter().copied().for_each(visit);
    }
}

impl<G> DumpTokensProjection<G> for tex_state::TokenListId<G> {
    fn visit(
        &self,
        stores: &CommandContext<'_, G>,
        visit: impl FnMut(tex_state::token::TokenWord),
    ) {
        stores.token_list(self.clone()).iter().for_each(visit);
    }
}

fn dump_projected_list<G, List: DumpListProjection<G, Storage>, Storage>(
    stores: &CommandContext<'_, G>,
    list: &List,
    config: &DumpConfig,
    depth: i32,
    context: ListContext,
    out: &mut String,
) {
    list.dump(stores, config, depth, context, false, out);
}

trait DumpNodeCollection<List, Glue, Tokens> {
    fn len(&self) -> usize;
    fn get(&self, index: usize) -> Option<&Node<List, Glue, Tokens>>;
}

impl<List, Glue, Tokens> DumpNodeCollection<List, Glue, Tokens> for &[Node<List, Glue, Tokens>] {
    fn len(&self) -> usize {
        <[Node<List, Glue, Tokens>]>::len(self)
    }

    fn get(&self, index: usize) -> Option<&Node<List, Glue, Tokens>> {
        <[Node<List, Glue, Tokens>]>::get(self, index)
    }
}

impl DumpNodeCollection<PageListId, GlueSpec, tex_state::node::NodeTokenList>
    for tex_state::node_arena::NodeCursor<'_>
{
    fn len(&self) -> usize {
        tex_state::node_arena::NodeCursor::len(self)
    }

    fn get(&self, index: usize) -> Option<&Node> {
        self.owned_node(index)
    }
}

impl DumpNodeCollection<PageListId, GlueSpec, tex_state::node::NodeTokenList>
    for tex_state::node_sequence::NodeSequenceView<'_>
{
    fn len(&self) -> usize {
        tex_state::node_sequence::NodeSequenceView::len(*self)
    }

    fn get(&self, index: usize) -> Option<&Node> {
        tex_state::node_sequence::NodeSequenceView::get(*self, index)
    }
}

fn dump_nodes<G, List, Glue, Tokens, Storage>(
    stores: &CommandContext<'_, G>,
    nodes: &impl DumpNodeCollection<List, Glue, Tokens>,
    config: &DumpConfig,
    depth: i32,
    context: ListContext,
    physical_replacement_spans: bool,
    out: &mut String,
) where
    List: DumpListProjection<G, Storage> + Clone,
    Glue: DumpGlueProjection<G>,
    Tokens: DumpTokensProjection<G>,
{
    if config.depth < 0 || depth > config.depth {
        return;
    }
    let limit = config.breadth.max(0) as usize;
    let mut index = 0;
    let mut displayed = 0;
    while displayed < limit && index < nodes.len() {
        if physical_replacement_spans
            && let (
                Some(Node::Lig { .. }),
                Some(
                    disc @ Node::Disc {
                        kind: tex_state::node::DiscKind::AutomaticHyphen,
                        physical_replace_count: 2,
                        ..
                    },
                ),
                Some(Node::Kern {
                    kind: KernKind::Font,
                    ..
                }),
            ) = (nodes.get(index), nodes.get(index + 1), nodes.get(index + 2))
        {
            // TeX82 §904 links the automatic discretionary ahead of the
            // preceding structured ligature and boundary kern. The semantic
            // carrier keeps the ligature in place; render its physical order
            // without mutating the frozen diagnostic list.
            dump_node(stores, disc, config, depth, context, out);
            displayed += 1;
            if displayed < limit {
                dump_node(
                    stores,
                    nodes.get(index).expect("diagnostic index is in bounds"),
                    config,
                    depth,
                    context,
                    out,
                );
                displayed += 1;
            }
            if displayed < limit {
                dump_node(
                    stores,
                    nodes.get(index + 2).expect("diagnostic index is in bounds"),
                    config,
                    depth,
                    context,
                    out,
                );
                displayed += 1;
            }
            index += 3;
            continue;
        }

        let node = nodes.get(index).expect("diagnostic index is in bounds");
        index += 1;
        displayed += 1;
        dump_node(stores, node, config, depth, context, out);
    }
    if index < nodes.len() {
        write_prefix(depth, out);
        out.push_str("etc.\n");
    }
}

fn dump_node<G, List, Glue, Tokens, Storage>(
    stores: &CommandContext<'_, G>,
    node: &Node<List, Glue, Tokens>,
    config: &DumpConfig,
    depth: i32,
    context: ListContext,
    out: &mut String,
) where
    List: DumpListProjection<G, Storage> + Clone,
    Glue: DumpGlueProjection<G>,
    Tokens: DumpTokensProjection<G>,
{
    write_prefix(depth, out);
    match node {
        // TeX82 §184 names both ordinary and mu kerns through `print_esc`,
        // so their headers observe the live `\escapechar` just like the
        // neighboring glue, math, and discretionary node headers.
        Node::Kern { amount, kind } => match kind {
            KernKind::Explicit => {
                append_escaped_name(stores, "kern", out);
                let _ = writeln!(out, " {}", format_scaled_without_unit(*amount));
            }
            KernKind::Font => {
                append_escaped_name(stores, "kern", out);
                let _ = writeln!(out, "{}", format_scaled_without_unit(*amount));
            }
            KernKind::Auto => {
                append_escaped_name(stores, "kern", out);
                let _ = writeln!(
                    out,
                    " {} (for \\pdfprependkern/\\pdfappendkern)",
                    format_scaled_without_unit(*amount)
                );
            }
            KernKind::Accent => {
                append_escaped_name(stores, "kern", out);
                let _ = writeln!(out, " {} (for accent)", format_scaled_without_unit(*amount));
            }
            KernKind::Mu => {
                append_escaped_name(stores, "mkern", out);
                let _ = writeln!(out, "{}mu", format_scaled_without_unit(*amount));
            }
            KernKind::LeftMargin => {
                append_escaped_name(stores, "kern", out);
                let _ = writeln!(out, "{} (left margin)", format_scaled_without_unit(*amount));
            }
            KernKind::RightMargin => {
                append_escaped_name(stores, "kern", out);
                let _ = writeln!(
                    out,
                    "{} (right margin)",
                    format_scaled_without_unit(*amount)
                );
            }
        },
        Node::MarginKern { amount, side, .. } => {
            let side = match side {
                tex_state::node::MarginKernSide::Left => "left",
                tex_state::node::MarginKernSide::Right => "right",
            };
            append_escaped_name(stores, "kern", out);
            let _ = writeln!(
                out,
                "{} ({side} margin)",
                format_scaled_without_unit(*amount)
            );
        }
        Node::Glue { spec, kind, leader } => {
            if let Some(leader) = leader {
                kind.append_leader_dump_prefix(stores, out);
                let _ = writeln!(
                    out,
                    "{}",
                    format_glue(spec.resolve(stores), kind.glue_unit())
                );
                dump_leader_payload(stores, leader, config, depth + 1, context, out);
            } else {
                kind.append_glue_dump_prefix(stores, out);
                if kind.prints_glue_spec() {
                    out.push_str(&format_glue(spec.resolve(stores), kind.glue_unit()));
                }
                out.push('\n');
            }
        }
        Node::HList(box_node) => {
            dump_box("hbox", stores, box_node, config, depth, context, out);
        }
        Node::VList(box_node) => {
            dump_box("vbox", stores, box_node, config, depth, context, out);
        }
        Node::Unset(unset) => {
            dump_unset(stores, unset, config, depth, out);
        }
        Node::Rule {
            width,
            height,
            depth,
        } => dump_rule(stores, *width, *height, *depth, out),
        Node::Penalty(value) => {
            // TeX82 §184 names a penalty node through §63 `print_esc`, so
            // the header observes the live `\escapechar` value.
            append_escaped_name(stores, "penalty", out);
            let _ = writeln!(out, " {value}");
        }
        Node::Char { font, ch, .. } => {
            let _ = writeln!(
                out,
                "{} {}",
                font_identifier(stores, *font),
                dump_character_string(stores, *ch)
            );
        }
        Node::Lig {
            font,
            ch,
            orig,
            left_hit,
            right_hit,
            ..
        } => {
            let _ = writeln!(
                out,
                "{} {}",
                font_identifier(stores, *font),
                dump_ligature(stores, *ch, orig, *left_hit, *right_hit)
            );
        }
        Node::Disc {
            pre,
            post,
            physical_replace_count,
            ..
        } => dump_disc(
            stores,
            pre,
            post,
            *physical_replace_count,
            config,
            depth,
            out,
        ),
        Node::Mark { class, tokens } => dump_mark_projected(stores, *class, tokens, out),
        Node::Adjust(adjust) => {
            out.push_str(if adjust.pre {
                "\\vadjust pre\n"
            } else {
                "\\vadjust\n"
            });
            dump_projected_list(
                stores,
                &adjust.content,
                config,
                depth + 1,
                ListContext::VList,
                out,
            );
        }
        Node::MathOn(width) => {
            dump_math_marker(stores, "mathon", *width, out);
        }
        Node::MathOff(width) => {
            dump_math_marker(stores, "mathoff", *width, out);
        }
        Node::Direction(direction) => {
            let name = match direction {
                tex_state::node::Direction::BeginM => "beginM",
                tex_state::node::Direction::EndM => "endM",
                tex_state::node::Direction::BeginL => "beginL",
                tex_state::node::Direction::EndL => "endL",
                tex_state::node::Direction::BeginR => "beginR",
                tex_state::node::Direction::EndR => "endR",
            };
            let _ = writeln!(out, "\\{name}");
        }
        Node::MathNoad(noad) => dump_math_noad(stores, noad, config, depth, out),
        Node::FractionNoad(fraction) => dump_fraction(stores, fraction, config, depth, out),
        Node::MathStyle(style) => {
            let _ = writeln!(out, "\\{}", math_style_name(*style));
        }
        Node::MathChoice(choice) => dump_math_choice(stores, choice, config, depth, out),
        Node::MathList(list) => dump_math_list(stores, list, config, depth, out),
        Node::Nonscript => out.push_str("\\glue(\\nonscript)\n"),
        Node::Whatsit(whatsit) => dump_whatsit(stores, whatsit, out),
        Node::Ins {
            class,
            size,
            split_top_skip,
            split_max_depth,
            floating_penalty,
            content,
        } => {
            let _ = writeln!(
                out,
                "\\insert{class}, natural size {}; split({},{}); float cost {floating_penalty}",
                format_scaled_without_unit(*size),
                format_glue(split_top_skip.resolve(stores), ""),
                format_scaled_without_unit(*split_max_depth),
            );
            dump_projected_list(stores, content, config, depth + 1, ListContext::VList, out);
        }
    }
}

/// TeX82 §1356's `Display the whatsit node` cases. The PDF variants are
/// extension-owned and retain the generic marker until their own diagnostic
/// vocabulary is specified.
fn dump_whatsit<G, Glue: DumpGlueProjection<G>, Tokens: DumpTokensProjection<G>>(
    stores: &CommandContext<'_, G>,
    whatsit: &Whatsit<Glue, Tokens>,
    out: &mut String,
) {
    match whatsit {
        Whatsit::OpenOut { slot, path } => {
            append_escaped_name(stores, "openout", out);
            let _ = writeln!(out, "{}={path}", slot.raw());
        }
        Whatsit::CloseOut { slot } => match slot {
            Some(slot) => {
                append_escaped_name(stores, "closeout", out);
                let _ = writeln!(out, "{}", slot.raw());
            }
            None => {
                append_escaped_name(stores, "closeout", out);
                out.push_str("*\n");
            }
        },
        Whatsit::DeferredWrite { sink, tokens } => {
            append_escaped_name(stores, "write", out);
            match sink {
                tex_state::PrintSink::Stream(slot) => {
                    let _ = write!(out, "{}", slot.raw());
                }
                tex_state::PrintSink::TerminalAndLog | tex_state::PrintSink::Terminal => {
                    out.push('*');
                }
                tex_state::PrintSink::Log => out.push('-'),
            }
            dump_token_projection(stores, tokens, out);
        }
        Whatsit::Special { payload, .. } => {
            append_escaped_name(stores, "special", out);
            out.push('{');
            for &byte in payload {
                append_tex_print_char(char::from(byte), out);
            }
            out.push_str("}\n");
        }
        Whatsit::Language {
            language,
            left_hyphen_min,
            right_hyphen_min,
        } => {
            append_escaped_name(stores, "setlanguage", out);
            let _ = writeln!(
                out,
                "{language} (hyphenmin {left_hyphen_min},{right_hyphen_min})"
            );
        }
        _ => out.push_str("[]\n"),
    }
}

/// TeX82 §§63/1356 names each whatsit through the live `print_esc` rule.
fn append_escaped_name<G>(stores: &CommandContext<'_, G>, name: &str, out: &mut String) {
    if let Ok(escape) = u8::try_from(stores.int_param(IntParam::ESCAPE_CHAR)) {
        out.push(char::from(escape));
    }
    out.push_str(name);
}

fn dump_token_projection<G, Tokens: DumpTokensProjection<G>>(
    stores: &CommandContext<'_, G>,
    tokens: &Tokens,
    out: &mut String,
) {
    out.push('{');
    tokens.visit(stores, |token| {
        append_token_show_text(stores, token.semantic_token(), out);
    });
    out.push_str("}\n");
}

fn dump_math_noad<G, List: DumpListProjection<G, Storage>, Storage>(
    stores: &CommandContext<'_, G>,
    noad: &MathNoad<List>,
    config: &DumpConfig,
    depth: i32,
    out: &mut String,
) {
    match &noad.kind {
        NoadKind::Radical { delimiter } => {
            append_escaped_name(stores, "radical", out);
            dump_delimiter(*delimiter, out);
        }
        NoadKind::Accent { accent } => {
            append_escaped_name(stores, "accent", out);
            dump_math_char_inline(stores, *accent, out);
        }
        NoadKind::LeftDelimiter { delimiter } => {
            append_escaped_name(stores, "left", out);
            dump_delimiter(*delimiter, out);
        }
        NoadKind::RightDelimiter { delimiter } => {
            append_escaped_name(stores, "right", out);
            dump_delimiter(*delimiter, out);
        }
        NoadKind::MiddleDelimiter { delimiter } => {
            append_escaped_name(stores, "middle", out);
            dump_delimiter(*delimiter, out);
        }
        _ => append_escaped_name(stores, noad_name(&noad.kind), out),
    }
    match &noad.kind {
        NoadKind::Operator(LimitType::Limits) => append_escaped_name(stores, "limits", out),
        NoadKind::Operator(LimitType::NoLimits) => append_escaped_name(stores, "nolimits", out),
        _ => {}
    }
    // TeX82 §692's `print_subsidiary_data` keeps nonempty math fields
    // observable when the depth threshold prevents their contents from being
    // shown. The marker belongs on the noad's line and empty fields contribute
    // nothing.
    if depth + 1 >= config.depth {
        for field in [&noad.nucleus, &noad.superscript, &noad.subscript] {
            if !matches!(field, MathField::Empty) {
                out.push_str(" []");
            }
        }
        out.push('\n');
        return;
    }
    out.push('\n');
    dump_math_field(stores, &noad.nucleus, config, depth + 1, '.', out);
    dump_math_field(stores, &noad.superscript, config, depth + 1, '^', out);
    dump_math_field(stores, &noad.subscript, config, depth + 1, '_', out);
}

fn dump_math_marker<G>(
    stores: &CommandContext<'_, G>,
    name: &str,
    width: Scaled,
    out: &mut String,
) {
    // TeX82 §§63/184: both `before` and `after` math-node subtypes name
    // themselves through `print_esc`, so the header observes the live
    // `\escapechar` even when the node is nested in subsidiary math data.
    append_escaped_name(stores, name, out);
    if width.raw() == 0 {
        out.push('\n');
    } else {
        let _ = writeln!(out, ", surrounded {}", format_scaled_without_unit(width));
    }
}

fn dump_leader_payload<G, List: DumpListProjection<G, Storage> + Clone, Storage>(
    stores: &CommandContext<'_, G>,
    payload: &LeaderPayload<List>,
    config: &DumpConfig,
    depth: i32,
    context: ListContext,
    out: &mut String,
) {
    write_prefix(depth, out);
    match payload {
        LeaderPayload::HList(box_node) => {
            dump_box("hbox", stores, box_node, config, depth, context, out);
        }
        LeaderPayload::VList(box_node) => {
            dump_box("vbox", stores, box_node, config, depth, context, out);
        }
        LeaderPayload::Rule {
            width,
            height,
            depth,
        } => dump_rule(stores, *width, *height, *depth, out),
    }
}

/// TeX82 §191's `Display rule` begins with `print_esc("rule(")`, so both
/// list rules and rules used as leader payloads observe the live escape byte.
fn dump_rule<G>(
    stores: &CommandContext<'_, G>,
    width: Option<Scaled>,
    height: Option<Scaled>,
    depth: Option<Scaled>,
    out: &mut String,
) {
    append_escaped_name(stores, "rule(", out);
    let _ = writeln!(
        out,
        "{}+{})x{}",
        format_rule_dimension(height),
        format_rule_dimension(depth),
        format_rule_dimension(width)
    );
}

fn noad_name(kind: &NoadKind) -> &'static str {
    match kind {
        NoadKind::Normal(NoadClass::Ord) => "mathord",
        NoadKind::Normal(NoadClass::Op) | NoadKind::Operator(_) => "mathop",
        NoadKind::Normal(NoadClass::Bin) => "mathbin",
        NoadKind::Normal(NoadClass::Rel) => "mathrel",
        NoadKind::Normal(NoadClass::Open) => "mathopen",
        NoadKind::Normal(NoadClass::Close) => "mathclose",
        NoadKind::Normal(NoadClass::Punct) => "mathpunct",
        NoadKind::Normal(NoadClass::Inner) => "mathinner",
        NoadKind::Radical { .. } => "radical",
        NoadKind::Accent { .. } => "accent",
        NoadKind::LeftDelimiter { .. } => "left",
        NoadKind::RightDelimiter { .. } => "right",
        NoadKind::MiddleDelimiter { .. } => "middle",
        NoadKind::Underline => "underline",
        NoadKind::Overline => "overline",
        NoadKind::VCenter => "vcenter",
    }
}

fn dump_math_field<G, List: DumpListProjection<G, Storage>, Storage>(
    stores: &CommandContext<'_, G>,
    field: &MathField<List>,
    config: &DumpConfig,
    depth: i32,
    marker: char,
    out: &mut String,
) {
    match field {
        MathField::Empty => {}
        MathField::MathChar(ch) | MathField::MathTextChar(ch) => {
            write_prefix(depth - 1, out);
            out.push(marker);
            dump_math_char(stores, *ch, out);
        }
        MathField::SubBox(list) => {
            let old_len = out.len();
            dump_projected_list(stores, list, config, depth, ListContext::Neutral, out);
            mark_subsidiary_lines(out, old_len, depth, marker);
        }
        MathField::SubMlist(list) => {
            let old_len = out.len();
            dump_projected_list(stores, list, config, depth, ListContext::Neutral, out);
            if old_len < out.len() {
                mark_subsidiary_lines(out, old_len, depth, marker);
            } else {
                // TeX82 §681 represents an empty sub-mlist by a present field
                // whose info pointer is null. Section 692's subsidiary-data
                // printer emits the field marker followed by `{}`; only an
                // `empty` math_type is silent.
                write_prefix(depth - 1, out);
                let _ = writeln!(out, "{marker}{{}}");
            }
        }
    }
}

/// TeX82 §692 calls `print_subsidiary_data` before recursively entering
/// §182's `show_node_list`. The field marker consequently remains at that
/// subsidiary level on every line of the recursive node dump; dots before it
/// belong to enclosing lists, and dots after it belong to descendants.
fn mark_subsidiary_lines(out: &mut String, start: usize, depth: i32, marker: char) {
    let prefix_level = depth.max(0) as usize;
    let mut line_start = start;
    while line_start < out.len() {
        let marker_index = line_start + prefix_level;
        if out.as_bytes().get(marker_index) == Some(&b'.') {
            out.replace_range(marker_index..marker_index + 1, &marker.to_string());
        }
        let Some(newline) = out[line_start..].find('\n') else {
            break;
        };
        line_start += newline + 1;
    }
}

fn dump_math_char<G>(stores: &CommandContext<'_, G>, ch: MathChar, out: &mut String) {
    dump_math_char_inline(stores, ch, out);
    out.push('\n');
}

/// TeX82 §691's `print_fam_and_char` names `fam` through §63 `print_esc`,
/// so math-character diagnostics observe the current `\escapechar`.
fn dump_math_char_inline<G>(stores: &CommandContext<'_, G>, ch: MathChar, out: &mut String) {
    append_escaped_name(stores, "fam", out);
    let _ = write!(out, "{} {}", ch.family, dump_char(ch.character));
}

fn dump_fraction<G, List: DumpListProjection<G, Storage>, Storage>(
    stores: &CommandContext<'_, G>,
    fraction: &MathFraction<List>,
    config: &DumpConfig,
    depth: i32,
    out: &mut String,
) {
    dump_fraction_header(
        stores,
        fraction.thickness,
        fraction.left_delimiter,
        fraction.right_delimiter,
        out,
    );
    dump_fraction_part(stores, &fraction.numerator, config, depth + 1, "\\", out);
    dump_fraction_part(stores, &fraction.denominator, config, depth + 1, "/", out);
}

fn dump_fraction_header<G>(
    stores: &CommandContext<'_, G>,
    thickness: FractionThickness,
    left_delimiter: Option<u32>,
    right_delimiter: Option<u32>,
    out: &mut String,
) {
    // TeX82 §697 passes the complete fraction heading through `print_esc`,
    // so it observes the current `\escapechar` just like every noad name.
    append_escaped_name(stores, "fraction, thickness", out);
    match thickness {
        FractionThickness::Default => out.push_str(" = default"),
        FractionThickness::Explicit(value) => {
            let _ = write!(out, " {}", format_scaled_without_unit(value));
        }
    }
    // TeX82 §§696--697 inspect and print the four delimiter quarters,
    // not the complete 27-bit scanner value. In particular, the math-class
    // bits above `small_fam` neither make a delimiter non-null nor appear in
    // its 24-bit diagnostic value.
    if let Some(left) = left_delimiter
        .map(delimiter_field)
        .filter(|field| *field != 0)
    {
        dump_packed_delimiter(", left-delimiter ", left, out);
    }
    if let Some(right) = right_delimiter
        .map(delimiter_field)
        .filter(|field| *field != 0)
    {
        dump_packed_delimiter(", right-delimiter ", right, out);
    }
    out.push('\n');
}

/// Reconstruct TeX82 §696's 24-bit delimiter diagnostic from its four
/// quarter fields: small family/character, then large family/character.
fn delimiter_field(delimiter: u32) -> u32 {
    let small_family = (delimiter >> 20) & 0xf;
    let small_character = (delimiter >> 12) & 0xff;
    let large_family = (delimiter >> 8) & 0xf;
    let large_character = delimiter & 0xff;

    (((small_family << 8) | small_character) << 12) | (large_family << 8) | large_character
}

/// Print a noad whose diagnostic payload is TeX82 §696's packed delimiter
/// field. Unlike §697's optional fraction delimiters, a noad remains visible
/// when all four quarters are zero.
fn dump_delimiter(delimiter: u32, out: &mut String) {
    let delimiter = delimiter_field(delimiter);
    dump_packed_delimiter("", delimiter, out);
}

fn dump_packed_delimiter(prefix: &str, delimiter: u32, out: &mut String) {
    let _ = write!(out, "{prefix}\"{delimiter:X}");
}

fn dump_fraction_part<G, List: DumpListProjection<G, Storage>, Storage>(
    stores: &CommandContext<'_, G>,
    list: &List,
    config: &DumpConfig,
    depth: i32,
    marker: &str,
    out: &mut String,
) {
    let old_len = out.len();
    dump_projected_list(stores, list, config, depth, ListContext::Neutral, out);
    if old_len == out.len() {
        // TeX82 §697 passes the numerator and denominator records to §692's
        // subsidiary-data printer as `sub_mlist` fields. A null list is
        // therefore a present empty mlist (`{}`), not an absent math field.
        write_prefix(depth - 1, out);
        let _ = writeln!(out, "{marker}{{}}");
        return;
    }
    let mut line_start = old_len;
    while line_start < out.len() {
        let marker_index = line_start + depth.max(0) as usize;
        if out.as_bytes().get(marker_index) == Some(&b'.') {
            out.replace_range(marker_index..marker_index + 1, marker);
        }
        let Some(newline) = out[line_start..].find('\n') else {
            break;
        };
        line_start += newline + 1;
    }
}

fn dump_math_choice<G, List: DumpListProjection<G, Storage>, Storage>(
    stores: &CommandContext<'_, G>,
    choice: &MathChoice<List>,
    config: &DumpConfig,
    depth: i32,
    out: &mut String,
) {
    // TeX82 §689's choice-node display calls §63 `print_esc`.
    append_escaped_name(stores, "mathchoice", out);
    out.push('\n');
    dump_choice_arm(stores, &choice.display, config, depth + 1, 'D', out);
    dump_choice_arm(stores, &choice.text, config, depth + 1, 'T', out);
    dump_choice_arm(stores, &choice.script, config, depth + 1, 'S', out);
    dump_choice_arm(stores, &choice.script_script, config, depth + 1, 's', out);
}

fn dump_choice_arm<G, List: DumpListProjection<G, Storage>, Storage>(
    stores: &CommandContext<'_, G>,
    list: &List,
    config: &DumpConfig,
    depth: i32,
    marker: char,
    out: &mut String,
) {
    let old_len = out.len();
    dump_projected_list(stores, list, config, depth, ListContext::Neutral, out);
    if old_len < out.len() {
        // Section 689 appends the arm marker to `cur_string` for the entire
        // recursive `show_node_list` call, so it replaces this prefix level
        // on every line in the arm, not only the first node's header.
        mark_subsidiary_lines(out, old_len, depth, marker);
    }
}

fn dump_math_list<G, List: DumpListProjection<G, Storage>, Storage>(
    stores: &CommandContext<'_, G>,
    list: &MathListNode<List>,
    config: &DumpConfig,
    depth: i32,
    out: &mut String,
) {
    let name = if list.display {
        "\\displaymath"
    } else {
        "\\math"
    };
    out.push_str(name);
    out.push('\n');
    dump_projected_list(
        stores,
        &list.content,
        config,
        depth + 1,
        ListContext::Neutral,
        out,
    );
}

fn math_style_name(style: MathStyle) -> &'static str {
    match style {
        MathStyle::Display => "displaystyle",
        MathStyle::Text => "textstyle",
        MathStyle::Script => "scriptstyle",
        MathStyle::ScriptScript => "scriptscriptstyle",
    }
}

fn dump_disc<G, List: DumpListProjection<G, Storage>, Storage>(
    stores: &CommandContext<'_, G>,
    pre: &List,
    post: &List,
    physical_replace_count: u8,
    config: &DumpConfig,
    depth: i32,
    out: &mut String,
) {
    // TeX82 §187 names a discretionary through §63's `print_esc`, so the
    // node header observes the live `\escapechar`.
    append_escaped_name(stores, "discretionary", out);
    if physical_replace_count == 0 {
        out.push('\n');
    } else {
        let _ = writeln!(out, " replacing {physical_replace_count}");
    }
    dump_projected_list(stores, pre, config, depth + 1, ListContext::Neutral, out);
    if !post.is_empty(stores) {
        let old_len = out.len();
        dump_projected_list(stores, post, config, depth + 1, ListContext::Neutral, out);
        let marker_offset = usize::try_from(depth.max(-1) + 1).unwrap_or(0);
        let mut line_start = old_len;
        while line_start < out.len() {
            let marker = line_start + marker_offset;
            out.replace_range(marker..marker + 1, "|");
            let Some(newline) = out[line_start..].find('\n') else {
                break;
            };
            line_start += newline + 1;
        }
    }
}

fn dump_mark_projected<G, Tokens: DumpTokensProjection<G>>(
    stores: &CommandContext<'_, G>,
    class: u16,
    tokens: &Tokens,
    out: &mut String,
) {
    if class == 0 {
        append_escaped_name(stores, "mark", out);
        out.push('{');
    } else {
        append_escaped_name(stores, "marks", out);
        let _ = write!(out, "{class}{{");
    }
    tokens.visit(stores, |token| {
        out.push_str(&token_text(stores, token.semantic_token()));
    });
    out.push_str("}\n");
}

/// TeX82 §267's `print_esc(font_id_text(f))`, the control sequence a font is
/// known by, with pdfTeX's optional expansion and file-name annotations.
pub(crate) fn font_identifier<G>(
    stores: &CommandContext<'_, G>,
    font: tex_state::ids::FontId,
) -> String {
    render_print_string(stores, &font_identifier_raw(stores, font))
}

/// Unrendered §267 font identifier for a caller whose enclosing diagnostic
/// will still send the completed message through `print`.
pub(crate) fn font_identifier_raw<G>(
    stores: &CommandContext<'_, G>,
    font: tex_state::ids::FontId,
) -> String {
    let recipe = stores.font_artifact_recipe(font);
    let (identifier_font, expansion_ratio) = match recipe.construction {
        tex_state::FontArtifactConstructionRecipe::Expanded {
            source_identity,
            ratio,
        } => (
            stores
                .font_id_for_source_identity(source_identity)
                .unwrap_or(font),
            Some(ratio),
        ),
        _ => (font, None),
    };
    let identifier = stores.font_identifier_symbol(identifier_font).map_or_else(
        || format!("\\{}", stores.font_name(font)),
        |symbol| tex_state::token_show::token_text(stores, Token::Cs(symbol)),
    );
    if !stores.pdf_font_configuration().traces_fonts() {
        expansion_ratio.map_or(identifier.clone(), |ratio| {
            format!("{identifier} ({}{ratio})", if ratio > 0 { "+" } else { "" })
        })
    } else {
        let mut result = format!("{identifier} ({})", recipe.name);
        if recipe.at_size != recipe.design_size {
            result.pop();
            let _ = write!(result, "@{}pt)", format_scaled_without_unit(recipe.at_size));
        }
        result
    }
}

/// Renders characters that TeX82 §§58--60 would send through `print` before
/// the surrounding node display becomes a completed `print_rendered` string.
fn render_print_string<G>(stores: &CommandContext<'_, G>, raw: &str) -> String {
    let newline_char = u32::try_from(stores.int_param(IntParam::NEWLINE_CHAR))
        .ok()
        .and_then(char::from_u32);
    let mut rendered = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if newline_char == Some(ch) {
            rendered.push('\n');
        } else {
            append_tex_print_char(ch, &mut rendered);
        }
    }
    rendered
}

/// TeX82 §68's `print_ASCII`, which renders an unprintable character in
/// `^^` form rather than emitting it raw.
pub(crate) fn printable_char(ch: char) -> String {
    dump_char(ch)
}

fn dump_char(ch: char) -> String {
    if ch as u32 <= u32::from(u8::MAX) {
        let mut rendered = String::new();
        append_tex_print_char(ch, &mut rendered);
        rendered
    } else {
        format!("\\char{}", ch as u32)
    }
}

/// TeX82 §§59/173--174 print node character codes as one-character strings.
/// The live new-line character is recognized before the string's otherwise
/// unprintable byte is expanded to its `^^` spelling.
fn dump_character_string<G>(stores: &CommandContext<'_, G>, ch: char) -> String {
    let newline_char = u32::try_from(stores.int_param(IntParam::NEWLINE_CHAR))
        .ok()
        .and_then(char::from_u32);
    if newline_char == Some(ch) {
        "\n".to_owned()
    } else {
        dump_char(ch)
    }
}

fn dump_ligature<G>(
    stores: &CommandContext<'_, G>,
    ch: char,
    orig: &[char],
    left_hit: bool,
    right_hit: bool,
) -> String {
    let mut rendered = dump_character_string(stores, ch);
    rendered.push_str(" (ligature ");
    if left_hit {
        rendered.push('|');
    }
    for &original in orig {
        rendered.push_str(&dump_character_string(stores, original));
    }
    if right_hit {
        rendered.push('|');
    }
    rendered.push(')');
    rendered
}

fn dump_box<G, List: DumpListProjection<G, Storage> + Clone, Storage>(
    name: &str,
    stores: &CommandContext<'_, G>,
    box_node: &BoxNode<List>,
    config: &DumpConfig,
    depth: i32,
    _context: ListContext,
    out: &mut String,
) {
    let (children, physical_replacement_spans) = box_node
        .diagnostic_children
        .as_ref()
        .map_or((&box_node.children, false), |children| (children, true));
    // TeX82 §183 names both list-node kinds through `print_esc`, so the
    // header follows the live `\escapechar` just like whatsit names below.
    append_escaped_name(stores, name, out);
    let _ = write!(
        out,
        "({}+{})x{}",
        format_scaled_without_unit(box_node.height),
        format_scaled_without_unit(box_node.depth),
        format_scaled_without_unit(box_node.width)
    );
    write_glue_set(box_node, out);
    if box_node.shift.raw() != 0 {
        let _ = write!(
            out,
            ", shifted {}",
            format_scaled_without_unit(box_node.shift)
        );
    }
    match box_node.box_lr {
        tex_state::node::BoxLr::Normal => {}
        tex_state::node::BoxLr::Reversed => out.push_str(", reversed"),
        // TeX82 §184 has no box subtype to print here. Merged e-TeX §53a
        // extends hlist subtypes with `dlist`, and its changed node dumper
        // identifies that subtype as `display`.
        tex_state::node::BoxLr::DList if config.profile.capabilities().supports_etex() => {
            out.push_str(", display");
        }
        tex_state::node::BoxLr::DList => {}
    }
    if depth + 1 >= config.depth {
        if !children.is_empty(stores) {
            out.push_str(" []");
        }
        out.push('\n');
        return;
    }
    out.push('\n');
    let child_context = if name == "hbox" {
        ListContext::HList
    } else {
        ListContext::VList
    };
    children.dump(
        stores,
        config,
        depth + 1,
        child_context,
        physical_replacement_spans,
        out,
    );
}

fn dump_unset<G, List: DumpListProjection<G, Storage>, Storage>(
    stores: &CommandContext<'_, G>,
    unset: &UnsetNode<List>,
    config: &DumpConfig,
    depth: i32,
    out: &mut String,
) {
    // TeX82 §§183--185 dispatch on the single `unset_node` type and always
    // print `\unsetbox`. Umber retains the former packing direction in
    // `kind` for measurement and recursive list context, not as a subtype.
    append_escaped_name(stores, "unsetbox", out);
    let _ = write!(
        out,
        "({}+{})x{}",
        format_scaled_without_unit(unset.height),
        format_scaled_without_unit(unset.depth),
        format_scaled_without_unit(unset.width)
    );
    // TeX82 §186 stores one less than the number of columns in the
    // quarterword field and omits the annotation only for a single column.
    if unset.span_count != 0 {
        let _ = write!(out, " ({} columns)", u32::from(unset.span_count) + 1);
    }
    if unset.stretch.raw() != 0 {
        let _ = write!(
            out,
            ", stretch {}{}",
            format_scaled_without_unit(unset.stretch),
            order_unit(unset.stretch_order)
        );
    }
    if unset.shrink.raw() != 0 {
        let _ = write!(
            out,
            ", shrink {}{}",
            format_scaled_without_unit(unset.shrink),
            order_unit(unset.shrink_order)
        );
    }
    if depth + 1 >= config.depth {
        if !unset.children.is_empty(stores) {
            out.push_str(" []");
        }
        out.push('\n');
        return;
    }
    out.push('\n');
    let child_context = match unset.kind {
        UnsetKind::HBox => ListContext::HList,
        UnsetKind::VBox => ListContext::VList,
    };
    dump_projected_list(
        stores,
        &unset.children,
        config,
        depth + 1,
        child_context,
        out,
    );
}

fn write_glue_set<List>(box_node: &BoxNode<List>, out: &mut String) {
    if box_node.glue_sign == Sign::Normal || box_node.glue_set.is_zero() {
        return;
    }
    let sign = match box_node.glue_sign {
        Sign::Normal => return,
        Sign::Stretching => "glue set",
        Sign::Shrinking => "glue set -",
    };
    let _ = write!(
        out,
        ", {sign} {}{}",
        format_glue_ratio(box_node.glue_set),
        order_unit(box_node.glue_order)
    );
}

fn write_prefix(depth: i32, out: &mut String) {
    for _ in 0..=depth.max(-1) {
        out.push('.');
    }
}

fn format_glue(spec: GlueSpec, unit: &str) -> String {
    let mut text = format_scaled_without_unit(spec.width);
    text.push_str(unit);
    if spec.stretch.raw() != 0 {
        text.push_str(" plus ");
        text.push_str(&format_scaled_without_unit(spec.stretch));
        text.push_str(&glue_component_unit(spec.stretch_order, unit));
    }
    if spec.shrink.raw() != 0 {
        text.push_str(" minus ");
        text.push_str(&format_scaled_without_unit(spec.shrink));
        text.push_str(&glue_component_unit(spec.shrink_order, unit));
    }
    text
}

fn format_rule_dimension(value: Option<Scaled>) -> String {
    value.map_or_else(|| "*".to_owned(), format_scaled_without_unit)
}

fn format_scaled_without_unit(value: Scaled) -> String {
    tex_state::scaled::print_scaled(value)
}

pub(crate) fn format_scaled_for_diagnostics(value: Scaled) -> String {
    format_scaled_without_unit(value)
}

/// tex.web §177's `print_spec(p, s)`, used by `\tracingassigns`/`show_eqtb`
/// glue-parameter display.
///
/// Unlike [`format_glue`] (used for node dumps, which supply one unit for an
/// entire list from outside), this prints `unit` ("pt" for ordinary glue,
/// "mu" for e-TeX's math-glue parameters) after every scaled component whose
/// order is normal, and that order's own suffix ("fil"/"fill"/"filll")
/// otherwise.
pub(crate) fn format_glue_with_unit(spec: GlueSpec, unit: &str) -> String {
    let mut text = format_scaled_without_unit(spec.width);
    text.push_str(unit);
    if spec.stretch.raw() != 0 {
        text.push_str(" plus ");
        text.push_str(&format_scaled_without_unit(spec.stretch));
        text.push_str(&glue_component_unit(spec.stretch_order, unit));
    }
    if spec.shrink.raw() != 0 {
        text.push_str(" minus ");
        text.push_str(&format_scaled_without_unit(spec.shrink));
        text.push_str(&glue_component_unit(spec.shrink_order, unit));
    }
    text
}

fn glue_component_unit(order: Order, unit: &str) -> String {
    match order {
        Order::Normal => unit.to_owned(),
        Order::Fil | Order::Fill | Order::Filll => order_unit(order).to_owned(),
    }
}

fn format_glue_ratio(value: GlueSetRatio) -> String {
    let numerator = i64::from(value.numerator()) * i64::from(Scaled::UNITY);
    let denominator = i64::from(value.denominator());
    let raw = if numerator >= 0 {
        (numerator + denominator / 2) / denominator
    } else {
        -((-numerator + denominator / 2) / denominator)
    };
    format_scaled_without_unit(Scaled::from_raw(i32::try_from(raw).unwrap_or(i32::MAX)))
}

fn order_unit(order: Order) -> &'static str {
    match order {
        Order::Normal => "",
        Order::Fil => "fil",
        Order::Fill => "fill",
        Order::Filll => "filll",
    }
}

#[cfg(test)]
mod tests;

trait GlueKindDump {
    fn append_glue_dump_prefix<G>(self, stores: &CommandContext<'_, G>, out: &mut String);
    fn append_leader_dump_prefix<G>(self, stores: &CommandContext<'_, G>, out: &mut String);
    fn parameter_name(self) -> Option<&'static str>;
    fn glue_unit(self) -> &'static str;
    fn prints_glue_spec(self) -> bool;
}

impl GlueKindDump for GlueKind {
    /// TeX82 §189 renders both `glue` and every non-normal subtype through
    /// `print_esc`, so each name observes the current `\escapechar`.
    fn append_glue_dump_prefix<G>(self, stores: &CommandContext<'_, G>, out: &mut String) {
        append_escaped_name(stores, "glue", out);
        if let Some(name) = self.parameter_name() {
            out.push('(');
            append_escaped_name(stores, name, out);
            out.push(')');
        }
        if self != Self::NonScript {
            out.push(' ');
        }
    }

    /// TeX82 §189's leader branch calls `print_esc("")` before printing the
    /// optional `c`/`x` and the common `leaders` suffix.
    fn append_leader_dump_prefix<G>(self, stores: &CommandContext<'_, G>, out: &mut String) {
        match self {
            Self::Leaders => append_escaped_name(stores, "leaders", out),
            Self::Cleaders => append_escaped_name(stores, "cleaders", out),
            Self::Xleaders => append_escaped_name(stores, "xleaders", out),
            _ => return self.append_glue_dump_prefix(stores, out),
        }
        out.push(' ');
    }

    fn parameter_name(self) -> Option<&'static str> {
        match self {
            Self::Normal | Self::Leaders | Self::Cleaders | Self::Xleaders => None,
            Self::SpaceSkip => Some("spaceskip"),
            Self::XSpaceSkip => Some("xspaceskip"),
            Self::TabSkip => Some("tabskip"),
            Self::BaselineSkip => Some("baselineskip"),
            Self::LineSkip => Some("lineskip"),
            Self::TopSkip => Some("topskip"),
            Self::SplitTopSkip => Some("splittopskip"),
            Self::LeftSkip => Some("leftskip"),
            Self::RightSkip => Some("rightskip"),
            Self::ParSkip => Some("parskip"),
            Self::ParFillSkip => Some("parfillskip"),
            Self::AboveDisplaySkip => Some("abovedisplayskip"),
            Self::BelowDisplaySkip => Some("belowdisplayskip"),
            Self::AboveDisplayShortSkip => Some("abovedisplayshortskip"),
            Self::BelowDisplayShortSkip => Some("belowdisplayshortskip"),
            Self::MuSkip => Some("mskip"),
            Self::ThinMuSkip => Some("thinmuskip"),
            Self::MedMuSkip => Some("medmuskip"),
            Self::ThickMuSkip => Some("thickmuskip"),
            Self::NonScript => Some("nonscript"),
        }
    }

    fn glue_unit(self) -> &'static str {
        match self {
            // TeX82 §189's `Display glue p` distinguishes explicit `mu_glue`
            // subtype from named muglue parameters: only `\mskip` reaches
            // `print_spec(glue_ptr(p), "mu")`; the parameter subtypes reach
            // `print_spec(glue_ptr(p), 0)`.
            Self::MuSkip => "mu",
            _ => "",
        }
    }

    fn prints_glue_spec(self) -> bool {
        // TeX82 §189's `cond_math_glue` branch prints the `\nonscript`
        // subtype label but deliberately skips both the separating space and
        // `print_spec`. The zero glue specification is only a sentinel here;
        // ordinary zero glue still prints as `0.0`.
        self != Self::NonScript
    }
}

#[cfg(test)]
mod unset_diagnostic_tests {
    use super::*;
    use tex_state::node::UnsetNodeFields;

    fn with_context<R>(
        test: impl for<'id> FnOnce(&mut CommandContext<'_, tex_state::GenerationBrand<'id>>) -> R,
    ) -> R {
        crate::test_harness::with_nonstop_tex82_universe(|universe| {
            crate::test_harness::with_admitted(universe, test)
        })
    }

    #[test]
    fn fresh_profile_prints_node_names_with_tex82_escape_character() {
        with_context(|context| {
            assert_eq!(context.int_param(IntParam::ESCAPE_CHAR), i32::from(b'\\'));
            assert_eq!(
                dump_node_slice(
                    context,
                    &[Node::Penalty(50)],
                    DumpConfig {
                        breadth: 5,
                        depth: 0,
                        profile: CommandProfile::TEX82,
                    },
                ),
                "\\penalty 50\n"
            );
        });
    }

    fn set_escape<G>(context: &mut CommandContext<'_, G>, value: i32) {
        context
            .assign_int_param(
                IntParam::ESCAPE_CHAR,
                value,
                tex_state::AssignmentScope::Global,
            )
            .expect("escape character assignment");
    }

    #[test]
    fn vertical_unset_node_uses_tex82_unsetbox_name() {
        with_context(|context| {
            let children = PageListId::empty();
            let unset = Node::Unset(UnsetNode::new(UnsetNodeFields {
                kind: UnsetKind::VBox,
                width: Scaled::from_raw(0),
                height: Scaled::from_raw(0),
                depth: Scaled::from_raw(0),
                span_count: 0,
                stretch: Scaled::from_raw(0),
                stretch_order: Order::Normal,
                shrink: Scaled::from_raw(0),
                shrink_order: Order::Normal,
                children,
            }));

            assert_eq!(
                dump_node_slice(
                    context,
                    &[unset],
                    DumpConfig {
                        breadth: 5,
                        depth: 0,
                        profile: CommandProfile::TEX82,
                    },
                ),
                "\\unsetbox(0.0+0.0)x0.0\n"
            );
        });
    }

    #[test]
    fn named_glue_node_uses_live_escape_character_for_both_names() {
        with_context(|context| {
            set_escape(context, i32::from(b'|'));
            let spec = GlueSpec::ZERO;

            assert_eq!(
                dump_node_slice(
                    context,
                    &[Node::Glue {
                        spec,
                        kind: GlueKind::LineSkip,
                        leader: None,
                    }],
                    DumpConfig {
                        breadth: 5,
                        depth: 0,
                        profile: CommandProfile::TEX82,
                    },
                ),
                "|glue(|lineskip) 0.0\n"
            );
        });
    }

    #[test]
    fn kern_headers_use_the_live_escape_character() {
        // TeX82 §§63/184: `show_node_list` routes both kern names through
        // `print_esc`; a negative `\escapechar` suppresses the prefix entirely.
        with_context(|context| {
            let nodes = [
                Node::Kern {
                    amount: Scaled::from_raw(2 * Scaled::UNITY),
                    kind: KernKind::Font,
                },
                Node::Kern {
                    amount: Scaled::from_raw(3 * Scaled::UNITY),
                    kind: KernKind::Mu,
                },
            ];
            let config = DumpConfig {
                breadth: 5,
                depth: 0,
                profile: CommandProfile::TEX82,
            };

            set_escape(context, i32::from(b'|'));
            assert_eq!(
                dump_node_slice(context, &nodes, config),
                "|kern2.0\n|mkern3.0mu\n"
            );
            set_escape(context, -1);
            assert_eq!(
                dump_node_slice(context, &nodes, config),
                "kern2.0\nmkern3.0mu\n"
            );
        });
    }

    #[test]
    fn rule_headers_use_the_live_escape_character() {
        // TeX82 §191 begins a displayed rule with `print_esc("rule(")`;
        // leader rules use the same node display after their glue header.
        with_context(|context| {
            set_escape(context, i32::from(b'|'));
            let glue = GlueSpec::ZERO;
            let nodes = [
                Node::Rule {
                    width: Some(Scaled::from_raw(3 * Scaled::UNITY)),
                    height: Some(Scaled::from_raw(2 * Scaled::UNITY)),
                    depth: Some(Scaled::from_raw(Scaled::UNITY)),
                },
                Node::Glue {
                    spec: glue,
                    kind: GlueKind::Leaders,
                    leader: Some(LeaderPayload::Rule {
                        width: Some(Scaled::from_raw(6 * Scaled::UNITY)),
                        height: Some(Scaled::from_raw(5 * Scaled::UNITY)),
                        depth: Some(Scaled::from_raw(4 * Scaled::UNITY)),
                    }),
                },
            ];

            assert_eq!(
                dump_node_slice(
                    context,
                    &nodes,
                    DumpConfig {
                        breadth: 5,
                        depth: 5,
                        profile: CommandProfile::TEX82,
                    },
                ),
                "|rule(2.0+1.0)x3.0\n|leaders 0.0\n.|rule(5.0+4.0)x6.0\n"
            );
        });
    }

    #[test]
    fn penalty_headers_use_the_live_escape_character() {
        // TeX82 §§63/184: the `penalty_node` arm routes its name through
        // `print_esc`, including suppression for a negative `\escapechar`.
        with_context(|context| {
            let nodes = [Node::Penalty(10_000)];
            let config = DumpConfig {
                breadth: 5,
                depth: 0,
                profile: CommandProfile::TEX82,
            };

            set_escape(context, i32::from(b'|'));
            assert_eq!(dump_node_slice(context, &nodes, config), "|penalty 10000\n");

            set_escape(context, -1);
            assert_eq!(
                dump_node_slice(
                    context,
                    &nodes,
                    DumpConfig {
                        breadth: 5,
                        depth: 0,
                        profile: CommandProfile::TEX82,
                    },
                ),
                "penalty 10000\n"
            );
        });
    }

    #[test]
    fn mark_headers_use_the_live_escape_character() {
        // TeX82 §§63/200: the mark-node arm routes its name through
        // `print_esc`, including suppression for a negative `\escapechar`.
        with_context(|context| {
            let tokens =
                tex_state::node::NodeTokenList::new(vec![tex_state::token::TokenWord::pack(
                    Token::Char {
                        ch: 'x',
                        cat: tex_state::token::Catcode::Letter,
                    },
                )]);
            let nodes = [Node::Mark { class: 0, tokens }];
            let config = DumpConfig {
                breadth: 5,
                depth: 0,
                profile: CommandProfile::TEX82,
            };

            set_escape(context, i32::from(b'|'));
            assert_eq!(dump_node_slice(context, &nodes, config), "|mark{x}\n");

            set_escape(context, -1);
            assert_eq!(
                dump_node_slice(
                    context,
                    &nodes,
                    DumpConfig {
                        breadth: 5,
                        depth: 0,
                        profile: CommandProfile::TEX82,
                    },
                ),
                "mark{x}\n"
            );
        });
    }

    #[test]
    fn math_node_headers_use_the_live_escape_character() {
        // TeX82 §§63/184 route both math-node subtype names through
        // `print_esc`, including suppression for a negative `\escapechar`.
        with_context(|context| {
            let nodes = [
                Node::MathOn(Scaled::from_raw(0)),
                Node::MathOff(Scaled::from_raw(3 * Scaled::UNITY)),
            ];
            let config = DumpConfig {
                breadth: 5,
                depth: 0,
                profile: CommandProfile::TEX82,
            };

            set_escape(context, i32::from(b'|'));
            assert_eq!(
                dump_node_slice(context, &nodes, config),
                "|mathon\n|mathoff, surrounded 3.0\n"
            );
            set_escape(context, -1);
            assert_eq!(
                dump_node_slice(context, &nodes, config),
                "mathon\nmathoff, surrounded 3.0\n"
            );
        });
    }

    /// TeX82 §§63/696 routes both the noad name and its limit suffix through
    /// `print_esc`, so each observes the live `\escapechar` independently.
    #[test]
    fn math_noad_names_use_live_escape_character() {
        with_context(|context| {
            set_escape(context, i32::from(b'|'));
            let list = context.publish_page_nodes(vec![Node::MathNoad(MathNoad::new(
                NoadKind::Operator(LimitType::Limits),
                MathField::Empty,
            ))]);

            assert_eq!(
                dump_page_list(
                    context,
                    list,
                    DumpConfig {
                        breadth: 5,
                        depth: 5,
                        profile: CommandProfile::TEX82,
                    },
                ),
                "|mathop|limits\n"
            );
        });
    }

    /// TeX82 §§63/689/691 route the choice and math-character family names
    /// independently through the live `print_esc` projection.
    #[test]
    fn math_choice_and_family_names_use_live_escape_character() {
        with_context(|context| {
            set_escape(context, i32::from(b'|'));
            let arm = context.publish_page_nodes(vec![Node::MathNoad(MathNoad::new(
                NoadKind::Normal(NoadClass::Ord),
                MathField::MathChar(MathChar {
                    family: 1,
                    character: 'a',
                    origin: Default::default(),
                }),
            ))]);
            let list = context.publish_page_nodes(vec![Node::MathChoice(MathChoice {
                display: arm,
                text: arm,
                script: arm,
                script_script: arm,
            })]);

            assert_eq!(
                dump_page_list(
                    context,
                    list,
                    DumpConfig {
                        breadth: 100,
                        depth: 100,
                        profile: CommandProfile::TEX82,
                    },
                ),
                concat!(
                    "|mathchoice\n",
                    "D|mathord\nD.|fam1 a\n",
                    "T|mathord\nT.|fam1 a\n",
                    "S|mathord\nS.|fam1 a\n",
                    "s|mathord\ns.|fam1 a\n",
                ),
            );
        });
    }

    /// TeX82 §§63/697 route the complete fraction heading through the live
    /// `print_esc` projection, including when it is nested in a choice arm.
    #[test]
    fn fraction_dump_uses_live_escape_character_inside_choice_arm() {
        with_context(|context| {
            set_escape(context, i32::from(b'|'));
            let empty = PageListId::empty();
            let fraction = context.publish_page_nodes(vec![Node::FractionNoad(MathFraction {
                numerator: empty,
                denominator: empty,
                thickness: FractionThickness::Default,
                left_delimiter: None,
                right_delimiter: None,
            })]);
            let list = context.publish_page_nodes(vec![Node::MathChoice(MathChoice {
                display: empty,
                text: empty,
                script: fraction,
                script_script: empty,
            })]);

            assert_eq!(
                dump_page_list(
                    context,
                    list,
                    DumpConfig {
                        breadth: 100,
                        depth: 100,
                        profile: CommandProfile::TEX82,
                    },
                ),
                "|mathchoice\nS|fraction, thickness = default\nS\\{}\nS/{}\n",
            );
        });
    }
}
