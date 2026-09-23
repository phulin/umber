//! Diagnostic text, command recovery, and live source or mode context.

use super::*;

#[test]
fn setbox_rejects_non_box_command_with_assignment_context_diagnostic() {
    // TeX82 §1084: genuine `scan_box` missing-box recovery backs the
    // rejected command for execution.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\nonstopmode\setbox0=\count0=7 \count1=9\end",
        );

        run_to_end(&mut control, stores);

        let terminal = terminal_text(stores);
        assert!(terminal.contains("Improper \\setbox"), "{terminal}");
        assert!(
            !terminal.contains("A <box> was supposed to be here"),
            "{terminal}"
        );
        assert!(stores.copy_box_to_page(0).is_none());
        assert_eq!(stores.count(0).expect("count register"), 7);
        assert_eq!(stores.count(1).expect("count register"), 9);
    });
}
#[test]
fn forbidden_setbox_reports_before_reading_the_following_command() {
    // TeX82 §§1241/1123: `\accent` clears `set_box_allowed` while its
    // assignment loop runs. The register and optional equals are consumed,
    // but the following command is still to be read when `error` renders the
    // context; it subsequently executes once and the destination stays void.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\nonstopmode\tracingonline=1\tracingcommands=2\accent65\setbox0=\count0=7 X\end",
        );

        run_to_end(&mut control, stores);

        let terminal = terminal_text(stores);
        assert!(terminal.contains("Improper \\setbox"), "{terminal}");
        let trace = terminal
            .find("{\\setbox}")
            .unwrap_or_else(|| panic!("missing rejected-command trace: {terminal}"));
        let error = terminal
            .find("Improper \\setbox")
            .expect("improper-setbox report");
        assert!(
            trace < error,
            "setbox error overtook its scan trace: {terminal}"
        );
        assert!(stores.copy_box_to_page(0).is_none());
        assert_eq!(stores.count(0).expect("count register"), 7);
    });
}
#[test]
fn invalid_prevgraf_reports_after_diagnostics_from_its_value_scan() {
    // TeX82 §§476/1244: expanded commands encountered while `scan_int`
    // finishes the assigned value have already printed their command trace
    // when `alter_prev_graf` diagnoses the negative result. The detached
    // scan-time trace must therefore be published before the synchronous
    // `int_error` report.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        stores.set_interaction_mode(tex_state::InteractionMode::Batch);
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\tracingonline=1\tracingcommands=2
{\if 11 \prevgraf=-1\if 0123\errmessage{skipped}\else\relax\fi
 \else\errmessage{outer skipped}\fi}\end",
        );

        run_to_end(&mut control, stores);

        let terminal = terminal_text(stores);
        let error = terminal
            .find("Bad \\prevgraf")
            .unwrap_or_else(|| panic!("missing prevgraf error: {terminal:?}"));
        let prevgraf_trace = terminal[..error]
            .rfind("\\prevgraf}")
            .unwrap_or_else(|| panic!("missing prevgraf command trace: {terminal:?}"));
        let false_trace = terminal[prevgraf_trace..error]
            .find("{false}")
            .map(|offset| prevgraf_trace + offset)
            .unwrap_or_else(|| {
                panic!("conditional result did not precede prevgraf error: {terminal:?}")
            });
        assert!(
            false_trace < error,
            "prevgraf error overtook its completed value-scan trace: {terminal:?}"
        );
    });
}
#[test]
fn global_escapechar_survives_off_save_inserted_group_recovery() {
    // TeX82 §§1064/1214: a globally assigned integer parameter remains live
    // while `off_save` backs up the offending command and inserts the closer.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            b"\\scrollmode\\tracingonline=1\\tracingcommands=1\\hbox{\\escapechar=127\\global\\escapechar=256\\end}",
        );

        run_to_end(&mut control, stores);

        assert_eq!(stores.int_param(IntParam::ESCAPE_CHAR), 256);
        let terminal = terminal_text(stores);
        assert!(terminal.contains("! Missing } inserted."), "{terminal:?}");
        let traced_end = terminal.find("{end}").expect("the offending command trace");
        let recovery = terminal
            .find("! Missing } inserted.")
            .expect("the balancing-brace recovery");
        assert!(
            traced_end < recovery,
            "§1030 command tracing precedes §1064 recovery: {terminal:?}"
        );
    });
}
#[test]
fn command_trace_precedes_synchronous_operand_scan_error() {
    // TeX82 §§1030/1211/1243/460: main control prints the outer `\global`
    // command at `reswitch` before `prefixed_command` scans the oversized
    // dimension. The scanner's live World reporter must not overtake that
    // already-complete detached trace.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\tracingonline=1\tracingcommands=1\global\vsize=16384pt\end",
        );

        run_to_end(&mut control, stores);

        let terminal = pending_sink_text(stores, true);
        let trace = terminal.find("\\global}").expect("global trace");
        let error = terminal
            .find("! Dimension too large.")
            .expect("dimension error");
        assert!(trace < error, "{terminal}");
    });
}
#[test]
fn leaders_skip_section_404_filler_and_preserve_non_glue_recovery() {
    // TeX82 §1078 fetches the glue after every payload with §404's shared
    // non-blank, non-relax loop. Cover rule, constructed-box, and register
    // payloads; soul terminates its rule specification with exactly this
    // explicit `\relax` before `\hskip`.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\nonstopmode
\setbox1=\hbox{\kern1pt}
\setbox0=\hbox{
  \leaders\hrule height1pt\relax \hskip3pt
  \cleaders\hbox{\kern1pt} \relax\hskip4pt
  \xleaders\copy1\relax \hskip5pt}
\end",
        );

        run_to_end(&mut control, stores);

        let children = box_child_nodes(stores, 0);
        assert_eq!(
            children
                .iter()
                .filter(|node| matches!(
                    node,
                    Node::Glue {
                        leader: Some(_),
                        ..
                    }
                ))
                .count(),
            3,
            "all leader payload forms retain their glue: {children:?}"
        );
        assert!(
            !pending_sink_text(stores, true).contains("Leaders not followed"),
            "valid §1078 filler is silent"
        );

        crate::test_harness::with_nonstop_plain_universe(|recovery_stores| {
            let mut recovery = MainControl::tex82_initex(recovery_stores);
            register_source(
                &mut recovery,
                br"\nonstopmode\setbox0=\hbox{\leaders\hbox{} \relax\kern2pt}\end",
            );

            run_to_end(&mut recovery, recovery_stores);

            let recovered = box_child_nodes(recovery_stores, 0);
            assert_eq!(
                pending_sink_text(recovery_stores, true)
                    .matches("Leaders not followed by proper glue")
                    .count(),
                1
            );
            assert!(
                matches!(recovered.as_slice(), [Node::Kern { amount, .. }] if amount.raw() == 2 * Scaled::UNITY),
                "§1078 back_error retains the first substantive non-glue command: {recovered:?}"
            );
        });
    });
}
#[test]
fn vsplit_infinite_shrink_reports_the_scanner_owned_live_context() {
    // TeX82 §§976/82: `vert_break` runs synchronously inside `\vsplit`, so
    // its error sees the backed-up command following the completed dimension.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        stores.set_interaction_mode(tex_state::InteractionMode::Nonstop);
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\setbox0=\vbox{\vskip0pt minus 1fil}\setbox1=\vsplit0 to 1pt\count0=23\end",
        );

        run_to_end(&mut control, stores);

        let output = terminal_text(stores);
        let error = output
            .find("! Infinite glue shrinkage found in box being split.")
            .expect("vsplit reports infinite shrink");
        assert!(output[error..].contains("<to be read again> "), "{output}");
        assert!(output[error..].contains("\\count"), "{output}");
        assert_eq!(
            stores.count(0).expect("count register"),
            23,
            "recovery resumes after the split"
        );
    });
}
#[test]
fn paragraph_shrink_error_uses_the_live_input_context() {
    // TeX82 §§82/825 reports the `\par` source line before the paragraph
    // recovery help, while command state still owns that cursor.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        stores.set_interaction_mode(tex_state::InteractionMode::Nonstop);
        let mut control = MainControl::tex82_initex(stores);
        register_source(
        &mut control,
        br"\tracingparagraphs=1\tracingonline=1{\rightskip0pt plus 104pt minus 100fil \looseness5 \spaceskip4pt plus 2pt minus 1fil A B\par}\end",
    );

        run_to_end(&mut control, stores);

        let log = String::from_utf8_lossy(stores.world().memory_log_output().unwrap_or_default());
        let error = log
            .find("! Infinite glue shrinkage found in a paragraph.")
            .expect("paragraph shrink recovery reports");
        assert_eq!(
            &log[..error],
            "\n",
            "§825 closes the tracing diagnostic exactly once before print_err: {log:?}"
        );
        let context = log[error..]
            .find("l.1 ")
            .expect("the report includes the live source line");
        let help = log[error..]
            .find("The paragraph just ended includes")
            .unwrap_or_else(|| panic!("the report includes TeX's recovery help: {log:?}"));
        assert!(context < help, "{log:?}");
        assert!(log[error..].contains("\\par"), "{log:?}");
    });
}
#[test]
fn etex_everyeof_error_precedes_pseudo_file_close() {
    // TeX82 §§370/82 report the error before §362 fetches past the
    // e-TeX §24.362 everyeof list and closes its pseudo-file.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = etex_initex(stores);
        register_source(
            &mut control,
            br"\tracingscantokens=1\everyeof={\undefined}\scantokens{}\end",
        );
        run_to_end(&mut control, stores);
        let output = terminal_text(stores);
        let error = output
            .find("! Undefined control sequence.")
            .expect("undefined error");
        assert!(!output[..error].contains(')'), "{output}");
        assert!(output[error..].lines().any(|line| line == ")"), "{output}");
    });
}
#[test]
fn expansion_error_precedes_following_pseudo_file_open() {
    // TeX82 §§370/82 complete the report before the next expansion can
    // reach e-TeX §53a's pseudo_start file framing.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = etex_initex(stores);
        register_source(
            &mut control,
            br"\tracingscantokens=1\relax\undefined\scantokens{}\end",
        );
        run_to_end(&mut control, stores);
        let output = terminal_text(stores);
        let error = output
            .find("! Undefined control sequence.")
            .expect("undefined error");
        let framing = output
            .find("( ")
            .unwrap_or_else(|| panic!("pseudo-file framing: {output}"));
        assert!(error < framing, "{output}");
    });
}
#[test]
fn showtokens_distinguishes_newlinechar_from_other_control_bytes() {
    // TeX82 §§262 and 1297: direct `token_show` output recognizes the live
    // newline character, while another non-printable byte keeps its `^^`
    // spelling. The control-sequence separator is part of `print_cs`.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        crate::test_harness::assign_int_param(
            stores,
            IntParam::NEWLINE_CHAR,
            10,
            tex_state::AssignmentScope::Global,
        )
        .expect("integer parameter assignment");
        let word = stores.intern("word").expect("symbol interning");
        let tokens = allocate_tokens(
            stores,
            &[
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
            ],
        );

        assert_eq!(
            admitted!(stores, |context| show_tokens_text(context, tokens)),
            "^^A\n\\word X"
        );
    });
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
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            let mut control = MainControl::tex82_initex(stores);
            control.set_fuel_limit(1_000).expect("bounded fuel");
            register_source(&mut control, source);

            assert_eq!(
                control.advance(stores).expect("prefix executes"),
                StepResult::Progress(MainControlStep::Continue)
            );
            assert_eq!(
                admitted!(stores, |context| context.innermost_group_kind()),
                Some(expected)
            );
            assert_eq!(
                admitted!(stores, |context| context
                    .innermost_group_kind()
                    .map(tex_state::GroupKind::etex_code)),
                Some(if expected == GroupKind::HBox { 2 } else { 3 })
            );
        })
    }
}
#[test]
fn discretionary_part_restoration_precedes_synchronous_validation_error() {
    // TeX82 §§1120--1121 runs `unsave` before validating an improper
    // part. The detached restoration program must not be overtaken
    // by its live error report. Canonical TRIP line 277 additionally covers
    // the same invariant for math mode's forbidden third part.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\tracingonline=1\tracingrestores=1
x\discretionary{\count0=1\hfil}{}{}\end",
        );
        run_to_end(&mut control, stores);

        let output = terminal_text(stores);
        let restoration = output
            .find("{restoring \\count0=0}")
            .unwrap_or_else(|| panic!("missing restoration in {output:?}"));
        let error = output
            .find("Improper discretionary list")
            .unwrap_or_else(|| panic!("missing discretionary error in {output:?}"));
        assert!(
            restoration < error,
            "discretionary error overtook group restoration: {output:?}"
        );
    });
}
#[test]
fn direct_missing_number_retains_offender_before_scalar_backup() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\nonstopmode\count0=* \count1=17\end");

        run_to_end(&mut control, stores);

        assert_eq!(stores.count(1).expect("following assignment executes"), 17);
        let first = control
            .first_recoverable_diagnostic()
            .expect("direct missing-number report is retained");
        assert_eq!(first.kind, "missing-number");
        assert_eq!(first.command.as_deref(), Some("other_char"));
        assert_eq!(first.command_operand, Some(i64::from(b'*')));
        assert_eq!(
            first.observed_token,
            Some(ObservedToken::Character {
                character: '*',
                catcode: Catcode::Other,
            })
        );
        assert!(first.origin.is_some(), "direct report keeps source origin");
        assert!(first.context.is_some(), "direct report keeps input context");
        assert_eq!(first.mode, Mode::Vertical);
        assert_eq!(first.scanner_status, "normal");
    });
}
#[test]
fn missing_number_keeps_report_time_mode_and_group_context() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\nonstopmode\hbox{\count0=*}\end");

        run_to_end(&mut control, stores);

        let first = control
            .first_recoverable_diagnostic()
            .expect("nested missing-number report is retained");
        assert_eq!(first.mode, Mode::RestrictedHorizontal);
        let context = first.context.as_ref().expect("frozen report context");
        assert_eq!(context.group_depth, 1);
        assert_eq!(context.group_tail[0].kind, "adjusted-hbox");
    });
}
#[test]
fn etex_showtokens_uses_recursive_general_text() {
    // e-TeX 2.6 etex.ch [17.3623--3671] routes \showtokens through
    // scan_general_text: its expanded opening-brace search is observable, but
    // the recursive absorbing scope is not a TeX82 scan_toks episode. The
    // following \message is the negative control that still publishes the
    // ordinary §473 absorbing transition.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = etex_initex(stores);
        control.set_fuel_limit(10_000).expect("bounded fuel");
        register_source(&mut control, br"\showtokens\expandafter{X}\message{Y}\end");
        let mut observations = ObservationRecorder::default();
        run_to_end_observed(&mut control, stores, &mut observations);

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
    });
}
#[test]
fn show_macro_body_honors_newlinechar() {
    // TeX82 §§59/262/296/1294: `\show` reaches a macro body through
    // active-selector `token_show`, so character 10 becomes a line break when
    // `\newlinechar=10`. The adjacent control byte proves generated caret
    // notation is not subsequently rescanned as diagnostic input.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = etex_initex(stores);
        control.set_fuel_limit(10_000).expect("bounded fuel");
        register_source(
            &mut control,
            br"\nonstopmode\newlinechar=10\def\shown{A^^JB^^AC}\show\shown\end",
        );
        run_to_end(&mut control, stores);

        let output = terminal_text(stores);
        assert!(
            output.contains("> \\shown=macro:\n->A\nB^^AC."),
            "{output:?}"
        );
        assert!(!output.contains("->A^^JB"), "{output:?}");
    });
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
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            let mut control = etex_initex(stores);
            control.set_fuel_limit(10_000).expect("bounded fuel");
            register_source(&mut control, source);

            run_to_end(&mut control, stores);

            let output = terminal_text(stores);
            for primitive in ["fontcharwd", "fontcharht", "fontchardp", "fontcharic"] {
                assert!(
                    output.contains(&format!("You can't use `\\{primitive}' in ")),
                    "{source:?}: {output}"
                );
            }
        });
    }
}
#[test]
fn standalone_internal_integer_shows_live_context_before_scrolled_help() {
    // TeX82 §§82, 90, 1048, and 1111: a standalone `last_item` reaches
    // `report_illegal_case`; `error` shows the live line before routing help
    // off the terminal in nonstop mode.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        control.set_fuel_limit(1_000).expect("bounded fuel");
        register_source(
            &mut control,
            b"\\nonstopmode\n\\hyphenpenalty 89 \\badness\n\\end",
        );

        run_to_end(&mut control, stores);

        let terminal = pending_sink_text(stores, true);
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
        let log = pending_sink_text(stores, false);
        assert!(
            log.contains("Sorry, but I'm not programmed to handle this case;"),
            "{log}"
        );
    });
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

    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        control.set_fuel_limit(10_000).expect("bounded fuel");
        register_source(&mut control, source.as_bytes());

        run_to_end(&mut control, stores);

        assert_eq!(control.fatal_error(), Some(FatalError::TooManyErrors));
        assert_eq!(stores.world().error_channel().error_count(), 100);
        assert_eq!(
            stores.world().error_channel().history(),
            tex_state::print::ErrorHistory::FatalErrorStop
        );
        assert_eq!(
            stores.count(0).expect("count register"),
            0,
            "fatal exit skips the later assignment"
        );
        assert!(
            pending_sink_text(stores, true).contains("(That makes 100 errors; please try again.)")
        );
        let first = control
            .first_recoverable_diagnostic()
            .expect("aggregate retains first committed recoverable diagnostic");
        assert_eq!(first.kind, "command-recoverable");
        assert_eq!(
            first.message.as_ref(),
            "You can't use `\\badness' in vertical mode"
        );
        assert_eq!(first.mode, Mode::Vertical);
        assert_eq!(first.interaction, tex_state::InteractionMode::Nonstop);
        assert_eq!(first.command.as_deref(), Some("last_item"));
        assert_eq!(first.command_operand, Some(4));
        assert_eq!(
            first.observed_token,
            Some(ObservedToken::ControlSequence("badness".into()))
        );
        assert!(first.origin.is_some(), "TooManyErrors retains first source");
        assert!(
            first.context.is_some(),
            "TooManyErrors retains first context"
        );
    });
}
#[test]
fn errorstop_standalone_internal_integer_prompts_after_live_context_and_resumes() {
    // TeX82 §§82, 90, 1048, and 1111: `report_illegal_case` reaches the
    // interactive advice path after showing context, then resumes on `s`.
    crate::test_harness::with_plain_universe(|stores| {
        stores
            .world_mut()
            .push_memory_terminal_line("s")
            .expect("memory terminal accepts the error response");
        let mut control = MainControl::tex82_initex(stores);
        control.set_fuel_limit(1_000).expect("bounded fuel");
        register_source(&mut control, b"\\badness \\count0=23\\end");

        run_to_end(&mut control, stores);

        let terminal = pending_sink_text(stores, true);
        let context = terminal.find("l.1 \\badness").expect("live context");
        let prompt = terminal.find("? ").expect("interactive prompt");
        assert!(context < prompt, "{terminal:?}");
        assert_eq!(
            stores.count(0).expect("count register"),
            23,
            "interactive recovery resumes input"
        );
        assert_eq!(stores.world().error_channel().error_count(), 0);
        assert_eq!(control.fatal_error(), None);
        assert_eq!(
            control
                .first_recoverable_diagnostic()
                .expect("error-stop report is retained")
                .interaction,
            tex_state::InteractionMode::ErrorStop,
            "capture precedes the dialog's switch to scroll mode"
        );
    });
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
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            let mut control = etex_initex(stores);
            control.set_fuel_limit(10_000).expect("bounded fuel");
            register_source(&mut control, source);

            run_to_end(&mut control, stores);

            let output = terminal_text(stores);
            for primitive in ["parshapelength", "parshapeindent", "parshapedimen"] {
                assert!(
                    output.contains(&format!("You can't use `\\{primitive}' in ")),
                    "{source:?}: {output}"
                );
            }
        });
    }
}
#[test]
fn invalid_middle_and_right_report_missing_delimiter_before_extra_command() {
    // TeX82 §§1160-1161 scan and recover the delimiter before §1192 tests
    // whether the boundary has a matching `\left`. The rejected `\par` is
    // therefore named by both errors, in that order, for each command.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = etex_initex(stores);
        register_source(
            &mut control,
            br"\nonstopmode\tracingonline=1\setbox0=\vbox{\middle \par \right \par}\end",
        );

        run_to_end(&mut control, stores);

        let log = pending_sink_text(stores, false);
        let first_missing = log
            .find("! Missing delimiter (. inserted).")
            .expect("first missing delimiter");
        let extra_middle = log.find("! Extra \\middle.").expect("extra middle");
        let second_missing = log[extra_middle..]
            .find("! Missing delimiter (. inserted).")
            .map(|offset| extra_middle + offset)
            .expect("second missing delimiter");
        let extra_right = log.find("! Extra \\right.").expect("extra right");
        assert!(first_missing < extra_middle);
        assert!(extra_middle < second_missing);
        assert!(second_missing < extra_right);
    });
}
#[test]
fn misplaced_category_five_character_routes_car_ret_help() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        stores.set_interaction_mode(tex_state::InteractionMode::Nonstop);
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, b"\\catcode90=5 Z\n\\global\\count0=17\\end");

        run_to_end(&mut control, stores);

        assert_eq!(stores.count(0).expect("count register"), 17);
        let output = terminal_text(stores);
        assert!(
            output.contains("! Misplaced end of line character Z."),
            "{output}"
        );
    });
}
#[test]
fn bare_macro_parameter_reports_illegal_case_and_continues_in_every_mode() {
    // TeX82 §1045: `any_mode(mac_param): report_illegal_case`.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
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

        run_to_end(&mut control, stores);

        let terminal = terminal_text(stores);
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
        assert_eq!(
            stores.count(0).expect("count register"),
            7,
            "each illegal command is discarded"
        );
    });
}
#[test]
fn production_batch_reuses_admitted_context_for_ordinary_source_run() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_cmr10_as(&mut control, stores, "cmr10.tfm");
        register_source(
            &mut control,
            br"\font\body=cmr10 \body \setbox0=\hbox{ABC}\end",
        );

        loop {
            if control.advance_episode(stores).expect("batch advances")
                == StepResult::Progress(ReplayStep::End)
            {
                break;
            }
        }
        assert!(matches!(
            box_child_nodes(stores, 0).as_slice(),
            [
                Node::Char { ch: 'A', .. },
                Node::Char { ch: 'B', .. },
                Node::Char { ch: 'C', .. },
            ]
        ));
    });
}
#[cfg(feature = "profiling")]
#[test]
fn one_and_4096_warmed_post_apply_fact_settlements_allocate_and_copy_no_context() {
    fn evidence(repetitions: usize) -> tex_state::measurement::HotCoreAllocationMeasurement {
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            let context = stores.command_context().expect("post-apply test admission");
            let context_address = std::ptr::from_ref(&context);
            let owner = tex_state::measurement::HotCoreAllocationOwner::SemanticApply;
            let before = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
            {
                let _scope = tex_state::measurement::hot_core_allocation_scope(owner);
                for _ in 0..repetitions {
                    let facts = PostApplyFacts::capture(
                        MainControlParking {
                            character: Some('A'),
                            resumes_interrupted_fetch: false,
                        },
                        Mode::Horizontal,
                        &context,
                    );
                    std::hint::black_box(facts);
                    assert_eq!(std::ptr::from_ref(&context), context_address);
                }
            }
            let after = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
            tex_state::measurement::HotCoreAllocationMeasurement {
                calls: after.calls - before.calls,
                requested_bytes: after.requested_bytes - before.requested_bytes,
            }
        })
    }

    let one = evidence(1);
    let many = evidence(4_096);
    assert_eq!(one.calls, 0);
    assert_eq!(one.requested_bytes, 0);
    assert_eq!(many.calls, 0);
    assert_eq!(many.requested_bytes, 0);
    assert!(std::mem::size_of::<PostApplyFacts>() < std::mem::size_of::<CommandContext<'_, ()>>());
}
#[test]
fn diagnostic_expand_step_preserves_undefined_for_the_diagnostic_host() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\undefined");
        match control
            .diagnostic_expand_step(stores)
            .expect("diagnostic expansion observes the undefined command")
        {
            DiagnosticStepResult::Progress(DiagnosticStep::Token { meaning, .. }) => {
                assert_eq!(
                    meaning,
                    Meaning::Undefined,
                    "the diagnostic host receives the undefined command instead of recovering"
                );
            }
            other => {
                panic!("diagnostic expansion must return the undefined token, got {other:?}")
            }
        }
    });
}
#[test]
fn committed_fatal_command_reclaims_from_live_roots() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, &spanning_alignment_source(r"\i"));
        let mut ledger = crate::OutputLedger::default();
        let mut checkpoints = Vec::new();

        let cancellation = crate::Cancellation::new();
        let mut terminal = None;
        for _ in 0..1_024 {
            match crate::CanonicalStepRunner::new(&mut control, stores, &mut ledger)
                .step_completing_fatal(&mut checkpoints, &cancellation)
            {
                crate::CanonicalStepResult::Completed(step) => {
                    terminal = Some(step);
                    break;
                }
                crate::CanonicalStepResult::Progress(_)
                | crate::CanonicalStepResult::Committed(_) => {}
                result => panic!("fatal command returned unexpected result: {result:?}"),
            }
        }

        assert!(
            matches!(terminal, Some(ReplayStep::End)),
            "fatal command terminalizes from its live attempt roots: {terminal:?}"
        );
        assert_eq!(
            control.fatal_error(),
            Some(FatalError::confusion("256 spans"))
        );
    });
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
            crate::test_harness::with_nonstop_plain_universe(|stores| {
                let mut control = MainControl::tex82_initex(stores);
                control.set_fuel_limit(128).expect("bounded command fuel");
                if mode != Mode::Vertical {
                    control.modes.push(mode).expect("test mode push");
                }
                register_source(&mut control, br"\endcsname\count0=17");
                if observed {
                    let mut observations = ObservationRecorder::default();
                    for _ in 0..2 {
                        control
                            .advance_with_observer(stores, &mut observations)
                            .expect("observed stray endcsname continues");
                    }
                } else {
                    for _ in 0..2 {
                        control
                            .advance(stores)
                            .expect("unobserved stray endcsname continues");
                    }
                }
                (
                    terminal_text(stores),
                    stores.count(0).expect("count register"),
                    control.fuel_burned(),
                )
            })
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
fn etex_showgroups_detaches_nested_save_and_mode_diagnostics() {
    crate::test_harness::with_nonstop_universe(|stores| {
        let _initialized = MainControl::tex82_initex(stores);
        crate::install_etex_unexpandable_primitives(stores);
        let mut control = MainControl::with_profile(tex_command::CommandProfile::ETEX26);
        register_source(
        &mut control,
        b"\\nonstopmode\n\\tracingonline=1\n\\showgroups\n\\begingroup\\showgroups\\endgroup\n\\global\\showgroups\\count0=7\n\\end",
    );

        run_to_end(&mut control, stores);

        let mut modes = ModeNest::new();
        let mut boxes = ReplayBoxes::default();
        let mut diagnostic_effects = DiagnosticEffects::new();
        crate::test_harness::begin_group(stores, GroupKind::AdjustedHBox, 6).expect("test group");
        modes
            .push(Mode::RestrictedHorizontal)
            .expect("test mode push");
        boxes.active_boxes.push(ActiveReplayBox {
            target: None,
            shipout_region: None,
            kind: ReplayBoxKind::HBox,
            group_kind: GroupKind::AdjustedHBox,
            packing: PackSpec::Exactly(Scaled::from_raw(20 * 65_536)),
            leader_kind: None,
            shift: None,
        });
        let diagnostic = admitted!(stores, |context| detached_showgroups(
            context,
            &None,
            &boxes,
            &[],
            &[],
            &[],
            &[],
        ));
        admitted!(stores, |context| crate::diagnostics::execute_showgroups(
            context,
            &mut diagnostic_effects,
            &diagnostic,
        ));

        crate::test_harness::begin_group(stores, GroupKind::MathShift, 7).expect("test group");
        modes.push(Mode::Math).expect("test mode push");
        crate::test_harness::begin_group(stores, GroupKind::Math, 7).expect("test group");
        modes.push(Mode::Math).expect("test mode push");
        let diagnostic = admitted!(stores, |context| detached_showgroups(
            context,
            &None,
            &boxes,
            &[],
            &[],
            &[],
            &[],
        ));
        admitted!(stores, |context| crate::diagnostics::execute_showgroups(
            context,
            &mut diagnostic_effects,
            &diagnostic,
        ));

        crate::test_harness::begin_group(stores, GroupKind::Align, 8).expect("test group");
        crate::test_harness::begin_group(stores, GroupKind::Align, 8).expect("test group");
        let diagnostic = admitted!(stores, |context| detached_showgroups(
            context,
            &None,
            &boxes,
            &[],
            &[],
            &[],
            &[],
        ));
        admitted!(stores, |context| crate::diagnostics::execute_showgroups(
            context,
            &mut diagnostic_effects,
            &diagnostic,
        ));

        crate::test_harness::begin_group(stores, GroupKind::NoAlign, 8).expect("test group");
        let diagnostic = admitted!(stores, |context| detached_showgroups(
            context,
            &None,
            &boxes,
            &[],
            &[],
            &[],
            &[],
        ));
        admitted!(stores, |context| crate::diagnostics::execute_showgroups(
            context,
            &mut diagnostic_effects,
            &diagnostic,
        ));

        stores
            .world_mut()
            .publish_diagnostic_effects(diagnostic_effects);
        let output = terminal_text(stores);
        for expected in [
            "### bottom level",
            "### semi simple group (level 1) entered at line 4 (\\begingroup)",
            "### adjusted hbox group (level 1) entered at line 6 (\\hbox to20.0pt{)",
            "### math group (level 3) entered at line 7 ({)",
            "### math shift group (level 2) entered at line 7 ($)",
            "### no align group (level 6) entered at line 8 (\\noalign{)",
            "### align group (level 5) entered at line 8 (align entry)",
            "### align group (level 5) entered at line 8 (\\cr)",
            "### align group (level 4) entered at line 8 (\\halign{)",
        ] {
            assert!(
                output.contains(expected),
                "missing {expected:?} in {output:?}"
            );
        }
        assert_eq!(
            stores.count(0).expect("count register"),
            7,
            "prefix recovery consumed following input"
        );
        assert_eq!(
            admitted!(stores, |context| context.execution_group_depth()),
            6,
            "diagnostic mutated the save stack"
        );
    });
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
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            let mut control = MainControl::tex82_initex(stores);
            register_source(&mut control, case.source);
            run_to_end(&mut control, stores);
            let output = terminal_text(stores);
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
            let committed = admitted!(stores, |context| {
                let symbol = if case.target == "~" {
                    context
                        .active_character_symbol('~')
                        .expect("active target is interned")
                } else {
                    context
                        .symbol(case.target)
                        .expect("named target is interned")
                };
                matches!(context.meaning(symbol), ResolvedMeaning::Macro { .. })
            });
            assert_eq!(
                committed, case.committed,
                "{:?}: recovered definition scope",
                case.source
            );
        });
    }
}
#[test]
fn fused_definition_scan_retains_the_first_command_error_context() {
    // TeX.web §§476 and 1218 report this recoverable scanner error while the
    // semi-simple group is still live. Continuing delivery in the same
    // processor must retain that causal point rather than falling back to the
    // terminal state reached later.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\begingroup\def\bad#2{x}\endgroup\end");

        run_to_end(&mut control, stores);

        let context = control
            .first_causal_context
            .as_ref()
            .expect("scanner error captures its live command context");
        assert_eq!(context.cause_kind, "command-error");
        assert_eq!(context.group_depth, 1);
        assert_eq!(context.group_tail.len(), 1);
        assert_eq!(context.group_tail[0].kind, "semi-simple");
    });
}
#[test]
fn macro_tenth_parameter_reports_exact_limit_error() {
    // TeX.web §476 consumes both tokens of the attempted tenth parameter,
    // reports the fixed limit diagnostic, and continues scanning the
    // definition. The resulting macro therefore still has exactly the nine
    // legal parameters and can be called normally.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
        &mut control,
        br"\nonstopmode\def\nine#1#2#3#4#5#6#7#8#9#0{[#1#9]}\message{RESULT:\nine abcdefghi}\end",
    );

        run_to_end(&mut control, stores);

        let terminal = terminal_text(stores);
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
        let parameter_text = admitted!(stores, |context| {
            let nine = context.intern_control_sequence("nine");
            let ResolvedMeaning::Macro { definition, .. } = context.meaning(nine) else {
                panic!("the recovered definition is committed")
            };
            context.definition(definition).parameter_text().to_vec()
        });
        assert_eq!(
            parameter_text,
            (1..=9)
                .map(Token::Param)
                .map(tex_state::token::TokenWord::pack)
                .collect::<Vec<_>>()
        );
        assert!(
            terminal.contains("RESULT:[ai]"),
            "the recovered nine-parameter macro remains callable: {terminal}"
        );
    });
}
#[test]
fn main_control_error_privilege_and_stop_paths_are_finite() {
    crate::test_harness::with_nonstop_plain_universe(|internal_stores| {
        let mut internal = MainControl::tex82_initex(internal_stores);
        internal
            .modes
            .push(Mode::InternalVertical)
            .expect("test mode push");
        register_source(&mut internal, br"\end\count0=9");
        run_to_end(&mut internal, internal_stores);
        assert_eq!(internal_stores.count(0).expect("count register"), 9);
        assert_eq!(internal.current_mode(), Mode::InternalVertical);
        assert!(terminal_text(internal_stores).contains("can't use `\\end'"));

        crate::test_harness::with_nonstop_plain_universe(|page_stores| {
            let mut page = MainControl::tex82_initex(page_stores);
            register_source(&mut page, br"\hrule\end");
            let mut observations = ObservationRecorder::default();
            for _ in 0..32 {
                if matches!(
                    page.advance_with_observer(page_stores, &mut observations)
                        .expect("page stop remains finite"),
                    StepResult::Progress(MainControlStep::End | MainControlStep::EndOfInput)
                ) {
                    break;
                }
            }
            assert_eq!(page_stores.world().artifact_commits().len(), 1);
            assert!(observations.0.iter().any(|observation| matches!(
        observation,
        CommandObservation::Effect(effect) if effect.kind == ObservationEffectKind::Terminate
    )));
        });
    });
}
#[test]
fn illegal_case_command_spelling_uses_live_escapechar() {
    // TeX82 §§63, 298, and 1049: `you_cant` renders the rejected command
    // through `print_cmd_chr`; its primitive cases use `print_esc`, whose
    // escape prefix is omitted when `\escapechar` is outside 0..255.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        control
            .modes
            .push(Mode::InternalVertical)
            .expect("test mode push");
        register_source(&mut control, br"\escapechar=256\end");
        run_to_end(&mut control, stores);

        let terminal = terminal_text(stores);
        assert!(
            terminal.contains("You can't use `end' in internal vertical mode"),
            "{terminal:?}"
        );
        assert!(!terminal.contains("You can't use `\\end'"), "{terminal:?}");
    });
}
/// TeX82 §314's macro arm is `print_ln; print_cs(name)`, and §319
/// pseudoprints `link(start)` -- the whole macro text -- so a macro level's
/// context line is `\\a #1->body`, naming the control sequence being expanded
/// and showing its parameter text ahead of the `->` §294 renders for
/// `end_match`.
#[test]
fn a_macro_context_level_names_the_macro_and_shows_its_parameter_text() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        crate::test_harness::assign_int_param(
            stores,
            tex_state::env::banks::IntParam::new(54),
            5,
            tex_state::AssignmentScope::Global,
        )
        .expect("integer parameter assignment");
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\def\a#1{ x #1 \undefinedthing y}\a{Q}\end");
        run_to_end(&mut control, stores);
        let terminal = terminal_text(stores);
        assert!(
            terminal.contains("\\a #1-> x #1 \\undefinedthing \n"),
            "{terminal}"
        );
        assert!(!terminal.contains("<macro>"), "{terminal}");
    });
}
/// TeX82 §1068's `handle_right_brace` sends `semi_simple_group`,
/// `math_shift_group` and `math_left_group` to §1069's `extra_right_brace`,
/// which names the opener the brace was standing in for. Only the remaining
/// `bottom_level` case is "Too many }'s".
#[test]
fn a_stray_right_brace_names_the_group_opener_it_replaced() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\hbox{$x}$}\begingroup}\end");
        run_to_end(&mut control, stores);
        let terminal = terminal_text(stores);
        assert!(
            terminal.contains("! Extra }, or forgotten $."),
            "{terminal}"
        );
        assert!(
            terminal.contains("! Extra }, or forgotten \\endgroup."),
            "{terminal}"
        );
        assert!(!terminal.contains("Too many }'s"), "{terminal}");
    });
}
#[test]
fn extra_right_brace_in_an_argument_names_the_macro() {
    // TeX82 §395: a bare `}` where an argument was expected is backed up, a
    // `\\par` is inserted, and `ins_error` reports "Argument of \\a has an
    // extra }" -- `sprint_cs(warning_index)`, the macro whose argument was
    // being matched, not a placeholder.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\def\a#1{[#1]}\a}\end");
        run_to_end(&mut control, stores);
        let terminal = terminal_text(stores);
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
    });
}
#[test]
fn out_of_range_read_selector_reaches_the_terminal_without_a_report() {
    // TeX82 §1225 scans `\\read`'s stream with a plain `scan_int`, not §435's
    // `scan_four_bit_int`, and §482 answers `(n<0)or(n>15)` with `m:=16` --
    // the never-open stream whose §483 branch is the terminal. Stream 16 is
    // therefore an ordinary terminal read, not a recovered zero, and nothing
    // is reported.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        stores.set_interaction_mode(tex_state::InteractionMode::Scroll);
        stores
            .world_mut()
            .push_memory_terminal_line("recovered")
            .expect("terminal line queues");
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\read16 to \line\end");
        let mut observations = ObservationRecorder::default();
        for _ in 0..64 {
            if matches!(
                control
                    .advance_with_observer(stores, &mut observations)
                    .expect("recovered read remains executable"),
                StepResult::Progress(MainControlStep::End | MainControlStep::EndOfInput)
            ) {
                break;
            }
        }

        assert_eq!(
            macro_semantic_tokens(stores, "line")[0],
            Token::Char {
                ch: 'r',
                cat: Catcode::Letter,
            }
        );
        let terminal = terminal_text(stores);
        assert!(!terminal.contains("Bad number"), "{terminal}");
        let integer = observations
            .0
            .iter()
            .position(|event| {
                matches!(
                    event,
                    CommandObservation::Scanner(scanner)
                        if scanner.kind == "integer"
                            && scanner.value == ObservationValue::Integer(16)
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
                        if observation_name(&mutation.key) == Some("line")
                )
            })
            .expect("recovered read target is committed");
        assert!(integer < mutation);
    });
}
#[test]
fn message_expands_balanced_text_and_applies_terminal_line_spacing() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\def\value{expanded}\message{left {\value} right}\count0=7\end",
        );
        run_to_end(&mut control, stores);
        assert_eq!(terminal_text(stores), "left {expanded} right");
        assert_eq!(
            stores.count(0).expect("count register"),
            7,
            "message consumes its body exactly once"
        );
    });
}
#[test]
fn message_slow_prints_nonprintable_character_tokens() {
    // tex.web §§59, 1279: message text is a string, so character 13 uses the
    // one-character string spelling rather than §58's raw `print_char` path.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\newlinechar=10\message{READLINE:[macro:->Alpha ^^M]}\end",
        );
        run_to_end(&mut control, stores);
        assert_eq!(terminal_text(stores), "READLINE:[macro:->Alpha ^^M]");
    });
}
#[test]
fn errmessage_selects_user_or_once_only_builtin_help_and_clears_flag() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\def\value{expanded}\errmessage{bad \value}\count0=8\end",
        );
        run_to_end(&mut control, stores);
        let output = terminal_text(stores);
        assert_eq!(output.matches("! bad expanded.").count(), 1, "{output}");
        assert_eq!(
            stores.count(0).expect("count register"),
            8,
            "error handling resumes main control"
        );
    });
}
#[test]
fn show_dispatch_selects_activities_box_meaning_or_value_without_mode_dependence() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
        &mut control,
        br"\def\shown{expanded}\show\shown\count0=17\showthe\count0\setbox0=\hbox{}\showbox0\end",
    );
        run_to_end(&mut control, stores);
        let output = terminal_text(stores);
        // §296's `print_meaning` breaks the line after a macro's `:`, so the
        // replacement text starts its own line under `\show` (but not under
        // `\meaning`, which runs the same routine at §471's `new_string`
        // selector, where `print_ln` does nothing).
        assert!(output.contains("> \\shown=macro:\n->expanded."), "{output}");
        assert!(output.contains("> 17."), "{output}");
        assert!(output.contains("> \\box0="), "{output}");
    });
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
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            stores.set_interaction_mode(mode);
            crate::test_harness::assign_int_param(
                stores,
                IntParam::NEWLINE_CHAR,
                10,
                tex_state::AssignmentScope::Global,
            )
            .expect("integer parameter assignment");
            if mode == tex_state::InteractionMode::ErrorStop {
                stores
                    .world_mut()
                    .push_memory_terminal_line("s")
                    .expect("memory terminal accepts the show response");
            }
            stores.printer().print("\\show\\errorstopmode").print_ln();
            let mut control = MainControl::tex82_initex(stores);
            stores.set_interaction_mode(mode);
            register_source(&mut control, br"\show\errorstopmode\end");
            run_to_end(&mut control, stores);

            let terminal = pending_sink_text(stores, true);
            let log = pending_sink_text(stores, false);
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
        });
    }
}
#[test]
fn errorstop_show_reports_live_source_context_before_prompting_and_resumes() {
    // TeX82 §§82/1293: every show common ending calls `error`, and `error`
    // shows the still-live input cursor before asking for terminal advice.
    crate::test_harness::with_plain_universe(|stores| {
        stores
            .world_mut()
            .push_memory_terminal_line("s")
            .expect("memory terminal accepts the show response");
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\show\errorstopmode\count0=23\end");

        run_to_end(&mut control, stores);

        let output = terminal_text(stores);
        assert!(
            output.contains("l.1 \\show\\errorstopmode\n                       \\count0=23\\end"),
            "{output:?}"
        );
        assert!(
            output.find("l.1 \\show\\errorstopmode").expect("context")
                < output.find("? ").expect("prompt"),
            "{output:?}"
        );
        assert_eq!(
            stores.count(0).expect("count register"),
            23,
            "show leaves the following input live"
        );
        assert_eq!(
            stores.world().error_channel().error_count(),
            0,
            "interactive show does not enter the scrolled error count"
        );
    });
}
#[test]
fn error_stop_inserts_replacement_line_before_suspended_input_once() {
    // TeX82 §87 opens the typed replacement as a new terminal source level;
    // it retires once, then the exact suspended source resumes underneath it.
    crate::test_harness::with_plain_universe(|stores| {
        stores
            .world_mut()
            .push_memory_terminal_line("I")
            .expect("insertion response queues");
        stores
            .world_mut()
            .push_memory_terminal_line("\\count0=17")
            .expect("replacement line queues");
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\show\errorstopmode\advance\count1 by 23\end",
        );

        run_to_end(&mut control, stores);

        assert_eq!(stores.count(0).expect("count register"), 17);
        assert_eq!(stores.count(1).expect("count register"), 23);
        let log = pending_sink_text(stores, false);
        assert_eq!(log.matches("\\count0=17").count(), 1, "{log:?}");
        assert!(log.contains("insert> \\count0=17\n"), "{log:?}");
    });
}
#[test]
fn consecutive_shows_and_following_error_preserve_only_canonical_separators() {
    // TeX82 §§82/90/1293 leave one blank separator after each noninteractive
    // show completion. The following `print_nl` must not add another.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\nonstopmode\show\errorstopmode\show\scrollmode\undefined\end",
        );
        run_to_end(&mut control, stores);

        let output = terminal_text(stores);
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
    });
}
#[test]
fn showlists_is_a_diagnostic_without_a_canonical_effect_event() {
    // TeX82 §1293 writes `show_activities` through the diagnostic printer.
    // The schema-v1 command stream has no detached effect for that report;
    // only actual engine effects such as messages, writes, and termination
    // are published as effect observations.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\showlists\end");
        let mut observations = ObservationRecorder::default();
        loop {
            match control
                .advance_with_observer(stores, &mut observations)
                .expect("showlists executes")
            {
                StepResult::Progress(MainControlStep::End | MainControlStep::EndOfInput) => break,
                StepResult::Progress(MainControlStep::Continue) => {}
                StepResult::Suspended(need) => panic!("unexpected resource suspension: {need:?}"),
            }
        }

        assert!(terminal_text(stores).contains("### vertical mode"));
        assert!(observations.0.iter().all(|observation| {
            !matches!(observation, CommandObservation::Effect(effect)
            if effect.kind != ObservationEffectKind::Terminate)
        }));
    });
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
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            let mut control = if extended {
                etex_initex(stores)
            } else {
                MainControl::tex82_initex(stores)
            };
            register_source(&mut control, SOURCE);

            // Stop immediately before the first diagnostic, after the interaction
            // command, assignments, and symbolic register aliases have committed.
            while stores.count(255).expect("count register") == 0 {
                assert_eq!(
                    control.advance(stores).expect("setup command executes"),
                    StepResult::Progress(MainControlStep::Continue)
                );
            }
            let glue_parameters = (0..18)
                .map(|index| admitted!(stores, |context| context.glue_param(GlueParam::new(index))))
                .collect::<Vec<_>>();
            let skip = admitted!(stores, |context| context
                .glue_register(0)
                .expect("skip register")
                .expect("assigned skip"));
            let muskip = admitted!(stores, |context| context.muskip(0));

            run_to_end(&mut control, stores);
            let output = terminal_text(stores);

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
                    .map(|index| admitted!(stores, |context| context
                        .glue_param(GlueParam::new(index))))
                    .collect::<Vec<_>>(),
                glue_parameters,
                "profile extended={extended} changed a parameter bank"
            );
            assert_eq!(
                admitted!(stores, |context| context
                    .glue_register(0)
                    .expect("skip register")
                    .expect("assigned skip")),
                skip,
                "profile extended={extended}"
            );
            assert_eq!(
                admitted!(stores, |context| context.muskip(0)),
                muskip,
                "profile extended={extended}"
            );
        });
    }
}
#[test]
fn showbox_scans_register_and_distinguishes_void_from_box_contents() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
        &mut control,
        br"\showboxbreadth=10\showboxdepth=10\setbox0=\hbox{\kern1pt}\setbox255=\hbox{}\showbox0\showbox255\showbox1\end",
    );
        run_to_end(&mut control, stores);
        let output = terminal_text(stores);
        assert!(output.contains("> \\box0="), "{output}");
        assert!(output.contains("\\kern 1.0"), "{output}");
        assert!(output.contains("> \\box255="), "{output}");
        assert!(output.contains("> \\box1=void"), "{output}");
        assert!(!output.contains("> \\box1=\nvoid"), "{output}");
        let first_dump = output.find("> \\box0=").expect("first showbox dump");
        let first_completion = output
            .find("! OK")
            .unwrap_or_else(|| panic!("§1293 completion missing from: {output}"));
        assert!(
            first_dump < first_completion,
            "the detached box dump must publish before §1293's completion: {output}"
        );
    });
}
#[test]
fn showthe_display_and_completion_follow_its_command_trace() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\tracingcommands=1\dimen0=1050pt\showthe\dimen0\end",
        );

        run_to_end(&mut control, stores);

        let log = pending_sink_text(stores, false);
        let trace = log
            .find("{\\showthe}")
            .unwrap_or_else(|| panic!("missing showthe command trace: {log}"));
        let display = log
            .find("> 1050.0pt.")
            .unwrap_or_else(|| panic!("missing showthe display: {log}"));
        assert!(trace < display, "showthe overtook its command trace: {log}");
    });
}
#[test]
fn showbox_retains_the_node_after_a_discretionary_replacement() {
    // TeX82 §§115/162 links replacement nodes after the discretionary,
    // and §182 resumes its outer diagnostic traversal after that span.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_cmr10_as(&mut control, stores, "cmr10.tfm");
        register_source(
        &mut control,
        br"\font\f=cmr10 \f\showboxbreadth=10\showboxdepth=10\setbox0=\hbox{a\discretionary{b}{c}{d}e}\showbox0\end",
    );

        run_to_end(&mut control, stores);

        let output = terminal_text(stores);
        assert!(
            output
                .contains(".\\f a\n.\\discretionary replacing 1\n..\\f b\n.|\\f c\n.\\f d\n.\\f e"),
            "{output}"
        );
    });
}
#[test]
fn showthe_uses_the_toks_for_each_internal_value_family_and_releases_output() {
    // TeX82 §§262/1297: the font identifier becomes a token shown through
    // `print_cs`, whose control-word delimiter precedes the display period.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let nullfont = stores.intern("nullfont").expect("symbol interning");
        admitted!(stores, |context| context
            .set_font_identifier_symbol(tex_state::font::NULL_FONT, nullfont,));
        let mut control = MainControl::tex82_initex(stores);
        register_source(
        &mut control,
        br"\count0=17\skip0=1pt plus 2fil\toks0={abc}\showthe\count0\showthe\skip0\showthe\font\showthe\toks0\end",
    );
        run_to_end(&mut control, stores);
        let output = terminal_text(stores);
        assert!(output.contains("> 17."), "{output}");
        assert!(output.contains("> 1.0pt plus 2.0fil."), "{output}");
        assert!(output.contains("> \\nullfont ."), "{output}");
        assert!(output.contains("> abc."), "{output}");
    });
}
#[test]
fn showthe_token_lists_use_print_cs_separator_rules() {
    // TeX82 §§262/1297: `\showthe` applies `token_show`, not `\string`, to
    // token-list values. Hash-table control words always gain a separator;
    // direct-address control symbols and active characters do not.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\catcode`\~=13 \toks0={A\count1\!B\?C~D\relax\!}\showthe\toks0\end",
        );

        run_to_end(&mut control, stores);

        assert!(
            terminal_text(stores).contains("> A\\count 1\\!B\\?C~D\\relax \\!."),
            "{}",
            terminal_text(stores)
        );
    });
}
#[test]
fn show_completion_routes_transcript_and_adjusts_error_count_by_interaction() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        stores.set_interaction_mode(tex_state::InteractionMode::Nonstop);
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\showthe\count0\count1=9\end");
        run_to_end(&mut control, stores);
        assert!(terminal_text(stores).contains("> 0."));
        assert_eq!(
            stores.count(1).expect("count register"),
            9,
            "show completion resumes execution"
        );
    });
}
#[test]
fn setlanguage_illegal_mode_recovers_without_scan_or_append() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        // TeX82 §1377 tests `abs(mode)<>hmode` before `new_whatsit` and before
        // `scan_int`, so the operand is never consumed: the following assignment
        // is the very next command main control sees.
        register_source(
            &mut control,
            br"\setbox0=\vbox{\setlanguage\global\count0=5}\end",
        );
        run_to_end(&mut control, stores);
        assert_eq!(stores.count(0).expect("count register"), 5);
        let text = terminal_text(stores);
        assert!(
            text.contains("You can't use `\\setlanguage' in internal vertical mode"),
            "{text}"
        );
        let outer = stores
            .copy_box_to_page(0)
            .expect("box 0 holds the constructed vbox");
        let Some(Node::VList(boxed)) = first_published_node(stores, outer) else {
            panic!("box 0 holds a vlist");
        };
        assert!(
            !page_vec(stores, boxed.children)
                .iter()
                .any(|node| matches!(node, Node::Whatsit(_))),
            "no whatsit is appended when the mode test fails"
        );
    });
}
#[test]
fn a_succumbed_session_stays_terminal_without_delivering_another_command() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, &spanning_alignment_source(r"\i"));

        run_to_end(&mut control, stores);
        let fatal = control.fatal_error();

        for _ in 0..4 {
            assert_eq!(
                control.advance(stores).expect("a terminal session reports"),
                StepResult::Progress(MainControlStep::End),
            );
        }
        assert_eq!(control.fatal_error(), fatal);
        assert_eq!(stores.count(0).expect("count register"), 0);
    });
}
#[test]
fn succumbing_commits_fatal_diagnostic_then_engine_termination() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, &spanning_alignment_source(r"\i"));

        let mut observations = ObservationRecorder::default();
        loop {
            match control
                .advance_with_observer(stores, &mut observations)
                .expect("a fatal error is a terminal state, never an Err")
            {
                StepResult::Progress(MainControlStep::End | MainControlStep::EndOfInput) => break,
                StepResult::Progress(MainControlStep::Continue) => {}
                StepResult::Suspended(need) => panic!("unexpected resource suspension: {need:?}"),
            }
        }

        let fatal = FatalError::confusion("256 spans");
        assert_eq!(control.fatal_error(), Some(fatal));
        assert!(matches!(
            observations.0.as_slice(),
            [.., CommandObservation::Diagnostic(record), CommandObservation::Effect(effect)]
                if *record == fatal.record()
                    && effect.kind == ObservationEffectKind::Terminate
                    && effect.channel == "engine"
        ));
        let terminal = pending_sink_text(stores, true);
        let log = pending_sink_text(stores, false);
        for output in [&terminal, &log] {
            assert!(
                output.contains("! This can't happen (256 spans)."),
                "{output:?}"
            );
            assert!(output.contains("<template> \\endtemplate"), "{output:?}");
        }
        assert!(
            log.contains("I'm broken. Please show this to someone who can fix can fix"),
            "{log:?}"
        );
        assert!(
            !terminal.contains("I'm broken. Please show this to someone who can fix can fix"),
            "{terminal:?}"
        );
    });
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
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            let original: Vec<_> = (1..=7)
                .map(|number| {
                    admitted!(stores, |context| context
                        .font_parameter(tex_state::font::NULL_FONT, number))
                })
                .collect();
            let mut control = MainControl::tex82_initex(stores);
            register_source(&mut control, source);
            run_to_end(&mut control, stores);

            assert_eq!(
                stores.count(0).expect("count register"),
                trailing_count,
                "{source:?}"
            );
            assert_eq!(
                admitted!(stores, |context| context
                    .font_parameter_count(tex_state::font::NULL_FONT)),
                final_len
            );
            assert_eq!(
                (1..=7)
                    .map(|number| admitted!(stores, |context| context
                        .font_parameter(tex_state::font::NULL_FONT, number)))
                    .collect::<Vec<_>>(),
                original,
                "{source:?}"
            );
            if final_len == 8 {
                assert_eq!(
                    admitted!(stores, |context| context
                        .font_parameter(tex_state::font::NULL_FONT, 8)),
                    Scaled::from_raw(Scaled::UNITY)
                );
            }
            let output = terminal_text(stores);
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
        });
    }
}
#[test]
fn malformed_tfm_recovers_to_nullfont_with_assignment_scope() {
    // TeX82 §564 reports malformed metrics without interning a partial font.
    // A local failed definition must roll back at group end, while a global
    // failed definition leaves the selector bound to nullfont.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_cmr10_as(&mut control, stores, "cmr10.tfm");
        stores
            .world_mut()
            .set_memory_file("broken.tfm", b"not a TFM".to_vec())
            .expect("malformed font fixture installs");
        let metrics = InputReadState::read_input_file(
            &mut stores.input_open_context(),
            std::path::Path::new("broken.tfm"),
        )
        .expect("malformed font fixture reads");
        control.capabilities_mut().register_font(
            "broken.tfm",
            FontResource::Tfm {
                metrics,
                opentype: None,
            },
        );
        register_source(
            &mut control,
            br"\font\local=cmr10 {\font\local=broken }\global\font\globalbad=broken \end",
        );

        run_to_end(&mut control, stores);

        assert_ne!(font_by_name(stores, "local"), tex_state::font::NULL_FONT);
        assert_eq!(
            font_by_name(stores, "globalbad"),
            tex_state::font::NULL_FONT
        );
        let output = terminal_text(stores);
        assert_eq!(
            output
                .matches("not loadable: Bad metric (TFM) file")
                .count(),
            2,
            "{output}"
        );
    });
}
#[test]
fn invalid_arithmetic_target_recovers_and_fires_afterassignment() {
    // TeX82 §1236 consumes an invalid target, reports the error, and returns
    // through §1269's common path, which still replays `\afterassignment`.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
        &mut control,
        br"\prevdepth=2pt \def\mark{\global\count0=7}\afterassignment\mark\advance\prevdepth \count1=9\end",
    );
        run_to_end(&mut control, stores);

        assert_eq!(
            stores.count(0).expect("count register"),
            7,
            "afterassignment token was replayed"
        );
        assert_eq!(
            stores.count(1).expect("count register"),
            9,
            "execution continued after the error"
        );
        assert_eq!(
            control.modes.current_list().prev_depth(),
            Some(Scaled::from_raw(2 * 65_536))
        );
        let output = terminal_text(stores);
        assert!(
            output.contains("! You can't use `\\prevdepth' after \\advance."),
            "{output}"
        );
    });
}
#[test]
fn invalid_arithmetic_target_uses_live_escapechar_for_operator() {
    // TeX82 §§63/298/1236: both commands in the diagnostic are printed via
    // `print_cmd_chr`/`print_esc`, so neither spelling hardcodes a backslash.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        crate::test_harness::assign_int_param(
            stores,
            tex_state::env::banks::IntParam::ESCAPE_CHAR,
            i32::from(b'|'),
            tex_state::AssignmentScope::Global,
        )
        .expect("escape character assignment");
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\advance\prevdepth\end");
        run_to_end(&mut control, stores);

        let output = terminal_text(stores);
        assert!(
            output.contains("! You can't use `|prevdepth' after |advance."),
            "{output}"
        );
    });
}
#[test]
fn invalid_arithmetic_targets_use_print_cmd_chr_and_commit_without_mutation() {
    // TeX82 §§298 and 1236 print the rejected command class, scan no operand,
    // and return through §1269 once. Prefix scope is therefore immaterial,
    // including both \globaldefs overrides.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
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
        run_to_end_observed(&mut control, stores, &mut observations);

        assert_eq!(
            stores.count(0).expect("count register"),
            3,
            "each afterassignment fires exactly once"
        );
        assert_eq!(
            stores.count(1).expect("count register"),
            19,
            "no rejected command scans an operand"
        );
        assert!(
            observations
                .0
                .iter()
                .any(|event| matches!(event, CommandObservation::Mutation(_))),
            "observer exercised the surrounding valid assignments"
        );

        let output = terminal_text(stores);
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

        crate::test_harness::with_nonstop_plain_universe(|isolated_stores| {
            let mut isolated = MainControl::tex82_initex(isolated_stores);
            register_source(&mut isolated, br"\advance x");
            let mut isolated_observations = ObservationRecorder::default();
            isolated
                .advance_with_observer(isolated_stores, &mut isolated_observations)
                .expect("observed invalid target recovers");
            assert!(
                !isolated_observations
                    .0
                    .iter()
                    .any(|event| matches!(event, CommandObservation::Mutation(_))),
                "invalid target must not publish a mutation: {:?}",
                isolated_observations.0
            );
        });
    });
}
#[test]
fn message_spacing_follows_the_texweb_1280_offset_rule() {
    // TeX82 §1280 separates consecutive `\message` texts with one space when
    // a line is already open.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\message{a}\message{b}\end");
        run_to_end(&mut control, stores);

        assert!(
            terminal_text(stores).contains("a b"),
            "{}",
            terminal_text(stores)
        );
    });
}
#[test]
fn errmessage_prefers_errhelp_over_the_builtin_help() {
    // TeX82 §1283: `if err_help<>null then use_err_help:=true`, and §90 shows
    // it on the transcript.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\nonstopmode\errhelp={user help}\errmessage{bad}\count0=1\end",
        );
        run_to_end(&mut control, stores);

        assert_eq!(stores.count(0).expect("count register"), 1);
        let output = terminal_text(stores);
        assert!(output.contains("! bad."), "{output}");
        assert!(output.contains("user help"), "{output}");
        assert!(!output.contains("Hercule Poirot"), "{output}");
    });
}
#[test]
fn show_completion_prompts_in_error_stop_mode_and_honors_the_answer() {
    // TeX82 §1293's `common_ending: ...; error`, whose §83 dialog prompts
    // `?␣` and whose §86 `S` answer switches to scroll mode.
    crate::test_harness::with_plain_universe(|stores| {
        stores
            .world_mut()
            .push_memory_terminal_line("s")
            .expect("memory terminal accepts a line");
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\showthe\count0 \count1=1\end");
        run_to_end(&mut control, stores);

        assert_eq!(stores.count(1).expect("count register"), 1);
        let output = terminal_text(stores);
        assert!(output.contains("> 0."), "{output}");
        assert!(output.contains("? "), "{output}");
        assert_eq!(
            stores.interaction_mode(),
            tex_state::InteractionMode::Scroll
        );
    });
}
#[test]
fn undefined_control_sequence_reports_once_and_drops_only_its_token() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\nonstopmode\missing\count0=17\end");
        run_to_end(&mut control, stores);
        assert_eq!(
            stores.count(0).expect("count register"),
            17,
            "the following command remains live"
        );
        assert_eq!(stores.world().error_channel().error_count(), 1);
        let output = terminal_text(stores);
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
        let first = control
            .first_recoverable_diagnostic()
            .expect("committed recoverable diagnostic");
        assert_eq!(first.kind, "undefined-control-sequence");
        assert_eq!(first.message.as_ref(), "Undefined control sequence");
        assert_eq!(first.command.as_deref(), Some("undefined_cs"));
        assert_eq!(first.command_operand, Some(-268_435_455));
        assert_eq!(
            first.observed_token,
            Some(tex_command::ObservedToken::ControlSequence("^^@".into()))
        );
        assert_eq!(first.mode, Mode::Vertical);
        assert_eq!(first.scanner_status, "normal");
        assert_eq!(first.interaction, tex_state::InteractionMode::Nonstop);
        assert!(first.origin.is_some(), "source origin is retained");
        assert!(first.context.is_some(), "input context is retained");
    });
}
#[test]
fn batch_undefined_recovery_keeps_the_log_only_selector() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        stores.set_interaction_mode(tex_state::InteractionMode::Batch);
        register_source(&mut control, br"\missing\count0=23\end");
        run_to_end(&mut control, stores);

        assert_eq!(
            stores.count(0).expect("count register"),
            23,
            "batch recovery continues the job"
        );
        assert_eq!(stores.world().error_channel().error_count(), 1);
        assert!(
            !pending_sink_text(stores, true).contains("Undefined control sequence"),
            "batch errors must not escape the log-only selector"
        );
        assert!(
            pending_sink_text(stores, false).contains("Undefined control sequence"),
            "batch errors remain in the transcript log"
        );
    });
}
#[test]
fn implicit_paragraph_pack_diagnostic_retains_its_input_line_range() {
    // TeX82 §§661--663: `new_graf` saves the current input line as
    // `pack_begin_line`; the closing vertical-box brace supplies the ending
    // line. The detached context must not replace either with zero.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            b"\\tracingonline=1\\hbadness=0\\hsize=0pt\\parindent=10pt\n\\setbox0=\\vbox{\\indent\n}\n\\end",
        );
        run_to_end_observed(&mut control, stores, &mut ObservationRecorder::default());

        let mut log = stores
            .world()
            .memory_log_output()
            .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
            .unwrap_or_default();
        log.push_str(&pending_sink_text(stores, false));
        assert!(
            log.contains("in paragraph at lines 2--3"),
            "paragraph pack origin must retain its source range: {log}"
        );
        assert!(!log.contains("detected at line 0"), "{log}");
    });
}
#[test]
fn display_interruption_pack_diagnostic_retains_its_input_line_range() {
    // TeX82 §§1138/661: the opening display shift ends the surrounding
    // paragraph and its still-live input line is the ending line reported by
    // `hpack`. Detaching the diagnostic presentation must not erase it.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            b"\\tracingonline=1\\hbadness=0\\hsize=0pt\\parindent=10pt\n\\setbox0=\\vbox{\\indent\n$$x$$\n}\n\\end",
        );
        run_to_end_observed(&mut control, stores, &mut ObservationRecorder::default());

        let mut log = stores
            .world()
            .memory_log_output()
            .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
            .unwrap_or_default();
        log.push_str(&pending_sink_text(stores, false));
        assert!(
            log.contains("in paragraph at lines 2--3"),
            "display-interrupted paragraph must retain its source range: {log}"
        );
        assert!(!log.contains("detected at line 0"), "{log}");
    });
}
#[test]
fn message_prints_expansion_trace_before_expanded_text() {
    // TeX82 §§366/1279: macro expansion and its tracing finish while
    // scanning the message token list; only then does `issue_message` print
    // the expanded text through the live selector.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            b"\\tracingonline=1\\tracingmacros=1\\def\\a{PAYLOAD}\\message{MESSAGE:\\a}\\end",
        );
        run_to_end_observed(&mut control, stores, &mut ObservationRecorder::default());

        let output = terminal_text(stores);
        let trace = output
            .find("\\a ->PAYLOAD")
            .unwrap_or_else(|| panic!("missing expansion trace from {output:?}"));
        let message = output
            .find("MESSAGE:PAYLOAD")
            .unwrap_or_else(|| panic!("missing expanded message from {output:?}"));
        assert!(
            trace < message,
            "message overtook expansion trace: {output:?}"
        );
    });
}
#[test]
fn batch_undefined_recovery_after_a_live_mode_transition_keeps_the_log_only_selector() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\nonstopmode\batchmode\missing\scrollmode\end",
        );
        run_to_end_observed(&mut control, stores, &mut ObservationRecorder::default());

        assert_eq!(
            stores.interaction_mode(),
            tex_state::InteractionMode::Scroll
        );
        assert_eq!(stores.world().error_channel().error_count(), 1);
        assert!(
            !pending_sink_text(stores, true).contains("Undefined control sequence"),
            "batch errors must not escape the log-only selector after a live transition"
        );
        assert!(
            pending_sink_text(stores, false).contains("Undefined control sequence"),
            "batch errors remain in the transcript log after a live transition"
        );
    });
}
#[test]
fn misplaced_tab_reports_once_and_drops_only_the_delimiter() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\nonstopmode&\count0=19\end");
        run_to_end(&mut control, stores);
        assert_eq!(
            stores.count(0).expect("count register"),
            19,
            "the delimiter was not backed up"
        );
        assert_eq!(stores.world().error_channel().error_count(), 1);
        let output = terminal_text(stores);
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
    });
}
#[test]
fn long_prefix_on_let_reports_tex_prefix_error() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\nonstopmode\long\let\a=b");
        run_to_end(&mut control, stores);
        assert!(terminal_text(stores).contains("You can't use `\\long'"));
        assert_eq!(
            admitted!(stores, |context| {
                let a = context.intern_control_sequence("a");
                context.meaning(a)
            }),
            ResolvedMeaning::Static(Meaning::CharToken {
                ch: 'b',
                cat: Catcode::Letter
            })
        );
    });
}
#[test]
fn etex_showgroups_and_showifs_render_live_nested_stacks() {
    with_etex(
        br"\nonstopmode\begingroup\iftrue\showgroups\showifs\fi\endgroup",
        |stores| {
            let output = terminal_text(stores);
            assert!(
                output.contains("### semi simple group (level 1) entered at line 1 (\\begingroup)"),
                "{output}"
            );
            assert!(output.contains("### bottom level"));
            assert!(output.contains("### level 1: \\iftrue"), "{output}");
        },
    );
}
