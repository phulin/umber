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
        Node::Kern { amount, kind } => {
            if *kind != KernKind::Explicit {
                out.push_str(kind.kern_dump_prefix());
            }
            let _ = writeln!(out, "\\kern {}", format_scaled_without_unit(*amount));
        }
        Node::Glue { spec, kind } => {
            if *kind != GlueKind::Normal {
                out.push_str(kind.glue_dump_prefix());
            }
            let _ = writeln!(out, "\\glue {}", format_glue(stores.glue(*spec)));
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
        Node::Char { ch, .. } => {
            let _ = writeln!(out, "\\char{}", *ch as u32);
        }
        Node::Lig { ch, .. } => {
            let _ = writeln!(out, "\\ligature {ch}");
        }
        Node::MathOn => out.push_str("\\mathon\n"),
        Node::MathOff => out.push_str("\\mathoff\n"),
        Node::Unset
        | Node::Disc { .. }
        | Node::Mark { .. }
        | Node::Ins { .. }
        | Node::Whatsit(_)
        | Node::Adjust(_) => out.push_str("[]\n"),
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

trait KernKindDump {
    fn kern_dump_prefix(self) -> &'static str;
}

impl KernKindDump for KernKind {
    fn kern_dump_prefix(self) -> &'static str {
        match self {
            Self::Explicit => "",
            Self::Font => "\\kern (font) ",
            Self::Accent => "\\kern (for accent) ",
        }
    }
}

trait GlueKindDump {
    fn glue_dump_prefix(self) -> &'static str;
}

impl GlueKindDump for GlueKind {
    fn glue_dump_prefix(self) -> &'static str {
        match self {
            Self::Normal => "",
            Self::Leaders => "\\leaders ",
            Self::Cleaders => "\\cleaders ",
            Self::Xleaders => "\\xleaders ",
        }
    }
}
