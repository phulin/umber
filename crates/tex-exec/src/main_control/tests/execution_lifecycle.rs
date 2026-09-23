//! Execution lifecycle and command transitions that cross multiple families.

use super::*;

#[test]
fn macro_trace_preserves_a_non_hash_parameter_marker() {
    // TeX82 §§389 prints the actual match-token character retained by
    // §476. TRIP makes `U` a parameter character and relies on `U3`, rather
    // than a duplicated `UU#3`, in the invocation trace.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\catcode`U=6 \def\m U1{OK}
\tracingonline=1\tracingmacros=1 \m X\end",
        );

        run_to_end(&mut control, stores);

        let output = terminal_text(stores);
        assert!(output.contains("\\m U1->OK"), "{output}");
        assert!(!output.contains("\\m UU#1->OK"), "{output}");
    });
}
#[test]
fn meaning_expansion_reports_chardef_meanings_as_hex_char_commands() {
    // TeX82 §1223's `char_given` `print_cmd_chr` arm prints `\char` and a
    // hexadecimal operand (tex.web lines 22876--22899). Both a printable
    // value and a control-code value must use that syntax: macro packages
    // parse the latter to recover encoded font slots.
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
                br#"\chardef\printable="41 \chardef\encoded="16
                    \message{[\meaning\printable][\meaning\encoded]}\end"#,
            );

            run_to_end(&mut control, stores);

            let output = terminal_text(stores);
            assert!(
                output.contains(r#"[\char"41][\char"16]"#),
                "{profile:?}: {output}"
            );
        });
    }
}
#[test]
fn etex_fire_up_distinguishes_empty_class_zero_and_sparse_botmarks() {
    // TeX82 §1012 preserves an empty class-zero `bot_mark` pointer as the new
    // `top_mark`, while e-TeX 2.6 `etex.ch` [26.1396] discards an empty old
    // sparse `botmarks` pointer. Only the later `topmarks0` enquiry therefore
    // installs and retires a `mark_text` input level.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        // Stage the exact post-fire-up state proved by page_output.rs's white-box
        // regression, then cross the command processor's enquiry boundary.
        admitted!(stores, |context| context.set_page_mark_class(
            PageMark::Top,
            0,
            tex_state::node::NodeTokenList::default(),
        ));
        let mut control = etex_initex(stores);
        register_source(
            &mut control,
            include_bytes!("../../fixtures/etex-empty-botmark-fire-up.tex"),
        );
        let mut observations = ObservationRecorder::default();

        run_to_end_observed(&mut control, stores, &mut observations);

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
    });
}
#[test]
fn etex_unexpanded_replays_protected_macros_as_ordinary_expandable_input() {
    // e-TeX 2.6 change section [27.465] implements `\unexpanded` through
    // `the_toks`, whose `ins_list` result re-enters the enclosing expansion
    // loop. Protection suppresses expansion only while an expanded token
    // list is being built; it is not persistent replay metadata.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = etex_initex(stores);
        register_source(
            &mut control,
            br"\protected\def\p{\global\advance\count0 by1}\unexpanded{\p}\end",
        );
        let mut observations = ObservationRecorder::default();

        run_to_end_observed(&mut control, stores, &mut observations);

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
        assert_eq!(
            stores.count(0).expect("count register"),
            1,
            "terminal: {}",
            terminal_text(stores)
        );
    });
}
#[test]
fn dimension_fraction_unit_and_internal_second_operand_scan_exact_phases() {
    for (source, resources, expected) in [
        (
            br"\def\gobble#1X{}\def\pa{\expandafter\gobble\pdffilesize{second}X}\def\pb{\expandafter\gobble\pdffilesize{third}X}\dimen0=1.2\pa3\pb4pt\message{[\the\dimen0]}\end"
                .as_slice(),
            &["second", "third"][..],
            "[1.234pt]",
        ),
        (
            br"\def\gobble#1X{}\def\pause{\expandafter\gobble\pdffilesize{second}X}\dimen0=1c\pause m\message{[\the\dimen0]}\end"
                .as_slice(),
            &["second"][..],
            "[28.45274pt]",
        ),
        (
            br"\def\gobble#1X{}\def\pause{\expandafter\gobble\pdffilesize{second}X}\dimen0=\fontdimen1\pause\font\message{[\the\dimen0]}\end"
                .as_slice(),
            &["second"][..],
            "[0.0pt]",
        ),
    ] {
        let preloaded_terminal = run_pdftex_file_probe_job(source, resources);
        assert!(
            preloaded_terminal.contains(expected),
            "{preloaded_terminal:?}"
        );

    }
}
#[test]
fn nested_file_enquiries_execute_through_their_typed_owners() {
    let source = br"\edef\result{\pdfmdfivesum file{\pdffilesize{first}}}\message{[\result]}\end";

    let preloaded_terminal = run_pdftex_file_probe_job(source, &["first", "4"]);
    assert!(preloaded_terminal.contains("[2C9B682412689D6723E3B31653B5774C]"));
}
#[test]
fn prefix_fetch_preserves_earlier_flags_and_its_exact_expansion_child() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = pdftex_initex(stores);
        register_source(
            &mut control,
            br"
\long\def\afterprobe#1X{\global}
\def\nextprefix{\expandafter\afterprobe\pdffilesize{first}X}
\def\start{\long\protected\nextprefix\xdef}
\start\result{ok}
\end
",
        );
        register_named_file_size_probe(&mut control, "first", b"ABCD");
        run_to_end(&mut control, stores);

        admitted!(stores, |context| {
            let result = context.intern_control_sequence("result");
            let ResolvedMeaning::Macro { flags, .. } = context.meaning(result) else {
                panic!("prefix-fetch continuation installs the definition")
            };
            assert!(flags.contains(MeaningFlags::LONG));
            assert!(flags.contains(MeaningFlags::PROTECTED));
        });
    });
}
#[test]
fn case_shift_preserves_raw_token_structure_at_code_table_boundaries() {
    // TeX82 §§1285--1289 scan unexpanded general text. §1288 substitutes
    // only character-token codes, preserving their command/category; zero
    // table entries and control-sequence tokens remain byte-for-byte tokens.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
        &mut control,
        br"\uccode`!=`Z\lccode`?=`y\catcode126=13\uccode126=88\uppercase{\gdef\up{!\relax}}\lowercase{\gdef\down{?\relax}}\uppercase{\gdef\active{~}}\uppercase{\gdef\zero{@}}\end",
    );
        run_to_end(&mut control, stores);
        assert!(matches!(
            macro_semantic_tokens(stores, "up").as_slice(),
            [
                Token::Char {
                    ch: 'Z',
                    cat: Catcode::Other
                },
                Token::Cs(_)
            ]
        ));
        assert!(matches!(
            macro_semantic_tokens(stores, "down").as_slice(),
            [
                Token::Char {
                    ch: 'y',
                    cat: Catcode::Other
                },
                Token::Cs(_)
            ]
        ));
        assert!(matches!(
            macro_semantic_tokens(stores, "active").as_slice(),
            [Token::Char {
                ch: 'X',
                cat: Catcode::Active
            }]
        ));
        assert!(matches!(
            macro_semantic_tokens(stores, "zero").as_slice(),
            [Token::Char { ch: '@', .. }]
        ));
    });
}
#[test]
fn openin_supplies_the_default_tex_extension() {
    // TeX82 §1275's `if cur_ext="" then cur_ext:=".tex"; pack_cur_name`.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        control.capabilities_mut().register_input(
            "child.tex",
            SourceRegistration::new(RegisteredSourceKind::World, Arc::<[u8]>::from(&b"body"[..])),
        );
        register_source(&mut control, br"\openin1=child \read1 to \line\end");
        run_to_end(&mut control, stores);

        let text = admitted!(stores, |context| {
            let line = context.intern_control_sequence("line");
            let ResolvedMeaning::Macro { definition, .. } = context.meaning(line) else {
                panic!("read defined its target")
            };
            context
                .definition(definition)
                .replacement_text()
                .iter()
                .filter_map(|word| match word.semantic_token() {
                    Token::Char { ch, .. } => Some(ch),
                    _ => None,
                })
                .collect::<String>()
        });
        // TeX82 §240's `\endlinechar` is appended to the line, but §348's
        // ⟨Finish line, emit a space⟩ tokenizes it as `cur_cmd:=spacer;
        // cur_chr:=" "` -- the trailing token is a space, never the raw byte.
        assert_eq!(text, "body ");
    });
}
#[test]
fn interaction_transition_prints_its_unconditional_newline_after_the_command_trace() {
    // TeX82 §§1030/1264: `show_cur_cmd_chr` completes before
    // `new_interaction` performs its unconditional `print_ln` under the old
    // selector. Detached trace publication must preserve that call order;
    // otherwise the resulting blank line moves before `\batchmode`.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\tracingcommands=1\batchmode\output={}\end");
        run_to_end_observed(&mut control, stores, &mut ObservationRecorder::default());

        let log = pending_sink_text(stores, false);
        assert!(
            log.contains("{vertical mode: \\batchmode}\n\n{\\output}"),
            "{log}"
        );
        assert!(
            !log.contains("\n\n{vertical mode: \\batchmode}\n{\\output}"),
            "the §1264 newline must not overtake the trace: {log}"
        );
    });
}
#[test]
fn interactionmode_rejects_out_of_range_values_without_changing_mode() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        tex_command::install_tex82_expandable_primitives(stores);
        tex_command::install_etex_expandable_primitives(stores);
        crate::install_unexpandable_primitives(stores);
        crate::install_etex_unexpandable_primitives(stores);
        stores.set_interaction_mode(tex_state::InteractionMode::Nonstop);
        let mut control = MainControl::prepared_initex(CommandProfile::ETEX26);
        register_source(
            &mut control,
            br"\interactionmode=-1\edef\result{\the\interactionmode}",
        );
        run_to_end(&mut control, stores);
        assert_eq!(macro_character_text(stores, "result"), "1");
        assert!(terminal_text(stores).contains("Bad interaction mode (-1)"));
    });
}
#[test]
fn protected_prefix_resumes_command_demand_after_unexpanded_tokens() {
    with_etex(
        br"\let\bgroup={\protected\def\two{}\let\three=\two\protected\unexpanded\bgroup\two\protected\three\protected\def\one{\two}}",
        |stores| {
            admitted!(stores, |context| {
                let one = context.intern_control_sequence("one");
                let ResolvedMeaning::Macro { definition, flags } = context.meaning(one) else {
                    panic!("one is defined")
                };
                assert!(flags.contains(tex_state::meaning::MeaningFlags::PROTECTED));
                assert_eq!(context.definition(definition).replacement_text().len(), 1);
            });
            assert!(!terminal_text(stores).contains("You can't use a prefix"));
        },
    );
}
#[test]
fn global_prefix_resumes_command_demand_inside_unexpanded_tokens() {
    with_etex(
        br"\let\flag\iftrue\def\setfalse{\let\flag\iffalse}\begingroup\global\unexpanded{\setfalse}\endgroup",
        |stores| {
            assert_eq!(
                admitted!(stores, |context| {
                    let flag = context.intern_control_sequence("flag");
                    context.meaning(flag)
                }),
                ResolvedMeaning::Static(Meaning::ExpandablePrimitive(
                    tex_state::meaning::ExpandablePrimitive::IfFalse,
                ))
            );
            assert!(!terminal_text(stores).contains("You can't use a prefix"));
        },
    );
}
