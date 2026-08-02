use std::sync::Arc;

use tex_command::{
    CommandObservation, CommandObserver, InputReason, InputTransition, RegisteredSourceKind,
    SourceRegistration,
};
use tex_state::hyphenation::PatternSpec;
use tex_state::page::PageMark;
use tex_state::token::{Catcode, Token};

use super::*;

mod etex_diagnostic_tracing;

fn register_source(control: &mut CanonicalMainControl, bytes: &[u8]) {
    let source = control
        .command_mut()
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(bytes),
        ))
        .expect("source registers");
    control
        .command_mut()
        .open_registered_source(source)
        .expect("source opens");
}

fn register_cmr10_as(control: &mut CanonicalMainControl, stores: &mut Universe, name: &str) {
    const CMR10: &[u8] = include_bytes!("../../../tex-fonts/tests/fixtures/cm/cmr10.tfm");
    stores
        .world_mut()
        .set_memory_file(name, CMR10.to_vec())
        .expect("font fixture installs");
    let metrics = InputReadState::read_input_file(
        &mut stores.input_open_context(),
        std::path::Path::new(name),
    )
    .expect("font fixture reads");
    control.capabilities_mut().register_font(
        name,
        FontResource::Tfm {
            metrics,
            opentype: None,
        },
    );
}

fn run_to_end(control: &mut CanonicalMainControl, stores: &mut Universe) {
    loop {
        match control.step(stores).expect("canonical program executes") {
            MainControlStep::End | MainControlStep::EndOfInput => break,
            MainControlStep::Continue => {}
        }
    }
}

#[test]
fn tracingcommands_reports_only_big_switch_commands_with_live_selector_and_mode() {
    // TeX82 §§299/1030/1211: `show_cur_cmd_chr` runs after `big_switch`'s
    // fetch, not at `reswitch`. Thus only the first prefix is traced; later
    // prefixes and the target are fetched within `prefixed_command`. The
    // `\tracingonline` trace is log-only because that assignment has not yet
    // executed, while the prefix uses the newly live terminal-and-log selector.
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        b"\\tracingcommands=1\\tracingonline=1\\global\\global\\escapechar=64\\end",
    );

    run_to_end(&mut control, &mut stores);

    let terminal = pending_sink_text(&stores, true);
    let log = pending_sink_text(&stores, false);
    assert!(!terminal.contains("tracingonline"));
    assert!(log.contains("{vertical mode: \\tracingonline}"));
    assert!(terminal.contains("{\\global}\n{@end}"), "{terminal:?}");
    assert!(log.contains("{\\global}\n{@end}"), "{log:?}");
    assert!(!terminal.contains("escapechar"), "{terminal:?}");
    assert!(terminal.contains("{@end}"), "{terminal:?}");
}

#[test]
fn tracingcommands_two_traces_nonmacro_expansion_before_big_switch_result() {
    // TeX82 §§299/366--367/1030: non-macro expansion traces inside `expand`,
    // then the settled unexpandable command traces at `reswitch`. The first
    // trace consumes the mode prefix; the second must not repeat it.
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        b"\\tracingcommands=2\\tracingonline=1\\romannumeral0\\relax\\end",
    );

    run_to_end(&mut control, &mut stores);

    let terminal = pending_sink_text(&stores, true);
    let log = pending_sink_text(&stores, false);
    assert!(
        log.contains("{vertical mode: \\tracingonline}\n{\\romannumeral}\n{\\relax}\n{\\end}"),
        "terminal={terminal:?} log={log:?}"
    );
    assert!(!terminal.contains("romannumeral"), "{terminal:?}");
}

#[test]
fn tracingcommands_precedes_recovery_reported_while_scanning_the_command() {
    let mut stores = Universe::new_with_plain_catcodes();
    stores.set_interaction_mode(tex_state::InteractionMode::Nonstop);
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        b"\\tracingcommands=1 \\tracingonline=1 \\openout-1=trace.out\\end",
    );

    run_to_end(&mut control, &mut stores);

    let output = terminal_text(&stores);
    let trace = output
        .find("{\\openout}")
        .unwrap_or_else(|| panic!("§1030 command trace: {output:?}"));
    let error = output.find("! Bad number (-1).").expect("§435 recovery");
    assert!(trace < error, "{output:?}");
}

#[test]
fn tracingcommands_caret_renders_a_nonprintable_live_escapechar() {
    // TeX82 §§58--59/63/298: `print_cmd_chr` reaches `print_esc`, whose
    // escape prefix is printed as a one-character string rather than by the
    // raw `print_char` primitive.
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        b"\\tracingcommands=1\\tracingonline=1\\escapechar=127\\global\\count0=1\\end",
    );

    run_to_end(&mut control, &mut stores);

    let terminal = pending_sink_text(&stores, true);
    assert!(terminal.contains("{^^?global}\n{^^?end}"), "{terminal:?}");
    assert!(!terminal.contains("count"), "{terminal:?}");
    assert!(!terminal.as_bytes().contains(&127), "{terminal:?}");
}

#[test]
fn global_escapechar_survives_off_save_inserted_group_recovery() {
    // TeX82 §§1064/1214: a globally assigned integer parameter remains live
    // while `off_save` backs up the offending command and inserts the closer.
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        b"\\scrollmode\\tracingonline=1\\hbox{\\escapechar=127\\global\\escapechar=256\\end}",
    );

    run_to_end(&mut control, &mut stores);

    assert_eq!(stores.int_param(IntParam::ESCAPE_CHAR), 256);
    let terminal = terminal_text(&stores);
    assert!(terminal.contains("! Missing } inserted."), "{terminal:?}");
}

#[test]
fn tracingcommands_traces_reswitch_but_not_prefixed_command_internal_fetches() {
    // TeX82 §§1030/1045/1211: `reswitch` precedes the diagnostic boundary, so
    // the command fetched by `\ignorespaces` is traced. A later prefix and
    // its target are fetched inside `prefixed_command` and remain untraced.
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        b"\\tracingcommands=1\\tracingonline=1\\global\\global\\count0=1\\ignorespaces\\relax\\end",
    );

    run_to_end(&mut control, &mut stores);

    let terminal = pending_sink_text(&stores, true);
    assert!(
        terminal.contains("{\\global}\n{\\ignorespaces}\n{\\relax}\n{\\end}"),
        "{terminal:?}"
    );
    assert!(!terminal.contains("count"), "{terminal:?}");
}

#[test]
fn disabled_tracingcommands_emits_no_command_diagnostic() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(&mut control, b"\\tracingonline=1\\escapechar=64\\end");

    run_to_end(&mut control, &mut stores);

    assert!(!pending_sink_text(&stores, true).contains("vertical mode:"));
    assert!(!pending_sink_text(&stores, false).contains("vertical mode:"));
}

#[test]
fn tracingmacros_reports_definition_then_arguments_with_live_routing() {
    // TeX82 §§389/400 and §245: the invocation line precedes completed
    // arguments and the live selector controls both routed copies.
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        b"\\def\\pair#1#2{}\\tracingmacros=1 \\tracingonline=1 \\pair CD\\end",
    );

    run_to_end(&mut control, &mut stores);

    let terminal = pending_sink_text(&stores, true);
    let log = pending_sink_text(&stores, false);
    let expected = "\n\\pair ->\n#1<-C\n#2<-D\n";
    assert_eq!(terminal, expected);
    assert_eq!(log, expected);

    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        b"\\def\\pair#1#2{}\\tracingmacros=1 \\pair AB\\end",
    );
    run_to_end(&mut control, &mut stores);
    assert_eq!(
        pending_sink_text(&stores, true),
        "(see the transcript file for additional information)"
    );
    assert_eq!(
        pending_sink_text(&stores, false),
        "\n\\pair ->\n#1<-A\n#2<-B\n"
    );
}

#[test]
fn disabled_tracingmacros_emits_no_macro_diagnostic() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        b"\\def\\pair#1#2{}\\tracingonline=1\\pair AB\\end",
    );

    run_to_end(&mut control, &mut stores);

    assert_eq!(pending_sink_text(&stores, true), "");
    assert_eq!(pending_sink_text(&stores, false), "");
}

#[test]
fn tracingrestores_reports_exact_restoration_through_the_live_selector() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        b"\\tracingrestores=1\\tracingonline=1{\\count0=7}\\end",
    );

    run_to_end(&mut control, &mut stores);

    assert_eq!(pending_sink_text(&stores, true), "{restoring \\count0=0}\n");
    assert_eq!(
        pending_sink_text(&stores, false),
        "{restoring \\count0=0}\n"
    );
}

#[test]
fn tracingrestores_reports_dimension_register_restoration() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        b"\\tracingrestores=1\\tracingonline=1{\\dimen9=1.25pt}\\end",
    );

    run_to_end(&mut control, &mut stores);

    assert_eq!(
        pending_sink_text(&stores, true),
        "{restoring \\dimen9=0.0pt}\n"
    );
}

#[test]
fn output_routine_box255_error_reports_live_command_context() {
    // TeX82 §§1026/1028 reach §82's error after retiring the output token
    // list, while the command-owned source level beneath it remains live.
    let mut stores = Universe::new_with_plain_catcodes();
    stores.set_interaction_mode(tex_state::InteractionMode::Nonstop);
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        b"\\maxdeadcycles=2\\output={\\relax}\\topskip=0pt\\setbox0=\\hbox{}\\copy0\\penalty-10000\\end",
    );

    run_to_end(&mut control, &mut stores);

    let output = terminal_text(&stores);
    let report = concat!(
        "! Output routine didn't use all of \\box255.\n",
        "<to be read again> \n",
        "                   \\end \n",
    );
    assert_eq!(output.matches(report).count(), 2, "{output:?}");
    assert!(!output.contains("<output>"), "{output:?}");
}

#[test]
fn tracingrestores_reports_restored_box_register_value() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        b"\\tracingrestores=1\\tracingonline=1\\setbox7=\\hbox{}{\\setbox7=\\vbox{}}\\end",
    );

    run_to_end(&mut control, &mut stores);

    assert_eq!(
        pending_sink_text(&stores, true),
        "{restoring \\box7=\n\\hbox(0.0+0.0)x0.0}\n"
    );
}

#[test]
fn tracingrestores_reports_retained_globals_and_obeys_routing_and_zero_suppression() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        b"{\\count0=7\\global\\count0=8}\\tracingrestores=1{\\count1=9\\global\\count1=10}{\\count2=11}\\tracingrestores=0{\\count3=12}\\end",
    );

    run_to_end(&mut control, &mut stores);

    assert_eq!(
        pending_sink_text(&stores, true),
        "(see the transcript file for additional information)"
    );
    assert_eq!(
        pending_sink_text(&stores, false),
        "{retaining \\count1=10}\n{restoring \\count2=0}\n"
    );
}

#[test]
fn tracingrestores_reports_retained_integer_parameter_with_live_escapechar() {
    // TeX82 §283 calls `restore_trace` for both retained and restored eqtb
    // words; §252's `show_eqtb` names integer parameters through `print_esc`.
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        b"\\tracingrestores=1\\tracingonline=1{\\escapechar=127\\global\\escapechar=256}\\end",
    );

    run_to_end(&mut control, &mut stores);

    assert_eq!(
        pending_sink_text(&stores, true),
        "{retaining escapechar=256}\n"
    );
}

#[test]
fn tracingrestores_reports_named_glue_parameters_with_exact_specs() {
    // TeX82 §§177/252/283: glue parameters use their §236 control-sequence
    // names and `print_spec` value, for both restored and globally retained
    // save-stack entries. The retained infinite-order component is the
    // negative control against formatting every component as ordinary `pt`.
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\tracingrestores=1\tracingonline=1{\lineskip=1pt plus 2fil minus 3pt}{\baselineskip=1pt\global\baselineskip=4pt plus 5fill}\end",
    );

    run_to_end(&mut control, &mut stores);

    let expected = "{restoring \\lineskip=0.0pt}\n{retaining \\baselineskip=4.0pt plus 5.0fill}\n";
    assert_eq!(pending_sink_text(&stores, true), expected);
    assert_eq!(pending_sink_text(&stores, false), expected);
}

#[test]
fn tracingrestores_reports_primitive_meaning_through_an_alias() {
    // TeX82 §§252/283 render the restored meaning, not the target control
    // sequence twice. An alias is the negative control: `\foo` must be named
    // on the left while primitive `\box` is selected on the right.
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\let\foo=\box\tracingrestores=1\tracingonline=1{\let\foo=\relax}\end",
    );

    run_to_end(&mut control, &mut stores);

    assert_eq!(
        pending_sink_text(&stores, true),
        "{restoring \\foo=\\box}\n"
    );
    assert_eq!(
        pending_sink_text(&stores, false),
        "{restoring \\foo=\\box}\n"
    );
}

#[test]
fn tracingparagraphs_reports_exact_first_pass_break_sequence() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        b"\\tracingparagraphs=1\\tracingonline=1\\linepenalty=10\\parfillskip=0pt plus 1fil\\indent\\par\\end",
    );

    run_to_end(&mut control, &mut stores);

    let expected =
        "@firstpass\n[] \n@\\par via @@0 b=0 p=-10000 d=100\n@@1: line 1.2- t=100 -> @@0\n";
    assert!(terminal_text(&stores).starts_with(expected));
    let log = String::from_utf8_lossy(stores.world().memory_log_output().unwrap_or_default());
    assert!(log.starts_with(expected));
}

#[test]
fn etex_direction_meanings_share_valigns_vertical_mode_paragraph_entry() {
    // TeX82 §1090 keys this transition by the `valign` command code, and
    // e-TeX 2.6 [53a.3826--3883] assigns that code to all four directions.
    for primitive in [
        UnexpandablePrimitive::VAlign,
        UnexpandablePrimitive::BeginL,
        UnexpandablePrimitive::EndL,
        UnexpandablePrimitive::BeginR,
        UnexpandablePrimitive::EndR,
    ] {
        assert!(starts_paragraph_in_vertical_mode(
            Meaning::UnexpandablePrimitive(primitive)
        ));
    }
}

#[test]
fn etex_everyeof_assignment_is_visible_to_scantokens_during_edef() {
    // e-TeX 2.6 etex.ch §24.362 inserts a non-null \everyeof token list
    // before retiring the pseudo-file, including while \edef is defining.
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = canonical_etex_initex(&mut stores);
    register_source(
        &mut control,
        br"\everyeof={\noexpand}\edef\x{\scantokens{\begingroup}\endgroup}\end",
    );
    let mut observations = ObservationRecorder::default();

    run_to_end_observed(&mut control, &mut stores, &mut observations);

    assert!(
        stores
            .tok_param_option(tex_state::env::banks::TokParam::EVERY_EOF)
            .is_some(),
        "the source assignment must remain present"
    );
    assert!(observations.0.iter().any(|event| matches!(
        event,
        CommandObservation::Input(record)
            if record.transition == InputTransition::Push
                && record.reason == InputReason::EveryEof
    )));
}

#[test]
fn etex_fire_up_distinguishes_empty_class_zero_and_sparse_botmarks() {
    // TeX82 §1012 preserves an empty class-zero `bot_mark` pointer as the new
    // `top_mark`, while e-TeX 2.6 `etex.ch` [26.1396] discards an empty old
    // sparse `botmarks` pointer. Only the later `topmarks0` enquiry therefore
    // installs and retires a `mark_text` input level.
    let mut stores = Universe::new_with_plain_catcodes();
    // Stage the exact post-fire-up state proved by output.rs's white-box
    // regression, then cross the command processor's enquiry boundary.
    stores.set_page_mark_class(PageMark::Top, 0, tex_state::ids::TokenListId::EMPTY);
    let mut control = canonical_etex_initex(&mut stores);
    register_source(
        &mut control,
        include_bytes!("../fixtures/etex-empty-botmark-fire-up.tex"),
    );
    let mut observations = ObservationRecorder::default();

    run_to_end_observed(&mut control, &mut stores, &mut observations);

    assert_eq!(
        observations
            .0
            .iter()
            .filter(|event| matches!(
                event,
                CommandObservation::Input(record) if record.reason == InputReason::Mark
            ))
            .count(),
        2,
        "the present-empty class-zero mark pushes and retires; sparse class one remains absent"
    );
}

#[test]
fn write_prints_a_control_character_equal_to_newlinechar_as_a_physical_newline() {
    // TeX82 §§262 and 1370: `token_show` prints character tokens through
    // `print`, whose stream selector recognizes `newlinechar` before the
    // non-printable-character `^^` rendering used for diagnostic strings.
    let mut stores = Universe::new_with_plain_catcodes();
    stores.set_int_param(IntParam::NEWLINE_CHAR, 10);
    let tokens = [
        Token::Char {
            ch: 'A',
            cat: Catcode::Letter,
        },
        Token::Char {
            ch: '\n',
            cat: Catcode::Other,
        },
        Token::Char {
            ch: 'B',
            cat: Catcode::Letter,
        },
    ];

    assert_eq!(canonical_write_text(&tokens, &stores), "A\nB\n");
}

#[test]
fn showtokens_distinguishes_newlinechar_from_other_control_bytes() {
    // TeX82 §§262 and 1297: direct `token_show` output recognizes the live
    // newline character, while another non-printable byte keeps its `^^`
    // spelling. The control-sequence separator is part of `print_cs`.
    let mut stores = Universe::new_with_plain_catcodes();
    stores.set_int_param(IntParam::NEWLINE_CHAR, 10);
    let word = stores.intern("word");
    let tokens = stores.intern_token_list(&[
        Token::Char {
            ch: '\u{1}',
            cat: Catcode::Other,
        },
        Token::Char {
            ch: '\n',
            cat: Catcode::Other,
        },
        Token::Cs(word.symbol()),
        Token::Char {
            ch: 'X',
            cat: Catcode::Letter,
        },
    ]);

    assert_eq!(show_tokens_text(&stores, tokens), "^^A\n\\word X");
}

#[test]
fn meaning_mutation_value_projects_protected_macro_storage_marker() {
    let mut stores = Universe::new_with_plain_catcodes();
    let empty = stores.intern_token_list(&[]);
    let definition = stores.intern_macro(MacroMeaning::new(MeaningFlags::PROTECTED, empty, empty));

    let (value, tokens) = meaning_mutation_value(
        Meaning::Macro {
            definition,
            flags: MeaningFlags::PROTECTED,
        },
        &stores,
    );

    assert_eq!(value, "macro definition");
    assert_eq!(
        tokens.as_deref(),
        Some(
            [
                tex_command::ObservedToken::Character {
                    character: '\u{1}',
                    catcode: Catcode::Comment,
                },
                tex_command::ObservedToken::MacroEndMatch,
            ]
            .as_slice()
        )
    );
}

#[test]
fn etex_unexpanded_replays_protected_macros_as_ordinary_expandable_input() {
    // e-TeX 2.6 change section [27.465] implements `\unexpanded` through
    // `the_toks`, whose `ins_list` result re-enters the enclosing expansion
    // loop. Protection suppresses expansion only while an expanded token
    // list is being built; it is not persistent replay metadata.
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = canonical_etex_initex(&mut stores);
    register_source(
        &mut control,
        br"\protected\def\p{\global\advance\count0 by1}\unexpanded{\p}\end",
    );
    let mut observations = ObservationRecorder::default();

    run_to_end_observed(&mut control, &mut stores, &mut observations);

    let p_deliveries = observations
        .0
        .iter()
        .filter_map(|event| match event {
            CommandObservation::Command(command)
                if command.boundary == tex_command::CommandDeliveryBoundary::Raw
                    && command.spelling
                        == tex_command::ObservedToken::ControlSequence("p".into()) =>
            {
                Some(command.command.as_str())
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(p_deliveries, ["undefined_cs", "call", "call"]);
    assert_eq!(stores.count(0), 1, "terminal: {}", terminal_text(&stores));
}

#[test]
fn etex_optimized_aftergroup_links_tokens_onto_one_backup_level() {
    // TeX82 §§282/326 create one `backed_up` level per saved token. e-TeX
    // 2.6 etex.ch [15.282] instead applies `back_input` only once, then links
    // the remaining tokens onto that level. The TeX82 run is the negative
    // control for the same bounded source microfixture.
    for (profile, expected_backups) in [(CommandProfile::TEX82, 3), (CommandProfile::ETEX26, 1)] {
        let mut stores = Universe::new_with_plain_catcodes();
        let mut control = if profile == CommandProfile::ETEX26 {
            canonical_etex_initex(&mut stores)
        } else {
            CanonicalMainControl::tex82_initex(&mut stores)
        };
        register_source(
            &mut control,
            br"{\aftergroup\relax\aftergroup\relax\aftergroup\relax}\end",
        );
        let mut observations = ObservationRecorder::default();

        run_to_end_observed(&mut control, &mut stores, &mut observations);

        let backups = observations
            .0
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    CommandObservation::Input(record)
                        if record.transition == InputTransition::Backup
                            && record.reason == InputReason::Backup
                )
            })
            .count();
        let relax_deliveries = observations
            .0
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    CommandObservation::Command(command)
                    if command.boundary == tex_command::CommandDeliveryBoundary::Raw
                        && command.spelling
                            == tex_command::ObservedToken::ControlSequence("relax".into())
                )
            })
            .count();
        assert_eq!(backups, expected_backups, "profile {profile:?}");
        assert_eq!(relax_deliveries, 6, "profile {profile:?}");
    }
}

#[test]
fn hbox_group_type_respects_box_context_and_vertical_mode() {
    // TeX82 §1083: a register-bound hbox uses hbox_group (e-TeX code 2),
    // even in vertical mode. The neighboring bare hbox is append-like and
    // therefore uses adjusted_hbox_group (code 3) in that same mode.
    for (source, expected) in [
        (br"\setbox0=\hbox{}".as_slice(), GroupKind::HBox),
        (br"\hbox{}".as_slice(), GroupKind::AdjustedHBox),
    ] {
        let mut stores = Universe::new_with_plain_catcodes();
        let mut control = CanonicalMainControl::tex82_initex(&mut stores);
        control
            .set_fuel_limit(1_000)
            .expect("bounded canonical fuel");
        register_source(&mut control, source);

        assert_eq!(
            control.step(&mut stores).expect("prefix executes"),
            MainControlStep::Continue
        );
        assert_eq!(stores.innermost_group_kind(), Some(expected));
        assert_eq!(
            stores
                .innermost_group_kind()
                .map(tex_state::GroupKind::etex_code),
            Some(if expected == GroupKind::HBox { 2 } else { 3 })
        );
    }
}

#[test]
fn discretionary_parts_execute_live_in_disc_group_without_duplicate_delivery() {
    // TeX82 §§1117/1120: each part returns to main control in restricted
    // horizontal mode under disc_group (e-TeX group code 10). Two macro
    // layers and a conditional make any fixed body-prefetch scheme invalid;
    // the literal `\kern` is the nonmacro negative control for duplicate
    // delivery.
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    stores.install_primitive_meaning(
        "currentgrouptype",
        Meaning::InternalInteger(tex_state::meaning::InternalInteger::CurrentGroupType),
    );
    control
        .set_fuel_limit(10_000)
        .expect("bounded canonical fuel");
    register_source(
        &mut control,
        br"\def\layera{\layerb}
          \def\layerb{\ifnum\currentgrouptype=10
            \global\count0=10
          \else
            \global\count0=-1
          \fi}
          \discretionary{\layera\kern1pt}{}{}",
    );

    run_to_end(&mut control, &mut stores);

    assert_eq!(
        stores.count(0),
        10,
        "body expansion saw disc_group; terminal={}",
        terminal_text(&stores)
    );
    let disc = control
        .modes
        .current_list()
        .nodes()
        .iter()
        .find_map(|node| match node {
            Node::Disc {
                pre, post, replace, ..
            } => Some((*pre, *post, *replace)),
            _ => None,
        })
        .expect("completed discretionary node");
    assert_eq!(
        stores
            .nodes(disc.0)
            .iter()
            .filter(|node| matches!(
                node.to_owned(),
                Node::Kern {
                    amount,
                    ..
                } if amount == Scaled::from_raw(Scaled::UNITY)
            ))
            .count(),
        1,
        "unexpandable body command executes exactly once"
    );
    assert!(stores.nodes(disc.1).is_empty());
    assert!(stores.nodes(disc.2).is_empty());
    assert_eq!(stores.innermost_group_kind(), None);
}

#[test]
fn nested_discretionary_preserves_aftergroup_before_rejecting_the_outer_part() {
    // TeX82 §§282/1120–1121: unsave inserts aftergroup material before
    // build_discretionary scans the next part's left brace. Make that token
    // itself the opener; the literal brace that follows must therefore be an
    // ordinary nested group inside the second part. The inner discretionary
    // simultaneously proves that ActiveDiscretionary is a proper stack, then
    // §1121 rejects it as a forbidden node in the outer discretionary list.
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    control
        .set_fuel_limit(10_000)
        .expect("bounded canonical fuel");
    register_source(
        &mut control,
        br"\let\opener={\noindent
          \discretionary{
            \discretionary{\kern1pt}{}{}
            \aftergroup\opener
          }{\kern2pt}}{\kern3pt}",
    );

    run_to_end(&mut control, &mut stores);

    assert!(
        !control
            .modes
            .current_list()
            .nodes()
            .iter()
            .any(|node| matches!(node, Node::Disc { .. })),
        "the forbidden nested discretionary deletes the outer discretionary"
    );
    assert!(terminal_text(&stores).contains("Improper discretionary list"));
    assert!(
        !terminal_text(&stores).contains("Missing { inserted"),
        "aftergroup token supplied the next part opener"
    );
    assert!(control.active_discretionaries.is_empty());
    assert_eq!(stores.innermost_group_kind(), None);
}

#[test]
fn discretionary_nest_overflow_leaves_group_and_active_stack_untouched() {
    // TeX82 §216 rejects a semantic-nest push before saving any new level.
    // Fatal overflow is committed rather than rolled back, so the
    // discretionary opener must not install disc_group or its executor frame
    // until that bounded push has succeeded.
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(&mut control, br"\noindent\discretionary{}{}{}");
    assert_eq!(
        control.step(&mut stores).expect("paragraph starts"),
        MainControlStep::Continue
    );
    while control.modes.depth() < 41 {
        control
            .modes
            .push(Mode::RestrictedHorizontal)
            .expect("fill the TeX82 semantic nest");
    }

    assert_eq!(
        control.step(&mut stores).expect("fatal overflow succumbs"),
        MainControlStep::End
    );
    assert_eq!(control.modes.depth(), 41);
    assert_eq!(stores.innermost_group_kind(), None);
    assert!(control.active_discretionaries.is_empty());
}

#[test]
fn vtop_resets_inherited_parshape_before_display_line_measurement() {
    // TeX82 §§1051--1052 run `normal_paragraph` after opening a `\vtop`.
    // The display therefore uses the box-local 100pt hsize, not the inherited
    // 12pt second `\parshape` line. The empty display's centered reference
    // point therefore extends the vtop's exact natural width to 50pt.
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    control
        .set_fuel_limit(10_000)
        .expect("bounded canonical fuel");
    register_source(
        &mut control,
        br"\nonstopmode
          \hsize=100pt
          \parshape=2 1pt 11pt 2pt 12pt
          \setbox0=\vtop{\noindent$$\kern5pt$$}
          \end",
    );

    run_to_end(&mut control, &mut stores);

    let root = stores.box_reg(0).expect("vtop is assigned to box 0");
    let Some(Node::VList(boxed)) = stores.nodes(root).first().map(|node| node.to_owned()) else {
        panic!("box 0 holds a vlist");
    };
    assert_eq!(boxed.width.raw(), 3_276_800);
}

#[test]
fn preamble_span_expands_one_token_and_preserves_later_template_meaning() {
    // TeX82 §759 expands exactly the token after each preamble `\span`.
    // Here \A is \relax while the preamble is scanned, then becomes a 3pt
    // kern before the spanned column template executes. The template must
    // retain \A itself and resolve its later meaning, producing exactly 3pt.
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    control
        .set_fuel_limit(20_000)
        .expect("bounded canonical fuel");
    register_source(
        &mut control,
        br"\nonstopmode
          \let\A=\relax
          \setbox0=\vbox{\halign{#&\iftrue\A\span\else\span\fi\span&#\cr
            \def\A{\kern3pt}\span\relax&\relax\cr}}
          \end",
    );

    run_to_end(&mut control, &mut stores);

    let root = stores.box_reg(0).expect("vbox is assigned");
    let Some(Node::VList(boxed)) = stores.nodes(root).first().map(|node| node.to_owned()) else {
        panic!("box 0 holds a vlist");
    };
    assert_eq!(boxed.width.raw(), 3 * Scaled::UNITY);
}

#[test]
fn nested_valign_rows_do_not_contribute_baseline_glue_to_outer_cell_width() {
    // TeX82 §799 appends a finished `\valign` row with a plain horizontal
    // splice. The two row widths therefore total exactly 5pt in the enclosing
    // `\halign` cell; routing them through §679 would insert 12pt baselineskip
    // and make the cell spuriously 17pt wide.
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    control
        .set_fuel_limit(20_000)
        .expect("bounded canonical fuel");
    register_source(
        &mut control,
        br"\nonstopmode
          \setbox0=\vbox{\halign{#\cr
            \valign{#\cr\hbox{\kern2pt}\cr\hbox{\kern3pt}\cr}\cr}}
          \end",
    );

    run_to_end(&mut control, &mut stores);

    let root = stores.box_reg(0).expect("outer vbox is assigned");
    let Some(Node::VList(boxed)) = stores.nodes(root).first().map(|node| node.to_owned()) else {
        panic!("box 0 holds a vlist");
    };
    assert_eq!(boxed.width.raw(), 5 * Scaled::UNITY);
}

fn canonical_etex_initex(stores: &mut Universe) -> CanonicalMainControl {
    tex_command::install_tex82_expandable_primitives(stores);
    tex_command::install_etex_expandable_primitives(stores);
    crate::install_unexpandable_primitives(stores);
    crate::install_etex_unexpandable_primitives(stores);
    CanonicalMainControl::prepared_initex(CommandProfile::ETEX26)
}

#[test]
fn etex_showtokens_uses_recursive_general_text_in_fresh_and_loaded_formats() {
    // e-TeX 2.6 etex.ch [17.3623--3671] routes \showtokens through
    // scan_general_text: its expanded opening-brace search is observable, but
    // the recursive absorbing scope is not a TeX82 scan_toks episode. The
    // following \message is the negative control that still publishes the
    // ordinary §473 absorbing transition.
    let mut initialized = crate::test_harness::universe_with_plain_catcodes();
    let fresh_control = canonical_etex_initex(&mut initialized);
    let format = initialized
        .dump_format()
        .expect("dump extended e-TeX format");
    let loaded = Universe::from_format(tex_state::World::memory(), &format)
        .expect("restore extended e-TeX format");

    for (mut stores, mut control) in [
        (initialized, fresh_control),
        (
            loaded,
            CanonicalMainControl::with_profile(CommandProfile::ETEX26),
        ),
    ] {
        control
            .set_fuel_limit(10_000)
            .expect("bounded canonical fuel");
        register_source(&mut control, br"\showtokens\expandafter{X}\message{Y}\end");
        let mut observations = ObservationRecorder::default();
        run_to_end_observed(&mut control, &mut stores, &mut observations);

        let expandafter = observations
            .0
            .iter()
            .position(|event| {
                matches!(
                    event,
                    CommandObservation::Command(command)
                        if command.boundary == tex_command::CommandDeliveryBoundary::Raw
                            && command.command == "expand_after"
                )
            })
            .expect("showtokens opener expands through expandafter");
        let absorbing: Vec<_> = observations
            .0
            .iter()
            .enumerate()
            .filter_map(|(index, event)| {
                matches!(
                    event,
                    CommandObservation::ScannerStatus(status)
                        if status.from == "normal" && status.to == "absorbing"
                )
                .then_some(index)
            })
            .collect();
        assert_eq!(
            absorbing.len(),
            1,
            "only the ordinary message scan publishes absorbing status"
        );
        assert!(
            expandafter < absorbing[0],
            "showtokens must expose its opener before the negative control"
        );
    }
}

#[test]
fn etex_raw_font_character_enquiries_are_forbidden_without_scanning_in_every_mode() {
    // e-TeX 2.6 etex.ch [3413--3453] registers these four read-only
    // dimensions as `last_item`. TeX82 §1048's `any_mode(last_item)` sends a
    // command delivered directly to main control through `report_illegal_case`;
    // its font and character operands are scanned only when a surrounding
    // internal-value scanner consumes it.
    for source in [
        br"\nonstopmode \fontcharwd a\fontcharht b\fontchardp c\fontcharic d\end".as_slice(),
        br"\nonstopmode x\fontcharwd a\fontcharht b\fontchardp c\fontcharic d\end",
        br"\nonstopmode \hbox{\fontcharwd a\fontcharht b\fontchardp c\fontcharic d}\end",
        br"\nonstopmode \vbox{\fontcharwd a\fontcharht b\fontchardp c\fontcharic d}\end",
        br"\nonstopmode $\fontcharwd a\fontcharht b\fontchardp c\fontcharic d$\end",
        br"\nonstopmode $$\fontcharwd a\fontcharht b\fontchardp c\fontcharic d$$\end",
    ] {
        let mut stores = Universe::new_with_plain_catcodes();
        let mut control = canonical_etex_initex(&mut stores);
        control
            .set_fuel_limit(10_000)
            .expect("bounded canonical fuel");
        register_source(&mut control, source);

        run_to_end(&mut control, &mut stores);

        let output = terminal_text(&stores);
        for primitive in ["fontcharwd", "fontcharht", "fontchardp", "fontcharic"] {
            assert!(
                output.contains(&format!("You can't use `\\{primitive}' in ")),
                "{source:?}: {output}"
            );
        }
    }
}

#[test]
fn standalone_internal_integer_shows_live_context_before_scrolled_help() {
    // TeX82 §§82, 90, 1048, and 1111: a standalone `last_item` reaches
    // `report_illegal_case`; `error` shows the live line before routing help
    // off the terminal in nonstop mode.
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    control
        .set_fuel_limit(1_000)
        .expect("bounded canonical fuel");
    register_source(
        &mut control,
        b"\\nonstopmode\n\\hyphenpenalty 89 \\badness\n\\end",
    );

    run_to_end(&mut control, &mut stores);

    let terminal = pending_sink_text(&stores, true);
    assert!(
        terminal.contains(
            "! You can't use `\\badness' in vertical mode.\n\
             l.2 \\hyphenpenalty 89 \\badness"
        ),
        "{terminal}"
    );
    assert!(
        !terminal.contains("Sorry, but I'm not programmed"),
        "{terminal}"
    );
    let log = pending_sink_text(&stores, false);
    assert!(
        log.contains("Sorry, but I'm not programmed to handle this case;"),
        "{log}"
    );
}

#[test]
fn hundredth_standalone_internal_integer_error_terminates_before_later_command() {
    // TeX82 §82: the hundredth scrolled error calls `succumb`, so §1048's
    // illegal `last_item` command cannot return to main control.
    let mut source = "\\nonstopmode\n".to_owned();
    for _ in 0..100 {
        source.push_str("\\badness ");
    }
    source.push_str("\\count0=23\\end");

    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    control
        .set_fuel_limit(10_000)
        .expect("bounded canonical fuel");
    register_source(&mut control, source.as_bytes());

    run_to_end(&mut control, &mut stores);

    assert_eq!(control.fatal_error(), Some(FatalError::TooManyErrors));
    assert_eq!(stores.world().error_channel().error_count(), 100);
    assert_eq!(
        stores.world().error_channel().history(),
        tex_state::print::ErrorHistory::FatalErrorStop
    );
    assert_eq!(stores.count(0), 0, "fatal exit skips the later assignment");
    assert!(
        pending_sink_text(&stores, true).contains("(That makes 100 errors; please try again.)")
    );
}

#[test]
fn errorstop_standalone_internal_integer_prompts_after_live_context_and_resumes() {
    // TeX82 §§82, 90, 1048, and 1111: `report_illegal_case` reaches the
    // interactive advice path after showing context, then resumes on `s`.
    let mut stores = Universe::new_with_plain_catcodes();
    stores
        .world_mut()
        .push_memory_terminal_line("s")
        .expect("memory terminal accepts the error response");
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    control
        .set_fuel_limit(1_000)
        .expect("bounded canonical fuel");
    register_source(&mut control, b"\\badness \\count0=23\\end");

    run_to_end(&mut control, &mut stores);

    let terminal = pending_sink_text(&stores, true);
    let context = terminal.find("l.1 \\badness").expect("live context");
    let prompt = terminal.find("? ").expect("interactive prompt");
    assert!(context < prompt, "{terminal:?}");
    assert_eq!(stores.count(0), 23, "interactive recovery resumes input");
    assert_eq!(stores.world().error_channel().error_count(), 0);
    assert_eq!(control.fatal_error(), None);
}

#[test]
fn etex_raw_font_character_enquiry_loaded_format_checkpoint_retry_is_atomic() {
    // The `last_item` command identity is serialized in an e-TeX format.
    // Restoring a quiescent checkpoint must restore both the diagnostic
    // effect and the unconsumed operand so a retry takes the identical path.
    let mut initex_stores = Universe::new_with_plain_catcodes();
    let _ = canonical_etex_initex(&mut initex_stores);
    let format = initex_stores
        .dump_format()
        .expect("dump extended e-TeX format");
    let mut stores = Universe::from_format(tex_state::World::memory(), &format)
        .expect("restore extended e-TeX format");
    let mut control = CanonicalMainControl::with_profile(CommandProfile::ETEX26);
    control
        .set_fuel_limit(1_000)
        .expect("bounded canonical fuel");
    register_source(&mut control, br"\nonstopmode \fontcharwd a\end");
    assert_eq!(
        control
            .step(&mut stores)
            .expect("interaction mode executes"),
        MainControlStep::Continue
    );
    let checkpoint = control
        .capture_checkpoint(
            crate::EngineBoundary::OuterParagraphEnd,
            &mut stores,
            crate::ExecutionBudgetCounters::default(),
        )
        .expect("raw font enquiry checkpoints");

    assert_eq!(
        control
            .step(&mut stores)
            .expect("raw font enquiry recovers"),
        MainControlStep::Continue
    );
    let first_hash = stores.testing_state_hash();
    let first_output = terminal_text(&stores);
    assert!(first_output.contains("You can't use `\\fontcharwd' in vertical mode"));

    control
        .restore_checkpoint(&checkpoint, &mut stores)
        .expect("raw font enquiry state restores");
    assert_eq!(
        control
            .step(&mut stores)
            .expect("raw font enquiry retry recovers"),
        MainControlStep::Continue
    );
    assert_eq!(stores.testing_state_hash(), first_hash);
    assert_eq!(terminal_text(&stores), first_output);
}

#[test]
fn etex_raw_parshape_enquiries_are_forbidden_without_scanning_in_every_mode() {
    // e-TeX 2.6 etex.ch [3455--3488] registers the coherent parshape
    // enquiry family as `last_item`. TeX82 §1048 therefore diagnoses raw
    // delivery in every mode and leaves each following integer unscanned.
    for source in [
        br"\nonstopmode \parshapelength1\parshapeindent2\parshapedimen3\end".as_slice(),
        br"\nonstopmode x\parshapelength1\parshapeindent2\parshapedimen3\end",
        br"\nonstopmode \hbox{\parshapelength1\parshapeindent2\parshapedimen3}\end",
        br"\nonstopmode \vbox{\parshapelength1\parshapeindent2\parshapedimen3}\end",
        br"\nonstopmode $\parshapelength1\parshapeindent2\parshapedimen3$\end",
        br"\nonstopmode $$\parshapelength1\parshapeindent2\parshapedimen3$$\end",
    ] {
        let mut stores = Universe::new_with_plain_catcodes();
        let mut control = canonical_etex_initex(&mut stores);
        control
            .set_fuel_limit(10_000)
            .expect("bounded canonical fuel");
        register_source(&mut control, source);

        run_to_end(&mut control, &mut stores);

        let output = terminal_text(&stores);
        for primitive in ["parshapelength", "parshapeindent", "parshapedimen"] {
            assert!(
                output.contains(&format!("You can't use `\\{primitive}' in ")),
                "{source:?}: {output}"
            );
        }
    }
}

#[test]
fn etex_parshape_enquiry_loaded_format_checkpoint_retry_is_atomic() {
    let mut initex_stores = Universe::new_with_plain_catcodes();
    let _ = canonical_etex_initex(&mut initex_stores);
    let format = initex_stores
        .dump_format()
        .expect("dump extended e-TeX format");
    let mut stores = Universe::from_format(tex_state::World::memory(), &format)
        .expect("restore extended e-TeX format");
    let mut control = CanonicalMainControl::with_profile(CommandProfile::ETEX26);
    control
        .set_fuel_limit(1_000)
        .expect("bounded canonical fuel");
    register_source(&mut control, br"\nonstopmode \parshapelength1\end");
    assert_eq!(
        control
            .step(&mut stores)
            .expect("interaction mode executes"),
        MainControlStep::Continue
    );
    let checkpoint = control
        .capture_checkpoint(
            crate::EngineBoundary::OuterParagraphEnd,
            &mut stores,
            crate::ExecutionBudgetCounters::default(),
        )
        .expect("raw parshape enquiry checkpoints");

    assert_eq!(
        control
            .step(&mut stores)
            .expect("raw parshape enquiry recovers"),
        MainControlStep::Continue
    );
    let first_hash = stores.testing_state_hash();
    let first_output = terminal_text(&stores);
    assert!(first_output.contains("You can't use `\\parshapelength' in vertical mode"));

    control
        .restore_checkpoint(&checkpoint, &mut stores)
        .expect("raw parshape enquiry state restores");
    assert_eq!(
        control
            .step(&mut stores)
            .expect("raw parshape enquiry retry recovers"),
        MainControlStep::Continue
    );
    assert_eq!(stores.testing_state_hash(), first_hash);
    assert_eq!(terminal_text(&stores), first_output);
}

#[test]
fn empty_equation_number_checks_math_fonts_on_both_sides() {
    // TeX82 §1194 checks the equation-number mlist and then the saved display
    // mlist independently, even though neither one contains a math noad.
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let mut control = canonical_etex_initex(&mut stores);
    control
        .set_fuel_limit(10_000)
        .expect("bounded canonical fuel");
    register_source(&mut control, br"$$\eqno^{}$\end");

    run_to_end(&mut control, &mut stores);

    assert_eq!(
        terminal_text(&stores)
            .matches("Math formula deleted: Insufficient symbol fonts")
            .count(),
        2
    );
}

#[test]
fn noalign_body_dispatches_nested_math_braces_by_save_stack_group() {
    // TeX82 §§785, 1068-1069, and 1133: material inside `no_align_group`
    // runs through ordinary main control. Only a right brace delivered while
    // that group is current ends `\noalign`; braces belonging to nested math
    // groups must close those groups first.
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = canonical_etex_initex(&mut stores);
    control
        .set_fuel_limit(10_000)
        .expect("bounded canonical fuel");
    register_source(
        &mut control,
        br"\valign{#\cr\noalign{$${\left.\middle.\right.}$$}}\end",
    );

    for _ in 0..256 {
        match control
            .step(&mut stores)
            .expect("nested noalign math executes")
        {
            MainControlStep::End | MainControlStep::EndOfInput => return,
            MainControlStep::Continue => {}
        }
    }
    panic!("canonical noalign regression exceeded its step bound");
}

fn run_to_end_observed(
    control: &mut CanonicalMainControl,
    stores: &mut Universe,
    observations: &mut dyn CommandObserver,
) {
    loop {
        match control
            .step_with_observer(stores, observations)
            .expect("canonical program executes")
        {
            MainControlStep::End | MainControlStep::EndOfInput => break,
            MainControlStep::Continue => {}
        }
    }
}

fn terminal_text(stores: &Universe) -> String {
    let committed = stores
        .world()
        .memory_terminal_output()
        .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
        .unwrap_or_default();
    let pending: String = stores
        .world()
        .effect_records()
        .iter()
        .filter_map(|effect| match effect {
            tex_state::EffectRecord::StreamWrite {
                sink:
                    tex_state::PrintSink::Terminal
                    | tex_state::PrintSink::TerminalAndLog
                    | tex_state::PrintSink::Log,
                text,
            } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    committed + &pending
}

#[test]
fn misplaced_alignment_commands_route_exact_help_and_continue() {
    let cases: &[(&[u8], &str, &[&str])] = &[
        (
            b"&",
            "Misplaced alignment tab character &.",
            &[
                "I can't figure out why you would want to use a tab mark",
                "here. If you just want an ampersand, the remedy is",
                "simple: Just type `I\\&' now. But if some right brace",
                "up above has ended a previous alignment prematurely,",
                "you're probably due for more error messages, and you",
                "might try typing `S' now just to see what is salvageable.",
            ],
        ),
        (
            br"\cr",
            "Misplaced \\cr.",
            &[
                "I can't figure out why you would want to use a tab mark",
                "or \\cr or \\span just now. If something like a right brace",
                "up above has ended a previous alignment prematurely,",
                "you're probably due for more error messages, and you",
                "might try typing `S' now just to see what is salvageable.",
            ],
        ),
        (br"\crcr", "Misplaced \\crcr.", &[]),
        (br"\span", "Misplaced \\span.", &[]),
        (
            br"\noalign",
            "Misplaced \\noalign.",
            &[
                "I expect to see \\noalign only after the \\cr of",
                "an alignment. Proceed, and I'll ignore this case.",
            ],
        ),
        (
            br"\omit",
            "Misplaced \\omit.",
            &[
                "I expect to see \\omit only after tab marks or the \\cr of",
                "an alignment. Proceed, and I'll ignore this case.",
            ],
        ),
    ];
    let delimiter_help = cases[1].2;

    for &(command, primary, help) in cases {
        let mut stores = Universe::new_with_plain_catcodes();
        stores
            .world_mut()
            .push_memory_terminal_line("h")
            .expect("memory terminal accepts the help request");
        stores
            .world_mut()
            .push_memory_terminal_line("s")
            .expect("memory terminal accepts the continuation request");
        let mut control = CanonicalMainControl::tex82_initex(&mut stores);
        let mut source = command.to_vec();
        source.extend_from_slice(br"\count0=17\end");
        register_source(&mut control, &source);

        run_to_end(&mut control, &mut stores);

        assert_eq!(
            stores.count(0),
            17,
            "recovery did not continue for {primary}"
        );
        let output = terminal_text(&stores);
        assert!(output.contains(&format!("! {primary}")), "{output}");
        let expected_help = if help.is_empty() {
            delimiter_help
        } else {
            help
        };
        let exact_help = expected_help.join("\n");
        assert!(
            output.contains(&exact_help),
            "missing exact help for {primary}: {output}"
        );
    }
}

#[test]
fn misplaced_category_five_character_routes_car_ret_help() {
    let mut stores = Universe::new_with_plain_catcodes();
    stores.set_interaction_mode(tex_state::InteractionMode::Nonstop);
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(&mut control, b"\\catcode90=5 Z\n\\global\\count0=17\\end");

    run_to_end(&mut control, &mut stores);

    assert_eq!(stores.count(0), 17);
    let output = terminal_text(&stores);
    assert!(
        output.contains("! Misplaced end of line character Z."),
        "{output}"
    );
}

fn pending_sink_text(stores: &Universe, terminal: bool) -> String {
    stores
        .world()
        .effect_records()
        .iter()
        .filter_map(|effect| match effect {
            tex_state::EffectRecord::StreamWrite { sink, text }
                if if terminal {
                    matches!(
                        sink,
                        tex_state::PrintSink::Terminal | tex_state::PrintSink::TerminalAndLog
                    )
                } else {
                    matches!(
                        sink,
                        tex_state::PrintSink::Log | tex_state::PrintSink::TerminalAndLog
                    )
                } =>
            {
                Some(text.as_str())
            }
            _ => None,
        })
        .collect()
}

fn macro_character_text(stores: &Universe, name: &str) -> String {
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
fn etex_identical_local_let_is_a_reassignment_but_global_let_is_not() {
    // e-TeX change [19.277] returns before local `eq_define` when both the
    // command type and equivalent are identical. A global definition still
    // commits, so the two controls distinguish the shortcut from filtering.
    let mut stores = Universe::new_with_plain_catcodes();
    tex_expand::install_expandable_primitives(&mut stores);
    tex_expand::install_etex_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    crate::install_etex_unexpandable_primitives(&mut stores);
    let mut control = CanonicalMainControl::prepared_initex(CommandProfile::ETEX26);
    register_source(
        &mut control,
        br"\catcode123=1 \let\bgroup={ \let\bgroup={ \global\let\bgroup={ \end",
    );
    let mut observations = ObservationRecorder::default();
    loop {
        match control
            .step_with_observer(&mut stores, &mut observations)
            .expect("e-TeX meaning reassignments execute")
        {
            MainControlStep::End | MainControlStep::EndOfInput => break,
            MainControlStep::Continue => {}
        }
    }

    let mutations: Vec<_> = observations
        .0
        .iter()
        .filter_map(|observation| match observation {
            CommandObservation::Mutation(record) if record.target == "meaning" => {
                Some((record.value.as_str(), record.global))
            }
            _ => None,
        })
        .collect();
    assert_eq!(mutations, [("left_brace", false), ("left_brace", true)]);
}

#[test]
fn bare_macro_parameter_reports_illegal_case_and_continues_in_every_mode() {
    // TeX82 §1045: `any_mode(mac_param): report_illegal_case`.
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\nonstopmode
          #
          \noindent#\par
          \hbox{#}
          \vbox{#}
          $#$
          $$#$$
          \count0=7
          \end",
    );

    run_to_end(&mut control, &mut stores);

    let terminal = terminal_text(&stores);
    for mode in [
        "vertical",
        "horizontal",
        "restricted horizontal",
        "internal vertical",
        "math",
        "display math",
    ] {
        assert!(
            terminal.contains(&format!(
                "You can't use `macro parameter character #' in {mode} mode"
            )),
            "missing {mode} diagnostic in {terminal:?}"
        );
    }
    assert_eq!(stores.count(0), 7, "each illegal command is discarded");
}

#[test]
fn bare_macro_parameter_commit_survives_later_input_retry_without_duplication() {
    // The §1045 diagnostic is part of the parameter command's committed
    // operation. A later resource suspension rolls back only its own input
    // attempt and must neither erase nor duplicate the earlier report.
    // The mode is the harness's `\nonstopmode` rather than an explicit
    // `\errorstopmode`: §1045's report is routed to the terminal either way,
    // and errorstop would send §82 into §83's dialog, which this harness's
    // terminal cannot answer and §71 ends the job over.
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(&mut control, br"#\input child\end");

    assert!(matches!(
        control.advance(&mut stores).expect("parameter recovers"),
        CanonicalStepResult::Progress(ReplayStep::Continue)
    ));
    let committed = terminal_text(&stores);
    assert_eq!(committed.matches("macro parameter character #").count(), 1);

    for _ in 0..3 {
        assert!(matches!(
            control.advance(&mut stores).expect("missing input suspends"),
            CanonicalStepResult::Suspended(CanonicalResourceNeed::Input { name })
                if name == "child.tex"
        ));
        assert_eq!(terminal_text(&stores), committed);
    }

    control.capabilities_mut().register_input(
        "child.tex",
        SourceRegistration::new(RegisteredSourceKind::Generated, Arc::<[u8]>::from(&b""[..])),
    );
    run_to_end(&mut control, &mut stores);
    assert_eq!(
        terminal_text(&stores)
            .matches("macro parameter character #")
            .count(),
        1
    );
}

#[test]
fn extra_endcsname_reports_once_and_continues_with_observer_parity_in_every_mode() {
    // TeX82 §1135: `cs_error` diagnoses and ignores one stray `\endcsname`.
    for mode in [
        Mode::Vertical,
        Mode::InternalVertical,
        Mode::Horizontal,
        Mode::RestrictedHorizontal,
        Mode::Math,
        Mode::DisplayMath,
    ] {
        let run = |observed: bool| {
            let mut stores = crate::test_harness::universe_with_plain_catcodes();
            let mut control = CanonicalMainControl::tex82_initex(&mut stores);
            control.set_fuel_limit(128).expect("bounded command fuel");
            if mode != Mode::Vertical {
                control.modes.push(mode).expect("test mode push");
            }
            register_source(&mut control, br"\endcsname\count0=17");
            if observed {
                let mut observations = ObservationRecorder::default();
                for _ in 0..2 {
                    control
                        .step_with_observer(&mut stores, &mut observations)
                        .expect("observed stray endcsname continues");
                }
            } else {
                for _ in 0..2 {
                    control
                        .step(&mut stores)
                        .expect("unobserved stray endcsname continues");
                }
            }
            (
                terminal_text(&stores),
                stores.count(0),
                control.fuel_burned(),
            )
        };

        let unobserved = run(false);
        let observed = run(true);
        assert_eq!(observed, unobserved, "mode {mode:?}");
        // §62's `print_nl` adds no newline at offset 0, so the headline opens
        // the terminal; §82's `show_context` follows it, and §1135's help is
        // last because §90 defers it to the transcript.
        assert_eq!(
            unobserved.0,
            "! Extra \\endcsname.\nl.1 \\endcsname\n              \\count0=17\n\
             I'm ignoring this, since I wasn't doing a \\csname.\n\n",
            "mode {mode:?}"
        );
        assert_eq!(unobserved.1, 17, "mode {mode:?}");
        assert!(unobserved.2 < 128, "mode {mode:?}");
    }
}

#[test]
fn stray_endv_outside_math_runs_off_save_once_and_continues_in_every_mode() {
    // TeX82 §§1130-1131: an end-v outside an alignment runs `off_save`.
    // With no group open, §1066 diagnoses and drops that command.
    for mode in [
        Mode::Vertical,
        Mode::InternalVertical,
        Mode::Horizontal,
        Mode::RestrictedHorizontal,
    ] {
        let mut stores = crate::test_harness::universe_with_plain_catcodes();
        let endv = stores.intern("forcedendv");
        stores.set_meaning(endv, Meaning::EndV);
        let mut control = CanonicalMainControl::tex82_initex(&mut stores);
        control.set_fuel_limit(128).expect("bounded command fuel");
        if mode != Mode::Vertical {
            control.modes.push(mode).expect("test mode push");
        }
        register_source(&mut control, br"\forcedendv\count0=23");

        assert_eq!(
            control.step(&mut stores).expect("stray end-v recovers"),
            MainControlStep::Continue
        );
        // §62's `print_nl` emits no newline at offset 0, so the headline opens
        // the terminal. What follows it is §§310-318's context and the §1131
        // help, whose exact bytes the minifixture channel corpus pins; this
        // test's claim is the diagnosis, not the transcript rendering.
        let terminal = terminal_text(&stores);
        assert!(
            terminal.starts_with("! Extra \\forcedendv.\n"),
            "mode {mode:?}: {terminal}"
        );
        assert_eq!(
            control
                .step(&mut stores)
                .expect("following command executes"),
            MainControlStep::Continue
        );
        assert_eq!(stores.count(0), 23, "mode {mode:?}");
        assert!(control.fuel_burned() < 128, "mode {mode:?}");
    }
}

#[test]
fn stray_endv_in_math_inserts_shift_then_replays_for_off_save() {
    // TeX82 §§1046-1047 insert `$` before the backed-up end-v. Once that
    // closes math, §§1130-1131 see the same command again and run `off_save`.
    for (opening, mode_name) in [
        (br"$".as_slice(), "math"),
        (br"$$".as_slice(), "display math"),
    ] {
        let mut stores = crate::test_harness::universe_with_plain_catcodes();
        let endv = stores.intern("forcedendv");
        stores.set_meaning(endv, Meaning::EndV);
        let mut control = CanonicalMainControl::tex82_initex(&mut stores);
        control.set_fuel_limit(256).expect("bounded command fuel");
        let mut source = opening.to_vec();
        source.extend_from_slice(br"\forcedendv\par\count0=29");
        register_source(&mut control, &source);

        for _ in 0..16 {
            control
                .step(&mut stores)
                .expect("math end-v recovery remains finite");
            if stores.count(0) == 29 {
                break;
            }
        }
        let terminal = terminal_text(&stores);
        assert_eq!(
            terminal.matches("Missing $ inserted").count(),
            1,
            "{mode_name}: {terminal:?}"
        );
        assert_eq!(
            terminal.matches("Extra \\forcedendv").count(),
            1,
            "{mode_name}: {terminal:?}"
        );
        assert_eq!(stores.count(0), 29, "{mode_name}: {terminal:?}");
        assert!(control.fuel_burned() < 256, "{mode_name}");
    }
}

#[test]
fn etex_lastnodetype_reads_each_live_mode_tail_without_mutation() {
    // e-TeX 2.6 `etex.ch` [26.424]: `find_effective_tail` returns -1 for an
    // empty list, otherwise the e-TRIP node code of the real current tail.
    let mut stores = Universe::new_with_plain_catcodes();
    tex_expand::install_expandable_primitives(&mut stores);
    tex_expand::install_etex_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    crate::install_etex_unexpandable_primitives(&mut stores);
    let mut control = CanonicalMainControl::prepared_initex(CommandProfile::ETEX26);
    register_source(
        &mut control,
        br"\xdef\outerempty{\the\lastnodetype}
          \hbox{\xdef\hempty{\the\lastnodetype}}
          \hbox{\vrule\xdef\hrule{\the\lastnodetype}}
          \hbox{\kern1pt\xdef\hkern{\the\lastnodetype}}
          \vbox{\hbox{}\xdef\vboxnode{\the\lastnodetype}}
          $\mathord{1}\xdef\mathnode{\the\lastnodetype}$
          \end",
    );

    run_to_end(&mut control, &mut stores);

    for (name, expected) in [
        ("outerempty", "-1"),
        ("hempty", "-1"),
        ("hrule", "3"),
        ("hkern", "12"),
        ("vboxnode", "1"),
        ("mathnode", "15"),
    ] {
        assert!(stores.symbol(name).is_some(), "missing probe macro {name}");
        assert_eq!(macro_character_text(&stores, name), expected, "{name}");
    }
}

#[test]
fn etex_lastnodetype_covers_every_node_code() {
    // e-TeX 2.6 `etex.ch` block 99 maps the complete 0..=15 node-type
    // interval.  Each enquiry is made while its node is still the live tail;
    // the alignment row is observed from `\noalign`, where it is an unset
    // node until `fin_align` resolves it.
    let mut stores = Universe::new_with_plain_catcodes();
    tex_expand::install_expandable_primitives(&mut stores);
    tex_expand::install_etex_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    crate::install_etex_unexpandable_primitives(&mut stores);
    let mut control = CanonicalMainControl::prepared_initex(CommandProfile::ETEX26);
    register_cmr10_as(&mut control, &mut stores, "cmr10.tfm");
    register_source(
        &mut control,
        br"\font\f=cmr10 \f
          \hbox{x\xdef\nzero{\the\lastnodetype}}
          \hbox{\hbox{}\xdef\none{\the\lastnodetype}}
          \hbox{\vbox{}\xdef\ntwo{\the\lastnodetype}}
          \hbox{\vrule\xdef\nthree{\the\lastnodetype}}
          \vbox{\insert0{}\xdef\nfour{\the\lastnodetype}}
          \vbox{\mark{}\xdef\nfive{\the\lastnodetype}}
          \hbox{\vadjust{}\xdef\nsix{\the\lastnodetype}}
          \hbox{\discretionary{}{}{}\xdef\neight{\the\lastnodetype}}
          \hbox{\special{}\xdef\nnine{\the\lastnodetype}}
          \hbox{\hskip1pt\xdef\neleven{\the\lastnodetype}}
          \hbox{\kern1pt\xdef\ntwelve{\the\lastnodetype}}
          \hbox{\penalty1\xdef\nthirteen{\the\lastnodetype}}
          \vbox{\halign{#\cr x\cr\noalign{\xdef\nfourteen{\the\lastnodetype}}}}
          \end",
    );

    run_to_end(&mut control, &mut stores);

    for (name, expected) in [
        ("nzero", "0"),
        ("none", "1"),
        ("ntwo", "2"),
        ("nthree", "3"),
        ("nfour", "4"),
        ("nfive", "5"),
        ("nsix", "6"),
        ("neight", "8"),
        ("nnine", "9"),
        ("neleven", "11"),
        ("ntwelve", "12"),
        ("nthirteen", "13"),
        ("nfourteen", "14"),
    ] {
        assert!(stores.symbol(name).is_some(), "missing probe macro {name}");
        assert_eq!(macro_character_text(&stores, name), expected, "{name}");
    }
}

#[test]
fn etex_lastnodetype_code_seven_after_unboxing_ligature() {
    let mut stores = Universe::new_with_plain_catcodes();
    tex_expand::install_expandable_primitives(&mut stores);
    tex_expand::install_etex_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    crate::install_etex_unexpandable_primitives(&mut stores);
    let mut control = CanonicalMainControl::prepared_initex(CommandProfile::ETEX26);
    register_cmr10_as(&mut control, &mut stores, "cmr10.tfm");
    register_source(
        &mut control,
        br"\font\f=cmr10 \f\hbox{\setbox0=\hbox{ff}\unhbox0\xdef\result{\the\lastnodetype}}\end",
    );
    run_to_end(&mut control, &mut stores);
    assert_eq!(macro_character_text(&stores, "result"), "7");
}

#[test]
fn outer_vertical_kern_joins_contributions_without_running_page_builder() {
    // TeX82 §§1057 and 1061: `append_kern` tail-appends in every mode but,
    // unlike `append_penalty`, does not call `build_page`. Canonical outer
    // vertical material lives in the page contribution queue rather than the
    // otherwise-empty root mode list.
    let mut stores = Universe::new_with_plain_catcodes();
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    let mut control = CanonicalMainControl::prepared_initex(CommandProfile::TEX82);
    register_source(&mut control, br"\kern-50pt");

    run_to_end(&mut control, &mut stores);

    assert!(control.modes.current_list().nodes().is_empty());
    assert!(matches!(
        stores.page_contributions().as_slices(),
        ([Node::Kern { amount, kind: KernKind::Explicit }], [])
            if amount.raw() == -3_276_800
    ));
    assert_eq!(
        stores.page_dimension(PageDimension::Total),
        Scaled::from_raw(0)
    );
}

#[test]
fn etex_marks_scans_extended_classes_and_expanded_text_in_every_mode() {
    // e-TeX 2.6 `etex.ch` [26.424]: `make_mark` scans an extended register
    // number before TeX82 §1101's expanded mark text and appends the node in
    // every mode. Invalid selectors recover to class zero before the text.
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    tex_expand::install_expandable_primitives(&mut stores);
    tex_expand::install_etex_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    crate::install_etex_unexpandable_primitives(&mut stores);
    let mut control = CanonicalMainControl::prepared_initex(CommandProfile::ETEX26);
    register_source(
        &mut control,
        br"\def\payload{expanded}
          \marks32767{\payload}
          {\global\marks-1{recovered}}
          \hbox{\marks7{horizontal}}
          \vbox{\marks8{vertical}}
          $\marks9{math}1$",
    );

    run_to_end(&mut control, &mut stores);

    let nodes = stores
        .current_page_nodes()
        .into_iter()
        .chain(stores.page_contributions().iter().cloned())
        .collect::<Vec<_>>();
    assert!(
        nodes
            .iter()
            .any(|node| matches!(node, Node::Mark { class: 32_767, .. }))
    );
    assert!(
        nodes
            .iter()
            .any(|node| matches!(node, Node::Mark { class: 0, .. }))
    );
    let expanded = nodes
        .iter()
        .find_map(|node| match node {
            Node::Mark {
                class: 32_767,
                tokens,
            } => Some(
                stores
                    .tokens(*tokens)
                    .iter()
                    .filter_map(|token| match token {
                        Token::Char { ch, .. } => Some(*ch),
                        Token::Cs(_) | Token::Param(_) | Token::Frozen(_) => None,
                    })
                    .collect::<String>(),
            ),
            _ => None,
        })
        .expect("class 32767 mark");
    assert_eq!(expanded, "expanded");
    assert!(terminal_text(&stores).contains("Bad register code"));
    assert!(terminal_text(&stores).contains("You can't use a prefix with"));
    assert!(!terminal_text(&stores).contains("Unimplemented primitive"));
}

#[test]
fn tex82_profile_leaves_numbered_marks_undefined() {
    let mut stores = Universe::new_with_plain_catcodes();
    let _control = CanonicalMainControl::tex82_initex(&mut stores);
    let marks = stores.intern("marks");
    assert_eq!(stores.meaning(marks), Meaning::Undefined);
    assert_eq!(stores.primitive_meaning("marks"), None);
}

#[test]
fn etex_showgroups_detaches_nested_save_and_mode_diagnostics() {
    let mut stores = Universe::new();
    let _initialized = CanonicalMainControl::tex82_initex(&mut stores);
    crate::install_etex_unexpandable_primitives(&mut stores);
    let mut control = CanonicalMainControl::with_profile(tex_command::CommandProfile::ETEX26);
    register_source(
        &mut control,
        b"\\nonstopmode\n\\tracingonline=1\n\\showgroups\n\\begingroup\\showgroups\\endgroup\n\\global\\showgroups\\count0=7\n\\end",
    );

    run_to_end(&mut control, &mut stores);

    let mut modes = ModeNest::new();
    let mut boxes = ReplayBoxes::default();
    stores.enter_group_with_kind_at_line(GroupKind::AdjustedHBox, 6);
    modes
        .push(Mode::RestrictedHorizontal)
        .expect("test mode push");
    boxes.active_boxes.push(ActiveReplayBox {
        target: None,
        ships_out: false,
        kind: ReplayBoxKind::HBox,
        group_kind: GroupKind::AdjustedHBox,
        packing: PackSpec::Exactly(Scaled::from_raw(20 * 65_536)),
        leader_kind: None,
        shift: None,
    });
    let diagnostic = detached_showgroups(&stores, &modes, &None, &boxes);
    crate::diagnostics::execute_canonical_showgroups(&mut stores, &diagnostic, String::new())
        .expect("\\showgroups reports no fatal error");

    stores.enter_group_with_kind_at_line(GroupKind::MathShift, 7);
    modes.push(Mode::Math).expect("test mode push");
    stores.enter_group_with_kind_at_line(GroupKind::Math, 7);
    modes.push(Mode::Math).expect("test mode push");
    let diagnostic = detached_showgroups(&stores, &modes, &None, &boxes);
    crate::diagnostics::execute_canonical_showgroups(&mut stores, &diagnostic, String::new())
        .expect("\\showgroups reports no fatal error");

    stores.enter_group_with_kind_at_line(GroupKind::Align, 8);
    stores.enter_group_with_kind_at_line(GroupKind::Align, 8);
    stores.enter_group_with_kind_at_line(GroupKind::NoAlign, 8);
    let diagnostic = detached_showgroups(&stores, &modes, &None, &boxes);
    crate::diagnostics::execute_canonical_showgroups(&mut stores, &diagnostic, String::new())
        .expect("\\showgroups reports no fatal error");

    let output = terminal_text(&stores);
    for expected in [
        "### bottom level",
        "### semi simple group (level 1) entered at line 4 (\\begingroup)",
        "### adjusted hbox group (level 1) entered at line 6 (\\hbox to20.0pt{)",
        "### math group (level 3) entered at line 7 ({)",
        "### math shift group (level 2) entered at line 7 ($)",
        "### no align group (level 6) entered at line 8 (\\noalign{)",
        "### align group (level 5) entered at line 8 (align entry)",
        "### align group (level 4) entered at line 8 (\\halign{)",
    ] {
        assert!(
            output.contains(expected),
            "missing {expected:?} in {output:?}"
        );
    }
    assert_eq!(
        stores.count(0),
        7,
        "prefix recovery consumed following input"
    );
    assert_eq!(stores.group_depth(), 6, "diagnostic mutated the save stack");
}

fn macro_tokens<'a>(stores: &'a Universe, name: &str) -> &'a [Token] {
    let meaning = stores
        .macro_meaning(stores.symbol(name).expect("macro target"))
        .expect("macro is defined");
    stores.tokens(meaning.replacement_text())
}

fn pdftex_random_control(stores: &mut Universe) -> CanonicalMainControl {
    let set_seed = stores.intern("pdfsetrandomseed");
    stores.set_meaning(
        set_seed,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfSetRandomSeed),
    );
    CanonicalMainControl::with_profile(tex_command::CommandProfile::PDFTEX14027)
}

fn pdftex_timer_control(stores: &mut Universe) -> CanonicalMainControl {
    let reset_timer = stores.intern("pdfresettimer");
    stores.set_meaning(
        reset_timer,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfResetTimer),
    );
    CanonicalMainControl::with_profile(tex_command::CommandProfile::PDFTEX14027)
}

fn pdftex_interword_control(stores: &mut Universe) -> CanonicalMainControl {
    for (name, primitive) in [
        (
            "pdfinterwordspaceon",
            UnexpandablePrimitive::PdfInterwordSpaceOn,
        ),
        (
            "pdfinterwordspaceoff",
            UnexpandablePrimitive::PdfInterwordSpaceOff,
        ),
        ("pdffakespace", UnexpandablePrimitive::PdfFakeSpace),
        ("pdfrunninglinkon", UnexpandablePrimitive::PdfRunningLinkOn),
        (
            "pdfrunninglinkoff",
            UnexpandablePrimitive::PdfRunningLinkOff,
        ),
        ("pdfspacefont", UnexpandablePrimitive::PdfSpaceFont),
    ] {
        let symbol = stores.intern(name);
        stores.set_meaning(symbol, Meaning::UnexpandablePrimitive(primitive));
    }
    CanonicalMainControl::with_profile(tex_command::CommandProfile::PDFTEX14027)
}

fn pdftex_font_action_control(stores: &mut Universe) -> CanonicalMainControl {
    let nullfont = stores.intern("nullfont");
    stores.set_meaning(nullfont, Meaning::Font(tex_state::font::NULL_FONT));
    for (name, primitive) in [
        ("pdffontexpand", UnexpandablePrimitive::PdfFontExpand),
        ("pdffontattr", UnexpandablePrimitive::PdfFontAttr),
        ("pdfincludechars", UnexpandablePrimitive::PdfIncludeChars),
        ("pdfmapfile", UnexpandablePrimitive::PdfMapFile),
        ("pdfmapline", UnexpandablePrimitive::PdfMapLine),
        (
            "pdfglyphtounicode",
            UnexpandablePrimitive::PdfGlyphToUnicode,
        ),
        (
            "pdfnobuiltintounicode",
            UnexpandablePrimitive::PdfNoBuiltinToUnicode,
        ),
    ] {
        let symbol = stores.intern(name);
        stores.set_meaning(symbol, Meaning::UnexpandablePrimitive(primitive));
    }
    CanonicalMainControl::with_profile(tex_command::CommandProfile::PDFTEX14027)
}

#[test]
fn pdftex_font_actions_route_through_canonical_expansion_and_font_state() {
    // pdftex.web §§1601--1607, 1680--1682: general text is expanded before
    // the action mutates the selected font or the global map/ToUnicode state.
    let mut stores = Universe::new_with_plain_catcodes();
    crate::install_unexpandable_primitives(&mut stores);
    tex_expand::install_expandable_primitives(&mut stores);
    stores
        .world_mut()
        .set_memory_file(
            "cmr10.tfm",
            include_bytes!("../../../tex-fonts/tests/fixtures/cm/cmr10.tfm").to_vec(),
        )
        .expect("seed cmr10");
    let mut setup = tex_lex::InputStack::new(tex_lex::MemoryInput::new("\\font\\base=cmr10 \\end"));
    crate::Executor::new()
        .run(&mut setup, &mut stores)
        .expect("seed base font through the ordinary loader");
    let base = match stores.meaning(stores.symbol("base").expect("base selector")) {
        Meaning::Font(font) => font,
        meaning => panic!("base is a font, got {meaning:?}"),
    };
    stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
    let mut control = pdftex_font_action_control(&mut stores);
    register_source(
        &mut control,
        concat!(
            "\\def\\attr{/StemV 70}\\def\\chars{CABA}\\def\\uni{0041}",
            "\\pdffontexpand\\base 100 50 10 autoexpand ",
            "\\pdffontattr\\base{\\attr}\\pdfincludechars\\base{\\chars}",
            "\\pdfmapline{+cmr10 CMR10 <cmr10.pfb}",
            "\\pdfglyphtounicode{A}{\\uni}\\pdfnobuiltintounicode\\base\\end",
        )
        .as_bytes(),
    );

    run_to_end(&mut control, &mut stores);

    assert_eq!(
        stores.font_expansion(base),
        Some(tex_state::font::FontExpansion {
            stretch: 100,
            shrink: 50,
            step: 10,
            auto_expand: true,
        })
    );
    assert_eq!(stores.pdf_font_attribute(base), b"/StemV 70");
    assert_eq!(stores.included_pdf_font_chars(base), b"ABC");
    assert_eq!(
        stores.pdf_glyph_to_unicode(b"cmr10", b"A"),
        Some([0x41].as_slice())
    );
    assert!(stores.pdf_builtin_to_unicode_disabled(base));
    assert!(matches!(
        stores.pdf_font_maps().next(),
        Some(tex_state::PdfFontMapOperation::Line(line)) if line.tex_name == b"cmr10"
    ));
}

#[test]
fn pdftex_font_actions_preserve_exact_dvi_mode_gate_and_tounicode_exceptions() {
    // pdftex.web §§1601--1607: these five extension codes require PDF mode;
    // glyph and built-in ToUnicode definitions are deliberately exempt.
    for (name, source) in [
        (
            "pdffontexpand",
            b"\\pdffontexpand\\nullfont 1 1 1".as_slice(),
        ),
        ("pdffontattr", b"\\pdffontattr\\nullfont{}".as_slice()),
        (
            "pdfincludechars",
            b"\\pdfincludechars\\nullfont{}".as_slice(),
        ),
        ("pdfmapfile", b"\\pdfmapfile{}".as_slice()),
        ("pdfmapline", b"\\pdfmapline{}".as_slice()),
    ] {
        let mut stores = Universe::new_with_plain_catcodes();
        let mut control = pdftex_font_action_control(&mut stores);
        register_source(&mut control, source);
        assert!(matches!(
            control.step(&mut stores),
            Err(ExecError::PdfExtensionInDviMode(actual)) if actual == name
        ));
    }

    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = pdftex_font_action_control(&mut stores);
    register_source(
        &mut control,
        b"\\pdfglyphtounicode{A}{0041}\\pdfnobuiltintounicode\\nullfont\\end",
    );
    run_to_end(&mut control, &mut stores);
    assert_eq!(
        stores.pdf_glyph_to_unicode(b"cmr10", b"A"),
        Some([0x41].as_slice())
    );
    assert!(stores.pdf_builtin_to_unicode_disabled(tex_state::font::NULL_FONT));
}

fn pdftex_snapping_control(stores: &mut Universe) -> CanonicalMainControl {
    for (name, primitive) in [
        ("pdfsnaprefpoint", UnexpandablePrimitive::PdfSnapRefPoint),
        ("pdfsnapy", UnexpandablePrimitive::PdfSnapY),
        ("pdfsnapycomp", UnexpandablePrimitive::PdfSnapYComp),
    ] {
        let symbol = stores.intern(name);
        stores.set_meaning(symbol, Meaning::UnexpandablePrimitive(primitive));
    }
    CanonicalMainControl::with_profile(tex_command::CommandProfile::PDFTEX14027)
}

fn pdftex_graphics_control(stores: &mut Universe) -> CanonicalMainControl {
    for (name, primitive) in [
        ("pdfliteral", UnexpandablePrimitive::PdfLiteral),
        ("pdfsetmatrix", UnexpandablePrimitive::PdfSetMatrix),
        ("pdfsave", UnexpandablePrimitive::PdfSave),
        ("pdfrestore", UnexpandablePrimitive::PdfRestore),
        ("pdfcolorstack", UnexpandablePrimitive::PdfColorStack),
        ("pdfsavepos", UnexpandablePrimitive::PdfSavePos),
    ] {
        let symbol = stores.intern(name);
        stores.set_meaning(symbol, Meaning::UnexpandablePrimitive(primitive));
    }
    CanonicalMainControl::with_profile(tex_command::CommandProfile::PDFTEX14027)
}

#[test]
fn pdf_graphics_reject_dvi_before_operands_and_retry_in_source_order() {
    // pdftex.web §§1524 and 1563: `check_pdfoutput` precedes operand scanning
    // for every graphics extension except `\pdfsavepos`. Aggregate rollback
    // therefore preserves each complete command for an exact PDF-mode retry.
    for (source, primitive, expected) in [
        (
            br"\pdfliteral direct{first}\pdfsave".as_slice(),
            "pdfliteral",
            "literal",
        ),
        (
            br"\pdfsetmatrix{1 0 0 1}\pdfsave".as_slice(),
            "pdfsetmatrix",
            "matrix",
        ),
        (
            br"\pdfcolorstack0 push{0 g}\pdfsave".as_slice(),
            "pdfcolorstack",
            "color",
        ),
    ] {
        let mut stores = Universe::new_with_plain_catcodes();
        let mut control = pdftex_graphics_control(&mut stores);
        register_source(&mut control, source);
        let state_before = stores.testing_state_hash();

        assert!(
            matches!(control.step(&mut stores), Err(ExecError::PdfExtensionInDviMode(name)) if name == primitive)
        );
        assert_eq!(stores.testing_state_hash(), state_before);
        assert!(control.modes.current_list().nodes().is_empty());

        stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
        assert_eq!(
            control.step(&mut stores).expect("graphics command retries"),
            MainControlStep::Continue
        );
        let [node] = control.modes.current_list().nodes() else {
            panic!("{expected}: retry must append exactly one node");
        };
        assert!(
            matches!(
                (expected, node),
                ("literal", Node::Whatsit(Whatsit::PdfLiteral { payload, .. })) if payload == b"first"
            ) || matches!((expected, node), ("matrix", Node::Whatsit(Whatsit::PdfSetMatrix { payload })) if payload == b"1 0 0 1")
                || matches!((expected, node), ("color", Node::Whatsit(Whatsit::PdfColorStack { id: 0, action: tex_state::PdfColorStackAction::Push(payload) })) if payload == b"0 g")
        );
        assert_eq!(
            control
                .step(&mut stores)
                .expect("following command remains"),
            MainControlStep::Continue
        );
        assert!(matches!(
            control.modes.current_list().nodes().last(),
            Some(Node::Whatsit(Whatsit::PdfSave))
        ));
    }
}

#[test]
fn pdfsavepos_remains_available_in_dvi_mode() {
    // pdftex.web §1563 deliberately excludes `\pdfsavepos` from the PDF
    // output preflight used by the neighboring graphics extensions.
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = pdftex_graphics_control(&mut stores);
    register_source(&mut control, br"\pdfsavepos");
    assert_eq!(
        control.step(&mut stores).expect("DVI save position"),
        MainControlStep::Continue
    );
    assert!(matches!(
        control.modes.current_list().nodes(),
        [Node::Whatsit(Whatsit::PdfSavePos)]
    ));
}

#[test]
fn pdf_color_stack_recovery_reports_help_and_preserves_action_order() {
    // pdftex.web §1563: invalid stack numbers fall back to stack zero, a
    // missing action is ignored after the four-action help, and subsequent
    // commands retain their order.
    for (source, diagnostic, help) in [
        (
            br"\pdfcolorstack-1 push{a}".as_slice(),
            "Invalid negative color stack number",
            "I'll use default color stack 0 here.",
        ),
        (
            br"\pdfcolorstack99 set{b}".as_slice(),
            "Unknown color stack number 99",
            "Allocate and initialize a color stack with \\pdfcolorstackinit.",
        ),
    ] {
        let mut stores = Universe::new_with_plain_catcodes();
        stores.set_interaction_mode(tex_state::InteractionMode::Scroll);
        stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
        let mut control = pdftex_graphics_control(&mut stores);
        register_source(&mut control, source);
        let _ = control.step(&mut stores).expect("recoverable bad stack id");
        assert!(matches!(
            control.modes.current_list().nodes(),
            [Node::Whatsit(Whatsit::PdfColorStack { id: 0, .. })]
        ));
        let terminal = terminal_text(&stores);
        assert!(terminal.contains(diagnostic));
        assert!(terminal.contains(help));
        assert!(terminal.contains("Proceed, with fingers crossed."));
    }

    let mut stores = Universe::new_with_plain_catcodes();
    stores.set_interaction_mode(tex_state::InteractionMode::Scroll);
    stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
    let mut control = pdftex_graphics_control(&mut stores);
    register_source(&mut control, br"\pdfcolorstack0\pdfsave");
    let _ = control
        .step(&mut stores)
        .expect("missing action is recoverable");
    assert!(control.modes.current_list().nodes().is_empty());
    let _ = control
        .step(&mut stores)
        .expect("following command remains available");
    assert!(matches!(
        control.modes.current_list().nodes(),
        [Node::Whatsit(Whatsit::PdfSave)]
    ));
    let terminal = terminal_text(&stores);
    assert!(terminal.contains("Color stack action is missing"));
    assert!(terminal.contains("set, push, pop, current"));
    assert!(terminal.contains("I'll ignore the color stack command."));
    assert!(terminal.contains("Proceed, with fingers crossed."));
}

fn pdftex_outline_control(stores: &mut Universe) -> CanonicalMainControl {
    let outline = stores.intern("pdfoutline");
    stores.set_meaning(
        outline,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfOutline),
    );
    CanonicalMainControl::with_profile(tex_command::CommandProfile::PDFTEX14027)
}

fn pdftex_thread_control(stores: &mut Universe) -> CanonicalMainControl {
    for (name, primitive) in [
        ("pdfthread", UnexpandablePrimitive::PdfThread),
        ("pdfstartthread", UnexpandablePrimitive::PdfStartThread),
        ("pdfendthread", UnexpandablePrimitive::PdfEndThread),
    ] {
        let symbol = stores.intern(name);
        stores.set_meaning(symbol, Meaning::UnexpandablePrimitive(primitive));
    }
    CanonicalMainControl::with_profile(tex_command::CommandProfile::PDFTEX14027)
}

fn pdftex_object_control(stores: &mut Universe) -> CanonicalMainControl {
    for (name, primitive) in [
        ("pdfobj", UnexpandablePrimitive::PdfObject),
        ("pdfrefobj", UnexpandablePrimitive::PdfReferenceObject),
        ("immediate", UnexpandablePrimitive::Immediate),
    ] {
        let symbol = stores.intern(name);
        stores.set_meaning(symbol, Meaning::UnexpandablePrimitive(primitive));
    }
    CanonicalMainControl::with_profile(tex_command::CommandProfile::PDFTEX14027)
}

fn pdftex_form_control(stores: &mut Universe) -> CanonicalMainControl {
    for (name, primitive) in [
        ("pdfxform", UnexpandablePrimitive::PdfXForm),
        ("pdfrefxform", UnexpandablePrimitive::PdfRefXForm),
        ("immediate", UnexpandablePrimitive::Immediate),
    ] {
        let symbol = stores.intern(name);
        stores.set_meaning(symbol, Meaning::UnexpandablePrimitive(primitive));
    }
    CanonicalMainControl::with_profile(tex_command::CommandProfile::PDFTEX14027)
}

fn pdftex_image_control(stores: &mut Universe) -> CanonicalMainControl {
    for (name, primitive) in [
        ("pdfximage", UnexpandablePrimitive::PdfXImage),
        ("pdfrefximage", UnexpandablePrimitive::PdfRefXImage),
        ("immediate", UnexpandablePrimitive::Immediate),
    ] {
        let symbol = stores.intern(name);
        stores.set_meaning(symbol, Meaning::UnexpandablePrimitive(primitive));
    }
    CanonicalMainControl::with_profile(tex_command::CommandProfile::PDFTEX14027)
}

fn test_pdf_image_source() -> tex_state::PdfExternalImageSource {
    tex_state::PdfExternalImageSource {
        identity: tex_state::ContentHash::from_bytes(b"canonical image preflight"),
        metadata: tex_state::PdfExternalImageMetadata::Raster(tex_state::PdfRasterImageMetadata {
            format: tex_state::PdfRasterFormat::Png,
            width: 1,
            height: 1,
            bits_per_component: 8,
            color_space: tex_state::PdfRasterColorSpace::Gray,
            alpha: false,
            png_color_type: Some(0),
        }),
        natural_width: Scaled::from_raw(Scaled::UNITY),
        natural_height: Scaled::from_raw(Scaled::UNITY),
        bytes: Arc::from(&b"image bytes"[..]),
    }
}

fn install_test_hbox(stores: &mut Universe, register: u16, width: Scaled) {
    let children = stores.freeze_node_list(&[]);
    let list = stores.freeze_node_list(&[Node::HList(tex_state::node::BoxNode::new(
        tex_state::node::BoxNodeFields {
            width,
            height: Scaled::from_raw(2),
            depth: Scaled::from_raw(3),
            shift: Scaled::from_raw(0),
            box_lr: tex_state::node::BoxLr::Normal,
            glue_set: tex_state::scaled::GlueSetRatio::ZERO,
            glue_sign: tex_state::node::Sign::Normal,
            glue_order: Order::Normal,
            children,
        },
    ))]);
    stores.set_box_reg(register, list);
}

fn install_test_form(stores: &mut Universe) {
    install_test_hbox(stores, 0, Scaled::from_raw(11));
    let list = stores.take_box_reg_same_level(0).expect("test form box");
    let identity = stores.reserve_pdf_form().expect("reserve test form");
    stores
        .initialize_pdf_form(
            identity,
            list,
            (
                Scaled::from_raw(11),
                Scaled::from_raw(2),
                Scaled::from_raw(3),
            ),
            None,
            None,
            false,
        )
        .expect("initialize test form");
}

fn token_character_text(stores: &Universe, tokens: tex_state::ids::TokenListId) -> String {
    stores
        .tokens(tokens)
        .iter()
        .filter_map(|token| match token {
            Token::Char { ch, .. } => Some(*ch),
            Token::Cs(_) | Token::Param(_) | Token::Frozen(_) => None,
        })
        .collect()
}

#[test]
fn pdf_object_rejects_dvi_before_every_option_operand_and_allocation() {
    // pdftex.web §§1535 and 1542 call `check_pdfoutput` before the complete
    // `reserveobjnum`/`useobjnum`, integer, stream/attr/file, body, and
    // allocation paths. Aggregate retry must therefore see the whole command.
    let mut reserve_stores = Universe::new_with_plain_catcodes();
    let mut reserve_control = pdftex_object_control(&mut reserve_stores);
    register_source(&mut reserve_control, br"\pdfobj reserveobjnum");
    assert!(matches!(
        reserve_control.step(&mut reserve_stores),
        Err(ExecError::PdfExtensionInDviMode("pdfobj"))
    ));
    assert!(reserve_stores.pdf_raw_objects().is_empty());
    assert_eq!(reserve_stores.pdf_last_object(), 0);

    reserve_stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
    assert_eq!(
        reserve_control
            .step(&mut reserve_stores)
            .expect("reserveobjnum retry preserves the complete command"),
        MainControlStep::Continue
    );
    assert_eq!(reserve_stores.pdf_raw_objects().len(), 1);
    assert!(reserve_stores.pdf_raw_objects()[0].data().is_none());

    let mut ordinary_stores = Universe::new_with_plain_catcodes();
    let mut ordinary_control = pdftex_object_control(&mut ordinary_stores);
    register_source(&mut ordinary_control, br"\pdfobj{ordinary}");
    assert!(matches!(
        ordinary_control.step(&mut ordinary_stores),
        Err(ExecError::PdfExtensionInDviMode("pdfobj"))
    ));
    assert!(ordinary_stores.pdf_raw_objects().is_empty());

    ordinary_stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
    assert_eq!(
        ordinary_control
            .step(&mut ordinary_stores)
            .expect("ordinary-object retry preserves its body"),
        MainControlStep::Continue
    );
    let ordinary = ordinary_stores.pdf_raw_objects()[0]
        .data()
        .expect("ordinary object is initialized");
    assert!(!ordinary.is_stream());
    assert!(!ordinary.is_file());
    assert_eq!(
        token_character_text(&ordinary_stores, ordinary.data()),
        "ordinary"
    );

    let mut define_stores = Universe::new_with_plain_catcodes();
    let mut define_control = pdftex_object_control(&mut define_stores);
    register_source(
        &mut define_control,
        br"\pdfobj useobjnum 37 stream attr{/Subtype /XML} file{payload}",
    );
    assert!(matches!(
        define_control.step(&mut define_stores),
        Err(ExecError::PdfExtensionInDviMode("pdfobj"))
    ));
    assert!(define_stores.pdf_raw_objects().is_empty());
    assert_eq!(define_stores.pdf_return_value(), 0);
    assert!(terminal_text(&define_stores).is_empty());

    define_stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
    assert_eq!(
        define_control
            .step(&mut define_stores)
            .expect("definition retry preserves every option and operand"),
        MainControlStep::Continue
    );
    assert_eq!(define_stores.pdf_return_value(), -1);
    assert!(terminal_text(&define_stores).contains("invalid object number being ignored"));
    let record = define_stores.pdf_raw_objects()[0];
    let data = record.data().expect("retried object is initialized");
    assert!(data.is_stream());
    assert!(data.is_file());
    assert_eq!(
        token_character_text(
            &define_stores,
            data.stream_attr().expect("stream attribute survives retry")
        ),
        "/Subtype /XML"
    );
    assert_eq!(token_character_text(&define_stores, data.data()), "payload");
}

#[test]
fn immediate_pdf_object_rejects_dvi_after_lookahead_before_operand_scan() {
    // pdftex.web §1621 expands the command after `\immediate`, then invokes
    // §1542's complete `\pdfobj` case. Its DVI check therefore wins over the
    // immediate-reserved-object error and every operand remains retryable.
    let mut reserve_stores = Universe::new_with_plain_catcodes();
    let mut reserve_control = pdftex_object_control(&mut reserve_stores);
    register_source(&mut reserve_control, br"\immediate\pdfobj reserveobjnum");
    assert!(matches!(
        reserve_control.step(&mut reserve_stores),
        Err(ExecError::PdfExtensionInDviMode("pdfobj"))
    ));
    assert!(reserve_stores.pdf_raw_objects().is_empty());

    reserve_stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
    assert!(matches!(
        reserve_control.step(&mut reserve_stores),
        Err(ExecError::PdfImmediateReservedObject)
    ));
    assert!(reserve_stores.pdf_raw_objects().is_empty());

    let mut define_stores = Universe::new_with_plain_catcodes();
    let mut define_control = pdftex_object_control(&mut define_stores);
    register_source(
        &mut define_control,
        br"\immediate\pdfobj useobjnum 41 stream attr{/Type /Metadata} file{retry.dat}",
    );
    assert!(matches!(
        define_control.step(&mut define_stores),
        Err(ExecError::PdfExtensionInDviMode("pdfobj"))
    ));
    assert!(define_stores.pdf_raw_objects().is_empty());
    assert_eq!(define_stores.pdf_return_value(), 0);

    define_stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
    assert_eq!(
        define_control
            .step(&mut define_stores)
            .expect("immediate retry preserves every option and operand"),
        MainControlStep::Continue
    );
    assert_eq!(define_stores.pdf_return_value(), -1);
    let record = define_stores.pdf_raw_objects()[0];
    assert!(record.is_immediate());
    let data = record.data().expect("immediate object is initialized");
    assert!(data.is_stream());
    assert!(data.is_file());
    assert_eq!(
        token_character_text(
            &define_stores,
            data.stream_attr().expect("stream attribute survives retry")
        ),
        "/Type /Metadata"
    );
    assert_eq!(
        token_character_text(&define_stores, data.data()),
        "retry.dat"
    );
}

#[test]
fn pdf_reference_object_rejects_dvi_before_scan_validation_or_list_mutation() {
    // pdftex.web §1544 orders `check_pdfoutput`, `scan_int`,
    // `pdf_check_obj`, `new_whatsit`, and object-number assignment. A DVI
    // failure must therefore preserve the integer and every aggregate owner
    // for transactional retry under the pdfTeX profile.
    let mut stores = Universe::new_with_plain_catcodes();
    let object = stores
        .reserve_pdf_raw_object()
        .expect("reserve reference target");
    assert_eq!(object.raw(), 1);
    let mut control = pdftex_object_control(&mut stores);
    register_source(&mut control, br"\pdfrefobj 1");
    let state_before = stores.testing_state_hash();

    assert!(matches!(
        control.step(&mut stores),
        Err(ExecError::PdfExtensionInDviMode("pdfrefobj"))
    ));
    assert_eq!(stores.testing_state_hash(), state_before);
    assert_eq!(stores.pdf_raw_objects().len(), 1);
    assert!(control.modes.current_list().nodes().is_empty());

    stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
    assert_eq!(
        control
            .step(&mut stores)
            .expect("PDF retry preserves the integer operand"),
        MainControlStep::Continue
    );
    assert!(matches!(
        control.modes.current_list().nodes(),
        [Node::Whatsit(Whatsit::PdfReferenceObject { object: 1 })]
    ));
}

#[test]
fn pdf_reference_object_dvi_error_precedes_invalid_object_validation() {
    // pdftex.web §1544 checks DVI mode before scanning or calling
    // `pdf_check_obj`; the missing-object error is reached only on a PDF-mode
    // retry of the same intact operand.
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = pdftex_object_control(&mut stores);
    register_source(&mut control, br"\pdfrefobj 99");
    let state_before = stores.testing_state_hash();

    assert!(matches!(
        control.step(&mut stores),
        Err(ExecError::PdfExtensionInDviMode("pdfrefobj"))
    ));
    assert_eq!(stores.testing_state_hash(), state_before);
    assert!(stores.pdf_raw_objects().is_empty());
    assert!(control.modes.current_list().nodes().is_empty());

    stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
    assert!(matches!(
        control.step(&mut stores),
        Err(ExecError::PdfReferencedObjectNotFound)
    ));
    assert!(stores.pdf_raw_objects().is_empty());
    assert!(control.modes.current_list().nodes().is_empty());
}

#[test]
fn pdf_form_family_rejects_dvi_before_operands_allocation_and_list_mutation() {
    // pdftex.web §§1548–1549 begin both cases with `check_pdfoutput`.
    // `\pdfxform` therefore preserves attr/resources/the register and its box,
    // while `\pdfrefxform` preserves its integer before lookup and whatsit
    // insertion. The two commands are one PDF-output-preflight family.
    let mut create_stores = Universe::new_with_plain_catcodes();
    install_test_hbox(&mut create_stores, 7, Scaled::from_raw(17));
    let mut create = pdftex_form_control(&mut create_stores);
    register_source(
        &mut create,
        br"\pdfxform attr{/Subtype /Form} resources{/ProcSet [/PDF]} 7",
    );
    let state_before = create_stores.testing_state_hash();

    assert!(matches!(
        create.step(&mut create_stores),
        Err(ExecError::PdfExtensionInDviMode("pdfxform"))
    ));
    assert_eq!(create_stores.testing_state_hash(), state_before);
    assert!(create_stores.box_reg(7).is_some());
    assert!(create_stores.pdf_forms().next().is_none());
    assert_eq!(create_stores.pdf_last_form(), 0);
    assert!(create.modes.current_list().nodes().is_empty());

    create_stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
    assert_eq!(
        create
            .step(&mut create_stores)
            .expect("PDF retry preserves all form options and the register"),
        MainControlStep::Continue
    );
    assert!(create_stores.box_reg(7).is_none());
    let form = create_stores
        .pdf_form(1)
        .expect("retried form is allocated");
    assert_eq!(form.width(), Scaled::from_raw(17));
    assert_eq!(
        token_character_text(
            &create_stores,
            form.attr().expect("form attribute survives retry")
        ),
        "/Subtype /Form"
    );
    assert_eq!(
        token_character_text(
            &create_stores,
            form.resources().expect("form resources survive retry")
        ),
        "/ProcSet [/PDF]"
    );

    let mut reference_stores = Universe::new_with_plain_catcodes();
    install_test_form(&mut reference_stores);
    let mut reference = pdftex_form_control(&mut reference_stores);
    reference.modes.push(Mode::Math).expect("test mode push");
    register_source(&mut reference, br"\pdfrefxform 1");
    let state_before = reference_stores.testing_state_hash();

    assert!(matches!(
        reference.step(&mut reference_stores),
        Err(ExecError::PdfExtensionInDviMode("pdfrefxform"))
    ));
    assert_eq!(reference_stores.testing_state_hash(), state_before);
    assert!(reference.modes.current_list().nodes().is_empty());

    reference_stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
    assert_eq!(
        reference
            .step(&mut reference_stores)
            .expect("PDF retry preserves the reference operand in math mode"),
        MainControlStep::Continue
    );
    assert!(matches!(
        reference.modes.current_list().nodes(),
        [Node::Whatsit(Whatsit::PdfRefXForm { object: 1, .. })]
    ));
}

#[test]
fn immediate_pdf_form_rejects_dvi_before_options_or_allocation() {
    // pdftex.web §§1548 and 1623 perform `\immediate` lookahead, then enter
    // the same `\pdfxform` case whose first operation is `check_pdfoutput`.
    let mut stores = Universe::new_with_plain_catcodes();
    install_test_hbox(&mut stores, 9, Scaled::from_raw(19));
    let mut control = pdftex_form_control(&mut stores);
    register_source(
        &mut control,
        br"\immediate\pdfxform attr{/A 1} resources{/R 2} 9",
    );
    let state_before = stores.testing_state_hash();

    assert!(matches!(
        control.step(&mut stores),
        Err(ExecError::PdfExtensionInDviMode("pdfxform"))
    ));
    assert_eq!(stores.testing_state_hash(), state_before);
    assert!(stores.box_reg(9).is_some());
    assert!(stores.pdf_forms().next().is_none());

    stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
    assert_eq!(
        control
            .step(&mut stores)
            .expect("immediate PDF retry preserves every form operand"),
        MainControlStep::Continue
    );
    assert!(stores.box_reg(9).is_none());
    let form = stores.pdf_form(1).expect("immediate form is allocated");
    assert!(form.immediate());
    assert_eq!(form.width(), Scaled::from_raw(19));
}

#[test]
fn pdf_form_dvi_error_precedes_invalid_register_void_box_and_missing_object() {
    // §§1548–1549 put DVI rejection before even the scans. On PDF retry,
    // e-TeX's `scan_register_num` recovers an invalid selector to zero before
    // §1548 allocates the form and diagnoses the resulting void box; §1549
    // scans an integer and then diagnoses a missing form object.
    let mut invalid_register_stores = crate::test_harness::universe_with_plain_catcodes();
    let mut invalid_register = pdftex_form_control(&mut invalid_register_stores);
    register_source(&mut invalid_register, br"\pdfxform 40000");
    let state_before = invalid_register_stores.testing_state_hash();

    assert!(matches!(
        invalid_register.step(&mut invalid_register_stores),
        Err(ExecError::PdfExtensionInDviMode("pdfxform"))
    ));
    assert_eq!(invalid_register_stores.testing_state_hash(), state_before);
    assert!(terminal_text(&invalid_register_stores).is_empty());
    assert!(invalid_register_stores.pdf_forms().next().is_none());

    invalid_register_stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
    assert!(matches!(
        invalid_register.step(&mut invalid_register_stores),
        Err(ExecError::PdfXFormVoidBox)
    ));
    assert!(invalid_register_stores.pdf_forms().next().is_none());

    let mut void_stores = crate::test_harness::universe_with_plain_catcodes();
    let mut void = pdftex_form_control(&mut void_stores);
    register_source(&mut void, br"\pdfxform 12");
    assert!(matches!(
        void.step(&mut void_stores),
        Err(ExecError::PdfExtensionInDviMode("pdfxform"))
    ));
    void_stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
    assert!(matches!(
        void.step(&mut void_stores),
        Err(ExecError::PdfXFormVoidBox)
    ));

    let mut missing_stores = crate::test_harness::universe_with_plain_catcodes();
    let mut missing = pdftex_form_control(&mut missing_stores);
    missing
        .modes
        .push(Mode::RestrictedHorizontal)
        .expect("test mode push");
    register_source(&mut missing, br"\pdfrefxform 99");
    assert!(matches!(
        missing.step(&mut missing_stores),
        Err(ExecError::PdfExtensionInDviMode("pdfrefxform"))
    ));
    assert!(missing.modes.current_list().nodes().is_empty());
    missing_stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
    assert!(matches!(
        missing.step(&mut missing_stores),
        Err(ExecError::PdfReferencedObjectNotFound)
    ));
    assert!(missing.modes.current_list().nodes().is_empty());
}

#[test]
fn pdf_image_create_rejects_dvi_before_operands_allocation_or_resource_lookup() {
    // pdftex.web §1551 orders `check_pdfoutput` before `check_pdfversion`,
    // image-object allocation, `scan_image`, and `read_image`. A failed
    // aggregate operation therefore preserves every supported rule, attr,
    // page, page-box, and filename operand for exact resource retry.
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = pdftex_image_control(&mut stores);
    register_source(
        &mut control,
        br"\pdfximage width 10pt height 20pt depth 3pt attr{/Interpolate true} page 2 mediabox {image.pdf}",
    );
    let state_before = stores.testing_state_hash();

    assert!(matches!(
        control.advance(&mut stores),
        Err(ExecError::PdfExtensionInDviMode("pdfximage"))
    ));
    assert_eq!(stores.testing_state_hash(), state_before);
    assert!(stores.pdf_external_images().is_empty());
    assert_eq!(stores.pdf_last_external_image(), None);
    assert!(control.modes.current_list().nodes().is_empty());

    stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
    let pdf_state_before = stores.testing_state_hash();
    let request = match control
        .advance(&mut stores)
        .expect("PDF image request suspends")
    {
        CanonicalStepResult::Suspended(CanonicalResourceNeed::PdfImage { request }) => request,
        other => panic!("expected image suspension, got {other:?}"),
    };
    assert_eq!(stores.testing_state_hash(), pdf_state_before);
    assert!(stores.pdf_external_images().is_empty());
    assert_eq!(request.name, "image.pdf");
    assert_eq!(request.width, Some(Scaled::from_raw(10 * Scaled::UNITY)));
    assert_eq!(request.height, Some(Scaled::from_raw(20 * Scaled::UNITY)));
    assert_eq!(request.depth, Some(Scaled::from_raw(3 * Scaled::UNITY)));
    assert_eq!(request.page, tex_command::PdfImagePageSelection::Number(2));
    assert_eq!(request.page_box, tex_command::PdfImagePageBox::Media);
    assert!(request.page_box_explicit);
    assert!(request.attr.is_some());

    control.capabilities_mut().register_pdf_image(
        request,
        PdfImageResource::Available(test_pdf_image_source()),
    );
    assert_eq!(
        control
            .advance(&mut stores)
            .expect("fulfilled retry preserves and consumes the complete request"),
        CanonicalStepResult::Progress(MainControlStep::Continue)
    );
    let image = stores
        .pdf_last_external_image()
        .expect("retried image is allocated");
    assert_eq!(
        image.dimensions().width,
        Scaled::from_raw(10 * Scaled::UNITY)
    );
    assert_eq!(
        image.dimensions().height,
        Scaled::from_raw(20 * Scaled::UNITY)
    );
    assert_eq!(
        image.dimensions().depth,
        Scaled::from_raw(3 * Scaled::UNITY)
    );
    assert!(control.modes.current_list().nodes().is_empty());
}

#[test]
fn immediate_pdf_image_uses_the_same_preflight_and_transactional_retry() {
    // pdftex.web §1621 expands the command after `\immediate`, then invokes
    // §1551's complete `\pdfximage` case. Its output check precedes every
    // image operand and the recursive call performs the allocation only
    // after resource lookup succeeds.
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = pdftex_image_control(&mut stores);
    register_source(
        &mut control,
        br"\immediate\pdfximage width 7pt height 8pt depth 2pt attr{/Intent /RelativeColorimetric} page 3 cropbox {immediate.pdf}",
    );
    let state_before = stores.testing_state_hash();

    assert!(matches!(
        control.advance(&mut stores),
        Err(ExecError::PdfExtensionInDviMode("pdfximage"))
    ));
    assert_eq!(stores.testing_state_hash(), state_before);
    assert!(stores.pdf_external_images().is_empty());
    assert!(control.modes.current_list().nodes().is_empty());

    stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
    let pdf_state_before = stores.testing_state_hash();
    let request = match control
        .advance(&mut stores)
        .expect("immediate image suspends")
    {
        CanonicalStepResult::Suspended(CanonicalResourceNeed::PdfImage { request }) => request,
        other => panic!("expected immediate image suspension, got {other:?}"),
    };
    assert_eq!(stores.testing_state_hash(), pdf_state_before);
    assert_eq!(request.name, "immediate.pdf");
    assert_eq!(request.width, Some(Scaled::from_raw(7 * Scaled::UNITY)));
    assert_eq!(request.height, Some(Scaled::from_raw(8 * Scaled::UNITY)));
    assert_eq!(request.depth, Some(Scaled::from_raw(2 * Scaled::UNITY)));
    assert_eq!(request.page, tex_command::PdfImagePageSelection::Number(3));
    assert_eq!(request.page_box, tex_command::PdfImagePageBox::Crop);
    assert!(request.attr.is_some());

    control.capabilities_mut().register_pdf_image(
        request,
        PdfImageResource::Available(test_pdf_image_source()),
    );
    assert_eq!(
        control
            .advance(&mut stores)
            .expect("immediate image retry allocates in the same operation"),
        CanonicalStepResult::Progress(MainControlStep::Continue)
    );
    assert_eq!(stores.pdf_external_images().len(), 1);
    assert!(control.modes.current_list().nodes().is_empty());
}

#[test]
fn pdf_image_reference_preflights_all_modes_before_scan_lookup_or_list_mutation() {
    // pdftex.web §1552 is an `any_mode(extension)` case whose first operation
    // is `check_pdfoutput`. The DVI error therefore wins over an invalid
    // object in every mode and leaves the integer and list untouched.
    for mode in [
        Mode::Vertical,
        Mode::InternalVertical,
        Mode::Horizontal,
        Mode::RestrictedHorizontal,
        Mode::Math,
        Mode::DisplayMath,
    ] {
        let mut stores = Universe::new_with_plain_catcodes();
        let mut control = pdftex_image_control(&mut stores);
        if mode != Mode::Vertical {
            control.modes.push(mode).expect("test mode push");
        }
        register_source(&mut control, br"\pdfrefximage 99");
        let state_before = stores.testing_state_hash();

        assert!(
            matches!(
                control.advance(&mut stores),
                Err(ExecError::PdfExtensionInDviMode("pdfrefximage"))
            ),
            "mode {mode:?}"
        );
        assert_eq!(stores.testing_state_hash(), state_before, "mode {mode:?}");
        assert!(control.modes.current_list().nodes().is_empty());
        assert!(terminal_text(&stores).is_empty());
    }

    let mut stores = Universe::new_with_plain_catcodes();
    let source = test_pdf_image_source();
    let image = stores
        .allocate_pdf_external_image(
            source,
            tex_state::PdfExternalImageDimensions {
                width: Scaled::from_raw(11),
                height: Scaled::from_raw(12),
                depth: Scaled::from_raw(13),
            },
            0,
        )
        .expect("reference target image");
    assert_eq!(image.id().raw(), 1);
    let mut control = pdftex_image_control(&mut stores);
    control.modes.push(Mode::Math).expect("test mode push");
    register_source(&mut control, br"\pdfrefximage 1");
    let state_before = stores.testing_state_hash();

    assert!(matches!(
        control.advance(&mut stores),
        Err(ExecError::PdfExtensionInDviMode("pdfrefximage"))
    ));
    assert_eq!(stores.testing_state_hash(), state_before);
    assert!(control.modes.current_list().nodes().is_empty());

    stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
    assert_eq!(
        control
            .advance(&mut stores)
            .expect("PDF retry preserves the reference integer"),
        CanonicalStepResult::Progress(MainControlStep::Continue)
    );
    assert!(matches!(
        control.modes.current_list().nodes(),
        [Node::Whatsit(Whatsit::PdfRefXImage {
            object: 1,
            width,
            height,
            depth,
        })] if *width == Scaled::from_raw(11)
            && *height == Scaled::from_raw(12)
            && *depth == Scaled::from_raw(13)
    ));

    let mut missing_stores = Universe::new_with_plain_catcodes();
    let mut missing = pdftex_image_control(&mut missing_stores);
    register_source(&mut missing, br"\pdfrefximage 99");
    assert!(matches!(
        missing.advance(&mut missing_stores),
        Err(ExecError::PdfExtensionInDviMode("pdfrefximage"))
    ));
    missing_stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
    assert!(matches!(
        missing.advance(&mut missing_stores),
        Err(ExecError::PdfReferencedObjectNotFound)
    ));
    assert!(missing.modes.current_list().nodes().is_empty());
}

fn pdftex_annotation_control(stores: &mut Universe) -> CanonicalMainControl {
    for (name, primitive) in [
        ("pdfannot", UnexpandablePrimitive::PdfAnnot),
        ("pdfstartlink", UnexpandablePrimitive::PdfStartLink),
        ("pdfendlink", UnexpandablePrimitive::PdfEndLink),
    ] {
        let symbol = stores.intern(name);
        stores.set_meaning(symbol, Meaning::UnexpandablePrimitive(primitive));
    }
    CanonicalMainControl::with_profile(tex_command::CommandProfile::PDFTEX14027)
}

#[test]
fn pdf_annotation_family_rejects_dvi_before_allocation_or_operand_scan() {
    // pdftex.web §§1558, 1560, and 1561 call `check_pdfoutput` before object
    // allocation, mode legality, dimensions, attributes, actions, or body
    // text. A failed step must therefore retain the complete command.
    for (source, primitive) in [
        (
            br"\pdfannot width 5pt height 6pt depth 7pt {/Subtype /Text}".as_slice(),
            "pdfannot",
        ),
        (
            br"\pdfstartlink width 8pt height 9pt depth 10pt attr{/Border [0 0 0]} user{/Subtype /Link}"
                .as_slice(),
            "pdfstartlink",
        ),
    ] {
        let mut stores = Universe::new_with_plain_catcodes();
        let mut control = pdftex_annotation_control(&mut stores);
        control.modes.push(Mode::Horizontal).expect("test mode push");
        register_source(&mut control, source);
        assert!(
            matches!(control.step(&mut stores), Err(ExecError::PdfExtensionInDviMode(name)) if name == primitive)
        );
        assert!(control.modes.current_list().nodes().is_empty());

        stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
        assert_eq!(
            control
                .step(&mut stores)
                .expect("PDF retry preserves the complete command"),
            MainControlStep::Continue
        );
        assert_eq!(control.modes.current_list().nodes().len(), 1);
    }

    // The source orders the PDF-output check before the vertical-mode check
    // for both link commands.
    for primitive in ["pdfstartlink", "pdfendlink"] {
        let mut stores = Universe::new_with_plain_catcodes();
        let mut control = pdftex_annotation_control(&mut stores);
        register_source(&mut control, format!("\\{primitive}").as_bytes());
        assert!(
            matches!(control.step(&mut stores), Err(ExecError::PdfExtensionInDviMode(name)) if name == primitive)
        );
        assert!(control.modes.current_list().nodes().is_empty());
    }
}

#[test]
fn pdf_link_vertical_mode_rejects_before_operand_scan_without_mutation() {
    // pdftex.web §1561 checks vertical mode before `new_annot_whatsit` and
    // therefore before the rule, attributes, and action.  The deliberately
    // malformed action must not mask the mode diagnostic, consume its
    // following token, allocate a link, or append a node.
    let mut stores = Universe::new_with_plain_catcodes();
    stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
    let mut control = pdftex_annotation_control(&mut stores);
    register_source(
        &mut control,
        br"\pdfstartlink width 5pt definitely-not-an-action\relax",
    );
    let state_before = stores.testing_state_hash();

    let error = control
        .step(&mut stores)
        .expect_err("vertical link start is rejected before its operands");
    assert!(matches!(
        error,
        ExecError::PdfLinkInVerticalMode("pdfstartlink")
    ));
    assert_eq!(
        error.to_string(),
        "pdfTeX error (ext1): \\pdfstartlink cannot be used in vertical mode"
    );
    assert_eq!(stores.testing_state_hash(), state_before);
    assert!(control.modes.current_list().nodes().is_empty());

    control
        .modes
        .push(Mode::Horizontal)
        .expect("test mode push");
    assert!(matches!(
        control.step(&mut stores),
        Err(ExecError::PdfNavigation(
            "pdfTeX error (ext1): action type missing"
        ))
    ));
    assert_eq!(stores.testing_state_hash(), state_before);
    assert!(control.modes.current_list().nodes().is_empty());
}

#[test]
fn pdf_end_link_dvi_retry_preserves_the_open_link_and_command() {
    // pdftex.web §1561 rejects DVI mode before appending the end whatsit. The
    // open-link stack and the unconsumed command both survive for retry.
    let mut stores = Universe::new_with_plain_catcodes();
    stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
    let mut control = pdftex_annotation_control(&mut stores);
    control
        .modes
        .push(Mode::Horizontal)
        .expect("test mode push");
    register_source(
        &mut control,
        br"\pdfstartlink height 4pt user{/Subtype /Link}\pdfendlink",
    );
    assert_eq!(
        control.step(&mut stores).expect("start link"),
        MainControlStep::Continue
    );
    assert_eq!(control.modes.current_list().nodes().len(), 1);

    stores.set_int_param_global(IntParam::PDF_OUTPUT, 0);
    assert!(matches!(
        control.step(&mut stores),
        Err(ExecError::PdfExtensionInDviMode("pdfendlink"))
    ));
    assert_eq!(control.modes.current_list().nodes().len(), 1);

    stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
    assert_eq!(
        control.step(&mut stores).expect("end-link retry"),
        MainControlStep::Continue
    );
    assert!(matches!(
        control.modes.current_list().nodes(),
        [
            Node::Whatsit(Whatsit::PdfLinkStart { .. }),
            Node::Whatsit(Whatsit::PdfLinkEnd { .. })
        ]
    ));
}

#[test]
fn pdf_thread_family_rejects_dvi_before_operand_scan() {
    // pdftex.web §1567 checks pdfoutput before allocation and operand scanning.
    for (source, primitive) in [
        (
            br"\pdfthread width 5pt attr{/I <<>>} name{retry}".as_slice(),
            "pdfthread",
        ),
        (
            br"\pdfstartthread depth 7pt num 42".as_slice(),
            "pdfstartthread",
        ),
        (br"\pdfendthread".as_slice(), "pdfendthread"),
    ] {
        let mut stores = Universe::new_with_plain_catcodes();
        let mut control = pdftex_thread_control(&mut stores);
        register_source(&mut control, source);
        assert!(
            matches!(control.step(&mut stores), Err(ExecError::PdfExtensionInDviMode(name)) if name == primitive)
        );
        assert!(control.modes.current_list().nodes().is_empty());
        stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
        assert_eq!(
            control
                .step(&mut stores)
                .expect("retry preserves every operand"),
            MainControlStep::Continue
        );
        assert_eq!(control.modes.current_list().nodes().len(), 1);
    }
}

fn pdftex_destination_control(stores: &mut Universe) -> CanonicalMainControl {
    let destination = stores.intern("pdfdest");
    stores.set_meaning(
        destination,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfDest),
    );
    CanonicalMainControl::with_profile(tex_command::CommandProfile::PDFTEX14027)
}

#[test]
fn pdf_destination_is_any_mode_ordered_typed_material() {
    // pdftex.web §§1524 and 1565: `\pdfdest` is an any-mode extension that
    // appends one typed whatsit after scanning its complete destination.
    const MODES: [Mode; 6] = [
        Mode::Vertical,
        Mode::InternalVertical,
        Mode::Horizontal,
        Mode::RestrictedHorizontal,
        Mode::Math,
        Mode::DisplayMath,
    ];
    for mode in MODES {
        let mut stores = Universe::new_with_plain_catcodes();
        stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
        let mut control = pdftex_destination_control(&mut stores);
        if mode != Mode::Vertical {
            control.modes.push(mode).expect("test mode push");
        }
        register_source(
            &mut control,
            br"\pdfdest struct 9 name{target} fitr width 2pt height 3pt depth 4pt",
        );
        assert_eq!(
            control.step(&mut stores).expect("destination command"),
            MainControlStep::Continue
        );
        let [Node::Whatsit(Whatsit::PdfDestination(destination))] =
            control.modes.current_list().nodes()
        else {
            panic!(
                "mode {mode:?}: expected one destination, got {:?}",
                control.modes.current_list().nodes()
            );
        };
        assert_eq!(destination.structure, Some(9));
        assert!(matches!(
            destination.kind,
            tex_state::node::PdfDestinationKind::FitRectangle(dimensions)
                if dimensions.width == Some(Scaled::from_raw(2 * Scaled::UNITY))
                    && dimensions.height == Some(Scaled::from_raw(3 * Scaled::UNITY))
                    && dimensions.depth == Some(Scaled::from_raw(4 * Scaled::UNITY))
        ));
        assert!(matches!(
            destination.identifier,
            tex_state::PdfActionIdentifier::Name(_)
        ));
    }
}

#[test]
fn pdf_destination_rejects_prefixes_and_dvi_before_operand_scan() {
    // pdftex.web §1565 calls `check_pdfoutput` before allocating the whatsit
    // or scanning `struct`, the identifier, the kind, or the rule dimensions.
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
    let global = stores.intern("global");
    stores.set_meaning(
        global,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Global),
    );
    let mut control = pdftex_destination_control(&mut stores);
    register_source(&mut control, br"\global\pdfdest name{prefixed} fit");
    assert_eq!(
        control.step(&mut stores).expect("prefix recovery"),
        MainControlStep::Continue
    );
    assert!(control.modes.current_list().nodes().is_empty());
    assert!(terminal_text(&stores).contains("You can't use a prefix with"));
    assert_eq!(
        control
            .step(&mut stores)
            .expect("replayed destination command"),
        MainControlStep::Continue
    );
    assert_eq!(control.modes.current_list().nodes().len(), 1);

    let mut dvi_stores = crate::test_harness::universe_with_plain_catcodes();
    let mut dvi = pdftex_destination_control(&mut dvi_stores);
    register_source(
        &mut dvi,
        br"\pdfdest struct 7 name{retry} fitr width 5pt height 6pt depth 7pt",
    );
    assert!(matches!(
        dvi.step(&mut dvi_stores),
        Err(ExecError::PdfExtensionInDviMode("pdfdest"))
    ));
    assert!(dvi.modes.current_list().nodes().is_empty());
    dvi_stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
    assert_eq!(
        dvi.step(&mut dvi_stores)
            .expect("failed destination retries with every operand intact"),
        MainControlStep::Continue
    );
    let [Node::Whatsit(Whatsit::PdfDestination(destination))] = dvi.modes.current_list().nodes()
    else {
        panic!("one retried destination expected");
    };
    assert_eq!(destination.structure, Some(7));
    assert!(matches!(
        destination.kind,
        tex_state::node::PdfDestinationKind::FitRectangle(dimensions)
            if dimensions.width == Some(Scaled::from_raw(5 * Scaled::UNITY))
                && dimensions.height == Some(Scaled::from_raw(6 * Scaled::UNITY))
                && dimensions.depth == Some(Scaled::from_raw(7 * Scaled::UNITY))
    ));
}

#[test]
fn pdf_destination_grouping_and_checkpoint_restore_preserve_node_ownership() {
    // pdftex.web §1565 appends a whatsit, not an eqtb assignment: ordinary
    // grouping does not undo it, while an engine checkpoint restores both the
    // current list and the unconsumed source for deterministic retry.
    let mut stores = Universe::new_with_plain_catcodes();
    stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
    let mut control = pdftex_destination_control(&mut stores);
    register_source(&mut control, br"{\pdfdest num 23 xyz zoom -40}");
    let checkpoint = control
        .capture_checkpoint(
            crate::EngineBoundary::OuterParagraphEnd,
            &mut stores,
            crate::ExecutionBudgetCounters::default(),
        )
        .expect("destination state checkpoints");
    for label in ["open group", "destination", "close group"] {
        assert_eq!(
            control.step(&mut stores).expect(label),
            MainControlStep::Continue
        );
    }
    assert_eq!(stores.group_depth(), 0);
    let first_hash = stores.testing_state_hash();
    assert!(matches!(
        control.modes.current_list().nodes(),
        [Node::Whatsit(Whatsit::PdfDestination(destination))]
            if matches!(
                destination.kind,
                tex_state::node::PdfDestinationKind::Xyz { zoom: Some(-40) }
            )
    ));

    control
        .restore_checkpoint(&checkpoint, &mut stores)
        .expect("destination state restores");
    assert!(control.modes.current_list().nodes().is_empty());
    for label in [
        "retried open group",
        "retried destination",
        "retried close group",
    ] {
        assert_eq!(
            control.step(&mut stores).expect(label),
            MainControlStep::Continue
        );
    }
    assert_eq!(stores.testing_state_hash(), first_hash);
    assert!(matches!(
        control.modes.current_list().nodes(),
        [Node::Whatsit(Whatsit::PdfDestination(destination))]
            if matches!(
                destination.kind,
                tex_state::node::PdfDestinationKind::Xyz { zoom: Some(-40) }
            )
    ));
}

#[test]
fn pdf_outline_is_immediate_any_mode_document_state() {
    const MODES: [Mode; 6] = [
        Mode::Vertical,
        Mode::InternalVertical,
        Mode::Horizontal,
        Mode::RestrictedHorizontal,
        Mode::Math,
        Mode::DisplayMath,
    ];
    for mode in MODES {
        let mut stores = Universe::new_with_plain_catcodes();
        stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
        let mut control = pdftex_outline_control(&mut stores);
        if mode != Mode::Vertical {
            control.modes.push(mode).expect("test mode push");
        }
        register_source(
            &mut control,
            br"\pdfoutline attr{/C [1 0 0]} goto name{later} count -2 {(Title)}",
        );
        assert_eq!(
            control.step(&mut stores).expect("outline command"),
            MainControlStep::Continue
        );
        assert!(
            control.modes.current_list().nodes().is_empty(),
            "mode {mode:?}: outlines are immediate document state"
        );
        let [outline] = stores.pdf_outlines() else {
            panic!("mode {mode:?}: one outline expected");
        };
        assert_eq!(outline.count(), -2);
        assert_ne!(outline.attributes(), TokenListId::EMPTY);
        assert_eq!(
            (
                outline.action_object(),
                outline.item_object(),
                outline.title_object()
            ),
            (1, 2, 3),
            "the outline reserves action, item, and title identities in order"
        );
    }
}

#[test]
fn pdf_outline_rejects_prefixes_and_dvi_before_operand_scan() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
    let global = stores.intern("global");
    stores.set_meaning(
        global,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Global),
    );
    let mut control = pdftex_outline_control(&mut stores);
    register_source(&mut control, br"\global\pdfoutline user{/S /URI}{Title}");
    assert_eq!(
        control.step(&mut stores).expect("prefix recovery"),
        MainControlStep::Continue
    );
    assert!(stores.pdf_outlines().is_empty());
    assert!(terminal_text(&stores).contains("You can't use a prefix with"));
    assert_eq!(
        control.step(&mut stores).expect("replayed outline"),
        MainControlStep::Continue
    );
    assert_eq!(stores.pdf_outlines().len(), 1);

    let mut dvi_stores = crate::test_harness::universe_with_plain_catcodes();
    let mut dvi = pdftex_outline_control(&mut dvi_stores);
    register_source(&mut dvi, br"\pdfoutline user{/S /URI}{Title}");
    assert!(matches!(
        dvi.step(&mut dvi_stores),
        Err(ExecError::PdfExtensionInDviMode("pdfoutline"))
    ));
    assert!(dvi_stores.pdf_outlines().is_empty());
    dvi_stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
    assert_eq!(
        dvi.step(&mut dvi_stores)
            .expect("failed command retries with every operand intact"),
        MainControlStep::Continue
    );
    assert_eq!(dvi_stores.pdf_outlines().len(), 1);
}

#[test]
fn pdf_outline_is_not_restored_by_ordinary_grouping() {
    let mut stores = Universe::new_with_plain_catcodes();
    stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
    let mut control = pdftex_outline_control(&mut stores);
    register_source(
        &mut control,
        br"{\pdfoutline goto name{later} count 1 {Title}}",
    );
    for label in ["open group", "outline", "close group"] {
        assert_eq!(
            control.step(&mut stores).expect(label),
            MainControlStep::Continue
        );
    }
    assert_eq!(stores.group_depth(), 0);
    assert_eq!(stores.pdf_outlines().len(), 1);
}

#[test]
fn pdf_outline_checkpoint_restore_replays_identical_ledger_state() {
    let mut stores = Universe::new_with_plain_catcodes();
    stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
    let mut control = pdftex_outline_control(&mut stores);
    register_source(
        &mut control,
        br"\pdfoutline goto name{later} count 1 {Title}",
    );
    let checkpoint = control
        .capture_checkpoint(
            crate::EngineBoundary::OuterParagraphEnd,
            &mut stores,
            crate::ExecutionBudgetCounters::default(),
        )
        .expect("outline state checkpoints");
    assert_eq!(
        control.step(&mut stores).expect("outline command"),
        MainControlStep::Continue
    );
    let first_hash = stores.testing_state_hash();
    let first_objects = {
        let first = stores.pdf_outlines()[0];
        (
            first.action_object(),
            first.item_object(),
            first.title_object(),
            first.count(),
        )
    };
    control
        .restore_checkpoint(&checkpoint, &mut stores)
        .expect("outline state restores");
    assert!(stores.pdf_outlines().is_empty());
    assert_eq!(
        control.step(&mut stores).expect("retried outline"),
        MainControlStep::Continue
    );
    let retried = stores.pdf_outlines()[0];
    assert_eq!(
        (
            retried.action_object(),
            retried.item_object(),
            retried.title_object(),
            retried.count(),
        ),
        first_objects
    );
    assert_eq!(stores.testing_state_hash(), first_hash);
}

#[test]
fn pdf_snapping_is_any_mode_ordered_typed_material() {
    const MODES: [Mode; 6] = [
        Mode::Vertical,
        Mode::InternalVertical,
        Mode::Horizontal,
        Mode::RestrictedHorizontal,
        Mode::Math,
        Mode::DisplayMath,
    ];
    for mode in MODES {
        let mut stores = Universe::new_with_plain_catcodes();
        stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
        let mut control = pdftex_snapping_control(&mut stores);
        if mode != Mode::Vertical {
            control.modes.push(mode).expect("test mode push");
        }
        register_source(
            &mut control,
            br"\pdfsnaprefpoint\pdfsnapy 4pt plus 2fil minus 1pt\pdfsnapycomp 1200",
        );
        for _ in 0..3 {
            assert_eq!(
                control.step(&mut stores).expect("snapping command"),
                MainControlStep::Continue
            );
        }
        let nodes = control.modes.current_list().nodes();
        assert!(
            matches!(
                nodes,
                [
                    Node::Whatsit(Whatsit::PdfSnapRefPoint),
                    Node::Whatsit(Whatsit::PdfSnapY { .. }),
                    Node::Whatsit(Whatsit::PdfSnapYComp { ratio: 1000 })
                ]
            ),
            "mode {mode:?}: {nodes:?}"
        );
        let Node::Whatsit(Whatsit::PdfSnapY { glue }) = nodes[1] else {
            unreachable!()
        };
        let glue = stores.glue(glue);
        assert_eq!(glue.width, Scaled::from_raw(4 * 65_536));
        assert_eq!(glue.stretch_order, tex_state::glue::Order::Fil);
    }
}

#[test]
fn pdf_snapping_rejects_prefixes_and_dvi_before_operand_scan() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
    let global = stores.intern("global");
    stores.set_meaning(
        global,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Global),
    );
    let mut control = pdftex_snapping_control(&mut stores);
    register_source(&mut control, br"\global\pdfsnaprefpoint");
    assert_eq!(
        control.step(&mut stores).expect("prefix recovery"),
        MainControlStep::Continue
    );
    assert!(control.modes.current_list().nodes().is_empty());
    assert!(terminal_text(&stores).contains("You can't use a prefix with"));
    assert_eq!(
        control
            .step(&mut stores)
            .expect("replayed snapping command"),
        MainControlStep::Continue
    );
    assert!(matches!(
        control.modes.current_list().nodes(),
        [Node::Whatsit(Whatsit::PdfSnapRefPoint)]
    ));

    let mut dvi_stores = crate::test_harness::universe_with_plain_catcodes();
    let mut dvi = pdftex_snapping_control(&mut dvi_stores);
    register_source(&mut dvi, br"\pdfsnapy 7pt");
    assert!(matches!(
        dvi.step(&mut dvi_stores),
        Err(ExecError::PdfExtensionInDviMode("pdfsnapy"))
    ));
    assert!(dvi.modes.current_list().nodes().is_empty());
    dvi_stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
    assert_eq!(
        dvi.step(&mut dvi_stores)
            .expect("failed command retries with its operand intact"),
        MainControlStep::Continue
    );
    assert!(matches!(
        dvi.modes.current_list().nodes(),
        [Node::Whatsit(Whatsit::PdfSnapY { .. })]
    ));
}

#[test]
fn pdfsnapy_rejects_negative_width_after_consuming_the_complete_glue() {
    let mut stores = Universe::new_with_plain_catcodes();
    stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
    let mut control = pdftex_snapping_control(&mut stores);
    register_source(&mut control, br"\pdfsnapy -1pt plus 2fil");
    assert!(matches!(
        control.step(&mut stores),
        Err(ExecError::PdfNavigation(
            "pdfTeX error (ext1): negative snap glue"
        ))
    ));
    assert!(control.modes.current_list().nodes().is_empty());
}

#[test]
fn pdf_snapping_checkpoint_restore_retries_without_duplicate_nodes() {
    let mut stores = Universe::new_with_plain_catcodes();
    stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
    let mut control = pdftex_snapping_control(&mut stores);
    register_source(
        &mut control,
        br"\pdfsnaprefpoint\pdfsnapy 3pt\pdfsnapycomp 500",
    );
    assert_eq!(
        control.step(&mut stores).expect("reference point"),
        MainControlStep::Continue
    );
    let checkpoint = control
        .capture_checkpoint(
            crate::EngineBoundary::OuterParagraphEnd,
            &mut stores,
            crate::ExecutionBudgetCounters::default(),
        )
        .expect("snapping state checkpoints");
    assert_eq!(
        control.step(&mut stores).expect("snap glue"),
        MainControlStep::Continue
    );
    assert_eq!(
        control.step(&mut stores).expect("snap compensation"),
        MainControlStep::Continue
    );
    control
        .restore_checkpoint(&checkpoint, &mut stores)
        .expect("snapping state restores");
    assert_eq!(
        control.step(&mut stores).expect("retried snap glue"),
        MainControlStep::Continue
    );
    assert_eq!(
        control
            .step(&mut stores)
            .expect("retried snap compensation"),
        MainControlStep::Continue
    );
    assert!(matches!(
        control.modes.current_list().nodes(),
        [
            Node::Whatsit(Whatsit::PdfSnapRefPoint),
            Node::Whatsit(Whatsit::PdfSnapY { .. }),
            Node::Whatsit(Whatsit::PdfSnapYComp { ratio: 500 })
        ]
    ));
}

fn step_until_pdf_seed(control: &mut CanonicalMainControl, stores: &mut Universe, expected: i32) {
    for _ in 0..4 {
        control.step(stores).expect("canonical random command");
        if stores.world().pdf_random_seed() == expected {
            return;
        }
    }
    panic!("pdfTeX random seed did not become {expected}");
}

#[test]
fn pdfsetrandomseed_is_an_ungrouped_signed_job_state_replacement() {
    let mut stores = Universe::default();
    let mut control = pdftex_random_control(&mut stores);
    register_source(
        &mut control,
        br"{\pdfsetrandomseed -1 }\pdfsetrandomseed 23 ",
    );

    step_until_pdf_seed(&mut control, &mut stores, 1);
    assert_eq!(stores.world().pdf_random_seed(), 1);
    assert_eq!(stores.world_mut().pdf_uniform_deviate(10), 7);

    assert_eq!(
        control.step(&mut stores).expect("end group"),
        MainControlStep::Continue
    );
    assert_eq!(
        stores.world().pdf_random_seed(),
        1,
        "the extension state is not restored when a TeX group closes"
    );
    step_until_pdf_seed(&mut control, &mut stores, 23);
    assert_eq!(stores.world().pdf_random_seed(), 23);
}

#[test]
fn pdfsetrandomseed_uses_the_ordinary_integer_scanner_and_preserves_lookahead() {
    let mut stores = Universe::default();
    let mut control = pdftex_random_control(&mut stores);
    register_source(
        &mut control,
        br"\pdfsetrandomseed 999999999999\pdfsetrandomseed 6 ",
    );

    assert_eq!(
        control.step(&mut stores).expect("bounded seed scan"),
        MainControlStep::Continue
    );
    assert_eq!(stores.world().pdf_random_seed(), i32::MAX);

    assert_eq!(
        control
            .step(&mut stores)
            .expect("backed-up following command"),
        MainControlStep::Continue
    );
    assert_eq!(stores.world().pdf_random_seed(), 6);
}

#[test]
fn pdfsetrandomseed_rejects_assignment_prefixes_then_replays_the_command() {
    let mut stores = crate::test_harness::universe();
    let global = stores.intern("global");
    stores.set_meaning(
        global,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Global),
    );
    let mut control = pdftex_random_control(&mut stores);
    register_source(&mut control, br"\global\pdfsetrandomseed 9 ");

    assert_eq!(
        control.step(&mut stores).expect("reject prefix"),
        MainControlStep::Continue
    );
    assert_eq!(stores.world().pdf_random_seed(), 0);
    assert!(
        terminal_text(&stores).contains("You can't use a prefix with"),
        "the extension is below max_non_prefixed_command"
    );

    assert_eq!(
        control.step(&mut stores).expect("replayed seed command"),
        MainControlStep::Continue
    );
    assert_eq!(stores.world().pdf_random_seed(), 9);
}

#[test]
fn pdfresettimer_is_no_operand_any_mode_ungrouped_job_state() {
    let mut stores = Universe::default();
    stores.world_mut().set_pdf_time_micros(1_250_000);
    let mut control = pdftex_timer_control(&mut stores);
    register_source(&mut control, br"{\pdfresettimer X}");

    assert_eq!(
        control.step(&mut stores).expect("begin group"),
        MainControlStep::Continue
    );
    for _ in 0..3 {
        control.step(&mut stores).expect("timer reset");
        if stores.world().pdf_elapsed_time() == 0 {
            break;
        }
    }
    assert_eq!(stores.world().pdf_elapsed_time(), 0);

    stores.world_mut().set_pdf_time_micros(2_250_000);
    run_to_end(&mut control, &mut stores);
    assert_eq!(
        stores.world().pdf_elapsed_time(),
        65_536,
        "the reset is not restored by a group, and the following token was not consumed"
    );
}

#[test]
fn pdfresettimer_rejects_assignment_prefixes_then_replays_the_command() {
    let mut stores = crate::test_harness::universe();
    stores.world_mut().set_pdf_time_micros(1_250_000);
    let global = stores.intern("global");
    stores.set_meaning(
        global,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Global),
    );
    let mut control = pdftex_timer_control(&mut stores);
    register_source(&mut control, br"\global\pdfresettimer ");

    assert_eq!(
        control.step(&mut stores).expect("reject prefix"),
        MainControlStep::Continue
    );
    assert_eq!(stores.world().pdf_elapsed_time(), 81_920);
    assert!(terminal_text(&stores).contains("You can't use a prefix with"));

    assert_eq!(
        control.step(&mut stores).expect("replayed timer reset"),
        MainControlStep::Continue
    );
    assert_eq!(stores.world().pdf_elapsed_time(), 0);
}

#[test]
fn pdfinterwordspace_controls_are_operand_free_any_mode_ordered_whatsits() {
    const MODES: [Mode; 6] = [
        Mode::Vertical,
        Mode::InternalVertical,
        Mode::Horizontal,
        Mode::RestrictedHorizontal,
        Mode::Math,
        Mode::DisplayMath,
    ];

    for mode in MODES {
        let mut stores = Universe::new_with_plain_catcodes();
        stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
        let mut control = pdftex_interword_control(&mut stores);
        if mode != Mode::Vertical {
            control.modes.push(mode).expect("test mode push");
        }
        register_source(
            &mut control,
            br"\pdfinterwordspaceon\pdffakespace\pdfinterwordspaceoff",
        );
        run_to_end(&mut control, &mut stores);

        let controls: Vec<_> = control
            .modes
            .current_list()
            .nodes()
            .iter()
            .filter_map(|node| match node {
                Node::Whatsit(Whatsit::PdfAccessibility(control)) => Some(*control),
                _ => None,
            })
            .collect();
        assert_eq!(
            controls,
            [
                tex_state::node::PdfAccessibilityControl::InterwordSpaceOn,
                tex_state::node::PdfAccessibilityControl::FakeSpace,
                tex_state::node::PdfAccessibilityControl::InterwordSpaceOff,
            ],
            "mode {mode:?}: the controls remain ordered and consume no operand"
        );
    }

    let mut grouped_stores = Universe::new_with_plain_catcodes();
    grouped_stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
    let mut grouped = pdftex_interword_control(&mut grouped_stores);
    register_source(&mut grouped, br"{\pdffakespace}");
    run_to_end(&mut grouped, &mut grouped_stores);
    assert!(matches!(
        grouped.modes.current_list().nodes(),
        [Node::Whatsit(Whatsit::PdfAccessibility(
            tex_state::node::PdfAccessibilityControl::FakeSpace
        ))]
    ));
}

#[test]
fn pdfinterwordspace_rejects_prefixes_and_dvi_mode_before_appending() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
    let global = stores.intern("global");
    stores.set_meaning(
        global,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Global),
    );
    let mut control = pdftex_interword_control(&mut stores);
    register_source(&mut control, br"\global\pdfinterwordspaceon");

    assert_eq!(
        control.step(&mut stores).expect("prefix recovery"),
        MainControlStep::Continue
    );
    assert!(control.modes.current_list().nodes().is_empty());
    assert!(terminal_text(&stores).contains("You can't use a prefix with"));
    assert_eq!(
        control.step(&mut stores).expect("replayed extension"),
        MainControlStep::Continue
    );
    assert!(matches!(
        control.modes.current_list().nodes(),
        [Node::Whatsit(Whatsit::PdfAccessibility(
            tex_state::node::PdfAccessibilityControl::InterwordSpaceOn
        ))]
    ));

    let mut dvi_stores = crate::test_harness::universe_with_plain_catcodes();
    let mut dvi_control = pdftex_interword_control(&mut dvi_stores);
    register_source(&mut dvi_control, br"\pdffakespace");
    assert!(matches!(
        dvi_control.step(&mut dvi_stores),
        Err(ExecError::PdfExtensionInDviMode("pdffakespace"))
    ));
    assert!(dvi_control.modes.current_list().nodes().is_empty());
}

#[test]
fn pdfinterwordspace_checkpoint_restore_retries_without_duplicate_effects() {
    let mut stores = Universe::new_with_plain_catcodes();
    stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
    let mut control = pdftex_interword_control(&mut stores);
    register_source(&mut control, br"\pdfinterwordspaceon\pdfinterwordspaceoff");

    assert_eq!(
        control.step(&mut stores).expect("first toggle"),
        MainControlStep::Continue
    );
    let checkpoint = control
        .capture_checkpoint(
            crate::EngineBoundary::OuterParagraphEnd,
            &mut stores,
            crate::ExecutionBudgetCounters::default(),
        )
        .expect("quiescent toggle state checkpoints");
    assert_eq!(
        control.step(&mut stores).expect("second toggle"),
        MainControlStep::Continue
    );
    control
        .restore_checkpoint(&checkpoint, &mut stores)
        .expect("toggle state restores");
    assert_eq!(
        control.step(&mut stores).expect("second toggle retries"),
        MainControlStep::Continue
    );

    let controls: Vec<_> = control
        .modes
        .current_list()
        .nodes()
        .iter()
        .filter_map(|node| match node {
            Node::Whatsit(Whatsit::PdfAccessibility(control)) => Some(*control),
            _ => None,
        })
        .collect();
    assert_eq!(
        controls,
        [
            tex_state::node::PdfAccessibilityControl::InterwordSpaceOn,
            tex_state::node::PdfAccessibilityControl::InterwordSpaceOff,
        ]
    );
}

#[test]
fn pdfrunninglink_controls_are_operand_free_any_mode_ordered_whatsits() {
    const MODES: [Mode; 6] = [
        Mode::Vertical,
        Mode::InternalVertical,
        Mode::Horizontal,
        Mode::RestrictedHorizontal,
        Mode::Math,
        Mode::DisplayMath,
    ];

    for mode in MODES {
        let mut stores = Universe::new_with_plain_catcodes();
        stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
        let mut control = pdftex_interword_control(&mut stores);
        if mode != Mode::Vertical {
            control.modes.push(mode).expect("test mode push");
        }
        register_source(&mut control, br"\pdfrunninglinkoff\pdfrunninglinkon");
        run_to_end(&mut control, &mut stores);

        let toggles = control
            .modes
            .current_list()
            .nodes()
            .iter()
            .filter_map(|node| match node {
                Node::Whatsit(Whatsit::PdfRunningLink(enabled)) => Some(*enabled),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            toggles,
            [false, true],
            "mode {mode:?}: ordered toggle whatsits consume no operand"
        );
    }

    let mut grouped_stores = Universe::new_with_plain_catcodes();
    grouped_stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
    let mut grouped = pdftex_interword_control(&mut grouped_stores);
    register_source(&mut grouped, br"{\pdfrunninglinkoff\pdfrunninglinkon}");
    run_to_end(&mut grouped, &mut grouped_stores);
    assert!(matches!(
        grouped.modes.current_list().nodes(),
        [
            Node::Whatsit(Whatsit::PdfRunningLink(false)),
            Node::Whatsit(Whatsit::PdfRunningLink(true))
        ]
    ));
}

#[test]
fn pdfrunninglink_rejects_prefixes_and_dvi_mode_before_appending() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
    let global = stores.intern("global");
    stores.set_meaning(
        global,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Global),
    );
    let mut control = pdftex_interword_control(&mut stores);
    register_source(&mut control, br"\global\pdfrunninglinkoff");

    assert_eq!(
        control.step(&mut stores).expect("prefix recovery"),
        MainControlStep::Continue
    );
    assert!(control.modes.current_list().nodes().is_empty());
    assert!(terminal_text(&stores).contains("You can't use a prefix with"));
    assert_eq!(
        control.step(&mut stores).expect("replayed extension"),
        MainControlStep::Continue
    );
    assert!(matches!(
        control.modes.current_list().nodes(),
        [Node::Whatsit(Whatsit::PdfRunningLink(false))]
    ));

    let mut dvi_stores = crate::test_harness::universe_with_plain_catcodes();
    let mut dvi_control = pdftex_interword_control(&mut dvi_stores);
    register_source(&mut dvi_control, br"\pdfrunninglinkon");
    assert!(matches!(
        dvi_control.step(&mut dvi_stores),
        Err(ExecError::PdfExtensionInDviMode("pdfrunninglinkon"))
    ));
    assert!(dvi_control.modes.current_list().nodes().is_empty());
}

#[test]
fn pdfrunninglink_checkpoint_restore_retries_without_duplicate_whatsits() {
    let mut stores = Universe::new_with_plain_catcodes();
    stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
    let mut control = pdftex_interword_control(&mut stores);
    register_source(&mut control, br"\pdfrunninglinkoff\pdfrunninglinkon");

    assert_eq!(
        control.step(&mut stores).expect("first toggle"),
        MainControlStep::Continue
    );
    let checkpoint = control
        .capture_checkpoint(
            crate::EngineBoundary::OuterParagraphEnd,
            &mut stores,
            crate::ExecutionBudgetCounters::default(),
        )
        .expect("running-link toggle checkpoints");
    assert_eq!(
        control.step(&mut stores).expect("second toggle"),
        MainControlStep::Continue
    );
    control
        .restore_checkpoint(&checkpoint, &mut stores)
        .expect("running-link toggle restores");
    assert_eq!(
        control.step(&mut stores).expect("second toggle retries"),
        MainControlStep::Continue
    );

    let toggles = control
        .modes
        .current_list()
        .nodes()
        .iter()
        .filter_map(|node| match node {
            Node::Whatsit(Whatsit::PdfRunningLink(enabled)) => Some(*enabled),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(toggles, [false, true]);
}

#[test]
fn pdfspacefont_scans_expanded_balanced_text_globally_in_every_mode() {
    const MODES: [Mode; 6] = [
        Mode::Vertical,
        Mode::InternalVertical,
        Mode::Horizontal,
        Mode::RestrictedHorizontal,
        Mode::Math,
        Mode::DisplayMath,
    ];

    for mode in MODES {
        let mut stores = Universe::new_with_plain_catcodes();
        stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
        let replacement = stores.intern_token_list(
            &"fixture"
                .chars()
                .map(|ch| Token::Char {
                    ch,
                    cat: Catcode::Letter,
                })
                .collect::<Vec<_>>(),
        );
        let name = stores.intern("n");
        stores.set_macro_meaning_global(
            name,
            MacroMeaning::new(MeaningFlags::EMPTY, TokenListId::EMPTY, replacement),
        );
        let mut control = pdftex_interword_control(&mut stores);
        if mode != Mode::Vertical {
            control.modes.push(mode).expect("test mode push");
        }
        register_source(&mut control, br"{\pdfspacefont{\n-space}}X");
        run_to_end(&mut control, &mut stores);

        assert_eq!(
            stores.pdf_space_font_name(1),
            Some(b"fixture-space".as_slice()),
            "mode {mode:?}: expanded general text selects the typed global name"
        );
    }
}

#[test]
fn pdfspacefont_rejects_prefixes_and_dvi_mode_before_scanning() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
    let global = stores.intern("global");
    stores.set_meaning(
        global,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Global),
    );
    let mut control = pdftex_interword_control(&mut stores);
    register_source(&mut control, br"\global\pdfspacefont{selected}");

    assert_eq!(
        control.step(&mut stores).expect("prefix recovery"),
        MainControlStep::Continue
    );
    assert_eq!(stores.pdf_space_font_name(1), None);
    assert!(terminal_text(&stores).contains("You can't use a prefix with"));
    assert_eq!(
        control.step(&mut stores).expect("replayed extension"),
        MainControlStep::Continue
    );
    assert_eq!(stores.pdf_space_font_name(1), Some(b"selected".as_slice()));

    let mut dvi_stores = crate::test_harness::universe_with_plain_catcodes();
    let mut dvi_control = pdftex_interword_control(&mut dvi_stores);
    register_source(&mut dvi_control, br"\pdfspacefont{unscanned}");
    assert!(matches!(
        dvi_control.step(&mut dvi_stores),
        Err(ExecError::PdfExtensionInDviMode("pdfspacefont"))
    ));
    assert_eq!(dvi_stores.pdf_space_font_name(1), None);
}

#[test]
fn pdfspacefont_checkpoint_restore_retries_the_global_selection_atomically() {
    let mut stores = Universe::new_with_plain_catcodes();
    stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
    let mut control = pdftex_interword_control(&mut stores);
    register_source(&mut control, br"\pdfspacefont{first}\pdfspacefont{second}");

    assert_eq!(
        control.step(&mut stores).expect("first selection"),
        MainControlStep::Continue
    );
    let checkpoint = control
        .capture_checkpoint(
            crate::EngineBoundary::OuterParagraphEnd,
            &mut stores,
            crate::ExecutionBudgetCounters::default(),
        )
        .expect("space-font state checkpoints");
    assert_eq!(
        control.step(&mut stores).expect("second selection"),
        MainControlStep::Continue
    );
    assert_eq!(stores.pdf_space_font_name(2), Some(b"second".as_slice()));

    control
        .restore_checkpoint(&checkpoint, &mut stores)
        .expect("space-font state restores");
    assert_eq!(stores.pdf_space_font_name(2), None);
    assert_eq!(
        control.step(&mut stores).expect("second selection retries"),
        MainControlStep::Continue
    );
    assert_eq!(stores.pdf_space_font_name(2), Some(b"second".as_slice()));
}

#[test]
fn macro_parameter_errors_have_distinct_tex82_diagnostics_and_commit_scope() {
    struct Case {
        source: &'static [u8],
        target: &'static str,
        required: &'static [&'static str],
        forbidden: &'static str,
        committed: bool,
    }
    let cases = [
        Case {
            source: br"\def\bad#2{x}\end",
            target: "bad",
            required: &[
                "! Parameters must be numbered consecutively.",
                "I've inserted the digit you should have used after the #.",
                "Type `1' to delete what you did use.",
            ],
            forbidden: "Illegal parameter number in definition",
            committed: true,
        },
        Case {
            source: br"\def\bad{#x}\end",
            target: "bad",
            required: &[
                "! Illegal parameter number in definition of \\bad.",
                "You meant to type ## instead of #, right?",
                "Or maybe a } was forgotten somewhere earlier, and things",
                "are all screwed up? I'm going to assume that you meant ##.",
            ],
            forbidden: "Parameters must be numbered consecutively",
            committed: true,
        },
        Case {
            source: br"{\def\local{#x}}\end",
            target: "local",
            required: &[
                "! Illegal parameter number in definition of \\local.",
                "You meant to type ## instead of #, right?",
            ],
            forbidden: "Parameters must be numbered consecutively",
            committed: false,
        },
        Case {
            source: br"{\global\def\global{#x}}\end",
            target: "global",
            required: &[
                "! Illegal parameter number in definition of \\global.",
                "You meant to type ## instead of #, right?",
            ],
            forbidden: "Parameters must be numbered consecutively",
            committed: true,
        },
        Case {
            source: br"\catcode`~=13 \def~{{#x}}\end",
            target: "~",
            required: &[
                "! Illegal parameter number in definition of ~.",
                "You meant to type ## instead of #, right?",
            ],
            forbidden: "Parameters must be numbered consecutively",
            committed: true,
        },
    ];

    for case in cases {
        let mut stores = crate::test_harness::universe_with_plain_catcodes();
        let mut control = CanonicalMainControl::tex82_initex(&mut stores);
        register_source(&mut control, case.source);
        run_to_end(&mut control, &mut stores);
        let output = terminal_text(&stores);
        for line in case.required {
            assert!(
                output.contains(line),
                "{:?}: missing {line:?} in {output}",
                case.source
            );
        }
        assert!(
            !output.contains(case.forbidden),
            "{:?}: unexpected {:?} in {output}",
            case.source,
            case.forbidden
        );
        let symbol = if case.target == "~" {
            stores
                .active_character_symbol('~')
                .expect("active target is interned")
        } else {
            stores
                .symbol(case.target)
                .expect("named target is interned")
        };
        assert_eq!(
            stores.macro_meaning(symbol).is_some(),
            case.committed,
            "{:?}: recovered definition scope",
            case.source
        );
    }
}

#[test]
fn macro_tenth_parameter_reports_exact_limit_error() {
    // TeX.web §476 consumes both tokens of the attempted tenth parameter,
    // reports the fixed limit diagnostic, and continues scanning the
    // definition. The resulting macro therefore still has exactly the nine
    // legal parameters and can be called normally.
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\nonstopmode\def\nine#1#2#3#4#5#6#7#8#9#0{[#1#9]}\message{RESULT:\nine abcdefghi}\end",
    );

    run_to_end(&mut control, &mut stores);

    let terminal = terminal_text(&stores);
    for exact_line in [
        "! You already have nine parameters.",
        "I'm going to ignore the # sign you just used,",
        "as well as the token that followed it.",
    ] {
        assert!(
            terminal.lines().any(|line| line == exact_line),
            "missing exact diagnostic line {exact_line:?} in {terminal}"
        );
    }
    assert_eq!(
        terminal
            .matches("! You already have nine parameters.")
            .count(),
        1,
        "the attempted tenth parameter is diagnosed once: {terminal}"
    );
    let nine = stores.symbol("nine").expect("macro target is interned");
    let meaning = stores
        .macro_meaning(nine)
        .expect("the recovered definition is committed");
    assert_eq!(
        stores.tokens(meaning.parameter_text()),
        &(1..=9).map(Token::Param).collect::<Vec<_>>()
    );
    assert!(
        terminal.contains("RESULT:[ai]"),
        "the recovered nine-parameter macro remains callable: {terminal}"
    );
}

#[derive(Default)]
struct ObservationRecorder(Vec<CommandObservation>);

impl CommandObserver for ObservationRecorder {
    fn committed(&mut self, observation: CommandObservation) {
        self.0.push(observation);
    }
}

#[test]
fn etex_identical_local_integer_parameter_reassignment_is_not_a_mutation() {
    // e-TeX §275: `eq_word_define` returns immediately when extended mode
    // locally assigns the value already present. The negative controls pin
    // that a changed local value and an identical global value still commit.
    let mut stores = Universe::new_with_plain_catcodes();
    tex_expand::install_expandable_primitives(&mut stores);
    tex_expand::install_etex_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    crate::install_etex_unexpandable_primitives(&mut stores);
    let mut control = CanonicalMainControl::prepared_initex(CommandProfile::ETEX26);
    register_source(
        &mut control,
        br"\endlinechar=13 \endlinechar=12 \global\endlinechar=12 \end",
    );
    let mut observations = ObservationRecorder::default();
    loop {
        match control
            .step_with_observer(&mut stores, &mut observations)
            .expect("e-TeX integer-parameter reassignments execute")
        {
            MainControlStep::End | MainControlStep::EndOfInput => break,
            MainControlStep::Continue => {}
        }
    }

    let mutations: Vec<_> = observations
        .0
        .iter()
        .filter_map(|observation| match observation {
            CommandObservation::Mutation(record) if record.target == "parameter" => {
                Some((record.value.as_str(), record.global))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        mutations,
        [
            ("integer_parameter:48=12", false),
            ("integer_parameter:48=12", true),
        ]
    );

    let mut tex82 = Universe::new_with_plain_catcodes();
    assert!(
        !etex_redundant_local_word_assignment(&tex82, 13, 13),
        "TeX82 has no e-TeX reassignment shortcut"
    );
    tex82.set_int_param_global(IntParam::ETEX_EXTENDED_MODE, 1);
    assert!(etex_redundant_local_word_assignment(&tex82, 13, 13));
    assert!(!etex_redundant_local_word_assignment(&tex82, 13, 12));
}

#[test]
fn etex_sparse_word_reassignment_retains_its_observed_boundary() {
    // e-TeX 2.6 [49.1236-1237] routes sparse count and dimen words through
    // `sa_w_def`, not §§277-278's dense `eq_word_define`. The canonical
    // oracle observes the sparse assignment boundary even when its value is
    // the default; dense identical assignments retain their shortcut.
    let mut stores = Universe::new_with_plain_catcodes();
    tex_expand::install_expandable_primitives(&mut stores);
    tex_expand::install_etex_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    crate::install_etex_unexpandable_primitives(&mut stores);
    let mut control = CanonicalMainControl::prepared_initex(CommandProfile::ETEX26);
    register_source(
        &mut control,
        br"{\count0=0 \dimen0=0pt \count300=0 \dimen301=0pt}\end",
    );
    let mut observations = ObservationRecorder::default();
    run_to_end_observed(&mut control, &mut stores, &mut observations);

    let mutations: Vec<_> = observations
        .0
        .iter()
        .filter_map(|observation| match observation {
            CommandObservation::Mutation(record) if record.target == "register" => {
                Some((record.key.as_deref(), record.value.as_str()))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        mutations,
        [(None, "count:300=0"), (Some("dimen:301"), "scaled:0"),]
    );
    assert_eq!(stores.count(300), 0);
    assert_eq!(stores.dimen(301), Scaled::from_raw(0));
}

#[test]
fn etex_sparse_register_reads_keep_the_extended_index_after_group_exit() {
    // e-TeX 2.6 [26.427] scans an internal word-register selector with
    // `scan_register_num`. Keep the real sparse value and the independently
    // chosen register-zero sentinel distinct so an eight-bit recovery cannot
    // masquerade as a state-restoration failure.
    let mut stores = Universe::new_with_plain_catcodes();
    tex_expand::install_expandable_primitives(&mut stores);
    tex_expand::install_etex_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    crate::install_etex_unexpandable_primitives(&mut stores);
    let mut control = CanonicalMainControl::prepared_initex(CommandProfile::ETEX26);
    register_source(
        &mut control,
        br"\begingroup\tracingrestores=1\count20=5\count2000=5\endgroup
           \begingroup{\tracingassigns=1\count2000=0}\count2001=5
           \ifnum\count2000=0 \global\count0=17\fi\endgroup\end",
    );
    run_to_end(&mut control, &mut stores);

    assert_eq!(
        stores.int_param(IntParam::ETEX_EXTENDED_MODE),
        1,
        "extended register domain must survive grouping"
    );
    assert_eq!(stores.count(2000), 0, "sparse state must restore to zero");
    assert_eq!(stores.count(0), 17);
}

#[test]
fn etex_toks_assignment_and_rhs_keep_sparse_register_indices() {
    // e-TeX 2.6 [49.1226--1227] uses `scan_register_num` for both the direct
    // token-register assignment target and a direct token-register RHS.
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = canonical_etex_initex(&mut stores);
    register_source(&mut control, br"\toks2000={a b c} \toks2001=\toks2000 \end");
    let mut observations = ObservationRecorder::default();
    run_to_end_observed(&mut control, &mut stores, &mut observations);

    let mutations = observations
        .0
        .iter()
        .filter_map(|observation| match observation {
            CommandObservation::Mutation(record) if record.target == "register" => {
                Some((record.key.as_deref(), record.tokens.as_ref()))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(mutations.len(), 2);
    assert_eq!(mutations[0].0, Some("toks:2000"));
    assert_eq!(mutations[1].0, Some("toks:2001"));
    assert_eq!(
        stores.tokens(stores.toks(2_001)),
        stores.tokens(stores.toks(2_000))
    );
    assert!(!stores.tokens(stores.toks(2_001)).is_empty());
    assert!(stores.tokens(stores.toks(0)).is_empty());
}

#[test]
fn etex_dense_token_list_reassignments_use_eq_define_shortcut() {
    // e-TeX 2.6 [19.277] returns from `eq_define` when both the command and
    // token-list pointer are unchanged. This covers both dense `\toks`
    // registers and token-list parameters; [49.1226]'s sparse `sa_def` path
    // retains its independently observed assignment boundary.
    let source = br"{\toks20={} \everypar={} \toks300={}
                      \global\toks20={} \global\everypar={}}\end";
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = canonical_etex_initex(&mut stores);
    register_source(&mut control, source);
    let mut observations = ObservationRecorder::default();
    run_to_end_observed(&mut control, &mut stores, &mut observations);

    let mutations = observations
        .0
        .iter()
        .filter_map(|observation| match observation {
            CommandObservation::Mutation(record)
                if record.target == "register" || record.target == "parameter" =>
            {
                Some((record.target, record.key.as_deref(), record.global))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        mutations,
        [
            ("register", Some("toks:300"), false),
            ("register", Some("toks:20"), true),
            ("parameter", Some("token_parameter:1"), true),
        ]
    );

    let mut tex82 = Universe::new_with_plain_catcodes();
    let mut tex82_control = CanonicalMainControl::tex82_initex(&mut tex82);
    register_source(&mut tex82_control, br"\toks20={} \everypar={} \end");
    let mut tex82_observations = ObservationRecorder::default();
    run_to_end_observed(&mut tex82_control, &mut tex82, &mut tex82_observations);
    assert_eq!(
        tex82_observations
            .0
            .iter()
            .filter(|observation| matches!(observation, CommandObservation::Mutation(_)))
            .count(),
        2,
        "TeX82 does not have e-TeX's identical-definition shortcut"
    );
}

#[test]
fn etex_sparse_setbox_observes_delayed_and_immediate_commits() {
    // TeX82 §§1077/1085 commits a constructed box only after its box group is
    // unsaved. e-TeX 2.6 [47.1077] sends targets above 255 through [53a]'s
    // `sa_def_box`, so those delayed writes (and immediate void operands) are
    // sparse mutation boundaries; the dense `eq_define` target stays silent.
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = canonical_etex_initex(&mut stores);
    register_source(
        &mut control,
        br"{\setbox20=\hbox{} \setbox300=\hbox{}
             \global\setbox301=\vbox{} \setbox302=\box0}\end",
    );
    let mut observations = ObservationRecorder::default();
    run_to_end_observed(&mut control, &mut stores, &mut observations);

    let mutations = observations
        .0
        .iter()
        .filter_map(|observation| match observation {
            CommandObservation::Mutation(record) if record.target == "register" => {
                Some((record.key.as_deref(), record.value.as_str(), record.global))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        mutations,
        [
            (Some("box:300"), "name:occupied", false),
            (Some("box:301"), "name:occupied", true),
            (Some("box:302"), "name:void", false),
        ]
    );
    assert!(stores.box_reg(20).is_none());
    assert!(stores.box_reg(300).is_none());
    assert!(stores.box_reg(301).is_some());
    assert!(stores.box_reg(302).is_none());
}

#[test]
fn etex_identical_local_code_reassignment_is_a_save_stack_noop() {
    // e-TeX §275 applies the `eq_word_define` reassignment shortcut to every
    // fullword eqtb location, including the code tables. The nested identical
    // assignment must not create a save-stack entry that can roll back over
    // the later global assignment.
    let mut stores = Universe::new_with_plain_catcodes();
    tex_expand::install_expandable_primitives(&mut stores);
    tex_expand::install_etex_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    crate::install_etex_unexpandable_primitives(&mut stores);
    let mut control = CanonicalMainControl::prepared_initex(CommandProfile::ETEX26);
    register_source(&mut control, br"{\lccode`A=`a \global\lccode`A=`z}\end");
    let mut observations = ObservationRecorder::default();
    loop {
        match control
            .step_with_observer(&mut stores, &mut observations)
            .expect("e-TeX code-table reassignments execute")
        {
            MainControlStep::End | MainControlStep::EndOfInput => break,
            MainControlStep::Continue => {}
        }
    }

    assert_eq!(stores.lccode('A'), u32::from('z'));
    let mutations: Vec<_> = observations
        .0
        .iter()
        .filter_map(|observation| match observation {
            CommandObservation::Mutation(record) if record.target == "code_table" => {
                Some((record.value.as_str(), record.global))
            }
            _ => None,
        })
        .collect();
    assert_eq!(mutations, [("lccode:65=122", true)]);

    let mut tex82 = Universe::new_with_plain_catcodes();
    assert!(
        !etex_redundant_local_word_assignment(&tex82, tex82.lccode('A'), u32::from('a')),
        "TeX82 performs the identical local eq_word_define"
    );
    tex82.set_int_param_global(IntParam::ETEX_EXTENDED_MODE, 1);
    assert!(etex_redundant_local_word_assignment(
        &tex82,
        tex82.lccode('A'),
        u32::from('a')
    ));

    let format = stores.dump_format().expect("dump extended e-TeX format");
    let mut loaded = Universe::from_format(tex_state::World::memory(), &format)
        .expect("restore extended e-TeX format");
    let mut loaded_control = CanonicalMainControl::with_profile(CommandProfile::ETEX26);
    register_source(
        &mut loaded_control,
        br"{\lccode`A=`z \global\lccode`A=`q}\end",
    );
    let mut loaded_observations = ObservationRecorder::default();
    loop {
        match loaded_control
            .step_with_observer(&mut loaded, &mut loaded_observations)
            .expect("format-loaded e-TeX code-table reassignments execute")
        {
            MainControlStep::End | MainControlStep::EndOfInput => break,
            MainControlStep::Continue => {}
        }
    }
    let loaded_mutations: Vec<_> = loaded_observations
        .0
        .iter()
        .filter_map(|observation| match observation {
            CommandObservation::Mutation(record) if record.target == "code_table" => {
                Some((record.value.as_str(), record.global))
            }
            _ => None,
        })
        .collect();
    assert_eq!(loaded_mutations, [("lccode:65=113", true)]);
}

#[test]
fn etex_zero_glue_parameter_reassignment_uses_canonical_pointer_identity() {
    // e-TeX §277 suppresses a local `eq_define` when both its type and
    // halfword identity are unchanged. TeX82 §1237 traps a scanned zero glue
    // specification to the shared `zero_glue` pointer before that test.
    // Separately scanned equal nonzero literals remain distinct pointers and
    // are the negative control.
    let mut stores = Universe::new_with_plain_catcodes();
    tex_expand::install_expandable_primitives(&mut stores);
    tex_expand::install_etex_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    crate::install_etex_unexpandable_primitives(&mut stores);
    let mut control = CanonicalMainControl::prepared_initex(CommandProfile::ETEX26);
    register_source(
        &mut control,
        br"\parfillskip=0pt \parfillskip=1pt \parfillskip=1pt \end",
    );
    let mut observations = ObservationRecorder::default();
    loop {
        match control
            .step_with_observer(&mut stores, &mut observations)
            .expect("e-TeX glue-parameter reassignments execute")
        {
            MainControlStep::End | MainControlStep::EndOfInput => break,
            MainControlStep::Continue => {}
        }
    }

    let mutations: Vec<_> = observations
        .0
        .iter()
        .filter_map(|observation| match observation {
            CommandObservation::Mutation(record)
                if record.key.as_deref() == Some("glue_parameter:14") =>
            {
                Some(record.value.as_str())
            }
            _ => None,
        })
        .collect();
    assert_eq!(mutations.len(), 2);
    assert_eq!(
        stores.glue(stores.glue_param(GlueParam::new(14))).width,
        Scaled::from_raw(65_536)
    );
}

#[test]
fn etex_glue_expression_reassignment_retains_source_pointer_identity() {
    // e-TeX expression change [53a.4945--5360] leaves a glue factor's node
    // untouched when no operator requires a copy. Section 277 therefore
    // classifies the local assignment back to the same register as a
    // reassignment. An equal literal, an expression that applies an operator,
    // and a global assignment are controls: all allocate or define and remain
    // observable.
    let mut stores = Universe::new_with_plain_catcodes();
    tex_expand::install_expandable_primitives(&mut stores);
    tex_expand::install_etex_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    crate::install_etex_unexpandable_primitives(&mut stores);
    let mut control = CanonicalMainControl::prepared_initex(CommandProfile::ETEX26);
    register_source(
        &mut control,
        br"\skip0=1pt \skip0=\glueexpr\skip0\relax \skip0=1pt \skip0=\glueexpr\skip0+0pt\relax \global\skip0=\glueexpr\skip0\relax \end",
    );
    let mut observations = ObservationRecorder::default();
    loop {
        match control
            .step_with_observer(&mut stores, &mut observations)
            .expect("e-TeX glue-expression reassignments execute")
        {
            MainControlStep::End | MainControlStep::EndOfInput => break,
            MainControlStep::Continue => {}
        }
    }

    let mutations: Vec<_> = observations
        .0
        .iter()
        .filter_map(|observation| match observation {
            CommandObservation::Mutation(record) if record.key.as_deref() == Some("skip:0") => {
                Some(record.global)
            }
            _ => None,
        })
        .collect();
    assert_eq!(mutations, [false, false, false, true]);
}

#[test]
fn etex_sparse_skip_reassignment_keeps_sa_def_mutation_boundary() {
    // e-TeX 2.6 [49.1221--1237] sends the sparse shorthand through `sa_def`.
    // Its identical-pointer branch avoids saving or rewriting the element but
    // still completes the sparse assignment boundary, unlike §§277-278's
    // dense `eq_define` shortcut.
    let mut stores = Universe::new_with_plain_catcodes();
    tex_expand::install_expandable_primitives(&mut stores);
    tex_expand::install_etex_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    crate::install_etex_unexpandable_primitives(&mut stores);
    let mut control = CanonicalMainControl::prepared_initex(CommandProfile::ETEX26);
    register_source(
        &mut control,
        br"\skipdef\alias=32767 \alias=1pt \alias=\glueexpr\alias\relax \end",
    );
    let mut observations = ObservationRecorder::default();
    run_to_end_observed(&mut control, &mut stores, &mut observations);

    let mutations = observations
        .0
        .iter()
        .filter(|observation| {
            matches!(
                observation,
                CommandObservation::Mutation(record)
                    if record.key.as_deref() == Some("skip:32767")
            )
        })
        .count();
    assert_eq!(mutations, 2);
    assert_eq!(
        stores.glue(stores.skip(32_767)).width,
        Scaled::from_raw(Scaled::UNITY)
    );
}

#[test]
fn etex_penalty_array_assignments_are_mode_complete_and_consume_exactly_their_values() {
    // e-TeX 2.6 change [49.1248] routes all four selectors through
    // TeX82 §1248's `set_shape`; e-TeX §§6336-6366 define the selector
    // family and its repeated-last-value enquiry semantics.
    const MODES: [Mode; 6] = [
        Mode::Vertical,
        Mode::InternalVertical,
        Mode::Horizontal,
        Mode::RestrictedHorizontal,
        Mode::Math,
        Mode::DisplayMath,
    ];
    const ARRAYS: [(&str, PenaltyArrayKind); 4] = [
        ("interlinepenalties", PenaltyArrayKind::InterLine),
        ("clubpenalties", PenaltyArrayKind::Club),
        ("widowpenalties", PenaltyArrayKind::Widow),
        ("displaywidowpenalties", PenaltyArrayKind::DisplayWidow),
    ];

    for (name, kind) in ARRAYS {
        for mode in MODES {
            let mut stores = Universe::new_with_plain_catcodes();
            let mut control = canonical_etex_initex(&mut stores);
            if mode != Mode::Vertical {
                control.modes.push(mode).expect("test mode push");
            }
            let source = format!(r"\{name}  =  2  101  -202 \count0=17");
            register_source(&mut control, source.as_bytes());

            assert_eq!(
                control.step(&mut stores).expect("penalty array assignment"),
                MainControlStep::Continue,
                "selector {name}, mode {mode:?}"
            );
            assert_eq!(stores.penalty_array(kind), vec![101, -202]);
            assert_eq!(stores.count(0), 0, "following command was not consumed");
            assert_eq!(control.current_mode(), mode);

            assert_eq!(
                control.step(&mut stores).expect("following assignment"),
                MainControlStep::Continue,
                "selector {name}, mode {mode:?}"
            );
            assert_eq!(stores.count(0), 17, "following command stayed live");
        }
    }
}

#[test]
fn etex_nonpositive_penalty_array_counts_clear_without_consuming_following_tokens() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = canonical_etex_initex(&mut stores);
    register_source(
        &mut control,
        br"\interlinepenalties=1 11 \interlinepenalties=0
           \clubpenalties=1 22 \clubpenalties=-1
           \widowpenalties=1 33 \widowpenalties=0
           \displaywidowpenalties=1 44 \displaywidowpenalties=-2
           \count0=19 \end",
    );

    run_to_end(&mut control, &mut stores);

    for kind in [
        PenaltyArrayKind::InterLine,
        PenaltyArrayKind::Club,
        PenaltyArrayKind::Widow,
        PenaltyArrayKind::DisplayWidow,
    ] {
        assert!(stores.penalty_array(kind).is_empty(), "array {kind:?}");
    }
    assert_eq!(
        stores.count(0),
        19,
        "zero and negative counts scan no values"
    );
}

#[test]
fn etex_penalty_array_scope_enquiries_and_afterassignment_match_set_shape() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = canonical_etex_initex(&mut stores);
    register_source(
        &mut control,
        br"\clubpenalties=2 200 100
           {\clubpenalties=1 7}
           \widowpenalties=2 300 400
           {\widowpenalties=1 7}
           {\globaldefs=1 \displaywidowpenalties=1 500}
           \interlinepenalties=2 9 8
           {\globaldefs=-1 \global\interlinepenalties=-4}
           \def\aftermark{\global\advance\count0 by1}
           \afterassignment\aftermark\clubpenalties=1 42
           \end",
    );

    run_to_end(&mut control, &mut stores);

    assert_eq!(stores.penalty_array_value(PenaltyArrayKind::Widow, 0), 2);
    assert_eq!(stores.penalty_array_value(PenaltyArrayKind::Widow, 1), 300);
    assert_eq!(stores.penalty_array_value(PenaltyArrayKind::Widow, 8), 400);
    assert_eq!(stores.penalty_array(PenaltyArrayKind::Club), vec![42]);
    assert_eq!(
        stores.penalty_array(PenaltyArrayKind::Widow),
        vec![300, 400]
    );
    assert_eq!(
        stores.penalty_array(PenaltyArrayKind::DisplayWidow),
        vec![500]
    );
    assert_eq!(
        stores.penalty_array(PenaltyArrayKind::InterLine),
        vec![9, 8]
    );
    assert_eq!(stores.count(0), 1, "afterassignment fired exactly once");
    assert_eq!(stores.take_afterassignment(), None);
}

#[test]
fn etex_penalty_array_assignment_restores_checkpoint_and_retries_atomically() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = canonical_etex_initex(&mut stores);
    register_source(&mut control, br"\clubpenalties=2 7 5 \count0=23 \end");
    let checkpoint = control
        .capture_checkpoint(
            crate::EngineBoundary::OuterParagraphEnd,
            &mut stores,
            crate::ExecutionBudgetCounters::default(),
        )
        .expect("penalty array state checkpoints");

    assert_eq!(
        control.step(&mut stores).expect("first assignment"),
        MainControlStep::Continue
    );
    assert_eq!(stores.penalty_array(PenaltyArrayKind::Club), vec![7, 5]);
    let assigned_hash = stores.testing_state_hash();

    control
        .restore_checkpoint(&checkpoint, &mut stores)
        .expect("penalty array state restores");
    assert!(stores.penalty_array(PenaltyArrayKind::Club).is_empty());
    assert_eq!(stores.count(0), 0);

    assert_eq!(
        control.step(&mut stores).expect("retried assignment"),
        MainControlStep::Continue
    );
    assert_eq!(stores.testing_state_hash(), assigned_hash);
    assert_eq!(stores.penalty_array(PenaltyArrayKind::Club), vec![7, 5]);
    assert_eq!(
        control.step(&mut stores).expect("following assignment"),
        MainControlStep::Continue
    );
    assert_eq!(stores.count(0), 23);
}

#[test]
fn main_control_dispatch_matrix_consumes_each_command_once() {
    const MODES: [Mode; 6] = [
        Mode::Vertical,
        Mode::InternalVertical,
        Mode::Horizontal,
        Mode::RestrictedHorizontal,
        Mode::Math,
        Mode::DisplayMath,
    ];

    for mode in MODES {
        let mut stores = Universe::new_with_plain_catcodes();
        let mut control = CanonicalMainControl::tex82_initex(&mut stores);
        if mode != Mode::Vertical {
            control.modes.push(mode).expect("test mode push");
        }
        register_source(&mut control, br"\count0=17\count1=29");

        let mut observations = ObservationRecorder::default();
        assert_eq!(
            control
                .step_with_observer(&mut stores, &mut observations)
                .expect("mode-independent assignment dispatches"),
            MainControlStep::Continue,
            "mode {mode:?}"
        );
        assert_eq!(stores.count(0), 17, "mode {mode:?}");
        assert_eq!(stores.count(1), 0, "mode {mode:?}");
        assert_eq!(control.current_mode(), mode);
        assert_eq!(
            observations
                .0
                .iter()
                .filter(|observation| matches!(observation, CommandObservation::Mutation(_)))
                .count(),
            1,
            "one main-control mutation committed in mode {mode:?}: {:?}",
            observations.0
        );
        assert!(observations.0.iter().any(|observation| matches!(observation, CommandObservation::Mutation(mutation) if mutation.value == "count:0=17")));

        observations.0.clear();
        assert_eq!(
            control
                .step_with_observer(&mut stores, &mut observations)
                .expect("following command remains available"),
            MainControlStep::Continue,
            "mode {mode:?}"
        );
        assert_eq!(stores.count(1), 29, "mode {mode:?}");
        assert_eq!(
            observations
                .0
                .iter()
                .filter(|observation| matches!(observation, CommandObservation::Mutation(_)))
                .count(),
            1,
            "the following command commits exactly once in mode {mode:?}"
        );
        assert!(observations.0.iter().any(|observation| matches!(observation, CommandObservation::Mutation(mutation) if mutation.value == "count:1=29")));
    }
}

#[test]
fn main_control_error_privilege_and_stop_paths_are_finite() {
    let mut internal_stores = crate::test_harness::universe_with_plain_catcodes();
    let mut internal = CanonicalMainControl::tex82_initex(&mut internal_stores);
    internal
        .modes
        .push(Mode::InternalVertical)
        .expect("test mode push");
    register_source(&mut internal, br"\end\count0=9");
    run_to_end(&mut internal, &mut internal_stores);
    assert_eq!(internal_stores.count(0), 9);
    assert_eq!(internal.current_mode(), Mode::InternalVertical);
    assert!(terminal_text(&internal_stores).contains("can't use `\\end'"));

    let mut page_stores = crate::test_harness::universe_with_plain_catcodes();
    let mut page = CanonicalMainControl::tex82_initex(&mut page_stores);
    register_source(&mut page, br"\hrule\end");
    let mut observations = ObservationRecorder::default();
    for _ in 0..32 {
        if matches!(
            page.step_with_observer(&mut page_stores, &mut observations)
                .expect("page stop remains finite"),
            MainControlStep::End | MainControlStep::EndOfInput
        ) {
            break;
        }
    }
    assert_eq!(page_stores.world().artifact_commits().len(), 1);
    assert!(observations.0.iter().any(|observation| matches!(
        observation,
        CommandObservation::Effect(effect) if effect.kind == "terminate"
    )));
}

#[test]
fn illegal_case_command_spelling_uses_live_escapechar() {
    // TeX82 §§63, 298, and 1049: `you_cant` renders the rejected command
    // through `print_cmd_chr`; its primitive cases use `print_esc`, whose
    // escape prefix is omitted when `\escapechar` is outside 0..255.
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    control
        .modes
        .push(Mode::InternalVertical)
        .expect("test mode push");
    register_source(&mut control, br"\escapechar=256\end");
    run_to_end(&mut control, &mut stores);

    let terminal = terminal_text(&stores);
    assert!(
        terminal.contains("You can't use `end' in internal vertical mode"),
        "{terminal:?}"
    );
    assert!(!terminal.contains("You can't use `\\end'"), "{terminal:?}");
}

#[test]
fn openin_closein_replace_stream_state_and_apply_filename_rules() {
    // TeX82 §§1272--1275 close an existing stream before replacement, retain
    // an explicit extension, supply `.tex` only when the extension is empty,
    // and make `\closein` restore the stream's closed/EOF state.
    let mut stores = Universe::new_with_plain_catcodes();
    stores.set_interaction_mode(tex_state::InteractionMode::ErrorStop);
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    for (name, bytes) in [("first.tex", &b"one"[..]), ("second.dat", &b"two"[..])] {
        control.capabilities_mut().register_input(
            name,
            SourceRegistration::new(RegisteredSourceKind::World, Arc::<[u8]>::from(bytes)),
        );
    }
    register_source(
        &mut control,
        br"\openin3=first \read3 to \first \openin3=second.dat \read3 to \second \closein3\end",
    );
    run_to_end(&mut control, &mut stores);
    assert_eq!(
        macro_tokens(&stores, "first")[0],
        Token::Char {
            ch: 'o',
            cat: Catcode::Letter,
        }
    );
    assert_eq!(
        macro_tokens(&stores, "second")[0],
        Token::Char {
            ch: 't',
            cat: Catcode::Letter,
        }
    );
    assert!(stores.input_stream_eof(tex_state::StreamSlot::new(3)));
}

/// TeX82 §314's macro arm is `print_ln; print_cs(name)`, and §319
/// pseudoprints `link(start)` -- the whole macro text -- so a macro level's
/// context line is `\\a #1->body`, naming the control sequence being expanded
/// and showing its parameter text ahead of the `->` §294 renders for
/// `end_match`.
#[test]
fn a_macro_context_level_names_the_macro_and_shows_its_parameter_text() {
    let mut stores = Universe::new_with_plain_catcodes();
    stores.set_int_param(tex_state::env::banks::IntParam::new(54), 5);
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(&mut control, br"\def\a#1{ x #1 \undefinedthing y}\a{Q}\end");
    run_to_end(&mut control, &mut stores);
    let terminal = terminal_text(&stores);
    assert!(
        terminal.contains("\\a #1-> x #1 \\undefinedthing \n"),
        "{terminal}"
    );
    assert!(!terminal.contains("<macro>"), "{terminal}");
}

/// TeX82 §1068's `handle_right_brace` sends `semi_simple_group`,
/// `math_shift_group` and `math_left_group` to §1069's `extra_right_brace`,
/// which names the opener the brace was standing in for. Only the remaining
/// `bottom_level` case is "Too many }'s".
#[test]
fn a_stray_right_brace_names_the_group_opener_it_replaced() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(&mut control, br"\hbox{$x}$}\begingroup}\end");
    run_to_end(&mut control, &mut stores);
    let terminal = terminal_text(&stores);
    assert!(
        terminal.contains("! Extra }, or forgotten $."),
        "{terminal}"
    );
    assert!(
        terminal.contains("! Extra }, or forgotten \\endgroup."),
        "{terminal}"
    );
    assert!(!terminal.contains("Too many }'s"), "{terminal}");
}

#[test]
fn extra_right_brace_in_an_argument_names_the_macro() {
    // TeX82 §395: a bare `}` where an argument was expected is backed up, a
    // `\\par` is inserted, and `ins_error` reports "Argument of \\a has an
    // extra }" -- `sprint_cs(warning_index)`, the macro whose argument was
    // being matched, not a placeholder.
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(&mut control, br"\def\a#1{[#1]}\a}\end");
    run_to_end(&mut control, &mut stores);
    let terminal = terminal_text(&stores);
    assert!(
        terminal.contains(
            "! Argument of \\a has an extra }.\n<inserted text> \n                \\par "
        ),
        "{terminal}"
    );
    // §395's `long_state:=call` is what makes §396 report next, on the very
    // `\\par` it just inserted.
    assert!(
        terminal.contains("! Paragraph ended before \\a was complete."),
        "{terminal}"
    );
}

#[test]
fn out_of_range_read_selector_reaches_the_terminal_without_a_report() {
    // TeX82 §1225 scans `\\read`'s stream with a plain `scan_int`, not §435's
    // `scan_four_bit_int`, and §482 answers `(n<0)or(n>15)` with `m:=16` --
    // the never-open stream whose §483 branch is the terminal. Stream 16 is
    // therefore an ordinary terminal read, not a recovered zero, and nothing
    // is reported.
    let mut stores = Universe::new_with_plain_catcodes();
    stores
        .world_mut()
        .push_memory_terminal_line("recovered")
        .expect("terminal line queues");
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(&mut control, br"\read16 to \line\end");
    let mut observations = ObservationRecorder::default();
    for _ in 0..64 {
        if matches!(
            control
                .step_with_observer(&mut stores, &mut observations)
                .expect("recovered read remains executable"),
            MainControlStep::End | MainControlStep::EndOfInput
        ) {
            break;
        }
    }

    assert_eq!(
        macro_tokens(&stores, "line")[0],
        Token::Char {
            ch: 'r',
            cat: Catcode::Letter,
        }
    );
    let terminal = terminal_text(&stores);
    assert!(!terminal.contains("Bad number"), "{terminal}");
    let integer = observations
        .0
        .iter()
        .position(|event| {
            matches!(
                event,
                CommandObservation::Scanner(scanner)
                    if scanner.kind == "integer" && scanner.value == "16"
            )
        })
        .expect("raw selector is observed");
    let mutation = observations
        .0
        .iter()
        .position(|event| {
            matches!(
                event,
                CommandObservation::Mutation(mutation)
                    if mutation.key.as_deref() == Some("line")
            )
        })
        .expect("recovered read target is committed");
    assert!(integer < mutation);
}

#[test]
fn read_to_definition_preserves_effective_scope_and_replay() {
    // TeX82 §§1214/1225 select scope before `read_toks`, then install its
    // parameterless macro after collection. Exercise explicit prefixes and
    // both `\globaldefs` overrides through ordinary replay.
    let mut stores = Universe::new_with_plain_catcodes();
    // `\read-1` first reports §433's out-of-range stream number. Keep this
    // scope/replay test in scroll mode so §82's error-stop dialog does not
    // canonically consume the terminal lines intended for the reads.
    stores.set_interaction_mode(tex_state::InteractionMode::Scroll);
    for line in ["local", "explicit", "forced-global", "forced-local"] {
        stores
            .world_mut()
            .push_memory_terminal_line(line)
            .expect("memory terminal accepts a line");
    }
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\def\local{old}{\read-1to\local}\def\explicit{old}{\global\read-1to\explicit}\globaldefs=1\def\forcedglobal{old}{\read-1to\forcedglobal}\globaldefs=-1\gdef\forcedlocal{old}{\global\read-1to\forcedlocal}\globaldefs=0\end",
    );
    run_to_end(&mut control, &mut stores);

    assert_eq!(
        macro_tokens(&stores, "local")[0],
        Token::Char {
            ch: 'o',
            cat: Catcode::Letter,
        }
    );
    assert_eq!(
        macro_tokens(&stores, "explicit")[0],
        Token::Char {
            ch: 'e',
            cat: Catcode::Letter,
        }
    );
    assert_eq!(
        macro_tokens(&stores, "forcedglobal")[0],
        Token::Char {
            ch: 'f',
            cat: Catcode::Letter,
        }
    );
    assert_eq!(
        macro_tokens(&stores, "forcedlocal")[0],
        Token::Char {
            ch: 'o',
            cat: Catcode::Letter,
        }
    );
}

#[test]
fn read_to_mutation_precedes_afterassignment_replay_and_carries_exact_meaning() {
    // TeX82 §1225 commits `define(p,call,cur_val)` before §1211 reaches
    // §1269's `done:` and backs up the saved afterassignment token.
    let mut stores = Universe::new_with_plain_catcodes();
    stores
        .world_mut()
        .push_memory_terminal_line("alpha")
        .expect("memory terminal accepts a line");
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\def\target{old}\afterassignment\relax\global\read-1to\target\end",
    );
    let mut observations = ObservationRecorder::default();
    loop {
        if matches!(
            control
                .step_with_observer(&mut stores, &mut observations)
                .expect("read and its replay execute"),
            MainControlStep::End | MainControlStep::EndOfInput
        ) {
            break;
        }
    }

    let mutation_index = observations
        .0
        .iter()
        .position(|observation| {
            matches!(
                observation,
                CommandObservation::Mutation(record)
                    if record.key.as_deref() == Some("target")
                        && record.value == "macro definition"
                        && record.global
            )
        })
        .expect("read meaning mutation is observed");
    let replay_index = observations
        .0
        .iter()
        .enumerate()
        .skip(mutation_index + 1)
        .position(|observation| {
            matches!(
                observation.1,
                CommandObservation::Input(record)
                    if record.transition == InputTransition::Backup
                        && record.reason == InputReason::Backup
            )
        })
        .map(|offset| mutation_index + 1 + offset)
        .expect("afterassignment replay is observed");
    assert!(mutation_index < replay_index, "{:?}", observations.0);
    let CommandObservation::Mutation(mutation) = &observations.0[mutation_index] else {
        unreachable!()
    };
    assert!(matches!(
        mutation.tokens.as_deref(),
        Some([
            tex_command::ObservedToken::MacroEndMatch,
            tex_command::ObservedToken::Character {
                character: 'a',
                catcode: Catcode::Letter,
            },
            ..
        ])
    ));
}

#[test]
fn message_expands_balanced_text_and_applies_terminal_line_spacing() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\def\value{expanded}\message{left {\value} right}\count0=7\end",
    );
    run_to_end(&mut control, &mut stores);
    assert_eq!(terminal_text(&stores), "left {expanded} right");
    assert_eq!(stores.count(0), 7, "message consumes its body exactly once");
}

#[test]
fn errmessage_selects_user_or_once_only_builtin_help_and_clears_flag() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\def\value{expanded}\errmessage{bad \value}\count0=8\end",
    );
    run_to_end(&mut control, &mut stores);
    let output = terminal_text(&stores);
    assert_eq!(output.matches("! bad expanded.").count(), 1, "{output}");
    assert_eq!(stores.count(0), 8, "error handling resumes main control");
}

#[test]
fn case_shift_preserves_raw_token_structure_at_code_table_boundaries() {
    // TeX82 §§1285--1289 scan unexpanded general text. §1288 substitutes
    // only character-token codes, preserving their command/category; zero
    // table entries and control-sequence tokens remain byte-for-byte tokens.
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\uccode`!=`Z\lccode`?=`y\catcode126=13\uccode126=88\uppercase{\gdef\up{!\relax}}\lowercase{\gdef\down{?\relax}}\uppercase{\gdef\active{~}}\uppercase{\gdef\zero{@}}\end",
    );
    run_to_end(&mut control, &mut stores);
    assert!(matches!(
        macro_tokens(&stores, "up"),
        [
            Token::Char {
                ch: 'Z',
                cat: Catcode::Other
            },
            Token::Cs(_)
        ]
    ));
    assert!(matches!(
        macro_tokens(&stores, "down"),
        [
            Token::Char {
                ch: 'y',
                cat: Catcode::Other
            },
            Token::Cs(_)
        ]
    ));
    assert!(matches!(
        macro_tokens(&stores, "active"),
        [Token::Char {
            ch: 'X',
            cat: Catcode::Active
        }]
    ));
    assert!(matches!(
        macro_tokens(&stores, "zero"),
        [Token::Char { ch: '@', .. }]
    ));
}

#[test]
fn show_dispatch_selects_activities_box_meaning_or_value_without_mode_dependence() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\def\shown{expanded}\show\shown\count0=17\showthe\count0\setbox0=\hbox{}\showbox0\end",
    );
    run_to_end(&mut control, &mut stores);
    let output = terminal_text(&stores);
    // §296's `print_meaning` breaks the line after a macro's `:`, so the
    // replacement text starts its own line under `\show` (but not under
    // `\meaning`, which runs the same routine at §471's `new_string`
    // selector, where `print_ln` does nothing).
    assert!(output.contains("> \\shown=macro:\n->expanded."), "{output}");
    assert!(output.contains("> 17."), "{output}");
    assert!(output.contains("> \\box0="), "{output}");
}

#[test]
fn show_uses_print_nl_at_closed_terminal_and_log_selector_boundaries() {
    // TeX82 §§62/1294: `print_nl("> ")` emits no leading newline when the
    // selected terminal/log line is already closed. Exercise every §75
    // interaction selector; `\newlinechar` must not turn this line transition
    // into literal diagnostic-text rewriting.
    for mode in [
        tex_state::InteractionMode::Batch,
        tex_state::InteractionMode::Nonstop,
        tex_state::InteractionMode::Scroll,
        tex_state::InteractionMode::ErrorStop,
    ] {
        let mut stores = Universe::new_with_plain_catcodes();
        stores.set_interaction_mode(mode);
        stores.set_int_param(IntParam::NEWLINE_CHAR, 10);
        if mode == tex_state::InteractionMode::ErrorStop {
            stores
                .world_mut()
                .push_memory_terminal_line("s")
                .expect("memory terminal accepts the show response");
        }
        stores.printer().print("\\show\\errorstopmode").print_ln();
        let mut control = CanonicalMainControl::tex82_initex(&mut stores);
        stores.set_interaction_mode(mode);
        register_source(&mut control, br"\show\errorstopmode\end");
        run_to_end(&mut control, &mut stores);

        let terminal = pending_sink_text(&stores, true);
        let log = pending_sink_text(&stores, false);
        let expected = "\\show\\errorstopmode\n> \\errorstopmode=\\errorstopmode.";
        if mode == tex_state::InteractionMode::Batch {
            assert_eq!(terminal, "", "batch mode wrote terminal records");
        } else {
            assert!(
                terminal.starts_with(expected),
                "{mode:?} terminal inserted output before the show line: {terminal:?}"
            );
        }
        assert!(
            log.starts_with(expected),
            "{mode:?} log inserted output before the show line: {log:?}"
        );
    }
}

#[test]
fn errorstop_show_reports_live_source_context_before_prompting_and_resumes() {
    // TeX82 §§82/1293: every show common ending calls `error`, and `error`
    // shows the still-live input cursor before asking for terminal advice.
    let mut stores = Universe::new_with_plain_catcodes();
    stores
        .world_mut()
        .push_memory_terminal_line("s")
        .expect("memory terminal accepts the show response");
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(&mut control, br"\show\errorstopmode\count0=23\end");

    run_to_end(&mut control, &mut stores);

    let output = terminal_text(&stores);
    assert!(
        output.contains("l.1 \\show\\errorstopmode\n                       \\count0=23\\end"),
        "{output:?}"
    );
    assert!(
        output.find("l.1 \\show\\errorstopmode").expect("context")
            < output.find("? ").expect("prompt"),
        "{output:?}"
    );
    assert_eq!(stores.count(0), 23, "show leaves the following input live");
    assert_eq!(
        stores.world().error_channel().error_count(),
        0,
        "interactive show does not enter the scrolled error count"
    );
}

#[test]
fn error_stop_deletes_requested_tokens_before_retry() {
    // TeX82 §§84--85: a one- or two-digit response consumes that many
    // unexpanded tokens, displays the resulting context, and prompts again.
    let mut stores = Universe::new_with_plain_catcodes();
    stores
        .world_mut()
        .push_memory_terminal_line("2")
        .expect("deletion response queues");
    stores
        .world_mut()
        .push_memory_terminal_line("")
        .expect("retry response queues");
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(&mut control, br"\show\errorstopmode ab\count0=17\end");

    run_to_end(&mut control, &mut stores);

    assert_eq!(
        stores.count(0),
        17,
        "only the two ignored letters disappear"
    );
    let terminal = terminal_text(&stores);
    assert_eq!(terminal.matches("? ").count(), 2, "{terminal:?}");
}

#[test]
fn error_stop_inserts_replacement_line_before_suspended_input_once() {
    // TeX82 §87 opens the typed replacement as a new terminal source level;
    // it retires once, then the exact suspended source resumes underneath it.
    let mut stores = Universe::new_with_plain_catcodes();
    stores
        .world_mut()
        .push_memory_terminal_line("I")
        .expect("insertion response queues");
    stores
        .world_mut()
        .push_memory_terminal_line("\\count0=17")
        .expect("replacement line queues");
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\show\errorstopmode\advance\count1 by 23\end",
    );

    run_to_end(&mut control, &mut stores);

    assert_eq!(stores.count(0), 17);
    assert_eq!(stores.count(1), 23);
    let log = pending_sink_text(&stores, false);
    assert_eq!(log.matches("\\count0=17").count(), 1, "{log:?}");
    assert!(log.contains("insert> \\count0=17\n"), "{log:?}");
}

#[test]
fn display_content_preserves_future_multiple_leading_newlines() {
    // The structured scanner never produces this malformed/future content.
    // If that contract expands, replay must still pass the content verbatim
    // to §62 rather than broadly deleting payload newlines.
    let mut stores = Universe::new_with_plain_catcodes();
    stores.printer().print("closed").print_ln();

    print_display_content(&mut stores, "\n\nfuture");

    assert_eq!(pending_sink_text(&stores, true), "closed\n\n\nfuture");
    assert_eq!(pending_sink_text(&stores, false), "closed\n\n\nfuture");
}

#[test]
fn consecutive_shows_and_following_error_preserve_only_canonical_separators() {
    // TeX82 §§82/90/1293 leave one blank separator after each noninteractive
    // show completion. The following `print_nl` must not add another.
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\nonstopmode\show\errorstopmode\show\scrollmode\undefined\end",
    );
    run_to_end(&mut control, &mut stores);

    let output = terminal_text(&stores);
    // §82's `show_context` sits between each report's own line and the
    // separator, so the separator is what these check, not adjacency.
    assert!(
        output.contains("> \\errorstopmode=\\errorstopmode."),
        "{output:?}"
    );
    assert!(
        output.contains("> \\scrollmode=\\scrollmode."),
        "{output:?}"
    );
    assert!(
        output.contains("\\show\\scrollmode\\undefined\\end\n\n> \\scrollmode"),
        "{output:?}"
    );
    assert!(
        output.contains("\\undefined\\end\n\n! Undefined control sequence."),
        "{output:?}"
    );
    assert!(!output.contains("\n\n\n> "), "{output:?}");
}

#[test]
fn showlists_is_a_diagnostic_without_a_canonical_effect_event() {
    // TeX82 §1293 writes `show_activities` through the diagnostic printer.
    // The schema-v1 command stream has no detached effect for that report;
    // only actual engine effects such as messages, writes, and termination
    // are published as effect observations.
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(&mut control, br"\showlists\end");
    let mut observations = ObservationRecorder::default();
    loop {
        match control
            .step_with_observer(&mut stores, &mut observations)
            .expect("showlists executes")
        {
            MainControlStep::End | MainControlStep::EndOfInput => break,
            MainControlStep::Continue => {}
        }
    }

    assert!(terminal_text(&stores).contains("### vertical mode"));
    assert!(!observations.0.iter().any(|observation| {
        matches!(
            observation,
            CommandObservation::Effect(effect) if effect.kind == "activities"
        )
    }));
}

#[test]
fn show_meaning_reads_raw_token_and_formats_each_macro_meaning_kind() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\def\macro{body}\show\undefined\show\relax\show\macro\end",
    );
    run_to_end(&mut control, &mut stores);
    let output = terminal_text(&stores);
    assert!(output.contains("> \\undefined=undefined."), "{output}");
    assert!(output.contains("> \\relax=\\relax."), "{output}");
    assert!(output.contains("> \\macro=macro:\n->body."), "{output}");
}

#[test]
fn show_meaning_prints_all_named_glue_and_register_symbols() {
    // TeX82 §§224, 230, 296, and 1297: `print_cmd_chr` retains the control
    // sequence spelling for named glue parameters, while `print_spec` uses
    // `pt` for ordinary glue and `mu` for math glue. e-TeX preserves these
    // command codes and only widens the register-number scanner.
    const GLUE_PARAMETERS: [&str; 15] = [
        "lineskip",
        "baselineskip",
        "parskip",
        "abovedisplayskip",
        "belowdisplayskip",
        "abovedisplayshortskip",
        "belowdisplayshortskip",
        "leftskip",
        "rightskip",
        "topskip",
        "splittopskip",
        "tabskip",
        "spaceskip",
        "xspaceskip",
        "parfillskip",
    ];
    const MU_GLUE_PARAMETERS: [&str; 3] = ["thinmuskip", "medmuskip", "thickmuskip"];
    const SOURCE: &[u8] = br"\nonstopmode
        \lineskip=1pt plus 2pt minus 3pt
        \baselineskip=1pt plus 2pt minus 3pt
        \parskip=1pt plus 2pt minus 3pt
        \abovedisplayskip=1pt plus 2pt minus 3pt
        \belowdisplayskip=1pt plus 2pt minus 3pt
        \abovedisplayshortskip=1pt plus 2pt minus 3pt
        \belowdisplayshortskip=1pt plus 2pt minus 3pt
        \leftskip=1pt plus 2pt minus 3pt
        \rightskip=1pt plus 2pt minus 3pt
        \topskip=1pt plus 2pt minus 3pt
        \splittopskip=1pt plus 2pt minus 3pt
        \tabskip=1pt plus 2pt minus 3pt
        \spaceskip=1pt plus 2pt minus 3pt
        \xspaceskip=1pt plus 2pt minus 3pt
        \parfillskip=1pt plus 2pt minus 3pt
        \thinmuskip=4mu plus 5mu minus 6mu
        \medmuskip=4mu plus 5mu minus 6mu
        \thickmuskip=4mu plus 5mu minus 6mu
        \skip0=7pt plus 8pt minus 9pt
        \muskip0=10mu plus 11mu minus 12mu
        \expandafter\skipdef\csname skip0\endcsname=0
        \expandafter\muskipdef\csname muskip0\endcsname=0
        \count255=1
        \show\lineskip\show\baselineskip\show\parskip
        \show\abovedisplayskip\show\belowdisplayskip
        \show\abovedisplayshortskip\show\belowdisplayshortskip
        \show\leftskip\show\rightskip\show\topskip\show\splittopskip
        \show\tabskip\show\spaceskip\show\xspaceskip\show\parfillskip
        \show\thinmuskip\show\medmuskip\show\thickmuskip
        \expandafter\show\csname skip0\endcsname
        \expandafter\show\csname muskip0\endcsname
        \showthe\lineskip\showthe\baselineskip\showthe\parskip
        \showthe\abovedisplayskip\showthe\belowdisplayskip
        \showthe\abovedisplayshortskip\showthe\belowdisplayshortskip
        \showthe\leftskip\showthe\rightskip\showthe\topskip\showthe\splittopskip
        \showthe\tabskip\showthe\spaceskip\showthe\xspaceskip\showthe\parfillskip
        \showthe\thinmuskip\showthe\medmuskip\showthe\thickmuskip
        \showthe\skip0\showthe\muskip0\end";

    for extended in [false, true] {
        let mut stores = Universe::new_with_plain_catcodes();
        let mut control = if extended {
            canonical_etex_initex(&mut stores)
        } else {
            CanonicalMainControl::tex82_initex(&mut stores)
        };
        register_source(&mut control, SOURCE);

        // Stop immediately before the first diagnostic, after the interaction
        // command, assignments, and symbolic register aliases have committed.
        while stores.count(255) == 0 {
            assert_eq!(
                control.step(&mut stores).expect("setup command executes"),
                MainControlStep::Continue
            );
        }
        let glue_parameters = (0..18)
            .map(|index| stores.glue_param(GlueParam::new(index)))
            .collect::<Vec<_>>();
        let skip = stores.skip(0);
        let muskip = stores.muskip(0);

        run_to_end(&mut control, &mut stores);
        let output = terminal_text(&stores);

        for name in GLUE_PARAMETERS {
            assert!(
                output.contains(&format!("> \\{name}=\\{name}.")),
                "profile extended={extended} omitted {name} meaning: {output}"
            );
            assert!(
                output.contains("> 1.0pt plus 2.0pt minus 3.0pt."),
                "profile extended={extended} omitted ordinary-glue units: {output}"
            );
        }
        for name in MU_GLUE_PARAMETERS {
            assert!(
                output.contains(&format!("> \\{name}=\\{name}.")),
                "profile extended={extended} omitted {name} meaning: {output}"
            );
            assert!(
                output.contains("> 4.0mu plus 5.0mu minus 6.0mu."),
                "profile extended={extended} omitted math-glue units: {output}"
            );
        }
        assert!(output.contains("> \\skip0=\\skip0."), "{output}");
        assert!(output.contains("> \\muskip0=\\muskip0."), "{output}");
        assert!(
            output.contains("> 7.0pt plus 8.0pt minus 9.0pt."),
            "{output}"
        );
        assert!(
            output.contains("> 10.0mu plus 11.0mu minus 12.0mu."),
            "{output}"
        );

        assert_eq!(
            (0..18)
                .map(|index| stores.glue_param(GlueParam::new(index)))
                .collect::<Vec<_>>(),
            glue_parameters,
            "profile extended={extended} changed a parameter bank"
        );
        assert_eq!(stores.skip(0), skip, "profile extended={extended}");
        assert_eq!(stores.muskip(0), muskip, "profile extended={extended}");
    }
}

#[test]
fn showbox_scans_register_and_distinguishes_void_from_box_contents() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\showboxbreadth=10\showboxdepth=10\setbox0=\hbox{\kern1pt}\setbox255=\hbox{}\showbox0\showbox255\showbox1\end",
    );
    run_to_end(&mut control, &mut stores);
    let output = terminal_text(&stores);
    assert!(output.contains("> \\box0="), "{output}");
    assert!(output.contains("\\kern 1.0"), "{output}");
    assert!(output.contains("> \\box255="), "{output}");
    assert!(output.contains("> \\box1="), "{output}");
    assert!(output.contains("\nvoid"), "{output}");
}

#[test]
fn etex_showbox_invalid_register_loaded_format_checkpoint_retry_recovers_to_zero() {
    // e-TeX 2.6 etex.ch [49.1296] replaces TeX82's `scan_eight_bit_int`
    // with `scan_register_num`, whose restricted scan diagnoses -1, recovers
    // it to zero, and leaves the following token for the next command.
    let mut initex_stores = crate::test_harness::universe_with_plain_catcodes();
    let _ = canonical_etex_initex(&mut initex_stores);
    let format = initex_stores
        .dump_format()
        .expect("dump extended e-TeX format");
    let mut stores = Universe::from_format(tex_state::World::memory(), &format)
        .expect("restore extended e-TeX format");
    let mut control = CanonicalMainControl::with_profile(CommandProfile::ETEX26);
    control
        .set_fuel_limit(1_000)
        .expect("bounded canonical fuel");
    register_source(&mut control, br"\showbox-1\count0=23\end");
    let checkpoint = control
        .capture_checkpoint(
            crate::EngineBoundary::OuterParagraphEnd,
            &mut stores,
            crate::ExecutionBudgetCounters::default(),
        )
        .expect("showbox checkpoints");

    assert_eq!(
        control
            .step(&mut stores)
            .expect("invalid showbox register recovers"),
        MainControlStep::Continue
    );
    assert_eq!(stores.count(0), 0, "following assignment remains unread");
    let first_hash = stores.testing_state_hash();
    let first_output = terminal_text(&stores);
    assert!(
        first_output.contains("Bad register code (-1)"),
        "{first_output}"
    );
    assert!(first_output.contains("> \\box0="), "{first_output}");

    control
        .restore_checkpoint(&checkpoint, &mut stores)
        .expect("showbox state restores");
    assert_eq!(
        control
            .step(&mut stores)
            .expect("invalid showbox register retries identically"),
        MainControlStep::Continue
    );
    assert_eq!(stores.testing_state_hash(), first_hash);
    assert_eq!(terminal_text(&stores), first_output);

    run_to_end(&mut control, &mut stores);
    assert_eq!(
        stores.count(0),
        23,
        "following token executes after recovery"
    );
    assert!(control.fuel_burned() < 1_000);
}

#[test]
fn showthe_uses_the_toks_for_each_internal_value_family_and_releases_output() {
    // TeX82 §§262/1297: the font identifier becomes a token shown through
    // `print_cs`, whose control-word delimiter precedes the display period.
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let nullfont = stores.intern("nullfont");
    stores.set_font_identifier_symbol(tex_state::font::NULL_FONT, nullfont);
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\count0=17\skip0=1pt plus 2fil\toks0={abc}\showthe\count0\showthe\skip0\showthe\font\showthe\toks0\end",
    );
    run_to_end(&mut control, &mut stores);
    let output = terminal_text(&stores);
    assert!(output.contains("> 17."), "{output}");
    assert!(output.contains("> 1.0pt plus 2.0fil."), "{output}");
    assert!(output.contains("> \\nullfont ."), "{output}");
    assert!(output.contains("> abc."), "{output}");
}

#[test]
fn show_completion_routes_transcript_and_adjusts_error_count_by_interaction() {
    let mut stores = Universe::new_with_plain_catcodes();
    stores.set_interaction_mode(tex_state::InteractionMode::Nonstop);
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(&mut control, br"\showthe\count0\count1=9\end");
    run_to_end(&mut control, &mut stores);
    assert!(terminal_text(&stores).contains("> 0."));
    assert_eq!(stores.count(1), 9, "show completion resumes execution");
}

#[test]
fn final_cleanup_retires_inputs_reports_open_state_and_selects_end_or_dump() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(&mut control, br"\def\stop{\end}\stop");
    let mut observations = ObservationRecorder::default();
    loop {
        if matches!(
            control
                .step_with_observer(&mut stores, &mut observations)
                .expect("final cleanup"),
            MainControlStep::End | MainControlStep::EndOfInput
        ) {
            break;
        }
    }
    assert!(observations.0.iter().any(|observation| matches!(
        observation,
        CommandObservation::Input(input)
            if input.transition == tex_command::InputTransition::Retire
    )));
    assert!(observations.0.iter().any(|observation| matches!(
        observation,
        CommandObservation::Effect(effect) if effect.kind == "terminate"
    )));
}

#[test]
fn end_and_dump_run_profile_specific_cleanup_in_observable_order() {
    // TeX82 §§1330--1337 enter the selected profile before main control,
    // retire live input during `final_cleanup`, close numbered streams, and
    // only then expose termination.  A successful INITEX `\dump` additionally
    // defers its announcement until the host confirms publication.
    for profile in [CommandProfile::TEX82, CommandProfile::ETEX26] {
        for dump in [false, true] {
            let mut stores = Universe::new_with_plain_catcodes();
            let mut control = if profile == CommandProfile::ETEX26 {
                canonical_etex_initex(&mut stores)
            } else {
                CanonicalMainControl::tex82_initex(&mut stores)
            };
            control.begin_job(&mut stores, "lifecycle.tex");
            register_source(
                &mut control,
                if dump {
                    br"\immediate\openout3=cleanup\dump"
                } else {
                    br"\immediate\openout3=cleanup\end"
                },
            );

            let mut observations = ObservationRecorder::default();
            run_to_end_observed(&mut control, &mut stores, &mut observations);
            let ordered: Vec<_> = observations
                .0
                .iter()
                .filter_map(|observation| match observation {
                    CommandObservation::Input(input)
                        if input.transition == InputTransition::Retire =>
                    {
                        Some("retire")
                    }
                    CommandObservation::Effect(effect) if effect.kind == "close" => Some("close"),
                    CommandObservation::Effect(effect) if effect.kind == "terminate" => {
                        Some("terminate")
                    }
                    _ => None,
                })
                .collect();
            let close = ordered
                .iter()
                .position(|event| *event == "close")
                .expect("cleanup closes the live numbered stream");
            assert!(
                ordered[..close].iter().all(|event| *event == "retire"),
                "every live input level retires before stream cleanup: {ordered:?}"
            );
            assert!(!ordered[..close].is_empty());
            assert_eq!(&ordered[close..], ["close", "terminate"]);

            let terminal = terminal_text(&stores);
            assert_eq!(
                terminal.contains("entering extended mode"),
                profile == CommandProfile::ETEX26
            );
            assert_eq!(control.dumped_format(), dump);
            assert!(!terminal.contains("Beginning to dump on file"));
            if dump {
                let mut receipt = control.format_dump_receipt().expect("dump receipt").clone();
                crate::confirm_format_dump_publication(&mut stores, &mut receipt, "lifecycle.fmt");
                assert!(terminal_text(&stores).contains("Beginning to dump on file lifecycle.fmt"));
            }
        }
    }
}

#[test]
fn initex_dump_owns_identifier_but_waits_for_publication_receipt() {
    let mut stores = Universe::new_with_plain_catcodes();
    stores.set_int_param(IntParam::YEAR, 2026);
    stores.set_int_param(IntParam::MONTH, 7);
    stores.set_int_param(IntParam::DAY, 9);
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    control
        .capabilities_mut()
        .set_startup_job_name("bounded-dump.tex");
    register_source(&mut control, br"\dump");

    run_to_end(&mut control, &mut stores);

    assert!(control.dumped_format());
    assert_eq!(terminal_text(&stores), "");
    let mut receipt = control.format_dump_receipt().expect("dump receipt").clone();
    assert_eq!(receipt.format_ident.format_name, "bounded-dump");
    crate::confirm_format_dump_publication(&mut stores, &mut receipt, "alternate-name.fmt");
    assert_eq!(
        terminal_text(&stores),
        "Beginning to dump on file alternate-name.fmt\n (preloaded format=bounded-dump 2026.7.9)"
    );
}

#[test]
fn valign_cell_endv_closes_an_open_paragraph_before_fin_col() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\catcode`\#=6 \catcode`\&=4
           \setbox0=\hbox{\valign{#\cr x\cr}}
           \ifhmode\count0=2\else\count0=1\fi
           \end",
    );

    run_to_end(&mut control, &mut stores);

    // TeX82 §1131 runs `end_graf` before `fin_col`. The paragraph opened by
    // `x` is therefore closed before the valign cell, row, alignment, and
    // enclosing hbox levels are packaged in order.
    assert_eq!(stores.count(0), 1);
    assert_eq!(control.current_mode(), Mode::Vertical);
}

#[test]
fn final_cleanup_reports_nested_condition_kinds_lines_and_order_exactly() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(&mut control, b"\\iftrue\n\\ifcase0\n\\ifnum1=1\n\\end");

    run_to_end(&mut control, &mut stores);

    assert_eq!(
        terminal_text(&stores),
        "(\\end occurred when \\ifnum on line 3 was incomplete)\
\n(\\end occurred when \\ifcase on line 2 was incomplete)\
\n(\\end occurred when \\iftrue on line 1 was incomplete)"
    );
}

/// Collects every `\setlanguage` whatsit inside box register zero.
fn language_whatsits(stores: &Universe) -> Vec<(u8, u8, u8)> {
    let outer = stores.box_reg(0).expect("box 0 holds the constructed hbox");
    let Some(Node::HList(boxed)) = stores.nodes(outer).first().map(|node| node.to_owned()) else {
        panic!("box 0 holds an hlist");
    };
    stores
        .nodes(boxed.children)
        .iter()
        .filter_map(|node| match node.to_owned() {
            Node::Whatsit(tex_state::node::Whatsit::Language {
                language,
                left_hyphen_min,
                right_hyphen_min,
            }) => Some((language, left_hyphen_min, right_hyphen_min)),
            _ => None,
        })
        .collect()
}

#[test]
fn language_normalization_and_same_language_append_boundaries_match_tex82() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    // TeX82 §1377 normalizes `cur_val` in both out-of-range directions to
    // language zero, and §1091's `norm_min` clamps each hyphen minimum into
    // `1..=63`. The exact 255/256 boundary proves that 255 is retained while
    // the first value above it joins negative values at language zero. The
    // repeated `7` proves §1377 appends unconditionally: only §1376's
    // `fix_language` is guarded by `l<>clang`.
    register_source(
        &mut control,
        br"\lefthyphenmin=2 \righthyphenmin=99 \setbox0=\hbox{\setlanguage7\setlanguage7\setlanguage255\setlanguage256\setlanguage-1}\end",
    );
    run_to_end(&mut control, &mut stores);
    assert_eq!(
        language_whatsits(&stores),
        vec![(7, 2, 63), (7, 2, 63), (255, 2, 63), (0, 2, 63), (0, 2, 63)]
    );
}

#[test]
fn setlanguage_illegal_mode_recovers_without_scan_or_append() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    // TeX82 §1377 tests `abs(mode)<>hmode` before `new_whatsit` and before
    // `scan_int`, so the operand is never consumed: the following assignment
    // is the very next command main control sees.
    register_source(
        &mut control,
        br"\setbox0=\vbox{\setlanguage\global\count0=5}\end",
    );
    run_to_end(&mut control, &mut stores);
    assert_eq!(stores.count(0), 5);
    let text = terminal_text(&stores);
    assert!(
        text.contains("You can't use `\\setlanguage' in internal vertical mode"),
        "{text}"
    );
    let outer = stores.box_reg(0).expect("box 0 holds the constructed vbox");
    let Some(Node::VList(boxed)) = stores.nodes(outer).first().map(|node| node.to_owned()) else {
        panic!("box 0 holds a vlist");
    };
    assert!(
        !stores
            .nodes(boxed.children)
            .iter()
            .any(|node| matches!(node.to_owned(), Node::Whatsit(_))),
        "no whatsit is appended when the mode test fails"
    );
}

/// TeX82 §796/§798's spanned-column packaging, at and just past its bound.
///
/// `#&&#` is a periodic preamble, so a body entry can span arbitrarily many
/// columns. §796 sets `n:=min_quarterword`, "this represents a span count of
/// 1", and §798 then runs `repeat incr(n); q:=link(link(q)); until q=cur_align`
/// over the spanned columns, so `n` is the number of `\span` delimiters.
/// §110's `max_quarterword` is 255.
fn spanning_alignment_source(spans: &str) -> Vec<u8> {
    format!(
        concat!(
            r"\catcode`{{=1 \catcode`}}=2 \catcode`\#=6 \catcode`\&=4",
            "\n",
            r"\def\a{{\span}}\def\b{{\a\a}}\def\c{{\b\b}}\def\d{{\c\c}}",
            "\n",
            r"\def\e{{\d\d}}\def\f{{\e\e}}\def\g{{\f\f}}\def\h{{\g\g}}\def\i{{\h\h}}",
            "\n",
            r"\setbox0=\vbox{{\halign{{#&&#\cr\relax{spans}\relax\cr}}}}",
            "\n",
            r"\global\count0=1\end",
            "\n",
        ),
        spans = spans
    )
    .into_bytes()
}

#[test]
fn two_hundred_fifty_five_span_steps_stay_within_section_798s_bound() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    // 128+64+32+16+8+4+2+1 = 255 `\span` delimiters, so §798's `n` is exactly
    // `max_quarterword` and the guard `n>max_quarterword` does not fire.
    register_source(
        &mut control,
        &spanning_alignment_source(r"\h\g\f\e\d\c\b\a"),
    );

    run_to_end(&mut control, &mut stores);

    assert_eq!(control.fatal_error(), None);
    assert_eq!(stores.count(0), 1, "the job ran on to \\global\\count0=1");
}

#[test]
fn two_hundred_fifty_six_span_steps_succumb_to_section_798s_confusion() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    // `\i` is 2^8 = 256 `\span` delimiters, so §798's `n` is 256 and
    // `if n>max_quarterword then confusion("256 spans")` fires.
    register_source(&mut control, &spanning_alignment_source(r"\i"));

    run_to_end(&mut control, &mut stores);

    assert_eq!(
        control.fatal_error(),
        Some(FatalError::confusion("256 spans"))
    );
    // §93 `succumb` calls §81 `jump_out`, so nothing after the alignment runs.
    assert_eq!(stores.count(0), 0);
}

#[test]
fn a_succumbed_session_stays_terminal_without_delivering_another_command() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(&mut control, &spanning_alignment_source(r"\i"));

    run_to_end(&mut control, &mut stores);
    let fatal = control.fatal_error();

    for _ in 0..4 {
        assert_eq!(
            control
                .step(&mut stores)
                .expect("a terminal session reports"),
            MainControlStep::End,
        );
    }
    assert_eq!(control.fatal_error(), fatal);
    assert_eq!(stores.count(0), 0);
}

#[test]
fn succumbing_commits_fatal_diagnostic_then_engine_termination() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(&mut control, &spanning_alignment_source(r"\i"));

    let mut observations = ObservationRecorder::default();
    loop {
        match control
            .step_with_observer(&mut stores, &mut observations)
            .expect("a fatal error is a terminal state, never an Err")
        {
            MainControlStep::End | MainControlStep::EndOfInput => break,
            MainControlStep::Continue => {}
        }
    }

    let fatal = FatalError::confusion("256 spans");
    assert_eq!(control.fatal_error(), Some(fatal));
    assert!(matches!(
        observations.0.as_slice(),
        [.., CommandObservation::Diagnostic(record), CommandObservation::Effect(effect)]
            if *record == fatal.record()
                && effect.kind == "terminate"
                && effect.detail == "engine\0"
    ));
}

#[test]
fn setbox_scope_is_globaldefs_adjusted_before_the_box_is_scanned() {
    // TeX82 §1214's `<Adjust for the setting of \globaldefs>` runs inside
    // `prefixed_command`, so a positive `\globaldefs` makes an unprefixed
    // `\setbox` global and a negative one makes `\global\setbox` local.
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\globaldefs=1 {\setbox0=\hbox{\kern1pt}}\globaldefs=-1 {\global\setbox1=\hbox{\kern1pt}}\globaldefs=0 \end",
    );
    run_to_end(&mut control, &mut stores);

    assert!(stores.box_reg(0).is_some(), "positive globaldefs is global");
    assert!(stores.box_reg(1).is_none(), "negative globaldefs is local");
}

#[test]
fn effective_scope_is_shared_by_provisional_and_committed_meaning_mutations() {
    // TeX82 §§1211/1214 resolve the assignment scope before §1224/§1257
    // install their provisional meanings. §§277-279 then expose that same
    // resolved choice for both provisional and final definitions.
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"{\globaldefs=1\chardef\forcedchar=65\countdef\forcedregister=2}{\globaldefs=-1\global\chardef\localchar=66\global\countdef\localregister=3}\globaldefs=0\end",
    );
    let mut observations = ObservationRecorder::default();
    loop {
        if matches!(
            control
                .step_with_observer(&mut stores, &mut observations)
                .expect("scope matrix executes"),
            MainControlStep::End | MainControlStep::EndOfInput
        ) {
            break;
        }
    }

    for (name, expected_global) in [
        ("forcedchar", true),
        ("forcedregister", true),
        ("localchar", false),
        ("localregister", false),
    ] {
        let scopes: Vec<_> = observations
            .0
            .iter()
            .filter_map(|observation| match observation {
                CommandObservation::Mutation(record) if record.key.as_deref() == Some(name) => {
                    Some(record.global)
                }
                _ => None,
            })
            .collect();
        assert!(!scopes.is_empty(), "{name} has an observed mutation");
        assert!(
            scopes.iter().all(|scope| *scope == expected_global),
            "{name} used one effective scope across provisional and final mutations: {scopes:?}"
        );
    }

    for name in ["forcedchar", "forcedregister"] {
        assert_ne!(
            stores.meaning(stores.symbol(name).expect("symbol was scanned")),
            Meaning::Undefined,
            "{name} survived its group"
        );
    }
    for name in ["localchar", "localregister"] {
        assert_eq!(
            stores.meaning(stores.symbol(name).expect("symbol was scanned")),
            Meaning::Undefined,
            "{name} was restored at group end"
        );
    }
}

#[test]
fn every_non_eqtb_assignment_family_fires_afterassignment_once() {
    // TeX82 §1210 includes all ten families below in prefixed_command, and
    // §1269 reaches `done` after each completed assignment. The saved token
    // must enter through ordinary §325 back_input exactly once.
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\def\mark{\global\advance\count0 by1}\afterassignment\mark\nullfont\afterassignment\mark\textfont0=\nullfont\afterassignment\mark\setbox0=\hbox{}\afterassignment\mark\prevdepth=0pt x\afterassignment\mark\spacefactor=1000\par\afterassignment\mark\prevgraf=0\afterassignment\mark\pagegoal=1pt\afterassignment\mark\deadcycles=0\afterassignment\mark\hyphenation{word}\afterassignment\mark\nonstopmode\end",
    );
    run_to_end(&mut control, &mut stores);

    assert_eq!(stores.count(0), 10);
    assert_eq!(stores.take_afterassignment(), None);
}

#[test]
fn openin_supplies_the_default_tex_extension() {
    // TeX82 §1275's `if cur_ext="" then cur_ext:=".tex"; pack_cur_name`.
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    control.capabilities_mut().register_input(
        "child.tex",
        SourceRegistration::new(RegisteredSourceKind::World, Arc::<[u8]>::from(&b"body"[..])),
    );
    register_source(&mut control, br"\openin1=child \read1 to \line\end");
    run_to_end(&mut control, &mut stores);

    let line = stores.intern("line");
    let replacement = stores
        .macro_meaning(line)
        .expect("read defined its target")
        .replacement_text();
    let text: String = stores
        .tokens(replacement)
        .iter()
        .filter_map(|token| match token {
            Token::Char { ch, .. } => Some(*ch),
            _ => None,
        })
        .collect();
    // TeX82 §240's `\endlinechar` is appended to the line, but §348's
    // ⟨Finish line, emit a space⟩ tokenizes it as `cur_cmd:=spacer;
    // cur_chr:=" "` -- the trailing token is a space, never the raw byte.
    assert_eq!(text, "body ");
}

#[test]
fn fontdimen_reports_an_unusable_parameter_number_and_leaves_the_font_alone() {
    // TeX82 §578 resolves `n<=0` to the scratch `fmem_ptr`; §579 reports it
    // and §1253 still consumes `=<dimen>`, so the next command runs.
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(&mut control, br"\fontdimen0\nullfont=1pt \count0=1\end");
    run_to_end(&mut control, &mut stores);

    assert_eq!(stores.count(0), 1);
    assert_eq!(
        stores.hyphen_positions_for_language(0, "ab", 0, 0),
        Vec::<usize>::new(),
        "§963 diagnoses the duplicate before replacing it with a2b"
    );
    let output = terminal_text(&stores);
    assert!(
        output.contains("! Font \\nullfont has only 7 fontdimen parameters."),
        "{output}"
    );
}

#[test]
fn fontdimen_identifier_and_bound_recovery_matrix_is_exact() {
    // TeX82 §§577--579/1253: an invalid identifier is backed up and replaced
    // by nullfont; nonpositive and unavailable parameter numbers all select
    // the scratch cell, diagnose, consume the dimension, and do not mutate it.
    for (source, missing_identifier, parameter_errors, trailing_count, final_len) in [
        (
            br"\fontdimen1\relax=1pt \count0=11\end".as_slice(),
            1,
            0,
            11,
            7,
        ),
        (
            br"\fontdimen-1\nullfont=1pt \count0=12\end".as_slice(),
            0,
            1,
            12,
            7,
        ),
        (
            br"\fontdimen0\nullfont=1pt \count0=13\end".as_slice(),
            0,
            1,
            13,
            7,
        ),
        // §578 permits growth on the newest font, including nullfont before
        // another font is loaded; 8 is therefore the adjacent valid bound.
        (
            br"\fontdimen8\nullfont=1pt \count0=14\end".as_slice(),
            0,
            0,
            14,
            8,
        ),
    ] {
        let mut stores = crate::test_harness::universe_with_plain_catcodes();
        let original: Vec<_> = (1..=7)
            .map(|number| stores.font_parameter(tex_state::font::NULL_FONT, number))
            .collect();
        let mut control = CanonicalMainControl::tex82_initex(&mut stores);
        register_source(&mut control, source);
        run_to_end(&mut control, &mut stores);

        assert_eq!(stores.count(0), trailing_count, "{source:?}");
        assert_eq!(
            stores.font_parameter_count(tex_state::font::NULL_FONT),
            final_len
        );
        assert_eq!(
            (1..=7)
                .map(|number| stores.font_parameter(tex_state::font::NULL_FONT, number))
                .collect::<Vec<_>>(),
            original,
            "{source:?}"
        );
        if final_len == 8 {
            assert_eq!(
                stores.font_parameter(tex_state::font::NULL_FONT, 8),
                Scaled::from_raw(Scaled::UNITY)
            );
        }
        let output = terminal_text(&stores);
        assert_eq!(
            output.matches("! Missing font identifier.").count(),
            missing_identifier,
            "{output}"
        );
        assert_eq!(
            output
                .matches("! Font \\nullfont has only 7 fontdimen parameters.")
                .count(),
            parameter_errors,
            "{output}"
        );
    }
}

#[test]
fn font_definition_size_boundaries_use_exact_replacements() {
    // TeX82 §§1258--1259 accept scaled 1..32768 and at sizes whose scaled
    // value is 1..(2048pt-1sp); each adjacent invalid value becomes 1000 or
    // 10pt respectively before §1257 interns the font.
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_cmr10_as(&mut control, &mut stores, "cmr10.tfm");
    register_source(
        &mut control,
        br"\font\slo=cmr10 scaled 1 \font\shi=cmr10 scaled 32768 \font\szero=cmr10 scaled 0 \font\sover=cmr10 scaled 32769 \font\alo=cmr10 at 0.00002pt \font\ahi=cmr10 at 2047.99998pt \font\azero=cmr10 at 0pt \font\aover=cmr10 at 2048pt \end",
    );
    run_to_end(&mut control, &mut stores);

    let size = |stores: &Universe, name: &str| match stores
        .meaning(stores.symbol(name).expect("font identifier was scanned"))
    {
        Meaning::Font(font) => stores.font(font).size().raw(),
        meaning => panic!("{name} has {meaning:?}"),
    };
    assert_eq!(size(&stores, "slo"), 655);
    assert_eq!(size(&stores, "shi"), 21_474_836);
    assert_eq!(size(&stores, "szero"), 655_360);
    assert_eq!(size(&stores, "sover"), 655_360);
    assert_eq!(size(&stores, "alo"), 1);
    assert_eq!(size(&stores, "ahi"), 134_217_727);
    assert_eq!(size(&stores, "azero"), 655_360);
    assert_eq!(size(&stores, "aover"), 655_360);
    let output = terminal_text(&stores);
    assert_eq!(
        output
            .matches("! Illegal magnification has been changed to 1000 (")
            .count(),
        2,
        "{output}"
    );
    assert_eq!(
        output.matches("! Improper `at' size (").count(),
        2,
        "{output}"
    );
}

#[test]
fn font_definition_identity_is_case_sensitive_and_tracks_newest_identifier() {
    // TeX82 §1257 compares the case-sensitive name and size when reusing a
    // font, then assigns font_id_text(f):=u even on the reuse path.
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_cmr10_as(&mut control, &mut stores, "cmr10.tfm");
    register_cmr10_as(&mut control, &mut stores, "CMR10.tfm");
    register_source(
        &mut control,
        br"\font\first=cmr10 \font\upper=CMR10 \font\newest=cmr10 \end",
    );
    run_to_end(&mut control, &mut stores);

    let font = |stores: &Universe, name: &str| match stores
        .meaning(stores.symbol(name).expect("font identifier was scanned"))
    {
        Meaning::Font(font) => font,
        meaning => panic!("{name} has {meaning:?}"),
    };
    let first = font(&stores, "first");
    let upper = font(&stores, "upper");
    let newest = font(&stores, "newest");
    assert_eq!(
        first, newest,
        "same case-sensitive name and size reuses the font"
    );
    assert_ne!(
        first, upper,
        "case-distinct names are distinct font identities"
    );
    assert_eq!(
        stores.font_identifier_symbol(first),
        stores.symbol("newest"),
        "the reused font retains the newest identifier"
    );
    assert_eq!(stores.font_identifier_symbol(upper), stores.symbol("upper"));
}

#[test]
fn arithmetic_overflow_reports_and_leaves_the_target_unchanged() {
    // TeX82 §1236 returns before `word_define` when `arith_error` is set.
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\count0=2000000000 \multiply\count0 by2 \count1=7 \divide\count1 by0 \count2=1\end",
    );
    run_to_end(&mut control, &mut stores);

    assert_eq!(stores.count(0), 2_000_000_000);
    assert_eq!(stores.count(1), 7);
    assert_eq!(stores.count(2), 1);
    let output = terminal_text(&stores);
    assert_eq!(
        output.matches("! Arithmetic overflow.").count(),
        2,
        "{output}"
    );
}

#[test]
fn invalid_arithmetic_target_recovers_and_fires_afterassignment() {
    // TeX82 §1236 consumes an invalid target, reports the error, and returns
    // through §1269's common path, which still replays `\afterassignment`.
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\prevdepth=2pt \def\mark{\global\count0=7}\afterassignment\mark\advance\prevdepth \count1=9\end",
    );
    run_to_end(&mut control, &mut stores);

    assert_eq!(stores.count(0), 7, "afterassignment token was replayed");
    assert_eq!(stores.count(1), 9, "execution continued after the error");
    assert_eq!(
        control.modes.current_list().prev_depth(),
        Some(Scaled::from_raw(2 * 65_536))
    );
    let output = terminal_text(&stores);
    assert!(
        output.contains("! You can't use `\\prevdepth' after \\advance."),
        "{output}"
    );
}

#[test]
fn invalid_arithmetic_targets_use_print_cmd_chr_and_commit_without_mutation() {
    // TeX82 §§298 and 1236 print the rejected command class, scan no operand,
    // and return through §1269 once. Prefix scope is therefore immaterial,
    // including both \globaldefs overrides.
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\def\mark{\global\advance\count0 by1}
           \afterassignment\mark\global\advance x
           \globaldefs=1
           \afterassignment\mark\multiply 7
           \globaldefs=-1
           \afterassignment\mark\global\divide\relax
           \globaldefs=0
           \count1=19\end",
    );
    let mut observations = ObservationRecorder::default();
    run_to_end_observed(&mut control, &mut stores, &mut observations);

    assert_eq!(
        stores.count(0),
        3,
        "each afterassignment fires exactly once"
    );
    assert_eq!(
        stores.take_afterassignment(),
        None,
        "pending slot is drained"
    );
    assert_eq!(stores.count(1), 19, "no rejected command scans an operand");
    assert!(
        observations
            .0
            .iter()
            .any(|event| matches!(event, CommandObservation::Mutation(_))),
        "observer exercised the surrounding valid assignments"
    );

    let output = terminal_text(&stores);
    let expected = [
        "! You can't use `the letter x' after \\advance.",
        "! You can't use `the character 7' after \\multiply.",
        "! You can't use `\\relax' after \\divide.",
    ];
    let positions = expected.map(|text| {
        assert_eq!(output.matches(text).count(), 1, "{text:?} in {output:?}");
        output.find(text).expect("diagnostic text")
    });
    assert!(
        positions.windows(2).all(|pair| pair[0] < pair[1]),
        "diagnostic order changed: {output:?}"
    );

    let mut isolated_stores = crate::test_harness::universe_with_plain_catcodes();
    let mut isolated = CanonicalMainControl::tex82_initex(&mut isolated_stores);
    register_source(&mut isolated, br"\advance x");
    let mut isolated_observations = ObservationRecorder::default();
    isolated
        .step_with_observer(&mut isolated_stores, &mut isolated_observations)
        .expect("observed invalid target recovers");
    assert!(
        !isolated_observations
            .0
            .iter()
            .any(|event| matches!(event, CommandObservation::Mutation(_))),
        "invalid target must not publish a mutation: {:?}",
        isolated_observations.0
    );
}

#[test]
fn invalid_arithmetic_target_commit_survives_later_resource_retry() {
    // The §1236 recovery and §1269 afterassignment replay are a committed
    // operation. A later missing-resource rollback cannot duplicate either.
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\def\mark{\global\advance\count0 by1}
           \afterassignment\mark\advance x
           \input child\end",
    );

    for _ in 0..8 {
        if stores.count(0) == 1 {
            break;
        }
        assert!(matches!(
            control.advance(&mut stores).expect("setup executes"),
            CanonicalStepResult::Progress(ReplayStep::Continue)
        ));
    }
    assert_eq!(stores.count(0), 1);
    let committed = terminal_text(&stores);
    assert_eq!(committed.matches("the letter x").count(), 1);

    for _ in 0..3 {
        assert!(matches!(
            control.advance(&mut stores).expect("missing input suspends"),
            CanonicalStepResult::Suspended(CanonicalResourceNeed::Input { name })
                if name == "child.tex"
        ));
        assert_eq!(stores.count(0), 1);
        assert_eq!(stores.take_afterassignment(), None);
        assert_eq!(terminal_text(&stores), committed);
    }
}

#[test]
fn message_spacing_follows_the_texweb_1280_offset_rule() {
    // TeX82 §1280 separates consecutive `\message` texts with one space when
    // a line is already open.
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(&mut control, br"\message{a}\message{b}\end");
    run_to_end(&mut control, &mut stores);

    assert!(
        terminal_text(&stores).contains("a b"),
        "{}",
        terminal_text(&stores)
    );
}

#[test]
fn errmessage_prefers_errhelp_over_the_builtin_help() {
    // TeX82 §1283: `if err_help<>null then use_err_help:=true`, and §90 shows
    // it on the transcript.
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\nonstopmode\errhelp={user help}\errmessage{bad}\count0=1\end",
    );
    run_to_end(&mut control, &mut stores);

    assert_eq!(stores.count(0), 1);
    let output = terminal_text(&stores);
    assert!(output.contains("! bad."), "{output}");
    assert!(output.contains("user help"), "{output}");
    assert!(!output.contains("Hercule Poirot"), "{output}");
}

#[test]
fn patterns_and_dump_are_initex_only_and_reported_in_a_production_session() {
    // TeX82 §1252 and §1335 are both `init`-guarded.
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let _initex = CanonicalMainControl::tex82_initex(&mut stores);
    let mut control = CanonicalMainControl::new();
    register_source(&mut control, br"\patterns{a1b}\count0=1\dump");
    run_to_end(&mut control, &mut stores);

    assert_eq!(stores.count(0), 1);
    assert!(!control.dumped_format());
    assert!(control.format_dump_receipt().is_none());
    let output = terminal_text(&stores);
    // §1252's production branch, which is a different rejection from §960's
    // "Too late" one and carries no help lines.
    assert!(
        output.contains("! Patterns can be loaded only by INITEX.\nl.1 \\patterns\n"),
        "{output}"
    );
    assert!(!output.contains("Too late for"), "{output}");
    assert!(
        output.contains("(\\dump is performed only by INITEX)"),
        "{output}"
    );
}

#[test]
fn initex_late_patterns_absorbs_its_discarded_group() {
    // TeX82 §919 closes pattern insertion when the first hyphenation pass
    // initializes the trie. §960's later `\patterns` recovery is
    // `scan_toks(false,false)`, so §473 enters absorbing status before §403
    // reads the group's left brace.
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    stores.close_hyphenation_patterns();
    register_source(
        &mut control,
        br"\nonstopmode\patterns{toolate}\count0=1\end",
    );
    let mut observations = ObservationRecorder::default();
    run_to_end_observed(&mut control, &mut stores, &mut observations);

    assert_eq!(stores.count(0), 1);
    let absorbing = observations
        .0
        .iter()
        .position(|event| {
            matches!(
                event,
                CommandObservation::ScannerStatus(status)
                    if status.from == "normal" && status.to == "absorbing"
            )
        })
        .expect("late pattern recovery enters absorbing");
    let opening = observations
        .0
        .iter()
        .position(|event| {
            matches!(
                event,
                CommandObservation::Command(command)
                    if command.boundary == tex_command::CommandDeliveryBoundary::Raw
                        && matches!(
                            command.spelling,
                            tex_command::ObservedToken::Character {
                                character: '{',
                                ..
                            }
                        )
            )
        })
        .expect("late pattern group has an opening brace");
    assert!(absorbing < opening, "{:?}", observations.0);
    assert!(
        terminal_text(&stores).contains("! Too late for \\patterns."),
        "{}",
        terminal_text(&stores)
    );
}

#[test]
fn initex_late_patterns_prompts_at_the_pre_scan_section_960_context() {
    // TeX82 §960 calls §82's `error` before §473 scans and discards the
    // braced group. A deferred executor report must therefore carry the
    // source cursor immediately after `\patterns`, not the post-group cursor.
    let mut stores = Universe::new_with_plain_catcodes();
    stores
        .world_mut()
        .push_memory_terminal_line("s")
        .expect("memory terminal accepts the error response");
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    stores.close_hyphenation_patterns();
    register_source(&mut control, b"\\patterns{toolate}\\count0=1\\end");

    run_to_end(&mut control, &mut stores);

    assert_eq!(stores.count(0), 1, "interactive recovery resumes input");
    let output = terminal_text(&stores);
    let context = output
        .find("! Too late for \\patterns.\nl.1 \\patterns\n")
        .expect("§960 reports at the pre-scan source cursor");
    let prompt = output.find("? ").expect("§82 interactive prompt");
    assert!(context < prompt, "{output}");
}

#[test]
fn hyphenation_diagnostics_preserve_tex82_recovery_and_apply_order() {
    // TeX82 §§936-937 and §§961-963: scanner othercases retain the
    // partially collected word; invalid lccodes are diagnosed during apply;
    // a duplicate is diagnosed after its replacement has been installed.
    // The schema-v1 TeX82 instrumentation publishes no diagnostic event for
    // either the scanner or apply sites.
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\nonstopmode
           \hyphenation{ab\relax cd ab!c-d}
           \patterns{a\relax b a!b a1b a2b}
           \count0=1\end",
    );
    let mut observations = ObservationRecorder::default();
    loop {
        match control
            .step_with_observer(&mut stores, &mut observations)
            .expect("canonical program executes")
        {
            MainControlStep::End | MainControlStep::EndOfInput => break,
            MainControlStep::Continue => {}
        }
    }

    assert_eq!(stores.count(0), 1);
    assert!(
        !observations
            .0
            .iter()
            .any(|event| matches!(event, CommandObservation::Diagnostic(_))),
        "§§936/961/963/966 have no schema-v1 diagnostic observation"
    );
    let output = terminal_text(&stores);
    for expected in [
        "! Improper \\hyphenation will be flushed.",
        "! Not a letter.",
        "! Bad \\patterns.",
        "! Nonletter.",
        "! Duplicate pattern.",
    ] {
        assert!(
            output.contains(expected),
            "missing {expected:?} in {output}"
        );
    }
    let positions = [
        "Improper \\hyphenation",
        "Not a letter",
        "Bad \\patterns",
        "Nonletter",
        "Duplicate pattern",
    ]
    .map(|message| output.find(message).expect("diagnostic is present"));
    assert!(
        positions.windows(2).all(|pair| pair[0] < pair[1]),
        "scanner/apply diagnostic order changed: {output}"
    );
}

#[test]
fn bad_patterns_reports_the_live_section_961_source_context() {
    // TeX82 §961 calls §82's `error` immediately after `get_x_token`
    // classifies the offending command. The context cursor is therefore
    // immediately after `\relax`, before scanning resumes.
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        b"\\nonstopmode\n\\patterns{ab\\relax cd}\n\\end",
    );

    run_to_end(&mut control, &mut stores);

    let output = terminal_text(&stores);
    assert!(
        output.contains("! Bad \\patterns.\nl.2 \\patterns{ab\\relax\n                       cd}"),
        "§82 must render the source cursor at §961's offending command: {output}"
    );
}

#[test]
fn pattern_nonletter_prompts_at_the_live_section_962_source_context() {
    // TeX82 §962 calls §82's `error` before the next `get_x_token`, while
    // the nonletter and the source cursor immediately after it are live.
    // Delaying this report until the whole group has scanned makes the
    // interaction consume its response after unrelated pattern input.
    let mut stores = Universe::new_with_plain_catcodes();
    stores
        .world_mut()
        .push_memory_terminal_line("s")
        .expect("memory terminal accepts the error response");
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(&mut control, b"\\patterns{ab!cd ef1gh}\\count0=1\\end");

    run_to_end(&mut control, &mut stores);

    assert_eq!(stores.count(0), 1, "interactive recovery resumes input");
    let output = terminal_text(&stores);
    let context = output
        .find("! Nonletter.\nl.1 \\patterns{ab!\n")
        .expect("§962 reports the live nonletter context");
    let prompt = output.find("? ").expect("§82 interactive prompt");
    assert!(context < prompt, "{output}");
    assert_eq!(
        output.matches("! Nonletter.").count(),
        1,
        "apply time must not report §962's already-reported error again: {output}"
    );
}

#[test]
fn duplicate_pattern_prompts_at_the_live_section_963_separator_context() {
    // TeX82 §963 tests trie_o[q] and calls §82 before the §961 loop asks for
    // another token. The separator is therefore still current, and an
    // interactive response must not be consumed from later source input.
    let mut stores = Universe::new_with_plain_catcodes();
    stores
        .world_mut()
        .push_memory_terminal_line("s")
        .expect("memory terminal accepts the error response");
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(&mut control, b"\\patterns{a1b a2b next}\\count0=1\\end");

    run_to_end(&mut control, &mut stores);

    assert_eq!(stores.count(0), 1, "interactive recovery resumes input");
    let output = terminal_text(&stores);
    let context = output
        .find("! Duplicate pattern.\nl.1 \\patterns{a1b a2b ")
        .expect("§963 reports at the live separator");
    let prompt = output.find("? ").expect("§82 interactive prompt");
    assert!(context < prompt, "{output}");
    assert_eq!(
        output.matches("! Duplicate pattern.").count(),
        1,
        "executor must not repeat §963's scan-time report: {output}"
    );
}

#[test]
fn distinct_pattern_paths_do_not_report_section_963_duplicate() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        b"\\nonstopmode\\patterns{a1b a2c}\\count0=1\\end",
    );

    run_to_end(&mut control, &mut stores);

    assert_eq!(stores.count(0), 1);
    assert!(
        !terminal_text(&stores).contains("! Duplicate pattern."),
        "different trie paths are the negative control"
    );
}

#[test]
fn pending_pattern_duplicate_view_follows_section_963_replacement_order() {
    // TeX82 §963 diagnoses from the path's current trie_o and then replaces
    // it; §965 computes min_trie_op for an operationless pattern. These
    // sequences cover both transitions through that ordered state.
    for (patterns, expected_duplicates) in [
        ("b1b bb b2b", 1), // real -> operationless -> real
        ("bb b1b b2b", 1), // operationless -> real -> real
        ("b1b b2b", 1),    // real -> real
        ("bb bb bb", 0),   // repeated operationless
    ] {
        let mut stores = Universe::new_with_plain_catcodes();
        let mut control = CanonicalMainControl::tex82_initex(&mut stores);
        register_source(
            &mut control,
            format!("\\nonstopmode\\patterns{{{patterns}}}\\count0=1\\end").as_bytes(),
        );

        run_to_end(&mut control, &mut stores);

        assert_eq!(stores.count(0), 1, "{patterns}");
        assert_eq!(
            terminal_text(&stores)
                .matches("! Duplicate pattern.")
                .count(),
            expected_duplicates,
            "{patterns}"
        );
    }
}

#[test]
fn operationless_pattern_path_is_not_a_section_963_duplicate() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        b"\\nonstopmode\\patterns{bb bb1 b2b}\\count0=1\\end",
    );

    run_to_end(&mut control, &mut stores);

    assert_eq!(stores.count(0), 1);
    assert_eq!(
        terminal_text(&stores)
            .matches("! Duplicate pattern.")
            .count(),
        1,
        "only the second real trie operation on the shared path is duplicate"
    );
}

#[test]
fn pattern_duplicate_paths_are_partitioned_by_language() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        b"\\nonstopmode\\language=1\\patterns{b1b}\\language=2\\patterns{b2b}\\count0=1\\end",
    );

    run_to_end(&mut control, &mut stores);

    assert_eq!(stores.count(0), 1);
    assert_eq!(
        terminal_text(&stores)
            .matches("! Duplicate pattern.")
            .count(),
        0
    );
}

#[test]
fn committed_and_pending_pattern_paths_share_replacement_order() {
    let mut stores = Universe::new_with_plain_catcodes();
    assert!(
        !stores
            .add_hyphenation_pattern_for_language(
                0,
                PatternSpec {
                    letters: vec!['b', 'b'],
                    values: vec![0, 1, 0],
                },
            )
            .expect("pattern fits the default trie capacity")
    );
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        b"\\nonstopmode\\patterns{bb b2b}\\count0=1\\end",
    );

    run_to_end(&mut control, &mut stores);

    assert_eq!(stores.count(0), 1);
    assert_eq!(
        terminal_text(&stores)
            .matches("! Duplicate pattern.")
            .count(),
        1,
        "committed real is diagnosed, its operationless replacement clears the pending view, and the following real is accepted"
    );
}

#[test]
fn first_pattern_digit_is_a_level_not_a_section_962_nonletter() {
    // TeX82 §962's `digit_sensed=false` branch treats the first ASCII digit
    // as a hyphen level and therefore never consults its zero `\lccode`.
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        b"\\nonstopmode\\patterns{ab1cd}\\count0=1\\end",
    );

    run_to_end(&mut control, &mut stores);

    assert_eq!(stores.count(0), 1);
    assert!(
        !terminal_text(&stores).contains("! Nonletter."),
        "a hyphen-level digit is the negative control"
    );
}

#[test]
fn pattern_length_bound_preserves_section_962_digit_state() {
    // TeX82 §962 changes `digit_sensed` only in the branches guarded by
    // `k<63`. Thus a digit after 63 stored letters is ignored without making
    // the next digit a letter, while consecutive digits below the bound do
    // classify the second digit as a letter and diagnose its zero `\lccode`.
    for (letters, suffix, expected_nonletters) in [
        (62, "11!", 2),
        (63, "11!", 1),
        (64, "11!", 1),
        (2, "11", 1),
        (2, "1a", 0),
    ] {
        let mut stores = Universe::new_with_plain_catcodes();
        let mut control = CanonicalMainControl::tex82_initex(&mut stores);
        let source = format!(
            "\\nonstopmode\\patterns{{{}{suffix}}}\\count0=1\\end",
            "a".repeat(letters)
        );
        register_source(&mut control, source.as_bytes());

        run_to_end(&mut control, &mut stores);

        assert_eq!(stores.count(0), 1, "letters={letters}, suffix={suffix}");
        assert_eq!(
            terminal_text(&stores).matches("! Nonletter.").count(),
            expected_nonletters,
            "letters={letters}, suffix={suffix}: {}",
            terminal_text(&stores)
        );
    }
}

#[test]
fn show_completion_prompts_in_error_stop_mode_and_honors_the_answer() {
    // TeX82 §1293's `common_ending: ...; error`, whose §83 dialog prompts
    // `?␣` and whose §86 `S` answer switches to scroll mode.
    let mut stores = Universe::new_with_plain_catcodes();
    stores
        .world_mut()
        .push_memory_terminal_line("s")
        .expect("memory terminal accepts a line");
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(&mut control, br"\showthe\count0 \count1=1\end");
    run_to_end(&mut control, &mut stores);

    assert_eq!(stores.count(1), 1);
    let output = terminal_text(&stores);
    assert!(output.contains("> 0."), "{output}");
    assert!(output.contains("? "), "{output}");
    assert_eq!(
        stores.interaction_mode(),
        tex_state::InteractionMode::Scroll
    );
}

#[test]
fn undefined_control_sequence_reports_once_and_drops_only_its_token() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(&mut control, br"\nonstopmode\missing\count0=17\end");
    run_to_end(&mut control, &mut stores);
    assert_eq!(stores.count(0), 17, "the following command remains live");
    assert_eq!(stores.world().error_channel().error_count(), 1);
    let output = terminal_text(&stores);
    assert_eq!(
        output.matches("! Undefined control sequence.").count(),
        1,
        "{output}"
    );
    assert!(
        output.contains("The control sequence at the end of the top line"),
        "{output}"
    );
    assert!(
        output.contains("and I'll forget about whatever was undefined."),
        "{output}"
    );
}

#[test]
fn misplaced_tab_reports_once_and_drops_only_the_delimiter() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(&mut control, br"\nonstopmode&\count0=19\end");
    run_to_end(&mut control, &mut stores);
    assert_eq!(stores.count(0), 19, "the delimiter was not backed up");
    assert_eq!(stores.world().error_channel().error_count(), 1);
    let output = terminal_text(&stores);
    assert_eq!(
        output
            .matches("! Misplaced alignment tab character &.")
            .count(),
        1,
        "{output}"
    );
    assert!(
        output.contains("here. If you just want an ampersand, the remedy is"),
        "{output}"
    );
}

#[test]
fn math_group_collapses_only_one_undecorated_ord_nucleus() {
    let mut stores = Universe::new_with_plain_catcodes();
    let empty_list = stores.freeze_node_list(&[]);
    let ch = MathChar {
        family: 0,
        character: 'x',
        origin: tex_state::token::OriginId::UNKNOWN,
    };
    for nucleus in [
        MathField::Empty,
        MathField::MathChar(ch),
        MathField::SubBox(empty_list),
        MathField::SubMlist(empty_list),
    ] {
        let list = stores.freeze_node_list(&[Node::MathNoad(MathNoad::new(
            NoadKind::Normal(NoadClass::Ord),
            nucleus.clone(),
        ))]);
        assert_eq!(collapse_singleton_math_group(&stores, list), nucleus);
    }

    let scripted = stores.freeze_node_list(&[Node::MathNoad(MathNoad {
        kind: NoadKind::Normal(NoadClass::Ord),
        nucleus: MathField::MathChar(ch),
        subscript: MathField::MathChar(ch),
        superscript: MathField::Empty,
    })]);
    let non_ord = stores.freeze_node_list(&[Node::MathNoad(MathNoad::new(
        NoadKind::Normal(NoadClass::Open),
        MathField::MathChar(ch),
    ))]);
    let multiple = stores.freeze_node_list(&[
        Node::MathNoad(MathNoad::new(
            NoadKind::Normal(NoadClass::Ord),
            MathField::MathChar(ch),
        )),
        Node::MathNoad(MathNoad::new(
            NoadKind::Normal(NoadClass::Ord),
            MathField::MathChar(ch),
        )),
    ]);
    for list in [scripted, non_ord, multiple] {
        assert_eq!(
            collapse_singleton_math_group(&stores, list),
            MathField::SubMlist(list)
        );
    }
}
