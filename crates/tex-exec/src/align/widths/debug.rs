use tex_state::Universe;
#[cfg(debug_assertions)]
use tex_state::math::MathField;
use tex_state::node::Node;

#[cfg(debug_assertions)]
pub(super) fn debug_assert_no_unset_nodes(_stores: &Universe, nodes: &[Node]) {
    for node in nodes {
        debug_assert_no_unset_node(node);
    }
}

#[cfg(not(debug_assertions))]
pub(super) fn debug_assert_no_unset_nodes(_stores: &Universe, _nodes: &[Node]) {}

#[cfg(debug_assertions)]
fn debug_assert_no_unset_node(node: &Node) {
    match node {
        Node::Unset(_) => panic!("unset node escaped fin_align"),
        Node::HList(box_node) | Node::VList(box_node) => {
            debug_assert_no_unset_nodes_in(&box_node.children)
        }
        Node::Disc {
            pre, post, replace, ..
        } => {
            debug_assert_no_unset_nodes_in(pre);
            debug_assert_no_unset_nodes_in(post);
            debug_assert_no_unset_nodes_in(replace);
        }
        Node::Ins { content, .. } => debug_assert_no_unset_nodes_in(content),
        Node::Adjust(adjust) => debug_assert_no_unset_nodes_in(&adjust.content),
        Node::MathNoad(noad) => {
            debug_assert_math_field(&noad.nucleus);
            debug_assert_math_field(&noad.subscript);
            debug_assert_math_field(&noad.superscript);
        }
        Node::FractionNoad(fraction) => {
            debug_assert_no_unset_nodes_in(&fraction.numerator);
            debug_assert_no_unset_nodes_in(&fraction.denominator);
        }
        Node::MathChoice(choice) => {
            debug_assert_no_unset_nodes_in(&choice.display);
            debug_assert_no_unset_nodes_in(&choice.text);
            debug_assert_no_unset_nodes_in(&choice.script);
            debug_assert_no_unset_nodes_in(&choice.script_script);
        }
        Node::MathList(list) => debug_assert_no_unset_nodes_in(&list.content),
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
fn debug_assert_no_unset_nodes_in(list: &tex_state::node_arena::NodeListRef) {
    for node in list.to_vec() {
        debug_assert_no_unset_node(&node);
    }
}

#[cfg(debug_assertions)]
fn debug_assert_math_field(field: &MathField) {
    match field {
        MathField::SubBox(list) | MathField::SubMlist(list) => debug_assert_no_unset_nodes_in(list),
        MathField::Empty | MathField::MathChar(_) | MathField::MathTextChar(_) => {}
    }
}
