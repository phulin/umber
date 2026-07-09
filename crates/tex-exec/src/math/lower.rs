use tex_state::Universe;
use tex_state::env::banks::DimenParam;
use tex_state::glue::Order;
use tex_state::math::MathListNode;
use tex_state::node::{BoxNode, BoxNodeFields, GlueKind, Node, Sign};
use tex_state::scaled::GlueSetRatio;
use tex_typeset::math::{
    FrozenHList, MathBox, MathGlueKind, MathNode, MathParams, Style, mlist_to_hlist,
};

pub(crate) fn finish_math_list_node(
    stores: &mut Universe,
    list: MathListNode,
    insert_penalties: bool,
) -> Vec<Node> {
    let params = MathParams::read(stores);
    let style = if list.display {
        Style::DISPLAY
    } else {
        Style::TEXT
    };
    let hlist = mlist_to_hlist(
        stores,
        list.content,
        style,
        insert_penalties && !list.display,
        &params,
    );
    let mut nodes = Vec::new();
    if !list.display {
        // AppG rule 22
        let surround = stores.dimen_param(DimenParam::MATH_SURROUND);
        nodes.push(Node::MathOn(surround));
    }
    nodes.extend(lower_math_hlist(stores, &hlist));
    if !list.display {
        // AppG rule 22
        let surround = stores.dimen_param(DimenParam::MATH_SURROUND);
        nodes.push(Node::MathOff(surround));
    }
    nodes
}

pub(crate) fn finish_math_lists(
    stores: &mut Universe,
    nodes: &[Node],
    insert_penalties: bool,
) -> Vec<Node> {
    let mut out = Vec::with_capacity(nodes.len());
    for node in nodes {
        match node {
            Node::MathList(list) => {
                out.extend(finish_math_list_node(stores, *list, insert_penalties))
            }
            node => out.push(node.clone()),
        }
    }
    out
}

pub(super) fn lower_math_hlist(stores: &mut Universe, hlist: &FrozenHList) -> Vec<Node> {
    hlist
        .nodes
        .iter()
        .map(|node| lower_math_node(stores, node))
        .collect()
}

fn lower_math_node(stores: &mut Universe, node: &MathNode) -> Node {
    match node {
        MathNode::Char { font, ch, .. } => Node::Char {
            font: *font,
            ch: *ch,
        },
        MathNode::Kern { amount, kind } => Node::Kern {
            amount: *amount,
            kind: *kind,
        },
        MathNode::Glue { spec, kind } => Node::Glue {
            spec: stores.intern_glue(*spec),
            kind: lower_math_glue_kind(*kind),
            leader: None,
        },
        MathNode::Penalty(penalty) => Node::Penalty(*penalty),
        MathNode::Rule {
            width,
            height,
            depth,
        } => Node::Rule {
            width: *width,
            height: *height,
            depth: *depth,
        },
        MathNode::HList(boxed) => Node::HList(lower_math_box(stores, boxed)),
        MathNode::VList(boxed) => Node::VList(lower_math_box(stores, boxed)),
        MathNode::Opaque(node) => node.clone(),
    }
}

fn lower_math_box(stores: &mut Universe, boxed: &MathBox) -> BoxNode {
    let lowered = lower_math_hlist(stores, &boxed.list);
    let children = stores.freeze_node_list(&lowered);
    BoxNode::new(BoxNodeFields {
        width: boxed.width,
        height: boxed.height,
        depth: boxed.depth,
        shift: boxed.shift,
        display: false,
        glue_set: GlueSetRatio::from_raw(0),
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children,
    })
}

fn lower_math_glue_kind(kind: MathGlueKind) -> GlueKind {
    match kind {
        MathGlueKind::NonScript => GlueKind::NonScript,
        MathGlueKind::MuSkip => GlueKind::MuSkip,
        MathGlueKind::ThinMuSkip => GlueKind::ThinMuSkip,
        MathGlueKind::MedMuSkip => GlueKind::MedMuSkip,
        MathGlueKind::ThickMuSkip => GlueKind::ThickMuSkip,
        MathGlueKind::Normal | MathGlueKind::Source => GlueKind::Normal,
    }
}
