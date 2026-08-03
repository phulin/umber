use tex_state::Universe;
use tex_state::math::{LimitType, MathFraction, MathStyle, NoadClass, NoadKind};
use tex_state::meaning::UnexpandablePrimitive;
use tex_state::node::Node;

use crate::ModeNest;

pub(super) fn finish_current_math_list(
    nest: &mut ModeNest,
    stores: &mut Universe,
) -> tex_state::ids::NodeListId {
    let (nodes, incomplete) = {
        let mut list = nest.current_list_mutation();
        (list.take_nodes(), list.take_incomplete_fraction())
    };
    let nodes = if let Some(incomplete) = incomplete {
        let denominator = stores.freeze_node_list(&nodes);
        let mut numerator_nodes: Vec<_> = stores
            .nodes(incomplete.numerator)
            .into_iter()
            .map(|node| node.to_owned())
            .collect();
        let leading_left = matches!(numerator_nodes.first(), Some(Node::MathNoad(noad)) if matches!(noad.kind, NoadKind::LeftDelimiter { .. }))
            .then(|| numerator_nodes.remove(0));
        let numerator = if leading_left.is_some() {
            stores.freeze_node_list(&numerator_nodes)
        } else {
            incomplete.numerator
        };
        let fraction = Node::FractionNoad(MathFraction {
            numerator,
            denominator,
            thickness: incomplete.thickness,
            left_delimiter: incomplete.left_delimiter,
            right_delimiter: incomplete.right_delimiter,
        });
        leading_left.into_iter().chain([fraction]).collect()
    } else {
        nodes
    };
    stores.freeze_node_list(&nodes)
}

pub(super) fn noad_kind_for_constructor(primitive: UnexpandablePrimitive) -> NoadKind {
    match primitive {
        UnexpandablePrimitive::MathOrd => NoadKind::Normal(NoadClass::Ord),
        UnexpandablePrimitive::MathOp => NoadKind::Operator(LimitType::DisplayLimits),
        UnexpandablePrimitive::MathBin => NoadKind::Normal(NoadClass::Bin),
        UnexpandablePrimitive::MathRel => NoadKind::Normal(NoadClass::Rel),
        UnexpandablePrimitive::MathOpen => NoadKind::Normal(NoadClass::Open),
        UnexpandablePrimitive::MathClose => NoadKind::Normal(NoadClass::Close),
        UnexpandablePrimitive::MathPunct => NoadKind::Normal(NoadClass::Punct),
        UnexpandablePrimitive::MathInner => NoadKind::Normal(NoadClass::Inner),
        _ => unreachable!("caller restricts constructor primitive"),
    }
}

pub(super) fn style_for_primitive(primitive: UnexpandablePrimitive) -> MathStyle {
    match primitive {
        UnexpandablePrimitive::DisplayStyle => MathStyle::Display,
        UnexpandablePrimitive::TextStyle => MathStyle::Text,
        UnexpandablePrimitive::ScriptStyle => MathStyle::Script,
        UnexpandablePrimitive::ScriptScriptStyle => MathStyle::ScriptScript,
        _ => unreachable!("caller restricts style primitive"),
    }
}

/// tex.web §1064's `off_save` help, shared by every group it can repair.
///
/// `off_save` prints one `help5` regardless of which terminator it inserted,
/// so `Missing }`, `Missing \endgroup` and `Missing \right.` all carry it.
pub(super) const OFF_SAVE_HELP: [&str; 5] = [
    "I've inserted something that you may have forgotten.",
    "(See the <inserted text> above.)",
    "With luck, this will get me unwedged. But if you",
    "really didn't forget anything, try typing `2' now; then",
    "my insertion and my current dilemma will both disappear.",
];
