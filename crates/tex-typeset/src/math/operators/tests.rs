use super::*;
use crate::math::tests::{list_nodes, math_char, noad, root_nodes, sc, setup_universe};
use crate::math::{MathParams, Style, mlist_to_hlist};
use tex_state::math::{FractionThickness, MathFontSize, MathFraction};

#[test]
fn character_operator_observes_temporary_clean_and_dimensions_packs() {
    // TeX82 §749 calls clean_box for a character operator nucleus. Its
    // temporary noad crosses §724 before §720 packages the result, and the
    // completed operator then reaches the enclosing §724 dimensions pack.
    let mut stores = setup_universe();
    let input = stores.publish_page_nodes(&[Node::MathNoad(MathNoad::new(
        NoadKind::Operator(LimitType::NoLimits),
        MathField::MathChar(math_char('o')),
    ))]);
    let params = MathParams::read(&stores);

    let layout = mlist_to_hlist(&stores, input, Style::TEXT, false, &params);
    let [MathNode::HList(operator)] = root_nodes(&layout).as_slice() else {
        panic!("operator lowers to one horizontal box");
    };

    assert_eq!(
        layout.pack_observations(),
        &[
            super::super::MathPackObservation {
                axis: super::super::BoxAxis::Horizontal,
                width: operator.width,
                height: operator.height,
                depth: operator.depth,
            },
            super::super::MathPackObservation {
                axis: super::super::BoxAxis::Horizontal,
                width: operator.width,
                height: operator.height,
                depth: operator.depth,
            },
            super::super::MathPackObservation {
                axis: super::super::BoxAxis::Horizontal,
                width: operator.width,
                height: operator.height,
                depth: operator.depth,
            },
        ]
    );
}

#[test]
fn missing_character_operator_still_centers_its_empty_box() {
    // TeX82 §749 continues from a failed `fetch` to the common operator-axis
    // centering step. TRIP reaches this through a malformed class-Op noad.
    let mut stores = setup_universe();
    let missing = MathChar {
        family: 15,
        character: '\u{10ffff}',
        origin: tex_state::token::OriginId::UNKNOWN,
    };
    let params = MathParams::read(&stores);
    let expected = -params.for_size(MathFontSize::Text).symbols.axis_height;

    let operator = stores.publish_page_nodes(&[Node::MathNoad(MathNoad::new(
        NoadKind::Operator(LimitType::NoLimits),
        MathField::MathChar(missing),
    ))]);
    let layout = mlist_to_hlist(&stores, operator, Style::TEXT, false, &params);
    let [MathNode::HList(boxed)] = root_nodes(&layout).as_slice() else {
        panic!("missing operator character must leave one empty hbox");
    };
    assert_eq!(
        (boxed.width, boxed.height, boxed.depth),
        (sc(0), sc(0), sc(0))
    );
    assert_eq!(boxed.shift, expected);
    assert_eq!(
        layout.pack_observations().len(),
        1,
        "failed §749 fetch reaches only the enclosing §724 dimensions pack"
    );

    let ordinary = stores.publish_page_nodes(&[Node::MathNoad(MathNoad::new(
        NoadKind::Normal(NoadClass::Ord),
        MathField::MathChar(missing),
    ))]);
    let layout = mlist_to_hlist(&stores, ordinary, Style::TEXT, false, &params);
    assert!(root_nodes(&layout).is_empty());
}

#[test]
fn displayed_limits_use_shared_rebox_completion() {
    // TRIP's final `\mathop...\limits^\mathchoice` reaches TeX82 §715 from
    // displayed limits. The operator path must publish the shared rebox's
    // exact package; the shared vertical-source test also proves its preceding
    // natural hpack.
    let mut stores = setup_universe();
    let children = stores.publish_page_nodes(&[]);
    let script = stores.publish_page_nodes(&[Node::VList(tex_state::node::BoxNode::new(
        tex_state::node::BoxNodeFields {
            width: sc(5),
            height: sc(40),
            depth: sc(10),
            shift: sc(0),
            box_lr: tex_state::node::BoxLr::Normal,
            glue_set: tex_state::scaled::GlueSetRatio::ZERO,
            glue_sign: tex_state::node::Sign::Normal,
            glue_order: tex_state::glue::Order::Normal,
            children,
        },
    ))]);
    let mut op = MathNoad::new(
        NoadKind::Operator(LimitType::Limits),
        MathField::SubBox(script.clone()),
    );
    op.superscript = MathField::MathChar(math_char('o'));
    let input = stores.publish_page_nodes(&[Node::MathNoad(op)]);
    let params = MathParams::read(&stores);

    let layout = mlist_to_hlist(&stores, input, Style::TEXT, false, &params);
    let packs = layout.pack_observations();
    assert_eq!(packs.len(), 3, "{packs:#?}");
    assert_eq!(packs.last().expect("exact rebox completion").width, sc(14));
}

#[test]
fn boxed_operator_nucleus_is_not_character_axis_centered() {
    let mut stores = setup_universe();
    let children = stores.publish_page_nodes(&[]);
    let source_box = stores.publish_page_nodes(&[Node::HList(tex_state::node::BoxNode::new(
        tex_state::node::BoxNodeFields {
            width: sc(0),
            height: sc(0),
            depth: sc(0),
            shift: sc(0),
            box_lr: tex_state::node::BoxLr::Normal,
            glue_set: tex_state::scaled::GlueSetRatio::ZERO,
            glue_sign: tex_state::node::Sign::Normal,
            glue_order: tex_state::glue::Order::Normal,
            children,
        },
    ))]);
    let params = MathParams::read(&stores);
    for limit_type in [LimitType::NoLimits, LimitType::Limits] {
        let input = stores.publish_page_nodes(&[Node::MathNoad(MathNoad::new(
            NoadKind::Operator(limit_type),
            MathField::SubBox(source_box.clone()),
        ))]);
        let layout = mlist_to_hlist(&stores, input, Style::TEXT, false, &params);
        assert!(
            root_nodes(&layout).iter().all(|node| match node {
                MathNode::HList(boxed) | MathNode::VList(boxed) => boxed.shift == sc(0),
                _ => true,
            }),
            "TeX82 §749 axis-centers only character operator nuclei: {limit_type:?}: {:#?}",
            root_nodes(&layout)
        );
    }
}

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
    let limits_list = stores.publish_page_nodes(&[Node::MathNoad(limits)]);
    let layout = mlist_to_hlist(&stores, limits_list, Style::DISPLAY, false, &params);
    let [MathNode::VList(limits)] = root_nodes(&layout).as_slice() else {
        panic!("limits operator builds one vertical box");
    };
    // TeX82 §749 constructs all four limit clearances with `new_kern`, whose
    // subtype is normal. They are synthesized font-layout kerns, not the
    // explicit subtype reserved for user `\kern` input.
    assert!(
        list_nodes(&layout, limits.list)
            .iter()
            .filter_map(|node| match node {
                MathNode::Kern { kind, .. } => Some(kind),
                _ => None,
            })
            .all(|kind| *kind == KernKind::Font)
    );

    let mut side = MathNoad::new(
        NoadKind::Operator(LimitType::NoLimits),
        MathField::MathChar(math_char('o')),
    );
    side.superscript = MathField::MathChar(math_char('b'));
    side.subscript = MathField::MathChar(math_char('c'));
    let side_list = stores.publish_page_nodes(&[Node::MathNoad(side)]);
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
    let scripted = stores.publish_page_nodes(&[Node::MathNoad(scripted)]);
    let layout = mlist_to_hlist(&stores, scripted, Style::TEXT, false, &params);
    assert!(matches!(
        root_nodes(&layout).as_slice(),
        [MathNode::Char { ch: 'a', .. }, MathNode::VList(_)]
    ));

    let numerator = stores.publish_page_nodes(&[Node::MathNoad(noad(NoadClass::Ord, 'a'))]);
    let denominator = stores.publish_page_nodes(&[Node::MathNoad(noad(NoadClass::Ord, 'b'))]);
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
        let input = stores.publish_page_nodes(&[constructor]);
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

#[test]
fn radical_and_accent_clearance_skew_and_script_matrix() {
    // TeX.web §§735--742 (tex.web:14441--14578): rule 11 selects the
    // display/non-display radical clearance, while rule 12 applies skew,
    // chooses the largest fitting accent, and moves character scripts into
    // the accentee before the accent is stacked.
    let mut stores = setup_universe();
    let text_font = stores.math_family_font(MathFontSize::Text, 0);
    stores.set_font_skew_char(text_font, i32::from(b'k'));
    let params = MathParams::read(&stores);

    for (style, clearance) in [(Style::DISPLAY, sc(14)), (Style::TEXT, sc(5))] {
        let radical = MathNoad::new(
            NoadKind::Radical { delimiter: 0 },
            MathField::MathChar(math_char('a')),
        );
        let input = stores.publish_page_nodes(&[Node::MathNoad(radical)]);
        let layout = mlist_to_hlist(&stores, input, style, false, &params);
        let [MathNode::HList(radical)] = root_nodes(&layout).as_slice() else {
            panic!("radical lowers to one hbox");
        };
        let [_, MathNode::VList(overbar)] = list_nodes(&layout, radical.list).as_slice() else {
            panic!("radical contains delimiter and overbar");
        };
        let [_, _, MathNode::Kern { amount, .. }, _] = list_nodes(&layout, overbar.list).as_slice()
        else {
            panic!("overbar contains its clearance kern");
        };
        assert_eq!(*amount, clearance, "style={style:?}");
    }

    for (subscript, superscript, has_scripts) in [
        (MathField::Empty, MathField::Empty, false),
        (MathField::MathChar(math_char('b')), MathField::Empty, true),
        (MathField::Empty, MathField::MathChar(math_char('c')), true),
        (
            MathField::MathChar(math_char('b')),
            MathField::MathChar(math_char('c')),
            true,
        ),
    ] {
        let mut accent = MathNoad::new(
            NoadKind::Accent {
                accent: math_char('^'),
            },
            MathField::MathChar(math_char('a')),
        );
        accent.subscript = subscript;
        accent.superscript = superscript;
        let input = stores.publish_page_nodes(&[Node::MathNoad(accent)]);
        let layout = mlist_to_hlist(&stores, input, Style::TEXT, false, &params);
        assert_eq!(root_nodes(&layout).len(), 1);
        let [MathNode::VList(accented)] = root_nodes(&layout).as_slice() else {
            panic!("accent and any scripts lower into one vertical box");
        };
        let accented_nodes = list_nodes(&layout, accented.list);
        if !has_scripts {
            let Some(MathNode::HList(accent)) = accented_nodes.first().copied() else {
                panic!("accent glyph is the first stacked box");
            };
            assert_eq!(accent.shift, sc(6), "skew plus centering displacement");
            assert!(matches!(
                list_nodes(&layout, accent.list).as_slice(),
                [MathNode::Char { ch: '~', .. }]
            ));
        } else {
            assert!(accented_nodes.len() >= 2, "scripts remain inside accentee");
        }
    }
}

#[test]
fn fraction_rule_delimiter_style_and_rebox_matrix() {
    // TeX.web §§743--748 (tex.web:14579--14662): rule 15 covers all style,
    // thickness, delimiter-target, clearance, and unequal-width rebox paths.
    let mut stores = setup_universe();
    let numerator = stores.publish_page_nodes(&[Node::MathNoad(noad(NoadClass::Ord, 'a'))]);
    let denominator = stores.publish_page_nodes(&[
        Node::MathNoad(noad(NoadClass::Ord, 'b')),
        Node::MathNoad(noad(NoadClass::Ord, 'c')),
    ]);
    let params = MathParams::read(&stores);
    for style in [
        Style::DISPLAY,
        Style::TEXT,
        Style::SCRIPT,
        Style::SCRIPT_SCRIPT,
    ] {
        for (thickness, ruled) in [
            (FractionThickness::Default, true),
            (FractionThickness::Explicit(sc(0)), false),
            (FractionThickness::Explicit(sc(6)), true),
        ] {
            for delimiters in [false, true] {
                let delimiter =
                    delimiters.then_some(super::super::tests::delimiter_code(1, b'(', 1, b'|'));
                let fraction = MathFraction {
                    numerator: numerator.clone(),
                    denominator: denominator.clone(),
                    thickness,
                    left_delimiter: delimiter,
                    right_delimiter: delimiter,
                };
                let input = stores.publish_page_nodes(&[Node::FractionNoad(fraction)]);
                let layout = mlist_to_hlist(&stores, input, style, false, &params);
                let [MathNode::HList(outer)] = root_nodes(&layout).as_slice() else {
                    panic!("fraction lowers to one hbox");
                };
                let outer_nodes = list_nodes(&layout, outer.list);
                let [left, MathNode::VList(stack), right] = outer_nodes.as_slice() else {
                    panic!("fraction has left delimiter, stack, right delimiter");
                };
                let stack_nodes = list_nodes(&layout, stack.list);
                assert_eq!(stack_nodes.len(), if ruled { 5 } else { 3 });
                assert_eq!(
                    stack_nodes
                        .iter()
                        .filter(|node| matches!(node, MathNode::Rule { .. }))
                        .count(),
                    usize::from(ruled)
                );
                let (MathNode::HList(num), MathNode::HList(denom)) =
                    (stack_nodes[0], *stack_nodes.last().expect("denominator"))
                else {
                    panic!("fraction stack begins and ends with reboxed fields");
                };
                assert_eq!(num.width, denom.width, "style={style:?}");
                if delimiters {
                    assert!(
                        !list_nodes(
                            &layout,
                            match left {
                                MathNode::HList(b) | MathNode::VList(b) => b.list,
                                _ => panic!("left delimiter box"),
                            }
                        )
                        .is_empty()
                    );
                    assert!(
                        !list_nodes(
                            &layout,
                            match right {
                                MathNode::HList(b) | MathNode::VList(b) => b.list,
                                _ => panic!("right delimiter box"),
                            }
                        )
                        .is_empty()
                    );
                }
            }
        }
    }
}

#[test]
fn operator_ligature_and_script_attachment_matrix() {
    // TeX.web §§749--760 (tex.web:14663--14983): rules 13, 14, and 18 cover
    // operator limit selection, math ligature/kern processing, and the empty,
    // sub-only, sup-only, and paired side-script attachment paths.
    let mut stores = setup_universe();
    let params = MathParams::read(&stores);
    for (limit, style, limits_box) in [
        (LimitType::Limits, Style::TEXT, true),
        (LimitType::NoLimits, Style::DISPLAY, false),
        (LimitType::DisplayLimits, Style::DISPLAY, true),
        (LimitType::DisplayLimits, Style::TEXT, false),
    ] {
        let op = MathNoad::new(
            NoadKind::Operator(limit),
            MathField::MathChar(math_char('o')),
        );
        let input = stores.publish_page_nodes(&[Node::MathNoad(op)]);
        let layout = mlist_to_hlist(&stores, input, style, false, &params);
        assert_eq!(
            matches!(root_nodes(&layout)[0], MathNode::VList(_)),
            limits_box
        );
    }

    for (left, right, expected_chars, expected_kern) in [('a', 'a', 1, false), ('a', 'b', 2, true)]
    {
        let input = stores.publish_page_nodes(&[
            Node::MathNoad(noad(NoadClass::Ord, left)),
            Node::MathNoad(noad(NoadClass::Ord, right)),
        ]);
        let layout = mlist_to_hlist(&stores, input, Style::TEXT, false, &params);
        assert_eq!(
            root_nodes(&layout)
                .iter()
                .filter(|node| matches!(node, MathNode::Char { .. }))
                .count(),
            expected_chars
        );
        assert_eq!(
            root_nodes(&layout)
                .iter()
                .any(|node| matches!(node, MathNode::Kern { amount, kind: KernKind::Font } if *amount == sc(7))),
            expected_kern
        );
    }

    for (subscript, superscript, scripted) in [
        (MathField::Empty, MathField::Empty, false),
        (MathField::MathChar(math_char('b')), MathField::Empty, true),
        (MathField::Empty, MathField::MathChar(math_char('c')), true),
        (
            MathField::MathChar(math_char('b')),
            MathField::MathChar(math_char('c')),
            true,
        ),
    ] {
        let mut base = noad(NoadClass::Ord, 'a');
        base.subscript = subscript;
        base.superscript = superscript;
        let input = stores.publish_page_nodes(&[Node::MathNoad(base)]);
        let layout = mlist_to_hlist(&stores, input, Style::TEXT, false, &params);
        assert_eq!(
            root_nodes(&layout)
                .iter()
                .any(|node| matches!(node, MathNode::HList(_) | MathNode::VList(_))),
            scripted
        );
    }
}
