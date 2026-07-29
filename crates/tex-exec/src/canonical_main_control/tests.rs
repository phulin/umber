use std::sync::Arc;

use tex_command::{
    CommandObservation, CommandObserver, InputReason, InputTransition, RegisteredSourceKind,
    SourceRegistration,
};
use tex_state::token::{Catcode, Token};

use super::*;

fn register_source(control: &mut CanonicalMainControl, bytes: &[u8]) {
    let source = control
        .command_mut()
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(bytes),
        ))
        .expect("source registers");
    control
        .command_mut()
        .open_registered_source(source)
        .expect("source opens");
}

fn run_to_end(control: &mut CanonicalMainControl, stores: &mut Universe) {
    loop {
        match control.step(stores).expect("canonical program executes") {
            MainControlStep::End | MainControlStep::EndOfInput => break,
            MainControlStep::Continue => {}
        }
    }
}

fn terminal_text(stores: &Universe) -> String {
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
fn etex_showgroups_detaches_nested_save_and_mode_diagnostics() {
    let mut stores = Universe::new();
    let _initialized = CanonicalMainControl::tex82_initex(&mut stores);
    crate::install_etex_unexpandable_primitives(&mut stores);
    let mut control = CanonicalMainControl::with_profile(tex_command::CommandProfile::ETEX26);
    register_source(
        &mut control,
        b"\\nonstopmode\n\\tracingonline=1\n\\showgroups\n\\begingroup\\showgroups\\endgroup\n\\global\\showgroups\\count0=7\n\\end",
    );

    run_to_end(&mut control, &mut stores);

    let mut modes = ModeNest::new();
    let mut boxes = ReplayBoxes::default();
    stores.enter_group_with_kind_at_line(GroupKind::AdjustedHBox, 6);
    modes.push(Mode::RestrictedHorizontal);
    boxes.active_boxes.push(ActiveReplayBox {
        target: None,
        ships_out: false,
        kind: ReplayBoxKind::HBox,
        group_kind: GroupKind::AdjustedHBox,
        packing: PackSpec::Exactly(Scaled::from_raw(20 * 65_536)),
        leader_kind: None,
        shift: None,
    });
    let diagnostic = detached_showgroups(&stores, &modes, &None, &boxes);
    crate::diagnostics::execute_canonical_showgroups(&mut stores, &diagnostic);

    stores.enter_group_with_kind_at_line(GroupKind::MathShift, 7);
    modes.push(Mode::Math);
    stores.enter_group_with_kind_at_line(GroupKind::Math, 7);
    modes.push(Mode::Math);
    let diagnostic = detached_showgroups(&stores, &modes, &None, &boxes);
    crate::diagnostics::execute_canonical_showgroups(&mut stores, &diagnostic);

    stores.enter_group_with_kind_at_line(GroupKind::Align, 8);
    stores.enter_group_with_kind_at_line(GroupKind::Align, 8);
    stores.enter_group_with_kind_at_line(GroupKind::NoAlign, 8);
    let diagnostic = detached_showgroups(&stores, &modes, &None, &boxes);
    crate::diagnostics::execute_canonical_showgroups(&mut stores, &diagnostic);

    let output = terminal_text(&stores);
    for expected in [
        "### bottom level",
        "### semi simple group (level 1) entered at line 4 (\\begingroup)",
        "### adjusted hbox group (level 1) entered at line 6 (\\hbox to20.0pt{)",
        "### math group (level 3) entered at line 7 ({)",
        "### math shift group (level 2) entered at line 7 ($)",
        "### no align group (level 6) entered at line 8 (\\noalign{)",
        "### align group (level 5) entered at line 8 (align entry)",
        "### align group (level 4) entered at line 8 (\\halign{)",
    ] {
        assert!(
            output.contains(expected),
            "missing {expected:?} in {output:?}"
        );
    }
    assert_eq!(
        stores.count(0),
        7,
        "prefix recovery consumed following input"
    );
    assert_eq!(stores.group_depth(), 6, "diagnostic mutated the save stack");
}

fn macro_tokens<'a>(stores: &'a Universe, name: &str) -> &'a [Token] {
    let meaning = stores
        .macro_meaning(stores.symbol(name).expect("macro target"))
        .expect("macro is defined");
    stores.tokens(meaning.replacement_text())
}

fn pdftex_random_control(stores: &mut Universe) -> CanonicalMainControl {
    let set_seed = stores.intern("pdfsetrandomseed");
    stores.set_meaning(
        set_seed,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfSetRandomSeed),
    );
    CanonicalMainControl::with_profile(tex_command::CommandProfile::PDFTEX14027)
}

fn pdftex_timer_control(stores: &mut Universe) -> CanonicalMainControl {
    let reset_timer = stores.intern("pdfresettimer");
    stores.set_meaning(
        reset_timer,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfResetTimer),
    );
    CanonicalMainControl::with_profile(tex_command::CommandProfile::PDFTEX14027)
}

fn pdftex_interword_control(stores: &mut Universe) -> CanonicalMainControl {
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
        let symbol = stores.intern(name);
        stores.set_meaning(symbol, Meaning::UnexpandablePrimitive(primitive));
    }
    CanonicalMainControl::with_profile(tex_command::CommandProfile::PDFTEX14027)
}

fn step_until_pdf_seed(control: &mut CanonicalMainControl, stores: &mut Universe, expected: i32) {
    for _ in 0..4 {
        control.step(stores).expect("canonical random command");
        if stores.world().pdf_random_seed() == expected {
            return;
        }
    }
    panic!("pdfTeX random seed did not become {expected}");
}

#[test]
fn pdfsetrandomseed_is_an_ungrouped_signed_job_state_replacement() {
    let mut stores = Universe::default();
    let mut control = pdftex_random_control(&mut stores);
    register_source(
        &mut control,
        br"{\pdfsetrandomseed -1 }\pdfsetrandomseed 23 ",
    );

    step_until_pdf_seed(&mut control, &mut stores, 1);
    assert_eq!(stores.world().pdf_random_seed(), 1);
    assert_eq!(stores.world_mut().pdf_uniform_deviate(10), 7);

    assert_eq!(
        control.step(&mut stores).expect("end group"),
        MainControlStep::Continue
    );
    assert_eq!(
        stores.world().pdf_random_seed(),
        1,
        "the extension state is not restored when a TeX group closes"
    );
    step_until_pdf_seed(&mut control, &mut stores, 23);
    assert_eq!(stores.world().pdf_random_seed(), 23);
}

#[test]
fn pdfsetrandomseed_uses_the_ordinary_integer_scanner_and_preserves_lookahead() {
    let mut stores = Universe::default();
    let mut control = pdftex_random_control(&mut stores);
    register_source(
        &mut control,
        br"\pdfsetrandomseed 999999999999\pdfsetrandomseed 6 ",
    );

    assert_eq!(
        control.step(&mut stores).expect("bounded seed scan"),
        MainControlStep::Continue
    );
    assert_eq!(stores.world().pdf_random_seed(), i32::MAX);

    assert_eq!(
        control
            .step(&mut stores)
            .expect("backed-up following command"),
        MainControlStep::Continue
    );
    assert_eq!(stores.world().pdf_random_seed(), 6);
}

#[test]
fn pdfsetrandomseed_rejects_assignment_prefixes_then_replays_the_command() {
    let mut stores = Universe::default();
    let global = stores.intern("global");
    stores.set_meaning(
        global,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Global),
    );
    let mut control = pdftex_random_control(&mut stores);
    register_source(&mut control, br"\global\pdfsetrandomseed 9 ");

    assert_eq!(
        control.step(&mut stores).expect("reject prefix"),
        MainControlStep::Continue
    );
    assert_eq!(stores.world().pdf_random_seed(), 0);
    assert!(
        terminal_text(&stores).contains("You can't use a prefix with"),
        "the extension is below max_non_prefixed_command"
    );

    assert_eq!(
        control.step(&mut stores).expect("replayed seed command"),
        MainControlStep::Continue
    );
    assert_eq!(stores.world().pdf_random_seed(), 9);
}

#[test]
fn pdfresettimer_is_no_operand_any_mode_ungrouped_job_state() {
    let mut stores = Universe::default();
    stores.world_mut().set_pdf_time_micros(1_250_000);
    let mut control = pdftex_timer_control(&mut stores);
    register_source(&mut control, br"{\pdfresettimer X}");

    assert_eq!(
        control.step(&mut stores).expect("begin group"),
        MainControlStep::Continue
    );
    for _ in 0..3 {
        control.step(&mut stores).expect("timer reset");
        if stores.world().pdf_elapsed_time() == 0 {
            break;
        }
    }
    assert_eq!(stores.world().pdf_elapsed_time(), 0);

    stores.world_mut().set_pdf_time_micros(2_250_000);
    run_to_end(&mut control, &mut stores);
    assert_eq!(
        stores.world().pdf_elapsed_time(),
        65_536,
        "the reset is not restored by a group, and the following token was not consumed"
    );
}

#[test]
fn pdfresettimer_rejects_assignment_prefixes_then_replays_the_command() {
    let mut stores = Universe::default();
    stores.world_mut().set_pdf_time_micros(1_250_000);
    let global = stores.intern("global");
    stores.set_meaning(
        global,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Global),
    );
    let mut control = pdftex_timer_control(&mut stores);
    register_source(&mut control, br"\global\pdfresettimer ");

    assert_eq!(
        control.step(&mut stores).expect("reject prefix"),
        MainControlStep::Continue
    );
    assert_eq!(stores.world().pdf_elapsed_time(), 81_920);
    assert!(terminal_text(&stores).contains("You can't use a prefix with"));

    assert_eq!(
        control.step(&mut stores).expect("replayed timer reset"),
        MainControlStep::Continue
    );
    assert_eq!(stores.world().pdf_elapsed_time(), 0);
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
        let mut stores = Universe::new_with_plain_catcodes();
        stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
        let mut control = pdftex_interword_control(&mut stores);
        if mode != Mode::Vertical {
            control.modes.push(mode);
        }
        register_source(
            &mut control,
            br"\pdfinterwordspaceon\pdffakespace\pdfinterwordspaceoff",
        );
        run_to_end(&mut control, &mut stores);

        let controls: Vec<_> = control
            .modes
            .current_list()
            .nodes()
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
    }

    let mut grouped_stores = Universe::new_with_plain_catcodes();
    grouped_stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
    let mut grouped = pdftex_interword_control(&mut grouped_stores);
    register_source(&mut grouped, br"{\pdffakespace}");
    run_to_end(&mut grouped, &mut grouped_stores);
    assert!(matches!(
        grouped.modes.current_list().nodes(),
        [Node::Whatsit(Whatsit::PdfAccessibility(
            tex_state::node::PdfAccessibilityControl::FakeSpace
        ))]
    ));
}

#[test]
fn pdfinterwordspace_rejects_prefixes_and_dvi_mode_before_appending() {
    let mut stores = Universe::new_with_plain_catcodes();
    stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
    let global = stores.intern("global");
    stores.set_meaning(
        global,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Global),
    );
    let mut control = pdftex_interword_control(&mut stores);
    register_source(&mut control, br"\global\pdfinterwordspaceon");

    assert_eq!(
        control.step(&mut stores).expect("prefix recovery"),
        MainControlStep::Continue
    );
    assert!(control.modes.current_list().nodes().is_empty());
    assert!(terminal_text(&stores).contains("You can't use a prefix with"));
    assert_eq!(
        control.step(&mut stores).expect("replayed extension"),
        MainControlStep::Continue
    );
    assert!(matches!(
        control.modes.current_list().nodes(),
        [Node::Whatsit(Whatsit::PdfAccessibility(
            tex_state::node::PdfAccessibilityControl::InterwordSpaceOn
        ))]
    ));

    let mut dvi_stores = Universe::new_with_plain_catcodes();
    let mut dvi_control = pdftex_interword_control(&mut dvi_stores);
    register_source(&mut dvi_control, br"\pdffakespace");
    assert!(matches!(
        dvi_control.step(&mut dvi_stores),
        Err(ExecError::PdfExtensionInDviMode("pdffakespace"))
    ));
    assert!(dvi_control.modes.current_list().nodes().is_empty());
}

#[test]
fn pdfinterwordspace_checkpoint_restore_retries_without_duplicate_effects() {
    let mut stores = Universe::new_with_plain_catcodes();
    stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
    let mut control = pdftex_interword_control(&mut stores);
    register_source(&mut control, br"\pdfinterwordspaceon\pdfinterwordspaceoff");

    assert_eq!(
        control.step(&mut stores).expect("first toggle"),
        MainControlStep::Continue
    );
    let checkpoint = control
        .capture_checkpoint(
            crate::EngineBoundary::OuterParagraphEnd,
            &mut stores,
            crate::ExecutionBudgetCounters::default(),
        )
        .expect("quiescent toggle state checkpoints");
    assert_eq!(
        control.step(&mut stores).expect("second toggle"),
        MainControlStep::Continue
    );
    control
        .restore_checkpoint(&checkpoint, &mut stores)
        .expect("toggle state restores");
    assert_eq!(
        control.step(&mut stores).expect("second toggle retries"),
        MainControlStep::Continue
    );

    let controls: Vec<_> = control
        .modes
        .current_list()
        .nodes()
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
        let mut stores = Universe::new_with_plain_catcodes();
        stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
        let mut control = pdftex_interword_control(&mut stores);
        if mode != Mode::Vertical {
            control.modes.push(mode);
        }
        register_source(&mut control, br"\pdfrunninglinkoff\pdfrunninglinkon");
        run_to_end(&mut control, &mut stores);

        let toggles = control
            .modes
            .current_list()
            .nodes()
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
    }

    let mut grouped_stores = Universe::new_with_plain_catcodes();
    grouped_stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
    let mut grouped = pdftex_interword_control(&mut grouped_stores);
    register_source(&mut grouped, br"{\pdfrunninglinkoff\pdfrunninglinkon}");
    run_to_end(&mut grouped, &mut grouped_stores);
    assert!(matches!(
        grouped.modes.current_list().nodes(),
        [
            Node::Whatsit(Whatsit::PdfRunningLink(false)),
            Node::Whatsit(Whatsit::PdfRunningLink(true))
        ]
    ));
}

#[test]
fn pdfrunninglink_rejects_prefixes_and_dvi_mode_before_appending() {
    let mut stores = Universe::new_with_plain_catcodes();
    stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
    let global = stores.intern("global");
    stores.set_meaning(
        global,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Global),
    );
    let mut control = pdftex_interword_control(&mut stores);
    register_source(&mut control, br"\global\pdfrunninglinkoff");

    assert_eq!(
        control.step(&mut stores).expect("prefix recovery"),
        MainControlStep::Continue
    );
    assert!(control.modes.current_list().nodes().is_empty());
    assert!(terminal_text(&stores).contains("You can't use a prefix with"));
    assert_eq!(
        control.step(&mut stores).expect("replayed extension"),
        MainControlStep::Continue
    );
    assert!(matches!(
        control.modes.current_list().nodes(),
        [Node::Whatsit(Whatsit::PdfRunningLink(false))]
    ));

    let mut dvi_stores = Universe::new_with_plain_catcodes();
    let mut dvi_control = pdftex_interword_control(&mut dvi_stores);
    register_source(&mut dvi_control, br"\pdfrunninglinkon");
    assert!(matches!(
        dvi_control.step(&mut dvi_stores),
        Err(ExecError::PdfExtensionInDviMode("pdfrunninglinkon"))
    ));
    assert!(dvi_control.modes.current_list().nodes().is_empty());
}

#[test]
fn pdfrunninglink_checkpoint_restore_retries_without_duplicate_whatsits() {
    let mut stores = Universe::new_with_plain_catcodes();
    stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
    let mut control = pdftex_interword_control(&mut stores);
    register_source(&mut control, br"\pdfrunninglinkoff\pdfrunninglinkon");

    assert_eq!(
        control.step(&mut stores).expect("first toggle"),
        MainControlStep::Continue
    );
    let checkpoint = control
        .capture_checkpoint(
            crate::EngineBoundary::OuterParagraphEnd,
            &mut stores,
            crate::ExecutionBudgetCounters::default(),
        )
        .expect("running-link toggle checkpoints");
    assert_eq!(
        control.step(&mut stores).expect("second toggle"),
        MainControlStep::Continue
    );
    control
        .restore_checkpoint(&checkpoint, &mut stores)
        .expect("running-link toggle restores");
    assert_eq!(
        control.step(&mut stores).expect("second toggle retries"),
        MainControlStep::Continue
    );

    let toggles = control
        .modes
        .current_list()
        .nodes()
        .iter()
        .filter_map(|node| match node {
            Node::Whatsit(Whatsit::PdfRunningLink(enabled)) => Some(*enabled),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(toggles, [false, true]);
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
        let mut stores = Universe::new_with_plain_catcodes();
        stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
        let replacement = stores.intern_token_list(
            &"fixture"
                .chars()
                .map(|ch| Token::Char {
                    ch,
                    cat: Catcode::Letter,
                })
                .collect::<Vec<_>>(),
        );
        let name = stores.intern("n");
        stores.set_macro_meaning_global(
            name,
            MacroMeaning::new(MeaningFlags::EMPTY, TokenListId::EMPTY, replacement),
        );
        let mut control = pdftex_interword_control(&mut stores);
        if mode != Mode::Vertical {
            control.modes.push(mode);
        }
        register_source(&mut control, br"{\pdfspacefont{\n-space}}X");
        run_to_end(&mut control, &mut stores);

        assert_eq!(
            stores.pdf_space_font_name(1),
            Some(b"fixture-space".as_slice()),
            "mode {mode:?}: expanded general text selects the typed global name"
        );
    }
}

#[test]
fn pdfspacefont_rejects_prefixes_and_dvi_mode_before_scanning() {
    let mut stores = Universe::new_with_plain_catcodes();
    stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
    let global = stores.intern("global");
    stores.set_meaning(
        global,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Global),
    );
    let mut control = pdftex_interword_control(&mut stores);
    register_source(&mut control, br"\global\pdfspacefont{selected}");

    assert_eq!(
        control.step(&mut stores).expect("prefix recovery"),
        MainControlStep::Continue
    );
    assert_eq!(stores.pdf_space_font_name(1), None);
    assert!(terminal_text(&stores).contains("You can't use a prefix with"));
    assert_eq!(
        control.step(&mut stores).expect("replayed extension"),
        MainControlStep::Continue
    );
    assert_eq!(stores.pdf_space_font_name(1), Some(b"selected".as_slice()));

    let mut dvi_stores = Universe::new_with_plain_catcodes();
    let mut dvi_control = pdftex_interword_control(&mut dvi_stores);
    register_source(&mut dvi_control, br"\pdfspacefont{unscanned}");
    assert!(matches!(
        dvi_control.step(&mut dvi_stores),
        Err(ExecError::PdfExtensionInDviMode("pdfspacefont"))
    ));
    assert_eq!(dvi_stores.pdf_space_font_name(1), None);
}

#[test]
fn pdfspacefont_checkpoint_restore_retries_the_global_selection_atomically() {
    let mut stores = Universe::new_with_plain_catcodes();
    stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
    let mut control = pdftex_interword_control(&mut stores);
    register_source(&mut control, br"\pdfspacefont{first}\pdfspacefont{second}");

    assert_eq!(
        control.step(&mut stores).expect("first selection"),
        MainControlStep::Continue
    );
    let checkpoint = control
        .capture_checkpoint(
            crate::EngineBoundary::OuterParagraphEnd,
            &mut stores,
            crate::ExecutionBudgetCounters::default(),
        )
        .expect("space-font state checkpoints");
    assert_eq!(
        control.step(&mut stores).expect("second selection"),
        MainControlStep::Continue
    );
    assert_eq!(stores.pdf_space_font_name(2), Some(b"second".as_slice()));

    control
        .restore_checkpoint(&checkpoint, &mut stores)
        .expect("space-font state restores");
    assert_eq!(stores.pdf_space_font_name(2), None);
    assert_eq!(
        control.step(&mut stores).expect("second selection retries"),
        MainControlStep::Continue
    );
    assert_eq!(stores.pdf_space_font_name(2), Some(b"second".as_slice()));
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
        let mut stores = Universe::new_with_plain_catcodes();
        let mut control = CanonicalMainControl::tex82_initex(&mut stores);
        register_source(&mut control, case.source);
        run_to_end(&mut control, &mut stores);
        let output = terminal_text(&stores);
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
        let symbol = if case.target == "~" {
            stores
                .active_character_symbol('~')
                .expect("active target is interned")
        } else {
            stores
                .symbol(case.target)
                .expect("named target is interned")
        };
        assert_eq!(
            stores.macro_meaning(symbol).is_some(),
            case.committed,
            "{:?}: recovered definition scope",
            case.source
        );
    }
}

#[derive(Default)]
struct ObservationRecorder(Vec<CommandObservation>);

impl CommandObserver for ObservationRecorder {
    fn committed(&mut self, observation: CommandObservation) {
        self.0.push(observation);
    }
}

#[test]
fn etex_identical_local_integer_parameter_reassignment_is_not_a_mutation() {
    // e-TeX §275: `eq_word_define` returns immediately when extended mode
    // locally assigns the value already present. The negative controls pin
    // that a changed local value and an identical global value still commit.
    let mut stores = Universe::new_with_plain_catcodes();
    tex_expand::install_expandable_primitives(&mut stores);
    tex_expand::install_etex_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    crate::install_etex_unexpandable_primitives(&mut stores);
    let mut control = CanonicalMainControl::prepared_initex(CommandProfile::ETEX26);
    register_source(
        &mut control,
        br"\endlinechar=13 \endlinechar=12 \global\endlinechar=12 \end",
    );
    let mut observations = ObservationRecorder::default();
    loop {
        match control
            .step_with_observer(&mut stores, &mut observations)
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
            CommandObservation::Mutation(record) if record.target == "parameter" => {
                Some((record.value.as_str(), record.global))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        mutations,
        [
            ("integer_parameter:48=12", false),
            ("integer_parameter:48=12", true),
        ]
    );

    let mut tex82 = Universe::new_with_plain_catcodes();
    assert!(
        !etex_redundant_local_int_parameter_assignment(&tex82, 13, 13),
        "TeX82 has no e-TeX reassignment shortcut"
    );
    tex82.set_int_param_global(IntParam::ETEX_EXTENDED_MODE, 1);
    assert!(etex_redundant_local_int_parameter_assignment(
        &tex82, 13, 13
    ));
    assert!(!etex_redundant_local_int_parameter_assignment(
        &tex82, 13, 12
    ));
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
        let mut stores = Universe::new_with_plain_catcodes();
        let mut control = CanonicalMainControl::tex82_initex(&mut stores);
        if mode != Mode::Vertical {
            control.modes.push(mode);
        }
        register_source(&mut control, br"\count0=17\count1=29");

        let mut observations = ObservationRecorder::default();
        assert_eq!(
            control
                .step_with_observer(&mut stores, &mut observations)
                .expect("mode-independent assignment dispatches"),
            MainControlStep::Continue,
            "mode {mode:?}"
        );
        assert_eq!(stores.count(0), 17, "mode {mode:?}");
        assert_eq!(stores.count(1), 0, "mode {mode:?}");
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
        assert!(observations.0.iter().any(|observation| matches!(observation, CommandObservation::Mutation(mutation) if mutation.value == "count:0=17")));

        observations.0.clear();
        assert_eq!(
            control
                .step_with_observer(&mut stores, &mut observations)
                .expect("following command remains available"),
            MainControlStep::Continue,
            "mode {mode:?}"
        );
        assert_eq!(stores.count(1), 29, "mode {mode:?}");
        assert_eq!(
            observations
                .0
                .iter()
                .filter(|observation| matches!(observation, CommandObservation::Mutation(_)))
                .count(),
            1,
            "the following command commits exactly once in mode {mode:?}"
        );
        assert!(observations.0.iter().any(|observation| matches!(observation, CommandObservation::Mutation(mutation) if mutation.value == "count:1=29")));
    }
}

#[test]
fn main_control_error_privilege_and_stop_paths_are_finite() {
    let mut internal_stores = Universe::new_with_plain_catcodes();
    let mut internal = CanonicalMainControl::tex82_initex(&mut internal_stores);
    internal.modes.push(Mode::InternalVertical);
    register_source(&mut internal, br"\end\count0=9");
    run_to_end(&mut internal, &mut internal_stores);
    assert_eq!(internal_stores.count(0), 9);
    assert_eq!(internal.current_mode(), Mode::InternalVertical);
    assert!(terminal_text(&internal_stores).contains("can't use `\\end'"));

    let mut page_stores = Universe::new_with_plain_catcodes();
    let mut page = CanonicalMainControl::tex82_initex(&mut page_stores);
    register_source(&mut page, br"\hrule\end");
    let mut observations = ObservationRecorder::default();
    for _ in 0..32 {
        if matches!(
            page.step_with_observer(&mut page_stores, &mut observations)
                .expect("page stop remains finite"),
            MainControlStep::End | MainControlStep::EndOfInput
        ) {
            break;
        }
    }
    assert_eq!(page_stores.world().artifact_commits().len(), 1);
    assert!(observations.0.iter().any(|observation| matches!(
        observation,
        CommandObservation::Effect(effect) if effect.kind == "terminate"
    )));
}

#[test]
fn openin_closein_replace_stream_state_and_apply_default_extension() {
    let mut stores = Universe::new_with_plain_catcodes();
    stores.set_interaction_mode(tex_state::InteractionMode::ErrorStop);
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    for (name, bytes) in [("first.tex", &b"one"[..]), ("second.tex", &b"two"[..])] {
        control.capabilities_mut().register_input(
            name,
            SourceRegistration::new(RegisteredSourceKind::World, Arc::<[u8]>::from(bytes)),
        );
    }
    register_source(
        &mut control,
        br"\openin3=first.tex \read3 to \first \openin3=second.tex \read3 to \second \closein3\end",
    );
    run_to_end(&mut control, &mut stores);
    assert_eq!(
        macro_tokens(&stores, "first")[0],
        Token::Char {
            ch: 'o',
            cat: Catcode::Letter,
        }
    );
    assert_eq!(
        macro_tokens(&stores, "second")[0],
        Token::Char {
            ch: 't',
            cat: Catcode::Letter,
        }
    );
    assert!(stores.input_stream_eof(tex_state::StreamSlot::new(3)));
}

#[test]
fn input_stream_recovery_reports_raw_selector_before_recovered_read_commit() {
    // TeX82 §§435/1225: `scan_four_bit_int` observes 16, reports it, and
    // substitutes zero before `read_toks` and the target definition.
    let mut stores = Universe::new_with_plain_catcodes();
    stores
        .world_mut()
        .push_memory_terminal_line("recovered")
        .expect("terminal line queues");
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(&mut control, br"\read16 to \line\end");
    let mut observations = ObservationRecorder::default();
    for _ in 0..64 {
        if matches!(
            control
                .step_with_observer(&mut stores, &mut observations)
                .expect("recovered read remains executable"),
            MainControlStep::End | MainControlStep::EndOfInput
        ) {
            break;
        }
    }

    assert_eq!(
        macro_tokens(&stores, "line")[0],
        Token::Char {
            ch: 'r',
            cat: Catcode::Letter,
        }
    );
    let terminal = terminal_text(&stores);
    assert!(terminal.contains("! Bad number (16)."));
    assert!(terminal.contains("I changed this one to zero."));
    let integer = observations
        .0
        .iter()
        .position(|event| {
            matches!(
                event,
                CommandObservation::Scanner(scanner)
                    if scanner.kind == "integer" && scanner.value == "16"
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
                    if mutation.key.as_deref() == Some("line")
            )
        })
        .expect("recovered read target is committed");
    assert!(integer < mutation);
}

#[test]
fn read_to_definition_preserves_effective_scope_and_replay() {
    // TeX82 §§1214/1225 select scope before `read_toks`, then install its
    // parameterless macro after collection. Exercise explicit prefixes and
    // both `\globaldefs` overrides through ordinary replay.
    let mut stores = Universe::new_with_plain_catcodes();
    for line in ["local", "explicit", "forced-global", "forced-local"] {
        stores
            .world_mut()
            .push_memory_terminal_line(line)
            .expect("memory terminal accepts a line");
    }
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\def\local{old}{\read-1to\local}\def\explicit{old}{\global\read-1to\explicit}\globaldefs=1\def\forcedglobal{old}{\read-1to\forcedglobal}\globaldefs=-1\gdef\forcedlocal{old}{\global\read-1to\forcedlocal}\globaldefs=0\end",
    );
    run_to_end(&mut control, &mut stores);

    assert_eq!(
        macro_tokens(&stores, "local")[0],
        Token::Char {
            ch: 'o',
            cat: Catcode::Letter,
        }
    );
    assert_eq!(
        macro_tokens(&stores, "explicit")[0],
        Token::Char {
            ch: 'e',
            cat: Catcode::Letter,
        }
    );
    assert_eq!(
        macro_tokens(&stores, "forcedglobal")[0],
        Token::Char {
            ch: 'f',
            cat: Catcode::Letter,
        }
    );
    assert_eq!(
        macro_tokens(&stores, "forcedlocal")[0],
        Token::Char {
            ch: 'o',
            cat: Catcode::Letter,
        }
    );
}

#[test]
fn read_to_mutation_precedes_afterassignment_replay_and_carries_exact_meaning() {
    // TeX82 §1225 commits `define(p,call,cur_val)` before §1211 reaches
    // §1269's `done:` and backs up the saved afterassignment token.
    let mut stores = Universe::new_with_plain_catcodes();
    stores
        .world_mut()
        .push_memory_terminal_line("alpha")
        .expect("memory terminal accepts a line");
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\def\target{old}\afterassignment\relax\global\read-1to\target\end",
    );
    let mut observations = ObservationRecorder::default();
    loop {
        if matches!(
            control
                .step_with_observer(&mut stores, &mut observations)
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
                    if record.key.as_deref() == Some("target")
                        && record.value == "macro definition"
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
        mutation.tokens.as_deref(),
        Some([
            tex_command::ObservedToken::MacroEndMatch,
            tex_command::ObservedToken::Character {
                character: 'a',
                catcode: Catcode::Letter,
            },
            ..
        ])
    ));
}

#[test]
fn message_expands_balanced_text_and_applies_terminal_line_spacing() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\def\value{expanded}\message{left {\value} right}\count0=7\end",
    );
    run_to_end(&mut control, &mut stores);
    assert_eq!(terminal_text(&stores), "left {expanded} right");
    assert_eq!(stores.count(0), 7, "message consumes its body exactly once");
}

#[test]
fn errmessage_selects_user_or_once_only_builtin_help_and_clears_flag() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\def\value{expanded}\errmessage{bad \value}\count0=8\end",
    );
    run_to_end(&mut control, &mut stores);
    let output = terminal_text(&stores);
    assert_eq!(output.matches("! bad expanded.").count(), 1, "{output}");
    assert_eq!(stores.count(0), 8, "error handling resumes main control");
}

#[test]
fn case_shift_substitutes_character_codes_preserves_commands_and_replays() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\uccode`!=`Z\lccode`?=`y\catcode126=13\uccode126=88\uppercase{\gdef\up{!\relax}}\lowercase{\gdef\down{?\relax}}\uppercase{\gdef\active{~}}\uppercase{\gdef\zero{@}}\end",
    );
    run_to_end(&mut control, &mut stores);
    assert!(matches!(
        macro_tokens(&stores, "up"),
        [
            Token::Char {
                ch: 'Z',
                cat: Catcode::Other
            },
            Token::Cs(_)
        ]
    ));
    assert!(matches!(
        macro_tokens(&stores, "down"),
        [
            Token::Char {
                ch: 'y',
                cat: Catcode::Other
            },
            Token::Cs(_)
        ]
    ));
    assert!(matches!(
        macro_tokens(&stores, "active"),
        [Token::Char {
            ch: 'X',
            cat: Catcode::Active
        }]
    ));
    assert!(matches!(
        macro_tokens(&stores, "zero"),
        [Token::Char { ch: '@', .. }]
    ));
}

#[test]
fn show_dispatch_selects_activities_box_meaning_or_value_without_mode_dependence() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\def\shown{expanded}\show\shown\count0=17\showthe\count0\setbox0=\hbox{}\showbox0\end",
    );
    run_to_end(&mut control, &mut stores);
    let output = terminal_text(&stores);
    assert!(output.contains("> \\shown=macro:->expanded."), "{output}");
    assert!(output.contains("> 17."), "{output}");
    assert!(output.contains("> \\box0="), "{output}");
}

#[test]
fn show_meaning_reads_raw_token_and_formats_each_macro_meaning_kind() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\def\macro{body}\show\undefined\show\relax\show\macro\end",
    );
    run_to_end(&mut control, &mut stores);
    let output = terminal_text(&stores);
    assert!(output.contains("> \\undefined=undefined."), "{output}");
    assert!(output.contains("> \\relax=\\relax."), "{output}");
    assert!(output.contains("> \\macro=macro:->body."), "{output}");
}

#[test]
fn showbox_scans_register_and_distinguishes_void_from_box_contents() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\showboxbreadth=10\showboxdepth=10\setbox0=\hbox{\kern1pt}\setbox255=\hbox{}\showbox0\showbox255\showbox1\end",
    );
    run_to_end(&mut control, &mut stores);
    let output = terminal_text(&stores);
    assert!(output.contains("> \\box0="), "{output}");
    assert!(output.contains("\\kern 1.0"), "{output}");
    assert!(output.contains("> \\box255="), "{output}");
    assert!(output.contains("> \\box1="), "{output}");
    assert!(output.contains("\nvoid"), "{output}");
}

#[test]
fn showthe_uses_the_toks_for_each_internal_value_family_and_releases_output() {
    let mut stores = Universe::new_with_plain_catcodes();
    let nullfont = stores.intern("nullfont");
    stores.set_font_identifier_symbol(tex_state::font::NULL_FONT, nullfont);
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\count0=17\skip0=1pt plus 2fil\toks0={abc}\showthe\count0\showthe\skip0\showthe\font\showthe\toks0\end",
    );
    run_to_end(&mut control, &mut stores);
    let output = terminal_text(&stores);
    assert!(output.contains("> 17."), "{output}");
    assert!(output.contains("> 1.0pt plus 2.0fil."), "{output}");
    assert!(output.contains("> \\nullfont."), "{output}");
    assert!(output.contains("> abc."), "{output}");
}

#[test]
fn show_completion_routes_transcript_and_adjusts_error_count_by_interaction() {
    let mut stores = Universe::new_with_plain_catcodes();
    stores.set_interaction_mode(tex_state::InteractionMode::Nonstop);
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(&mut control, br"\showthe\count0\count1=9\end");
    run_to_end(&mut control, &mut stores);
    assert!(terminal_text(&stores).contains("> 0."));
    assert_eq!(stores.count(1), 9, "show completion resumes execution");
}

#[test]
fn final_cleanup_retires_inputs_reports_open_state_and_selects_end_or_dump() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(&mut control, br"\def\stop{\end}\stop");
    let mut observations = ObservationRecorder::default();
    loop {
        if matches!(
            control
                .step_with_observer(&mut stores, &mut observations)
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
        CommandObservation::Effect(effect) if effect.kind == "terminate"
    )));
}

#[test]
fn final_cleanup_reports_nested_condition_kinds_lines_and_order_exactly() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(&mut control, b"\\iftrue\n\\ifcase0\n\\ifnum1=1\n\\end");

    run_to_end(&mut control, &mut stores);

    assert_eq!(
        terminal_text(&stores),
        "(\\end occurred when \\ifnum on line 3 was incomplete)\
\n(\\end occurred when \\ifcase on line 2 was incomplete)\
\n(\\end occurred when \\iftrue on line 1 was incomplete)"
    );
}

/// Collects every `\setlanguage` whatsit inside box register zero.
fn language_whatsits(stores: &Universe) -> Vec<(u8, u8, u8)> {
    let outer = stores.box_reg(0).expect("box 0 holds the constructed hbox");
    let Some(Node::HList(boxed)) = stores.nodes(outer).first().map(|node| node.to_owned()) else {
        panic!("box 0 holds an hlist");
    };
    stores
        .nodes(boxed.children)
        .iter()
        .filter_map(|node| match node.to_owned() {
            Node::Whatsit(tex_state::node::Whatsit::Language {
                language,
                left_hyphen_min,
                right_hyphen_min,
            }) => Some((language, left_hyphen_min, right_hyphen_min)),
            _ => None,
        })
        .collect()
}

#[test]
fn setlanguage_appends_one_normalized_language_whatsit_per_request() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    // TeX82 §1377 normalizes `cur_val` in both out-of-range directions to
    // language zero, and §1091's `norm_min` clamps each hyphen minimum into
    // `1..=63`. The repeated `7` proves §1377 appends unconditionally: only
    // §1376's `fix_language` is guarded by `l<>clang`.
    register_source(
        &mut control,
        br"\lefthyphenmin=2 \righthyphenmin=99 \setbox0=\hbox{\setlanguage7\setlanguage7\setlanguage300\setlanguage-1}\end",
    );
    run_to_end(&mut control, &mut stores);
    assert_eq!(
        language_whatsits(&stores),
        vec![(7, 2, 63), (7, 2, 63), (0, 2, 63), (0, 2, 63)]
    );
}

#[test]
fn setlanguage_outside_horizontal_mode_reports_the_illegal_case_and_scans_nothing() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    // TeX82 §1377 tests `abs(mode)<>hmode` before `new_whatsit` and before
    // `scan_int`, so the operand is never consumed: the following assignment
    // is the very next command main control sees.
    register_source(
        &mut control,
        br"\setbox0=\vbox{\setlanguage\global\count0=5}\end",
    );
    run_to_end(&mut control, &mut stores);
    assert_eq!(stores.count(0), 5);
    let text = terminal_text(&stores);
    assert!(
        text.contains("You can't use `\\setlanguage' in internal vertical mode"),
        "{text}"
    );
    let outer = stores.box_reg(0).expect("box 0 holds the constructed vbox");
    let Some(Node::VList(boxed)) = stores.nodes(outer).first().map(|node| node.to_owned()) else {
        panic!("box 0 holds a vlist");
    };
    assert!(
        !stores
            .nodes(boxed.children)
            .iter()
            .any(|node| matches!(node.to_owned(), Node::Whatsit(_))),
        "no whatsit is appended when the mode test fails"
    );
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
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    // 128+64+32+16+8+4+2+1 = 255 `\span` delimiters, so §798's `n` is exactly
    // `max_quarterword` and the guard `n>max_quarterword` does not fire.
    register_source(
        &mut control,
        &spanning_alignment_source(r"\h\g\f\e\d\c\b\a"),
    );

    run_to_end(&mut control, &mut stores);

    assert_eq!(control.fatal_error(), None);
    assert_eq!(stores.count(0), 1, "the job ran on to \\global\\count0=1");
}

#[test]
fn two_hundred_fifty_six_span_steps_succumb_to_section_798s_confusion() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    // `\i` is 2^8 = 256 `\span` delimiters, so §798's `n` is 256 and
    // `if n>max_quarterword then confusion("256 spans")` fires.
    register_source(&mut control, &spanning_alignment_source(r"\i"));

    run_to_end(&mut control, &mut stores);

    assert_eq!(
        control.fatal_error(),
        Some(FatalError::confusion("256 spans"))
    );
    // §93 `succumb` calls §81 `jump_out`, so nothing after the alignment runs.
    assert_eq!(stores.count(0), 0);
}

#[test]
fn a_succumbed_session_stays_terminal_without_delivering_another_command() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(&mut control, &spanning_alignment_source(r"\i"));

    run_to_end(&mut control, &mut stores);
    let fatal = control.fatal_error();

    for _ in 0..4 {
        assert_eq!(
            control
                .step(&mut stores)
                .expect("a terminal session reports"),
            MainControlStep::End,
        );
    }
    assert_eq!(control.fatal_error(), fatal);
    assert_eq!(stores.count(0), 0);
}

#[test]
fn succumbing_commits_a_fatal_diagnostic_observation() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(&mut control, &spanning_alignment_source(r"\i"));

    let mut observations = ObservationRecorder::default();
    loop {
        match control
            .step_with_observer(&mut stores, &mut observations)
            .expect("a fatal error is a terminal state, never an Err")
        {
            MainControlStep::End | MainControlStep::EndOfInput => break,
            MainControlStep::Continue => {}
        }
    }

    let fatal = FatalError::confusion("256 spans");
    assert_eq!(control.fatal_error(), Some(fatal));
    assert_eq!(
        observations.0.last(),
        Some(&CommandObservation::Diagnostic(fatal.record())),
    );
}

#[test]
fn setbox_scope_is_globaldefs_adjusted_before_the_box_is_scanned() {
    // TeX82 §1214's `<Adjust for the setting of \globaldefs>` runs inside
    // `prefixed_command`, so a positive `\globaldefs` makes an unprefixed
    // `\setbox` global and a negative one makes `\global\setbox` local.
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\globaldefs=1 {\setbox0=\hbox{\kern1pt}}\globaldefs=-1 {\global\setbox1=\hbox{\kern1pt}}\globaldefs=0 \end",
    );
    run_to_end(&mut control, &mut stores);

    assert!(stores.box_reg(0).is_some(), "positive globaldefs is global");
    assert!(stores.box_reg(1).is_none(), "negative globaldefs is local");
}

#[test]
fn effective_scope_is_shared_by_provisional_and_committed_meaning_mutations() {
    // TeX82 §§1211/1214 resolve the assignment scope before §1224/§1257
    // install their provisional meanings. §§277-279 then expose that same
    // resolved choice for both provisional and final definitions.
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"{\globaldefs=1\chardef\forcedchar=65\countdef\forcedregister=2}{\globaldefs=-1\global\chardef\localchar=66\global\countdef\localregister=3}\globaldefs=0\end",
    );
    let mut observations = ObservationRecorder::default();
    loop {
        if matches!(
            control
                .step_with_observer(&mut stores, &mut observations)
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
                CommandObservation::Mutation(record) if record.key.as_deref() == Some(name) => {
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
            stores.meaning(stores.symbol(name).expect("symbol was scanned")),
            Meaning::Undefined,
            "{name} survived its group"
        );
    }
    for name in ["localchar", "localregister"] {
        assert_eq!(
            stores.meaning(stores.symbol(name).expect("symbol was scanned")),
            Meaning::Undefined,
            "{name} was restored at group end"
        );
    }
}

#[test]
fn every_non_eqtb_assignment_family_fires_afterassignment_once() {
    // TeX82 §1210 includes all ten families below in prefixed_command, and
    // §1269 reaches `done` after each completed assignment. The saved token
    // must enter through ordinary §325 back_input exactly once.
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\def\mark{\global\advance\count0 by1}\afterassignment\mark\nullfont\afterassignment\mark\textfont0=\nullfont\afterassignment\mark\setbox0=\hbox{}\afterassignment\mark\prevdepth=0pt x\afterassignment\mark\spacefactor=1000\par\afterassignment\mark\prevgraf=0\afterassignment\mark\pagegoal=1pt\afterassignment\mark\deadcycles=0\afterassignment\mark\hyphenation{word}\afterassignment\mark\nonstopmode\end",
    );
    run_to_end(&mut control, &mut stores);

    assert_eq!(stores.count(0), 10);
    assert_eq!(stores.take_afterassignment(), None);
}

#[test]
fn openin_supplies_the_default_tex_extension() {
    // TeX82 §1275's `if cur_ext="" then cur_ext:=".tex"; pack_cur_name`.
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    control.capabilities_mut().register_input(
        "child.tex",
        SourceRegistration::new(RegisteredSourceKind::World, Arc::<[u8]>::from(&b"body"[..])),
    );
    register_source(&mut control, br"\openin1=child \read1 to \line\end");
    run_to_end(&mut control, &mut stores);

    let line = stores.intern("line");
    let replacement = stores
        .macro_meaning(line)
        .expect("read defined its target")
        .replacement_text();
    let text: String = stores
        .tokens(replacement)
        .iter()
        .filter_map(|token| match token {
            Token::Char { ch, .. } => Some(*ch),
            _ => None,
        })
        .collect();
    // TeX82 §240's `\endlinechar` is appended to the line, but §348's
    // ⟨Finish line, emit a space⟩ tokenizes it as `cur_cmd:=spacer;
    // cur_chr:=" "` -- the trailing token is a space, never the raw byte.
    assert_eq!(text, "body ");
}

#[test]
fn fontdimen_reports_an_unusable_parameter_number_and_leaves_the_font_alone() {
    // TeX82 §578 resolves `n<=0` to the scratch `fmem_ptr`; §579 reports it
    // and §1253 still consumes `=<dimen>`, so the next command runs.
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(&mut control, br"\fontdimen0\nullfont=1pt \count0=1\end");
    run_to_end(&mut control, &mut stores);

    assert_eq!(stores.count(0), 1);
    assert_eq!(
        stores.hyphen_positions_for_language(0, "ab", 0, 0),
        Vec::<usize>::new(),
        "§963 diagnoses the duplicate before replacing it with a2b"
    );
    let output = terminal_text(&stores);
    assert!(
        output.contains("! Font \\nullfont has only 7 fontdimen parameters."),
        "{output}"
    );
}

#[test]
fn arithmetic_overflow_reports_and_leaves_the_target_unchanged() {
    // TeX82 §1236 returns before `word_define` when `arith_error` is set.
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\count0=2000000000 \multiply\count0 by2 \count1=7 \divide\count1 by0 \count2=1\end",
    );
    run_to_end(&mut control, &mut stores);

    assert_eq!(stores.count(0), 2_000_000_000);
    assert_eq!(stores.count(1), 7);
    assert_eq!(stores.count(2), 1);
    let output = terminal_text(&stores);
    assert_eq!(
        output.matches("! Arithmetic overflow.").count(),
        2,
        "{output}"
    );
}

#[test]
fn message_spacing_follows_the_texweb_1280_offset_rule() {
    // TeX82 §1280 separates consecutive `\message` texts with one space when
    // a line is already open.
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(&mut control, br"\message{a}\message{b}\end");
    run_to_end(&mut control, &mut stores);

    assert!(
        terminal_text(&stores).contains("a b"),
        "{}",
        terminal_text(&stores)
    );
}

#[test]
fn errmessage_prefers_errhelp_over_the_builtin_help() {
    // TeX82 §1283: `if err_help<>null then use_err_help:=true`, and §90 shows
    // it on the transcript.
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\nonstopmode\errhelp={user help}\errmessage{bad}\count0=1\end",
    );
    run_to_end(&mut control, &mut stores);

    assert_eq!(stores.count(0), 1);
    let output = terminal_text(&stores);
    assert!(output.contains("! bad."), "{output}");
    assert!(output.contains("user help"), "{output}");
    assert!(!output.contains("Hercule Poirot"), "{output}");
}

#[test]
fn patterns_and_dump_are_initex_only_and_reported_in_a_production_session() {
    // TeX82 §1252 and §1335 are both `init`-guarded.
    let mut stores = Universe::new_with_plain_catcodes();
    let _initex = CanonicalMainControl::tex82_initex(&mut stores);
    let mut control = CanonicalMainControl::new();
    register_source(&mut control, br"\patterns{a1b}\count0=1\dump");
    run_to_end(&mut control, &mut stores);

    assert_eq!(stores.count(0), 1);
    let output = terminal_text(&stores);
    assert!(output.contains("! Too late for \\patterns."), "{output}");
    assert!(
        output.contains("(\\dump is performed only by INITEX)"),
        "{output}"
    );
}

#[test]
fn hyphenation_diagnostics_preserve_tex82_recovery_and_apply_order() {
    // TeX82 §§936-937 and §§961-963: scanner othercases retain the
    // partially collected word; invalid lccodes are diagnosed during apply;
    // a duplicate is diagnosed after its replacement has been installed.
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\nonstopmode
           \hyphenation{ab\relax cd ab!c-d}
           \patterns{a\relax b a!b a1b a2b}
           \count0=1\end",
    );
    run_to_end(&mut control, &mut stores);

    assert_eq!(stores.count(0), 1);
    let output = terminal_text(&stores);
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
}

#[test]
fn show_completion_prompts_in_error_stop_mode_and_honors_the_answer() {
    // TeX82 §1293's `common_ending: ...; error`, whose §83 dialog prompts
    // `?␣` and whose §86 `S` answer switches to scroll mode.
    let mut stores = Universe::new_with_plain_catcodes();
    stores
        .world_mut()
        .push_memory_terminal_line("s")
        .expect("memory terminal accepts a line");
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(&mut control, br"\showthe\count0 \count1=1\end");
    run_to_end(&mut control, &mut stores);

    assert_eq!(stores.count(1), 1);
    let output = terminal_text(&stores);
    assert!(output.contains("> 0."), "{output}");
    assert!(output.contains("? "), "{output}");
    assert_eq!(
        stores.interaction_mode(),
        tex_state::InteractionMode::Scroll
    );
}

#[test]
fn undefined_control_sequence_reports_once_and_drops_only_its_token() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(&mut control, br"\nonstopmode\missing\count0=17\end");
    run_to_end(&mut control, &mut stores);
    assert_eq!(stores.count(0), 17, "the following command remains live");
    assert_eq!(stores.world().error_channel().error_count(), 1);
    let output = terminal_text(&stores);
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
}

#[test]
fn misplaced_tab_reports_once_and_drops_only_the_delimiter() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(&mut control, br"\nonstopmode&\count0=19\end");
    run_to_end(&mut control, &mut stores);
    assert_eq!(stores.count(0), 19, "the delimiter was not backed up");
    assert_eq!(stores.world().error_channel().error_count(), 1);
    let output = terminal_text(&stores);
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
}

#[test]
fn math_group_collapses_only_one_undecorated_ord_nucleus() {
    let mut stores = Universe::new_with_plain_catcodes();
    let empty_list = stores.freeze_node_list(&[]);
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
        let list = stores.freeze_node_list(&[Node::MathNoad(MathNoad::new(
            NoadKind::Normal(NoadClass::Ord),
            nucleus.clone(),
        ))]);
        assert_eq!(collapse_singleton_math_group(&stores, list), nucleus);
    }

    let scripted = stores.freeze_node_list(&[Node::MathNoad(MathNoad {
        kind: NoadKind::Normal(NoadClass::Ord),
        nucleus: MathField::MathChar(ch),
        subscript: MathField::MathChar(ch),
        superscript: MathField::Empty,
    })]);
    let non_ord = stores.freeze_node_list(&[Node::MathNoad(MathNoad::new(
        NoadKind::Normal(NoadClass::Open),
        MathField::MathChar(ch),
    ))]);
    let multiple = stores.freeze_node_list(&[
        Node::MathNoad(MathNoad::new(
            NoadKind::Normal(NoadClass::Ord),
            MathField::MathChar(ch),
        )),
        Node::MathNoad(MathNoad::new(
            NoadKind::Normal(NoadClass::Ord),
            MathField::MathChar(ch),
        )),
    ]);
    for list in [scripted, non_ord, multiple] {
        assert_eq!(
            collapse_singleton_math_group(&stores, list),
            MathField::SubMlist(list)
        );
    }
}
