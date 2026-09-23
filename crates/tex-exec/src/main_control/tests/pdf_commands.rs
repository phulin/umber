//! PDF command operands, preflight ordering, typed material, and document state.

use super::*;

#[test]
fn ready_pdf_image_provider_stays_in_one_execution() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = pdftex_initex(stores);
        register_source(&mut control, br"\pdfoutput=1 \pdfximage{image.pdf}\end");
        let mut host = ImmediatePdfImageResourceHost { calls: 0 };
        let mut provider = ResourceHostProvider::new(&mut host);
        let mut finished = false;
        for _ in 0..TEST_STEP_LIMIT {
            match control
                .advance_with_resource_provider(stores, &mut provider)
                .expect("image provider route executes")
            {
                StepResult::Progress(MainControlStep::End | MainControlStep::EndOfInput) => {
                    finished = true;
                    break;
                }
                StepResult::Progress(MainControlStep::Continue) => {}
                StepResult::Suspended(need) => {
                    panic!("ready image unexpectedly suspended: {need:?}")
                }
            }
        }
        assert!(
            finished,
            "ready image provider route exceeded the step bound"
        );
        assert_eq!(host.calls, 1);
        assert_eq!(control.advance_telemetry().resource_replayed_dispatches, 0);
    });
}
#[test]
fn math_choice_nested_pdf_image_rejects_direct_retry_after_need() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = pdftex_initex(stores);
        register_cmr10_as(&mut control, stores, "cmr10.tfm");
        crate::test_harness::assign_int_param(
            stores,
            IntParam::PDF_OUTPUT,
            1,
            tex_state::AssignmentScope::Global,
        )
        .expect("PDF output enables image loading");
        register_source(
            &mut control,
            br"\font\body=cmr10 \body $\mathchoice{\global\advance\count0 by1 \pdfximage{image.pdf}A}{B}{C}{D}$\global\count1=23\end",
        );

        let request = {
            let mut request = None;
            for _ in 0..TEST_STEP_LIMIT {
                match control.advance_episode(stores).expect("nested image step") {
                    StepResult::Suspended(ResourceNeed::PdfImage { request: need }) => {
                        request = Some(need);
                        break;
                    }
                    StepResult::Suspended(need) => {
                        panic!("unexpected nested resource: {need:?}")
                    }
                    StepResult::Progress(_) => {}
                }
            }
            request.expect("nested image resource need must be reached in bounds")
        };
        assert_eq!(stores.count(0).expect("count register"), 0);
        control.capabilities_mut().register_pdf_image(
            request,
            PdfImageResource::Available(test_pdf_image_source()),
        );
        assert!(matches!(
            control.advance_episode(stores),
            Err(ExecError::ResourceReplayRequired)
        ));
        assert_eq!(stores.count(0).expect("count register"), 0);
        assert_eq!(stores.count(1).expect("count register"), 0);
        assert_eq!(
            admitted!(stores, |context| context
                .internal_integer(tex_state::meaning::InternalInteger::PdfLastXImage)
                .expect("last image integer")),
            0
        );
    });
}
#[test]
fn pdftex_engine_announces_deferred_openout_inside_shipout_for_tex82_profile() {
    // Web2C's `[53.1374]` change announces the successful open immediately
    // after tex.web §1374 sets `write_open[j]`. This is compiled pdfTeX
    // behavior even when the loaded format selects the TeX82 command family.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        control.set_engine_binary(crate::EngineBinaryIdentity::Pdftex14029);
        control.begin_job(stores, "openout.tex");
        register_source(
            &mut control,
            br"\shipout\hbox{\openout3=deferred\closeout3}\end",
        );

        run_to_end(&mut control, stores);
        let pages = control.take_prepared_dvi_pages();
        assert_eq!(pages.len(), 1);
        control.finish_job(
            stores,
            Some(crate::DviJobOutput {
                file_name: "openout.dvi".into(),
                byte_len: 0,
            }),
            None,
        );

        let terminal = format!(
            "{}{}",
            String::from_utf8_lossy(stores.world().memory_terminal_output().unwrap_or_default()),
            pending_sink_text(stores, true)
        );
        let log = format!(
            "{}{}",
            String::from_utf8_lossy(stores.world().memory_log_output().unwrap_or_default()),
            pending_sink_text(stores, false)
        );
        let notice = "\\openout3 = `deferred.tex'.";
        assert!(!terminal.contains(notice), "{terminal:?}");
        assert_eq!(log.matches(notice).count(), 1, "{log:?}");
        let marker_open = log.find("[0").expect("shipout marker opens");
        let announcement = log.find(notice).expect("openout is announced");
        let marker_close = log[announcement..]
            .find(']')
            .map(|offset| announcement + offset)
            .expect("shipout marker closes");
        assert!(
            marker_open < announcement && announcement < marker_close,
            "{log:?}"
        );
    });
}
#[test]
fn pdftex_partokencontext_replays_par_at_numbered_boundaries() {
    // Web2C/pdfTeX partoken.ch replaces TeX82 §§1085/1096's direct end_graf
    // at vbox/vtop boundaries for context 1. Context 2 additionally covers
    // §§1100/1130/1133's insertion, valign-item, and no-align boundaries.
    // Redefining \par distinguishes a real inserted-token replay from merely
    // calling the paragraph-ending implementation directly.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = pdftex_initex(stores);
        register_source(
            &mut control,
            br"\let\endgraf=\par
               \def\par{\global\advance\count0 by1 \endgraf}
               \partokencontext=0 \setbox0=\vbox{\hskip1pt}\count1=\count0
               \partokencontext=1 \setbox0=\vbox{\hskip1pt}\count2=\count0
               \setbox0=\vbox{\insert0{\hskip1pt}}\count3=\count0
               \setbox0=\vbox{\halign{#\cr\noalign{\hskip1pt}}}\count4=\count0
               \partokencontext=2 \setbox0=\vbox{\insert0{\hskip1pt}}\count5=\count0
               \setbox0=\vbox{\halign{#\cr\noalign{\hskip1pt}}}\count6=\count0
               \partokencontext=1 {\partokencontext=2}\count7=\partokencontext
               \end",
        );

        run_to_end(&mut control, stores);

        assert_eq!(
            stores.count(1).expect("count register"),
            0,
            "context zero calls end_graf directly"
        );
        assert_eq!(
            stores.count(2).expect("count register"),
            1,
            "context one replays par at vbox end"
        );
        assert_eq!(
            stores.count(3).expect("count register"),
            1,
            "context one excludes insert end"
        );
        assert_eq!(
            stores.count(4).expect("count register"),
            1,
            "context one excludes noalign end"
        );
        assert_eq!(
            stores.count(5).expect("count register"),
            2,
            "context two includes insert end"
        );
        assert_eq!(
            stores.count(6).expect("count register"),
            3,
            "context two includes noalign end"
        );
        assert_eq!(
            stores.count(7).expect("count register"),
            1,
            "the integer parameter is grouped"
        );
        assert_eq!(stores.int_param(IntParam::PAR_TOKEN_CONTEXT), 1);
    });
}
#[test]
fn pdf_glyph_to_unicode_operands_scan_their_exact_destinations() {
    let source = br"\pdfglyphtounicode{\pdffiledump length 2{second}}{\pdffiledump length 2{third}}\message{[done]}\end";

    let preloaded_terminal = run_pdftex_file_probe_job(source, &["second", "third"]);
    assert!(
        preloaded_terminal.contains("[done]"),
        "{preloaded_terminal:?}"
    );
}
#[test]
fn pdf_start_link_action_scans_its_exact_destination() {
    let source =
        br"\pdfoutput=1 A\pdfstartlink goto name{\pdffiledump length 2{second}}B\pdfendlink\end";

    let _preloaded_terminal = run_pdftex_file_probe_job(source, &["second"]);
}
#[test]
fn pdf_xform_optional_texts_scan_their_exact_destinations() {
    let source = br"\pdfoutput=1 \setbox0=\hbox{A}\pdfxform attr{\pdffiledump length 2{second}} resources{\pdffiledump length 2{third}}0\message{[done]}\end";

    let _preloaded_terminal = run_pdftex_file_probe_job(source, &["second", "third"]);
}
#[test]
fn pdf_match_operands_scan_their_exact_destinations() {
    let source = br"\edef\result{\pdfmatch{\pdffiledump length 2{second}}{\pdffiledump length 2{third}}}\message{[\result]}\end";

    let _preloaded_terminal = run_pdftex_file_probe_job(source, &["second", "third"]);
}
#[test]
fn pdf_object_optional_texts_scan_their_exact_destinations() {
    let source = br"\pdfoutput=1 \pdfobj stream attr{\pdffiledump length 2{second}}{\pdffiledump length 2{third}}\message{[done]}\end";

    let _preloaded_terminal = run_pdftex_file_probe_job(source, &["second", "third"]);
}
#[test]
fn pdf_outline_texts_scan_their_exact_destinations() {
    let source = br"\pdfoutput=1 \pdfoutline attr{\pdffiledump length 2{first}} goto name{\pdffiledump length 2{second}} count 1 {\pdffiledump length 2{third}}\message{[done]}\end";

    let _preloaded_terminal = run_pdftex_file_probe_job(source, &["first", "second", "third"]);
}
#[test]
fn pdf_catalog_text_and_action_scan_their_exact_destinations() {
    let source = br"\pdfoutput=1 \pdfcatalog{\pdffiledump length 2{second}} openaction goto name{\pdffiledump length 2{third}}\message{[done]}\end";

    let _preloaded_terminal = run_pdftex_file_probe_job(source, &["second", "third"]);
}
#[test]
fn pdf_graphics_payloads_scan_their_exact_destinations() {
    for source in [
        br"\pdfoutput=1 \pdfliteral direct{\pdffiledump length 2{second}}\message{[done]}\end"
            .as_slice(),
        br"\pdfoutput=1 \edef\stack{\pdfcolorstackinit page direct{\pdffiledump length 2{second}}}\pdfcolorstack\stack push{\pdffiledump length 2{third}}\message{[done]}\end"
            .as_slice(),
        br"\special{\pdffiledump length 2{second}}\message{[done]}\end".as_slice(),
    ] {
        let resources = if source.windows(5).any(|window| window == b"third") {
            &["second", "third"][..]
        } else {
            &["second"][..]
        };
        let preloaded_terminal = run_pdftex_file_probe_job(source, resources);
        assert!(preloaded_terminal.contains("[done]"), "{preloaded_terminal:?}");
    }
}
#[test]
fn pdfstrcmp_operands_scan_their_exact_child_scanners() {
    for source in [
        br"\edef\result{\pdfstrcmp{\pdffiledump length 2{second}}{right}}\message{[\result]}\end"
            .as_slice(),
        br"\edef\result{\pdfstrcmp{left}{\pdffiledump length 2{second}}}\message{[\result]}\end"
            .as_slice(),
    ] {
        let preloaded_terminal = run_pdftex_file_probe_job(source, &["second"]);
        assert!(preloaded_terminal.contains("["), "{preloaded_terminal:?}");
    }
}
#[test]
fn deferred_effect_and_ordinary_pdf_commands_open_no_aggregate_savepoint() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\openout0=deferred");

        assert_eq!(
            control.advance(stores).expect("deferred open commits"),
            StepResult::Progress(ReplayStep::Continue)
        );
        assert_eq!(control.advance_telemetry().maximum_live_savepoints, 0);

        crate::test_harness::with_nonstop_plain_universe(|pdf_stores| {
            let mut pdf_control = pdftex_graphics_control(pdf_stores);
            crate::test_harness::assign_int_param(
                pdf_stores,
                IntParam::PDF_OUTPUT,
                1,
                tex_state::AssignmentScope::Global,
            )
            .expect("integer parameter assignment");
            register_source(&mut pdf_control, br"\pdfliteral direct{q Q}");

            assert_eq!(
                pdf_control
                    .advance(pdf_stores)
                    .expect("ordinary PDF node commits"),
                StepResult::Progress(ReplayStep::Continue)
            );
            assert_eq!(pdf_control.advance_telemetry().maximum_live_savepoints, 0);
        });
    });
}
#[test]
fn pdftex_font_actions_route_through_command_expansion_and_font_state() {
    // pdftex.web §§1601--1607, 1680--1682: general text is expanded before
    // the action mutates the selected font or the global map/ToUnicode state.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        crate::install_unexpandable_primitives(stores);
        tex_command::install_tex82_expandable_primitives(stores);
        crate::test_harness::assign_int_param(
            stores,
            IntParam::PDF_OUTPUT,
            1,
            tex_state::AssignmentScope::Global,
        )
        .expect("integer parameter assignment");
        let mut control = pdftex_font_action_control(stores);
        register_cmr10_as(&mut control, stores, "cmr10.tfm");
        register_source(
            &mut control,
            concat!(
                "\\font\\base=cmr10 ",
                "\\def\\attr{/StemV 70}\\def\\chars{CABA}\\def\\uni{0041}",
                "\\pdffontexpand\\base 100 50 10 autoexpand ",
                "\\pdffontattr\\base{\\attr}\\pdfincludechars\\base{\\chars}",
                "\\pdfmapline{+cmr10 CMR10 <cmr10.pfb}",
                "\\pdfglyphtounicode{A}{\\uni}\\pdfnobuiltintounicode\\base\\end",
            )
            .as_bytes(),
        );

        run_to_end(&mut control, stores);
        let base = admitted!(stores, |context| {
            let symbol = context.intern_control_sequence("base");
            match context.meaning(symbol) {
                ResolvedMeaning::Static(Meaning::Font(font)) => font,
                meaning => panic!("base is a font, got {meaning:?}"),
            }
        });

        assert_eq!(
            admitted!(stores, |context| context.font_expansion(base)),
            Some(tex_state::font::FontExpansion {
                stretch: 100,
                shrink: 50,
                step: 10,
                auto_expand: true,
            })
        );
    });
}
#[test]
fn pdftex_font_actions_preserve_exact_dvi_mode_gate_and_tounicode_exceptions() {
    // pdftex.web §§1601--1607: these four extension codes require PDF mode;
    // glyph and built-in ToUnicode definitions are deliberately exempt.
    // §§1680--1682's font expansion configuration is likewise output-mode
    // independent because it configures generated font metrics.
    for (name, source) in [
        ("pdffontattr", b"\\pdffontattr\\nullfont{}".as_slice()),
        (
            "pdfincludechars",
            b"\\pdfincludechars\\nullfont{}".as_slice(),
        ),
        ("pdfmapfile", b"\\pdfmapfile{}".as_slice()),
        ("pdfmapline", b"\\pdfmapline{}".as_slice()),
    ] {
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            let mut control = pdftex_font_action_control(stores);
            register_source(&mut control, source);
            assert!(matches!(
                control.step(stores),
                Err(ExecError::PdfExtensionInDviMode(actual)) if actual == name
            ));
        });
    }

    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = pdftex_font_action_control(stores);
        register_source(
            &mut control,
            b"\\pdffontexpand\\nullfont 10 5 1 autoexpand\\end",
        );
        run_to_end(&mut control, stores);
        assert_eq!(
            admitted!(stores, |context| context
                .font_expansion(tex_state::font::NULL_FONT)),
            Some(tex_state::font::FontExpansion {
                stretch: 10,
                shrink: 5,
                step: 1,
                auto_expand: true,
            })
        );

        crate::test_harness::with_nonstop_plain_universe(|stores| {
            let mut control = pdftex_font_action_control(stores);
            register_source(
                &mut control,
                b"\\pdfglyphtounicode{A}{0041}\\pdfnobuiltintounicode\\nullfont\\end",
            );
            run_to_end(&mut control, stores);
            assert!(control.fatal_error().is_none());
        });
    });
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
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            let mut control = pdftex_graphics_control(stores);
            register_source(&mut control, source);
            let state_before = stores.journal_cursor().expect("state cursor");

            let error = control
                .step(stores)
                .expect_err("DVI preflight rejects the extension");
            assert!(matches!(
                &error,
                ExecError::PdfExtensionInDviMode(name) if *name == primitive
            ));
            assert!(!error.requires_terminal_settlement());
            assert_eq!(stores.journal_cursor().expect("state cursor"), state_before);
            assert!(current_list_owner_vec(&control, stores).is_empty());

            crate::test_harness::assign_int_param(
                stores,
                IntParam::PDF_OUTPUT,
                1,
                tex_state::AssignmentScope::Global,
            )
            .expect("integer parameter assignment");
            assert_eq!(
                control.step(stores).expect("graphics command retries"),
                MainControlStep::Continue
            );
            let current_nodes = current_list_owner_vec(&control, stores);
            let [node] = current_nodes.as_slice() else {
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
                control.step(stores).expect("following command remains"),
                MainControlStep::Continue
            );
            assert!(matches!(
                current_list_owner_vec(&control, stores).last(),
                Some(Node::Whatsit(Whatsit::PdfSave))
            ));
        });
    }
}
#[test]
fn pdfsavepos_remains_available_in_dvi_mode() {
    // pdftex.web §1563 deliberately excludes `\pdfsavepos` from the PDF
    // output preflight used by the neighboring graphics extensions.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = pdftex_graphics_control(stores);
        register_source(&mut control, br"\pdfsavepos");
        assert_eq!(
            control.step(stores).expect("DVI save position"),
            MainControlStep::Continue
        );
        assert!(matches!(
            current_list_owner_vec(&control, stores).as_slice(),
            [Node::Whatsit(Whatsit::PdfSavePos)]
        ));
    });
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
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            stores.set_interaction_mode(tex_state::InteractionMode::Scroll);
            crate::test_harness::assign_int_param(
                stores,
                IntParam::PDF_OUTPUT,
                1,
                tex_state::AssignmentScope::Global,
            )
            .expect("integer parameter assignment");
            let mut control = pdftex_graphics_control(stores);
            register_source(&mut control, source);
            let _ = control.step(stores).expect("recoverable bad stack id");
            assert!(matches!(
                current_list_owner_vec(&control, stores).as_slice(),
                [Node::Whatsit(Whatsit::PdfColorStack { id: 0, .. })]
            ));
            let terminal = terminal_text(stores);
            assert!(terminal.contains(diagnostic));
            assert!(terminal.contains(help));
            assert!(terminal.contains("Proceed, with fingers crossed."));
        });
    }

    crate::test_harness::with_nonstop_plain_universe(|stores| {
        stores.set_interaction_mode(tex_state::InteractionMode::Scroll);
        crate::test_harness::assign_int_param(
            stores,
            IntParam::PDF_OUTPUT,
            1,
            tex_state::AssignmentScope::Global,
        )
        .expect("integer parameter assignment");
        let mut control = pdftex_graphics_control(stores);
        register_source(&mut control, br"\pdfcolorstack0\pdfsave");
        let _ = control.step(stores).expect("missing action is recoverable");
        assert!(current_list_owner_vec(&control, stores).is_empty());
        let _ = control
            .step(stores)
            .expect("following command remains available");
        assert!(matches!(
            current_list_owner_vec(&control, stores).as_slice(),
            [Node::Whatsit(Whatsit::PdfSave)]
        ));
        let terminal = terminal_text(stores);
        assert!(terminal.contains("Color stack action is missing"));
        assert!(terminal.contains("set, push, pop, current"));
        assert!(terminal.contains("I'll ignore the color stack command."));
        assert!(terminal.contains("Proceed, with fingers crossed."));
    });
}
#[test]
fn pdf_object_rejects_dvi_before_every_option_operand_and_allocation() {
    // pdftex.web §§1535 and 1542 call `check_pdfoutput` before the complete
    // `reserveobjnum`/`useobjnum`, integer, stream/attr/file, body, and
    // allocation paths. Aggregate retry must therefore see the whole command.
    crate::test_harness::with_nonstop_plain_universe(|reserve_stores| {
        let mut reserve_control = pdftex_object_control(reserve_stores);
        register_source(&mut reserve_control, br"\pdfobj reserveobjnum");
        assert!(matches!(
            reserve_control.step(reserve_stores),
            Err(ExecError::PdfExtensionInDviMode("pdfobj"))
        ));
        assert!(admitted!(reserve_stores, |context| context.pdf_raw_object(1)).is_none());
        assert_eq!(
            admitted!(reserve_stores, |context| context
                .internal_integer(tex_state::meaning::InternalInteger::PdfLastObject)
                .expect("PDF integer")),
            0
        );

        crate::test_harness::assign_int_param(
            reserve_stores,
            IntParam::PDF_OUTPUT,
            1,
            tex_state::AssignmentScope::Global,
        )
        .expect("integer parameter assignment");
        assert_eq!(
            reserve_control
                .step(reserve_stores)
                .expect("reserveobjnum retry preserves the complete command"),
            MainControlStep::Continue
        );
        assert_eq!(
            usize::from(admitted!(reserve_stores, |context| context.pdf_raw_object(1)).is_some()),
            1
        );
        assert!(
            admitted!(reserve_stores, |context| context.pdf_raw_object(1))
                .expect("PDF object 1")
                .data()
                .is_none()
        );

        crate::test_harness::with_nonstop_plain_universe(|ordinary_stores| {
            let mut ordinary_control = pdftex_object_control(ordinary_stores);
            register_source(&mut ordinary_control, br"\pdfobj{ordinary}");
            assert!(matches!(
                ordinary_control.step(ordinary_stores),
                Err(ExecError::PdfExtensionInDviMode("pdfobj"))
            ));
            assert!(admitted!(ordinary_stores, |context| context.pdf_raw_object(1)).is_none());

            crate::test_harness::assign_int_param(
                ordinary_stores,
                IntParam::PDF_OUTPUT,
                1,
                tex_state::AssignmentScope::Global,
            )
            .expect("integer parameter assignment");
            assert_eq!(
                ordinary_control
                    .step(ordinary_stores)
                    .expect("ordinary-object retry preserves its body"),
                MainControlStep::Continue
            );
            let ordinary = admitted!(ordinary_stores, |context| context.pdf_raw_object(1))
                .expect("PDF object 1")
                .data()
                .expect("ordinary object is initialized");
            assert!(!ordinary.is_stream());
            assert!(!ordinary.is_file());
            assert_eq!(
                token_character_text(ordinary_stores, ordinary.data()),
                "ordinary"
            );

            crate::test_harness::with_nonstop_plain_universe(|define_stores| {
                let mut define_control = pdftex_object_control(define_stores);
                register_source(
                    &mut define_control,
                    br"\pdfobj useobjnum 37 stream attr{/Subtype /XML} file{payload}",
                );
                assert!(matches!(
                    define_control.step(define_stores),
                    Err(ExecError::PdfExtensionInDviMode("pdfobj"))
                ));
                assert!(admitted!(define_stores, |context| context.pdf_raw_object(1)).is_none());
                assert_eq!(
                    admitted!(define_stores, |context| context
                        .internal_integer(tex_state::meaning::InternalInteger::PdfReturnValue)
                        .expect("PDF integer")),
                    0
                );
                assert!(terminal_text(define_stores).is_empty());

                crate::test_harness::assign_int_param(
                    define_stores,
                    IntParam::PDF_OUTPUT,
                    1,
                    tex_state::AssignmentScope::Global,
                )
                .expect("integer parameter assignment");
                assert_eq!(
                    define_control
                        .step(define_stores)
                        .expect("definition retry preserves every option and operand"),
                    MainControlStep::Continue
                );
                assert_eq!(
                    admitted!(define_stores, |context| context
                        .internal_integer(tex_state::meaning::InternalInteger::PdfReturnValue)
                        .expect("PDF integer")),
                    -1
                );
                assert!(
                    terminal_text(define_stores).contains("invalid object number being ignored")
                );
                let record = &admitted!(define_stores, |context| context.pdf_raw_object(1))
                    .expect("PDF object 1");
                let data = record.data().expect("retried object is initialized");
                assert!(data.is_stream());
                assert!(data.is_file());
                assert_eq!(
                    token_character_text(
                        define_stores,
                        data.stream_attr().expect("stream attribute survives retry")
                    ),
                    "/Subtype /XML"
                );
                assert_eq!(token_character_text(define_stores, data.data()), "payload");
            });
        });
    });
}
#[test]
fn immediate_pdf_object_rejects_dvi_after_lookahead_before_operand_scan() {
    // pdftex.web §1621 expands the command after `\immediate`, then invokes
    // §1542's complete `\pdfobj` case. Its DVI check therefore wins over the
    // immediate-reserved-object error and every operand remains retryable.
    crate::test_harness::with_nonstop_plain_universe(|reserve_stores| {
        let mut reserve_control = pdftex_object_control(reserve_stores);
        register_source(&mut reserve_control, br"\immediate\pdfobj reserveobjnum");
        assert!(matches!(
            reserve_control.step(reserve_stores),
            Err(ExecError::PdfExtensionInDviMode("pdfobj"))
        ));
        assert!(admitted!(reserve_stores, |context| context.pdf_raw_object(1)).is_none());

        crate::test_harness::assign_int_param(
            reserve_stores,
            IntParam::PDF_OUTPUT,
            1,
            tex_state::AssignmentScope::Global,
        )
        .expect("integer parameter assignment");
        assert!(matches!(
            reserve_control.step(reserve_stores),
            Err(ExecError::PdfImmediateReservedObject)
        ));
        assert!(admitted!(reserve_stores, |context| context.pdf_raw_object(1)).is_none());

        crate::test_harness::with_nonstop_plain_universe(|define_stores| {
            let mut define_control = pdftex_object_control(define_stores);
            register_source(
                &mut define_control,
                br"\immediate\pdfobj useobjnum 41 stream attr{/Type /Metadata} file{retry.dat}\pdfobj reserveobjnum",
            );
            assert!(matches!(
                define_control.step(define_stores),
                Err(ExecError::PdfExtensionInDviMode("pdfobj"))
            ));
            assert!(admitted!(define_stores, |context| context.pdf_raw_object(1)).is_none());
            assert_eq!(
                admitted!(define_stores, |context| context
                    .internal_integer(tex_state::meaning::InternalInteger::PdfReturnValue)
                    .expect("PDF integer")),
                0
            );

            crate::test_harness::assign_int_param(
                define_stores,
                IntParam::PDF_OUTPUT,
                1,
                tex_state::AssignmentScope::Global,
            )
            .expect("integer parameter assignment");
            assert_eq!(
                define_control
                    .step(define_stores)
                    .expect("immediate retry preserves every option and operand"),
                MainControlStep::Continue
            );
            assert_eq!(
                admitted!(define_stores, |context| context
                    .internal_integer(tex_state::meaning::InternalInteger::PdfReturnValue)
                    .expect("PDF integer")),
                -1
            );
            let record = &admitted!(define_stores, |context| context.pdf_raw_object(1))
                .expect("PDF object 1");
            assert!(record.is_immediate());
            let data = record.data().expect("immediate object is initialized");
            assert!(data.is_stream());
            assert!(data.is_file());
            assert_eq!(
                token_character_text(
                    define_stores,
                    data.stream_attr().expect("stream attribute survives retry")
                ),
                "/Type /Metadata"
            );
            assert_eq!(
                token_character_text(define_stores, data.data()),
                "retry.dat"
            );
            // The inner PDF command and its outer `\immediate` were restored
            // as two backup levels. The next source command must therefore
            // remain behind the complete retried pair and reserve object 2.
            assert_eq!(
                define_control
                    .step(define_stores)
                    .expect("following command remains after immediate retry"),
                MainControlStep::Continue
            );
            assert!(admitted!(define_stores, |context| context.pdf_raw_object(2)).is_some());
        });
    });
}
#[test]
fn pdf_reference_object_rejects_dvi_before_scan_validation_or_list_mutation() {
    // pdftex.web §1544 orders `check_pdfoutput`, `scan_int`,
    // `pdf_check_obj`, `new_whatsit`, and object-number assignment. A DVI
    // failure must therefore preserve the integer and every aggregate owner
    // for transactional retry under the pdfTeX profile.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let object = admitted!(stores, |context| context.reserve_pdf_raw_object())
            .expect("reserve reference target");
        assert_eq!(object.raw(), 1);
        let mut control = pdftex_object_control(stores);
        register_source(&mut control, br"\pdfrefobj 1");
        let state_before = stores.journal_cursor().expect("state cursor");

        assert!(matches!(
            control.step(stores),
            Err(ExecError::PdfExtensionInDviMode("pdfrefobj"))
        ));
        assert_eq!(stores.journal_cursor().expect("state cursor"), state_before);
        assert_eq!(
            usize::from(admitted!(stores, |context| context.pdf_raw_object(1)).is_some()),
            1
        );
        assert!(mode_vec(&control, stores).is_empty());

        crate::test_harness::assign_int_param(
            stores,
            IntParam::PDF_OUTPUT,
            1,
            tex_state::AssignmentScope::Global,
        )
        .expect("integer parameter assignment");
        assert_eq!(
            control
                .step(stores)
                .expect("PDF retry preserves the integer operand"),
            MainControlStep::Continue
        );
        assert!(mode_vec(&control, stores).is_empty());
        assert!(matches!(
            admitted!(stores, |context| context.page_contributions().to_vec()).as_slice(),
            [Node::Whatsit(Whatsit::PdfReferenceObject { object: 1 })]
        ));
    });
}
#[test]
fn pdf_reference_object_dvi_error_precedes_invalid_object_validation() {
    // pdftex.web §1544 checks DVI mode before scanning or calling
    // `pdf_check_obj`; the missing-object error is reached only on a PDF-mode
    // retry of the same intact operand.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = pdftex_object_control(stores);
        register_source(&mut control, br"\pdfrefobj 99");
        let state_before = stores.journal_cursor().expect("state cursor");

        assert!(matches!(
            control.step(stores),
            Err(ExecError::PdfExtensionInDviMode("pdfrefobj"))
        ));
        assert_eq!(stores.journal_cursor().expect("state cursor"), state_before);
        assert!(admitted!(stores, |context| context.pdf_raw_object(1)).is_none());
        assert!(mode_vec(&control, stores).is_empty());

        crate::test_harness::assign_int_param(
            stores,
            IntParam::PDF_OUTPUT,
            1,
            tex_state::AssignmentScope::Global,
        )
        .expect("integer parameter assignment");
        assert!(matches!(
            control.step(stores),
            Err(ExecError::PdfReferencedObjectNotFound)
        ));
        assert!(admitted!(stores, |context| context.pdf_raw_object(1)).is_none());
        assert!(mode_vec(&control, stores).is_empty());
    });
}
#[test]
fn pdf_form_family_rejects_dvi_before_operands_allocation_and_list_mutation() {
    // pdftex.web §§1548–1549 begin both cases with `check_pdfoutput`.
    // `\pdfxform` therefore preserves attr/resources/the register and its box,
    // while `\pdfrefxform` preserves its integer before lookup and whatsit
    // insertion. The two commands are one PDF-output-preflight family.
    crate::test_harness::with_nonstop_plain_universe(|create_stores| {
        install_test_hbox(create_stores, 7, Scaled::from_raw(17));
        let mut create = pdftex_form_control(create_stores);
        register_source(
            &mut create,
            br"\pdfxform attr{/Subtype /Form} resources{/ProcSet [/PDF]} 7",
        );
        let state_before = create_stores.journal_cursor().expect("state cursor");

        assert!(matches!(
            create.step(create_stores),
            Err(ExecError::PdfExtensionInDviMode("pdfxform"))
        ));
        assert_eq!(
            create_stores.journal_cursor().expect("state cursor"),
            state_before
        );
        assert!(create_stores.copy_box_to_page(7).is_some());
        assert!(admitted!(create_stores, |context| context.pdf_form(1)).is_none());
        assert_eq!(
            admitted!(create_stores, |context| context
                .internal_integer(tex_state::meaning::InternalInteger::PdfLastXForm)
                .expect("PDF integer")),
            0
        );
        assert!(mode_vec(&create, create_stores).is_empty());

        crate::test_harness::assign_int_param(
            create_stores,
            IntParam::PDF_OUTPUT,
            1,
            tex_state::AssignmentScope::Global,
        )
        .expect("integer parameter assignment");
        let before_form = create_stores.page_region_counters();
        assert_eq!(
            create
                .step(create_stores)
                .expect("PDF retry preserves all form options and the register"),
            MainControlStep::Continue
        );
        let after_form = create_stores.page_region_counters();
        assert_eq!(
            after_form.page_to_durable_nodes_copied, before_form.page_to_durable_nodes_copied,
            "a direct PDF form move does not copy page payload"
        );
        assert_eq!(
            after_form.history_preservation_nodes_copied,
            before_form.history_preservation_nodes_copied,
            "the live command operation uses a transfer loan"
        );
        assert!(create_stores.copy_box_to_page(7).is_none());
        let form = admitted!(create_stores, |context| context.pdf_form(1))
            .expect("retried form is allocated");
        assert_eq!(form.width(), Scaled::from_raw(17));
        assert_eq!(
            token_character_text(
                create_stores,
                form.attr().expect("form attribute survives retry")
            ),
            "/Subtype /Form"
        );
        assert_eq!(
            token_character_text(
                create_stores,
                form.resources().expect("form resources survive retry")
            ),
            "/ProcSet [/PDF]"
        );

        crate::test_harness::with_nonstop_plain_universe(|reference_stores| {
            install_test_form(reference_stores);
            let mut reference = pdftex_form_control(reference_stores);
            reference.modes.push(Mode::Math).expect("test mode push");
            register_source(&mut reference, br"\pdfrefxform 1");
            let state_before = reference_stores.journal_cursor().expect("state cursor");

            assert!(matches!(
                reference.step(reference_stores),
                Err(ExecError::PdfExtensionInDviMode("pdfrefxform"))
            ));
            assert_eq!(
                reference_stores.journal_cursor().expect("state cursor"),
                state_before
            );
            assert!(mode_vec(&reference, reference_stores).is_empty());

            crate::test_harness::assign_int_param(
                reference_stores,
                IntParam::PDF_OUTPUT,
                1,
                tex_state::AssignmentScope::Global,
            )
            .expect("integer parameter assignment");
            assert_eq!(
                reference
                    .step(reference_stores)
                    .expect("PDF retry preserves the reference operand in math mode"),
                MainControlStep::Continue
            );
            assert!(matches!(
                mode_vec(&reference, reference_stores).as_slice(),
                [Node::Whatsit(Whatsit::PdfRefXForm { object: 1, .. })]
            ));
        });
    });
}
#[test]
fn immediate_pdf_form_rejects_dvi_before_options_or_allocation() {
    // pdftex.web §§1548 and 1623 perform `\immediate` lookahead, then enter
    // the same `\pdfxform` case whose first operation is `check_pdfoutput`.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        install_test_hbox(stores, 9, Scaled::from_raw(19));
        let mut control = pdftex_form_control(stores);
        register_source(
            &mut control,
            br"\immediate\pdfxform attr{/A 1} resources{/R 2} 9",
        );
        let state_before = stores.journal_cursor().expect("state cursor");

        assert!(matches!(
            control.step(stores),
            Err(ExecError::PdfExtensionInDviMode("pdfxform"))
        ));
        assert_eq!(stores.journal_cursor().expect("state cursor"), state_before);
        assert!(stores.copy_box_to_page(9).is_some());
        assert!(admitted!(stores, |context| context.pdf_form(1)).is_none());

        crate::test_harness::assign_int_param(
            stores,
            IntParam::PDF_OUTPUT,
            1,
            tex_state::AssignmentScope::Global,
        )
        .expect("integer parameter assignment");
        assert_eq!(
            control
                .step(stores)
                .expect("immediate PDF retry preserves every form operand"),
            MainControlStep::Continue
        );
        assert!(stores.copy_box_to_page(9).is_none());
        let form =
            admitted!(stores, |context| context.pdf_form(1)).expect("immediate form is allocated");
        assert!(form.immediate());
        assert_eq!(form.width(), Scaled::from_raw(19));
    });
}
#[test]
fn pdf_form_dvi_error_precedes_invalid_register_void_box_and_missing_object() {
    // §§1548–1549 put DVI rejection before even the scans. On PDF retry,
    // e-TeX's `scan_register_num` recovers an invalid selector to zero before
    // §1548 allocates the form and diagnoses the resulting void box; §1549
    // scans an integer and then diagnoses a missing form object.
    crate::test_harness::with_nonstop_plain_universe(|invalid_register_stores| {
        let mut invalid_register = pdftex_form_control(invalid_register_stores);
        register_source(&mut invalid_register, br"\pdfxform 40000");
        let state_before = invalid_register_stores
            .journal_cursor()
            .expect("state cursor");

        assert!(matches!(
            invalid_register.step(invalid_register_stores),
            Err(ExecError::PdfExtensionInDviMode("pdfxform"))
        ));
        assert_eq!(
            invalid_register_stores
                .journal_cursor()
                .expect("state cursor"),
            state_before
        );
        assert!(terminal_text(invalid_register_stores).is_empty());
        assert!(admitted!(invalid_register_stores, |context| context.pdf_form(1)).is_none());

        crate::test_harness::assign_int_param(
            invalid_register_stores,
            IntParam::PDF_OUTPUT,
            1,
            tex_state::AssignmentScope::Global,
        )
        .expect("integer parameter assignment");
        assert!(matches!(
            invalid_register.step(invalid_register_stores),
            Err(ExecError::PdfXFormVoidBox)
        ));
        assert!(admitted!(invalid_register_stores, |context| context.pdf_form(1)).is_none());

        crate::test_harness::with_nonstop_plain_universe(|void_stores| {
            let mut void = pdftex_form_control(void_stores);
            register_source(&mut void, br"\pdfxform 12");
            assert!(matches!(
                void.step(void_stores),
                Err(ExecError::PdfExtensionInDviMode("pdfxform"))
            ));
            crate::test_harness::assign_int_param(
                void_stores,
                IntParam::PDF_OUTPUT,
                1,
                tex_state::AssignmentScope::Global,
            )
            .expect("integer parameter assignment");
            assert!(matches!(
                void.step(void_stores),
                Err(ExecError::PdfXFormVoidBox)
            ));

            crate::test_harness::with_nonstop_plain_universe(|missing_stores| {
                let mut missing = pdftex_form_control(missing_stores);
                missing
                    .modes
                    .push(Mode::RestrictedHorizontal)
                    .expect("test mode push");
                register_source(&mut missing, br"\pdfrefxform 99");
                assert!(matches!(
                    missing.step(missing_stores),
                    Err(ExecError::PdfExtensionInDviMode("pdfrefxform"))
                ));
                assert!(mode_vec(&missing, missing_stores).is_empty());
                crate::test_harness::assign_int_param(
                    missing_stores,
                    IntParam::PDF_OUTPUT,
                    1,
                    tex_state::AssignmentScope::Global,
                )
                .expect("integer parameter assignment");
                assert!(matches!(
                    missing.step(missing_stores),
                    Err(ExecError::PdfReferencedObjectNotFound)
                ));
                assert!(mode_vec(&missing, missing_stores).is_empty());
            });
        });
    });
}
#[test]
fn pdf_image_create_rejects_dvi_and_direct_retry_before_allocation() {
    // pdftex.web §1551 orders `check_pdfoutput` before `check_pdfversion`,
    // image-object allocation, `scan_image`, and `read_image`. A failed
    // aggregate operation therefore preserves every supported rule, attr,
    // page, page-box, and filename operand for exact resource retry.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = pdftex_image_control(stores);
        register_source(
            &mut control,
            br"\pdfximage width 10pt height 20pt depth 3pt attr{/Interpolate true} page 2 mediabox {image.pdf}",
        );
        let state_before = stores.journal_cursor().expect("state cursor");

        assert!(matches!(
            control.advance(stores),
            Err(ExecError::Captured { error, .. })
                if matches!(*error, ExecError::PdfExtensionInDviMode("pdfximage"))
        ));
        assert_eq!(stores.journal_cursor().expect("state cursor"), state_before);
        assert!(
            admitted!(stores, |context| context
                .internal_integer(tex_state::meaning::InternalInteger::PdfLastXImage)
                .expect("PDF image integer"))
                == 0
        );
        assert!(mode_vec(&control, stores).is_empty());
    });

    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = pdftex_image_control(stores);
        crate::test_harness::assign_int_param(
            stores,
            IntParam::PDF_OUTPUT,
            1,
            tex_state::AssignmentScope::Global,
        )
        .expect("integer parameter assignment");
        register_source(
            &mut control,
            br"\pdfximage width 10pt height 20pt depth 3pt attr{/Interpolate true} page 2 mediabox {image.pdf}",
        );
        let pdf_state_before = stores.journal_cursor().expect("state cursor");
        let request = {
            let mut request = None;
            for _ in 0..TEST_STEP_LIMIT {
                match control.advance(stores).expect("PDF image request suspends") {
                    StepResult::Suspended(ResourceNeed::PdfImage { request: need }) => {
                        request = Some(need);
                        break;
                    }
                    StepResult::Progress(_) => {}
                    other => panic!("expected image suspension, got {other:?}"),
                }
            }
            request.expect("PDF image resource need must be reached in bounds")
        };
        assert_eq!(
            stores.journal_cursor().expect("state cursor"),
            pdf_state_before
        );
        assert!(
            admitted!(stores, |context| context
                .internal_integer(tex_state::meaning::InternalInteger::PdfLastXImage)
                .expect("PDF image integer"))
                == 0
        );
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
        assert!(matches!(
            control.advance(stores),
            Err(ExecError::ResourceReplayRequired)
        ));
        assert_eq!(
            admitted!(stores, |context| context
                .internal_integer(tex_state::meaning::InternalInteger::PdfLastXImage)
                .expect("last image integer")),
            0
        );
        assert!(mode_vec(&control, stores).is_empty());
    });
}
#[test]
fn immediate_pdf_image_rejects_direct_retry_after_resource_need() {
    // pdftex.web §1621 expands the command after `\immediate`, then invokes
    // §1551's complete `\pdfximage` case. Its output check precedes every
    // image operand. A direct MainControl caller must reject the stale retry;
    // full checkpoint replay is owned by the incremental session.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = pdftex_image_control(stores);
        register_source(
            &mut control,
            br"\immediate\pdfximage width 7pt height 8pt depth 2pt attr{/Intent /RelativeColorimetric} page 3 cropbox {immediate.pdf}",
        );
        let state_before = stores.journal_cursor().expect("state cursor");

        assert!(matches!(
            control.advance(stores),
            Err(ExecError::PdfExtensionInDviMode("pdfximage"))
        ));
        assert_eq!(stores.journal_cursor().expect("state cursor"), state_before);
        assert!(
            admitted!(stores, |context| context
                .internal_integer(tex_state::meaning::InternalInteger::PdfLastXImage)
                .expect("PDF image integer"))
                == 0
        );
        assert!(mode_vec(&control, stores).is_empty());
    });

    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = pdftex_image_control(stores);
        crate::test_harness::assign_int_param(
            stores,
            IntParam::PDF_OUTPUT,
            1,
            tex_state::AssignmentScope::Global,
        )
        .expect("integer parameter assignment");
        register_source(
            &mut control,
            br"\immediate\pdfximage width 7pt height 8pt depth 2pt attr{/Intent /RelativeColorimetric} page 3 cropbox {immediate.pdf}",
        );
        let pdf_state_before = stores.journal_cursor().expect("state cursor");
        let request = {
            let mut request = None;
            for _ in 0..TEST_STEP_LIMIT {
                match control.advance(stores).expect("immediate image suspends") {
                    StepResult::Suspended(ResourceNeed::PdfImage { request: need }) => {
                        request = Some(need);
                        break;
                    }
                    StepResult::Progress(_) => {}
                    other => panic!("expected immediate image suspension, got {other:?}"),
                }
            }
            request.expect("immediate PDF image resource need must be reached in bounds")
        };
        assert_eq!(
            stores.journal_cursor().expect("state cursor"),
            pdf_state_before
        );
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
        assert!(matches!(
            control.advance(stores),
            Err(ExecError::ResourceReplayRequired)
        ));
        assert_eq!(
            admitted!(stores, |context| context
                .internal_integer(tex_state::meaning::InternalInteger::PdfLastXImage)
                .expect("PDF image integer")),
            0
        );
        assert!(mode_vec(&control, stores).is_empty());
    });
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
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            let mut control = pdftex_image_control(stores);
            if mode != Mode::Vertical {
                control.modes.push(mode).expect("test mode push");
            }
            register_source(&mut control, br"\pdfrefximage 99");
            let state_before = stores.journal_cursor().expect("state cursor");

            assert!(
                matches!(
                    control.advance(stores),
                    Err(ExecError::Captured { error, .. })
                        if matches!(*error, ExecError::PdfExtensionInDviMode("pdfrefximage"))
                ),
                "mode {mode:?}"
            );
            assert_eq!(
                stores.journal_cursor().expect("state cursor"),
                state_before,
                "mode {mode:?}"
            );
            assert!(mode_vec(&control, stores).is_empty());
            assert!(terminal_text(stores).is_empty());
        });
    }

    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let source = test_pdf_image_source();
        let image = admitted!(stores, |context| context.allocate_pdf_external_image(
            source,
            tex_state::PdfExternalImageDimensions {
                width: Scaled::from_raw(11),
                height: Scaled::from_raw(12),
                depth: Scaled::from_raw(13),
            },
            0,
            Vec::new(),
        ))
        .expect("reference target image");
        assert_eq!(image.id().raw(), 1);
        let mut control = pdftex_image_control(stores);
        control.modes.push(Mode::Math).expect("test mode push");
        register_source(&mut control, br"\pdfrefximage 1");
        let state_before = stores.journal_cursor().expect("state cursor");

        assert!(matches!(
            control.advance(stores),
            Err(ExecError::Captured { error, .. })
                if matches!(*error, ExecError::PdfExtensionInDviMode("pdfrefximage"))
        ));
        assert_eq!(stores.journal_cursor().expect("state cursor"), state_before);
        assert!(mode_vec(&control, stores).is_empty());

        crate::test_harness::assign_int_param(
            stores,
            IntParam::PDF_OUTPUT,
            1,
            tex_state::AssignmentScope::Global,
        )
        .expect("integer parameter assignment");
        assert_eq!(
            control
                .advance(stores)
                .expect("PDF retry preserves the reference integer"),
            StepResult::Progress(MainControlStep::Continue)
        );
        assert!(matches!(
            mode_vec(&control, stores).as_slice(),
            [Node::Whatsit(Whatsit::PdfRefXImage {
                object: 1,
                width,
                height,
                depth,
            })] if *width == Scaled::from_raw(11)
                && *height == Scaled::from_raw(12)
                && *depth == Scaled::from_raw(13)
        ));

        crate::test_harness::with_nonstop_plain_universe(|missing_stores| {
            let mut missing = pdftex_image_control(missing_stores);
            register_source(&mut missing, br"\pdfrefximage 99");
            assert!(matches!(
                missing.advance(missing_stores),
                Err(ExecError::Captured { error, .. })
                    if matches!(*error, ExecError::PdfExtensionInDviMode("pdfrefximage"))
            ));
            crate::test_harness::assign_int_param(
                missing_stores,
                IntParam::PDF_OUTPUT,
                1,
                tex_state::AssignmentScope::Global,
            )
            .expect("integer parameter assignment");
            assert!(matches!(
                missing.advance(missing_stores),
                Err(ExecError::PdfReferencedObjectNotFound)
            ));
            assert!(mode_vec(&missing, missing_stores).is_empty());
        });
    });
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
        crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = pdftex_annotation_control(stores);
        control.modes.push(Mode::Horizontal).expect("test mode push");
        register_source(&mut control, source);
        assert!(
            matches!(control.step(stores), Err(ExecError::PdfExtensionInDviMode(name)) if name == primitive)
        );
        assert!(mode_vec(&control, stores).is_empty());

        crate::test_harness::assign_int_param(

            stores,

            IntParam::PDF_OUTPUT,

            1,

            tex_state::AssignmentScope::Global,

        )

        .expect("integer parameter assignment");
        assert_eq!(
            control
                .step(stores)
                .expect("PDF retry preserves the complete command"),
            MainControlStep::Continue
        );
        assert_eq!(mode_vec(&control, stores).len(), 1);
            });
}

    // The source orders the PDF-output check before the vertical-mode check
    // for both link commands.
    for primitive in ["pdfstartlink", "pdfendlink"] {
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            let mut control = pdftex_annotation_control(stores);
            register_source(&mut control, format!("\\{primitive}").as_bytes());
            assert!(
                matches!(control.step(stores), Err(ExecError::PdfExtensionInDviMode(name)) if name == primitive)
            );
            assert!(mode_vec(&control, stores).is_empty());
        });
    }
}
#[test]
fn pdf_link_vertical_mode_rejects_before_operand_scan_without_mutation() {
    // pdftex.web §1561 checks vertical mode before `new_annot_whatsit` and
    // therefore before the rule, attributes, and action.  The deliberately
    // malformed action must not mask the mode diagnostic, consume its
    // following token, allocate a link, or append a node.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        crate::test_harness::assign_int_param(
            stores,
            IntParam::PDF_OUTPUT,
            1,
            tex_state::AssignmentScope::Global,
        )
        .expect("integer parameter assignment");
        let mut control = pdftex_annotation_control(stores);
        register_source(
            &mut control,
            br"\pdfstartlink width 5pt definitely-not-an-action\relax",
        );
        let state_before = stores.journal_cursor().expect("state cursor");

        let error = control
            .step(stores)
            .expect_err("vertical link start is rejected before its operands");
        assert!(matches!(
            error,
            ExecError::PdfLinkInVerticalMode("pdfstartlink")
        ));
        assert_eq!(
            error.to_string(),
            "pdfTeX error (ext1): \\pdfstartlink cannot be used in vertical mode"
        );
        assert_eq!(stores.journal_cursor().expect("state cursor"), state_before);
        assert!(mode_vec(&control, stores).is_empty());

        control
            .modes
            .push(Mode::Horizontal)
            .expect("test mode push");
        let action_error = control.step(stores);
        assert!(
            matches!(
                action_error,
                Err(ExecError::PdfNavigation(
                    "pdfTeX error (ext1): action type missing"
                ))
            ),
            "unexpected action error: {action_error:?}"
        );
        let terminal = terminal_text(stores);
        assert!(terminal.contains("! pdfTeX error (ext1): action type missing."));
        assert!(terminal.contains("Fatal error occurred, no output PDF file produced!"));
        assert_eq!(
            stores.world().error_channel().history(),
            tex_state::print::ErrorHistory::FatalErrorStop
        );
        assert!(mode_vec(&control, stores).is_empty());
    });
}
#[test]
fn pdf_end_link_dvi_retry_preserves_the_open_link_and_command() {
    // pdftex.web §1561 rejects DVI mode before appending the end whatsit. The
    // open-link stack and the unconsumed command both survive for retry.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        crate::test_harness::assign_int_param(
            stores,
            IntParam::PDF_OUTPUT,
            1,
            tex_state::AssignmentScope::Global,
        )
        .expect("integer parameter assignment");
        let mut control = pdftex_annotation_control(stores);
        control
            .modes
            .push(Mode::Horizontal)
            .expect("test mode push");
        register_source(
            &mut control,
            br"\pdfstartlink height 4pt user{/Subtype /Link}\pdfendlink",
        );
        assert_eq!(
            control.step(stores).expect("start link"),
            MainControlStep::Continue
        );
        assert_eq!(mode_vec(&control, stores).len(), 1);

        crate::test_harness::assign_int_param(
            stores,
            IntParam::PDF_OUTPUT,
            0,
            tex_state::AssignmentScope::Global,
        )
        .expect("integer parameter assignment");
        assert!(matches!(
            control.step(stores),
            Err(ExecError::PdfExtensionInDviMode("pdfendlink"))
        ));
        assert_eq!(mode_vec(&control, stores).len(), 1);

        crate::test_harness::assign_int_param(
            stores,
            IntParam::PDF_OUTPUT,
            1,
            tex_state::AssignmentScope::Global,
        )
        .expect("integer parameter assignment");
        assert_eq!(
            control.step(stores).expect("end-link retry"),
            MainControlStep::Continue
        );
        assert!(matches!(
            mode_vec(&control, stores).as_slice(),
            [
                Node::Whatsit(Whatsit::PdfLinkStart { .. }),
                Node::Whatsit(Whatsit::PdfLinkEnd { .. })
            ]
        ));
    });
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
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            let mut control = pdftex_thread_control(stores);
            register_source(&mut control, source);
            assert!(
                matches!(control.step(stores), Err(ExecError::PdfExtensionInDviMode(name)) if name == primitive)
            );
            assert!(current_list_owner_vec(&control, stores).is_empty());
            crate::test_harness::assign_int_param(
                stores,
                IntParam::PDF_OUTPUT,
                1,
                tex_state::AssignmentScope::Global,
            )
            .expect("integer parameter assignment");
            assert_eq!(
                control.step(stores).expect("retry preserves every operand"),
                MainControlStep::Continue
            );
            assert_eq!(current_list_owner_vec(&control, stores).len(), 1);
        });
    }
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
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            crate::test_harness::assign_int_param(
                stores,
                IntParam::PDF_OUTPUT,
                1,
                tex_state::AssignmentScope::Global,
            )
            .expect("integer parameter assignment");
            let mut control = pdftex_destination_control(stores);
            if mode != Mode::Vertical {
                control.modes.push(mode).expect("test mode push");
            }
            register_source(
                &mut control,
                br"\pdfdest struct 9 name{target} fitr width 2pt height 3pt depth 4pt",
            );
            assert_eq!(
                control.step(stores).expect("destination command"),
                MainControlStep::Continue
            );
            let current_nodes = current_list_owner_vec(&control, stores);
            let [Node::Whatsit(Whatsit::PdfDestination(destination))] = current_nodes.as_slice()
            else {
                panic!(
                    "mode {mode:?}: expected one destination, got {:?}",
                    current_nodes
                );
            };
            if mode == Mode::Vertical {
                assert!(
                    mode_vec(&control, stores).is_empty(),
                    "outer vertical material has no separate ModeList owner"
                );
            } else {
                assert!(
                    admitted!(stores, |context| context.page_contributions().is_empty()),
                    "mode {mode:?} retains its own current-list owner"
                );
            }
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
                tex_state::node::NodePdfActionIdentifier::Name(_)
            ));
        });
    }
}
#[test]
fn pdf_destination_rejects_prefixes_and_dvi_before_operand_scan() {
    // pdftex.web §1565 calls `check_pdfoutput` before allocating the whatsit
    // or scanning `struct`, the identifier, the kind, or the rule dimensions.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        crate::test_harness::assign_int_param(
            stores,
            IntParam::PDF_OUTPUT,
            1,
            tex_state::AssignmentScope::Global,
        )
        .expect("integer parameter assignment");
        let global = stores.intern("global").expect("symbol interning");
        assign_static_meaning(
            stores,
            global,
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Global),
        );
        let mut control = pdftex_destination_control(stores);
        register_source(&mut control, br"\global\pdfdest name{prefixed} fit");
        assert_eq!(
            control.step(stores).expect("prefix recovery"),
            MainControlStep::Continue
        );
        assert!(current_list_owner_vec(&control, stores).is_empty());
        assert!(terminal_text(stores).contains("You can't use a prefix with"));
        assert_eq!(
            control.step(stores).expect("replayed destination command"),
            MainControlStep::Continue
        );
        assert_eq!(current_list_owner_vec(&control, stores).len(), 1);

        crate::test_harness::with_nonstop_plain_universe(|dvi_stores| {
            let mut dvi = pdftex_destination_control(dvi_stores);
            register_source(
                &mut dvi,
                br"\pdfdest struct 7 name{retry} fitr width 5pt height 6pt depth 7pt",
            );
            assert!(matches!(
                dvi.step(dvi_stores),
                Err(ExecError::PdfExtensionInDviMode("pdfdest"))
            ));
            assert!(current_list_owner_vec(&dvi, dvi_stores).is_empty());
            crate::test_harness::assign_int_param(
                dvi_stores,
                IntParam::PDF_OUTPUT,
                1,
                tex_state::AssignmentScope::Global,
            )
            .expect("integer parameter assignment");
            assert_eq!(
                dvi.step(dvi_stores)
                    .expect("failed destination retries with every operand intact"),
                MainControlStep::Continue
            );
            let current_nodes = current_list_owner_vec(&dvi, dvi_stores);
            let [Node::Whatsit(Whatsit::PdfDestination(destination))] = current_nodes.as_slice()
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
        });
    });
}
#[test]
fn pdf_destination_scanner_failure_publishes_the_pdf_fatal_channels() {
    // pdftex.web §1565 calls `pdf_error` for a nonpositive numeric
    // destination. The scanner may run in the ordinary delivery episode, but
    // its typed failure must still cross the same PDF fatal publication seam.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        crate::test_harness::assign_int_param(
            stores,
            IntParam::PDF_OUTPUT,
            1,
            tex_state::AssignmentScope::Global,
        )
        .expect("integer parameter assignment");
        let mut control = pdftex_destination_control(stores);
        register_source(&mut control, br"\pdfdest num 0 fit");

        let error = control.step(stores).expect_err("zero destination is fatal");
        assert!(error.is_pdftex_output_fatal());
        assert!(
            terminal_text(stores).contains("pdfTeX error (ext1): num identifier must be positive")
        );
        let log = stores
            .world()
            .memory_log_output()
            .map(String::from_utf8_lossy)
            .unwrap_or_default();
        assert!(log.contains("pdfTeX error (ext1): num identifier must be positive"));
    });
}
#[test]
fn observed_pdf_fatal_error_publishes_its_committed_receipt() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        crate::test_harness::assign_int_param(
            stores,
            IntParam::PDF_OUTPUT,
            1,
            tex_state::AssignmentScope::Global,
        )
        .expect("integer parameter assignment");
        let mut control = pdftex_destination_control(stores);
        register_source(&mut control, br"\pdfdest num 0 fit");
        let mut observations = ObservationRecorder::default();

        let error = control
            .advance_with_observer(stores, &mut observations)
            .expect_err("zero destination is fatal");
        assert!(error.is_pdftex_output_fatal());
        assert!(
            control.error_operation_committed(),
            "committed terminal error: {error:?}; observations: {:?}",
            observations.0
        );
        let terminations = observations
            .0
            .iter()
            .filter(|observation| {
                matches!(
                    observation,
                    CommandObservation::Effect(effect)
                        if effect.kind == ObservationEffectKind::Terminate
                )
            })
            .count();
        assert_eq!(
            terminations, 1,
            "terminal receipt publication: {:?}",
            observations.0
        );
    });
}
#[test]
fn observed_pdf_dvi_preflight_error_discards_its_uncommitted_receipt() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = pdftex_destination_control(stores);
        register_source(&mut control, br"\pdfdest name{retry} fit");
        let mut observations = ObservationRecorder::default();

        let error = control
            .advance_with_observer(stores, &mut observations)
            .expect_err("DVI destination preflight rejects the command");
        assert!(
            matches!(
                &error,
                ExecError::Captured { error, .. }
                    if matches!(**error, ExecError::PdfExtensionInDviMode("pdfdest"))
            ),
            "unexpected DVI preflight error: {error:?}"
        );
        assert!(!control.error_operation_committed());
        assert!(
            observations.0.is_empty(),
            "recoverable preflight suffix was published: {:?}",
            observations.0
        );
    });
}
#[test]
fn pdf_destination_grouping_and_checkpoint_restore_preserve_node_ownership() {
    // pdftex.web §1565 appends a whatsit, not an eqtb assignment: ordinary
    // grouping does not undo it, while an engine checkpoint restores both the
    // current list and the unconsumed source for deterministic retry.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        crate::test_harness::assign_int_param(
            stores,
            IntParam::PDF_OUTPUT,
            1,
            tex_state::AssignmentScope::Global,
        )
        .expect("integer parameter assignment");
        let mut control = pdftex_destination_control(stores);
        register_source(&mut control, br"{\pdfdest num 23 xyz zoom -40}");
        let checkpoint = control
            .capture_checkpoint(
                crate::EngineBoundary::OuterParagraphEnd,
                stores,
                crate::ExecutionBudgetCounters::default(),
            )
            .expect("destination state checkpoints");
        for label in ["open group", "destination", "close group"] {
            assert_eq!(
                control.step(stores).expect(label),
                MainControlStep::Continue
            );
        }
        assert_eq!(
            admitted!(stores, |context| context.execution_group_depth()),
            0
        );
        let first_hash = stores.journal_cursor().expect("state cursor");
        assert!(matches!(
            current_list_owner_vec(&control, stores).as_slice(),
            [Node::Whatsit(Whatsit::PdfDestination(destination))]
                if matches!(
                    destination.kind,
                    tex_state::node::PdfDestinationKind::Xyz { zoom: Some(-40) }
                )
        ));

        control
            .restore_checkpoint(&checkpoint, stores)
            .expect("destination state restores");
        assert!(current_list_owner_vec(&control, stores).is_empty());
        for label in [
            "retried open group",
            "retried destination",
            "retried close group",
        ] {
            assert_eq!(
                control.step(stores).expect(label),
                MainControlStep::Continue
            );
        }
        assert_eq!(stores.journal_cursor().expect("state cursor"), first_hash);
        assert!(matches!(
            current_list_owner_vec(&control, stores).as_slice(),
            [Node::Whatsit(Whatsit::PdfDestination(destination))]
                if matches!(
                    destination.kind,
                    tex_state::node::PdfDestinationKind::Xyz { zoom: Some(-40) }
                )
        ));
    });
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
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            crate::test_harness::assign_int_param(
                stores,
                IntParam::PDF_OUTPUT,
                1,
                tex_state::AssignmentScope::Global,
            )
            .expect("integer parameter assignment");
            let mut control = pdftex_outline_control(stores);
            if mode != Mode::Vertical {
                control.modes.push(mode).expect("test mode push");
            }
            register_source(
                &mut control,
                br"\pdfoutline attr{/C [1 0 0]} goto name{later} count -2 {(Title)}",
            );
            assert_eq!(
                control.step(stores).expect("outline command"),
                MainControlStep::Continue
            );
            assert!(
                mode_vec(&control, stores).is_empty(),
                "mode {mode:?}: outlines are immediate document state"
            );
        });
    }
}
#[test]
fn pdf_outline_rejects_prefixes_and_dvi_before_operand_scan() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        crate::test_harness::assign_int_param(
            stores,
            IntParam::PDF_OUTPUT,
            1,
            tex_state::AssignmentScope::Global,
        )
        .expect("integer parameter assignment");
        let global = stores.intern("global").expect("symbol interning");
        assign_static_meaning(
            stores,
            global,
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Global),
        );
        let mut control = pdftex_outline_control(stores);
        register_source(&mut control, br"\global\pdfoutline user{/S /URI}{Title}");
        assert_eq!(
            control.step(stores).expect("prefix recovery"),
            MainControlStep::Continue
        );
        assert!(terminal_text(stores).contains("You can't use a prefix with"));
        assert_eq!(
            control.step(stores).expect("replayed outline"),
            MainControlStep::Continue
        );

        crate::test_harness::with_nonstop_plain_universe(|dvi_stores| {
            let mut dvi = pdftex_outline_control(dvi_stores);
            register_source(&mut dvi, br"\pdfoutline user{/S /URI}{Title}");
            assert!(matches!(
                dvi.step(dvi_stores),
                Err(ExecError::PdfExtensionInDviMode("pdfoutline"))
            ));
            crate::test_harness::assign_int_param(
                dvi_stores,
                IntParam::PDF_OUTPUT,
                1,
                tex_state::AssignmentScope::Global,
            )
            .expect("integer parameter assignment");
            assert_eq!(
                dvi.step(dvi_stores)
                    .expect("failed command retries with every operand intact"),
                MainControlStep::Continue
            );
        });
    });
}
#[test]
fn pdf_outline_is_not_restored_by_ordinary_grouping() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        crate::test_harness::assign_int_param(
            stores,
            IntParam::PDF_OUTPUT,
            1,
            tex_state::AssignmentScope::Global,
        )
        .expect("integer parameter assignment");
        let mut control = pdftex_outline_control(stores);
        register_source(
            &mut control,
            br"{\pdfoutline goto name{later} count 1 {Title}}",
        );
        for label in ["open group", "outline", "close group"] {
            assert_eq!(
                control.step(stores).expect(label),
                MainControlStep::Continue
            );
        }
        assert_eq!(
            admitted!(stores, |context| context.execution_group_depth()),
            0
        );
    });
}
#[test]
fn pdf_outline_checkpoint_restore_replays_identical_ledger_state() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        crate::test_harness::assign_int_param(
            stores,
            IntParam::PDF_OUTPUT,
            1,
            tex_state::AssignmentScope::Global,
        )
        .expect("integer parameter assignment");
        let mut control = pdftex_outline_control(stores);
        register_source(
            &mut control,
            br"\pdfoutline goto name{later} count 1 {Title}",
        );
        let checkpoint = control
            .capture_checkpoint(
                crate::EngineBoundary::OuterParagraphEnd,
                stores,
                crate::ExecutionBudgetCounters::default(),
            )
            .expect("outline state checkpoints");
        assert_eq!(
            control.step(stores).expect("outline command"),
            MainControlStep::Continue
        );
        let first_hash = stores.journal_cursor().expect("state cursor");
        control
            .restore_checkpoint(&checkpoint, stores)
            .expect("outline state restores");
        assert_eq!(
            control.step(stores).expect("retried outline"),
            MainControlStep::Continue
        );
        assert_eq!(stores.journal_cursor().expect("state cursor"), first_hash);
    });
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
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            crate::test_harness::assign_int_param(
                stores,
                IntParam::PDF_OUTPUT,
                1,
                tex_state::AssignmentScope::Global,
            )
            .expect("integer parameter assignment");
            let mut control = pdftex_snapping_control(stores);
            if mode != Mode::Vertical {
                control.modes.push(mode).expect("test mode push");
            }
            register_source(
                &mut control,
                br"\pdfsnaprefpoint\pdfsnapy 4pt plus 2fil minus 1pt\pdfsnapycomp 1200",
            );
            for _ in 0..3 {
                assert_eq!(
                    control.step(stores).expect("snapping command"),
                    MainControlStep::Continue
                );
            }
            let nodes = current_list_owner_vec(&control, stores);
            assert!(
                matches!(
                    nodes.as_slice(),
                    [
                        Node::Whatsit(Whatsit::PdfSnapRefPoint),
                        Node::Whatsit(Whatsit::PdfSnapY { .. }),
                        Node::Whatsit(Whatsit::PdfSnapYComp { ratio: 1000 })
                    ]
                ),
                "mode {mode:?}: {nodes:?}"
            );
            let Node::Whatsit(Whatsit::PdfSnapY { ref glue }) = nodes[1] else {
                unreachable!()
            };
            assert_eq!(glue.width, Scaled::from_raw(4 * 65_536));
            assert_eq!(glue.stretch_order, tex_state::glue::Order::Fil);
        });
    }
}
#[test]
fn pdf_snapping_rejects_prefixes_and_dvi_before_operand_scan() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        crate::test_harness::assign_int_param(
            stores,
            IntParam::PDF_OUTPUT,
            1,
            tex_state::AssignmentScope::Global,
        )
        .expect("integer parameter assignment");
        let global = stores.intern("global").expect("symbol interning");
        assign_static_meaning(
            stores,
            global,
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Global),
        );
        let mut control = pdftex_snapping_control(stores);
        register_source(&mut control, br"\global\pdfsnaprefpoint");
        assert_eq!(
            control.step(stores).expect("prefix recovery"),
            MainControlStep::Continue
        );
        assert!(current_list_owner_vec(&control, stores).is_empty());
        assert!(terminal_text(stores).contains("You can't use a prefix with"));
        assert_eq!(
            control.step(stores).expect("replayed snapping command"),
            MainControlStep::Continue
        );
        assert!(matches!(
            current_list_owner_vec(&control, stores).as_slice(),
            [Node::Whatsit(Whatsit::PdfSnapRefPoint)]
        ));

        crate::test_harness::with_nonstop_plain_universe(|dvi_stores| {
            let mut dvi = pdftex_snapping_control(dvi_stores);
            register_source(&mut dvi, br"\pdfsnapy 7pt");
            assert!(matches!(
                dvi.step(dvi_stores),
                Err(ExecError::PdfExtensionInDviMode("pdfsnapy"))
            ));
            assert!(current_list_owner_vec(&dvi, dvi_stores).is_empty());
            crate::test_harness::assign_int_param(
                dvi_stores,
                IntParam::PDF_OUTPUT,
                1,
                tex_state::AssignmentScope::Global,
            )
            .expect("integer parameter assignment");
            assert_eq!(
                dvi.step(dvi_stores)
                    .expect("failed command retries with its operand intact"),
                MainControlStep::Continue
            );
            assert!(matches!(
                current_list_owner_vec(&dvi, dvi_stores).as_slice(),
                [Node::Whatsit(Whatsit::PdfSnapY { .. })]
            ));
        });
    });
}
#[test]
fn pdfsnapy_rejects_negative_width_after_consuming_the_complete_glue() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        crate::test_harness::assign_int_param(
            stores,
            IntParam::PDF_OUTPUT,
            1,
            tex_state::AssignmentScope::Global,
        )
        .expect("integer parameter assignment");
        let mut control = pdftex_snapping_control(stores);
        register_source(&mut control, br"\pdfsnapy -1pt plus 2fil");
        assert!(matches!(
            control.step(stores),
            Err(ExecError::PdfNavigation(
                "pdfTeX error (ext1): negative snap glue"
            ))
        ));
        assert!(current_list_owner_vec(&control, stores).is_empty());
    });
}
#[test]
fn pdf_snapping_checkpoint_restore_retries_without_duplicate_nodes() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        crate::test_harness::assign_int_param(
            stores,
            IntParam::PDF_OUTPUT,
            1,
            tex_state::AssignmentScope::Global,
        )
        .expect("integer parameter assignment");
        let mut control = pdftex_snapping_control(stores);
        register_source(
            &mut control,
            br"\pdfsnaprefpoint\pdfsnapy 3pt\pdfsnapycomp 500",
        );
        let checkpoint = control
            .capture_checkpoint(
                crate::EngineBoundary::OuterParagraphEnd,
                stores,
                crate::ExecutionBudgetCounters::default(),
            )
            .expect("snapping state checkpoints");
        assert_eq!(
            control.step(stores).expect("reference point"),
            MainControlStep::Continue
        );
        assert_eq!(
            control.step(stores).expect("snap glue"),
            MainControlStep::Continue
        );
        assert_eq!(
            control.step(stores).expect("snap compensation"),
            MainControlStep::Continue
        );
        control
            .restore_checkpoint(&checkpoint, stores)
            .expect("snapping state restores");
        assert_eq!(
            control.step(stores).expect("retried reference point"),
            MainControlStep::Continue
        );
        assert_eq!(
            control.step(stores).expect("retried snap glue"),
            MainControlStep::Continue
        );
        assert_eq!(
            control.step(stores).expect("retried snap compensation"),
            MainControlStep::Continue
        );
        assert!(matches!(
            current_list_owner_vec(&control, stores).as_slice(),
            [
                Node::Whatsit(Whatsit::PdfSnapRefPoint),
                Node::Whatsit(Whatsit::PdfSnapY { .. }),
                Node::Whatsit(Whatsit::PdfSnapYComp { ratio: 500 })
            ]
        ));
    });
}
#[test]
fn pdfsetrandomseed_is_an_ungrouped_signed_job_state_replacement() {
    crate::test_harness::with_nonstop_universe(|stores| {
        let mut control = pdftex_random_control(stores);
        register_source(
            &mut control,
            br"{\pdfsetrandomseed -1 }\pdfsetrandomseed 23 ",
        );

        step_until_pdf_seed(&mut control, stores, 1);
        assert_eq!(stores.world().pdf_random_seed(), 1);
        assert_eq!(stores.world_mut().pdf_uniform_deviate(10), 7);

        assert_eq!(
            control.step(stores).expect("end group"),
            MainControlStep::Continue
        );
        assert_eq!(
            stores.world().pdf_random_seed(),
            1,
            "the extension state is not restored when a TeX group closes"
        );
        step_until_pdf_seed(&mut control, stores, 23);
        assert_eq!(stores.world().pdf_random_seed(), 23);
    });
}
#[test]
fn pdfsetrandomseed_uses_the_ordinary_integer_scanner_and_preserves_lookahead() {
    crate::test_harness::with_nonstop_universe(|stores| {
        stores.set_interaction_mode(tex_state::InteractionMode::Nonstop);
        let mut control = pdftex_random_control(stores);
        register_source(
            &mut control,
            br"\pdfsetrandomseed 999999999999\pdfsetrandomseed 6 ",
        );

        assert_eq!(
            control.step(stores).expect("bounded seed scan"),
            MainControlStep::Continue
        );
        assert_eq!(stores.world().pdf_random_seed(), i32::MAX);

        assert_eq!(
            control.step(stores).expect("backed-up following command"),
            MainControlStep::Continue
        );
        assert_eq!(stores.world().pdf_random_seed(), 6);
    });
}
#[test]
fn pdfsetrandomseed_rejects_assignment_prefixes_then_replays_the_command() {
    crate::test_harness::with_nonstop_universe(|stores| {
        let global = stores.intern("global").expect("symbol interning");
        assign_static_meaning(
            stores,
            global,
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Global),
        );
        let mut control = pdftex_random_control(stores);
        register_source(&mut control, br"\global\pdfsetrandomseed 9 ");

        assert_eq!(
            control.step(stores).expect("reject prefix"),
            MainControlStep::Continue
        );
        assert_eq!(stores.world().pdf_random_seed(), 0);
        assert!(
            terminal_text(stores).contains("You can't use a prefix with"),
            "the extension is below max_non_prefixed_command"
        );

        assert_eq!(
            control.step(stores).expect("replayed seed command"),
            MainControlStep::Continue
        );
        assert_eq!(stores.world().pdf_random_seed(), 9);
    });
}
#[test]
fn pdfresettimer_is_no_operand_any_mode_ungrouped_job_state() {
    crate::test_harness::with_nonstop_universe(|stores| {
        stores.world_mut().set_pdf_time_micros(1_250_000);
        let mut control = pdftex_timer_control(stores);
        register_source(&mut control, br"{\pdfresettimer X}");

        assert_eq!(
            control.step(stores).expect("begin group"),
            MainControlStep::Continue
        );
        for _ in 0..3 {
            control.step(stores).expect("timer reset");
            if stores.world().pdf_elapsed_time() == 0 {
                break;
            }
        }
        assert_eq!(stores.world().pdf_elapsed_time(), 0);

        stores.world_mut().set_pdf_time_micros(2_250_000);
        run_to_end(&mut control, stores);
        assert_eq!(
            stores.world().pdf_elapsed_time(),
            65_536,
            "the reset is not restored by a group, and the following token was not consumed"
        );
    });
}
#[test]
fn pdfresettimer_rejects_assignment_prefixes_then_replays_the_command() {
    crate::test_harness::with_nonstop_universe(|stores| {
        stores.world_mut().set_pdf_time_micros(1_250_000);
        let global = stores.intern("global").expect("symbol interning");
        assign_static_meaning(
            stores,
            global,
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Global),
        );
        let mut control = pdftex_timer_control(stores);
        register_source(&mut control, br"\global\pdfresettimer ");

        assert_eq!(
            control.step(stores).expect("reject prefix"),
            MainControlStep::Continue
        );
        assert_eq!(stores.world().pdf_elapsed_time(), 81_920);
        assert!(terminal_text(stores).contains("You can't use a prefix with"));

        assert_eq!(
            control.step(stores).expect("replayed timer reset"),
            MainControlStep::Continue
        );
        assert_eq!(stores.world().pdf_elapsed_time(), 0);
    });
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
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            crate::test_harness::assign_int_param(
                stores,
                IntParam::PDF_OUTPUT,
                1,
                tex_state::AssignmentScope::Global,
            )
            .expect("integer parameter assignment");
            let mut control = pdftex_interword_control(stores);
            if mode != Mode::Vertical {
                control.modes.push(mode).expect("test mode push");
            }
            register_source(
                &mut control,
                br"\pdfinterwordspaceon\pdffakespace\pdfinterwordspaceoff",
            );
            run_to_end(&mut control, stores);

            let controls: Vec<_> = current_list_owner_vec(&control, stores)
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
        });
    }

    crate::test_harness::with_nonstop_plain_universe(|grouped_stores| {
        crate::test_harness::assign_int_param(
            grouped_stores,
            IntParam::PDF_OUTPUT,
            1,
            tex_state::AssignmentScope::Global,
        )
        .expect("integer parameter assignment");
        let mut grouped = pdftex_interword_control(grouped_stores);
        register_source(&mut grouped, br"{\pdffakespace}");
        run_to_end(&mut grouped, grouped_stores);
        assert!(matches!(
            current_list_owner_vec(&grouped, grouped_stores).as_slice(),
            [Node::Whatsit(Whatsit::PdfAccessibility(
                tex_state::node::PdfAccessibilityControl::FakeSpace
            ))]
        ));
    });
}
#[test]
fn pdfinterwordspace_rejects_prefixes_and_dvi_mode_before_appending() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        crate::test_harness::assign_int_param(
            stores,
            IntParam::PDF_OUTPUT,
            1,
            tex_state::AssignmentScope::Global,
        )
        .expect("integer parameter assignment");
        let global = stores.intern("global").expect("symbol interning");
        assign_static_meaning(
            stores,
            global,
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Global),
        );
        let mut control = pdftex_interword_control(stores);
        register_source(&mut control, br"\global\pdfinterwordspaceon");

        assert_eq!(
            control.step(stores).expect("prefix recovery"),
            MainControlStep::Continue
        );
        assert!(current_list_owner_vec(&control, stores).is_empty());
        assert!(terminal_text(stores).contains("You can't use a prefix with"));
        assert_eq!(
            control.step(stores).expect("replayed extension"),
            MainControlStep::Continue
        );
        assert!(matches!(
            current_list_owner_vec(&control, stores).as_slice(),
            [Node::Whatsit(Whatsit::PdfAccessibility(
                tex_state::node::PdfAccessibilityControl::InterwordSpaceOn
            ))]
        ));

        crate::test_harness::with_nonstop_plain_universe(|dvi_stores| {
            let mut dvi_control = pdftex_interword_control(dvi_stores);
            register_source(&mut dvi_control, br"\pdffakespace");
            assert!(matches!(
                dvi_control.step(dvi_stores),
                Err(ExecError::PdfExtensionInDviMode("pdffakespace"))
            ));
            assert!(current_list_owner_vec(&dvi_control, dvi_stores).is_empty());
        });
    });
}
#[test]
fn pdfinterwordspace_checkpoint_restore_retries_without_duplicate_effects() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        crate::test_harness::assign_int_param(
            stores,
            IntParam::PDF_OUTPUT,
            1,
            tex_state::AssignmentScope::Global,
        )
        .expect("integer parameter assignment");
        let mut control = pdftex_interword_control(stores);
        register_source(&mut control, br"\pdfinterwordspaceon\pdfinterwordspaceoff");

        let checkpoint = control
            .capture_checkpoint(
                crate::EngineBoundary::OuterParagraphEnd,
                stores,
                crate::ExecutionBudgetCounters::default(),
            )
            .expect("quiescent toggle state checkpoints");
        assert_eq!(
            control.step(stores).expect("first toggle"),
            MainControlStep::Continue
        );
        assert_eq!(
            control.step(stores).expect("second toggle"),
            MainControlStep::Continue
        );
        control
            .restore_checkpoint(&checkpoint, stores)
            .expect("toggle state restores");
        assert_eq!(
            control.step(stores).expect("first toggle retries"),
            MainControlStep::Continue
        );
        assert_eq!(
            control.step(stores).expect("second toggle retries"),
            MainControlStep::Continue
        );

        let controls: Vec<_> = current_list_owner_vec(&control, stores)
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
    });
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
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            crate::test_harness::assign_int_param(
                stores,
                IntParam::PDF_OUTPUT,
                1,
                tex_state::AssignmentScope::Global,
            )
            .expect("integer parameter assignment");
            let mut control = pdftex_interword_control(stores);
            if mode != Mode::Vertical {
                control.modes.push(mode).expect("test mode push");
            }
            register_source(&mut control, br"\pdfrunninglinkoff\pdfrunninglinkon");
            run_to_end(&mut control, stores);

            let toggles = current_list_owner_vec(&control, stores)
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
        });
    }

    crate::test_harness::with_nonstop_plain_universe(|grouped_stores| {
        crate::test_harness::assign_int_param(
            grouped_stores,
            IntParam::PDF_OUTPUT,
            1,
            tex_state::AssignmentScope::Global,
        )
        .expect("integer parameter assignment");
        let mut grouped = pdftex_interword_control(grouped_stores);
        register_source(&mut grouped, br"{\pdfrunninglinkoff\pdfrunninglinkon}");
        run_to_end(&mut grouped, grouped_stores);
        assert!(matches!(
            current_list_owner_vec(&grouped, grouped_stores).as_slice(),
            [
                Node::Whatsit(Whatsit::PdfRunningLink(false)),
                Node::Whatsit(Whatsit::PdfRunningLink(true))
            ]
        ));
    });
}
#[test]
fn pdfrunninglink_rejects_prefixes_and_dvi_mode_before_appending() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        crate::test_harness::assign_int_param(
            stores,
            IntParam::PDF_OUTPUT,
            1,
            tex_state::AssignmentScope::Global,
        )
        .expect("integer parameter assignment");
        let global = stores.intern("global").expect("symbol interning");
        assign_static_meaning(
            stores,
            global,
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Global),
        );
        let mut control = pdftex_interword_control(stores);
        register_source(&mut control, br"\global\pdfrunninglinkoff");

        assert_eq!(
            control.step(stores).expect("prefix recovery"),
            MainControlStep::Continue
        );
        assert!(current_list_owner_vec(&control, stores).is_empty());
        assert!(terminal_text(stores).contains("You can't use a prefix with"));
        assert_eq!(
            control.step(stores).expect("replayed extension"),
            MainControlStep::Continue
        );
        assert!(matches!(
            current_list_owner_vec(&control, stores).as_slice(),
            [Node::Whatsit(Whatsit::PdfRunningLink(false))]
        ));

        crate::test_harness::with_nonstop_plain_universe(|dvi_stores| {
            let mut dvi_control = pdftex_interword_control(dvi_stores);
            register_source(&mut dvi_control, br"\pdfrunninglinkon");
            assert!(matches!(
                dvi_control.step(dvi_stores),
                Err(ExecError::PdfExtensionInDviMode("pdfrunninglinkon"))
            ));
            assert!(current_list_owner_vec(&dvi_control, dvi_stores).is_empty());
        });
    });
}
#[test]
fn pdfrunninglink_checkpoint_restore_retries_without_duplicate_whatsits() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        crate::test_harness::assign_int_param(
            stores,
            IntParam::PDF_OUTPUT,
            1,
            tex_state::AssignmentScope::Global,
        )
        .expect("integer parameter assignment");
        let mut control = pdftex_interword_control(stores);
        register_source(&mut control, br"\pdfrunninglinkoff\pdfrunninglinkon");

        let checkpoint = control
            .capture_checkpoint(
                crate::EngineBoundary::OuterParagraphEnd,
                stores,
                crate::ExecutionBudgetCounters::default(),
            )
            .expect("running-link toggle checkpoints");
        assert_eq!(
            control.step(stores).expect("first toggle"),
            MainControlStep::Continue
        );
        assert_eq!(
            control.step(stores).expect("second toggle"),
            MainControlStep::Continue
        );
        control
            .restore_checkpoint(&checkpoint, stores)
            .expect("running-link toggle restores");
        assert_eq!(
            control.step(stores).expect("first toggle retries"),
            MainControlStep::Continue
        );
        assert_eq!(
            control.step(stores).expect("second toggle retries"),
            MainControlStep::Continue
        );

        let toggles = current_list_owner_vec(&control, stores)
            .iter()
            .filter_map(|node| match node {
                Node::Whatsit(Whatsit::PdfRunningLink(enabled)) => Some(*enabled),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(toggles, [false, true]);
    });
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
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            crate::test_harness::assign_int_param(
                stores,
                IntParam::PDF_OUTPUT,
                1,
                tex_state::AssignmentScope::Global,
            )
            .expect("integer parameter assignment");
            let replacement = "fixture"
                .chars()
                .map(|ch| {
                    tex_state::token::TokenWord::pack(Token::Char {
                        ch,
                        cat: Catcode::Letter,
                    })
                })
                .collect::<Vec<_>>();
            let definition = stores
                .allocate_definition(&[], &replacement)
                .expect("fixture macro definition");
            let name = stores.intern("n").expect("symbol interning");
            admitted!(stores, |context| context
                .assign_resolved_meaning(
                    name.symbol(),
                    tex_state::ResolvedMeaning::Macro {
                        flags: MeaningFlags::EMPTY,
                        definition,
                    },
                    tex_state::AssignmentScope::Global,
                )
                .expect("fixture macro assignment"));
            let mut control = pdftex_interword_control(stores);
            // These synthetic open modes exercise only the assignment. They
            // are an authored fragment, so stop at root EOF without inventing
            // a mode-specific final-cleanup sequence or silently adding `\end`.
            control.set_root_completion_policy(RootCompletionPolicy::StopAtRootEof);
            if mode != Mode::Vertical {
                control.modes.push(mode).expect("test mode push");
            }
            register_source(&mut control, br"{\pdfspacefont{\n-space}}X");
            run_to_end(&mut control, stores);

            assert!(control.fatal_error().is_none(), "mode {mode:?}");
        });
    }
}
#[test]
fn pdfspacefont_rejects_prefixes_and_dvi_mode_before_scanning() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        crate::test_harness::assign_int_param(
            stores,
            IntParam::PDF_OUTPUT,
            1,
            tex_state::AssignmentScope::Global,
        )
        .expect("integer parameter assignment");
        let global = stores.intern("global").expect("symbol interning");
        assign_static_meaning(
            stores,
            global,
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Global),
        );
        let mut control = pdftex_interword_control(stores);
        register_source(&mut control, br"\global\pdfspacefont{selected}");

        assert_eq!(
            control.step(stores).expect("prefix recovery"),
            MainControlStep::Continue
        );
        assert!(terminal_text(stores).contains("You can't use a prefix with"));
        assert_eq!(
            control.step(stores).expect("replayed extension"),
            MainControlStep::Continue
        );

        crate::test_harness::with_nonstop_plain_universe(|dvi_stores| {
            let mut dvi_control = pdftex_interword_control(dvi_stores);
            register_source(&mut dvi_control, br"\pdfspacefont{unscanned}");
            assert!(matches!(
                dvi_control.step(dvi_stores),
                Err(ExecError::PdfExtensionInDviMode("pdfspacefont"))
            ));
        });
    });
}
#[test]
fn pdfspacefont_checkpoint_restore_retries_the_global_selection_atomically() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        crate::test_harness::assign_int_param(
            stores,
            IntParam::PDF_OUTPUT,
            1,
            tex_state::AssignmentScope::Global,
        )
        .expect("integer parameter assignment");
        let mut control = pdftex_interword_control(stores);
        register_source(&mut control, br"\pdfspacefont{first}\pdfspacefont{second}");

        assert_eq!(
            control.step(stores).expect("first selection"),
            MainControlStep::Continue
        );
        let checkpoint = control
            .capture_checkpoint(
                crate::EngineBoundary::OuterParagraphEnd,
                stores,
                crate::ExecutionBudgetCounters::default(),
            )
            .expect("space-font state checkpoints");
        assert_eq!(
            control.step(stores).expect("second selection"),
            MainControlStep::Continue
        );
        let selected = stores.journal_cursor().expect("selected state cursor");

        control
            .restore_checkpoint(&checkpoint, stores)
            .expect("space-font state restores");
        assert_eq!(
            control.step(stores).expect("second selection retries"),
            MainControlStep::Continue
        );
        assert_eq!(
            stores.journal_cursor().expect("retried state cursor"),
            selected
        );
    });
}
#[test]
fn outer_vertical_pdf_whatsits_cross_page_successors_before_final_end() {
    // pdftex.web §§1524/1563/1565 append graphics and destination whatsits to
    // the current list in every mode. TeX82 §§994--1026 then completes either
    // default or explicit output and resumes the page builder on its successor
    // before §1054 may accept the final stop. The late whatsits therefore
    // belong to the successor's contribution queue, never the old outer mode
    // root, and each of the two pages ships exactly once.
    for (name, output) in [
        ("default", ""),
        ("explicit", "\\output={\\shipout\\box255}"),
    ] {
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            let mut control = pdftex_initex(stores);
            register_source(
                &mut control,
                format!(
                    "\\pdfoutput=1\\vsize=5pt{output}\\hrule height10pt\\penalty-10000\\pdfdest name{{late}} fit\\pdfcolorstack0 push{{0 g}}\\end"
                )
                .as_bytes(),
            );
            let mut observations = ObservationRecorder::default();
            run_to_end_observed(&mut control, stores, &mut observations);

            assert_eq!(stores.world().artifact_commits().len(), 2, "{name}");
            assert!(mode_vec(&control, stores).is_empty(), "{name}");
            assert!(admitted!(stores, |context| context
                .page_contributions()
                .is_empty()));
            assert!(admitted!(stores, |context| context
                .current_page_nodes()
                .next()
                .is_none()));
            assert!(!control.page_region_succession_pending, "{name}");
            assert!(!control.boxes.output_routine_active, "{name}");
            admitted!(stores, |context| {
                assert!(context.page_fire_up().is_none(), "{name}");
                assert!(
                    !context.page_builder_resume_after_output_pending(),
                    "{name}"
                );
            });
            let shipouts = observations
                .0
                .iter()
                .filter(|observation| {
                    matches!(
                        observation,
                        CommandObservation::Effect(effect)
                            if effect.kind == ObservationEffectKind::Shipout
                    )
                })
                .count();
            assert_eq!(shipouts, 2, "{name}");
            assert!(observations.0.iter().any(|observation| matches!(
                observation,
                CommandObservation::Effect(effect)
                    if effect.kind == ObservationEffectKind::Terminate
            )));
        });
    }
}
