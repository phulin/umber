use super::support::terminal_effect_text;
use super::*;
use tex_state::math::{
    FractionThickness, LimitType, MathChoice, MathField, MathListNode, MathNoad, NoadClass,
    NoadKind,
};
use tex_state::node::{GlueKind, KernKind, Node};
use tex_state::provenance::{InsertedOriginKind, OriginRecord};

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
    assert_math_char(&noad.nucleus, 1, 'a');
    assert_math_char(&noad.subscript, 1, 'b');
    assert_math_char(&noad.superscript, 1, 'c');

    assert!(matches!(
        math_noad(&nodes[1]).kind,
        tex_state::math::NoadKind::Normal(tex_state::math::NoadClass::Bin)
    ));

    let op = math_noad(&nodes[2]);
    assert!(matches!(
        op.kind,
        tex_state::math::NoadKind::Operator(LimitType::Limits)
    ));
    assert_math_char(&op.nucleus, 1, 'x');
    assert_math_char(&op.subscript, 1, 'y');

    let overline = math_noad(&nodes[3]);
    assert!(matches!(overline.kind, tex_state::math::NoadKind::Overline));
    assert_math_char(&overline.nucleus, 1, 'z');

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
fn semi_simple_groups_execute_assignments_and_aftergroup_in_math_mode() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        r"\def\after{\global\count2=7}\count0=1\count1=1$\begingroup\count0=2\global\count1=3\aftergroup\after\endgroup$",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("semi-simple math group should execute");

    assert_eq!(stores.count(0), 1, "local assignment should be restored");
    assert_eq!(stores.count(1), 3, "global assignment should survive");
    assert_eq!(
        stores.count(2),
        7,
        "aftergroup token should replay in math mode"
    );
}

#[test]
fn semi_simple_math_aftergroup_replay_has_aftergroup_provenance() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        r"$\begingroup\aftergroup\missing\endgroup",
    ));

    let err = Executor::new()
        .run(&mut input, &mut stores)
        .expect_err("replayed undefined control sequence should fail");
    let origin = err.primary_origin().expect("replayed token origin");
    let OriginRecord::Inserted(inserted) = stores.origin(origin) else {
        panic!("aftergroup replay should have inserted provenance");
    };
    assert_eq!(inserted.kind(), InsertedOriginKind::AfterGroup);
    assert_ne!(inserted.parent(), OriginId::UNKNOWN);
}

#[test]
fn plain_active_prime_shape_closes_brace_alias_math_field() {
    let (stores, executor) = run_math_source(
        r"\let\bgroup={\let\egroup=}\def\prime{p}\def\prim@s{\prime\futurelet\next\pr@m@s}\def\pr@m@s{\let\nxt\egroup\nxt}$x^\bgroup\prim@s$",
    );
    let nodes = math_nodes(&stores, &executor);

    assert_eq!(nodes.len(), 1);
    let noad = math_noad(&nodes[0]);
    assert_math_char(&noad.nucleus, 1, 'x');
    assert_math_char(&noad.superscript, 1, 'p');
}

#[test]
fn math_field_groups_remove_braces_around_single_unscripted_ord_box() {
    let (stores, executor) = run_math_source(r"$\mathopen{{\hbox{}}}$");
    let nodes = math_nodes(&stores, &executor);

    let [node] = nodes else {
        panic!("expected one math-open noad")
    };
    let noad = math_noad(node);
    assert!(matches!(noad.kind, NoadKind::Normal(NoadClass::Open)));
    let MathField::SubBox(list) = noad.nucleus else {
        panic!("TeX's math-group simplification should expose the hbox nucleus")
    };
    assert!(matches!(stores.nodes(list), [Node::HList(_)]));
}

#[test]
fn math_group_mismatch_reports_the_closing_token_origin() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(r"$\begingroup}"));

    let err = Executor::new()
        .run(&mut input, &mut stores)
        .expect_err("right brace cannot close a semi-simple math group");

    assert!(matches!(
        &err,
        ExecError::ExtraRightBraceOrForgottenEndgroup { .. }
    ));
    assert_ne!(err.primary_origin(), Some(OriginId::UNKNOWN));

    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(r"$\endgroup"));
    let err = Executor::new()
        .run(&mut input, &mut stores)
        .expect_err("endgroup cannot close the outer math level");

    assert!(matches!(&err, ExecError::ExtraEndGroup { .. }));
    assert_ne!(err.primary_origin(), Some(OriginId::UNKNOWN));
}

#[test]
fn math_brace_groups_restore_local_box_assignments_and_keep_globals() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let baseline = stores.freeze_node_list(&[Node::Kern {
        amount: tex_state::scaled::Scaled::from_raw(17),
        kind: KernKind::Explicit,
    }]);
    stores.set_box_reg(0, baseline);

    Executor::new()
        .run(
            &mut InputStack::new(MemoryInput::new(
                r"${\setbox0=\hbox{x}\global\setbox1=\hbox{y}}$",
            )),
            &mut stores,
        )
        .expect("assignments in a math brace group should execute");

    let restored = stores.box_reg(0).expect("local box should be restored");
    assert!(matches!(
        stores.nodes(restored),
        [Node::Kern { amount, kind: KernKind::Explicit }] if amount.raw() == 17
    ));
    assert!(stores.box_reg(1).is_some(), "global box should survive");
}

#[test]
fn explicit_groups_in_math_restore_local_box_assignments_and_keep_globals() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let baseline = stores.freeze_node_list(&[Node::Kern {
        amount: tex_state::scaled::Scaled::from_raw(23),
        kind: KernKind::Explicit,
    }]);
    stores.set_box_reg(0, baseline);

    Executor::new()
        .run(
            &mut InputStack::new(MemoryInput::new(
                r"$\begingroup\setbox0=\hbox{x}\global\setbox1=\hbox{y}\endgroup$",
            )),
            &mut stores,
        )
        .expect("explicit groups in math mode should execute");

    let restored = stores.box_reg(0).expect("local box should be restored");
    assert!(matches!(
        stores.nodes(restored),
        [Node::Kern { amount, kind: KernKind::Explicit }] if amount.raw() == 23
    ));
    assert!(stores.box_reg(1).is_some(), "global box should survive");
}

#[test]
fn penalty_builds_ordinary_list_material_in_inline_math() {
    let (stores, executor) = run_math_source(r"$a\penalty123 b$");
    let nodes = math_nodes(&stores, &executor);

    assert!(matches!(
        nodes,
        [Node::MathNoad(_), Node::Penalty(123), Node::MathNoad(_)]
    ));
}

#[test]
fn penalty_builds_ordinary_list_material_in_display_math() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(r"\noindent$$a\penalty456 b"));
    let mut executor = Executor::new();

    executor
        .run(&mut input, &mut stores)
        .expect("penalty should execute in display math");

    assert_eq!(executor.nest().current_mode(), Mode::DisplayMath);
    assert!(matches!(
        executor.nest().current_list().nodes(),
        [Node::MathNoad(_), Node::Penalty(456), Node::MathNoad(_)]
    ));
}

#[test]
fn lowered_math_box_rolls_back_without_leaking_arena_handles() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let baseline = stores.freeze_node_list(&[Node::Kern {
        amount: tex_state::scaled::Scaled::from_raw(17),
        kind: KernKind::Explicit,
    }]);
    stores.set_box_reg(0, baseline);
    let snapshot = stores.snapshot();
    let before = snapshot.state_hash();

    Executor::new()
        .run(
            &mut InputStack::new(MemoryInput::new(
                r"\setbox0=\hbox{$\left({a+b\over c_d^e}\right)$}",
            )),
            &mut stores,
        )
        .expect("nested math box should lower into epoch-owned node lists");
    let converted = stores.box_reg(0).expect("converted box should be assigned");
    assert_ne!(converted, baseline);
    assert_ne!(stores.snapshot().state_hash(), before);

    stores.rollback(&snapshot);

    assert_eq!(stores.snapshot().state_hash(), before);
    let restored = stores.box_reg(0).expect("baseline box should be restored");
    assert!(matches!(
        stores.nodes(restored),
        [Node::Kern { amount, kind: KernKind::Explicit }] if amount.raw() == 17
    ));
}

#[test]
fn mathcode_8000_uses_current_active_meaning_and_fam_overrides_variable_family() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    stores.set_mathcode('?', 0x8000);
    let active_question = stores.intern_active_character('?');
    stores.set_meaning(active_question, Meaning::MathCharGiven(0x0231));

    let mut input = InputStack::new(MemoryInput::new(
        r#"\mathcode`x="7131 $?$ $\fam=5 x$ $x^?$"#,
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
    assert_math_char(&math_noad(&third[0]).nucleus, 1, '1');
    assert_math_char(&math_noad(&third[0]).superscript, 2, '1');
}

#[test]
fn initex_letter_mathcodes_use_variable_family_one_and_honor_fam() {
    let (stores, executor) = run_math_source(r"$a$ $\fam=2 S$");
    assert_eq!(stores.mathcode('a'), 0x7161);
    assert_eq!(stores.mathcode('S'), 0x7153);
    let math_lists = math_list_nodes(&executor);

    let default = stores.nodes(math_lists[0].content);
    assert_math_char(&math_noad(&default[0]).nucleus, 1, 'a');

    let overridden = stores.nodes(math_lists[1].content);
    assert_math_char(&math_noad(&overridden[0]).nucleus, 2, 'S');
}

#[test]
fn showlists_reports_unfinished_math_noad_fields() {
    let (stores, _) = run_math_source(r"$a_b^c\mathchoice{d}{t}{s}{u}\showlists$");
    let log = terminal_effect_text(&stores);

    assert!(log.contains("### math mode entered at line 0"));
    assert!(log.contains("\\mathord"));
    assert!(log.contains(".\\fam1 a"));
    assert!(log.contains("^\\fam1 c"));
    assert!(log.contains("_\\fam1 b"));
    assert!(log.contains("\\mathchoice"));
}

#[test]
fn par_in_math_finishes_math_with_tex_error_text() {
    let (stores, executor) = run_math_source(r"$a\par");
    let nodes = math_nodes(&stores, &executor);

    assert_eq!(nodes.len(), 1);
    assert_math_char(&math_noad(&nodes[0]).nucleus, 1, 'a');
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
    assert_math_char(&math_noad(&enclosed[1]).nucleus, 1, 'a');
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
    assert_math_char(&math_noad(&extra_nodes[0]).nucleus, 1, 'a');
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
        r#"$\delimiter"4266308 \radical"270370 x \mathaccent"7013 y \vcenter{\hrule width1pt}$"#,
    );
    let nodes = math_nodes(&stores, &executor);

    assert_eq!(nodes.len(), 4);
    assert!(matches!(
        math_noad(&nodes[0]).kind,
        tex_state::math::NoadKind::Normal(tex_state::math::NoadClass::Open)
    ));
    assert_math_char(&math_noad(&nodes[0]).nucleus, 2, 'f');

    let radical = math_noad(&nodes[1]);
    assert!(matches!(
        radical.kind,
        tex_state::math::NoadKind::Radical {
            delimiter: 0x270370
        }
    ));
    assert_math_char(&radical.nucleus, 1, 'x');

    let accent = math_noad(&nodes[2]);
    assert!(matches!(
        accent.kind,
        tex_state::math::NoadKind::Accent { .. }
    ));
    assert_math_char(&accent.nucleus, 1, 'y');

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
