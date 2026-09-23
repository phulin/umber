//! Page contribution, output routines, deferred effects, and final publication.

use super::*;

#[test]
fn automatic_output_box_remains_page_owned_until_shipout() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\output={\shipout\box255}\vsize=5pt\hrule height10pt\penalty-10000\end",
        );

        run_to_end(&mut control, stores);

        assert_eq!(stores.world().committed_artifacts().len(), 1);
        let lifecycle = stores.page_region_counters();
        assert_eq!(
            lifecycle.page_to_durable_nodes_copied, 0,
            "automatic box 255 remains a page-region root"
        );
        assert_eq!(
            lifecycle.history_preservation_nodes_copied, 0,
            "output handoff and page succession preserve the same coordinates"
        );
    });
}
#[test]
fn finish_job_publishes_each_live_stack_owner() {
    // TeX82 §1334 reports five independently owned maxima. Derive the
    // expected row from those owners after a source that exercises all five;
    // this catches a zero-filled detachment seam without pinning any corpus
    // fixture's totals.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\tracingstats=1\def\m#1{#1}\m{\relax}\setbox0=\hbox{\hbox{{\count0=1\aftergroup\relax}}}\end",
        );
        run_to_end(&mut control, stores);

        let command = control.command.stack_usage();
        let nest = control.modes.maximum_saved_depth();
        let save = control.max_save_stack.saturating_add(6);
        assert!(command.input_stack > 0);
        assert!(nest > 0);
        assert!(command.parameter_stack > 0);
        assert!(command.buffer_stack > 0);
        assert!(save > 6);
        let expected = format!(
            "{}i,{}n,{}p,{}b,{}s stack positions",
            command.input_stack, nest, command.parameter_stack, command.buffer_stack, save
        );

        control.finish_job(stores, None, None);
        assert!(pending_sink_text(stores, false).contains(&expected));
    });
}
#[test]
fn tracingcommands_does_not_trace_output_routine_scanner_brace() {
    // TeX82 §§1025/1030: `scan_left_brace` consumes the output routine's
    // opening brace before `big_switch`. The first body command therefore
    // receives the internal-vertical-mode prefix instead of the brace.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        stores.set_interaction_mode(tex_state::InteractionMode::Nonstop);
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\tracingcommands=1\tracingonline=1
\maxdeadcycles=1\output={\dimen0=1pt}
\topskip=0pt\setbox0=\vbox to1pt{}\copy0\penalty-10000\end",
        );

        run_to_end(&mut control, stores);

        let terminal = terminal_text(stores);
        assert!(
            terminal.contains("{internal vertical mode: \\dimen}"),
            "{terminal:?}"
        );
        assert!(!terminal.contains("begin-group character"), "{terminal:?}");
    });
}
#[test]
fn output_routine_unsave_replays_aftergroup_before_source_resumes() {
    // TeX82 §§1026/282: closing output_group runs unsave, which backs each
    // insert_token into input before main control reads the following source.
    for (aftergroup, expected) in [("\\aftergroup\\aftermark", 1), ("", 0)] {
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            let mut control = MainControl::tex82_initex(stores);
            register_source(
                &mut control,
                format!(
                    "\\count0=0\\def\\aftermark{{\\global\\count0=1 }}\
                 \\output={{\\shipout\\box255 {aftergroup}}}\
                 \\vsize=1pt\\hrule height2pt\\penalty-10000\\end"
                )
                .as_bytes(),
            );

            run_to_end(&mut control, stores);

            assert_eq!(
                stores.count(0).expect("count register"),
                expected,
                "aftergroup={aftergroup:?}"
            );
        });
    }
}
#[test]
fn tracingcommands_does_not_trace_shipout_box_constructor() {
    // TeX82 §§1030/1075/1084: `\shipout` calls `scan_box` inside its already
    // traced main-control case. Its constructor is scanner-owned, while a
    // later standalone constructor returns normally through `reswitch`.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\tracingcommands=1\tracingonline=1\shipout\hbox{}\hbox{}\end",
        );

        run_to_end(&mut control, stores);

        let terminal = terminal_text(stores);
        assert!(terminal.contains("{\\shipout}"), "{terminal:?}");
        assert_eq!(terminal.matches("\\hbox}").count(), 1, "{terminal:?}");
    });
}
#[test]
fn output_routine_box255_error_reports_live_command_context() {
    // TeX82 §§1026/1028 reach §82's error after retiring the output token
    // list, while the command-owned source level beneath it remains live.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        stores.set_interaction_mode(tex_state::InteractionMode::Nonstop);
        let mut control = MainControl::tex82_initex(stores);
        register_source(
        &mut control,
        b"\\maxdeadcycles=2\\output={\\relax}\\topskip=0pt\\setbox0=\\hbox{}\\copy0\\penalty-10000\\end",
    );

        run_to_end(&mut control, stores);

        let output = terminal_text(stores);
        let report = concat!(
            "! Output routine didn't use all of \\box255.\n",
            "<to be read again> \n",
            "                   \\end \n",
        );
        assert_eq!(output.matches(report).count(), 2, "{output:?}");
        assert!(!output.contains("<output>"), "{output:?}");
        let deleted = "The following box has been deleted:\n\\vbox(0.0+0.0)x0.0 []\n\n";
        let log = String::from_utf8_lossy(stores.world().memory_log_output().unwrap_or_default());
        assert_eq!(log.matches(deleted).count(), 2, "{log:?}");
        let terminal =
            String::from_utf8_lossy(stores.world().memory_terminal_output().unwrap_or_default());
        assert!(!terminal.contains("The following box"), "{terminal:?}");
    });
}
#[test]
fn write_prints_a_control_character_equal_to_newlinechar_as_a_physical_newline() {
    // TeX82 §§262 and 1370: `token_show` prints character tokens through
    // `print`, whose stream selector recognizes `newlinechar` before the
    // non-printable-character `^^` rendering used for diagnostic strings.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        crate::test_harness::assign_int_param(
            stores,
            IntParam::NEWLINE_CHAR,
            10,
            tex_state::AssignmentScope::Global,
        )
        .expect("integer parameter assignment");
        let tokens = allocate_tokens(
            stores,
            &[
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
            ],
        );

        assert_eq!(
            admitted!(stores, |context| write_text(&tokens, context)),
            "A\nB\n"
        );
    });
}
#[test]
fn terminal_write_uses_live_line_width_and_breaks_after_message() {
    // TeX82 §§58/62/1370: stream 16 is a temporary print selector. Its text
    // wraps at the process-selected width, and its leading `print_nl("")`
    // closes a preceding newline-less `\message`. This is the e-TRIP
    // `\typeout`/current-if transition in bounded form.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        stores.set_error_context_widths(
            tex_state::print::ErrorContextWidths::default()
                .with_max_print_line(72)
                .expect("e-TRIP line width is valid"),
        );
        let mut control = MainControl::prepared_initex(CommandProfile::ETEX26);
        tex_command::install_tex82_expandable_primitives(stores);
        crate::install_unexpandable_primitives(stores);
        tex_command::install_etex_expandable_primitives(stores);
        crate::install_etex_unexpandable_primitives(stores);
        register_source(
        &mut control,
        br"\nonstopmode
\immediate\write16{Checking \string\showifs, \string\currentiftype, \string\currentiflevel, and \string\currentifbranch:}
\message{current branch OK}
\immediate\write16{current if level: \number\currentiflevel}
\end",
    );

        run_to_end(&mut control, stores);

        let expected = "Checking \\showifs, \\currentiftype, \\currentiflevel, and \\currentifbranch\n:\ncurrent branch OK\ncurrent if level: 0\n";
        let terminal = pending_sink_text(stores, true);
        let log = pending_sink_text(stores, false);
        assert!(terminal.ends_with(expected), "{terminal:?}");
        assert!(log.ends_with(expected), "{log:?}");
    });
}
#[test]
fn tracingstats_frames_consecutive_shipouts_with_live_memory_reports() {
    // TeX82 §638 snapshots allocator use around each page and closes the
    // progress marker before printing its complete report. The diagnostic is
    // per shipout; consecutive pages must not share one marker line.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\tracingstats=2\shipout\hbox{}\shipout\hbox{}\end",
        );

        run_to_end(&mut control, stores);

        let terminal = terminal_text(stores);
        assert!(!terminal.contains("[0] [0]"), "{terminal:?}");
        assert_eq!(terminal.lines().filter(|line| *line == "[0]").count(), 2);
        let reports = terminal
            .lines()
            .filter(|line| line.starts_with("Memory usage before: "))
            .collect::<Vec<_>>();
        assert_eq!(reports.len(), 2, "{terminal:?}");
        for report in reports {
            assert!(report.contains("; after: "), "{report:?}");
            assert!(report.contains("; still untouched: "), "{report:?}");
        }
    });
}
#[test]
fn huge_page_deleted_box_precedes_shipout_close_and_statistics() {
    // TeX82 §§638 and 641: huge-page recovery displays the rejected box
    // inside `ship_out`, before the closing page marker and allocator report.
    // Positive `\tracingoutput` has already displayed the box at §638 and is
    // the negative control: §641 must not display it a second time.
    for (tracing_output, expected_deleted_boxes) in [(0, 1), (1, 0)] {
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            let mut control = MainControl::tex82_initex(stores);
            register_source(
                &mut control,
                format!(
                    "\\tracingstats=2\\tracingoutput={tracing_output}\\voffset=1sp\
                     \\shipout\\vbox to 16383.99998pt{{}}\\end"
                )
                .as_bytes(),
            );

            run_to_end(&mut control, stores);

            let log = format!(
                "{}{}",
                String::from_utf8_lossy(stores.world().memory_log_output().unwrap_or_default()),
                pending_sink_text(stores, false)
            );
            let terminal = format!(
                "{}{}",
                String::from_utf8_lossy(
                    stores.world().memory_terminal_output().unwrap_or_default()
                ),
                pending_sink_text(stores, true)
            );
            assert_eq!(
                log.matches("The following box has been deleted:").count(),
                expected_deleted_boxes,
                "{log}"
            );
            assert!(
                !terminal.contains("The following box has been deleted:"),
                "{terminal}"
            );
            if tracing_output == 0 {
                let deleted = log
                    .find("The following box has been deleted:")
                    .expect("untraced huge page displays the rejected box");
                let marker_close = log[deleted..]
                    .find("\n]\n")
                    .map(|offset| deleted + offset)
                    .expect("page marker closes after the deleted-box display");
                let statistics = log[marker_close..]
                    .find("Memory usage before:")
                    .map(|offset| marker_close + offset)
                    .expect("allocator report follows the page marker");
                assert!(deleted < marker_close && marker_close < statistics, "{log}");
            }
        });
    }
}
#[test]
fn base_whatsits_preserve_scan_timing_normalization_and_payload_ownership() {
    // TeX82 §§1349--1361: write text remains unexpanded, ordinary special
    // text expands immediately, and normalized closeout fallback slots do not
    // pretend to own a numbered output file.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
        &mut control,
        br"\def\payload{early}
           \setbox0=\hbox{\openout0=owned\write-1{\payload}\closeout16\special{\payload}\setlanguage7}
           \def\payload{late}\end",
    );

        run_to_end(&mut control, stores);

        let nodes = box_child_nodes(stores, 0);
        let [
            Node::Whatsit(tex_state::node::Whatsit::OpenOut { slot, path }),
            Node::Whatsit(tex_state::node::Whatsit::DeferredWrite { sink, tokens }),
            Node::Whatsit(tex_state::node::Whatsit::CloseOut { slot: close_slot }),
            Node::Whatsit(tex_state::node::Whatsit::Special { class, payload }),
            Node::Whatsit(tex_state::node::Whatsit::Language { language, .. }),
        ] = nodes.as_slice()
        else {
            panic!("base whatsits retain their construction order: {nodes:?}");
        };
        assert_eq!(slot.raw(), 0);
        assert_eq!(path, "owned");
        assert_eq!(*sink, PrintSink::Log);
        let payload_symbol = stores
            .intern("payload")
            .expect("payload remains defined")
            .symbol();
        assert_eq!(
            admitted!(stores, |context| context
                .node_token_words(*tokens)
                .expect("live deferred write")
                .to_vec()),
            [tex_state::token::TokenWord::pack(Token::Cs(payload_symbol))]
        );
        assert_eq!(*close_slot, None);
        assert_eq!(class, "dvi");
        assert_eq!(payload, b"early");
        assert_eq!(*language, 7);
    });
}
#[test]
fn deferred_write_expands_at_shipout_once() {
    // TeX82 §§1362--1374: hlist traversal reaches the retained write once and
    // `write_out` expands its text only when the enclosing box is shipped.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\def\payload{early}
           \setbox0=\hbox{\write16{\payload}\special{fixed}}
           \def\payload{late}\shipout\box0\end",
        );

        run_to_end(&mut control, stores);

        let pages = control.take_prepared_dvi_pages();
        let [page] = pages.as_slice() else {
            panic!("exactly one page ships: {pages:?}");
        };
        let committed_write = page
            .committed_effects
            .iter()
            .filter_map(|effect| match effect {
                tex_state::EffectRecord::StreamWrite { sink, text }
                    if *sink == PrintSink::TerminalAndLog =>
                {
                    Some(text.as_str())
                }
                _ => None,
            })
            .collect::<String>();
        assert_eq!(committed_write, "\nlate\n");
        let terminal = terminal_text(stores);
        assert_eq!(terminal.matches("late").count(), 1, "{terminal:?}");
        assert!(!terminal.contains("early"), "{terminal:?}");
    });
}
#[test]
fn deferred_write_retains_unfinished_condition_for_final_cleanup() {
    // TeX82 §1370 expands a deferred write on the live conditional stack;
    // §1335 consequently reports an unfinished conditional from that write
    // before an older outer condition. Attempt-local write tokens are scratch,
    // but the condition frames are committed command semantics.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            b"\\ifcase0\n\\shipout\\hbox{\\write16{\\iftrue x}}\n\\end",
        );

        run_to_end(&mut control, stores);

        let output = terminal_text(stores);
        let write_condition = output
            .find("(\\end occurred when \\iftrue on line 2 was incomplete)")
            .expect("the deferred-write condition remains live");
        let outer_condition = output
            .find("(\\end occurred when \\ifcase on line 1 was incomplete)")
            .expect("the pre-existing outer condition remains live");
        assert!(write_condition < outer_condition, "{output}");
    });
}
#[test]
fn batch_deferred_write_traces_materialize_inside_the_shipout_marker() {
    // TeX82 §§245, 638, and 1370: batch-mode diagnostics select the log
    // alone, but they still execute on the live `write_out` call stack. The
    // aggregate shipout transaction must therefore commit the trace between
    // its opening and closing markers rather than leave it for job-final
    // detached publication.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\batchmode\tracingcommands=2
               \shipout\hbox{\write16{\romannumeral0\relax}}\end",
        );

        run_to_end(&mut control, stores);

        let terminal =
            String::from_utf8_lossy(stores.world().memory_terminal_output().unwrap_or_default());
        let log = String::from_utf8_lossy(stores.world().memory_log_output().unwrap_or_default());
        let marker_open = log.find('[').expect("shipout marker opens");
        let trace = log
            .find("{no mode: \\romannumeral}")
            .expect("deferred expansion trace is materialized");
        let marker_close = log[trace..]
            .find(']')
            .map(|offset| trace + offset)
            .expect("shipout marker closes after the trace");
        assert!(marker_open < trace && trace < marker_close, "{log}");
        assert!(!terminal.contains("romannumeral"), "{terminal}");
        assert!(
            !pending_sink_text(stores, false).contains("romannumeral"),
            "the committed trace must not survive as a pending suffix"
        );
    });
}
#[test]
fn batch_page_builder_diagnostics_materialize_before_the_shipout_marker() {
    // TeX82 §§367, 1006, and 638: the command trace and its page-cost
    // trace are complete before `fire_up` reaches `ship_out`. Batch mode
    // changes only their sink, not their position in that ordered log stream.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\batchmode\tracingcommands=2\tracingpages=1
               \topskip=0pt\vsize=100pt\hrule height2pt\penalty-10000\end",
        );

        run_to_end(&mut control, stores);

        let terminal =
            String::from_utf8_lossy(stores.world().memory_terminal_output().unwrap_or_default());
        let log = String::from_utf8_lossy(stores.world().memory_log_output().unwrap_or_default());
        let command_trace = log
            .find("{\\penalty}")
            .unwrap_or_else(|| panic!("penalty command trace is materialized: {log}"));
        let page_trace = log[command_trace..]
            .find("% t=")
            .map(|offset| command_trace + offset)
            .expect("page cost follows the command trace");
        let marker = log[page_trace..]
            .find('[')
            .map(|offset| page_trace + offset)
            .expect("shipout marker follows page diagnostics");
        assert!(command_trace < page_trace && page_trace < marker, "{log}");
        assert!(!terminal.contains("penalty"), "{terminal}");
    });
}
#[test]
fn batch_page_builder_diagnostics_precede_the_output_loop_error() {
    // TeX82 §§1006, 1012, and 1024: `build_page` completes the forced
    // break's tracing-pages report before `fire_up` diagnoses the exhausted
    // dead-cycle allowance. Batch mode changes only the report's sink. A
    // successful output routine is the negative control: it retains the same
    // traced forced break without reaching the synchronous error boundary.
    for (output, expects_loop) in [("\\relax", true), ("\\shipout\\box255", false)] {
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            let mut control = MainControl::tex82_initex(stores);
            register_source(
                &mut control,
                format!(
                    "\\batchmode\\tracingpages=1\\maxdeadcycles=1\\output={{{output}}}\\topskip=0pt\\vsize=1pt\\hrule height2pt\\penalty-10000\\end"
                )
                .as_bytes(),
            );

            run_to_end(&mut control, stores);

            let log =
                String::from_utf8_lossy(stores.world().memory_log_output().unwrap_or_default());
            let page_trace = log.rfind("% t=").expect("forced page-break trace");
            let output_loop = log.find("! Output loop---");
            assert_eq!(output_loop.is_some(), expects_loop, "{log}");
            if let Some(output_loop) = output_loop {
                assert!(page_trace < output_loop, "{log}");
            }
        });
    }
}
#[test]
fn immediate_openout_applies_one_print_nl_after_an_open_log_line() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = pdftex_initex(stores);
        register_source(
            &mut control,
            br"\message{prefix}\immediate\openout0=zero\immediate\closeout0\end",
        );

        run_to_end(&mut control, stores);

        let log = pending_sink_text(stores, false);
        assert!(
            log.contains("prefix\n\\openout0 = `zero.tex'.\n\n"),
            "{log:?}"
        );
        assert!(!log.contains("prefix\n\n\\openout0"), "{log:?}");
    });
}
#[test]
fn post_apply_facts_preserve_character_font_and_page_output_decisions() {
    // TeX82 §§552/1030/1034/1036: the post-apply handoff distinguishes a
    // present character from nullfont's empty range, preserves an interrupted
    // fetch's existing parking, and carries §1012's selected break without
    // keeping the admitted command facade alive.
    const CMR10: &[u8] = include_bytes!("../../../../tex-fonts/tests/fixtures/cm/cmr10.tfm");
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let font = admitted!(stores, |context| {
            let loaded = tex_fonts::TfmFont::parse(CMR10)
                .expect("cmr10 parses")
                .into_loaded_font("cmr10", "cmr10.tfm", tex_fonts::font_content_hash(CMR10));
            context.intern_font(loaded)
        });
        let mut context = stores.command_context().expect("post-apply test admission");
        context
            .assign_current_font(font, tex_state::AssignmentScope::Global)
            .expect("test font assignment");
        context.record_best_page_break(0, Scaled::from_raw(0), 0);
        context.record_page_fire_up(0);

        let present = PostApplyFacts::capture(
            MainControlParking {
                character: Some('A'),
                resumes_interrupted_fetch: false,
            },
            Mode::Horizontal,
            &context,
        );
        assert_eq!(present.main_loop_active, Some(true));
        assert!(present.page_output.fire_up.is_some());
        assert!(!present.page_output.resume_after_output);

        context
            .assign_current_font(
                tex_state::font::NULL_FONT,
                tex_state::AssignmentScope::Global,
            )
            .expect("nullfont assignment");
        let missing = MainControlParking {
            character: Some('A'),
            resumes_interrupted_fetch: false,
        }
        .post_apply(Mode::Horizontal, &context);
        assert_eq!(missing, Some(false));
        let preserved = MainControlParking {
            character: None,
            resumes_interrupted_fetch: true,
        }
        .post_apply(Mode::Vertical, &context);
        assert_eq!(preserved, None);
    });
}
#[test]
fn outer_vertical_kern_joins_contributions_without_running_page_builder() {
    // TeX82 §§1057 and 1061: `append_kern` tail-appends in every mode but,
    // unlike `append_penalty`, does not call `build_page`. Canonical outer
    // vertical material lives in the page contribution queue rather than the
    // otherwise-empty root mode list.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        tex_command::install_tex82_expandable_primitives(stores);
        crate::install_unexpandable_primitives(stores);
        let mut control = MainControl::prepared_initex(CommandProfile::TEX82);
        register_source(&mut control, br"\kern-50pt");

        run_to_end(&mut control, stores);

        assert!(mode_vec(&control, stores).is_empty());
        assert!(matches!(
            admitted!(stores, |context| context.page_contributions().to_vec()).as_slice(),
            [Node::Kern { amount, kind: KernKind::Explicit }]
                if amount.raw() == -3_276_800
        ));
        assert_eq!(
            admitted!(stores, |context| context
                .page_dimension(PageDimension::Total)),
            Scaled::from_raw(0)
        );
    });
}
#[test]
fn etex_marks_scans_extended_classes_and_expanded_text_in_every_mode() {
    // e-TeX 2.6 `etex.ch` [26.424]: `make_mark` scans an extended register
    // number before TeX82 §1101's expanded mark text and appends the node in
    // every mode. Invalid selectors recover to class zero before the text.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        tex_command::install_tex82_expandable_primitives(stores);
        tex_command::install_etex_expandable_primitives(stores);
        crate::install_unexpandable_primitives(stores);
        crate::install_etex_unexpandable_primitives(stores);
        let mut control = MainControl::prepared_initex(CommandProfile::ETEX26);
        register_source(
            &mut control,
            br"\def\payload{expanded}
          \marks32767{\payload}
          {\global\marks-1{recovered}}
          \hbox{\marks7{horizontal}}
          \vbox{\marks8{vertical}}
          $\marks9{math}1$",
        );

        run_to_end(&mut control, stores);

        let nodes = admitted!(stores, |context| context
            .current_page_nodes()
            .cloned()
            .chain(context.page_contributions().iter().cloned())
            .collect::<Vec<_>>());
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
                    admitted!(stores, |context| context
                        .node_token_words(*tokens)
                        .expect("live numbered mark")
                        .to_vec())
                    .iter()
                    .filter_map(|token| match token.token() {
                        Some(Token::Char { ch, .. }) => Some(ch),
                        Some(Token::Cs(_) | Token::Param(_) | Token::Frozen(_)) | None => None,
                    })
                    .collect::<String>(),
                ),
                _ => None,
            })
            .expect("class 32767 mark");
        assert_eq!(expanded, "expanded");
        assert!(terminal_text(stores).contains("Bad register code"));
        assert!(terminal_text(stores).contains("You can't use a prefix with"));
        assert!(!terminal_text(stores).contains("Unimplemented primitive"));
    });
}
#[test]
fn end_job_transition_census_covers_output_and_residual_paths() {
    // TeX82 §§1054/994--1026: every expanded stop after the initial
    // ejection must follow a completed page-builder/output transition. The
    // page builder resumes default output itself; End delivery is never its
    // scheduler.
    for (name, source, expected_stops, expected_pages) in [
        ("default-output", br"\hrule\end".as_slice(), 5, 1),
        ("terminal-kern", br"\kern1pt\end".as_slice(), 3, 1),
        (
            "explicit-output",
            br"\output={\shipout\box255}\hrule\end".as_slice(),
            5,
            1,
        ),
        (
            "dead-cycle",
            br"\maxdeadcycles=1\output={}\hrule\end".as_slice(),
            5,
            1,
        ),
        (
            "split-insertion",
            br"\vsize=10pt\count0=1000\dimen0=5pt\skip0=0pt\insert0{\hrule height20pt}\hrule height20pt\end"
                .as_slice(),
            6,
            2,
        ),
    ] {
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            let mut control = MainControl::tex82_initex(stores);
            register_source(&mut control, source);
            crate::page_builder::reset_page_context_render_measurement();
            let mut observations = ObservationRecorder::default();
            let mut terminal = None;
            for step in 1..=128 {
                let result = control
                    .advance_with_observer(stores, &mut observations)
                    .unwrap_or_else(|error| panic!("{name} step {step}: {error}"));
                if matches!(result, StepResult::Progress(MainControlStep::End | MainControlStep::EndOfInput)) {
                    terminal = Some((step, result));
                    break;
                }
            }
            let stop_positions = observations
                .0
                .iter()
                .enumerate()
                .filter_map(|(index, observation)| {
                    matches!(
                        observation,
                        CommandObservation::Command(command)
                            if command.boundary == CommandDeliveryBoundary::Expanded
                                && command.command == "stop"
                    )
                    .then_some(index)
                })
                .collect::<Vec<_>>();
            let shipout_positions = observations
                .0
                .iter()
                .enumerate()
                .filter_map(|(index, observation)| {
                    matches!(
                        observation,
                        CommandObservation::Effect(effect)
                            if effect.kind == ObservationEffectKind::Shipout
                    )
                    .then_some(index)
                })
                .collect::<Vec<_>>();
            let termination = observations
                .0
                .iter()
                .position(|observation| {
                    matches!(
                        observation,
                        CommandObservation::Effect(effect)
                            if effect.kind == ObservationEffectKind::Terminate
                    )
                })
                .expect("terminating stop publishes its effect");
            assert!(terminal.is_some(), "{name} did not terminate");
            assert_eq!(stop_positions.len(), expected_stops, "{name}");
            assert_eq!(shipout_positions.len(), expected_pages, "{name}");
            assert_eq!(
                stores.world().committed_artifacts().len(),
                expected_pages,
                "{name}"
            );
            assert_eq!(
                crate::page_builder::page_context_render_measurement(),
                crate::page_builder::PageContextRenderMeasurement::default(),
                "{name}: successful page retries must not render or own context bytes"
            );
            assert!(
                stop_positions.first() < shipout_positions.first(),
                "{name}: ejection stop precedes output"
            );
            assert!(
                shipout_positions.last() < stop_positions.last(),
                "{name}: accepted stop follows completed output"
            );
            assert!(
                stop_positions.last().is_some_and(|last| *last < termination),
                "{name}: termination follows its accepted stop"
            );
        });
    }
}
#[test]
fn openin_closein_replace_stream_state_and_apply_filename_rules() {
    // TeX82 §§1272--1275 close an existing stream before replacement, retain
    // an explicit extension, supply `.tex` only when the extension is empty,
    // and make `\closein` restore the stream's closed/EOF state.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        stores.set_interaction_mode(tex_state::InteractionMode::ErrorStop);
        let mut control = MainControl::tex82_initex(stores);
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
        run_to_end(&mut control, stores);
        assert_eq!(
            macro_semantic_tokens(stores, "first")[0],
            Token::Char {
                ch: 'o',
                cat: Catcode::Letter,
            }
        );
        assert_eq!(
            macro_semantic_tokens(stores, "second")[0],
            Token::Char {
                ch: 't',
                cat: Catcode::Letter,
            }
        );
        assert!(
            stores
                .world()
                .input_stream_eof(tex_state::StreamSlot::new(3))
        );
    });
}
#[test]
fn final_cleanup_reports_nested_condition_kinds_lines_and_order_exactly() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, b"\\iftrue\n\\ifcase0\n\\ifnum1=1\n\\end");

        run_to_end(&mut control, stores);

        assert_eq!(
            terminal_text(stores),
            "(\\end occurred when \\ifnum on line 3 was incomplete)\
\n(\\end occurred when \\ifcase on line 2 was incomplete)\
\n(\\end occurred when \\iftrue on line 1 was incomplete)"
        );
    });
}
#[test]
fn active_output_routine_reads_retained_page_dimensions() {
    // TeX82 §§422/1012: `page_so_far` remains live while the output routine
    // runs even though `fire_up` has emptied the current page list. The
    // ordinary empty-page projection applies only outside that routine.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\vsize=100pt
               \output={\xdef\seen{\the\pageshrink}\shipout\box255}
               \hbox{}\vskip20pt minus 3pt\penalty-10000\end",
        );
        run_to_end(&mut control, stores);

        let rendered = macro_character_text(stores, "seen");
        assert_eq!(rendered, "3.0pt");
    });
}
#[test]
fn immediate_write_prints_expansion_trace_before_expanded_text() {
    // TeX82 §§366/1375: an immediate write expands its token list before
    // the outer selector publishes the resulting text.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            b"\\tracingonline=1\\tracingmacros=1\\def\\a{PAYLOAD}\\immediate\\write16{WRITE:\\a}\\end",
        );
        run_to_end_observed(&mut control, stores, &mut ObservationRecorder::default());

        let output = terminal_text(stores);
        let trace = output
            .find("\\a ->PAYLOAD")
            .unwrap_or_else(|| panic!("missing expansion trace from {output:?}"));
        let write = output
            .find("WRITE:PAYLOAD")
            .unwrap_or_else(|| panic!("missing immediate write from {output:?}"));
        assert!(trace < write, "write overtook expansion trace: {output:?}");
    });
}
#[test]
fn immediate_write_retains_unexpanded_child_spelling_in_its_final_text() {
    // e-TeX §27.465 and TeX82 §1375: the write collector expands normally,
    // but its nested `\unexpanded` child joins the parent result directly.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = pdftex_initex(stores);
        register_source(
            &mut control,
            br"\def\payload{EXPANDED}\immediate\write16{WRITE:\unexpanded{\payload}:END}\end",
        );
        run_to_end_observed(&mut control, stores, &mut ObservationRecorder::default());

        let output = terminal_text(stores);
        assert!(output.contains("WRITE:\\payload :END"), "{output:?}");
        assert!(!output.contains("WRITE:EXPANDED:END"), "{output:?}");
    });
}
