use super::support::terminal_effect_text;
use super::*;
use tex_state::scaled::Scaled;

#[test]
fn register_assignments_cover_sparse_aliases_and_arithmetic() {
    let stores = super::core::run_canonical_etex(
        "\\count300 = 7 \\countdef\\foo=300 \\advance\\foo by 5 \\multiply\\foo 3 \\divide\\foo by 2 \\end",
    );

    assert_eq!(stores.count(300), 18);
}

#[test]
fn arithmetic_character_target_is_consumed_before_recovery() {
    let stores = super::core::run_canonical_tex82("\\count0=7 \\advance= \\count1=9 \\end");

    assert_eq!(stores.count(0), 7);
    assert_eq!(stores.count(1), 9);
    assert_eq!(
        terminal_effect_text(&stores)
            .matches("! You can't use `the character =' after \\advance.")
            .count(),
        1
    );
}

#[test]
fn macro_recovery_stops_at_tex82s_global_hundred_error_limit() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    let source = "\\def\\a{#x}".repeat(100) + "\\count0=23\\end";
    control
        .register_root_source(tex_command::SourceRegistration::new(
            tex_command::RegisteredSourceKind::Generated,
            source.into_bytes(),
        ))
        .expect("register canonical source");

    let error = loop {
        match control.step(&mut stores) {
            Ok(MainControlStep::Continue) => {}
            Ok(step) => panic!("hundredth macro-scan error must be fatal, got {step:?}"),
            Err(error) => break error,
        }
    };

    assert_eq!(error.as_fatal(), Some(tex_command::FatalError::TooManyErrors));
    assert_eq!(stores.count(0), 0, "fatal exit skips the later assignment");
}

#[test]
fn tex82_compatibility_rejects_sparse_register_numbers() {
    let stores = super::core::run_canonical_tex82("\\count300=7 \\end");

    assert_eq!(stores.count(0), 7);
    assert_eq!(stores.count(300), 0);
    let output = terminal_effect_text(&stores);
    assert!(output.contains("Bad register code (300)"));
    assert!(output.contains("between 0 and 255"));
}

#[test]
fn etex_extended_register_boundary_matrix_covers_aliases_and_grouping() {
    // e-TeX manual section 3.4 extends all six register families from 0..255
    // to 0..32767. Pin the dense/sparse boundary and the final sparse slot,
    // including definition aliases and both local restoration and global
    // assignment through a group.
    let stores = super::core::run_canonical_etex(concat!(
        "\\nonstopmode ",
        "\\count255=1 \\count256=2 \\count32767=3 ",
        "\\dimen255=1pt \\dimen256=2pt \\dimen32767=3pt ",
        "\\skip255=1pt \\skip256=2pt \\skip32767=3pt ",
        "\\muskip255=1mu \\muskip256=2mu \\muskip32767=3mu ",
        "\\toks255={a} \\toks256={b} \\toks32767={c} ",
        "\\setbox255=\\hbox{a} \\setbox256=\\hbox{b} ",
        "\\setbox32767=\\hbox{c} ",
        "\\countdef\\C=32767 \\dimendef\\D=32767 ",
        "\\skipdef\\S=32767 \\muskipdef\\M=32767 \\toksdef\\T=32767 ",
        "\\mathchardef\\B=32767 ",
        "\\C=30 \\D=30pt \\S=30pt \\M=30mu \\T={z} ",
        "\\advance\\C by1 \\advance\\D by1pt ",
        "\\advance\\S by1pt \\advance\\M by1mu ",
        "\\setbox\\B=\\hbox{z} ",
        "{\\count256=20 \\dimen256=20pt \\skip256=20pt ",
        "\\muskip256=20mu \\toks256={x} \\setbox256=\\hbox{x}} ",
        "{\\global\\count32767=31 \\global\\dimen32767=31pt ",
        "\\global\\skip32767=31pt \\global\\muskip32767=31mu ",
        "\\global\\toks32767={y} \\global\\setbox32767=\\hbox{y}} ",
        "\\setbox254=\\hbox{\\leaders\\copy\\B\\hskip1pt} ",
        "\\output=\\T \\showbox\\B \\count0=44 \\end",
    ));

    for (index, expected) in [(255, 1), (256, 2), (32_767, 31)] {
        assert_eq!(stores.count(index), expected);
        assert_eq!(stores.dimen(index).raw(), expected * Scaled::UNITY);
        assert_eq!(
            stores.glue(stores.skip(index)).width.raw(),
            expected * Scaled::UNITY
        );
        assert_eq!(
            stores.glue(stores.muskip(index)).width.raw(),
            expected * Scaled::UNITY
        );
        assert!(
            stores.box_reg(index).is_some(),
            "box {index} must remain populated"
        );
    }
    assert_eq!(
        stores.count(0),
        44,
        "showbox must consume only its sparse operand"
    );
    assert!(
        stores.box_reg(254).is_some(),
        "leaders must accept a sparse box operand"
    );
    assert_eq!(
        stores.tokens(stores.tok_param(TokParam::OUTPUT)),
        &[Token::Char {
            ch: 'y',
            cat: Catcode::Letter,
        }],
        "output toks assignment must consume the sparse alias value",
    );
    assert_eq!(
        [255, 256, 32_767].map(|index| stores.tokens(stores.toks(index))[0]),
        ['a', 'b', 'y'].map(|ch| Token::Char {
            ch,
            cat: Catcode::Letter
        })
    );
}

#[test]
fn etex_register_definitions_recover_bad_codes_to_register_zero() {
    let stores = super::core::run_canonical_etex(
        "\\countdef\\negative=-1 \\negative=7 \\countdef\\large=32768 \\large=8 \\end",
    );

    assert_eq!(stores.count(0), 8);
    let output = terminal_effect_text(&stores);
    assert!(output.contains("Bad register code (-1)"));
    assert!(output.contains("Bad register code (32768)"));
}

#[test]
fn dimension_assignment_reports_recoverable_scanner_diagnostic() {
    let stores = super::core::run_canonical_tex82("\\mag=40000 \\dimen0=1truept \\end");

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
    let stores = super::core::run_canonical_tex82(
        "\\mag=1200 \\dimen0=0pt \\dimen1=1truept \\mag=2000 \\advance\\dimen0 by 1truept \\end",
    );

    assert_eq!(stores.mag(), 1200);
    assert_eq!(stores.prepared_mag(), Some(1200));
    assert_eq!(stores.dimen(0).raw(), 54_613);
    assert!(terminal_effect_text(&stores).contains(
        "! Incompatible magnification (2000);\n the previous value will be retained (1200)."
    ));
}

#[test]
fn chardef_and_mathchardef_are_internal_integers() {
    let stores = super::core::run_canonical_tex82(
        "\\chardef\\A=65 \\mathchardef\\M=\"7132 \\count0=\\A \\count1=\\M \\end",
    );

    assert_eq!(stores.count(0), 65);
    assert_eq!(stores.count(1), 0x7132);
}

#[test]
fn restricted_character_definitions_report_and_substitute_zero() {
    let stores = super::core::run_canonical_tex82(
        "\\chardef\\A=256 \\mathchardef\\M=32768 \\count0=\\A \\count1=\\M \\end",
    );

    assert_eq!(stores.count(0), 0);
    assert_eq!(stores.count(1), 0);
    let output = terminal_effect_text(&stores);
    assert!(output.contains("Bad character code (256)"));
    assert!(output.contains("Bad mathchar (32768)"));
}

#[test]
fn the_renders_chardef_and_mathchardef_as_internal_integers() {
    let stores = super::core::run_canonical_tex82(
        "\\chardef\\A=65 \\mathchardef\\M=32767 \
         \\edef\\result{\\the\\A/\\the\\M} \\end",
    );

    let result = stores.symbol("result").expect("result macro");
    let result = stores.macro_meaning(result).expect("result meaning");
    let text = stores
        .tokens(result.replacement_text())
        .iter()
        .filter_map(|token| match token {
            Token::Char { ch, .. } => Some(*ch),
            _ => None,
        })
        .collect::<String>();
    assert_eq!(text, "65/32767");
}

#[test]
fn the_non_internal_target_reports_and_substitutes_zero() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    stores.set_count(0, 7);
    let stores = super::core::run_canonical_tex82_with_universe(stores, "\\count0=\\the e%\n\\end");

    assert_eq!(stores.count(0), 0);
    let output = terminal_effect_text(&stores);
    assert!(output.contains("You can't use `the letter e' after \\the"));
    assert!(output.contains("using zero instead"));
}

#[test]
fn register_definition_target_terminates_its_own_number_scan() {
    let stores = super::core::run_canonical_tex82("\\skipdef\\s100\\s=7pt \\end");

    assert_eq!(stores.glue(stores.skip(100)).width.raw(), 7 * 65_536);
}

#[test]
fn parshape_is_an_internal_integer_equal_to_its_line_count() {
    let stores =
        super::core::run_canonical_tex82("\\parshape=2 1pt 2pt 3pt 4pt \\count0=\\parshape \\end");

    assert_eq!(stores.count(0), 2);
}

#[test]
fn setbox_missing_box_is_recoverable_and_replays_the_rejected_command() {
    let stores = super::core::run_canonical_tex82("\\setbox0=\\count0=7 \\count1=9 \\end");

    assert!(stores.box_reg(0).is_none());
    assert_eq!(stores.count(0), 7);
    assert_eq!(stores.count(1), 9);
    assert!(terminal_effect_text(&stores).contains("Improper \\setbox"));
}

#[test]
fn setbox_skips_relax_before_the_box_command() {
    let stores = super::core::run_canonical_tex82(
        "\\setbox0=\\relax\\relax\\hbox{A}\\count0=7 \\end",
    );

    assert!(stores.box_reg(0).is_some());
    assert_eq!(stores.count(0), 7);
    assert!(!terminal_effect_text(&stores).contains("A <box> was supposed to be here"));
}

#[test]
fn extra_endgroup_is_recoverable() {
    let stores = super::core::run_canonical_tex82("\\endgroup \\count0=7 \\end");

    assert_eq!(stores.count(0), 7);
    assert!(terminal_effect_text(&stores).contains("Extra \\endgroup"));
}

#[test]
fn character_definition_substitutes_inaccessible_target_and_replays_bad_token() {
    let mut stores = super::core::run_canonical_tex82("\\mathchardef A=7 \\count0=9 \\end");

    let inaccessible = stores.intern("inaccessible");
    assert_eq!(stores.meaning(inaccessible), Meaning::MathCharGiven(0));
    assert_eq!(stores.count(0), 9);
    assert!(terminal_effect_text(&stores).contains("Missing control sequence inserted"));
}

#[test]
fn macro_definition_substitutes_inaccessible_target_and_replays_body_start() {
    let mut stores = super::core::run_canonical_tex82("\\outer\\def{}\\end");

    let inaccessible = stores.intern("inaccessible");
    let meaning = stores
        .macro_meaning(inaccessible)
        .expect("inaccessible macro definition");
    assert!(stores.tokens(meaning.replacement_text()).is_empty());
    assert!(terminal_effect_text(&stores).contains("Missing control sequence inserted"));
}

#[test]
fn mathchardef_constants_scan_for_penalty_count_ifnum_and_signed_macro_replay() {
    let stores = super::core::run_canonical_tex82(
        "\\mathchardef\\M=10000 \
         \\def\\wrapped{\\M} \
         \\penalty\\M \\penalty-\\wrapped \
         \\count0=\\M \\count1=-\\wrapped \
         \\ifnum\\M=10000 \\count2=1 \\fi \
         \\ifnum-\\wrapped=-10000 \\count3=1 \\fi \\end",
    );

    assert_eq!(stores.count(0), 10_000);
    assert_eq!(stores.count(1), -10_000);
    assert_eq!(stores.count(2), 1);
    assert_eq!(stores.count(3), 1);
}

#[test]
fn mathchardef_meaning_restores_and_replays_with_identical_state_hash() {
    let source = "\\mathchardef\\M=10000 \
                  {\\mathchardef\\M=20000 \\global\\count0=\\M} \
                  \\count1=\\M \\end";
    let mut first = super::core::run_canonical_tex82(source);
    assert_eq!(first.count(0), 20_000);
    assert_eq!(first.count(1), 10_000);
    let first_hash = first.snapshot().state_hash();

    let mut replay = super::core::run_canonical_tex82(source);
    assert_eq!(replay.count(0), 20_000);
    assert_eq!(replay.count(1), 10_000);
    assert_eq!(replay.snapshot().state_hash(), first_hash);
}

#[test]
fn token_register_assignments_scan_balanced_text_and_copy_variables() {
    let stores = super::core::run_canonical_tex82(
        "\\toks0={a{b}c}\\toksdef\\T=1 \\T=\\toks0 \\end",
    );

    assert_eq!(stores.tokens(stores.toks(0)), stores.tokens(stores.toks(1)));
    assert_eq!(stores.tokens(stores.toks(0)).len(), 5);
}

#[test]
#[ignore = "xfail: umber2-alfh.4.26 canonical scan_toks left-brace recovery"]
fn token_register_assignment_uses_tex_scan_left_brace_recovery() {
    let stores = super::core::run_canonical_tex82(concat!(
        "\\let\\open={ ",
        "\\toks0=\\relax\\relax\\open a{b}c} ",
        "\\toks1=x} ",
        "\\count0=7 \\end",
    ));

    assert_eq!(stores.tokens(stores.toks(0)).len(), 5);
    assert_eq!(
        stores.tokens(stores.toks(1)),
        &[Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        }]
    );
    assert_eq!(stores.count(0), 7);
    assert!(terminal_effect_text(&stores).contains("Missing { inserted"));
}

#[test]
fn noexpand_in_edef_preserves_a_token_register_assignment() {
    let stores = super::core::run_canonical_tex82(
        r"\toksdef\T=0 \T={OLD} \edef\set{\noexpand\T={NEW}} \set \end",
    );

    assert_eq!(
        stores.tokens(stores.toks(0)),
        &[
            Token::Char {
                ch: 'N',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: 'E',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: 'W',
                cat: Catcode::Letter,
            },
        ]
    );
}

#[test]
#[ignore = "xfail: umber2-alfh.4.28 canonical runaway recovery retains inserted space"]
fn token_register_runaway_closes_before_outer_macro_and_replays_it() {
    let stores = super::core::run_canonical_tex82(
        "\\outer\\def\\a{}\\toks0={x\\a\\count0=7\\end",
    );

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
fn token_register_output_preserves_nested_expandafter_csname_order() {
    let stores = super::core::run_canonical_tex82(concat!(
        "\\toksdef\\A=0 ",
        "\\def\\source{S} ",
        "\\A={\\expandafter\\global\\expandafter\\let",
        "\\csname target\\endcsname} ",
        "\\expandafter\\the\\expandafter\\A",
        "\\csname source\\endcsname ",
        "\\ifx\\target\\source\\count0=1\\else\\count0=2\\fi \\end",
    ));

    let target = stores.symbol("target").expect("target cs exists");
    let source = stores.symbol("source").expect("source cs exists");
    assert_eq!(
        stores.meaning(target),
        stores.meaning(source),
        "{}",
        terminal_effect_text(&stores),
    );
    assert_eq!(stores.count(0), 1);
}

#[test]
fn token_register_output_is_copied_inside_expanded_message_text() {
    let stores = super::core::run_canonical_tex82(concat!(
        "\\toksdef\\A=0 ",
        "\\def\\a#1{\\message{\\the\\A}} ",
        "\\A={\\a X} ",
        "\\the\\A\\end",
    ));

    assert!(terminal_effect_text(&stores).contains("\\a X"));
}

#[test]
fn glue_arithmetic_preserves_fil_order_rules() {
    let mut stores = Universe::new_with_plain_catcodes();
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
    let mut stores = Universe::new_with_plain_catcodes();
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
    let mut stores = Universe::new_with_plain_catcodes();
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
    let mut stores = Universe::new_with_plain_catcodes();
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
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
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
fn arithmetic_failures_preserve_every_target_after_consuming_the_operand() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(concat!(
        "\\count0=1073741824 \\multiply\\count0 by 2 ",
        "\\dimen0=16383pt \\multiply\\dimen0 by 2 ",
        "\\skip0=1pt plus 2fil minus 3pt \\divide\\skip0 by 0 ",
        "\\count1=41 \\divide\\count1 by 0 ",
        "\\count2=9",
    )));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("arithmetic failures are recoverable");

    assert_eq!(stores.count(0), 1_073_741_824);
    assert_eq!(stores.dimen(0).raw(), 16_383 * Scaled::UNITY);
    let skip = stores.glue(stores.skip(0));
    assert_eq!(skip.width.raw(), Scaled::UNITY);
    assert_eq!(skip.stretch.raw(), 2 * Scaled::UNITY);
    assert_eq!(skip.stretch_order, tex_state::glue::Order::Fil);
    assert_eq!(skip.shrink.raw(), 3 * Scaled::UNITY);
    assert_eq!(stores.count(1), 41);
    assert_eq!(stores.count(2), 9, "all failed operands must be consumed");
    assert_eq!(
        terminal_effect_text(&stores)
            .matches("Arithmetic overflow")
            .count(),
        4
    );
}

#[test]
fn code_table_assignment_validates_and_bumps_generation_on_same_value() {
    let mut stores = Universe::new_with_plain_catcodes();
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
    let mut stores = Universe::new_with_plain_catcodes();
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
    let mut stores = Universe::new_with_plain_catcodes();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\catcode`\\{=1"));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("backtick control symbol constant should not expand");

    assert_eq!(stores.catcode('{'), Catcode::BeginGroup);
}
