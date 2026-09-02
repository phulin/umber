use std::sync::Arc;

use tex_command::{
    CommandDeliveryBoundary, CommandObservation, CommandObserver, InputReason, InputTransition,
    ObservedToken, RecoveryKind, RegisteredSourceKind, SourceRegistration,
};
use tex_state::hyphenation::PatternSpec;
use tex_state::page::PageMark;
use tex_state::token::{Catcode, Token};

use super::*;
mod etex_diagnostic_tracing;
#[path = "tests/material_property_matrices.rs"]
mod material_property_matrices;
#[path = "tests/tex82_whatsit_evidence.rs"]
mod tex82_whatsit_evidence;
#[path = "tests/tracked_region_coverage.rs"]
mod tracked_region_coverage;

/// Evaluates one short test projection inside an admitted command episode.
///
/// Keeping admission visible at each call site prevents branded state values
/// from being mistaken for detached test results.
macro_rules! admitted {
    ($stores:expr, |$context:ident| $projection:expr) => {
        crate::test_harness::with_admitted($stores, |$context| $projection)
    };
}

fn assign_static_meaning<G>(
    stores: &mut Universe<G>,
    symbol: tex_state::interner::SymbolId,
    meaning: Meaning,
) {
    admitted!(stores, |context| context
        .assign_resolved_meaning(
            symbol.symbol(),
            tex_state::ResolvedMeaning::Static(meaning),
            tex_state::AssignmentScope::Global,
        )
        .expect("static test meaning assignment"));
}

fn allocate_tokens<G>(stores: &mut Universe<G>, tokens: &[Token]) -> tex_state::TokenListId<G> {
    let words = tokens
        .iter()
        .copied()
        .map(tex_state::token::TokenWord::pack)
        .collect::<Vec<_>>();
    stores
        .allocate_token_list(&words)
        .expect("test token-list allocation")
}

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

fn font_by_name<G>(stores: &mut Universe<G>, name: &str) -> FontId {
    admitted!(stores, |context| {
        let symbol = context.intern_control_sequence(name);
        match context.meaning(symbol) {
            ResolvedMeaning::Static(Meaning::Font(font)) => font,
            meaning => panic!("{name} has {meaning:?}"),
        }
    })
}

fn register_source<G>(control: &mut MainControl<G>, bytes: &[u8]) {
    control
        .register_root_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(bytes),
        ))
        .expect("root source registers and opens");
}

fn page_vec<G>(stores: &Universe<G>, root: tex_state::node_arena::PageListId) -> Vec<Node> {
    stores
        .page_node_list(root)
        .expect("test list belongs to the page arena")
        .nodes()
        .iter()
        .cloned()
        .collect()
}

fn mode_vec<G>(control: &MainControl<G>, stores: &mut Universe<G>) -> Vec<Node> {
    admitted!(stores, |context| control
        .modes
        .current_list()
        .nodes(context)
        .iter()
        .cloned()
        .collect())
}

fn current_list_owner_vec<G>(control: &MainControl<G>, stores: &mut Universe<G>) -> Vec<Node> {
    if crate::vertical::is_outer_vertical(&control.modes) {
        admitted!(stores, |context| context.page_contributions().to_vec())
    } else {
        mode_vec(control, stores)
    }
}

#[test]
fn private_box_construction_retains_only_committed_lists() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\setbox0=\hbox{\kern1pt}");

        run_to_end(&mut control, stores);

        let boxed = stores
            .copy_box_to_page(0)
            .expect("completed box is committed");
        let children = match stores
            .page_node_list(boxed)
            .expect("copied box belongs to the page arena")
            .nodes()
            .first()
        {
            Some(tex_state::NodeView::HList(node)) => stores
                .page_node_list(node.children)
                .expect("hbox children belong to the page arena"),
            other => panic!("expected committed hbox, got {other:?}"),
        };
        assert!(matches!(
            children.nodes().first(),
            Some(tex_state::NodeView::Kern { .. })
        ));
    });
}

#[test]
fn repeated_setbox_regions_preserve_durable_aliases_and_publish_pages() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut source = br"\setbox0=\hbox{\kern1pt}\setbox1=\copy0".to_vec();
        for _ in 0..128 {
            source.extend_from_slice(br"\setbox0=\hbox{\kern2pt}");
        }
        source.extend_from_slice(br"\shipout\copy1\shipout\box0\end");

        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, &source);
        run_to_end(&mut control, stores);

        assert_eq!(stores.world().committed_artifacts().len(), 2);
        let lifecycle = stores.page_region_counters();
        assert_eq!(
            lifecycle.page_to_durable_nodes_copied, 0,
            "ordinary setbox construction transfers its closure"
        );
        assert_eq!(
            lifecycle.history_preservation_nodes_copied, 0,
            "a live command operation uses a rollbackable transfer loan"
        );
        assert!(
            lifecycle.tex_copy_nodes_copied > 0,
            "explicit TeX copy remains the one deep-copy seam"
        );
        let alias = stores
            .copy_box_to_page(1)
            .expect("overwriting box 0 preserves the copied durable alias");
        let alias = stores
            .page_node_list(alias)
            .expect("alias publishes back into the current page arena");
        assert!(matches!(
            alias.nodes().first(),
            Some(tex_state::NodeView::HList(_))
        ));
    });
}

#[test]
fn tracked_advance_records_command_and_execution_reads_after_commit() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\count0=17");

        let tracked = control
            .advance_with_tracked_region(stores)
            .expect("tracked operation executes");

        assert_eq!(
            tracked.step,
            StepResult::Progress(MainControlStep::Continue)
        );
        assert_eq!(
            stores.count(0).expect("count register"),
            17,
            "the TeX operation committed first"
        );
        let record = tracked
            .region
            .expect("committed operation finishes its region")
            .expect("ordinary assignment is supported");
        assert!(record.observations().iter().any(|observation| {
            observation.key == DependencyKey::Engine(DependencyEngineField::Mode)
        }));
        assert!(record.observations().iter().any(|observation| {
            observation.key == DependencyKey::Engine(DependencyEngineField::GroupType)
        }));
        assert!(!admitted!(stores, |context| context.tracked_region_is_active()));
    });
}

#[test]
fn tracked_advance_abandons_before_resource_suspension_rollback() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\font\missing=not-installed");

        let tracked = control
            .advance_with_tracked_region(stores)
            .expect("resource suspension is a step result");

        assert!(matches!(
            tracked.step,
            StepResult::Suspended(ResourceNeed::Font { .. })
        ));
        assert_eq!(tracked.region, None);
        assert!(!admitted!(stores, |context| context.tracked_region_is_active()));
    });
}

#[test]
fn math_choice_nested_font_definition_suspends_and_resumes_once() {
    // TeX82 §§1172/1174 executes each math-choice branch through ordinary
    // main control, and §1270 dispatches assignments in that nested episode.
    // A missing TFM is therefore a typed host suspension of the enclosing
    // operation, not a terminal execution error. The increment immediately
    // before the font definition proves rollback and retry do not duplicate
    // a nested side effect.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_cmr10_as(&mut control, stores, "cmr10.tfm");
        register_source(
            &mut control,
            br"\font\body=cmr10 \body $\mathchoice{\global\advance\count0 by1 \font\nested=cmti8 A}{B}{C}{D}$\global\count1=23\end",
        );

        let request = loop {
            match control.advance_episode(stores) {
                Ok(StepResult::Suspended(ResourceNeed::Font { request })) => break request,
                Ok(StepResult::Progress(_)) => {}
                other => panic!("unexpected nested font step: {other:?}"),
            }
        };
        assert_eq!(request.name, "cmti8");
        assert_eq!(stores.count(0).expect("count register"), 0);

        register_cmr10_as(&mut control, stores, "cmti8.tfm");
        run_to_end(&mut control, stores);

        assert_eq!(stores.count(0).expect("count register"), 1);
        assert_eq!(stores.count(1).expect("count register"), 23);
        assert!(control.pending_direct_operation.is_none());
        assert!(control.pending_resource_operation.is_none());
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
fn math_choice_nested_input_and_probe_resume_without_duplicate_effects() {
    let child = SourceRegistration::new(
        RegisteredSourceKind::Generated,
        Arc::<[u8]>::from(&br"\global\advance\count2 by1 \endinput"[..]),
    );
    for probe in [false, true] {
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            let mut control = pdftex_initex(stores);
            register_cmr10_as(&mut control, stores, "cmr10.tfm");
            let source = if probe {
                br"\font\body=cmr10 \body $\mathchoice{\global\advance\count0 by1 \openin0=child \ifeof0\fi A}{B}{C}{D}$\global\count1=23\end".as_slice()
            } else {
                br"\font\body=cmr10 \body $\mathchoice{\global\advance\count0 by1 \input child A}{B}{C}{D}$\global\count1=23\end".as_slice()
            };
            register_source(&mut control, source);

            let need = loop {
                match control
                    .advance_episode(stores)
                    .expect("nested resource step")
                {
                    StepResult::Suspended(need) => break need,
                    StepResult::Progress(_) => {}
                }
            };
            assert_eq!(stores.count(0).expect("count register"), 0);
            if probe {
                let ResourceNeed::InputProbe { request } = need else {
                    panic!("expected nested input probe, got {need:?}");
                };
                control.capabilities_mut().register_input_probe(
                    request.name,
                    tex_command::FileEnquiryResource::new(child.clone(), None),
                );
            } else {
                let ResourceNeed::Input { name, .. } = need else {
                    panic!("expected nested input request, got {need:?}");
                };
                control
                    .capabilities_mut()
                    .register_input(name, child.clone());
            }

            run_to_end(&mut control, stores);
            assert_eq!(stores.count(0).expect("count register"), 1);
            assert_eq!(stores.count(1).expect("count register"), 23);
            assert_eq!(
                stores.count(2).expect("count register"),
                usize::from(!probe) as i32
            );
        });
    }
}

#[test]
fn math_choice_nested_pdf_image_suspends_and_resumes_once() {
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

        let request = loop {
            match control.advance_episode(stores).expect("nested image step") {
                StepResult::Suspended(ResourceNeed::PdfImage { request }) => break request,
                StepResult::Suspended(need) => panic!("unexpected nested resource: {need:?}"),
                StepResult::Progress(_) => {}
            }
        };
        assert_eq!(stores.count(0).expect("count register"), 0);
        control.capabilities_mut().register_pdf_image(
            request,
            PdfImageResource::Available(test_pdf_image_source()),
        );

        run_to_end(&mut control, stores);
        assert_eq!(stores.count(0).expect("count register"), 1);
        assert_eq!(stores.count(1).expect("count register"), 23);
        assert_ne!(
            admitted!(stores, |context| context
                .internal_integer(tex_state::meaning::InternalInteger::PdfLastXImage)
                .expect("last image integer")),
            0
        );
    });
}

#[test]
fn tracked_group_exit_fails_closed_at_the_journal_timeline_barrier() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\begingroup\endgroup");

        let entered = control
            .advance_with_tracked_region(stores)
            .expect("group entry executes");
        assert!(
            matches!(entered.region, Some(Ok(_))),
            "group entry result: {:?}",
            entered.region
        );
        let exited = control
            .advance_with_tracked_region(stores)
            .expect("group exit executes");

        assert!(matches!(
            exited.region,
            Some(Err(DependencyRegionError::Unsupported(
                TrackedRegionBarrier::EnvironmentTimelineChange,
            )))
        ));
        assert!(!admitted!(stores, |context| context.tracked_region_is_active()));
    });
}

fn register_cmr10_as<G>(control: &mut MainControl<G>, stores: &mut Universe<G>, name: &str) {
    const CMR10: &[u8] = include_bytes!("../../../tex-fonts/tests/fixtures/cm/cmr10.tfm");
    stores
        .world_mut()
        .set_memory_file(name, CMR10.to_vec())
        .expect("font fixture installs");
    let metrics = InputReadState::read_input_file(
        &mut stores.input_open_context(),
        std::path::Path::new(name),
    )
    .expect("font fixture reads");
    control.capabilities_mut().register_font(
        name,
        FontResource::Tfm {
            metrics,
            opentype: None,
        },
    );
}

fn run_to_end<G>(control: &mut MainControl<G>, stores: &mut Universe<G>) {
    loop {
        match control.step(stores).expect("program executes") {
            MainControlStep::End | MainControlStep::EndOfInput => break,
            MainControlStep::Continue => {}
        }
    }
}

#[test]
fn box_save_stack_projection_distinguishes_scan_spec_callers() {
    // TeX82 §§645/1083's ordinary boxes preserve box_context plus the
    // packing pair. Section 1167's vcenter omits the context, and §1099's
    // insertion opens its group without calling scan_spec at all.
    for kind in [
        ReplayBoxKind::HBox,
        ReplayBoxKind::VBox,
        ReplayBoxKind::VTop,
    ] {
        assert_eq!(kind.save_stack_spec_words(), 3);
    }
    assert_eq!(ReplayBoxKind::VCenter.save_stack_spec_words(), 2);
    assert_eq!(ReplayBoxKind::Insert(7, false).save_stack_spec_words(), 0);
}

#[test]
fn save_stack_high_water_samples_each_checked_push_without_hardcoded_job_totals() {
    // TeX82 §§273/275--276 and §645: the high-water mark is the depth
    // immediately before a checked push, not the completed live depth. These
    // cases separate one/two-word restores, global no-save assignment,
    // command-owned aftergroup ordering, and executor-owned box specs.
    for (source, expected) in [
        (br"{}\end".as_slice(), 0),
        (br"{\global\count0=1}\end", 0),
        (br"{\count0=1}\end", 1),
        (br"{\def\fresh{}\count0=1}\end", 2),
        (br"{\count0=1\count1=1}\end", 3),
        (br"{\count0=1\aftergroup\relax\count1=1}\end", 4),
        (br"\setbox0=\hbox{}\end", 3),
    ] {
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            let mut control = MainControl::tex82_initex(stores);
            register_source(&mut control, source);
            run_to_end(&mut control, stores);
            assert_eq!(control.max_save_stack, expected, "source: {source:?}");
        });
    }
}

#[test]
fn outer_level_aftergroup_consumes_its_token_without_saving_or_replaying_it() {
    // TeX82 §280: `save_for_after` always consumes the following token, but
    // appends an `insert_token` save word only above `level_one`. The outer-
    // level case is shared unchanged by e-TeX 2.6 and pdfTeX 1.40.29.
    for profile in [
        CommandProfile::TEX82,
        CommandProfile::ETEX26,
        CommandProfile::PDFTEX14029,
    ] {
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            let mut control = match profile {
                CommandProfile::TEX82 => MainControl::tex82_initex(stores),
                CommandProfile::ETEX26 => etex_initex(stores),
                CommandProfile::PDFTEX14029 => pdftex_initex(stores),
                _ => unreachable!("test enumerates the three canonical profiles"),
            };
            register_source(
                &mut control,
                br"\def\unexpected{\global\count0=1}\aftergroup\unexpected\end",
            );
            let mut observations = ObservationRecorder::default();

            run_to_end_observed(&mut control, stores, &mut observations);

            assert_eq!(stores.count(0).expect("count register"), 0, "{profile:?}");
            assert_eq!(
                observations
                    .0
                    .iter()
                    .filter(|event| matches!(
                        event,
                        CommandObservation::Input(record)
                            if record.transition == InputTransition::Backup
                                && record.reason == InputReason::Backup
                    ))
                    .count(),
                0,
                "{profile:?}"
            );
        });
    }
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

fn box_child_nodes<G>(stores: &mut Universe<G>, register: u16) -> Vec<Node> {
    let list = stores
        .copy_box_to_page(register)
        .unwrap_or_else(|| panic!("box register {register} is nonvoid"));
    let boxed = page_vec(stores, list)
        .into_iter()
        .next()
        .unwrap_or_else(|| panic!("box register {register} has a root node"));
    let children = match boxed {
        Node::HList(boxed) | Node::VList(boxed) => boxed.children,
        other => panic!("box register {register} has a box root: {other:?}"),
    };
    page_vec(stores, children)
}

fn first_published_node<G>(
    stores: &Universe<G>,
    list: tex_state::node_arena::PageListId,
) -> Option<Node> {
    page_vec(stores, list).into_iter().next()
}

fn tabskip_widths<G>(stores: &Universe<G>, nodes: &[Node], widths: &mut Vec<i32>) {
    for node in nodes {
        match node {
            Node::Glue {
                spec,
                kind: GlueKind::TabSkip,
                ..
            } => widths.push(spec.width.raw()),
            Node::HList(boxed) | Node::VList(boxed) => {
                tabskip_widths(stores, &page_vec(stores, boxed.children), widths);
            }
            _ => {}
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AlignmentRuntimeSnapshot {
    alignment: AlignmentIdentity,
    column: usize,
    cell_span: u16,
    rows: usize,
    captured_cells: usize,
    row_mode: Mode,
    row_space_factor: i32,
    row_prev_depth: Option<i32>,
    cell_mode: Mode,
    cell_space_factor: i32,
    cell_prev_depth: Option<i32>,
}

fn active_alignment_runtime_snapshot<G>(
    control: &MainControl<G>,
) -> Option<AlignmentRuntimeSnapshot> {
    let active = control.active_alignment.as_ref()?;
    if !active.row_open || !active.cell_open {
        return None;
    }
    let summary = control.modes.summary();
    let levels = summary.levels();
    let [.., row, cell] = levels else {
        return None;
    };
    Some(AlignmentRuntimeSnapshot {
        alignment: active.identity,
        column: active.column,
        cell_span: active.cell_span,
        rows: active.captured_row_count,
        captured_cells: active.captured_cell_count,
        row_mode: row.mode(),
        row_space_factor: row.list().raw_space_factor(),
        row_prev_depth: row.list().prev_depth().map(Scaled::raw),
        cell_mode: cell.mode(),
        cell_space_factor: cell.list().raw_space_factor(),
        cell_prev_depth: cell.list().prev_depth().map(Scaled::raw),
    })
}

fn step_until_alignment_snapshot<G>(
    control: &mut MainControl<G>,
    stores: &mut Universe<G>,
    observations: &mut dyn CommandObserver,
    accept: impl Fn(AlignmentRuntimeSnapshot) -> bool,
) -> AlignmentRuntimeSnapshot {
    loop {
        match control
            .step_with_observer(stores, observations)
            .expect("program executes")
        {
            MainControlStep::End | MainControlStep::EndOfInput => {
                panic!("input ended before the requested alignment state")
            }
            MainControlStep::Continue => {}
        }
        if let Some(snapshot) = active_alignment_runtime_snapshot(control)
            && accept(snapshot)
        {
            return snapshot;
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum AlignmentNodeProjection {
    TabSkip(i32),
    Cell { span_count: u16 },
    Box { shift: i32, kerns: Vec<i32> },
    Penalty(i32),
    AboveDisplay(i32),
    BelowDisplay(i32),
    Baseline(i32),
    Kern(i32),
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum PackagedRowItem {
    TabSkip(i32),
    HorizontalCell(Vec<i32>),
    VerticalCell(Vec<i32>),
}

fn packaged_row_projection<G>(stores: &Universe<G>, row: &Node) -> Vec<PackagedRowItem> {
    fn material_widths<G>(
        stores: &Universe<G>,
        nodes: &tex_state::node_arena::PageListId,
    ) -> Vec<i32> {
        let mut widths = Vec::new();
        for node in page_vec(stores, *nodes) {
            match node {
                Node::Kern { amount, .. } => widths.push(amount.raw()),
                Node::Glue {
                    spec,
                    kind: GlueKind::Normal,
                    ..
                } => widths.push(spec.width.raw()),
                Node::HList(boxed) | Node::VList(boxed) => {
                    widths.extend(material_widths(stores, &boxed.children));
                }
                _ => {}
            }
        }
        widths
    }

    let children = match row {
        Node::HList(boxed) | Node::VList(boxed) => page_vec(stores, boxed.children),
        other => panic!("alignment outcome is a packaged row: {other:?}"),
    };
    children
        .iter()
        .filter_map(|node| match node {
            Node::Glue {
                spec,
                kind: GlueKind::TabSkip,
                ..
            } => Some(PackagedRowItem::TabSkip(spec.width.raw())),
            Node::HList(boxed) => Some(PackagedRowItem::HorizontalCell(material_widths(
                stores,
                &boxed.children,
            ))),
            Node::VList(boxed) => Some(PackagedRowItem::VerticalCell(material_widths(
                stores,
                &boxed.children,
            ))),
            _ => None,
        })
        .collect()
}

fn alignment_node_projection<G>(
    stores: &Universe<G>,
    nodes: &[Node],
) -> Vec<AlignmentNodeProjection> {
    fn kerns<G>(stores: &Universe<G>, nodes: tex_state::node_arena::PageListId) -> Vec<i32> {
        let mut out = Vec::new();
        for node in page_vec(stores, nodes) {
            match node {
                Node::Kern { amount, .. } => out.push(amount.raw()),
                Node::HList(boxed) | Node::VList(boxed) => {
                    out.extend(kerns(stores, boxed.children));
                }
                _ => {}
            }
        }
        out
    }

    nodes
        .iter()
        .filter_map(|node| match node {
            Node::Glue {
                spec,
                kind: GlueKind::TabSkip,
                ..
            } => Some(AlignmentNodeProjection::TabSkip(spec.width.raw())),
            Node::Glue {
                spec,
                kind: GlueKind::AboveDisplaySkip,
                ..
            } => Some(AlignmentNodeProjection::AboveDisplay(spec.width.raw())),
            Node::Glue {
                spec,
                kind: GlueKind::BelowDisplaySkip,
                ..
            } => Some(AlignmentNodeProjection::BelowDisplay(spec.width.raw())),
            Node::Glue {
                spec,
                kind: GlueKind::BaselineSkip,
                ..
            } => Some(AlignmentNodeProjection::Baseline(spec.width.raw())),
            Node::Unset(unset) => Some(AlignmentNodeProjection::Cell {
                span_count: unset.span_count,
            }),
            Node::HList(boxed) | Node::VList(boxed) => Some(AlignmentNodeProjection::Box {
                shift: boxed.shift.raw(),
                kerns: kerns(stores, boxed.children),
            }),
            Node::Penalty(value) => Some(AlignmentNodeProjection::Penalty(*value)),
            Node::Kern { amount, .. } => Some(AlignmentNodeProjection::Kern(amount.raw())),
            _ => None,
        })
        .collect()
}

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
fn accent_assignment_dispatches_backed_up_font_without_redelivery() {
    // TeX82 §§1123--1124 and 1270: the accent-code scan backs up its
    // non-space terminator, then `do_assignments` executes that already
    // expanded current command in place. Its backup level retires before the
    // following base character is fetched; the assignment must not synthesize
    // a second expanded delivery for the same command.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_cmr10_as(&mut control, stores, "cmr10.tfm");
        register_source(
            &mut control,
            br#"\font\f=cmr10 \font\accentfont=cmr10 \f\accent"7F\accentfont o\end"#,
        );
        let mut observations = ObservationRecorder::default();
        run_to_end_observed(&mut control, stores, &mut observations);

        let deliveries: Vec<_> = observations
            .0
            .iter()
            .enumerate()
            .filter_map(|(index, observation)| match observation {
                CommandObservation::Command(record)
                    if record.spelling == ObservedToken::ControlSequence("accentfont".into())
                        && record.command == "set_font" =>
                {
                    Some((index, record.boundary))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            deliveries
                .iter()
                .map(|(_, boundary)| *boundary)
                .collect::<Vec<_>>(),
            [
                CommandDeliveryBoundary::Raw,
                CommandDeliveryBoundary::Expanded,
                CommandDeliveryBoundary::Raw,
                CommandDeliveryBoundary::Expanded,
            ]
        );
        let backup = observations.0[deliveries[1].0 + 1..deliveries[2].0]
            .iter()
            .any(|observation| {
                matches!(
                    observation,
                    CommandObservation::Input(record)
                        if record.reason == InputReason::Backup
                            && record.transition == InputTransition::Backup
                )
            });
        assert!(backup, "the integer terminator is backed up before §1270");
        let retirement = observations.0[deliveries[3].0 + 1..]
            .iter()
            .position(|observation| {
                matches!(
                    observation,
                    CommandObservation::Input(record)
                        if record.reason == InputReason::Backup
                            && record.transition == InputTransition::Retire
                )
            });
        assert_eq!(retirement, Some(0), "the backup retires next");
    });
}

#[test]
fn ordinary_font_selection_keeps_its_expanded_delivery() {
    // Negative control: only §1270's already-settled handoff suppresses a
    // duplicate observation. An ordinary §1030 `big_switch` font command is
    // still one raw plus one expanded delivery.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\nullfont\end");
        let mut observations = ObservationRecorder::default();
        run_to_end_observed(&mut control, stores, &mut observations);

        let deliveries: Vec<_> = observations
            .0
            .iter()
            .filter_map(|observation| match observation {
                CommandObservation::Command(record)
                    if record.spelling == ObservedToken::ControlSequence("nullfont".into())
                        && record.command == "set_font" =>
                {
                    Some(record.boundary)
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            deliveries,
            [
                CommandDeliveryBoundary::Raw,
                CommandDeliveryBoundary::Expanded,
            ]
        );
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
fn trip_valign_row_uses_raw_main_loop_lookahead_before_assignment() {
    // TeX82 §§785/1034/1038: an alignment cell body is ordinary main
    // control. Once `7` enters `main_loop`, adjacent `A` is fetched by bare
    // `get_next`; only the following assignment returns to `x_token`.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_cmr10_as(&mut control, stores, "cmr10.tfm");
        register_source(
            &mut control,
            // Reduced from trip.tex:76--77. Keep the negative glue scan before
            // the adjacent characters: it proves that the first `7` is a fresh
            // §1030 entry and only `A` comes from §1038's raw lookahead.
            br"\font\f=cmr10 \f\setbox0=\hbox{\valign{#\cr \hskip-9pt7A\righthyphenmin0\cr}}\end",
        );
        let mut observations = ObservationRecorder::default();
        loop {
            match control
                .step_with_observer(stores, &mut observations)
                .expect("source-fed valign executes")
            {
                MainControlStep::End | MainControlStep::EndOfInput => break,
                MainControlStep::Continue => {}
            }
        }

        let deliveries: Vec<_> = observations
            .0
            .iter()
            .filter_map(|observation| match observation {
                CommandObservation::Command(record)
                    if record.command == "other_char" && record.command_operand == Some(55) =>
                {
                    Some((record.boundary, "7"))
                }
                CommandObservation::Command(record)
                    if record.command == "letter" && record.command_operand == Some(65) =>
                {
                    Some((record.boundary, "A"))
                }
                CommandObservation::Command(record)
                    if record.command == "assign_int"
                        && record.spelling
                            == tex_command::ObservedToken::ControlSequence(
                                "righthyphenmin".into(),
                            ) =>
                {
                    Some((record.boundary, "righthyphenmin"))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            deliveries,
            [
                (tex_command::CommandDeliveryBoundary::Raw, "7"),
                (tex_command::CommandDeliveryBoundary::Expanded, "7"),
                (tex_command::CommandDeliveryBoundary::Raw, "7"),
                (tex_command::CommandDeliveryBoundary::Expanded, "7"),
                (tex_command::CommandDeliveryBoundary::Raw, "7"),
                (tex_command::CommandDeliveryBoundary::Expanded, "7"),
                (tex_command::CommandDeliveryBoundary::Raw, "7"),
                (tex_command::CommandDeliveryBoundary::Expanded, "7"),
                (tex_command::CommandDeliveryBoundary::Raw, "A"),
                (tex_command::CommandDeliveryBoundary::Raw, "righthyphenmin"),
                (
                    tex_command::CommandDeliveryBoundary::Expanded,
                    "righthyphenmin"
                ),
            ]
        );
        assert_eq!(stores.int_param(IntParam::RIGHT_HYPHEN_MIN), 0);
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
fn named_output_token_list_trace_uses_live_escape_character() {
    // TeX82 §§63/323: `begin_token_list(output_routine,output_text)` names
    // `output` through `print_esc`, so an out-of-range escape character emits
    // no prefix.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        stores.set_interaction_mode(tex_state::InteractionMode::Nonstop);
        let mut control = MainControl::tex82_initex(stores);
        register_source(
        &mut control,
        b"\\tracingmacros=2\\tracingonline=1\\maxdeadcycles=1\\output={\\dimen0=1pt}\\escapechar=256\\topskip=0pt\\setbox0=\\vbox to1pt{}\\copy0\\penalty-10000\\end",
    );

        run_to_end(&mut control, stores);

        let terminal = terminal_text(stores);
        assert!(terminal.contains("output->{dimen 0=1pt}"), "{terminal:?}");
        assert!(!terminal.contains("\\output->"), "{terminal:?}");
    });
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
fn consuming_current_group_box_preserves_original_void_restore() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            b"\\tracingrestores=1\\tracingonline=1{\\setbox2=\\hbox to2pt{}\\setbox3=\\box2}\\end",
        );

        run_to_end(&mut control, stores);

        assert_eq!(
            pending_sink_text(stores, true),
            "{restoring \\box3=void}\n{restoring \\box2=void}\n"
        );
        assert!(stores.copy_box_to_page(2).is_none());
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
fn etex_sparse_box_restore_reports_value_before_first_local_assignment() {
    // e-TeX [47.1077] sends box registers above 255 through [53a]'s
    // `sa_def_box`; repeated local assignments save only the original value,
    // and `sa_restore` displays that value when the group ends.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = etex_initex(stores);
        register_source(
        &mut control,
        b"\\tracingrestores=1\\tracingonline=1{\\setbox32106=\\vbox{}\\setbox32106=\\hbox{X}}\\end",
    );

        run_to_end(&mut control, stores);

        assert_eq!(
            pending_sink_text(stores, true),
            "{restoring \\box32106=void}\n"
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
fn etex_identical_sparse_pointer_assignments_do_not_create_restore_entries() {
    // e-TeX 2.6 [53a] `sa_def` reports an identical pointer as
    // `reassigning`, destroys the scanned reference, and never calls
    // `sa_save`. The sparse mutation remains observable, but §283 therefore
    // has no register entry to restore before the ordinary parameter entry.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = etex_initex(stores);
        register_source(
        &mut control,
        br"\tracingrestores=1\tracingonline=1{\tracingassigns=1\muskip2000=0mu\toks2000={}}\end",
    );

        run_to_end(&mut control, stores);

        assert_eq!(
            pending_sink_text(stores, true),
            concat!(
                "{into \\tracingassigns=1}\n",
                "{reassigning \\muskip2000=0.0mu}\n",
                "{reassigning \\toks2000=}\n",
                "{restoring \\tracingassigns=0}\n",
            )
        );
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
fn etex_direction_meanings_share_valigns_vertical_mode_paragraph_entry() {
    // TeX82 §1090 keys this transition by the `valign` command code, and
    // e-TeX 2.6 [53a.3826--3883] assigns that code to all four directions.
    for primitive in [
        UnexpandablePrimitive::VAlign,
        UnexpandablePrimitive::BeginL,
        UnexpandablePrimitive::EndL,
        UnexpandablePrimitive::BeginR,
        UnexpandablePrimitive::EndR,
    ] {
        assert!(starts_paragraph_in_vertical_mode::<()>(
            ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(primitive))
        ));
    }
}

#[test]
fn etex_everyeof_assignment_is_visible_to_scantokens_during_edef() {
    // e-TeX 2.6 etex.ch §24.362 inserts a non-null \everyeof token list
    // before retiring the pseudo-file, including while \edef is defining.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = etex_initex(stores);
        register_source(
            &mut control,
            br"\everyeof={\noexpand}\edef\x{\scantokens{\begingroup}\endgroup}\end",
        );
        let mut observations = ObservationRecorder::default();

        run_to_end_observed(&mut control, stores, &mut observations);

        assert!(
            admitted!(stores, |context| context
                .token_parameter(tex_state::env::banks::TokParam::EVERY_EOF)
                .expect("everyeof parameter"))
            .is_some(),
            "the source assignment must remain present"
        );
        assert!(observations.0.iter().any(|event| matches!(
            event,
            CommandObservation::Input(record)
                if record.transition == InputTransition::Push
                    && record.reason == InputReason::EveryEof
        )));
    });
}

#[test]
fn etex_scantokens_warns_for_box_group_before_following_conditional() {
    // e-TeX 2.6 [23.328]: each closer warns immediately before its own
    // `unsave`/conditional pop. The two lines of one scantokens source must
    // therefore report the hbox group before the enclosing ifcase.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = etex_initex(stores);
        register_source(
            &mut control,
            br"\let\egroup=}\tracingonline=1\tracingnesting=1
           \setbox0=\hbox{\ifcase0
           \scantokens{\egroup^^J\fi}
           \end",
        );

        run_to_end(&mut control, stores);

        let output = terminal_text(stores);
        let group = output
            .find("Warning: end of hbox group")
            .unwrap_or_else(|| panic!("box group warning is rendered: {output:?}"));
        let condition = output
            .find("Warning: end of \\ifcase")
            .unwrap_or_else(|| panic!("conditional warning is rendered: {output:?}"));
        assert!(group < condition, "{output:?}");
    });
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
            include_bytes!("../fixtures/etex-empty-botmark-fire-up.tex"),
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
fn meaning_mutation_value_projects_protected_macro_storage_marker() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let definition = stores
            .allocate_definition(&[], &[])
            .expect("empty protected macro definition");

        let value = admitted!(stores, |context| meaning_mutation_value(
            tex_state::ResolvedMeaning::Macro {
                definition,
                flags: MeaningFlags::PROTECTED,
            },
            context,
        ));

        assert_eq!(
            value,
            ObservationValue::Tokens(vec![
                tex_command::ObservedToken::Character {
                    character: '\u{1}',
                    catcode: Catcode::Comment,
                },
                tex_command::ObservedToken::MacroEndMatch,
            ])
        );
    });
}

#[test]
fn protected_macro_marker_observation_precedes_meaning_mutation() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = etex_initex(stores);
        register_source(&mut control, br"\protected\def\p{X}\end");
        let mut observations = ObservationRecorder::default();

        run_to_end_observed(&mut control, stores, &mut observations);

        let marker = observations
            .0
            .iter()
            .position(|observation| {
                matches!(
                    observation,
                    CommandObservation::TokenList(TokenListRecord {
                        transition: "complete",
                        purpose: "protected_macro",
                        tokens,
                    }) if tokens == &[
                        ObservedToken::MacroEndMatch,
                        ObservedToken::Character {
                            character: 'X',
                            catcode: Catcode::Letter,
                        },
                    ]
                )
            })
            .expect("protected marker transition is observed");
        let mutation = observations
            .0
            .iter()
            .position(|observation| {
                matches!(
                    observation,
                    CommandObservation::Mutation(MutationRecord {
                        target: MutationTarget::Meaning,
                        key: ObservationValue::Name(name),
                        ..
                    }) if name == "p"
                )
            })
            .expect("protected definition mutation is observed");
        assert_eq!(marker + 1, mutation, "{:?}", observations.0);
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
fn etex_unexpanded_input_survives_the_first_main_control_operation() {
    // e-TeX change file §27.465 implements `\unexpanded` as `the_toks` plus
    // `ins_list`. The inserted list remains input after its first
    // unexpandable command reaches main control, so later tokens must not
    // borrow the attempt arena retired with that command operation.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = etex_initex(stores);
        register_source(
            &mut control,
            br"\count255=0 \unexpanded{\relax\global\advance\count255 by1}\end",
        );

        run_to_end_observed(&mut control, stores, &mut ObservationRecorder::default());

        assert_eq!(
            stores.count(255).expect("count register"),
            1,
            "terminal: {}",
            terminal_text(stores)
        );
    });
}

#[test]
fn etex_optimized_aftergroup_links_tokens_onto_one_backup_level() {
    // TeX82 §§282/326 create one `backed_up` level per saved token. e-TeX
    // 2.6 etex.ch [15.282] instead applies `back_input` only once, then links
    // the remaining tokens onto that level. The TeX82 run is the negative
    // control for the same bounded source microfixture.
    for (profile, expected_backups) in [(CommandProfile::TEX82, 3), (CommandProfile::ETEX26, 1)] {
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            let mut control = if profile == CommandProfile::ETEX26 {
                etex_initex(stores)
            } else {
                MainControl::tex82_initex(stores)
            };
            register_source(
                &mut control,
                br"{\aftergroup\relax\aftergroup\relax\aftergroup\relax}\end",
            );
            let mut observations = ObservationRecorder::default();

            run_to_end_observed(&mut control, stores, &mut observations);

            let backups = observations
                .0
                .iter()
                .filter(|event| {
                    matches!(
                        event,
                        CommandObservation::Input(record)
                            if record.transition == InputTransition::Backup
                                && record.reason == InputReason::Backup
                    )
                })
                .count();
            let relax_deliveries = observations
                .0
                .iter()
                .filter(|event| {
                    matches!(
                        event,
                        CommandObservation::Command(command)
                        if command.boundary == tex_command::CommandDeliveryBoundary::Raw
                            && command.spelling
                                == tex_command::ObservedToken::ControlSequence("relax".into())
                    )
                })
                .count();
            assert_eq!(backups, expected_backups, "profile {profile:?}");
            assert_eq!(relax_deliveries, 6, "profile {profile:?}");
        });
    }
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
                control.step(stores).expect("prefix executes"),
                MainControlStep::Continue
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
fn discretionary_parts_execute_live_in_disc_group_without_duplicate_delivery() {
    // TeX82 §§1117/1120: each part returns to main control in restricted
    // horizontal mode under disc_group (e-TeX group code 10). Two macro
    // layers and a conditional make any fixed body-prefetch scheme invalid;
    // the literal `\kern` is the nonmacro negative control for duplicate
    // delivery.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        stores.install_primitive_meaning(
            "currentgrouptype",
            Meaning::InternalInteger(tex_state::meaning::InternalInteger::CurrentGroupType),
        );
        control.set_fuel_limit(10_000).expect("bounded fuel");
        register_source(
            &mut control,
            br"\def\layera{\layerb}
          \def\layerb{\ifnum\currentgrouptype=10
            \global\count0=10
          \else
            \global\count0=-1
          \fi}
          \discretionary{\layera\kern1pt}{}{}",
        );

        run_to_end(&mut control, stores);

        assert_eq!(
            stores.count(0).expect("count register"),
            10,
            "body expansion saw disc_group; terminal={}",
            terminal_text(stores)
        );
        let current_nodes = mode_vec(&control, stores);
        let disc = current_nodes
            .iter()
            .find_map(|node| match node {
                Node::Disc {
                    pre, post, replace, ..
                } => Some((*pre, *post, *replace)),
                _ => None,
            })
            .expect("completed discretionary node");
        assert_eq!(
            page_vec(stores, disc.0)
                .iter()
                .filter(|node| matches!(
                    node,
                    Node::Kern {
                        amount,
                        ..
                    } if *amount == Scaled::from_raw(Scaled::UNITY)
                ))
                .count(),
            1,
            "unexpandable body command executes exactly once"
        );
        assert!(disc.1.is_empty());
        assert!(disc.2.is_empty());
        assert_eq!(
            admitted!(stores, |context| context.innermost_group_kind()),
            None
        );
    });
}

#[test]
fn nested_discretionary_preserves_aftergroup_before_rejecting_the_outer_part() {
    // TeX82 §§282/1120–1121: unsave inserts aftergroup material before
    // build_discretionary scans the next part's left brace. Make that token
    // itself the opener; the literal brace that follows must therefore be an
    // ordinary nested group inside the second part. The inner discretionary
    // simultaneously proves that ActiveDiscretionary is a proper stack, then
    // §1121 rejects it as a forbidden node in the outer discretionary list.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        control.set_fuel_limit(10_000).expect("bounded fuel");
        register_source(
            &mut control,
            br"\let\opener={\noindent
          \discretionary{
            \discretionary{\kern1pt}{}{}
            \aftergroup\opener
          }{\kern2pt}}{\kern3pt}",
        );

        run_to_end(&mut control, stores);

        let current_nodes = mode_vec(&control, stores);
        let [
            Node::Disc {
                pre, post, replace, ..
            },
            ..,
        ] = current_nodes.as_slice()
        else {
            panic!(
                "the forbidden nested discretionary is pruned from the retained outer discretionary: {:?}",
                current_nodes
            );
        };
        assert!(
            pre.is_empty(),
            "the forbidden nested discretionary and its suffix were pruned"
        );
        assert!(matches!(
            page_vec(stores, *post).as_slice(),
            [Node::Kern { .. }]
        ));
        assert!(matches!(
            page_vec(stores, *replace).as_slice(),
            [Node::Kern { .. }]
        ));
        assert!(terminal_text(stores).contains("Improper discretionary list"));
        assert!(
            !terminal_text(stores).contains("Missing { inserted"),
            "aftergroup token supplied the next part opener"
        );
        assert!(control.active_discretionaries.is_empty());
        assert_eq!(
            admitted!(stores, |context| context.innermost_group_kind()),
            None
        );
    });
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
fn discretionary_nest_overflow_leaves_group_and_active_stack_untouched() {
    // TeX82 §216 rejects a semantic-nest push before saving any new level.
    // Fatal overflow is committed rather than rolled back, so the
    // discretionary opener must not install disc_group or its executor frame
    // until that bounded push has succeeded.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\noindent\discretionary{}{}{}");
        assert_eq!(
            control.step(stores).expect("paragraph starts"),
            MainControlStep::Continue
        );
        while control.modes.depth() < 41 {
            control
                .modes
                .push(Mode::RestrictedHorizontal)
                .expect("fill the TeX82 semantic nest");
        }

        assert_eq!(
            control.step(stores).expect("fatal overflow succumbs"),
            MainControlStep::End
        );
        assert_eq!(control.modes.depth(), 41);
        assert_eq!(
            admitted!(stores, |context| context.innermost_group_kind()),
            None
        );
        assert!(control.active_discretionaries.is_empty());
    });
}

#[test]
fn vtop_resets_inherited_parshape_before_display_line_measurement() {
    // TeX82 §§1051--1052 run `normal_paragraph` after opening a `\vtop`.
    // The display therefore uses the box-local 100pt hsize, not the inherited
    // 12pt second `\parshape` line. The empty display's centered reference
    // point therefore extends the vtop's exact natural width to 50pt.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        control.set_fuel_limit(10_000).expect("bounded fuel");
        register_source(
            &mut control,
            br"\nonstopmode
          \hsize=100pt
          \parshape=2 1pt 11pt 2pt 12pt
          \setbox0=\vtop{\noindent$$\kern5pt$$}
          \end",
        );

        run_to_end(&mut control, stores);

        let root = stores
            .copy_box_to_page(0)
            .expect("vtop is assigned to box 0");
        let Some(Node::VList(boxed)) = first_published_node(stores, root) else {
            panic!("box 0 holds a vlist");
        };
        assert_eq!(boxed.width.raw(), 3_276_800);
    });
}

#[test]
fn preamble_span_expands_one_token_and_preserves_later_template_meaning() {
    // TeX82 §759 expands exactly the token after each preamble `\span`.
    // Here \A is \relax while the preamble is scanned, then becomes a 3pt
    // kern before the spanned column template executes. The template must
    // retain \A itself and resolve its later meaning, producing exactly 3pt.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        control.set_fuel_limit(20_000).expect("bounded fuel");
        register_source(
            &mut control,
            br"\nonstopmode
          \let\A=\relax
          \setbox0=\vbox{\halign{#&\iftrue\A\span\else\span\fi\span&#\cr
            \def\A{\kern3pt}\span\relax&\relax\cr}}
          \end",
        );

        run_to_end(&mut control, stores);

        let root = stores.copy_box_to_page(0).expect("vbox is assigned");
        let Some(Node::VList(boxed)) = first_published_node(stores, root) else {
            panic!("box 0 holds a vlist");
        };
        assert_eq!(boxed.width.raw(), 3 * Scaled::UNITY);
    });
}

#[test]
fn span_delimiter_ends_the_pending_ligkern_run() {
    // TeX82 §§1034--1036 finish a character word when the alignment
    // delimiter interrupts `main_loop`. Although §791 keeps a spanned cell's
    // list open, the characters on opposite sides of `\span` are therefore
    // distinct lig/kern runs. CMR10 kerns `bc` by 0.27779pt, so this fixture
    // detects an accidental run carried across either span boundary.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        control.set_fuel_limit(20_000).expect("bounded fuel");
        register_cmr10_as(&mut control, stores, "cmr10.tfm");
        register_source(
            &mut control,
            br"\font\f=cmr10 \f
          \setbox0=\vbox{\halign{<#>&[#]&( # )\cr
            \omit a\span\omit b\span\omit c\cr}}
          \end",
        );

        run_to_end(&mut control, stores);

        let root = stores.copy_box_to_page(0).expect("vbox is assigned");
        let Some(Node::VList(boxed)) = first_published_node(stores, root) else {
            panic!("box 0 holds a vlist");
        };
        assert_eq!(boxed.width.raw(), 983_042, "natural width is 15.00003pt");
        let children = box_child_nodes(stores, 0);
        assert_eq!(
            alignment_node_projection(stores, &children),
            vec![AlignmentNodeProjection::Box {
                shift: 0,
                kerns: Vec::new(),
            }],
        );
    });
}

#[test]
fn alignment_v_template_continues_the_pending_ligkern_run() {
    // TeX82 §§1034--1038: `main_loop_lookahead` crosses the §342 alignment
    // interception into the v-template. CMR10's `fi` ligature therefore
    // combines a final body character with the template's first character.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        control.set_fuel_limit(20_000).expect("bounded fuel");
        register_cmr10_as(&mut control, stores, "cmr10.tfm");
        register_source(
            &mut control,
            br"\font\f=cmr10 \f
          \setbox0=\vbox{\halign{#i\cr f\cr}}
          \end",
        );

        run_to_end(&mut control, stores);

        fn collect_ligatures<G>(
            stores: &Universe<G>,
            root: tex_state::node_arena::PageListId,
            found: &mut Vec<Vec<char>>,
        ) {
            for node in stores
                .page_node_list(root)
                .expect("test list belongs to the page arena")
                .nodes()
            {
                match node {
                    tex_state::NodeView::Lig { orig, .. } => found.push(orig.to_vec()),
                    tex_state::NodeView::HList(boxed) | tex_state::NodeView::VList(boxed) => {
                        collect_ligatures(stores, boxed.children, found);
                    }
                    _ => {}
                }
            }
        }

        let root = stores.copy_box_to_page(0).expect("vbox is assigned");
        let mut ligatures = Vec::new();
        collect_ligatures(stores, root, &mut ligatures);
        assert_eq!(ligatures, [vec!['f', 'i']]);
    });
}

#[test]
fn alignment_macro_ending_in_parameter_marker_preserves_the_v_template_sink() {
    // TeX82 §§359 and 760: fetching a macro's final replacement token does
    // not retire its input level until the next demand. Both template sinks
    // therefore belong to the enclosing preamble scanner before expansion
    // starts; retiring the macro at the first v-template token must not
    // reclaim that parent-owned sink.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\def\parameter{##}\setbox0=\vbox{\halign{\span\parameter Z\cr A\cr}}\end",
        );

        run_to_end(&mut control, stores);

        assert!(stores.copy_box_to_page(0).is_some());
        assert!(
            terminal_text(stores).is_empty(),
            "{}",
            terminal_text(stores)
        );
    });
}

#[test]
fn alignment_end_template_replayed_as_macro_argument_is_a_valid_endv_shape() {
    // TeX82 §§325, 390, and 1131: a v-template ending in an undelimited macro
    // call can make the frozen end-template token that call's argument. When
    // it becomes `endv`, §1131 walks through the now-exhausted parameter and
    // macro-body token lists to the retained v-template below them.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\def\identity#1{#1}\setbox0=\vbox{\halign{#\noexpand\identity\cr A\cr}}\end",
        );

        run_to_end(&mut control, stores);

        assert!(stores.copy_box_to_page(0).is_some());
        assert!(
            terminal_text(stores).is_empty(),
            "{}",
            terminal_text(stores)
        );
    });
}

#[test]
fn nested_valign_rows_do_not_contribute_baseline_glue_to_outer_cell_width() {
    // TeX82 §799 appends a finished `\valign` row with a plain horizontal
    // splice. The two row widths therefore total exactly 5pt in the enclosing
    // `\halign` cell; routing them through §679 would insert 12pt baselineskip
    // and make the cell spuriously 17pt wide.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        control.set_fuel_limit(20_000).expect("bounded fuel");
        register_source(
            &mut control,
            br"\nonstopmode
          \setbox0=\vbox{\halign{#\cr
            \valign{#\cr\hbox{\kern2pt}\cr\hbox{\kern3pt}\cr}\cr}}
          \end",
        );

        run_to_end(&mut control, stores);

        let root = stores.copy_box_to_page(0).expect("outer vbox is assigned");
        let Some(Node::VList(boxed)) = first_published_node(stores, root) else {
            panic!("box 0 holds a vlist");
        };
        assert_eq!(boxed.width.raw(), 5 * Scaled::UNITY);
    });
}

#[test]
fn display_alignment_tail_runs_assignments_before_main_control() {
    // TeX82 §1206 runs §1270 `do_assignments` after `fin_align` and
    // before checking for the closing `$$`. Its §404 fetch suppresses the
    // separating blank, so the malformed postdisplaypenalty assignment must
    // diagnose before any later display-mode command trace.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        control.set_fuel_limit(20_000).expect("bounded fuel");
        register_source(
            &mut control,
            br"\nonstopmode\tracingcommands=1\tracingonline=1
              \noindent$$\halign{#\cr\cr} \global\postdisplaypenalty=*$$\end",
        );

        run_to_end(&mut control, stores);

        let terminal = terminal_text(stores);
        assert!(
            terminal.contains("Missing number, treated as zero"),
            "assignment reports its missing integer: {terminal}"
        );
        assert!(
            !terminal.contains("{display math mode: blank space}"),
            "the do_assignments blank must not reach main control: {terminal}"
        );
    });
}

#[test]
fn display_alignment_finish_replays_missing_double_math_shift_offender() {
    // TeX82 §§1206--1207: a command other than the required closing math
    // shift reports the display-math delimiter error, is backed up, and
    // executes once after the alignment has restored its enclosing mode.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\nonstopmode\noindent$$\halign{#\cr\cr}\global\count0=17\par\end",
        );

        run_to_end(&mut control, stores);

        let terminal = terminal_text(stores);
        assert_eq!(
            terminal.matches("Missing $$ inserted.").count(),
            1,
            "{terminal}"
        );
        assert_eq!(stores.count(0).expect("count register"), 17);
        assert_eq!(control.current_mode(), Mode::Vertical);
    });
}

#[test]
fn align_peek_full_branch_prefix_recovery_and_nesting_matrix() {
    // TeX82 §785: the expanded row probe owns blanks/macros, repeated
    // `\crcr`, `\noalign` (including its recovered opener), the closing
    // right brace, and the backed-up first command of an ordinary row.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        control.set_fuel_limit(30_000).expect("bounded fuel");
        register_source(
            &mut control,
            br"\nonstopmode
          \def\empty{}\count0=0\count1=0\count2=0
          \setbox0=\vbox{\halign{\global\advance\count1 by1 #\cr
            \empty \global\advance\count0 by1\cr
            \crcr\crcr
            \noalign{\global\advance\count2 by1}
            \empty \global\advance\count0 by1\cr}}
          \setbox1=\vbox{\halign{#\cr\cr\noalign
            \global\advance\count2 by1}\crcr}}
          \setbox2=\vbox{\halign{#\cr
            \omit\vbox{\halign{#\cr\cr}}\cr}}
          \setbox3=\vbox{\halign{#\cr}}
          \end",
        );
        let mut observations = ObservationRecorder::default();

        run_to_end_observed(&mut control, stores, &mut observations);

        assert_eq!(
            stores.count(0).expect("count register"),
            2,
            "each ordinary row opener executes once"
        );
        assert_eq!(
            stores.count(1).expect("count register"),
            2,
            "the nonempty u-template runs once per row"
        );
        assert_eq!(
            stores.count(2).expect("count register"),
            2,
            "valid and recovered noalign bodies run once"
        );
        let terminal = terminal_text(stores);
        assert_eq!(
            terminal.matches("Missing { inserted").count(),
            1,
            "{terminal}"
        );
        assert!(!terminal.contains("Extra alignment tab"), "{terminal}");

        let transitions = observations
            .0
            .iter()
            .filter_map(|observation| match observation {
                CommandObservation::Alignment(record) => {
                    Some((record.transition, record.nesting, record.align_state))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            transitions
                .iter()
                .filter(|(transition, _, _)| *transition == "u_template_push")
                .count(),
            4,
            "only ordinary (not omit, crcr, noalign, or immediate-close) rows push u-templates"
        );
        assert!(
            transitions
                .iter()
                .any(|(transition, nesting, _)| *transition == "suspend" && *nesting == Some(1))
        );
        assert!(
            transitions
                .iter()
                .any(|(transition, nesting, _)| *transition == "begin" && *nesting == Some(2))
        );
        assert!(
            transitions
                .iter()
                .any(|(transition, nesting, _)| *transition == "resume" && *nesting == Some(1))
        );
        assert_eq!(control.current_mode(), Mode::Vertical);
        assert_eq!(control.advance_telemetry().maximum_live_savepoints, 0);

        // The direct-operation counters above cannot prove §785's ordering. Isolate an
        // ordinary row opener and project the command-owned reset, backup, and
        // u-template input events in the order they committed.
        crate::test_harness::with_nonstop_plain_universe(|ordered_stores| {
            let mut ordered = MainControl::tex82_initex(ordered_stores);
            register_source(&mut ordered, br"\setbox0=\vbox{\halign{#\cr x\cr}}\end");
            let mut ordered_observations = ObservationRecorder::default();
            run_to_end_observed(&mut ordered, ordered_stores, &mut ordered_observations);
            let reset = ordered_observations
                .0
                .iter()
                .position(|observation| {
                    matches!(
                        observation,
                        CommandObservation::Alignment(record)
                            if record.transition == "state_change"
                                && record.align_state == 1_000_000
                                && record.previous_align_state.is_none()
                    )
                })
                .expect("align_peek publishes its reset before classifying the row opener");
            let backup = ordered_observations
                .0
                .iter()
                .position(|observation| {
                    matches!(
                        observation,
                        CommandObservation::Recovery(record)
                            if record.kind == RecoveryKind::Backup
                                && record.tokens == [ObservedToken::Character {
                                    character: 'x',
                                    catcode: Catcode::Letter,
                                }]
                    )
                })
                .expect("the ordinary row opener is backed up exactly once");
            let u_template = ordered_observations
                .0
                .iter()
                .position(|observation| {
                    matches!(
                        observation,
                        CommandObservation::Input(record)
                            if record.transition == InputTransition::Push
                                && record.reason == InputReason::AlignmentUTemplate
                    )
                })
                .expect("the selected first column installs its u-template");
            assert!(
                reset < backup && backup < u_template,
                "{:#?}",
                ordered_observations.0
            );

            // Every restart caused by `\crcr` resets the sentinel, while noalign and
            // the closing right brace consume their own lookahead and create no
            // backed-up input level.
            crate::test_harness::with_nonstop_plain_universe(|branch_stores| {
                let mut branches = MainControl::tex82_initex(branch_stores);
                register_source(
                    &mut branches,
                    br"\setbox0=\vbox{\halign{#\cr\crcr\crcr\noalign{}\crcr}}\end",
                );
                let mut branch_observations = ObservationRecorder::default();
                run_to_end_observed(&mut branches, branch_stores, &mut branch_observations);
                assert_eq!(
                    branch_observations
                        .0
                        .iter()
                        .filter(|observation| matches!(
                            observation,
                            CommandObservation::Alignment(record)
                                if record.transition == "state_change"
                                    && record.align_state == 1_000_000
                                    && record.previous_align_state.is_none()
                        ))
                        .count(),
                    5,
                    "initial, two crcr, post-noalign, and final crcr probes each reset"
                );
                let branch_backups = branch_observations
                    .0
                    .iter()
                    .filter_map(|observation| match observation {
                        CommandObservation::Recovery(record)
                            if record.kind == RecoveryKind::Backup =>
                        {
                            Some(record.tokens.clone())
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                assert_eq!(
                    branch_backups,
                    vec![
                        vec![ObservedToken::Character {
                            character: '=',
                            catcode: Catcode::Other,
                        }],
                        vec![ObservedToken::Character {
                            character: '{',
                            catcode: Catcode::BeginGroup,
                        }],
                        vec![ObservedToken::Character {
                            character: '{',
                            catcode: Catcode::BeginGroup,
                        }],
                        vec![ObservedToken::Character {
                            character: '{',
                            catcode: Catcode::BeginGroup,
                        }],
                        vec![ObservedToken::Character {
                            character: '{',
                            catcode: Catcode::BeginGroup,
                        }],
                    ],
                    "only the setbox/alignment/noalign opening scanners back input; crcr and the alignment-closing right brace are consumed"
                );
            });
        });
    });
}

#[test]
fn ignorespaces_surfaces_an_alignment_delimiter_before_fin_col() {
    // TeX82 §1045 implements `\ignorespaces` by §406's in-place expanded
    // fetch. When that fetch reaches `&`, §§342/789 must install the
    // v-template before §791 `fin_col` advances the structural column. The
    // split executor therefore has to see the typed delimiter event; letting
    // the scalar helper consume it can dispatch frozen `\endv` in the same
    // operation and lose this canonical boundary.
    fn column_at_v_template(source: &[u8]) -> usize {
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            let mut control = MainControl::tex82_initex(stores);
            register_source(&mut control, source);
            let mut observations = ObservationRecorder::default();
            loop {
                let before = observations.0.len();
                match control
                    .step_with_observer(stores, &mut observations)
                    .expect("alignment operation executes")
                {
                    MainControlStep::Continue => {}
                    MainControlStep::End | MainControlStep::EndOfInput => {
                        panic!("input ended before the first v-template")
                    }
                }
                if observations.0[before..].iter().any(|observation| {
                    matches!(
                        observation,
                        CommandObservation::Alignment(record)
                            if record.transition == "v_template_push"
                    )
                }) {
                    let column = active_alignment_runtime_snapshot(&control)
                        .expect("fin_col has not advanced the active entry")
                        .column;
                    run_to_end_observed(&mut control, stores, &mut observations);
                    assert!(
                        terminal_text(stores).is_empty(),
                        "{}",
                        terminal_text(stores)
                    );
                    return column;
                }
            }
        })
    }

    let direct = column_at_v_template(br"\setbox0=\vbox{\halign{#&#\cr X&Y\cr}}\end");
    let ignored =
        column_at_v_template(br"\setbox0=\vbox{\halign{#&#\cr X\ignorespaces  &Y\cr}}\end");
    assert_eq!(
        direct, 0,
        "a direct delimiter leaves fin_col for the next step"
    );
    assert_eq!(
        ignored, direct,
        "the nested §406 fetch must preserve the direct-delimiter boundary"
    );
}

#[test]
fn init_row_halign_valign_leading_tabskip_template_span_and_aux_matrix() {
    // TeX82 §786: first and later rows use one fresh semantic row/cell
    // level, the leading tabskip, the selected first alignrecord, and the
    // canonical h/v cell mode and auxiliary initialization.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        control.set_fuel_limit(30_000).expect("bounded fuel");
        register_source(
            &mut control,
            br"\nonstopmode\tabskip=2pt
          \setbox0=\vbox{\halign{
            \ifhmode\global\advance\count0 by1\fi\hskip1pt#\cr
            \hskip3pt\cr \hskip4pt\cr}}
          \looseness=7\hangafter=9\hangindent=12pt
          \setbox1=\hbox{\valign{
            \ifvmode\ifnum\looseness=0 \ifnum\hangafter=1
              \ifdim\hangindent=0pt \global\advance\count1 by1\fi\fi\fi\fi#\cr
            \hbox{\kern3pt}\cr \hbox{\kern4pt}\cr}}
          \setbox2=\vbox{\halign{#&#\cr
            \omit\hskip1pt\span\hskip2pt\cr}}
          \end",
        );
        let mut observations = ObservationRecorder::default();

        run_to_end_observed(&mut control, stores, &mut observations);

        assert_eq!(
            stores.count(0).expect("count register"),
            2,
            "halign first/later rows enter restricted hmode"
        );
        assert_eq!(
            stores.count(1).expect("count register"),
            2,
            "valign first/later rows reset paragraph aux in internal vmode"
        );
        for register in [0, 1] {
            let rows = box_child_nodes(stores, register);
            let boxed_rows = rows
                .iter()
                .filter_map(|node| match node {
                    Node::HList(boxed) | Node::VList(boxed) => Some(boxed),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(boxed_rows.len(), 2, "register {register}: {rows:?}");
            for row in boxed_rows {
                let first = page_vec(stores, row.children).into_iter().next();
                let Some(Node::Glue { spec, kind, .. }) = first else {
                    panic!("row begins with tabskip glue: {rows:?}");
                };
                assert_eq!(kind, GlueKind::TabSkip);
                assert_eq!(spec.width.raw(), 2 * Scaled::UNITY);
            }
        }
        assert!(
            observations.0.iter().any(|observation| matches!(
                observation,
                CommandObservation::Alignment(record)
                if record.transition == "state_change"
                    && record.previous_align_state == Some(1_000_000)
                    && record.align_state == 0
            )),
            "omit initializes the cell without a u-template input level"
        );
        assert_eq!(control.current_mode(), Mode::Vertical);

        for (
            source,
            row_mode,
            row_space_factor,
            row_prev_depth,
            cell_mode,
            cell_space_factor,
            cell_prev_depth,
        ) in [
            (
                br"\setbox0=\vbox{\halign{#\cr x\cr y\cr}}\end".as_slice(),
                Mode::RestrictedHorizontal,
                0,
                None,
                Mode::RestrictedHorizontal,
                1000,
                None,
            ),
            (
                br"\setbox0=\hbox{\valign{#\cr\hbox{x}\cr\hbox{y}\cr}}\end".as_slice(),
                Mode::InternalVertical,
                0,
                Some(0),
                Mode::InternalVertical,
                0,
                Some(crate::mode::IGNORE_DEPTH.raw()),
            ),
        ] {
            crate::test_harness::with_nonstop_plain_universe(|snapshot_stores| {
                let mut snapshot_control = MainControl::tex82_initex(snapshot_stores);
                register_source(&mut snapshot_control, source);
                let mut snapshot_observations = ObservationRecorder::default();
                let first = step_until_alignment_snapshot(
                    &mut snapshot_control,
                    snapshot_stores,
                    &mut snapshot_observations,
                    |snapshot| snapshot.rows == 1,
                );
                let alignment = first.alignment;
                assert_eq!(
                    first,
                    AlignmentRuntimeSnapshot {
                        alignment,
                        column: 0,
                        cell_span: 1,
                        rows: 1,
                        captured_cells: 0,
                        row_mode,
                        row_space_factor,
                        row_prev_depth,
                        cell_mode,
                        cell_space_factor,
                        cell_prev_depth,
                    }
                );
                let later = step_until_alignment_snapshot(
                    &mut snapshot_control,
                    snapshot_stores,
                    &mut snapshot_observations,
                    |snapshot| snapshot.rows == 2,
                );
                assert_eq!(later.alignment, alignment);
                assert_eq!(later.column, 0, "every row starts at the first alignrecord");
                assert_eq!(later.cell_span, 1, "cur_span starts at that alignrecord");
                assert_eq!(
                    later.captured_cells, 0,
                    "the new row owns a fresh cell list"
                );
                assert_eq!(later.row_space_factor, row_space_factor);
                assert_eq!(later.row_prev_depth, row_prev_depth);
                assert_eq!(later.cell_space_factor, cell_space_factor);
                assert_eq!(later.cell_prev_depth, cell_prev_depth);
                run_to_end_observed(
                    &mut snapshot_control,
                    snapshot_stores,
                    &mut snapshot_observations,
                );
            });
        }

        crate::test_harness::with_nonstop_plain_universe(|span_stores| {
            let mut span_control = MainControl::tex82_initex(span_stores);
            register_source(
                &mut span_control,
                br"\setbox0=\vbox{\halign{#&#\cr x\span y\cr\omit z&z\cr}}\end",
            );
            let mut span_observations = ObservationRecorder::default();
            let spanned = step_until_alignment_snapshot(
                &mut span_control,
                span_stores,
                &mut span_observations,
                |snapshot| snapshot.column == 1 && snapshot.cell_span == 2,
            );
            assert_eq!(
                spanned.captured_cells, 0,
                "span keeps the first cell list open"
            );
            run_to_end_observed(&mut span_control, span_stores, &mut span_observations);
            assert!(span_observations.0.iter().any(|observation| matches!(
                observation,
                CommandObservation::Alignment(record) if record.transition == "omit_template_push"
            )));

            // An exhausted preamble is scanner recovery, not permission for init_row
            // to manufacture a first alignrecord. The fragment boundary keeps this
            // deliberately incomplete input bounded without appending `\end`.
            crate::test_harness::with_nonstop_plain_universe(|exhausted_stores| {
                exhausted_stores.set_interaction_mode(tex_state::InteractionMode::Nonstop);
                let mut exhausted = MainControl::tex82_initex(exhausted_stores);
                exhausted.set_root_completion_policy(RootCompletionPolicy::StopAtRootEof);
                exhausted.set_fuel_limit(2_000).expect("bounded fuel");
                register_source(&mut exhausted, br"\halign{");
                let mut exhausted_observations = ObservationRecorder::default();
                run_to_end_observed(
                    &mut exhausted,
                    exhausted_stores,
                    &mut exhausted_observations,
                );
                assert!(!exhausted_observations.0.iter().any(|observation| matches!(
                    observation,
                    CommandObservation::Alignment(record) if record.transition == "u_template_push"
                )));
                assert!(
                    terminal_text(exhausted_stores).contains("File ended while scanning"),
                    "exhausted preamble reports before row initialization"
                );
            });
        });
    });
}

#[test]
fn fin_col_delimiter_periodic_extra_tab_and_brace_depth_matrix() {
    // TeX82 §§791--795: tab/span/cr/crcr select exactly one next-cell,
    // continued-span, or row result; `\omit` uses the empty template;
    // periodic columns reuse their u/v pair and tabskip; exhausted tab/span
    // recover to cr; and a delimiter at nonzero brace depth is corrected
    // before `fin_col` sees it. Exercise both halign and valign packaging.
    for delimiter in ["&", "\\span"] {
        let source = format!(
            "\\nonstopmode\\setbox0=\\vbox{{\\halign{{#\\cr \\hskip1pt{delimiter}\\hskip2pt\\cr}}}}\\end"
        );
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            let mut control = MainControl::tex82_initex(stores);
            control.set_fuel_limit(20_000).expect("bounded fuel");
            register_source(&mut control, source.as_bytes());
            let mut observations = ObservationRecorder::default();
            run_to_end_observed(&mut control, stores, &mut observations);
            let terminal = terminal_text(stores);
            assert_eq!(
                terminal
                    .matches("Extra alignment tab has been changed to \\cr")
                    .count(),
                1,
                "{delimiter}: {terminal}"
            );
            assert_eq!(
                observations
                    .0
                    .iter()
                    .filter(|observation| matches!(
                        observation,
                        CommandObservation::Alignment(record) if record.transition == "extra_tab"
                    ))
                    .count(),
                1,
                "{delimiter} converts once"
            );
        });
    }

    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        control.set_fuel_limit(40_000).expect("bounded fuel");
        register_source(
            &mut control,
            br"\nonstopmode\tabskip=1pt\count0=0\count1=0
          \setbox0=\vbox{\halign{
            \global\advance\count0 by1 #\global\advance\count1 by1
            &&\tabskip=3pt
            \global\advance\count0 by10 #\global\advance\count1 by10\cr
            \omit\hskip1pt&\hskip2pt\span\hskip3pt&\hskip4pt\cr
            {\hskip1pt&\hskip2pt}\crcr}}
          \setbox1=\hbox{\valign{#&#\cr
            \hbox{\kern1pt}&\hbox{\kern2pt}\crcr
            \omit\vskip1pt\span\vskip2pt\cr}}
          \end",
        );
        let mut observations = ObservationRecorder::default();
        run_to_end_observed(&mut control, stores, &mut observations);

        assert_eq!(
            stores.count(0).expect("count register"),
            41,
            "periodic u-template selection is exact"
        );
        assert_eq!(
            stores.count(1).expect("count register"),
            41,
            "periodic v-template selection is exact"
        );
        let mut widths = Vec::new();
        let children = box_child_nodes(stores, 0);
        tabskip_widths(stores, &children, &mut widths);
        widths.sort_unstable();
        assert_eq!(
            widths,
            vec![
                Scaled::UNITY,
                Scaled::UNITY,
                Scaled::UNITY,
                Scaled::UNITY,
                3 * Scaled::UNITY,
                3 * Scaled::UNITY,
                3 * Scaled::UNITY,
                3 * Scaled::UNITY,
            ],
            "periodic copies retain the repeated column's following tabskip"
        );
        let halign_rows = box_child_nodes(stores, 0)
            .into_iter()
            .filter(|node| matches!(node, Node::HList(_)))
            .collect::<Vec<_>>();
        assert_eq!(halign_rows.len(), 2);
        assert_eq!(
            packaged_row_projection(stores, &halign_rows[0]),
            vec![
                PackagedRowItem::TabSkip(Scaled::UNITY),
                PackagedRowItem::HorizontalCell(vec![Scaled::UNITY]),
                PackagedRowItem::TabSkip(Scaled::UNITY),
                PackagedRowItem::HorizontalCell(vec![2 * Scaled::UNITY, 3 * Scaled::UNITY]),
                PackagedRowItem::TabSkip(3 * Scaled::UNITY),
                PackagedRowItem::HorizontalCell(vec![]),
                PackagedRowItem::TabSkip(3 * Scaled::UNITY),
                PackagedRowItem::HorizontalCell(vec![4 * Scaled::UNITY]),
                PackagedRowItem::TabSkip(3 * Scaled::UNITY),
            ],
            "each packaged cell retains the tabskip associated with its ending column; the span material is one cell followed by its resolved empty column"
        );
        assert_eq!(
            packaged_row_projection(stores, &halign_rows[1]),
            vec![
                PackagedRowItem::TabSkip(Scaled::UNITY),
                PackagedRowItem::HorizontalCell(vec![Scaled::UNITY]),
                PackagedRowItem::TabSkip(Scaled::UNITY),
                PackagedRowItem::HorizontalCell(vec![2 * Scaled::UNITY]),
                PackagedRowItem::TabSkip(3 * Scaled::UNITY),
            ],
            "brace-depth recovery still packages the corrected tab branch as a complete row"
        );
        let valign_rows = box_child_nodes(stores, 1)
            .into_iter()
            .filter(|node| matches!(node, Node::VList(_)))
            .collect::<Vec<_>>();
        assert_eq!(valign_rows.len(), 2);
        assert_eq!(
            packaged_row_projection(stores, &valign_rows[0]),
            vec![
                PackagedRowItem::TabSkip(Scaled::UNITY),
                PackagedRowItem::VerticalCell(vec![Scaled::UNITY]),
                PackagedRowItem::TabSkip(Scaled::UNITY),
                PackagedRowItem::VerticalCell(vec![2 * Scaled::UNITY]),
                PackagedRowItem::TabSkip(Scaled::UNITY),
            ]
        );
        assert_eq!(
            packaged_row_projection(stores, &valign_rows[1]),
            vec![
                PackagedRowItem::TabSkip(Scaled::UNITY),
                PackagedRowItem::VerticalCell(vec![Scaled::UNITY, 2 * Scaled::UNITY]),
                PackagedRowItem::TabSkip(Scaled::UNITY),
                PackagedRowItem::VerticalCell(vec![]),
                PackagedRowItem::TabSkip(Scaled::UNITY),
            ],
            "the omit/span branch packages one two-column vertical cell and one resolved empty column"
        );
        let terminal = terminal_text(stores);
        assert_eq!(
            terminal.matches("Missing } inserted").count(),
            1,
            "{terminal}"
        );
        assert!(
            !terminal.contains("Extra alignment tab"),
            "periodic suffix absorbs columns: {terminal}"
        );
        let transitions = observations
            .0
            .iter()
            .filter_map(|observation| match observation {
                CommandObservation::Alignment(record) => Some(record.transition),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(transitions.contains(&"omit_template_push"));
        assert!(
            transitions
                .iter()
                .filter(|transition| **transition == "v_template_push")
                .count()
                >= 6
        );
        assert_eq!(control.current_mode(), Mode::Vertical);
    });
}

#[test]
fn display_alignment_finish_complete_content_delimiter_and_spacing_matrix() {
    // TeX82 §§1206--1207: all row shapes share the assignment-before-$$
    // tail, exact delimiter recovery, direct finished-list splice, display
    // indent, penalties/glue, prevdepth restoration, and enclosing-mode resume.
    for (body, expected_rows) in [
        ("\\hskip1pt\\cr", 1usize),
        ("\\hskip1pt\\cr\\hskip2pt\\cr", 2),
        ("\\omit\\hskip1pt\\span\\hskip2pt\\cr", 1),
    ] {
        let source = format!(
            "\\nonstopmode\\setbox0=\\vbox{{\\hsize=50pt\\prevdepth=6pt\\baselineskip=20pt
             \\abovedisplayskip=3pt\\belowdisplayskip=4pt
             \\predisplaypenalty=111\\postdisplaypenalty=222
             \\noindent$$\\displayindent=7pt\\halign{{#&#\\cr {body}}}
             \\global\\advance\\count0 by1 $$\\hbox{{\\kern13pt}}}}\\end"
        );
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            let mut control = MainControl::tex82_initex(stores);
            control.set_fuel_limit(30_000).expect("bounded fuel");
            register_source(&mut control, source.as_bytes());
            run_to_end(&mut control, stores);
            assert_eq!(
                stores.count(0).expect("count register"),
                1,
                "post-alignment assignment executes first"
            );
            let terminal = terminal_text(stores);
            assert!(!terminal.contains("Display math should end"), "{terminal}");
            let nodes = box_child_nodes(stores, 0);
            let projection = alignment_node_projection(stores, &nodes);
            let pre = projection
                .iter()
                .position(|node| *node == AlignmentNodeProjection::Penalty(111))
                .expect("pre-display penalty");
            let post = projection
                .iter()
                .position(|node| *node == AlignmentNodeProjection::Penalty(222))
                .expect("post-display penalty");
            assert_eq!(
                projection[pre + 1],
                AlignmentNodeProjection::AboveDisplay(3 * Scaled::UNITY)
            );
            assert_eq!(
                projection[post + 1],
                AlignmentNodeProjection::BelowDisplay(4 * Scaled::UNITY)
            );
            assert!(pre + 1 < post);
            assert!(
                projection[pre + 2..post].iter().any(|node| matches!(
                    node,
                    AlignmentNodeProjection::Box { shift, .. } if *shift == 7 * Scaled::UNITY
                )),
                "display rows carry displayindent: {projection:?}"
            );
            assert_eq!(
                projection[post + 2],
                AlignmentNodeProjection::Baseline(20 * Scaled::UNITY),
                "§1207 restores the completed alignment's zero aux prevdepth before the following zero-height hbox"
            );
            assert_eq!(
                projection[post + 3],
                AlignmentNodeProjection::Box {
                    shift: 0,
                    kerns: vec![13 * Scaled::UNITY],
                },
                "post-display material resumes after the ordered display tail"
            );
            let display_rows = nodes
            .iter()
            .filter(
                |node| matches!(node, Node::HList(boxed) if boxed.shift.raw() == 7 * Scaled::UNITY),
            )
            .count();
            assert_eq!(display_rows, expected_rows, "{nodes:?}");
            assert_eq!(control.current_mode(), Mode::Vertical);
        });
    }

    for (tail, diagnostic, offender_kern) in [
        (
            "$\\global\\advance\\count0 by1\\par",
            "Display math should end with $$.",
            None,
        ),
        (
            "\\global\\advance\\count0 by1\\kern13pt",
            "Missing $$ inserted.",
            Some(13 * Scaled::UNITY),
        ),
    ] {
        let source = format!(
            "\\nonstopmode\\setbox0=\\vbox{{\\noindent$$\\halign{{#\\cr\\cr}}{tail}}}\\end"
        );
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            let mut control = MainControl::tex82_initex(stores);
            control.set_fuel_limit(20_000).expect("bounded fuel");
            register_source(&mut control, source.as_bytes());
            let mut recovery_observations = ObservationRecorder::default();
            run_to_end_observed(&mut control, stores, &mut recovery_observations);
            let terminal = terminal_text(stores);
            assert_eq!(terminal.matches(diagnostic).count(), 1, "{terminal}");
            assert_eq!(
                stores.count(0).expect("count register"),
                1,
                "offending assignment is backed up once"
            );
            if offender_kern.is_some() {
                let backup = recovery_observations
                .0
                .iter()
                .position(|observation| {
                    matches!(
                        observation,
                        CommandObservation::Recovery(record)
                            if record.kind == RecoveryKind::Backup
                                && record.tokens == [ObservedToken::ControlSequence("kern".into())]
                    )
                })
                .expect("the non-math-shift command is backed up");
                let replay = recovery_observations
                    .0
                    .iter()
                    .enumerate()
                    .skip(backup + 1)
                    .find(|(_, observation)| {
                        matches!(
                            observation,
                            CommandObservation::Command(record)
                                if record.boundary == CommandDeliveryBoundary::Raw
                                    && record.command == "kern"
                        )
                    })
                    .map(|(index, _)| index)
                    .expect("the backed-up command is delivered after display recovery");
                assert!(backup < replay);
            }
            assert_eq!(control.current_mode(), Mode::Vertical);
        });
    }
}

#[test]
fn vsplit_kernel_separates_result_remainder_and_split_marks() {
    // TeX82 §§977--979: the chosen prefix becomes a separately packed box,
    // the source register is replaced by its pruned remainder, and the split
    // marks describe only the extracted prefix.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
        &mut control,
        br"\setbox0=\vbox{\mark{first}\hrule height10pt\penalty-10000\mark{second}\hrule height10pt}
           \setbox1=\vsplit0 to10pt\end",
    );

        run_to_end(&mut control, stores);

        let split = stores
            .copy_box_to_page(1)
            .expect("split prefix is assigned");
        let remainder = stores
            .copy_box_to_page(0)
            .expect("split remainder replaces source");
        assert_ne!(
            split, remainder,
            "prefix and remainder have distinct ownership"
        );
        assert!(matches!(
            first_published_node(stores, split),
            Some(Node::VList(_))
        ));
        assert!(matches!(
            first_published_node(stores, remainder),
            Some(Node::VList(_))
        ));
        for mark in [PageMark::SplitFirst, PageMark::SplitBot] {
            let tokens = admitted!(stores, |context| {
                let key = context.page_mark(mark);
                context
                    .node_token_words(key)
                    .expect("live page mark")
                    .to_vec()
            });
            assert_eq!(
                tokens,
                [
                    Token::Char {
                        ch: 'f',
                        cat: Catcode::Letter,
                    },
                    Token::Char {
                        ch: 'i',
                        cat: Catcode::Letter,
                    },
                    Token::Char {
                        ch: 'r',
                        cat: Catcode::Letter,
                    },
                    Token::Char {
                        ch: 's',
                        cat: Catcode::Letter,
                    },
                    Token::Char {
                        ch: 't',
                        cat: Catcode::Letter,
                    }
                ]
                .map(tex_state::token::TokenWord::pack)
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
fn direct_material_appends_typed_nodes_in_source_order() {
    // TeX82 §§1055--1061: each completed typed operand is appended exactly
    // once and preserves its distinct node kind and numeric value.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\setbox0=\hbox{\kern1pt\hskip2pt\vrule width3pt height4pt depth5pt}\end",
        );

        run_to_end(&mut control, stores);

        let nodes = box_child_nodes(stores, 0);
        let [
            Node::Kern { amount, kind },
            Node::Glue { spec, .. },
            Node::Rule {
                width,
                height,
                depth,
            },
        ] = nodes.as_slice()
        else {
            panic!("direct material remains in source order: {nodes:?}");
        };
        assert_eq!(*kind, tex_state::node::KernKind::Explicit);
        assert_eq!(amount.raw(), Scaled::UNITY);
        assert_eq!(spec.width.raw(), 2 * Scaled::UNITY);
        assert_eq!(width.map(Scaled::raw), Some(3 * Scaled::UNITY));
        assert_eq!(height.map(Scaled::raw), Some(4 * Scaled::UNITY));
        assert_eq!(depth.map(Scaled::raw), Some(5 * Scaled::UNITY));
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
        assert_eq!(stores.world().artifact_commits().len(), 1);
    });
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

fn etex_initex<G>(stores: &mut Universe<G>) -> MainControl<G> {
    tex_command::install_tex82_expandable_primitives(stores);
    tex_command::install_etex_expandable_primitives(stores);
    crate::install_unexpandable_primitives(stores);
    crate::install_etex_unexpandable_primitives(stores);
    MainControl::prepared_initex(CommandProfile::ETEX26)
}

fn pdftex_initex<G>(stores: &mut Universe<G>) -> MainControl<G> {
    tex_command::install_tex82_expandable_primitives(stores);
    tex_command::install_etex_expandable_primitives(stores);
    tex_command::install_pdftex_expandable_primitives(stores);
    crate::install_unexpandable_primitives(stores);
    crate::install_etex_unexpandable_primitives(stores);
    tex_command::install_pdftex_unexpandable_primitives(stores);
    MainControl::prepared_initex(CommandProfile::PDFTEX14029)
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
    });
}

#[test]
fn etex_raw_font_character_enquiry_checkpoint_retry_is_atomic() {
    // The `last_item` command identity is serialized in an e-TeX format.
    // Restoring a quiescent checkpoint must restore both the diagnostic
    // effect and the unconsumed operand so a retry takes the identical path.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = etex_initex(stores);
        control.set_fuel_limit(1_000).expect("bounded fuel");
        register_source(&mut control, br"\nonstopmode \fontcharwd a\end");
        assert_eq!(
            control.step(stores).expect("interaction mode executes"),
            MainControlStep::Continue
        );
        let checkpoint = control
            .capture_checkpoint(
                crate::EngineBoundary::OuterParagraphEnd,
                stores,
                crate::ExecutionBudgetCounters::default(),
            )
            .expect("raw font enquiry checkpoints");

        assert_eq!(
            control.step(stores).expect("raw font enquiry recovers"),
            MainControlStep::Continue
        );
        let first_hash = stores.journal_cursor().expect("state cursor");
        let first_output = terminal_text(stores);
        assert!(first_output.contains("You can't use `\\fontcharwd' in vertical mode"));

        control
            .restore_checkpoint(&checkpoint, stores)
            .expect("raw font enquiry state restores");
        assert_eq!(
            control
                .step(stores)
                .expect("raw font enquiry retry recovers"),
            MainControlStep::Continue
        );
        assert_eq!(stores.journal_cursor().expect("state cursor"), first_hash);
        assert_eq!(terminal_text(stores), first_output);
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
fn etex_parshape_enquiry_checkpoint_retry_is_atomic() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = etex_initex(stores);
        control.set_fuel_limit(1_000).expect("bounded fuel");
        register_source(&mut control, br"\nonstopmode \parshapelength1\end");
        assert_eq!(
            control.step(stores).expect("interaction mode executes"),
            MainControlStep::Continue
        );
        let checkpoint = control
            .capture_checkpoint(
                crate::EngineBoundary::OuterParagraphEnd,
                stores,
                crate::ExecutionBudgetCounters::default(),
            )
            .expect("raw parshape enquiry checkpoints");

        assert_eq!(
            control.step(stores).expect("raw parshape enquiry recovers"),
            MainControlStep::Continue
        );
        let first_hash = stores.journal_cursor().expect("state cursor");
        let first_output = terminal_text(stores);
        assert!(first_output.contains("You can't use `\\parshapelength' in vertical mode"));

        control
            .restore_checkpoint(&checkpoint, stores)
            .expect("raw parshape enquiry state restores");
        assert_eq!(
            control
                .step(stores)
                .expect("raw parshape enquiry retry recovers"),
            MainControlStep::Continue
        );
        assert_eq!(stores.journal_cursor().expect("state cursor"), first_hash);
        assert_eq!(terminal_text(stores), first_output);
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
fn tex82_display_parameters_are_local_to_the_math_shift_group() {
    // TeX82 §§1145/1194/283: display parameters are defined after
    // `push_math(math_shift_group)` and restored in reverse assignment order.
    // e-TeX's `\predisplaydirection` extension is absent in TeX82 mode.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\tracingrestores=1\tracingonline=1\noindent $$x$$\end",
        );

        run_to_end(&mut control, stores);

        let terminal = terminal_text(stores);
        let display_indent = terminal
            .find("{restoring \\displayindent=0.0pt}")
            .expect("display indent restore");
        let display_width = terminal
            .find("{restoring \\displaywidth=0.0pt}")
            .expect("display width restore");
        let pre_display_size = terminal
            .find("{restoring \\predisplaysize=0.0pt}")
            .expect("pre-display size restore");
        let family = terminal
            .find("{restoring \\fam=0}")
            .expect("display family restore");
        assert!(display_indent < display_width);
        assert!(display_width < pre_display_size);
        assert!(pre_display_size < family);
        assert!(!terminal.contains("predisplaydirection"));
    });
}

#[test]
fn noalign_body_dispatches_nested_math_braces_by_save_stack_group() {
    // TeX82 §§785, 1068-1069, and 1133: material inside `no_align_group`
    // runs through ordinary main control. Only a right brace delivered while
    // that group is current ends `\noalign`; braces belonging to nested math
    // groups must close those groups first.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = etex_initex(stores);
        control.set_fuel_limit(10_000).expect("bounded fuel");
        register_source(
            &mut control,
            br"\valign{#\cr\noalign{$${\left.\middle.\right.}$$}}\end",
        );

        for _ in 0..256 {
            match control.step(stores).expect("nested noalign math executes") {
                MainControlStep::End | MainControlStep::EndOfInput => return,
                MainControlStep::Continue => {}
            }
        }
        panic!("noalign regression exceeded its step bound");
    });
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

fn run_to_end_observed<G>(
    control: &mut MainControl<G>,
    stores: &mut Universe<G>,
    observations: &mut dyn CommandObserver,
) {
    loop {
        match control
            .step_with_observer(stores, observations)
            .expect("program executes")
        {
            MainControlStep::End | MainControlStep::EndOfInput => break,
            MainControlStep::Continue => {}
        }
    }
}

fn terminal_text<G>(stores: &Universe<G>) -> String {
    let committed = stores
        .world()
        .memory_terminal_output()
        .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
        .unwrap_or_default();
    let pending: String = stores
        .world()
        .effect_records()
        .iter()
        .filter_map(|effect| match effect {
            tex_state::EffectRecord::StreamWrite {
                sink:
                    tex_state::PrintSink::Terminal
                    | tex_state::PrintSink::TerminalAndLog
                    | tex_state::PrintSink::Log,
                text,
            } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    committed + &pending
}

#[test]
fn misplaced_alignment_commands_route_exact_help_and_continue() {
    let cases: &[(&[u8], &str, &[&str])] = &[
        (
            b"&",
            "Misplaced alignment tab character &.",
            &[
                "I can't figure out why you would want to use a tab mark",
                "here. If you just want an ampersand, the remedy is",
                "simple: Just type `I\\&' now. But if some right brace",
                "up above has ended a previous alignment prematurely,",
                "you're probably due for more error messages, and you",
                "might try typing `S' now just to see what is salvageable.",
            ],
        ),
        (
            br"\cr",
            "Misplaced \\cr.",
            &[
                "I can't figure out why you would want to use a tab mark",
                "or \\cr or \\span just now. If something like a right brace",
                "up above has ended a previous alignment prematurely,",
                "you're probably due for more error messages, and you",
                "might try typing `S' now just to see what is salvageable.",
            ],
        ),
        (br"\crcr", "Misplaced \\crcr.", &[]),
        (br"\span", "Misplaced \\span.", &[]),
        (
            br"\noalign",
            "Misplaced \\noalign.",
            &[
                "I expect to see \\noalign only after the \\cr of",
                "an alignment. Proceed, and I'll ignore this case.",
            ],
        ),
        (
            br"\omit",
            "Misplaced \\omit.",
            &[
                "I expect to see \\omit only after tab marks or the \\cr of",
                "an alignment. Proceed, and I'll ignore this case.",
            ],
        ),
    ];
    let delimiter_help = cases[1].2;

    for &(command, primary, help) in cases {
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            stores
                .world_mut()
                .push_memory_terminal_line("h")
                .expect("memory terminal accepts the help request");
            stores
                .world_mut()
                .push_memory_terminal_line("s")
                .expect("memory terminal accepts the continuation request");
            let mut control = MainControl::tex82_initex(stores);
            let mut source = command.to_vec();
            source.extend_from_slice(br"\count0=17\end");
            register_source(&mut control, &source);

            run_to_end(&mut control, stores);

            assert_eq!(
                stores.count(0).expect("count register"),
                17,
                "recovery did not continue for {primary}"
            );
            let output = terminal_text(stores);
            assert!(output.contains(&format!("! {primary}")), "{output}");
            let expected_help = if help.is_empty() {
                delimiter_help
            } else {
                help
            };
            let exact_help = expected_help.join("\n");
            assert!(
                output.contains(&exact_help),
                "missing exact help for {primary}: {output}"
            );
        });
    }
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

fn pending_sink_text<G>(stores: &Universe<G>, terminal: bool) -> String {
    stores
        .world()
        .effect_records()
        .iter()
        .filter_map(|effect| match effect {
            tex_state::EffectRecord::StreamWrite { sink, text }
                if if terminal {
                    matches!(
                        sink,
                        tex_state::PrintSink::Terminal | tex_state::PrintSink::TerminalAndLog
                    )
                } else {
                    matches!(
                        sink,
                        tex_state::PrintSink::Log | tex_state::PrintSink::TerminalAndLog
                    )
                } =>
            {
                Some(text.as_str())
            }
            _ => None,
        })
        .collect()
}

fn macro_words<G>(stores: &mut Universe<G>, name: &str) -> Vec<tex_state::token::TokenWord> {
    let symbol = stores
        .intern(name)
        .expect("macro control sequence")
        .symbol();
    admitted!(stores, |context| match context.meaning(symbol) {
        tex_state::ResolvedMeaning::Macro { definition, .. } => {
            context.definition(definition).replacement_text().to_vec()
        }
        _ => panic!("{name} is a macro"),
    })
}

fn macro_character_text<G>(stores: &mut Universe<G>, name: &str) -> String {
    macro_words(stores, name)
        .into_iter()
        .filter_map(|word| match word.semantic_token() {
            Token::Char { ch, .. } => Some(ch),
            Token::Cs(_) | Token::Param(_) | Token::Frozen(_) => None,
        })
        .collect()
}

fn macro_semantic_tokens<G>(stores: &mut Universe<G>, name: &str) -> Vec<Token> {
    macro_words(stores, name)
        .into_iter()
        .map(tex_state::token::TokenWord::semantic_token)
        .collect()
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
fn let_recognizes_only_raw_other_equals() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\def\source{X}\let\otherequals==\let\rawtest\otherequals\source\end",
        );
        run_to_end(&mut control, stores);

        let raw_test = stores.intern("rawtest").expect("name").symbol();
        assert_eq!(
            stores.meaning(raw_test).expect("meaning"),
            tex_state::meaning::ResolvedMeaning::Static(Meaning::CharToken {
                ch: '=',
                cat: Catcode::Other,
            })
        );
    });
}

#[test]
fn hot_definition_group_and_catcode_apply_is_observation_neutral() {
    // TeX82 §§1211--1234: attaching the detached command observer must not
    // select another semantic implementation. This source exercises every
    // measured direct-apply family, local restoration, explicit and forced
    // global scope, expanded definitions, and future-let replay.
    const SOURCE: &[u8] = br"\def\a#1{A#1}
        \edef\b{\a B}
        \long\gdef\c#1{C#1}
        \xdef\d{D}
        \begingroup
          \def\local{inside}
          \let\alias=\a
          \futurelet\peek\relax\relax
          \catcode64=11
        \endgroup
        \globaldefs=-1 \global\def\forcedlocal{gone}
        \globaldefs=1 {\def\forcedglobal{kept}}
        \globaldefs=0
        \end";

    with_etex(SOURCE, |unobserved| {
        let unobserved_terminal = terminal_text(unobserved);
        let unobserved_log = pending_sink_text(unobserved, false);
        crate::test_harness::with_nonstop_plain_universe(|observed| {
            tex_command::install_tex82_expandable_primitives(observed);
            tex_command::install_etex_expandable_primitives(observed);
            crate::install_unexpandable_primitives(observed);
            crate::install_etex_unexpandable_primitives(observed);
            let mut control = MainControl::prepared_initex(CommandProfile::ETEX26);
            register_source(&mut control, SOURCE);
            let mut observations = ObservationRecorder::default();
            run_to_end_observed(&mut control, observed, &mut observations);

            assert_eq!(terminal_text(observed), unobserved_terminal);
            assert_eq!(pending_sink_text(observed, false), unobserved_log);
            assert_eq!(observed.catcode('@'), Catcode::Other);
            assert!(admitted!(observed, |context| {
                context.symbol("local").is_none_or(|symbol| {
                    context.meaning(symbol) == ResolvedMeaning::Static(Meaning::Undefined)
                })
            }));
            assert_eq!(macro_character_text(observed, "forcedglobal"), "kept");
            assert!(observations.0.iter().any(|observation| matches!(
                observation,
                CommandObservation::Mutation(record)
                    if record.target == MutationTarget::Meaning
                        && observation_tokens(&record.value).is_some()
            )));
        });
    });
}

#[test]
fn local_definition_region_survives_active_body_and_global_let_escape() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\def\result{bad}
                \begingroup
                  \def\source{promoted}
                  \global\let\escaped=\source
                  \def\cross{\endgroup\gdef\result{continued}}
                  \cross
                \end",
        );
        run_to_end(&mut control, stores);

        assert_eq!(macro_character_text(stores, "escaped"), "promoted");
        assert_eq!(macro_character_text(stores, "result"), "continued");
        assert!(admitted!(stores, |context| {
            let source = context.intern_control_sequence("source");
            let cross = context.intern_control_sequence("cross");
            context.meaning(source) == ResolvedMeaning::Static(Meaning::Undefined)
                && context.meaning(cross) == ResolvedMeaning::Static(Meaning::Undefined)
        }));
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
fn bare_macro_parameter_commit_survives_later_input_retry_without_duplication() {
    // The §1045 diagnostic is part of the parameter command's committed
    // operation. A later resource suspension rolls back only its own input
    // attempt and must neither erase nor duplicate the earlier report.
    // The mode is the harness's `\nonstopmode` rather than an explicit
    // `\errorstopmode`: §1045's report is routed to the terminal either way,
    // and errorstop would send §82 into §83's dialog, which this harness's
    // terminal cannot answer and §71 ends the job over.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"#\input child\end");

        assert!(matches!(
            control.advance(stores).expect("parameter recovers"),
            StepResult::Progress(ReplayStep::Continue)
        ));
        let committed = terminal_text(stores);
        assert_eq!(committed.matches("macro parameter character #").count(), 1);

        for _ in 0..3 {
            assert!(matches!(
                control.advance(stores).expect("missing input suspends"),
                StepResult::Suspended(ResourceNeed::Input {
                    name,
                    original_name,
                }) if name == "child.tex" && original_name == "child"
            ));
            assert_eq!(terminal_text(stores), committed);
        }

        control.capabilities_mut().register_input(
            "child.tex",
            SourceRegistration::new(RegisteredSourceKind::Generated, Arc::<[u8]>::from(&b""[..])),
        );
        run_to_end(&mut control, stores);
        assert_eq!(
            terminal_text(stores)
                .matches("macro parameter character #")
                .count(),
            1
        );
    });
}

#[test]
fn production_batch_commits_ordinary_prefix_before_terminal_transaction() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\count0=11 \count1=22 \end");

        assert_eq!(
            control.advance_episode(stores).expect("batch completes"),
            StepResult::Progress(ReplayStep::End)
        );
        assert_eq!(stores.count(0).expect("count register"), 11);
        assert_eq!(stores.count(1).expect("count register"), 22);
        assert_eq!(control.advance_telemetry().attempts, 2);
        assert_eq!(control.advance_telemetry().commits, 2);
    });
}

#[test]
fn ranked_assignments_use_one_processor_borrow_each() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\def\a{A}\let\b=\a\catcode65=11 ");

        for operation in 0..3 {
            let before = control.command.lifecycle_stats();
            assert_eq!(
                control
                    .advance(stores)
                    .expect("ranked assignment completes"),
                StepResult::Progress(ReplayStep::Continue)
            );
            let after = control.command.lifecycle_stats();
            assert_eq!(
                after.processor_entries,
                before.processor_entries + 1,
                "operation {operation} must deliver, expand, and scan in one borrow: {after:?}"
            );
            assert_eq!(after.processor_entries, after.processor_completions);
            assert_eq!(after.live_processors, 0);
            assert_eq!(after.maximum_live_processors, 1);
        }

        let lifecycle = control.command.lifecycle_stats();
        assert_eq!(lifecycle.processor_entries, 3);
        assert_eq!(
            lifecycle.processor_entries, lifecycle.processor_completions,
            "every command facade retires before the episode barrier"
        );
        assert_eq!(lifecycle.live_processors, 0);
        assert_eq!(lifecycle.maximum_live_processors, 1);
    });
}

#[test]
fn production_batch_keeps_ordinary_prefix_on_resource_need() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\count0=11 \input child\end");

        let batch_step = control.advance_episode(stores).expect("batch suspends");
        assert!(
            matches!(
                batch_step,
                StepResult::Suspended(ResourceNeed::Input { ref name, .. }) if name == "child.tex"
            ),
            "unexpected batch step: {batch_step:?}"
        );
        assert_eq!(
            stores.count(0).expect("count register"),
            11,
            "the successful ordinary prefix commits"
        );
        let suspended = control.advance_telemetry();
        assert_eq!(suspended.rollbacks, 1);
        assert_eq!(suspended.resource_replayed_delivered_tokens, 0);
        assert_eq!(suspended.resource_replayed_dispatches, 0);
        let suspended_interpreter = control.command.lifecycle_stats();
        assert_eq!(
            suspended_interpreter.processor_entries, suspended_interpreter.processor_completions,
            "resource suspension retires the interpreter facade"
        );
        assert_eq!(suspended_interpreter.live_processors, 0);

        control.capabilities_mut().register_input(
            "child.tex",
            SourceRegistration::new(RegisteredSourceKind::Generated, Arc::<[u8]>::from(&b""[..])),
        );
        let mut retried = control.advance_episode(stores).expect("retry resumes");
        for _ in 0..8 {
            if retried == StepResult::Progress(ReplayStep::End) {
                break;
            }
            retried = control
                .advance_episode(stores)
                .expect("effect-bounded retry continues");
        }
        assert_eq!(retried, StepResult::Progress(ReplayStep::End));
        assert_eq!(stores.count(0).expect("count register"), 11);
        let telemetry = control.advance_telemetry();
        let resumed_interpreter = control.command.lifecycle_stats();
        assert!(
            resumed_interpreter.processor_entries > suspended_interpreter.processor_entries,
            "retry resumes the same persistent interpreter through new borrow scopes"
        );
        assert_eq!(
            resumed_interpreter.processor_entries,
            resumed_interpreter.processor_completions
        );
        assert_eq!(resumed_interpreter.live_processors, 0);
        assert_eq!(resumed_interpreter.maximum_live_processors, 1);
        let direct_work = control.command_work();
        assert_eq!(direct_work.fuel_charges, 17);
        #[cfg(feature = "profiling")]
        assert_eq!(
            direct_work,
            tex_command::CommandWorkCounters {
                fuel_charges: 17,
                token_frame_steps: 17,
                expanded_deliveries: 14,
                meaning_lookups: 5,
                scanner_tokens: 0,
                write_expansions: 0,
                raw_delivery_kinds: [17, 0, 0, 0],
            },
            "the direct prefix and one-command retry have exact actual work"
        );
        assert_eq!(telemetry.rollbacks, 1);
        assert_eq!(telemetry.resource_replayed_delivered_tokens, 0);
        assert_eq!(telemetry.resource_replayed_dispatches, 0);
        assert_eq!(telemetry.attempts, telemetry.commits + telemetry.rollbacks);
    });
}

#[test]
fn prepared_openin_probe_resumes_after_the_blocked_macro_command() {
    let source = br"\font\bodyfont=cmr10 \bodyfont A\def\sectionref{0}\def\pagerefvalue{0}\def\newlabel#1#2{\gdef\sectionref{1}\gdef\pagerefvalue{1}}\def\load{\openin0=child \ifeof0\else\closein0\input child\fi \openin2=second \ifeof2\else\closein2\input second\fi \count0=7}\load\end";
    let child = SourceRegistration::new(
        RegisteredSourceKind::Generated,
        Arc::<[u8]>::from(
            &br"\newlabel{sec:intro}{{1}{1}}
"[..],
        ),
    );
    let second = SourceRegistration::new(
        RegisteredSourceKind::Generated,
        Arc::<[u8]>::from(&br"\global\count2=11\endinput"[..]),
    );

    let run = |preloaded: bool| {
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            let mut control = MainControl::tex82_initex(stores);
            if preloaded {
                register_cmr10_as(&mut control, stores, "cmr10.tfm");
                control
                    .capabilities_mut()
                    .register_input("child.tex", child.clone());
                control
                    .capabilities_mut()
                    .register_input("second.tex", second.clone());
            }
            register_source(&mut control, source);
            if !preloaded {
                assert!(matches!(
                    control.advance_episode(stores).expect("font suspends"),
                    StepResult::Suspended(ResourceNeed::Font { .. })
                ));
                register_cmr10_as(&mut control, stores, "cmr10.tfm");
                let mut child_probe = control.advance_episode(stores).expect("probe step");
                for _ in 0..8 {
                    if matches!(child_probe, StepResult::Suspended(_)) {
                        break;
                    }
                    child_probe = control.advance_episode(stores).expect("probe step");
                }
                assert!(matches!(
                    child_probe,
                    StepResult::Suspended(ResourceNeed::InputProbe { ref request })
                        if request.name == "child.tex"
                ));
                control
                    .capabilities_mut()
                    .register_input("child.tex", child.clone());
                let mut second_probe = control.advance_episode(stores).expect("second probe step");
                for _ in 0..8 {
                    if matches!(second_probe, StepResult::Suspended(_)) {
                        break;
                    }
                    second_probe = control.advance_episode(stores).expect("second probe step");
                }
                assert!(
                    matches!(
                        second_probe,
                        StepResult::Suspended(ResourceNeed::InputProbe { ref request })
                            if request.name == "second.tex"
                    ),
                    "unexpected second probe: {second_probe:?}"
                );
                control
                    .capabilities_mut()
                    .register_input("second.tex", second.clone());
            }
            run_to_end(&mut control, stores);
            (
                stores.count(0).expect("count register"),
                stores.count(1).expect("count register"),
                stores.count(2).expect("count register"),
                terminal_text(stores),
            )
        })
    };

    let uninterrupted = run(true);
    assert_eq!(uninterrupted.0, 7);
    assert_eq!(uninterrupted.1, 0);
    assert_eq!(uninterrupted.2, 11);
    let suspended = run(false);
    assert_eq!(suspended, uninterrupted);
}

#[test]
fn superscript_math_group_propagates_and_resumes_input_probe() {
    // TeX82 §1153 returns from `scan_math` immediately after `push_math`;
    // §1030 ordinary main control executes this braced superscript until
    // §1186's right-brace command stores the finished mlist. A resource need
    // in that body must therefore use the ordinary typed suspension seam. The
    // global increment before the probe proves that resumption neither
    // replays the opener nor restarts already committed body commands.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"$^{\global\advance\count0 by1 \openin0=child \ifeof0\else\closein0\fi}\global\count1=23",
        );

        let need = loop {
            match control
                .advance_episode(stores)
                .expect("superscript file enquiry suspends")
            {
                StepResult::Suspended(need @ ResourceNeed::InputProbe { .. }) => break need,
                StepResult::Progress(_) => {}
                other => panic!("unexpected superscript probe step: {other:?}"),
            }
        };
        let ResourceNeed::InputProbe { request } = &need else {
            unreachable!();
        };
        assert_eq!(request.name, "child.tex");
        assert_eq!(stores.count(0).expect("count register"), 1);
        control.capabilities_mut().register_input_probe(
            request.name.clone(),
            tex_command::FileEnquiryResource::new(
                SourceRegistration::new(
                    RegisteredSourceKind::Generated,
                    Arc::<[u8]>::from(&b""[..]),
                ),
                None,
            ),
        );

        for _ in 0..16 {
            control
                .advance_episode(stores)
                .expect("superscript probe resumes through its right brace");
            if stores.count(1).expect("count register") == 23 {
                break;
            }
        }
        assert_eq!(stores.count(0).expect("count register"), 1);
        assert_eq!(stores.count(1).expect("count register"), 23);
        assert!(control.active_math_fields.is_empty());
        assert!(control.pending_resource_operation.is_none());
        assert_eq!(
            control
                .advance_telemetry()
                .resource_replayed_delivered_tokens,
            0
        );
        assert_eq!(control.advance_telemetry().resource_replayed_dispatches, 0);
    });
}

#[test]
fn nested_file_probe_resumes_expandafter_collector_csname_and_integer_frames() {
    // e-TeX [27.465] enters a nested general-text collector for `\unexpanded`.
    // Its expanded opener may suspend inside pdfTeX §1590 file enquiry; retry
    // must resume the special direct-splice route rather than expand the
    // retained `\unexpanded` command as an ordinary command. TeX82 §§368 and
    // 372 must likewise retain `\expandafter`'s first operand and `\csname`'s
    // accumulated name when their nested expansion suspends. TeX82 §§440--445
    // also retain a leading scan, consumed radix prefix, or §442 character
    // constant whose expanded optional-space probe suspended. Restarting the
    // opcode would treat the resolved terminator as a fresh number.
    let child = SourceRegistration::new(
        RegisteredSourceKind::Generated,
        Arc::<[u8]>::from(&b"AB"[..]),
    );

    let run = |source: &[u8], preloaded: bool| {
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            let mut control = pdftex_initex(stores);
            if preloaded {
                control.capabilities_mut().register_input_probe(
                    "child",
                    tex_command::FileEnquiryResource::new(child.clone(), None),
                );
            }
            register_source(&mut control, source);
            if !preloaded {
                let need = loop {
                    match control.advance_episode(stores).expect("probe step") {
                        StepResult::Suspended(ResourceNeed::InputProbe { request }) => {
                            break request;
                        }
                        StepResult::Progress(_) => {}
                        other => panic!("unexpected nested enquiry step: {other:?}"),
                    }
                };
                assert_eq!(need.name, "child");
                control.capabilities_mut().register_input_probe(
                    need.name,
                    tex_command::FileEnquiryResource::new(child.clone(), None),
                );
            }
            run_to_end(&mut control, stores);
            terminal_text(stores)
        })
    };

    for (source, expected) in [
        (
            br"\edef\result{\unexpanded\expandafter{\pdffiledump length 2{child}}}\message{[\result]}\end"
                .as_slice(),
            "[4142]",
        ),
        (
            br"\edef\result{P\unexpanded\expandafter{{A\pdffiledump length 2{child}B}}Q}\message{[\result]}\end"
                .as_slice(),
            "[P{A4142B}Q]",
        ),
        (
            br"\edef\result{\csname a\pdffiledump length 2{child}b\endcsname}\message{[\meaning\result]}\end"
                .as_slice(),
            "a4142b",
        ),
        (
            br"\edef\result{\romannumeral\pdffilesize{child}}\message{[\result]}\end".as_slice(),
            "[ii]",
        ),
        (
            br"\nonstopmode\def\gobble#1X{Z}\def\term{\expandafter\gobble\pdffilesize{child}X}\edef\result{\romannumeral0\term}\message{[\result]}\end"
                .as_slice(),
            "[Z]",
        ),
        (
            br"\catcode0=13 \protected\def^^@{}\edef\result{\romannumeral`^^@\pdffilesize{child}}\message{[\result]}\end"
                .as_slice(),
            "[2]",
        ),
    ] {
        let uninterrupted = run(source, true);
        assert!(uninterrupted.contains(expected), "{uninterrupted:?}");
        assert_eq!(run(source, false), uninterrupted);
    }
}

#[test]
fn nested_csname_and_ifcsname_accumulators_resume_their_typed_parents() {
    // TeX82 §372 and e-TeX [17.4765--4779] use the same expanded name scan,
    // but each suspended invocation owns a different accumulated spelling and
    // `\ifcsname` additionally owns an already-pushed condition frame. A
    // caller-order name stack can make these examples appear to work only by
    // matching recursive return order; the continuation must instead move
    // each name directly with its enclosing expansion or conditional phase.
    let source = br"\expandafter\def\csname inner4143\endcsname{Z}\expandafter\def\csname outerZtail\endcsname{OK}\expandafter\def\csname inner4\endcsname{Y}\expandafter\def\csname outerYtail\endcsname{YES}\edef\result{\csname outer\csname inner\pdffiledump length 1{second}\pdffiledump length 1{third}\endcsname tail\endcsname}\ifcsname outer\csname inner\pdffilesize{first}\endcsname tail\endcsname\message{[\result:YES]}\else\message{[bad-ifcsname]}\fi\unless\ifcsname missing\pdffilesize{4}\endcsname\message{[UNLESS]}\else\message{[bad-unless]}\fi\end";

    let (preloaded_terminal, preloaded_requests) =
        run_pdftex_file_probe_job(source, &["second", "third", "first", "4"]);
    assert!(preloaded_requests.is_empty());
    assert!(
        preloaded_terminal.contains("[OK:YES] [UNLESS]"),
        "{preloaded_terminal:?}"
    );

    let (staged_terminal, staged_requests) = run_pdftex_file_probe_job(source, &[]);
    assert_eq!(staged_requests, ["second", "third", "first", "4"]);
    assert_eq!(staged_terminal, preloaded_terminal);
}

#[test]
fn count_assignment_resumes_exact_integer_operand_after_file_probe() {
    // The literal prefix has already committed when `\pdffilesize` asks the
    // host for `second`. Restarting the assignment or the integer scanner
    // loses that prefix; the retained direct-operation and scalar frames must
    // instead continue the same radix tail, yielding 12 followed by 2.
    for (source, resources, expected) in [
        (
            br"\count0=12\pdffilesize{second}\message{[\the\count0]}\end".as_slice(),
            &["second"][..],
            "[122]",
        ),
        (
            br"\count0=12\pdffilesize{second}\pdffilesize{third}\message{[\the\count0]}\end"
                .as_slice(),
            &["second", "third"][..],
            "[1222]",
        ),
    ] {
        let (preloaded_terminal, preloaded_requests) = run_pdftex_file_probe_job(source, resources);
        assert!(preloaded_requests.is_empty());
        assert!(
            preloaded_terminal.contains(expected),
            "{preloaded_terminal:?}"
        );

        let (staged_terminal, staged_requests) = run_pdftex_file_probe_job(source, &[]);
        assert_eq!(staged_requests, resources);
        assert_eq!(staged_terminal, preloaded_terminal);
    }

    let source = br"\dimen0=12\pdffilesize{second}pt\message{[\the\dimen0]}\end";
    let (preloaded_terminal, preloaded_requests) = run_pdftex_file_probe_job(source, &["second"]);
    assert!(preloaded_requests.is_empty());
    assert!(
        preloaded_terminal.contains("[122.0pt]"),
        "{preloaded_terminal:?}"
    );
    let (staged_terminal, staged_requests) = run_pdftex_file_probe_job(source, &[]);
    assert_eq!(staged_requests, ["second"]);
    assert_eq!(staged_terminal, preloaded_terminal);

    let source =
        br"\skip0=1\pdffilesize{second}pt plus 3\pdffilesize{third}fil\message{[\the\skip0]}\end";
    let resources = &["second", "third"];
    let (preloaded_terminal, preloaded_requests) = run_pdftex_file_probe_job(source, resources);
    assert!(preloaded_requests.is_empty());
    assert!(
        preloaded_terminal.contains("[12.0pt plus 32.0fil]"),
        "{preloaded_terminal:?}"
    );
    let (staged_terminal, staged_requests) = run_pdftex_file_probe_job(source, &[]);
    assert_eq!(staged_requests, resources);
    assert_eq!(staged_terminal, preloaded_terminal);
}

#[test]
fn dimension_fraction_unit_and_internal_second_operand_resume_exact_phases() {
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
        let (preloaded_terminal, preloaded_requests) = run_pdftex_file_probe_job(source, resources);
        assert!(preloaded_requests.is_empty());
        assert!(
            preloaded_terminal.contains(expected),
            "{preloaded_terminal:?}"
        );

        let (staged_terminal, staged_requests) = run_pdftex_file_probe_job(source, &[]);
        assert_eq!(staged_requests, resources);
        assert_eq!(staged_terminal, preloaded_terminal);
    }
}

fn run_pdftex_file_probe_job(source: &[u8], preloaded: &[&str]) -> (String, Vec<String>) {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = pdftex_initex(stores);
        let resource = |name: &str| {
            let bytes: &[u8] = match name {
                "first" => b"ABCD",
                "second" => b"AB",
                "third" => b"CD",
                "4" => b"EF",
                other => panic!("unexpected file enquiry {other:?}"),
            };
            tex_command::FileEnquiryResource::new(
                SourceRegistration::new(RegisteredSourceKind::Generated, Arc::<[u8]>::from(bytes)),
                None,
            )
        };
        for name in preloaded {
            control
                .capabilities_mut()
                .register_input_probe(*name, resource(name));
        }
        register_source(&mut control, source);

        let mut requested = Vec::new();
        let mut ledger = crate::OutputLedger::new();
        let mut checkpoints = Vec::new();
        let cancellation = crate::Cancellation::new();
        let terminal = loop {
            match crate::CanonicalStepRunner::new(&mut control, stores, &mut ledger)
                .step(&mut checkpoints, &cancellation)
            {
                crate::CanonicalStepResult::ResourceNeed(
                    need @ ResourceNeed::InputProbe { .. },
                ) => {
                    let request = match &need {
                        ResourceNeed::InputProbe { request } => request.clone(),
                        _ => unreachable!(),
                    };
                    requested.push(request.name.clone());
                    let resource = resource(&request.name);
                    ledger
                        .fulfill(
                            &mut control,
                            &need,
                            crate::ResourceFulfillment::InputProbe { request, resource },
                        )
                        .expect("file-enquiry fulfillment matches the suspended request");
                }
                crate::CanonicalStepResult::Completed(step @ ReplayStep::End) => break step,
                crate::CanonicalStepResult::Progress(_)
                | crate::CanonicalStepResult::Committed(_) => {}
                other => panic!("unexpected file-enquiry step: {other:?}"),
            }
        };
        assert_eq!(control.pending_resource_site(), None);
        assert!(
            control.command.named_boundary_is_quiescent(),
            "terminal command continuation remained live after preloading {preloaded:?}: {}",
            terminal_text(stores)
        );
        ledger
            .terminal_receipt(&control, terminal)
            .expect("fulfilled file enquiries leave terminal completion quiescent");
        (terminal_text(stores), requested)
    })
}

#[test]
fn expanding_retry_settles_before_resuming_nested_expanded_scanner() {
    // TeX82 §§380 and 473--479: once the retained expandable preflight has
    // settled to `\edef`, that command owns retry through its operand scan.
    // pdfTeX §§495/1535 then resumes the outer macro-definition collector
    // before its nested `\expanded` collector in exact LIFO order.
    let source = br"\def\afterfirst#1{\edef\result{\expanded{\pdffiledump length 2{second}}}}\expandafter\afterfirst\pdffilesize{first}\message{[\result]}\end";

    let (preloaded_terminal, preloaded_requests) =
        run_pdftex_file_probe_job(source, &["first", "second"]);
    assert!(preloaded_requests.is_empty());
    assert!(preloaded_terminal.contains("[4142]"));

    let (staged_terminal, staged_requests) = run_pdftex_file_probe_job(source, &[]);
    assert_eq!(staged_requests, ["first", "second"]);
    assert_eq!(staged_terminal, preloaded_terminal);
}

#[test]
fn directly_delivered_edef_resumes_its_inner_expanded_scanner() {
    // Negative control: without the earlier expanding-preflight suspension,
    // the directly delivered `\edef` already owns the nested scanner retry.
    let source = br"\edef\result{\expanded{\pdffiledump length 2{second}}}\message{[\result]}\end";

    let (preloaded_terminal, preloaded_requests) = run_pdftex_file_probe_job(source, &["second"]);
    assert!(preloaded_requests.is_empty());
    assert!(preloaded_terminal.contains("[4142]"));

    let (staged_terminal, staged_requests) = run_pdftex_file_probe_job(source, &[]);
    assert_eq!(staged_requests, ["second"]);
    assert_eq!(staged_terminal, preloaded_terminal);

    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = pdftex_initex(stores);
        register_source(&mut control, source);
        loop {
            if matches!(
                control.advance_episode(stores).expect("alignment advances"),
                StepResult::Suspended(ResourceNeed::InputProbe { .. })
            ) {
                break;
            }
        }
        assert!(matches!(
            control.pending_direct_operation.as_ref(),
            Some(PendingDirectOperation {
                state: PendingDirectState::Retained(_),
                destination: PendingDirectDestination::Frame(frame),
            }) if frame.frame.episode.as_ref().is_some_and(|episode| episode.scanner.is_some())
                && frame.frame.episode.as_ref().is_some_and(CommandEpisode::is_command_scan)
                && matches!(
                    frame.frame.episode.as_ref().expect("suspended frame owns its episode").current().meaning(),
                    ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                        UnexpandablePrimitive::Edef
                    ))
                )
        ));
    });
}

#[cfg(feature = "profiling")]
fn ordinary_command_episode_evidence(
    repetitions: usize,
) -> (
    tex_state::measurement::HotCoreAllocationMeasurement,
    usize,
    usize,
    usize,
    u64,
) {
    let owner = tex_state::measurement::HotCoreAllocationOwner::DeliveryAndScan;
    let before = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
    let frames_before = operation_frame_constructions();
    let mut scalar_transitions = 0;
    let mut whole_frame_copies = 0;
    let mut overlapping_frame_moves = 0;
    {
        let _scope = tex_state::measurement::hot_core_allocation_scope(owner);
        let mut frame = CommandEpisode::<()>::default();
        let stationary_address = std::ptr::addr_of!(frame);
        for _ in 0..repetitions {
            frame.admit_immediate_pdf(UnexpandablePrimitive::PdfObject);
            assert!(matches!(
                frame.phase,
                Some(PreflightCommandPhase::ImmediatePdfRetry(
                    UnexpandablePrimitive::PdfObject
                ))
            ));
            scalar_transitions += 1;
            whole_frame_copies += usize::from(std::ptr::addr_of!(frame) != stationary_address);
            frame.clear_preflight();
            frame.assert_empty();
            scalar_transitions += 1;
            whole_frame_copies += usize::from(std::ptr::addr_of!(frame) != stationary_address);
            frame.write_hot(hot_apply::HotOperation::end_ordinary_group());
            frame.assert_hot_only();
            scalar_transitions += 1;
            whole_frame_copies += usize::from(std::ptr::addr_of!(frame) != stationary_address);
            let _ = frame.hot_mut();
            frame.hot = None;
            frame.assert_empty();
            scalar_transitions += 1;
            whole_frame_copies += usize::from(std::ptr::addr_of!(frame) != stationary_address);
        }
        // Every ordinary transition above mutates the same nonoverlapping
        // destination in place. Only typed suspension code moves a complete
        // frame, so the ordinary-path memmove-shaped count is exactly zero.
        overlapping_frame_moves += 0;
    }
    let after = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
    let frames_after = operation_frame_constructions();
    (
        tex_state::measurement::HotCoreAllocationMeasurement {
            calls: after.calls - before.calls,
            requested_bytes: after.requested_bytes - before.requested_bytes,
        },
        scalar_transitions,
        whole_frame_copies,
        overlapping_frame_moves,
        frames_after - frames_before,
    )
}

#[cfg(feature = "profiling")]
fn suspended_operation_frame_evidence(
    repetitions: usize,
) -> (
    tex_state::measurement::HotCoreAllocationMeasurement,
    u64,
    u64,
) {
    let owner = tex_state::measurement::HotCoreAllocationOwner::DeliveryAndScan;
    let allocations_before = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
    let frames_before = operation_frame_constructions();
    let mut checksum = 0_u64;
    {
        let _scope = tex_state::measurement::hot_core_allocation_scope(owner);
        for index in 0..repetitions {
            let mut episode = CommandEpisode::<()>::default();
            episode.admit_immediate_pdf(UnexpandablePrimitive::PdfObject);
            let frame = OperationFrame::new(episode, ColdOperationSlot::default());
            let (mut resumed, cold) = std::hint::black_box(frame).into_parts();
            checksum = checksum.wrapping_add(
                resumed
                    .phase
                    .is_some()
                    .then_some((index as u64).rotate_left(11))
                    .unwrap_or_default(),
            );
            resumed.clear_preflight();
            resumed.assert_empty();
            assert!(cold.operation.is_none());
        }
    }
    let allocations_after = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
    let frames_after = operation_frame_constructions();
    (
        tex_state::measurement::HotCoreAllocationMeasurement {
            calls: allocations_after.calls - allocations_before.calls,
            requested_bytes: allocations_after.requested_bytes - allocations_before.requested_bytes,
        },
        frames_after - frames_before,
        std::hint::black_box(checksum),
    )
}

#[cfg(feature = "profiling")]
fn resident_cold_scan_evidence(
    repetitions: usize,
) -> (
    tex_state::measurement::HotCoreAllocationMeasurement,
    usize,
    usize,
    usize,
    u64,
) {
    let owner = tex_state::measurement::HotCoreAllocationOwner::DeliveryAndScan;
    let before = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
    let mut scalar_transitions = 0;
    let mut address_changes = 0;
    let mut overlapping_leaf_moves = 0;
    let mut checksum = 0_u64;
    {
        let _scope = tex_state::measurement::hot_core_allocation_scope(owner);
        let mut frame = CommandEpisode::<()>::default();
        let mut cold = ColdOperationSlot::<()>::default();
        let stationary_frame = std::ptr::addr_of!(frame);
        let stationary_leaf = std::ptr::addr_of!(cold);
        for index in 0..repetitions {
            let index = std::hint::black_box(index + 1);
            write_cold_scan!(
                cold,
                ColdOperation::Count {
                    index: (index & 0xff) as u16,
                    value: index as i32,
                    global: index & 1 != 0,
                }
            );
            frame.mark_resident_cold(&cold);
            scalar_transitions += 1;
            address_changes += usize::from(std::ptr::addr_of!(cold) != stationary_leaf);
            address_changes += usize::from(std::ptr::addr_of!(frame) != stationary_frame);
            let ColdOperation::Count {
                index,
                value,
                global,
            } = frame.unavailable(&cold)
            else {
                unreachable!("resident cold scan installs its exact typed leaf")
            };
            checksum = checksum.wrapping_add(
                (*index as u64).rotate_left(7)
                    ^ (*value as u64).rotate_left(19)
                    ^ u64::from(*global),
            );
            frame.clear_cold(&mut cold);
            scalar_transitions += 1;
            address_changes += usize::from(std::ptr::addr_of!(cold) != stationary_leaf);
            address_changes += usize::from(std::ptr::addr_of!(frame) != stationary_frame);
            overlapping_leaf_moves += 0;
        }
        frame.assert_empty();
    }
    let after = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
    (
        tex_state::measurement::HotCoreAllocationMeasurement {
            calls: after.calls - before.calls,
            requested_bytes: after.requested_bytes - before.requested_bytes,
        },
        scalar_transitions,
        address_changes,
        overlapping_leaf_moves,
        std::hint::black_box(checksum),
    )
}

#[test]
fn command_episode_and_suspension_frame_have_separate_lifetimes() {
    assert_eq!(
        std::mem::size_of::<OperationFrame<()>>(),
        std::mem::size_of::<Option<CommandEpisode<()>>>()
            + std::mem::size_of::<Option<ColdOperationSlot<()>>>()
    );
    assert!(
        std::mem::size_of::<CommandEpisode<()>>() < std::mem::size_of::<OperationFrame<()>>(),
        "ordinary command episodes do not reserve the suspension-only cold slot"
    );
}

#[cfg(feature = "profiling")]
#[test]
fn one_and_4096_ordinary_episodes_construct_zero_operation_frames() {
    let (one_allocations, one_transitions, one_copies, one_overlapping_moves, one_frames) =
        ordinary_command_episode_evidence(1);
    let (many_allocations, many_transitions, many_copies, many_overlapping_moves, many_frames) =
        ordinary_command_episode_evidence(4_096);

    assert_eq!(one_allocations.calls, 0);
    assert_eq!(one_allocations.requested_bytes, 0);
    assert_eq!(many_allocations.calls, 0);
    assert_eq!(many_allocations.requested_bytes, 0);
    assert_eq!(one_transitions, 4);
    assert_eq!(many_transitions, 16_384);
    assert_eq!(one_copies, 0);
    assert_eq!(many_copies, 0);
    assert_eq!(one_overlapping_moves, 0);
    assert_eq!(many_overlapping_moves, 0);
    assert_eq!(one_frames, 0);
    assert_eq!(many_frames, 0);
}

#[cfg(feature = "profiling")]
#[test]
fn one_and_4096_suspensions_construct_exactly_one_frame_each() {
    let (one_allocations, one_frames, one_checksum) = suspended_operation_frame_evidence(1);
    let (many_allocations, many_frames, many_checksum) = suspended_operation_frame_evidence(4_096);

    assert_eq!(one_allocations.calls, 0);
    assert_eq!(one_allocations.requested_bytes, 0);
    assert_eq!(many_allocations.calls, 0);
    assert_eq!(many_allocations.requested_bytes, 0);
    assert_eq!(one_frames, 1);
    assert_eq!(many_frames, 4_096);
    assert_ne!(one_checksum, many_checksum);
}

#[cfg(feature = "profiling")]
fn executed_relax_frame_count(repetitions: usize) -> u64 {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut source = Vec::with_capacity(repetitions * b"\\relax".len());
        for _ in 0..repetitions {
            source.extend_from_slice(br"\relax");
        }
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, &source);
        let before = operation_frame_constructions();
        for _ in 0..repetitions {
            assert_eq!(
                control.advance(stores).expect("relax executes"),
                StepResult::Progress(MainControlStep::Continue)
            );
        }
        operation_frame_constructions() - before
    })
}

#[cfg(feature = "profiling")]
#[test]
fn one_and_4096_executed_simple_primitives_use_zero_suspension_frames() {
    assert_eq!(executed_relax_frame_count(1), 0);
    assert_eq!(executed_relax_frame_count(4_096), 0);
}

#[cfg(feature = "profiling")]
#[test]
fn synchronous_assignment_mode_and_list_families_preserve_semantics_without_frames() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\count0=7\advance\count0 by5\begingroup\count0=2\endgroup\setbox0=\hbox{\kern1pt}\end",
        );
        let before = operation_frame_constructions();
        run_to_end(&mut control, stores);

        assert_eq!(stores.count(0).expect("count register"), 12);
        assert!(stores.copy_box_to_page(0).is_some());
        assert_eq!(control.modes.current_mode(), Mode::Vertical);
        assert_eq!(operation_frame_constructions() - before, 0);
    });
}

#[cfg(feature = "profiling")]
#[test]
fn one_and_4096_cold_scan_cycles_are_allocation_free_and_stationary() {
    let (one_allocations, one_transitions, one_address_changes, one_moves, one_checksum) =
        resident_cold_scan_evidence(1);
    let (many_allocations, many_transitions, many_address_changes, many_moves, many_checksum) =
        resident_cold_scan_evidence(4_096);

    assert_eq!(one_allocations.calls, 0);
    assert_eq!(one_allocations.requested_bytes, 0);
    assert_eq!(many_allocations.calls, 0);
    assert_eq!(many_allocations.requested_bytes, 0);
    assert_eq!(one_transitions, 2);
    assert_eq!(many_transitions, 8_192);
    assert_eq!(one_address_changes, 0);
    assert_eq!(many_address_changes, 0);
    assert_eq!(one_moves, 0);
    assert_eq!(many_moves, 0);
    assert_ne!(one_checksum, many_checksum);
}

#[test]
fn post_apply_facts_preserve_character_font_and_page_output_decisions() {
    // TeX82 §§552/1030/1034/1036: the post-apply handoff distinguishes a
    // present character from nullfont's empty range, preserves an interrupted
    // fetch's existing parking, and carries §1012's selected break without
    // keeping the admitted command facade alive.
    const CMR10: &[u8] = include_bytes!("../../../tex-fonts/tests/fixtures/cm/cmr10.tfm");
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
fn pdf_glyph_to_unicode_operands_resume_their_exact_destinations() {
    let source = br"\pdfglyphtounicode{\pdffiledump length 2{second}}{\pdffiledump length 2{third}}\message{[done]}\end";

    let (preloaded_terminal, preloaded_requests) =
        run_pdftex_file_probe_job(source, &["second", "third"]);
    assert!(preloaded_requests.is_empty());

    let (staged_terminal, staged_requests) = run_pdftex_file_probe_job(source, &[]);
    assert_eq!(staged_requests, ["second", "third"]);
    assert_eq!(staged_terminal, preloaded_terminal);
}

#[test]
fn pdf_start_link_action_resumes_its_exact_destination() {
    let source =
        br"\pdfoutput=1 A\pdfstartlink goto name{\pdffiledump length 2{second}}B\pdfendlink\end";

    let (preloaded_terminal, preloaded_requests) = run_pdftex_file_probe_job(source, &["second"]);
    assert!(preloaded_requests.is_empty());

    let (staged_terminal, staged_requests) = run_pdftex_file_probe_job(source, &[]);
    assert_eq!(staged_requests, ["second"]);
    assert_eq!(staged_terminal, preloaded_terminal);
}

#[test]
fn pdf_xform_optional_texts_resume_their_exact_destinations() {
    let source = br"\pdfoutput=1 \setbox0=\hbox{A}\pdfxform attr{\pdffiledump length 2{second}} resources{\pdffiledump length 2{third}}0\message{[done]}\end";

    let (preloaded_terminal, preloaded_requests) =
        run_pdftex_file_probe_job(source, &["second", "third"]);
    assert!(preloaded_requests.is_empty());

    let (staged_terminal, staged_requests) = run_pdftex_file_probe_job(source, &[]);
    assert_eq!(staged_requests, ["second", "third"]);
    assert_eq!(staged_terminal, preloaded_terminal);
}

#[test]
fn scalar_optional_space_keeps_a_nested_scanner_as_its_child() {
    // The nested csname/expandafter shape matches LaTeX's format-time
    // primitive-name construction. `\number` has finished its scalar before
    // its optional-space lookahead enters the suspended `\expanded` scanner.
    let source = br"\edef\result{\csname outer\expandafter\csname inner\expandafter\expandafter\expandafter\number1\expanded{\unexpanded{A}\pdffiledump length 2{second}}\endcsname\endcsname}\message{[done]}\end";

    let (preloaded_terminal, preloaded_requests) = run_pdftex_file_probe_job(source, &["second"]);
    assert!(preloaded_requests.is_empty());
    assert!(
        preloaded_terminal.contains("[done]"),
        "{preloaded_terminal:?}"
    );

    let (staged_terminal, staged_requests) = run_pdftex_file_probe_job(source, &[]);
    assert_eq!(staged_requests, ["second"]);
    assert_eq!(staged_terminal, preloaded_terminal);
}

#[test]
fn nested_file_enquiries_resume_their_typed_owners() {
    let source = br"\edef\result{\pdfmdfivesum file{\pdffilesize{first}}}\message{[\result]}\end";

    let (preloaded_terminal, preloaded_requests) =
        run_pdftex_file_probe_job(source, &["first", "4"]);
    assert!(preloaded_requests.is_empty());
    assert!(preloaded_terminal.contains("[2C9B682412689D6723E3B31653B5774C]"));

    let (staged_terminal, staged_requests) = run_pdftex_file_probe_job(source, &[]);
    assert_eq!(staged_requests, ["first", "4"]);
    assert_eq!(staged_terminal, preloaded_terminal);
}

#[test]
fn pdf_match_operands_resume_their_exact_destinations() {
    let source = br"\edef\result{\pdfmatch{\pdffiledump length 2{second}}{\pdffiledump length 2{third}}}\message{[\result]}\end";

    let (preloaded_terminal, preloaded_requests) =
        run_pdftex_file_probe_job(source, &["second", "third"]);
    assert!(preloaded_requests.is_empty());

    let (staged_terminal, staged_requests) = run_pdftex_file_probe_job(source, &[]);
    assert_eq!(staged_requests, ["second", "third"]);
    assert_eq!(staged_terminal, preloaded_terminal);
}

#[test]
fn pdf_object_optional_texts_resume_their_exact_destinations() {
    let source = br"\pdfoutput=1 \pdfobj stream attr{\pdffiledump length 2{second}}{\pdffiledump length 2{third}}\message{[done]}\end";

    let (preloaded_terminal, preloaded_requests) =
        run_pdftex_file_probe_job(source, &["second", "third"]);
    assert!(preloaded_requests.is_empty());

    let (staged_terminal, staged_requests) = run_pdftex_file_probe_job(source, &[]);
    assert_eq!(staged_requests, ["second", "third"]);
    assert_eq!(staged_terminal, preloaded_terminal);
}

#[test]
fn pdf_outline_texts_resume_their_exact_destinations() {
    let source = br"\pdfoutput=1 \pdfoutline attr{\pdffiledump length 2{first}} goto name{\pdffiledump length 2{second}} count 1 {\pdffiledump length 2{third}}\message{[done]}\end";

    let (preloaded_terminal, preloaded_requests) =
        run_pdftex_file_probe_job(source, &["first", "second", "third"]);
    assert!(preloaded_requests.is_empty());

    let (staged_terminal, staged_requests) = run_pdftex_file_probe_job(source, &[]);
    assert_eq!(staged_requests, ["first", "second", "third"]);
    assert_eq!(staged_terminal, preloaded_terminal);
}

#[test]
fn pdf_catalog_text_and_action_resume_their_exact_destinations() {
    let source = br"\pdfoutput=1 \pdfcatalog{\pdffiledump length 2{second}} openaction goto name{\pdffiledump length 2{third}}\message{[done]}\end";

    let (preloaded_terminal, preloaded_requests) =
        run_pdftex_file_probe_job(source, &["second", "third"]);
    assert!(preloaded_requests.is_empty());

    let (staged_terminal, staged_requests) = run_pdftex_file_probe_job(source, &[]);
    assert_eq!(staged_requests, ["second", "third"]);
    assert_eq!(staged_terminal, preloaded_terminal);
}

#[test]
fn pdf_graphics_payloads_resume_their_exact_destinations() {
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
        let (preloaded_terminal, preloaded_requests) =
            run_pdftex_file_probe_job(source, resources);
        assert!(preloaded_requests.is_empty());

        let (staged_terminal, staged_requests) = run_pdftex_file_probe_job(source, &[]);
        assert_eq!(staged_requests, resources);
        assert_eq!(staged_terminal, preloaded_terminal);
    }
}

#[test]
fn settled_alignment_scanner_retry_has_one_exact_operation_destination() {
    // Alignment interception replaces the generic settled-command retry: the
    // alignment destination owns both the delivery cursor and `\edef`'s live
    // file-enquiry scanner. Retaining both destinations would let two callers
    // reuse one command-operation coordinate after the first caller commits.
    let source = br"\setbox0=\vbox{\halign{#\cr \edef\result{\pdffiledump length 2{second}}\message{[\result]}\cr}}\end";

    let (preloaded_terminal, preloaded_requests) = run_pdftex_file_probe_job(source, &["second"]);
    assert!(preloaded_requests.is_empty());
    assert!(preloaded_terminal.contains("[4142]"));

    let (staged_terminal, staged_requests) = run_pdftex_file_probe_job(source, &[]);
    assert_eq!(staged_requests, ["second"]);
    assert_eq!(staged_terminal, preloaded_terminal);
}

#[test]
fn expandafter_child_completion_resumes_its_owning_expanded_collector() {
    // The outer macro-definition collector owns `\expandafter`; that frame
    // owns its second-command `\expanded` invocation; and the nested scanner
    // owns each file-enquiry expansion. Two host suspensions force the exact
    // child edge to be consumed and reinstalled more than once before the
    // outer scanner may retire its attempt scope.
    let source = br"\edef\result{\expandafter Q\expanded{\unexpanded{U}\pdffiledump length 2{second}\pdffiledump length 2{third}}}\message{[\result]}\end";

    let (preloaded_terminal, preloaded_requests) =
        run_pdftex_file_probe_job(source, &["second", "third"]);
    assert!(preloaded_requests.is_empty());
    assert!(preloaded_terminal.contains("[QU41424344]"));

    let (staged_terminal, staged_requests) = run_pdftex_file_probe_job(source, &[]);
    assert_eq!(staged_requests, ["second", "third"]);
    assert_eq!(staged_terminal, preloaded_terminal);
}

#[test]
fn pdfstrcmp_right_operand_resumes_its_exact_child_scanner() {
    for source in [
        br"\edef\result{\pdfstrcmp{\pdffiledump length 2{second}}{right}}\message{[\result]}\end"
            .as_slice(),
        br"\edef\result{\pdfstrcmp{left}{\pdffiledump length 2{second}}}\message{[\result]}\end"
            .as_slice(),
    ] {
        let (preloaded_terminal, preloaded_requests) =
            run_pdftex_file_probe_job(source, &["second"]);
        assert!(preloaded_requests.is_empty());

        let (staged_terminal, staged_requests) = run_pdftex_file_probe_job(source, &[]);
        assert_eq!(staged_requests, ["second"]);
        assert_eq!(staged_terminal, preloaded_terminal);
    }
}

#[test]
fn resource_retry_fuel_abort_releases_its_scanner_child() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = pdftex_initex(stores);
        register_source(
            &mut control,
            br"\edef\result{\expanded{\pdffiledump length 2{second}}}\end",
        );

        let need = loop {
            match control
                .advance_episode(stores)
                .expect("file enquiry suspends")
            {
                StepResult::Suspended(need @ ResourceNeed::InputProbe { .. }) => break need,
                StepResult::Progress(_) => {}
                other => panic!("unexpected file-enquiry step: {other:?}"),
            }
        };
        let ResourceNeed::InputProbe { request } = &need else {
            unreachable!();
        };
        control.capabilities_mut().register_input_probe(
            request.name.clone(),
            tex_command::FileEnquiryResource::new(
                SourceRegistration::new(
                    RegisteredSourceKind::Generated,
                    Arc::<[u8]>::from(&b"AB"[..]),
                ),
                None,
            ),
        );
        control.set_fuel_limit(1).expect("bounded abort fuel");

        let aborted = control.advance_episode(stores);
        assert!(
            matches!(
                &aborted,
                Err(ExecError::Command(CommandError::FuelExhausted { .. }))
            ) || matches!(
                &aborted,
                Err(ExecError::Captured { error, .. })
                    if matches!(**error, ExecError::Command(CommandError::FuelExhausted { .. }))
            ),
            "unexpected scalar abort: {aborted:?}"
        );
    });
}

#[test]
fn scalar_operation_retry_fuel_abort_releases_parent_and_deepest_child() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = pdftex_initex(stores);
        register_source(
            &mut control,
            br"\count0=12\pdffilesize{second}\message{unreachable}\end",
        );

        let need = loop {
            match control
                .advance_episode(stores)
                .expect("integer operand suspends")
            {
                StepResult::Suspended(need @ ResourceNeed::InputProbe { .. }) => break need,
                StepResult::Progress(_) => {}
                other => panic!("unexpected integer-operand step: {other:?}"),
            }
        };
        let ResourceNeed::InputProbe { request } = &need else {
            unreachable!();
        };
        control.capabilities_mut().register_input_probe(
            request.name.clone(),
            tex_command::FileEnquiryResource::new(
                SourceRegistration::new(
                    RegisteredSourceKind::Generated,
                    Arc::<[u8]>::from(&b"AB"[..]),
                ),
                None,
            ),
        );
        control.set_fuel_limit(1).expect("bounded abort fuel");

        let aborted = control.advance_episode(stores);
        assert!(
            matches!(
                &aborted,
                Err(ExecError::Command(CommandError::FuelExhausted { .. }))
            ) || matches!(
                &aborted,
                Err(ExecError::Captured { error, .. })
                    if matches!(**error, ExecError::Command(CommandError::FuelExhausted { .. }))
            ),
            "unexpected scalar operation abort: {aborted:?}"
        );
        assert!(
            control.pending_direct_operation.is_none(),
            "fuel abort must recursively close the scalar child before its operation parent"
        );
    });
}

#[test]
fn sequential_generated_reference_probes_preserve_the_macro_cursor() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_cmr10_as(&mut control, stores, "cmr10.tfm");
        register_source(
            &mut control,
            include_bytes!("../../../../tests/corpus/stabilization/latex-references/source.tex"),
        );
        let mut requested = Vec::new();
        let mut ledger = crate::OutputLedger::new();
        let mut checkpoints = Vec::new();
        let cancellation = crate::Cancellation::new();
        let mut terminal_step = None;
        for _ in 0..512 {
            let result = crate::CanonicalStepRunner::new(&mut control, stores, &mut ledger)
                .step(&mut checkpoints, &cancellation);
            match result {
                crate::CanonicalStepResult::ResourceNeed(ResourceNeed::InputProbe { request }) => {
                    let (name, bytes): (&str, &[u8]) = match request.name.as_str() {
                        "main.aux" => (
                            "main.aux",
                            br"\newlabel{sec:intro}{{1}{1}}
",
                        ),
                        "main.toc" => (
                            "main.toc",
                            br"\contentsline{section}{Introduction}{1}
",
                        ),
                        other => panic!("unexpected probe {other:?}"),
                    };
                    requested.push(name);
                    let source = SourceRegistration::new(
                        RegisteredSourceKind::Generated,
                        Arc::<[u8]>::from(bytes),
                    );
                    ledger
                        .fulfill(
                            &mut control,
                            &ResourceNeed::InputProbe {
                                request: request.clone(),
                            },
                            crate::ResourceFulfillment::InputProbe {
                                request,
                                resource: tex_command::FileEnquiryResource::new(source, None),
                            },
                        )
                        .expect("probe fulfillment matches");
                }
                crate::CanonicalStepResult::ResourceNeed(need @ ResourceNeed::Input { .. }) => {
                    let name = match &need {
                        ResourceNeed::Input { name, .. } => name.clone(),
                        _ => unreachable!(),
                    };
                    let bytes: &[u8] = match name.as_str() {
                        "main.aux" => {
                            br"\newlabel{sec:intro}{{1}{1}}
"
                        }
                        "main.toc" => {
                            br"\contentsline{section}{Introduction}{1}
"
                        }
                        other => panic!("unexpected input {other:?}"),
                    };
                    ledger
                        .fulfill(
                            &mut control,
                            &need,
                            crate::ResourceFulfillment::Input {
                                name,
                                source: SourceRegistration::new(
                                    RegisteredSourceKind::Generated,
                                    Arc::<[u8]>::from(bytes),
                                ),
                            },
                        )
                        .expect("input fulfillment matches");
                }
                crate::CanonicalStepResult::Completed(step @ ReplayStep::End) => {
                    terminal_step = Some(step);
                    break;
                }
                crate::CanonicalStepResult::Progress(_)
                | crate::CanonicalStepResult::Committed(_) => {}
                other => panic!("unexpected reference step {other:?}"),
            }
        }
        assert_eq!(requested, ["main.aux", "main.toc"]);
        assert_eq!(control.pending_resource_site(), None);
        ledger
            .terminal_receipt(&control, terminal_step.expect("terminal step"))
            .expect("answered probes leave terminal completion quiescent");
    });
}

#[test]
fn unavailable_input_probe_releases_its_diagnostic_site_before_terminal_close() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = pdftex_initex(stores);
        register_source(
            &mut control,
            br"\message{[\pdffilesize{missing-resource}]}\end",
        );
        let mut ledger = crate::OutputLedger::new();
        let mut checkpoints = Vec::new();
        let cancellation = crate::Cancellation::new();
        let terminal = loop {
            match crate::CanonicalStepRunner::new(&mut control, stores, &mut ledger)
                .step(&mut checkpoints, &cancellation)
            {
                crate::CanonicalStepResult::ResourceNeed(
                    need @ ResourceNeed::InputProbe { .. },
                ) => {
                    assert!(control.pending_resource_site().is_some());
                    ledger.mark_unavailable(&mut control, &need, false);
                    assert_eq!(control.pending_resource_site(), None);
                }
                crate::CanonicalStepResult::Completed(step) => break step,
                crate::CanonicalStepResult::Progress(_)
                | crate::CanonicalStepResult::Committed(_) => {}
                other => panic!("unexpected unavailable-probe step: {other:?}"),
            }
        };
        ledger
            .terminal_receipt(&control, terminal)
            .expect("unavailable probe leaves terminal completion quiescent");
    });
}

const PREFIXED_DEFINITION_RESOURCE_SOURCE: &[u8] = br"
\protected\long\def\gsetNpx{\protected\outer\long\global\edef}
\long\def\expArgsNc#1#2{\expandafter#1\csname#2\endcsname}
\protected\def\gsetCpx{\expArgsNc\gsetNpx}
\gsetCpx{result\pdffilesize{first}}{\pdffilesize{second}}
\end
";

fn register_file_size_probe<G>(
    control: &mut MainControl<G>,
    need: &ResourceNeed,
    bytes: &'static [u8],
) {
    let ResourceNeed::InputProbe { request } = need else {
        panic!("expected file-size probe, got {need:?}");
    };
    control.capabilities_mut().register_input_probe(
        request.name.clone(),
        tex_command::FileEnquiryResource::new(
            SourceRegistration::new(RegisteredSourceKind::Generated, Arc::<[u8]>::from(bytes)),
            None,
        ),
    );
}

fn next_input_probe<G>(control: &mut MainControl<G>, stores: &mut Universe<G>) -> ResourceNeed {
    loop {
        match control
            .advance_episode(stores)
            .expect("prefixed definition probe suspends")
        {
            StepResult::Suspended(need @ ResourceNeed::InputProbe { .. }) => return need,
            StepResult::Progress(_) => {}
            other => panic!("unexpected prefixed-definition step: {other:?}"),
        }
    }
}

#[test]
fn prefixed_definition_scanner_resumes_its_exact_substantive_command() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = pdftex_initex(stores);
        register_source(&mut control, PREFIXED_DEFINITION_RESOURCE_SOURCE);

        let first = next_input_probe(&mut control, stores);
        assert!(matches!(
            &first,
            ResourceNeed::InputProbe { request } if request.name == "first"
        ));
        register_file_size_probe(&mut control, &first, b"ABCD");

        let second = next_input_probe(&mut control, stores);
        assert!(matches!(
            &second,
            ResourceNeed::InputProbe { request } if request.name == "second"
        ));
        register_file_size_probe(&mut control, &second, b"AB");
        run_to_end(&mut control, stores);

        admitted!(stores, |context| {
            let result = context.intern_control_sequence("result4");
            let ResolvedMeaning::Macro { definition, flags } = context.meaning(result) else {
                panic!("resumed prefixed definition is installed")
            };
            assert!(flags.contains(MeaningFlags::LONG));
            assert!(flags.contains(MeaningFlags::OUTER));
            assert!(flags.contains(MeaningFlags::PROTECTED));
            let definition = context.definition(definition);
            let replacement = definition.replacement_text();
            assert_eq!(replacement.len(), 1);
            assert_eq!(
                replacement
                    .get(0)
                    .expect("replacement word")
                    .semantic_token(),
                Token::Char {
                    ch: '2',
                    cat: Catcode::Other,
                }
            );
        });
    });
}

#[test]
fn prefix_fetch_resumes_with_earlier_flags_and_its_exact_expansion_child() {
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

        let first = next_input_probe(&mut control, stores);
        register_file_size_probe(&mut control, &first, b"ABCD");
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
fn prefixed_definition_scanner_repeatedly_resuspends_at_the_same_child() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = pdftex_initex(stores);
        register_source(&mut control, PREFIXED_DEFINITION_RESOURCE_SOURCE);

        let first = next_input_probe(&mut control, stores);
        register_file_size_probe(&mut control, &first, b"ABCD");
        let second = next_input_probe(&mut control, stores);
        for _ in 0..3 {
            let repeated = next_input_probe(&mut control, stores);
            assert!(matches!(
                repeated,
                ResourceNeed::InputProbe { request } if request.name == "second"
            ));
        }

        register_file_size_probe(&mut control, &second, b"AB");
        run_to_end(&mut control, stores);
        assert_eq!(macro_character_text(stores, "result4"), "2");
    });
}

#[test]
fn prefixed_definition_scanner_fuel_abort_releases_its_operation_child() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = pdftex_initex(stores);
        register_source(&mut control, PREFIXED_DEFINITION_RESOURCE_SOURCE);

        let first = next_input_probe(&mut control, stores);
        register_file_size_probe(&mut control, &first, b"ABCD");
        let second = next_input_probe(&mut control, stores);
        register_file_size_probe(&mut control, &second, b"AB");
        control.set_fuel_limit(1).expect("bounded abort fuel");

        let aborted = control.advance_episode(stores);
        assert!(
            matches!(
                &aborted,
                Err(ExecError::Command(CommandError::FuelExhausted { .. }))
            ) || matches!(
                &aborted,
                Err(ExecError::Captured { error, .. })
                    if matches!(**error, ExecError::Command(CommandError::FuelExhausted { .. }))
            ),
            "unexpected prefixed-definition abort: {aborted:?}"
        );
        assert!(
            control.pending_direct_operation.is_none(),
            "fuel abort closes the scanner child before its operation parent"
        );
    });
}

#[test]
fn named_boundary_queue_publishes_literal_and_macro_paragraphs_before_the_next_command() {
    for source in [
        br"\count0=1 A\par\count0=2\end".as_slice(),
        br"\count0=1\def\finish{A\par}\finish\count0=2\end".as_slice(),
    ] {
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            let mut control = MainControl::tex82_initex(stores);
            register_cmr10_as(&mut control, stores, "cmr10.tfm");
            register_source(&mut control, source);
            let mut ledger = crate::OutputLedger::new();
            let mut checkpoints = Vec::new();
            let cancellation = crate::Cancellation::new();

            let committed = crate::CanonicalStepRunner::new(&mut control, stores, &mut ledger)
                .step(&mut checkpoints, &cancellation);
            assert!(
                matches!(committed, crate::CanonicalStepResult::Committed(_)),
                "paragraph boundary result: {committed:?}"
            );
            assert_eq!(stores.count(0).expect("count register"), 1);
            assert_eq!(
                checkpoints
                    .iter()
                    .filter(|checkpoint| {
                        checkpoint.boundary() == crate::EngineBoundary::OuterParagraphEnd
                    })
                    .count(),
                1
            );

            for _ in 0..32 {
                if matches!(
                    crate::CanonicalStepRunner::new(&mut control, stores, &mut ledger,)
                        .step(&mut checkpoints, &cancellation),
                    crate::CanonicalStepResult::Completed(_)
                ) {
                    break;
                }
            }
            assert_eq!(stores.count(0).expect("count register"), 2);
            assert_eq!(
                checkpoints
                    .iter()
                    .filter(|checkpoint| {
                        checkpoint.boundary() == crate::EngineBoundary::OuterParagraphEnd
                    })
                    .count(),
                1,
                "one paragraph intent publishes exactly once"
            );
        });
    }
}

fn checkpoint_boundaries<G>(
    checkpoints: &[crate::EngineCheckpoint<G>],
) -> Vec<crate::EngineBoundary> {
    checkpoints
        .iter()
        .map(crate::EngineCheckpoint::boundary)
        .collect()
}

fn assert_source_role_checkpoint_schedule(
    root: SourceRegistration,
    inputs: Vec<(&'static str, SourceRegistration)>,
    expected: &[crate::EngineBoundary],
) {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_cmr10_as(&mut control, stores, "cmr10.tfm");
        for (name, source) in inputs {
            control.capabilities_mut().register_input(name, source);
        }
        control
            .register_root_source(root)
            .expect("root source registers and opens");

        let mut ledger = crate::OutputLedger::new();
        let mut checkpoints = Vec::new();
        ledger
            .commit_job_start(&mut control, stores, &mut checkpoints)
            .expect("JobStart captures");
        let cancellation = crate::Cancellation::new();
        for _ in 0..64 {
            let result = crate::CanonicalStepRunner::new(&mut control, stores, &mut ledger)
                .step(&mut checkpoints, &cancellation);
            assert!(
                !matches!(result, crate::CanonicalStepResult::Failed(_)),
                "checkpoint-origin job failed: {result:?}"
            );
            if matches!(result, crate::CanonicalStepResult::Completed(_)) {
                break;
            }
        }

        assert_eq!(checkpoint_boundaries(&checkpoints), expected);
    });
}

#[test]
fn named_checkpoint_retention_uses_explicit_source_roles() {
    let document_body = Arc::<[u8]>::from(&br"\font\ten=cmr10 \ten A\par\shipout\vbox{}\end"[..]);
    assert_source_role_checkpoint_schedule(
        SourceRegistration::new(RegisteredSourceKind::Generated, Arc::clone(&document_body)),
        Vec::new(),
        &[
            crate::EngineBoundary::JobStart,
            crate::EngineBoundary::OuterParagraphEnd,
            crate::EngineBoundary::ShipoutComplete,
            crate::EngineBoundary::ShipoutComplete,
        ],
    );

    let nested_root = || {
        SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(&br"\font\ten=cmr10 \ten \input child \end"[..]),
        )
    };
    let nested_body = || Arc::<[u8]>::from(&br"A\par\shipout\vbox{}\endinput"[..]);
    for (role, expected) in [
        (
            tex_command::SourceRole::UserDocumentInclude,
            &[
                crate::EngineBoundary::JobStart,
                crate::EngineBoundary::OuterParagraphEnd,
                crate::EngineBoundary::ShipoutComplete,
                crate::EngineBoundary::ShipoutComplete,
            ][..],
        ),
        (
            tex_command::SourceRole::ProjectPackageClass,
            &[
                crate::EngineBoundary::JobStart,
                crate::EngineBoundary::ShipoutComplete,
            ][..],
        ),
        (
            tex_command::SourceRole::DistributionPackageClass,
            &[
                crate::EngineBoundary::JobStart,
                crate::EngineBoundary::ShipoutComplete,
            ][..],
        ),
        (
            tex_command::SourceRole::GeneratedInput,
            &[
                crate::EngineBoundary::JobStart,
                crate::EngineBoundary::ShipoutComplete,
            ][..],
        ),
    ] {
        assert_source_role_checkpoint_schedule(
            nested_root(),
            vec![(
                "child.tex",
                SourceRegistration::new(RegisteredSourceKind::Generated, nested_body())
                    .with_role(role),
            )],
            expected,
        );
    }

    assert_source_role_checkpoint_schedule(
        SourceRegistration::new(RegisteredSourceKind::Generated, document_body)
            .with_role(tex_command::SourceRole::FormatInitialization),
        Vec::new(),
        &[crate::EngineBoundary::JobStart],
    );
}

#[test]
fn nested_sources_inherit_package_role_and_package_return_restores_document_policy() {
    assert_source_role_checkpoint_schedule(
        SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(&br"\font\ten=cmr10 \ten \input package \end"[..]),
        ),
        vec![
            (
                "package.tex",
                SourceRegistration::new(
                    RegisteredSourceKind::Generated,
                    Arc::<[u8]>::from(&br"\input helper \endinput"[..]),
                )
                .with_role(tex_command::SourceRole::ProjectPackageClass),
            ),
            (
                "helper.tex",
                SourceRegistration::new(
                    RegisteredSourceKind::Generated,
                    Arc::<[u8]>::from(&br"A\par\shipout\vbox{}\endinput"[..]),
                ),
            ),
        ],
        &[
            crate::EngineBoundary::JobStart,
            crate::EngineBoundary::ShipoutComplete,
        ],
    );

    assert_source_role_checkpoint_schedule(
        SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(
                &br"\font\ten=cmr10 \ten \def\finish{A\par\input package}\finish\shipout\vbox{}\end"[..],
            ),
        ),
        vec![(
            "package.tex",
            SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(&br"\endinput"[..]),
            )
            .with_role(tex_command::SourceRole::ProjectPackageClass),
        )],
        &[
            crate::EngineBoundary::JobStart,
            crate::EngineBoundary::OuterParagraphEnd,
            crate::EngineBoundary::ShipoutComplete,
            crate::EngineBoundary::ShipoutComplete,
        ],
    );
}

#[test]
fn nested_shipout_origin_stays_frozen_across_return_and_resource_resume() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        control.capabilities_mut().register_input(
            "child.tex",
            SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(&br"\shipout\vbox{}\endinput"[..]),
            )
            .with_role(tex_command::SourceRole::ProjectPackageClass),
        );
        register_source(
            &mut control,
            br"\begingroup\input child \input missing \endgroup\shipout\vbox{}\end",
        );

        let mut ledger = crate::OutputLedger::new();
        let mut checkpoints = Vec::new();
        ledger
            .commit_job_start(&mut control, stores, &mut checkpoints)
            .expect("JobStart captures");
        let cancellation = crate::Cancellation::new();
        let need = loop {
            match crate::CanonicalStepRunner::new(&mut control, stores, &mut ledger)
                .step(&mut checkpoints, &cancellation)
            {
                crate::CanonicalStepResult::ResourceNeed(need @ ResourceNeed::Input { .. }) => {
                    break need;
                }
                crate::CanonicalStepResult::Progress(_)
                | crate::CanonicalStepResult::Committed(_) => {}
                other => panic!("unexpected pre-suspension step: {other:?}"),
            }
        };
        assert_eq!(
            checkpoint_boundaries(&checkpoints),
            [crate::EngineBoundary::JobStart]
        );
        assert_eq!(control.pending_named_boundaries.len(), 1);
        assert_eq!(
            control.pending_named_boundaries.front(),
            Some(&PendingNamedBoundary {
                boundary: crate::EngineBoundary::ShipoutComplete,
                source_role: Some(tex_command::SourceRole::ProjectPackageClass),
            })
        );
        assert_eq!(
            control.command.current_file_source_id(),
            control.root_main_source,
            "resource suspension occurs after nested input retirement returned to root"
        );

        let name = match &need {
            ResourceNeed::Input { name, .. } => name.clone(),
            _ => unreachable!(),
        };
        ledger
            .fulfill(
                &mut control,
                &need,
                crate::ResourceFulfillment::Input {
                    name,
                    source: SourceRegistration::new(
                        RegisteredSourceKind::Generated,
                        Arc::<[u8]>::from(&b""[..]),
                    ),
                },
            )
            .expect("missing input fulfillment matches");

        for _ in 0..64 {
            let result = crate::CanonicalStepRunner::new(&mut control, stores, &mut ledger)
                .step(&mut checkpoints, &cancellation);
            assert!(
                !matches!(result, crate::CanonicalStepResult::Failed(_)),
                "resumed checkpoint-origin job failed: {result:?}"
            );
            if matches!(result, crate::CanonicalStepResult::Completed(_)) {
                break;
            }
        }
        assert_eq!(
            checkpoint_boundaries(&checkpoints),
            [
                crate::EngineBoundary::JobStart,
                crate::EngineBoundary::ShipoutComplete,
            ],
            "the project-package shipout remains filtered across source return and retry; the later root-document shipout is retained"
        );
    });
}

#[test]
fn terminal_named_boundary_drain_publishes_the_quiescent_suffix_in_order() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\end");
        control
            .pending_named_boundaries
            .push_back(PendingNamedBoundary {
                boundary: crate::EngineBoundary::OuterParagraphEnd,
                source_role: Some(tex_command::SourceRole::RootDocument),
            });
        control
            .pending_named_boundaries
            .push_back(PendingNamedBoundary {
                boundary: crate::EngineBoundary::ShipoutComplete,
                source_role: Some(tex_command::SourceRole::RootDocument),
            });

        control
            .publish_terminal_named_boundaries(stores)
            .expect("quiescent terminal boundaries publish");

        assert!(control.pending_named_boundaries.is_empty());
        assert_eq!(
            control.take_completed_boundaries(),
            [
                crate::EngineBoundary::OuterParagraphEnd,
                crate::EngineBoundary::ShipoutComplete,
            ]
        );
        assert_eq!(
            control
                .take_checkpoint_eligibilities()
                .iter()
                .map(crate::checkpoint::CheckpointEligibility::boundary)
                .collect::<Vec<_>>(),
            [
                crate::EngineBoundary::OuterParagraphEnd,
                crate::EngineBoundary::ShipoutComplete,
            ],
            "approved document roles retain paragraph and shipout restart boundaries"
        );
    });
}

#[test]
fn terminal_named_boundary_without_a_live_root_is_not_restartable() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        control
            .pending_named_boundaries
            .push_back(PendingNamedBoundary {
                boundary: crate::EngineBoundary::ShipoutComplete,
                source_role: Some(tex_command::SourceRole::RootDocument),
            });

        control
            .publish_terminal_named_boundaries(stores)
            .expect("terminal output evidence remains publishable");

        assert!(control.pending_named_boundaries.is_empty());
        assert_eq!(
            control.take_completed_boundaries(),
            [crate::EngineBoundary::ShipoutComplete]
        );
        let eligibilities = control.take_checkpoint_eligibilities();
        assert_eq!(eligibilities.len(), 1);
        assert_eq!(
            eligibilities[0].boundary(),
            crate::EngineBoundary::ShipoutComplete
        );
        assert!(!eligibilities[0].is_restartable());
    });
}

#[test]
fn named_boundary_queue_waits_for_a_live_macro_argument_record() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_cmr10_as(&mut control, stores, "cmr10.tfm");
        register_source(
            &mut control,
            br"\def\finish#1{A\par#1}\finish{\count0=2}\end",
        );
        let mut ledger = crate::OutputLedger::new();
        let mut checkpoints = Vec::new();
        let cancellation = crate::Cancellation::new();

        let committed = crate::CanonicalStepRunner::new(&mut control, stores, &mut ledger)
            .step(&mut checkpoints, &cancellation);
        assert!(
            matches!(committed, crate::CanonicalStepResult::Committed(_)),
            "macro-owned boundary result: {committed:?}"
        );
        assert_eq!(
            stores.count(0).expect("count register"),
            2,
            "the argument remains live and executes before its owner can retire"
        );
        assert_eq!(
            checkpoints
                .iter()
                .filter(|checkpoint| {
                    checkpoint.boundary() == crate::EngineBoundary::OuterParagraphEnd
                })
                .count(),
            1
        );
    });
}

#[test]
fn named_boundary_queue_waits_for_outer_mode_after_a_macro_argument_starts_a_paragraph() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_cmr10_as(&mut control, stores, "cmr10.tfm");
        register_source(
            &mut control,
            br"\def\finish#1{A\par#1}\finish{B}\par\count0=2\end",
        );
        let mut ledger = crate::OutputLedger::new();
        let mut checkpoints = Vec::new();
        let cancellation = crate::Cancellation::new();

        for expected in 1..=2 {
            let result = crate::CanonicalStepRunner::new(&mut control, stores, &mut ledger)
                .step(&mut checkpoints, &cancellation);
            assert!(
                matches!(result, crate::CanonicalStepResult::Committed(_)),
                "delayed paragraph {expected} result: {result:?}"
            );
            assert_eq!(control.current_mode(), Mode::Vertical);
            assert_eq!(stores.count(0).expect("count register"), 0);
            assert_eq!(
                checkpoints
                    .iter()
                    .filter(|checkpoint| {
                        checkpoint.boundary() == crate::EngineBoundary::OuterParagraphEnd
                    })
                    .count(),
                expected,
            );
        }
    });
}

#[test]
fn named_boundary_queue_does_not_cross_a_resource_suspension() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_cmr10_as(&mut control, stores, "cmr10.tfm");
        register_source(
            &mut control,
            br"\def\finish{A\par\input child}\finish\count0=2\end",
        );
        let mut ledger = crate::OutputLedger::new();
        let mut checkpoints = Vec::new();
        let cancellation = crate::Cancellation::new();

        let need = crate::CanonicalStepRunner::new(&mut control, stores, &mut ledger)
            .step(&mut checkpoints, &cancellation);
        let need = match need {
            crate::CanonicalStepResult::ResourceNeed(need @ ResourceNeed::Input { .. }) => need,
            other => panic!("expected input suspension, got {other:?}"),
        };
        assert!(checkpoints.is_empty());
        let name = match &need {
            ResourceNeed::Input { name, .. } => name.clone(),
            _ => unreachable!(),
        };
        ledger
            .fulfill(
                &mut control,
                &need,
                crate::ResourceFulfillment::Input {
                    name,
                    source: SourceRegistration::new(
                        RegisteredSourceKind::Generated,
                        Arc::<[u8]>::from(&b""[..]),
                    ),
                },
            )
            .expect("input fulfillment matches");

        for _ in 0..32 {
            let result = crate::CanonicalStepRunner::new(&mut control, stores, &mut ledger)
                .step(&mut checkpoints, &cancellation);
            if matches!(result, crate::CanonicalStepResult::Committed(_)) {
                break;
            }
            assert!(
                !matches!(result, crate::CanonicalStepResult::Failed(_)),
                "resumed boundary result: {result:?}"
            );
        }
        assert_eq!(stores.count(0).expect("count register"), 0);
        assert_eq!(
            checkpoints
                .iter()
                .filter(|checkpoint| {
                    checkpoint.boundary() == crate::EngineBoundary::OuterParagraphEnd
                })
                .count(),
            1
        );
    });
}

#[test]
fn named_boundary_queue_drains_two_macro_paragraphs_in_order() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_cmr10_as(&mut control, stores, "cmr10.tfm");
        register_source(&mut control, br"\def\two{A\par B\par}\two\count0=3\end");
        let mut ledger = crate::OutputLedger::new();
        let mut checkpoints = Vec::new();
        let cancellation = crate::Cancellation::new();

        for expected in 1..=2 {
            let result = crate::CanonicalStepRunner::new(&mut control, stores, &mut ledger)
                .step(&mut checkpoints, &cancellation);
            assert!(
                matches!(result, crate::CanonicalStepResult::Committed(_)),
                "queued paragraph {expected} result: {result:?}"
            );
            assert_eq!(stores.count(0).expect("count register"), 0);
            assert_eq!(
                checkpoints
                    .iter()
                    .filter(|checkpoint| {
                        checkpoint.boundary() == crate::EngineBoundary::OuterParagraphEnd
                    })
                    .count(),
                expected,
            );
        }
    });
}

#[test]
fn named_boundary_queue_waits_for_macro_wrapped_shipout_content() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_cmr10_as(&mut control, stores, "cmr10.tfm");
        register_source(
            &mut control,
            br"\def\toc{A\par}\def\send{\shipout\vbox{\toc}}\send\count0=4\end",
        );
        let mut ledger = crate::OutputLedger::new();
        let mut checkpoints = Vec::new();
        let cancellation = crate::Cancellation::new();

        for _ in 0..32 {
            let result = crate::CanonicalStepRunner::new(&mut control, stores, &mut ledger)
                .step(&mut checkpoints, &cancellation);
            assert!(
                !matches!(result, crate::CanonicalStepResult::Failed(_)),
                "shipout boundary result: {result:?}"
            );
            if matches!(result, crate::CanonicalStepResult::Completed(_)) {
                break;
            }
        }
        assert_eq!(
            stores.count(0).expect("count register"),
            4,
            "the runner reaches terminal completion after publishing shipout evidence"
        );
        assert_eq!(
            checkpoint_boundaries(&checkpoints),
            [crate::EngineBoundary::ShipoutComplete],
            "the root-document shipout publishes after its macro content unwinds"
        );
    });
}

#[test]
fn named_boundary_queue_drains_mixed_intents_in_producer_order() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_cmr10_as(&mut control, stores, "cmr10.tfm");
        register_source(
            &mut control,
            br"\def\mixed{A\par\shipout\vbox{B\par}}\mixed\count0=5\end",
        );
        let mut ledger = crate::OutputLedger::new();
        let mut checkpoints = Vec::new();
        let cancellation = crate::Cancellation::new();

        for _ in 0..32 {
            let result = crate::CanonicalStepRunner::new(&mut control, stores, &mut ledger)
                .step(&mut checkpoints, &cancellation);
            assert!(
                !matches!(result, crate::CanonicalStepResult::Failed(_)),
                "mixed boundary result: {result:?}"
            );
            if matches!(result, crate::CanonicalStepResult::Completed(_)) {
                break;
            }
        }
        assert_eq!(
            stores.count(0).expect("count register"),
            5,
            "the runner reaches terminal completion after draining both intents"
        );
        assert_eq!(
            checkpoints
                .iter()
                .map(crate::EngineCheckpoint::boundary)
                .collect::<Vec<_>>(),
            [
                crate::EngineBoundary::OuterParagraphEnd,
                crate::EngineBoundary::ShipoutComplete,
                crate::EngineBoundary::ShipoutComplete,
            ]
        );
    });
}

#[test]
fn observed_resource_retry_moves_the_unpublished_prefix_exactly_once() {
    let source = br"\input child\end";
    let child =
        SourceRegistration::new(RegisteredSourceKind::Generated, Arc::<[u8]>::from(&b""[..]));

    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, source);
        let mut retried = ObservationRecorder::default();
        assert!(matches!(
            control
                .advance_with_observer(stores, &mut retried)
                .expect("missing input suspends"),
            StepResult::Suspended(ResourceNeed::Input { ref name, .. }) if name == "child.tex"
        ));
        assert!(
            retried.0.is_empty(),
            "a suspended observed operation publishes no prefix"
        );
        control
            .capabilities_mut()
            .register_input("child.tex", child.clone());
        run_to_end_observed(&mut control, stores, &mut retried);

        crate::test_harness::with_nonstop_plain_universe(|direct_stores| {
            let mut direct = MainControl::tex82_initex(direct_stores);
            direct.capabilities_mut().register_input("child.tex", child);
            register_source(&mut direct, source);
            let mut direct_observations = ObservationRecorder::default();
            run_to_end_observed(&mut direct, direct_stores, &mut direct_observations);

            assert_eq!(retried.0, direct_observations.0);
        });
    });
}

#[test]
fn observed_alignment_resource_retry_resumes_the_exact_delivery_once() {
    let source = br"\setbox0=\vbox{\halign{#\cr \input child\cr}}\end";
    let child = SourceRegistration::new(
        RegisteredSourceKind::Generated,
        Arc::<[u8]>::from(&br"X\endinput"[..]),
    );

    crate::test_harness::with_nonstop_plain_universe(|retried_stores| {
        let mut retried_control = MainControl::tex82_initex(retried_stores);
        register_source(&mut retried_control, source);
        let mut retried = ObservationRecorder::default();
        loop {
            if matches!(
                retried_control
                    .advance_with_observer(retried_stores, &mut retried)
                    .expect("alignment advances to its resource"),
                StepResult::Suspended(ResourceNeed::Input { ref name, .. })
                    if name == "child.tex"
            ) {
                break;
            }
        }
        assert!(
            matches!(
                retried_control.pending_direct_operation.as_ref(),
                Some(PendingDirectOperation {
                    state: PendingDirectState::Retained(_),
                    destination: PendingDirectDestination::Alignment(PendingAlignmentDelivery {
                        scanner: None,
                        expansion: Some(_),
                        ..
                    }),
                })
            ),
            "alignment retry must own only its exact parked expansion key"
        );
        retried_control
            .capabilities_mut()
            .register_input("child.tex", child.clone());
        run_to_end_observed(&mut retried_control, retried_stores, &mut retried);

        crate::test_harness::with_nonstop_plain_universe(|direct_stores| {
            let mut direct_control = MainControl::tex82_initex(direct_stores);
            direct_control
                .capabilities_mut()
                .register_input("child.tex", child);
            register_source(&mut direct_control, source);
            let mut direct = ObservationRecorder::default();
            run_to_end_observed(&mut direct_control, direct_stores, &mut direct);

            if retried.0 != direct.0 {
                let mismatch = retried
                    .0
                    .iter()
                    .zip(&direct.0)
                    .position(|(left, right)| left != right)
                    .unwrap_or(retried.0.len().min(direct.0.len()));
                panic!(
                    "first observation mismatch at {mismatch}: retried={:?} direct={:?}",
                    retried.0.get(mismatch),
                    direct.0.get(mismatch)
                );
            }
            assert_eq!(
                retried_control.advance_telemetry().maximum_live_savepoints,
                0
            );
        });
    });
}

#[test]
fn alignment_preamble_span_expansion_resumes_its_resource_child() {
    for (source, resources) in [
        (
            br"\setbox0=\vbox{\halign{\span\pdffiledump length 2{second}#\cr X\cr}}\end"
                .as_slice(),
            &["second"][..],
        ),
        (
            br"\setbox0=\vbox{\halign{\span\expanded{\pdffiledump length 2{second}\pdffiledump length 2{third}}#\cr X\cr}}\end"
                .as_slice(),
            &["second", "third"][..],
        ),
    ] {
        let (preloaded_terminal, preloaded_requests) =
            run_pdftex_file_probe_job(source, resources);
        assert!(preloaded_requests.is_empty());

        let (staged_terminal, staged_requests) = run_pdftex_file_probe_job(source, &[]);
        assert_eq!(staged_requests, resources);
        assert_eq!(staged_terminal, preloaded_terminal);
    }
}

#[test]
fn alignment_preamble_span_expansion_abort_releases_its_resource_child() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = pdftex_initex(stores);
        register_source(
            &mut control,
            br"\setbox0=\vbox{\halign{\span\pdffiledump length 2{second}#\cr X\cr}}\end",
        );

        let need = loop {
            match control
                .advance_episode(stores)
                .expect("preamble file enquiry suspends")
            {
                StepResult::Suspended(need @ ResourceNeed::InputProbe { .. }) => break need,
                StepResult::Progress(_) => {}
                other => panic!("unexpected preamble file-enquiry step: {other:?}"),
            }
        };
        let ResourceNeed::InputProbe { request } = &need else {
            unreachable!();
        };
        control.capabilities_mut().register_input_probe(
            request.name.clone(),
            tex_command::FileEnquiryResource::new(
                SourceRegistration::new(
                    RegisteredSourceKind::Generated,
                    Arc::<[u8]>::from(&b"AB"[..]),
                ),
                None,
            ),
        );
        control.set_fuel_limit(1).expect("bounded abort fuel");

        let aborted = control.advance_episode(stores);
        assert!(
            matches!(
                &aborted,
                Err(ExecError::Command(CommandError::FuelExhausted { .. }))
            ),
            "unexpected preamble abort: {aborted:?}"
        );
        assert!(
            control.pending_direct_operation.is_none(),
            "aborted preamble scanner must release its retained direct operation"
        );
    });
}

#[test]
fn ordinary_assignment_opens_no_aggregate_savepoint() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\count0=11");

        assert_eq!(
            control.advance(stores).expect("assignment commits"),
            StepResult::Progress(ReplayStep::Continue)
        );
        assert_eq!(stores.count(0).expect("count register"), 11);
        assert_eq!(control.advance_telemetry().attempts, 1);
        assert_eq!(control.advance_telemetry().commits, 1);
        assert_eq!(control.advance_telemetry().maximum_live_savepoints, 0);
    });
}

#[test]
fn committed_token_scanner_attempt_is_discarded_before_named_checkpoint() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\toks0={A}");

        assert_eq!(
            control.advance(stores).expect("token assignment commits"),
            StepResult::Progress(ReplayStep::Continue)
        );
        let stored = admitted!(stores, |context| {
            let tokens = context
                .token_register(0)
                .expect("token register lookup")
                .expect("token assignment installs its promoted root");
            context.token_list(tokens).iter().collect::<Vec<_>>()
        });
        assert_eq!(
            stored,
            [tex_state::token::TokenWord::pack(Token::Char {
                ch: 'A',
                cat: Catcode::Letter,
            })]
        );
        control
            .capture_checkpoint(
                crate::EngineBoundary::OuterParagraphEnd,
                stores,
                crate::ExecutionBudgetCounters::default(),
            )
            .expect("committed scanner attempt no longer blocks a named checkpoint");
    });
}

#[test]
fn diagnostic_assignment_resumes_font_request_without_an_aggregate_savepoint() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        let checkpoint = control
            .capture_checkpoint(
                crate::EngineBoundary::JobStart,
                stores,
                crate::ExecutionBudgetCounters::default(),
            )
            .expect("quiescent diagnostic control captures a checkpoint");
        register_source(&mut control, br"\font\body=cmr10 X");

        assert!(matches!(
            control
                .diagnostic_expand_step(stores)
                .expect("font request suspends"),
            DiagnosticStepResult::Suspended(ResourceNeed::Font { .. })
        ));
        let state_before = stores.journal_cursor().expect("state cursor");
        assert!(matches!(
            control.capture_checkpoint(
                crate::EngineBoundary::OuterParagraphEnd,
                stores,
                crate::ExecutionBudgetCounters::default(),
            ),
            Err(tex_command::CommandSummaryError::AttemptSuspended)
        ));
        assert!(matches!(
            control.restore_checkpoint(&checkpoint, stores),
            Err(crate::CheckpointRestoreError::AttemptSuspended)
        ));
        assert_eq!(
            stores.journal_cursor().expect("state cursor"),
            state_before,
            "checkpoint rejection must not mutate the suspended operation"
        );
        register_cmr10_as(&mut control, stores, "cmr10.tfm");
        assert_eq!(
            control
                .diagnostic_expand_step(stores)
                .expect("font assignment resumes"),
            DiagnosticStepResult::Progress(DiagnosticStep::Assignment)
        );
        assert!(matches!(
            control
                .diagnostic_expand_step(stores)
                .expect("following token is delivered once"),
            DiagnosticStepResult::Progress(DiagnosticStep::Token {
                spelling,
                meaning: Meaning::CharToken {
                    ch: 'X',
                    cat: Catcode::Letter,
                },
                ..
            }) if spelling.token() == Some(Token::Char { ch: 'X', cat: Catcode::Letter })
        ));
        assert_eq!(control.advance_telemetry().maximum_live_savepoints, 0);
    });
}

#[test]
fn diagnostic_input_retry_reuses_the_retained_delivery_attempt() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\input child after");
        assert!(matches!(
            control
                .diagnostic_expand_step(stores)
                .expect("input request suspends"),
            DiagnosticStepResult::Suspended(ResourceNeed::Input { ref name, .. })
                if name == "child.tex"
        ));

        control.capabilities_mut().register_input(
            "child.tex",
            SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(
                    &br"\def\frominput{IN}%
\frominput
"[..],
                ),
            ),
        );
        let mut characters = String::new();
        for _ in 0..16 {
            match control
                .diagnostic_expand_step(stores)
                .expect("retained input attempt resumes")
            {
                DiagnosticStepResult::Progress(DiagnosticStep::Token { spelling, .. }) => {
                    if let Some(Token::Char { ch, .. }) = spelling.token() {
                        characters.push(ch);
                    }
                }
                DiagnosticStepResult::Progress(DiagnosticStep::Assignment) => {}
                DiagnosticStepResult::Progress(DiagnosticStep::EndOfInput) => break,
                DiagnosticStepResult::Suspended(need) => {
                    panic!("registered input requested another resource: {need:?}")
                }
            }
        }
        assert_eq!(characters, "INafter ");
        control
            .capture_checkpoint(
                crate::EngineBoundary::OuterParagraphEnd,
                stores,
                crate::ExecutionBudgetCounters::default(),
            )
            .expect("completed diagnostic input retry releases its attempt owner");
    });
}

#[test]
fn group_entry_local_restore_and_exit_open_no_aggregate_savepoint() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"{\count0=11}");

        control.advance(stores).expect("group enters");
        assert_eq!(
            admitted!(stores, |context| context.execution_group_depth()),
            1
        );
        control.advance(stores).expect("local assignment commits");
        assert_eq!(stores.count(0).expect("count register"), 11);
        control.advance(stores).expect("group exits");
        assert_eq!(
            admitted!(stores, |context| context.execution_group_depth()),
            0
        );
        assert_eq!(stores.count(0).expect("count register"), 0);
        assert_eq!(control.advance_telemetry().maximum_live_savepoints, 0);
    });
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
fn production_batch_returns_after_a_world_effect() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\message{effect}\count0=11 \end");

        assert_eq!(
            control.advance_episode(stores).expect("effect commits"),
            StepResult::Progress(ReplayStep::Continue)
        );
        assert_eq!(
            stores.count(0).expect("count register"),
            0,
            "later input remains for the next host step"
        );
        assert_eq!(control.advance_telemetry().attempts, 1);
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
                            .step_with_observer(stores, &mut observations)
                            .expect("observed stray endcsname continues");
                    }
                } else {
                    for _ in 0..2 {
                        control
                            .step(stores)
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
fn stray_endv_outside_math_runs_off_save_once_and_continues_in_every_mode() {
    // TeX82 §§1130-1131: an end-v outside an alignment runs `off_save`.
    // With no group open, §1066 diagnoses and drops that command.
    for mode in [
        Mode::Vertical,
        Mode::InternalVertical,
        Mode::Horizontal,
        Mode::RestrictedHorizontal,
    ] {
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            let endv = stores.intern("forcedendv").expect("symbol interning");
            assign_static_meaning(stores, endv, Meaning::EndV);
            let mut control = MainControl::tex82_initex(stores);
            control.set_fuel_limit(128).expect("bounded command fuel");
            if mode != Mode::Vertical {
                control.modes.push(mode).expect("test mode push");
            }
            register_source(&mut control, br"\forcedendv\count0=23");

            assert_eq!(
                control.step(stores).expect("stray end-v recovers"),
                MainControlStep::Continue
            );
            // §62's `print_nl` emits no newline at offset 0, so the headline opens
            // the terminal. What follows it is §§310-318's context and the §1131
            // help, whose exact bytes the minifixture channel corpus pins; this
            // test's claim is the diagnosis, not the transcript rendering.
            let terminal = terminal_text(stores);
            assert!(
                terminal.starts_with("! Extra \\forcedendv.\n"),
                "mode {mode:?}: {terminal}"
            );
            assert_eq!(
                control.step(stores).expect("following command executes"),
                MainControlStep::Continue
            );
            assert_eq!(
                stores.count(0).expect("count register"),
                23,
                "mode {mode:?}"
            );
            assert!(control.fuel_burned() < 128, "mode {mode:?}");
        });
    }
}

#[test]
fn stray_endv_in_math_inserts_shift_then_replays_for_off_save() {
    // TeX82 §§1046-1047 insert `$` before the backed-up end-v. Once that
    // closes math, §§1130-1131 see the same command again and run `off_save`.
    for (opening, mode_name) in [
        (br"$".as_slice(), "math"),
        (br"$$".as_slice(), "display math"),
    ] {
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            let endv = stores.intern("forcedendv").expect("symbol interning");
            assign_static_meaning(stores, endv, Meaning::EndV);
            let mut control = MainControl::tex82_initex(stores);
            control.set_fuel_limit(256).expect("bounded command fuel");
            let mut source = opening.to_vec();
            source.extend_from_slice(br"\forcedendv\par\count0=29");
            register_source(&mut control, &source);

            for _ in 0..16 {
                control
                    .step(stores)
                    .expect("math end-v recovery remains finite");
                if stores.count(0).expect("count register") == 29 {
                    break;
                }
            }
            let terminal = terminal_text(stores);
            assert_eq!(
                terminal.matches("Missing $ inserted").count(),
                1,
                "{mode_name}: {terminal:?}"
            );
            assert_eq!(
                terminal.matches("Extra \\forcedendv").count(),
                1,
                "{mode_name}: {terminal:?}"
            );
            assert_eq!(
                stores.count(0).expect("count register"),
                29,
                "{mode_name}: {terminal:?}"
            );
            assert!(control.fuel_burned() < 256, "{mode_name}");
        });
    }
}

fn recursive_test_box<G>(stores: &mut Universe<G>) -> tex_state::node_arena::PageListId {
    use tex_state::font::NULL_FONT;
    use tex_state::glue::Order;
    use tex_state::node::{
        AdjustNode, BoxLr, BoxNode, BoxNodeFields, DiscKind, GlueKind, LeaderPayload, MathBoundary,
        Sign, UnsetKind, UnsetNode, UnsetNodeFields,
    };
    use tex_state::scaled::GlueSetRatio;

    let leaf = crate::test_harness::publish_page_nodes(
        stores,
        [
            Node::Penalty(19),
            Node::Rule {
                width: Some(Scaled::from_raw(101)),
                height: Some(Scaled::from_raw(102)),
                depth: Some(Scaled::from_raw(103)),
            },
        ],
    );
    let box_node = |children| {
        BoxNode::new(BoxNodeFields {
            width: Scaled::from_raw(201),
            height: Scaled::from_raw(202),
            depth: Scaled::from_raw(203),
            shift: Scaled::from_raw(204),
            box_lr: BoxLr::Normal,
            glue_set: GlueSetRatio::ZERO,
            glue_sign: Sign::Stretching,
            glue_order: Order::Fill,
            children,
        })
    };
    let glue = GlueSpec {
        width: Scaled::from_raw(301),
        stretch: Scaled::from_raw(302),
        stretch_order: Order::Fil,
        shrink: Scaled::from_raw(303),
        shrink_order: Order::Filll,
    };
    let tokens = allocate_tokens(
        stores,
        &[
            Token::Char {
                ch: 'm',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: '!',
                cat: Catcode::Other,
            },
        ],
    );
    let tokens = admitted!(stores, |context| context.node_token_list(&tokens));
    let pre = crate::test_harness::publish_page_nodes(
        stores,
        [Node::Char {
            font: NULL_FONT,
            ch: 'p',
            origin: tex_state::token::OriginId::UNKNOWN,
        }],
    );
    let post = crate::test_harness::publish_page_nodes(
        stores,
        [Node::Kern {
            amount: Scaled::from_raw(401),
            kind: tex_state::node::KernKind::Explicit,
        }],
    );
    let replace = crate::test_harness::publish_page_nodes(
        stores,
        [Node::Lig {
            font: NULL_FONT,
            ch: 'L',
            orig: vec!['f', 'i'],
            origins: vec![tex_state::token::OriginId::UNKNOWN; 2],
            left_hit: false,
            right_hit: false,
        }],
    );

    let children = crate::test_harness::publish_page_nodes(
        stores,
        [
            Node::Rule {
                width: Some(Scaled::from_raw(1)),
                height: None,
                depth: Some(Scaled::from_raw(3)),
            },
            Node::Glue {
                spec: glue,
                kind: GlueKind::Leaders,
                leader: Some(LeaderPayload::HList(box_node(leaf))),
            },
            Node::Ins {
                class: 7,
                size: Scaled::from_raw(501),
                split_top_skip: glue,
                split_max_depth: Scaled::from_raw(502),
                floating_penalty: 503,
                content: leaf,
            },
            Node::Mark { class: 9, tokens },
            Node::Adjust(AdjustNode {
                content: post,
                pre: true,
            }),
            Node::MathOn(Scaled::from_raw(601)),
            Node::MathOff(Scaled::from_raw(602)),
            Node::Direction(MathBoundary::BeginR),
            Node::Lig {
                font: NULL_FONT,
                ch: 'L',
                orig: vec!['f', 'i'],
                origins: vec![tex_state::token::OriginId::UNKNOWN; 2],
                left_hit: false,
                right_hit: false,
            },
            Node::Disc {
                kind: DiscKind::Discretionary,
                pre,
                post,
                replace,
                physical_replace_count: 1,
            },
            Node::HList(box_node(pre)),
            Node::VList(box_node(post)),
            Node::Unset(UnsetNode::new(UnsetNodeFields {
                kind: UnsetKind::HBox,
                width: Scaled::from_raw(701),
                height: Scaled::from_raw(702),
                depth: Scaled::from_raw(703),
                span_count: 4,
                stretch: Scaled::from_raw(704),
                stretch_order: Order::Fill,
                shrink: Scaled::from_raw(705),
                shrink_order: Order::Fil,
                children: replace,
            })),
        ],
    );
    crate::test_harness::publish_page_nodes(stores, [Node::HList(box_node(children))])
}

fn recursive_node_signature<G>(
    stores: &Universe<G>,
    list: &tex_state::node_arena::PageListId,
) -> String {
    recursive_owned_node_signature(stores, list)
}

fn recursive_owned_node_signature<G>(
    stores: &Universe<G>,
    list: &tex_state::node_arena::PageListId,
) -> String {
    use tex_state::node::{LeaderPayload, Node};

    page_vec(stores, *list)
        .iter()
        .map(|node| match node {
            Node::HList(box_node) | Node::VList(box_node) => format!(
                "box={}/{:?}/{:?}/{:?}/{:?}/{:?}/{:?}/{:?}/{:?}/children={}",
                if matches!(node, Node::HList(_)) {
                    "h"
                } else {
                    "v"
                },
                box_node.width,
                box_node.height,
                box_node.depth,
                box_node.shift,
                box_node.box_lr,
                box_node.glue_set,
                box_node.glue_sign,
                box_node.glue_order,
                recursive_owned_node_signature(stores, &box_node.children)
            ),
            Node::Unset(unset) => format!(
                "unset={:?}/{:?}/{:?}/{:?}/{}/{:?}/{:?}/{:?}/{:?}/children={}",
                unset.kind,
                unset.width,
                unset.height,
                unset.depth,
                unset.span_count,
                unset.stretch,
                unset.stretch_order,
                unset.shrink,
                unset.shrink_order,
                recursive_owned_node_signature(stores, &unset.children)
            ),
            Node::Glue { spec, leader, .. } => {
                let leader = leader.as_ref().map(|leader| match leader {
                    LeaderPayload::HList(box_node) | LeaderPayload::VList(box_node) => format!(
                        "box={}/{:?}/{:?}/{:?}/{:?}/{:?}/{:?}/{:?}/{:?}/children={}",
                        if matches!(leader, LeaderPayload::HList(_)) {
                            "h"
                        } else {
                            "v"
                        },
                        box_node.width,
                        box_node.height,
                        box_node.depth,
                        box_node.shift,
                        box_node.box_lr,
                        box_node.glue_set,
                        box_node.glue_sign,
                        box_node.glue_order,
                        recursive_owned_node_signature(stores, &box_node.children)
                    ),
                    LeaderPayload::Rule { .. } => format!("{leader:?}"),
                });
                format!("glue={spec:?}/leader={leader:?}")
            }
            Node::Disc {
                pre,
                post,
                replace,
                kind,
                ..
            } => format!(
                "disc={kind:?}/pre={}/post={}/replace={}",
                recursive_owned_node_signature(stores, pre),
                recursive_owned_node_signature(stores, post),
                recursive_owned_node_signature(stores, replace)
            ),
            Node::Mark { class, tokens } => {
                format!("mark={class}/tokens={tokens:?}")
            }
            Node::Ins {
                class,
                size,
                split_top_skip,
                split_max_depth,
                floating_penalty,
                content,
            } => format!(
                "ins={class}/{size:?}/{:?}/{split_max_depth:?}/{floating_penalty}/content={}",
                split_top_skip,
                recursive_owned_node_signature(stores, content)
            ),
            Node::Adjust(adjust) => format!(
                "adjust={}/content={}",
                adjust.pre,
                recursive_owned_node_signature(stores, &adjust.content)
            ),
            _ => format!("{node:?}"),
        })
        .collect::<Vec<_>>()
        .join("|")
}

#[test]
fn copy_preserves_every_recursive_node_payload_and_source_register() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let graph = recursive_test_box(stores);
        stores.assign_page_box_local(0, graph);
        let source = stores.copy_box_to_page(0).expect("promoted source graph");

        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\setbox1=\copy0");
        run_to_end(&mut control, stores);
        let source_after_copy = stores.copy_box_to_page(0).expect("copy retains its source");
        assert_eq!(
            recursive_node_signature(stores, &source_after_copy),
            recursive_node_signature(stores, &source)
        );

        let copied = stores.copy_box_to_page(1).expect("copied register");
        let expected = recursive_node_signature(stores, &copied);
        assert_eq!(
            recursive_node_signature(stores, &source),
            expected,
            "copy retains the exact recursive structure"
        );
        let copied_nodes = page_vec(stores, copied);
        let [Node::HList(root)] = copied_nodes.as_slice() else {
            panic!("fixture root should be an hbox")
        };
        let children = page_vec(stores, root.children);
        assert_eq!(children.len(), 13, "every payload remains in child order");
        assert!(
            matches!(&children[1], Node::Glue { spec, leader: Some(_), .. } if spec.width.raw() == 301)
        );
        assert!(
            matches!(&children[3], Node::Mark { tokens, .. } if admitted!(stores, |context| context.node_token_words(*tokens).expect("live mark").to_vec()) == [
                tex_state::token::TokenWord::pack(Token::Char { ch: 'm', cat: Catcode::Letter }),
                tex_state::token::TokenWord::pack(Token::Char { ch: '!', cat: Catcode::Other }),
            ])
        );

        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\setbox2=\box0");
        run_to_end(&mut control, stores);
        assert!(
            stores.copy_box_to_page(0).is_none(),
            "box consumes its source"
        );
        let surviving_copy = stores
            .copy_box_to_page(1)
            .expect("copy survives source release");
        assert_eq!(recursive_node_signature(stores, &surviving_copy), expected);
        let consumed = stores.copy_box_to_page(2).expect("consumed destination");
        assert_eq!(
            recursive_node_signature(stores, &consumed),
            expected,
            "consumption preserves graph"
        );
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
fn destructive_unbox_transfers_nested_structural_children() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\setbox0=\hbox{\hbox{\kern1pt}}\setbox1=\vbox{\vbox{\kern2pt}}",
        );
        run_to_end(&mut control, stores);

        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\setbox2=\hbox{\unhbox0}\setbox3=\vbox{\unvbox1}",
        );
        run_to_end(&mut control, stores);

        assert!(stores.copy_box_to_page(0).is_none());
        assert!(stores.copy_box_to_page(1).is_none());
        assert!(stores.copy_box_to_page(2).is_some());
        assert!(stores.copy_box_to_page(3).is_some());
    });
}

#[test]
fn grouped_copy_keeps_structural_children() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"{\setbox0\hbox{X}\copy0}");
        run_to_end(&mut control, stores);

        assert_eq!(stores.copy_box_to_page(0), None);
    });
}

#[test]
fn incompatible_unbox_commands_preserve_registers_and_replay_state() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\setbox0=\vbox{\hbox{}}\setbox1=\hbox{\kern1pt}",
        );
        run_to_end(&mut control, stores);
        let vbox_root = stores
            .copy_box_to_page(0)
            .expect("vbox register is nonvoid");
        let hbox_root = stores
            .copy_box_to_page(1)
            .expect("hbox register is nonvoid");
        let vbox = recursive_node_signature(stores, &vbox_root);
        let hbox = recursive_node_signature(stores, &hbox_root);
        let source = "\\unhbox0\\par\\unhcopy0\\par\\unvbox1\\unvcopy1";

        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, source.as_bytes());
        let checkpoint = control
            .capture_checkpoint(
                crate::EngineBoundary::OuterParagraphEnd,
                stores,
                crate::ExecutionBudgetCounters::default(),
            )
            .expect("incompatible unbox source checkpoints");
        run_to_end(&mut control, stores);
        let current_vbox = stores.copy_box_to_page(0).expect("vbox remains nonvoid");
        let current_hbox = stores.copy_box_to_page(1).expect("hbox remains nonvoid");
        assert_eq!(recursive_node_signature(stores, &current_vbox), vbox);
        assert_eq!(recursive_node_signature(stores, &current_hbox), hbox);
        let first_output = terminal_text(stores);

        control
            .restore_checkpoint(&checkpoint, stores)
            .expect("incompatible unbox source restores");
        run_to_end(&mut control, stores);
        let replayed_vbox = stores.copy_box_to_page(0).expect("vbox remains nonvoid");
        let replayed_hbox = stores.copy_box_to_page(1).expect("hbox remains nonvoid");
        assert_eq!(recursive_node_signature(stores, &replayed_vbox), vbox);
        assert_eq!(recursive_node_signature(stores, &replayed_hbox), hbox);
        assert_eq!(terminal_text(stores), first_output);
    });
}

#[test]
fn unvbox_splices_vertical_nodes_without_inserting_baseline_glue() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\vsize=1000pt \setbox0=\vbox{\hrule\hbox{}}\unvbox0",
        );
        run_to_end(&mut control, stores);

        assert!(
            !admitted!(stores, |context| context
                .current_page_nodes()
                .cloned()
                .collect::<Vec<_>>())
            .iter()
            .any(|node| matches!(
                node,
                tex_state::node::Node::Glue {
                    kind: tex_state::node::GlueKind::BaselineSkip,
                    ..
                }
            ))
        );
    });
}

#[test]
fn badness_reads_most_recent_pack_and_is_not_assignable() {
    // TeX82 §§422--424 reads `\badness` from `last_badness`; §§644/660
    // initializes and updates that same cell during horizontal packing.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let initial = admitted!(stores, |context| (
            context.int_param(IntParam::LAST_BADNESS),
            context
                .internal_integer(tex_state::meaning::InternalInteger::Badness)
                .expect("badness is state-owned"),
        ));
        assert_eq!(initial, (0, 0), "badness is zero before the first pack");

        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"{\setbox0=\hbox to 10pt{\hskip0pt plus1pt}}\count0=\badness\edef\x{\the\badness}",
        );
        run_to_end(&mut control, stores);

        assert_eq!(
            stores.count(0).expect("count register"),
            tex_typeset::INF_BAD
        );
        let packed = admitted!(stores, |context| (
            context.int_param(IntParam::LAST_BADNESS),
            context
                .internal_integer(tex_state::meaning::InternalInteger::Badness)
                .expect("badness is state-owned"),
        ));
        assert_eq!(packed, (tex_typeset::INF_BAD, tex_typeset::INF_BAD));
        let rendered: String = macro_semantic_tokens(stores, "x")
            .into_iter()
            .filter_map(|token| match token {
                Token::Char { ch, .. } => Some(ch),
                _ => None,
            })
            .collect();
        assert_eq!(rendered, "10000");

        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\badness=0");
        run_to_end(&mut control, stores);
        assert!(terminal_text(stores).contains("You can't use `\\badness'"));
    });
}

#[test]
fn vbox_sets_overfull_badness_when_the_box_cannot_shrink() {
    // TeX82 §§668/674 initializes and updates `last_badness` during
    // vertical packing; §§422--424 exposes the resulting value as `\badness`.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\setbox0=\vbox to10pt{\hrule height20pt}\count0=\badness",
        );
        run_to_end(&mut control, stores);

        assert_eq!(
            stores.count(0).expect("count register"),
            tex_typeset::OVERFULL_BADNESS
        );
    });
}

#[test]
fn etex_lastnodetype_reads_each_live_mode_tail_without_mutation() {
    // e-TeX 2.6 `etex.ch` [26.424]: `find_effective_tail` returns -1 for an
    // empty list, otherwise the e-TRIP node code of the real current tail.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        tex_command::install_tex82_expandable_primitives(stores);
        tex_command::install_etex_expandable_primitives(stores);
        crate::install_unexpandable_primitives(stores);
        crate::install_etex_unexpandable_primitives(stores);
        let mut control = MainControl::prepared_initex(CommandProfile::ETEX26);
        register_source(
            &mut control,
            br"\xdef\outerempty{\the\lastnodetype}
          \hbox{\xdef\hempty{\the\lastnodetype}}
          \hbox{\vrule\xdef\hrule{\the\lastnodetype}}
          \hbox{\kern1pt\xdef\hkern{\the\lastnodetype}}
          \vbox{\hbox{}\xdef\vboxnode{\the\lastnodetype}}
          $\mathord{1}\xdef\mathnode{\the\lastnodetype}$
          \end",
        );

        run_to_end(&mut control, stores);

        for (name, expected) in [
            ("outerempty", "-1"),
            ("hempty", "-1"),
            ("hrule", "3"),
            ("hkern", "12"),
            ("vboxnode", "1"),
            ("mathnode", "15"),
        ] {
            assert!(stores.intern(name).is_ok(), "missing probe macro {name}");
            assert_eq!(macro_character_text(stores, name), expected, "{name}");
        }
    });
}

#[test]
fn etex_lastnodetype_covers_every_node_code() {
    // e-TeX 2.6 `etex.ch` block 99 maps the complete 0..=15 node-type
    // interval.  Each enquiry is made while its node is still the live tail;
    // the alignment row is observed from `\noalign`, where it is an unset
    // node until `fin_align` resolves it.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        tex_command::install_tex82_expandable_primitives(stores);
        tex_command::install_etex_expandable_primitives(stores);
        crate::install_unexpandable_primitives(stores);
        crate::install_etex_unexpandable_primitives(stores);
        let mut control = MainControl::prepared_initex(CommandProfile::ETEX26);
        register_cmr10_as(&mut control, stores, "cmr10.tfm");
        register_source(
            &mut control,
            br"\font\f=cmr10 \f
          \hbox{x\xdef\nzero{\the\lastnodetype}}
          \hbox{\hbox{}\xdef\none{\the\lastnodetype}}
          \hbox{\vbox{}\xdef\ntwo{\the\lastnodetype}}
          \hbox{\vrule\xdef\nthree{\the\lastnodetype}}
          \vbox{\insert0{}\xdef\nfour{\the\lastnodetype}}
          \vbox{\mark{}\xdef\nfive{\the\lastnodetype}}
          \hbox{\vadjust{}\xdef\nsix{\the\lastnodetype}}
          \hbox{\discretionary{}{}{}\xdef\neight{\the\lastnodetype}}
          \hbox{\special{}\xdef\nnine{\the\lastnodetype}}
          \hbox{\hskip1pt\xdef\neleven{\the\lastnodetype}}
          \hbox{\kern1pt\xdef\ntwelve{\the\lastnodetype}}
          \hbox{\penalty1\xdef\nthirteen{\the\lastnodetype}}
          \vbox{\halign{#\cr x\cr\noalign{\xdef\nfourteen{\the\lastnodetype}}}}
          \end",
        );

        run_to_end(&mut control, stores);

        for (name, expected) in [
            ("nzero", "0"),
            ("none", "1"),
            ("ntwo", "2"),
            ("nthree", "3"),
            ("nfour", "4"),
            ("nfive", "5"),
            ("nsix", "6"),
            ("neight", "8"),
            ("nnine", "9"),
            ("neleven", "11"),
            ("ntwelve", "12"),
            ("nthirteen", "13"),
            ("nfourteen", "14"),
        ] {
            assert!(stores.intern(name).is_ok(), "missing probe macro {name}");
            assert_eq!(macro_character_text(stores, name), expected, "{name}");
        }
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

fn pdftex_random_control<G>(stores: &mut Universe<G>) -> MainControl<G> {
    let set_seed = stores.intern("pdfsetrandomseed").expect("symbol interning");
    assign_static_meaning(
        stores,
        set_seed,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfSetRandomSeed),
    );
    MainControl::with_profile(tex_command::CommandProfile::PDFTEX14029)
}

fn pdftex_timer_control<G>(stores: &mut Universe<G>) -> MainControl<G> {
    let reset_timer = stores.intern("pdfresettimer").expect("symbol interning");
    assign_static_meaning(
        stores,
        reset_timer,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfResetTimer),
    );
    MainControl::with_profile(tex_command::CommandProfile::PDFTEX14029)
}

fn pdftex_interword_control<G>(stores: &mut Universe<G>) -> MainControl<G> {
    for (name, primitive) in [
        (
            "pdfinterwordspaceon",
            UnexpandablePrimitive::PdfInterwordSpaceOn,
        ),
        (
            "pdfinterwordspaceoff",
            UnexpandablePrimitive::PdfInterwordSpaceOff,
        ),
        ("pdffakespace", UnexpandablePrimitive::PdfFakeSpace),
        ("pdfrunninglinkon", UnexpandablePrimitive::PdfRunningLinkOn),
        (
            "pdfrunninglinkoff",
            UnexpandablePrimitive::PdfRunningLinkOff,
        ),
        ("pdfspacefont", UnexpandablePrimitive::PdfSpaceFont),
    ] {
        let symbol = stores.intern(name).expect("symbol interning");
        assign_static_meaning(stores, symbol, Meaning::UnexpandablePrimitive(primitive));
    }
    MainControl::with_profile(tex_command::CommandProfile::PDFTEX14029)
}

fn pdftex_font_action_control<G>(stores: &mut Universe<G>) -> MainControl<G> {
    let nullfont = stores.intern("nullfont").expect("symbol interning");
    assign_static_meaning(stores, nullfont, Meaning::Font(tex_state::font::NULL_FONT));
    for (name, primitive) in [
        ("pdffontexpand", UnexpandablePrimitive::PdfFontExpand),
        ("pdffontattr", UnexpandablePrimitive::PdfFontAttr),
        ("pdfincludechars", UnexpandablePrimitive::PdfIncludeChars),
        ("pdfmapfile", UnexpandablePrimitive::PdfMapFile),
        ("pdfmapline", UnexpandablePrimitive::PdfMapLine),
        (
            "pdfglyphtounicode",
            UnexpandablePrimitive::PdfGlyphToUnicode,
        ),
        (
            "pdfnobuiltintounicode",
            UnexpandablePrimitive::PdfNoBuiltinToUnicode,
        ),
    ] {
        let symbol = stores.intern(name).expect("symbol interning");
        assign_static_meaning(stores, symbol, Meaning::UnexpandablePrimitive(primitive));
    }
    MainControl::with_profile(tex_command::CommandProfile::PDFTEX14029)
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

fn pdftex_snapping_control<G>(stores: &mut Universe<G>) -> MainControl<G> {
    for (name, primitive) in [
        ("pdfsnaprefpoint", UnexpandablePrimitive::PdfSnapRefPoint),
        ("pdfsnapy", UnexpandablePrimitive::PdfSnapY),
        ("pdfsnapycomp", UnexpandablePrimitive::PdfSnapYComp),
    ] {
        let symbol = stores.intern(name).expect("symbol interning");
        assign_static_meaning(stores, symbol, Meaning::UnexpandablePrimitive(primitive));
    }
    MainControl::with_profile(tex_command::CommandProfile::PDFTEX14029)
}

fn pdftex_graphics_control<G>(stores: &mut Universe<G>) -> MainControl<G> {
    for (name, primitive) in [
        ("pdfliteral", UnexpandablePrimitive::PdfLiteral),
        ("pdfsetmatrix", UnexpandablePrimitive::PdfSetMatrix),
        ("pdfsave", UnexpandablePrimitive::PdfSave),
        ("pdfrestore", UnexpandablePrimitive::PdfRestore),
        ("pdfcolorstack", UnexpandablePrimitive::PdfColorStack),
        ("pdfsavepos", UnexpandablePrimitive::PdfSavePos),
    ] {
        let symbol = stores.intern(name).expect("symbol interning");
        assign_static_meaning(stores, symbol, Meaning::UnexpandablePrimitive(primitive));
    }
    MainControl::with_profile(tex_command::CommandProfile::PDFTEX14029)
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

            assert!(matches!(
                control.step(stores),
                Err(ExecError::PdfExtensionInDviMode(name)) if name == primitive
            ));
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

fn pdftex_outline_control<G>(stores: &mut Universe<G>) -> MainControl<G> {
    let outline = stores.intern("pdfoutline").expect("symbol interning");
    assign_static_meaning(
        stores,
        outline,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfOutline),
    );
    MainControl::with_profile(tex_command::CommandProfile::PDFTEX14029)
}

fn pdftex_thread_control<G>(stores: &mut Universe<G>) -> MainControl<G> {
    for (name, primitive) in [
        ("pdfthread", UnexpandablePrimitive::PdfThread),
        ("pdfstartthread", UnexpandablePrimitive::PdfStartThread),
        ("pdfendthread", UnexpandablePrimitive::PdfEndThread),
    ] {
        let symbol = stores.intern(name).expect("symbol interning");
        assign_static_meaning(stores, symbol, Meaning::UnexpandablePrimitive(primitive));
    }
    MainControl::with_profile(tex_command::CommandProfile::PDFTEX14029)
}

fn pdftex_object_control<G>(stores: &mut Universe<G>) -> MainControl<G> {
    for (name, primitive) in [
        ("pdfobj", UnexpandablePrimitive::PdfObject),
        ("pdfrefobj", UnexpandablePrimitive::PdfReferenceObject),
        ("immediate", UnexpandablePrimitive::Immediate),
    ] {
        let symbol = stores.intern(name).expect("symbol interning");
        assign_static_meaning(stores, symbol, Meaning::UnexpandablePrimitive(primitive));
    }
    MainControl::with_profile(tex_command::CommandProfile::PDFTEX14029)
}

fn pdftex_form_control<G>(stores: &mut Universe<G>) -> MainControl<G> {
    for (name, primitive) in [
        ("pdfxform", UnexpandablePrimitive::PdfXForm),
        ("pdfrefxform", UnexpandablePrimitive::PdfRefXForm),
        ("immediate", UnexpandablePrimitive::Immediate),
    ] {
        let symbol = stores.intern(name).expect("symbol interning");
        assign_static_meaning(stores, symbol, Meaning::UnexpandablePrimitive(primitive));
    }
    MainControl::with_profile(tex_command::CommandProfile::PDFTEX14029)
}

fn pdftex_image_control<G>(stores: &mut Universe<G>) -> MainControl<G> {
    for (name, primitive) in [
        ("pdfximage", UnexpandablePrimitive::PdfXImage),
        ("pdfrefximage", UnexpandablePrimitive::PdfRefXImage),
        ("immediate", UnexpandablePrimitive::Immediate),
    ] {
        let symbol = stores.intern(name).expect("symbol interning");
        assign_static_meaning(stores, symbol, Meaning::UnexpandablePrimitive(primitive));
    }
    MainControl::with_profile(tex_command::CommandProfile::PDFTEX14029)
}

fn test_pdf_image_source() -> tex_state::PdfExternalImageSource {
    tex_state::PdfExternalImageSource {
        identity: tex_state::ContentHash::from_bytes(b"canonical image preflight"),
        metadata: tex_state::PdfExternalImageMetadata::Raster(tex_state::PdfRasterImageMetadata {
            format: tex_state::PdfRasterFormat::Png,
            width: 1,
            height: 1,
            bits_per_component: 8,
            color_space: tex_state::PdfRasterColorSpace::Gray,
            alpha: false,
            png_color_type: Some(0),
        }),
        natural_width: Scaled::from_raw(Scaled::UNITY),
        natural_height: Scaled::from_raw(Scaled::UNITY),
        bytes: b"image bytes".to_vec().into(),
    }
}

fn install_test_hbox<G>(stores: &mut Universe<G>, register: u16, width: Scaled) {
    let children = tex_state::node_arena::PageListId::empty();
    let list = crate::test_harness::publish_page_nodes(
        stores,
        [Node::HList(tex_state::node::BoxNode::new(
            tex_state::node::BoxNodeFields {
                width,
                height: Scaled::from_raw(2),
                depth: Scaled::from_raw(3),
                shift: Scaled::from_raw(0),
                box_lr: tex_state::node::BoxLr::Normal,
                glue_set: tex_state::scaled::GlueSetRatio::ZERO,
                glue_sign: tex_state::node::Sign::Normal,
                glue_order: Order::Normal,
                children,
            },
        ))],
    );
    stores.assign_page_box_local(register, list);
}

fn install_test_form<G>(stores: &mut Universe<G>) {
    install_test_hbox(stores, 0, Scaled::from_raw(11));
    let list = stores.take_box_to_page(0).expect("test form box");
    admitted!(stores, |context| {
        let identity = context.reserve_pdf_form().expect("reserve test form");
        context
            .initialize_pdf_form(
                identity,
                list,
                (
                    Scaled::from_raw(11),
                    Scaled::from_raw(2),
                    Scaled::from_raw(3),
                ),
                None,
                None,
                false,
            )
            .expect("initialize test form");
    });
}

fn token_character_text<G>(stores: &mut Universe<G>, tokens: tex_state::TokenListId<G>) -> String {
    admitted!(stores, |context| context
        .token_list(tokens)
        .iter()
        .collect::<Vec<_>>())
    .into_iter()
    .filter_map(|word| match word.semantic_token() {
        Token::Char { ch, .. } => Some(ch),
        Token::Cs(_) | Token::Param(_) | Token::Frozen(_) => None,
    })
    .collect()
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
                br"\immediate\pdfobj useobjnum 41 stream attr{/Type /Metadata} file{retry.dat}",
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
fn pdf_image_create_rejects_dvi_before_operands_allocation_or_resource_lookup() {
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

        crate::test_harness::assign_int_param(
            stores,
            IntParam::PDF_OUTPUT,
            1,
            tex_state::AssignmentScope::Global,
        )
        .expect("integer parameter assignment");
        let pdf_state_before = stores.journal_cursor().expect("state cursor");
        let request = match control.advance(stores).expect("PDF image request suspends") {
            StepResult::Suspended(ResourceNeed::PdfImage { request }) => request,
            other => panic!("expected image suspension, got {other:?}"),
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
        assert!(matches!(
            control.capture_checkpoint(
                crate::EngineBoundary::OuterParagraphEnd,
                stores,
                crate::ExecutionBudgetCounters::default(),
            ),
            Err(tex_command::CommandSummaryError::AttemptSuspended)
        ));

        control.capabilities_mut().register_pdf_image(
            request,
            PdfImageResource::Available(test_pdf_image_source()),
        );
        assert_eq!(
            control
                .advance(stores)
                .expect("fulfilled retry preserves and consumes the complete request"),
            StepResult::Progress(MainControlStep::Continue)
        );
        let image = admitted!(stores, |context| {
            let raw = context
                .internal_integer(tex_state::meaning::InternalInteger::PdfLastXImage)
                .expect("last image integer");
            let id =
                tex_state::PdfExternalImageId::new(raw as u32).expect("retried image is allocated");
            context
                .pdf_external_image_record(id)
                .expect("image metadata")
        });
        assert_eq!(
            image.dimensions().width,
            Scaled::from_raw(10 * Scaled::UNITY)
        );
        assert_eq!(
            image.dimensions().height,
            Scaled::from_raw(20 * Scaled::UNITY)
        );
        assert_eq!(
            image.dimensions().depth,
            Scaled::from_raw(3 * Scaled::UNITY)
        );
        assert!(mode_vec(&control, stores).is_empty());
        control
            .capture_checkpoint(
                crate::EngineBoundary::OuterParagraphEnd,
                stores,
                crate::ExecutionBudgetCounters::default(),
            )
            .expect("fulfilled retry discards the exact retained attempt suffix");
    });
}

#[test]
fn immediate_pdf_image_uses_the_same_preflight_and_transactional_retry() {
    // pdftex.web §1621 expands the command after `\immediate`, then invokes
    // §1551's complete `\pdfximage` case. Its output check precedes every
    // image operand and the recursive call performs the allocation only
    // after resource lookup succeeds.
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

        crate::test_harness::assign_int_param(
            stores,
            IntParam::PDF_OUTPUT,
            1,
            tex_state::AssignmentScope::Global,
        )
        .expect("integer parameter assignment");
        let pdf_state_before = stores.journal_cursor().expect("state cursor");
        let request = match control.advance(stores).expect("immediate image suspends") {
            StepResult::Suspended(ResourceNeed::PdfImage { request }) => request,
            other => panic!("expected immediate image suspension, got {other:?}"),
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
        assert_eq!(
            control
                .advance(stores)
                .expect("immediate image retry allocates in the same operation"),
            StepResult::Progress(MainControlStep::Continue)
        );
        assert_eq!(
            usize::from(
                admitted!(stores, |context| context
                    .internal_integer(tex_state::meaning::InternalInteger::PdfLastXImage)
                    .expect("PDF image integer"))
                    != 0
            ),
            1
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

fn pdftex_annotation_control<G>(stores: &mut Universe<G>) -> MainControl<G> {
    for (name, primitive) in [
        ("pdfannot", UnexpandablePrimitive::PdfAnnot),
        ("pdfstartlink", UnexpandablePrimitive::PdfStartLink),
        ("pdfendlink", UnexpandablePrimitive::PdfEndLink),
    ] {
        let symbol = stores.intern(name).expect("symbol interning");
        assign_static_meaning(stores, symbol, Meaning::UnexpandablePrimitive(primitive));
    }
    MainControl::with_profile(tex_command::CommandProfile::PDFTEX14029)
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

fn pdftex_destination_control<G>(stores: &mut Universe<G>) -> MainControl<G> {
    let destination = stores.intern("pdfdest").expect("symbol interning");
    assign_static_meaning(
        stores,
        destination,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfDest),
    );
    MainControl::with_profile(tex_command::CommandProfile::PDFTEX14029)
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
        assert!(error.is_pdftex_navigation_fatal());
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

fn step_until_pdf_seed<G>(control: &mut MainControl<G>, stores: &mut Universe<G>, expected: i32) {
    for _ in 0..4 {
        control.step(stores).expect("random command");
        if stores.world().pdf_random_seed() == expected {
            return;
        }
    }
    panic!("pdfTeX random seed did not become {expected}");
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

#[derive(Default)]
struct ObservationRecorder(Vec<CommandObservation>);

impl CommandObserver for ObservationRecorder {
    fn committed(&mut self, observation: CommandObservation) {
        self.0.push(observation);
    }
}

#[derive(Default)]
struct GeometryObservationRecorder(Vec<CommandObservation>);

impl CommandObserver for GeometryObservationRecorder {
    fn observes_geometry(&self) -> bool {
        true
    }

    fn committed(&mut self, observation: CommandObservation) {
        self.0.push(observation);
    }
}

#[test]
fn observed_box_packaging_commits_geometry_at_the_operation_boundary() {
    // TeX82 §§649--668 and §§668--676 commit the finished hpack/vpack
    // dimensions. The observer sees those transitions only after each
    // enclosing command operation commits; no Universe-owned queue is
    // involved.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\setbox0=\hbox{}\setbox1=\vbox{}\end");
        let mut observations = GeometryObservationRecorder::default();
        loop {
            match control
                .step_with_observer(stores, &mut observations)
                .expect("box packaging executes")
            {
                MainControlStep::End | MainControlStep::EndOfInput => break,
                MainControlStep::Continue => {}
            }
        }
        let geometry = observations
            .0
            .iter()
            .filter_map(|observation| match observation {
                CommandObservation::Geometry(record) => Some(record),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(matches!(
            geometry.as_slice(),
            [
                GeometryRecord::Hpack {
                    width_sp: 0,
                    height_sp: 0,
                    depth_sp: 0,
                    ..
                },
                GeometryRecord::Vpack {
                    width_sp: 0,
                    height_sp: 0,
                    depth_sp: 0,
                    ..
                }
            ]
        ));
    });
}

fn observation_name(value: &ObservationValue) -> Option<&str> {
    match value {
        ObservationValue::Name(name) => Some(name),
        _ => None,
    }
}

fn observation_tokens(value: &ObservationValue) -> Option<&[tex_command::ObservedToken]> {
    match value {
        ObservationValue::Tokens(tokens) => Some(tokens),
        _ => None,
    }
}

#[test]
fn etex_identical_local_integer_parameter_reassignment_is_not_a_mutation() {
    // e-TeX §275: `eq_word_define` returns immediately when extended mode
    // locally assigns the value already present. The negative controls pin
    // that a changed local value and an identical global value still commit.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        tex_command::install_tex82_expandable_primitives(stores);
        tex_command::install_etex_expandable_primitives(stores);
        crate::install_unexpandable_primitives(stores);
        crate::install_etex_unexpandable_primitives(stores);
        let mut control = MainControl::prepared_initex(CommandProfile::ETEX26);
        register_source(
            &mut control,
            br"\endlinechar=13 \endlinechar=12 \global\endlinechar=12 \end",
        );
        let mut observations = ObservationRecorder::default();
        loop {
            match control
                .step_with_observer(stores, &mut observations)
                .expect("e-TeX integer-parameter reassignments execute")
            {
                MainControlStep::End | MainControlStep::EndOfInput => break,
                MainControlStep::Continue => {}
            }
        }

        let mutations: Vec<_> = observations
            .0
            .iter()
            .filter_map(|observation| match observation {
                CommandObservation::Mutation(record)
                    if record.target == MutationTarget::Parameter =>
                {
                    Some((&record.key, &record.value, record.global))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            mutations,
            [
                (
                    &ObservationValue::Name("integer_parameter:48".into()),
                    &ObservationValue::Integer(12),
                    false,
                ),
                (
                    &ObservationValue::Name("integer_parameter:48".into()),
                    &ObservationValue::Integer(12),
                    true,
                ),
            ]
        );
    });
}

#[test]
fn etex_sparse_word_reassignment_retains_its_observed_boundary() {
    // e-TeX 2.6 [49.1236-1237] routes sparse count and dimen words through
    // `sa_w_def`, not §§277-278's dense `eq_word_define`. The canonical
    // oracle observes the sparse assignment boundary even when its value is
    // the default; dense identical assignments retain their shortcut.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        tex_command::install_tex82_expandable_primitives(stores);
        tex_command::install_etex_expandable_primitives(stores);
        crate::install_unexpandable_primitives(stores);
        crate::install_etex_unexpandable_primitives(stores);
        let mut control = MainControl::prepared_initex(CommandProfile::ETEX26);
        register_source(
            &mut control,
            br"{\count0=0 \dimen0=0pt \count300=0 \dimen301=0pt}\end",
        );
        let mut observations = ObservationRecorder::default();
        run_to_end_observed(&mut control, stores, &mut observations);

        let mutations: Vec<_> = observations
            .0
            .iter()
            .filter_map(|observation| match observation {
                CommandObservation::Mutation(record)
                    if record.target == MutationTarget::Register =>
                {
                    Some((&record.key, &record.value))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            mutations,
            [
                (
                    &ObservationValue::Name("count:300".into()),
                    &ObservationValue::Integer(0),
                ),
                (
                    &ObservationValue::Name("dimen:301".into()),
                    &ObservationValue::Scaled(0),
                ),
            ]
        );
        assert_eq!(stores.count(300).expect("count register"), 0);
        assert_eq!(
            admitted!(stores, |context| context.dimen(301)),
            Scaled::from_raw(0)
        );
    });
}

#[test]
fn etex_sparse_register_reads_keep_the_extended_index_after_group_exit() {
    // e-TeX 2.6 [26.427] scans an internal word-register selector with
    // `scan_register_num`. Keep the real sparse value and the independently
    // chosen register-zero sentinel distinct so an eight-bit recovery cannot
    // masquerade as a state-restoration failure.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        tex_command::install_tex82_expandable_primitives(stores);
        tex_command::install_etex_expandable_primitives(stores);
        crate::install_unexpandable_primitives(stores);
        crate::install_etex_unexpandable_primitives(stores);
        let mut control = MainControl::prepared_initex(CommandProfile::ETEX26);
        register_source(
            &mut control,
            br"\begingroup\tracingrestores=1\count20=5\count2000=5\endgroup
           \begingroup{\tracingassigns=1\count2000=0}\count2001=5
           \ifnum\count2000=0 \global\count0=17\fi\endgroup\end",
        );
        run_to_end(&mut control, stores);

        assert_eq!(
            stores.int_param(IntParam::ETEX_EXTENDED_MODE),
            1,
            "extended register domain must survive grouping"
        );
        assert_eq!(
            stores.count(2000).expect("count register"),
            0,
            "sparse state must restore to zero"
        );
        assert_eq!(stores.count(0).expect("count register"), 17);
    });
}

#[test]
fn etex_toks_assignment_and_rhs_keep_sparse_register_indices() {
    // e-TeX 2.6 [49.1226--1227] uses `scan_register_num` for both the direct
    // token-register assignment target and a direct token-register RHS.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = etex_initex(stores);
        register_source(&mut control, br"\toks2000={a b c} \toks2001=\toks2000 \end");
        let mut observations = ObservationRecorder::default();
        run_to_end_observed(&mut control, stores, &mut observations);

        let mutations = observations
            .0
            .iter()
            .filter_map(|observation| match observation {
                CommandObservation::Mutation(record)
                    if record.target == MutationTarget::Register =>
                {
                    Some((
                        observation_name(&record.key),
                        observation_tokens(&record.value),
                    ))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(mutations.len(), 2);
        assert_eq!(mutations[0].0, Some("toks:2000"));
        assert_eq!(mutations[1].0, Some("toks:2001"));
        let (copied, source, zero) = admitted!(stores, |context| {
            let token_register = |index| {
                context
                    .token_register(index)
                    .expect("token register")
                    .map(|tokens| context.token_list(tokens).iter().collect::<Vec<_>>())
                    .unwrap_or_default()
            };
            (
                token_register(2_001),
                token_register(2_000),
                token_register(0),
            )
        });
        assert_eq!(copied, source);
        assert!(!copied.is_empty());
        assert!(zero.is_empty());
    });
}

#[test]
fn etex_dense_token_list_reassignments_use_eq_define_shortcut() {
    // e-TeX 2.6 [19.277] returns from `eq_define` when both the command and
    // token-list pointer are unchanged. This covers both dense `\toks`
    // registers and token-list parameters; [49.1226]'s sparse `sa_def` path
    // retains its independently observed assignment boundary.
    let source = br"{\toks20={} \everypar={} \toks300={}
                      \global\toks20={} \global\everypar={}}\end";
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = etex_initex(stores);
        register_source(&mut control, source);
        let mut observations = ObservationRecorder::default();
        run_to_end_observed(&mut control, stores, &mut observations);

        let mutations = observations
            .0
            .iter()
            .filter_map(|observation| match observation {
                CommandObservation::Mutation(record)
                    if record.target == MutationTarget::Register
                        || record.target == MutationTarget::Parameter =>
                {
                    Some((record.target, observation_name(&record.key), record.global))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            mutations,
            [
                (MutationTarget::Register, Some("toks:300"), false),
                (MutationTarget::Register, Some("toks:20"), true),
                (MutationTarget::Parameter, Some("token_parameter:1"), true),
            ]
        );

        crate::test_harness::with_nonstop_plain_universe(|tex82| {
            let mut tex82_control = MainControl::tex82_initex(tex82);
            register_source(&mut tex82_control, br"\toks20={} \everypar={} \end");
            let mut tex82_observations = ObservationRecorder::default();
            run_to_end_observed(&mut tex82_control, tex82, &mut tex82_observations);
            assert_eq!(
                tex82_observations
                    .0
                    .iter()
                    .filter(|observation| matches!(observation, CommandObservation::Mutation(_)))
                    .count(),
                2,
                "TeX82 does not have e-TeX's identical-definition shortcut"
            );
        });
    });
}

#[test]
fn braced_token_parameter_assignment_normalizes_empty_to_null_and_restores_scope() {
    // TeX82 §1226 maps a braced scan with `link(def_ref)=null` to
    // `undefined_cs,null`, while a nonempty scan installs `call,def_ref`.
    // Sections 275--283 then restore the exact outer pointer at group exit.
    crate::test_harness::with_nonstop_plain_universe(|empty_stores| {
        let mut empty_control = MainControl::tex82_initex(empty_stores);
        register_source(&mut empty_control, br"\everypar={}\end");
        assert_eq!(
            empty_control
                .step(empty_stores)
                .expect("empty assignment executes"),
            MainControlStep::Continue
        );
        assert_eq!(
            admitted!(empty_stores, |context| context
                .token_parameter(TokParam::EVERY_PAR)
                .expect("everypar parameter")),
            None,
            "a braced empty assignment must store TeX's null pointer"
        );

        crate::test_harness::with_nonstop_plain_universe(|stores| {
            let mut control = MainControl::tex82_initex(stores);
            register_source(&mut control, br"\everypar={A}{\everypar={}}\end");
            assert_eq!(
                control.step(stores).expect("nonempty assignment executes"),
                MainControlStep::Continue
            );
            let outer = admitted!(stores, |context| context
                .token_parameter(TokParam::EVERY_PAR)
                .expect("everypar parameter"))
            .expect("nonempty assignment stores a pointer");
            assert_eq!(
                admitted!(stores, |context| context
                    .token_list(outer.clone())
                    .iter()
                    .collect::<Vec<_>>()),
                [tex_state::token::TokenWord::pack(Token::Char {
                    ch: 'A',
                    cat: Catcode::Letter,
                })]
            );
            assert_eq!(
                control.step(stores).expect("group opens"),
                MainControlStep::Continue
            );
            assert_eq!(
                control
                    .step(stores)
                    .expect("scoped empty assignment executes"),
                MainControlStep::Continue
            );
            assert_eq!(
                admitted!(stores, |context| context
                    .token_parameter(TokParam::EVERY_PAR)
                    .expect("everypar parameter")),
                None
            );
            assert_eq!(
                control.step(stores).expect("group closes"),
                MainControlStep::Continue
            );
            assert_eq!(
                admitted!(stores, |context| context
                    .token_parameter(TokParam::EVERY_PAR)
                    .expect("everypar parameter")),
                Some(outer)
            );
        });
    });
}

#[test]
fn etex_sparse_setbox_observes_delayed_and_immediate_commits() {
    // TeX82 §§1077/1085 commits a constructed box only after its box group is
    // unsaved. e-TeX 2.6 [47.1077] sends targets above 255 through [53a]'s
    // `sa_def_box`, so those delayed writes (and immediate void operands) are
    // sparse mutation boundaries; the dense `eq_define` target stays silent.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = etex_initex(stores);
        register_source(
            &mut control,
            br"{\setbox20=\hbox{} \setbox300=\hbox{}
             \global\setbox301=\vbox{} \setbox302=\box0}\end",
        );
        let mut observations = ObservationRecorder::default();
        run_to_end_observed(&mut control, stores, &mut observations);

        let mutations = observations
            .0
            .iter()
            .filter_map(|observation| match observation {
                CommandObservation::Mutation(record)
                    if record.target == MutationTarget::Register =>
                {
                    Some((
                        observation_name(&record.key),
                        observation_name(&record.value),
                        record.global,
                    ))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            mutations,
            [
                (Some("box:300"), Some("occupied"), false),
                (Some("box:301"), Some("occupied"), true),
                (Some("box:302"), Some("void"), false),
            ]
        );
        assert!(stores.copy_box_to_page(20).is_none());
        assert!(stores.copy_box_to_page(300).is_none());
        assert!(stores.copy_box_to_page(301).is_some());
        assert!(stores.copy_box_to_page(302).is_none());
    });
}

#[test]
fn etex_sparse_copy_keeps_a_nested_constructed_source_box() {
    // TeX82 §§1079--1081 make `\copy` a non-destructive read. e-TeX 2.6
    // [47.1077] extends the same operation to sparse box registers.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = etex_initex(stores);
        register_source(
            &mut control,
            br"\nonstopmode
           \setbox32101=\hbox{\global\setbox32102=\vbox{\setbox32103=\vtop{}}}
           \showbox32101
           \setbox32103=\copy32101 \end",
        );
        let mut observations = ObservationRecorder::default();
        run_to_end_observed(&mut control, stores, &mut observations);

        let mutations = observations
            .0
            .iter()
            .filter_map(|observation| match observation {
                CommandObservation::Mutation(record)
                    if observation_name(&record.key) == Some("box:32103") =>
                {
                    observation_name(&record.value)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(mutations, ["occupied", "occupied"]);
        assert!(stores.copy_box_to_page(32101).is_some());
        assert!(stores.copy_box_to_page(32103).is_some());
    });
}

#[test]
fn etex_sparse_box_dimension_assignment_is_visible_to_internal_scans() {
    // e-TeX 2.6 [49.1247] widens `alter_box_dimen` with
    // `scan_register_num`; [26.420] uses the same sparse fetch when `\ht`
    // is subsequently scanned as an internal dimension.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = etex_initex(stores);
        register_source(
            &mut control,
            br"\setbox32101=\hbox{} \ht32101=2pt
           \ifdim\ht32101=2pt \count0=1\fi \end",
        );

        run_to_end(&mut control, stores);

        assert_eq!(
            admitted!(stores, |context| context
                .box_dimension(32101, tex_state::BoxDimension::Height)),
            Some(Scaled::from_raw(2 * Scaled::UNITY))
        );
        assert_eq!(stores.count(0).expect("count register"), 1);
    });
}

#[test]
fn etex_identical_local_code_reassignment_is_a_save_stack_noop() {
    // e-TeX §275 applies the `eq_word_define` reassignment shortcut to every
    // fullword eqtb location, including the code tables. The nested identical
    // assignment must not create a save-stack entry that can roll back over
    // the later global assignment.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        tex_command::install_tex82_expandable_primitives(stores);
        tex_command::install_etex_expandable_primitives(stores);
        crate::install_unexpandable_primitives(stores);
        crate::install_etex_unexpandable_primitives(stores);
        let mut control = MainControl::prepared_initex(CommandProfile::ETEX26);
        register_source(&mut control, br"{\lccode`A=`a \global\lccode`A=`z}\end");
        let mut observations = ObservationRecorder::default();
        loop {
            match control
                .step_with_observer(stores, &mut observations)
                .expect("e-TeX code-table reassignments execute")
            {
                MainControlStep::End | MainControlStep::EndOfInput => break,
                MainControlStep::Continue => {}
            }
        }

        assert_eq!(
            admitted!(stores, |context| context.lccode('A')),
            u32::from('z')
        );
        let mutations: Vec<_> = observations
            .0
            .iter()
            .filter_map(|observation| match observation {
                CommandObservation::Mutation(record)
                    if record.target == MutationTarget::CodeTable =>
                {
                    Some((&record.key, &record.value, record.global))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            mutations,
            [(
                &ObservationValue::Name("lccode:65".into()),
                &ObservationValue::Integer(122),
                true,
            )]
        );
    });
}

#[test]
fn etex_zero_glue_parameter_reassignment_uses_canonical_pointer_identity() {
    // e-TeX §277 suppresses a local `eq_define` when both its type and
    // halfword identity are unchanged. TeX82 §1237 traps a scanned zero glue
    // specification to the shared `zero_glue` pointer before that test.
    // Separately scanned equal nonzero literals remain distinct pointers and
    // are the negative control.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        tex_command::install_tex82_expandable_primitives(stores);
        tex_command::install_etex_expandable_primitives(stores);
        crate::install_unexpandable_primitives(stores);
        crate::install_etex_unexpandable_primitives(stores);
        let mut control = MainControl::prepared_initex(CommandProfile::ETEX26);
        register_source(
            &mut control,
            br"\parfillskip=0pt \parfillskip=1pt \parfillskip=1pt \end",
        );
        let mut observations = ObservationRecorder::default();
        loop {
            match control
                .step_with_observer(stores, &mut observations)
                .expect("e-TeX glue-parameter reassignments execute")
            {
                MainControlStep::End | MainControlStep::EndOfInput => break,
                MainControlStep::Continue => {}
            }
        }

        let mutations: Vec<_> = observations
            .0
            .iter()
            .filter_map(|observation| match observation {
                CommandObservation::Mutation(record)
                    if observation_name(&record.key) == Some("glue_parameter:14") =>
                {
                    Some(&record.value)
                }
                _ => None,
            })
            .collect();
        assert_eq!(mutations.len(), 2);
        assert_eq!(
            admitted!(stores, |context| {
                let glue = context
                    .glue_param(GlueParam::new(14))
                    .expect("glue parameter");
                context.glue(glue).width
            }),
            Scaled::from_raw(65_536)
        );
    });
}

#[test]
fn etex_signed_internal_glue_is_not_an_identical_pointer_reassignment() {
    // TeX82 §§430/461 negate all three components of an internal glue
    // specification. The transformed value no longer has the source register's
    // pointer identity, so e-TeX §277 must not discard either assignment as a
    // same-pointer reassignment.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = etex_initex(stores);
        register_source(
            &mut control,
            br"\skip0=1pt plus 2fil minus 3fill \skip0=-\skip0
               \muskip0=4mu plus 5fill minus 6fil \muskip0=-\muskip0 \end",
        );

        run_to_end(&mut control, stores);

        admitted!(stores, |context| {
            let skip = context.glue(
                context
                    .glue_register(0)
                    .expect("skip register")
                    .expect("assigned skip"),
            );
            assert_eq!(skip.width, Scaled::from_raw(-Scaled::UNITY));
            assert_eq!(skip.stretch, Scaled::from_raw(-2 * Scaled::UNITY));
            assert_eq!(skip.stretch_order, tex_state::glue::Order::Fil);
            assert_eq!(skip.shrink, Scaled::from_raw(-3 * Scaled::UNITY));
            assert_eq!(skip.shrink_order, tex_state::glue::Order::Fill);

            let muskip = context.glue(context.muskip(0).expect("assigned muskip"));
            assert_eq!(muskip.width, Scaled::from_raw(-4 * Scaled::UNITY));
            assert_eq!(muskip.stretch, Scaled::from_raw(-5 * Scaled::UNITY));
            assert_eq!(muskip.stretch_order, tex_state::glue::Order::Fill);
            assert_eq!(muskip.shrink, Scaled::from_raw(-6 * Scaled::UNITY));
            assert_eq!(muskip.shrink_order, tex_state::glue::Order::Fil);
        });
    });
}

#[test]
fn etex_glue_expression_reassignment_retains_source_pointer_identity() {
    // e-TeX expression change [53a.4945--5360] leaves a glue factor's node
    // untouched when no operator requires a copy. Section 277 therefore
    // classifies the local assignment back to the same register as a
    // reassignment. An equal literal, an expression that applies an operator,
    // and a global assignment are controls: all allocate or define and remain
    // observable.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        tex_command::install_tex82_expandable_primitives(stores);
        tex_command::install_etex_expandable_primitives(stores);
        crate::install_unexpandable_primitives(stores);
        crate::install_etex_unexpandable_primitives(stores);
        let mut control = MainControl::prepared_initex(CommandProfile::ETEX26);
        register_source(
        &mut control,
        br"\skip0=1pt \skip0=\glueexpr\skip0\relax \skip0=1pt \skip0=\glueexpr\skip0+0pt\relax \global\skip0=\glueexpr\skip0\relax \end",
    );
        let mut observations = ObservationRecorder::default();
        loop {
            match control
                .step_with_observer(stores, &mut observations)
                .expect("e-TeX glue-expression reassignments execute")
            {
                MainControlStep::End | MainControlStep::EndOfInput => break,
                MainControlStep::Continue => {}
            }
        }

        let mutations: Vec<_> = observations
            .0
            .iter()
            .filter_map(|observation| match observation {
                CommandObservation::Mutation(record)
                    if observation_name(&record.key) == Some("skip:0") =>
                {
                    Some(record.global)
                }
                _ => None,
            })
            .collect();
        assert_eq!(mutations, [false, false, false, true]);
    });
}

#[test]
fn etex_sparse_skip_reassignment_keeps_sa_def_mutation_boundary() {
    // e-TeX 2.6 [49.1221--1237] sends the sparse shorthand through `sa_def`.
    // Its identical-pointer branch avoids saving or rewriting the element but
    // still completes the sparse assignment boundary, unlike §§277-278's
    // dense `eq_define` shortcut.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        tex_command::install_tex82_expandable_primitives(stores);
        tex_command::install_etex_expandable_primitives(stores);
        crate::install_unexpandable_primitives(stores);
        crate::install_etex_unexpandable_primitives(stores);
        let mut control = MainControl::prepared_initex(CommandProfile::ETEX26);
        register_source(
            &mut control,
            br"\skipdef\alias=32767 \alias=1pt \alias=\glueexpr\alias\relax \end",
        );
        let mut observations = ObservationRecorder::default();
        run_to_end_observed(&mut control, stores, &mut observations);

        let mutations = observations
            .0
            .iter()
            .filter(|observation| {
                matches!(
                    observation,
                    CommandObservation::Mutation(record)
                        if observation_name(&record.key) == Some("skip:32767")
                )
            })
            .count();
        assert_eq!(mutations, 2);
        assert_eq!(
            admitted!(stores, |context| {
                let glue = context
                    .glue_register(32_767)
                    .expect("skip register")
                    .expect("assigned sparse skip");
                context.glue(glue).width
            }),
            Scaled::from_raw(Scaled::UNITY)
        );
    });
}

#[test]
fn etex_penalty_array_assignments_are_mode_complete_and_consume_exactly_their_values() {
    // e-TeX 2.6 change [49.1248] routes all four selectors through
    // TeX82 §1248's `set_shape`; e-TeX §§6336-6366 define the selector
    // family and its repeated-last-value enquiry semantics.
    const MODES: [Mode; 6] = [
        Mode::Vertical,
        Mode::InternalVertical,
        Mode::Horizontal,
        Mode::RestrictedHorizontal,
        Mode::Math,
        Mode::DisplayMath,
    ];
    const ARRAYS: [(&str, PenaltyArrayKind); 4] = [
        ("interlinepenalties", PenaltyArrayKind::InterLine),
        ("clubpenalties", PenaltyArrayKind::Club),
        ("widowpenalties", PenaltyArrayKind::Widow),
        ("displaywidowpenalties", PenaltyArrayKind::DisplayWidow),
    ];

    for (name, kind) in ARRAYS {
        for mode in MODES {
            crate::test_harness::with_nonstop_plain_universe(|stores| {
                let mut control = etex_initex(stores);
                if mode != Mode::Vertical {
                    control.modes.push(mode).expect("test mode push");
                }
                let source = format!(r"\{name}  =  2  101  -202 \count0=17");
                register_source(&mut control, source.as_bytes());

                assert_eq!(
                    control.step(stores).expect("penalty array assignment"),
                    MainControlStep::Continue,
                    "selector {name}, mode {mode:?}"
                );
                assert_eq!(
                    admitted!(stores, |context| context.penalty_array(kind)),
                    vec![101, -202]
                );
                assert_eq!(
                    stores.count(0).expect("count register"),
                    0,
                    "following command was not consumed"
                );
                assert_eq!(control.current_mode(), mode);

                assert_eq!(
                    control.step(stores).expect("following assignment"),
                    MainControlStep::Continue,
                    "selector {name}, mode {mode:?}"
                );
                assert_eq!(
                    stores.count(0).expect("count register"),
                    17,
                    "following command stayed live"
                );
            });
        }
    }
}

#[test]
fn etex_penalty_array_mutations_use_their_extended_token_register_slots() {
    // e-TeX 2.6 [17.230] inserts these eqtb entries after the 256 dense token
    // registers, and [49.1248] assigns each with `define`.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = etex_initex(stores);
        register_source(
            &mut control,
            br"\interlinepenalties=1 10
           \global\clubpenalties=1 20
           \widowpenalties=1 30
           \global\displaywidowpenalties=1 40 \end",
        );
        let mut observations = ObservationRecorder::default();
        run_to_end_observed(&mut control, stores, &mut observations);

        let mutations = observations
            .0
            .iter()
            .filter_map(|observation| match observation {
                CommandObservation::Mutation(record)
                    if record.target == MutationTarget::Register =>
                {
                    Some((
                        observation_name(&record.key),
                        observation_tokens(&record.value),
                        record.global,
                    ))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            mutations,
            [
                (Some("toks:256"), Some([].as_slice()), false),
                (Some("toks:257"), Some([].as_slice()), true),
                (Some("toks:258"), Some([].as_slice()), false),
                (Some("toks:259"), Some([].as_slice()), true),
            ]
        );
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
fn etex_nonpositive_penalty_array_counts_clear_without_consuming_following_tokens() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = etex_initex(stores);
        register_source(
            &mut control,
            br"\interlinepenalties=1 11 \interlinepenalties=0
           \clubpenalties=1 22 \clubpenalties=-1
           \widowpenalties=1 33 \widowpenalties=0
           \displaywidowpenalties=1 44 \displaywidowpenalties=-2
           \count0=19 \end",
        );

        run_to_end(&mut control, stores);

        for kind in [
            PenaltyArrayKind::InterLine,
            PenaltyArrayKind::Club,
            PenaltyArrayKind::Widow,
            PenaltyArrayKind::DisplayWidow,
        ] {
            assert!(
                admitted!(stores, |context| context.penalty_array(kind)).is_empty(),
                "array {kind:?}"
            );
        }
        assert_eq!(
            stores.count(0).expect("count register"),
            19,
            "zero and negative counts scan no values"
        );
    });
}

#[test]
fn etex_penalty_array_scope_enquiries_and_afterassignment_match_set_shape() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = etex_initex(stores);
        register_source(
            &mut control,
            br"\clubpenalties=2 200 100
           {\clubpenalties=1 7}
           \widowpenalties=2 300 400
           {\widowpenalties=1 7}
           {\globaldefs=1 \displaywidowpenalties=1 500}
           \interlinepenalties=2 9 8
           {\globaldefs=-1 \global\interlinepenalties=-4}
           \def\aftermark{\global\advance\count0 by1}
           \afterassignment\aftermark\clubpenalties=1 42
           \end",
        );

        run_to_end(&mut control, stores);

        assert_eq!(
            admitted!(stores, |context| context
                .penalty_array_value(PenaltyArrayKind::Widow, 0)),
            2
        );
        assert_eq!(
            admitted!(stores, |context| context
                .penalty_array_value(PenaltyArrayKind::Widow, 1)),
            300
        );
        assert_eq!(
            admitted!(stores, |context| context
                .penalty_array_value(PenaltyArrayKind::Widow, 8)),
            400
        );
        assert_eq!(
            admitted!(stores, |context| context
                .penalty_array(PenaltyArrayKind::Club)),
            vec![42]
        );
        assert_eq!(
            admitted!(stores, |context| context
                .penalty_array(PenaltyArrayKind::Widow)),
            vec![300, 400]
        );
        assert_eq!(
            admitted!(stores, |context| context
                .penalty_array(PenaltyArrayKind::DisplayWidow)),
            vec![500]
        );
        assert_eq!(
            admitted!(stores, |context| context
                .penalty_array(PenaltyArrayKind::InterLine)),
            vec![9, 8]
        );
        assert_eq!(
            stores.count(0).expect("count register"),
            1,
            "afterassignment fired exactly once"
        );
    });
}

#[test]
fn etex_penalty_array_assignment_restores_checkpoint_and_retries_atomically() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = etex_initex(stores);
        register_source(&mut control, br"\clubpenalties=2 7 5 \count0=23 \end");
        let checkpoint = control
            .capture_checkpoint(
                crate::EngineBoundary::OuterParagraphEnd,
                stores,
                crate::ExecutionBudgetCounters::default(),
            )
            .expect("penalty array state checkpoints");

        assert_eq!(
            control.step(stores).expect("first assignment"),
            MainControlStep::Continue
        );
        assert_eq!(
            admitted!(stores, |context| context
                .penalty_array(PenaltyArrayKind::Club)),
            vec![7, 5]
        );
        let assigned_hash = stores.journal_cursor().expect("state cursor");

        control
            .restore_checkpoint(&checkpoint, stores)
            .expect("penalty array state restores");
        assert!(
            admitted!(stores, |context| context
                .penalty_array(PenaltyArrayKind::Club))
            .is_empty()
        );
        assert_eq!(stores.count(0).expect("count register"), 0);

        assert_eq!(
            control.step(stores).expect("retried assignment"),
            MainControlStep::Continue
        );
        assert_eq!(
            stores.journal_cursor().expect("state cursor"),
            assigned_hash
        );
        assert_eq!(
            admitted!(stores, |context| context
                .penalty_array(PenaltyArrayKind::Club)),
            vec![7, 5]
        );
        assert_eq!(
            control.step(stores).expect("following assignment"),
            MainControlStep::Continue
        );
        assert_eq!(stores.count(0).expect("count register"), 23);
    });
}

#[test]
fn main_control_dispatch_matrix_consumes_each_command_once() {
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
            let mut control = MainControl::tex82_initex(stores);
            if mode != Mode::Vertical {
                control.modes.push(mode).expect("test mode push");
            }
            register_source(&mut control, br"\count0=17\count1=29");

            let mut observations = ObservationRecorder::default();
            assert_eq!(
                control
                    .step_with_observer(stores, &mut observations)
                    .expect("mode-independent assignment dispatches"),
                MainControlStep::Continue,
                "mode {mode:?}"
            );
            assert_eq!(
                stores.count(0).expect("count register"),
                17,
                "mode {mode:?}"
            );
            assert_eq!(stores.count(1).expect("count register"), 0, "mode {mode:?}");
            assert_eq!(control.current_mode(), mode);
            assert_eq!(
                observations
                    .0
                    .iter()
                    .filter(|observation| matches!(observation, CommandObservation::Mutation(_)))
                    .count(),
                1,
                "one main-control mutation committed in mode {mode:?}: {:?}",
                observations.0
            );
            assert!(observations.0.iter().any(|observation| matches!(
                observation,
                CommandObservation::Mutation(mutation)
                    if mutation.key == ObservationValue::Name("count:0".into())
                        && mutation.value == ObservationValue::Integer(17)
            )));

            observations.0.clear();
            assert_eq!(
                control
                    .step_with_observer(stores, &mut observations)
                    .expect("following command remains available"),
                MainControlStep::Continue,
                "mode {mode:?}"
            );
            assert_eq!(
                stores.count(1).expect("count register"),
                29,
                "mode {mode:?}"
            );
            assert_eq!(
                observations
                    .0
                    .iter()
                    .filter(|observation| matches!(observation, CommandObservation::Mutation(_)))
                    .count(),
                1,
                "the following command commits exactly once in mode {mode:?}"
            );
            assert!(observations.0.iter().any(|observation| matches!(
                observation,
                CommandObservation::Mutation(mutation)
                    if mutation.key == ObservationValue::Name("count:1".into())
                        && mutation.value == ObservationValue::Integer(29)
            )));
        });
    }
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
                    page.step_with_observer(page_stores, &mut observations)
                        .expect("page stop remains finite"),
                    MainControlStep::End | MainControlStep::EndOfInput
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
                    .step_with_observer(stores, &mut observations)
                    .unwrap_or_else(|error| panic!("{name} step {step}: {error}"));
                if matches!(result, MainControlStep::End | MainControlStep::EndOfInput) {
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
                stores.world().artifact_commits().len(),
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

#[test]
fn suspended_output_resume_preserves_end_job_progress_and_observation_order() {
    // TeX82 §§1025--1026/1054: immutable input acquisition inside an
    // explicit output routine may suspend the host episode, but it cannot
    // publish or roll back the page-builder progress that admitted that
    // routine. Resumption must match a run where the resource was present
    // from the start, including the stop/shipout/termination order.
    let source = br"\output={\input child\shipout\box255}\hrule\end";
    let child =
        SourceRegistration::new(RegisteredSourceKind::Generated, Arc::<[u8]>::from(&b""[..]));

    crate::test_harness::with_nonstop_plain_universe(|retried_stores| {
        let mut retried_control = MainControl::tex82_initex(retried_stores);
        register_source(&mut retried_control, source);
        let mut retried = ObservationRecorder::default();
        for _ in 0..3 {
            loop {
                match retried_control
                    .advance_with_observer(retried_stores, &mut retried)
                    .expect("output routine advances to its input resource")
                {
                    StepResult::Suspended(ResourceNeed::Input { name, .. }) => {
                        assert_eq!(name, "child.tex");
                        break;
                    }
                    StepResult::Progress(ReplayStep::Continue) => {}
                    other => panic!("output resource reached {other:?}"),
                }
            }
        }
        retried_control
            .capabilities_mut()
            .register_input("child.tex", child.clone());
        run_to_end_observed(&mut retried_control, retried_stores, &mut retried);

        crate::test_harness::with_nonstop_plain_universe(|direct_stores| {
            let mut direct_control = MainControl::tex82_initex(direct_stores);
            direct_control
                .capabilities_mut()
                .register_input("child.tex", child);
            register_source(&mut direct_control, source);
            let mut direct = ObservationRecorder::default();
            run_to_end_observed(&mut direct_control, direct_stores, &mut direct);

            assert_eq!(retried.0, direct.0);
            assert_eq!(retried_stores.world().artifact_commits().len(), 1);
            assert_eq!(direct_stores.world().artifact_commits().len(), 1);
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
fn readline_assignment_trace_precedes_the_next_command_trace() {
    // TeX82 §1225 calls `define(p,call,cur_val)` as soon as `read_toks`
    // returns. e-TeX [17.687-750] therefore renders both halves of that eqtb
    // write before §299 can trace the following command.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        stores.set_interaction_mode(tex_state::InteractionMode::Scroll);
        stores
            .world_mut()
            .push_memory_terminal_line("replacement")
            .expect("terminal line queues");
        let mut control = etex_initex(stores);
        register_source(
        &mut control,
        br"\def\line{\begingroup\scantokens{\message{level=\the\currentgrouplevel}}}\tracingassigns=1\tracingcommands=2\readline16to\line\endlinechar=-1\end",
    );

        run_to_end(&mut control, stores);

        let log = pending_sink_text(stores, false);
        let changing = log
            .find("{changing \\line =macro:->\\begingroup \\scantokens {\\message \\ETC.}")
            .unwrap_or_else(|| panic!("missing read target pre-image: {log:?}"));
        let into = log
            .find("{into \\line =macro:->replacement")
            .unwrap_or_else(|| panic!("missing read target post-image: {log:?}"));
        let next = log
            .find("{\\endlinechar}")
            .unwrap_or_else(|| panic!("missing following command trace: {log:?}"));
        assert!(changing < into && into < next, "{log:?}");
    });
}

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
                    .step_with_observer(stores, &mut observations)
                    .expect("recovered read remains executable"),
                MainControlStep::End | MainControlStep::EndOfInput
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
fn read_to_definition_preserves_effective_scope_and_replay() {
    // TeX82 §§1214/1225 select scope before `read_toks`, then install its
    // parameterless macro after collection. Exercise explicit prefixes and
    // both `\globaldefs` overrides through ordinary replay.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        // `\read-1` first reports §433's out-of-range stream number. Keep this
        // scope/replay test in scroll mode so §82's error-stop dialog does not
        // canonically consume the terminal lines intended for the reads.
        stores.set_interaction_mode(tex_state::InteractionMode::Scroll);
        for line in ["local", "explicit", "forced-global", "forced-local"] {
            stores
                .world_mut()
                .push_memory_terminal_line(line)
                .expect("memory terminal accepts a line");
        }
        let mut control = MainControl::tex82_initex(stores);
        register_source(
        &mut control,
        br"\def\local{old}{\read-1to\local}\def\explicit{old}{\global\read-1to\explicit}\globaldefs=1\def\forcedglobal{old}{\read-1to\forcedglobal}\globaldefs=-1\gdef\forcedlocal{old}{\global\read-1to\forcedlocal}\globaldefs=0\end",
    );
        run_to_end(&mut control, stores);

        assert_eq!(
            macro_semantic_tokens(stores, "local")[0],
            Token::Char {
                ch: 'o',
                cat: Catcode::Letter,
            }
        );
        assert_eq!(
            macro_semantic_tokens(stores, "explicit")[0],
            Token::Char {
                ch: 'e',
                cat: Catcode::Letter,
            }
        );
        assert_eq!(
            macro_semantic_tokens(stores, "forcedglobal")[0],
            Token::Char {
                ch: 'f',
                cat: Catcode::Letter,
            }
        );
        assert_eq!(
            macro_semantic_tokens(stores, "forcedlocal")[0],
            Token::Char {
                ch: 'o',
                cat: Catcode::Letter,
            }
        );
    });
}

#[test]
fn read_to_mutation_precedes_afterassignment_replay_and_carries_exact_meaning() {
    // TeX82 §1225 commits `define(p,call,cur_val)` before §1211 reaches
    // §1269's `done:` and backs up the saved afterassignment token.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        stores.set_interaction_mode(tex_state::InteractionMode::Scroll);
        stores
            .world_mut()
            .push_memory_terminal_line("alpha")
            .expect("memory terminal accepts a line");
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\def\target{old}\afterassignment\relax\global\read-1to\target\end",
        );
        let mut observations = ObservationRecorder::default();
        loop {
            if matches!(
                control
                    .step_with_observer(stores, &mut observations)
                    .expect("read and its replay execute"),
                MainControlStep::End | MainControlStep::EndOfInput
            ) {
                break;
            }
        }

        let mutation_index = observations
            .0
            .iter()
            .position(|observation| {
                matches!(
                    observation,
                    CommandObservation::Mutation(record)
                        if observation_name(&record.key) == Some("target")
                            && matches!(record.value, ObservationValue::Tokens(_))
                            && record.global
                )
            })
            .expect("read meaning mutation is observed");
        let replay_index = observations
            .0
            .iter()
            .enumerate()
            .skip(mutation_index + 1)
            .position(|observation| {
                matches!(
                    observation.1,
                    CommandObservation::Input(record)
                        if record.transition == InputTransition::Backup
                            && record.reason == InputReason::Backup
                )
            })
            .map(|offset| mutation_index + 1 + offset)
            .expect("afterassignment replay is observed");
        assert!(mutation_index < replay_index, "{:?}", observations.0);
        let CommandObservation::Mutation(mutation) = &observations.0[mutation_index] else {
            unreachable!()
        };
        assert!(matches!(
            observation_tokens(&mutation.value),
            Some([
                tex_command::ObservedToken::MacroEndMatch,
                tex_command::ObservedToken::Character {
                    character: 'a',
                    catcode: Catcode::Letter,
                },
                ..
            ])
        ));
    });
}

#[test]
fn hot_definition_publication_precedes_afterassignment_and_its_host_effect() {
    // TeX82 §§1211/1269 commits `define`, publishes its mutation evidence,
    // and invokes §325 `back_input` before the saved macro can execute its
    // §1375 immediate write on the following main-control operation.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\def\mark{\immediate\write16{after}}\def\target{old}\afterassignment\mark\global\def\target{new}\end",
        );
        let mut observations = ObservationRecorder::default();
        loop {
            if matches!(
                control
                    .step_with_observer(stores, &mut observations)
                    .expect("hot definition and saved token execute"),
                MainControlStep::End | MainControlStep::EndOfInput
            ) {
                break;
            }
        }

        let mutation = observations
            .0
            .iter()
            .rposition(|observation| {
                matches!(
                    observation,
                    CommandObservation::Mutation(record)
                        if observation_name(&record.key) == Some("target") && record.global
                )
            })
            .expect("global hot definition mutation");
        let backup = observations
            .0
            .iter()
            .enumerate()
            .skip(mutation + 1)
            .find_map(|(index, observation)| {
                matches!(
                    observation,
                    CommandObservation::Input(record)
                        if record.transition == InputTransition::Backup
                            && record.reason == InputReason::Backup
                )
                .then_some(index)
            })
            .expect("afterassignment backup");
        let write = observations
            .0
            .iter()
            .enumerate()
            .skip(backup + 1)
            .find_map(|(index, observation)| {
                matches!(
                    observation,
                    CommandObservation::Effect(record)
                        if record.kind == ObservationEffectKind::Write
                )
                .then_some(index)
            })
            .expect("saved macro host effect");
        assert!(mutation < backup && backup < write, "{:?}", observations.0);
        assert_eq!(macro_character_text(stores, "target"), "new");
    });
}

#[test]
fn hot_definition_checkpoint_restore_replays_one_atomic_mutation() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\global\def\target{new}\end");
        let checkpoint = control
            .capture_checkpoint(
                crate::EngineBoundary::OuterParagraphEnd,
                stores,
                crate::ExecutionBudgetCounters::default(),
            )
            .expect("quiescent hot definition checkpoints");

        assert_eq!(
            control.step(stores).expect("hot definition executes"),
            MainControlStep::Continue
        );
        assert_eq!(macro_character_text(stores, "target"), "new");
        control
            .restore_checkpoint(&checkpoint, stores)
            .expect("hot definition checkpoint restores");
        admitted!(stores, |context| {
            let target = context.intern_control_sequence("target");
            assert!(matches!(
                context.meaning(target),
                ResolvedMeaning::Static(Meaning::Undefined)
            ));
        });
        assert_eq!(
            control.step(stores).expect("hot definition retries"),
            MainControlStep::Continue
        );
        assert_eq!(macro_character_text(stores, "target"), "new");
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
fn error_stop_deletes_requested_tokens_before_retry() {
    // TeX82 §§84--85: a one- or two-digit response consumes that many
    // unexpanded tokens, displays the resulting context, and prompts again.
    crate::test_harness::with_plain_universe(|stores| {
        stores
            .world_mut()
            .push_memory_terminal_line("2")
            .expect("deletion response queues");
        stores
            .world_mut()
            .push_memory_terminal_line("")
            .expect("retry response queues");
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\show\errorstopmode ab\count0=17\end");

        run_to_end(&mut control, stores);

        assert_eq!(
            stores.count(0).expect("count register"),
            17,
            "only the two ignored letters disappear"
        );
        let terminal = terminal_text(stores);
        assert_eq!(terminal.matches("? ").count(), 2, "{terminal:?}");
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
                .step_with_observer(stores, &mut observations)
                .expect("showlists executes")
            {
                MainControlStep::End | MainControlStep::EndOfInput => break,
                MainControlStep::Continue => {}
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
                    control.step(stores).expect("setup command executes"),
                    MainControlStep::Continue
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
fn etex_showbox_invalid_register_checkpoint_retry_recovers_to_zero() {
    // e-TeX 2.6 etex.ch [49.1296] replaces TeX82's `scan_eight_bit_int`
    // with `scan_register_num`, whose restricted scan diagnoses -1, recovers
    // it to zero, and leaves the following token for the next command.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = etex_initex(stores);
        control.set_fuel_limit(1_000).expect("bounded fuel");
        register_source(&mut control, br"\showbox-1\count0=23\end");
        let checkpoint = control
            .capture_checkpoint(
                crate::EngineBoundary::OuterParagraphEnd,
                stores,
                crate::ExecutionBudgetCounters::default(),
            )
            .expect("showbox checkpoints");

        assert_eq!(
            control
                .step(stores)
                .expect("invalid showbox register recovers"),
            MainControlStep::Continue
        );
        assert_eq!(
            stores.count(0).expect("count register"),
            0,
            "following assignment remains unread"
        );
        let first_hash = stores.journal_cursor().expect("state cursor");
        let first_output = terminal_text(stores);
        assert!(
            first_output.contains("Bad register code (-1)"),
            "{first_output}"
        );
        assert!(first_output.contains("> \\box0="), "{first_output}");

        control
            .restore_checkpoint(&checkpoint, stores)
            .expect("showbox state restores");
        assert_eq!(
            control
                .step(stores)
                .expect("invalid showbox register retries identically"),
            MainControlStep::Continue
        );
        assert_eq!(stores.journal_cursor().expect("state cursor"), first_hash);
        assert_eq!(terminal_text(stores), first_output);

        run_to_end(&mut control, stores);
        assert_eq!(
            stores.count(0).expect("count register"),
            23,
            "following token executes after recovery"
        );
        assert!(control.fuel_burned() < 1_000);
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
fn valign_cell_endv_closes_an_open_paragraph_before_fin_col() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\catcode`\#=6 \catcode`\&=4
           \setbox0=\hbox{\valign{#\cr x\cr}}
           \ifhmode\count0=2\else\count0=1\fi
           \end",
        );

        run_to_end(&mut control, stores);

        // TeX82 §1131 runs `end_graf` before `fin_col`. The paragraph opened by
        // `x` is therefore closed before the valign cell, row, alignment, and
        // enclosing hbox levels are packaged in order.
        assert_eq!(stores.count(0).expect("count register"), 1);
        assert_eq!(control.current_mode(), Mode::Vertical);
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

/// Collects every `\setlanguage` whatsit inside box register zero.
fn language_whatsits<G>(stores: &mut Universe<G>) -> Vec<(u8, u8, u8)> {
    let outer = stores
        .copy_box_to_page(0)
        .expect("box 0 holds the constructed hbox");
    let Some(Node::HList(boxed)) = first_published_node(stores, outer) else {
        panic!("box 0 holds an hlist");
    };
    page_vec(stores, boxed.children)
        .iter()
        .filter_map(|node| match node {
            Node::Whatsit(tex_state::node::Whatsit::Language {
                language,
                left_hyphen_min,
                right_hyphen_min,
            }) => Some((*language, *left_hyphen_min, *right_hyphen_min)),
            _ => None,
        })
        .collect()
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

/// TeX82 §796/§798's spanned-column packaging, at and just past its bound.
///
/// `#&&#` is a periodic preamble, so a body entry can span arbitrarily many
/// columns. §796 sets `n:=min_quarterword`, "this represents a span count of
/// 1", and §798 then runs `repeat incr(n); q:=link(link(q)); until q=cur_align`
/// over the spanned columns, so `n` is the number of `\span` delimiters.
/// §110's `max_quarterword` is 255.
fn spanning_alignment_source(spans: &str) -> Vec<u8> {
    format!(
        concat!(
            r"\catcode`{{=1 \catcode`}}=2 \catcode`\#=6 \catcode`\&=4",
            "\n",
            r"\def\a{{\span}}\def\b{{\a\a}}\def\c{{\b\b}}\def\d{{\c\c}}",
            "\n",
            r"\def\e{{\d\d}}\def\f{{\e\e}}\def\g{{\f\f}}\def\h{{\g\g}}\def\i{{\h\h}}",
            "\n",
            r"\setbox0=\vbox{{\halign{{#&&#\cr\relax{spans}\relax\cr}}}}",
            "\n",
            r"\global\count0=1\end",
            "\n",
        ),
        spans = spans
    )
    .into_bytes()
}

#[test]
fn two_hundred_fifty_five_span_steps_stay_within_section_798s_bound() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        // 128+64+32+16+8+4+2+1 = 255 `\span` delimiters, so §798's `n` is exactly
        // `max_quarterword` and the guard `n>max_quarterword` does not fire.
        register_source(
            &mut control,
            &spanning_alignment_source(r"\h\g\f\e\d\c\b\a"),
        );

        run_to_end(&mut control, stores);

        assert_eq!(control.fatal_error(), None);
        assert_eq!(
            stores.count(0).expect("count register"),
            1,
            "the job ran on to \\global\\count0=1"
        );
    });
}

#[test]
fn two_hundred_fifty_six_span_steps_succumb_to_section_798s_confusion() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        // `\i` is 2^8 = 256 `\span` delimiters, so §798's `n` is 256 and
        // `if n>max_quarterword then confusion("256 spans")` fires.
        register_source(&mut control, &spanning_alignment_source(r"\i"));

        run_to_end(&mut control, stores);

        assert_eq!(
            control.fatal_error(),
            Some(FatalError::confusion("256 spans"))
        );
        // §93 `succumb` calls §81 `jump_out`, so nothing after the alignment runs.
        assert_eq!(stores.count(0).expect("count register"), 0);
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
                control.step(stores).expect("a terminal session reports"),
                MainControlStep::End,
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
                .step_with_observer(stores, &mut observations)
                .expect("a fatal error is a terminal state, never an Err")
            {
                MainControlStep::End | MainControlStep::EndOfInput => break,
                MainControlStep::Continue => {}
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
fn setbox_scope_is_globaldefs_adjusted_before_the_box_is_scanned() {
    // TeX82 §1214's `<Adjust for the setting of \globaldefs>` runs inside
    // `prefixed_command`, so a positive `\globaldefs` makes an unprefixed
    // `\setbox` global and a negative one makes `\global\setbox` local.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
        &mut control,
        br"\globaldefs=1 {\setbox0=\hbox{\kern1pt}}\globaldefs=-1 {\global\setbox1=\hbox{\kern1pt}}\globaldefs=0 \end",
    );
        run_to_end(&mut control, stores);

        assert!(
            stores.copy_box_to_page(0).is_some(),
            "positive globaldefs is global"
        );
        assert!(
            stores.copy_box_to_page(1).is_none(),
            "negative globaldefs is local"
        );
    });
}

#[test]
fn effective_scope_is_shared_by_provisional_and_committed_meaning_mutations() {
    // TeX82 §§1211/1214 resolve the assignment scope before §1224/§1257
    // install their provisional meanings. §§277-279 then expose that same
    // resolved choice for both provisional and final definitions.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
        &mut control,
        br"{\globaldefs=1\chardef\forcedchar=65\countdef\forcedregister=2}{\globaldefs=-1\global\chardef\localchar=66\global\countdef\localregister=3}\globaldefs=0\end",
    );
        let mut observations = ObservationRecorder::default();
        loop {
            if matches!(
                control
                    .step_with_observer(stores, &mut observations)
                    .expect("scope matrix executes"),
                MainControlStep::End | MainControlStep::EndOfInput
            ) {
                break;
            }
        }

        for (name, expected_global) in [
            ("forcedchar", true),
            ("forcedregister", true),
            ("localchar", false),
            ("localregister", false),
        ] {
            let scopes: Vec<_> = observations
                .0
                .iter()
                .filter_map(|observation| match observation {
                    CommandObservation::Mutation(record)
                        if observation_name(&record.key) == Some(name) =>
                    {
                        Some(record.global)
                    }
                    _ => None,
                })
                .collect();
            assert!(!scopes.is_empty(), "{name} has an observed mutation");
            assert!(
                scopes.iter().all(|scope| *scope == expected_global),
                "{name} used one effective scope across provisional and final mutations: {scopes:?}"
            );
        }

        for name in ["forcedchar", "forcedregister"] {
            assert_ne!(
                admitted!(stores, |context| {
                    let symbol = context.intern_control_sequence(name);
                    context.meaning(symbol)
                }),
                ResolvedMeaning::Static(Meaning::Undefined),
                "{name} survived its group"
            );
        }
        for name in ["localchar", "localregister"] {
            assert_eq!(
                admitted!(stores, |context| {
                    let symbol = context.intern_control_sequence(name);
                    context.meaning(symbol)
                }),
                ResolvedMeaning::Static(Meaning::Undefined),
                "{name} was restored at group end"
            );
        }
    });
}

#[test]
fn every_non_eqtb_assignment_family_fires_afterassignment_once() {
    // TeX82 §1210 includes all ten families below in prefixed_command, and
    // §1269 reaches `done` after each completed assignment. The saved token
    // must enter through ordinary §325 back_input exactly once.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
        &mut control,
        br"\def\mark{\global\advance\count0 by1}\afterassignment\mark\nullfont\afterassignment\mark\textfont0=\nullfont\afterassignment\mark\setbox0=\hbox{}\afterassignment\mark\prevdepth=0pt x\afterassignment\mark\spacefactor=1000\par\afterassignment\mark\prevgraf=0\afterassignment\mark\pagegoal=1pt\afterassignment\mark\deadcycles=0\afterassignment\mark\hyphenation{word}\afterassignment\mark\nonstopmode\end",
    );
        run_to_end(&mut control, stores);

        assert_eq!(stores.count(0).expect("count register"), 10);
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
fn fontdimen_reports_an_unusable_parameter_number_and_leaves_the_font_alone() {
    // TeX82 §578 resolves `n<=0` to the scratch `fmem_ptr`; §579 reports it
    // and §1253 still consumes `=<dimen>`, so the next command runs.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\fontdimen0\nullfont=1pt \count0=1\end");
        run_to_end(&mut control, stores);

        assert_eq!(stores.count(0).expect("count register"), 1);
        assert_eq!(
            admitted!(stores, |context| context
                .hyphen_positions_for_language(0, "ab", 0, 0)),
            Vec::<usize>::new(),
            "§963 diagnoses the duplicate before replacing it with a2b"
        );
        let output = terminal_text(stores);
        assert!(
            output.contains("! Font \\nullfont has only 7 fontdimen parameters."),
            "{output}"
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
    let bytes = include_bytes!("../../../umber-wasm/assets/cmu-serif-500-roman.woff2").to_vec();
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
fn dimension_advance_accepts_the_negative_max_dimen_boundary() {
    // TeX82 §104 deliberately leaves dimension addition unchecked, and
    // §1238 applies `advance` with a plain sum. Thus `-max_dimen-1sp`
    // commits the representable `-2^30sp` value instead of setting
    // `arith_error`. This is the e-TRIP line-781 boundary case.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\dimen44=-1073741823sp \advance\dimen44 by-1sp \end",
        );
        let mut observations = ObservationRecorder::default();
        run_to_end_observed(&mut control, stores, &mut observations);

        assert_eq!(
            admitted!(stores, |context| context.dimen(44)),
            Scaled::from_raw(-1_073_741_824)
        );
        assert!(!terminal_text(stores).contains("Arithmetic overflow"));
        assert!(observations.0.iter().any(|observation| {
            matches!(
                observation,
                CommandObservation::Mutation(record)
                    if record.target == MutationTarget::Register
                        && observation_name(&record.key) == Some("dimen:44")
                        && record.value == ObservationValue::Scaled(-1_073_741_824)
                        && !record.global
            )
        }));
    });
}

#[test]
fn arithmetic_overflow_reports_and_leaves_the_target_unchanged() {
    // TeX82 §1236 returns before `word_define` when `arith_error` is set.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\count0=2000000000 \multiply\count0 by2 \count1=7 \divide\count1 by0 \count2=1\end",
        );
        run_to_end(&mut control, stores);

        assert_eq!(stores.count(0).expect("count register"), 2_000_000_000);
        assert_eq!(stores.count(1).expect("count register"), 7);
        assert_eq!(stores.count(2).expect("count register"), 1);
        let output = terminal_text(stores);
        assert_eq!(
            output.matches("! Arithmetic overflow.").count(),
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
fn frozen_page_scalar_rejection_is_checkpoint_atomic() {
    // TeX82 §1236 rejects set_page_dimen as an arithmetic target before
    // scanning an operand. Restoring the command checkpoint must restore both
    // the live frozen page values and the rejected target for an identical
    // retry through §1269's recovery path.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\nonstopmode \topskip=0pt \setbox0=\hbox{}\copy0
           \pagegoal=12pt \insertpenalties=4
           \advance\pagegoal by 3pt \edef\snapshot{\the\pagegoal/\the\insertpenalties}",
        );
        while admitted!(stores, |context| context
            .page_dimension(PageDimension::Goal))
        .raw()
            != 12 * Scaled::UNITY
            || admitted!(stores, |context| context
                .page_integer(PageInteger::InsertPenalties))
                != 4
        {
            assert_eq!(
                control.step(stores).expect("setup executes"),
                MainControlStep::Continue
            );
        }
        let checkpoint = control
            .capture_checkpoint(
                crate::EngineBoundary::OuterParagraphEnd,
                stores,
                crate::ExecutionBudgetCounters::default(),
            )
            .expect("frozen page checkpoint captures");

        run_to_end(&mut control, stores);
        let first_output = terminal_text(stores);
        let first_snapshot = macro_semantic_tokens(stores, "snapshot").to_vec();
        assert_eq!(
            admitted!(stores, |context| context
                .page_dimension(PageDimension::Goal))
            .raw(),
            12 * Scaled::UNITY
        );
        assert!(first_output.contains("You can't use `\\pagegoal' after \\advance"));

        control
            .restore_checkpoint(&checkpoint, stores)
            .expect("frozen page checkpoint restores");
        run_to_end(&mut control, stores);
        assert_eq!(
            admitted!(stores, |context| context
                .page_dimension(PageDimension::Goal))
            .raw(),
            12 * Scaled::UNITY
        );
        assert_eq!(
            admitted!(stores, |context| context
                .page_integer(PageInteger::InsertPenalties)),
            4
        );
        assert_eq!(macro_semantic_tokens(stores, "snapshot"), first_snapshot);
        assert_eq!(terminal_text(stores), first_output);
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
                .step_with_observer(isolated_stores, &mut isolated_observations)
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
fn invalid_arithmetic_target_commit_survives_later_resource_retry() {
    // The §1236 recovery and §1269 afterassignment replay are a committed
    // operation. A later missing-resource rollback cannot duplicate either.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\def\mark{\global\advance\count0 by1}
           \afterassignment\mark\advance x
           \input child\end",
        );

        for _ in 0..8 {
            if stores.count(0).expect("count register") == 1 {
                break;
            }
            assert!(matches!(
                control.advance(stores).expect("setup executes"),
                StepResult::Progress(ReplayStep::Continue)
            ));
        }
        assert_eq!(stores.count(0).expect("count register"), 1);
        let committed = terminal_text(stores);
        assert_eq!(committed.matches("the letter x").count(), 1);

        for _ in 0..3 {
            assert!(matches!(
                control.advance(stores).expect("missing input suspends"),
                StepResult::Suspended(ResourceNeed::Input {
                    name,
                    original_name,
                }) if name == "child.tex" && original_name == "child"
            ));
            assert_eq!(stores.count(0).expect("count register"), 1);
            assert_eq!(terminal_text(stores), committed);
        }
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
fn valign_cell_paragraph_pack_retains_the_intercepted_delimiter_line() {
    // TeX82 §§789/1131/661: `\cr` is retained below the v-template and
    // delivered to `fin_col` only after synthetic `endv` runs `end_graf`.
    // The paragraph diagnostic still uses the delimiter's live input line.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            b"\\tracingonline=1\\hbadness=0\\hsize=0pt\\parindent=10pt\n\\setbox0=\\hbox{\\valign{#\\cr\n\\indent x\n\\cr % exhaust the delimiter's physical line\n}}\n\\end",
        );
        run_to_end_observed(&mut control, stores, &mut ObservationRecorder::default());

        let mut log = stores
            .world()
            .memory_log_output()
            .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
            .unwrap_or_default();
        log.push_str(&pending_sink_text(stores, false));
        assert!(
            log.contains("in paragraph at lines 3--4"),
            "alignment paragraph must retain its delimiter line: {log}"
        );
        assert!(!log.contains("lines 3--0"), "{log}");
    });
}

#[test]
fn alignment_setting_pack_retains_the_closing_brace_line() {
    // TeX82 §§800/661: `fin_align` negates the alignment's opening
    // `mode_line` while its closing right brace supplies the current line.
    // The cold setting pass therefore must retain that consumed delimiter's
    // line after the source delivery has completed.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            b"\\tracingonline=1\\hbadness=0\n\\setbox0=\\vbox{\\halign to100pt{#\\cr\nx\\cr\n}}\n\\end",
        );
        run_to_end_observed(&mut control, stores, &mut ObservationRecorder::default());

        let mut log = stores
            .world()
            .memory_log_output()
            .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
            .unwrap_or_default();
        log.push_str(&pending_sink_text(stores, false));
        assert!(
            log.contains("in alignment at lines 2--4"),
            "alignment setting must retain its closing-brace line: {log}"
        );
        assert!(!log.contains("in alignment at lines 2--0"), "{log}");
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
fn unavailable_font_retry_preserves_batch_mode_for_later_diagnostics() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(
            &mut control,
            br"\batchmode\font\missingfont=absent\missing\scrollmode\end",
        );
        let mut ledger = crate::OutputLedger::new();
        let mut checkpoints = Vec::new();
        let cancellation = crate::Cancellation::new();
        loop {
            match crate::CanonicalStepRunner::new(&mut control, stores, &mut ledger)
                .step_with_observer(
                    &mut checkpoints,
                    &cancellation,
                    &mut ObservationRecorder::default(),
                ) {
                crate::CanonicalStepResult::ResourceNeed(need @ ResourceNeed::Font { .. }) => {
                    ledger.mark_unavailable(&mut control, &need, false);
                }
                crate::CanonicalStepResult::Completed(_) => break,
                crate::CanonicalStepResult::Progress(_)
                | crate::CanonicalStepResult::Committed(_) => {}
                other => panic!("unexpected unavailable-font step: {other:?}"),
            }
        }

        assert_eq!(
            stores.interaction_mode(),
            tex_state::InteractionMode::Scroll
        );
        assert!(
            !pending_sink_text(stores, true).contains("Undefined control sequence"),
            "batch errors must not escape the log-only selector across a resource retry"
        );
        assert!(
            pending_sink_text(stores, false).contains("Undefined control sequence"),
            "batch errors remain in the transcript log across a resource retry"
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
fn math_group_collapses_only_one_undecorated_ord_nucleus() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let empty_list = tex_state::node_arena::PageListId::empty();
        let ch = MathChar {
            family: 0,
            character: 'x',
            origin: tex_state::token::OriginId::UNKNOWN,
        };
        for nucleus in [
            MathField::Empty,
            MathField::MathChar(ch),
            MathField::SubBox(empty_list),
            MathField::SubMlist(empty_list),
        ] {
            let list = crate::test_harness::publish_page_nodes(
                stores,
                [Node::MathNoad(MathNoad::new(
                    NoadKind::Normal(NoadClass::Ord),
                    nucleus,
                ))],
            );
            assert_eq!(
                collapse_singleton_math_group(
                    &stores.command_context().expect("live generation"),
                    list,
                ),
                nucleus
            );
        }

        let scripted = crate::test_harness::publish_page_nodes(
            stores,
            [Node::MathNoad(MathNoad {
                kind: NoadKind::Normal(NoadClass::Ord),
                nucleus: MathField::MathChar(ch),
                subscript: MathField::MathChar(ch),
                superscript: MathField::Empty,
            })],
        );
        let non_ord = crate::test_harness::publish_page_nodes(
            stores,
            [Node::MathNoad(MathNoad::new(
                NoadKind::Normal(NoadClass::Open),
                MathField::MathChar(ch),
            ))],
        );
        let multiple = crate::test_harness::publish_page_nodes(
            stores,
            [
                Node::MathNoad(MathNoad::new(
                    NoadKind::Normal(NoadClass::Ord),
                    MathField::MathChar(ch),
                )),
                Node::MathNoad(MathNoad::new(
                    NoadKind::Normal(NoadClass::Ord),
                    MathField::MathChar(ch),
                )),
            ],
        );
        for list in [scripted, non_ord, multiple] {
            assert_eq!(
                collapse_singleton_math_group(
                    &stores.command_context().expect("live generation"),
                    list,
                ),
                MathField::SubMlist(list)
            );
        }
    });
}

fn with_etex<R>(
    source: &[u8],
    test: impl for<'id> FnOnce(&mut Universe<tex_state::GenerationBrand<'id>>) -> R,
) -> R {
    crate::test_harness::with_plain_universe(|stores| {
        tex_command::install_tex82_expandable_primitives(stores);
        tex_command::install_etex_expandable_primitives(stores);
        crate::install_unexpandable_primitives(stores);
        crate::install_etex_unexpandable_primitives(stores);
        let mut control = MainControl::prepared_initex(CommandProfile::ETEX26);
        register_source(&mut control, source);
        run_to_end(&mut control, stores);
        test(stores)
    })
}

#[test]
fn end_inside_unterminated_box_reaches_outer_cleanup() {
    // TeX82 §§1064--1065/1095/1054: the stop is backed up behind an inserted
    // right brace, the recovered hbox is appended to the outer vertical list,
    // and the same stop then ejects that residual page exactly once. Use the
    // standard nonstop test host so §82 tests the recovery instead of ending
    // at an exhausted interactive terminal while asking for error advice.
    // The host-visible ShipoutComplete checkpoint is its own step between the
    // ejection and the backed-up stop; assert that boundary explicitly rather
    // than counting it as a second TeX command.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\hbox{A\end");

        let mut terminal_step = None;
        let mut artifact_counts = Vec::new();
        for step_index in 1..=16 {
            let step = control
                .step(stores)
                .expect("unterminated-box recovery executes");
            artifact_counts.push(stores.world().artifact_commits().len());
            assert!(
                artifact_counts.last() <= Some(&1),
                "end-job recovery must not repeat shipout"
            );
            if matches!(step, MainControlStep::End | MainControlStep::EndOfInput) {
                terminal_step = Some((step_index, step));
                break;
            }
        }

        assert_eq!(terminal_step, Some((7, MainControlStep::End)));
        assert_eq!(artifact_counts, [0, 0, 0, 0, 1, 1, 1]);
        assert_eq!(
            control.take_completed_boundaries(),
            [crate::EngineBoundary::ShipoutComplete]
        );
        assert_eq!(stores.world().artifact_commits().len(), 1);
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
        assert_eq!(
            admitted!(stores, |context| context.execution_group_depth()),
            0
        );
        assert_eq!(control.current_mode(), Mode::Vertical);
        assert!(control.fatal_error().is_none());
        let terminal = terminal_text(stores);
        assert_eq!(terminal.matches("! Missing } inserted.").count(), 1);
        assert!(!terminal.contains("That makes 100 errors"), "{terminal}");
    });
}

#[test]
fn parshape_and_hanging_parameters_reset_after_paragraph() {
    with_etex(
        br"\parshape=1 3pt 40pt\hangindent=5pt\hangafter=2\looseness=2 x\par",
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
        },
    );
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

#[test]
fn parshape_assignment_obeys_local_and_global_grouping() {
    with_etex(br"\parshape=1 3pt 40pt{\parshape=0}\end", |local| {
        assert_eq!(
            admitted!(local, |context| context.paragraph_shape()).len(),
            1
        );
        assert_eq!(
            admitted!(local, |context| context.paragraph_shape())[0]
                .indent
                .raw(),
            3 * 65_536
        );
        with_etex(br"{\global\parshape=1 7pt 80pt}\end", |global| {
            assert_eq!(
                admitted!(global, |context| context.paragraph_shape()).len(),
                1
            );
            assert_eq!(
                admitted!(global, |context| context.paragraph_shape())[0]
                    .indent
                    .raw(),
                7 * 65_536
            );
        });
    });
}

#[test]
fn etex_parshape_enquiries_return_explicit_and_repeated_components() {
    with_etex(
        br"\parshape=2 1pt 2pt 3pt 4pt
          \edef\result{\the\parshapeindent1/\the\parshapelength1/\the\parshapedimen3/\the\parshapedimen4/\the\parshapeindent8/\the\parshapelength8/\the\parshapeindent0}\end",
    |stores| {
    assert_eq!(
        macro_character_text(stores, "result"),
        "1.0pt/2.0pt/3.0pt/4.0pt/3.0pt/4.0pt/0.0pt"
    );
    });
}

#[test]
fn etex_penalty_arrays_assign_query_restore_and_reset_interline_at_par() {
    with_etex(
        br"\clubpenalties=2 200 100 \widowpenalties=2 300 400
          \displaywidowpenalties=1 500 {\clubpenalties=1 7}
          \interlinepenalties=2 8 7
          \edef\before{\number\clubpenalties0/\the\clubpenalties1/\the\clubpenalties8/\the\widowpenalties1/\the\widowpenalties8/\the\displaywidowpenalties0/\the\displaywidowpenalties8/\the\interlinepenalties0}
          \noindent\par \edef\after{\the\interlinepenalties0}\end",
    |stores| {
    assert_eq!(
        macro_character_text(stores, "before"),
        "2/200/100/300/400/1/500/2"
    );
    assert_eq!(macro_character_text(stores, "after"), "0");
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
fn interactionmode_reads_and_assigns_globally() {
    with_etex(
        br"\edef\before{\the\interactionmode}\begingroup\interactionmode=1\endgroup\edef\after{\the\interactionmode}",
    |stores| {
    assert_eq!(macro_character_text(stores, "before"), "3");
    assert_eq!(macro_character_text(stores, "after"), "1");
    assert_eq!(
        stores.interaction_mode(),
        tex_state::InteractionMode::Nonstop
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
