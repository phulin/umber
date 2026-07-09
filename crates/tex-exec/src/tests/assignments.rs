use super::support::terminal_effect_text;
use super::*;

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
fn dimension_assignment_reports_recoverable_scanner_diagnostic() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\mag=40000 \\dimen0=1truept"));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("dimension assignment executes");

    assert_eq!(stores.mag(), 1000);
    assert_eq!(stores.prepared_mag(), Some(1000));
    assert_eq!(stores.dimen(0).raw(), tex_state::scaled::Scaled::UNITY);
    assert!(
        terminal_effect_text(&stores)
            .contains("! Illegal magnification has been changed to 1000 (40000).")
    );
}

#[test]
fn dimension_arithmetic_reports_recoverable_scanner_diagnostic() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\mag=1200 \\dimen0=0pt \\dimen1=1truept \\mag=2000 \\advance\\dimen0 by 1truept",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("dimension arithmetic executes");

    assert_eq!(stores.mag(), 1200);
    assert_eq!(stores.prepared_mag(), Some(1200));
    assert_eq!(stores.dimen(0).raw(), 54_613);
    assert!(
        terminal_effect_text(&stores)
            .contains("! Incompatible magnification (2000); the previous value will be retained.")
    );
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
fn named_math_glue_parameters_scan_muglue_without_aliasing_muskip_registers() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\thinmuskip=3mu \
         \\medmuskip=4mu plus 2mu minus 4mu \
         \\thickmuskip=5mu \
         {\\advance\\thinmuskip by 1mu \\showthe\\thinmuskip}\
         \\showthe\\thinmuskip \\showthe\\medmuskip \\showthe\\thickmuskip",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("named muglue parameters execute");

    let thin = stores.glue(stores.glue_param(GlueParam::new(15)));
    assert_eq!(thin.width.raw(), 3 * tex_state::scaled::Scaled::UNITY);
    assert_eq!(stores.muskip(15), tex_state::ids::GlueId::ZERO);
    let output = terminal_effect_text(&stores);
    assert!(output.contains("> 4.0mu."));
    assert!(output.contains("> 3.0mu."));
    assert!(output.contains("> 4.0mu plus 2.0mu minus 4.0mu."));
    assert!(output.contains("> 5.0mu."));
}

#[test]
fn ordinary_glue_parameters_recover_mu_units_as_pt() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\baselineskip=3mu"));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("ordinary glue parameter should recover mu units");

    let baseline = stores.glue(stores.glue_param(GlueParam::BASELINE_SKIP));
    assert_eq!(baseline.width.raw(), 3 * tex_state::scaled::Scaled::UNITY);
    assert!(terminal_effect_text(&stores).contains("! Illegal unit of measure (pt inserted)."));
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
    let mut input = InputStack::new(MemoryInput::new("\\catcode`@=12 \\catcode`@=12"));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("catcode assignments execute");
    let after = stores.code_table_generations();

    assert_eq!(stores.catcode('@'), Catcode::Other);
    assert_eq!(after.catcode, before.catcode + 2);
}

#[test]
fn code_table_assignments_obey_groups_global_prefix_and_globaldefs() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "{\\catcode`@=11}{\\global\\catcode`!=11}\\globaldefs=1 \
         {\\catcode`?=11}\\globaldefs=-1 {\\global\\catcode`*=11}",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("code-table assignment scope should match other definitions");

    assert_eq!(stores.catcode('@'), Catcode::Other);
    assert_eq!(stores.catcode('!'), Catcode::Letter);
    assert_eq!(stores.catcode('?'), Catcode::Letter);
    assert_eq!(stores.catcode('*'), Catcode::Other);
}

#[test]
fn catcode_accepts_a_backtick_control_symbol_constant() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\catcode`\\{=1"));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("backtick control symbol constant should not expand");

    assert_eq!(stores.catcode('{'), Catcode::BeginGroup);
}
