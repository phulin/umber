use tex_state::CommandContext;
#[cfg(debug_assertions)]
use tex_state::math::MathField;
use tex_state::node::Node;
use tex_state::node_arena::PageListId;

#[cfg(debug_assertions)]
pub(super) fn debug_assert_no_unset_nodes<G>(stores: &CommandContext<'_, G>, nodes: PageListId) {
    for node in stores
        .page_node_list(nodes)
        .expect("finished alignment belongs to the live page arena")
        .iter()
    {
        debug_assert_no_unset_node(stores, node);
    }
}

#[cfg(not(debug_assertions))]
pub(super) fn debug_assert_no_unset_nodes<G>(_stores: &CommandContext<'_, G>, _nodes: PageListId) {}

#[cfg(debug_assertions)]
fn debug_assert_no_unset_node<G>(stores: &CommandContext<'_, G>, node: &Node) {
    match node {
        Node::Unset(_) => panic!("unset node escaped fin_align"),
        Node::HList(box_node) | Node::VList(box_node) => {
            debug_assert_no_unset_nodes_in(stores, box_node.children)
        }
        Node::Disc {
            pre, post, replace, ..
        } => {
            debug_assert_no_unset_nodes_in(stores, *pre);
            debug_assert_no_unset_nodes_in(stores, *post);
            debug_assert_no_unset_nodes_in(stores, *replace);
        }
        Node::Ins { content, .. } => debug_assert_no_unset_nodes_in(stores, *content),
        Node::Adjust(adjust) => debug_assert_no_unset_nodes_in(stores, adjust.content),
        Node::MathNoad(noad) => {
            debug_assert_math_field(stores, &noad.nucleus);
            debug_assert_math_field(stores, &noad.subscript);
            debug_assert_math_field(stores, &noad.superscript);
        }
        Node::FractionNoad(fraction) => {
            debug_assert_no_unset_nodes_in(stores, fraction.numerator);
            debug_assert_no_unset_nodes_in(stores, fraction.denominator);
        }
        Node::MathChoice(choice) => {
            debug_assert_no_unset_nodes_in(stores, choice.display);
            debug_assert_no_unset_nodes_in(stores, choice.text);
            debug_assert_no_unset_nodes_in(stores, choice.script);
            debug_assert_no_unset_nodes_in(stores, choice.script_script);
        }
        Node::MathList(list) => debug_assert_no_unset_nodes_in(stores, list.content),
        Node::Char { .. }
        | Node::Lig { .. }
        | Node::Kern { .. }
        | Node::MarginKern { .. }
        | Node::Glue { .. }
        | Node::Penalty(_)
        | Node::Rule { .. }
        | Node::Mark { .. }
        | Node::Whatsit(_)
        | Node::MathOn(_)
        | Node::MathOff(_)
        | Node::Direction(_)
        | Node::MathStyle(_)
        | Node::Nonscript => {}
    }
}

#[cfg(debug_assertions)]
fn debug_assert_no_unset_nodes_in<G>(
    stores: &CommandContext<'_, G>,
    list: tex_state::node_arena::PageListId,
) {
    for node in stores
        .page_node_list(list)
        .expect("alignment child belongs to the live page arena")
        .nodes()
    {
        debug_assert_no_unset_node(stores, node);
    }
}

#[cfg(debug_assertions)]
fn debug_assert_math_field<G>(stores: &CommandContext<'_, G>, field: &MathField) {
    match field {
        MathField::SubBox(list) | MathField::SubMlist(list) => {
            debug_assert_no_unset_nodes_in(stores, *list)
        }
        MathField::Empty | MathField::MathChar(_) | MathField::MathTextChar(_) => {}
    }
}
