use std::sync::Arc;

use tex_command::{CommandObservation, CommandObserver, RegisteredSourceKind, SourceRegistration};
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

fn macro_tokens<'a>(stores: &'a Universe, name: &str) -> &'a [Token] {
    let meaning = stores
        .macro_meaning(stores.symbol(name).expect("macro target"))
        .expect("macro is defined");
    stores.tokens(meaning.replacement_text())
}

#[derive(Default)]
struct ObservationRecorder(Vec<CommandObservation>);

impl CommandObserver for ObservationRecorder {
    fn committed(&mut self, observation: CommandObservation) {
        self.0.push(observation);
    }
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
    // The trailing `\r` is TeX82 §240's `\endlinechar`, appended to the line.
    assert_eq!(text, "body\r");
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
    assert_eq!(output.matches("! Arithmetic overflow.").count(), 2, "{output}");
}
