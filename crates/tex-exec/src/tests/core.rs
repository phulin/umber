use super::support::*;
use super::*;

#[test]
fn nest_push_pop_and_summary_cover_all_modes() {
    let mut nest = ModeNest::new();
    for mode in [
        Mode::InternalVertical,
        Mode::Horizontal,
        Mode::RestrictedHorizontal,
        Mode::Math,
        Mode::DisplayMath,
    ] {
        nest.push(mode);
    }

    assert_eq!(nest.depth(), 6);
    assert_eq!(nest.current_mode(), Mode::DisplayMath);

    let summary = nest.summary();
    let restored = ModeNest::from_summary(summary.clone()).expect("valid summary");
    assert_eq!(restored.summary(), summary);

    assert_eq!(nest.pop().expect("display math").mode(), Mode::DisplayMath);
    assert_eq!(nest.pop().expect("math").mode(), Mode::Math);
    assert_eq!(
        nest.pop().expect("restricted h").mode(),
        Mode::RestrictedHorizontal
    );
    assert_eq!(nest.pop().expect("h").mode(), Mode::Horizontal);
    assert_eq!(
        nest.pop().expect("internal v").mode(),
        Mode::InternalVertical
    );
    assert_eq!(
        nest.pop().expect_err("base cannot pop").to_string(),
        "cannot pop the base vertical mode level"
    );
}

#[test]
fn mode_queries_are_backed_by_current_nest_level() {
    let mut executor = Executor::new();
    assert_eq!(
        <Executor as ExpansionHooks<MemoryInput>>::mode(&executor),
        EngineMode::Vertical
    );
    assert!(!<Executor as ExpansionHooks<MemoryInput>>::is_inner_mode(
        &executor
    ));

    executor.nest_mut().push(Mode::RestrictedHorizontal);
    assert_eq!(
        <Executor as ExpansionHooks<MemoryInput>>::mode(&executor),
        EngineMode::Horizontal
    );
    assert!(<Executor as ExpansionHooks<MemoryInput>>::is_inner_mode(
        &executor
    ));

    executor.nest_mut().push(Mode::DisplayMath);
    assert_eq!(
        <Executor as ExpansionHooks<MemoryInput>>::mode(&executor),
        EngineMode::Math
    );
    assert!(!<Executor as ExpansionHooks<MemoryInput>>::is_inner_mode(
        &executor
    ));
}

#[test]
fn dispatch_relax_continues_without_state_mutation() {
    let mut stores = Universe::new();
    let relax = stores.intern("relax");
    stores.set_meaning(relax, Meaning::Relax);
    let mut input = InputStack::new(MemoryInput::new(""));
    let mut hooks = NoopExecHooks;

    assert_eq!(
        dispatch_delivered_token(
            &mut ModeNest::new(),
            Token::Cs(relax),
            &mut input,
            &mut stores,
            &mut hooks
        )
        .expect("relax dispatch"),
        DispatchAction::Continue
    );
}

#[test]
fn dispatch_character_hits_loud_typesetting_stub() {
    let mut stores = Universe::new();
    let token = Token::Char {
        ch: 'x',
        cat: Catcode::Letter,
    };
    let mut input = InputStack::new(MemoryInput::new(""));
    let mut hooks = NoopExecHooks;
    let mut nest = ModeNest::new();

    nest.push(Mode::Horizontal);
    assert_eq!(
        dispatch_delivered_token(&mut nest, token, &mut input, &mut stores, &mut hooks)
            .expect("character dispatch"),
        DispatchAction::Continue
    );
}

#[test]
fn main_control_uses_get_x_token_and_expands_macros_before_dispatch() {
    let mut stores = Universe::new();
    let relax = stores.intern("relax");
    stores.set_meaning(relax, Meaning::Relax);
    let mut input = InputStack::new(MemoryInput::new("\\relax"));

    let stats = Executor::new()
        .run(&mut input, &mut stores)
        .expect("execution succeeds");
    assert_eq!(stats.delivered_tokens, 1);
}

#[test]
fn def_and_gdef_assign_macro_meanings_through_group_barrier() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\def\\a{A}\\gdef\\b{B}"));
    stores.enter_group();

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("definitions execute");
    let a = stores.symbol("a").expect("a was interned");
    let b = stores.symbol("b").expect("b was interned");
    assert!(matches!(stores.meaning(a), Meaning::Macro { .. }));
    assert!(matches!(stores.meaning(b), Meaning::Macro { .. }));

    let _ = stores.leave_group();
    assert_eq!(stores.meaning(a), Meaning::Undefined);
    assert!(matches!(stores.meaning(b), Meaning::Macro { .. }));
}

#[test]
fn edef_omits_noexpand_command_and_freezes_the_output() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    install_expandable(&mut stores, "noexpand", ExpandablePrimitive::NoExpand);
    install_expandable(&mut stores, "the", ExpandablePrimitive::The);
    stores.intern("toks");
    stores.set_int_param(IntParam::END_LINE_CHAR, -1);
    let a = stores.intern("a");
    let b = stores.intern("b");
    let toks_body = stores.intern_token_list(&[Token::Cs(b)]);
    stores.set_toks(0, toks_body);
    let mut input = InputStack::new(MemoryInput::new("\\edef\\e{\\noexpand\\a\\the\\toks0}"));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("edef executes");
    let e = stores.symbol("e").expect("e was interned");
    let meaning = stores.macro_meaning(e).expect("e is a macro");

    assert_eq!(
        stores.tokens(meaning.replacement_text()),
        &[Token::Cs(a), Token::Cs(b)]
    );
}

#[test]
fn edef_expansion_uses_active_input_hooks() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    install_expandable(&mut stores, "input", ExpandablePrimitive::Input);
    stores.set_int_param(IntParam::END_LINE_CHAR, -1);
    let mut input = InputStack::new(MemoryInput::new("\\edef\\e{\\input{inc}}"));
    let mut hooks = EdefInputHooks;

    Executor::new()
        .run_with_recorder_and_hooks(&mut input, &mut stores, &mut NoopRecorder, &mut hooks)
        .expect("edef executes through input hook");
    let e = stores.symbol("e").expect("e was interned");
    let meaning = stores.macro_meaning(e).expect("e is a macro");

    assert_eq!(
        stores.tokens(meaning.replacement_text()),
        &[
            Token::Char {
                ch: 'O',
                cat: Catcode::Letter
            },
            Token::Char {
                ch: 'K',
                cat: Catcode::Letter
            },
        ]
    );
}

#[test]
fn input_expands_while_scanning_assignment_values() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    stores.set_int_param(IntParam::END_LINE_CHAR, -1);
    let mut input = InputStack::new(MemoryInput::new(
        "\\dimen0=\\input{dim}\\skip0=\\input{glue}\\end",
    ));
    let mut hooks = MemoryInputHooks::new()
        .with_source("dim", "12pt")
        .with_source("glue", "3pt plus 2pt");

    Executor::new()
        .run_with_recorder_and_hooks(&mut input, &mut stores, &mut NoopRecorder, &mut hooks)
        .expect("assignments scan through input hooks");

    assert_eq!(
        stores.dimen(0),
        tex_state::scaled::Scaled::from_raw(12 * 65_536)
    );
    let glue = stores.glue(stores.skip(0));
    assert_eq!(glue.width, tex_state::scaled::Scaled::from_raw(3 * 65_536));
    assert_eq!(
        glue.stretch,
        tex_state::scaled::Scaled::from_raw(2 * 65_536)
    );
}

#[test]
fn input_expands_while_scanning_conditional_operands() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    stores.set_int_param(IntParam::END_LINE_CHAR, -1);
    let mut input = InputStack::new(MemoryInput::new(
        "\\ifdim\\input{left}<\\input{right}\\count0=1\\fi\
         \\ifcat\\input{a}\\input{b}\\count1=1\\fi\
         \\ifnum 1 \\input{relation} 2\\count2=1\\fi\
         \\ifeof\\input{stream}\\count3=1\\fi\\end",
    ));
    let mut hooks = MemoryInputHooks::new()
        .with_source("left", "1pt")
        .with_source("right", "2pt")
        .with_source("a", "a")
        .with_source("b", "b")
        .with_source("relation", "<")
        .with_source("stream", "15");

    Executor::new()
        .run_with_recorder_and_hooks(&mut input, &mut stores, &mut NoopRecorder, &mut hooks)
        .expect("conditionals scan through input hooks");

    assert_eq!(stores.count(0), 1);
    assert_eq!(stores.count(1), 1);
    assert_eq!(stores.count(2), 1);
    assert_eq!(stores.count(3), 1);
}

#[test]
fn input_expands_while_scanning_register_indices_and_the_operands() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    stores.set_int_param(IntParam::END_LINE_CHAR, -1);
    let mut input = InputStack::new(MemoryInput::new(
        "\\count\\input{idx}=9\\edef\\e{\\the\\count\\input{idx}}\\end",
    ));
    let mut hooks = MemoryInputHooks::new().with_source("idx", "5");

    Executor::new()
        .run_with_recorder_and_hooks(&mut input, &mut stores, &mut NoopRecorder, &mut hooks)
        .expect("register and the scans use input hooks");

    assert_eq!(stores.count(5), 9);
    let e = stores.symbol("e").expect("macro was defined");
    let meaning = stores.macro_meaning(e).expect("e is a macro");
    assert_eq!(
        stores.tokens(meaning.replacement_text()),
        &[Token::Char {
            ch: '9',
            cat: Catcode::Other
        }]
    );
}

#[test]
fn let_assigns_control_sequence_and_implicit_character_meanings() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let a = stores.intern("a");
    stores.set_meaning(a, Meaning::CharGiven('Q'));
    let mut input = InputStack::new(MemoryInput::new("\\let\\b=\\a\\let\\c = Z"));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("let assignments execute");
    assert_eq!(
        stores.meaning(stores.symbol("b").expect("b was interned")),
        Meaning::CharGiven('Q')
    );
    assert_eq!(
        stores.meaning(stores.symbol("c").expect("c was interned")),
        Meaning::CharGiven('Z')
    );
}

#[test]
fn futurelet_assigns_second_token_meaning_and_preserves_order() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let futurelet = stores.symbol("futurelet").expect("futurelet");
    let mut input = InputStack::new(MemoryInput::new("\\n\\first x"));
    let mut hooks = NoopExecHooks;

    dispatch_delivered_token(
        &mut ModeNest::new(),
        Token::Cs(futurelet),
        &mut input,
        &mut stores,
        &mut hooks,
    )
    .expect("futurelet executes");

    let n = stores.symbol("n").expect("n was interned");
    assert_eq!(stores.meaning(n), Meaning::CharGiven('x'));
    assert_eq!(
        input.next_token(&mut stores).expect("first replayed"),
        Some(Token::Cs(stores.symbol("first").expect("first")))
    );
    assert_eq!(
        input.next_token(&mut stores).expect("second replayed"),
        Some(Token::Char {
            ch: 'x',
            cat: Catcode::Letter
        })
    );
}

#[test]
fn box_primitives_round_trip_through_registers() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\setbox0=\\hbox to 10pt{}\\setbox1=\\copy0\\box0",
    ));
    let mut executor = Executor::new();

    executor
        .run(&mut input, &mut stores)
        .expect("box primitives execute");

    assert!(stores.box_reg(0).is_none(), "\\box should void register 0");
    let box1 = stores.box_reg(1).expect("copy should preserve register 1");
    let [tex_state::node::Node::HList(box_node)] = stores.nodes(box1) else {
        panic!("register 1 should hold an hbox");
    };
    assert_eq!(box_node.width.raw(), 10 * tex_state::scaled::Scaled::UNITY);
    let [tex_state::node::Node::HList(appended)] = executor.nest().current_list().nodes() else {
        panic!("main vertical list should contain copied-out hbox");
    };
    assert_eq!(appended.width.raw(), 10 * tex_state::scaled::Scaled::UNITY);
}

#[test]
fn box_dimension_writes_are_readable_by_the() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    install_expandable(&mut stores, "the", ExpandablePrimitive::The);
    let mut input = InputStack::new(MemoryInput::new(
        "\\setbox0=\\hbox{}\\wd0=12pt\\edef\\x{\\the\\wd0}",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("box dimension assignment executes");

    assert_eq!(
        stores
            .box_dimension(0, tex_state::BoxDimension::Width)
            .expect("box dimension")
            .raw(),
        12 * tex_state::scaled::Scaled::UNITY
    );
    let x = stores.symbol("x").expect("x was interned");
    let meaning = stores.macro_meaning(x).expect("x is a macro");
    let rendered: String = stores
        .tokens(meaning.replacement_text())
        .iter()
        .filter_map(|token| match token {
            Token::Char { ch, .. } => Some(*ch),
            _ => None,
        })
        .collect();
    assert_eq!(rendered, "12.0pt");
}

#[test]
fn everypar_replays_through_input_stack_and_mutates_state() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let global = stores.intern("global");
    let count = stores.intern("count");
    let everypar = stores.intern_token_list(&[
        Token::Cs(global),
        Token::Cs(count),
        Token::Char {
            ch: '0',
            cat: Catcode::Other,
        },
        Token::Char {
            ch: '=',
            cat: Catcode::Other,
        },
        Token::Char {
            ch: '7',
            cat: Catcode::Other,
        },
    ]);
    stores.set_tok_param(TokParam::EVERY_PAR, everypar);
    let mut input = InputStack::new(MemoryInput::new("x\\par"));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("paragraph executes");

    assert_eq!(stores.count(0), 7);
}

#[test]
fn paragraph_end_appends_single_line_through_vertical_spacing() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    stores.set_dimen_param(
        DimenParam::PAR_INDENT,
        tex_state::scaled::Scaled::from_raw(0),
    );
    let baseline = stores.intern_glue(GlueSpec {
        width: tex_state::scaled::Scaled::from_raw(12 * 65_536),
        ..GlueSpec::ZERO
    });
    stores.set_glue_param(GlueParam::BASELINE_SKIP, baseline);
    let mut input = InputStack::new(MemoryInput::new(
        "\\setbox0=\\hbox{}\\ht0=4pt\\dp0=1pt\\copy0\\par\\copy0",
    ));
    let mut executor = Executor::new();

    executor
        .run(&mut input, &mut stores)
        .expect("paragraph and box execute");

    let nodes = executor.nest().current_list().nodes();
    assert!(nodes.iter().any(|node| matches!(
        node,
        tex_state::node::Node::Glue {
            kind: tex_state::node::GlueKind::BaselineSkip,
            ..
        }
    )));
}

#[test]
fn parshape_and_hanging_parameters_reset_after_paragraph() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\parshape=1 3pt 40pt\\hangindent=5pt\\hangafter=2 x\\par",
    ));
    let mut executor = Executor::new();

    executor
        .run(&mut input, &mut stores)
        .expect("paragraph executes");

    assert_eq!(stores.dimen_param(DimenParam::HANG_INDENT).raw(), 0);
    assert_eq!(stores.int_param(IntParam::HANG_AFTER), 1);
    assert!(executor.nest().current_list().par_shape().is_none());
}

#[test]
fn long_prefix_on_let_reports_tex_prefix_error() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\long\\let\\a=b"));

    let err = Executor::new()
        .run(&mut input, &mut stores)
        .expect_err("prefix is illegal");
    assert!(err.to_string().contains("You can't use a prefix with"));
}
