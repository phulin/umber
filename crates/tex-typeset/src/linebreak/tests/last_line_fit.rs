//! e-TeX last-line-fit numeric, trace, and pass boundaries.

use super::*;

/// e-TeX change-file sections 38.827 and 38.851--38.855: positive values
/// below 1000 scale the inherited finite-glue ratio, while 1000 and larger
/// use it unchanged. Nonpositive values retain TeX's ordinary final glue.
#[test]
fn etex_last_line_fit_numeric_boundaries_preserve_break_and_artifact_state() {
    let (universe, nodes, parameters) = last_line_fit_paragraph();
    let cases = [
        (-1, &[6, 11][..], 244, None),
        (0, &[6, 11][..], 244, None),
        (
            1,
            &[6, 11][..],
            244,
            Some(45 * Scaled::UNITY - (5 * Scaled::UNITY + 500) / 1000),
        ),
        (
            500,
            &[6, 11][..],
            244,
            Some(42 * Scaled::UNITY + Scaled::UNITY / 2),
        ),
        (1_000, &[6, 11][..], 288, Some(40 * Scaled::UNITY)),
        (1_001, &[6, 11][..], 288, Some(40 * Scaled::UNITY)),
        (i32::MAX, &[6, 11][..], 288, Some(40 * Scaled::UNITY)),
    ];

    for (last_line_fit, expected_breaks, expected_demerits, expected_width) in cases {
        let mut parameters = parameters.clone();
        parameters.last_line_fit = last_line_fit;
        let mut hook = NoHyphenation;
        let result = line_break(&universe, &nodes, parameters, &mut hook);
        assert_eq!(
            result
                .breaks
                .iter()
                .map(|br| br.position)
                .collect::<Vec<_>>(),
            expected_breaks,
            "last_line_fit={last_line_fit}"
        );
        assert_eq!(
            result.last_line_fill.map(|spec| spec.width.raw()),
            expected_width,
            "last_line_fit={last_line_fit}"
        );
        assert_eq!(
            result.demerits, expected_demerits,
            "last_line_fit={last_line_fit}"
        );
        assert_eq!(
            result
                .last_line_fill
                .map(|spec| (spec.stretch.raw(), spec.stretch_order)),
            expected_width.map(|_| (0, Order::Fill)),
            "last_line_fit={last_line_fit}"
        );
    }
}

/// e-TeX change-file section 38.846 prints the saved shortfall and glue (or
/// final adjustment) from every active node while last-line fitting is active.
#[test]
fn etex_last_line_fit_trace_retains_active_node_diagnostic_words() {
    let (universe, nodes, parameters) = last_line_fit_paragraph();

    let (_, trace) = try_line_break_without_hyphenation_traced(&universe, &nodes, &parameters);
    let active = trace
        .iter()
        .filter_map(|event| match event {
            LineBreakTrace::Active { last_line_fit, .. } => Some(*last_line_fit),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert!(!active.is_empty());
    assert!(active.iter().all(|evidence| evidence.is_some()));
    assert!(
        active
            .iter()
            .flatten()
            .any(|evidence| evidence.terminal && evidence.glue.raw() != 0),
        "{active:?}"
    );
}

/// e-TeX change-file section 38.827: the extension requires positive
/// infinite `par_fill_skip` stretch and finite `left_skip + right_skip`.
#[test]
fn etex_last_line_fit_enablement_requires_exact_infinite_fill_component() {
    let (universe, nodes, parameters) = last_line_fit_paragraph();
    let finite = GlueSpec {
        stretch: sp(Scaled::UNITY),
        stretch_order: Order::Normal,
        ..parameters.par_fill_skip
    };
    let zero = GlueSpec {
        stretch: sp(0),
        ..parameters.par_fill_skip
    };
    let negative = GlueSpec {
        stretch: sp(-Scaled::UNITY),
        ..parameters.par_fill_skip
    };
    let infinite_background = GlueSpec {
        stretch: sp(Scaled::UNITY),
        stretch_order: Order::Fil,
        ..GlueSpec::ZERO
    };
    let cases = [
        (finite, GlueSpec::ZERO, "finite par_fill_skip"),
        (zero, GlueSpec::ZERO, "zero par_fill_skip stretch"),
        (negative, GlueSpec::ZERO, "negative par_fill_skip stretch"),
        (
            parameters.par_fill_skip,
            infinite_background,
            "infinite right_skip stretch",
        ),
    ];

    for (par_fill_skip, right_skip, label) in cases {
        let mut parameters = parameters.clone();
        parameters.par_fill_skip = par_fill_skip;
        parameters.right_skip = right_skip;
        let mut hook = NoHyphenation;
        let result = line_break(&universe, &nodes, parameters, &mut hook);
        assert_eq!(result.breaks.last().map(|br| br.position), Some(11));
        assert_eq!(result.last_line_fill, None, "{label}");
    }
}

/// e-TeX change-file sections 38.851--38.855: the saved preceding-line
/// shortfall selects finite stretch or shrink, and every missing or infinite
/// component falls back to the ordinary last-line calculation.
#[test]
fn etex_last_line_fit_adjustment_and_disable_state_boundaries() {
    let (_, _, mut parameters) = last_line_fit_paragraph();
    parameters.last_line_fit = 500;
    let fit = LastLineFit::new(&parameters, Widths::zero());
    assert!(fit.enabled);

    let previous = |line_shortfall, line_glue| Candidate {
        serial: 1,
        position: 1,
        break_site: INITIAL_BREAK_SITE,
        penalty: 0,
        line: 1,
        fitness: Fitness::Decent,
        path_demerits: 0,
        passive: None,
        previous: None,
        hyphenated: false,
        line_shortfall: sp(line_shortfall),
        line_glue: sp(line_glue),
    };
    let terminal_widths = |natural, stretch, shrink, extra_fill| {
        let mut widths = Widths::from_glue(GlueSpec {
            width: sp(natural),
            stretch: sp(stretch),
            stretch_order: Order::Normal,
            shrink: sp(shrink),
            shrink_order: Order::Normal,
        });
        widths.add_assign(Widths::from_glue(parameters.par_fill_skip));
        if extra_fill != 0 {
            widths.add_assign(Widths::from_glue(GlueSpec {
                stretch: sp(extra_fill),
                stretch_order: Order::Fil,
                ..GlueSpec::ZERO
            }));
        }
        widths
    };

    assert_eq!(
        fit.badness(
            &previous(20 * Scaled::UNITY, 40 * Scaled::UNITY),
            terminal_widths(70 * Scaled::UNITY, 30 * Scaled::UNITY, 0, 0),
            sp(100 * Scaled::UNITY),
        ),
        Some((2, Fitness::Decent, sp(15 * Scaled::UNITY / 2)))
    );
    assert_eq!(
        fit.badness(
            &previous(-20 * Scaled::UNITY, 10 * Scaled::UNITY),
            terminal_widths(70 * Scaled::UNITY, 0, 20 * Scaled::UNITY, 0),
            sp(100 * Scaled::UNITY),
        ),
        Some((100, Fitness::Tight, sp(-20 * Scaled::UNITY)))
    );
    assert_eq!(
        fit.adjusted_fill(&previous(30 * Scaled::UNITY, 15 * Scaled::UNITY / 2))
            .map(|spec| (spec.width.raw(), spec.stretch.raw())),
        Some((45 * Scaled::UNITY / 2, 0))
    );
    assert_eq!(
        fit.adjusted_fill(&previous(-10 * Scaled::UNITY, -20 * Scaled::UNITY))
            .map(|spec| (spec.width.raw(), spec.stretch.raw())),
        Some((10 * Scaled::UNITY, 0))
    );

    let disabled = [
        fit.badness(
            &previous(0, 40 * Scaled::UNITY),
            terminal_widths(70 * Scaled::UNITY, 30 * Scaled::UNITY, 0, 0),
            sp(100 * Scaled::UNITY),
        ),
        fit.badness(
            &previous(20 * Scaled::UNITY, 0),
            terminal_widths(70 * Scaled::UNITY, 30 * Scaled::UNITY, 0, 0),
            sp(100 * Scaled::UNITY),
        ),
        fit.badness(
            &previous(20 * Scaled::UNITY, 40 * Scaled::UNITY),
            terminal_widths(70 * Scaled::UNITY, 0, 0, 0),
            sp(100 * Scaled::UNITY),
        ),
        fit.badness(
            &previous(-20 * Scaled::UNITY, 10 * Scaled::UNITY),
            terminal_widths(110 * Scaled::UNITY, 0, 0, 0),
            sp(100 * Scaled::UNITY),
        ),
        fit.badness(
            &previous(20 * Scaled::UNITY, 40 * Scaled::UNITY),
            terminal_widths(70 * Scaled::UNITY, 30 * Scaled::UNITY, 0, Scaled::UNITY),
            sp(100 * Scaled::UNITY),
        ),
    ];
    assert_eq!(disabled, [None; 5]);
}

/// e-TeX change-file section 38.852: the special last-line computation is
/// reached only for a short line with infinite stretch. A long terminal line
/// retains TeX's ordinary finite-shrink badness even when the preceding line
/// saved a positive stretch ratio.
#[test]
fn etex_last_line_fit_preserves_ordinary_badness_for_long_terminal_line() {
    let (_, _, mut parameters) = last_line_fit_paragraph();
    parameters.last_line_fit = 500;
    let fit = LastLineFit::new(&parameters, Widths::zero());
    let previous = Candidate {
        serial: 1,
        position: 1,
        break_site: INITIAL_BREAK_SITE,
        penalty: 0,
        line: 1,
        fitness: Fitness::Decent,
        path_demerits: 0,
        passive: None,
        previous: None,
        hyphenated: false,
        line_shortfall: sp(31 * Scaled::UNITY),
        line_glue: sp(20 * Scaled::UNITY),
    };
    let mut terminal_widths = Widths::from_glue(GlueSpec {
        width: sp(100 * Scaled::UNITY),
        stretch: sp(40 * Scaled::UNITY),
        stretch_order: Order::Normal,
        shrink: sp(8 * Scaled::UNITY),
        shrink_order: Order::Normal,
    });
    terminal_widths.add_assign(Widths::from_glue(parameters.par_fill_skip));
    let target = sp(96 * Scaled::UNITY);

    assert_eq!(fit.badness(&previous, terminal_widths, target), None);
    assert_eq!(
        line_badness(terminal_widths, target, Scaled::from_raw(0), None),
        12
    );
}

/// e-TeX change-file sections 38.851 and 38.863: a one-line paragraph has
/// no preceding finite-glue ratio to inherit, so its terminal artifact stays
/// ordinary even when the global enablement conditions hold.
#[test]
fn etex_last_line_fit_does_not_adjust_a_single_line_paragraph() {
    let universe = TestState::new();
    let par_fill_spec = GlueSpec {
        stretch: sp(Scaled::UNITY),
        stretch_order: Order::Fill,
        ..GlueSpec::ZERO
    };
    let par_fill = par_fill_spec;
    let nodes = vec![
        rule(30 * Scaled::UNITY),
        Node::Penalty(INF_PENALTY),
        Node::Glue {
            spec: par_fill,
            kind: GlueKind::ParFillSkip,
            leader: None,
        },
    ];
    let mut parameters = params(110 * Scaled::UNITY);
    parameters.last_line_fit = 500;
    parameters.par_fill_skip = par_fill_spec;
    let mut hook = NoHyphenation;

    let result = line_break(&universe, &nodes, parameters, &mut hook);

    assert_eq!(
        result
            .breaks
            .iter()
            .map(|br| br.position)
            .collect::<Vec<_>>(),
        [3]
    );
    assert_eq!(result.demerits, 100);
    assert_eq!(result.last_line_fill, None);
}

/// e-TeX change-file sections 38.827 and 38.851--38.855, together with
/// TeX.web section 828: emergency stretch is finite background stretch and
/// therefore participates in both saved preceding-line glue and final fit.
#[test]
fn etex_last_line_fit_is_applied_on_the_emergency_final_pass() {
    let (universe, nodes, mut parameters) = last_line_fit_paragraph();
    parameters.pretolerance = -1;
    parameters.tolerance = 0;
    parameters.emergency_stretch = sp(20 * Scaled::UNITY);

    let (result, trace) = line_break_hyphenated_traced(&universe, &nodes, &parameters, Vec::new());

    assert_eq!(
        trace
            .iter()
            .filter_map(|event| match event {
                LineBreakTrace::Pass(pass) => Some(*pass),
                _ => None,
            })
            .collect::<Vec<_>>(),
        [LineBreakPass::Emergency]
    );
    assert_eq!(
        result
            .breaks
            .iter()
            .map(|br| br.position)
            .collect::<Vec<_>>(),
        [6, 11]
    );
    assert!(trace.iter().any(|event| matches!(
        event,
        LineBreakTrace::Feasible {
            breakpoint: TraceBreakpoint::Paragraph,
            demerits: None,
            ..
        }
    )));
    assert_eq!(
        result
            .last_line_fill
            .map(|spec| (spec.width.raw(), spec.stretch.raw())),
        Some((41 * Scaled::UNITY + 2 * Scaled::UNITY / 3, 0))
    );
}

#[test]
fn breaks_at_legal_glue() {
    let universe = TestState::new();
    let glue = GlueSpec {
        width: sp(10),
        stretch: sp(10),
        stretch_order: Order::Normal,
        shrink: sp(5),
        shrink_order: Order::Normal,
    };
    let nodes = vec![
        Node::Kern {
            amount: sp(20),
            kind: KernKind::Explicit,
        },
        Node::Glue {
            spec: glue,
            kind: GlueKind::Normal,

            leader: None,
        },
        Node::Kern {
            amount: sp(20),
            kind: KernKind::Explicit,
        },
        Node::Glue {
            spec: glue,
            kind: GlueKind::Normal,

            leader: None,
        },
        Node::Kern {
            amount: sp(20),
            kind: KernKind::Explicit,
        },
    ];
    let mut hook = NoHyphenation;
    let result = line_break(&universe, &nodes, params(30), &mut hook);
    assert_eq!(
        result.breaks.last().map(|br| br.position),
        Some(nodes.len())
    );
}
