//! Profile initialization, format loading or dumping, and INITEX-only state.

use super::*;

#[test]
fn fresh_initex_installs_canonical_parameters_and_clock_before_execution() {
    let clock = tex_state::JobClock {
        time: 13 * 60 + 37,
        second: 11,
        day: 21,
        month: 8,
        year: 2026,
    };
    crate::test_harness::with_world_universe(
        tex_state::World::memory_with_clock(clock),
        |stores| {
            let mut control = MainControl::tex82_initex(stores);
            admitted!(stores, |context| {
                assert_eq!(context.int_param(IntParam::TOLERANCE), 10_000);
                assert_eq!(context.int_param(IntParam::MAG), 1_000);
                assert_eq!(context.int_param(IntParam::ESCAPE_CHAR), i32::from(b'\\'));
                assert_eq!(context.int_param(IntParam::END_LINE_CHAR), i32::from(b'\r'));
                assert_eq!(context.int_param(IntParam::NEWLINE_CHAR), 0);
                assert_eq!(context.int_param(IntParam::MAX_DEAD_CYCLES), 25);
                assert_eq!(context.int_param(IntParam::HANG_AFTER), 1);
            });
            let widths = stores.error_context_widths();
            assert_eq!(widths.error_line(), 79);
            assert_eq!(widths.half_error_line(), 50);
            assert_eq!(widths.max_print_line(), 79);

            admitted!(stores, |context| context
                .assign_int_param(IntParam::MAG, 1_200, tex_state::AssignmentScope::Global)
                .expect("plain prelude magnification"));
            tex_command::install_tex82_expandable_primitives(stores);
            crate::install_unexpandable_primitives(stores);
            admitted!(stores, |context| assert_eq!(
                context.int_param(IntParam::MAG),
                1_200,
                "repeat profile installation cannot overwrite a Plain assignment"
            ));

            control.begin_job(stores, "defaults.tex");
            admitted!(stores, |context| {
                assert_eq!(context.int_param(IntParam::TIME), clock.time);
                assert_eq!(context.int_param(IntParam::DAY), clock.day);
                assert_eq!(context.int_param(IntParam::MONTH), clock.month);
                assert_eq!(context.int_param(IntParam::YEAR), clock.year);
            });
        },
    );
}
#[test]
fn restored_profile_registration_preserves_format_parameters_except_clock() {
    let clock = tex_state::JobClock {
        time: 719,
        second: 3,
        day: 22,
        month: 8,
        year: 2031,
    };
    crate::test_harness::with_world_universe(
        tex_state::World::memory_with_clock(clock),
        |stores| {
            let pages_attr = allocate_tokens(
                stores,
                &[Token::Char {
                    ch: 'x',
                    cat: Catcode::Other,
                }],
            );
            admitted!(stores, |context| {
                for (parameter, value) in [
                    (IntParam::MAG, 1_200),
                    (IntParam::ESCAPE_CHAR, i32::from(b'!')),
                    (IntParam::PDF_COMPRESS_LEVEL, 2),
                ] {
                    context
                        .assign_int_param(parameter, value, tex_state::AssignmentScope::Global)
                        .expect("restored integer parameter");
                }
                context
                    .assign_dimen_param(
                        tex_state::env::banks::DimenParam::PDF_H_ORIGIN,
                        Scaled::from_raw(123),
                        tex_state::AssignmentScope::Global,
                    )
                    .expect("restored PDF origin");
                context
                    .assign_token_parameter(
                        tex_state::env::banks::TokParam::PDF_PAGES_ATTR,
                        Some(pages_attr.clone()),
                        tex_state::AssignmentScope::Global,
                    )
                    .expect("restored PDF token parameter");
            });

            tex_command::register_tex82_expandable_primitives(stores);
            crate::register_unexpandable_primitives(stores);
            tex_command::register_etex_expandable_primitives(stores);
            crate::register_etex_unexpandable_primitives(stores);
            tex_command::register_pdftex_expandable_primitives(stores);
            tex_command::register_pdftex_unexpandable_primitives(stores);
            let mut control = MainControl::with_profile(CommandProfile::PDFTEX14029);
            control.set_preloaded_format(crate::PreloadedFormat {
                dump_name: "plain".to_owned(),
                format_name: "plain".to_owned(),
                year: 2026,
                month: 8,
                day: 21,
            });
            control.begin_job(stores, "restored.tex");

            admitted!(stores, |context| {
                assert_eq!(context.int_param(IntParam::MAG), 1_200);
                assert_eq!(context.int_param(IntParam::ESCAPE_CHAR), i32::from(b'!'));
                assert_eq!(context.int_param(IntParam::PDF_COMPRESS_LEVEL), 2);
                assert_eq!(
                    context.dimen_param(tex_state::env::banks::DimenParam::PDF_H_ORIGIN),
                    Scaled::from_raw(123)
                );
                assert_eq!(
                    context
                        .token_parameter(tex_state::env::banks::TokParam::PDF_PAGES_ATTR)
                        .expect("PDF token parameter"),
                    Some(pages_attr)
                );
                assert_eq!(context.int_param(IntParam::TIME), clock.time);
                assert_eq!(context.int_param(IntParam::DAY), clock.day);
                assert_eq!(context.int_param(IntParam::MONTH), clock.month);
                assert_eq!(context.int_param(IntParam::YEAR), clock.year);
            });
        },
    );
}
#[test]
fn math_choice_nested_font_definition_executes_with_preloaded_font() {
    // TeX82 §§1172/1174 executes each math-choice branch through ordinary
    // main control, and §1270 dispatches assignments in that nested episode.
    // The increment immediately before the nested definition proves that the
    // nested branch remains an ordinary execution context. Resource replay
    // itself is owned by the incremental session tests; this direct semantic
    // test admits the fixture before execution.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_cmr10_as(&mut control, stores, "cmr10.tfm");
        register_cmr10_as(&mut control, stores, "cmti8.tfm");
        register_source(
            &mut control,
            br"\font\body=cmr10 \body $\mathchoice{\global\advance\count0 by1 \font\nested=cmti8 A}{B}{C}{D}$\global\count1=23\end",
        );

        run_to_end(&mut control, stores);

        assert_eq!(stores.count(0).expect("count register"), 1);
        assert_eq!(stores.count(1).expect("count register"), 23);
        assert!(control.pending_resource_site().is_none());
    });
}
#[test]
fn format_font_suspension_while_closing_box_retains_active_owner() {
    // TeX82 §1086 keeps `box_context` and the scan-spec values live through
    // `package`. A loaded format restores the logical font without carrying
    // its host resource, so materializing the box's final pending character
    // is a typed suspension inside that same packaging operation. Repeating
    // the suspension proves retry does not consume the move-only box owner.
    let image = crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_cmr10_as(&mut control, stores, "cmr10.tfm");
        register_source(&mut control, br"\font\f=cmr10 \dump");
        run_to_end(&mut control, stores);
        control
            .take_format_dump(stores)
            .expect("quiescent font format captures")
            .expect("INITEX produced a format")
            .image
    });

    tex_state::with_materialized_format(
        tex_state::interner::InternerBudget::new(16_384, 16_384, 1 << 20)
            .expect("test interner budget"),
        tex_state::World::memory(),
        image,
        |stores| {
            tex_command::install_tex82_expandable_primitives(stores);
            crate::install_unexpandable_primitives(stores);
            let mut control = MainControl::with_profile(CommandProfile::TEX82);
            control.set_preloaded_format(crate::PreloadedFormat {
                dump_name: "box-font".to_owned(),
                format_name: "box-font".to_owned(),
                year: 2026,
                month: 8,
                day: 31,
            });
            control.begin_job(stores, "box-font.tex");
            register_cmr10_as(&mut control, stores, "cmr10.tfm");
            register_source(&mut control, br"\setbox0=\vbox{\hbox{\f X}}\count0=23\end");
            run_to_end(&mut control, stores);

            assert_eq!(stores.count(0).expect("count register"), 23);
            assert!(stores.copy_box_to_page(0).is_some());
            assert!(control.boxes.active_boxes.is_empty());
            assert!(
                !terminal_text(stores).contains("Too many }'s"),
                "{}",
                terminal_text(stores)
            );
        },
    )
    .expect("font format materializes");
}
#[test]
fn loaded_format_everyjob_preserves_number_signs_and_internal_operands() {
    // TeX82 §440 keeps every leading sign in the integer scanner, even when
    // expansion supplies a later sign from a restored macro.  The same
    // loaded-format boundary exercises §413/§429 admission for an integer
    // parameter, register, dimension, and glue value, then loads it before
    // the root job so format restoration and job-start input remain part of
    // the regression.
    let image = crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\def\minusone{-1}\count0=17\dimen0=123sp\skip0=1sp\everyjob{\message{X=\number-\minusone T=\number\time C=\number\count0 D=\number\dimen0 G=\number\skip0 S=\number--\count0}}\dump",
        );
        run_to_end(&mut control, stores);
        admitted!(stores, |context| assert!(
            context
                .token_parameter(tex_state::env::banks::TokParam::EVERY_JOB)
                .expect("everyjob parameter")
                .is_some(),
            "INITEX must retain the everyjob token list before dumping"
        ));
        control
            .take_format_dump(stores)
            .expect("quiescent sign format capture")
            .expect("INITEX produced a sign format")
            .image
    });

    tex_state::with_materialized_format(
        tex_state::interner::InternerBudget::new(16_384, 16_384, 1 << 20)
            .expect("test interner budget"),
        tex_state::World::memory(),
        image,
        |stores| {
            admitted!(stores, |context| assert!(
                context
                    .token_parameter(tex_state::env::banks::TokParam::EVERY_JOB)
                    .expect("everyjob parameter")
                    .is_some(),
                "dumped everyjob token list must survive format materialization"
            ));
            tex_command::register_tex82_expandable_primitives(stores);
            crate::register_unexpandable_primitives(stores);
            let mut control = MainControl::with_profile(CommandProfile::TEX82);
            control.set_preloaded_format(crate::PreloadedFormat {
                dump_name: "signs".to_owned(),
                format_name: "signs".to_owned(),
                year: 2026,
                month: 9,
                day: 5,
            });
            control.begin_job(stores, "signs.tex");
            register_source(&mut control, br"\end");
            run_to_end(&mut control, stores);

            let terminal = terminal_text(stores);
            assert!(terminal.contains("X=1"), "{terminal}");
            assert!(terminal.contains("T="), "{terminal}");
            assert!(terminal.contains("C=17"), "{terminal}");
            assert!(terminal.contains("D=123"), "{terminal}");
            assert!(terminal.contains("G=1"), "{terminal}");
            assert!(terminal.contains("S=17"), "{terminal}");
            assert!(!terminal.contains("Missing number"), "{terminal}");
        },
    )
    .expect("sign format materializes");
}
#[test]
fn fontname_expansion_includes_a_non_design_size() {
    // TeX82 §§471--472: `\fontname` emits the external name followed by
    // `at <size>pt` when the selected size differs from the TFM design size.
    // TRIP line 339 captures this exact expansion inside a global `\edef`.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_cmr10_as(&mut control, stores, "cmr10.tfm");
        register_source(
            &mut control,
            br"\font\small=cmr10 scaled 500
\edef\result{\fontname\small}\message{RESULT:[\result]}\end",
        );

        run_to_end(&mut control, stores);

        assert!(
            terminal_text(stores).contains("RESULT:[cmr10 at 5.0pt]"),
            "{}",
            terminal_text(stores)
        );
    });
}
#[test]
fn tracingrestores_reports_chardef_meanings_as_char_commands_in_every_profile() {
    // TeX82 §§252/298/1223: `show_eqtb` renders a `char_given` eqtb word
    // through `print_cmd_chr`, whose vocabulary is `\char` plus a hexadecimal
    // operand. e-TeX 2.6 and pdfTeX retain that profile-independent arm. The
    // primitive-alias and mathchardef tests on either side are negative
    // controls for the other region-one meaning classes.
    for profile in [
        CommandProfile::TEX82,
        CommandProfile::ETEX26,
        CommandProfile::PDFTEX14029,
    ] {
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            let mut control = if profile == CommandProfile::TEX82 {
                MainControl::tex82_initex(stores)
            } else if profile == CommandProfile::ETEX26 {
                etex_initex(stores)
            } else {
                debug_assert_eq!(profile, CommandProfile::PDFTEX14029);
                pdftex_initex(stores)
            };
            register_source(
                &mut control,
                br#"\chardef\x="C8 \tracingrestores=1\tracingonline=1
                    {\let\x=\relax}\end"#,
            );

            run_to_end(&mut control, stores);

            let expected = "{restoring \\x=\\char\"C8}\n";
            assert_eq!(pending_sink_text(stores, true), expected, "{profile:?}");
            assert_eq!(pending_sink_text(stores, false), expected, "{profile:?}");
        });
    }
}
#[test]
fn tracingoutput_box_dump_precedes_deferred_write_expansion() {
    // TeX82 §638 closes and displays the box before §1370 expands any
    // deferred write inside it. Both reports are log-only in batch mode, so
    // this also proves that splitting their admitted builders does not split
    // or reverse their outer publication order.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\batchmode\tracingcommands=2\tracingoutput=1
               \shipout\hbox{\write16{\romannumeral0\relax}}\end",
        );

        run_to_end(&mut control, stores);

        let terminal =
            String::from_utf8_lossy(stores.world().memory_terminal_output().unwrap_or_default());
        let log = String::from_utf8_lossy(stores.world().memory_log_output().unwrap_or_default());
        let announcement = log
            .find("Completed box being shipped out")
            .expect("shipout announcement is materialized");
        let dump = log[announcement..]
            .find("\\hbox(")
            .map(|offset| announcement + offset)
            .expect("box dump follows its announcement");
        let write_trace = log[dump..]
            .find("{no mode: \\romannumeral}")
            .map(|offset| dump + offset)
            .expect("deferred write trace follows the box dump");
        assert!(announcement < dump && dump < write_trace, "{log}");
        assert!(!terminal.contains("romannumeral"), "{terminal}");
    });
}
#[test]
fn identical_local_let_is_profile_gated_and_global_let_always_commits() {
    // TeX82 §§277/1221 execute both identical local `eq_define` calls. e-TeX
    // change [19.277] suppresses the second one in extended mode. The changed
    // first assignment and identical global assignment are negative controls.
    for (profile, expected) in [
        (
            CommandProfile::TEX82,
            vec![
                (Some("left_brace"), false),
                (Some("left_brace"), false),
                (Some("left_brace"), true),
            ],
        ),
        (
            CommandProfile::ETEX26,
            vec![(Some("left_brace"), false), (Some("left_brace"), true)],
        ),
    ] {
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            let mut control = if profile == CommandProfile::ETEX26 {
                etex_initex(stores)
            } else {
                MainControl::tex82_initex(stores)
            };
            register_source(
                &mut control,
                br"\catcode123=1 \let\bgroup={ \let\bgroup={ \global\let\bgroup={ \end",
            );
            let mut observations = ObservationRecorder::default();
            run_to_end_observed(&mut control, stores, &mut observations);

            let mutations: Vec<_> = observations
                .0
                .iter()
                .filter_map(|observation| match observation {
                    CommandObservation::Mutation(record)
                        if record.target == MutationTarget::Meaning =>
                    {
                        Some((observation_name(&record.value), record.global))
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(mutations, expected, "profile: {profile:?}");
        });
    }
}
#[test]
fn tex82_profile_leaves_numbered_marks_undefined() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let _control = MainControl::tex82_initex(stores);
        admitted!(stores, |context| {
            let marks = context.intern_control_sequence("marks");
            assert_eq!(
                context.meaning(marks),
                ResolvedMeaning::Static(Meaning::Undefined)
            );
        });
        assert_eq!(stores.primitive_meaning("marks"), None);
    });
}
#[test]
fn show_meaning_reads_raw_token_and_formats_each_macro_meaning_kind() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\def\macro{body}\show\undefined\show\relax\show\macro\end",
        );
        run_to_end(&mut control, stores);
        let output = terminal_text(stores);
        assert!(output.contains("> \\undefined=undefined."), "{output}");
        assert!(output.contains("> \\relax=\\relax."), "{output}");
        assert!(output.contains("> \\macro=macro:\n->body."), "{output}");
    });
}
#[test]
fn final_cleanup_retires_inputs_reports_open_state_and_selects_end_or_dump() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\def\stop{\end}\stop");
        let mut observations = ObservationRecorder::default();
        loop {
            if matches!(
                control
                    .step_with_observer(stores, &mut observations)
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
            CommandObservation::Effect(effect) if effect.kind == ObservationEffectKind::Terminate
        )));
    });
}
#[test]
fn end_and_dump_run_profile_specific_cleanup_in_observable_order() {
    // TeX82 §§1330--1337 enter the selected profile before main control,
    // retire live input during `final_cleanup`, close numbered streams, and
    // only then expose termination.  A successful INITEX `\dump` additionally
    // defers its announcement until the host confirms publication.
    for profile in [CommandProfile::TEX82, CommandProfile::ETEX26] {
        for dump in [false, true] {
            crate::test_harness::with_nonstop_plain_universe(|stores| {
                let mut control = if profile == CommandProfile::ETEX26 {
                    etex_initex(stores)
                } else {
                    MainControl::tex82_initex(stores)
                };
                control.begin_job(stores, "lifecycle.tex");
                register_source(
                    &mut control,
                    if dump {
                        br"\immediate\openout3=cleanup\dump"
                    } else {
                        br"\immediate\openout3=cleanup\end"
                    },
                );

                let mut observations = ObservationRecorder::default();
                run_to_end_observed(&mut control, stores, &mut observations);
                let ordered: Vec<_> = observations
                    .0
                    .iter()
                    .filter_map(|observation| match observation {
                        CommandObservation::Input(input)
                            if input.transition == InputTransition::Retire =>
                        {
                            Some("retire")
                        }
                        CommandObservation::Effect(effect)
                            if effect.kind == ObservationEffectKind::Close =>
                        {
                            Some("close")
                        }
                        CommandObservation::Effect(effect)
                            if effect.kind == ObservationEffectKind::Terminate =>
                        {
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

                let terminal = terminal_text(stores);
                assert_eq!(
                    terminal.contains("entering extended mode"),
                    profile == CommandProfile::ETEX26
                );
                assert_eq!(control.dumped_format(), dump);
                assert!(!terminal.contains("Beginning to dump on file"));
                if dump {
                    let mut receipt = control.format_dump_receipt().expect("dump receipt").clone();
                    crate::confirm_format_dump_publication(stores, &mut receipt, "lifecycle.fmt");
                    assert!(
                        terminal_text(stores).contains("Beginning to dump on file lifecycle.fmt")
                    );
                }
            });
        }
    }
}
#[test]
fn initex_dump_owns_identifier_but_waits_for_publication_receipt() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        crate::test_harness::assign_int_param(
            stores,
            IntParam::YEAR,
            2026,
            tex_state::AssignmentScope::Global,
        )
        .expect("integer parameter assignment");
        crate::test_harness::assign_int_param(
            stores,
            IntParam::MONTH,
            7,
            tex_state::AssignmentScope::Global,
        )
        .expect("integer parameter assignment");
        crate::test_harness::assign_int_param(
            stores,
            IntParam::DAY,
            9,
            tex_state::AssignmentScope::Global,
        )
        .expect("integer parameter assignment");
        let mut control = MainControl::tex82_initex(stores);
        control
            .capabilities_mut()
            .set_startup_job_name("bounded-dump.tex");
        register_source(&mut control, br"\dump");
        let before = admitted!(stores, |context| context.detach_engine_usage_statistics());

        run_to_end(&mut control, stores);

        assert!(control.dumped_format());
        assert_eq!(terminal_text(stores), "");
        let mut receipt = control.format_dump_receipt().expect("dump receipt").clone();
        assert_eq!(receipt.format_ident.format_name, "bounded-dump");
        let retained = admitted!(stores, |context| context.detach_engine_usage_statistics());
        assert_eq!(retained.strings - before.strings, 1);
        assert_eq!(
            retained.string_characters - before.string_characters,
            receipt.pool_string().len()
        );
        crate::confirm_format_dump_publication(stores, &mut receipt, "alternate-name.fmt");
        assert_eq!(
            terminal_text(stores),
            "Beginning to dump on file alternate-name.fmt\n (preloaded format=bounded-dump 2026.7.9)"
        );
        assert_eq!(
            admitted!(stores, |context| context.detach_engine_usage_statistics()),
            retained
        );

        let detached = control
            .take_format_dump(stores)
            .expect("quiescent dump capture")
            .expect("successful INITEX dump");
        assert_eq!(detached.receipt.format_ident.format_name, "bounded-dump");
        assert!(!detached.image.as_bytes().is_empty());
        assert!(
            control
                .take_format_dump(stores)
                .expect("exact-once follow-up")
                .is_none()
        );
    });
}
#[test]
fn initex_dump_discards_unread_terminal_command_state_after_image_capture() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        control
            .capabilities_mut()
            .set_startup_job_name("trailing-dump.tex");
        register_source(&mut control, br"\dump\relax");

        run_to_end(&mut control, stores);

        let detached = control
            .take_format_dump(stores)
            .expect("terminal unread input is discardable")
            .expect("successful INITEX dump");
        assert!(!detached.image.as_bytes().is_empty());
        assert!(control.command_mut().format_dump_is_quiescent());
    });
}
#[test]
fn executable_profile_selects_the_process_font_info_capacity() {
    // TeX82 §11 compiles a 20,000-word font_info array, while the pinned
    // Web2C pdfTeX process reads font_mem_size=8,000,000. Modern l3kernel's
    // integer-array fallback relies on growing the newest font through
    // \fontdimen65536, so the executable identity -- not the format image --
    // must select the operational bound before the first command runs.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        control.begin_job(stores, "tex82-capacity.tex");
        register_source(&mut control, br"\fontdimen65536\nullfont=1sp \count0=1\end");
        run_to_end(&mut control, stores);

        assert_eq!(stores.count(0).expect("trailing count"), 1);
        assert_eq!(
            admitted!(stores, |context| context
                .font_parameter_count(tex_state::font::NULL_FONT)),
            7
        );
        assert!(
            terminal_text(stores).contains("! Font \\nullfont has only 7 fontdimen parameters.")
        );
    });

    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = pdftex_initex(stores);
        control.begin_job(stores, "pdftex-capacity.tex");
        register_source(&mut control, br"\fontdimen65536\nullfont=1sp \count0=1\end");
        run_to_end(&mut control, stores);

        assert_eq!(stores.count(0).expect("trailing count"), 1);
        assert_eq!(
            admitted!(stores, |context| context
                .font_parameter_count(tex_state::font::NULL_FONT)),
            65_536
        );
        assert_eq!(
            admitted!(stores, |context| context
                .font_parameter(tex_state::font::NULL_FONT, 65_536)),
            Scaled::from_raw(1)
        );
        assert!(!terminal_text(stores).contains("fontdimen parameters"));
    });
}
#[test]
fn executable_profile_selects_the_process_string_pool_capacity() {
    // TeX82 §44 owns the pool coordinates, while Web2C tex.ch [51.1332]
    // selects the executable process's max_strings and pool_size bounds.
    // The TRIP executables retain their compact conformance profile; the
    // pinned pdfTeX process uses the TeX Live distribution configuration.
    for (pdftex, expected) in [(false, (13_973, 18_192)), (true, (498_918, 6_142_271))] {
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            let mut control = if pdftex {
                pdftex_initex(stores)
            } else {
                MainControl::tex82_initex(stores)
            };
            control.begin_job(stores, "capacity.tex");
            let usage = admitted!(stores, |context| context.detach_engine_usage_statistics());
            assert_eq!(
                (usage.string_capacity, usage.string_character_capacity),
                expected
            );
            assert_eq!(
                (usage.capacity_profile, usage.memory_word_capacity),
                if pdftex {
                    (tex_state::EngineCapacityProfile::Texlive2026, 5_000_000)
                } else {
                    (tex_state::EngineCapacityProfile::Tex82Etex, 250_000)
                }
            );
        });
    }
}
#[test]
fn font_definition_size_boundaries_use_exact_replacements() {
    // TeX82 §§1258--1259 accept scaled 1..32768 and at sizes whose scaled
    // value is 1..(2048pt-1sp); each adjacent invalid value becomes 1000 or
    // 10pt respectively before §1257 interns the font.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_cmr10_as(&mut control, stores, "cmr10.tfm");
        register_source(
        &mut control,
        br"\font\slo=cmr10 scaled 1 \font\shi=cmr10 scaled 32768 \font\szero=cmr10 scaled 0 \font\sover=cmr10 scaled 32769 \font\alo=cmr10 at 0.00002pt \font\ahi=cmr10 at 2047.99998pt \font\azero=cmr10 at 0pt \font\aover=cmr10 at 2048pt \end",
    );
        run_to_end(&mut control, stores);

        let size = |stores: &mut Universe<_>, name: &str| {
            let font = font_by_name(stores, name);
            admitted!(stores, |context| context.font_size(font).raw())
        };
        assert_eq!(size(stores, "slo"), 655);
        assert_eq!(size(stores, "shi"), 21_474_836);
        assert_eq!(size(stores, "szero"), 655_360);
        assert_eq!(size(stores, "sover"), 655_360);
        assert_eq!(size(stores, "alo"), 1);
        assert_eq!(size(stores, "ahi"), 134_217_727);
        assert_eq!(size(stores, "azero"), 655_360);
        assert_eq!(size(stores, "aover"), 655_360);
        let output = terminal_text(stores);
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
    });
}
#[test]
fn font_definition_identity_is_case_sensitive_and_tracks_newest_identifier() {
    // TeX82 §1257 compares the case-sensitive name and size when reusing a
    // font, then assigns font_id_text(f):=u even on the reuse path.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_cmr10_as(&mut control, stores, "cmr10.tfm");
        register_cmr10_as(&mut control, stores, "CMR10.tfm");
        register_source(
            &mut control,
            br"\font\first=cmr10 \font\upper=CMR10 \font\newest=cmr10 \end",
        );
        run_to_end(&mut control, stores);

        let first = font_by_name(stores, "first");
        let upper = font_by_name(stores, "upper");
        let newest = font_by_name(stores, "newest");
        assert_eq!(
            first, newest,
            "same case-sensitive name and size reuses the font"
        );
        assert_ne!(
            first, upper,
            "case-distinct names are distinct font identities"
        );
        let (first_identifier, newest_symbol, upper_identifier, upper_symbol) =
            admitted!(stores, |context| {
                (
                    context.font_identifier_symbol(first),
                    context.symbol("newest"),
                    context.font_identifier_symbol(upper),
                    context.symbol("upper"),
                )
            });
        assert_eq!(
            first_identifier, newest_symbol,
            "the reused font retains the newest identifier"
        );
        assert_eq!(upper_identifier, upper_symbol);
    });
}
#[test]
fn patterns_and_dump_are_initex_only_and_reported_in_a_production_session() {
    // TeX82 §1252 and §1335 are both `init`-guarded.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let _initex = MainControl::tex82_initex(stores);
        let mut control = MainControl::new();
        register_source(&mut control, br"\patterns{a1b}\count0=1\dump");
        run_to_end(&mut control, stores);

        assert_eq!(stores.count(0).expect("count register"), 1);
        assert!(!control.dumped_format());
        assert!(control.format_dump_receipt().is_none());
        let output = terminal_text(stores);
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
    });
}
#[test]
fn initex_late_patterns_absorbs_its_discarded_group() {
    // TeX82 §919 closes pattern insertion when the first hyphenation pass
    // initializes the trie. §960's later `\patterns` recovery is
    // `scan_toks(false,false)`, so §473 enters absorbing status before §403
    // reads the group's left brace.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        admitted!(stores, |context| context.close_hyphenation_patterns());
        register_source(
            &mut control,
            br"\nonstopmode\patterns{toolate}\count0=1\end",
        );
        let mut observations = ObservationRecorder::default();
        run_to_end_observed(&mut control, stores, &mut observations);

        assert_eq!(stores.count(0).expect("count register"), 1);
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
            terminal_text(stores).contains("! Too late for \\patterns."),
            "{}",
            terminal_text(stores)
        );
    });
}
#[test]
fn initex_late_patterns_prompts_at_the_pre_scan_section_960_context() {
    // TeX82 §960 calls §82's `error` before §473 scans and discards the
    // braced group. A deferred executor report must therefore carry the
    // source cursor immediately after `\patterns`, not the post-group cursor.
    crate::test_harness::with_plain_universe(|stores| {
        stores
            .world_mut()
            .push_memory_terminal_line("s")
            .expect("memory terminal accepts the error response");
        let mut control = MainControl::tex82_initex(stores);
        admitted!(stores, |context| context.close_hyphenation_patterns());
        register_source(&mut control, b"\\patterns{toolate}\\count0=1\\end");

        run_to_end(&mut control, stores);

        assert_eq!(
            stores.count(0).expect("count register"),
            1,
            "interactive recovery resumes input"
        );
        let output = terminal_text(stores);
        let context = output
            .find("! Too late for \\patterns.\nl.1 \\patterns\n")
            .expect("§960 reports at the pre-scan source cursor");
        let prompt = output.find("? ").expect("§82 interactive prompt");
        assert!(context < prompt, "{output}");
    });
}
#[test]
fn hyphenation_diagnostics_preserve_tex82_recovery_and_apply_order() {
    // TeX82 §§936-937 and §§961-963: scanner othercases retain the
    // partially collected word; invalid lccodes are diagnosed during apply;
    // a duplicate is diagnosed after its replacement has been installed.
    // The schema-v1 TeX82 instrumentation publishes no diagnostic event for
    // either the scanner or apply sites.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
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
                .step_with_observer(stores, &mut observations)
                .expect("program executes")
            {
                MainControlStep::End | MainControlStep::EndOfInput => break,
                MainControlStep::Continue => {}
            }
        }

        assert_eq!(stores.count(0).expect("count register"), 1);
        assert!(
            !observations
                .0
                .iter()
                .any(|event| matches!(event, CommandObservation::Diagnostic(_))),
            "§§936/961/963/966 have no schema-v1 diagnostic observation"
        );
        let output = terminal_text(stores);
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
    });
}
#[test]
fn nonletter_zero_pattern_uses_the_edge_sentinel() {
    // TeX82 §962 retains `cur_chr=0` after diagnosing the `0` whose lccode is
    // zero. It therefore anchors AA1b3 at the word edge. The duplicate bb/bb1
    // and overlapping 0B2B0 patterns are negative controls for max-level
    // resolution: only the maximal odd positions survive.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
        &mut control,
        b"\\nonstopmode \\lccode`A=1 \\chardef\\?=`b \\patterns{\\?50AA1b3 bb bb1 0B2B0 b1c}\\end",
    );

        run_to_end(&mut control, stores);

        let word = "\u{1}\u{1}bbbbc\u{1}c\u{1}";
        assert_eq!(
            admitted!(stores, |context| context
                .hyphen_positions_for_language(0, word, 2, 3)),
            [2, 3, 6],
            "{}",
            terminal_text(stores)
        );
    });
}
#[test]
fn bad_patterns_reports_the_live_section_961_source_context() {
    // TeX82 §961 calls §82's `error` immediately after `get_x_token`
    // classifies the offending command. The context cursor is therefore
    // immediately after `\relax`, before scanning resumes.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            b"\\nonstopmode\n\\patterns{ab\\relax cd}\n\\end",
        );

        run_to_end(&mut control, stores);

        let output = terminal_text(stores);
        assert!(
            output.contains(
                "! Bad \\patterns.\nl.2 \\patterns{ab\\relax\n                       cd}"
            ),
            "§82 must render the source cursor at §961's offending command: {output}"
        );
    });
}
#[test]
fn pattern_nonletter_prompts_at_the_live_section_962_source_context() {
    // TeX82 §962 calls §82's `error` before the next `get_x_token`, while
    // the nonletter and the source cursor immediately after it are live.
    // Delaying this report until the whole group has scanned makes the
    // interaction consume its response after unrelated pattern input.
    crate::test_harness::with_plain_universe(|stores| {
        stores
            .world_mut()
            .push_memory_terminal_line("s")
            .expect("memory terminal accepts the error response");
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, b"\\patterns{ab!cd ef1gh}\\count0=1\\end");

        run_to_end(&mut control, stores);

        assert_eq!(
            stores.count(0).expect("count register"),
            1,
            "interactive recovery resumes input"
        );
        let output = terminal_text(stores);
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
    });
}
#[test]
fn duplicate_pattern_prompts_at_the_live_section_963_separator_context() {
    // TeX82 §963 tests trie_o[q] and calls §82 before the §961 loop asks for
    // another token. The separator is therefore still current, and an
    // interactive response must not be consumed from later source input.
    crate::test_harness::with_plain_universe(|stores| {
        stores
            .world_mut()
            .push_memory_terminal_line("s")
            .expect("memory terminal accepts the error response");
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, b"\\patterns{a1b a2b next}\\count0=1\\end");

        run_to_end(&mut control, stores);

        assert_eq!(
            stores.count(0).expect("count register"),
            1,
            "interactive recovery resumes input"
        );
        let output = terminal_text(stores);
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
    });
}
#[test]
fn distinct_pattern_paths_do_not_report_section_963_duplicate() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            b"\\nonstopmode\\patterns{a1b a2c}\\count0=1\\end",
        );

        run_to_end(&mut control, stores);

        assert_eq!(stores.count(0).expect("count register"), 1);
        assert!(
            !terminal_text(stores).contains("! Duplicate pattern."),
            "different trie paths are the negative control"
        );
    });
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
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            let mut control = MainControl::tex82_initex(stores);
            register_source(
                &mut control,
                format!("\\nonstopmode\\patterns{{{patterns}}}\\count0=1\\end").as_bytes(),
            );

            run_to_end(&mut control, stores);

            assert_eq!(stores.count(0).expect("count register"), 1, "{patterns}");
            assert_eq!(
                terminal_text(stores)
                    .matches("! Duplicate pattern.")
                    .count(),
                expected_duplicates,
                "{patterns}"
            );
        });
    }
}
#[test]
fn operationless_pattern_path_is_not_a_section_963_duplicate() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            b"\\nonstopmode\\patterns{bb bb1 b2b}\\count0=1\\end",
        );

        run_to_end(&mut control, stores);

        assert_eq!(stores.count(0).expect("count register"), 1);
        assert_eq!(
            terminal_text(stores)
                .matches("! Duplicate pattern.")
                .count(),
            1,
            "only the second real trie operation on the shared path is duplicate"
        );
    });
}
#[test]
fn pattern_duplicate_paths_are_partitioned_by_language() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            b"\\nonstopmode\\language=1\\patterns{b1b}\\language=2\\patterns{b2b}\\count0=1\\end",
        );

        run_to_end(&mut control, stores);

        assert_eq!(stores.count(0).expect("count register"), 1);
        assert_eq!(
            terminal_text(stores)
                .matches("! Duplicate pattern.")
                .count(),
            0
        );
    });
}
#[test]
fn committed_and_pending_pattern_paths_share_replacement_order() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        assert!(
            !admitted!(stores, |context| context
                .add_hyphenation_pattern_for_language(
                    0,
                    PatternSpec {
                        letters: vec!['b', 'b'],
                        values: vec![0, 1, 0],
                    },
                ))
            .expect("pattern fits the default trie capacity")
        );
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            b"\\nonstopmode\\patterns{bb b2b}\\count0=1\\end",
        );

        run_to_end(&mut control, stores);

        assert_eq!(stores.count(0).expect("count register"), 1);
        assert_eq!(
            terminal_text(stores)
                .matches("! Duplicate pattern.")
                .count(),
            1,
            "committed real is diagnosed, its operationless replacement clears the pending view, and the following real is accepted"
        );
    });
}
#[test]
fn first_pattern_digit_is_a_level_not_a_section_962_nonletter() {
    // TeX82 §962's `digit_sensed=false` branch treats the first ASCII digit
    // as a hyphen level and therefore never consults its zero `\lccode`.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            b"\\nonstopmode\\patterns{ab1cd}\\count0=1\\end",
        );

        run_to_end(&mut control, stores);

        assert_eq!(stores.count(0).expect("count register"), 1);
        assert!(
            !terminal_text(stores).contains("! Nonletter."),
            "a hyphen-level digit is the negative control"
        );
    });
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
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            let mut control = MainControl::tex82_initex(stores);
            let source = format!(
                "\\nonstopmode\\patterns{{{}{suffix}}}\\count0=1\\end",
                "a".repeat(letters)
            );
            register_source(&mut control, source.as_bytes());

            run_to_end(&mut control, stores);

            assert_eq!(
                stores.count(0).expect("count register"),
                1,
                "letters={letters}, suffix={suffix}"
            );
            assert_eq!(
                terminal_text(stores).matches("! Nonletter.").count(),
                expected_nonletters,
                "letters={letters}, suffix={suffix}: {}",
                terminal_text(stores)
            );
        });
    }
}
