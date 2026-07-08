use super::*;
use crate::executor::NoopExecHooks;
use std::collections::HashMap;
use tex_expand::{EngineMode, ExpansionHooks, NoopRecorder};
use tex_lex::{InputStack, MemoryInput};
use tex_state::env::banks::{DimenParam, GlueParam, IntParam, TokParam};
use tex_state::glue::GlueSpec;
use tex_state::meaning::{ExpandablePrimitive, Meaning};
use tex_state::token::{Catcode, Token};
use tex_state::{EffectRecord, PrintSink};
use tex_state::{InteractionMode, Universe};

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
         \\ifcat\\input{a}\\input{b}\\count1=1\\fi\\end",
    ));
    let mut hooks = MemoryInputHooks::new()
        .with_source("left", "1pt")
        .with_source("right", "2pt")
        .with_source("a", "a")
        .with_source("b", "b");

    Executor::new()
        .run_with_recorder_and_hooks(&mut input, &mut stores, &mut NoopRecorder, &mut hooks)
        .expect("conditionals scan through input hooks");

    assert_eq!(stores.count(0), 1);
    assert_eq!(stores.count(1), 1);
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

#[test]
fn globaldefs_forces_and_suppresses_global_assignments() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    stores.enter_group();
    let mut input = InputStack::new(MemoryInput::new(
        "\\globaldefs=1 \\def\\a{A}\\globaldefs=-1 \\gdef\\b{B}",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("globaldefs assignments execute");
    let a = stores.symbol("a").expect("a");
    let b = stores.symbol("b").expect("b");
    assert!(matches!(stores.meaning(a), Meaning::Macro { .. }));
    assert!(matches!(stores.meaning(b), Meaning::Macro { .. }));

    let _ = stores.leave_group();
    assert!(matches!(stores.meaning(a), Meaning::Macro { .. }));
    assert_eq!(stores.meaning(b), Meaning::Undefined);
}

#[test]
fn brace_and_begingroup_groups_restore_local_assignments() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "{\\count0=1\\global\\count1=2}\\begingroup\\count2=3\\endgroup",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("grouping primitives execute");

    assert_eq!(stores.count(0), 0);
    assert_eq!(stores.count(1), 2);
    assert_eq!(stores.count(2), 0);
}

#[test]
fn aftergroup_replays_tokens_fifo_on_group_exit() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\def\\A{\\count0=1}\\def\\B{\\count0=2}{\\aftergroup\\A\\aftergroup\\B}",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("aftergroup executes");

    assert_eq!(stores.count(0), 2);
}

#[test]
fn afterassignment_fires_before_aftergroup_tokens() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\def\\A{\\global\\count0=1}\\def\\B{\\global\\count0=2}{\\aftergroup\\B\\afterassignment\\A\\count1=7}",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("afterassignment and aftergroup execute");

    assert_eq!(stores.count(0), 2);
    assert_eq!(stores.count(1), 0);
}

#[test]
fn afterassignment_slot_is_single_token_and_overwrites_previous() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\def\\A{\\count0=1}\\def\\B{\\count0=2}\\afterassignment\\A\\afterassignment\\B\\count1=7",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("afterassignment executes");

    assert_eq!(stores.count(0), 2);
    assert_eq!(stores.count(1), 7);
}

#[test]
fn group_mismatch_errors_use_tex_primary_text() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("}"));

    let err = Executor::new()
        .run(&mut input, &mut stores)
        .expect_err("extra right brace is an error");
    assert_eq!(err.to_string(), "Too many }'s.");

    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\begingroup}"));
    let err = Executor::new()
        .run(&mut input, &mut stores)
        .expect_err("right brace cannot close begingroup");
    assert_eq!(err.to_string(), "Extra }, or forgotten \\endgroup.");

    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\endgroup"));
    let err = Executor::new()
        .run(&mut input, &mut stores)
        .expect_err("extra endgroup is an error");
    assert_eq!(err.to_string(), "Extra \\endgroup.");

    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("{\\endgroup"));
    let err = Executor::new()
        .run(&mut input, &mut stores)
        .expect_err("endgroup cannot close brace group");
    assert_eq!(err.to_string(), "\\endgroup ended a group started by {");
}

#[test]
fn register_assignments_cover_sparse_aliases_and_arithmetic() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\count300 = 7 \\countdef\\foo=300 \\advance\\foo by 5 \\multiply\\foo 3 \\divide\\foo by 2",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("register assignments execute");

    assert_eq!(stores.count(300), 18);
}

#[test]
fn chardef_and_mathchardef_are_internal_integers() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\chardef\\A=65 \\mathchardef\\M=\"7132 \\count0=\\A \\count1=\\M",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("character definitions execute");

    assert_eq!(stores.count(0), 65);
    assert_eq!(stores.count(1), 0x7132);
}

#[test]
fn token_register_assignments_scan_balanced_text_and_copy_variables() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\toks0={a{b}c}\\toksdef\\T=1 \\T=\\toks0",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("token assignments execute");

    assert_eq!(stores.tokens(stores.toks(0)), stores.tokens(stores.toks(1)));
    assert_eq!(stores.tokens(stores.toks(0)).len(), 5);
}

#[test]
fn glue_arithmetic_preserves_fil_order_rules() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\skip0=1pt plus 2fil minus 6pt \\advance\\skip0 by 3pt plus 4fill minus 1pt \\divide\\skip0 by 2",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("glue arithmetic executes");
    let spec = stores.glue(stores.skip(0));

    assert_eq!(spec.width.raw(), 2 * tex_state::scaled::Scaled::UNITY);
    assert_eq!(spec.stretch.raw(), 2 * tex_state::scaled::Scaled::UNITY);
    assert_eq!(spec.stretch_order, tex_state::glue::Order::Fill);
    assert_eq!(spec.shrink.raw(), 7 * tex_state::scaled::Scaled::UNITY / 2);
    assert_eq!(spec.shrink_order, tex_state::glue::Order::Normal);
}

#[test]
fn arithmetic_overflow_reports_tex_error_text() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\count0=2147483647 \\advance\\count0 by 1",
    ));

    let err = Executor::new()
        .run(&mut input, &mut stores)
        .expect_err("advance should overflow");

    assert_eq!(err.to_string(), "Arithmetic overflow");
}

#[test]
fn code_table_assignment_validates_and_bumps_generation_on_same_value() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let before = stores.code_table_generations();
    let mut input = InputStack::new(MemoryInput::new("\\catcode`\\@=12 \\catcode`\\@=12"));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("catcode assignments execute");
    let after = stores.code_table_generations();

    assert_eq!(stores.catcode('@'), Catcode::Other);
    assert_eq!(after.catcode, before.catcode + 2);
}

fn install_expandable(stores: &mut Universe, name: &str, primitive: ExpandablePrimitive) {
    let symbol = stores.intern(name);
    stores.set_meaning(symbol, Meaning::ExpandablePrimitive(primitive));
}

#[test]
fn openin_read_defines_control_sequence_from_world_stream() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    stores
        .world_mut()
        .set_memory_file("stream.tex", b"abc\nnext".to_vec())
        .expect("seed stream");
    let mut input = InputStack::new(MemoryInput::new(
        "\\openin1=stream.tex \\read1 to \\foo \\message{\\foo}\\end",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("read from opened stream");

    let output = terminal_effect_text(&stores);
    assert!(output.contains("abc"));
    assert!(
        !stores
            .world()
            .input_stream_eof(tex_state::StreamSlot::new(1))
    );
}

#[test]
fn read_consumes_additional_stream_lines_until_braces_balance() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    stores
        .world_mut()
        .set_memory_file("stream.tex", b"{abc\ndef}\nnext".to_vec())
        .expect("seed stream");
    let mut input = InputStack::new(MemoryInput::new(
        "\\openin1=stream.tex \\read1 to \\foo \\message{\\foo}\\end",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("read balanced multiline stream");

    let output = terminal_effect_text(&stores);
    assert!(output.contains("abc"));
    assert!(output.contains("def"));
    assert!(
        !stores
            .world()
            .input_stream_eof(tex_state::StreamSlot::new(1))
    );
}

#[test]
fn read_stream_cursor_rolls_back_with_universe_snapshot() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    stores
        .world_mut()
        .set_memory_file("stream.tex", b"one\ntwo".to_vec())
        .expect("seed stream");
    let mut open = InputStack::new(MemoryInput::new("\\openin1=stream.tex"));
    Executor::new()
        .run(&mut open, &mut stores)
        .expect("open stream");
    let snapshot = stores.snapshot();

    let mut first = InputStack::new(MemoryInput::new("\\read1 to \\foo \\message{\\foo}\\end"));
    Executor::new()
        .run(&mut first, &mut stores)
        .expect("first read");
    assert!(terminal_effect_text(&stores).contains("one"));

    stores.rollback(&snapshot);
    let mut second = InputStack::new(MemoryInput::new("\\read1 to \\foo \\message{\\foo}\\end"));
    Executor::new()
        .run(&mut second, &mut stores)
        .expect("reread after rollback");

    assert!(terminal_effect_text(&stores).contains("one"));
    assert!(
        !stores
            .world()
            .input_stream_eof(tex_state::StreamSlot::new(1))
    );
}

#[test]
fn read_at_open_stream_eof_defines_empty_line_and_closes_stream() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    stores
        .world_mut()
        .set_memory_file("stream.tex", b"abc".to_vec())
        .expect("seed stream");
    let mut input = InputStack::new(MemoryInput::new(
        "\\openin1=stream.tex \\read1 to \\foo \\read1 to \\bar \\message{[\\bar]}\\end",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("read EOF line");

    assert!(
        stores
            .world()
            .stream_bufs()
            .read_stream_target(tex_state::StreamSlot::new(1))
            .is_none()
    );
    let bar = stores.symbol("bar").expect("bar was defined");
    assert!(
        stores.macro_meaning(bar).is_some(),
        "EOF read still defines the target macro"
    );
}

#[test]
fn read_missing_stream_in_nonstop_mode_errors_without_terminal_prompt() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    stores.set_interaction_mode(InteractionMode::Nonstop);
    let mut input = InputStack::new(MemoryInput::new("\\openin1=missing.tex \\read1 to \\foo"));

    let err = Executor::new()
        .run(&mut input, &mut stores)
        .expect_err("nonstop mode cannot read terminal");

    assert_eq!(
        err.to_string(),
        "I can't \\read from terminal in nonstop modes"
    );
}

#[test]
fn read_missing_stream_in_errorstop_mode_uses_terminal_line() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    stores.set_interaction_mode(InteractionMode::ErrorStop);
    stores
        .world_mut()
        .push_memory_terminal_line("typed")
        .expect("seed terminal");
    let mut input = InputStack::new(MemoryInput::new(
        "\\openin1=missing.tex \\read1 to \\foo \\message{\\foo}\\end",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("terminal read");

    let output = terminal_effect_text(&stores);
    assert!(output.contains("\\foo="));
    assert!(output.contains("typed"));
}

#[test]
fn openout_closeout_append_world_effect_records() {
    let mut stores = Universe::new();
    crate::install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\openout2=out.aux \\closeout2\\end"));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("openout closeout");

    assert!(matches!(
        stores.world().effect_records(),
        [
            EffectRecord::StreamOpen { slot, target },
            EffectRecord::StreamClose { slot: close_slot }
        ] if *slot == tex_state::StreamSlot::new(2)
            && *close_slot == tex_state::StreamSlot::new(2)
            && target.path() == std::path::Path::new("out.aux")
    ));
}

#[test]
fn font_definition_loads_tfm_via_world_and_reuses_identity() {
    let mut stores = stores_with_fonts();
    let mut input = InputStack::new(MemoryInput::new("\\font\\a=cmr10 \\font\\b=cmr10 \\end"));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("font definitions execute");

    let a = font_meaning(&stores, "a");
    let b = font_meaning(&stores, "b");
    assert_eq!(a, b);
    assert_eq!(stores.font_name(a), "cmr10");
    assert_eq!(stores.world().input_records().len(), 2);
}

#[test]
fn fontdimen_assignment_is_grouping_aware() {
    let mut stores = stores_with_fonts();
    tex_expand::install_expandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\font\\f=cmr10 \\fontdimen2\\f=10pt {\\fontdimen2\\f=20pt \\message{in=\\the\\fontdimen2\\f}}\\message{out=\\the\\fontdimen2\\f}\\end",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("fontdimen assignments execute");

    let output = terminal_effect_text(&stores);
    assert!(output.contains("in=20.0pt"));
    assert!(output.contains("out=10.0pt"));
}

#[test]
fn fontdimen_growth_is_limited_to_most_recently_loaded_font() {
    let mut stores = stores_with_fonts();
    let mut ok = InputStack::new(MemoryInput::new(
        "\\font\\a=cmr10 \\fontdimen8\\a=1pt \\end",
    ));
    Executor::new()
        .run(&mut ok, &mut stores)
        .expect("last loaded font may grow");

    let mut bad = InputStack::new(MemoryInput::new(
        "\\font\\b=cmtt10 \\fontdimen9\\a=2pt \\end",
    ));
    let err = Executor::new()
        .run(&mut bad, &mut stores)
        .expect_err("older font cannot grow");

    assert!(err.to_string().contains("CannotGrow"));
}

#[test]
fn scanner_em_ex_units_use_current_font_parameters() {
    let mut stores = stores_with_fonts();
    let mut input = InputStack::new(MemoryInput::new(
        "\\font\\f=cmr10 \\f\\dimen0=1em \\dimen1=1ex \\end",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("em/ex assignments execute");

    let font = font_meaning(&stores, "f");
    assert_eq!(stores.dimen(0), stores.font_parameter(font, 6));
    assert_eq!(stores.dimen(1), stores.font_parameter(font, 5));
}

#[test]
fn scanner_em_ex_units_are_zero_for_nullfont() {
    let mut stores = stores_with_fonts();
    let mut input = InputStack::new(MemoryInput::new("\\dimen0=1em \\dimen1=1ex \\end"));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("nullfont em/ex assignments execute");

    assert_eq!(stores.dimen(0).raw(), 0);
    assert_eq!(stores.dimen(1).raw(), 0);
}

#[test]
fn scanner_em_unit_observes_runtime_fontdimen_write() {
    let mut stores = stores_with_fonts();
    let mut input = InputStack::new(MemoryInput::new(
        "\\font\\f=cmr10 \\f\\fontdimen6\\f=12pt \\dimen0=1em \\end",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("fontdimen write affects em");

    assert_eq!(stores.dimen(0).raw(), 12 * tex_state::scaled::Scaled::UNITY);
}

#[test]
fn nullfont_the_font_and_fontname_render_from_font_state() {
    let mut stores = stores_with_fonts();
    tex_expand::install_expandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\message{A=\\the\\font|N=\\fontname\\nullfont}\\font\\foo=cmr10 \\foo\\message{B=\\the\\font|F=\\fontname\\foo}\\font\\bar=cmr10 at 12pt \\message{C=\\fontname\\bar}\\end",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("font rendering execute");

    let output = terminal_effect_text(&stores);
    assert!(output.contains("A=\\nullfont |N=nullfont"));
    assert!(output.contains("B=\\foo |F=cmr10"));
    assert!(output.contains("C=cmr10 at 12.0pt"));
}

fn terminal_effect_text(stores: &Universe) -> String {
    let mut output = String::new();
    for record in stores.world().effect_records() {
        if let EffectRecord::StreamWrite { sink, text } = record
            && matches!(
                sink,
                PrintSink::Terminal | PrintSink::TerminalAndLog | PrintSink::Log
            )
        {
            output.push_str(text);
        }
    }
    output
}

fn stores_with_fonts() -> Universe {
    const CMR10: &[u8] = include_bytes!("../../tex-fonts/tests/fixtures/cm/cmr10.tfm");
    const CMTT10: &[u8] = include_bytes!("../../tex-fonts/tests/fixtures/cm/cmtt10.tfm");

    let mut stores = Universe::with_world(tex_state::World::memory());
    crate::install_unexpandable_primitives(&mut stores);
    stores
        .world_mut()
        .set_memory_file("cmr10.tfm", CMR10.to_vec())
        .expect("seed cmr10");
    stores
        .world_mut()
        .set_memory_file("cmtt10.tfm", CMTT10.to_vec())
        .expect("seed cmtt10");
    stores
}

fn font_meaning(stores: &Universe, name: &str) -> tex_state::ids::FontId {
    let symbol = stores.symbol(name).expect("font control sequence");
    match stores.meaning(symbol) {
        Meaning::Font(id) => id,
        meaning => panic!("expected font meaning, got {meaning:?}"),
    }
}

struct EdefInputHooks;

impl ExpansionHooks<MemoryInput> for EdefInputHooks {
    fn open_input<C: tex_state::InputReadState>(
        &mut self,
        _input: &mut C,
        name: &str,
    ) -> Result<MemoryInput, String> {
        if name == "inc" {
            Ok(MemoryInput::new("OK"))
        } else {
            Err(format!("unexpected input {name}"))
        }
    }
}

struct MemoryInputHooks {
    sources: HashMap<String, String>,
}

impl MemoryInputHooks {
    fn new() -> Self {
        Self {
            sources: HashMap::new(),
        }
    }

    fn with_source(mut self, name: &str, source: &str) -> Self {
        self.sources.insert(name.to_owned(), source.to_owned());
        self
    }
}

impl ExpansionHooks<MemoryInput> for MemoryInputHooks {
    fn open_input<C: tex_state::InputReadState>(
        &mut self,
        _input: &mut C,
        name: &str,
    ) -> Result<MemoryInput, String> {
        self.sources
            .get(name)
            .map(|source| MemoryInput::new(source.clone()))
            .ok_or_else(|| format!("unexpected input {name}"))
    }
}
