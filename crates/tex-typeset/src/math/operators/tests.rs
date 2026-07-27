use super::*;
use crate::math::tests::{list_nodes, math_char, noad, root_nodes, sc, setup_universe};
use crate::math::{MathParams, Style, mlist_to_hlist};
use tex_state::math::{FractionThickness, MathFraction};

#[test]
fn tex82_noad_constructor_clearance_and_italic_matrix() {
    let mut stores = setup_universe();
    let params = MathParams::read(&stores);

    let mut limits = MathNoad::new(
        NoadKind::Operator(LimitType::DisplayLimits),
        MathField::MathChar(math_char('o')),
    );
    limits.superscript = MathField::MathChar(math_char('b'));
    limits.subscript = MathField::MathChar(math_char('c'));
    let limits_list = stores.freeze_node_list(&[Node::MathNoad(limits)]);
    let layout = mlist_to_hlist(&stores, limits_list, Style::DISPLAY, false, &params);
    assert!(matches!(
        root_nodes(&layout).as_slice(),
        [MathNode::VList(_)]
    ));

    let mut side = MathNoad::new(
        NoadKind::Operator(LimitType::NoLimits),
        MathField::MathChar(math_char('o')),
    );
    side.superscript = MathField::MathChar(math_char('b'));
    side.subscript = MathField::MathChar(math_char('c'));
    let side_list = stores.freeze_node_list(&[Node::MathNoad(side)]);
    let layout = mlist_to_hlist(&stores, side_list, Style::DISPLAY, false, &params);
    let [MathNode::HList(operator), MathNode::VList(scripts)] = root_nodes(&layout).as_slice()
    else {
        panic!("nolimits operator keeps side scripts");
    };
    assert_eq!(operator.width, sc(14));
    let Some(MathNode::HList(sup)) = list_nodes(&layout, scripts.list).first().copied() else {
        panic!("paired script box begins with superscript");
    };
    assert_eq!(sup.shift, sc(2));

    let mut scripted = noad(NoadClass::Ord, 'a');
    scripted.superscript = MathField::MathChar(math_char('b'));
    scripted.subscript = MathField::MathChar(math_char('c'));
    let scripted = stores.freeze_node_list(&[Node::MathNoad(scripted)]);
    let layout = mlist_to_hlist(&stores, scripted, Style::TEXT, false, &params);
    assert!(matches!(
        root_nodes(&layout).as_slice(),
        [MathNode::Char { ch: 'a', .. }, MathNode::VList(_)]
    ));

    let numerator = stores.freeze_node_list(&[Node::MathNoad(noad(NoadClass::Ord, 'a'))]);
    let denominator = stores.freeze_node_list(&[Node::MathNoad(noad(NoadClass::Ord, 'b'))]);
    let constructors = [
        Node::FractionNoad(MathFraction {
            numerator,
            denominator,
            thickness: FractionThickness::Default,
            left_delimiter: None,
            right_delimiter: None,
        }),
        Node::MathNoad(MathNoad::new(
            NoadKind::Radical { delimiter: 0 },
            MathField::MathChar(math_char('a')),
        )),
        Node::MathNoad(MathNoad::new(
            NoadKind::Accent {
                accent: math_char('^'),
            },
            MathField::MathChar(math_char('a')),
        )),
        Node::MathNoad(MathNoad::new(
            NoadKind::Overline,
            MathField::MathChar(math_char('a')),
        )),
        Node::MathNoad(MathNoad::new(
            NoadKind::Underline,
            MathField::MathChar(math_char('a')),
        )),
    ];
    for constructor in constructors {
        let input = stores.freeze_node_list(&[constructor]);
        let layout = mlist_to_hlist(&stores, input, Style::TEXT, false, &params);
        assert!(
            matches!(
                root_nodes(&layout).as_slice(),
                [MathNode::HList(_) | MathNode::VList(_)]
            ),
            "constructor lowers to one deterministic box: {:?}",
            root_nodes(&layout)
        );
    }
}
