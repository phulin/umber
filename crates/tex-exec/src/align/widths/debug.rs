use tex_state::Universe;
#[cfg(debug_assertions)]
use tex_state::ids::NodeListId;
use tex_state::node::Node;

#[cfg(debug_assertions)]
pub(super) fn debug_assert_no_unset_nodes(stores: &Universe, nodes: &[Node]) {
    let mut stack = Vec::new();
    for node in nodes {
        debug_assert_no_unset_node(node, &mut stack);
    }
    while let Some(list) = stack.pop() {
        for node in stores.nodes(list) {
            debug_assert_no_unset_node(&node.to_owned(), &mut stack);
        }
    }
}

#[cfg(not(debug_assertions))]
pub(super) fn debug_assert_no_unset_nodes(_stores: &Universe, _nodes: &[Node]) {}

#[cfg(debug_assertions)]
fn debug_assert_no_unset_node(node: &Node, stack: &mut Vec<NodeListId>) {
    match node {
        Node::Unset(_) => panic!("unset node escaped fin_align"),
        Node::HList(box_node) | Node::VList(box_node) => stack.push(box_node.children),
        Node::Disc {
            pre, post, replace, ..
        } => {
            stack.push(*pre);
            stack.push(*post);
            stack.push(*replace);
        }
        Node::Ins { content, .. } | Node::Adjust(content) => stack.push(*content),
        Node::MathNoad(noad) => {
            debug_assert_math_field(&noad.nucleus, stack);
            debug_assert_math_field(&noad.subscript, stack);
            debug_assert_math_field(&noad.superscript, stack);
        }
        Node::FractionNoad(fraction) => {
            stack.push(fraction.numerator);
            stack.push(fraction.denominator);
        }
        Node::MathChoice(choice) => {
            stack.push(choice.display);
            stack.push(choice.text);
            stack.push(choice.script);
            stack.push(choice.script_script);
        }
        Node::MathList(list) => stack.push(list.content),
        Node::Char { .. }
        | Node::Lig { .. }
        | Node::Kern { .. }
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
fn debug_assert_math_field(field: &tex_state::math::MathField, stack: &mut Vec<NodeListId>) {
    match field {
        tex_state::math::MathField::SubBox(list) | tex_state::math::MathField::SubMlist(list) => {
            stack.push(*list)
        }
        tex_state::math::MathField::Empty
        | tex_state::math::MathField::MathChar(_)
        | tex_state::math::MathField::MathTextChar(_) => {}
    }
}
