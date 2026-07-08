//! Reusable TeX node-list diagnostic dumping.

use std::fmt::Write as _;

use tex_state::Universe;
use tex_state::env::banks::IntParam;
use tex_state::glue::{GlueSpec, Order};
use tex_state::ids::NodeListId;
use tex_state::node::{BoxNode, GlueKind, KernKind, Node, Sign};
use tex_state::scaled::Scaled;

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
    dump_list(stores, id, &config, -1, &mut out);
    out
}

fn dump_list(stores: &Universe, id: NodeListId, config: &DumpConfig, depth: i32, out: &mut String) {
    if config.depth < 0 || depth > config.depth {
        return;
    }
    let nodes = stores.nodes(id);
    let limit = config.breadth.max(0) as usize;
    for node in nodes.iter().take(limit) {
        dump_node(stores, node, config, depth, out);
    }
    if nodes.len() > limit {
        write_prefix(depth, out);
        out.push_str("etc.\n");
    }
}

fn dump_node(stores: &Universe, node: &Node, config: &DumpConfig, depth: i32, out: &mut String) {
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
        },
        Node::Glue { spec, kind } => {
            let _ = writeln!(
                out,
                "{}{}",
                kind.glue_dump_prefix(),
                format_glue(stores.glue(*spec))
            );
        }
        Node::HList(box_node) => {
            dump_box("hbox", stores, box_node, config, depth, out);
        }
        Node::VList(box_node) => {
            dump_box("vbox", stores, box_node, config, depth, out);
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
        Node::Disc { pre, post, replace } => {
            dump_disc(stores, *pre, *post, *replace, config, depth, out)
        }
        Node::MathOn => out.push_str("\\mathon\n"),
        Node::MathOff => out.push_str("\\mathoff\n"),
        Node::Unset | Node::Mark { .. } | Node::Ins { .. } | Node::Whatsit(_) | Node::Adjust(_) => {
            out.push_str("[]\n")
        }
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
    dump_list(stores, pre, config, depth + 1, out);
    if !stores.nodes(post).is_empty() {
        let old_len = out.len();
        dump_list(stores, post, config, depth + 1, out);
        if old_len + 1 < out.len() {
            out.replace_range(old_len + 1..old_len + 2, "|");
        }
    }
    dump_list(stores, replace, config, depth, out);
}

fn dump_font(stores: &Universe, font: tex_state::ids::FontId) -> String {
    if stores.current_font() == font
        && let Some(symbol) = stores.current_font_symbol()
    {
        return format!("\\{}", stores.resolve(symbol));
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
    if depth + 1 >= config.depth {
        if !stores.nodes(box_node.children).is_empty() {
            out.push_str(" []");
        }
        out.push('\n');
        return;
    }
    out.push('\n');
    dump_list(stores, box_node.children, config, depth + 1, out);
}

fn write_glue_set(box_node: &BoxNode, out: &mut String) {
    if box_node.glue_sign == Sign::Normal || box_node.glue_set == 0.0 {
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
    let raw = value.raw();
    let negative = raw < 0;
    let magnitude = if negative {
        i64::from(raw).wrapping_neg()
    } else {
        i64::from(raw)
    };
    let unity = i64::from(Scaled::UNITY);
    let mut integer = magnitude / unity;
    let fraction = magnitude % unity;
    let mut decimal = ((fraction * 100_000) + (unity / 2)) / unity;
    if decimal == 100_000 {
        integer += 1;
        decimal = 0;
    }
    let mut fraction_text = format!("{decimal:05}");
    while fraction_text.len() > 1 && fraction_text.ends_with('0') {
        fraction_text.pop();
    }
    let sign = if negative { "-" } else { "" };
    format!("{sign}{integer}.{fraction_text}")
}

fn format_glue_ratio(value: f64) -> String {
    let mut text = format!("{:.5}", value.abs());
    while text.matches('.').count() == 1 && text.ends_with('0') && !text.ends_with(".0") {
        text.pop();
    }
    text
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
}

impl GlueKindDump for GlueKind {
    fn glue_dump_prefix(self) -> &'static str {
        match self {
            Self::Normal => "\\glue ",
            Self::BaselineSkip => "\\glue(\\baselineskip) ",
            Self::LineSkip => "\\glue(\\lineskip) ",
            Self::Leaders => "\\leaders \\glue ",
            Self::Cleaders => "\\cleaders \\glue ",
            Self::Xleaders => "\\xleaders \\glue ",
        }
    }
}
