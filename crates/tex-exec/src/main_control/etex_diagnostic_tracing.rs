//! e-TeX 2.6's `\tracingassigns`, `\tracinggroups`, and `\tracingifs`
//! rendered transcript trace, pinned against real e-TeX/pdfTeX 1.40.25
//! output captured with `\tracingonline=1` and each parameter set in
//! isolation (`docs/etex_primitives.md`).

use super::*;

fn with_etex_control<R>(
    test: impl for<'id> FnOnce(
        &mut Universe<tex_state::GenerationBrand<'id>>,
        &mut MainControl<tex_state::GenerationBrand<'id>>,
    ) -> R,
) -> R {
    crate::test_harness::with_plain_universe(|stores| {
        let _initialized = MainControl::tex82_initex(stores);
        tex_command::install_etex_expandable_primitives(stores);
        crate::install_etex_unexpandable_primitives(stores);
        let mut control = MainControl::with_profile(tex_command::CommandProfile::ETEX26);
        test(stores, &mut control)
    })
}

#[test]
fn tracinggroups_reports_entering_and_leaving_with_level_and_line() {
    with_etex_control(|stores, control| {
        register_source(
            control,
            b"\\nonstopmode\\tracingonline=1\\tracinggroups=1\n{\n{\n}\n}\n\\end",
        );

        run_to_end(control, stores);

        let log = terminal_text(stores);
        assert!(
            log.contains("{entering simple group (level 1) at line 2}"),
            "{log:?}"
        );
        assert!(
            log.contains("{entering simple group (level 2) at line 3}"),
            "{log:?}"
        );
        assert!(
            log.contains("{leaving simple group (level 2) entered at line 3}"),
            "{log:?}"
        );
        assert!(
            log.contains("{leaving simple group (level 1) entered at line 2}"),
            "{log:?}"
        );
    });
}

#[test]
fn tracinggroups_names_begingroup_and_endgroup_as_semi_simple() {
    with_etex_control(|stores, control| {
        register_source(
            control,
            b"\\nonstopmode\\tracingonline=1\\tracinggroups=1\n\\begingroup\n\\endgroup\n\\end",
        );

        run_to_end(control, stores);

        let log = terminal_text(stores);
        assert!(
            log.contains("{entering semi simple group (level 1) at line 2}"),
            "{log:?}"
        );
        assert!(
            log.contains("{leaving semi simple group (level 1) entered at line 2}"),
            "{log:?}"
        );
    });
}

#[test]
fn disabled_tracinggroups_emits_no_group_diagnostic() {
    with_etex_control(|stores, control| {
        register_source(control, b"\\nonstopmode\\tracingonline=1\n{}\n\\end");

        run_to_end(control, stores);

        assert!(!terminal_text(stores).contains("entering"));
        assert!(!terminal_text(stores).contains("leaving"));
    });
}

#[test]
fn middle_traces_consecutive_math_left_groups() {
    with_etex_control(|stores, control| {
        register_source(
            control,
            b"\\nonstopmode\\tracingonline=1\\tracinggroups=1\n$\\left.\\middle.\\right.$\\end",
        );

        run_to_end(control, stores);

        let log = terminal_text(stores);
        let leaving = "{leaving math left group (level 2) entered at line 2}";
        let entering = "{entering math left group (level 2) at line 2}";
        assert_eq!(log.matches(leaving).count(), 2, "{log:?}");
        assert_eq!(log.matches(entering).count(), 2, "{log:?}");
        let middle_boundary = format!("{leaving}\n{entering}");
        assert!(log.contains(&middle_boundary), "{log:?}");
    });
}

#[test]
fn middle_restores_the_preceding_math_left_group() {
    with_etex_control(|stores, control| {
        register_source(
            control,
            b"$\\left.\\count17=7\\middle.\\global\\count18=\\count17\\right.$\\end",
        );

        run_to_end(control, stores);

        assert_eq!(stores.count(17).expect("count register"), 0);
        assert_eq!(stores.count(18).expect("count register"), 0);
    });
}

#[test]
fn showgroups_names_a_math_left_group_reopened_by_middle() {
    with_etex_control(|stores, control| {
        register_source(
            control,
            b"\\nonstopmode\\tracingonline=1\n$\\left.x\\middle.y\\showgroups\\right.$\\end",
        );

        run_to_end(control, stores);

        let log = terminal_text(stores);
        // e-TeX 2.6 [49.1292] examines the delimiter noad retained by
        // [48.1191], even after more material has followed the `\middle`.
        assert!(
            log.contains("math left group (level 2) entered at line 2 (\\middle)"),
            "{log:?}"
        );
    });
}

#[test]
fn showgroups_reconstructs_every_math_shift_opener() {
    with_etex_control(|stores, control| {
        register_source(
        control,
        b"\\nonstopmode\\tracingonline=1\n$\\showgroups$\n$$\\showgroups$$\n$$\\eqno{\\showgroups}$$\n$$\\leqno{\\showgroups}$$\n\\end",
    );

        run_to_end(control, stores);

        let log = terminal_text(stores);
        // TeX82 §§1176–1177 and e-TeX [49.1292] retain the opening math-shift
        // context independently of the current semantic-nest traversal index.
        for expected in [
            "math shift group (level 1) entered at line 2 ($)",
            "math shift group (level 1) entered at line 3 ($$)",
            "math shift group (level 2) entered at line 4 (\\eqno)",
            "math shift group (level 2) entered at line 5 (\\leqno)",
        ] {
            assert!(log.contains(expected), "missing {expected:?} from {log:?}");
        }
    });
}

#[test]
fn showgroups_names_the_alignment_entry_enclosing_noalign_as_cr() {
    with_etex_control(|stores, control| {
        register_source(
        control,
        b"\\nonstopmode\\tracingonline=1\n\\setbox0=\\vbox{\\halign{#\\cr\\noalign{\\showgroups}\\cr}}\\end",
    );

        run_to_end(control, stores);

        let log = terminal_text(stores);
        // e-TeX [49.1292] lets the inner no_align_group set `a := -1`, so the
        // immediately enclosing align_group is reconstructed as `\cr`.
        assert!(
            log.contains("align group") && log.contains("(\\cr)"),
            "{log:?}"
        );
    });
}

#[test]
fn showgroups_reconstructs_box_shift_axis_sign_and_magnitude() {
    with_etex_control(|stores, control| {
        register_source(
        control,
        b"\\nonstopmode\\tracingonline=1\n\\setbox0=\\hbox{\\raise1pt\\hbox{\\showgroups}\\lower2pt\\vbox{\\showgroups}}\n\\setbox0=\\vbox{\\moveleft3pt\\hbox{\\showgroups}\\moveright4pt\\vbox{\\showgroups}}\n\\end",
    );

        run_to_end(control, stores);

        let log = terminal_text(stores);
        // TeX82 §1073's signed box_context and e-TeX [49.1292]'s enclosing-mode
        // test jointly reconstruct the prefix; neither the box kind nor sign is
        // sufficient by itself.
        for expected in [
            "(\\raise1.0pt\\hbox{)",
            "(\\lower2.0pt\\vbox{)",
            "(\\moveleft3.0pt\\hbox{)",
            "(\\moveright4.0pt\\vbox{)",
        ] {
            assert!(log.contains(expected), "missing {expected:?} from {log:?}");
        }
    });
}

#[test]
fn showgroups_prints_the_synthetic_output_group_without_a_brace() {
    with_etex_control(|stores, control| {
        register_source(
        control,
        b"\\nonstopmode\\tracingonline=1\n\\output={\\showgroups\\shipout\\box255}\n\\hbox{}\\vfil\\penalty-10000\\end",
    );

        run_to_end(control, stores);

        let log = terminal_text(stores);
        // e-TeX [49.1292] routes `output_group` directly to `found`, bypassing
        // the `found2` branch that prints an opening brace for source groups.
        assert!(
            log.contains("(\\output)"),
            "missing output context from {log:?}"
        );
        assert!(
            !log.contains("(\\output{"),
            "synthetic output group gained a source brace in {log:?}"
        );
    });
}

#[test]
fn shifted_hbox_group_kind_depends_on_the_enclosing_mode() {
    with_etex_control(|stores, control| {
        register_source(
        control,
        b"\\nonstopmode\\tracingonline=1\\tracinggroups=1\n\\setbox0=\\hbox{\\raise1pt\\hbox{}}\n\\moveleft1pt\\hbox{}\n\\end",
    );

        run_to_end(control, stores);

        let log = terminal_text(stores);
        // TeX82 §1083: the raised box is built in restricted horizontal mode.
        assert!(
            log.contains("{entering hbox group (level 2) at line 2}"),
            "{log:?}"
        );
        // The moved box is built in vertical mode and remains adjusted.
        assert!(
            log.contains("{entering adjusted hbox group (level 1) at line 3}"),
            "{log:?}"
        );
    });
}

#[test]
fn showgroups_discretionary_opener_includes_completed_parts() {
    with_etex_control(|stores, control| {
        register_source(
            control,
            b"\\nonstopmode\\tracingonline=1\\discretionary{}{\\showgroups}{}\\end",
        );

        run_to_end(control, stores);

        let log = terminal_text(stores);
        assert!(
            log.contains("(\\discretionary{}{)"),
            "second-part opener missing from {log:?}"
        );
    });
}

#[test]
fn showgroups_mathchoice_opener_includes_completed_branches() {
    with_etex_control(|stores, control| {
        register_source(
            control,
            b"\\nonstopmode\\tracingonline=1$\\mathchoice{}{}{\\showgroups}{}$\\end",
        );

        run_to_end(control, stores);

        let log = terminal_text(stores);
        assert!(
            log.contains("(\\mathchoice{}{}{)"),
            "third-branch opener missing from {log:?}"
        );
    });
}

#[test]
fn tracingassigns_reports_into_reassigning_and_changing_for_count_registers() {
    with_etex_control(|stores, control| {
        register_source(
        control,
        b"\\nonstopmode\\tracingonline=1\\tracingassigns=1\n\\count17=7\n\\count17=7\n\\count17=0\n\\end",
    );

        run_to_end(control, stores);

        let log = terminal_text(stores);
        // \count17 starts at its default 0: the first write changes it.
        assert!(log.contains("{changing \\count17=0}"), "{log:?}");
        assert!(log.contains("{into \\count17=7}"), "{log:?}");
        // The second write repeats the same value: e-TeX reports it as a single
        // "reassigning" line rather than a changing/into pair.
        assert!(log.contains("{reassigning \\count17=7}"), "{log:?}");
        // The third write differs again.
        assert!(log.contains("{changing \\count17=7}"), "{log:?}");
        assert!(log.contains("{into \\count17=0}"), "{log:?}");
    });
}

#[test]
fn tracingassigns_reports_glue_arithmetic_across_dimension_bounds() {
    // TeX82 §1236 commits glue arithmetic through `define`, whose e-TeX
    // [19.277--279] hook traces the old and committed glue values. Glue
    // widths may cross `max_dimen` in either direction; only a later scan as
    // a dimension diagnoses and saturates them.
    with_etex_control(|stores, control| {
        register_source(
            control,
            br#"\nonstopmode\tracingonline=1\tracingassigns=1
\skip44="3FFFFFFFsp \advance\skip44 by 1sp
\skip45=-"3FFFFFFFsp \advance\skip45 by -1sp
\end"#,
        );

        run_to_end(control, stores);

        let log = terminal_text(stores);
        assert!(
            log.contains("{changing \\skip44=16383.99998pt}\n{into \\skip44=16384.0pt}"),
            "{log:?}"
        );
        assert!(
            log.contains("{changing \\skip45=-16383.99998pt}\n{into \\skip45=-16384.0pt}"),
            "{log:?}"
        );
    });
}

#[test]
fn tracingassigns_uses_storage_identity_for_repeated_assignment_families() {
    // e-TeX [19.277--279] tests the eqtb representation, not rendered value
    // equality. `eq_word_define` scalars and a reused token-list pointer take
    // the reassigning return; a nonzero `\glueexpr` result is a fresh TeX
    // glue node and therefore takes changing/into even when its components
    // equal the old specification.
    with_etex_control(|stores, control| {
        register_source(
            control,
            br"\nonstopmode\tracingonline=1\tracingassigns=1
\skip16=0pt
\skip17=1pt \skip17=\glueexpr\skip17+0pt\relax
\dimen18=1pt \dimen18=1pt
\count19=1 \count19=1
\toks20={A} \toks20=\toks20
\end",
        );

        run_to_end(control, stores);

        let log = terminal_text(stores);
        assert!(log.contains("{reassigning \\skip16=0.0pt}"), "{log:?}");
        assert!(
            log.contains("{changing \\skip17=1.0pt}\n{into \\skip17=1.0pt}"),
            "{log:?}"
        );
        for repeated in [
            "{reassigning \\dimen18=1.0pt}",
            "{reassigning \\count19=1}",
            "{reassigning \\toks20=A}",
        ] {
            assert!(log.contains(repeated), "missing {repeated:?}: {log:?}");
        }
    });
}

#[test]
fn tracingassigns_preserves_muglue_pointer_identity_through_noop_conversions() {
    // e-TeX [19.277] compares the `eq_define` halfword, while change
    // [53a.5404--5425] makes `\gluetomu`/`\mutoglue` change only the value
    // level. The nested no-op expression chain therefore returns the exact
    // pointer already stored in the sparse muskip register. A separately
    // scanned equal literal is a distinct TeX glue node and is the negative
    // control against comparing rendered components.
    with_etex_control(|stores, control| {
        register_source(
            control,
            br"\nonstopmode\tracingonline=1
\skipdef\1=32767 \1=7pt
\muskipdef\2=32766 \2=\gluetomu\1
\tracingassigns=1
\1=--\mutoglue--\muexpr(--\gluetomu--\glueexpr(--\1))
\2=--\gluetomu--\glueexpr(--\mutoglue--\muexpr(--\2))
\muskip32765=7mu \muskip32765=7mu
\end",
        );

        run_to_end(control, stores);

        let log = terminal_text(stores);
        assert!(
            log.contains("{reassigning \\skip32767=7.0pt}\n{reassigning \\muskip32766=7.0mu}"),
            "{log:?}"
        );
        assert!(
            log.contains("{changing \\muskip32765=7.0mu}\n{into \\muskip32765=7.0mu}"),
            "{log:?}"
        );
    });
}

#[test]
fn tracingrestores_names_the_restored_etex_tracingassigns_parameter() {
    with_etex_control(|stores, control| {
        register_source(
            control,
            b"\\tracingrestores=1\\tracingonline=1{\\tracingassigns=1}\\end",
        );

        run_to_end(control, stores);

        let log = terminal_text(stores);
        assert!(log.contains("{into \\tracingassigns=1}"), "{log:?}");
        assert!(log.contains("{restoring \\tracingassigns=0}"), "{log:?}");
    });
}

#[test]
fn tracingrestores_reports_a_locally_defined_control_sequence_as_undefined() {
    with_etex_control(|stores, control| {
        register_source(
            control,
            b"\\tracingrestores=1\\tracingonline=1{\\def\\B{B}}\\end",
        );

        run_to_end(control, stores);

        let log = terminal_text(stores);
        assert!(log.contains("{restoring \\B=undefined}"), "{log:?}");
    });
}

#[test]
fn tracingrestores_reports_penalty_arrays_through_show_eqtb() {
    // e-TeX [17.233] adds all four penalty arrays to `show_eqtb`; TeX82
    // §283 invokes that renderer after installing each restored or retained
    // save-stack value.  A repeated null assignment is the negative control:
    // e-TeX [19.277] recognizes the identical eqtb word and saves nothing.
    with_etex_control(|stores, control| {
        register_source(
            control,
            br"\tracingonline=1 \tracingrestores=1
\interlinepenalties=3 1 2 3
\clubpenalties=1 -4
\widowpenalties=2 5 6
\displaywidowpenalties=1 7
{\interlinepenalties=0
 \clubpenalties=2 8 9
 \widowpenalties=0
 \displaywidowpenalties=2 10 11}
{\global\interlinepenalties=1 99
 \interlinepenalties=2 44 55}
{\clubpenalties=0}
\interlinepenalties=0 {\interlinepenalties=0}
\end",
        );

        run_to_end(control, stores);

        let log = terminal_text(stores);
        let restores = log
            .lines()
            .filter(|line| line.contains("penalties="))
            .collect::<Vec<_>>();
        assert_eq!(
            restores,
            [
                "{restoring \\displaywidowpenalties=1 7}",
                "{restoring \\widowpenalties=2 5\\ETC.}",
                "{restoring \\clubpenalties=1 -4}",
                "{restoring \\interlinepenalties=3 1\\ETC.}",
                "{restoring \\interlinepenalties=1 99}",
                "{restoring \\clubpenalties=1 -4}",
            ],
            "{log:?}"
        );
    });
}

#[test]
fn tracingassigns_reports_globally_changing_unconditionally() {
    with_etex_control(|stores, control| {
        register_source(
        control,
        b"\\nonstopmode\\tracingonline=1\\tracingassigns=1\n\\global\\count3=5\n\\global\\count3=5\n\\end",
    );

        run_to_end(control, stores);

        let log = terminal_text(stores);
        // `geq_word_define` never has a "reassigning" case: both global writes,
        // even the repeated one, report "globally changing" + "into".
        assert_eq!(
            log.matches("{globally changing \\count3=").count(),
            2,
            "{log:?}"
        );
        assert_eq!(log.matches("{into \\count3=5}").count(), 2, "{log:?}");
    });
}

#[test]
fn tracingassigns_reports_int_parameters_by_name() {
    with_etex_control(|stores, control| {
        register_source(
        control,
        b"\\nonstopmode\\tracingonline=1\\tracingassigns=1\n\\tolerance=100\n\\tracingassigns=0\n\\end",
    );

        run_to_end(control, stores);

        let log = terminal_text(stores);
        assert!(log.contains("{changing \\tolerance=10000}"), "{log:?}");
        assert!(log.contains("{into \\tolerance=100}"), "{log:?}");
        // e-TeX's self-referential bootstrap: turning tracing off itself only
        // ever shows "changing" (the "into" call reads the now-zero gate).
        assert!(log.contains("{changing \\tracingassigns=1}"), "{log:?}");
        assert!(!log.contains("{into \\tracingassigns=0}"), "{log:?}");
    });
}

#[test]
fn tracingassigns_reports_dimension_parameters_with_units() {
    with_etex_control(|stores, control| {
        register_source(
            control,
            b"\\nonstopmode\\tracingonline=1\\tracingassigns=1\n\\hsize=100pt\n\\end",
        );

        run_to_end(control, stores);

        let log = terminal_text(stores);
        assert!(log.contains("{into \\hsize=100.0pt}"), "{log:?}");
    });
}

#[test]
fn tracingassigns_reports_glue_parameters_with_plus_minus_and_orders() {
    with_etex_control(|stores, control| {
        register_source(
        control,
        b"\\nonstopmode\\tracingonline=1\\tracingassigns=1\n\\baselineskip=1pt plus 2pt minus 3fil\n\\end",
    );

        run_to_end(control, stores);

        let log = terminal_text(stores);
        assert!(
            log.contains("{into \\baselineskip=1.0pt plus 2.0pt minus 3.0fil}"),
            "{log:?}"
        );
    });
}

#[test]
fn tracingassigns_reports_mu_glue_parameters_with_mu_units() {
    with_etex_control(|stores, control| {
        register_source(
            control,
            b"\\nonstopmode\\tracingonline=1\\tracingassigns=1\n\\thinmuskip=1mu plus 2mu\n\\end",
        );

        run_to_end(control, stores);

        let log = terminal_text(stores);
        assert!(
            log.contains("{into \\thinmuskip=1.0mu plus 2.0mu}"),
            "{log:?}"
        );
    });
}

#[test]
fn tracingassigns_reports_token_parameters_as_their_replacement_text() {
    with_etex_control(|stores, control| {
        register_source(
            control,
            b"\\nonstopmode\\tracingonline=1\\tracingassigns=1\n\\everypar={abc}\n\\end",
        );

        run_to_end(control, stores);

        let log = terminal_text(stores);
        assert!(log.contains("{into \\everypar=abc}"), "{log:?}");
    });
}

#[test]
fn tracingassigns_reports_exact_penalty_array_show_eqtb_values_and_scope() {
    // Merged etex.web §17 extends show_eqtb for all four penalty-array
    // locations: empty arrays print `0`, populated arrays print their count
    // and first value (plus `\ETC.` when more follow), and group restoration
    // supplies the pre-image observed by the next assignment.
    with_etex_control(|stores, control| {
        register_source(
            control,
            br"\nonstopmode\tracingonline=1\tracingassigns=1
           \interlinepenalties=2 10 -20
           {\interlinepenalties=1 7}
           \interlinepenalties=0
           \clubpenalties=0
           \clubpenalties=2 30 40
           \widowpenalties=1 -50
           \displaywidowpenalties=3 60 70 80
           \global\displaywidowpenalties=0
           \end",
        );

        run_to_end(control, stores);

        let log = terminal_text(stores);
        let array_lines: Vec<_> = log
            .lines()
            .filter(|line| {
                [
                    "interlinepenalties",
                    "clubpenalties",
                    "widowpenalties",
                    "displaywidowpenalties",
                ]
                .iter()
                .any(|name| line.contains(name))
            })
            .collect();
        assert_eq!(
            array_lines,
            [
                "{changing \\interlinepenalties=0}",
                "{into \\interlinepenalties=2 10\\ETC.}",
                "{changing \\interlinepenalties=2 10\\ETC.}",
                "{into \\interlinepenalties=1 7}",
                // The local singleton was restored at group exit.
                "{changing \\interlinepenalties=2 10\\ETC.}",
                "{into \\interlinepenalties=0}",
                "{reassigning \\clubpenalties=0}",
                "{changing \\clubpenalties=0}",
                "{into \\clubpenalties=2 30\\ETC.}",
                "{changing \\widowpenalties=0}",
                "{into \\widowpenalties=1 -50}",
                "{changing \\displaywidowpenalties=0}",
                "{into \\displaywidowpenalties=3 60\\ETC.}",
                "{globally changing \\displaywidowpenalties=3 60\\ETC.}",
                "{into \\displaywidowpenalties=0}",
            ]
        );
    });
}

#[test]
fn normal_paragraph_traces_the_nonempty_interline_penalty_reset() {
    // e-TeX [47.1070] clears a non-null interline array through `eq_define`
    // when a vertical box enters `normal_paragraph`. Section [19.277] routes
    // that write through the ordinary changing/into assignment trace; an
    // hbox is the negative control because it does not enter normal_paragraph.
    for (box_command, expected) in [
        (
            "vbox",
            [
                "{changing \\interlinepenalties=0}",
                "{into \\interlinepenalties=3 101\\ETC.}",
                "{changing \\interlinepenalties=3 101\\ETC.}",
                "{into \\interlinepenalties=0}",
            ]
            .as_slice(),
        ),
        (
            "hbox",
            [
                "{changing \\interlinepenalties=0}",
                "{into \\interlinepenalties=3 101\\ETC.}",
            ]
            .as_slice(),
        ),
    ] {
        with_etex_control(|stores, control| {
            register_source(
                control,
                format!(
                    "\\nonstopmode\\tracingonline=1\\tracingassigns=1 \\interlinepenalties=3 101 102 103 \\setbox0=\\{box_command}{{}} \\end"
                )
                .as_bytes(),
            );

            run_to_end(control, stores);

            let log = terminal_text(stores);
            let traces = log
                .lines()
                .filter(|line| line.contains("interlinepenalties"))
                .collect::<Vec<_>>();
            assert_eq!(traces, expected, "\\{box_command}: {log:?}");
        });
    }
}

#[test]
fn tracingassigns_reports_catcode_table_writes() {
    with_etex_control(|stores, control| {
        register_source(
        control,
        b"\\nonstopmode\\tracingonline=1\\tracingassigns=1\n\\catcode65=12\n\\catcode65=12\n\\end",
    );

        run_to_end(control, stores);

        let log = terminal_text(stores);
        assert!(log.contains("{changing \\catcode65=11}"), "{log:?}");
        assert!(log.contains("{into \\catcode65=12}"), "{log:?}");
        assert!(log.contains("{reassigning \\catcode65=12}"), "{log:?}");
    });
}

#[test]
fn tracingassigns_reports_def_as_changing_even_for_an_identical_body() {
    with_etex_control(|stores, control| {
        register_source(
        control,
        b"\\nonstopmode\\tracingonline=1\\tracingassigns=1\n\\def\\9{\\relax}\n\\def\\9{\\relax}\n\\end",
    );

        run_to_end(control, stores);

        let log = terminal_text(stores);
        // Real TeX82 compares equivalents by pointer, so \def always allocates a
        // fresh definition and is never "reassigning", even with byte-identical
        // bodies.
        assert!(log.contains("{changing \\9=undefined}"), "{log:?}");
        assert_eq!(
            log.matches("{into \\9=macro:->\\relax }").count(),
            2,
            "{log:?}"
        );
        assert!(!log.contains("reassigning \\9"), "{log:?}");
    });
}

#[test]
fn tracingassigns_reports_let_as_reassigning_when_the_meaning_repeats() {
    with_etex_control(|stores, control| {
        register_source(
        control,
        b"\\nonstopmode\\tracingonline=1\\tracingassigns=1\n\\let\\9=\\relax\n\\let\\9=\\relax\n\\end",
    );

        run_to_end(control, stores);

        let log = terminal_text(stores);
        assert!(log.contains("{changing \\9=undefined}"), "{log:?}");
        assert!(log.contains("{into \\9=\\relax}"), "{log:?}");
        assert!(log.contains("{reassigning \\9=\\relax}"), "{log:?}");
    });
}

#[test]
fn tracingassigns_reports_both_shorthand_definition_writes() {
    // TeX82 §1224 first defines the target as `\relax`, then replaces that
    // provisional meaning with the scanned register shorthand. e-TeX
    // [17.687--750] observes both writes through `eq_define`.
    with_etex_control(|stores, control| {
        register_source(
            control,
            br"\nonstopmode\tracingonline=1\tracingassigns=1
\skipdef\1=32767
\muskipdef\2=32766
\end",
        );

        run_to_end(control, stores);

        let log = terminal_text(stores);
        let definition_lines: Vec<_> = log
            .lines()
            .filter(|line| line.contains("\\1=") || line.contains("\\2="))
            .collect();
        assert_eq!(
            definition_lines,
            [
                "{changing \\1=undefined}",
                "{into \\1=\\relax}",
                "{changing \\1=\\relax}",
                "{into \\1=\\skip32767}",
                "{changing \\2=undefined}",
                "{into \\2=\\relax}",
                "{changing \\2=\\relax}",
                "{into \\2=\\muskip32766}",
            ],
            "{log:?}"
        );
    });
}

#[test]
fn disabled_tracingassigns_emits_no_assignment_diagnostic() {
    with_etex_control(|stores, control| {
        register_source(
            control,
            b"\\nonstopmode\\tracingonline=1\n\\count0=1\n\\end",
        );

        run_to_end(control, stores);

        assert!(!terminal_text(stores).contains("\\count0"));
    });
}

#[test]
fn tracingifs_reports_entering_and_the_ordinary_closing_delimiter() {
    with_etex_control(|stores, control| {
        register_source(
            control,
            b"\\nonstopmode\\tracingonline=1\\tracingifs=1\n\\iftrue\\fi\n\\end",
        );

        run_to_end(control, stores);

        let log = terminal_text(stores);
        assert!(
            log.contains("{vertical mode: \\iftrue: (level 1) entered on line 2}"),
            "{log:?}"
        );
        assert!(
            log.contains("{\\fi: \\iftrue (level 1) entered on line 2}"),
            "{log:?}"
        );
    });
}

#[test]
fn tracingcommands_conditional_trace_retains_tracingifs_stack_details() {
    with_etex_control(|stores, control| {
        register_source(
            control,
            b"\\nonstopmode\\tracingonline=1\\tracingifs=1\\tracingcommands=2\n\
          \\ifdefined\\relax\\fi\n\
          \\unless\\iffalse X\\else Y\\fi\n\
          \\end",
        );

        run_to_end(control, stores);

        let log = terminal_text(stores);
        assert!(
            log.contains("{vertical mode: \\ifdefined: (level 1) entered on line 2}"),
            "{log:?}"
        );
        assert!(
            log.contains("{\\unless}\n{\\unless\\iffalse: (level 1) entered on line 3}"),
            "{log:?}"
        );
        assert!(
            log.contains("{\\else: \\unless\\iffalse (level 1) entered on line 3}"),
            "{log:?}"
        );
    });
}

#[test]
fn tracingifs_reports_the_else_branch_skip_and_its_closing_fi_separately() {
    with_etex_control(|stores, control| {
        register_source(
            control,
            b"\\nonstopmode\\tracingonline=1\\tracingifs=1\n\\iftrue X\\else Y\\fi\n\\end",
        );

        run_to_end(control, stores);

        let log = terminal_text(stores);
        assert!(
            log.contains("{vertical mode: \\iftrue: (level 1) entered on line 2}"),
            "{log:?}"
        );
        // The true branch is read normally, so its own \else arrives through
        // ordinary expansion...
        assert!(
            log.contains("{\\else: \\iftrue (level 1) entered on line 2}"),
            "{log:?}"
        );
        // ...while the skipped else-branch's own \fi is found by pass_text, and
        // is a second, separate trace line.
        assert!(
            log.contains("{\\fi: \\iftrue (level 1) entered on line 2}"),
            "{log:?}"
        );
    });
}

#[test]
fn tracingifs_reports_a_false_conditions_skip_to_else_via_pass_text() {
    with_etex_control(|stores, control| {
        register_source(
            control,
            b"\\nonstopmode\\tracingonline=1\\tracingifs=1\n\\iffalse X\\else Y\\fi\n\\end",
        );

        run_to_end(control, stores);

        let log = terminal_text(stores);
        assert!(
            log.contains("{vertical mode: \\iffalse: (level 1) entered on line 2}"),
            "{log:?}"
        );
        // The false branch is skipped by pass_text, so its own \else is found
        // there rather than through ordinary expansion.
        assert!(
            log.contains("{\\else: \\iffalse (level 1) entered on line 2}"),
            "{log:?}"
        );
        assert!(
            log.contains("{\\fi: \\iffalse (level 1) entered on line 2}"),
            "{log:?}"
        );
    });
}

#[test]
fn tracingifs_prefixes_an_inverted_conditional_with_unless() {
    with_etex_control(|stores, control| {
        register_source(
            control,
            b"\\nonstopmode\\tracingonline=1\\tracingifs=1\n\\unless\\iftrue\\else\\fi\n\\end",
        );

        run_to_end(control, stores);

        let log = terminal_text(stores);
        assert!(
            log.contains("{vertical mode: \\unless\\iftrue: (level 1) entered on line 2}"),
            "{log:?}"
        );
        assert!(
            log.contains("{\\else: \\unless\\iftrue (level 1) entered on line 2}"),
            "{log:?}"
        );
    });
}

#[test]
fn tracingifs_numbers_nested_conditionals_by_open_depth() {
    with_etex_control(|stores, control| {
        register_source(
            control,
            b"\\nonstopmode\\tracingonline=1\\tracingifs=1\n\\iftrue\\iffalse\\else\\fi\\fi\n\\end",
        );

        run_to_end(control, stores);

        let log = terminal_text(stores);
        assert!(
            log.contains("{vertical mode: \\iftrue: (level 1) entered on line 2}"),
            "{log:?}"
        );
        assert!(
            log.contains("{\\iffalse: (level 2) entered on line 2}"),
            "{log:?}"
        );
        assert!(
            log.contains("{\\else: \\iffalse (level 2) entered on line 2}"),
            "{log:?}"
        );
        assert!(
            log.contains("{\\fi: \\iffalse (level 2) entered on line 2}"),
            "{log:?}"
        );
        assert!(
            log.contains("{\\fi: \\iftrue (level 1) entered on line 2}"),
            "{log:?}"
        );
    });
}

#[test]
fn disabled_tracingifs_emits_no_conditional_diagnostic() {
    with_etex_control(|stores, control| {
        register_source(
            control,
            b"\\nonstopmode\\tracingonline=1\n\\iftrue\\fi\n\\end",
        );

        run_to_end(control, stores);

        assert!(!terminal_text(stores).contains("\\iftrue"));
    });
}
