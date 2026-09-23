//! Focused loaded TRIP diagnostic and scanner compatibility cases.

use super::*;

#[test]
#[ignore = "manual compatibility/parity tier: not a cutover closure gate"]
fn trip_loaded_vadjust_diagnostic_uses_detached_replacement_layout() {
    // Construct the real TRIP format, then retain the loaded-job prefix that
    // establishes the page, paragraph, and three preceding \vadjust states.
    // Replaying INITEX source cannot reproduce the dumped font ligature and
    // hyphenation state responsible for this diagnostic.
    let log = run_focused_loaded_trip_through(203);
    assert!(
        log.contains(concat!(
            "Underfull \\hbox (badness 10000) in paragraph at lines 109--109\n",
            " [] []\\rip BB-B-BBB\n",
        )),
        "{log}"
    );
    assert!(!log.contains(" [] []\\rip BB-BBBB\n"), "{log}");
}

#[test]
#[ignore = "manual compatibility/parity tier: not a cutover closure gate"]
fn trip_loaded_display_diagnostic_includes_overfull_rule() {
    let log = run_focused_loaded_trip_through(285);
    assert!(
        log.contains(concat!(
            "Overfull \\hbox (48.4746pt too wide) detected at line 193\n",
            "[][][] [] [] []|\n",
        )),
        "{log}"
    );
    assert_eq!(
        log.matches("{horizontal mode: \\expandafter}").count(),
        1,
        "{log}"
    );
    let undefined = log
        .find("{undefined}")
        .unwrap_or_else(|| panic!("undefined command trace:\n{log}"));
    let page = log
        .find("% t=21.7 plus")
        .unwrap_or_else(|| panic!("page-builder trace:\n{log}"));
    assert!(undefined < page, "{log}");
    assert!(
        !log.contains("% t=191.11256 plus 40.0 plus 1.0fil"),
        "{log}"
    );
    assert!(
        log.contains("% t=262.41258 plus 80.0 plus 1.0fil plus -803.0fill g=10000.0 b=0 p=7 c="),
        "{log}"
    );
    assert!(
        log.as_bytes()
            .windows(10)
            .any(|window| window == b"\\bigtr\np -"),
        "{log}"
    );
    assert!(
        !log.as_bytes()
            .windows(10)
            .any(|window| window == b"\\bigtr\0p -"),
        "{log}"
    );
}

#[test]
#[ignore = "manual compatibility/parity tier: not a cutover closure gate"]
fn trip_loaded_vsplit_diagnostics_freeze_canonical_scan_contexts() {
    let log = run_focused_loaded_trip_through(377);
    assert!(
        log.contains(concat!(
            "! Missing `to' inserted.\n",
            "<to be read again> \n",
            "                   0\n",
            "l.285 ...\\hbox{\\vfill\\vsplit 3 0\n",
            "                                pt}\n",
        )),
        "{log}"
    );
    assert!(
        log.contains(concat!(
            "! \\vsplit needs a \\vbox.\n",
            "<to be read again> \n",
            "                   }\n",
            "l.285 ...ox{\\vfill\\vsplit 3 0pt}\n",
        )),
        "{log}"
    );
}

#[test]
#[ignore = "manual compatibility/parity tier: not a cutover closure gate"]
fn trip_loaded_deferred_write_condition_replaces_the_final_stack_front() {
    // Exact TRIP source through lines 419 and 441--442. TeX82 §§1370/1335:
    // the deferred write's ordinary `\if` remains above the older selected
    // `\ifcase` and is reported first during final cleanup.
    let log = run_focused_loaded_trip_through(442);
    let condition_reports = || {
        log.lines()
            .filter(|line| line.contains("end occurred when"))
            .collect::<Vec<_>>()
    };
    let write_if = log
        .find("(end occurred when if on line 350 was incomplete)")
        .unwrap_or_else(|| panic!("deferred-write condition: {:?}", condition_reports()));
    let old_ifcase = log
        .find("(end occurred when ifcase on line 327 was incomplete)")
        .unwrap_or_else(|| panic!("older condition: {:?}", condition_reports()));
    assert!(
        write_if < old_ifcase,
        "innermost condition reports first: {:?}",
        condition_reports()
    );
}

#[test]
#[ignore = "manual compatibility/parity tier: not a cutover closure gate"]
fn trip_loaded_character_constant_alignment_template_traces_endv_once() {
    let log = run_focused_loaded_trip_through(337);
    assert!(
        log.contains(
            "Missing character: There is no } in font trip!\n\
             {end of alignment template}\n\
             @firstpass"
        ),
        "the character-constant brace must preserve alignment depth without duplicating end-v"
    );
}

#[test]
#[ignore = "manual compatibility/parity tier: not a cutover closure gate"]
fn trip_loaded_delete_last_error_shows_command_context_before_help() {
    // TeX82 §§82/1105: the page-removal apology installs its help and calls
    // `error`; the live command input context is printed before §90's help.
    let log = run_focused_loaded_trip_through(345);
    let message = log
        .rfind("You can't use `\\unpenalty' in vertical mode.")
        .expect("line-345 unpenalty error");
    let report = &log[message..];
    let context = report
        .find("\\lastpenalty\\unpenalty")
        .expect("unpenalty source context");
    let help = report
        .find("Sorry...I usually can't take things from the current page.")
        .expect("delete-last help");
    assert!(context < help, "{report}");
}

#[test]
#[ignore = "manual compatibility/parity tier: not a cutover closure gate"]
fn trip_loaded_lastbox_error_shows_command_context_before_help() {
    // TeX82 §§82/1081: `begin_box` rejects `\lastbox` on the current page,
    // then §82 prints the still-live command input before §90's help.
    let log = run_focused_loaded_trip_through(346);
    let message = log
        .rfind("You can't use `\\lastbox' in vertical mode.")
        .expect("line-346 lastbox error");
    let report = &log[message..];
    let context = report.find("\\penalty5").expect("lastbox source context");
    let help = report
        .find("Sorry...I usually can't take things from the current page.")
        .expect("lastbox help");
    assert!(context < help, "{report}");
}

#[test]
#[ignore = "manual compatibility/parity tier: not a cutover closure gate"]
fn trip_loaded_missing_definition_target_recovers_once() {
    // TeX82 §1215's `get_r_token` owns one complete `ins_error` and restart:
    // the rejected `{` supplies the empty definition and `?` resumes normally.
    let log = run_focused_loaded_trip_through(347);
    let lastbox = log
        .rfind("You can't use `\\lastbox' in vertical mode.")
        .expect("line-346 lastbox error");
    let report = &log[lastbox..];
    assert_eq!(
        report
            .matches("! Missing control sequence inserted.")
            .count(),
        1,
        "{report}"
    );
    assert!(report.contains("{the character ?}"), "{report}");
    assert!(
        report.contains("{horizontal mode: the character ?}"),
        "{report}"
    );
}

#[test]
#[ignore = "manual compatibility/parity tier: not a cutover closure gate"]
fn trip_loaded_invalid_character_error_precedes_following_trace() {
    // TeX82 §§345/367/370/380 complete both expansion-time errors before the
    // next begin-group command reaches tracing.
    let log = run_focused_loaded_trip_through(352);
    let undefined = log
        .rfind("{undefined}")
        .expect("line-351 undefined command trace");
    let report = &log[undefined..];
    let undefined_error = report
        .find("! Undefined control sequence.")
        .expect("undefined-control report");
    let invalid_error = report
        .find("! Text line contains an invalid character.")
        .expect("invalid-character report");
    let following = report
        .find("{begin-group character {}")
        .unwrap_or_else(|| panic!("following begin-group trace:\n{report}"));
    assert!(undefined_error < invalid_error, "{report}");
    assert!(invalid_error < following, "{report}");
}

#[test]
#[ignore = "manual compatibility/parity tier: not a cutover closure gate"]
fn trip_loaded_tokens_runaway_names_assignment_scanner() {
    // TeX82 §§306/336/1227 retain the current token-register shorthand as
    // `warning_index` while its balanced right-hand side is absorbed.
    let log = run_focused_loaded_trip_through(354);
    assert!(
        log.contains("Forbidden control sequence found while scanning text of \\tokens."),
        "{log}"
    );
}

#[test]
#[ignore = "manual compatibility/parity tier: not a cutover closure gate"]
fn trip_loaded_runaway_context_escapes_nul_control_sequence_name() {
    // TeX82 §§59/262/315 pseudoprint token-list context through the active
    // selector, so NUL bytes use printable double-caret notation.
    let log = run_focused_loaded_trip_through(354);
    let runaway = log
        .rfind("Forbidden control sequence found while scanning text of \\tokens.")
        .expect("line-354 tokens runaway");
    let report = &log[runaway..];
    assert!(report.contains("\\a^^@^^@a"), "{report}");
    assert!(!report.contains('\0'), "{report:?}");
}

#[test]
#[ignore = "manual compatibility/parity tier: not a cutover closure gate"]
fn trip_loaded_nested_ifcase_operand_preserves_skip_nesting() {
    // TeX82 §509 keeps skipping while an operand-expanded conditional is
    // above the saved `\ifcase` frame, popping only that newer frame's `\fi`.
    let log = run_focused_loaded_trip_through(359);
    let case_negative = log.rfind("{case -1}").expect("line-359 negative case");
    let report = &log[case_negative..];
    let nested_ifcase = report.find("{\\ifcase}").expect("skipped nested ifcase");
    let nested_fi = report.find("{\\fi}").expect("skipped nested fi");
    let case_five = report.find("{case 5}").expect("outer else-branch case");
    assert!(
        nested_ifcase < nested_fi && nested_fi < case_five,
        "{report}"
    );
    assert!(!report[..case_five].contains("{case 0}"), "{report}");
}

#[test]
#[ignore = "manual compatibility/parity tier: not a cutover closure gate"]
fn trip_loaded_runaway_preamble_finishes_partial_before_error() {
    // TeX82 §§338--339 prints the already collected preamble token list
    // before `print_err` opens the forbidden-control diagnostic.
    let log = run_focused_loaded_trip_through(363);
    assert!(
        log.contains(
            "Runaway preamble?\n{\n! Forbidden control sequence found while scanning preamble"
        ),
        "{log}"
    );
    let frontier = log
        .rfind("! Incomplete \\ifcase;")
        .expect("line-363 conditional recovery");
    let recovery = &log[frontier..];
    let first = recovery
        .find("Runaway preamble?\n{")
        .expect("line-363 runaway");
    let after_first = &recovery[first + "Runaway preamble?".len()..];
    let missing = after_first
        .find("! Missing # inserted in alignment preamble.")
        .expect("missing-parameter recovery follows runaway");
    assert!(
        !after_first[..missing].contains("Runaway preamble?"),
        "{recovery}"
    );
    assert!(
        recovery.contains("\\lo #1#2U3#4#5#6#7#8#989{"),
        "{recovery}"
    );
    assert!(recovery.contains("\nU3<-.\n"), "{recovery}");
    assert!(!recovery.contains("\\lo #1#2#3#4#5"), "{recovery}");
}

#[test]
#[ignore = "manual compatibility/parity tier: not a cutover closure gate"]
fn trip_loaded_runaway_definition_pseudoprints_nonstandard_match_marker() {
    let log = run_focused_loaded_trip_through(364);
    let runaway = log.rfind("Runaway definition?").expect("line-364 runaway");
    let report = &log[runaway..];
    assert!(
        report.contains("^^C1->\\d ^^C1\\d \\l {##2}\\l ^^C1\\par"),
        "{report}"
    );
    assert!(!report.contains('\u{3}'), "{report:?}");
}

#[test]
#[ignore = "manual compatibility/parity tier: not a cutover closure gate"]
fn trip_loaded_macro_mismatch_precedes_following_malformed_invocation_trace() {
    // TeX82 §§82/391 completes \T's compulsory-prefix mismatch before
    // §389 can trace the later malformed \a invocation.
    let log = run_focused_loaded_trip_through(366);
    let mismatch = log
        .rfind("Use of \\T doesn't match its definition")
        .expect("line-366 compulsory-prefix mismatch");
    let report = &log[mismatch..];
    let inserted_context = report
        .find("<inserted text> ")
        .expect("§336 inserted paragraph context");
    let help = report
        .find("If you say, e.g., `\\def\\a1{...}'")
        .expect("§391 mismatch help");
    let following_trace = report
        .find("\\a^^@^^@a #1\\par #2->")
        .expect("following malformed macro trace");
    assert!(
        inserted_context < help && help < following_trace,
        "{report}"
    );
}
