use super::*;
use tex_fonts::metrics::CharTag;
use tex_fonts::{CharMetrics, FontMetrics, LoadedFont};
use tex_state::glue::{GlueSpec, Order};
use tex_state::math::{MathChar, MathField, MathFontSize, MathNoad, NoadClass, NoadKind};

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
    let input = universe.freeze_node_list(&[
        Node::MathNoad(noad(NoadClass::Ord, 'b')),
        Node::MathNoad(noad(NoadClass::Bin, '+')),
        Node::MathNoad(noad(NoadClass::Ord, 'c')),
        Node::MathNoad(noad(NoadClass::Rel, '=')),
        Node::MathNoad(noad(NoadClass::Ord, 'b')),
    ]);
    let params = MathParams::read(&universe);

    let hlist = mlist_to_hlist(&universe, input, Style::TEXT, true, &params);

    assert!(matches!(hlist.nodes[0], MathNode::Char { ch: 'b', .. }));
    assert_glue_width(&hlist.nodes[1], 240);
    assert!(matches!(hlist.nodes[2], MathNode::Char { ch: '+', .. }));
    assert!(matches!(hlist.nodes[3], MathNode::Penalty(BIN_OP_PENALTY)));
    assert_glue_width(&hlist.nodes[4], 240);
    assert!(matches!(hlist.nodes[5], MathNode::Char { ch: 'c', .. }));
    assert_glue_width(&hlist.nodes[6], 360);
    assert!(matches!(hlist.nodes[7], MathNode::Char { ch: '=', .. }));
    assert!(matches!(hlist.nodes[8], MathNode::Penalty(REL_PENALTY)));
    assert_glue_width(&hlist.nodes[9], 360);
    assert!(matches!(hlist.nodes[10], MathNode::Char { ch: 'b', .. }));
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

    assert!(matches!(hlist.nodes[0], MathNode::Char { ch: 'a', .. }));
    let MathNode::HList(script_box) = &hlist.nodes[1] else {
        panic!("expected script box");
    };
    assert_eq!(script_box.axis, BoxAxis::Vertical);
    assert_eq!(script_box.shift, sc(15));
    let [
        MathNode::HList(sup),
        MathNode::Kern { amount, .. },
        MathNode::HList(sub),
    ] = script_box.list.nodes.as_slice()
    else {
        panic!("expected sup/kern/sub script vlist");
    };
    assert_eq!(sup.shift, sc(2));
    assert_eq!(sup.width, sc(10));
    assert_eq!(sub.width, sc(10));
    assert_eq!(*amount, sc(21));
}

fn assert_glue_width(node: &MathNode, expected: i32) {
    let MathNode::Glue { spec, .. } = node else {
        panic!("expected glue, got {node:?}");
    };
    assert_eq!(spec.width, sc(expected));
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
    let symbols = universe.intern_font(param_font("symbols", symbol_params()));
    let extension = universe.intern_font(param_font("extension", extension_params()));

    for (size, font) in [
        (MathFontSize::Text, text),
        (MathFontSize::Script, script),
        (MathFontSize::ScriptScript, script_script),
    ] {
        universe.set_math_family_font(size, 0, font, false);
        universe.set_math_family_font(size, 2, symbols, false);
        universe.set_math_family_font(size, 3, extension, false);
    }
    universe.set_dimen_param(DimenParam::new(12), sc(2));
    for (index, width) in [(15, 3), (16, 4), (17, 6)] {
        let id = universe.intern_glue(GlueSpec {
            width: sc(width * Scaled::UNITY),
            stretch: sc(0),
            stretch_order: Order::Normal,
            shrink: sc(0),
            shrink_order: Order::Normal,
        });
        universe.set_muskip(index, id);
    }
    universe
}

fn test_font(name: &str, scale: i32) -> LoadedFont {
    let mut chars = vec![None; 256];
    for ch in ['a', 'b', 'c', '+', '='] {
        chars[ch as usize] = Some(CharMetrics {
            width: sc(scale),
            height: sc(scale / 2),
            depth: sc(1),
            italic_correction: if ch == 'a' { sc(2) } else { sc(0) },
            tag: CharTag::None,
        });
    }
    LoadedFont::new(
        name,
        name,
        [0; 32],
        0,
        sc(10),
        sc(scale),
        vec![sc(0); 7],
        FontMetrics::new(chars, Vec::new(), None, None, Vec::new()),
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
        (13, 12),
        (14, 11),
        (15, 9),
        (16, 13),
        (17, 15),
        (18, 2),
        (19, 2),
        (22, 3),
    ] {
        params[number - 1] = sc(value);
    }
    params
}

fn extension_params() -> Vec<Scaled> {
    let mut params = vec![sc(0); 13];
    params[7] = sc(4);
    params
}
