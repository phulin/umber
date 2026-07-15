use super::support::*;
use super::*;
use tex_state::scaled::Scaled;

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
fn etex_font_character_enquiries_share_loaded_metrics() {
    let mut stores = stores_with_fonts();
    tex_expand::install_expandable_primitives(&mut stores);
    tex_expand::install_etex_expandable_primitives(&mut stores);
    crate::install_etex_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\font\\f=cmr10 \
         \\message{\\iffontchar\\f65Y\\else N\\fi/\\iffontchar\\f255Y\\else N\\fi}\
         \\message{\\the\\fontcharwd\\f65/\\the\\fontcharht\\f65/\\the\\fontchardp\\f65/\\the\\fontcharic\\f65/\\the\\fontcharwd\\f255}\
         \\end",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("font character enquiries");

    let output = terminal_effect_text(&stores);
    assert!(output.contains("Y/N"), "{output:?}");
    assert!(output.contains("/0.0pt"));
    assert!(!output.contains("0.0pt/0.0pt/0.0pt/0.0pt/0.0pt"));
}

#[test]
fn font_file_name_backs_up_the_first_non_character_token() {
    let mut stores = stores_with_fonts();
    tex_expand::install_expandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\font\\a=cmr10\\relax\\message{loaded}\\end",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("font name terminator should be redispatched");

    assert_eq!(stores.font_name(font_meaning(&stores, "a")), "cmr10");
    assert!(terminal_effect_text(&stores).contains("loaded"));
}

#[test]
fn illegal_font_magnification_reports_and_uses_design_size() {
    let mut stores = stores_with_fonts();
    tex_expand::install_expandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\font\\a=cmr10 scaled 32769 \\end"));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("illegal font scale is recoverable");

    let font = font_meaning(&stores, "a");
    assert_eq!(stores.font(font).size(), stores.font(font).design_size());
    assert!(
        terminal_effect_text(&stores)
            .contains("Illegal magnification has been changed to 1000 (32769)")
    );
}

#[test]
fn font_definition_uses_driver_font_resolution_and_records_resolved_path() {
    const CMR10: &[u8] = include_bytes!("../../../tex-fonts/tests/fixtures/cm/cmr10.tfm");
    let mut stores = Universe::with_world(tex_state::World::memory());
    crate::install_unexpandable_primitives(&mut stores);
    stores
        .world_mut()
        .set_memory_file("/fonts/cmr10.tfm", CMR10.to_vec())
        .expect("seed redirected font");
    let snapshot = stores.snapshot();
    let mut input = InputStack::new(MemoryInput::new("\\font\\f=cmr10 \\end"));
    let mut resolvers = MemoryResolvers::new().with_font_root("/fonts");
    let mut context = resolvers.context();

    Executor::new()
        .run_with_context(&mut input, &mut stores, &mut context)
        .expect("font definition resolves through driver hook");

    let font = font_meaning(&stores, "f");
    assert_eq!(
        stores.font(font).path(),
        std::path::Path::new("/fonts/cmr10.tfm")
    );
    assert_eq!(stores.world().input_records().len(), 1);
    assert_eq!(
        stores.world().input_records()[0].path(),
        std::path::Path::new("/fonts/cmr10.tfm")
    );

    stores.rollback(&snapshot);
    assert!(stores.world().input_records().is_empty());
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
fn fontdimen_capacity_boundary_is_injective_and_recovers_without_mutation() {
    let mut stores = stores_with_fonts();
    tex_expand::install_expandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\font\\f=cmr10 \\relax \\fontdimen1\\f=1pt \\fontdimen32768\\f=2pt \\fontdimen32769\\f=9pt \\message{first=\\the\\fontdimen1\\f,max=\\the\\fontdimen32768\\f,bad=\\the\\fontdimen32769\\f}\\end",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("out-of-range fontdimen assignment recovers");

    let font = font_meaning(&stores, "f");
    assert_eq!(stores.font_parameter_count(font), 32_768);
    assert_eq!(
        stores.font_parameter(font, 1),
        Scaled::from_raw(Scaled::UNITY)
    );
    assert_eq!(
        stores.font_parameter(font, 32_768),
        Scaled::from_raw(2 * Scaled::UNITY)
    );
    let output = terminal_effect_text(&stores);
    assert!(output.contains("I ignored this assignment"));
    assert!(output.contains("first=1.0pt,max=2.0pt,bad=0.0pt"));
}

#[test]
fn font_backed_integer_array_can_extend_and_read_entries() {
    let mut stores = stores_with_fonts();
    tex_expand::install_expandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\font\\a=cmr10 at 1sp \\fontdimen8\\a=0sp \\hyphenchar\\a=128 \\fontdimen85\\a=85sp \\end",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("font-backed integer array setup executes");
    let font = font_meaning(&stores, "a");
    assert_eq!(stores.font_hyphen_char(font), 128);
    assert_eq!(stores.font_parameter_count(font), 85);
    assert_eq!(stores.font_parameter(font, 85), Scaled::from_raw(85));
}

#[test]
fn the_fontdimen_reads_the_current_font_selector() {
    let mut stores = stores_with_fonts();
    tex_expand::install_expandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\font\\f=cmr10 \\fontdimen1\\f=1.5pt \\f\\message{slant=\\the\\fontdimen1\\font}\\end",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("current-font fontdimen expands");

    assert!(terminal_effect_text(&stores).contains("slant=1.5pt"));
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
    Executor::new()
        .run(&mut bad, &mut stores)
        .expect("older font growth failure is recoverable");

    let a = font_meaning(&stores, "a");
    assert_eq!(stores.font_parameter(a, 9).raw(), 0);
    assert!(terminal_effect_text(&stores).contains("has only"));
}

#[test]
fn short_tfm_keeps_fontdimen_seven_writable_after_a_later_font_load() {
    let mut stores = stores_with_fonts();
    tex_expand::install_expandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\font\\a=cmmi10 \\font\\b=cmr10 \\fontdimen7\\a=2pt \\message{p7=\\the\\fontdimen7\\a}\\end",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("guaranteed fontdimens remain assignable");

    let a = font_meaning(&stores, "a");
    assert_eq!(stores.font_parameter_count(a), 7);
    assert_eq!(
        stores.font_parameter(a, 7),
        Scaled::from_raw(2 * Scaled::UNITY)
    );
    assert!(terminal_effect_text(&stores).contains("p7=2.0pt"));
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

#[test]
fn math_family_assignment_recovers_bad_family_and_missing_font() {
    let mut stores = stores_with_fonts();
    let mut input = InputStack::new(MemoryInput::new("\\textfont16=\\relax \\textfont1=="));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("bounded family and missing font recover like TeX");

    assert_eq!(
        stores.math_family_font(tex_state::math::MathFontSize::Text, 0),
        tex_state::font::NULL_FONT
    );
    assert_eq!(
        stores.math_family_font(tex_state::math::MathFontSize::Text, 1),
        tex_state::font::NULL_FONT
    );
    let output = terminal_effect_text(&stores);
    assert!(output.contains("Bad number (16)"));
    assert!(output.contains("Missing font identifier"));
}
