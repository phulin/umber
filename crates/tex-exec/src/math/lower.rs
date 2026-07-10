use tex_state::Universe;
use tex_state::env::banks::DimenParam;
use tex_state::glue::{GlueSpec, Order};
use tex_state::ids::GlueId;
use tex_state::math::MathListNode;
use tex_state::node::{BoxNode, BoxNodeFields, GlueKind, Node, Sign};
use tex_state::scaled::GlueSetRatio;
use tex_typeset::math::{
    FrozenHList, MathBox, MathGlueKind, MathLayout, MathNode, MathParams, Style, mlist_to_hlist,
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
    nodes.extend(lower_math_hlist(stores, hlist));
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

pub(super) fn lower_math_hlist(stores: &mut Universe, layout: MathLayout) -> Vec<Node> {
    enum Task<'a> {
        Span(FrozenHList),
        Node(&'a MathNode),
        BeginBox,
        EndBox(&'a MathBox, bool),
    }

    let root = layout.root();
    let mut tasks = Vec::with_capacity(32);
    tasks.push(Task::Span(root));
    let mut frames = Vec::with_capacity(8);
    let mut scratch = Vec::with_capacity(root.node_count());
    let mut glue_cache = Vec::<(GlueSpec, GlueId)>::with_capacity(8);
    while let Some(task) = tasks.pop() {
        match task {
            Task::Span(span) => tasks.extend(layout.nodes(span).iter().rev().map(Task::Node)),
            Task::Node(MathNode::Sequence(child)) => tasks.push(Task::Span(*child)),
            Task::Node(MathNode::HList(boxed)) => {
                tasks.push(Task::EndBox(boxed, false));
                tasks.push(Task::Span(boxed.list));
                tasks.push(Task::BeginBox);
            }
            Task::Node(MathNode::VList(boxed)) => {
                tasks.push(Task::EndBox(boxed, true));
                tasks.push(Task::Span(boxed.list));
                tasks.push(Task::BeginBox);
            }
            Task::Node(MathNode::Char { font, ch, .. }) => scratch.push(Node::Char {
                font: *font,
                ch: *ch,
            }),
            Task::Node(MathNode::Kern { amount, kind }) => scratch.push(Node::Kern {
                amount: *amount,
                kind: *kind,
            }),
            Task::Node(MathNode::Glue { spec, kind }) => {
                let id = if let Some((_, id)) = glue_cache.iter().find(|(cached, _)| cached == spec)
                {
                    *id
                } else {
                    let id = stores.intern_glue(*spec);
                    glue_cache.push((*spec, id));
                    id
                };
                scratch.push(Node::Glue {
                    spec: id,
                    kind: lower_math_glue_kind(*kind),
                    leader: None,
                });
            }
            Task::Node(MathNode::Penalty(penalty)) => scratch.push(Node::Penalty(*penalty)),
            Task::Node(MathNode::Rule {
                width,
                height,
                depth,
            }) => scratch.push(Node::Rule {
                width: *width,
                height: *height,
                depth: *depth,
            }),
            Task::Node(MathNode::Opaque(node)) => scratch.push(node.as_ref().clone()),
            Task::BeginBox => frames.push(scratch.len()),
            Task::EndBox(boxed, vertical) => {
                let start = frames.pop().expect("math box frame must be active");
                let children = stores.freeze_node_list(&scratch[start..]);
                scratch.truncate(start);
                let boxed = lower_math_box(boxed, children);
                scratch.push(if vertical {
                    Node::VList(boxed)
                } else {
                    Node::HList(boxed)
                });
            }
        }
    }
    assert!(
        frames.is_empty(),
        "math lowering left unfinished box frames"
    );
    scratch
}

fn lower_math_box(boxed: &MathBox, children: tex_state::ids::NodeListId) -> BoxNode {
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
