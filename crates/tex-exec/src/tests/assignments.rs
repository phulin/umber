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
