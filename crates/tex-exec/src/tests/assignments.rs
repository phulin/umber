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
fn etex_register_definitions_recover_bad_codes_to_register_zero() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    install_etex_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\countdef\\negative=-1 \\negative=7 \\countdef\\large=32768 \\large=8",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("bad register definitions recover");

    assert_eq!(stores.count(0), 8);
    let output = terminal_effect_text(&stores);
    assert!(output.contains("Bad register code (-1)"));
    assert!(output.contains("Bad register code (32768)"));
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
fn restricted_character_definitions_report_and_substitute_zero() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\chardef\\A=256 \\mathchardef\\M=32768 \\count0=\\A \\count1=\\M",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("bad restricted codes are recoverable TeX errors");

    assert_eq!(stores.count(0), 0);
    assert_eq!(stores.count(1), 0);
    let output = terminal_effect_text(&stores);
    assert!(output.contains("Bad character code (256)"));
    assert!(output.contains("Bad mathchar (32768)"));
}

#[test]
fn register_definition_target_terminates_its_own_number_scan() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\skipdef\\s100\\s=7pt "));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("register alias should terminate its definition scan");

    assert_eq!(stores.glue(stores.skip(100)).width.raw(), 7 * 65_536);
}

#[test]
fn parshape_is_an_internal_integer_equal_to_its_line_count() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\parshape=2 1pt 2pt 3pt 4pt \\count0=\\parshape",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("parshape should scan as an internal integer");

    assert_eq!(stores.count(0), 2);
}

#[test]
fn setbox_missing_box_is_recoverable_and_replays_the_rejected_command() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\setbox0=\\count0=7 \\count1=9"));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("TeX backs up a non-box command after scan_box recovery");

    assert!(stores.box_reg(0).is_none());
    assert_eq!(stores.count(0), 7);
    assert_eq!(stores.count(1), 9);
    assert!(terminal_effect_text(&stores).contains("A <box> was supposed to be here"));
}

#[test]
fn extra_endgroup_is_recoverable() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\endgroup \\count0=7"));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("TeX reports and ignores an unmatched endgroup");

    assert_eq!(stores.count(0), 7);
    assert!(terminal_effect_text(&stores).contains("Extra \\endgroup"));
}

#[test]
fn character_definition_substitutes_inaccessible_target_and_replays_bad_token() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\mathchardef A=7 \\count0=9"));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("get_r_token recovery should complete the definition and continue");

    let inaccessible = stores.intern("inaccessible");
    assert_eq!(stores.meaning(inaccessible), Meaning::MathCharGiven(0));
    assert_eq!(stores.count(0), 9);
    assert!(terminal_effect_text(&stores).contains("Missing control sequence inserted"));
}

#[test]
fn macro_definition_substitutes_inaccessible_target_and_replays_body_start() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\outer\\def{}"));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("get_r_token recovery should complete the macro definition");

    let inaccessible = stores.intern("inaccessible");
    let meaning = stores
        .macro_meaning(inaccessible)
        .expect("inaccessible macro definition");
    assert!(stores.tokens(meaning.replacement_text()).is_empty());
    assert!(terminal_effect_text(&stores).contains("Missing control sequence inserted"));
}

#[test]
fn mathchardef_constants_scan_for_penalty_count_ifnum_and_signed_macro_replay() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\mathchardef\\M=10000 \
         \\def\\wrapped{\\M} \
         \\penalty\\M \\penalty-\\wrapped \
         \\count0=\\M \\count1=-\\wrapped \
         \\ifnum\\M=10000 \\count2=1 \\fi \
         \\ifnum-\\wrapped=-10000 \\count3=1 \\fi",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("mathchardef constants scan through all integer consumers");

    assert_eq!(stores.count(0), 10_000);
    assert_eq!(stores.count(1), -10_000);
    assert_eq!(stores.count(2), 1);
    assert_eq!(stores.count(3), 1);
}

#[test]
fn mathchardef_meaning_restores_and_replays_with_identical_state_hash() {
    let source = "\\mathchardef\\M=10000 \
                  {\\mathchardef\\M=20000 \\global\\count0=\\M} \
                  \\count1=\\M";
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    let checkpoint = stores.snapshot();

    let mut first_input = InputStack::new(MemoryInput::new(source));
    Executor::new()
        .run(&mut first_input, &mut stores)
        .expect("first mathchardef replay succeeds");
    assert_eq!(stores.count(0), 20_000);
    assert_eq!(stores.count(1), 10_000);
    let first_hash = stores.snapshot().state_hash();

    stores.rollback(&checkpoint);
    let mut second_input = InputStack::new(MemoryInput::new(source));
    Executor::new()
        .run(&mut second_input, &mut stores)
        .expect("second mathchardef replay succeeds");

    assert_eq!(stores.count(0), 20_000);
    assert_eq!(stores.count(1), 10_000);
    assert_eq!(stores.snapshot().state_hash(), first_hash);
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
fn token_register_runaway_closes_before_outer_macro_and_replays_it() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\outer\\def\\a{}\\toks0={x\\a\\count0=7"));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("outer token closes absorbing token-list scan");

    assert_eq!(
        stores.tokens(stores.toks(0)),
        &[Token::Char {
            ch: 'x',
            cat: Catcode::Letter
        }]
    );
    assert_eq!(stores.count(0), 7);
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
fn plain_medbreak_condition_compares_lastskip_with_named_skip_width() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\skipdef\\medskipamount=42 \
         \\medskipamount=12pt plus 4fil minus 2pt \
         \\vskip 1pt \
         \\ifdim\\lastskip<\\medskipamount \
           \\count0=1 \
         \\else \
           \\count0=2 \
         \\fi",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("Plain-style medbreak condition executes");

    assert_eq!(stores.count(0), 1);
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

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("advance overflow is recoverable");

    assert_eq!(stores.count(0), i32::MAX);
    assert!(terminal_effect_text(&stores).contains("Arithmetic overflow"));
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
