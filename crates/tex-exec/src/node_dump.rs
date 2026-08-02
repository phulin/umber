//! Reusable TeX node-list diagnostic dumping.

use std::fmt::Write as _;

use tex_expand::{append_token_show_text, token_text};
use tex_state::Universe;
use tex_state::env::banks::IntParam;
use tex_state::glue::{GlueSpec, Order};
use tex_state::ids::{NodeListId, TokenListId};
use tex_state::math::{
    FractionThickness, LimitType, MathChar, MathChoice, MathField, MathFraction, MathListNode,
    MathNoad, MathStyle, NoadClass, NoadKind,
};
use tex_state::node::{
    BoxNode, GlueKind, KernKind, LeaderPayload, Node, Sign, UnsetKind, UnsetNode, Whatsit,
};
use tex_state::scaled::{GlueSetRatio, Scaled};
use tex_state::token::Token;
use tex_state::token_show::append_tex_print_char;

pub(crate) struct DumpConfig {
    pub(crate) breadth: i32,
    pub(crate) depth: i32,
}

impl DumpConfig {
    /// TeX82 §198's `show_box`: `depth_threshold:=show_box_depth;
    /// breadth_max:=show_box_breadth` (§236), then `if breadth_max<=0 then
    /// breadth_max:=5`. INITEX leaves both `\showboxbreadth` and
    /// `\showboxdepth` at their default of 0 (tex.web §240's `int_par` table
    /// initialization), so every level's item count must fall back to 5
    /// rather than truncating to zero items and printing `etc.` immediately;
    /// `depth_threshold` has no such fallback and is used as read.
    pub(crate) fn read(stores: &Universe) -> Self {
        let breadth = stores.int_param(IntParam::SHOW_BOX_BREADTH);
        Self {
            breadth: if breadth <= 0 { 5 } else { breadth },
            depth: stores.int_param(IntParam::SHOW_BOX_DEPTH),
        }
    }
}

pub(crate) fn dump_node_list(stores: &Universe, id: NodeListId, config: DumpConfig) -> String {
    let mut out = String::new();
    dump_list(stores, id, &config, -1, ListContext::Neutral, &mut out);
    out
}

pub(crate) fn dump_node_slice(stores: &Universe, nodes: &[Node], config: DumpConfig) -> String {
    let mut out = String::new();
    dump_nodes(
        stores,
        nodes,
        &config,
        -1,
        ListContext::Neutral,
        false,
        &mut out,
    );
    out
}

pub(crate) fn dump_incomplete_fraction(
    stores: &Universe,
    fraction: &crate::mode::IncompleteFraction,
    config: DumpConfig,
) -> String {
    let mut out = String::new();
    dump_fraction_header(
        fraction.thickness,
        fraction.left_delimiter,
        fraction.right_delimiter,
        &mut out,
    );
    dump_fraction_part(stores, fraction.numerator, &config, 0, "\\", &mut out);
    out
}

#[derive(Clone, Copy)]
enum ListContext {
    Neutral,
    HList,
    VList,
}

fn dump_list(
    stores: &Universe,
    id: NodeListId,
    config: &DumpConfig,
    depth: i32,
    context: ListContext,
    out: &mut String,
) {
    let nodes = stores.nodes(id).to_vec();
    dump_nodes(stores, &nodes, config, depth, context, false, out);
}

fn dump_nodes(
    stores: &Universe,
    nodes: &[Node],
    config: &DumpConfig,
    depth: i32,
    context: ListContext,
    physical_replacement_spans: bool,
    out: &mut String,
) {
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
            index += 3;
            displayed += 1;
            continue;
        }

        let node = &nodes[index];
        index += 1;
        displayed += 1;
        dump_node(stores, node, config, depth, context, out);
        if physical_replacement_spans
            && let Node::Disc {
                physical_replace_count,
                ..
            } = node
        {
            let mut remaining = usize::from(*physical_replace_count);
            while remaining > 0 && index < nodes.len() {
                remaining = remaining.saturating_sub(diagnostic_physical_width(&nodes[index]));
                index += 1;
            }
        }
    }
    if index < nodes.len() {
        write_prefix(depth, out);
        out.push_str("etc.\n");
    }
}

fn diagnostic_physical_width(node: &Node) -> usize {
    match node {
        Node::Lig { orig, .. } => 1 + orig.len(),
        _ => 1,
    }
}

fn dump_node(
    stores: &Universe,
    node: &Node,
    config: &DumpConfig,
    depth: i32,
    context: ListContext,
    out: &mut String,
) {
    write_prefix(depth, out);
    match node {
        Node::Kern { amount, kind } => match kind {
            KernKind::Explicit => {
                let _ = writeln!(out, "\\kern {}", format_scaled_without_unit(*amount));
            }
            KernKind::Font => {
                let _ = writeln!(out, "\\kern{}", format_scaled_without_unit(*amount));
            }
            KernKind::Auto => {
                let _ = writeln!(
                    out,
                    "\\kern {} (for \\pdfprependkern/\\pdfappendkern)",
                    format_scaled_without_unit(*amount)
                );
            }
            KernKind::Accent => {
                let _ = writeln!(
                    out,
                    "\\kern {} (for accent)",
                    format_scaled_without_unit(*amount)
                );
            }
            KernKind::Mu => {
                let _ = writeln!(out, "\\mkern{}mu", format_scaled_without_unit(*amount));
            }
            KernKind::LeftMargin => {
                let _ = writeln!(
                    out,
                    "\\kern{} (left margin)",
                    format_scaled_without_unit(*amount)
                );
            }
            KernKind::RightMargin => {
                let _ = writeln!(
                    out,
                    "\\kern{} (right margin)",
                    format_scaled_without_unit(*amount)
                );
            }
        },
        Node::MarginKern { amount, side, .. } => {
            let side = match side {
                tex_state::node::MarginKernSide::Left => "left",
                tex_state::node::MarginKernSide::Right => "right",
            };
            let _ = writeln!(
                out,
                "\\kern{} ({side} margin)",
                format_scaled_without_unit(*amount)
            );
        }
        Node::Glue { spec, kind, leader } => {
            if let Some(leader) = leader {
                let _ = writeln!(
                    out,
                    "{}{}",
                    kind.leader_dump_prefix(),
                    format_glue(stores.glue(*spec), kind.glue_unit())
                );
                dump_leader_payload(stores, leader, config, depth + 1, context, out);
            } else {
                let _ = writeln!(
                    out,
                    "{}{}",
                    kind.glue_dump_prefix(),
                    format_glue(stores.glue(*spec), kind.glue_unit())
                );
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
        } => {
            let _ = writeln!(
                out,
                "\\rule({}+{})x{}",
                format_rule_dimension(*height),
                format_rule_dimension(*depth),
                format_rule_dimension(*width)
            );
        }
        Node::Penalty(value) => {
            let _ = writeln!(out, "\\penalty {value}");
        }
        Node::Char { font, ch, .. } => {
            let _ = writeln!(out, "{} {}", dump_font(stores, *font), dump_char(*ch));
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
                dump_font(stores, *font),
                dump_ligature(*ch, orig, *left_hit, *right_hit)
            );
        }
        Node::Disc {
            pre,
            post,
            physical_replace_count,
            ..
        } => dump_disc(
            stores,
            *pre,
            *post,
            *physical_replace_count,
            config,
            depth,
            out,
        ),
        Node::Mark { class, tokens } => dump_mark(stores, *class, *tokens, out),
        Node::Adjust(adjust) => {
            out.push_str(if adjust.pre {
                "\\vadjust pre\n"
            } else {
                "\\vadjust\n"
            });
            dump_list(
                stores,
                adjust.content,
                config,
                depth + 1,
                ListContext::VList,
                out,
            );
        }
        Node::MathOn(width) => {
            dump_math_marker("\\mathon", *width, out);
        }
        Node::MathOff(width) => {
            dump_math_marker("\\mathoff", *width, out);
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
                format_glue(stores.glue(*split_top_skip), ""),
                format_scaled_without_unit(*split_max_depth),
            );
            dump_list(stores, *content, config, depth + 1, ListContext::VList, out);
        }
    }
}

/// TeX82 §1356's `Display the whatsit node` cases. The PDF variants are
/// extension-owned and retain the generic marker until their own diagnostic
/// vocabulary is specified.
fn dump_whatsit(stores: &Universe, whatsit: &Whatsit, out: &mut String) {
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
            dump_token_list(stores, *tokens, out);
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
fn append_escaped_name(stores: &Universe, name: &str, out: &mut String) {
    if let Ok(escape) = u8::try_from(stores.int_param(IntParam::ESCAPE_CHAR)) {
        out.push(char::from(escape));
    }
    out.push_str(name);
}

fn dump_token_list(stores: &Universe, tokens: TokenListId, out: &mut String) {
    out.push('{');
    for &token in stores.tokens(tokens) {
        // §1356 delegates write-node contents to §262 `show_token_list`,
        // including `print_cs`'s control-word separator.
        append_token_show_text(stores, token, out);
    }
    out.push_str("}\n");
}

fn dump_math_noad(
    stores: &Universe,
    noad: &MathNoad,
    config: &DumpConfig,
    depth: i32,
    out: &mut String,
) {
    match &noad.kind {
        NoadKind::Radical { delimiter } => {
            let _ = write!(out, "\\radical\"{delimiter:X}");
        }
        NoadKind::Accent { accent } => {
            out.push_str("\\accent");
            dump_math_char_inline(*accent, out);
        }
        NoadKind::LeftDelimiter { delimiter } => {
            let _ = write!(out, "\\left\"{delimiter:X}");
        }
        NoadKind::RightDelimiter { delimiter } => {
            let _ = write!(out, "\\right\"{delimiter:X}");
        }
        NoadKind::MiddleDelimiter { delimiter } => {
            let _ = write!(out, "\\middle\"{delimiter:X}");
        }
        _ => out.push_str(noad_name(&noad.kind)),
    }
    match &noad.kind {
        NoadKind::Operator(LimitType::Limits) => out.push_str("\\limits"),
        NoadKind::Operator(LimitType::NoLimits) => out.push_str("\\nolimits"),
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

fn dump_math_marker(name: &str, width: Scaled, out: &mut String) {
    if width.raw() == 0 {
        let _ = writeln!(out, "{name}");
    } else {
        let _ = writeln!(
            out,
            "{name}, surrounded {}",
            format_scaled_without_unit(width)
        );
    }
}

fn dump_leader_payload(
    stores: &Universe,
    payload: &LeaderPayload,
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
        } => {
            let _ = writeln!(
                out,
                "\\rule({}+{})x{}",
                format_rule_dimension(*height),
                format_rule_dimension(*depth),
                format_rule_dimension(*width)
            );
        }
    }
}

fn noad_name(kind: &NoadKind) -> &'static str {
    match kind {
        NoadKind::Normal(NoadClass::Ord) => "\\mathord",
        NoadKind::Normal(NoadClass::Op) | NoadKind::Operator(_) => "\\mathop",
        NoadKind::Normal(NoadClass::Bin) => "\\mathbin",
        NoadKind::Normal(NoadClass::Rel) => "\\mathrel",
        NoadKind::Normal(NoadClass::Open) => "\\mathopen",
        NoadKind::Normal(NoadClass::Close) => "\\mathclose",
        NoadKind::Normal(NoadClass::Punct) => "\\mathpunct",
        NoadKind::Normal(NoadClass::Inner) => "\\mathinner",
        NoadKind::Radical { .. } => "\\radical",
        NoadKind::Accent { .. } => "\\accent",
        NoadKind::LeftDelimiter { .. } => "\\left",
        NoadKind::RightDelimiter { .. } => "\\right",
        NoadKind::MiddleDelimiter { .. } => "\\middle",
        NoadKind::Underline => "\\underline",
        NoadKind::Overline => "\\overline",
        NoadKind::VCenter => "\\vcenter",
    }
}

fn dump_math_field(
    stores: &Universe,
    field: &MathField,
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
            dump_math_char(*ch, out);
        }
        MathField::SubBox(list) => {
            let old_len = out.len();
            dump_list(stores, *list, config, depth, ListContext::Neutral, out);
            if old_len < out.len() {
                out.replace_range(old_len..old_len + 1, &marker.to_string());
            }
        }
        MathField::SubMlist(list) => {
            let old_len = out.len();
            dump_list(stores, *list, config, depth, ListContext::Neutral, out);
            if old_len < out.len() {
                out.replace_range(old_len..old_len + 1, &marker.to_string());
            } else {
                // TeX82 §681 represents an empty sub-mlist by a present field
                // whose info pointer is null. Section 692's subsidiary-data
                // printer still emits the field marker and newline; only an
                // `empty` math_type is silent.
                write_prefix(depth - 1, out);
                let _ = writeln!(out, "{marker}");
            }
        }
    }
}

fn dump_math_char(ch: MathChar, out: &mut String) {
    let _ = writeln!(out, "\\fam{} {}", ch.family, dump_char(ch.character));
}

fn dump_math_char_inline(ch: MathChar, out: &mut String) {
    let _ = write!(out, "\\fam{} {}", ch.family, dump_char(ch.character));
}

fn dump_fraction(
    stores: &Universe,
    fraction: &MathFraction,
    config: &DumpConfig,
    depth: i32,
    out: &mut String,
) {
    dump_fraction_header(
        fraction.thickness,
        fraction.left_delimiter,
        fraction.right_delimiter,
        out,
    );
    dump_fraction_part(stores, fraction.numerator, config, depth + 1, "\\", out);
    dump_fraction_part(stores, fraction.denominator, config, depth + 1, "/", out);
}

fn dump_fraction_header(
    thickness: FractionThickness,
    left_delimiter: Option<u32>,
    right_delimiter: Option<u32>,
    out: &mut String,
) {
    out.push_str("\\fraction, thickness");
    match thickness {
        FractionThickness::Default => out.push_str(" = default"),
        FractionThickness::Explicit(value) => {
            let _ = write!(out, " {}", format_scaled_without_unit(value));
        }
    }
    if let Some(left) = left_delimiter {
        let _ = write!(out, ", left-delimiter \"{left:X}");
    }
    if let Some(right) = right_delimiter {
        let _ = write!(out, ", right-delimiter \"{right:X}");
    }
    out.push('\n');
}

fn dump_fraction_part(
    stores: &Universe,
    list: NodeListId,
    config: &DumpConfig,
    depth: i32,
    marker: &str,
    out: &mut String,
) {
    let old_len = out.len();
    dump_list(stores, list, config, depth, ListContext::Neutral, out);
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

fn dump_math_choice(
    stores: &Universe,
    choice: &MathChoice,
    config: &DumpConfig,
    depth: i32,
    out: &mut String,
) {
    out.push_str("\\mathchoice\n");
    dump_choice_arm(stores, choice.display, config, depth + 1, 'D', out);
    dump_choice_arm(stores, choice.text, config, depth + 1, 'T', out);
    dump_choice_arm(stores, choice.script, config, depth + 1, 'S', out);
    dump_choice_arm(stores, choice.script_script, config, depth + 1, 's', out);
}

fn dump_choice_arm(
    stores: &Universe,
    list: NodeListId,
    config: &DumpConfig,
    depth: i32,
    marker: char,
    out: &mut String,
) {
    let old_len = out.len();
    dump_list(stores, list, config, depth, ListContext::Neutral, out);
    if old_len < out.len() {
        out.replace_range(old_len..old_len + 1, &marker.to_string());
    }
}

fn dump_math_list(
    stores: &Universe,
    list: &MathListNode,
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
    dump_list(
        stores,
        list.content,
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

fn dump_disc(
    stores: &Universe,
    pre: NodeListId,
    post: NodeListId,
    physical_replace_count: u8,
    config: &DumpConfig,
    depth: i32,
    out: &mut String,
) {
    if physical_replace_count == 0 {
        out.push_str("\\discretionary\n");
    } else {
        let _ = writeln!(out, "\\discretionary replacing {}", physical_replace_count);
    }
    dump_list(stores, pre, config, depth + 1, ListContext::Neutral, out);
    if !stores.nodes(post).is_empty() {
        let old_len = out.len();
        dump_list(stores, post, config, depth + 1, ListContext::Neutral, out);
        if old_len < out.len() {
            out.replace_range(old_len..old_len + 1, "|");
        }
    }
}

fn dump_mark(stores: &Universe, class: u16, tokens: TokenListId, out: &mut String) {
    if class == 0 {
        out.push_str("\\mark{");
    } else {
        let _ = write!(out, "\\marks{class}{{");
    }
    for &token in stores.tokens(tokens) {
        out.push_str(&token_text(stores, token));
    }
    out.push_str("}\n");
}

/// TeX82 §267's `print_esc(font_id_text(f))`, the control sequence a font is
/// known by, with pdfTeX's optional expansion and file-name annotations.
pub(crate) fn font_identifier(stores: &Universe, font: tex_state::ids::FontId) -> String {
    dump_font(stores, font)
}

fn dump_font(stores: &Universe, font: tex_state::ids::FontId) -> String {
    let loaded = stores.font(font);
    let (identifier_font, expansion_ratio) = match loaded.construction() {
        tex_fonts::FontConstruction::Expanded { source, ratio } => (
            stores.font_by_source_identity(*source).unwrap_or(font),
            Some(*ratio),
        ),
        _ => (font, None),
    };
    let identifier = stores.font_identifier_symbol(identifier_font).map_or_else(
        || format!("\\{}", stores.font_name(font)),
        |symbol| tex_expand::token_text(stores, Token::Cs(symbol.symbol())),
    );
    if !stores.pdf_font_configuration().traces_fonts() {
        return expansion_ratio.map_or(identifier.clone(), |ratio| {
            format!("{identifier} ({}{ratio})", if ratio > 0 { "+" } else { "" })
        });
    }
    let mut result = format!("{identifier} ({})", loaded.name());
    if loaded.size() != loaded.design_size() {
        result.pop();
        let _ = write!(result, "@{}pt)", format_scaled_without_unit(loaded.size()));
    }
    result
}

/// TeX82 §68's `print_ASCII`, which renders an unprintable character in
/// `^^` form rather than emitting it raw.
pub(crate) fn printable_char(ch: char) -> String {
    dump_char(ch)
}

fn dump_char(ch: char) -> String {
    if ch.is_ascii_graphic() {
        ch.to_string()
    } else if (0..=31).contains(&(ch as u32)) {
        let marker = char::from_u32((ch as u32) + 64).expect("control marker is ASCII");
        format!("^^{marker}")
    } else {
        format!("\\char{}", ch as u32)
    }
}

fn dump_ligature(ch: char, orig: &[char], left_hit: bool, right_hit: bool) -> String {
    let mut rendered = dump_char(ch);
    rendered.push_str(" (ligature ");
    if left_hit {
        rendered.push('|');
    }
    for &original in orig {
        rendered.push_str(&dump_char(original));
    }
    if right_hit {
        rendered.push('|');
    }
    rendered.push(')');
    rendered
}

fn dump_box(
    name: &str,
    stores: &Universe,
    box_node: &BoxNode,
    config: &DumpConfig,
    depth: i32,
    _context: ListContext,
    out: &mut String,
) {
    let (children, physical_replacement_spans) = box_node
        .diagnostic_children
        .map_or((box_node.children, false), |children| (children, true));
    let _ = write!(
        out,
        "\\{}({}+{})x{}",
        name,
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
        tex_state::node::BoxLr::DList => out.push_str(", display"),
    }
    if depth + 1 >= config.depth {
        if !stores.nodes(children).is_empty() {
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
    let nodes = stores.nodes(children).to_vec();
    dump_nodes(
        stores,
        &nodes,
        config,
        depth + 1,
        child_context,
        physical_replacement_spans,
        out,
    );
}

fn dump_unset(
    stores: &Universe,
    unset: &UnsetNode,
    config: &DumpConfig,
    depth: i32,
    out: &mut String,
) {
    let name = match unset.kind {
        UnsetKind::HBox => "unsetbox",
        UnsetKind::VBox => "unsetvbox",
    };
    let _ = write!(
        out,
        "\\{}({}+{})x{}",
        name,
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
        if !stores.nodes(unset.children).is_empty() {
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
    dump_list(
        stores,
        unset.children,
        config,
        depth + 1,
        child_context,
        out,
    );
}

fn write_glue_set(box_node: &BoxNode, out: &mut String) {
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
    fn glue_dump_prefix(self) -> &'static str;
    fn leader_dump_prefix(self) -> &'static str;
    fn glue_unit(self) -> &'static str;
}

impl GlueKindDump for GlueKind {
    fn glue_dump_prefix(self) -> &'static str {
        match self {
            Self::Normal => "\\glue ",
            Self::SpaceSkip => "\\glue(\\spaceskip) ",
            Self::XSpaceSkip => "\\glue(\\xspaceskip) ",
            Self::TabSkip => "\\glue(\\tabskip) ",
            Self::BaselineSkip => "\\glue(\\baselineskip) ",
            Self::LineSkip => "\\glue(\\lineskip) ",
            Self::TopSkip => "\\glue(\\topskip) ",
            Self::SplitTopSkip => "\\glue(\\splittopskip) ",
            Self::LeftSkip => "\\glue(\\leftskip) ",
            Self::RightSkip => "\\glue(\\rightskip) ",
            Self::ParSkip => "\\glue(\\parskip) ",
            Self::ParFillSkip => "\\glue(\\parfillskip) ",
            Self::AboveDisplaySkip => "\\glue(\\abovedisplayskip) ",
            Self::BelowDisplaySkip => "\\glue(\\belowdisplayskip) ",
            Self::AboveDisplayShortSkip => "\\glue(\\abovedisplayshortskip) ",
            Self::BelowDisplayShortSkip => "\\glue(\\belowdisplayshortskip) ",
            Self::Leaders => "\\leaders \\glue ",
            Self::Cleaders => "\\cleaders \\glue ",
            Self::Xleaders => "\\xleaders \\glue ",
            Self::MuSkip => "\\glue(\\mskip) ",
            Self::ThinMuSkip => "\\glue(\\thinmuskip) ",
            Self::MedMuSkip => "\\glue(\\medmuskip) ",
            Self::ThickMuSkip => "\\glue(\\thickmuskip) ",
            Self::NonScript => "\\glue(\\nonscript) ",
        }
    }

    fn leader_dump_prefix(self) -> &'static str {
        match self {
            Self::Leaders => "\\leaders ",
            Self::Cleaders => "\\cleaders ",
            Self::Xleaders => "\\xleaders ",
            _ => self.glue_dump_prefix(),
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
}
