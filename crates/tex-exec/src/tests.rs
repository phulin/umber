use super::*;
use crate::executor::NoopExecHooks;
use tex_expand::{EngineMode, ExpansionHooks, NoopRecorder};
use tex_lex::{InputStack, MemoryInput};
use tex_state::env::banks::IntParam;
use tex_state::meaning::{ExpandablePrimitive, Meaning};
use tex_state::stores::Stores;
use tex_state::token::{Catcode, Token};

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
    let mut stores = Stores::new();
    let relax = stores.intern("relax");
    stores.set_meaning(relax, Meaning::Relax);
    let mut input = InputStack::new(MemoryInput::new(""));
    let mut hooks = NoopExecHooks;

    assert_eq!(
        dispatch_delivered_token(
            Mode::Vertical,
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
    let mut stores = Stores::new();
    let token = Token::Char {
        ch: 'x',
        cat: Catcode::Letter,
    };
    let mut input = InputStack::new(MemoryInput::new(""));
    let mut hooks = NoopExecHooks;

    assert_eq!(
        dispatch_delivered_token(Mode::Horizontal, token, &mut input, &mut stores, &mut hooks)
            .expect("character dispatch"),
        DispatchAction::NotConsumed
    );
}

#[test]
fn main_control_uses_get_x_token_and_expands_macros_before_dispatch() {
    let mut stores = Stores::new();
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
    let mut stores = Stores::new();
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
    let mut stores = Stores::new();
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
    let mut stores = Stores::new();
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
fn let_assigns_control_sequence_and_implicit_character_meanings() {
    let mut stores = Stores::new();
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
    let mut stores = Stores::new();
    install_unexpandable_primitives(&mut stores);
    let futurelet = stores.symbol("futurelet").expect("futurelet");
    let mut input = InputStack::new(MemoryInput::new("\\n\\first x"));
    let mut hooks = NoopExecHooks;

    dispatch_delivered_token(
        Mode::Vertical,
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
fn long_prefix_on_let_reports_tex_prefix_error() {
    let mut stores = Stores::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\long\\let\\a=b"));

    let err = Executor::new()
        .run(&mut input, &mut stores)
        .expect_err("prefix is illegal");
    assert!(err.to_string().contains("You can't use a prefix with"));
}

#[test]
fn globaldefs_forces_and_suppresses_global_assignments() {
    let mut stores = Stores::new();
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
    let mut stores = Stores::new();
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
    let mut stores = Stores::new();
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
    let mut stores = Stores::new();
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
    let mut stores = Stores::new();
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
    let mut stores = Stores::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("}"));

    let err = Executor::new()
        .run(&mut input, &mut stores)
        .expect_err("extra right brace is an error");
    assert_eq!(err.to_string(), "Too many }'s.");

    let mut stores = Stores::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\begingroup}"));
    let err = Executor::new()
        .run(&mut input, &mut stores)
        .expect_err("right brace cannot close begingroup");
    assert_eq!(err.to_string(), "Extra }, or forgotten \\endgroup.");

    let mut stores = Stores::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\endgroup"));
    let err = Executor::new()
        .run(&mut input, &mut stores)
        .expect_err("extra endgroup is an error");
    assert_eq!(err.to_string(), "Extra \\endgroup.");

    let mut stores = Stores::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("{\\endgroup"));
    let err = Executor::new()
        .run(&mut input, &mut stores)
        .expect_err("endgroup cannot close brace group");
    assert_eq!(err.to_string(), "\\endgroup ended a group started by {");
}

#[test]
fn register_assignments_cover_sparse_aliases_and_arithmetic() {
    let mut stores = Stores::new();
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
    let mut stores = Stores::new();
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
    let mut stores = Stores::new();
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
    let mut stores = Stores::new();
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
    let mut stores = Stores::new();
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
    let mut stores = Stores::new();
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

fn install_expandable(stores: &mut Stores, name: &str, primitive: ExpandablePrimitive) {
    let symbol = stores.intern(name);
    stores.set_meaning(symbol, Meaning::ExpandablePrimitive(primitive));
}

struct EdefInputHooks;

impl ExpansionHooks<MemoryInput> for EdefInputHooks {
    fn open_input(&mut self, name: &str) -> Result<MemoryInput, String> {
        if name == "inc" {
            Ok(MemoryInput::new("OK"))
        } else {
            Err(format!("unexpected input {name}"))
        }
    }
}
