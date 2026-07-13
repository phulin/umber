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
fn engine_checkpoint_restores_input_modes_and_universe_atomically() {
    let mut stores = Universe::new();
    let mut input = InputStack::new(MemoryInput::new(""));
    let mut executor = Executor::new();
    stores.set_count(3, 41);
    let mut checkpoints = Vec::new();
    executor
        .run_with_recorder_hooks_and_checkpoints(
            &mut input,
            &mut stores,
            &mut NoopRecorder,
            &mut NoopExecHooks,
            &mut checkpoints,
        )
        .expect("empty job");
    let checkpoint = &checkpoints[0];

    executor.nest_mut().push(Mode::Horizontal);
    stores.set_count(3, 99);
    executor
        .restore_checkpoint(&mut input, &mut stores, checkpoint, |_, _, _| {
            Ok::<_, ()>(MemoryInput::new(""))
        })
        .expect("published aggregate checkpoint");

    assert_eq!(stores.count(3), 41);
    assert_eq!(executor.nest().current_mode(), Mode::Vertical);
    assert_eq!(input.summary(), *checkpoint.input_summary());
}

#[test]
fn engine_session_publishes_named_outer_paragraph_boundary() {
    let mut stores = support::stores_with_fonts();
    let mut input = InputStack::new(MemoryInput::new("\\font\\f=cmr10 \\f x\\par"));
    let mut checkpoints = Vec::new();
    Executor::new()
        .run_with_recorder_hooks_and_checkpoints(
            &mut input,
            &mut stores,
            &mut NoopRecorder,
            &mut NoopExecHooks,
            &mut checkpoints,
        )
        .expect("paragraph job");
    assert_eq!(checkpoints[0].boundary(), EngineBoundary::JobStart);
    assert!(
        checkpoints
            .iter()
            .any(|checkpoint| checkpoint.boundary() == EngineBoundary::OuterParagraphEnd)
    );
}

#[test]
fn shipout_checkpoint_restores_after_nested_work_has_unwound() {
    let source = "\\font\\f=cmr10 \\f \\setbox0=\\hbox{\\shipout\\hbox{A}B}\\end";
    let mut stores = support::stores_with_fonts();
    let mut input = InputStack::new(MemoryInput::new(source));
    let mut executor = Executor::new();
    let mut checkpoints = Vec::new();
    executor
        .run_with_recorder_hooks_and_checkpoints(
            &mut input,
            &mut stores,
            &mut NoopRecorder,
            &mut NoopExecHooks,
            &mut checkpoints,
        )
        .expect("nested shipout job");
    let checkpoint = checkpoints
        .iter()
        .find(|checkpoint| checkpoint.boundary() == EngineBoundary::ShipoutComplete)
        .expect("outer executor publishes shipout completion");
    assert_eq!(checkpoint.mode_summary().levels().len(), 1);

    stores.set_count(7, 99);
    executor
        .restore_checkpoint(&mut input, &mut stores, checkpoint, |_, _, _| {
            Ok::<_, ()>(MemoryInput::new(source))
        })
        .expect("shipout checkpoint restores");
    assert_eq!(stores.count(7), 0);
    assert_eq!(executor.nest().current_mode(), Mode::Vertical);
}

#[test]
fn successful_execution_publishes_the_exact_final_input_cursor() {
    let mut stores = Universe::new();
    let mut input = InputStack::new(MemoryInput::new(""));
    let mut executor = Executor::new();

    executor.run(&mut input, &mut stores).expect("empty run");

    assert_eq!(stores.input_summary(), &input.summary());
}

#[test]
fn virtualized_execution_trace_is_opt_in_and_semantically_neutral() {
    fn run(tracing: bool) -> Universe {
        let mut stores = support::stores_with_fonts();
        stores.world_mut().set_execution_tracing(tracing);
        let mut input = InputStack::new(MemoryInput::new(
            "\\font\\f=cmr10 \\f x\\par \\setbox0=\\hbox{y}",
        ));
        Executor::new()
            .run(&mut input, &mut stores)
            .expect("trace comparison source executes");
        stores
    }

    let mut ordinary = run(false);
    let mut traced = run(true);
    assert!(ordinary.world().execution_trace().is_empty());
    assert!(!traced.world().execution_trace().is_empty());
    assert!(
        traced
            .world()
            .execution_trace()
            .iter()
            .any(|event| event.subsystem() == "executor")
    );
    assert_eq!(
        ordinary.world().effect_records(),
        traced.world().effect_records()
    );
    assert_eq!(
        ordinary.snapshot().state_hash(),
        traced.snapshot().state_hash()
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
            TracedTokenWord::pack(Token::Cs(relax.symbol()), OriginId::UNKNOWN),
            &mut input,
            &mut stores,
            &mut hooks
        )
        .expect("relax dispatch"),
        DispatchAction::Continue
    );
}

#[test]
fn dump_marks_format_stop_and_stops_before_following_input() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    stores.set_page_dimension(tex_state::page::PageDimension::Goal, Scaled::from_raw(123));
    let mut input = InputStack::new(MemoryInput::new(r"\dump\dump"));

    let stats = Executor::new()
        .run(&mut input, &mut stores)
        .expect("dump should finish through the end cleanup path");
    assert!(stats.dumped_format);
    assert!(stores.input_summary().is_empty());
    stores
        .dump_format()
        .expect("dump should leave a quiescent serializable format boundary");
}

#[test]
fn immediate_puts_back_non_io_extension_tokens() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        r"\immediate\catcode`A=12\message{C=\the\catcode`A}\end",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("non-I/O token after immediate should be redispatched");

    assert!(terminal_effect_text(&stores).contains("C=12"));
}

#[test]
fn interaction_mode_primitives_update_checkpointed_engine_state() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    let snapshot = stores.snapshot();
    let mut input = InputStack::new(MemoryInput::new(r"\nonstopmode\end"));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("interaction mode assignment");
    assert_eq!(stores.interaction_mode(), InteractionMode::Nonstop);

    stores.rollback(&snapshot);
    assert_eq!(stores.interaction_mode(), InteractionMode::ErrorStop);
}

#[test]
fn bare_internal_quantity_reports_illegal_mode_and_continues() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(r"\badness\message{continued}\end"));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("illegal-case diagnostics are recoverable");

    let output = terminal_effect_text(&stores);
    assert!(output.contains("You can't use `\\badness' in vertical mode"));
    assert!(output.contains("continued"));
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
    let [tex_state::node::Node::HList(box_node)] = stores.nodes(box0).testing_decoded() else {
        panic!("register 0 should hold an hbox");
    };
    assert!(matches!(
        stores.nodes(box_node.children).testing_decoded(),
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
fn dispatch_undefined_control_sequence_reports_and_continues() {
    let mut stores = Universe::new();
    let undefined = stores.intern("undefined");
    let origin = stores.source_origin(tex_state::SourceId::new(1), 12, 3, 4);
    let mut input = InputStack::new(MemoryInput::new(""));
    let mut hooks = NoopExecHooks;

    let action = dispatch_delivered_token(
        &mut ModeNest::new(),
        TracedTokenWord::pack(Token::Cs(undefined.symbol()), origin),
        &mut input,
        &mut stores,
        &mut hooks,
    )
    .expect("undefined control sequence is recoverable");

    assert_eq!(action, DispatchAction::Continue);
    assert!(
        support::terminal_effect_text(&stores).contains("Undefined control sequence \\undefined")
    );
}

#[test]
fn execution_error_capture_retains_macro_trace_after_frame_pop() {
    let mut stores = Universe::new();
    let body = stores.intern_token_list(&[]);
    let params = stores.intern_token_list(&[]);
    let macro_symbol = stores.intern("m");
    stores.set_macro_meaning(
        macro_symbol,
        tex_state::macro_store::MacroMeaning::new(
            tex_state::meaning::MeaningFlags::EMPTY,
            params,
            body,
        ),
    );
    let tex_state::meaning::Meaning::Macro { definition, .. } = stores.meaning(macro_symbol) else {
        panic!("macro meaning");
    };
    let invocation_origin = stores.source_origin(tex_state::SourceId::new(1), 1, 1, 1);
    let definition_origin = stores.source_origin(tex_state::SourceId::new(1), 2, 1, 2);
    let invocation = stores.macro_invocation_origin(
        definition,
        invocation_origin,
        definition_origin,
        OriginId::UNKNOWN,
    );
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_macro_body_with_origins_and_invocation(
        body,
        tex_state::ids::OriginListId::EMPTY,
        tex_lex::MacroArguments::new(),
        invocation,
    );
    let origin = stores.source_origin(tex_state::SourceId::new(1), 3, 1, 3);
    let error = ExecError::UndefinedControlSequence {
        name: "bad".to_owned(),
        origin,
    }
    .capture(&input);
    assert_eq!(error.diagnostic_site().expansion_head(), Some(invocation));

    assert!(
        input
            .next_traced_token(&mut stores)
            .expect("pop frame")
            .is_none()
    );
    assert_eq!(error.diagnostic_site().expansion_head(), Some(invocation));
}

#[test]
fn extra_endcsname_delivery_reports_and_continues() {
    let mut stores = Universe::new();
    install_expandable(&mut stores, "endcsname", ExpandablePrimitive::EndCsName);
    let endcsname = stores.symbol("endcsname").expect("endcsname");
    let origin = stores.source_origin(tex_state::SourceId::new(2), 20, 5, 6);
    let mut input = InputStack::new(MemoryInput::new(""));
    let mut hooks = NoopExecHooks;

    let action = dispatch_delivered_token(
        &mut ModeNest::new(),
        TracedTokenWord::pack(Token::Cs(endcsname.symbol()), origin),
        &mut input,
        &mut stores,
        &mut hooks,
    )
    .expect("extra endcsname is recoverable");

    assert_eq!(action, DispatchAction::Continue);
    assert!(support::terminal_effect_text(&stores).contains("Extra \\endcsname"));
}

#[test]
fn illegal_prefix_replays_scanned_token_with_its_origin() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let global = stores.symbol("global").expect("global");
    let prefix_origin = stores.source_origin(tex_state::SourceId::new(3), 30, 7, 8);
    let mut input = InputStack::new(MemoryInput::new("x"));
    let mut hooks = NoopExecHooks;

    let action = dispatch_delivered_token(
        &mut ModeNest::new(),
        TracedTokenWord::pack(Token::Cs(global.symbol()), prefix_origin),
        &mut input,
        &mut stores,
        &mut hooks,
    )
    .expect("TeX recovers by backing up the non-assignment token");

    assert_eq!(action, DispatchAction::Continue);
    let replayed = input
        .next_traced_token(&mut stores)
        .expect("read replayed token")
        .expect("replayed token");
    assert_eq!(
        tex_expand::semantic_token(replayed),
        Token::Char {
            ch: 'x',
            cat: Catcode::Letter
        }
    );
    assert_ne!(replayed.origin(), OriginId::UNKNOWN);
    assert_ne!(replayed.origin(), prefix_origin);
    assert!(support::terminal_effect_text(&stores).contains("You can't use a prefix"));
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
fn main_control_recovers_from_undefined_control_sequence() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\missing\\count0=7"));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("undefined command is diagnosed and consumed");

    assert_eq!(stores.count(0), 7);
    assert!(
        support::terminal_effect_text(&stores).contains("Undefined control sequence \\missing")
    );
}

#[test]
fn main_control_keeps_replaying_macro_after_undefined_control_sequence() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\def\\resume{\\missing\\let\\x\\relax}\\resume",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("undefined command inside macro is diagnosed and consumed");

    let x = stores.symbol("x").expect("let target exists");
    assert_eq!(stores.meaning(x), Meaning::Relax);
}

#[test]
fn main_control_consumes_invalid_category_character() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    stores.set_catcode('@', Catcode::Invalid);
    let mut input = InputStack::new(MemoryInput::new("@\\count0=7"));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("invalid input character is diagnosed and consumed");

    assert_eq!(stores.count(0), 7);
}

#[test]
fn main_control_aborts_nonlong_macro_argument_at_par_and_replays_par() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\def\\b#1\\par{}\\b{x\\par\\count0=7"));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("runaway macro argument is aborted recoverably");

    assert_eq!(stores.count(0), 7);
    assert!(support::terminal_effect_text(&stores).contains("Runaway argument"));
}

#[test]
fn main_control_ignores_extra_conditional_terminator() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\else\\count0=7"));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("extra conditional command is recoverable");

    assert_eq!(stores.count(0), 7);
    assert!(support::terminal_effect_text(&stores).contains("Extra \\else"));
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
    let toks_body = stores.intern_token_list(&[Token::Cs(b.symbol())]);
    stores.set_toks(0, toks_body);
    let mut input = InputStack::new(MemoryInput::new("\\edef\\e{\\noexpand\\a\\the\\toks0}"));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("edef executes");
    let e = stores.symbol("e").expect("e was interned");
    let meaning = stores.macro_meaning(e).expect("e is a macro");

    assert_eq!(
        stores.tokens(meaning.replacement_text()),
        &[Token::Cs(a.symbol()), Token::Cs(b.symbol())]
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
        TracedTokenWord::pack(Token::Cs(futurelet.symbol()), OriginId::UNKNOWN),
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
        Some(Token::Cs(stores.symbol("first").expect("first").symbol()))
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
            Token::Cs(stores.symbol("futurelet").expect("futurelet").symbol()),
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
    let [tex_state::node::Node::HList(box_node)] = stores.nodes(box1).testing_decoded() else {
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
fn box_scanner_inserts_missing_left_brace_and_replays_body_token() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\setbox0=\\hbox \\global\\count0=7}"));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("box scanner inserts a missing opening brace");

    assert_eq!(stores.count(0), 7);
    assert!(stores.box_reg(0).is_some());
    assert!(support::terminal_effect_text(&stores).contains("Missing { inserted"));
}

#[test]
fn box_scanner_closes_by_execution_group_after_message_argument() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\setbox0=\\hbox{\\message{x}\\vbox{\\hrule height2pt}}\\hrule height3pt",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("box with a message argument executes");

    let box0 = stores.box_reg(0).expect("setbox destination remains owned");
    let [Node::HList(hbox)] = stores.nodes(box0).testing_decoded() else {
        panic!("box0 should contain the outer hbox");
    };
    assert!(
        stores
            .nodes(hbox.children)
            .testing_decoded()
            .iter()
            .any(|node| matches!(node, Node::VList(_))),
        "box0 should own the nested vbox"
    );
    assert!(
        stores
            .current_page_nodes()
            .iter()
            .all(|node| !matches!(node, Node::VList(_))),
        "the nested vbox must not escape to the outer vertical list"
    );
    assert!(
        stores.page_contributions().iter().any(
            |node| matches!(node, Node::Rule { height: Some(height), .. } if height.raw() == 3 * Scaled::UNITY)
        ),
        "outer material should remain outside box0"
    );
}

#[test]
fn trip_math_mode_box_closure_preserves_ownership_and_replays() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let checkpoint = stores.snapshot();
    // This is the decisive topology reduced from the malformed tail of
    // trip.tex: Box is innermost when its brace arrives, but Math is still
    // the current mode. The vbox corresponds to the material box9 must own.
    let source = "\\setbox0=\\hbox{\x24x}\x24\\vbox{\\hrule height2pt}}\\hrule height3pt";
    let mut first_hash = None;

    for pass in 0..2 {
        let mut input = InputStack::new(MemoryInput::new(source));
        Executor::new()
            .run(&mut input, &mut stores)
            .expect("malformed math recovery remains inside the hbox scan");

        assert_eq!(stores.execution_group_depth(), 0, "pass {pass}");
        let box0 = stores.box_reg(0).expect("recovered setbox remains nonvoid");
        let [Node::HList(hbox)] = stores.nodes(box0).testing_decoded() else {
            panic!("box0 should own the recovered outer hbox");
        };
        assert!(
            stores
                .nodes(hbox.children)
                .testing_decoded()
                .iter()
                .any(|node| matches!(node, Node::HList(_) | Node::VList(_))),
            "recovered nested material remains owned by box0"
        );
        assert!(
            stores
                .current_page_nodes()
                .iter()
                .all(|node| !matches!(node, Node::VList(_))),
            "nested vbox must not leak to the outer page"
        );
        assert!(stores.page_contributions().iter().any(
            |node| matches!(node, Node::Rule { height: Some(height), .. } if height.raw() == 3 * Scaled::UNITY)
        ));

        let hash = stores.snapshot().state_hash();
        if let Some(expected) = first_hash {
            assert_eq!(hash, expected, "rollback replay must converge");
        } else {
            first_hash = Some(hash);
            stores.rollback(&checkpoint);
        }
    }
}

#[test]
fn recoverable_assignment_error_inside_box_preserves_box_ownership() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\setbox0=\\hbox{\\afterassignment\\relax\\advance\\prevdepth\\undefined\\vbox{\\hrule height2pt}}",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("improper arithmetic target recovers inside the box scanner");

    let box0 = stores
        .box_reg(0)
        .expect("setbox must not roll back to void");
    let [Node::HList(hbox)] = stores.nodes(box0).testing_decoded() else {
        panic!("box0 should contain the recovered hbox");
    };
    assert!(
        stores
            .nodes(hbox.children)
            .testing_decoded()
            .iter()
            .any(|node| matches!(node, Node::VList(_))),
        "the remaining box body must stay owned by box0"
    );
    assert!(
        stores
            .current_page_nodes()
            .iter()
            .all(|node| !matches!(node, Node::VList(_))),
        "the nested vbox must not leak onto the outer page"
    );
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
    let [Node::HList(first_node)] = stores.nodes(first_box).testing_decoded() else {
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
fn control_space_uses_space_skip_without_space_factor_scaling() {
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
    let [tex_state::node::Node::HList(box_node)] = stores.nodes(box0).testing_decoded() else {
        panic!("register 0 should hold an hbox");
    };
    let children = stores.nodes(box_node.children).testing_decoded();
    assert!(matches!(
        children,
        [
            tex_state::node::Node::Char { ch: 'A', .. },
            tex_state::node::Node::Glue { spec, kind: tex_state::node::GlueKind::Normal, leader: None },
            tex_state::node::Node::Char { ch: 'B', .. },
        ] if stores.glue(*spec) == GlueSpec {
            width: Scaled::from_raw(20 * Scaled::UNITY),
            stretch: Scaled::from_raw(0),
            stretch_order: tex_state::glue::Order::Normal,
            shrink: Scaled::from_raw(0),
            shrink_order: tex_state::glue::Order::Normal,
        }
    ));
}

#[test]
fn invalid_space_factor_reports_and_preserves_the_previous_value() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);

    Executor::new()
        .run(
            &mut InputStack::new(MemoryInput::new(
                r"\noindent\spacefactor=2000\spacefactor=0\count0=\spacefactor",
            )),
            &mut stores,
        )
        .expect("bad space factor should be recoverable");

    assert_eq!(stores.count(0), 2000);
    assert!(support::terminal_effect_text(&stores).contains("Bad space factor (0)"));
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
    let [Node::VList(box_node)] = stores.nodes(box0).testing_decoded() else {
        panic!("register 0 should hold a vbox");
    };
    let line = stores
        .nodes(box_node.children)
        .testing_decoded()
        .iter()
        .find_map(|node| match node {
            Node::HList(line) => Some(line),
            _ => None,
        })
        .expect("paragraph should produce a line");
    let children = stores.nodes(line.children).testing_decoded();
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
    let [Node::HList(ligated_box)] = stores.nodes(ligated).testing_decoded() else {
        panic!("register 0 should hold an hbox");
    };
    let [Node::HList(grouped_box)] = stores.nodes(grouped).testing_decoded() else {
        panic!("register 1 should hold an hbox");
    };

    assert!(matches!(
        stores.nodes(ligated_box.children).testing_decoded().first(),
        Some(Node::Lig {
            ch: '\u{c}',
            orig: ('f', 'i'),
            ..
        })
    ));
    assert!(matches!(
        stores.nodes(grouped_box.children).testing_decoded(),
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
    let [tex_state::node::Node::HList(box_node)] = stores.nodes(box0).testing_decoded() else {
        panic!("register 0 should hold an hbox");
    };
    let children = stores.nodes(box_node.children).testing_decoded();
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
    let [tex_state::node::Node::HList(hbox)] = stores.nodes(hcopy).testing_decoded() else {
        panic!("register 1 should hold an hbox");
    };
    assert!(matches!(
        stores.nodes(hbox.children).testing_decoded(),
        [tex_state::node::Node::Kern { .. }]
    ));

    let vcopy = stores.box_reg(3).expect("vcopy destination");
    let [tex_state::node::Node::VList(vbox)] = stores.nodes(vcopy).testing_decoded() else {
        panic!("register 3 should hold a vbox");
    };
    assert!(matches!(
        stores.nodes(vbox.children).testing_decoded(),
        [tex_state::node::Node::Kern { .. }]
    ));
}

#[test]
fn incompatible_unbox_commands_preserve_registers_and_replay_state() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut setup = InputStack::new(MemoryInput::new(
        "\\setbox0=\\vbox{\\hbox{}}\\setbox1=\\hbox{\\kern1pt}",
    ));
    Executor::new()
        .run(&mut setup, &mut stores)
        .expect("box setup executes");
    let vbox = stores.box_reg(0);
    let hbox = stores.box_reg(1);
    let checkpoint = stores.snapshot();
    let source = "\\unhbox0\\par\\unhcopy0\\par\\unvbox1\\unvcopy1";

    let mut input = InputStack::new(MemoryInput::new(source));
    Executor::new()
        .run(&mut input, &mut stores)
        .expect("incompatible unbox commands recover");
    assert_eq!(stores.box_reg(0), vbox);
    assert_eq!(stores.box_reg(1), hbox);
    let first_hash = stores.snapshot().state_hash();

    stores.rollback(&checkpoint);
    let mut input = InputStack::new(MemoryInput::new(source));
    Executor::new()
        .run(&mut input, &mut stores)
        .expect("incompatible unbox replay recovers");
    assert_eq!(stores.box_reg(0), vbox);
    assert_eq!(stores.box_reg(1), hbox);
    assert_eq!(stores.snapshot().state_hash(), first_hash);
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
    Executor::new()
        .run(&mut bad_assignment, &mut stores)
        .expect("a bare read-only internal reports an illegal case and continues");
    assert!(support::terminal_effect_text(&stores).contains("You can't use `\\badness'"));
}

#[test]
fn etex_lastnodetype_tracks_effective_outer_vertical_tail() {
    // e-TeX short reference manual section 3.3 assigns -1 to an empty list
    // and the e-TRIP node codes 1, 12, and 13 to hlist, kern, and penalty.
    for (material, expected) in [("\\hbox{}", "1"), ("\\kern1pt", "12"), ("\\penalty7", "13")] {
        let mut stores = Universe::new();
        tex_expand::install_expandable_primitives(&mut stores);
        tex_expand::install_etex_expandable_primitives(&mut stores);
        crate::install_unexpandable_primitives(&mut stores);
        crate::install_etex_unexpandable_primitives(&mut stores);
        let mut input = InputStack::new(MemoryInput::new(format!(
            "\\relax{material}\\edef\\result{{\\the\\lastnodetype}}"
        )));

        Executor::new()
            .run(&mut input, &mut stores)
            .expect("lastnodetype program");

        assert_eq!(macro_text(&stores, "result"), expected);
    }
}

#[test]
fn etex_tracingscantokens_closes_after_everyeof() {
    // The e-TeX manual sections 3.2 and 3.6 require `( ` on pseudo-file
    // entry and the matching `)` only when scanning, including everyeof, ends.
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    tex_expand::install_etex_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    crate::install_etex_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\tracingscantokens=1\\everyeof{\\message{EOF}}\\scantokens{\\message{BODY}}\\end",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("traced scantokens program");

    let output = terminal_effect_text(&stores);
    let open = output.find('(').expect("pseudo-file opening trace");
    let body = output.find("BODY").expect("pseudo-file body");
    let eof = output.find("EOF").expect("everyeof body");
    let close = output.find(')').expect("pseudo-file closing trace");
    assert!(open < body && body < eof && eof < close, "{output:?}");
}

#[test]
fn etex_glue_component_and_conversion_enquiries_match_manual_types() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    tex_expand::install_etex_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    crate::install_etex_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\skip0=1pt plus 2fill minus 3fil\\muskip0=4mu plus 5fil\
         \\edef\\result{\\the\\gluestretch\\skip0/\\the\\glueshrink\\skip0/\
         \\the\\gluestretchorder\\skip0,\\the\\glueshrinkorder\\skip0/\
         \\the\\gluetomu\\skip0/\\the\\mutoglue\\muskip0}",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("glue enquiries");
    assert_eq!(
        macro_text(&stores, "result"),
        "2.0pt/3.0pt/2,1/1.0mu plus 2.0fill minus 3.0fil/4.0pt plus 5.0fil"
    );
}

#[test]
fn etex_showtokens_decomposes_unexpanded_balanced_text() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    tex_expand::install_etex_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    crate::install_etex_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\def\\foo#1{X#1}\\showtokens{a \\foo{b}}\\end",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("showtokens");
    assert!(terminal_effect_text(&stores).contains("> a \\foo {b}."));
}

#[test]
fn etex_showtokens_expands_only_to_find_its_opening_brace() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    tex_expand::install_etex_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    crate::install_etex_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\def\\payload{kept}\\showtokens\\expandafter{\\payload}\\end",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("showtokens with expanded opening brace");
    assert!(terminal_effect_text(&stores).contains("> kept."));
}

#[test]
fn etex_showgroups_and_showifs_report_live_checkpointed_stacks() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    tex_expand::install_etex_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    crate::install_etex_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\begingroup\\iftrue\\showgroups\\showifs\\fi\\endgroup\\end",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("stack displays");
    let output = terminal_effect_text(&stores);
    assert!(
        output.contains("### semi simple group (level 1)"),
        "{output:?}"
    );
    assert!(output.contains("### bottom level"), "{output:?}");
    assert!(output.contains("### level 1: \\iftrue"), "{output:?}");
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
    let [tex_state::node::Node::HList(hbox)] = stores.nodes(hbox).testing_decoded() else {
        panic!("register 0 should hold an hbox");
    };
    let [
        tex_state::node::Node::Glue {
            spec,
            kind,
            leader: Some(tex_state::node::LeaderPayload::HList(payload)),
        },
    ] = stores.nodes(hbox.children).testing_decoded()
    else {
        panic!("hbox should contain leader glue with hlist payload");
    };
    assert_eq!(*kind, tex_state::node::GlueKind::Leaders);
    assert_eq!(
        stores.glue(*spec).width.raw(),
        10 * tex_state::scaled::Scaled::UNITY
    );
    assert!(matches!(
        stores.nodes(payload.children).testing_decoded(),
        [tex_state::node::Node::Kern { .. }]
    ));

    let vbox = stores.box_reg(1).expect("vbox register");
    let [tex_state::node::Node::VList(vbox)] = stores.nodes(vbox).testing_decoded() else {
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
    ] = stores.nodes(vbox.children).testing_decoded()
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
    Executor::new()
        .run(
            &mut InputStack::new(MemoryInput::new(
                "\\setbox0=\\hbox{\\leaders\\hbox{}\\global\\count0=7}",
            )),
            &mut missing_glue,
        )
        .expect("leader without proper glue should recover");
    assert_eq!(missing_glue.count(0), 7);
    assert!(
        support::terminal_effect_text(&missing_glue)
            .contains("Leaders not followed by proper glue")
    );
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
fn box_motion_uses_tex_web_shift_amount_signs_and_diagnostics() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\showboxbreadth=100 \\showboxdepth=100 \
         \\setbox0=\\hbox{\\raise2pt\\hbox{}\\lower3pt\\hbox{}} \
         \\setbox1=\\vbox{\\moveleft4pt\\hbox{}\\moveright5pt\\hbox{}} \
         \\showbox0 \\showbox1",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("box motions execute");

    let hbox = stores.box_reg(0).expect("hbox register");
    let [Node::HList(hbox)] = stores.nodes(hbox).testing_decoded() else {
        panic!("register 0 should hold an hbox");
    };
    let [Node::HList(raised), Node::HList(lowered)] = stores.nodes(hbox.children).testing_decoded()
    else {
        panic!("hbox should contain raised and lowered boxes");
    };
    assert_eq!(raised.shift.raw(), -2 * Scaled::UNITY);
    assert_eq!(lowered.shift.raw(), 3 * Scaled::UNITY);

    let vbox = stores.box_reg(1).expect("vbox register");
    let [Node::VList(vbox)] = stores.nodes(vbox).testing_decoded() else {
        panic!("register 1 should hold a vbox");
    };
    let horizontal_shifts: Vec<_> = stores
        .nodes(vbox.children)
        .testing_decoded()
        .iter()
        .filter_map(|node| match node {
            Node::HList(boxed) => Some(boxed.shift.raw()),
            _ => None,
        })
        .collect();
    assert_eq!(horizontal_shifts, [-4 * Scaled::UNITY, 5 * Scaled::UNITY]);

    let log = terminal_effect_text(&stores);
    for shift in ["shifted -2.0", "shifted 3.0", "shifted -4.0", "shifted 5.0"] {
        assert!(log.contains(shift), "missing {shift:?} in {log}");
    }
}

#[test]
fn everypar_replays_through_input_stack_and_mutates_state() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let global = stores.intern("global");
    let count = stores.intern("count");
    let everypar = stores.intern_token_list(&[
        Token::Cs(global.symbol()),
        Token::Cs(count.symbol()),
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
fn paragraph_hpack_appends_overfull_rule_for_insufficient_normal_shrink() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(concat!(
        "\\setbox0=\\vbox{\\hsize=10pt \\overfullrule=5pt ",
        "\\leftskip=8pt minus4pt \\noindent\\kern9pt\\par}"
    )));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("overfull paragraph executes");

    let root = stores.box_reg(0).expect("box0");
    let Some(tex_state::node_arena::NodeRef::VList(vbox)) = stores.nodes(root).first() else {
        panic!("box0 should contain a vbox");
    };
    let has_rule = stores.nodes(vbox.children).iter().any(|node| {
        let tex_state::node_arena::NodeRef::HList(line) = node else {
            return false;
        };
        stores.nodes(line.children).iter().any(|node| {
            matches!(
                node,
                tex_state::node_arena::NodeRef::Rule {
                    width: Some(width),
                    height: None,
                    depth: None,
                } if width.raw() == 5 * Scaled::UNITY
            )
        })
    });
    assert!(
        has_rule,
        "overfull paragraph line should end in a five-point rule"
    );
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
    let [Node::VList(vbox)] = stores.nodes(box0).testing_decoded() else {
        panic!("register 0 should hold a vbox");
    };
    assert!(matches!(
        stores.nodes(vbox.children).testing_decoded(),
        [Node::HList(_)]
    ));
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
    let [Node::VList(vbox)] = stores.nodes(box0).testing_decoded() else {
        panic!("register 0 should hold a vbox");
    };
    let children = stores.nodes(vbox.children).testing_decoded();
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
    let [Node::VList(vbox)] = stores.nodes(box0).testing_decoded() else {
        panic!("register 0 should hold a vbox");
    };
    let line = stores
        .nodes(vbox.children)
        .testing_decoded()
        .iter()
        .find_map(|node| match node {
            Node::HList(line) => Some(line),
            _ => None,
        })
        .expect("paragraph should produce a line");
    let explicit_glue: Vec<_> = stores
        .nodes(line.children)
        .testing_decoded()
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
fn vertical_skip_in_hbox_closes_box_and_retries_in_outer_mode() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\setbox0=\\hbox{\\vfill}"));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("restricted horizontal vskip uses off_save recovery");

    assert!(stores.box_reg(0).is_some());
    assert!(support::terminal_effect_text(&stores).contains("Missing } inserted"));
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
fn negative_prevgraf_is_recoverable_and_leaves_value_unchanged() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\prevgraf=3\\prevgraf=-1"));
    let mut executor = Executor::new();

    executor
        .run(&mut input, &mut stores)
        .expect("negative prevgraf is recoverable");

    assert_eq!(executor.nest().enclosing_vertical_prev_graf(), 3);
    assert!(support::terminal_effect_text(&stores).contains("Bad \\prevgraf"));
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
    let [Node::VList(vbox)] = stores.nodes(box0).testing_decoded() else {
        panic!("register 0 should hold a vbox");
    };
    let lines = stores
        .nodes(vbox.children)
        .testing_decoded()
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
    let Some(tex_state::node::Node::Rule {
        width,
        height,
        depth,
    }) = stores.page_contributions().front()
    else {
        panic!("recent contributions should contain one rule");
    };
    assert_eq!(stores.page_contributions().len(), 1);
    assert_eq!(width.map(tex_state::scaled::Scaled::raw), Some(7 * 65_536));
    assert_eq!(height.map(tex_state::scaled::Scaled::raw), Some(26_214));
    assert_eq!(depth.map(tex_state::scaled::Scaled::raw), Some(0));
}

#[test]
fn hrule_in_restricted_horizontal_mode_reports_and_is_ignored() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\setbox0=\\hbox{\\hrule}"));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("hrule in hbox is recoverable");

    assert!(stores.box_reg(0).is_some());
    assert!(support::terminal_effect_text(&stores).contains("hrule' here except with leaders"));
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
fn macro_parameter_in_vertical_mode_does_not_build_recent_rule() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\hrule width7pt#\\showlists"));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("forbidden macro parameter is diagnosed and consumed");

    assert!(stores.current_page_nodes().is_empty());
    assert_eq!(stores.page_contributions().len(), 1);
    assert!(matches!(
        stores.page_contributions().front(),
        Some(Node::Rule { .. })
    ));
    let log = terminal_effect_text(&stores);
    assert!(log.contains("You can't use `Char { ch: '#', cat: Parameter }' in vertical mode"));
    assert!(log.contains("### recent contributions:"));
}

#[test]
fn outer_paragraph_retains_zero_parskip_after_existing_material() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\vsize=100pt \\parskip=0pt \\hrule \\noindent\\vrule\\par",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("paragraph executes");

    let page = stores.current_page_nodes();
    assert!(page.windows(2).any(|nodes| {
        matches!(
            nodes,
            [
                Node::Rule { .. },
                Node::Glue {
                    spec,
                    kind: tex_state::node::GlueKind::Normal,
                    leader: None,
                },
            ] if stores.glue(*spec) == GlueSpec::ZERO
        )
    }));
}

#[test]
fn vertical_unhbox_of_void_box_still_builds_indented_empty_line() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\vsize=100pt \\parskip=0pt \\hrule \\vskip12pt \\unhbox0 \\par",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("void unhbox paragraph executes");

    assert!(stores.current_page_nodes().iter().any(|node| {
        matches!(
            node,
            Node::HList(line)
                if line.height.raw() == 0
                    && line.depth.raw() == 0
                    && matches!(stores.nodes(line.children).testing_decoded(), [Node::HList(indent), ..] if indent.width == stores.dimen_param(DimenParam::PAR_INDENT))
        )
    }));
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
    assert_eq!(stores.nodes(insert.5).testing_decoded().len(), 2);
}

#[test]
fn explicit_hbox_migrates_vadjust_material_to_enclosing_vlist() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\setbox0=\\vbox{\\hbox{\\vadjust{\\penalty123}}}",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("explicit hbox adjustment migrates");

    let root = stores.box_reg(0).expect("box0");
    let Some(tex_state::node_arena::NodeRef::VList(vbox)) = stores.nodes(root).first() else {
        panic!("box0 should contain a vbox");
    };
    let children = stores.nodes(vbox.children).testing_decoded();
    assert!(matches!(children, [Node::HList(_), Node::Penalty(123)]));
    let Node::HList(hbox) = &children[0] else {
        unreachable!()
    };
    assert!(stores.nodes(hbox.children).is_empty());
}

#[test]
fn nested_hbox_retains_vadjust_through_incompatible_unhbox() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\setbox10=\\vbox to8192pt{\\hbox{\\hbox{\\vadjust{A}}}}%\n\\vrule\\unhbox10\\hrule",
    ));
    Executor::new()
        .run(&mut input, &mut stores)
        .expect("incompatible unboxing recovers without moving nested adjustment material");

    let root = stores
        .box_reg(10)
        .expect("incompatible unhbox leaves box10 intact");
    let Some(tex_state::node_arena::NodeRef::VList(vbox)) = stores.nodes(root).first() else {
        panic!("box10 should remain a vbox");
    };
    let Some(tex_state::node_arena::NodeRef::HList(outer)) = stores.nodes(vbox.children).first()
    else {
        panic!("vbox should retain its outer hbox");
    };
    let Some(tex_state::node_arena::NodeRef::HList(inner)) = stores.nodes(outer.children).first()
    else {
        panic!("outer hbox should retain its inner hbox");
    };
    assert!(matches!(
        stores.nodes(inner.children).first(),
        Some(tex_state::node_arena::NodeRef::Adjust(_))
    ));
    assert!(
        !stores
            .current_page_nodes()
            .iter()
            .any(|node| matches!(node, Node::VList(_)))
    );
}

#[test]
fn empty_negative_width_hbox_does_not_gain_an_overfull_rule() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\overfullrule=5pt \\setbox0=\\hbox to -10pt{}",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("empty negative-width hbox packs");

    let root = stores.box_reg(0).expect("box0");
    let Some(tex_state::node_arena::NodeRef::HList(hbox)) = stores.nodes(root).first() else {
        panic!("box0 should contain an hbox");
    };
    assert_eq!(hbox.width.raw(), -655_360);
    assert!(stores.nodes(hbox.children).is_empty());
}

#[test]
fn vertical_mode_discretionary_hyphen_starts_a_paragraph() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\setbox0=\\vbox{\\-\\par}"));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("discretionary paragraph executes");

    let root = stores.box_reg(0).expect("box0");
    let Some(tex_state::node_arena::NodeRef::VList(vbox)) = stores.nodes(root).first() else {
        panic!("box0 should contain a vbox");
    };
    assert!(matches!(
        stores.nodes(vbox.children).testing_decoded(),
        [Node::HList(_)]
    ));
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
        stores.nodes(content).testing_decoded(),
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
fn vtop_normalizes_paragraph_parameters_locally_before_display() {
    let mut stores = stores_with_fonts();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    let source = concat!(
        "\\font\\f=cmr10 \\f \\hsize=100pt ",
        "\\parshape=1 3pt 13pt \\hangindent=-10pt \\hangafter=-12 \\looseness=-2 ",
        "\\setbox0=\\vtop{\\noindent$$$$}"
    );
    let checkpoint = stores.snapshot();

    let mut input = InputStack::new(MemoryInput::new(source));
    Executor::new()
        .run(&mut input, &mut stores)
        .expect("vtop display executes");

    let first_hash = stores.snapshot().state_hash();
    let root = stores.box_reg(0).expect("box0");
    let Some(tex_state::node_arena::NodeRef::VList(vtop)) = stores.nodes(root).first() else {
        panic!("box0 should contain a vtop");
    };
    assert_eq!(vtop.width.raw(), 50 * Scaled::UNITY);
    let display = stores
        .nodes(vtop.children)
        .iter()
        .find_map(|node| match node {
            tex_state::node_arena::NodeRef::HList(node) if node.display => Some(node),
            _ => None,
        })
        .expect("display box");
    assert_eq!(display.width.raw(), 0);
    assert_eq!(display.shift.raw(), 50 * Scaled::UNITY);

    // begin_box's normal_paragraph assignments are local to the box group.
    assert_eq!(stores.paragraph_shape()[0].indent.raw(), 3 * Scaled::UNITY);
    assert_eq!(
        stores.dimen_param(DimenParam::HANG_INDENT).raw(),
        -10 * Scaled::UNITY
    );
    assert_eq!(stores.int_param(IntParam::HANG_AFTER), -12);
    assert_eq!(stores.int_param(IntParam::LOOSENESS), -2);

    stores.rollback(&checkpoint);
    let mut replay = InputStack::new(MemoryInput::new(source));
    Executor::new()
        .run(&mut replay, &mut stores)
        .expect("vtop display replay executes");
    assert_eq!(stores.snapshot().state_hash(), first_hash);
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
        stores.nodes(content).testing_decoded(),
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
        .testing_decoded()
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
    let [tex_state::node::Node::VList(box_node)] = stores.nodes(box1).testing_decoded() else {
        panic!("box1 should be a vbox");
    };
    let split_glue = stores
        .nodes(box_node.children)
        .testing_decoded()
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
fn vsplit_recovers_a_missing_to_keyword() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\setbox0=\\vbox{}\\setbox1=\\vsplit0 0pt",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("vsplit inserts a missing to keyword");

    assert!(support::terminal_effect_text(&stores).contains("Missing `to' inserted"));
}

#[test]
fn vsplit_leaves_hbox_source_untouched_and_returns_void() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\setbox3=\\hbox{}\\setbox4=\\vsplit3 to 0pt",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("vsplit of hbox is recoverable");

    assert!(stores.box_reg(3).is_some());
    assert!(stores.box_reg(4).is_none());
    assert!(support::terminal_effect_text(&stores).contains("vsplit needs a \\vbox"));
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
    let [tex_state::node::Node::VList(box_node)] = stores.nodes(box7).testing_decoded() else {
        panic!("box7 should be a vbox");
    };
    assert!(
        stores
            .nodes(box_node.children)
            .testing_decoded()
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
        for node in stores.nodes(list).testing_decoded() {
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
fn etex_marks_appends_the_scanned_mark_class() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    tex_expand::install_etex_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    install_etex_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(r"\marks27{classed}\marks-1{class-zero}"));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("classed mark executes");

    let mark = stores
        .current_page_nodes()
        .iter()
        .chain(stores.page_contributions())
        .find(|node| matches!(node, Node::Mark { class: 27, .. }));
    assert!(mark.is_some());
    assert!(
        stores
            .current_page_nodes()
            .iter()
            .chain(stores.page_contributions())
            .any(|node| matches!(node, Node::Mark { class: 0, .. }))
    );
    assert!(terminal_effect_text(&stores).contains("Bad register code (-1)"));
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
fn fire_up_tracks_etex_mark_classes_independently() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    tex_expand::install_etex_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    install_etex_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\output={\\global\\advance\\count0 by 1 \\
         \\ifnum\\count0=1 \\xdef\\pagea{\\topmarks7/\\firstmarks7/\\botmarks7}\\else \\
         \\xdef\\pageb{\\topmarks7/\\firstmarks7/\\botmarks7}\\fi \\shipout\\box255}\\
         \\topskip=0pt \\vsize=1pt \\setbox0=\\hbox{}\\ht0=2pt \\
         \\marks7{A}\\copy0\\penalty-10000",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("classed marks survive page fire-up");

    assert_eq!(macro_text(&stores, "pagea"), "/A/A");
    assert_eq!(macro_text(&stores, "pageb"), "A/A/A");
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
fn output_routine_emits_one_checkpoint_only_after_teardown() {
    let source = "\\output={\\advance\\count0 by 1 \\
                  \\global\\advance\\count1 by 1 \\shipout\\hbox{}\\shipout\\box255}
                  \\count0=10 \\count1=20
                  \\topskip=0pt \\setbox0=\\hbox{}\\copy0 \\penalty-10000";
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(source));
    let mut executor = Executor::new();
    let mut checkpoints = Vec::new();
    executor
        .run_with_recorder_hooks_and_checkpoints(
            &mut input,
            &mut stores,
            &mut NoopRecorder,
            &mut NoopExecHooks,
            &mut checkpoints,
        )
        .expect("custom output routine executes");

    let shipouts = checkpoints
        .iter()
        .filter(|checkpoint| checkpoint.boundary() == EngineBoundary::ShipoutComplete)
        .collect::<Vec<_>>();
    assert_eq!(shipouts.len(), 1);
    let checkpoint = shipouts[0];
    assert_eq!(checkpoint.mode_summary().levels().len(), 1);
    assert_eq!(stores.count(0), 10, "output local was restored");
    assert_eq!(stores.count(1), 21, "output global survived");
    assert!(stores.box_reg(255).is_none(), "output box was consumed");

    stores.set_count(1, 99);
    executor
        .restore_checkpoint(&mut input, &mut stores, checkpoint, |_, _, _| {
            Ok::<_, ()>(MemoryInput::new(source))
        })
        .expect("post-output checkpoint restores");
    assert_eq!(stores.count(1), 21);
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

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("TeX reports and discards box255 left by the output routine");

    assert!(
        support::terminal_effect_text(&stores)
            .contains("Output routine didn't use all of \\box255")
    );
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

    let stats = Executor::new()
        .run(&mut input, &mut stores)
        .expect("TeX recovers from an output loop with default shipout");

    assert_eq!(stats.shipped_artifacts.len(), 1);
    assert!(
        String::from_utf8_lossy(
            stores
                .world()
                .memory_terminal_output()
                .expect("memory output")
        )
        .contains("Output loop---1 consecutive dead cycles")
    );
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
fn end_inside_unterminated_box_reaches_outer_cleanup() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\hbox{A\\end"));

    let stats = Executor::new()
        .run(&mut input, &mut stores)
        .expect("stop command is reconsidered after box recovery");

    assert_eq!(stats.shipped_artifacts.len(), 1);
    assert!(stores.current_page_nodes().is_empty());
    assert!(stores.page_contributions().is_empty());
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
    assert!(stores.paragraph_shape().is_empty());
}

#[test]
fn parshape_assignment_obeys_local_and_global_grouping() {
    let mut local_stores = Universe::new();
    install_unexpandable_primitives(&mut local_stores);
    let mut local_input =
        InputStack::new(MemoryInput::new("\\parshape=1 3pt 40pt{\\parshape=0}\\end"));
    Executor::new()
        .run(&mut local_input, &mut local_stores)
        .expect("locally grouped parshape executes");
    assert_eq!(local_stores.paragraph_shape().len(), 1);
    assert_eq!(local_stores.paragraph_shape()[0].indent.raw(), 3 * 65_536);

    let mut global_stores = Universe::new();
    install_unexpandable_primitives(&mut global_stores);
    let mut global_input =
        InputStack::new(MemoryInput::new("{\\global\\parshape=1 7pt 80pt}\\end"));
    Executor::new()
        .run(&mut global_input, &mut global_stores)
        .expect("globally grouped parshape executes");
    assert_eq!(global_stores.paragraph_shape().len(), 1);
    assert_eq!(global_stores.paragraph_shape()[0].indent.raw(), 7 * 65_536);
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

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("irrelevant long prefix is reported, discarded, and let continues");
    assert!(support::terminal_effect_text(&stores).contains("You can't use `\\long'"));
    let a = stores.symbol("a").expect("let target exists");
    assert_eq!(
        stores.meaning(a),
        Meaning::CharToken {
            ch: 'b',
            cat: Catcode::Letter
        }
    );
}

#[test]
fn interactionmode_reads_and_assigns_globally() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    tex_expand::install_etex_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    crate::install_etex_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\edef\\before{\\the\\interactionmode}\
         \\begingroup\\interactionmode=1\\endgroup\
         \\edef\\after{\\the\\interactionmode}",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("interaction mode assignment");

    assert_eq!(macro_text(&stores, "before"), "3");
    assert_eq!(macro_text(&stores, "after"), "1");
    assert_eq!(
        stores.interaction_mode(),
        tex_state::InteractionMode::Nonstop
    );
}

#[test]
fn interactionmode_rejects_out_of_range_values_without_changing_mode() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    tex_expand::install_etex_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    crate::install_etex_unexpandable_primitives(&mut stores);
    stores.set_interaction_mode(tex_state::InteractionMode::Nonstop);
    let mut input = InputStack::new(MemoryInput::new(
        "\\interactionmode=-1\\edef\\result{\\the\\interactionmode}",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("bad mode recovers");
    assert_eq!(macro_text(&stores, "result"), "1");
    assert!(terminal_effect_text(&stores).contains("Bad interaction mode (-1)"));
}

#[test]
fn etex_showgroups_and_showifs_render_live_nested_stacks() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    tex_expand::install_etex_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    crate::install_etex_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\begingroup\\iftrue\\showgroups\\showifs\\fi\\endgroup",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("stack diagnostics execute");

    let output = support::terminal_effect_text(&stores);
    assert!(output.contains("### semi simple group (level 1) (\\begingroup)"));
    assert!(output.contains("### bottom level"));
    assert!(output.contains("### level 1: \\iftrue"));
}

#[test]
fn protected_prefix_resumes_command_demand_after_unexpanded_tokens() {
    // e-TeX manual section 3.1 / e-TRIP's protected-macro check: tokens
    // returned by `\unexpanded` are suppressed for that expansion step, but
    // protected macros encountered while the prefix scanner continues are
    // expanded before the eventual definition command.
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    tex_expand::install_etex_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    crate::install_etex_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        r"\let\bgroup={\protected\def\two{}\let\three=\two\protected\unexpanded\bgroup\two\protected\three\protected\def\one{\two}}",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("protected prefix chain executes");

    let one = stores.intern("one");
    let Meaning::Macro { definition, flags } = stores.meaning(one) else {
        panic!("one is defined")
    };
    assert!(flags.contains(tex_state::meaning::MeaningFlags::PROTECTED));
    let replacement = stores.macro_definition(definition).replacement_text();
    assert_eq!(stores.tokens(replacement).len(), 1);
    assert!(!terminal_effect_text(&stores).contains("You can't use a prefix"));
}
