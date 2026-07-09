use super::support::terminal_effect_text;
use super::*;
use tex_state::math::{
    FractionThickness, LimitType, MathChoice, MathField, MathListNode, MathNoad, NoadClass,
    NoadKind,
};
use tex_state::node::{GlueKind, KernKind, Node};

#[test]
fn math_mode_builds_noads_styles_choices_and_mu_nodes() {
    let (stores, executor) = run_math_source(
        r"$a_b^c\mathbin+\mathop{x}\limits_y\overline{z}\mskip3mu\mkern2mu\nonscript\displaystyle\mathchoice{d}{t}{s}{u}$",
    );
    let nodes = math_nodes(&stores, &executor);

    assert_eq!(nodes.len(), 9);
    let noad = math_noad(&nodes[0]);
    assert!(matches!(
        noad.kind,
        tex_state::math::NoadKind::Normal(tex_state::math::NoadClass::Ord)
    ));
    assert_math_char(&noad.nucleus, 0, 'a');
    assert_math_char(&noad.subscript, 0, 'b');
    assert_math_char(&noad.superscript, 0, 'c');

    assert!(matches!(
        math_noad(&nodes[1]).kind,
        tex_state::math::NoadKind::Normal(tex_state::math::NoadClass::Bin)
    ));

    let op = math_noad(&nodes[2]);
    assert!(matches!(
        op.kind,
        tex_state::math::NoadKind::Operator(LimitType::Limits)
    ));
    assert!(matches!(op.nucleus, MathField::SubMlist(_)));
    assert_math_char(&op.subscript, 0, 'y');

    let overline = math_noad(&nodes[3]);
    assert!(matches!(overline.kind, tex_state::math::NoadKind::Overline));
    let MathField::SubMlist(overline_list) = overline.nucleus else {
        panic!("expected grouped overline nucleus");
    };
    assert_one_char_list(&stores, overline_list, 'z');

    assert!(matches!(
        nodes[4],
        Node::Glue {
            kind: GlueKind::MuSkip,
            ..
        }
    ));
    assert!(matches!(
        nodes[5],
        Node::Kern {
            kind: KernKind::Mu,
            ..
        }
    ));
    assert!(matches!(
        nodes[6],
        Node::Glue {
            kind: GlueKind::NonScript,
            ..
        }
    ));
    assert!(matches!(
        nodes[7],
        Node::MathStyle(tex_state::math::MathStyle::Display)
    ));

    let Node::MathChoice(MathChoice {
        display,
        text,
        script,
        script_script,
    }) = nodes[8]
    else {
        panic!("expected math choice");
    };
    assert_one_char_list(&stores, display, 'd');
    assert_one_char_list(&stores, text, 't');
    assert_one_char_list(&stores, script, 's');
    assert_one_char_list(&stores, script_script, 'u');
}

#[test]
fn generalized_fraction_absorbs_prior_list_and_reports_doubled_fraction() {
    let (stores, executor) = run_math_source(r"$a\over b\over c$");
    let nodes = math_nodes(&stores, &executor);

    assert_eq!(nodes.len(), 1);
    let Node::FractionNoad(fraction) = &nodes[0] else {
        panic!("expected fraction noad");
    };
    assert_eq!(fraction.thickness, FractionThickness::Default);
    assert_one_char_list(&stores, fraction.numerator, 'a');
    assert_char_list(&stores, fraction.denominator, &['b', 'c']);
    assert!(
        terminal_effect_text(&stores).contains("! Ambiguous; you need another { and }."),
        "doubled fraction should emit TeX's ambiguity diagnostic"
    );
}

#[test]
fn grouped_fraction_inside_hbox_keeps_box_brace_accounting_balanced() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        r"\setbox0=\hbox{${a+b\over c+d}$}\setbox1=\hbox{$x$}",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("grouped fraction in hbox should not leak braces");

    let Some(box0) = stores.box_reg(0) else {
        panic!("first hbox should be assigned");
    };
    assert!(
        matches!(stores.nodes(box0), [Node::HList(_)]),
        "first hbox should be stored as an hlist"
    );
    let Some(box1) = stores.box_reg(1) else {
        panic!("following hbox should still parse after grouped math");
    };
    assert!(
        matches!(stores.nodes(box1), [Node::HList(_)]),
        "second hbox should be stored as an hlist"
    );
}

#[test]
fn mathcode_8000_uses_current_active_meaning_and_fam_overrides_variable_family() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    stores.set_mathcode('?', 0x8000);
    let active_question = stores.intern("?");
    stores.set_meaning(active_question, Meaning::MathCharGiven(0x0231));

    let mut input = InputStack::new(MemoryInput::new(
        r#"\fam=5 \mathcode`x="7131 $?$ $x$ $x^?$"#,
    ));
    let mut executor = Executor::new();
    executor
        .run(&mut input, &mut stores)
        .expect("mathcode source executes");
    let math_lists = math_list_nodes(&executor);

    let first = stores.nodes(math_lists[0].content);
    assert_eq!(first.len(), 1);
    assert_math_char(&math_noad(&first[0]).nucleus, 2, '1');

    let second = stores.nodes(math_lists[1].content);
    assert_eq!(second.len(), 1);
    assert_math_char(&math_noad(&second[0]).nucleus, 5, '1');

    let third = stores.nodes(math_lists[2].content);
    assert_eq!(third.len(), 1);
    assert_math_char(&math_noad(&third[0]).superscript, 2, '1');
}

#[test]
fn showlists_reports_unfinished_math_noad_fields() {
    let (stores, _) = run_math_source(r"$a_b^c\mathchoice{d}{t}{s}{u}\showlists$");
    let log = terminal_effect_text(&stores);

    assert!(log.contains("### math mode entered at line 0"));
    assert!(log.contains("\\mathord"));
    assert!(log.contains(".\\fam0 a"));
    assert!(log.contains("^\\fam0 c"));
    assert!(log.contains("_\\fam0 b"));
    assert!(log.contains("\\mathchoice"));
}

#[test]
fn par_in_math_finishes_math_with_tex_error_text() {
    let (stores, executor) = run_math_source(r"$a\par");
    let nodes = math_nodes(&stores, &executor);

    assert_eq!(nodes.len(), 1);
    assert_math_char(&math_noad(&nodes[0]).nucleus, 0, 'a');
    assert!(terminal_effect_text(&stores).contains("! Missing $ inserted."));
}

#[test]
fn left_right_scans_nested_list_as_inner_noad() {
    let (stores, executor) = run_math_source(r"$\left. a \right.$");
    let nodes = math_nodes(&stores, &executor);

    assert_eq!(nodes.len(), 1);
    let inner = math_noad(&nodes[0]);
    assert!(matches!(
        inner.kind,
        tex_state::math::NoadKind::Normal(tex_state::math::NoadClass::Inner)
    ));
    let MathField::SubMlist(list) = inner.nucleus else {
        panic!("expected left/right inner noad to hold a sub-mlist");
    };
    let enclosed = stores.nodes(list);
    assert!(matches!(
        math_noad(&enclosed[0]).kind,
        tex_state::math::NoadKind::LeftDelimiter { delimiter: 0 }
    ));
    assert_math_char(&math_noad(&enclosed[1]).nucleus, 0, 'a');
    assert!(matches!(
        math_noad(&enclosed[2]).kind,
        tex_state::math::NoadKind::RightDelimiter { delimiter: 0 }
    ));
}

#[test]
fn mismatched_right_and_missing_right_use_tex_error_text() {
    let (extra_stores, extra_executor) = run_math_source(r"$a\right.$");
    let extra_nodes = math_nodes(&extra_stores, &extra_executor);
    assert_eq!(extra_nodes.len(), 1);
    assert_math_char(&math_noad(&extra_nodes[0]).nucleus, 0, 'a');
    assert!(terminal_effect_text(&extra_stores).contains("! Extra \\right."));

    let (missing_stores, missing_executor) = run_math_source(r"$\left. a$");
    let missing_nodes = math_nodes(&missing_stores, &missing_executor);
    assert_eq!(missing_nodes.len(), 1);
    assert!(matches!(
        math_noad(&missing_nodes[0]).kind,
        tex_state::math::NoadKind::Normal(tex_state::math::NoadClass::Inner)
    ));
    assert!(
        terminal_effect_text(&missing_stores).contains("! Missing \\right. inserted."),
        "missing right delimiter should use reference primary wording"
    );
}

#[test]
fn inline_math_finishing_emits_mathsurround_markers_and_penalties() {
    let (mut stores, executor) = run_math_source(
        r"\mathsurround=3pt \binoppenalty=700 \relpenalty=500 $a\mathbin+b\mathrel=c$",
    );
    let list = math_list_nodes(&executor)
        .pop()
        .expect("inline math list should be present");

    let nodes = crate::math::finish_math_list_node(&mut stores, list, true);

    assert!(matches!(
        nodes.first(),
        Some(Node::MathOn(width)) if width.raw() == 3 * tex_state::scaled::Scaled::UNITY
    ));
    assert!(matches!(
        nodes.last(),
        Some(Node::MathOff(width)) if width.raw() == 3 * tex_state::scaled::Scaled::UNITY
    ));
    assert!(
        nodes.iter().any(|node| matches!(node, Node::Penalty(700))),
        "binoppenalty should be inserted in outer inline conversion"
    );
    assert!(
        nodes.iter().any(|node| matches!(node, Node::Penalty(500))),
        "relpenalty should be inserted in outer inline conversion"
    );
    assert!(
        nodes.iter().all(|node| !matches!(node, Node::MathList(_))),
        "paragraph line breaking must see converted hlist nodes"
    );
}

#[test]
fn restricted_inline_math_finishing_suppresses_line_break_penalties() {
    let (mut stores, executor) = run_math_source(r"$a\mathbin+b\mathrel=c$");
    let list = math_list_nodes(&executor)
        .pop()
        .expect("inline math list should be present");

    let nodes = crate::math::finish_math_list_node(&mut stores, list, false);

    assert!(
        nodes
            .iter()
            .all(|node| !matches!(node, Node::Penalty(700 | 500))),
        "restricted hbox math conversion should not emit line-break penalties"
    );
}

#[test]
fn converted_math_glue_preserves_explicit_and_named_provenance() {
    let mut stores = Universe::new();
    let explicit = stores.intern_glue(tex_state::glue::GlueSpec::ZERO);
    let content = stores.freeze_node_list(&[
        Node::MathNoad(MathNoad::new(
            NoadKind::Normal(NoadClass::Ord),
            MathField::Empty,
        )),
        Node::MathNoad(MathNoad::new(
            NoadKind::Operator(LimitType::NoLimits),
            MathField::Empty,
        )),
        Node::MathNoad(MathNoad::new(
            NoadKind::Normal(NoadClass::Ord),
            MathField::Empty,
        )),
        Node::MathNoad(MathNoad::new(
            NoadKind::Normal(NoadClass::Bin),
            MathField::Empty,
        )),
        Node::MathNoad(MathNoad::new(
            NoadKind::Normal(NoadClass::Ord),
            MathField::Empty,
        )),
        Node::MathNoad(MathNoad::new(
            NoadKind::Normal(NoadClass::Rel),
            MathField::Empty,
        )),
        Node::Glue {
            spec: explicit,
            kind: GlueKind::MuSkip,
            leader: None,
        },
    ]);
    let list = MathListNode {
        display: false,
        content,
    };

    let nodes = crate::math::finish_math_list_node(&mut stores, list, true);

    assert!(
        nodes.iter().any(|node| matches!(
            node,
            Node::Glue {
                kind: GlueKind::ThinMuSkip,
                ..
            }
        )),
        "ord-op spacing should lower as named thinmuskip"
    );
    assert!(
        nodes.iter().any(|node| matches!(
            node,
            Node::Glue {
                kind: GlueKind::MedMuSkip,
                ..
            }
        )),
        "ord-bin spacing should lower as named medmuskip"
    );
    assert!(
        nodes.iter().any(|node| matches!(
            node,
            Node::Glue {
                kind: GlueKind::ThickMuSkip,
                ..
            }
        )),
        "ord-rel spacing should lower as named thickmuskip"
    );
    assert!(
        nodes.iter().any(|node| matches!(
            node,
            Node::Glue {
                kind: GlueKind::MuSkip,
                ..
            }
        )),
        "explicit \\mskip should remain plain mu skip provenance"
    );
}

#[test]
fn delimiter_radical_accent_and_vcenter_parse_to_math_noads() {
    let (stores, executor) = run_math_source(
        r#"$\delimiter"1234 \radical"270370 x \mathaccent"7013 y \vcenter{\hrule width1pt}$"#,
    );
    let nodes = math_nodes(&stores, &executor);

    assert_eq!(nodes.len(), 4);
    assert_math_char(&math_noad(&nodes[0]).nucleus, 2, '4');

    let radical = math_noad(&nodes[1]);
    assert!(matches!(
        radical.kind,
        tex_state::math::NoadKind::Radical {
            delimiter: 0x270370
        }
    ));
    assert_math_char(&radical.nucleus, 0, 'x');

    let accent = math_noad(&nodes[2]);
    assert!(matches!(
        accent.kind,
        tex_state::math::NoadKind::Accent { .. }
    ));
    assert_math_char(&accent.nucleus, 0, 'y');

    let vcenter = math_noad(&nodes[3]);
    assert!(matches!(vcenter.kind, tex_state::math::NoadKind::VCenter));
    let MathField::SubBox(list) = vcenter.nucleus else {
        panic!("expected vcenter sub-box field");
    };
    assert!(matches!(stores.nodes(list)[0], Node::VList(_)));
}

#[test]
fn every_math_and_every_display_tokens_are_inserted_on_entry() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    let displaystyle = stores.symbol("displaystyle").expect("displaystyle");
    let every_math = stores.intern_token_list(&[Token::Cs(displaystyle)]);
    stores.set_tok_param(TokParam::EVERY_MATH, every_math);
    let mut input = InputStack::new(MemoryInput::new("$a$"));
    let mut executor = Executor::new();
    executor
        .run(&mut input, &mut stores)
        .expect("math source executes");
    let lists = math_list_nodes(&executor);

    assert_eq!(lists.len(), 1);
    assert!(!lists[0].display);
    assert!(matches!(
        stores.nodes(lists[0].content)[0],
        Node::MathStyle(tex_state::math::MathStyle::Display)
    ));

    let (display_stores, _) = run_math_source(r"\everydisplay{\message{ED}}\noindent$$b$$\end");
    let display_output = String::from_utf8_lossy(
        display_stores
            .world()
            .memory_terminal_output()
            .expect("memory terminal output"),
    );
    assert!(display_output.contains("ED"));
}

fn run_math_source(source: &str) -> (Universe, Executor) {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    stores.set_int_param(IntParam::SHOW_BOX_BREADTH, 100);
    stores.set_int_param(IntParam::SHOW_BOX_DEPTH, 100);
    let mut input = InputStack::new(MemoryInput::new(source));
    let mut executor = Executor::new();
    executor
        .run(&mut input, &mut stores)
        .expect("math source executes");
    (stores, executor)
}

fn math_nodes<'a>(stores: &'a Universe, executor: &Executor) -> &'a [Node] {
    let lists = math_list_nodes(executor);
    assert_eq!(lists.len(), 1);
    stores.nodes(lists[0].content)
}

fn math_list_nodes(executor: &Executor) -> Vec<MathListNode> {
    executor
        .nest()
        .current_list()
        .nodes()
        .iter()
        .filter_map(|node| match node {
            Node::MathList(list) => Some(*list),
            _ => None,
        })
        .collect()
}

fn math_noad(node: &Node) -> &tex_state::math::MathNoad {
    match node {
        Node::MathNoad(noad) => noad,
        other => panic!("expected noad, got {other:?}"),
    }
}

fn assert_math_char(field: &MathField, family: u8, character: char) {
    match field {
        MathField::MathChar(ch) => {
            assert_eq!(ch.family, family);
            assert_eq!(ch.character, character);
        }
        other => panic!("expected math char field, got {other:?}"),
    }
}

fn assert_one_char_list(stores: &Universe, list: tex_state::ids::NodeListId, character: char) {
    assert_char_list(stores, list, &[character]);
}

fn assert_char_list(stores: &Universe, list: tex_state::ids::NodeListId, expected: &[char]) {
    let actual: Vec<_> = stores
        .nodes(list)
        .iter()
        .map(|node| {
            let noad = math_noad(node);
            match &noad.nucleus {
                MathField::MathChar(ch) => ch.character,
                other => panic!("expected math char nucleus, got {other:?}"),
            }
        })
        .collect();
    assert_eq!(actual, expected);
}
