use super::support::*;
use super::*;

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
        "\\font\\f=cmr10 \\relax \\fontdimen2\\f=10pt {\\fontdimen2\\f=20pt \\message{in=\\the\\fontdimen2\\f}}\\message{out=\\the\\fontdimen2\\f}\\end",
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
        "\\font\\f=cmr10 \\relax \\f\\dimen0=1em \\dimen1=1ex \\end",
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
        "\\font\\f=cmr10 \\relax \\f\\fontdimen6\\f=12pt \\dimen0=1em \\end",
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
        "\\message{A=\\the\\font|N=\\fontname\\nullfont}\\font\\foo=cmr10 \\relax \\foo\\message{B=\\the\\font|F=\\fontname\\foo}\\font\\bar=cmr10 at 12pt \\message{C=\\fontname\\bar}\\end",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("font rendering execute");

    let output = terminal_effect_text(&stores);
    assert!(output.contains("A=\\nullfont |N=nullfont"));
    assert!(output.contains("B=\\foo |F=cmr10"));
    assert!(output.contains("C=cmr10 at 12.0pt"));
}

#[test]
fn math_family_font_selectors_are_grouping_aware() {
    let mut stores = stores_with_fonts();
    let mut input = InputStack::new(MemoryInput::new(
        "\\font\\a=cmr10 \\font\\b=cmtt10 \\textfont2=\\a {\\textfont2=\\b \\scriptfont2=\\b}",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("math family font assignments execute");

    let a = font_meaning(&stores, "a");
    assert_eq!(
        stores.math_family_font(tex_state::math::MathFontSize::Text, 2),
        a
    );
    assert_eq!(
        stores.math_family_font(tex_state::math::MathFontSize::Script, 2),
        tex_state::font::NULL_FONT
    );
}
