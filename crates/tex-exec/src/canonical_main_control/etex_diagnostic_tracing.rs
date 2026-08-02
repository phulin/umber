//! e-TeX 2.6's `\tracingassigns`, `\tracinggroups`, and `\tracingifs`
//! rendered transcript trace, pinned against real e-TeX/pdfTeX 1.40.25
//! output captured with `\tracingonline=1` and each parameter set in
//! isolation (`docs/etex_primitives.md`).

use super::*;

fn etex_control() -> (Universe, CanonicalMainControl) {
    let mut stores = Universe::new_with_plain_catcodes();
    let _initialized = CanonicalMainControl::tex82_initex(&mut stores);
    tex_command::install_etex_expandable_primitives(&mut stores);
    crate::install_etex_unexpandable_primitives(&mut stores);
    let control = CanonicalMainControl::with_profile(tex_command::CommandProfile::ETEX26);
    (stores, control)
}

#[test]
fn tracinggroups_reports_entering_and_leaving_with_level_and_line() {
    let (mut stores, mut control) = etex_control();
    register_source(
        &mut control,
        b"\\nonstopmode\\tracingonline=1\\tracinggroups=1\n{\n{\n}\n}\n\\end",
    );

    run_to_end(&mut control, &mut stores);

    let log = terminal_text(&stores);
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
}

#[test]
fn tracinggroups_names_begingroup_and_endgroup_as_semi_simple() {
    let (mut stores, mut control) = etex_control();
    register_source(
        &mut control,
        b"\\nonstopmode\\tracingonline=1\\tracinggroups=1\n\\begingroup\n\\endgroup\n\\end",
    );

    run_to_end(&mut control, &mut stores);

    let log = terminal_text(&stores);
    assert!(
        log.contains("{entering semi simple group (level 1) at line 2}"),
        "{log:?}"
    );
    assert!(
        log.contains("{leaving semi simple group (level 1) entered at line 2}"),
        "{log:?}"
    );
}

#[test]
fn disabled_tracinggroups_emits_no_group_diagnostic() {
    let (mut stores, mut control) = etex_control();
    register_source(&mut control, b"\\nonstopmode\\tracingonline=1\n{}\n\\end");

    run_to_end(&mut control, &mut stores);

    assert!(!terminal_text(&stores).contains("entering"));
    assert!(!terminal_text(&stores).contains("leaving"));
}

#[test]
fn middle_traces_consecutive_math_left_groups() {
    let (mut stores, mut control) = etex_control();
    register_source(
        &mut control,
        b"\\nonstopmode\\tracingonline=1\\tracinggroups=1\n$\\left.\\middle.\\right.$\\end",
    );

    run_to_end(&mut control, &mut stores);

    let log = terminal_text(&stores);
    let leaving = "{leaving math left group (level 2) entered at line 2}";
    let entering = "{entering math left group (level 2) at line 2}";
    assert_eq!(log.matches(leaving).count(), 2, "{log:?}");
    assert_eq!(log.matches(entering).count(), 2, "{log:?}");
    let middle_boundary = format!("{leaving}\n{entering}");
    assert!(log.contains(&middle_boundary), "{log:?}");
}

#[test]
fn middle_restores_the_preceding_math_left_group() {
    let (mut stores, mut control) = etex_control();
    register_source(
        &mut control,
        b"$\\left.\\count17=7\\middle.\\global\\count18=\\count17\\right.$\\end",
    );

    run_to_end(&mut control, &mut stores);

    assert_eq!(stores.count(17), 0);
    assert_eq!(stores.count(18), 0);
}

#[test]
fn showgroups_names_a_math_left_group_reopened_by_middle() {
    let (mut stores, mut control) = etex_control();
    register_source(
        &mut control,
        b"\\nonstopmode\\tracingonline=1\n$\\left.x\\middle.y\\showgroups\\right.$\\end",
    );

    run_to_end(&mut control, &mut stores);

    let log = terminal_text(&stores);
    // e-TeX 2.6 [49.1292] examines the delimiter noad retained by
    // [48.1191], even after more material has followed the `\middle`.
    assert!(
        log.contains("math left group (level 2) entered at line 2 (\\middle)"),
        "{log:?}"
    );
}

#[test]
fn showgroups_reconstructs_every_math_shift_opener() {
    let (mut stores, mut control) = etex_control();
    register_source(
        &mut control,
        b"\\nonstopmode\\tracingonline=1\n$\\showgroups$\n$$\\showgroups$$\n$$\\eqno{\\showgroups}$$\n$$\\leqno{\\showgroups}$$\n\\end",
    );

    run_to_end(&mut control, &mut stores);

    let log = terminal_text(&stores);
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
}

#[test]
fn shifted_hbox_group_kind_depends_on_the_enclosing_mode() {
    let (mut stores, mut control) = etex_control();
    register_source(
        &mut control,
        b"\\nonstopmode\\tracingonline=1\\tracinggroups=1\n\\setbox0=\\hbox{\\raise1pt\\hbox{}}\n\\moveleft1pt\\hbox{}\n\\end",
    );

    run_to_end(&mut control, &mut stores);

    let log = terminal_text(&stores);
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
}

#[test]
fn showgroups_discretionary_opener_includes_completed_parts() {
    let (mut stores, mut control) = etex_control();
    register_source(
        &mut control,
        b"\\nonstopmode\\tracingonline=1\\discretionary{}{\\showgroups}{}\\end",
    );

    run_to_end(&mut control, &mut stores);

    let log = terminal_text(&stores);
    assert!(
        log.contains("(\\discretionary{}{)"),
        "second-part opener missing from {log:?}"
    );
}

#[test]
fn showgroups_mathchoice_opener_includes_completed_branches() {
    let (mut stores, mut control) = etex_control();
    register_source(
        &mut control,
        b"\\nonstopmode\\tracingonline=1$\\mathchoice{}{}{\\showgroups}{}$\\end",
    );

    run_to_end(&mut control, &mut stores);

    let log = terminal_text(&stores);
    assert!(
        log.contains("(\\mathchoice{}{}{)"),
        "third-branch opener missing from {log:?}"
    );
}

#[test]
fn tracingassigns_reports_into_reassigning_and_changing_for_count_registers() {
    let (mut stores, mut control) = etex_control();
    register_source(
        &mut control,
        b"\\nonstopmode\\tracingonline=1\\tracingassigns=1\n\\count17=7\n\\count17=7\n\\count17=0\n\\end",
    );

    run_to_end(&mut control, &mut stores);

    let log = terminal_text(&stores);
    // \count17 starts at its default 0: the first write changes it.
    assert!(log.contains("{changing \\count17=0}"), "{log:?}");
    assert!(log.contains("{into \\count17=7}"), "{log:?}");
    // The second write repeats the same value: e-TeX reports it as a single
    // "reassigning" line rather than a changing/into pair.
    assert!(log.contains("{reassigning \\count17=7}"), "{log:?}");
    // The third write differs again.
    assert!(log.contains("{changing \\count17=7}"), "{log:?}");
    assert!(log.contains("{into \\count17=0}"), "{log:?}");
}

#[test]
fn tracingrestores_names_the_restored_etex_tracingassigns_parameter() {
    let (mut stores, mut control) = etex_control();
    register_source(
        &mut control,
        b"\\tracingrestores=1\\tracingonline=1{\\tracingassigns=1}\\end",
    );

    run_to_end(&mut control, &mut stores);

    let log = terminal_text(&stores);
    assert!(log.contains("{into \\tracingassigns=1}"), "{log:?}");
    assert!(log.contains("{restoring \\tracingassigns=0}"), "{log:?}");
}

#[test]
fn tracingassigns_reports_globally_changing_unconditionally() {
    let (mut stores, mut control) = etex_control();
    register_source(
        &mut control,
        b"\\nonstopmode\\tracingonline=1\\tracingassigns=1\n\\global\\count3=5\n\\global\\count3=5\n\\end",
    );

    run_to_end(&mut control, &mut stores);

    let log = terminal_text(&stores);
    // `geq_word_define` never has a "reassigning" case: both global writes,
    // even the repeated one, report "globally changing" + "into".
    assert_eq!(
        log.matches("{globally changing \\count3=").count(),
        2,
        "{log:?}"
    );
    assert_eq!(log.matches("{into \\count3=5}").count(), 2, "{log:?}");
}

#[test]
fn tracingassigns_reports_int_parameters_by_name() {
    let (mut stores, mut control) = etex_control();
    register_source(
        &mut control,
        b"\\nonstopmode\\tracingonline=1\\tracingassigns=1\n\\tolerance=100\n\\tracingassigns=0\n\\end",
    );

    run_to_end(&mut control, &mut stores);

    let log = terminal_text(&stores);
    assert!(log.contains("{changing \\tolerance=10000}"), "{log:?}");
    assert!(log.contains("{into \\tolerance=100}"), "{log:?}");
    // e-TeX's self-referential bootstrap: turning tracing off itself only
    // ever shows "changing" (the "into" call reads the now-zero gate).
    assert!(log.contains("{changing \\tracingassigns=1}"), "{log:?}");
    assert!(!log.contains("{into \\tracingassigns=0}"), "{log:?}");
}

#[test]
fn tracingassigns_reports_dimension_parameters_with_units() {
    let (mut stores, mut control) = etex_control();
    register_source(
        &mut control,
        b"\\nonstopmode\\tracingonline=1\\tracingassigns=1\n\\hsize=100pt\n\\end",
    );

    run_to_end(&mut control, &mut stores);

    let log = terminal_text(&stores);
    assert!(log.contains("{into \\hsize=100.0pt}"), "{log:?}");
}

#[test]
fn tracingassigns_reports_glue_parameters_with_plus_minus_and_orders() {
    let (mut stores, mut control) = etex_control();
    register_source(
        &mut control,
        b"\\nonstopmode\\tracingonline=1\\tracingassigns=1\n\\baselineskip=1pt plus 2pt minus 3fil\n\\end",
    );

    run_to_end(&mut control, &mut stores);

    let log = terminal_text(&stores);
    assert!(
        log.contains("{into \\baselineskip=1.0pt plus 2.0pt minus 3.0fil}"),
        "{log:?}"
    );
}

#[test]
fn tracingassigns_reports_mu_glue_parameters_with_mu_units() {
    let (mut stores, mut control) = etex_control();
    register_source(
        &mut control,
        b"\\nonstopmode\\tracingonline=1\\tracingassigns=1\n\\thinmuskip=1mu plus 2mu\n\\end",
    );

    run_to_end(&mut control, &mut stores);

    let log = terminal_text(&stores);
    assert!(
        log.contains("{into \\thinmuskip=1.0mu plus 2.0mu}"),
        "{log:?}"
    );
}

#[test]
fn tracingassigns_reports_token_parameters_as_their_replacement_text() {
    let (mut stores, mut control) = etex_control();
    register_source(
        &mut control,
        b"\\nonstopmode\\tracingonline=1\\tracingassigns=1\n\\everypar={abc}\n\\end",
    );

    run_to_end(&mut control, &mut stores);

    let log = terminal_text(&stores);
    assert!(log.contains("{into \\everypar=abc}"), "{log:?}");
}

#[test]
fn tracingassigns_reports_exact_penalty_array_show_eqtb_values_and_scope() {
    // Merged etex.web §17 extends show_eqtb for all four penalty-array
    // locations: empty arrays print `0`, populated arrays print their count
    // and first value (plus `\ETC.` when more follow), and group restoration
    // supplies the pre-image observed by the next assignment.
    let (mut stores, mut control) = etex_control();
    register_source(
        &mut control,
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

    run_to_end(&mut control, &mut stores);

    let log = terminal_text(&stores);
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
}

#[test]
fn tracingassigns_reports_catcode_table_writes() {
    let (mut stores, mut control) = etex_control();
    register_source(
        &mut control,
        b"\\nonstopmode\\tracingonline=1\\tracingassigns=1\n\\catcode65=12\n\\catcode65=12\n\\end",
    );

    run_to_end(&mut control, &mut stores);

    let log = terminal_text(&stores);
    assert!(log.contains("{changing \\catcode65=11}"), "{log:?}");
    assert!(log.contains("{into \\catcode65=12}"), "{log:?}");
    assert!(log.contains("{reassigning \\catcode65=12}"), "{log:?}");
}

#[test]
fn tracingassigns_reports_def_as_changing_even_for_an_identical_body() {
    let (mut stores, mut control) = etex_control();
    register_source(
        &mut control,
        b"\\nonstopmode\\tracingonline=1\\tracingassigns=1\n\\def\\9{\\relax}\n\\def\\9{\\relax}\n\\end",
    );

    run_to_end(&mut control, &mut stores);

    let log = terminal_text(&stores);
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
}

#[test]
fn tracingassigns_reports_let_as_reassigning_when_the_meaning_repeats() {
    let (mut stores, mut control) = etex_control();
    register_source(
        &mut control,
        b"\\nonstopmode\\tracingonline=1\\tracingassigns=1\n\\let\\9=\\relax\n\\let\\9=\\relax\n\\end",
    );

    run_to_end(&mut control, &mut stores);

    let log = terminal_text(&stores);
    assert!(log.contains("{changing \\9=undefined}"), "{log:?}");
    assert!(log.contains("{into \\9=\\relax}"), "{log:?}");
    assert!(log.contains("{reassigning \\9=\\relax}"), "{log:?}");
}

#[test]
fn disabled_tracingassigns_emits_no_assignment_diagnostic() {
    let (mut stores, mut control) = etex_control();
    register_source(
        &mut control,
        b"\\nonstopmode\\tracingonline=1\n\\count0=1\n\\end",
    );

    run_to_end(&mut control, &mut stores);

    assert!(!terminal_text(&stores).contains("\\count0"));
}

#[test]
fn tracingifs_reports_entering_and_the_ordinary_closing_delimiter() {
    let (mut stores, mut control) = etex_control();
    register_source(
        &mut control,
        b"\\nonstopmode\\tracingonline=1\\tracingifs=1\n\\iftrue\\fi\n\\end",
    );

    run_to_end(&mut control, &mut stores);

    let log = terminal_text(&stores);
    assert!(
        log.contains("{vertical mode: \\iftrue: (level 1) entered on line 2}"),
        "{log:?}"
    );
    assert!(
        log.contains("{\\fi: \\iftrue (level 1) entered on line 2}"),
        "{log:?}"
    );
}

#[test]
fn tracingcommands_conditional_trace_retains_tracingifs_stack_details() {
    let (mut stores, mut control) = etex_control();
    register_source(
        &mut control,
        b"\\nonstopmode\\tracingonline=1\\tracingifs=1\\tracingcommands=2\n\
          \\ifdefined\\relax\\fi\n\
          \\unless\\iffalse X\\else Y\\fi\n\
          \\end",
    );

    run_to_end(&mut control, &mut stores);

    let log = terminal_text(&stores);
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
}

#[test]
fn tracingifs_reports_the_else_branch_skip_and_its_closing_fi_separately() {
    let (mut stores, mut control) = etex_control();
    register_source(
        &mut control,
        b"\\nonstopmode\\tracingonline=1\\tracingifs=1\n\\iftrue X\\else Y\\fi\n\\end",
    );

    run_to_end(&mut control, &mut stores);

    let log = terminal_text(&stores);
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
}

#[test]
fn tracingifs_reports_a_false_conditions_skip_to_else_via_pass_text() {
    let (mut stores, mut control) = etex_control();
    register_source(
        &mut control,
        b"\\nonstopmode\\tracingonline=1\\tracingifs=1\n\\iffalse X\\else Y\\fi\n\\end",
    );

    run_to_end(&mut control, &mut stores);

    let log = terminal_text(&stores);
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
}

#[test]
fn tracingifs_prefixes_an_inverted_conditional_with_unless() {
    let (mut stores, mut control) = etex_control();
    register_source(
        &mut control,
        b"\\nonstopmode\\tracingonline=1\\tracingifs=1\n\\unless\\iftrue\\else\\fi\n\\end",
    );

    run_to_end(&mut control, &mut stores);

    let log = terminal_text(&stores);
    assert!(
        log.contains("{vertical mode: \\unless\\iftrue: (level 1) entered on line 2}"),
        "{log:?}"
    );
    assert!(
        log.contains("{\\else: \\unless\\iftrue (level 1) entered on line 2}"),
        "{log:?}"
    );
}

#[test]
fn tracingifs_numbers_nested_conditionals_by_open_depth() {
    let (mut stores, mut control) = etex_control();
    register_source(
        &mut control,
        b"\\nonstopmode\\tracingonline=1\\tracingifs=1\n\\iftrue\\iffalse\\else\\fi\\fi\n\\end",
    );

    run_to_end(&mut control, &mut stores);

    let log = terminal_text(&stores);
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
}

#[test]
fn disabled_tracingifs_emits_no_conditional_diagnostic() {
    let (mut stores, mut control) = etex_control();
    register_source(
        &mut control,
        b"\\nonstopmode\\tracingonline=1\n\\iftrue\\fi\n\\end",
    );

    run_to_end(&mut control, &mut stores);

    assert!(!terminal_text(&stores).contains("\\iftrue"));
}
