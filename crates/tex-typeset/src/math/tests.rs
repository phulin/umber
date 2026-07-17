use super::*;
use sha2::{Digest, Sha256};
use tex_fonts::metrics::CharTag;
use tex_fonts::{
    AcceptedFontContainers, CharMetrics, FontContainer, FontFeaturePolicy, FontLimits, FontMetrics,
    FontPurposes, FontRequest, FontRequestKey, LigKernCommand, LigKernInstruction, LigatureCommand,
    LoadedFont, OpenTypeFont, OpenTypeProgramSelection, ResolvedFont, VariationSelection,
    WritingDirection,
};
use tex_state::env::banks::{GlueParam, IntParam};
use tex_state::glue::{GlueSpec, Order};
use tex_state::math::{
    FractionThickness, LimitType, MathChar, MathChoice, MathField, MathFontSize, MathFraction,
    MathNoad, NoadClass, NoadKind,
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
fn pinned_opentype_math_fixture_drives_basic_formula_layout_deterministically() {
    let woff2 = include_bytes!("../../../tex-fonts/tests/fixtures/stix-two-math.woff2");
    let web = parse_stix_math(FontContainer::Woff2, woff2.to_vec());
    let ttf = woff2_patched::convert_woff2_to_ttf(&mut woff2.as_slice())
        .expect("decode equivalent native fixture");
    let native = parse_stix_math(FontContainer::TrueType, ttf);
    assert_eq!(web.identity, native.identity);
    assert_ne!(web.object_identity, native.object_identity);
    assert_eq!(web.math, native.math);

    let web_layouts = positioned_math_fixture_layouts(web);
    let native_layouts = positioned_math_fixture_layouts(native);
    assert_eq!(
        math_layout_projection(&web_layouts),
        math_layout_projection(&native_layouts)
    );
    assert_eq!(
        math_layout_digest(&web_layouts),
        "2c9bbe287f7c772381051b4fde4a679e885fce3e2a2ebeb0ec5d4b1dcaa2d661"
    );
}

fn parse_stix_math(container: FontContainer, bytes: Vec<u8>) -> OpenTypeFont {
    let key = FontRequestKey::new(
        "stix-two-math",
        0,
        VariationSelection::default(),
        FontFeaturePolicy::default(),
    )
    .expect("fixture key");
    let request = FontRequest {
        key: key.clone(),
        accepted_containers: match container {
            FontContainer::Woff2 => AcceptedFontContainers::WASM,
            FontContainer::OpenType | FontContainer::TrueType | FontContainer::Collection => {
                AcceptedFontContainers::NATIVE
            }
        },
        purposes: FontPurposes::LAYOUT_AND_HTML,
    };
    OpenTypeFont::parse(
        &request,
        ResolvedFont {
            request: key,
            container,
            bytes,
            declared_object_sha256: None,
            declared_program_identity: None,
            provenance: Some("STIX Two Math under the SIL OFL".to_owned()),
        },
        FontLimits::default(),
    )
    .expect("STIX fixture")
}

fn positioned_math_fixture_layouts(font: OpenTypeFont) -> Vec<MathLayout> {
    let size = sc(10 * Scaled::UNITY);
    let loaded = LoadedFont::new_opentype(
        "stix-two-math",
        "stix-two-math.woff2",
        size,
        size,
        OpenTypeProgramSelection {
            font,
            variation: VariationSelection::default(),
            features: FontFeaturePolicy::default(),
            direction: WritingDirection::LeftToRight,
        },
    );
    let MathMetricsSource::OpenType(stix_math) = loaded.math_metrics_source() else {
        panic!("fixture has MATH metrics");
    };
    let base_sum = stix_math.glyph('∑', 0).expect("sum glyph").glyph_id;
    assert!(
        stix_math
            .construction(base_sum, tex_fonts::MathVariantDirection::Vertical)
            .is_some()
    );
    let mut universe = Universe::new();
    let font = universe.intern_font(loaded);
    for size in [
        MathFontSize::Text,
        MathFontSize::Script,
        MathFontSize::ScriptScript,
    ] {
        for family in 0..=3 {
            universe.set_math_family_font(size, family, font, false);
        }
    }
    let numerator = universe.freeze_node_list(&[Node::MathNoad(noad(NoadClass::Ord, 'A'))]);
    let denominator = universe.freeze_node_list(&[Node::MathNoad(noad(NoadClass::Ord, 'B'))]);
    let mut scripted = noad(NoadClass::Ord, 'f');
    scripted.superscript = MathField::MathChar(math_char('A'));
    scripted.subscript = MathField::MathChar(math_char('B'));
    let mut operator = MathNoad::new(
        NoadKind::Operator(LimitType::Limits),
        MathField::MathChar(math_char('∑')),
    );
    operator.superscript = MathField::MathChar(math_char('A'));
    operator.subscript = MathField::MathChar(math_char('B'));
    let input = universe.freeze_node_list(&[
        Node::MathNoad(scripted),
        Node::FractionNoad(MathFraction {
            numerator,
            denominator,
            thickness: FractionThickness::Default,
            left_delimiter: None,
            right_delimiter: None,
        }),
        Node::MathNoad(MathNoad::new(
            NoadKind::Accent {
                accent: math_char('^'),
            },
            MathField::MathChar(math_char('A')),
        )),
        Node::MathNoad(operator),
    ]);
    let params = MathParams::read(&universe);
    let formula = mlist_to_hlist(&universe, input, Style::DISPLAY, false, &params);
    assert!(params.text.extension.default_rule_thickness.raw() > 0);
    assert!(all_math_glyphs(&formula).iter().all(Option::is_some));
    assert!(all_math_glyphs(&formula).len() >= 7);
    assert!(!all_math_glyphs(&formula).contains(&Some(base_sum)));

    let delimiter = delimiter_code(1, b'(', 1, b'(');
    let tall = universe.freeze_node_list(&[Node::Rule {
        width: Some(sc(4 * Scaled::UNITY)),
        height: Some(sc(80 * Scaled::UNITY)),
        depth: Some(sc(20 * Scaled::UNITY)),
    }]);
    let delimiter_input = universe.freeze_node_list(&[
        Node::MathNoad(MathNoad::new(
            NoadKind::LeftDelimiter { delimiter },
            MathField::Empty,
        )),
        Node::MathNoad(MathNoad::new(
            NoadKind::Normal(NoadClass::Ord),
            MathField::SubBox(tall),
        )),
        Node::MathNoad(MathNoad::new(
            NoadKind::RightDelimiter { delimiter },
            MathField::Empty,
        )),
    ]);
    let delimiter_layout =
        mlist_to_hlist(&universe, delimiter_input, Style::DISPLAY, false, &params);
    assert!(all_math_glyphs(&delimiter_layout).len() > 1);

    let radical_input = universe.freeze_node_list(&[Node::MathNoad(MathNoad::new(
        NoadKind::Radical { delimiter },
        MathField::MathChar(math_char('A')),
    ))]);
    let radical = mlist_to_hlist(&universe, radical_input, Style::DISPLAY, false, &params);
    assert!(all_math_glyphs(&radical).len() >= 2);

    let wide_base = universe.freeze_node_list(
        &(0..20)
            .map(|_| Node::MathNoad(noad(NoadClass::Ord, 'A')))
            .collect::<Vec<_>>(),
    );
    let wide_accent = universe.freeze_node_list(&[Node::MathNoad(MathNoad::new(
        NoadKind::Accent {
            accent: math_char('⏞'),
        },
        MathField::SubMlist(wide_base),
    ))]);
    let accent = mlist_to_hlist(&universe, wide_accent, Style::DISPLAY, false, &params);
    assert!(all_math_glyphs(&accent).len() > 20);
    vec![formula, delimiter_layout, radical, accent]
}

fn math_layout_digest(layouts: &[MathLayout]) -> String {
    format!("{:x}", Sha256::digest(math_layout_projection(layouts)))
}

fn math_layout_projection(layouts: &[MathLayout]) -> String {
    fn span(layout: &MathLayout, list: FrozenHList, out: &mut String) {
        use std::fmt::Write;

        write!(
            out,
            "[{}:{}:{}:{}|",
            list.width().raw(),
            list.height().raw(),
            list.depth().raw(),
            list.node_count()
        )
        .expect("write projection");
        for node in layout.nodes(list) {
            match node {
                MathNode::Char {
                    ch,
                    glyph_id,
                    metrics,
                    ..
                } => write!(
                    out,
                    "c{:x}/{glyph_id:?}/{}/{}/{}/{};",
                    *ch as u32,
                    metrics.width.raw(),
                    metrics.height.raw(),
                    metrics.depth.raw(),
                    metrics.italic_correction.raw()
                )
                .expect("write char"),
                MathNode::Kern { amount, kind } => {
                    write!(out, "k{}/{kind:?};", amount.raw()).expect("write kern")
                }
                MathNode::Glue { spec, kind, .. } => write!(
                    out,
                    "g{}/{}/{:?}/{}/{:?}/{kind:?};",
                    spec.width.raw(),
                    spec.stretch.raw(),
                    spec.stretch_order,
                    spec.shrink.raw(),
                    spec.shrink_order
                )
                .expect("write glue"),
                MathNode::Penalty(value) => write!(out, "p{value};").expect("write penalty"),
                MathNode::Rule {
                    width,
                    height,
                    depth,
                } => write!(
                    out,
                    "r{:?}/{:?}/{:?};",
                    width.map(Scaled::raw),
                    height.map(Scaled::raw),
                    depth.map(Scaled::raw)
                )
                .expect("write rule"),
                MathNode::HList(boxed) | MathNode::VList(boxed) => {
                    write!(
                        out,
                        "b{:?}/{}/{}/{}/{}/",
                        boxed.axis,
                        boxed.width.raw(),
                        boxed.height.raw(),
                        boxed.depth.raw(),
                        boxed.shift.raw()
                    )
                    .expect("write box");
                    span(layout, boxed.list, out);
                }
                MathNode::Sequence(child) => span(layout, *child, out),
                MathNode::Opaque(node) => write!(out, "o{node:?};").expect("write opaque"),
            }
        }
        out.push(']');
    }

    let mut out = String::new();
    for layout in layouts {
        span(layout, layout.root(), &mut out);
        out.push('\n');
    }
    out
}

fn all_math_glyphs(layout: &MathLayout) -> Vec<Option<u16>> {
    let mut glyphs = Vec::new();
    let mut stack = vec![layout.root()];
    while let Some(list) = stack.pop() {
        for node in layout.logical_nodes(list) {
            match node {
                MathNode::Char { glyph_id, .. } => glyphs.push(*glyph_id),
                MathNode::HList(boxed) | MathNode::VList(boxed) => stack.push(boxed.list),
                _ => {}
            }
        }
    }
    glyphs
}

#[test]
fn deeply_nested_math_choices_use_an_explicit_work_stack() {
    let mut universe = Universe::new();
    let mut selected = universe.freeze_node_list(&[]);
    for _ in 0..20_000 {
        selected = universe.freeze_node_list(&[Node::MathChoice(MathChoice {
            display: selected,
            text: selected,
            script: selected,
            script_script: selected,
        })]);
    }
    let params = MathParams::read(&universe);

    let layout = mlist_to_hlist(&universe, selected, Style::TEXT, false, &params);

    assert!(layout.root().is_empty());
}

#[test]
fn deeply_nested_sub_mlists_use_an_explicit_work_stack() {
    let mut universe = Universe::new();
    let mut nested = universe.freeze_node_list(&[]);
    for _ in 0..20_000 {
        nested = universe.freeze_node_list(&[Node::MathNoad(MathNoad::new(
            NoadKind::Normal(NoadClass::Ord),
            MathField::SubMlist(nested),
        ))]);
    }
    let params = MathParams::read(&universe);

    let layout = mlist_to_hlist(&universe, nested, Style::TEXT, false, &params);

    assert!(!layout.root().is_empty());
}

#[test]
fn deeply_nested_sub_boxes_use_an_explicit_work_stack() {
    let mut universe = Universe::new();
    let mut children = universe.freeze_node_list(&[]);
    for _ in 0..20_000 {
        let boxed = Node::HList(BoxNode::new(BoxNodeFields {
            width: sc(1),
            height: sc(1),
            depth: sc(0),
            shift: sc(0),
            display: false,
            glue_set: GlueSetRatio::from_raw(0),
            glue_sign: Sign::Normal,
            glue_order: Order::Normal,
            children,
        }));
        children = universe.freeze_node_list(&[boxed]);
    }
    let input = universe.freeze_node_list(&[Node::MathNoad(MathNoad::new(
        NoadKind::Normal(NoadClass::Ord),
        MathField::SubBox(children),
    ))]);
    let params = MathParams::read(&universe);

    let layout = mlist_to_hlist(&universe, input, Style::TEXT, false, &params);

    assert!(!layout.root().is_empty());
}

#[test]
fn math_choice_preserves_the_full_cramped_style() {
    let mut universe = setup_universe();
    let mut scripted = noad(NoadClass::Ord, 'b');
    scripted.superscript = MathField::MathChar(math_char('c'));
    let selected = universe.freeze_node_list(&[Node::MathNoad(scripted)]);
    let choice = universe.freeze_node_list(&[Node::MathChoice(MathChoice {
        display: selected,
        text: selected,
        script: selected,
        script_script: selected,
    })]);
    let params = MathParams::read(&universe);
    let cramped = Style::new(StyleFamily::Text, true);

    let direct = mlist_to_hlist(&universe, selected, cramped, false, &params);
    let through_choice = mlist_to_hlist(&universe, choice, cramped, false, &params);

    assert_eq!(through_choice, direct);
}

#[test]
fn structural_dependency_order_is_deterministic() {
    let mut universe = setup_universe();
    let left = universe.freeze_node_list(&[Node::MathNoad(noad(NoadClass::Ord, 'b'))]);
    let right = universe.freeze_node_list(&[Node::MathNoad(noad(NoadClass::Ord, 'c'))]);
    let input = universe.freeze_node_list(&[
        Node::MathNoad(MathNoad::new(
            NoadKind::Normal(NoadClass::Ord),
            MathField::SubMlist(left),
        )),
        Node::MathNoad(MathNoad::new(
            NoadKind::Normal(NoadClass::Ord),
            MathField::SubMlist(right),
        )),
    ]);
    let params = MathParams::read(&universe);
    let expected = mlist_to_hlist(&universe, input, Style::TEXT, false, &params);

    for _ in 0..32 {
        assert_eq!(
            mlist_to_hlist(&universe, input, Style::TEXT, false, &params),
            expected
        );
    }
}

#[test]
fn delimiter_noad_ignores_structural_fields_during_planning() {
    let mut universe = setup_universe();
    let unused = universe.freeze_node_list(&[Node::MathNoad(noad(NoadClass::Ord, 'b'))]);
    let plain = universe.freeze_node_list(&[Node::MathNoad(MathNoad::new(
        NoadKind::LeftDelimiter { delimiter: 0 },
        MathField::Empty,
    ))]);
    let malformed = universe.freeze_node_list(&[Node::MathNoad(MathNoad {
        kind: NoadKind::LeftDelimiter { delimiter: 0 },
        nucleus: MathField::SubMlist(unused),
        subscript: MathField::SubMlist(unused),
        superscript: MathField::SubMlist(unused),
    })]);
    let params = MathParams::read(&universe);

    assert_eq!(
        mlist_to_hlist(&universe, malformed, Style::TEXT, false, &params),
        mlist_to_hlist(&universe, plain, Style::TEXT, false, &params)
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
    assert_eq!(script_box.shift, sc(15));
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
fn bin_normalization_runs_math_ligatures_before_translation() {
    let mut universe = setup_universe();
    let input = universe.freeze_node_list(&[
        Node::MathNoad(noad(NoadClass::Op, 'a')),
        Node::MathNoad(noad(NoadClass::Bin, 'a')),
        Node::MathNoad(noad(NoadClass::Open, 'a')),
        Node::MathNoad(noad(NoadClass::Punct, 'a')),
    ]);
    let params = MathParams::read(&universe);

    let hlist = mlist_to_hlist(&universe, input, Style::SCRIPT_SCRIPT, false, &params);
    let chars: Vec<_> = root_nodes(&hlist)
        .iter()
        .filter_map(|node| match node {
            MathNode::Char { ch, .. } => Some(*ch),
            _ => None,
        })
        .collect();

    assert_eq!(chars, ['a']);
    assert!(matches!(root_nodes(&hlist)[0], MathNode::HList(_)));
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
fn fraction_rebox_keeps_an_empty_denominator_structurally_empty() {
    let mut universe = setup_universe();
    let numerator = universe.freeze_node_list(&[Node::MathNoad(noad(NoadClass::Ord, 'a'))]);
    let denominator = universe.freeze_node_list(&[]);
    let input = universe.freeze_node_list(&[Node::FractionNoad(MathFraction {
        numerator,
        denominator,
        thickness: FractionThickness::Default,
        left_delimiter: None,
        right_delimiter: None,
    })]);
    let params = MathParams::read(&universe);

    let layout = mlist_to_hlist(&universe, input, Style::TEXT, false, &params);

    let [MathNode::HList(fraction)] = root_nodes(&layout).as_slice() else {
        panic!("expected fraction hbox");
    };
    let [_, MathNode::VList(stack), _] = list_nodes(&layout, fraction.list).as_slice() else {
        panic!("expected delimited fraction stack");
    };
    let [
        MathNode::HList(numerator),
        _,
        _,
        _,
        MathNode::HList(denominator),
    ] = list_nodes(&layout, stack.list).as_slice()
    else {
        panic!("expected ruled fraction stack");
    };
    assert_eq!(denominator.width, numerator.width);
    assert!(list_nodes(&layout, denominator.list).is_empty());
}

#[test]
fn fraction_reuses_single_explicit_numerator_box() {
    let mut universe = setup_universe();
    let children = universe.freeze_node_list(&[]);
    let explicit = Node::HList(BoxNode::new(BoxNodeFields {
        width: sc(40),
        height: sc(12),
        depth: sc(3),
        shift: sc(0),
        display: true,
        glue_set: GlueSetRatio::from_raw(123),
        glue_sign: Sign::Stretching,
        glue_order: Order::Fil,
        children,
    }));
    let explicit_list = universe.freeze_node_list(&[explicit]);
    let field = MathField::SubBox(explicit_list);
    let numerator = universe.freeze_node_list(&[Node::MathNoad(MathNoad::new(
        NoadKind::Normal(NoadClass::Ord),
        field.clone(),
    ))]);
    let denominator = universe.freeze_node_list(&[Node::MathNoad(MathNoad::new(
        NoadKind::Normal(NoadClass::Ord),
        field,
    ))]);
    let input = universe.freeze_node_list(&[Node::FractionNoad(MathFraction {
        numerator,
        denominator,
        thickness: FractionThickness::Default,
        left_delimiter: None,
        right_delimiter: None,
    })]);
    let params = MathParams::read(&universe);

    let layout = mlist_to_hlist(&universe, input, Style::TEXT, false, &params);

    let [MathNode::HList(fraction)] = root_nodes(&layout).as_slice() else {
        panic!("expected fraction hbox");
    };
    let [_, MathNode::VList(stack), _] = list_nodes(&layout, fraction.list).as_slice() else {
        panic!("expected delimited fraction stack");
    };
    let [MathNode::HList(numerator), ..] = list_nodes(&layout, stack.list).as_slice() else {
        panic!("expected numerator box");
    };
    assert!(list_nodes(&layout, numerator.list).is_empty());
    assert!(numerator.display);
    assert_eq!(numerator.glue_set, GlueSetRatio::from_raw(123));
    assert_eq!(numerator.glue_sign, Sign::Stretching);
    assert_eq!(numerator.glue_order, Order::Fil);
}

#[test]
fn fraction_retains_box_around_nested_sub_mlist_nucleus() {
    let mut universe = setup_universe();
    let children = universe.freeze_node_list(&[]);
    let explicit = Node::HList(BoxNode::new(BoxNodeFields {
        width: sc(40),
        height: sc(12),
        depth: sc(3),
        shift: sc(0),
        display: true,
        glue_set: GlueSetRatio::from_raw(123),
        glue_sign: Sign::Stretching,
        glue_order: Order::Fil,
        children,
    }));
    let explicit_list = universe.freeze_node_list(&[explicit]);
    let nested = universe.freeze_node_list(&[Node::MathNoad(MathNoad::new(
        NoadKind::Normal(NoadClass::Ord),
        MathField::SubBox(explicit_list),
    ))]);
    let field = MathField::SubMlist(nested);
    let numerator = universe.freeze_node_list(&[Node::MathNoad(MathNoad::new(
        NoadKind::Normal(NoadClass::Ord),
        field.clone(),
    ))]);
    let denominator = universe.freeze_node_list(&[Node::MathNoad(MathNoad::new(
        NoadKind::Normal(NoadClass::Ord),
        field,
    ))]);
    let input = universe.freeze_node_list(&[Node::FractionNoad(MathFraction {
        numerator,
        denominator,
        thickness: FractionThickness::Default,
        left_delimiter: None,
        right_delimiter: None,
    })]);
    let params = MathParams::read(&universe);

    let layout = mlist_to_hlist(&universe, input, Style::TEXT, false, &params);

    let [MathNode::HList(fraction)] = root_nodes(&layout).as_slice() else {
        panic!("expected fraction hbox");
    };
    let [_, MathNode::VList(stack), _] = list_nodes(&layout, fraction.list).as_slice() else {
        panic!("expected delimited fraction stack");
    };
    let [MathNode::HList(outer), ..] = list_nodes(&layout, stack.list).as_slice() else {
        panic!("expected numerator box");
    };
    let [MathNode::HList(inner)] = list_nodes(&layout, outer.list).as_slice() else {
        panic!("expected retained box around explicit author box");
    };
    assert!(!outer.display);
    assert_eq!(outer.glue_set, GlueSetRatio::from_raw(0));
    assert_eq!(outer.glue_sign, Sign::Normal);
    assert_eq!(outer.glue_order, Order::Normal);
    assert!(inner.display);
    assert_eq!(inner.glue_set, GlueSetRatio::from_raw(123));
    assert_eq!(inner.glue_sign, Sign::Stretching);
    assert_eq!(inner.glue_order, Order::Fil);
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
fn middle_delimiter_uses_common_extent_and_boundary_spacing() {
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
            NoadKind::MiddleDelimiter { delimiter },
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

    assert_eq!(
        nodes.len(),
        5,
        "middle boundaries must not add relation glue"
    );
    for index in [0, 2, 4] {
        let MathNode::VList(delimiter) = nodes[index] else {
            panic!("expected extensible delimiter at index {index}");
        };
        assert!(list_nodes(&hlist, delimiter.list).len() > 3);
    }
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
    let sub_box = universe.freeze_node_list(&[sub_box]);
    let input = universe.freeze_node_list(&[Node::MathNoad(MathNoad::new(
        NoadKind::Normal(NoadClass::Ord),
        MathField::SubBox(sub_box),
    ))]);
    let params = MathParams::read(&universe);

    let hlist = mlist_to_hlist(&universe, input, Style::TEXT, false, &params);

    let logical = hlist.logical_nodes(hlist.root());
    assert_eq!(logical.len(), 1);
    let MathNode::VList(boxed) = logical[0] else {
        panic!("expected source vbox");
    };
    assert_eq!(boxed.width, sc(4));
    assert_eq!(boxed.height, sc(40));
    assert_eq!(boxed.depth, sc(10));
    assert!(list_nodes(&hlist, boxed.list).is_empty());
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
        let MathNode::HList(operator) = node else {
            return false;
        };
        matches!(
            list_nodes(&hlist, operator.list).as_slice(),
            [MathNode::Char { ch: 'O', .. }]
        )
    }));
}

#[test]
fn display_limits_does_not_rewrap_clean_compound_operator() {
    let mut universe = setup_universe();
    let nucleus = universe.freeze_node_list(&[
        Node::MathNoad(noad(NoadClass::Ord, 'b')),
        Node::MathNoad(noad(NoadClass::Ord, 'c')),
        Node::MathNoad(noad(NoadClass::Ord, 'b')),
    ]);
    let op = MathNoad::new(
        NoadKind::Operator(LimitType::Limits),
        MathField::SubMlist(nucleus),
    );
    let input = universe.freeze_node_list(&[Node::MathNoad(op)]);
    let params = MathParams::read(&universe);

    let layout = mlist_to_hlist(&universe, input, Style::DISPLAY, false, &params);

    let [MathNode::VList(limits)] = root_nodes(&layout).as_slice() else {
        panic!("expected displayed-limits vbox");
    };
    let [MathNode::HList(operator)] = list_nodes(&layout, limits.list).as_slice() else {
        panic!("expected one clean compound operator box");
    };
    let operator_nodes = list_nodes(&layout, operator.list);
    assert!(
        operator_nodes
            .iter()
            .any(|node| matches!(node, MathNode::Char { .. }))
    );
    assert!(
        operator_nodes
            .iter()
            .all(|node| !matches!(node, MathNode::HList(_) | MathNode::VList(_)))
    );
}

#[test]
fn display_limits_preserve_same_width_source_vbox_axis() {
    let mut universe = setup_universe();
    let children = universe.freeze_node_list(&[]);
    let source_vbox = Node::VList(BoxNode::new(BoxNodeFields {
        width: sc(30),
        height: sc(40),
        depth: sc(10),
        shift: sc(0),
        display: false,
        glue_set: GlueSetRatio::from_raw(0),
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children,
    }));
    let source_vbox = universe.freeze_node_list(&[source_vbox]);
    let mut op = MathNoad::new(
        NoadKind::Operator(LimitType::Limits),
        MathField::MathChar(math_char('o')),
    );
    op.superscript = MathField::SubBox(source_vbox);
    let input = universe.freeze_node_list(&[Node::MathNoad(op)]);
    let params = MathParams::read(&universe);

    let layout = mlist_to_hlist(&universe, input, Style::TEXT, false, &params);

    let [MathNode::VList(limits)] = root_nodes(&layout).as_slice() else {
        panic!("expected displayed-limits vbox");
    };
    assert!(list_nodes(&layout, limits.list).iter().any(|node| {
        matches!(
            node,
            MathNode::VList(boxed)
                if boxed.width == sc(30) && boxed.height == sc(40) && boxed.depth == sc(10)
        )
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
    assert_eq!(op_box.shift, sc(-1));
}

#[test]
fn nolimits_operator_does_not_center_compound_nucleus() {
    let mut universe = setup_universe();
    let nucleus = universe.freeze_node_list(&[
        Node::MathNoad(noad(NoadClass::Ord, 'l')),
        Node::MathNoad(noad(NoadClass::Ord, 'i')),
        Node::MathNoad(noad(NoadClass::Ord, 'm')),
    ]);
    let op = MathNoad::new(
        NoadKind::Operator(LimitType::NoLimits),
        MathField::SubMlist(nucleus),
    );
    let input = universe.freeze_node_list(&[Node::MathNoad(op)]);
    let params = MathParams::read(&universe);

    let hlist = mlist_to_hlist(&universe, input, Style::TEXT, false, &params);

    let nodes = root_nodes(&hlist);
    let [op_node] = nodes.as_slice() else {
        panic!("expected compound operator hbox");
    };
    let MathNode::HList(op_box) = op_node else {
        panic!("expected compound operator hbox")
    };
    assert_eq!(op_box.shift, sc(0));
}

#[test]
fn nolimits_operator_hpacks_a_single_box_sub_mlist_nucleus() {
    let mut universe = setup_universe();
    let inner = universe.freeze_node_list(&[Node::MathNoad(MathNoad::new(
        NoadKind::Underline,
        MathField::MathChar(math_char('b')),
    ))]);
    let op = MathNoad::new(
        NoadKind::Operator(LimitType::NoLimits),
        MathField::SubMlist(inner),
    );
    let input = universe.freeze_node_list(&[Node::MathNoad(op)]);
    let params = MathParams::read(&universe);

    let layout = mlist_to_hlist(&universe, input, Style::TEXT, false, &params);

    let [MathNode::HList(operator)] = root_nodes(&layout).as_slice() else {
        panic!("expected operator nucleus box");
    };
    let operator_nodes = list_nodes(&layout, operator.list);
    assert!(
        matches!(operator_nodes.as_slice(), [MathNode::VList(_)]),
        "{operator_nodes:#?}"
    );
}

#[test]
fn mathchar_operator_centers_inline_nucleus_and_places_side_scripts() {
    let mut universe = setup_universe();
    let mut op = MathNoad::new(
        NoadKind::Normal(NoadClass::Op),
        MathField::MathChar(math_char('c')),
    );
    op.subscript = MathField::MathChar(math_char('b'));
    op.superscript = MathField::MathChar(math_char('c'));
    let input = universe.freeze_node_list(&[Node::MathNoad(op)]);
    let params = MathParams::read(&universe);

    let hlist = mlist_to_hlist(&universe, input, Style::TEXT, false, &params);

    let nodes = root_nodes(&hlist);
    let [op_node, scripts_node] = nodes.as_slice() else {
        panic!("expected centered operator followed by side scripts");
    };
    let MathNode::HList(op_box) = op_node else {
        panic!("expected operator hbox");
    };
    assert_eq!(op_box.shift, sc(-1));
    assert!(matches!(scripts_node, MathNode::VList(_)));
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
    universe.set_font_skew_char(text_font, i32::from(b'k'));
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

#[test]
fn nested_math_accent_preserves_the_inner_vertical_box() {
    let mut universe = setup_universe();
    let inner = universe.freeze_node_list(&[Node::MathNoad(MathNoad::new(
        NoadKind::Accent {
            accent: math_char('^'),
        },
        MathField::MathChar(math_char('a')),
    ))]);
    let input = universe.freeze_node_list(&[Node::MathNoad(MathNoad::new(
        NoadKind::Accent {
            accent: math_char('^'),
        },
        MathField::SubMlist(inner),
    ))]);
    let params = MathParams::read(&universe);

    let hlist = mlist_to_hlist(&universe, input, Style::TEXT, false, &params);

    let [MathNode::VList(outer)] = root_nodes(&hlist).as_slice() else {
        panic!("expected outer accent vbox");
    };
    let outer_nodes = list_nodes(&hlist, outer.list);
    assert!(matches!(outer_nodes.last(), Some(MathNode::VList(_))));
}

#[test]
fn nested_under_overline_retains_inner_vertical_box() {
    let mut universe = setup_universe();
    let sum = universe.freeze_node_list(&[
        Node::MathNoad(noad(NoadClass::Ord, 'x')),
        Node::MathNoad(noad(NoadClass::Bin, '+')),
        Node::MathNoad(noad(NoadClass::Ord, 'y')),
    ]);
    let overline = universe.freeze_node_list(&[Node::MathNoad(MathNoad::new(
        NoadKind::Overline,
        MathField::SubMlist(sum),
    ))]);
    let input = universe.freeze_node_list(&[Node::MathNoad(MathNoad::new(
        NoadKind::Underline,
        MathField::SubMlist(overline),
    ))]);
    let params = MathParams::read(&universe);

    let layout = mlist_to_hlist(&universe, input, Style::TEXT, false, &params);

    let [MathNode::VList(underline)] = root_nodes(&layout).as_slice() else {
        panic!("expected underline vbox");
    };
    let underline_nodes = list_nodes(&layout, underline.list);
    let [
        MathNode::VList(overline),
        MathNode::Kern { .. },
        MathNode::Rule { .. },
    ] = underline_nodes.as_slice()
    else {
        panic!(
            "expected nested overline vbox followed by underline material: {underline_nodes:#?}"
        );
    };
    assert!(matches!(
        list_nodes(&layout, overline.list).as_slice(),
        [
            MathNode::Kern { .. },
            MathNode::Rule { .. },
            MathNode::Kern { .. },
            MathNode::HList(_)
        ]
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
        origin: tex_state::token::OriginId::UNKNOWN,
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
            next_char: b'a',
            command: Some(LigKernCommand::Ligature(LigatureCommand {
                replacement: b'a',
                delete_current: true,
                delete_next: true,
                pass_over: 0,
            })),
        },
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
