use super::support::*;
use super::*;
use tex_state::ids::ArenaRef;
use tex_state::node::Node;
use tex_state::scaled::Scaled;

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
            TracedTokenWord::pack(Token::Cs(relax), OriginId::UNKNOWN),
            &mut input,
            &mut stores,
            &mut hooks
        )
        .expect("relax dispatch"),
        DispatchAction::Continue
    );
}

#[test]
fn dump_warns_once_and_stops_before_following_input() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(r"\dump\dump"));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("dump should finish through the end cleanup path");

    let log = terminal_effect_text(&stores);
    assert_eq!(log.matches("\\dump format serialization").count(), 1);
}

#[test]
fn inputlineno_reports_current_physical_source_line() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\relax\n\\message{L=\\the\\inputlineno}\\end",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("inputlineno should expand as an internal integer");

    assert!(terminal_effect_text(&stores).contains("L=2"));
}

#[test]
fn setlanguage_appends_normalized_language_whatsit_in_hmode() {
    let mut stores = Universe::new();
    crate::install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        r"\lefthyphenmin=0 \righthyphenmin=99 \setbox0=\hbox{\setlanguage7}",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("setlanguage should append a language whatsit");

    let box0 = stores.box_reg(0).expect("box should be assigned");
    let [tex_state::node::Node::HList(box_node)] = stores.nodes(box0) else {
        panic!("register 0 should hold an hbox");
    };
    assert!(matches!(
        stores.nodes(box_node.children),
        [tex_state::node::Node::Whatsit(
            tex_state::node::Whatsit::Language {
                language: 7,
                left_hyphen_min: 1,
                right_hyphen_min: 63,
            }
        )]
    ));
}

#[test]
fn internal_integer_assignment_leaves_following_expandafter_unexpanded() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    let source = r#"
        \catcode`@=11
        \countdef\m@ne=22 \m@ne=-1
        \countdef\count@=255
        {\uccode`1=`i \uccode`2=`f \uppercase{\gdef\if@12{\message{ok}}}}
        \escapechar\m@ne
        \expandafter\if@\string\ifplain
        \end
    "#;
    let mut input = InputStack::new(MemoryInput::new(source));
    Executor::new()
        .run(&mut input, &mut stores)
        .expect("internal integer assignment preserves following expandafter");
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
        dispatch_delivered_token(
            &mut nest,
            TracedTokenWord::pack(token, OriginId::UNKNOWN),
            &mut input,
            &mut stores,
            &mut hooks,
        )
        .expect("character dispatch"),
        DispatchAction::Continue
    );
}

#[test]
fn dispatch_errors_expose_primary_origins() {
    let mut stores = Universe::new();
    let undefined = stores.intern("undefined");
    let origin = stores.source_origin(tex_state::SourceId::new(1), 12, 3, 4);
    let mut input = InputStack::new(MemoryInput::new(""));
    let mut hooks = NoopExecHooks;

    let err = dispatch_delivered_token(
        &mut ModeNest::new(),
        TracedTokenWord::pack(Token::Cs(undefined), origin),
        &mut input,
        &mut stores,
        &mut hooks,
    )
    .expect_err("undefined control sequence");

    assert_eq!(err.primary_origin(), Some(origin));
    assert!(matches!(
        err,
        ExecError::UndefinedControlSequence {
            name,
            origin: reported
        } if name == "undefined" && reported == origin
    ));
}

#[test]
fn extra_expandable_delivery_exposes_responsible_token_origin() {
    let mut stores = Universe::new();
    install_expandable(&mut stores, "endcsname", ExpandablePrimitive::EndCsName);
    let endcsname = stores.symbol("endcsname").expect("endcsname");
    let origin = stores.source_origin(tex_state::SourceId::new(2), 20, 5, 6);
    let mut input = InputStack::new(MemoryInput::new(""));
    let mut hooks = NoopExecHooks;

    let err = dispatch_delivered_token(
        &mut ModeNest::new(),
        TracedTokenWord::pack(Token::Cs(endcsname), origin),
        &mut input,
        &mut stores,
        &mut hooks,
    )
    .expect_err("extra endcsname");

    assert_eq!(err.primary_origin(), Some(origin));
    assert!(matches!(err, ExecError::ExtraEndCsName { origin: reported } if reported == origin));
}

#[test]
fn prefix_error_uses_scanned_token_origin() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let global = stores.symbol("global").expect("global");
    let prefix_origin = stores.source_origin(tex_state::SourceId::new(3), 30, 7, 8);
    let mut input = InputStack::new(MemoryInput::new("x"));
    let mut hooks = NoopExecHooks;

    let err = dispatch_delivered_token(
        &mut ModeNest::new(),
        TracedTokenWord::pack(Token::Cs(global), prefix_origin),
        &mut input,
        &mut stores,
        &mut hooks,
    )
    .expect_err("prefix before non-assignment");

    let reported = err.primary_origin().expect("prefix error origin");
    assert_ne!(reported, OriginId::UNKNOWN);
    assert_ne!(reported, prefix_origin);
    assert!(matches!(
        err,
        ExecError::PrefixWithNonAssignment {
            token: Token::Char {
                ch: 'x',
                cat: Catcode::Letter
            },
            origin
        } if origin == reported
    ));
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
    stores.set_meaning(b, Meaning::Relax);
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
        Meaning::CharToken {
            ch: 'Z',
            cat: Catcode::Letter
        }
    );
}

#[test]
fn let_skips_spaces_before_optional_equals_and_aliases_control_symbol() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\def\\\\#1{#1}\\let\\alias   = \\\\ "));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("let should scan the raw control-symbol meaning");

    let control_symbol = stores.symbol("\\").expect("control symbol");
    let alias = stores.symbol("alias").expect("alias");
    assert_eq!(stores.meaning(alias), stores.meaning(control_symbol));
}

#[test]
fn plain_getf_ctor_setup_restores_catcodes_before_control_symbol_alias() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    stores.set_catcode('@', Catcode::Letter);
    let mut input = InputStack::new(MemoryInput::new(
        "{\\catcode`p=12 \\catcode`t=12 \\gdef\\\\#1pt{#1}} \\let\\getf@ctor=\\\\",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("plain.tex getf@ctor setup should execute");

    assert_eq!(stores.catcode('p'), Catcode::Letter);
    assert_eq!(stores.catcode('t'), Catcode::Letter);
    let control_symbol = stores.symbol("\\").expect("control symbol");
    let alias = stores.symbol("getf@ctor").expect("getf@ctor alias");
    assert_eq!(stores.meaning(alias), stores.meaning(control_symbol));
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
        TracedTokenWord::pack(Token::Cs(futurelet), OriginId::UNKNOWN),
        &mut input,
        &mut stores,
        &mut hooks,
    )
    .expect("futurelet executes");

    let n = stores.symbol("n").expect("n was interned");
    assert_eq!(
        stores.meaning(n),
        Meaning::CharToken {
            ch: 'x',
            cat: Catcode::Letter
        }
    );
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
fn def_accepts_active_character_target_and_expands_it() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    stores.set_catcode('~', Catcode::Active);
    let mut input = InputStack::new(MemoryInput::new("\\def~{OK}\\edef\\x{~}"));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("active character definition executes");

    assert!(
        stores
            .macro_meaning(stores.active_character_symbol('~').expect("active symbol"))
            .is_some()
    );
    assert_eq!(macro_text(&stores, "x"), "OK");
}

#[test]
fn active_character_and_same_spelling_control_symbol_expand_independently() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    stores.set_catcode('~', Catcode::Active);
    let mut input = InputStack::new(MemoryInput::new(
        "\\def~{ACTIVE}\\def\\~{NAMED}\\edef\\a{~}\\edef\\b{\\~}",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("colliding printed spellings remain independent");

    let named = stores.symbol("~").expect("named control symbol");
    let active = stores.active_character_symbol('~').expect("active symbol");
    assert_ne!(named, active);
    assert_eq!(macro_text(&stores, "a"), "ACTIVE");
    assert_eq!(macro_text(&stores, "b"), "NAMED");
}

#[test]
fn let_accepts_active_character_target() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    stores.set_catcode('~', Catcode::Active);
    let mut input = InputStack::new(MemoryInput::new("\\def\\a{A}\\let~=\\a\\edef\\x{~}"));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("active character let executes");

    assert_eq!(macro_text(&stores, "x"), "A");
}

#[test]
fn futurelet_accepts_active_character_target() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    stores.set_catcode('~', Catcode::Active);
    let mut input = InputStack::new(MemoryInput::new("~\\first x"));
    let mut hooks = NoopExecHooks;

    dispatch_delivered_token(
        &mut ModeNest::new(),
        TracedTokenWord::pack(
            Token::Cs(stores.symbol("futurelet").expect("futurelet")),
            OriginId::UNKNOWN,
        ),
        &mut input,
        &mut stores,
        &mut hooks,
    )
    .expect("futurelet executes");

    assert_eq!(
        stores.meaning(stores.active_character_symbol('~').expect("active symbol")),
        Meaning::CharToken {
            ch: 'x',
            cat: Catcode::Letter
        }
    );
}

#[test]
fn countdef_accepts_active_character_target_and_assigns_through_it() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    stores.set_catcode('~', Catcode::Active);
    let mut input = InputStack::new(MemoryInput::new("\\countdef~=12 ~=7"));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("active character countdef executes");

    assert_eq!(
        stores.meaning(stores.active_character_symbol('~').expect("active symbol")),
        Meaning::CountRegister(12)
    );
    assert_eq!(stores.count(12), 7);
}

#[test]
fn outer_def_accepts_active_character_target() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    stores.set_catcode('~', Catcode::Active);
    let mut input = InputStack::new(MemoryInput::new("\\outer\\def~{A}"));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("outer active character definition executes");

    assert!(matches!(
        stores.meaning(stores.active_character_symbol('~').expect("active symbol")),
        Meaning::Macro { flags, .. } if flags.contains(tex_state::meaning::MeaningFlags::OUTER)
    ));
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
    let Some(tex_state::node::Node::HList(appended)) = stores
        .current_page_nodes()
        .iter()
        .find(|node| matches!(node, tex_state::node::Node::HList(_)))
    else {
        panic!("current page should contain copied-out hbox");
    };
    assert_eq!(appended.width.raw(), 10 * tex_state::scaled::Scaled::UNITY);
}

#[test]
fn last_box_assignment_replays_with_identical_state_hash() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let checkpoint = stores.snapshot();
    let source = "\\setbox0=\\hbox{\\raise2pt\\hbox to7pt{}\\global\\setbox1=\\lastbox}";

    let mut input = InputStack::new(MemoryInput::new(source));
    Executor::new()
        .run(&mut input, &mut stores)
        .expect("first lastbox execution");
    let first_box = stores.box_reg(1).expect("global lastbox destination");
    let [Node::HList(first_node)] = stores.nodes(first_box) else {
        panic!("lastbox destination should contain an hbox");
    };
    assert_eq!(first_node.shift.raw(), 0, "lastbox clears box shift");
    let first_hash = stores.snapshot().state_hash();
    stores.rollback(&checkpoint);
    let mut input = InputStack::new(MemoryInput::new(source));
    Executor::new()
        .run(&mut input, &mut stores)
        .expect("replayed lastbox execution");

    assert_eq!(stores.snapshot().state_hash(), first_hash);
}

#[test]
fn control_space_appends_normal_font_space_glue() {
    let mut stores = stores_with_fonts();
    let mut input = InputStack::new(MemoryInput::new(
        "\\font\\f=cmr10 \\relax \\f\
         \\fontdimen2\\f=10pt \\fontdimen3\\f=2pt \\fontdimen4\\f=3pt \
         \\spaceskip=20pt \\xspaceskip=30pt \
         \\setbox0=\\hbox{A\\spacefactor=3000\\ B}",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("control space executes");

    let box0 = stores.box_reg(0).expect("box should be assigned");
    let [tex_state::node::Node::HList(box_node)] = stores.nodes(box0) else {
        panic!("register 0 should hold an hbox");
    };
    let children = stores.nodes(box_node.children);
    assert!(matches!(
        children,
        [
            tex_state::node::Node::Char { ch: 'A', .. },
            tex_state::node::Node::Glue { spec, kind: tex_state::node::GlueKind::Normal, leader: None },
            tex_state::node::Node::Char { ch: 'B', .. },
        ] if stores.glue(*spec) == GlueSpec {
            width: Scaled::from_raw(10 * Scaled::UNITY),
            stretch: Scaled::from_raw(2 * Scaled::UNITY),
            stretch_order: tex_state::glue::Order::Normal,
            shrink: Scaled::from_raw(3 * Scaled::UNITY),
            shrink_order: tex_state::glue::Order::Normal,
        }
    ));
}

#[test]
fn adjacent_cmr10_characters_emit_tfm_kern() {
    let mut stores = stores_with_fonts();
    let mut input = InputStack::new(MemoryInput::new(
        "\\font\\f=cmr10 \\relax \\f \\everypar{\\penalty10000}\\setbox0=\\vbox{Yo\\par}",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("font kern program executes");

    let box0 = stores.box_reg(0).expect("box should be assigned");
    let [Node::VList(box_node)] = stores.nodes(box0) else {
        panic!("register 0 should hold a vbox");
    };
    let line = stores
        .nodes(box_node.children)
        .iter()
        .find_map(|node| match node {
            Node::HList(line) => Some(line),
            _ => None,
        })
        .expect("paragraph should produce a line");
    let children = stores.nodes(line.children);
    assert!(
        children.windows(3).any(|nodes| matches!(
            nodes,
            [
                Node::Char { ch: 'Y', .. },
                Node::Kern {
                    amount,
                    kind: tex_state::node::KernKind::Font,
                },
                Node::Char { ch: 'o', .. },
            ] if amount.raw() == -54_614
        )),
        "unexpected Yo nodes: {children:?}"
    );
}

#[test]
fn literal_groups_break_ligature_runs_and_preserve_natural_width() {
    let mut stores = stores_with_fonts();
    let mut input = InputStack::new(MemoryInput::new(
        "\\font\\f=cmr10 \\relax \\f \\setbox0=\\hbox{first}\\setbox1=\\hbox{{f}irst}",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("grouped ligature boundary executes");

    let ligated = stores.box_reg(0).expect("ligated box should be assigned");
    let grouped = stores.box_reg(1).expect("grouped box should be assigned");
    let [Node::HList(ligated_box)] = stores.nodes(ligated) else {
        panic!("register 0 should hold an hbox");
    };
    let [Node::HList(grouped_box)] = stores.nodes(grouped) else {
        panic!("register 1 should hold an hbox");
    };

    assert!(matches!(
        stores.nodes(ligated_box.children).first(),
        Some(Node::Lig {
            ch: '\u{c}',
            orig: ('f', 'i'),
            ..
        })
    ));
    assert!(matches!(
        stores.nodes(grouped_box.children),
        [Node::Char { ch: 'f', .. }, Node::Char { ch: 'i', .. }, ..]
    ));
    assert_eq!(
        grouped_box.width.raw() - ligated_box.width.raw(),
        18_205,
        "cmr10's unligated f+i pair has TeX82's larger natural width"
    );
}

#[test]
fn appended_box_resets_space_factor_before_sentence_punctuation() {
    let mut stores = stores_with_fonts();
    tex_expand::install_expandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\font\\f=cmr10 \\relax \\f \\sfcode46=3000 A\\hbox{}.\\message{S=\\the\\spacefactor}\\end",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("box and following punctuation execute");

    let output = terminal_effect_text(&stores);
    assert!(output.contains("S=3000"), "unexpected output: {output:?}");
}

#[test]
fn overfull_hbox_appends_running_rule_when_enabled() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    stores.set_dimen_param(
        DimenParam::OVERFULL_RULE,
        tex_state::scaled::Scaled::from_raw(3 * tex_state::scaled::Scaled::UNITY),
    );
    let mut input = InputStack::new(MemoryInput::new("\\setbox0=\\hbox to 10pt{\\kern20pt}"));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("overfull hbox executes");

    let box0 = stores.box_reg(0).expect("box should be assigned");
    let [tex_state::node::Node::HList(box_node)] = stores.nodes(box0) else {
        panic!("register 0 should hold an hbox");
    };
    let children = stores.nodes(box_node.children);
    assert!(matches!(
        children.last(),
        Some(tex_state::node::Node::Rule {
            width: Some(width),
            height: None,
            depth: None,
        }) if width.raw() == 3 * tex_state::scaled::Scaled::UNITY
    ));
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
fn uncopy_primitives_unbox_without_clearing_registers() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\setbox0=\\hbox{\\kern1pt}\
         \\setbox1=\\hbox{\\unhcopy0}\
         \\setbox2=\\vbox{\\kern2pt}\
         \\setbox3=\\vbox{\\unvcopy2}",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("uncopy primitives execute");

    assert!(stores.box_reg(0).is_some(), "\\unhcopy should not clear");
    assert!(stores.box_reg(2).is_some(), "\\unvcopy should not clear");

    let hcopy = stores.box_reg(1).expect("hcopy destination");
    let [tex_state::node::Node::HList(hbox)] = stores.nodes(hcopy) else {
        panic!("register 1 should hold an hbox");
    };
    assert!(matches!(
        stores.nodes(hbox.children),
        [tex_state::node::Node::Kern { .. }]
    ));

    let vcopy = stores.box_reg(3).expect("vcopy destination");
    let [tex_state::node::Node::VList(vbox)] = stores.nodes(vcopy) else {
        panic!("register 3 should hold a vbox");
    };
    assert!(matches!(
        stores.nodes(vbox.children),
        [tex_state::node::Node::Kern { .. }]
    ));
}

#[test]
fn unvbox_splices_vertical_nodes_without_inserting_baseline_glue() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\vsize=1000pt \
         \\setbox0=\\vbox{\\hrule\\hbox{}}\\unvbox0",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("unvbox executes");

    assert!(!stores.current_page_nodes().iter().any(|node| matches!(
        node,
        tex_state::node::Node::Glue {
            kind: tex_state::node::GlueKind::BaselineSkip,
            ..
        }
    )));
}

#[test]
fn badness_reads_most_recent_pack_and_is_not_assignable() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    install_expandable(&mut stores, "the", ExpandablePrimitive::The);
    let mut input = InputStack::new(MemoryInput::new(
        "{\\setbox0=\\hbox to 10pt{\\hskip0pt plus1pt}}\\count0=\\badness\\edef\\x{\\the\\badness}",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("badness reads execute");

    assert_eq!(stores.count(0), tex_typeset::INF_BAD);
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
    assert_eq!(rendered, "10000");

    let mut bad_assignment = InputStack::new(MemoryInput::new("\\badness=0"));
    assert!(
        Executor::new()
            .run(&mut bad_assignment, &mut stores)
            .is_err(),
        "\\badness must remain read-only"
    );
}

#[test]
fn leaders_parse_box_and_rule_payloads_on_glue_nodes() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\setbox0=\\hbox{\\leaders\\hbox{\\kern1pt}\\hskip10pt}\
         \\setbox1=\\vbox{\\cleaders\\hrule height2pt\\vskip5pt}",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("leader payloads execute");

    let hbox = stores.box_reg(0).expect("hbox register");
    let [tex_state::node::Node::HList(hbox)] = stores.nodes(hbox) else {
        panic!("register 0 should hold an hbox");
    };
    let [
        tex_state::node::Node::Glue {
            spec,
            kind,
            leader: Some(tex_state::node::LeaderPayload::HList(payload)),
        },
    ] = stores.nodes(hbox.children)
    else {
        panic!("hbox should contain leader glue with hlist payload");
    };
    assert_eq!(*kind, tex_state::node::GlueKind::Leaders);
    assert_eq!(
        stores.glue(*spec).width.raw(),
        10 * tex_state::scaled::Scaled::UNITY
    );
    assert!(matches!(
        stores.nodes(payload.children),
        [tex_state::node::Node::Kern { .. }]
    ));

    let vbox = stores.box_reg(1).expect("vbox register");
    let [tex_state::node::Node::VList(vbox)] = stores.nodes(vbox) else {
        panic!("register 1 should hold a vbox");
    };
    let [
        tex_state::node::Node::Glue {
            spec,
            kind,
            leader:
                Some(tex_state::node::LeaderPayload::Rule {
                    height: Some(height),
                    ..
                }),
        },
    ] = stores.nodes(vbox.children)
    else {
        panic!("vbox should contain leader glue with rule payload");
    };
    assert_eq!(*kind, tex_state::node::GlueKind::Cleaders);
    assert_eq!(
        stores.glue(*spec).width.raw(),
        5 * tex_state::scaled::Scaled::UNITY
    );
    assert_eq!(height.raw(), 2 * tex_state::scaled::Scaled::UNITY);
}

#[test]
fn leaders_report_missing_payload_and_glue_diagnostics() {
    let mut missing_payload = Universe::new();
    install_unexpandable_primitives(&mut missing_payload);
    let err = Executor::new()
        .run(
            &mut InputStack::new(MemoryInput::new("\\setbox0=\\hbox{\\leaders x\\hskip10pt}")),
            &mut missing_payload,
        )
        .expect_err("invalid leader payload should fail");
    assert_eq!(err.to_string(), "A <box> was supposed to be here.");

    let mut missing_glue = Universe::new();
    install_unexpandable_primitives(&mut missing_glue);
    let err = Executor::new()
        .run(
            &mut InputStack::new(MemoryInput::new("\\setbox0=\\hbox{\\leaders\\hbox{}10pt}")),
            &mut missing_glue,
        )
        .expect_err("leader without proper glue should fail");
    assert_eq!(err.to_string(), "Leaders not followed by proper glue.");
}

#[test]
fn leader_payloads_participate_in_state_hash_and_rollback() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let snapshot = stores.snapshot();
    let before = snapshot.state_hash();

    Executor::new()
        .run(
            &mut InputStack::new(MemoryInput::new(
                "\\setbox0=\\hbox{\\xleaders\\hbox{\\kern1pt}\\hskip10pt}",
            )),
            &mut stores,
        )
        .expect("leader source executes");
    let with_one_point_payload = stores.snapshot().state_hash();
    assert_ne!(with_one_point_payload, before);

    stores.rollback(&snapshot);
    assert_eq!(stores.snapshot().state_hash(), before);

    Executor::new()
        .run(
            &mut InputStack::new(MemoryInput::new(
                "\\setbox0=\\hbox{\\xleaders\\hbox{\\kern2pt}\\hskip10pt}",
            )),
            &mut stores,
        )
        .expect("different leader source executes");
    assert_ne!(stores.snapshot().state_hash(), with_one_point_payload);
}

#[test]
fn showbox_dumps_leader_glue_payloads_like_reference() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\showboxbreadth=100 \\showboxdepth=100 \
         \\setbox0=\\hbox{\\leaders\\hbox{\\kern1pt}\\hskip10pt}\\showbox0",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("showbox executes");

    let log = terminal_effect_text(&stores);
    assert!(log.contains(".\\leaders 10.0"), "{log}");
    assert!(log.contains("..\\hbox"), "{log}");
    assert!(log.contains("...\\kern 1.0"), "{log}");
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
        "\\vsize=1000pt \\setbox0=\\hbox{}\\ht0=4pt\\dp0=1pt\\copy0\\par\\copy0",
    ));
    let mut executor = Executor::new();

    executor
        .run(&mut input, &mut stores)
        .expect("paragraph and box execute");

    let nodes = stores.current_page_nodes();
    assert!(nodes.iter().any(|node| matches!(
        node,
        tex_state::node::Node::Glue {
            kind: tex_state::node::GlueKind::BaselineSkip,
            ..
        }
    )));
}

#[test]
fn paragraph_end_ignores_empty_unindented_paragraph() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\setbox0=\\vbox{\\noindent\\par\\indent\\par}",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("empty and indented paragraphs execute");

    let box0 = stores.box_reg(0).expect("vbox register");
    let [Node::VList(vbox)] = stores.nodes(box0) else {
        panic!("register 0 should hold a vbox");
    };
    assert!(matches!(stores.nodes(vbox.children), [Node::HList(_)]));
}

#[test]
fn vbox_closing_brace_ends_paragraph_resumed_after_display() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\setbox0=\\vbox{\\hrule $$\\hbox{}$$}"));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("vbox containing terminal display executes");

    let box0 = stores.box_reg(0).expect("vbox register");
    let [Node::VList(vbox)] = stores.nodes(box0) else {
        panic!("register 0 should hold a vbox");
    };
    let children = stores.nodes(vbox.children);
    assert!(matches!(children.first(), Some(Node::Rule { .. })));
    assert!(children.iter().any(|node| matches!(node, Node::HList(_))));
}

#[test]
fn paragraph_end_removes_only_the_final_trailing_glue() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\setbox0=\\vbox{\\noindent x\\hskip1pt\\hskip2pt\\par}",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("paragraph executes");

    let box0 = stores.box_reg(0).expect("vbox register");
    let [Node::VList(vbox)] = stores.nodes(box0) else {
        panic!("register 0 should hold a vbox");
    };
    let line = stores
        .nodes(vbox.children)
        .iter()
        .find_map(|node| match node {
            Node::HList(line) => Some(line),
            _ => None,
        })
        .expect("paragraph should produce a line");
    let explicit_glue: Vec<_> = stores
        .nodes(line.children)
        .iter()
        .filter_map(|node| match node {
            Node::Glue {
                spec,
                kind: tex_state::node::GlueKind::Normal,
                ..
            } => Some(stores.glue(*spec).width.raw()),
            _ => None,
        })
        .collect();

    assert_eq!(explicit_glue, [65_536]);
}

#[test]
fn last_items_read_current_horizontal_tail_by_type() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\setbox0=\\hbox{\
         \\kern3pt\\xdef\\lk{\\the\\lastkern}\
         \\penalty42\\xdef\\lp{\\the\\lastpenalty}\
         \\hskip1pt plus 2fil\\xdef\\ls{\\the\\lastskip}\
         \\kern4pt\\xdef\\lszero{\\the\\lastskip}}",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("last-item reads execute");

    assert_eq!(macro_text(&stores, "lk"), "3.0pt");
    assert_eq!(macro_text(&stores, "lp"), "42");
    assert_eq!(macro_text(&stores, "ls"), "1.0pt plus 2.0fil");
    assert_eq!(macro_text(&stores, "lszero"), "0.0pt");
}

#[test]
fn delete_last_removes_only_matching_current_list_tail() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\vskip1pt\\unpenalty\\edef\\stillglue{\\the\\lastskip}\
         \\unskip\\edef\\noglue{\\the\\lastskip}",
    ));
    let mut executor = Executor::new();

    executor
        .run(&mut input, &mut stores)
        .expect("delete-last commands execute");

    assert_eq!(macro_text(&stores, "stillglue"), "1.0pt");
    assert_eq!(macro_text(&stores, "noglue"), "0.0pt");
    assert!(executor.nest().current_list().nodes().is_empty());
    assert!(stores.page_contributions().is_empty());
}

#[test]
fn vertical_infinite_skip_primitives_preserve_glue_orders() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\vfil\\edef\\vfilglue{\\the\\lastskip}\
         \\vfill\\edef\\vfillglue{\\the\\lastskip}\
         \\vss\\edef\\vssglue{\\the\\lastskip}\
         \\vfilneg\\edef\\vfilnegglue{\\the\\lastskip}",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("vertical infinite skips execute");

    assert_eq!(macro_text(&stores, "vfilglue"), "0.0pt plus 1.0fil");
    assert_eq!(macro_text(&stores, "vfillglue"), "0.0pt plus 1.0fill");
    assert_eq!(
        macro_text(&stores, "vssglue"),
        "0.0pt plus 1.0fil minus 1.0fil"
    );
    assert_eq!(macro_text(&stores, "vfilnegglue"), "0.0pt plus -1.0fil");
}

#[test]
fn delete_last_outer_vertical_empty_matches_tex_error_asymmetry() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\unskip"));
    Executor::new()
        .run(&mut input, &mut stores)
        .expect("empty outer unskip is silent");

    for (source, command) in [("\\unpenalty", "\\unpenalty"), ("\\unkern", "\\unkern")] {
        let mut stores = Universe::new();
        install_unexpandable_primitives(&mut stores);
        let mut input = InputStack::new(MemoryInput::new(source));
        let err = Executor::new()
            .run(&mut input, &mut stores)
            .expect_err("empty outer delete should error");
        assert_eq!(
            err.to_string(),
            format!("You can't use `{command}' in vertical mode.")
        );
    }
}

#[test]
fn new_paragraph_resets_prevgraf_before_tracking_finished_lines() {
    let mut stores = stores_with_fonts();
    tex_expand::install_expandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\font\\tenrm=cmr10 \\relax \\tenrm\
         \\parindent=0pt \\hsize=20pt \\parfillskip=0pt\
         \\prevgraf=5 \\edef\\pg{\\the\\prevgraf}\
         a\\penalty-10000 b\\penalty-10000 c\\par",
    ));
    let mut executor = Executor::new();

    executor
        .run(&mut input, &mut stores)
        .expect("prevgraf program executes");

    assert_eq!(macro_text(&stores, "pg"), "5");
    assert_eq!(executor.nest().enclosing_vertical_prev_graf(), 3);
}

#[test]
fn fresh_hanging_paragraph_keeps_its_first_item_line_at_full_width() {
    let mut stores = stores_with_fonts();
    tex_expand::install_expandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\font\\tenrm=cmr10 \\relax \\tenrm\
         \\setbox0=\\vbox{\\hsize=100pt \\parindent=20pt \\parfillskip=0pt plus 1fil\
         \\noindent previous\\par\
         \\hangindent=20pt \\indent\
         \\hbox to 0pt{\\hss X\\hskip10pt}first\\penalty-10000 second\\par}",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("item-shaped paragraphs execute");

    let box0 = stores.box_reg(0).expect("vbox register");
    let [Node::VList(vbox)] = stores.nodes(box0) else {
        panic!("register 0 should hold a vbox");
    };
    let lines = stores
        .nodes(vbox.children)
        .iter()
        .filter_map(|node| match node {
            Node::HList(line) => Some(line),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[1].shift.raw(), 0);
    assert_eq!(lines[1].width.raw(), 100 * Scaled::UNITY);
    assert_eq!(lines[2].shift.raw(), 20 * Scaled::UNITY);
    assert_eq!(lines[2].width.raw(), 80 * Scaled::UNITY);
}

#[test]
fn vertical_hrule_uses_defaults_and_sets_prevdepth_ignore_sentinel() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\hrule width7pt"));
    let mut executor = Executor::new();

    executor
        .run(&mut input, &mut stores)
        .expect("hrule executes");

    assert_eq!(
        executor.nest().current_list().prev_depth(),
        Some(crate::mode::IGNORE_DEPTH)
    );
    assert!(executor.nest().current_list().nodes().is_empty());
    let [
        tex_state::node::Node::Rule {
            width,
            height,
            depth,
        },
    ] = stores.page_contributions()
    else {
        panic!("recent contributions should contain one rule");
    };
    assert_eq!(width.map(tex_state::scaled::Scaled::raw), Some(7 * 65_536));
    assert_eq!(height.map(tex_state::scaled::Scaled::raw), Some(26_214));
    assert_eq!(depth.map(tex_state::scaled::Scaled::raw), Some(0));
}

#[test]
fn showlists_reports_vertical_rule_and_ignored_prevdepth() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\showboxbreadth=100 \\showboxdepth=100 \\hrule width7pt\\showlists",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("showlists executes");

    let log = terminal_effect_text(&stores);
    assert!(log.contains("### recent contributions:"));
    assert!(log.contains("\\rule(0.4+0.0)x7.0"));
    assert!(log.contains("prevdepth ignored"));
}

#[test]
fn page_builder_moves_box_and_updates_page_scalars() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\topskip=10pt \\vsize=100pt \\maxdepth=2pt \
         \\setbox0=\\hbox{}\\ht0=7pt \\dp0=3pt \
         \\copy0 \\edef\\snapshot{\\the\\pagegoal,\\the\\pagetotal,\\the\\pagedepth}",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("page snapshot executes");

    assert!(stores.page_contributions().is_empty());
    assert_eq!(stores.current_page_nodes().len(), 2);
    assert_eq!(macro_text(&stores, "snapshot"), "100.0pt,11.0pt,2.0pt");
}

#[test]
fn page_builder_discards_glue_before_first_box() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\vskip 5pt\\setbox0=\\hbox{}\\copy0"));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("discardable glue executes");

    assert!(stores.page_contributions().is_empty());
    assert!(stores.current_page_nodes().iter().all(|node| {
        !matches!(node, tex_state::node::Node::Glue { spec, .. }
        if stores.glue(*spec).width.raw() == 5 * tex_state::scaled::Scaled::UNITY)
    }));
}

#[test]
fn page_builder_reports_and_normalizes_infinite_shrink_glue() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\topskip=0pt \\vsize=100pt \\setbox0=\\hbox{}\\copy0\
         \\vskip0pt minus 1fil\\copy0",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("page glue executes");

    let log = terminal_effect_text(&stores);
    assert!(log.contains("! Infinite glue shrinkage found on current page."));
    let page_glue = stores
        .current_page_nodes()
        .iter()
        .find_map(|node| match node {
            tex_state::node::Node::Glue { spec, .. } => {
                let spec = stores.glue(*spec);
                (spec.shrink.raw() != 0).then_some(spec)
            }
            _ => None,
        })
        .expect("page glue");
    assert_eq!(page_glue.shrink.raw(), tex_state::scaled::Scaled::UNITY);
    assert_eq!(page_glue.shrink_order, tex_state::glue::Order::Normal);
}

#[test]
fn writable_page_scalars_read_after_page_freeze() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\topskip=0pt \\setbox0=\\hbox{}\\copy0 \
         \\pagegoal=12pt \\advance\\pagegoal by 3pt \
         \\insertpenalties=4 \\edef\\snapshot{\\the\\pagegoal/\\the\\insertpenalties}",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("page scalar assignment executes");

    assert_eq!(macro_text(&stores, "snapshot"), "15.0pt/4");
}

#[test]
fn insert_node_captures_split_parameters_and_natural_size() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\count7=1000 \\dimen7=100pt \
         \\splittopskip=9pt \\splitmaxdepth=3pt \\floatingpenalty=77 \
         \\insert7{\\vskip2pt\\hrule height5pt depth1pt}",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("\\insert captures parameters");

    let insert = stores
        .current_page_nodes()
        .iter()
        .find_map(|node| match node {
            tex_state::node::Node::Ins {
                class,
                size,
                split_top_skip,
                split_max_depth,
                floating_penalty,
                content,
            } => Some((
                *class,
                *size,
                stores.glue(*split_top_skip),
                *split_max_depth,
                *floating_penalty,
                *content,
            )),
            _ => None,
        })
        .expect("insert node");
    assert_eq!(insert.0, 7);
    assert_eq!(insert.1.raw(), 8 * tex_state::scaled::Scaled::UNITY);
    assert_eq!(insert.2.width.raw(), 9 * tex_state::scaled::Scaled::UNITY);
    assert_eq!(insert.3.raw(), 3 * tex_state::scaled::Scaled::UNITY);
    assert_eq!(insert.4, 77);
    assert_eq!(stores.nodes(insert.5).len(), 2);
}

#[test]
fn insertion_starts_with_normal_paragraph_parameters() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(concat!(
        "\\hsize=100pt ",
        "\\hangindent=99pt \\hangafter=0 \\looseness=2 ",
        "\\insert7{a b c d e f g h i j}"
    )));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("insertion paragraph executes");

    let (size, content) = stores
        .current_page_nodes()
        .iter()
        .find_map(|node| match node {
            tex_state::node::Node::Ins { size, content, .. } => Some((*size, *content)),
            _ => None,
        })
        .expect("insert node");
    assert!(matches!(
        stores.nodes(content),
        [tex_state::node::Node::HList(_)]
    ));
    assert!(size.raw() < 20 * tex_state::scaled::Scaled::UNITY);
    assert_eq!(
        stores.dimen_param(DimenParam::HANG_INDENT).raw(),
        99 * tex_state::scaled::Scaled::UNITY
    );
    assert_eq!(stores.int_param(IntParam::HANG_AFTER), 0);
    assert_eq!(stores.int_param(IntParam::LOOSENESS), 2);
}

#[test]
fn insertion_omits_parskip_before_first_internal_vlist_paragraph() {
    let mut stores = stores_with_fonts();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\parskip=12pt \\insert7{x}"));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("insertion paragraph executes");

    let content = stores
        .current_page_nodes()
        .iter()
        .find_map(|node| match node {
            tex_state::node::Node::Ins { content, .. } => Some(*content),
            _ => None,
        })
        .expect("insert node");
    assert!(matches!(
        stores.nodes(content),
        [tex_state::node::Node::HList(_)]
    ));
}

#[test]
fn insertion_skip_reports_infinite_shrink_correction() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\topskip=0pt \\vsize=100pt \\count7=1000 \\dimen7=100pt \
         \\skip7=0pt minus 1fil \\insert7{\\hrule height1pt}",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("insert skip executes");

    let log = terminal_effect_text(&stores);
    assert!(log.contains("! Infinite glue shrinkage inserted from \\skip7."));
    assert_eq!(
        stores
            .page_dimension(tex_state::page::PageDimension::Shrink)
            .raw(),
        tex_state::scaled::Scaled::UNITY
    );
}

#[test]
fn split_insertion_reports_and_normalizes_infinite_shrink_content() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\topskip=0pt \\vsize=20pt \\count7=1000 \\dimen7=12pt \
         \\insert7{\\hrule height8pt\\vskip0pt minus 1fil\\penalty123\\hrule height8pt}",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("split insert executes");

    let log = terminal_effect_text(&stores);
    assert!(log.contains("! Infinite glue shrinkage found in box being split."));
    let content = stores
        .current_page_nodes()
        .iter()
        .find_map(|node| match node {
            tex_state::node::Node::Ins { content, .. } => Some(*content),
            _ => None,
        })
        .expect("insert content");
    let split_glue = stores
        .nodes(content)
        .iter()
        .find_map(|node| match node {
            tex_state::node::Node::Glue { spec, .. } => Some(stores.glue(*spec)),
            _ => None,
        })
        .expect("split glue");
    assert_eq!(split_glue.shrink.raw(), tex_state::scaled::Scaled::UNITY);
    assert_eq!(split_glue.shrink_order, tex_state::glue::Order::Normal);
}

#[test]
fn vsplit_reports_and_normalizes_infinite_shrink_glue() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\setbox0=\\vbox{\\hrule height10pt\\vskip0pt minus 1fil\\hrule height10pt}\
         \\setbox1=\\vsplit0 to 30pt",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("\\vsplit executes");

    let log = terminal_effect_text(&stores);
    assert!(log.contains("! Infinite glue shrinkage found in box being split."));
    let box1 = stores.box_reg(1).expect("split box");
    let [tex_state::node::Node::VList(box_node)] = stores.nodes(box1) else {
        panic!("box1 should be a vbox");
    };
    let split_glue = stores
        .nodes(box_node.children)
        .iter()
        .find_map(|node| match node {
            tex_state::node::Node::Glue { spec, .. } => Some(stores.glue(*spec)),
            _ => None,
        })
        .expect("split glue");
    assert_eq!(split_glue.shrink.raw(), tex_state::scaled::Scaled::UNITY);
    assert_eq!(split_glue.shrink_order, tex_state::glue::Order::Normal);
}

#[test]
fn insertion_page_goal_uses_skip_once_and_count_scaling() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\topskip=0pt \\vsize=100pt \
         \\count7=500 \\dimen7=100pt \\skip7=10pt \
         \\insert7{\\hrule height20pt depth0pt}\
         \\edef\\firstpenalties{\\the\\insertpenalties}\
         \\insert7{\\hrule height10pt depth0pt}\
         \\edef\\secondpenalties{\\the\\insertpenalties}",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("insert page goal accounting executes");

    assert_eq!(macro_text(&stores, "firstpenalties"), "0");
    assert_eq!(macro_text(&stores, "secondpenalties"), "0");
    assert_eq!(
        stores
            .page_dimension(tex_state::page::PageDimension::Goal)
            .raw(),
        75 * tex_state::scaled::Scaled::UNITY + 540
    );
}

#[test]
fn split_insertion_penalty_is_mainline_then_heldover_count_in_output() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\topskip=0pt \\vsize=20pt \\count7=1000 \\dimen7=12pt \
         \\output={\\xdef\\held{\\the\\insertpenalties}\\shipout\\box255}\
         \\insert7{\\hrule height8pt depth0pt\\penalty123\\hrule height8pt depth0pt}\
         \\edef\\main{\\the\\insertpenalties}\
         \\setbox0=\\hbox{}\\copy0\\penalty-10000",
    ));

    let stats = Executor::new()
        .run(&mut input, &mut stores)
        .expect("split insertion output executes");

    assert_eq!(stats.shipped_artifacts.len(), 1);
    assert_eq!(macro_text(&stores, "main"), "123");
    assert_eq!(macro_text(&stores, "held"), "1");

    let box7 = stores.box_reg(7).expect("insertion box");
    let [tex_state::node::Node::VList(box_node)] = stores.nodes(box7) else {
        panic!("box7 should be a vbox");
    };
    assert!(
        stores
            .nodes(box_node.children)
            .iter()
            .any(|node| matches!(node, tex_state::node::Node::Rule { .. })),
        "split-off insertion material should be appended to box7"
    );
}

#[test]
fn forced_page_penalty_runs_default_output() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\topskip=0pt \\setbox0=\\hbox{}\\copy0 \\penalty-10000",
    ));

    let stats = Executor::new()
        .run(&mut input, &mut stores)
        .expect("forced penalty executes");

    assert_eq!(stats.shipped_artifacts.len(), 1);
    assert!(stores.box_reg(255).is_none());
    assert!(stores.page_fire_up().is_none());
    assert!(stores.current_page_nodes().is_empty());
    assert!(stores.page_contributions().is_empty());
    assert_eq!(
        stores.page_dimension(tex_state::page::PageDimension::Goal),
        tex_state::scaled::Scaled::MAX_DIMEN
    );
}

#[test]
fn page_output_promotes_nested_survivor_children_into_one_root() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\output={\\global\\setbox2=\\copy255 \\shipout\\box255}\
         \\topskip=0pt \\setbox0=\\hbox{X}\\copy0 \\penalty-10000",
    ));

    let stats = Executor::new()
        .run(&mut input, &mut stores)
        .expect("nested page output should promote without panicking");

    assert_eq!(stats.shipped_artifacts.len(), 1);
    let root = stores
        .box_reg(2)
        .expect("output routine should retain page copy");
    let ArenaRef::Survivor(root_id) = root.arena() else {
        panic!("retained page should be survivor-owned");
    };
    let mut pending = vec![root];
    let mut nested_boxes = 0;
    while let Some(list) = pending.pop() {
        for node in stores.nodes(list) {
            if let Node::HList(box_node) | Node::VList(box_node) = node {
                let ArenaRef::Survivor(child_root) = box_node.children.arena() else {
                    panic!("promoted page contains an epoch child");
                };
                assert_eq!(child_root, root_id);
                nested_boxes += 1;
                pending.push(box_node.children);
            }
        }
    }
    assert!(
        nested_boxes >= 2,
        "page should retain packed and source boxes"
    );
}

#[test]
fn page_output_keeps_locally_moved_box_children_live() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\topskip=0pt {\\setbox0=\\hbox{X}\\box0} \\penalty-10000",
    ));

    let stats = Executor::new()
        .run(&mut input, &mut stores)
        .expect("locally moved page box should remain live through output");

    assert_eq!(stats.shipped_artifacts.len(), 1);
    assert!(stores.box_reg(0).is_none());
    assert!(stores.box_reg(255).is_none());
}

#[test]
fn page_output_keeps_shifted_copy_children_live_after_source_replacement() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\topskip=0pt \\setbox0=\\hbox{X} \\raise1pt\\copy0 \
         \\setbox0=\\hbox{Y} \\penalty-10000",
    ));

    let stats = Executor::new()
        .run(&mut input, &mut stores)
        .expect("shifted shared box should own epoch children on the page");

    assert_eq!(stats.shipped_artifacts.len(), 1);
    assert!(stores.box_reg(255).is_none());
}

#[test]
fn mark_scans_raw_general_text_then_expands_payload() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\def\\a{A}\\mark{#\\a}"));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("\\mark executes");

    let mark = stores
        .current_page_nodes()
        .iter()
        .chain(stores.page_contributions())
        .find_map(|node| match node {
            tex_state::node::Node::Mark { tokens, .. } => Some(*tokens),
            _ => None,
        })
        .expect("mark node");
    assert_eq!(
        stores.tokens(mark),
        &[
            Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
            },
            Token::Char {
                ch: 'A',
                cat: Catcode::Letter,
            },
        ]
    );
}

#[test]
fn fire_up_updates_top_first_bot_marks_across_no_mark_page() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\output={\\global\\advance\\count0 by 1 \
         \\ifnum\\count0=1 \\xdef\\pagea{\\topmark/\\firstmark/\\botmark}\
         \\else\\ifnum\\count0=2 \\xdef\\pageb{\\topmark/\\firstmark/\\botmark}\
         \\else\\ifnum\\count0=3 \\xdef\\pagec{\\topmark/\\firstmark/\\botmark}\
         \\else\\ifnum\\count0=4 \\xdef\\paged{\\topmark/\\firstmark/\\botmark}\
         \\else \\xdef\\pagee{\\topmark/\\firstmark/\\botmark}\\fi\\fi\\fi\\fi \
         \\shipout\\box255}\
         \\topskip=0pt \\vsize=1pt \\setbox0=\\hbox{}\\ht0=2pt \
         \\mark{A}\\copy0\\penalty-10000 \
         \\copy0\\penalty-10000 \
         \\mark{B}\\copy0\\penalty-10000",
    ));

    let stats = Executor::new()
        .run(&mut input, &mut stores)
        .expect("marked pages ship");

    assert_eq!(stats.shipped_artifacts.len(), 5);
    assert_eq!(macro_text(&stores, "pagea"), "/A/A");
    assert_eq!(macro_text(&stores, "pageb"), "A/A/A");
    assert_eq!(macro_text(&stores, "pagec"), "A/A/A");
    assert_eq!(macro_text(&stores, "paged"), "A/B/B");
    assert_eq!(macro_text(&stores, "pagee"), "B/B/B");
}

#[test]
fn output_routine_replays_in_implicit_group_and_consumes_box255() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\output={\\advance\\count0 by 1 \\global\\advance\\count1 by 1 \\shipout\\box255}\
         \\count0=10 \\count1=20 \
         \\topskip=0pt \\setbox0=\\hbox{}\\copy0 \\penalty-10000",
    ));

    let stats = Executor::new()
        .run(&mut input, &mut stores)
        .expect("custom output routine executes");

    assert_eq!(stats.shipped_artifacts.len(), 1);
    assert_eq!(
        stores.count(0),
        10,
        "plain assignments in \\output are local"
    );
    assert_eq!(
        stores.count(1),
        21,
        "global assignments in \\output survive"
    );
    assert_eq!(
        stores.page_integer(tex_state::page::PageInteger::DeadCycles),
        0
    );
    assert!(stores.box_reg(255).is_none());
}

#[test]
fn lastbox_reappend_runs_page_builder_before_enclosing_group_ends() {
    let mut stores = support::stores_with_fonts();
    tex_expand::install_expandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\font\\tenrm=cmr10 \\font\\tt=cmtt10 \\tenrm \
         \\topskip=0pt \\vsize=1pt \
         \\output={\\global\\advance\\count1 by 1 \
           \\ifnum\\count1=1 \\global\\dimen1=1em\\fi \
           \\shipout\\box255} \
         \\setbox0=\\vbox{\\hbox{}\\penalty-10000\\hbox{}} \
         {\\tt \\unvbox0\\lastbox} \
         \\end",
    ));

    let stats = Executor::new()
        .run(&mut input, &mut stores)
        .expect("lastbox reappend should fire output within the font group");

    assert!(!stats.shipped_artifacts.is_empty());
    let typewriter = support::font_meaning(&stores, "tt");
    assert_eq!(stores.dimen(1), stores.font_parameter(typewriter, 6));
}

#[test]
fn output_routine_reports_nonvoid_box255_after_output() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\output={\\relax}\\topskip=0pt \\setbox0=\\hbox{}\\copy0 \\penalty-10000",
    ));

    let err = Executor::new()
        .run(&mut input, &mut stores)
        .expect_err("empty custom output leaves box255 behind");

    assert_eq!(err.to_string(), "Output routine didn't use all of \\box255");
}

#[test]
fn deadcycles_overflow_reports_output_loop() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\maxdeadcycles=1 \\output={\\setbox1=\\box255}\
         \\topskip=0pt \\setbox0=\\hbox{}\
         \\copy0 \\penalty-10000 \
         \\copy0 \\penalty-10000",
    ));

    let err = Executor::new()
        .run(&mut input, &mut stores)
        .expect_err("second dead cycle should overflow");

    assert_eq!(err.to_string(), "Output loop---1 consecutive dead cycles");
}

#[test]
fn end_cleanup_ejects_residual_page() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\topskip=0pt \\setbox0=\\hbox{}\\copy0 \\end",
    ));

    let stats = Executor::new()
        .run(&mut input, &mut stores)
        .expect("\\end cleanup ships residual page");

    assert_eq!(stats.shipped_artifacts.len(), 1);
    assert!(stores.current_page_nodes().is_empty());
    assert!(stores.page_contributions().is_empty());
    assert_eq!(
        stores.page_integer(tex_state::page::PageInteger::DeadCycles),
        0
    );
}

#[test]
fn parshape_and_hanging_parameters_reset_after_paragraph() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\parshape=1 3pt 40pt\\hangindent=5pt\\hangafter=2\\looseness=2 x\\par",
    ));
    let mut executor = Executor::new();

    executor
        .run(&mut input, &mut stores)
        .expect("paragraph executes");

    assert_eq!(stores.dimen_param(DimenParam::HANG_INDENT).raw(), 0);
    assert_eq!(stores.int_param(IntParam::HANG_AFTER), 1);
    assert_eq!(stores.int_param(IntParam::LOOSENESS), 0);
    assert!(executor.nest().current_list().par_shape().is_none());
}

fn macro_text(stores: &Universe, name: &str) -> String {
    let symbol = stores.symbol(name).expect("macro control sequence");
    let meaning = stores.macro_meaning(symbol).expect("macro meaning");
    stores
        .tokens(meaning.replacement_text())
        .iter()
        .filter_map(|token| match token {
            Token::Char { ch, .. } => Some(*ch),
            Token::Cs(_) | Token::Param(_) | Token::Frozen(_) => None,
        })
        .collect()
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
