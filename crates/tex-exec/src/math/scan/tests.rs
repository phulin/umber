use super::*;
use crate::push_traced_tokens;
use tex_lex::MemoryInput;
use tex_state::provenance::SyntheticOriginKind;

#[test]
fn invalid_delimiter_pushback_preserves_traced_origin() {
    let mut stores = crate::test_harness::universe();
    crate::install_unexpandable_primitives(&mut stores);
    let origin = stores.synthetic_origin(SyntheticOriginKind::Test);
    let invalid = TracedTokenWord::pack(Token::Param(1), origin);
    let mut input = InputStack::new(MemoryInput::new(""));
    push_traced_tokens(&mut input, &mut stores, [invalid]);

    let delimiter = scan_delimiter_token(
        &mut input,
        &mut stores,
        &mut crate::ExecutionContext::new("texput"),
    )
    .expect("invalid delimiter should recover");

    assert_eq!(delimiter, 0);
    let replayed = input
        .next_traced_token(&mut stores)
        .expect("read recovered token")
        .expect("invalid token should be backed up");
    assert_eq!(tex_expand::semantic_token(replayed), Token::Param(1));
    assert_eq!(replayed.origin(), origin);
}

#[test]
fn delimiter_command_scans_all_twenty_seven_bits() {
    let mut stores = crate::test_harness::universe();
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(r#"\delimiter"7FFFFFF "#));

    let delimiter = scan_delimiter_token(
        &mut input,
        &mut stores,
        &mut crate::ExecutionContext::new("texput"),
    )
    .expect("numeric delimiter should scan");

    assert_eq!(delimiter, 0x07ff_ffff);
}

fn math_nest() -> ModeNest {
    let mut nest = ModeNest::new();
    nest.push(Mode::Math).expect("test mode push");
    nest
}

fn math_stores() -> Universe {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    stores
}

fn pending_terminal_text(stores: &Universe) -> String {
    stores
        .world()
        .effect_records()
        .iter()
        .filter_map(|effect| match effect {
            tex_state::EffectRecord::StreamWrite { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn owned_nodes(stores: &Universe, list: tex_state::ids::NodeListId) -> Vec<Node> {
    stores
        .nodes(list)
        .into_iter()
        .map(|node| node.to_owned())
        .collect()
}

#[test]
fn tex82_math_field_and_atom_construction_matrix() {
    let mut stores = math_stores();
    let mut nest = math_nest();
    let mut execution = crate::ExecutionContext::new("texput");

    let mut bare = InputStack::new(MemoryInput::new("a"));
    let bare = scan_math_field(&mut nest, &mut bare, &mut stores, &mut execution)
        .expect("unbraced field scans");
    let mut grouped = InputStack::new(MemoryInput::new("{a}"));
    let grouped = scan_math_field(&mut nest, &mut grouped, &mut stores, &mut execution)
        .expect("one-atom group scans");
    assert_eq!(grouped, bare, "a sole unscripted Ord field is simplified");

    let mut empty = InputStack::new(MemoryInput::new("{}"));
    let MathField::SubMlist(empty) =
        scan_math_field(&mut nest, &mut empty, &mut stores, &mut execution)
            .expect("empty group scans")
    else {
        panic!("an empty group remains a sub-mlist");
    };
    assert!(stores.nodes(empty).is_empty());

    let mut compound = InputStack::new(MemoryInput::new("{ab}"));
    let MathField::SubMlist(compound) =
        scan_math_field(&mut nest, &mut compound, &mut stores, &mut execution)
            .expect("compound group scans")
    else {
        panic!("a compound group remains a sub-mlist");
    };
    assert_eq!(stores.nodes(compound).len(), 2);

    let constructors = [
        UnexpandablePrimitive::MathOrd,
        UnexpandablePrimitive::MathOp,
        UnexpandablePrimitive::MathBin,
        UnexpandablePrimitive::MathRel,
        UnexpandablePrimitive::MathOpen,
        UnexpandablePrimitive::MathClose,
        UnexpandablePrimitive::MathPunct,
        UnexpandablePrimitive::MathInner,
    ];
    for primitive in constructors {
        append_noad(
            &mut nest,
            crate::math::support::noad_kind_for_constructor(primitive),
            bare.clone(),
        );
    }
    assert_eq!(nest.current_list().nodes().len(), constructors.len());
    for (node, primitive) in nest.current_list().nodes().iter().zip(constructors) {
        let Node::MathNoad(noad) = node else {
            panic!("constructor appends a noad");
        };
        assert_eq!(
            noad.kind,
            crate::math::support::noad_kind_for_constructor(primitive)
        );
        assert_eq!(noad.nucleus, bare);
        assert_eq!(noad.subscript, MathField::Empty);
        assert_eq!(noad.superscript, MathField::Empty);
    }
}

#[test]
fn tex82_radical_accent_style_and_limits_request_matrix() {
    let mut stores = math_stores();
    let mut execution = crate::ExecutionContext::new("texput");
    let mut input = InputStack::new(MemoryInput::new(r#". \delimiter"7FFFFFF "#));
    assert_eq!(
        scan_delimiter_token(&mut input, &mut stores, &mut execution).expect("null delimiter"),
        0
    );
    assert_eq!(
        scan_delimiter_token(&mut input, &mut stores, &mut execution).expect("max delimiter"),
        0x07ff_ffff
    );

    let accent = math_char_from_code(0x1341, &stores, OriginId::UNKNOWN).expect("accent code");
    let mut nest = math_nest();
    append_noad(
        &mut nest,
        NoadKind::Radical {
            delimiter: 0x07ff_ffff,
        },
        MathField::Empty,
    );
    append_noad(
        &mut nest,
        NoadKind::Accent { accent },
        MathField::MathChar(accent),
    );
    append_noad(&mut nest, NoadKind::VCenter, MathField::Empty);
    for primitive in [
        UnexpandablePrimitive::DisplayStyle,
        UnexpandablePrimitive::TextStyle,
        UnexpandablePrimitive::ScriptStyle,
        UnexpandablePrimitive::ScriptScriptStyle,
    ] {
        nest.current_list_mutation().push(Node::MathStyle(
            crate::math::support::style_for_primitive(primitive),
        ));
    }
    append_noad(
        &mut nest,
        NoadKind::Operator(LimitType::DisplayLimits),
        MathField::Empty,
    );
    let limit_input = InputStack::new(MemoryInput::new(""));
    for primitive in [
        UnexpandablePrimitive::Limits,
        UnexpandablePrimitive::NoLimits,
        UnexpandablePrimitive::DisplayLimits,
    ] {
        apply_limit_switch(&mut nest, &limit_input, &mut stores, primitive)
            .expect("limit switch reports no fatal error");
        let Some(Node::MathNoad(noad)) = nest.current_list().nodes().last() else {
            panic!("operator remains last");
        };
        let expected = match primitive {
            UnexpandablePrimitive::Limits => LimitType::Limits,
            UnexpandablePrimitive::NoLimits => LimitType::NoLimits,
            _ => LimitType::DisplayLimits,
        };
        assert_eq!(noad.kind, NoadKind::Operator(expected));
    }

    let before = nest.current_list().nodes().len();
    nest.current_list_mutation().push(Node::Penalty(1));
    apply_limit_switch(
        &mut nest,
        &limit_input,
        &mut stores,
        UnexpandablePrimitive::Limits,
    )
    .expect("limit switch reports no fatal error");
    assert_eq!(nest.current_list().nodes().len(), before + 1);
    assert!(pending_terminal_text(&stores).contains("Limit controls must follow"));
}

#[test]
fn tex82_mathchoice_four_group_order_scope_and_retirement() {
    let mut stores = math_stores();
    let mut nest = math_nest();
    let mut execution = crate::ExecutionContext::new("texput");
    let mut input = InputStack::new(MemoryInput::new("{}{a}{b_c}{\\mathchoice{}{}{}{}}"));
    append_math_choice(&mut nest, &mut input, &mut stores, &mut execution)
        .expect("four choice groups scan");

    let [Node::MathChoice(choice)] = nest.current_list().nodes() else {
        panic!("one choice owns four arms");
    };
    assert!(stores.nodes(choice.display).is_empty());
    assert_eq!(stores.nodes(choice.text).len(), 1);
    assert_eq!(stores.nodes(choice.script).len(), 1);
    assert!(matches!(
        stores.nodes(choice.script_script).first(),
        Some(tex_state::node_arena::NodeRef::MathChoice(_))
    ));
    assert_eq!(stores.innermost_group_kind(), None);
    assert_eq!(nest.depth(), 2);

    // §1172 ends in §403's `scan_left_brace`, which backs the offending token
    // up and then behaves as though the mandatory `{` had been read, so the
    // fourth arm is scanned starting from that token rather than left empty.
    let mut recovered_nest = math_nest();
    let mut recovered = InputStack::new(MemoryInput::new("{a}{b}{c}d}"));
    append_math_choice(
        &mut recovered_nest,
        &mut recovered,
        &mut stores,
        &mut execution,
    )
    .expect("missing fourth opener recovers locally");
    let [Node::MathChoice(choice)] = recovered_nest.current_list().nodes() else {
        panic!("recovered scan still appends one choice");
    };
    assert_eq!(stores.nodes(choice.script_script).len(), 1);
    assert!(pending_terminal_text(&stores).contains("Missing { inserted"));
}

#[test]
fn tex82_script_attachment_dummy_noad_and_duplicate_matrix() {
    let mut stores = math_stores();
    let mut execution = crate::ExecutionContext::new("texput");

    let mut nest = math_nest();
    let mut input = InputStack::new(MemoryInput::new("a"));
    attach_script(&mut nest, &mut input, &mut stores, &mut execution, true)
        .expect("orphan superscript attaches");
    let [Node::MathNoad(dummy)] = nest.current_list().nodes() else {
        panic!("orphan script synthesizes one noad");
    };
    assert_eq!(dummy.nucleus, MathField::Empty);
    assert!(matches!(dummy.superscript, MathField::MathChar(_)));

    let mut paired = math_nest();
    append_noad(
        &mut paired,
        NoadKind::Normal(NoadClass::Ord),
        MathField::Empty,
    );
    let mut sub = InputStack::new(MemoryInput::new("a"));
    attach_script(&mut paired, &mut sub, &mut stores, &mut execution, false)
        .expect("subscript attaches");
    let mut sup = InputStack::new(MemoryInput::new("b"));
    attach_script(&mut paired, &mut sup, &mut stores, &mut execution, true)
        .expect("superscript attaches");
    let [Node::MathNoad(paired)] = paired.current_list().nodes() else {
        panic!("paired scripts share the nucleus noad");
    };
    assert!(matches!(paired.subscript, MathField::MathChar(_)));
    assert!(matches!(paired.superscript, MathField::MathChar(_)));

    let mut duplicate = InputStack::new(MemoryInput::new("c"));
    attach_script(&mut nest, &mut duplicate, &mut stores, &mut execution, true)
        .expect("duplicate recovers");
    let [Node::MathNoad(first), Node::MathNoad(second)] = nest.current_list().nodes() else {
        panic!("duplicate field is redirected to a new dummy noad");
    };
    assert!(matches!(first.superscript, MathField::MathChar(_)));
    assert!(matches!(second.superscript, MathField::MathChar(_)));
    assert!(pending_terminal_text(&stores).contains("Double superscript"));
}

#[test]
fn canonical_fraction_and_left_right_nesting_recovery_matrix() {
    for (primitive, source, expected_thickness, expected_delimiters) in [
        (
            UnexpandablePrimitive::Over,
            "",
            FractionThickness::Default,
            (None, None),
        ),
        (
            UnexpandablePrimitive::Atop,
            "",
            FractionThickness::Explicit(Scaled::from_raw(0)),
            (None, None),
        ),
        (
            UnexpandablePrimitive::Above,
            "3sp ",
            FractionThickness::Explicit(Scaled::from_raw(3)),
            (None, None),
        ),
        (
            UnexpandablePrimitive::OverWithDelims,
            "..",
            FractionThickness::Default,
            (Some(0), Some(0)),
        ),
        (
            UnexpandablePrimitive::AtopWithDelims,
            "..",
            FractionThickness::Explicit(Scaled::from_raw(0)),
            (Some(0), Some(0)),
        ),
        (
            UnexpandablePrimitive::AboveWithDelims,
            "..3sp ",
            FractionThickness::Explicit(Scaled::from_raw(3)),
            (Some(0), Some(0)),
        ),
    ] {
        let mut stores = math_stores();
        let mut nest = math_nest();
        append_noad(
            &mut nest,
            NoadKind::Normal(NoadClass::Ord),
            MathField::Empty,
        );
        let mut input = InputStack::new(MemoryInput::new(source));
        let mut execution = crate::ExecutionContext::new("texput");
        let context = TracedTokenWord::pack(
            Token::Cs(stores.intern("fraction").symbol()),
            OriginId::UNKNOWN,
        );
        start_fraction(
            primitive,
            context,
            &mut nest,
            &mut input,
            &mut stores,
            &mut execution,
        )
        .expect("fraction starts");
        append_noad(
            &mut nest,
            NoadKind::Normal(NoadClass::Ord),
            MathField::Empty,
        );
        let finished = finish_current_math_list(&mut nest, &mut stores);
        let finished_nodes = owned_nodes(&stores, finished);
        let [Node::FractionNoad(fraction)] = finished_nodes.as_slice() else {
            panic!("one fraction owns numerator and denominator");
        };
        assert_eq!(fraction.thickness, expected_thickness);
        assert_eq!(
            (fraction.left_delimiter, fraction.right_delimiter),
            expected_delimiters
        );
        assert_eq!(stores.nodes(fraction.numerator).len(), 1);
        assert_eq!(stores.nodes(fraction.denominator).len(), 1);
    }

    let mut stores = math_stores();
    let mut nest = math_nest();
    let mut input = InputStack::new(MemoryInput::new("..."));
    let mut execution = crate::ExecutionContext::new("texput");
    start_left_group(&mut nest, &mut input, &mut stores, &mut execution)
        .expect("left group starts");
    append_noad(
        &mut nest,
        NoadKind::Normal(NoadClass::Ord),
        MathField::Empty,
    );
    append_middle_delimiter(&mut nest, &mut input, &mut stores, &mut execution)
        .expect("middle appends");
    finish_left_group(&mut nest, &mut input, &mut stores, &mut execution).expect("right closes");
    let [Node::MathNoad(inner)] = nest.current_list().nodes() else {
        panic!("balanced delimiters append one inner noad");
    };
    let MathField::SubMlist(delimited) = inner.nucleus else {
        panic!("inner noad owns the delimited sub-mlist");
    };
    let nodes = owned_nodes(&stores, delimited);
    assert!(matches!(
        nodes.first(),
        Some(Node::MathNoad(MathNoad {
            kind: NoadKind::LeftDelimiter { delimiter: 0 },
            ..
        }))
    ));
    assert!(matches!(
        nodes.get(2),
        Some(Node::MathNoad(MathNoad {
            kind: NoadKind::MiddleDelimiter { delimiter: 0 },
            ..
        }))
    ));
    assert!(matches!(
        nodes.last(),
        Some(Node::MathNoad(MathNoad {
            kind: NoadKind::RightDelimiter { delimiter: 0 },
            ..
        }))
    ));

    let mut missing = math_nest();
    let mut left = InputStack::new(MemoryInput::new("."));
    start_left_group(&mut missing, &mut left, &mut stores, &mut execution).expect("left starts");
    assert!(
        close_missing_left_group(
            &mut missing,
            &left,
            &mut stores,
            tex_command::CommandFuelLedger::default().fuel_mut(),
        )
        .expect("missing right recovers")
    );
    assert!(pending_terminal_text(&stores).contains("Missing \\right. inserted"));

    let mut extra = math_nest();
    let mut right = InputStack::new(MemoryInput::new("."));
    finish_left_group(&mut extra, &mut right, &mut stores, &mut execution)
        .expect("extra right is diagnosed");
    assert!(extra.current_list().is_empty());
    assert!(pending_terminal_text(&stores).contains("Extra \\right"));
}
