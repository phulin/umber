//! Reusable TeX node-list diagnostic dumping.

use std::fmt::Write as _;

use tex_expand::token_text;
use tex_state::Universe;
use tex_state::env::banks::IntParam;
use tex_state::glue::{GlueSpec, Order};
use tex_state::ids::{NodeListId, TokenListId};
use tex_state::math::{
    FractionThickness, LimitType, MathChar, MathChoice, MathField, MathFraction, MathListNode,
    MathNoad, MathStyle, NoadClass, NoadKind,
};
use tex_state::node::{
    BoxNode, GlueKind, KernKind, LeaderPayload, Node, Sign, UnsetKind, UnsetNode,
};
use tex_state::scaled::{GlueSetRatio, Scaled};
use tex_state::token::Token;

pub(crate) struct DumpConfig {
    pub(crate) breadth: i32,
    pub(crate) depth: i32,
}

impl DumpConfig {
    pub(crate) fn read(stores: &Universe) -> Self {
        Self {
            breadth: stores.int_param(IntParam::SHOW_BOX_BREADTH),
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
    dump_nodes(stores, nodes, &config, -1, ListContext::Neutral, &mut out);
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
    dump_nodes(stores, &nodes, config, depth, context, out);
}

fn dump_nodes(
    stores: &Universe,
    nodes: &[Node],
    config: &DumpConfig,
    depth: i32,
    context: ListContext,
    out: &mut String,
) {
    if config.depth < 0 || depth > config.depth {
        return;
    }
    let limit = config.breadth.max(0) as usize;
    for node in nodes.iter().take(limit) {
        dump_node(stores, node, config, depth, context, out);
    }
    if nodes.len() > limit {
        write_prefix(depth, out);
        out.push_str("etc.\n");
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
        },
        Node::Glue { spec, kind, leader } => {
            if let Some(leader) = leader {
                let _ = writeln!(
                    out,
                    "{}{}",
                    kind.leader_dump_prefix(),
                    format_glue(stores.glue(*spec))
                );
                dump_leader_payload(stores, leader, config, depth + 1, context, out);
            } else {
                let _ = writeln!(
                    out,
                    "{}{}",
                    kind.glue_dump_prefix(),
                    format_glue(stores.glue(*spec))
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
        Node::Char { font, ch } => {
            let _ = writeln!(out, "{} {}", dump_font(stores, *font), dump_char(*ch));
        }
        Node::Lig { font, ch, .. } => {
            let _ = writeln!(out, "{} {}", dump_font(stores, *font), dump_ligature(*ch));
        }
        Node::Disc {
            pre, post, replace, ..
        } => dump_disc(stores, *pre, *post, *replace, config, depth, out),
        Node::Mark { tokens, .. } => dump_mark(stores, *tokens, out),
        Node::Adjust(list) => {
            out.push_str("\\vadjust\n");
            dump_list(stores, *list, config, depth + 1, ListContext::VList, out);
        }
        Node::MathOn(width) => {
            dump_math_marker("\\mathon", *width, out);
        }
        Node::MathOff(width) => {
            dump_math_marker("\\mathoff", *width, out);
        }
        Node::MathNoad(noad) => dump_math_noad(stores, noad, config, depth, out),
        Node::FractionNoad(fraction) => dump_fraction(stores, fraction, config, depth, out),
        Node::MathStyle(style) => {
            let _ = writeln!(out, "\\{}", math_style_name(*style));
        }
        Node::MathChoice(choice) => dump_math_choice(stores, choice, config, depth, out),
        Node::MathList(list) => dump_math_list(stores, list, config, depth, out),
        Node::Nonscript => out.push_str("\\glue(\\nonscript)\n"),
        Node::Ins { .. } | Node::Whatsit(_) => out.push_str("[]\n"),
    }
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
        _ => out.push_str(noad_name(&noad.kind)),
    }
    match &noad.kind {
        NoadKind::Operator(LimitType::Limits) => out.push_str("\\limits"),
        NoadKind::Operator(LimitType::NoLimits) => out.push_str("\\nolimits"),
        _ => {}
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
        MathField::SubBox(list) | MathField::SubMlist(list) => {
            let old_len = out.len();
            dump_list(stores, *list, config, depth, ListContext::Neutral, out);
            if old_len < out.len() {
                out.replace_range(old_len..old_len + 1, &marker.to_string());
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
    out.push_str("\\fraction, thickness");
    match fraction.thickness {
        FractionThickness::Default => out.push_str(" = default"),
        FractionThickness::Explicit(value) => {
            let _ = write!(out, " {}", format_scaled_without_unit(value));
        }
    }
    if let Some(left) = fraction.left_delimiter {
        let _ = write!(out, ", left-delimiter \"{left:X}");
    }
    if let Some(right) = fraction.right_delimiter {
        let _ = write!(out, ", right-delimiter \"{right:X}");
    }
    out.push('\n');
    dump_fraction_part(stores, fraction.numerator, config, depth + 1, "\\", out);
    dump_fraction_part(stores, fraction.denominator, config, depth + 1, "/", out);
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
    if old_len < out.len() {
        out.replace_range(old_len..old_len + 1, marker);
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
    replace: NodeListId,
    config: &DumpConfig,
    depth: i32,
    out: &mut String,
) {
    if stores.nodes(replace).is_empty() {
        out.push_str("\\discretionary\n");
    } else {
        let _ = writeln!(
            out,
            "\\discretionary replacing {}",
            stores.nodes(replace).len()
        );
    }
    dump_list(stores, pre, config, depth + 1, ListContext::Neutral, out);
    if !stores.nodes(post).is_empty() {
        let old_len = out.len();
        dump_list(stores, post, config, depth + 1, ListContext::Neutral, out);
        if old_len + 1 < out.len() {
            out.replace_range(old_len + 1..old_len + 2, "|");
        }
    }
    dump_list(stores, replace, config, depth, ListContext::Neutral, out);
}

fn dump_mark(stores: &Universe, tokens: TokenListId, out: &mut String) {
    out.push_str("\\mark{");
    for &token in stores.tokens(tokens) {
        out.push_str(&token_text(stores, token));
    }
    out.push_str("}\n");
}

fn dump_font(stores: &Universe, font: tex_state::ids::FontId) -> String {
    if let Some(symbol) = stores.font_identifier_symbol(font) {
        return tex_expand::token_text(stores, Token::Cs(symbol.symbol()));
    }
    format!("\\{}", stores.font_name(font))
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

fn dump_ligature(ch: char) -> String {
    match ch as u32 {
        11 => "^^K (ligature ff)".to_owned(),
        12 => "^^L (ligature fi)".to_owned(),
        13 => "^^M (ligature fl)".to_owned(),
        14 => "^^N (ligature ffi)".to_owned(),
        15 => "^^O (ligature ffl)".to_owned(),
        _ => dump_char(ch),
    }
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
    if box_node.display {
        out.push_str(", display");
    }
    if depth + 1 >= config.depth {
        if !stores.nodes(box_node.children).is_empty() {
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
    dump_list(
        stores,
        box_node.children,
        config,
        depth + 1,
        child_context,
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
    if unset.span_count > 1 {
        let _ = write!(out, ", spans {}", unset.span_count);
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

fn format_glue(spec: GlueSpec) -> String {
    let mut text = format_scaled_without_unit(spec.width);
    if spec.stretch.raw() != 0 {
        text.push_str(" plus ");
        text.push_str(&format_scaled_without_unit(spec.stretch));
        text.push_str(order_unit(spec.stretch_order));
    }
    if spec.shrink.raw() != 0 {
        text.push_str(" minus ");
        text.push_str(&format_scaled_without_unit(spec.shrink));
        text.push_str(order_unit(spec.shrink_order));
    }
    text
}

fn format_rule_dimension(value: Option<Scaled>) -> String {
    value.map_or_else(|| "*".to_owned(), format_scaled_without_unit)
}

fn format_scaled_without_unit(value: Scaled) -> String {
    let mut raw = i64::from(value.raw());
    let mut out = String::new();
    if raw < 0 {
        out.push('-');
        raw = -raw;
    }
    let unity = i64::from(Scaled::UNITY);
    out.push_str(&(raw / unity).to_string());
    out.push('.');
    let mut scaled = 10 * (raw % unity) + 5;
    let mut delta = 10;
    loop {
        if delta > unity {
            scaled += 0o100000 - 50_000;
        }
        out.push(char::from(
            b'0' + u8::try_from(scaled / unity).expect("scaled digit fits u8"),
        ));
        scaled = 10 * (scaled % unity);
        delta *= 10;
        if scaled <= delta {
            break;
        }
    }
    out
}

pub(crate) fn format_scaled_for_diagnostics(value: Scaled) -> String {
    format_scaled_without_unit(value)
}

fn format_glue_ratio(value: GlueSetRatio) -> String {
    let numerator = i64::from(value.numerator()) * i64::from(Scaled::UNITY);
    let denominator = i64::from(value.denominator());
    let raw = (numerator + denominator / 2) / denominator;
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

trait GlueKindDump {
    fn glue_dump_prefix(self) -> &'static str;
    fn leader_dump_prefix(self) -> &'static str;
}

impl GlueKindDump for GlueKind {
    fn glue_dump_prefix(self) -> &'static str {
        match self {
            Self::Normal => "\\glue ",
            Self::TabSkip => "\\glue(\\tabskip) ",
            Self::BaselineSkip => "\\glue(\\baselineskip) ",
            Self::LineSkip => "\\glue(\\lineskip) ",
            Self::TopSkip => "\\glue(\\topskip) ",
            Self::SplitTopSkip => "\\glue(\\splittopskip) ",
            Self::LeftSkip => "\\glue(\\leftskip) ",
            Self::RightSkip => "\\glue(\\rightskip) ",
            Self::ParFillSkip => "\\glue(\\parfillskip) ",
            Self::AboveDisplaySkip => "\\glue(\\abovedisplayskip) ",
            Self::BelowDisplaySkip => "\\glue(\\belowdisplayskip) ",
            Self::AboveDisplayShortSkip => "\\glue(\\abovedisplayshortskip) ",
            Self::BelowDisplayShortSkip => "\\glue(\\belowdisplayshortskip) ",
            Self::Leaders => "\\leaders \\glue ",
            Self::Cleaders => "\\cleaders \\glue ",
            Self::Xleaders => "\\xleaders \\glue ",
            Self::MuSkip => "\\glue ",
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
}
