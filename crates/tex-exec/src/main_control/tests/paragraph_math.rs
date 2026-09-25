//! Paragraph and math execution with state and material observations.

use super::*;

#[test]
fn main_loop_boundary_keeps_vertical_glue_delivery_live_for_backup() {
    // TeX82 §1038 leaves the already-read `\vfil` in cur_cmd when a
    // character run ends. Section 1095 then backs it up before inserting
    // `\par`. Both transitions must address the same delivery even when
    // executor preflight uses two processor borrows.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_cmr10_as(&mut control, stores, "cmr10.tfm");
        register_source(&mut control, br"\font\f=cmr10 \f A\vfil\end");

        run_to_end(&mut control, stores);

        assert_eq!(stores.world().committed_artifacts().len(), 2);
    });
}

#[test]
fn etex_lastlinefit_traces_saved_shortfall_glue_and_final_adjustment() {
    // e-TeX change-file section 38.846 prints the two extra active-node
    // words whenever last-line fitting is enabled, naming the terminal
    // candidate's second value as its adjustment rather than ordinary glue.
    with_etex(
        br"\def\z{\hbox to30pt{}\hskip5pt plus20pt minus4pt }\tracingparagraphs=1\tracingonline=1\hbadness=100\pretolerance=9000\parfillskip=0pt plus1fill\hsize=96pt\lastlinefit=500\setbox0=\vbox{\noindent\z\z\z\z\z}\end",
    |stores| {

    let trace = terminal_text(stores);
    for expected in [
        "@@1: line 1.0 t=137641 s=31.0 g=20.0 -> @@0",
        "@@2: line 1.2 t=144 s=-4.0 g=8.0 -> @@0",
        "@@4: line 2.2- t=148 s=31.0 a=-1.0 -> @@2",
    ] {
        assert!(
            trace.contains(expected),
            "missing {expected:?} from {trace:?}"
        );
    }
    });
}
#[test]
fn text_material_preserves_ligature_space_factor_and_font_glue() {
    // TeX82 §§1033--1042: the pending character run applies the font's
    // ligature program before the following space is selected and scaled by
    // the live space factor.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_cmr10_as(&mut control, stores, "cmr10.tfm");
        register_source(
            &mut control,
            br"\font\f=cmr10 \f
           \setbox0=\hbox{A fi B}
           \setbox1=\hbox{A\spacefactor=3000\relax{} X}\end",
        );

        run_to_end(&mut control, stores);

        let ordinary = box_child_nodes(stores, 0);
        assert!(matches!(
            ordinary.as_slice(),
            [
                Node::Char { ch: 'A', .. },
                Node::Glue { .. },
                Node::Lig { orig, .. },
                Node::Glue { .. },
                Node::Char { ch: 'B', .. },
            ] if orig.as_slice() == ['f', 'i']
        ));
        let sentence = box_child_nodes(stores, 1);
        let [
            Node::Char { ch: 'A', .. },
            Node::Glue { spec, .. },
            Node::Char { ch: 'X', .. },
        ] = sentence.as_slice()
        else {
            panic!("sentence-space fixture has character/glue/character: {sentence:?}");
        };
        let sentence = spec;
        assert_eq!(sentence.width.raw(), 291_271);
        assert_eq!(sentence.stretch.raw(), 327_678);
        assert_eq!(sentence.shrink.raw(), 24_272);
    });
}
#[test]
fn paragraph_boundaries_run_everypar_in_outer_and_internal_vertical_modes() {
    // TeX82 §§1088--1096: both outer and internal vertical paragraph entry
    // run `everypar`, and both completed paragraphs return to their enclosing
    // vertical mode without losing the body material.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\everypar{\global\advance\count0 by1}
           \noindent\kern1pt\par
           \setbox0=\vbox{\noindent\kern2pt\par}\end",
        );

        run_to_end(&mut control, stores);

        assert_eq!(stores.count(0).expect("count register"), 2);
        assert_eq!(control.current_mode(), Mode::Vertical);
        assert!(stores.copy_box_to_page(0).is_some());
        assert_eq!(stores.world().committed_artifacts().len(), 1);
    });
}
#[test]
fn empty_equation_number_checks_math_fonts_on_both_sides() {
    // TeX82 §1194 checks the equation-number mlist and then the saved display
    // mlist independently, even though neither one contains a math noad.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = etex_initex(stores);
        control.set_fuel_limit(10_000).expect("bounded fuel");
        register_source(
            &mut control,
            br"\tracingrestores=1\tracingonline=1$$\eqno^{}$\end",
        );

        run_to_end(&mut control, stores);

        let terminal = terminal_text(stores);
        assert_eq!(
            terminal
                .matches("Math formula deleted: Insufficient symbol fonts")
                .count(),
            2
        );
        let first_font_error = terminal
            .find("Math formula deleted: Insufficient symbol fonts")
            .expect("equation-number font error");
        let display_end_error = terminal
            .find("Display math should end with $$")
            .expect("unpaired display end error");
        let second_font_error = terminal
            .rfind("Math formula deleted: Insufficient symbol fonts")
            .expect("display font error");
        let equation_number_restore = terminal
            .find("{restoring \\fam=-1}")
            .expect("equation-number family restore");
        assert!(first_font_error < display_end_error);
        assert!(display_end_error < equation_number_restore);
        assert!(equation_number_restore < second_font_error);
        assert!(terminal.contains("{restoring \\predisplaydirection=0}"));
    });
}
#[test]
fn paragraph_with_later_macro_tokens_is_permanently_skipped() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_cmr10_as(&mut control, stores, "cmr10.tfm");
        register_source(&mut control, br"\def\finish{A\par\count0=7}\finish\end");
        let mut ledger = crate::OutputLedger::new();
        let mut checkpoints = Vec::new();
        let cancellation = crate::Cancellation::new();

        for _ in 0..32 {
            if matches!(
                crate::CanonicalStepRunner::new(&mut control, stores, &mut ledger)
                    .step(&mut checkpoints, &cancellation),
                crate::CanonicalStepResult::Completed(_)
            ) {
                break;
            }
        }
        assert_eq!(stores.count(0).expect("count register"), 7);
        assert!(checkpoints.is_empty());
    });
}
#[test]
fn vertical_unbox_in_horizontal_mode_ends_the_paragraph_before_splicing() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\setbox0=\vbox{\hbox{\kern1pt}}\setbox1=\vbox{\noindent\kern2pt\unvbox0}",
        );
        run_to_end(&mut control, stores);

        let box1 = stores.copy_box_to_page(1).expect("outer vbox exists");
        let box1_nodes = page_vec(stores, box1);
        let [tex_state::node::Node::VList(outer)] = box1_nodes.as_slice() else {
            panic!("register 1 should hold a vbox");
        };
        let children = page_vec(stores, outer.children);
        assert!(
            children
                .iter()
                .filter(|node| matches!(node, tex_state::node::Node::HList(_)))
                .count()
                >= 2,
            "the paragraph line and unboxed vertical child remain sibling vlist nodes"
        );
        assert!(
            stores.copy_box_to_page(0).is_none(),
            "the retried unvbox is destructive"
        );
    });
}
#[test]
fn etex_lastnodetype_code_seven_after_unboxing_ligature() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        tex_command::install_tex82_expandable_primitives(stores);
        tex_command::install_etex_expandable_primitives(stores);
        crate::install_unexpandable_primitives(stores);
        crate::install_etex_unexpandable_primitives(stores);
        let mut control = MainControl::prepared_initex(CommandProfile::ETEX26);
        register_cmr10_as(&mut control, stores, "cmr10.tfm");
        register_source(
        &mut control,
        br"\font\f=cmr10 \f\hbox{\setbox0=\hbox{ff}\unhbox0\xdef\result{\the\lastnodetype}}\end",
    );
        run_to_end(&mut control, stores);
        assert_eq!(macro_character_text(stores, "result"), "7");
    });
}
#[test]
fn etex_vertical_box_normal_paragraph_observes_interline_penalty_reset() {
    // e-TeX 2.6 [47.1070] extends TeX82 §1070's `normal_paragraph` to clear
    // the interline-penalty array. TeX82 §§1070/1085 invoke it for vertical
    // boxes, while an hbox must leave the array alone.
    for (box_command, expected_mutations) in [("vbox", 2), ("vtop", 2), ("hbox", 1)] {
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            let mut control = etex_initex(stores);
            let source = format!(r"\interlinepenalties=1 10 \setbox0=\{box_command}{{}} \end");
            register_source(&mut control, source.as_bytes());
            let mut observations = ObservationRecorder::default();
            run_to_end_observed(&mut control, stores, &mut observations);

            let mutations = observations
                .0
                .iter()
                .filter(|observation| {
                    matches!(
                        observation,
                        CommandObservation::Mutation(record)
                            if record.target == MutationTarget::Register
                                && observation_name(&record.key) == Some("toks:256")
                                && observation_tokens(&record.value) == Some([].as_slice())
                                && !record.global
                    )
                })
                .count();
            assert_eq!(mutations, expected_mutations, "\\{box_command}");
        });
    }
}
#[test]
fn display_content_preserves_future_multiple_leading_newlines() {
    // The structured scanner never produces this malformed/future content.
    // If that contract expands, replay must still pass the content verbatim
    // to §62 rather than broadly deleting payload newlines.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut effects = DiagnosticEffects::new();
        admitted!(stores, |context| {
            context.printer().print("closed").print_ln();
            print_display_content(context, &mut effects, "\n\nfuture");
        });
        stores.world_mut().publish_diagnostic_effects(effects);

        assert_eq!(pending_sink_text(stores, true), "closed\n\n\nfuture");
        assert_eq!(pending_sink_text(stores, false), "closed\n\n\nfuture");
    });
}
#[test]
fn language_normalization_and_same_language_append_boundaries_match_tex82() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
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
        run_to_end(&mut control, stores);
        assert_eq!(
            language_whatsits(stores),
            vec![(7, 2, 63), (7, 2, 63), (255, 2, 63), (0, 2, 63), (0, 2, 63)]
        );
    });
}
#[test]
fn paragraph_entry_snapshots_language_before_first_character() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        // TeX82 §1091 runs `set_cur_lang; clang:=cur_lang` on each `new_graf`.
        // Thus §1376 appends one language whatsit when the first paragraph changes
        // 7 -> 0 before its first character, while the second paragraph's
        // unchanged 0 -> 0 state is the negative control.
        register_source(
            &mut control,
            br"\language=7 \lefthyphenmin=2 \righthyphenmin=3
           \setbox0=\vbox{\noindent\language=0 a\hskip1pt\par
                           \noindent a\hskip1pt\par}\end",
        );
        run_to_end(&mut control, stores);

        let outer = stores
            .copy_box_to_page(0)
            .expect("box 0 holds the paragraph vbox");
        let Some(Node::VList(vbox)) = first_published_node(stores, outer) else {
            panic!("box 0 holds a vlist");
        };
        let lines = page_vec(stores, vbox.children)
            .iter()
            .filter_map(|node| match node {
                Node::HList(line) => Some(*line),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(lines.len(), 2);
        let languages = lines
            .iter()
            .map(|line| {
                page_vec(stores, line.children)
                    .iter()
                    .filter_map(|node| match node {
                        Node::Whatsit(tex_state::node::Whatsit::Language {
                            language,
                            left_hyphen_min,
                            right_hyphen_min,
                        }) => Some((*language, *left_hyphen_min, *right_hyphen_min)),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        assert_eq!(languages, [vec![(0, 2, 3)], vec![]]);
    });
}
#[test]
fn opentype_only_math_family_rejection_precedes_state_mutation() {
    let key = tex_fonts::FontRequestKey::new(
        "cmu-serif-roman",
        0,
        tex_fonts::VariationSelection::default(),
        tex_fonts::FontFeaturePolicy::default(),
    )
    .expect("OpenType request key");
    let request = tex_fonts::FontRequest {
        key: key.clone(),
        accepted_containers: tex_fonts::AcceptedFontContainers::WASM,
        purposes: tex_fonts::FontPurposes::LAYOUT_AND_HTML,
    };
    let bytes = include_bytes!("../../../../umber-wasm/assets/cmu-serif-500-roman.woff2").to_vec();
    let font = tex_fonts::OpenTypeFont::parse(
        &request,
        tex_fonts::ResolvedFont {
            request: key,
            container: tex_fonts::FontContainer::Woff2,
            bytes,
            declared_object_ahash64: None,
            declared_program_identity: None,
            provenance: None,
            legacy_mapping: None,
        },
        tex_fonts::FontLimits::default(),
    )
    .expect("OpenType fixture parses");
    let selection = font;
    let size = Scaled::from_raw(10 * Scaled::UNITY);
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let unsupported = admitted!(stores, |context| context.intern_font(
            tex_fonts::LoadedFont::new_opentype(
                "cmu-serif-roman",
                "cmu-serif-roman",
                size,
                size,
                selection,
            ),
        ));
        let family_before = admitted!(stores, |context| context
            .math_family_font(MathFontSize::Text, 0));
        let state_before = stores.journal_cursor().expect("state cursor");

        let error = admitted!(stores, |context| assign_math_family_font(
            context,
            MathFontSize::Text,
            0,
            unsupported,
            true,
        ))
        .expect_err("OpenType-only font cannot enter a classic math family");

        assert!(matches!(error, ExecError::OpenTypeMathUnsupported));
        assert_eq!(
            admitted!(stores, |context| context
                .math_family_font(MathFontSize::Text, 0)),
            family_before
        );
        assert_eq!(stores.journal_cursor().expect("state cursor"), state_before);
        admitted!(stores, |context| assign_math_family_font(
            context,
            MathFontSize::Text,
            0,
            tex_state::font::NULL_FONT,
            true,
        ))
        .expect("classic nullfont remains assignable");
    });
}
#[test]
fn vertical_par_resets_normal_paragraph_parameters_without_material() {
    with_etex(
        br"\parshape=1 3pt 40pt\hangindent=5pt\hangafter=2\looseness=2\par",
        |stores| {
            assert_eq!(
                stores
                    .dimen_param(DimenParam::HANG_INDENT)
                    .expect("dimension parameter")
                    .raw(),
                0
            );
            assert_eq!(stores.int_param(IntParam::HANG_AFTER), 1);
            assert_eq!(stores.int_param(IntParam::LOOSENESS), 0);
            assert!(admitted!(stores, |context| context.paragraph_shape()).is_empty());
            assert!(
                admitted!(stores, |context| context
                    .current_page_nodes()
                    .cloned()
                    .collect::<Vec<_>>())
                .is_empty()
            );
            assert!(admitted!(stores, |context| context
                .page_contributions()
                .is_empty()));
        },
    );
}
