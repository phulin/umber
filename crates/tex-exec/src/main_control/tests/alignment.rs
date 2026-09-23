//! Alignment scanning, row construction, and recovery against TeX command, node, and diagnostic observations.

use super::*;

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
                .advance_with_observer(stores, &mut observations)
                .expect("source-fed valign executes")
            {
                StepResult::Progress(MainControlStep::End | MainControlStep::EndOfInput) => break,
                StepResult::Progress(MainControlStep::Continue) => {}
                StepResult::Suspended(need) => panic!("unexpected resource suspension: {need:?}"),
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
        let first = control
            .first_recoverable_diagnostic()
            .expect("missing-number report is retained");
        assert_eq!(first.kind, "missing-number");
        assert!(first.message.starts_with("Missing number, treated as zero"));
        assert_eq!(first.interaction, tex_state::InteractionMode::Nonstop);
        assert_eq!(first.command.as_deref(), Some("other_char"));
        assert_eq!(first.command_operand, Some(i64::from(b'*')));
        assert_eq!(
            first.observed_token,
            Some(ObservedToken::Character {
                character: '*',
                catcode: Catcode::Other,
            })
        );
        assert!(first.origin.is_some(), "missing-number source is retained");
        assert!(
            first.context.is_some(),
            "missing-number context is retained"
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
                    .advance_with_observer(stores, &mut observations)
                    .expect("alignment operation executes")
                {
                    StepResult::Progress(MainControlStep::Continue) => {}
                    StepResult::Progress(MainControlStep::End | MainControlStep::EndOfInput) => {
                        panic!("input ended before the first v-template")
                    }
                    StepResult::Suspended(need) => {
                        panic!("unexpected resource suspension: {need:?}")
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
            match control
                .advance(stores)
                .expect("nested noalign math executes")
            {
                StepResult::Progress(MainControlStep::End)
                | StepResult::Progress(MainControlStep::EndOfInput) => return,
                StepResult::Progress(MainControlStep::Continue) => {}
                StepResult::Suspended(need) => panic!("unexpected resource suspension: {need:?}"),
            }
        }
        panic!("noalign regression exceeded its step bound");
    });
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
fn settled_alignment_scanner_has_one_exact_operation_destination() {
    // Alignment interception replaces the generic settled-command retry: the
    // alignment destination owns both the delivery cursor and `\edef`'s live
    // file-enquiry scanner. Retaining both destinations would let two callers
    // reuse one command-operation coordinate after the first caller commits.
    let source = br"\setbox0=\vbox{\halign{#\cr \edef\result{\pdffiledump length 2{second}}\message{[\result]}\cr}}\end";

    let preloaded_terminal = run_pdftex_file_probe_job(source, &["second"]);
    assert!(preloaded_terminal.contains("[4142]"));
}
#[test]
fn observed_alignment_resource_need_rejects_same_control_retry() {
    let source = br"\setbox0=\vbox{\halign{#\cr \input child\cr}}\end";
    let child = SourceRegistration::new(
        RegisteredSourceKind::Generated,
        Arc::<[u8]>::from(&br"X\endinput"[..]),
    );

    crate::test_harness::with_nonstop_plain_universe(|retried_stores| {
        let mut retried_control = MainControl::tex82_initex(retried_stores);
        register_source(&mut retried_control, source);
        let mut retried = ObservationRecorder::default();
        let mut suspended = false;
        for _ in 0..TEST_STEP_LIMIT {
            if matches!(
                retried_control
                    .advance_with_observer(retried_stores, &mut retried)
                    .expect("alignment advances to its resource"),
                StepResult::Suspended(ResourceNeed::Input { ref name, .. })
                    if name == "child.tex"
            ) {
                suspended = true;
                break;
            }
        }
        assert!(
            suspended,
            "alignment resource need must be reached in bounds"
        );
        let observations_at_need = retried.0.len();
        assert_eq!(
            retried_control.pending_resource_site(),
            None,
            "alignment resource suspension leaves no scanner continuation"
        );
        retried_control
            .capabilities_mut()
            .register_input("child.tex", child.clone());
        assert!(matches!(
            retried_control.advance_with_observer(retried_stores, &mut retried),
            Err(ExecError::ResourceReplayRequired)
        ));
        assert_eq!(retried.0.len(), observations_at_need);
    });
}
#[test]
fn alignment_preamble_span_expansion_rejects_same_stack_retry() {
    // Ordinary scanner calls unwind completely on a resource miss. A direct
    // MainControl caller has no retained full-checkpoint owner, so answering
    // the detached need cannot revive this execution object; the incremental
    // production path below tex-incr owns the checkpoint replay and compares
    // the preavailable-resource output.
    for source in [
        br"\setbox0=\vbox{\halign{\span\pdffiledump length 2{second}#\cr X\cr}}\end"
            .as_slice(),
        br"\setbox0=\vbox{\halign{\span\expanded{\pdffiledump length 2{second}\pdffiledump length 2{third}}#\cr X\cr}}\end"
            .as_slice(),
    ] {
        crate::test_harness::with_nonstop_plain_universe(|stores| {
            let mut control = pdftex_initex(stores);
            register_source(&mut control, source);
            let request = {
                let mut request = None;
                for _ in 0..TEST_STEP_LIMIT {
                    match control.advance_episode(stores) {
                        Ok(StepResult::Suspended(ResourceNeed::InputProbe { request: need })) => {
                            request = Some(need);
                            break;
                        }
                        Ok(StepResult::Progress(_)) => {}
                        Ok(StepResult::Suspended(need)) => {
                            panic!("unexpected alignment resource: {need:?}")
                        }
                        Err(error) => panic!("unexpected alignment preflight error: {error:?}"),
                    }
                }
                request.expect("alignment resource need must be reached in bounds")
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
            assert!(matches!(
                control.advance_episode(stores),
                Err(ExecError::ResourceReplayRequired)
            ));
        });
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

        control.capabilities_mut().register_input_probe(
            "second",
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
            "unexpected preamble abort: {aborted:?}"
        );
        assert!(
            control.pending_resource_site().is_none(),
            "aborted preamble scanner must leave no resource site after discard"
        );
    });
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
                control.advance(stores).expect("stray end-v recovers"),
                StepResult::Progress(MainControlStep::Continue)
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
                control.advance(stores).expect("following command executes"),
                StepResult::Progress(MainControlStep::Continue)
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
                    .advance(stores)
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
/// TeX82 §796/§798's spanned-column packaging, at and just past its bound.
///
/// `#&&#` is a periodic preamble, so a body entry can span arbitrarily many
/// columns. §796 sets `n:=min_quarterword`, "this represents a span count of
/// 1", and §798 then runs `repeat incr(n); q:=link(link(q)); until q=cur_align`
/// over the spanned columns, so `n` is the number of `\span` delimiters.
/// §110's `max_quarterword` is 255.
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
