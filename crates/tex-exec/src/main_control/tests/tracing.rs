//! Command, macro, assignment, and restoration tracing on live output channels.

use super::*;

#[test]
fn tracingcommands_reports_only_big_switch_commands_with_live_selector_and_mode() {
    // TeX82 §§299/1030/1211: `show_cur_cmd_chr` runs after `big_switch`'s
    // fetch, not at `reswitch`. Thus only the first prefix is traced; later
    // prefixes and the target are fetched within `prefixed_command`. The
    // `\tracingonline` trace is log-only because that assignment has not yet
    // executed, while the prefix uses the newly live terminal-and-log selector.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            b"\\tracingcommands=1\\tracingonline=1\\global\\global\\escapechar=64\\end",
        );

        run_to_end(&mut control, stores);

        let terminal = pending_sink_text(stores, true);
        let log = pending_sink_text(stores, false);
        assert!(!terminal.contains("tracingonline"));
        assert!(log.contains("{vertical mode: \\tracingonline}"));
        assert!(terminal.contains("{\\global}\n{@end}"), "{terminal:?}");
        assert!(log.contains("{\\global}\n{@end}"), "{log:?}");
        assert!(!terminal.contains("escapechar"), "{terminal:?}");
        assert!(terminal.contains("{@end}"), "{terminal:?}");
    });
}
#[test]
fn tracingcommands_two_traces_nonmacro_expansion_before_big_switch_result() {
    // TeX82 §§299/366--367/1030: non-macro expansion traces inside `expand`,
    // then the settled unexpandable command traces at `reswitch`. The first
    // trace consumes the mode prefix; the second must not repeat it.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            b"\\tracingcommands=2\\tracingonline=1\\romannumeral0\\relax\\end",
        );

        run_to_end(&mut control, stores);

        let terminal = pending_sink_text(stores, true);
        let log = pending_sink_text(stores, false);
        assert!(
            log.contains("{vertical mode: \\tracingonline}\n{\\romannumeral}\n{\\relax}\n{\\end}"),
            "terminal={terminal:?} log={log:?}"
        );
        assert!(!terminal.contains("romannumeral"), "{terminal:?}");
    });
}
#[test]
fn tracingcommands_preserves_shown_mode_across_expansion_diagnostic_barrier() {
    // TeX82 §§299/367/370: tracing an undefined control sequence consumes
    // the mode prefix before §370 reports its recoverable error. Resuming the
    // settled command after that report must retain `shown_mode` rather than
    // print the restricted-horizontal prefix a second time.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        stores.set_interaction_mode(tex_state::InteractionMode::Nonstop);
        let mut control = MainControl::tex82_initex(stores);
        let undefined = stores.intern("undefined").expect("undefined symbol");
        assign_static_meaning(stores, undefined, Meaning::Undefined);
        register_source(
            &mut control,
            b"\\tracingcommands=2\\tracingonline=1\\hbox{\\undefined\\relax}\\end",
        );

        run_to_end(&mut control, stores);

        let log = terminal_text(stores);
        assert!(
            log.contains("{restricted horizontal mode: undefined}"),
            "{log}"
        );
        assert!(log.contains("{\\relax}"), "{log}");
        assert_eq!(
            log.matches("restricted horizontal mode:").count(),
            1,
            "the expansion trace, not the post-diagnostic command, owns the sole mode prefix: {log}"
        );
    });
}
#[test]
fn tracingcommands_expansion_after_eqno_reports_restored_display_mode() {
    // TeX82 §§299/1193: the math shift finishes the equation-number mlist in
    // ordinary math mode, then `fin_mlist` restores the enclosing display
    // before `get_x_token` expands the next command. Section 367 must compare
    // that restored mode with `shown_mode` and print the new mode prefix.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        stores.set_interaction_mode(tex_state::InteractionMode::Nonstop);
        register_source(
            &mut control,
            br"\def\s{{\tracingcommands=0\showlists}}\tracingcommands=2\tracingrestores=2\tracingonline=1 $$x\eqno y\s$\expandafter$\csname!\endcsname\end",
        );

        run_to_end(&mut control, stores);

        let log = terminal_text(stores);
        let restore = log
            .find("{restoring \\tracingcommands=2}")
            .unwrap_or_else(|| panic!("nested diagnostic group restores tracing: {log}"));
        let eqno_shift = restore
            + log[restore..]
                .find("{math shift character $}")
                .unwrap_or_else(|| panic!("equation-number closer is traced: {log}"));
        let restored = log
            .find("{display math mode: \\expandafter}\n{\\csname}")
            .unwrap_or_else(|| panic!("restored display expansion is traced: {log}"));
        assert!(restore < eqno_shift && eqno_shift < restored, "{log}");
        assert_eq!(log.matches("\\expandafter}").count(), 1, "{log}");
        assert_eq!(log.matches("{\\csname}").count(), 1, "{log}");
    });
}
#[test]
fn tracingcommands_aftergroup_expansion_reports_resumed_horizontal_mode() {
    // TeX82 §§299/1200: ending the display releases its aftergroup token,
    // pushes horizontal mode, and then expands that token while scanning the
    // optional space. This is a distinct nested expansion boundary from
    // §1197's display-mode second-$ probe above, and consumes the new mode
    // prefix exactly once.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        stores.set_interaction_mode(tex_state::InteractionMode::Nonstop);
        register_source(
            &mut control,
            br"\tracingcommands=2\tracingonline=1 $$x\aftergroup\expandafter\eqno y$\expandafter$\csname!\endcsname\end",
        );

        run_to_end(&mut control, stores);

        let log = terminal_text(stores);
        let display = log
            .find("{display math mode: \\expandafter}")
            .unwrap_or_else(|| panic!("display probe owns its prefix: {log}"));
        let horizontal = log
            .find("{horizontal mode: \\expandafter}")
            .unwrap_or_else(|| panic!("optional-space probe owns its prefix: {log}"));
        assert!(display < horizontal, "{log}");
        assert_eq!(log.matches("\\expandafter}").count(), 2, "{log}");
        assert!(!log.contains("{\\expandafter}"), "{log}");
        assert!(log.contains("{undefined}"), "{log}");
    });
}
#[test]
fn tracingcommands_omits_characters_retired_inside_main_loop() {
    // TeX82 §§1034/1038: after the first character enters `main_loop`,
    // adjacent characters are retired by its raw lookahead and never reach
    // §1030's `reswitch` trace boundary.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_cmr10_as(&mut control, stores, "cmr10.tfm");
        register_source(
        &mut control,
        br"\font\f=cmr10 \f\chardef\bee=66 \tracingcommands=1\tracingonline=1\setbox0=\hbox{AA\bee\char67}\end",
    );

        run_to_end(&mut control, stores);

        let log = pending_sink_text(stores, false);
        assert_eq!(log.matches("the letter A").count(), 1, "{log}");
        assert!(!log.contains("the letter B"), "{log}");
        assert!(!log.contains(r"{\char"), "{log}");
        assert!(log.contains("{end-group character }}"), "{log}");
    });
}
#[test]
fn tracingcommands_precedes_recovery_reported_while_scanning_the_command() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            b"\\tracingcommands=1 \\tracingonline=1 \\openout-1=trace.out\\end",
        );

        run_to_end(&mut control, stores);

        let output = terminal_text(stores);
        let trace = output
            .find("{\\openout}")
            .unwrap_or_else(|| panic!("§1030 command trace: {output:?}"));
        let error = output.find("! Bad number (-1).").expect("§435 recovery");
        assert!(trace < error, "{output:?}");
    });
}
#[test]
fn tracingcommands_caret_renders_a_nonprintable_live_escapechar() {
    // TeX82 §§58--59/63/298: `print_cmd_chr` reaches `print_esc`, whose
    // escape prefix is printed as a one-character string rather than by the
    // raw `print_char` primitive.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            b"\\tracingcommands=1\\tracingonline=1\\escapechar=127\\global\\count0=1\\end",
        );

        run_to_end(&mut control, stores);

        let terminal = pending_sink_text(stores, true);
        assert!(terminal.contains("{^^?global}\n{^^?end}"), "{terminal:?}");
        assert!(!terminal.contains("count"), "{terminal:?}");
        assert!(!terminal.as_bytes().contains(&127), "{terminal:?}");
    });
}
#[test]
fn tracingcommands_traces_reswitch_but_not_prefixed_command_internal_fetches() {
    // TeX82 §§1030/1045/1211: `reswitch` precedes the diagnostic boundary, so
    // the command fetched by `\ignorespaces` is traced. A later prefix and
    // its target are fetched inside `prefixed_command` and remain untraced.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
        &mut control,
        b"\\tracingcommands=1\\tracingonline=1\\global\\global\\count0=1\\ignorespaces\\relax\\end",
    );

        run_to_end(&mut control, stores);

        let terminal = pending_sink_text(stores, true);
        assert!(
            terminal.contains("{\\global}\n{\\ignorespaces}\n{\\relax}\n{\\end}"),
            "{terminal:?}"
        );
        assert!(!terminal.contains("count"), "{terminal:?}");
    });
}
#[test]
fn disabled_tracingcommands_emits_no_command_diagnostic() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, b"\\tracingonline=1\\escapechar=64\\end");

        run_to_end(&mut control, stores);

        assert!(!pending_sink_text(stores, true).contains("vertical mode:"));
        assert!(!pending_sink_text(stores, false).contains("vertical mode:"));
    });
}
#[test]
fn tracingcommands_does_not_trace_constructed_leader_glue_internal_fetch() {
    // TeX82 §§1030/1078: `box_end` fetches a constructed leader's glue
    // operand inside the leader case, without returning to `big_switch`'s
    // `show_cur_cmd_chr`. A later ordinary `\hskip` remains a main-control
    // command and is the negative control.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\tracingcommands=1\tracingonline=1
\setbox0=\hbox{\leaders\hbox{}\hskip1pt\hskip2pt}
\end",
        );

        run_to_end(&mut control, stores);

        let terminal = pending_sink_text(stores, true);
        assert!(terminal.contains("\\leaders}"), "{terminal:?}");
        assert_eq!(
            terminal.matches("{\\hskip}").count(),
            1,
            "only the ordinary post-leader hskip reaches §1030: {terminal:?}"
        );
    });
}
#[test]
fn tracingmacros_two_traces_the_named_output_token_list() {
    // TeX82 §§323/1025: `begin_token_list(output_routine,output_text)` traces
    // the named token-list parameter only at the stronger tracing level.
    for (level, expected) in [(1, false), (2, true)] {
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            stores.set_interaction_mode(tex_state::InteractionMode::Nonstop);
            let mut control = MainControl::tex82_initex(stores);
            register_source(
            &mut control,
            format!(
                "\\tracingmacros={level}\\tracingonline=1\n\\maxdeadcycles=1\\output={{\\dimen0=1pt}}\n\\topskip=0pt\\setbox0=\\vbox to1pt{{}}\\copy0\\penalty-10000\\end"
            )
            .as_bytes(),
        );

            run_to_end(&mut control, stores);

            let terminal = terminal_text(stores);
            assert_eq!(
                terminal.contains("\\output->{\\dimen 0=1pt}"),
                expected,
                "tracingmacros={level}: {terminal:?}"
            );
            assert!(
                !terminal.contains("\n\n\\output->"),
                "named-list tracing must use §323's conditional newline: {terminal:?}"
            );
        });
    }
}
#[test]
fn tracingmacros_reports_definition_then_arguments_with_live_routing() {
    // TeX82 §§389/400 and §245: the invocation line precedes completed
    // arguments and the live selector controls both routed copies.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            b"\\def\\pair#1#2{}\\tracingmacros=1 \\tracingonline=1 \\pair CD\\end",
        );

        run_to_end(&mut control, stores);

        let terminal = pending_sink_text(stores, true);
        let log = pending_sink_text(stores, false);
        let expected = "\n\\pair #1#2->\n#1<-C\n#2<-D\n";
        assert_eq!(terminal, expected);
        assert_eq!(log, expected);

        crate::test_harness::with_nonstop_plain_universe(|stores| {
            let mut control = MainControl::tex82_initex(stores);
            register_source(
                &mut control,
                b"\\def\\pair#1#2{}\\tracingmacros=1 \\pair AB\\end",
            );
            run_to_end(&mut control, stores);
            assert_eq!(
                pending_sink_text(stores, true),
                "(see the transcript file for additional information)"
            );
            assert_eq!(
                pending_sink_text(stores, false),
                "\n\\pair #1#2->\n#1<-A\n#2<-B\n"
            );
        });
    });
}
#[test]
fn tracingmacros_precedes_condition_result_during_operand_expansion() {
    // TeX82 §§389/400/498: `macro_call` prints the complete definition
    // before matching arguments. A macro expanded while `conditional` scans
    // an operand therefore precedes both its argument trace and the result.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\def\t#1{#1pt}\tracingcommands=2\tracingmacros=1\tracingonline=1
\ifdim\t1=1pt\relax\fi\end",
        );

        run_to_end(&mut control, stores);

        let terminal = terminal_text(stores);
        let invocation = terminal
            .find("\\t #1->#1pt")
            .expect("macro definition trace");
        let argument = terminal.find("#1<-1").expect("macro argument trace");
        let result = terminal.find("{true}").expect("conditional result trace");
        assert!(invocation < argument && argument < result, "{terminal:?}");
    });
}
#[test]
fn disabled_tracingmacros_emits_no_macro_diagnostic() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            b"\\def\\pair#1#2{}\\tracingonline=1\\pair AB\\end",
        );

        run_to_end(&mut control, stores);

        assert_eq!(pending_sink_text(stores, true), "");
        assert_eq!(pending_sink_text(stores, false), "");
    });
}
#[test]
fn tracingrestores_reports_exact_restoration_through_the_live_selector() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            b"\\tracingrestores=1\\tracingonline=1{\\count0=7}\\end",
        );

        run_to_end(&mut control, stores);

        assert_eq!(pending_sink_text(stores, true), "{restoring \\count0=0}\n");
        assert_eq!(pending_sink_text(stores, false), "{restoring \\count0=0}\n");
    });
}
#[test]
fn tracingrestores_uses_the_restored_gate_for_its_own_save_entry() {
    // TeX82 §283 restores the word before consulting `tracing_restores`.
    // The count entry is still suppressed while the local zero is live, then
    // restoring `\tracingrestores` to one makes that entry report itself.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\tracingrestores=1\tracingonline=1{\tracingrestores=0\count0=7}\end",
        );

        run_to_end(&mut control, stores);

        let expected = "{restoring \\tracingrestores=1}\n";
        assert_eq!(pending_sink_text(stores, true), expected);
        assert_eq!(pending_sink_text(stores, false), expected);
    });
}
#[test]
fn tracingrestores_preserves_nested_reverse_save_order_and_retained_values() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\tracingrestores=1\tracingonline=1
\count0=1
{\count0=2\skip0=1pt\toks0={outer}\def\foo{outer}
 {\count0=3\global\count0=4\skip0=2pt\toks0={inner}\def\foo{inner}}}
\end",
        );

        run_to_end(&mut control, stores);

        let expected = concat!(
            "{restoring \\foo=macro:->outer}\n",
            "{restoring \\toks0=outer}\n",
            "{restoring \\skip0=1.0pt}\n",
            "{retaining \\count0=4}\n",
            "{restoring \\foo=undefined}\n",
            "{restoring \\toks0=}\n",
            "{restoring \\skip0=0.0pt}\n",
            "{retaining \\count0=4}\n",
        );
        assert_eq!(pending_sink_text(stores, true), expected);
        assert_eq!(pending_sink_text(stores, false), expected);
    });
}
#[test]
fn tracingrestores_reports_dimension_register_restoration() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            b"\\tracingrestores=1\\tracingonline=1{\\dimen9=1.25pt}\\end",
        );

        run_to_end(&mut control, stores);

        assert_eq!(
            pending_sink_text(stores, true),
            "{restoring \\dimen9=0.0pt}\n"
        );
    });
}
#[test]
fn tracingrestores_projects_the_logical_parshape_cell() {
    // TeX82 §§252/283 reports the logical `\parshape` entry as its line
    // count. Umber's internal immutable byte payload is storage only and must
    // neither leak its token-parameter coordinate nor its encoded bytes.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\tracingrestores=1\tracingonline=1
\parshape=2 1pt 9pt 2pt 8pt
{\parshape=1 3pt 7pt}\end",
        );

        run_to_end(&mut control, stores);

        assert_eq!(
            pending_sink_text(stores, true),
            "{restoring \\parshape=2}\n"
        );
        assert_eq!(
            pending_sink_text(stores, false),
            "{restoring \\parshape=2}\n"
        );
    });
}
#[test]
fn tracingrestores_preserves_dense_and_sparse_register_unsave_order() {
    // e-TeX 2.6 [53a] keeps classic registers in eqtb and extended registers
    // in the sparse array. Its `unsave`/`sa_restore` interleaving is observable
    // through `\tracingrestores`; neither bank may disappear from the ordered
    // receipt.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = etex_initex(stores);
        register_source(
            &mut control,
            br"\tracingonline=1\begingroup\tracingrestores=1
\count20=5\count2000=5\dimen21=5pt\dimen2100=5pt
\skip22=5pt\relax\muskip2200=5mu\relax\endgroup\end",
        );

        run_to_end(&mut control, stores);

        let expected = concat!(
            "{restoring \\skip22=0.0pt}\n",
            "{restoring \\dimen21=0.0pt}\n",
            "{restoring \\muskip2200=0.0mu}\n",
            "{restoring \\dimen2100=0.0pt}\n",
            "{restoring \\count2000=0}\n",
            "{restoring \\count20=0}\n",
        );
        assert_eq!(pending_sink_text(stores, true), expected);
        assert_eq!(pending_sink_text(stores, false), expected);
    });
}
#[test]
fn tracingrestores_matches_etex_box_save_stack_oracle_cases() {
    // TeX82 §283 restores the ordinary save stack top-down, while e-TeX
    // [53a] restores extended registers through the first `restore_sa`
    // marker. Box registers must retain that same ordering even though their
    // node owners live in the durable box store.
    let cases = [
        (
            &br"\tracingrestores=1\tracingonline=1{\setbox0=\hbox{}{}\setbox300=\hbox{}}\end"[..],
            "{restoring \\box300=void}\n{restoring \\box0=void}\n",
            "{restoring \\box300=void}\n{restoring \\box0=void}\n",
        ),
        (
            &br"\tracingrestores=1\tracingonline=1{\setbox300=\hbox{}{}\setbox0=\hbox{}}\end"[..],
            "{restoring \\box0=void}\n{restoring \\box300=void}\n",
            "{restoring \\box0=void}\n{restoring \\box300=void}\n",
        ),
        (
            &br"\catcode`\{=1 \catcode`\}=2 \tracingrestores=1\tracingonline=1{\count0=1\setbox0=\hbox{}\count1=2\setbox1=\hbox{}}\end"[..],
            concat!(
                "{restoring \\box1=void}\n",
                "{restoring \\count1=0}\n",
                "{restoring \\box0=void}\n",
                "{restoring \\count0=0}\n",
            ),
            concat!(
                "{restoring \\box1=void}\n",
                "{restoring \\count1=0}\n",
                "{restoring \\box0=void}\n",
                "{restoring \\count0=0}\n",
            ),
        ),
        (
            &br"\catcode`\{=1 \catcode`\}=2 \tracingrestores=1\tracingonline=1{\count300=1\count0=1\setbox301=\hbox{}\count301=2\setbox0=\hbox{}}\end"[..],
            concat!(
                "{restoring \\box0=void}\n",
                "{restoring \\count0=0}\n",
                "{restoring \\count301=0}\n",
                "{restoring \\box301=void}\n",
                "{restoring \\count300=0}\n",
            ),
            concat!(
                "{restoring \\box0=void}\n",
                "{restoring \\count0=0}\n",
                "{restoring \\count301=0}\n",
                "{restoring \\box301=void}\n",
                "{restoring \\count300=0}\n",
            ),
        ),
        (
            &br"\catcode`\{=1 \catcode`\}=2 \tracingrestores=1\tracingonline=1{\setbox300=\hbox{}\count0=1\count301=2}\end"[..],
            concat!(
                "{restoring \\count0=0}\n",
                "{restoring \\count301=0}\n",
                "{restoring \\box300=void}\n",
            ),
            concat!(
                "{restoring \\count0=0}\n",
                "{restoring \\count301=0}\n",
                "{restoring \\box300=void}\n",
            ),
        ),
        (
            &br"\catcode`\{=1 \catcode`\}=2 {\tracingrestores=1\tracingonline=1\setbox25=\hbox{}\tracingassigns=1}\end"[..],
            concat!(
                "{restoring \\tracingassigns=0}\n",
                "{restoring \\box25=void}\n",
            ),
            concat!(
                "{restoring \\tracingassigns=0}\n",
                "{restoring \\box25=void}\n",
                "{restoring \\tracingonline=0}\n",
            ),
        ),
    ];

    for (source, expected_terminal, expected_log) in cases {
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            let mut control = etex_initex(stores);
            register_source(&mut control, source);
            run_to_end(&mut control, stores);
            let terminal = restoration_trace_lines(&pending_sink_text(stores, true));
            let log = restoration_trace_lines(&pending_sink_text(stores, false));
            assert_eq!(terminal, expected_terminal);
            assert_eq!(log, expected_log);
        });
    }
}
#[test]
fn tracingrestores_reports_code_table_restoration_and_retained_globals() {
    for (source, expected) in [
        (
            &br"\tracingrestores=1\tracingonline=1{\sfcode`B=1234}\end"[..],
            "{restoring \\sfcode66=999}\n",
        ),
        (
            &br"\tracingrestores=1\tracingonline=1{\sfcode`B=1234\global\sfcode`B=777}\end"[..],
            "{retaining \\sfcode66=777}\n",
        ),
    ] {
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            let mut control = MainControl::tex82_initex(stores);
            register_source(&mut control, source);

            run_to_end(&mut control, stores);

            assert_eq!(pending_sink_text(stores, true), expected);
            assert_eq!(pending_sink_text(stores, false), expected);
        });
    }
}
#[test]
fn tracingrestores_reports_current_font_selector_restoration() {
    // TeX82 §§252/283: `cur_font_loc` has the unescaped label `current font`,
    // followed by the restored font's frozen identifier, not the selector
    // token used to choose it. Loading a format also exercises frozen symbols.
    crate::test_harness::with_nonstop_plain_universe(|initialized| {
        let mut initex = MainControl::tex82_initex(initialized);
        register_cmr10_as(&mut initex, initialized, "cmr10.tfm");
        register_source(&mut initex, br"\font\f=cmr10 \font\g=cmr10 at 9pt \f\end");
        run_to_end(&mut initex, initialized);
        let stores = initialized;
        let mut control = MainControl::with_profile(CommandProfile::TEX82);
        register_source(
            &mut control,
            br"\let\alias=\g\tracingrestores=1\tracingonline=1{\alias}\end",
        );

        run_to_end(&mut control, stores);

        assert_eq!(
            pending_sink_text(stores, true),
            "{restoring current font=\\f}\n"
        );
    });
}
#[test]
fn tracingrestores_spells_active_character_names_without_an_escape() {
    // TeX82 §§252/263: region-1 `show_eqtb` uses `sprint_cs`, under which
    // an active-character control sequence prints as the bare character.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\catcode`\?=13 \tracingrestores=1\tracingonline=1{\def?{x}}\end",
        );

        run_to_end(&mut control, stores);

        assert_eq!(pending_sink_text(stores, true), "{restoring ?=undefined}\n");
    });
}
#[test]
fn tracingrestores_reports_math_family_font_restoration() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        stores.set_interaction_mode(tex_state::InteractionMode::Nonstop);
        let mut control = MainControl::tex82_initex(stores);
        register_cmr10_as(&mut control, stores, "cmr10.tfm");
        register_source(
        &mut control,
        br"\font\small=cmr10 \scriptfont2=\small \tracingrestores=1\tracingonline=1{\scriptfont2=\small}\end",
    );

        run_to_end(&mut control, stores);

        let expected = "{restoring \\scriptfont2=\\small}\n";
        let terminal = pending_sink_text(stores, true);
        let log = pending_sink_text(stores, false);
        assert!(
            terminal.contains(expected) && log.contains(expected),
            "terminal={terminal:?} log={log:?}"
        );
    });
}
#[test]
fn tracingrestores_prints_nonprintable_font_identifiers_through_the_live_selector() {
    // TeX82 §§59--60/252 and pdftex.web §252: `restore_trace` delegates to
    // `show_eqtb`, whose math-family font arm reaches `print_esc` and hence
    // `slow_print` for every byte of the frozen font identifier.  The two
    // `\newlinechar` cases challenge the printer rule rather than pinning a
    // replacement spelling for byte zero; the printable-name test above is
    // the control that must remain unchanged.
    for (newline_char, expected) in [
        (-1, "{restoring \\textfont3=\\bigtr^^@p}\n"),
        (0, "{restoring \\textfont3=\\bigtr\np}\n"),
    ] {
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            let mut control = MainControl::tex82_initex(stores);
            register_cmr10_as(&mut control, stores, "cmr10.tfm");
            register_source(
                &mut control,
                format!(
                    "\\catcode0=11 \\font\\bigtr^^@p=cmr10 \\font\\other=cmr10 at 9pt \\textfont3=\\bigtr^^@p \\newlinechar={newline_char} \\tracingrestores=1 \\tracingonline=1 {{\\textfont3=\\other}}\\end"
                )
                .as_bytes(),
            );

            run_to_end(&mut control, stores);

            let terminal = pending_sink_text(stores, true);
            let log = pending_sink_text(stores, false);
            assert_eq!(terminal, expected);
            assert_eq!(log, expected);
            assert!(!terminal.contains('\0'), "{terminal:?}");
            assert!(!log.contains('\0'), "{log:?}");
        });
    }
}
#[test]
fn tracingrestores_reports_restored_box_register_value() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            b"\\tracingrestores=1\\tracingonline=1\\setbox7=\\hbox{}{\\setbox7=\\vbox{}}\\end",
        );

        run_to_end(&mut control, stores);

        assert_eq!(
            pending_sink_text(stores, true),
            "{restoring \\box7=\n\\hbox(0.0+0.0)x0.0}\n"
        );
    });
}
#[test]
fn tracingrestores_prints_restored_void_box_inline() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            b"\\tracingrestores=1\\tracingonline=1{\\setbox254=\\hbox{}}\\end",
        );

        run_to_end(&mut control, stores);

        assert_eq!(
            pending_sink_text(stores, true),
            "{restoring \\box254=void}\n"
        );
    });
}
#[test]
fn tracingrestores_reports_value_before_first_local_box_assignment() {
    // TeX82 §§275/283 save a box only on its first local assignment at the
    // current level, then display that restored value after `unsave`.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
        &mut control,
        b"\\tracingrestores=1\\tracingonline=1\\setbox7=\\hbox{}{\\setbox7=\\vbox{}\\setbox7=\\hbox{X}}\\end",
    );

        run_to_end(&mut control, stores);

        assert_eq!(
            pending_sink_text(stores, true),
            "{restoring \\box7=\n\\hbox(0.0+0.0)x0.0}\n"
        );
    });
}
#[test]
fn tracingrestores_reports_retained_box_after_global_assignment() {
    // TeX82 §283 retains and displays a global value instead of reinstalling
    // the value saved by an earlier local assignment in the same group.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
        &mut control,
        b"\\tracingrestores=1\\tracingonline=1\\setbox7=\\vbox{}{\\setbox7=\\hbox{}\\global\\setbox7=\\hbox{X}}\\end",
    );

        run_to_end(&mut control, stores);

        assert_eq!(
            pending_sink_text(stores, true),
            "{retaining \\box7=\n\\hbox(0.0+0.0)x0.0}\n"
        );
    });
}
#[test]
fn tracingrestores_uses_live_value_after_refiling_a_global_box_save() {
    // TeX82 §§275/283 retain and display the effective global eqtb value.
    // The global save record refiled into the outer group also carries an
    // internal `old` redo word whose box has been retired with the inner
    // group's local assignment; that word is not a TeX save-stack value.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
        &mut control,
        b"\\tracingrestores=1\\tracingonline=1{{\\setbox7=\\vbox{}\\global\\setbox7=\\hbox{X}}}\\end",
    );

        run_to_end(&mut control, stores);

        assert_eq!(
            pending_sink_text(stores, true),
            "{retaining \\box7=\n\\hbox(0.0+0.0)x0.0}\n"
        );
    });
}
#[test]
fn tracingrestores_reports_retained_globals_and_obeys_routing_and_zero_suppression() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
        &mut control,
        b"{\\count0=7\\global\\count0=8}\\tracingrestores=1{\\count1=9\\global\\count1=10}{\\count2=11}\\tracingrestores=0{\\count3=12}\\end",
    );

        run_to_end(&mut control, stores);

        assert_eq!(
            pending_sink_text(stores, true),
            "(see the transcript file for additional information)"
        );
        assert_eq!(
            pending_sink_text(stores, false),
            "{retaining \\count1=10}\n{restoring \\count2=0}\n"
        );
    });
}
#[test]
fn tracingrestores_reports_retained_integer_parameter_with_live_escapechar() {
    // TeX82 §283 calls `restore_trace` for both retained and restored eqtb
    // words; §252's `show_eqtb` names integer parameters through `print_esc`.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            b"\\tracingrestores=1\\tracingonline=1{\\escapechar=127\\global\\escapechar=256}\\end",
        );

        run_to_end(&mut control, stores);

        assert_eq!(
            pending_sink_text(stores, true),
            "{retaining escapechar=256}\n"
        );
    });
}
#[test]
fn tracingrestores_reports_named_glue_parameters_with_exact_specs() {
    // TeX82 §§177/252/283: glue parameters use their §236 control-sequence
    // names and `print_spec` value, for both restored and globally retained
    // save-stack entries. The retained infinite-order component is the
    // negative control against formatting every component as ordinary `pt`.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
        &mut control,
        br"\tracingrestores=1\tracingonline=1{\lineskip=1pt plus 2fil minus 3pt}{\baselineskip=1pt\global\baselineskip=4pt plus 5fill}\end",
    );

        run_to_end(&mut control, stores);

        let expected =
            "{restoring \\lineskip=0.0pt}\n{retaining \\baselineskip=4.0pt plus 5.0fill}\n";
        assert_eq!(pending_sink_text(stores, true), expected);
        assert_eq!(pending_sink_text(stores, false), expected);
    });
}
#[test]
fn tracingassigns_global_glue_arithmetic_keeps_the_displaced_spec_live() {
    // e-TeX 2.6 [19.277--279] traces the pre-image before `geq_define`
    // destroys it and the post-image after the write. A global assignment has
    // no save-stack root, so the combined Umber boundary must retain the old
    // glue spec operation-locally while rendering both observations.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = etex_initex(stores);
        register_source(
            &mut control,
            br"\skip0=1pt\tracingonline=1\tracingassigns=1\global\advance\skip0 by 2pt\end",
        );

        run_to_end(&mut control, stores);

        let expected = concat!(
            "{into \\tracingassigns=1}\n",
            "{globally changing \\skip0=1.0pt}\n",
            "{into \\skip0=3.0pt}\n",
        );
        assert_eq!(pending_sink_text(stores, true), expected);
        assert_eq!(pending_sink_text(stores, false), expected);
    });
}
#[test]
fn etex_sparse_toks_restore_tracing_decodes_register_words_without_parameter_offset() {
    // e-TeX [53a] saves a sparse token-register pointer and restores that
    // exact value before tracing it through `show_sa`; unlike token-parameter
    // cells, register words encode `TokenListId` directly. TeX82 §§252/283
    // likewise show the just-restored value. The preceding nonempty list
    // detects an erroneous optional-parameter offset, while the empty
    // `\toks2200` restoration is the zero-word negative control.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = etex_initex(stores);
        register_source(
        &mut control,
        br"\toks2001={a b c}\toks2002={d e f}\tracingrestores=1\tracingonline=1{\toks2002=\toks2001\toks2200=\toks2001}\end",
    );

        run_to_end(&mut control, stores);

        let expected = concat!(
            "{restoring \\toks2200=}\n",
            "{restoring \\toks2002=d e f}\n",
        );
        assert_eq!(pending_sink_text(stores, true), expected);
        assert_eq!(pending_sink_text(stores, false), expected);
    });
}
#[test]
fn tracingrestores_keeps_a_control_sequence_atomic_at_the_show_token_list_breadth() {
    // TeX82 §§252/262/283: the 32-character `show_token_list` bound is tested
    // before a token is printed. `\outputpenalty` starts below the bound and
    // must therefore be printed whole before the remaining suffix becomes
    // `\ETC.`; clipping the control-sequence spelling is not a legal trace.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\tracingrestores=1\tracingonline=1\output={\tracingcommands 0\showthe \outputpenalty x}{\output={}}
\end",
        );

        run_to_end(&mut control, stores);

        let terminal = pending_sink_text(stores, true);
        assert!(
            terminal.contains(
                "{restoring \\output={\\tracingcommands 0\\showthe \\outputpenalty \\ETC.}"
            ),
            "{terminal:?}"
        );
        assert!(!terminal.contains("\\out\\ETC."), "{terminal:?}");
    });
}
#[test]
fn tracingrestores_coalesces_same_level_writes_and_renders_parameter_banks() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
        &mut control,
        br"\tracingrestores=1\tracingonline=1\everypar={aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa}{\vsize=1pt\global\vsize=2pt\everypar={B}\splitmaxdepth=3pt\count15=1\count15=2}\end",
    );

        run_to_end(&mut control, stores);

        assert_eq!(
            pending_sink_text(stores, true),
            concat!(
                "{restoring \\count15=0}\n",
                "{restoring \\splitmaxdepth=0.0pt}\n",
                "{restoring \\everypar=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\\ETC.}\n",
                "{retaining \\vsize=2.0pt}\n",
            )
        );
    });
}
#[test]
fn tracingrestores_reports_primitive_meaning_through_an_alias() {
    // TeX82 §§252/283 render the restored meaning, not the target control
    // sequence twice. An alias is the negative control: `\foo` must be named
    // on the left while primitive `\box` is selected on the right.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\let\foo=\box\tracingrestores=1\tracingonline=1{\let\foo=\relax}\end",
        );

        run_to_end(&mut control, stores);

        assert_eq!(pending_sink_text(stores, true), "{restoring \\foo=\\box}\n");
        assert_eq!(
            pending_sink_text(stores, false),
            "{restoring \\foo=\\box}\n"
        );
    });
}
#[test]
fn tracingrestores_reports_loaded_mathchar_meanings_in_unsave_order() {
    // TeX82 §§252/283 restore the saved typed eqtb word before `show_eqtb`
    // renders it. A genuine format boundary proves the saved shorthand
    // operands and frozen symbol identities survive serialization; the three
    // target spellings prove this is the region-one meaning path, while
    // `\fam` pins reverse save-stack publication order from the TRIP case.
    crate::test_harness::with_nonstop_plain_universe(|initialized| {
        let mut initex = MainControl::tex82_initex(initialized);
        register_source(
            &mut initex,
            br#"\mathchardef\minus="232D \mathchardef\+="1234
            \catcode`\?=13 \mathchardef?="4567 \end"#,
        );
        run_to_end(&mut initex, initialized);
        let stores = initialized;
        let mut control = MainControl::with_profile(CommandProfile::TEX82);
        register_source(
            &mut control,
            br#"\tracingrestores=1\tracingonline=1
            {\fam=7 \mathchardef\minus="322D \mathchardef\+="2345
             \mathchardef?="5670}\end"#,
        );

        run_to_end(&mut control, stores);

        let expected = concat!(
            "{restoring ?=\\mathchar\"4567}\n",
            "{restoring \\+=\\mathchar\"1234}\n",
            "{restoring \\minus=\\mathchar\"232D}\n",
            "{restoring \\fam=0}\n",
        );
        assert_eq!(pending_sink_text(stores, true), expected);
        assert_eq!(pending_sink_text(stores, false), expected);
        for (name, code) in [("minus", 0x232D), ("+", 0x1234)] {
            let symbol = stores.intern(name).expect("mathchar name").symbol();
            assert_eq!(
                stores.meaning(symbol).expect("mathchar meaning"),
                tex_state::ResolvedMeaning::Static(Meaning::MathCharGiven(code))
            );
        }
    });
}
#[test]
fn tracingrestores_reports_macro_old_value() {
    // TeX82 §§252/283 show the restored macro's saved body after copying the
    // saved eqtb word back, with §262's breadth bound.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
        &mut control,
        br"\def\foo{abcdefghijklmnopqrstuvwx}\tracingrestores=1\tracingonline=1{\def\foo{X}}\end",
    );

        run_to_end(&mut control, stores);

        let expected = "{restoring \\foo=macro:->abcdefghijklmnopqrstuvwx}\n";
        assert_eq!(pending_sink_text(stores, true), expected);
        assert_eq!(pending_sink_text(stores, false), expected);
    });
}
#[test]
fn tracingassigns_reports_setbox_change_and_committed_box() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let _initialized = MainControl::tex82_initex(stores);
        tex_command::install_etex_expandable_primitives(stores);
        crate::install_etex_unexpandable_primitives(stores);
        let mut control = MainControl::with_profile(CommandProfile::ETEX26);
        register_source(
            &mut control,
            br"\tracingonline=1\tracingassigns=1\setbox25=\hbox{}\end",
        );

        run_to_end(&mut control, stores);

        let trace = concat!(
            "{changing \\box25=void}\n",
            "{into \\box25=\n",
            "\\hbox(0.0+0.0)x0.0}\n",
        );
        let terminal = pending_sink_text(stores, true);
        let log = pending_sink_text(stores, false);
        assert!(terminal.contains(trace), "{terminal:?}");
        assert!(log.contains(trace), "{log:?}");
    });
}
#[test]
fn tracingparagraphs_reports_exact_first_pass_break_sequence() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
        &mut control,
        b"\\tracingparagraphs=1\\tracingonline=1\\linepenalty=10\\parfillskip=0pt plus 1fil\\indent\\par\\end",
    );

        run_to_end(&mut control, stores);

        let expected =
            "@firstpass\n[] \n@\\par via @@0 b=0 p=-10000 d=100\n@@1: line 1.2- t=100 -> @@0\n";
        assert!(terminal_text(stores).starts_with(expected));
        let log = String::from_utf8_lossy(stores.world().memory_log_output().unwrap_or_default());
        assert!(log.starts_with(expected));
    });
}
