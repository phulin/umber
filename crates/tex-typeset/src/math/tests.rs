use super::*;
use tex_fonts::metrics::CharTag;
use tex_fonts::{CharMetrics, FontMetrics, LigKernCommand, LigKernInstruction, LoadedFont};
use tex_state::env::banks::{GlueParam, IntParam};
use tex_state::glue::{GlueSpec, Order};
use tex_state::math::{
    FractionThickness, LimitType, MathChar, MathField, MathFontSize, MathFraction, MathNoad,
    NoadClass, NoadKind,
};
use tex_state::node::{BoxNode, BoxNodeFields, Sign};
use tex_state::scaled::GlueSetRatio;

fn sc(raw: i32) -> Scaled {
    Scaled::from_raw(raw)
}

#[test]
fn style_transitions_follow_tex_style_codes() {
    assert_eq!(
        Style::DISPLAY.cramped_style(),
        Style::new(StyleFamily::Display, true)
    );
    assert_eq!(
        Style::DISPLAY.sub_style(),
        Style::new(StyleFamily::Script, true)
    );
    assert_eq!(Style::TEXT.sup_style(), Style::SCRIPT);
    assert_eq!(
        Style::new(StyleFamily::Text, true).sup_style(),
        Style::new(StyleFamily::Script, true)
    );
    assert_eq!(Style::DISPLAY.num_style(), Style::TEXT);
    assert_eq!(Style::TEXT.num_style(), Style::SCRIPT);
    assert_eq!(
        Style::SCRIPT_SCRIPT.denom_style(),
        Style::new(StyleFamily::ScriptScript, true)
    );
}

#[test]
fn math_glue_converts_mu_dimensions_with_current_math_quad() {
    let mu = sc(60);
    let glue = GlueSpec {
        width: sc(3 * Scaled::UNITY),
        stretch: sc(2 * Scaled::UNITY),
        stretch_order: Order::Normal,
        shrink: sc(Scaled::UNITY),
        shrink_order: Order::Fil,
    };

    let converted = math_glue(glue, mu);

    assert_eq!(converted.width, sc(180));
    assert_eq!(converted.stretch, sc(120));
    assert_eq!(converted.shrink, sc(Scaled::UNITY));
    assert_eq!(converted.shrink_order, Order::Fil);
}

#[test]
fn mlist_second_pass_inserts_spacing_and_penalties() {
    let mut universe = setup_universe();
    universe.set_int_param(IntParam::BIN_OP_PENALTY, 700);
    universe.set_int_param(IntParam::REL_PENALTY, 500);
    let input = universe.freeze_node_list(&[
        Node::MathNoad(noad(NoadClass::Ord, 'b')),
        Node::MathNoad(noad(NoadClass::Bin, '+')),
        Node::MathNoad(noad(NoadClass::Ord, 'c')),
        Node::MathNoad(noad(NoadClass::Rel, '=')),
        Node::MathNoad(noad(NoadClass::Ord, 'b')),
    ]);
    let params = MathParams::read(&universe);

    let hlist = mlist_to_hlist(&universe, input, Style::TEXT, true, &params);
    let nodes = root_nodes(&hlist);

    assert!(matches!(nodes[0], MathNode::Char { ch: 'b', .. }));
    assert_glue_width(nodes[1], 240);
    assert!(matches!(nodes[2], MathNode::Char { ch: '+', .. }));
    assert!(matches!(nodes[3], MathNode::Penalty(700)));
    assert_glue_width(nodes[4], 240);
    assert!(matches!(nodes[5], MathNode::Char { ch: 'c', .. }));
    assert_glue_width(nodes[6], 360);
    assert!(matches!(nodes[7], MathNode::Char { ch: '=', .. }));
    assert!(matches!(nodes[8], MathNode::Penalty(500)));
    assert_glue_width(nodes[9], 360);
    assert!(matches!(nodes[10], MathNode::Char { ch: 'b', .. }));
}

#[test]
fn mlist_second_pass_preserves_named_math_glue_provenance() {
    assert_inserted_math_glue_kind(
        &[NoadClass::Ord, NoadClass::Op],
        MathGlueKind::ThinMuSkip,
        180,
    );
    assert_inserted_math_glue_kind(
        &[NoadClass::Ord, NoadClass::Bin, NoadClass::Ord],
        MathGlueKind::MedMuSkip,
        240,
    );
    assert_inserted_math_glue_kind(
        &[NoadClass::Ord, NoadClass::Rel],
        MathGlueKind::ThickMuSkip,
        360,
    );
}

#[test]
fn mlist_penalties_use_current_parameters_and_infinite_threshold() {
    let mut universe = setup_universe();
    universe.set_int_param(IntParam::BIN_OP_PENALTY, 12_345);
    universe.set_int_param(IntParam::REL_PENALTY, 99);
    let input = universe.freeze_node_list(&[
        Node::MathNoad(noad(NoadClass::Ord, 'b')),
        Node::MathNoad(noad(NoadClass::Bin, '+')),
        Node::MathNoad(noad(NoadClass::Ord, 'c')),
        Node::MathNoad(noad(NoadClass::Rel, '=')),
        Node::MathNoad(noad(NoadClass::Ord, 'd')),
    ]);
    let params = MathParams::read(&universe);
    universe.set_int_param(IntParam::BIN_OP_PENALTY, 1);
    universe.set_int_param(IntParam::REL_PENALTY, 2);

    let hlist = mlist_to_hlist(&universe, input, Style::TEXT, true, &params);
    let nodes = root_nodes(&hlist);

    assert!(
        !nodes
            .iter()
            .any(|node| matches!(node, MathNode::Penalty(12_345)))
    );
    assert!(
        nodes
            .iter()
            .any(|node| matches!(node, MathNode::Penalty(99)))
    );
    assert!(
        !nodes
            .iter()
            .any(|node| matches!(node, MathNode::Penalty(2)))
    );
}

#[test]
fn script_pair_uses_italic_delta_scriptspace_and_cramped_substyle() {
    let mut universe = setup_universe();
    let mut noad = noad(NoadClass::Ord, 'a');
    noad.subscript = MathField::MathChar(math_char('b'));
    noad.superscript = MathField::MathChar(math_char('c'));
    let input = universe.freeze_node_list(&[Node::MathNoad(noad)]);
    let params = MathParams::read(&universe);

    let hlist = mlist_to_hlist(&universe, input, Style::TEXT, false, &params);
    let nodes = root_nodes(&hlist);

    assert!(matches!(nodes[0], MathNode::Char { ch: 'a', .. }));
    let MathNode::VList(script_box) = nodes[1] else {
        panic!("expected script box");
    };
    assert_eq!(script_box.axis, BoxAxis::Vertical);
    assert_eq!(script_box.shift, sc(-15));
    let script_nodes = list_nodes(&hlist, script_box.list);
    let [sup_node, kern_node, sub_node] = script_nodes.as_slice() else {
        panic!("expected sup/kern/sub script vlist");
    };
    let MathNode::HList(sup) = sup_node else {
        panic!("expected superscript")
    };
    let MathNode::Kern { amount, .. } = kern_node else {
        panic!("expected script kern")
    };
    let MathNode::HList(sub) = sub_node else {
        panic!("expected subscript")
    };
    assert_eq!(sup.shift, sc(2));
    assert_eq!(sup.width, sc(10));
    assert_eq!(sub.width, sc(10));
    assert_eq!(*amount, sc(21));
}

#[test]
fn make_ord_inserts_font_kern_between_adjacent_math_chars() {
    let mut universe = setup_universe();
    let input = universe.freeze_node_list(&[
        Node::MathNoad(noad(NoadClass::Ord, 'a')),
        Node::MathNoad(noad(NoadClass::Ord, 'b')),
    ]);
    let params = MathParams::read(&universe);

    let hlist = mlist_to_hlist(&universe, input, Style::TEXT, false, &params);
    let nodes = root_nodes(&hlist);

    assert!(matches!(nodes[0], MathNode::Char { ch: 'a', .. }));
    assert!(matches!(
        nodes[1],
        MathNode::Kern {
            amount,
            kind: KernKind::Font,
        } if *amount == sc(2)
    ));
    assert!(matches!(
        nodes[2],
        MathNode::Kern {
            amount,
            kind: KernKind::Font,
        } if *amount == sc(7)
    ));
    assert!(matches!(nodes[3], MathNode::Char { ch: 'b', .. }));
}

#[test]
fn var_delimiter_searches_small_chain_before_large_and_builds_extensible() {
    let universe = setup_universe();
    let params = MathParams::read(&universe);
    let delimiter = delimiter_code(1, b'(', 1, b'|');

    let (small_layout, small) =
        test_var_delimiter(&universe, &params, delimiter, MathFontSize::Text, sc(25));
    assert_eq!(small.axis, BoxAxis::Horizontal);
    assert!(matches!(
        list_nodes(&small_layout, small.list).as_slice(),
        [MathNode::Char { ch: '[', .. }]
    ));

    let (extensible_layout, extensible) =
        test_var_delimiter(&universe, &params, delimiter, MathFontSize::Text, sc(35));
    assert_eq!(extensible.axis, BoxAxis::Vertical);
    assert_eq!(extensible.height, sc(4));
    assert_eq!(extensible.depth, sc(34));
    assert_eq!(list_nodes(&extensible_layout, extensible.list).len(), 5);
}

#[test]
fn make_fraction_uses_default_rule_and_delimiter_target() {
    let mut universe = setup_universe();
    let numerator = universe.freeze_node_list(&[Node::MathNoad(noad(NoadClass::Ord, 'a'))]);
    let denominator = universe.freeze_node_list(&[Node::MathNoad(noad(NoadClass::Ord, 'b'))]);
    let input = universe.freeze_node_list(&[Node::FractionNoad(MathFraction {
        numerator,
        denominator,
        thickness: FractionThickness::Default,
        left_delimiter: Some(delimiter_code(1, b'(', 1, b'|')),
        right_delimiter: None,
    })]);
    let params = MathParams::read(&universe);

    let hlist = mlist_to_hlist(&universe, input, Style::TEXT, false, &params);

    let nodes = root_nodes(&hlist);
    let [fraction_node] = nodes.as_slice() else {
        panic!("expected fraction hbox");
    };
    let MathNode::HList(fraction) = fraction_node else {
        panic!("expected fraction hbox")
    };
    let fraction_nodes = list_nodes(&hlist, fraction.list);
    let [left, vlist_node, _right] = fraction_nodes.as_slice() else {
        panic!("expected delimited fraction hlist");
    };
    let MathNode::VList(vlist) = vlist_node else {
        panic!("expected fraction vlist")
    };
    let MathNode::HList(left_box) = left else {
        panic!("expected left delimiter box")
    };
    assert!(matches!(
        list_nodes(&hlist, left_box.list).as_slice(),
        [MathNode::Char { ch: '[', .. }]
    ));
    assert_eq!(vlist.height, sc(26));
    assert_eq!(vlist.depth, sc(18));
    let vnodes = list_nodes(&hlist, vlist.list);
    let [_, _, rule, _, _] = vnodes.as_slice() else {
        panic!("expected fraction stack")
    };
    assert!(matches!(rule, MathNode::Rule { height: Some(thickness), .. } if *thickness == sc(4)));
}

#[test]
fn left_right_delimiters_size_to_enclosed_list() {
    let mut universe = setup_universe();
    let tall_box = universe.freeze_node_list(&[Node::Rule {
        width: Some(sc(4)),
        height: Some(sc(40)),
        depth: Some(sc(10)),
    }]);
    let delimiter = delimiter_code(1, b'(', 1, b'|');
    let input = universe.freeze_node_list(&[
        Node::MathNoad(MathNoad::new(
            NoadKind::LeftDelimiter { delimiter },
            MathField::Empty,
        )),
        Node::MathNoad(MathNoad::new(
            NoadKind::Normal(NoadClass::Ord),
            MathField::SubBox(tall_box),
        )),
        Node::MathNoad(MathNoad::new(
            NoadKind::RightDelimiter { delimiter },
            MathField::Empty,
        )),
    ]);
    let params = MathParams::read(&universe);

    let hlist = mlist_to_hlist(&universe, input, Style::TEXT, false, &params);
    let nodes = root_nodes(&hlist);

    let Some(MathNode::VList(left)) = nodes.first().copied() else {
        panic!("expected left delimiter")
    };
    let Some(MathNode::VList(right)) = nodes.last().copied() else {
        panic!("expected right delimiter")
    };
    assert!(list_nodes(&hlist, left.list).len() > 3);
    assert!(list_nodes(&hlist, right.list).len() > 3);
}

#[test]
fn ordinary_sub_box_nucleus_is_not_repacked() {
    let mut universe = setup_universe();
    let children = universe.freeze_node_list(&[]);
    let sub_box = Node::VList(BoxNode::new(BoxNodeFields {
        width: sc(4),
        height: sc(40),
        depth: sc(10),
        shift: sc(0),
        display: false,
        glue_set: GlueSetRatio::from_raw(0),
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children,
    }));
    let expected = sub_box.clone();
    let sub_box = universe.freeze_node_list(&[sub_box]);
    let input = universe.freeze_node_list(&[Node::MathNoad(MathNoad::new(
        NoadKind::Normal(NoadClass::Ord),
        MathField::SubBox(sub_box),
    ))]);
    let params = MathParams::read(&universe);

    let hlist = mlist_to_hlist(&universe, input, Style::TEXT, false, &params);

    let logical = hlist.logical_nodes(hlist.root());
    assert_eq!(logical.len(), 1);
    assert_eq!(logical[0], &MathNode::Opaque(Box::new(expected)));
}

#[test]
fn display_operator_uses_larger_variant_and_places_limits() {
    let mut universe = setup_universe();
    let mut op = MathNoad::new(
        NoadKind::Operator(LimitType::DisplayLimits),
        MathField::MathChar(math_char('o')),
    );
    op.subscript = MathField::MathChar(math_char('b'));
    op.superscript = MathField::MathChar(math_char('c'));
    let input = universe.freeze_node_list(&[Node::MathNoad(op)]);
    let params = MathParams::read(&universe);

    let hlist = mlist_to_hlist(&universe, input, Style::DISPLAY, false, &params);

    let nodes = root_nodes(&hlist);
    let [limits_node] = nodes.as_slice() else {
        panic!("expected displayed-limits vbox");
    };
    let MathNode::VList(limits) = limits_node else {
        panic!("expected displayed-limits vbox")
    };
    assert_eq!(limits.width, sc(16));
    assert!(list_nodes(&hlist, limits.list).iter().any(|node| {
        let MathNode::HList(outer) = node else {
            return false;
        };
        list_nodes(&hlist, outer.list).iter().any(|node| {
            let MathNode::HList(inner) = node else {
                return false;
            };
            matches!(
                list_nodes(&hlist, inner.list).as_slice(),
                [MathNode::Char { ch: 'O', .. }]
            )
        })
    }));
}

#[test]
fn display_limits_operator_without_scripts_keeps_italic_correction_width() {
    let mut universe = setup_universe();
    let op = MathNoad::new(
        NoadKind::Operator(LimitType::DisplayLimits),
        MathField::MathChar(math_char('o')),
    );
    let input = universe.freeze_node_list(&[Node::MathNoad(op)]);
    let params = MathParams::read(&universe);

    let hlist = mlist_to_hlist(&universe, input, Style::DISPLAY, false, &params);

    let nodes = root_nodes(&hlist);
    let [limits_node] = nodes.as_slice() else {
        panic!("expected displayed-limits vbox");
    };
    let MathNode::VList(limits) = limits_node else {
        panic!("expected displayed-limits vbox")
    };
    assert_eq!(limits.width, sc(16));
}

#[test]
fn nolimits_operator_splits_italic_correction_into_script_placement() {
    let mut universe = setup_universe();
    let mut op = MathNoad::new(
        NoadKind::Operator(LimitType::NoLimits),
        MathField::MathChar(math_char('o')),
    );
    op.subscript = MathField::MathChar(math_char('b'));
    op.superscript = MathField::MathChar(math_char('c'));
    let input = universe.freeze_node_list(&[Node::MathNoad(op)]);
    let params = MathParams::read(&universe);

    let hlist = mlist_to_hlist(&universe, input, Style::DISPLAY, false, &params);

    let nodes = root_nodes(&hlist);
    let [op_node, scripts_node] = nodes.as_slice() else {
        panic!("expected operator followed by script box");
    };
    let MathNode::HList(op_box) = op_node else {
        panic!("expected operator box")
    };
    let MathNode::VList(scripts) = scripts_node else {
        panic!("expected script box")
    };
    assert_eq!(op_box.width, sc(14));
    let script_nodes = list_nodes(&hlist, scripts.list);
    let Some(MathNode::HList(sup)) = script_nodes.first().copied() else {
        panic!("expected script pair");
    };
    assert_eq!(sup.shift, sc(2));
}

#[test]
fn nolimits_operator_centers_nucleus_on_math_axis() {
    let mut universe = setup_universe();
    let op = MathNoad::new(
        NoadKind::Operator(LimitType::NoLimits),
        MathField::MathChar(math_char('c')),
    );
    let input = universe.freeze_node_list(&[Node::MathNoad(op)]);
    let params = MathParams::read(&universe);

    let hlist = mlist_to_hlist(&universe, input, Style::TEXT, false, &params);

    let nodes = root_nodes(&hlist);
    let [op_node] = nodes.as_slice() else {
        panic!("expected operator hbox");
    };
    let MathNode::HList(op_box) = op_node else {
        panic!("expected operator hbox")
    };
    assert_eq!(op_box.shift, sc(1));
}

#[test]
fn radical_clearance_uses_display_and_nondisplay_formulas() {
    let mut universe = setup_universe();
    let noad = MathNoad::new(
        NoadKind::Radical { delimiter: 0 },
        MathField::MathChar(math_char('a')),
    );
    let input = universe.freeze_node_list(&[Node::MathNoad(noad)]);
    let params = MathParams::read(&universe);

    let display = mlist_to_hlist(&universe, input, Style::DISPLAY, false, &params);
    let text = mlist_to_hlist(&universe, input, Style::TEXT, false, &params);

    assert_radical_clearance(&display, sc(14));
    assert_radical_clearance(&text, sc(5));
}

#[test]
fn math_accent_uses_skewchar_kern_and_larger_accent() {
    let mut universe = setup_universe();
    let text_font = universe.math_family_font(MathFontSize::Text, 0);
    universe.set_font_skew_char(text_font, i32::from(b'k'), false);
    let noad = MathNoad::new(
        NoadKind::Accent {
            accent: math_char('^'),
        },
        MathField::MathChar(math_char('a')),
    );
    let input = universe.freeze_node_list(&[Node::MathNoad(noad)]);
    let params = MathParams::read(&universe);

    let hlist = mlist_to_hlist(&universe, input, Style::TEXT, false, &params);

    let nodes = root_nodes(&hlist);
    let [accented_node] = nodes.as_slice() else {
        panic!("expected accent vbox");
    };
    let MathNode::VList(accented) = accented_node else {
        panic!("expected accent vbox")
    };
    let accented_nodes = list_nodes(&hlist, accented.list);
    let Some(MathNode::HList(accent)) = accented_nodes.first().copied() else {
        panic!("expected accent on top");
    };
    assert_eq!(accent.shift, sc(6));
    assert_eq!(accent.width, sc(0));
    assert!(matches!(
        list_nodes(&hlist, accent.list).as_slice(),
        [MathNode::Char { ch: '~', .. }]
    ));
}

fn assert_glue_width(node: &MathNode, expected: i32) {
    let MathNode::Glue { spec, .. } = node else {
        panic!("expected glue, got {node:?}");
    };
    assert_eq!(spec.width, sc(expected));
}

fn assert_inserted_math_glue_kind(classes: &[NoadClass], expected_kind: MathGlueKind, width: i32) {
    let mut universe = setup_universe();
    let input_nodes = classes
        .iter()
        .enumerate()
        .map(|(index, class)| Node::MathNoad(noad(*class, char::from(b'a' + index as u8))))
        .collect::<Vec<_>>();
    let input = universe.freeze_node_list(&input_nodes);
    let params = MathParams::read(&universe);

    let hlist = mlist_to_hlist(&universe, input, Style::TEXT, false, &params);

    assert!(
        root_nodes(&hlist).iter().any(|node| {
            matches!(
                node,
                MathNode::Glue { spec, kind, .. } if *kind == expected_kind && spec.width == sc(width)
            )
        }),
        "expected {expected_kind:?} glue in {hlist:?}"
    );
}

fn assert_radical_clearance(layout: &MathLayout, expected: Scaled) {
    let nodes = root_nodes(layout);
    let [radical_node] = nodes.as_slice() else {
        panic!("expected radical hbox");
    };
    let MathNode::HList(radical) = radical_node else {
        panic!("expected radical hbox")
    };
    let radical_nodes = list_nodes(layout, radical.list);
    let [_delimiter, overbar_node] = radical_nodes.as_slice() else {
        panic!("expected delimiter plus overbar");
    };
    let MathNode::VList(overbar) = overbar_node else {
        panic!("expected overbar")
    };
    let overbar_nodes = list_nodes(layout, overbar.list);
    let [_, _, kern, _] = overbar_nodes.as_slice() else {
        panic!("expected overbar list");
    };
    let MathNode::Kern { amount, .. } = kern else {
        panic!("expected clearance kern")
    };
    assert_eq!(*amount, expected);
}

fn root_nodes(layout: &MathLayout) -> Vec<&MathNode> {
    layout.logical_nodes(layout.root())
}

fn list_nodes(layout: &MathLayout, list: FrozenHList) -> Vec<&MathNode> {
    layout.logical_nodes(list)
}

fn noad(class: NoadClass, ch: char) -> MathNoad {
    MathNoad::new(NoadKind::Normal(class), MathField::MathChar(math_char(ch)))
}

fn math_char(ch: char) -> MathChar {
    MathChar {
        family: 0,
        character: ch,
    }
}

fn setup_universe() -> Universe {
    let mut universe = Universe::new();
    let text = universe.intern_font(test_font("math-text", 10));
    let script = universe.intern_font(test_font("math-script", 8));
    let script_script = universe.intern_font(test_font("math-script-script", 6));
    let delimiter = universe.intern_font(delimiter_font("delimiter"));
    let symbols = universe.intern_font(param_font("symbols", symbol_params()));
    let extension = universe.intern_font(param_font("extension", extension_params()));

    for (size, font) in [
        (MathFontSize::Text, text),
        (MathFontSize::Script, script),
        (MathFontSize::ScriptScript, script_script),
    ] {
        universe.set_math_family_font(size, 0, font, false);
        universe.set_math_family_font(size, 1, delimiter, false);
        universe.set_math_family_font(size, 2, symbols, false);
        universe.set_math_family_font(size, 3, extension, false);
    }
    universe.set_int_param(IntParam::DELIMITER_FACTOR, 901);
    universe.set_dimen_param(DimenParam::DELIMITER_SHORTFALL, sc(5));
    universe.set_dimen_param(DimenParam::NULL_DELIMITER_SPACE, sc(0));
    universe.set_dimen_param(DimenParam::new(12), sc(2));
    for (index, width) in [(15, 3), (16, 4), (17, 6)] {
        let id = universe.intern_glue(GlueSpec {
            width: sc(width * Scaled::UNITY),
            stretch: sc(0),
            stretch_order: Order::Normal,
            shrink: sc(0),
            shrink_order: Order::Normal,
        });
        universe.set_glue_param(GlueParam::new(index), id);
    }
    universe
}

fn test_font(name: &str, scale: i32) -> LoadedFont {
    let mut chars = vec![None; 256];
    for ch in ['a', 'b', 'c', '+', '=', 'k'] {
        chars[ch as usize] = Some(CharMetrics {
            width: sc(scale),
            height: sc(scale / 2),
            depth: sc(1),
            italic_correction: if ch == 'a' { sc(2) } else { sc(0) },
            tag: if ch == 'a' {
                CharTag::LigKern {
                    program_index: 0,
                    start_index: 0,
                }
            } else {
                CharTag::None
            },
        });
    }
    chars['o' as usize] = Some(CharMetrics {
        width: sc(12),
        height: sc(7),
        depth: sc(2),
        italic_correction: sc(2),
        tag: CharTag::NextLarger(b'O'),
    });
    chars['O' as usize] = Some(CharMetrics {
        width: sc(14),
        height: sc(9),
        depth: sc(3),
        italic_correction: sc(2),
        tag: CharTag::None,
    });
    chars['^' as usize] = Some(CharMetrics {
        width: sc(5),
        height: sc(3),
        depth: sc(0),
        italic_correction: sc(0),
        tag: CharTag::NextLarger(b'~'),
    });
    chars['~' as usize] = Some(CharMetrics {
        width: sc(9),
        height: sc(3),
        depth: sc(0),
        italic_correction: sc(0),
        tag: CharTag::None,
    });
    let lig_kern_program = vec![
        LigKernInstruction {
            skip_byte: 0,
            next_char: b'b',
            command: Some(LigKernCommand::Kern(sc(7))),
        },
        LigKernInstruction {
            skip_byte: 128,
            next_char: b'k',
            command: Some(LigKernCommand::Kern(sc(4))),
        },
    ];
    LoadedFont::new(
        name,
        name,
        [0; 32],
        0,
        sc(10),
        sc(scale),
        vec![sc(0); 7],
        FontMetrics::new(chars, lig_kern_program, None, None, Vec::new()),
    )
}

fn delimiter_font(name: &str) -> LoadedFont {
    let mut chars = vec![None; 256];
    for (ch, width, height, depth, tag) in [
        ('(', 5, 8, 4, CharTag::NextLarger(b'[')),
        ('[', 6, 20, 10, CharTag::None),
        ('|', 6, 4, 4, CharTag::Extensible(0)),
        ('^', 6, 4, 0, CharTag::None),
        ('!', 6, 5, 5, CharTag::None),
        ('v', 6, 0, 4, CharTag::None),
    ] {
        chars[ch as usize] = Some(CharMetrics {
            width: sc(width),
            height: sc(height),
            depth: sc(depth),
            italic_correction: sc(0),
            tag,
        });
    }
    LoadedFont::new(
        name,
        name,
        [2; 32],
        0,
        sc(10),
        sc(10),
        vec![sc(0); 7],
        FontMetrics::new(
            chars,
            Vec::new(),
            None,
            None,
            vec![tex_fonts::metrics::ExtensibleRecipe {
                top: Some(b'^'),
                middle: None,
                bottom: Some(b'v'),
                repeated: b'!',
            }],
        ),
    )
}

fn param_font(name: &str, params: Vec<Scaled>) -> LoadedFont {
    LoadedFont::new(
        name,
        name,
        [1; 32],
        0,
        sc(10),
        sc(10),
        params,
        FontMetrics::default(),
    )
}

fn symbol_params() -> Vec<Scaled> {
    let mut params = vec![sc(0); 22];
    for (number, value) in [
        (5, 40),
        (6, 18 * 60),
        (8, 30),
        (9, 22),
        (10, 12),
        (11, 31),
        (12, 17),
        (13, 12),
        (14, 11),
        (15, 9),
        (16, 13),
        (17, 15),
        (18, 2),
        (19, 2),
        (20, 25),
        (21, 20),
        (22, 3),
    ] {
        params[number - 1] = sc(value);
    }
    params
}

fn extension_params() -> Vec<Scaled> {
    let mut params = vec![sc(0); 13];
    params[7] = sc(4);
    params[8] = sc(5);
    params[9] = sc(6);
    params[10] = sc(7);
    params[11] = sc(8);
    params[12] = sc(9);
    params
}

fn delimiter_code(small_family: u8, small: u8, large_family: u8, large: u8) -> u32 {
    (u32::from(small_family) << 20)
        | (u32::from(small) << 12)
        | (u32::from(large_family) << 8)
        | u32::from(large)
}
