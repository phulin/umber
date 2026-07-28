use std::sync::Arc;

use tex_command::{
    AlignmentCellTemplates, AlignmentRequest, CommandDeliveryBoundary, CommandObservation,
    CommandObserver, FontResource, InputReason, InputTransition, ObservedToken, RecoveryKind,
    RegisteredSourceKind, SourceRegistration, TracedTokenList,
};
use tex_state::env::banks::{DimenParam, GlueParam, IntParam, TokParam};
use tex_state::ids::TokenListId;
use tex_state::macro_store::MacroMeaning;
use tex_state::meaning::{ExpandablePrimitive, Meaning, MeaningFlags};
use tex_state::node::Node;
use tex_state::provenance::OriginRecord;
use tex_state::scaled::Scaled;
use tex_state::{EffectRecord, InputOpenState, StreamSlot, Universe};

use super::*;

fn install_input(universe: &mut Universe) {
    let input = universe.intern("input").symbol();
    universe.set_meaning(
        input,
        Meaning::ExpandablePrimitive(ExpandablePrimitive::Input),
    );
}

fn register_source(control: &mut CommandReplayControl, bytes: &[u8]) {
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

fn run_to_end(control: &mut CanonicalMainControl, universe: &mut Universe) {
    loop {
        match control.step(universe).expect("canonical program executes") {
            MainControlStep::End | MainControlStep::EndOfInput => break,
            MainControlStep::Continue => {}
        }
    }
}

/// Runs TeX82 §1054's two-delivery `\end` when the run typeset material.
///
/// `its_all_over` is true only when the current page and the contribution
/// list are both empty and the last output was not a dead cycle, so a run
/// that put anything on the page delivers the stop twice: the first delivery
/// backs it up and ejects the residual page through §994's `build_page`, and
/// the retry -- reached after the (INITEX-empty) `\output` has shipped that
/// page -- ends the job.
fn assert_end_after_ejecting_residual_page(
    control: &mut CanonicalMainControl,
    universe: &mut Universe,
) {
    assert_eq!(
        control
            .step(universe)
            .expect("end ejects the residual page first"),
        ReplayStep::Continue
    );
    assert_eq!(
        control.step(universe).expect("retried end ends the job"),
        ReplayStep::End
    );
}

/// Every terminal/log write the run has made, committed or not.
///
/// Committing a shipout drains the applied effect records, so reading only
/// `effect_records` would silently lose every diagnostic issued before the
/// first page was shipped.
fn terminal_text(universe: &Universe) -> String {
    let committed = universe
        .world()
        .memory_terminal_output()
        .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
        .unwrap_or_default();
    let pending: String = universe
        .world()
        .effect_records()
        .iter()
        .filter_map(|effect| match effect {
            EffectRecord::StreamWrite {
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

fn register_cmr10_font(control: &mut CanonicalMainControl, universe: &mut Universe) {
    const CMR10: &[u8] = include_bytes!("../../../tex-fonts/tests/fixtures/cm/cmr10.tfm");
    universe
        .world_mut()
        .set_memory_file("cmr10.tfm", CMR10.to_vec())
        .expect("font fixture installs");
    let metrics = tex_state::InputReadState::read_input_file(
        &mut universe.input_open_context(),
        std::path::Path::new("cmr10.tfm"),
    )
    .expect("font fixture reads");
    control.capabilities_mut().register_font(
        "cmr10.tfm",
        FontResource::Tfm {
            metrics,
            opentype: None,
        },
    );
}

fn register_boundary_probe_font(control: &mut CanonicalMainControl, universe: &mut Universe) {
    // A compact valid TFM with boundary character space and a visible kern
    // for both `A + boundary` and `boundary + C`. TeX82 §545's first/last lig-kern instruction
    // conventions make the two boundary directions independently visible.
    let lh = 2_u16;
    let bc = u16::from(b'A');
    let ec = u16::from(b'D');
    let char_info = [
        [1, 0, 1, 1], // A starts lig/kern program 1.
        [1, 0, 0, 0],
        [1, 0, 0, 0],
        [1, 0, 0, 0],
    ];
    let lig_kerns = [
        [255, b' ', 0, 0], // right boundary character
        [128, b' ', 128, 0],
        [128, b'C', 128, 0],
        [255, 0, 0, 2], // left boundary program starts at 2
    ];
    let nw = 2_u16;
    let nh = 1_u16;
    let nd = 1_u16;
    let ni = 1_u16;
    let lf = 6
        + lh
        + u16::try_from(char_info.len()).expect("probe character count fits u16")
        + nw
        + nh
        + nd
        + ni
        + u16::try_from(lig_kerns.len()).expect("probe lig/kern count fits u16")
        + 1;
    let mut tfm = Vec::new();
    for value in [lf, lh, bc, ec, nw, nh, nd, ni, 4, 1, 0, 0] {
        tfm.extend_from_slice(&value.to_be_bytes());
    }
    for word in [[0, 0, 0, 0], [0, 0xa0, 0, 0]]
        .into_iter()
        .chain(char_info)
        .chain([[0, 0, 0, 0], [0, 8, 0, 0]])
        .chain([[0, 0, 0, 0]])
        .chain([[0, 0, 0, 0]])
        .chain([[0, 0, 0, 0]])
        .chain(lig_kerns)
        .chain([[0, 8, 0, 0]])
    {
        tfm.extend_from_slice(&word);
    }
    universe
        .world_mut()
        .set_memory_file("boundary-probe.tfm", tfm)
        .expect("boundary font fixture installs");
    let metrics = tex_state::InputReadState::read_input_file(
        &mut universe.input_open_context(),
        std::path::Path::new("boundary-probe.tfm"),
    )
    .expect("boundary font fixture reads");
    control.capabilities_mut().register_font(
        "boundary-probe.tfm",
        FontResource::Tfm {
            metrics,
            opentype: None,
        },
    );
}

#[test]
fn canonical_character_definitions_scan_scope_and_recovery() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\chardef\local=65 \mathchardef\math=4660
           {\chardef\local=66 \global\mathchardef\math=9029}
           \globaldefs=1 {\chardef\forced=67}
           \globaldefs=-1 {\global\chardef\suppressed=68}
           \globaldefs=0 \chardef\self=69\self
           \chardef\badchar=256 \mathchardef\badmath=32768 \end",
    );

    run_to_end(&mut control, &mut universe);

    assert_eq!(
        universe.meaning(universe.symbol("local").expect("local character")),
        Meaning::CharGiven('A')
    );
    assert_eq!(
        universe.meaning(universe.symbol("math").expect("math character")),
        Meaning::MathCharGiven(9029)
    );
    assert_eq!(
        universe.meaning(universe.symbol("badchar").expect("bad character")),
        Meaning::CharGiven('\0')
    );
    assert_eq!(
        universe.meaning(universe.symbol("badmath").expect("bad math character")),
        Meaning::MathCharGiven(0)
    );
    assert_eq!(
        universe.meaning(universe.symbol("forced").expect("globaldefs character")),
        Meaning::CharGiven('C')
    );
    assert_eq!(
        universe.meaning(universe.symbol("suppressed").expect("suppressed character")),
        Meaning::Undefined
    );
    assert_eq!(
        universe.meaning(universe.symbol("self").expect("self-referential character")),
        Meaning::CharGiven('E')
    );
    let output = terminal_text(&universe);
    assert!(output.contains("Bad character code (256)"));
    assert!(output.contains("Bad mathchar (32768)"));
}

#[test]
fn canonical_character_definition_recovers_a_non_control_sequence_target() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\mathchardef A\/\end");
    run_to_end(&mut control, &mut universe);
    let output = terminal_text(&universe);
    assert!(output.contains("Missing control sequence inserted"));
    assert!(output.contains("Missing number, treated as zero"));
    assert_eq!(
        universe.meaning(universe.symbol("inaccessible").expect("recovery target")),
        Meaning::MathCharGiven(0)
    );
}

/// TeX82 §1224 reads `cur_val` only after §434's `scan_char_num` or §436's
/// `scan_fifteen_bit_int` has already recovered it, so the `define` it
/// performs -- and therefore every observation of that definition -- carries
/// the recovered zero and never the rejected operand. The `scan_int` that
/// preceded the bound still reports its own unrecovered result.
#[test]
fn out_of_range_character_definitions_observe_the_recovered_value() {
    for (source, scanner_value, mutation_value) in [
        (
            br"\chardef\badchar=256\end".as_slice(),
            "256",
            "character:0",
        ),
        (
            br"\mathchardef\badmath=32768\end".as_slice(),
            "32768",
            "integer:0",
        ),
    ] {
        let mut universe = Universe::new_with_plain_catcodes();
        let mut control = CommandReplayControl::tex82_initex(&mut universe);
        register_source(&mut control, source);
        let mut observations = ObservationRecorder::default();

        assert_eq!(
            control
                .step_with_observer(&mut universe, &mut observations)
                .expect("shorthand definition"),
            ReplayStep::Continue
        );

        assert!(
            matches!(
                observations.0.as_slice(),
                [..,
                    CommandObservation::Scanner(scanner),
                    CommandObservation::Mutation(mutation)]
                    if scanner.kind == "integer"
                        && scanner.value == scanner_value
                        && mutation.target == "meaning"
                        && mutation.value == mutation_value
            ),
            "unexpected observations for {}: {:?}",
            String::from_utf8_lossy(source),
            observations.0
        );
    }
}

#[test]
fn restricted_math_operand_diagnostics_cover_every_primitive_variant() {
    for (source, expected, count) in [
        (br"$\mathchar32768$".as_slice(), "Bad mathchar (32768)", 1),
        (
            br"$\mathaccent32768 a$".as_slice(),
            "Bad mathchar (32768)",
            1,
        ),
        (
            br"$\delimiter134217728$".as_slice(),
            "Bad delimiter code (134217728)",
            1,
        ),
        (
            br"$\radical134217728 a$".as_slice(),
            "Bad delimiter code (134217728)",
            1,
        ),
        (
            br"$\left\delimiter134217728 a\right.$".as_slice(),
            "Bad delimiter code (134217728)",
            1,
        ),
        (
            br"$\left.a\right\delimiter134217728$".as_slice(),
            "Bad delimiter code (134217728)",
            1,
        ),
        (
            br"$a\overwithdelims\delimiter134217728\delimiter134217728 b$".as_slice(),
            "Bad delimiter code (134217728)",
            2,
        ),
        (
            br"$a\atopwithdelims\delimiter134217728\delimiter134217728 b$".as_slice(),
            "Bad delimiter code (134217728)",
            2,
        ),
        (
            br"$a\abovewithdelims\delimiter134217728 \delimiter134217728 1pt b$".as_slice(),
            "Bad delimiter code (134217728)",
            2,
        ),
    ] {
        let mut universe = Universe::new_with_plain_catcodes();
        let mut control = CanonicalMainControl::tex82_initex(&mut universe);
        register_source(&mut control, source);
        run_to_end(&mut control, &mut universe);
        let text = terminal_text(&universe);
        assert_eq!(
            text.matches(expected).count(),
            count,
            "{}: {text}",
            String::from_utf8_lossy(source)
        );
        assert!(text.contains("I changed this one to zero."), "{text}");
    }
}

#[test]
fn restricted_math_family_diagnostics_recover_locally_and_globally_to_family_zero() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"{\textfont16=\nullfont}\global\scriptfont-1=\nullfont
           \scriptscriptfont16=\nullfont\end",
    );
    run_to_end(&mut control, &mut universe);

    let text = terminal_text(&universe);
    assert_eq!(text.matches("Bad number (16)").count(), 2, "{text}");
    assert_eq!(text.matches("Bad number (-1)").count(), 1, "{text}");
    for size in [
        tex_state::math::MathFontSize::Text,
        tex_state::math::MathFontSize::Script,
        tex_state::math::MathFontSize::ScriptScript,
    ] {
        assert_eq!(
            universe.math_family_font(size, 0),
            tex_state::font::NULL_FONT
        );
    }
}

#[test]
fn restricted_register_diagnostics_cover_all_six_register_families() {
    for source in [
        br"\count256=11".as_slice(),
        br"\dimen256=12pt".as_slice(),
        br"\skip256=13pt".as_slice(),
        br"\muskip256=14mu".as_slice(),
        br"\toks256={zero}".as_slice(),
        br"\setbox256=\hbox{}".as_slice(),
    ] {
        let mut universe = Universe::new_with_plain_catcodes();
        let mut control = CanonicalMainControl::tex82_initex(&mut universe);
        register_source(&mut control, source);
        run_to_end(&mut control, &mut universe);
        let text = terminal_text(&universe);
        assert_eq!(
            text.matches("Bad register code (256)").count(),
            1,
            "{}: {text}",
            String::from_utf8_lossy(source)
        );
    }
}

/// TeX82 keeps `char_given` and `char_num` interchangeable everywhere they
/// typeset: §1034's `main_loop` (`hmode+char_given`), §1090's
/// `vmode+char_given`, §1154's `mmode+char_given`, and even §1038's ligature
/// lookahead, which accepts both at the same label. `Meaning::CharGiven` had
/// no `scan_command` arm at all before umber2-johp.108, so a `\chardef`'d
/// character was silently dropped from the document while the literal
/// `\char` beside it typeset normally.
#[test]
fn canonical_chardef_character_typesets_exactly_like_char() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_cmr10_font(&mut control, &mut universe);
    register_source(
        &mut control,
        br"\font\f=cmr10 \chardef\bee=66 \setbox0=\hbox{\f \char65\bee\char67}",
    );

    run_to_end(&mut control, &mut universe);

    let stored = universe
        .box_reg(0)
        .and_then(|id| universe.nodes(id).first().map(|node| node.to_owned()))
        .expect("setbox0 stores an hbox");
    let Node::HList(stored) = stored else {
        panic!("setbox0 contains an hbox");
    };
    let chars: String = universe
        .nodes(stored.children)
        .iter()
        .filter_map(|node| match node.to_owned() {
            Node::Char { ch, .. } => Some(ch),
            _ => None,
        })
        .collect();
    assert_eq!(
        chars, "ABC",
        "the \\chardef'd character sits between its two \\char neighbours"
    );
}

/// Collects the delivery boundaries observed for each ordinary letter of the
/// box body, in stream order, as `('A', [Raw, Expanded])` pairs.
///
/// Collection starts at `\hbox` so that the letters of a preceding
/// `\font\f=cmr10` file name are not counted: those are delivered by
/// `scan_file_name` (TeX82 §526), not by `main_control` at all.
fn letter_delivery_boundaries(
    observations: &[CommandObservation],
) -> Vec<(char, Vec<CommandDeliveryBoundary>)> {
    let mut collected: Vec<(char, Vec<CommandDeliveryBoundary>)> = Vec::new();
    let mut in_box = false;
    for observation in observations {
        let CommandObservation::Command(delivery) = observation else {
            continue;
        };
        if matches!(&delivery.spelling, ObservedToken::ControlSequence(name) if name == "hbox") {
            in_box = true;
            continue;
        }
        let ObservedToken::Character {
            character,
            catcode: tex_state::token::Catcode::Letter,
        } = delivery.spelling
        else {
            continue;
        };
        if !in_box {
            continue;
        }
        match collected.last_mut() {
            Some((last, boundaries)) if *last == character => boundaries.push(delivery.boundary),
            _ => collected.push((character, vec![delivery.boundary])),
        }
    }
    collected
}

fn observe_run(source: &[u8]) -> Vec<(char, Vec<CommandDeliveryBoundary>)> {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_cmr10_font(&mut control, &mut universe);
    register_source(&mut control, source);
    let mut observations = ObservationRecorder::default();
    loop {
        match control
            .step_with_observer(&mut universe, &mut observations)
            .expect("canonical program executes")
        {
            MainControlStep::End | MainControlStep::EndOfInput => break,
            MainControlStep::Continue => {}
        }
    }
    letter_delivery_boundaries(&observations.0)
}

/// TeX82 §1030 gives `main_control` two fetch labels. `big_switch` calls
/// `get_x_token`, but §1034's inner character loop resumes at §1038's
/// `main_loop_lookahead`, whose bare `get_next` returns a
/// `letter`/`other_char`/`char_given` straight to the loop without ever
/// reaching `x_token`.
///
/// So only the character that entered the loop is delivered twice; every
/// later character of the run is delivered once. Before umber2-johp.172 every
/// character went through `get_x_token`, which produced an extra expanded
/// delivery per character from the first word of body text onward.
#[test]
fn canonical_character_run_lookahead_delivers_later_characters_raw_only() {
    assert_eq!(
        observe_run(br"\font\f=cmr10 \setbox0=\hbox{\f ABC}"),
        vec![
            (
                'A',
                vec![
                    CommandDeliveryBoundary::Raw,
                    CommandDeliveryBoundary::Expanded
                ]
            ),
            ('B', vec![CommandDeliveryBoundary::Raw]),
            ('C', vec![CommandDeliveryBoundary::Raw]),
        ],
        "only the character that entered §1034's main loop is delivered through x_token"
    );
}

/// TeX82 §1030's two fetch labels do not agree about the `end_template` that
/// closes an alignment cell's ⟨v_j⟩ template.
///
/// §380's `get_x_token` disposes of it in place -- `cur_cs:=frozen_endv;
/// cur_cmd:=endv; goto done` -- while §380's `x_token` has no such case and
/// calls §366 `expand`, whose §375 module is
/// `cur_tok:=cs_token_flag+frozen_endv; back_input`. So a cell whose last
/// item is an ordinary character, which parks main control at §1038's
/// `main_loop_lookahead`, reaches `endv` through a backup level and a second
/// raw delivery that the `big_switch` form never produces.
///
/// §342 inserts the ⟨v_j⟩ template inside `get_next` and jumps back to its own
/// `restart`, so the fetch that triggered it is still in progress and main
/// control is still parked at §1038. Umber routes that push out to the
/// executor, and must not let the round trip demote `x_token` to
/// `get_x_token` (`umber2-johp.257`).
#[test]
fn canonical_alignment_cell_ending_in_a_character_reaches_endv_through_a_backup() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_cmr10_font(&mut control, &mut universe);
    register_source(
        &mut control,
        br"\font\f=cmr10 \setbox0=\vbox{\f\halign{#\cr AB\cr}}",
    );
    let mut observations = ObservationRecorder::default();
    loop {
        match control
            .step_with_observer(&mut universe, &mut observations)
            .expect("canonical program executes")
        {
            MainControlStep::End | MainControlStep::EndOfInput => break,
            MainControlStep::Continue => {}
        }
    }
    assert_eq!(
        end_template_to_endv_shape(&observations.0),
        vec![
            "raw end_template",
            "push backup",
            "backup frozen_endv",
            "raw endv",
            "expanded endv",
        ],
        "§375 backs a frozen_endv token up for x_token's own get_next to reread"
    );
}

/// §1030's `big_switch` form of the same cell: an empty cell parks main
/// control nowhere, so `get_x_alignment_delivery` is `get_x_token` and §380's
/// in-place rewrite applies with no backup level and no raw `endv` at all.
#[test]
fn canonical_empty_alignment_cell_reaches_endv_without_a_backup() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_cmr10_font(&mut control, &mut universe);
    register_source(
        &mut control,
        br"\font\f=cmr10 \setbox0=\vbox{\f\halign{#\cr\cr}}",
    );
    let mut observations = ObservationRecorder::default();
    loop {
        match control
            .step_with_observer(&mut universe, &mut observations)
            .expect("canonical program executes")
        {
            MainControlStep::End | MainControlStep::EndOfInput => break,
            MainControlStep::Continue => {}
        }
    }
    assert_eq!(
        end_template_to_endv_shape(&observations.0),
        vec!["raw end_template", "expanded endv"],
        "get_x_token rewrites the live command instead of backing a token up"
    );
}

/// Labels the observations from the first raw `end_template` delivery through
/// the `endv` it becomes, so the two forms of §380 can be compared directly.
fn end_template_to_endv_shape(observations: &[CommandObservation]) -> Vec<&'static str> {
    let start = observations
        .iter()
        .position(|observation| {
            matches!(
                observation,
                CommandObservation::Command(delivery)
                    if delivery.command == "end_template"
                        && delivery.boundary == CommandDeliveryBoundary::Raw
            )
        })
        .expect("the cell's v-template ends with a frozen end_template");
    let mut shape = Vec::new();
    for observation in &observations[start..] {
        let label = match observation {
            CommandObservation::Command(delivery) if delivery.command == "end_template" => {
                "raw end_template"
            }
            CommandObservation::Command(delivery) if delivery.command == "endv" => {
                match delivery.boundary {
                    CommandDeliveryBoundary::Raw => "raw endv",
                    CommandDeliveryBoundary::Expanded => "expanded endv",
                }
            }
            CommandObservation::Input(input) if input.transition == InputTransition::Backup => {
                "push backup"
            }
            CommandObservation::Recovery(recovery)
                if recovery.kind == RecoveryKind::Backup
                    && recovery.tokens == [ObservedToken::FrozenEndV] =>
            {
                "backup frozen_endv"
            }
            _ => continue,
        };
        shape.push(label);
        if label == "expanded endv" {
            break;
        }
    }
    shape
}

/// §1036's `main_loop_move+2` answers a character the current font does not
/// contain with `char_warning`, frees the would-be node, and jumps to
/// `big_switch` rather than to the lookahead. §552 gives `\nullfont`
/// `font_bc=1` and `font_ec=0`, so it contains no character at all and every
/// character of a font-free run is fetched at `big_switch`.
#[test]
fn canonical_character_run_under_nullfont_never_reaches_the_lookahead() {
    let raw_and_expanded = vec![
        CommandDeliveryBoundary::Raw,
        CommandDeliveryBoundary::Expanded,
    ];
    assert_eq!(
        observe_run(br"\setbox0=\hbox{ABC}"),
        vec![
            ('A', raw_and_expanded.clone()),
            ('B', raw_and_expanded.clone()),
            ('C', raw_and_expanded),
        ],
        "no \\nullfont character is appended, so none of them parks main control at §1038"
    );
}

/// TeX82 §1210 lists `set_page_dimen` and `set_page_int` among
/// `prefixed_command`'s ordinary assignment forms; §1242 routes them to
/// `alter_page_so_far` (§1245) and `alter_integer` (§1246). Neither had a
/// `scan_command` arm before umber2-johp.106, so the assignment was a silent
/// no-op. Only a legacy-path test covered these before.
///
/// The page is deliberately frozen first (`\copy0` contributes a box to it):
/// §986's `@<Fetch the |page_so_far|@>` reads back `max_dimen` for
/// `\pagegoal` and zero for the rest while `page_contents=empty`, no matter
/// what was stored.
#[test]
fn canonical_page_dimension_and_page_integer_assignments_write_engine_state() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\topskip=0pt \setbox0=\hbox{}\copy0 \pagegoal=100pt\pagetotal=12pt\pageshrink 3pt\pagedepth=1pt\deadcycles=7\insertpenalties=4",
    );

    run_to_end(&mut control, &mut universe);

    assert_eq!(
        universe.page_dimension(tex_state::page::PageDimension::Goal),
        Scaled::from_raw(100 * Scaled::UNITY)
    );
    assert_eq!(
        universe.page_dimension(tex_state::page::PageDimension::Total),
        Scaled::from_raw(12 * Scaled::UNITY)
    );
    // `scan_optional_equals` makes the `=` optional, exactly as it is for
    // `\dimen`/`\count`; `\pageshrink 3pt` must assign just like the others.
    assert_eq!(
        universe.page_dimension(tex_state::page::PageDimension::Shrink),
        Scaled::from_raw(3 * Scaled::UNITY)
    );
    assert_eq!(
        universe.page_dimension(tex_state::page::PageDimension::Depth),
        Scaled::from_raw(Scaled::UNITY)
    );
    assert_eq!(
        universe.page_integer(tex_state::page::PageInteger::DeadCycles),
        7
    );
    assert_eq!(
        universe.page_integer(tex_state::page::PageInteger::InsertPenalties),
        4
    );
}

/// The undispatched-assignment symptom that made umber2-johp.106 a
/// silent-corruption bug rather than a missing feature: `scan_command`
/// returned `Continue` without consuming `=100pt`, so main control typeset
/// the operand as literal document text.
#[test]
fn canonical_page_scalar_assignment_consumes_its_own_operand() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_cmr10_font(&mut control, &mut universe);
    register_source(
        &mut control,
        br"\font\f=cmr10 \setbox1=\hbox{\f \pagegoal=100pt\deadcycles=7}",
    );

    run_to_end(&mut control, &mut universe);

    let stored = universe
        .box_reg(1)
        .and_then(|id| universe.nodes(id).first().map(|node| node.to_owned()))
        .expect("setbox1 stores an hbox");
    let Node::HList(stored) = stored else {
        panic!("setbox1 contains an hbox");
    };
    let children = universe.nodes(stored.children).to_vec();
    assert!(
        children.is_empty(),
        "no operand escaped into the document as literal text: {children:?}"
    );
}

/// TeX82 §1242 states outright that the `set_page_dimen`/`set_page_int`
/// definitions "are always global": `page_so_far`, `dead_cycles`, and
/// `insert_penalties` are engine variables rather than `eqtb` entries, so no
/// save-stack entry is pushed and a group boundary cannot restore them.
#[test]
fn canonical_page_scalar_assignments_ignore_grouping_entirely() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\topskip=0pt \setbox0=\hbox{}\copy0 \pagegoal=100pt\deadcycles=1{\pagegoal=5pt\deadcycles=9}",
    );

    run_to_end(&mut control, &mut universe);

    assert_eq!(
        universe.page_dimension(tex_state::page::PageDimension::Goal),
        Scaled::from_raw(5 * Scaled::UNITY),
        "the grouped \\pagegoal survives the closing brace"
    );
    assert_eq!(
        universe.page_integer(tex_state::page::PageInteger::DeadCycles),
        9,
        "the grouped \\deadcycles survives the closing brace"
    );
}

#[test]
fn canonical_pdf_navigation_scans_rules_actions_and_deferred_markers() {
    let mut universe = Universe::new_with_plain_catcodes();
    universe.set_int_param(IntParam::PDF_OUTPUT, 1);
    for (name, primitive) in [
        (
            "pdfannot",
            tex_state::meaning::UnexpandablePrimitive::PdfAnnot,
        ),
        (
            "pdfdest",
            tex_state::meaning::UnexpandablePrimitive::PdfDest,
        ),
        (
            "pdfstartlink",
            tex_state::meaning::UnexpandablePrimitive::PdfStartLink,
        ),
        (
            "pdfendlink",
            tex_state::meaning::UnexpandablePrimitive::PdfEndLink,
        ),
        (
            "pdfthread",
            tex_state::meaning::UnexpandablePrimitive::PdfThread,
        ),
        (
            "pdfstartthread",
            tex_state::meaning::UnexpandablePrimitive::PdfStartThread,
        ),
        (
            "pdfendthread",
            tex_state::meaning::UnexpandablePrimitive::PdfEndThread,
        ),
    ] {
        let symbol = universe.intern(name).symbol();
        universe.set_meaning(symbol, Meaning::UnexpandablePrimitive(primitive));
    }
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    control.modes.push(Mode::Horizontal);
    register_source(
        &mut control,
        br"\pdfannot width 2pt height 3pt { /Subtype /Text }\pdfdest name {target} fitr depth 4pt\pdfstartlink width 5pt attr { /Border [0 0 0] } goto name {target}\pdfendlink\pdfthread depth 3pt width 10pt height 4pt attr { /I << /Title (custom) >> } name {chapter}\pdfstartthread height 7pt name {running}\pdfendthread\pdfstartlink thread name {reserved}\pdfendlink",
    );
    run_to_end(&mut control, &mut universe);

    let nodes = control.modes.current_list().nodes();
    assert!(
        matches!(
            nodes[0],
            Node::Whatsit(tex_state::node::Whatsit::PdfAnnotation { .. })
        ),
        "nodes: {nodes:#?}"
    );
    assert!(matches!(
        nodes[1],
        Node::Whatsit(tex_state::node::Whatsit::PdfDestination(_))
    ));
    assert!(matches!(
        nodes[2],
        Node::Whatsit(tex_state::node::Whatsit::PdfLinkStart { .. })
    ));
    assert!(matches!(
        nodes[3],
        Node::Whatsit(tex_state::node::Whatsit::PdfLinkEnd { .. })
    ));
    let Node::Whatsit(tex_state::node::Whatsit::PdfThread(thread)) = &nodes[4] else {
        panic!("expected one-shot thread marker: {nodes:#?}");
    };
    assert_eq!(thread.dimensions.width, Some(Scaled::from_raw(10 * 65_536)));
    assert_eq!(thread.dimensions.height, Some(Scaled::from_raw(4 * 65_536)));
    assert_eq!(thread.dimensions.depth, Some(Scaled::from_raw(3 * 65_536)));
    assert!(!thread.running);
    assert_ne!(thread.attributes, TokenListId::EMPTY);
    let Node::Whatsit(tex_state::node::Whatsit::PdfThread(thread)) = &nodes[5] else {
        panic!("expected running thread marker: {nodes:#?}");
    };
    assert_eq!(thread.dimensions.width, None);
    assert_eq!(thread.dimensions.height, Some(Scaled::from_raw(7 * 65_536)));
    assert_eq!(thread.dimensions.depth, None);
    assert!(thread.running);
    assert!(matches!(
        nodes[6],
        Node::Whatsit(tex_state::node::Whatsit::PdfEndThread)
    ));
    assert!(matches!(
        nodes[7],
        Node::Whatsit(tex_state::node::Whatsit::PdfLinkStart { .. })
    ));
    assert!(matches!(
        nodes[8],
        Node::Whatsit(tex_state::node::Whatsit::PdfLinkEnd { .. })
    ));
    assert!(universe.pdf_threads().iter().any(|thread| {
        thread.identity() == &tex_state::PdfDestinationIdentity::Name(b"reserved".to_vec())
            && thread.beads().is_empty()
    }));
    assert!(
        universe.open_pdf_links().is_empty(),
        "canonical scanning defers link nesting to final traversal"
    );
}

#[test]
fn canonical_pdf_graphics_objects_and_forms_cross_only_typed_requests() {
    let mut universe = Universe::new_with_plain_catcodes();
    universe.set_int_param(IntParam::PDF_OUTPUT, 1);
    for (name, primitive) in [
        (
            "pdfliteral",
            tex_state::meaning::UnexpandablePrimitive::PdfLiteral,
        ),
        (
            "pdfsetmatrix",
            tex_state::meaning::UnexpandablePrimitive::PdfSetMatrix,
        ),
        (
            "pdfsave",
            tex_state::meaning::UnexpandablePrimitive::PdfSave,
        ),
        (
            "pdfrestore",
            tex_state::meaning::UnexpandablePrimitive::PdfRestore,
        ),
        (
            "pdfcolorstack",
            tex_state::meaning::UnexpandablePrimitive::PdfColorStack,
        ),
        (
            "pdfsavepos",
            tex_state::meaning::UnexpandablePrimitive::PdfSavePos,
        ),
        (
            "pdfobj",
            tex_state::meaning::UnexpandablePrimitive::PdfObject,
        ),
        (
            "pdfrefobj",
            tex_state::meaning::UnexpandablePrimitive::PdfReferenceObject,
        ),
        (
            "pdfxform",
            tex_state::meaning::UnexpandablePrimitive::PdfXForm,
        ),
        (
            "pdfrefxform",
            tex_state::meaning::UnexpandablePrimitive::PdfRefXForm,
        ),
        (
            "pdfinfo",
            tex_state::meaning::UnexpandablePrimitive::PdfInfo,
        ),
        (
            "pdfcatalog",
            tex_state::meaning::UnexpandablePrimitive::PdfCatalog,
        ),
    ] {
        let symbol = universe.intern(name).symbol();
        universe.set_meaning(symbol, Meaning::UnexpandablePrimitive(primitive));
    }
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    control.modes.push(Mode::Horizontal);
    register_source(
        &mut control,
        br"\pdfliteral direct{q}\pdfliteral shipout page{Q}\pdfsetmatrix{1 0 0 1}\pdfsave\pdfrestore\pdfcolorstack0 current\pdfsavepos\setbox0=\hbox{}\pdfobj{raw}\pdfrefobj1\pdfxform0\pdfrefxform2\pdfinfo{/Producer(test)}\pdfcatalog{/PageMode/UseNone}openaction thread name{catalog-thread}",
    );
    run_to_end(&mut control, &mut universe);

    let nodes = control.modes.current_list().nodes();
    assert!(matches!(
        nodes[0],
        Node::Whatsit(tex_state::node::Whatsit::PdfLiteral { .. })
    ));
    assert!(matches!(
        nodes[1],
        Node::Whatsit(tex_state::node::Whatsit::DeferredPdfLiteral { .. })
    ));
    assert!(matches!(
        nodes[2],
        Node::Whatsit(tex_state::node::Whatsit::PdfSetMatrix { .. })
    ));
    assert!(matches!(
        nodes[3],
        Node::Whatsit(tex_state::node::Whatsit::PdfSave)
    ));
    assert!(matches!(
        nodes[4],
        Node::Whatsit(tex_state::node::Whatsit::PdfRestore)
    ));
    assert!(matches!(
        nodes[5],
        Node::Whatsit(tex_state::node::Whatsit::PdfColorStack { .. })
    ));
    assert!(matches!(
        nodes[6],
        Node::Whatsit(tex_state::node::Whatsit::PdfSavePos)
    ));
    assert!(matches!(
        nodes[7],
        Node::Whatsit(tex_state::node::Whatsit::PdfReferenceObject { object: 1 })
    ));
    assert!(matches!(
        nodes[8],
        Node::Whatsit(tex_state::node::Whatsit::PdfRefXForm { object: 2, .. })
    ));
    assert_eq!(universe.pdf_raw_objects().len(), 1);
    assert!(
        !universe.pdf_raw_objects()[0].is_referenced(),
        "pdfrefobj remains a deferred shipout handoff"
    );
    assert_eq!(
        universe.pdf_form(2).expect("form exists").resource(),
        1,
        "form resource names use their independent source-order identity"
    );
    assert_eq!(
        universe
            .pdf_document_fragments(tex_state::PdfDocumentFragmentKind::Info)
            .count(),
        1
    );
    assert!(universe.pdf_catalog_open_action().is_some());
    assert!(universe.pdf_threads().iter().any(|thread| {
        thread.identity() == &tex_state::PdfDestinationIdentity::Name(b"catalog-thread".to_vec())
            && thread.beads().is_empty()
    }));
}

#[test]
fn canonical_math_replay_finalizes_fields_and_delimiter_groups_before_parent_source() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    control.modes.push(Mode::Math);
    register_source(&mut control, br"\mathop{a}^b\left( c d \right)\over e");
    run_to_end(&mut control, &mut universe);

    let content = take_finished_canonical_math_list(&mut control.modes, &mut universe)
        .expect("math material freezes");
    let nodes = universe.nodes(content);
    assert_eq!(nodes.len(), 1, "generalized fraction completes at list end");
    let tex_state::node_arena::NodeRef::FractionNoad(fraction) = nodes.first().expect("fraction")
    else {
        panic!("expected generalized fraction");
    };
    let numerator = universe.nodes(fraction.numerator);
    assert!(numerator.len() >= 2);
    let tex_state::node_arena::NodeRef::MathNoad(operator) = numerator.first().expect("operator")
    else {
        panic!("operator noad");
    };
    assert!(matches!(
        operator.kind,
        tex_state::math::NoadKind::Normal(tex_state::math::NoadClass::Op)
    ));
    assert!(!matches!(
        operator.superscript,
        tex_state::math::MathField::Empty
    ));
    let inner = numerator
        .iter()
        .find_map(|node| match node {
            tex_state::node_arena::NodeRef::MathNoad(noad)
                if matches!(
                    noad.kind,
                    tex_state::math::NoadKind::Normal(tex_state::math::NoadClass::Inner)
                ) =>
            {
                Some(noad)
            }
            _ => None,
        })
        .expect("left/right inner noad");
    let tex_state::math::MathField::SubMlist(inner_list) = inner.nucleus else {
        panic!("left/right inner noad");
    };
    assert!(universe.nodes(inner_list).iter().any(|node| matches!(node, tex_state::node_arena::NodeRef::MathNoad(noad) if matches!(noad.kind, tex_state::math::NoadKind::RightDelimiter { .. }))));
}

/// TeX82 §1154 lists exactly seven `main_control` cases that reach §1155's
/// `set_math_char`: `mmode+letter`, `mmode+other_char`, `mmode+char_given`,
/// `mmode+char_num`, `mmode+math_char_num`, `mmode+math_given`, and
/// `mmode+delim_num`. `math_given` -- a `\mathchardef` target, defined by
/// §1224 through §436's `scan_fifteen_bit_int` -- differs from
/// `math_char_num` only in that the fifteen-bit scan already happened at
/// definition time, so the use site scans nothing and hands `cur_chr`
/// straight to `set_math_char`. Both must therefore build byte-identical
/// noads, including §1155's `var_code` branch, which replaces class 7's
/// family with `cur_fam` when `fam_in_range`.
///
/// §1151's `scan_math` repeats the same seven cases for a math *field*, so an
/// unbraced `\mathinner\ldotp` must resolve its `math_given` field exactly as
/// `\mathinner\mathchar"613A` resolves its `math_char_num` one.
#[test]
fn canonical_math_given_builds_the_same_noads_as_math_char_num() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    control.modes.push(Mode::Math);
    register_source(
        &mut control,
        br#"\mathchardef\ldotp="613A \mathchardef\vari="7141 \fam=3
            \mathchar"613A \ldotp \mathchar"7141 \vari
            \mathinner\mathchar"613A \mathinner\ldotp"#,
    );
    run_to_end(&mut control, &mut universe);

    let content = take_finished_canonical_math_list(&mut control.modes, &mut universe)
        .expect("math material freezes");
    let nodes = universe.nodes(content);
    assert_eq!(nodes.len(), 6, "each math char becomes one noad");
    let noad = |index: usize| match nodes.get(index).expect("noad") {
        tex_state::node_arena::NodeRef::MathNoad(noad) => noad,
        other => panic!("expected a math noad, found {other:?}"),
    };

    // `\mathchar"613A` and the `\mathchardef`'d `\ldotp` with the same code.
    assert_eq!(noad(0).kind, noad(1).kind);
    assert!(matches!(
        noad(1).kind,
        tex_state::math::NoadKind::Normal(tex_state::math::NoadClass::Punct)
    ));
    let (tex_state::math::MathField::MathChar(plain), tex_state::math::MathField::MathChar(given)) =
        (noad(0).nucleus, noad(1).nucleus)
    else {
        panic!("both forms build a math-char nucleus");
    };
    assert_eq!((plain.family, plain.character), (1, ':'));
    assert_eq!(
        (given.family, given.character),
        (plain.family, plain.character)
    );

    // §1155's `if c>=var_code then ... fam(nucleus(p)):=cur_fam` applies to
    // `math_given` exactly as it does to `math_char_num`: class 7 becomes an
    // `ord_noad` in the current `\fam`, not family 1.
    assert_eq!(noad(2).kind, noad(3).kind);
    assert!(matches!(
        noad(3).kind,
        tex_state::math::NoadKind::Normal(tex_state::math::NoadClass::Ord)
    ));
    let (
        tex_state::math::MathField::MathChar(plain_var),
        tex_state::math::MathField::MathChar(given_var),
    ) = (noad(2).nucleus, noad(3).nucleus)
    else {
        panic!("both forms build a math-char nucleus");
    };
    assert_eq!((plain_var.family, plain_var.character), (3, 'A'));
    assert_eq!(
        (given_var.family, given_var.character),
        (plain_var.family, plain_var.character)
    );

    // §1151's `math_given: c:=cur_chr` field case resolves identically to
    // the `math_char_num` case one noad earlier. In particular, the class
    // nibble that made both codes Punct noads in the surrounding mlist does
    // not survive inside the Inner noad's scalar field.
    assert!(matches!(
        noad(5).kind,
        tex_state::math::NoadKind::Normal(tex_state::math::NoadClass::Inner)
    ));
    assert_eq!(noad(4).kind, noad(5).kind);
    let (
        tex_state::math::MathField::MathChar(numeric_field),
        tex_state::math::MathField::MathChar(given_field),
    ) = (noad(4).nucleus, noad(5).nucleus)
    else {
        panic!("both §1151 scalar forms build a math-char field");
    };
    assert_eq!((numeric_field.family, numeric_field.character), (1, ':'));
    assert_eq!(given_field, numeric_field);
}

/// TeX82 §1151 stores every unbraced scalar field as `math_type:=math_char`
/// after extracting only the family and character from `c`. The class nibble
/// therefore cannot turn the field into a nested one-noad mlist, regardless
/// of which non-Ord noad that same code would create under §1155.
#[test]
fn canonical_non_ord_mathchar_fields_discard_the_class_nibble() {
    for (outer, outer_class, field_class) in [
        (
            br"\mathop".as_slice(),
            tex_state::math::NoadClass::Op,
            1_u16,
        ),
        (br"\mathbin".as_slice(), tex_state::math::NoadClass::Bin, 2),
        (br"\mathrel".as_slice(), tex_state::math::NoadClass::Rel, 3),
        (
            br"\mathopen".as_slice(),
            tex_state::math::NoadClass::Open,
            4,
        ),
        (
            br"\mathclose".as_slice(),
            tex_state::math::NoadClass::Close,
            5,
        ),
        (
            br"\mathpunct".as_slice(),
            tex_state::math::NoadClass::Punct,
            6,
        ),
        (
            br"\mathinner".as_slice(),
            tex_state::math::NoadClass::Inner,
            7,
        ),
    ] {
        let mut universe = Universe::new_with_plain_catcodes();
        let mut control = CanonicalMainControl::tex82_initex(&mut universe);
        control.modes.push(Mode::Math);
        let mut source = br"\fam=3 ".to_vec();
        source.extend_from_slice(outer);
        source.extend_from_slice(format!("\\mathchar\"{field_class:X}13A").as_bytes());
        register_source(&mut control, &source);
        run_to_end(&mut control, &mut universe);

        let content = take_finished_canonical_math_list(&mut control.modes, &mut universe)
            .expect("math material freezes");
        let nodes = universe.nodes(content);
        assert_eq!(nodes.len(), 1, "{outer:?} builds one outer noad");
        let tex_state::node_arena::NodeRef::MathNoad(noad) = nodes.first().expect("outer noad")
        else {
            panic!("{outer:?} must build a math noad");
        };
        assert_eq!(
            noad.kind,
            tex_state::math::NoadKind::Normal(outer_class),
            "{outer:?} retains its own noad class"
        );
        let tex_state::math::MathField::MathChar(field) = noad.nucleus else {
            panic!("{outer:?} must store the unbraced field as a scalar math char");
        };
        assert_eq!(
            (field.family, field.character),
            (if field_class == 7 { 3 } else { 1 }, ':'),
            "§1151 discards field class {field_class} for {outer:?}"
        );
    }
}

/// TeX82 §1046 lists `non_math(math_given)` beside `non_math(math_char_num)`
/// among the "Math-only cases in non-math modes", so §1045 routes it to
/// §1047's `insert_dollar_sign` rather than to any list-building case.
#[test]
fn canonical_math_given_outside_math_mode_inserts_a_dollar_sign() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    control.modes.push(Mode::Horizontal);
    register_source(&mut control, br#"\mathchardef\ldotp="613A \ldotp$"#);
    run_to_end(&mut control, &mut universe);

    assert!(
        terminal_text(&universe).contains("Missing $ inserted"),
        "a math_given outside math mode takes §1047's insert_dollar_sign"
    );
}

#[test]
fn canonical_math_field_kern_lookahead_does_not_leak_past_field_boundary() {
    // TeX82's `scan_dimen` (§455) reads one token past a unit like `pt` to
    // check for an optional trailing space, backing it up when absent
    // (§448's `scan_keyword`/optional-space handling). When that lookahead
    // lands on the very last token of a bounded body -- `{\kern1pt}` as a
    // braced nucleus, say -- the body's boundary is crossed deep inside
    // `scan_dimension`, not at the driving loop's own top-level command
    // fetch, because TeX82's `get_x_token` deliberately "retains TeX82's
    // uninterrupted behavior by consuming this boundary internally"
    // (docs/tex_command_core.md §2.1). A loop that waits for its own
    // completion event therefore runs past the end: this source used to
    // consume `Z` and beyond into the kern field's sub-mlist and then hit an
    // EOF-driven `ExecError::MissingToken`. §1153's braced field is now a
    // live `math_group` closed by group depth (`execute_live_math_group`),
    // and `execute_discretionary_part` polls
    // `CommandState::replay_episode_is_active`; both decide the end from
    // state rather than from an event the lookahead never produces.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    control.modes.push(Mode::Math);
    register_source(&mut control, br"\mathord{\kern1pt}Z");
    run_to_end(&mut control, &mut universe);

    let content = take_finished_canonical_math_list(&mut control.modes, &mut universe)
        .expect("math material freezes");
    let nodes = universe.nodes(content);
    assert_eq!(
        nodes.len(),
        2,
        "the kern field and the following Z must be separate noads"
    );
    assert!(
        matches!(
            nodes.first(),
            Some(tex_state::node_arena::NodeRef::MathNoad(
                tex_state::math::MathNoad {
                    nucleus: tex_state::math::MathField::SubMlist(_),
                    ..
                }
            ))
        ),
        "the braced kern field freezes as its own sub-mlist nucleus"
    );
    assert!(
        matches!(
            nodes.get(1),
            Some(tex_state::node_arena::NodeRef::MathNoad(
                tex_state::math::MathNoad {
                    nucleus: tex_state::math::MathField::MathChar(tex_state::math::MathChar {
                        character: 'Z',
                        ..
                    }),
                    ..
                }
            ))
        ),
        "Z must land as ordinary trailing math content, not inside the kern field"
    );
}

/// TeX82 §1138 `init_math` opens with `get_token`, and the comment on that
/// very line says why: "`get_x_token` would fail on `\ifmmode`". The probe
/// runs while the mode nest is still horizontal, so expanding it there
/// evaluates `\ifmmode` against the wrong mode and takes the wrong branch.
///
/// §1138 backs the peeked token up unconditionally when it is not a paired
/// shift, so the conditional is expanded once, by the main loop, after
/// `@<Go into ordinary math mode@>` has pushed the math nest.
#[test]
fn canonical_init_math_probe_leaves_the_following_conditional_unexpanded() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    control.modes.push(Mode::RestrictedHorizontal);
    register_source(
        &mut control,
        br"$\ifmmode\global\count0=1 \else\global\count0=2 \fi$",
    );
    run_to_end(&mut control, &mut universe);

    assert_eq!(
        universe.count(0),
        1,
        "§1138's `get_token` must let \\ifmmode reach the main loop unexpanded, \
         so it evaluates inside the math nest it opened"
    );
}

/// TeX82 §1138 pairs the two shifts only under `(cur_cmd=math_shift)and(mode>0)`.
/// In restricted horizontal mode `mode<0`, so `$$` is *not* a display opener:
/// the second `$` is backed up and immediately reread as the end of an empty
/// inline formula, leaving the mode nest exactly where it started.
#[test]
fn canonical_init_math_never_pairs_shifts_in_restricted_horizontal_mode() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    control.modes.push(Mode::RestrictedHorizontal);
    let depth = control.modes.depth();
    register_source(&mut control, br"$$");
    run_to_end(&mut control, &mut universe);

    assert_eq!(
        control.current_mode(),
        Mode::RestrictedHorizontal,
        "the backed-up second `$` must close the empty formula §1138 opened"
    );
    assert_eq!(
        control.modes.depth(),
        depth,
        "a consumed second `$` would strand the math nest open"
    );
}

/// TeX82 §1137's `hmode+math_shift: init_math` and §1193's
/// `mmode+math_shift: if cur_group=math_shift_group then after_math` are
/// applied by `CanonicalMainControl::apply_host_owned_step`, which every
/// delivery entry point shares.
///
/// Observation is an instrumentation boundary, never an alternate execution
/// mode. The observed entry point once carried its own copy of the
/// host-applied step list and that copy omitted `ScannedStep::MathShift`, so
/// an observed `$` fell through to `apply_scanned_step`'s `unreachable!()`
/// and panicked while the byte-identical unobserved `$` executed
/// (umber2-johp.118).
#[test]
fn canonical_math_shift_replays_identically_with_and_without_an_observer() {
    let source = br"$a+b$";
    let mut plain_universe = Universe::new_with_plain_catcodes();
    let mut plain = CanonicalMainControl::tex82_initex(&mut plain_universe);
    plain.modes.push(Mode::RestrictedHorizontal);
    register_source(&mut plain, source);
    run_to_end(&mut plain, &mut plain_universe);

    let mut observed_universe = Universe::new_with_plain_catcodes();
    let mut observed = CanonicalMainControl::tex82_initex(&mut observed_universe);
    observed.modes.push(Mode::RestrictedHorizontal);
    register_source(&mut observed, source);
    let mut observations = ObservationRecorder::default();
    loop {
        match observed
            .step_with_observer(&mut observed_universe, &mut observations)
            .expect("observed canonical math shift executes")
        {
            MainControlStep::End | MainControlStep::EndOfInput => break,
            MainControlStep::Continue => {}
        }
    }

    assert_eq!(plain.current_mode(), crate::Mode::RestrictedHorizontal);
    assert_eq!(observed.current_mode(), plain.current_mode());
    assert_eq!(plain.modes.depth(), observed.modes.depth());
    assert!(
        !plain.modes.current_list().nodes().is_empty(),
        "the inline math list must reach the enclosing horizontal list"
    );
    assert_eq!(
        format!("{:#?}", plain.modes.current_list().nodes()),
        format!("{:#?}", observed.modes.current_list().nodes()),
        "observation must not change the material a math shift appends"
    );
    assert_eq!(
        plain_universe.world().effect_records(),
        observed_universe.world().effect_records()
    );
}

#[test]
fn canonical_math_replay_observer_does_not_change_frozen_mlist() {
    let source = br"\mathord{a}_b^c\mskip2mu\mkern3mu\over d";
    let mut plain_universe = Universe::new_with_plain_catcodes();
    let mut plain = CanonicalMainControl::tex82_initex(&mut plain_universe);
    plain.modes.push(Mode::Math);
    register_source(&mut plain, source);
    run_to_end(&mut plain, &mut plain_universe);
    let plain_list = take_finished_canonical_math_list(&mut plain.modes, &mut plain_universe)
        .expect("plain mlist freezes");

    let mut observed_universe = Universe::new_with_plain_catcodes();
    let mut observed = CanonicalMainControl::tex82_initex(&mut observed_universe);
    observed.modes.push(Mode::Math);
    register_source(&mut observed, source);
    let mut observations = ObservationRecorder::default();
    loop {
        match observed
            .step_with_observer(&mut observed_universe, &mut observations)
            .expect("observed canonical math executes")
        {
            MainControlStep::End | MainControlStep::EndOfInput => break,
            MainControlStep::Continue => {}
        }
    }
    let observed_list =
        take_finished_canonical_math_list(&mut observed.modes, &mut observed_universe)
            .expect("observed mlist freezes");
    let plain_nodes = plain_universe.nodes(plain_list);
    let observed_nodes = observed_universe.nodes(observed_list);
    assert_eq!(plain_nodes.len(), observed_nodes.len());
    assert!(matches!(
        plain_nodes.first(),
        Some(tex_state::node_arena::NodeRef::FractionNoad(_))
    ));
    assert!(matches!(
        observed_nodes.first(),
        Some(tex_state::node_arena::NodeRef::FractionNoad(_))
    ));
    assert_eq!(
        plain_universe.world().effect_records(),
        observed_universe.world().effect_records()
    );
}

/// TeX82 §1176's `sub_sup` finishes its script through §1151's `scan_math`,
/// which reads the field with `get_x_token` and, for a braced field, §1153's
/// `back_input; scan_left_brace`. That episode runs nested inside a host-applied step
/// (`docs/tex_command_core.md` §33.5), and its command processor used to be
/// constructed at a call site that never installed the operation's observer,
/// so the entire braced field was consumed with zero observations while the
/// unobserved run consumed it identically (umber2-johp.195).
#[test]
fn canonical_math_script_field_is_observed_like_every_other_episode() {
    let source = br"a^{bc}";
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    control.modes.push(Mode::Math);
    register_source(&mut control, source);
    let mut observations = ObservationRecorder::default();
    loop {
        match control
            .step_with_observer(&mut universe, &mut observations)
            .expect("observed canonical math script executes")
        {
            MainControlStep::End | MainControlStep::EndOfInput => break,
            MainControlStep::Continue => {}
        }
    }

    let delivered = observations
        .0
        .iter()
        .filter_map(|observation| match observation {
            CommandObservation::Command(record) => Some(record.command.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(
        delivered.contains(&"sup_mark"),
        "the script marker must be delivered: {delivered:?}"
    );
    for inside_the_field in ["left_brace", "letter", "right_brace"] {
        assert!(
            delivered.contains(&inside_the_field),
            "the script field's own {inside_the_field} delivery must be observed: {delivered:?}"
        );
    }
}

#[test]
fn canonical_math_family_assignment_and_fam_select_variable_mathcode_family() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_cmr10_font(&mut control, &mut universe);
    control.modes.push(Mode::Math);
    register_source(
        &mut control,
        br#"\font\f=cmr10 \textfont2\f \mathcode`a="7161 \fam2 a"#,
    );
    run_to_end(&mut control, &mut universe);
    let f = universe.intern("f");
    let font = match universe.meaning(f) {
        Meaning::Font(font) => font,
        meaning => panic!("font definition missing: {meaning:?}"),
    };
    assert_eq!(
        universe.math_family_font(tex_state::math::MathFontSize::Text, 2),
        font
    );
    let nodes = control.modes.current_list().nodes();
    let tex_state::node::Node::MathNoad(noad) = nodes.last().expect("variable-family noad") else {
        panic!("math noad");
    };
    let tex_state::math::MathField::MathChar(character) = noad.nucleus else {
        panic!("math character field");
    };
    assert_eq!(character.family, 2);
}

#[test]
fn canonical_font_definition_scans_size_and_respects_local_global_scope() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_cmr10_font(&mut control, &mut universe);
    register_source(
        &mut control,
        br"\begingroup\font\local=cmr10 at 11pt\global\font\global=cmr10 scaled 1200\endgroup\end",
    );

    run_to_end(&mut control, &mut universe);

    let local = universe.intern("local");
    let global = universe.intern("global");
    assert!(matches!(universe.meaning(local), Meaning::Undefined));
    assert!(matches!(universe.meaning(global), Meaning::Font(_)));
}

#[test]
fn canonical_missing_font_rolls_back_then_retries_once_with_registered_resource() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\font\f=cmr10\message{ok}\end");
    let mut observations = ObservationRecorder::default();

    assert!(matches!(
        control
            .advance_with_observer(&mut universe, &mut observations)
            .expect("missing font suspends"),
        CanonicalStepResult::Suspended(CanonicalResourceNeed::Font { request })
            if request.name == "cmr10"
    ));
    let f = universe.intern("f");
    assert!(matches!(universe.meaning(f), Meaning::Undefined));
    assert!(
        observations.0.is_empty(),
        "suspended command leaked observations"
    );

    register_cmr10_font(&mut control, &mut universe);
    assert!(matches!(
        control
            .advance_with_observer(&mut universe, &mut observations)
            .expect("fresh retry installs font"),
        CanonicalStepResult::Progress(ReplayStep::Continue)
    ));
    assert!(matches!(universe.meaning(f), Meaning::Font(_)));
    run_to_end(&mut control, &mut universe);
    assert_eq!(terminal_text(&universe), "ok");
}

#[test]
fn canonical_unavailable_font_recovers_to_nullfont() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    control
        .capabilities_mut()
        .register_font("missing.tfm", FontResource::Unavailable);
    register_source(&mut control, br"\font\missing=missing\end");

    run_to_end(&mut control, &mut universe);

    let missing = universe.intern("missing");
    assert_eq!(
        universe.meaning(missing),
        Meaning::Font(tex_state::font::NULL_FONT)
    );
}

#[test]
fn canonical_openin_read_and_closein_use_registered_immutable_input() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    control.capabilities_mut().register_input(
        "child.tex",
        SourceRegistration::new(
            RegisteredSourceKind::World,
            Arc::<[u8]>::from(&b"hello"[..]),
        ),
    );
    register_source(
        &mut control,
        br"\openin1=child.tex \read1 to \line \closein1\end",
    );

    run_to_end(&mut control, &mut universe);

    let line = universe.intern("line");
    let replacement = universe
        .macro_meaning(line)
        .expect("read target is defined")
        .replacement_text();
    assert_eq!(
        universe.tokens(replacement).first(),
        Some(&tex_state::token::Token::Char {
            ch: 'h',
            cat: tex_state::token::Catcode::Letter
        })
    );
    assert!(universe.world().input_stream_eof(StreamSlot::new(1)));
}

#[test]
fn canonical_read_collects_balanced_multiline_text_and_recovers_file_eof() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    control.capabilities_mut().register_input(
        "child.tex",
        SourceRegistration::new(
            RegisteredSourceKind::World,
            Arc::<[u8]>::from(&b"{one\ntwo"[..]),
        ),
    );
    register_source(&mut control, br"\openin1=child.tex \read1 to \line\end");

    run_to_end(&mut control, &mut universe);

    let line = universe.intern("line");
    let replacement = universe
        .macro_meaning(line)
        .expect("read target is defined")
        .replacement_text();
    let text: String = universe
        .tokens(replacement)
        .iter()
        .filter_map(|token| match token {
            tex_state::token::Token::Char { ch, .. } => Some(*ch),
            _ => None,
        })
        .collect();
    assert!(text.contains("one"));
    assert!(text.contains("two"));
    // tex.web §486 does not close a runaway `\read` by inventing braces. Its
    // whole recovery is `runaway; print_err("File ended within \read");
    // help1(...); align_state:=1000000; limit:=0; error`, so the stored list
    // keeps exactly the tokens the file supplied and the unmatched `{` stays
    // unmatched. Umber appended one `}` per open group until §482-§486 moved
    // into the command core (umber2-johp.253).
    assert!(!text.ends_with('}'));
}

#[test]
fn canonical_terminal_read_prompts_once_and_collects_until_balanced() {
    let mut universe = Universe::new_with_plain_catcodes();
    universe.set_interaction_mode(tex_state::InteractionMode::ErrorStop);
    universe
        .world_mut()
        .push_memory_terminal_line("{first")
        .expect("terminal line");
    universe
        .world_mut()
        .push_memory_terminal_line("second}")
        .expect("terminal line");
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\read1 to \line\end");

    run_to_end(&mut control, &mut universe);

    let output = terminal_text(&universe);
    assert_eq!(output.matches("\\line=").count(), 1, "{output:?}");
    let line = universe.intern("line");
    let replacement = universe
        .macro_meaning(line)
        .expect("read target")
        .replacement_text();
    assert!(
        universe
            .tokens(replacement)
            .iter()
            .any(|token| { matches!(token, tex_state::token::Token::Char { ch: 's', .. }) })
    );
}

#[test]
fn canonical_read_closes_partial_text_at_an_outer_token() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    control.capabilities_mut().register_input(
        "child.tex",
        SourceRegistration::new(
            RegisteredSourceKind::World,
            Arc::<[u8]>::from(&b"{x\\stop"[..]),
        ),
    );
    register_source(
        &mut control,
        br"\outer\def\stop{}\openin1=child.tex \read1 to \line\end",
    );

    run_to_end(&mut control, &mut universe);

    let line = universe.intern("line");
    let replacement = universe
        .macro_meaning(line)
        .expect("read target")
        .replacement_text();
    // §482 sets `scanner_status:=defining`, so §306's `check_outer_validity`
    // ends the collection at `\stop` and reports a runaway definition. It
    // does not balance the partial text: the `{` the file opened stays open,
    // exactly as §486 leaves a runaway `\read`'s.
    let text: String = universe
        .tokens(replacement)
        .iter()
        .filter_map(|token| match token {
            tex_state::token::Token::Char { ch, .. } => Some(*ch),
            _ => None,
        })
        .collect();
    assert!(text.starts_with("{x"), "{text:?}");
}

#[test]
fn canonical_openin_missing_resource_rolls_back_and_retries_fresh() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\openin1=child.tex\read1to\line\end");

    assert!(matches!(
        control.advance(&mut universe).expect("openin suspends"),
        CanonicalStepResult::Suspended(CanonicalResourceNeed::Input { name }) if name == "child.tex"
    ));
    assert!(universe.world().input_stream_eof(StreamSlot::new(1)));

    control.capabilities_mut().register_input(
        "child.tex",
        SourceRegistration::new(
            RegisteredSourceKind::World,
            Arc::<[u8]>::from(&b"retry"[..]),
        ),
    );
    assert!(matches!(
        control.advance(&mut universe).expect("fresh openin retry"),
        CanonicalStepResult::Progress(ReplayStep::Continue)
    ));
    run_to_end(&mut control, &mut universe);
    let line = universe.intern("line");
    assert!(universe.macro_meaning(line).is_some());
}

#[test]
fn canonical_begingroup_uses_semisimple_local_and_global_restoration() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\begingroup\count0=1\global\count1=2\endgroup\count2=3\end",
    );

    run_to_end(&mut control, &mut universe);

    assert_eq!(universe.count(0), 0);
    assert_eq!(universe.count(1), 2);
    assert_eq!(universe.count(2), 3);
}

#[test]
fn canonical_definition_recovery_keeps_target_and_parameter_tokens_command_owned() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\def A{}\def\f#2{#2}\count7=9\end");

    run_to_end(&mut control, &mut universe);

    let inaccessible = universe.intern("inaccessible");
    assert!(universe.macro_meaning(inaccessible).is_some());
    assert_eq!(universe.count(7), 9);
    let output = terminal_text(&universe);
    assert!(output.contains("Missing control sequence inserted"));
    assert!(output.contains("Illegal parameter number in definition"));
}

#[test]
fn canonical_macro_prefix_mismatch_surfaces_its_own_command_error() {
    // `\foo` requires the literal text `XY` before its (parameterless) body,
    // but the invocation supplies `AB`. `CommandError::MacroPrefixMismatch`
    // is not recovered anywhere in `tex-command`'s expansion loop (unlike
    // `ParagraphInMacroArgument`/`OuterInMacroArgument`, which are absorbed
    // in `processor/expand.rs`), so it must reach `tex-exec` as its own
    // named `ExecError::Command` variant rather than collapsing into a
    // generic `ExecError::MissingToken`.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\def\foo XY{Z}\foo AB");

    let error = loop {
        match control.step(&mut universe) {
            Ok(MainControlStep::End | MainControlStep::EndOfInput) => {
                panic!("expected the mismatched macro invocation to fail")
            }
            Ok(MainControlStep::Continue) => {}
            Err(error) => break error,
        }
    };

    assert!(
        matches!(error, ExecError::Command(CommandError::MacroPrefixMismatch)),
        "expected ExecError::Command(CommandError::MacroPrefixMismatch), got {error:?}"
    );
    assert_eq!(
        error.to_string(),
        "macro invocation does not match its definition"
    );
}

#[test]
fn canonical_grouping_reports_and_recovers_extra_closers() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"}\endgroup\begingroup}\count7=9\end");

    run_to_end(&mut control, &mut universe);

    assert_eq!(universe.count(7), 9);
    let output = terminal_text(&universe);
    assert!(output.contains("Too many }'s"));
    assert!(output.contains("Extra \\endgroup"));
    assert!(output.contains("Extra }, or forgotten \\endgroup"));
}

#[test]
fn replay_uses_typed_scanners_for_definitions_assignments_and_termination() {
    let mut universe = Universe::new_with_plain_catcodes();
    crate::install_unexpandable_primitives(&mut universe);
    let mut control = CommandReplayControl::default();
    register_source(
        &mut control,
        br"\def\id#1{#1}\count12=\id{7}\global\def\g{z}\end",
    );

    assert_eq!(
        control.step(&mut universe).expect("definition"),
        ReplayStep::Continue
    );
    assert_eq!(
        control.step(&mut universe).expect("assignment"),
        ReplayStep::Continue
    );
    assert_eq!(universe.count(12), 7);
    assert_eq!(
        control.step(&mut universe).expect("global definition"),
        ReplayStep::Continue
    );
    let id = universe.intern("id").symbol();
    let g = universe.intern("g").symbol();
    assert!(universe.macro_meaning(id).is_some());
    assert!(universe.macro_meaning(g).is_some());
    assert_eq!(control.step(&mut universe).expect("end"), ReplayStep::End);
    assert_eq!(
        control.step(&mut universe).expect("eof"),
        ReplayStep::EndOfInput
    );
}

#[test]
fn production_driver_replays_paragraph_backup_and_typed_assignment() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, b"a\\count3=9\\end");

    // TeX82 §1095 starts the paragraph only after the command processor has
    // backed up the triggering character.  The next step redelivers it.
    assert_eq!(
        control.step(&mut universe).expect("paragraph start"),
        MainControlStep::Continue
    );
    assert_eq!(control.current_mode(), crate::Mode::Horizontal);
    assert_eq!(
        control.step(&mut universe).expect("replayed character"),
        MainControlStep::Continue
    );
    assert_eq!(
        control.step(&mut universe).expect("typed count scan"),
        MainControlStep::Continue
    );
    assert_eq!(universe.count(3), 9);
}

#[test]
fn production_driver_schedules_everypar_before_replayed_first_character() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\everypar{\count7=41}a\par\end");

    assert_eq!(
        control.step(&mut universe).expect("everypar assignment"),
        MainControlStep::Continue
    );
    assert_eq!(
        control.step(&mut universe).expect("paragraph start"),
        MainControlStep::Continue
    );
    assert_eq!(control.current_mode(), crate::Mode::Horizontal);
    assert_eq!(
        control.step(&mut universe).expect("backup replay boundary"),
        MainControlStep::Continue
    );
    assert_eq!(
        control.step(&mut universe).expect("everypar replay"),
        MainControlStep::Continue
    );
    assert_eq!(
        universe.count(7),
        41,
        "everypar runs before the backup replay"
    );
    assert_eq!(
        control.step(&mut universe).expect("first character replay"),
        MainControlStep::Continue
    );
    assert_eq!(
        control.step(&mut universe).expect("paragraph end"),
        MainControlStep::Continue
    );
}

/// Counts the `\everypar` token-list pushes a canonical run commits.
fn everypar_push_count(source: &[u8]) -> usize {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, source);
    let mut observations = ObservationRecorder::default();
    loop {
        match control
            .step_with_observer(&mut universe, &mut observations)
            .expect("canonical program executes")
        {
            MainControlStep::End | MainControlStep::EndOfInput => break,
            MainControlStep::Continue => {}
        }
    }
    observations
        .0
        .iter()
        .filter(|observation| {
            matches!(
                observation,
                CommandObservation::Input(record)
                    if record.transition == InputTransition::Push
                        && record.reason == InputReason::EveryPar
            )
        })
        .count()
}

/// tex.web §1090 routes only `vmode+start_par` to §1091 `new_graf`, the sole
/// site that pushes `\everypar`. §1092 routes both `hmode+start_par` and
/// `mmode+start_par` to §1093 `indent_in_hmode`, which appends the
/// `\parindent` box without starting a paragraph.
///
/// So a paragraph opened by `\indent` and continued by a second `\indent`
/// -- plain.tex's `\item`/`\textindent` shape -- replays `\everypar` exactly
/// once, and a `\noindent` inside a live paragraph replays it not at all.
#[test]
fn indent_inside_a_live_paragraph_does_not_replay_everypar() {
    assert_eq!(
        everypar_push_count(br"\everypar{\relax}\indent a\par\end"),
        1,
        "the vertical-mode \\indent starts the paragraph"
    );
    assert_eq!(
        everypar_push_count(br"\everypar{\relax}\indent a\indent b\par\end"),
        1,
        "the second \\indent is §1093 indent_in_hmode, not §1091 new_graf"
    );
    assert_eq!(
        everypar_push_count(br"\everypar{\relax}\indent a\noindent b\par\end"),
        1,
        "\\noindent inside a paragraph contributes nothing at all"
    );
}

/// Counts the `\everycr` token-list pushes a canonical run commits.
fn everycr_push_count(source: &[u8]) -> usize {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, source);
    let mut observations = ObservationRecorder::default();
    loop {
        match control
            .step_with_observer(&mut universe, &mut observations)
            .expect("canonical program executes")
        {
            MainControlStep::End | MainControlStep::EndOfInput => break,
            MainControlStep::Continue => {}
        }
    }
    observations
        .0
        .iter()
        .filter(|observation| {
            matches!(
                observation,
                CommandObservation::Input(record)
                    if record.transition == InputTransition::Push
                        && record.reason == InputReason::EveryCr
            )
        })
        .count()
}

/// tex.web pushes `\everycr` in exactly two places, both immediately before
/// `align_peek`: §774 `init_align`, once the preamble and the entry save
/// level exist, and §799 `fin_row`, once a row's unset box is appended.
///
/// §785's `align_peek` itself never pushes it, and neither does §1133's
/// `no_align_group` case of `handle_right_brace`, so an alignment replays the
/// hook once per row boundary plus once at entry -- not once per `align_peek`
/// call, which a `\noalign` body would double.
#[test]
fn halign_replays_everycr_at_init_align_and_at_every_row_end() {
    const HOOK: &[u8] = br"\everycr{\noalign{\relax}}";
    let with_hook = |body: &[u8]| -> Vec<u8> { [HOOK, body].concat() };

    assert_eq!(
        everycr_push_count(&with_hook(br"\halign{#\cr}\end")),
        1,
        "§774 pushes \\everycr before its own align_peek, with no row at all"
    );
    assert_eq!(
        everycr_push_count(&with_hook(br"\halign{#\cr\kern1pt\cr}\end")),
        2,
        "§774's push plus one §799 fin_row"
    );
    assert_eq!(
        everycr_push_count(&with_hook(br"\halign{#\cr\kern1pt\cr\kern2pt\cr}\end")),
        3,
        "one §799 fin_row push per completed row"
    );
    assert_eq!(
        everycr_push_count(&with_hook(br"\halign{#&#\cr\kern1pt&\kern2pt\cr}\end")),
        2,
        "a `&` ends an entry, not a row, so §791 fin_col alone pushes nothing"
    );
}

/// tex.web §1131's `do_endv` inspects the input stack and pops nothing; §357's
/// `end_token_list` retires the depleted v-template the next time `get_next`
/// reaches it. §799's `fin_row` pushes `\everycr` *before* `align_peek`, so a
/// non-empty hook buries the depleted frame: `align_peek` reads `\noalign`
/// from the hook, and the frame survives the whole `\noalign` group, retiring
/// only at the `align_peek` §1133's `no_align_group` brace runs.
///
/// Retiring it at the `do_endv` call site instead put the retirement between
/// the hook's push and its first command, which is the whole divergence.
#[test]
fn a_noalign_from_everycr_is_delivered_before_the_v_template_retires() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\everycr{\noalign{\kern1pt}}\halign{#\cr\kern2pt\cr}\end",
    );
    let mut observations = ObservationRecorder::default();
    loop {
        match control
            .step_with_observer(&mut universe, &mut observations)
            .expect("canonical program executes")
        {
            MainControlStep::End | MainControlStep::EndOfInput => break,
            MainControlStep::Continue => {}
        }
    }

    let script: Vec<&'static str> = observations
        .0
        .iter()
        .filter_map(|observation| match observation {
            CommandObservation::Input(record)
                if record.reason == InputReason::EveryCr
                    && record.transition == InputTransition::Push =>
            {
                Some("push everycr")
            }
            CommandObservation::Input(record)
                if record.reason == InputReason::EveryCr
                    && record.transition == InputTransition::Retire =>
            {
                Some("retire everycr")
            }
            CommandObservation::Input(record)
                if record.reason == InputReason::AlignmentVTemplate
                    && record.transition == InputTransition::Retire =>
            {
                Some("retire v_template")
            }
            CommandObservation::Command(delivery)
                if delivery.boundary == CommandDeliveryBoundary::Raw
                    && delivery.command == "no_align" =>
            {
                Some("raw noalign")
            }
            _ => None,
        })
        .collect();

    assert_eq!(
        script
            .iter()
            .filter(|event| **event == "retire v_template")
            .count(),
        1,
        "the single cell installs and retires one v-template: {script:?}"
    );
    assert_eq!(
        &script[script.len() - 4..],
        [
            // §799 `fin_row` pushes the hook, then §785 `align_peek` reads
            // `\noalign` out of it -- with the depleted v-template still
            // buried underneath, unretired.
            "push everycr",
            "raw noalign",
            // §325's `back_input` drains the depleted hook but stops at the
            // v-template, which the `align_peek` after §1133's
            // `no_align_group` brace finally reaches.
            "retire everycr",
            "retire v_template",
        ],
        "full script: {script:?}"
    );
}

/// `\everycr` is an ordinary token parameter, so a later assignment governs
/// the next alignment and an empty value makes both guards `null` again --
/// plain.tex's `\ialign` and `\@lign` both rely on exactly that.
#[test]
fn a_cleared_everycr_stops_being_pushed_by_the_next_alignment() {
    assert_eq!(
        everycr_push_count(
            br"\everycr{\noalign{\relax}}\halign{#\cr\kern1pt\cr}\everycr{}\halign{#\cr\kern2pt\cr}\end"
        ),
        2,
        "only the first alignment sees a non-null \\everycr"
    );
    assert_eq!(
        everycr_push_count(br"{\everycr{\noalign{\relax}}}\halign{#\cr\kern1pt\cr}\end"),
        0,
        "the value was assigned locally and restored before the alignment"
    );
}

/// The pushed level is ordinary replayed input, so its `\noalign` body runs
/// between the rows exactly as a literal one would. `\noalign` opens
/// §785's `no_align_group`, so only a global assignment survives it.
#[test]
fn everycr_noalign_body_executes_at_each_row_boundary() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\everycr{\noalign{\global\advance\count7 by1 }}\halign{#\cr\kern1pt\cr\kern2pt\cr}\end",
    );
    run_to_end(&mut control, &mut universe);

    assert_eq!(
        universe.count(7),
        3,
        "§774's entry push and one §799 fin_row push per row, all replayed"
    );
}

/// An empty `\everycr` is `null`, and both §774 and §799 guard their
/// `begin_token_list` with `if every_cr<>null`.
#[test]
fn an_empty_everycr_pushes_no_token_list_at_all() {
    assert_eq!(
        everycr_push_count(br"\halign{#\cr\kern1pt\cr}\end"),
        0,
        "no \\everycr value was ever assigned"
    );
}

/// tex.web §1030 opens `main_control` with
/// `if every_job<>null then begin_token_list(every_job,every_job_text)`,
/// before `big_switch` fetches its first token. The hook therefore belongs to
/// entering main control, not to any command, and its tokens precede every
/// token the root input contributes.
#[test]
fn format_loaded_canonical_job_replays_everyjob_before_root_input() {
    let mut initex = Universe::new_with_plain_catcodes();
    let mut builder = CanonicalMainControl::tex82_initex(&mut initex);
    register_source(&mut builder, br"\everyjob{\count7=41}\end");
    run_to_end(&mut builder, &mut initex);
    let format = initex.dump_format().expect("dump format");

    let mut universe =
        Universe::from_format(tex_state::World::memory(), &format).expect("load format");
    tex_expand::register_expandable_primitives(&mut universe);
    crate::register_unexpandable_primitives(&mut universe);
    let mut control = CanonicalMainControl::with_profile(CommandProfile::TEX82);
    register_source(&mut control, br"\count8=\count7 \end");
    let mut observations = ObservationRecorder::default();
    loop {
        match control
            .step_with_observer(&mut universe, &mut observations)
            .expect("canonical program executes")
        {
            MainControlStep::End | MainControlStep::EndOfInput => break,
            MainControlStep::Continue => {}
        }
    }

    assert_eq!(
        universe.count(8),
        41,
        "\\everyjob ran before the root input read \\count7"
    );
    assert!(
        matches!(
            observations.0.first(),
            Some(CommandObservation::Input(record))
                if record.transition == InputTransition::Push
                    && record.reason == InputReason::EveryJob
        ),
        "the §1030 prologue precedes big_switch: {:?}",
        observations.0.first()
    );
}

/// §1093 appends the `\parindent`-wide null box for `\indent` in horizontal
/// mode and resets the space factor to 1000, without pushing a new nest
/// level; `\noindent` is inert there and touches neither.
#[test]
fn indent_in_horizontal_mode_appends_the_parindent_box_and_resets_space_factor() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    universe.set_dimen_param(DimenParam::PAR_INDENT, Scaled::from_raw(1_310_720));
    register_source(
        &mut control,
        br"\indent\spacefactor=1234 \noindent\count1=\spacefactor \indent\count2=\spacefactor ",
    );
    loop {
        match control
            .step(&mut universe)
            .expect("canonical program executes")
        {
            MainControlStep::End | MainControlStep::EndOfInput => break,
            MainControlStep::Continue => {}
        }
    }

    assert_eq!(control.current_mode(), crate::Mode::Horizontal);
    assert_eq!(
        universe.count(1),
        1234,
        "\\noindent in horizontal mode does nothing at all"
    );
    assert_eq!(
        universe.count(2),
        1000,
        "\\indent in horizontal mode resets the space factor"
    );
    let widths: Vec<_> = control
        .modes
        .current_list()
        .nodes()
        .iter()
        .map(|node| match node {
            Node::HList(box_node) => box_node.width.raw(),
            other => panic!("unexpected paragraph node {other:?}"),
        })
        .collect();
    assert_eq!(
        widths,
        vec![1_310_720, 1_310_720],
        "one indent box per \\indent, none for \\noindent, and no nested list"
    );
}

/// tex.web §473 keeps `scan_toks`'s delimiting braces out of the collected
/// list, and §1226 puts one enclosing pair back only for `output_routine_loc`.
#[test]
fn production_driver_encloses_only_the_output_routine_in_braces() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\everypar{\relax}\output={\relax}\everymath{}",
    );

    for context in ["everypar", "output", "everymath"] {
        assert_eq!(
            control.step(&mut universe).expect(context),
            MainControlStep::Continue
        );
    }

    let relax = universe.intern("relax").symbol();
    let every_par = universe.tok_param(TokParam::EVERY_PAR);
    assert_eq!(
        universe.tokens(every_par),
        [tex_state::token::Token::Cs(relax)],
        "the braces around an \\everypar value are scan_toks delimiters"
    );
    let output = universe.tok_param(TokParam::OUTPUT);
    assert_eq!(
        universe.tokens(output),
        [
            tex_state::token::Token::Char {
                ch: '{',
                cat: tex_state::token::Catcode::BeginGroup,
            },
            tex_state::token::Token::Cs(relax),
            tex_state::token::Token::Char {
                ch: '}',
                cat: tex_state::token::Catcode::EndGroup,
            },
        ],
        "\\output alone is re-enclosed in braces"
    );
    let every_math = universe.tok_param(TokParam::EVERY_MATH);
    assert!(universe.tokens(every_math).is_empty());
}

/// tex.web §1226 tests `link(def_ref)=null` before it encloses, so an empty
/// `\output` reverts to the default rather than storing a brace pair.
#[test]
fn production_driver_leaves_an_empty_output_routine_empty() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\output={}");

    assert_eq!(
        control.step(&mut universe).expect("empty output"),
        MainControlStep::Continue
    );

    let output = universe.tok_param(TokParam::OUTPUT);
    assert!(universe.tokens(output).is_empty());
}

#[test]
fn production_driver_applies_typed_parshape_indent_and_horizontal_nodes() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\parshape=1 2pt 20pt\noindent\char65\kern2pt\hskip3pt\hfil",
    );

    for context in ["parshape", "noindent", "char", "kern", "hskip", "hfil"] {
        assert_eq!(
            control.step(&mut universe).expect(context),
            MainControlStep::Continue
        );
    }

    assert_eq!(control.current_mode(), crate::Mode::Horizontal);
    assert_eq!(universe.paragraph_shape().len(), 1);
    assert_eq!(
        universe.paragraph_shape()[0].indent,
        Scaled::from_raw(2 * Scaled::UNITY)
    );
    assert_eq!(
        universe.paragraph_shape()[0].width,
        Scaled::from_raw(20 * Scaled::UNITY)
    );
    assert!(
        control
            .modes
            .current_list()
            .nodes()
            .iter()
            .any(|node| matches!(node, tex_state::node::Node::Kern { .. }))
    );
    assert!(
        control
            .modes
            .current_list()
            .nodes()
            .iter()
            .any(|node| matches!(node, tex_state::node::Node::Glue { .. }))
    );
}

/// TeX82 §1214's "Adjust for the setting of `\globaldefs`" runs before
/// `prefixed_command`'s assignment `case`, so it governs all thirty forms
/// §1210 lists -- including `set_shape` (§1248's
/// `define(par_shape_loc,shape_ref,p)`). The canonical apply arm passed the
/// raw `\global` prefix bit through instead of resolving it through
/// `assignment_global`, so a grouped `\parshape` under `\globaldefs=1` was
/// silently reverted at the closing brace and a `\global\parshape` under
/// `\globaldefs=-1` was silently made global anyway.
#[test]
fn canonical_parshape_assignment_resolves_scope_through_globaldefs() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\globaldefs=1 {\parshape=1 2pt 20pt}\globaldefs=-1 {\global\parshape=1 3pt 30pt}",
    );

    run_to_end(&mut control, &mut universe);

    let shape = universe.paragraph_shape();
    assert_eq!(
        shape.len(),
        1,
        "the unprefixed \\parshape under \\globaldefs=1 outlived its group"
    );
    assert_eq!(shape[0].indent, Scaled::from_raw(2 * Scaled::UNITY));
    assert_eq!(shape[0].width, Scaled::from_raw(20 * Scaled::UNITY));
}

#[test]
fn canonical_paragraph_page_builder_is_observer_neutral() {
    #[derive(Debug, Eq, PartialEq)]
    struct MemoParity {
        lookups: u64,
        hits: u64,
        misses: u64,
        inserts: u64,
        page_lookups: u64,
        page_hits: u64,
        page_inserts: u64,
        page_contributions_skipped: u64,
        page_key_misses: u64,
        page_validation_failures: u64,
    }

    fn run(observed: bool) -> (u64, Vec<EffectRecord>, MemoParity, Mode) {
        let mut universe = Universe::with_world(tex_state::World::memory()).with_plain_catcodes();
        let mut control = CanonicalMainControl::tex82_initex(&mut universe);
        universe.enable_pure_memo(tex_state::PureMemoConfig::default());
        universe.enable_page_memo();
        register_source(
            &mut control,
            br"\vsize=1pt\noindent paragraph text\ifhmode\count7=1\else\count7=2\fi\par\hrule height 10pt\vfill\eject\end",
        );
        let mut observations = ObservationRecorder::default();

        loop {
            let step = if observed {
                control.step_with_observer(&mut universe, &mut observations)
            } else {
                control.step(&mut universe)
            }
            .expect("canonical paragraph/page-builder program");
            if matches!(step, ReplayStep::End | ReplayStep::EndOfInput) {
                break;
            }
        }
        assert_eq!(universe.count(7), 1, "conditional sees horizontal mode");
        let memo = universe.pure_memo_stats();
        (
            universe.snapshot().state_hash(),
            universe.world().effect_records().to_vec(),
            MemoParity {
                lookups: memo.lookups,
                hits: memo.hits,
                misses: memo.misses,
                inserts: memo.inserts,
                page_lookups: memo.page_lookups,
                page_hits: memo.page_hits,
                page_inserts: memo.page_inserts,
                page_contributions_skipped: memo.page_contributions_skipped,
                page_key_misses: memo.page.key_misses,
                page_validation_failures: memo.page.validation_failures,
            },
            control.current_mode(),
        )
    }

    let cold = run(false);
    let observed = run(true);
    assert_eq!(observed, cold);
}

#[test]
fn production_driver_executes_discretionary_parts_in_isolated_hmode_episodes() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\noindent\discretionary{\count3=7\kern1pt}{\kern2pt}{\kern3pt}\count4=9",
    );

    assert_eq!(
        control.step(&mut universe).expect("paragraph start"),
        MainControlStep::Continue
    );
    assert_eq!(
        control.step(&mut universe).expect("discretionary episodes"),
        MainControlStep::Continue
    );
    assert_eq!(universe.count(3), 0, "disc group assignments stay local");
    let Some(tex_state::node::Node::Disc {
        kind: tex_state::node::DiscKind::Discretionary,
        pre,
        post,
        replace,
    }) = control.modes.current_list().nodes().last()
    else {
        panic!("canonical replay appended a discretionary node");
    };
    for (part, expected) in [
        (pre, Scaled::from_raw(Scaled::UNITY)),
        (post, Scaled::from_raw(2 * Scaled::UNITY)),
        (replace, Scaled::from_raw(3 * Scaled::UNITY)),
    ] {
        assert!(matches!(
            universe.nodes(*part).first(),
            Some(tex_state::node_arena::NodeRef::Kern { amount, .. }) if amount == expected
        ));
    }
    assert_eq!(
        control.step(&mut universe).expect("following command"),
        MainControlStep::Continue
    );
    assert_eq!(universe.count(4), 9);
}

#[test]
fn canonical_discretionary_hyphen_appends_an_explicit_hyphen_node() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\noindent\-\end");

    assert_eq!(
        control.step(&mut universe).expect("paragraph start"),
        MainControlStep::Continue
    );
    assert_eq!(
        control.step(&mut universe).expect("explicit hyphen"),
        MainControlStep::Continue
    );
    let expected_hyphen = u8::try_from(universe.font_hyphen_char(universe.current_font()))
        .ok()
        .map(char::from)
        .unwrap_or('-');
    let Some(Node::Disc {
        kind: tex_state::node::DiscKind::ExplicitHyphen,
        pre,
        post,
        replace,
    }) = control.modes.current_list().nodes().last()
    else {
        panic!("canonical replay appended an explicit discretionary hyphen");
    };
    assert!(matches!(
        universe.nodes(*pre).first(),
        Some(tex_state::node_arena::NodeRef::Char { ch, .. }) if ch == expected_hyphen
    ));
    assert!(universe.nodes(*post).is_empty());
    assert!(universe.nodes(*replace).is_empty());
}

#[test]
fn production_driver_enters_and_packs_hbox_without_legacy_dispatch() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\setbox0=\hbox{A}");

    // TeX82 §645's `scan_spec` consumes the box body's mandatory left brace
    // itself, so `\hbox{` is one step: no brace is redelivered to main
    // control before the body's first character.
    for context in ["setbox", "hbox opening", "hbox character", "hbox close"] {
        assert_eq!(
            control.step(&mut universe).expect(context),
            MainControlStep::Continue
        );
    }
    let box_nodes = universe
        .box_reg(0)
        .map(|id| universe.nodes(id))
        .expect("canonical hbox was assigned");
    assert!(matches!(
        box_nodes.first(),
        Some(tex_state::node_arena::NodeRef::HList(_))
    ));
}

#[test]
fn production_driver_hands_box_math_and_alignment_to_typed_control() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\setbox0=\vbox{}$$\halign{#\cr a\cr}\end");

    assert_eq!(
        control.step(&mut universe).expect("setbox scan"),
        MainControlStep::Continue
    );
    assert_eq!(
        control.step(&mut universe).expect("vbox opening"),
        MainControlStep::Continue
    );
    // §645's `scan_spec` already consumed `{`; the next step is the `}` that
    // packages the (empty) body.
    assert_eq!(
        control.step(&mut universe).expect("vbox completion"),
        MainControlStep::Continue
    );
    assert_eq!(
        control
            .step(&mut universe)
            .expect("paragraph start before math"),
        MainControlStep::Continue
    );
    assert!(matches!(control.current_mode(), crate::Mode::Horizontal));
    assert_eq!(
        control.step(&mut universe).expect("display math entry"),
        MainControlStep::Continue
    );
    assert!(matches!(control.current_mode(), crate::Mode::DisplayMath));
    assert_eq!(
        control.step(&mut universe).expect("alignment begin"),
        MainControlStep::Continue
    );
    assert!(control.active_alignment().is_some());
}

#[test]
fn production_driver_indent_in_display_math_appends_an_ord_sub_box() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, b"$$\\indent");

    assert_eq!(
        control.step(&mut universe).expect("paragraph start"),
        MainControlStep::Continue
    );
    assert_eq!(
        control.step(&mut universe).expect("display math entry"),
        MainControlStep::Continue
    );
    assert_eq!(
        control.step(&mut universe).expect("display-math indent"),
        MainControlStep::Continue
    );

    assert_eq!(control.current_mode(), crate::Mode::DisplayMath);
    assert!(matches!(
        control.modes.current_list().nodes(),
        [Node::MathNoad(MathNoad {
            kind: NoadKind::Normal(NoadClass::Ord),
            nucleus: MathField::SubBox(_),
            ..
        })]
    ));
}

#[test]
fn production_driver_vbox_in_display_math_appends_an_ord_sub_box() {
    // TeX82 §1075's `box_end` reaches §1076's
    // `<Append box |cur_box| to the current list, shifted by |box_context|>`
    // for every non-register, non-`\shipout`, non-leader box. That module's
    // third mode branch wraps the box in a fresh ordinary noad
    // (`p:=new_noad; math_type(nucleus(p)):=sub_box`) rather than linking it
    // into the mlist directly, because §727's `check_dimensions` updates
    // `max_h`/`max_d` only from noads -- §726 routes a bare `vlist_node`
    // straight to `done_with_node` -- and §762's `make_left_right` sizes
    // `\left`/`\right` delimiters from exactly those maxima.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"$$\vbox{}");

    for expectation in [
        "paragraph start before math",
        "display math entry",
        "vbox opening",
        "vbox completion",
    ] {
        assert_eq!(
            control.step(&mut universe).expect(expectation),
            MainControlStep::Continue
        );
    }

    assert_eq!(control.current_mode(), crate::Mode::DisplayMath);
    assert!(matches!(
        control.modes.current_list().nodes(),
        [Node::MathNoad(MathNoad {
            kind: NoadKind::Normal(NoadClass::Ord),
            nucleus: MathField::SubBox(_),
            ..
        })]
    ));
}

#[test]
fn production_driver_char_num_in_display_math_appends_a_math_char_noad() {
    // TeX82 §1154's `mmode+char_num` scans the character number and calls
    // `set_math_char` (§1155) exactly like an ordinary math-mode letter or
    // other character: it must append a math-char noad, not reach the
    // horizontal-mode character path.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, b"$$\\char43 ");

    assert_eq!(
        control.step(&mut universe).expect("paragraph start"),
        MainControlStep::Continue
    );
    assert_eq!(
        control.step(&mut universe).expect("display math entry"),
        MainControlStep::Continue
    );
    assert_eq!(
        control.step(&mut universe).expect("display-math char_num"),
        MainControlStep::Continue
    );

    assert_eq!(control.current_mode(), crate::Mode::DisplayMath);
    assert!(matches!(
        control.modes.current_list().nodes(),
        [Node::MathNoad(MathNoad {
            kind: NoadKind::Normal(NoadClass::Ord),
            nucleus: MathField::MathChar(_),
            ..
        })]
    ));
}

#[test]
fn production_driver_hrule_in_math_mode_inserts_missing_dollar_and_replays_hrule() {
    // TeX82 §1046 lists `mmode+hrule` among the "math-only cases in
    // non-math modes, or vice versa"; §1047's `insert_dollar_sign` closes
    // math mode with an inserted `$` and replays `\hrule` in the resulting
    // mode instead of reaching the generic unimplemented-typesetting error.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, b"a$\\hrule\\par");

    // TeX82 §1090's `vmode+letter` backs the letter up and starts the
    // paragraph; the letter itself is appended on the next delivery.
    assert_eq!(
        control
            .step(&mut universe)
            .expect("letter starts paragraph"),
        MainControlStep::Continue
    );
    assert_eq!(control.current_mode(), crate::Mode::Horizontal);
    assert_eq!(
        control
            .step(&mut universe)
            .expect("backed-up letter appends"),
        MainControlStep::Continue
    );
    assert_eq!(control.current_mode(), crate::Mode::Horizontal);

    assert_eq!(
        control.step(&mut universe).expect("math shift enters math"),
        MainControlStep::Continue
    );
    assert_eq!(control.current_mode(), crate::Mode::Math);

    assert_eq!(
        control
            .step(&mut universe)
            .expect("hrule recovers a missing math shift"),
        MainControlStep::Continue
    );
    assert!(terminal_text(&universe).contains("Missing $ inserted"));
    // The recovery only rewrites pending input; the mode transition happens
    // once the inserted `$` is itself replayed as the next command.
    assert_eq!(control.current_mode(), crate::Mode::Math);

    assert_eq!(
        control.step(&mut universe).expect("inserted $ closes math"),
        MainControlStep::Continue
    );
    assert_eq!(control.current_mode(), crate::Mode::Horizontal);

    assert_eq!(
        control
            .step(&mut universe)
            .expect("replayed hrule ends the paragraph and contributes a rule"),
        MainControlStep::Continue
    );
    assert_eq!(control.current_mode(), crate::Mode::Vertical);
    // §1056's `vmode+hrule` resets `prev_depth` to the ignored-depth
    // sentinel; observing that confirms the replayed `\hrule` reached the
    // ordinary vertical rule path rather than being silently dropped.
    assert_eq!(
        control.modes.current_list().prev_depth(),
        Some(crate::mode::ignored_depth(&universe))
    );
}

#[test]
fn show_reads_its_target_raw_without_starting_macro_matching() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\def\shown#1{#1}\show\shown\end");
    let mut observations = ObservationRecorder::default();

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("definition"),
        ReplayStep::Continue
    );
    observations.0.clear();

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("show"),
        ReplayStep::Continue
    );
    assert!(matches!(
        observations.0.as_slice(),
        [
            CommandObservation::Command(show_raw),
            CommandObservation::Command(show_expanded),
            CommandObservation::Command(target_raw),
        ] if show_raw.command == "xray"
            && show_raw.boundary == CommandDeliveryBoundary::Raw
            && show_expanded.command == "xray"
            && show_expanded.boundary == CommandDeliveryBoundary::Expanded
            && target_raw.command == "call"
            && target_raw.boundary == CommandDeliveryBoundary::Raw
    ));
}

#[test]
fn canonical_initex_replay_scans_and_applies_integer_parameters() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\year=2026\month=7\end");
    let mut observations = ObservationRecorder::default();

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("year assignment"),
        ReplayStep::Continue
    );
    assert_eq!(universe.int_param(IntParam::YEAR), 2026);
    assert_eq!(
        control.step(&mut universe).expect("month assignment"),
        ReplayStep::Continue
    );
    assert_eq!(universe.int_param(IntParam::MONTH), 7);
    assert_eq!(control.step(&mut universe).expect("end"), ReplayStep::End);

    assert!(matches!(
        observations.0.as_slice(),
        [..,
            CommandObservation::Scanner(scanner),
            CommandObservation::Mutation(mutation)]
            if scanner.kind == "integer"
                && scanner.value == "2026"
                && mutation.target == "parameter"
                && mutation.value == "integer_parameter:23=2026"
    ));
}

#[test]
fn nested_math_replay_observes_dimension_parameter_assignments() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\dimendef\z=0$\mathchoice{\nulldelimiterspace\z \mathsurround\z}{}{}{}$\end",
    );
    let mut observations = ObservationRecorder::default();

    while control.current_mode() != Mode::Math {
        assert_eq!(
            control.step(&mut universe).expect("fixture setup"),
            ReplayStep::Continue
        );
    }
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("math-choice replay"),
        ReplayStep::Continue
    );
    assert_eq!(
        universe.dimen_param(DimenParam::NULL_DELIMITER_SPACE),
        Scaled::from_raw(0)
    );
    assert!(
        ["dimension_parameter:11", "dimension_parameter:1"]
            .into_iter()
            .all(|key| observations.0.iter().any(|event| matches!(
                event,
                CommandObservation::Mutation(mutation)
                    if mutation.target == "parameter"
                        && mutation.key.as_deref() == Some(key)
                        && mutation.value == "scaled:0"
                        && !mutation.global
            ))),
        "unexpected observations: {:?}",
        observations.0
    );
}

#[test]
fn replay_executes_immediate_stream_extensions_and_replays_other_lookahead() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\immediate\openout2=trace \immediate\write2{ready}\immediate\closeout2\immediate\catcode`A=12\end",
    );

    for _ in 0..5 {
        assert_eq!(
            control.step(&mut universe).expect("immediate replay"),
            ReplayStep::Continue
        );
    }
    assert_eq!(universe.catcode('A'), tex_state::token::Catcode::Other);
    assert_eq!(control.step(&mut universe).expect("end"), ReplayStep::End);
    assert!(matches!(
        universe.world().effect_records(),
        [
            EffectRecord::StreamOpen { slot, target },
            EffectRecord::StreamWrite { sink: tex_state::PrintSink::Stream(write_slot), text },
            EffectRecord::StreamClose { slot: close_slot },
        ] if *slot == StreamSlot::new(2)
            && target.path() == std::path::Path::new("trace.tex")
            && *write_slot == StreamSlot::new(2)
            && text == "ready\n"
            && *close_slot == StreamSlot::new(2)
    ));
}

#[test]
fn replay_closeout_normalizes_immediate_and_deferred_write_streams() {
    for stream in ["-1", "16", "999999"] {
        let mut universe = Universe::new_with_plain_catcodes();
        let mut control = CommandReplayControl::tex82_initex(&mut universe);
        register_source(
            &mut control,
            format!("\\immediate\\openout0=kept \\immediate\\closeout{stream}").as_bytes(),
        );
        for _ in 0..2 {
            assert_eq!(
                control
                    .step(&mut universe)
                    .expect("immediate stream command"),
                ReplayStep::Continue
            );
        }
        assert!(matches!(
            universe.world().effect_records(),
            [EffectRecord::StreamOpen { slot, .. }] if *slot == StreamSlot::new(0)
        ));
    }

    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\closeout-1\closeout0\closeout15\closeout16\closeout999999",
    );
    for _ in 0..5 {
        assert_eq!(
            control.step(&mut universe).expect("deferred closeout"),
            ReplayStep::Continue
        );
    }
    let slots: Vec<_> = control
        .modes
        .current_list()
        .nodes()
        .iter()
        .map(|node| match node {
            Node::Whatsit(tex_state::node::Whatsit::CloseOut { slot }) => *slot,
            node => panic!("unexpected closeout contribution {node:?}"),
        })
        .collect();
    assert_eq!(
        slots,
        [
            None,
            Some(StreamSlot::new(0)),
            Some(StreamSlot::new(15)),
            None,
            None,
        ]
    );
    assert!(universe.world().effect_records().is_empty());
}

#[test]
fn replay_openout_keeps_four_bit_recovery_before_stream_zero_effect() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\immediate\openout-1=recovered");
    assert_eq!(
        control.step(&mut universe).expect("openout recovery"),
        ReplayStep::Continue
    );
    assert!(matches!(
        universe.world().effect_records(),
        [
            EffectRecord::StreamWrite {
                sink: tex_state::PrintSink::TerminalAndLog,
                text,
            },
            EffectRecord::StreamOpen { slot, .. },
        ] if text == "\n! Bad number (-1).\nSince I expected to read a number between 0 and 15,\nI changed this one to zero.\n"
            && *slot == StreamSlot::new(0)
    ));
}

#[test]
fn replay_closeout_stream_selector_committed_microfixture() {
    let source =
        include_bytes!("../../../../tests/corpus/tex_exec_io/closeout_stream_selectors.tex");
    let expected =
        test_support::read_fixture("tex_exec_io", "closeout_stream_selectors", "effects");
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(&mut control, source);
    run_to_end(&mut control, &mut universe);

    let actual = universe
        .world()
        .effect_records()
        .iter()
        .map(|effect| match effect {
            EffectRecord::StreamOpen { slot, target } => {
                format!("open:{}:{}", slot.raw(), target.path().display())
            }
            EffectRecord::StreamClose { slot } => format!("close:{}", slot.raw()),
            EffectRecord::StreamWrite {
                sink: tex_state::PrintSink::TerminalAndLog,
                text,
            } if text.contains("Bad number (-1)") => "diagnostic:-1".to_owned(),
            effect => panic!("unexpected microfixture effect {effect:?}"),
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(format!("{actual}\n"), expected);
}

#[test]
fn replay_appends_an_unexpanded_deferred_write_whatsit() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    control.modes.push(Mode::RestrictedHorizontal);
    register_source(&mut control, br"\write2{during}");

    assert_eq!(
        control.step(&mut universe).expect("deferred write"),
        ReplayStep::Continue
    );
    assert!(matches!(
        control.modes.current_list().nodes(),
        [Node::Whatsit(tex_state::node::Whatsit::DeferredWrite { sink, .. })]
            if *sink == tex_state::PrintSink::Stream(StreamSlot::new(2))
    ));
}

#[test]
fn canonical_initex_replay_scans_tabskip_before_alignment_preamble() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\tabskip = 2pt\halign&\end");
    let mut observations = ObservationRecorder::default();

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("tabskip assignment"),
        ReplayStep::Continue
    );
    let tabskip = universe.glue(universe.glue_param(GlueParam::TAB_SKIP));
    assert_eq!(tabskip.width, Scaled::from_raw(2 * Scaled::UNITY));
    assert!(matches!(
        observations.0.as_slice(),
        [.., CommandObservation::Scanner(scanner), CommandObservation::Mutation(mutation)]
            if scanner.kind == "glue"
                && mutation.target == "parameter"
                && mutation.key.as_deref() == Some("glue_parameter:11")
                && mutation.value.starts_with("glue:width=131072")
    ));
    observations.0.clear();

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("alignment"),
        ReplayStep::Continue
    );
    assert!(matches!(
        observations.0.as_slice(),
        [..,
            CommandObservation::Command(raw),
            CommandObservation::Command(expanded),
            CommandObservation::Alignment(alignment)]
            if raw.command == "halign"
                && expanded.command == "halign"
                && alignment.transition == "begin"
                && alignment.align_state == -1_000_000
    ));
    let alignment = control
        .active_alignment()
        .expect("alignment begins after tabskip");
    control
        .apply_alignment_request(AlignmentRequest::Preamble(alignment))
        .expect("preamble lifecycle remains available");
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("scanner-backed alignment token retires before the next delivery"),
        ReplayStep::Continue
    );
    let alignment_begin = observations
        .0
        .iter()
        .position(|event| {
            matches!(event, CommandObservation::Alignment(alignment) if alignment.transition == "begin")
        })
        .expect("replayed hAlign publishes its typed begin transition");
    let backup_retirement = observations
        .0
        .iter()
        .position(|event| {
            matches!(event, CommandObservation::Input(input)
                if input.transition == InputTransition::Retire && input.reason == InputReason::Backup)
        })
        .expect("exhausted hAlign backup retires on the following delivery");
    assert!(alignment_begin < backup_retirement);
}

#[test]
fn omit_cell_sets_body_state_without_backing_up_or_installing_a_u_template() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\halign{u#v\cr {a}\cr \omit b\cr}\end");
    let mut observations = ObservationRecorder::default();

    for _ in 0..20 {
        if control
            .step_with_observer(&mut universe, &mut observations)
            .is_err()
        {
            break;
        }
        if observations.0.windows(3).any(|events| {
            matches!(
                events,
                [
                    CommandObservation::Command(raw),
                    CommandObservation::Command(expanded),
                    CommandObservation::Alignment(state_change),
                ] if raw.command == "omit"
                    && expanded.command == "omit"
                    && state_change.transition == "state_change"
                    && state_change.align_state == 0
                    && state_change.previous_align_state == Some(1_000_000)
            )
        }) {
            break;
        }
    }

    assert!(
        observations.0.windows(3).any(|events| {
            matches!(
                events,
                [
                    CommandObservation::Command(raw),
                    CommandObservation::Command(expanded),
                    CommandObservation::Alignment(state_change),
                ] if raw.command == "omit"
                    && expanded.command == "omit"
                    && state_change.transition == "state_change"
                    && state_change.align_state == 0
                    && state_change.previous_align_state == Some(1_000_000)
            )
        }),
        "omit must transition directly from the lookahead sentinel to the cell body: {:?}",
        observations.0
    );
    assert!(
        !observations.0.windows(3).any(|events| {
            matches!(
                events,
                [
                    CommandObservation::Command(command),
                    CommandObservation::Input(input),
                    ..
                ] if command.command == "omit"
                    && input.transition == InputTransition::Backup
            )
        }),
        "TeX82 init_col never backs up its omit lookahead: {:?}",
        observations.0
    );

    for _ in 0..20 {
        if control
            .step_with_observer(&mut universe, &mut observations)
            .is_err()
        {
            break;
        }
    }
    assert!(
        observations.0.iter().any(|event| {
            matches!(event, CommandObservation::Alignment(template)
                if template.transition == "omit_template_push")
        }),
        "omit must install TeX82's omit_template, not the selected v-template: {:?}",
        observations.0
    );
}

#[test]
fn noalign_uses_command_owned_brace_scan_without_a_generic_backup() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\halign{#\cr {a}\cr\noalign{\relax}{b}\cr}\end",
    );
    let mut observations = ObservationRecorder::default();

    for _ in 0..40 {
        if control
            .step_with_observer(&mut universe, &mut observations)
            .is_err()
        {
            break;
        }
        if observations.0.windows(4).any(|events| {
            matches!(
                events,
                [
                    CommandObservation::Command(raw),
                    CommandObservation::Command(expanded),
                    CommandObservation::Alignment(state_change),
                    CommandObservation::Command(brace),
                ] if raw.command == "no_align"
                    && raw.command_operand == Some(0)
                    && expanded.command == "no_align"
                    && expanded.command_operand == Some(0)
                    && state_change.transition == "begin_group"
                    && state_change.previous_align_state == Some(1_000_000)
                    && state_change.align_state == 1_000_001
                    && brace.spelling == ObservedToken::Character {
                        character: '{',
                        catcode: Catcode::BeginGroup,
                    }
            )
        }) {
            break;
        }
    }

    let noalign = observations
        .0
        .iter()
        .position(|event| {
            matches!(event, CommandObservation::Command(command)
            if command.boundary == CommandDeliveryBoundary::Raw && command.command == "no_align")
        })
        .expect("raw TeX82 no_align delivery");
    let brace = observations
        .0
        .iter()
        .skip(noalign + 1)
        .position(|event| {
            matches!(event, CommandObservation::Command(command)
            if command.boundary == CommandDeliveryBoundary::Raw
                && command.spelling == ObservedToken::Character {
                    character: '{', catcode: Catcode::BeginGroup
                })
        })
        .map(|offset| noalign + 1 + offset)
        .expect("command-owned noalign opening brace");
    assert!(
        observations.0[noalign..=brace]
            .iter()
            .any(|event| matches!(event,
                CommandObservation::Alignment(state_change)
                    if state_change.transition == "begin_group"
                        && state_change.previous_align_state == Some(1_000_000)
                        && state_change.align_state == 1_000_001
            ))
    );
    assert!(
        !observations.0[noalign..=brace]
            .iter()
            .any(|event| matches!(event,
                CommandObservation::Input(input) if input.transition == InputTransition::Backup
            ))
    );
}

#[test]
fn alignment_preamble_opener_uses_command_owned_backup_before_source_resumes() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\halign{ U#\cr{\end");

    assert_eq!(
        control.step(&mut universe).expect("alignment begins"),
        ReplayStep::Continue
    );
    let mut observations = ObservationRecorder::default();
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("scan_spec consumes the preamble opener"),
        ReplayStep::Continue
    );
    // TeX82 §774's `scan_spec(align_group,false)` reads this `{` three times:
    // §407 `scan_keyword` reads and backs it up once per failed keyword
    // (`to`, then `spread`), and §403's `scan_left_brace` reads it a third
    // time and keeps it.  All three reads and both backups are command-owned;
    // none is an executor replay of the brace.
    assert!(
        matches!(
            observations.0.as_slice(),
            [
                CommandObservation::Alignment(first_group),
                CommandObservation::Command(first_raw),
                CommandObservation::Command(first_expanded),
                CommandObservation::Input(first_backup),
                CommandObservation::Recovery(first_recovery),
                CommandObservation::Alignment(first_correction),
                CommandObservation::Alignment(second_group),
                CommandObservation::Command(second_raw),
                CommandObservation::Command(second_expanded),
                CommandObservation::Input(retirement),
                CommandObservation::Input(second_backup),
                CommandObservation::Recovery(second_recovery),
                CommandObservation::Alignment(second_correction),
                CommandObservation::Alignment(third_group),
                CommandObservation::Command(third_raw),
                CommandObservation::Command(third_expanded),
            ]
                if [first_group, second_group, third_group].iter().all(|group| {
                    group.transition == "begin_group"
                        && group.align_state == -999_999
                        && group.previous_align_state == Some(-1_000_000)
                })
                    && [first_correction, second_correction].iter().all(|correction| {
                        correction.transition == "backup_correction"
                            && correction.align_state == -1_000_000
                            && correction.previous_align_state == Some(-999_999)
                    })
                    && [
                        first_raw,
                        first_expanded,
                        second_raw,
                        second_expanded,
                        third_raw,
                        third_expanded,
                    ]
                    .iter()
                    .all(|delivery| matches!(
                        delivery.spelling,
                        ObservedToken::Character { character: '{', .. }
                    ))
                    && first_raw.provenance.has_origin
                    && second_raw.provenance.source_location.is_none()
                    && third_raw.provenance.source_location.is_none()
                    && [first_backup, second_backup].iter().all(|backup| {
                        backup.transition == InputTransition::Backup
                            && backup.reason == InputReason::Backup
                    })
                    && [first_recovery, second_recovery]
                        .iter()
                        .all(|recovery| recovery.kind == RecoveryKind::Backup)
                    && retirement.transition == InputTransition::Retire
                    && retirement.reason == InputReason::Backup
        ),
        "unexpected observations: {:?}",
        observations.0
    );

    observations.0.clear();
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("the consumed brace enters the live preamble scanner"),
        ReplayStep::Continue
    );
    assert!(
        matches!(
            observations.0.as_slice(),
            [
                CommandObservation::ScannerStatus(status),
                CommandObservation::Alignment(preamble_start),
                CommandObservation::Input(retirement),
                CommandObservation::Command(space),
                CommandObservation::Command(template),
                CommandObservation::Command(parameter),
                CommandObservation::Command(terminator),
                CommandObservation::Alignment(preamble_finish),
                CommandObservation::ScannerStatus(finished),
            ]
                if status.from == "normal"
                    && status.to == "aligning"
                    && preamble_start.transition == "preamble_start"
                    && preamble_start.align_state == -1_000_000
                    && retirement.transition == InputTransition::Retire
                    && retirement.reason == InputReason::Backup
                    && matches!(space.spelling, ObservedToken::Character { character: ' ', .. })
                    && matches!(template.spelling, ObservedToken::Character { character: 'U', .. })
                    && matches!(parameter.spelling, ObservedToken::Character { character: '#', .. })
                    && matches!(terminator.spelling, ObservedToken::ControlSequence(ref name) if name == "cr")
                    && finished.from == "aligning"
                    && finished.to == "normal"
                    && preamble_finish.transition == "preamble_finish"
        ),
        "unexpected observations: {:?}",
        observations.0
    );

    observations.0.clear();
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("first cell opener is backed up before the u-template"),
        ReplayStep::Continue
    );
    assert!(
        matches!(
            observations.0.as_slice(),
            [
                CommandObservation::Alignment(peek),
                CommandObservation::Alignment(state_change),
                CommandObservation::Command(raw),
                CommandObservation::Command(expanded),
                CommandObservation::Input(backup),
                CommandObservation::Recovery(recovery),
                CommandObservation::Alignment(correction),
                CommandObservation::Input(template),
                CommandObservation::Alignment(template_alignment),
            ]
                if peek.transition == "state_change"
                    && peek.align_state == 1_000_000
                    && peek.previous_align_state.is_none()
                    && state_change.transition == "begin_group"
                    && state_change.align_state == 1_000_001
                    && state_change.previous_align_state == Some(1_000_000)
                    && matches!(raw.spelling, ObservedToken::Character { character: '{', .. })
                    && matches!(expanded.spelling, ObservedToken::Character { character: '{', .. })
                    && backup.transition == InputTransition::Backup
                    && backup.reason == InputReason::Backup
                    && recovery.kind == RecoveryKind::Backup
                    && correction.transition == "backup_correction"
                    && correction.align_state == 1_000_000
                    && correction.previous_align_state == Some(1_000_001)
                    && template.transition == InputTransition::Push
                    && template.reason == InputReason::AlignmentUTemplate
                    && template_alignment.transition == "u_template_push"
                    && template_alignment.align_state == 1_000_000
        ),
        "unexpected observations: {:?}",
        observations.0
    );

    observations.0.clear();
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("u-template delivers its final token"),
        ReplayStep::Continue
    );
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("backed-up u-template token replays"),
        ReplayStep::Continue
    );
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("u-template retires before the cell body resumes"),
        ReplayStep::Continue
    );
    assert!(
        observations.0.windows(3).any(|events| {
            matches!(
                events,
                [
                    CommandObservation::Input(input),
                    CommandObservation::Alignment(retirement),
                    CommandObservation::Alignment(body),
                ] if input.transition == InputTransition::Retire
                    && input.reason == InputReason::AlignmentUTemplate
                    && retirement.transition == "u_template_retire"
                    && retirement.align_state == 1_000_000
                    && body.transition == "state_change"
                    && body.align_state == 0
                    && body.previous_align_state == Some(1_000_000)
            )
        }),
        "unexpected observations: {:?}",
        observations.0
    );
    assert_eq!(
        control
            .step(&mut universe)
            .expect("inserted paragraph ends the outer horizontal list"),
        ReplayStep::Continue
    );
    // TeX82 §1051's `privileged`: the retried stop is delivered below outer
    // vertical mode -- inside the alignment -- so it reports an illegal case
    // and leaves the job running until the source runs out.
    assert_eq!(
        control
            .step(&mut universe)
            .expect("stop is not privileged inside the alignment"),
        ReplayStep::Continue
    );
    assert!(terminal_text(&universe).contains("You can't use `\\end'"));
    assert_eq!(
        control.step(&mut universe).expect("input runs out"),
        ReplayStep::EndOfInput
    );
}

#[test]
fn empty_ordinary_u_template_pushes_and_retires_before_the_cell_opener_replays() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    // The empty list before `#` is an ordinary u-template, not `\omit`.
    // `init_col` backs up any ordinary first-cell command, not just `{`.
    // This mirrors the nested `\halign{#\cr\vrule...}` trace case.
    register_source(&mut control, br"\halign{#\cr\vrule\end");
    let mut observations = ObservationRecorder::default();

    for phase in [
        "alignment begin",
        "preamble opener backup",
        "preamble opener replay",
        "preamble scan and cell setup",
        "cell opener backup and empty template installation",
    ] {
        assert_eq!(
            control
                .step_with_observer(&mut universe, &mut observations)
                .expect(phase),
            ReplayStep::Continue
        );
    }

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("empty template retires before the backed-up opener"),
        ReplayStep::Continue
    );
    assert!(
        observations.0.windows(5).any(|events| {
            matches!(
                events,
                [
                    CommandObservation::Input(push),
                    CommandObservation::Alignment(template_push),
                    CommandObservation::Input(retire),
                    CommandObservation::Alignment(template_retire),
                    CommandObservation::Alignment(body),
                ] if push.transition == InputTransition::Push
                    && push.reason == InputReason::AlignmentUTemplate
                    && template_push.transition == "u_template_push"
                    && template_push.align_state == 1_000_000
                    && retire.transition == InputTransition::Retire
                    && retire.reason == InputReason::AlignmentUTemplate
                    && template_retire.transition == "u_template_retire"
                    && template_retire.align_state == 1_000_000
                    && body.transition == "state_change"
                    && body.align_state == 0
                    && body.previous_align_state == Some(1_000_000)
            )
        }),
        "empty ordinary u-template must retain the TeX82 lifecycle: {:?}",
        observations.0
    );
}

#[test]
fn periodic_preamble_replays_its_u_template_before_retirement() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    // TeX82 §760 treats `&&` as the start of the periodic preamble suffix,
    // not as an empty second column. The following cell must therefore see
    // `\hskip` from that u-template before `end_token_list` retires it.
    register_source(&mut control, br"\halign{#&&\hskip1pt#\cr\relax&\relax\end");
    let mut observations = ObservationRecorder::default();

    for _ in 0..16 {
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("periodic preamble replay");
        if observations.0.windows(6).any(|events| {
            matches!(
                events,
                [
                    CommandObservation::Input(push),
                    CommandObservation::Alignment(template_push),
                    CommandObservation::Command(raw_hskip),
                    CommandObservation::Command(expanded_hskip),
                    CommandObservation::Command(raw_numeric),
                    CommandObservation::Command(expanded_numeric),
                ] if push.transition == InputTransition::Push
                    && push.reason == InputReason::AlignmentUTemplate
                    && template_push.transition == "u_template_push"
                    && raw_hskip.boundary == CommandDeliveryBoundary::Raw
                    && raw_hskip.command == "hskip"
                    && expanded_hskip.boundary == CommandDeliveryBoundary::Expanded
                    && expanded_hskip.command == "hskip"
                    && matches!(raw_numeric.spelling, ObservedToken::Character { character: '1', .. })
                    && matches!(expanded_numeric.spelling, ObservedToken::Character { character: '1', .. })
            )
        }) && observations.0.windows(6).any(|events| {
            matches!(
                events,
                [
                    CommandObservation::Command(raw_hskip),
                    CommandObservation::Command(expanded_hskip),
                    CommandObservation::Command(raw_numeric),
                    CommandObservation::Command(expanded_numeric),
                    CommandObservation::Input(backup),
                    CommandObservation::Recovery(recovery),
                ] if raw_hskip.boundary == CommandDeliveryBoundary::Raw
                    && raw_hskip.command == "hskip"
                    && expanded_hskip.boundary == CommandDeliveryBoundary::Expanded
                    && expanded_hskip.command == "hskip"
                    && matches!(raw_numeric.spelling, ObservedToken::Character { character: '1', .. })
                    && matches!(expanded_numeric.spelling, ObservedToken::Character { character: '1', .. })
                    && backup.transition == InputTransition::Backup
                    && backup.reason == InputReason::Backup
                    && recovery.kind == RecoveryKind::Backup
                    && matches!(recovery.tokens.as_slice(), [ObservedToken::Character { character: '1', .. }])
            )
        }) {
            return;
        }
    }
    panic!(
        "periodic u-template must deliver hskip before retirement: {:?}",
        observations.0
    );
}

#[test]
fn completed_rule_spec_restarts_active_cell_through_typed_delimiter_delivery() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\halign{#\cr{\vrule width1pt}&\end");
    let mut observations = ObservationRecorder::default();

    for phase in [
        "alignment begin",
        "preamble scan_spec",
        "preamble scan and first cell",
        "cell opener and template installation",
        "replayed cell opener",
    ] {
        assert_eq!(
            control
                .step_with_observer(&mut universe, &mut observations)
                .expect(phase),
            ReplayStep::Continue
        );
        observations.0.clear();
    }

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("rule specification"),
        ReplayStep::Continue
    );
    assert!(
        observations.0.iter().any(|observation| {
            matches!(
                observation,
                CommandObservation::Scanner(scanner)
                    if scanner.kind == "dimension" && scanner.value == "65536"
            )
        }),
        "unexpected rule observations: {:?}",
        observations.0
    );
    observations.0.clear();

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("cell-body closing brace"),
        ReplayStep::Continue
    );
    observations.0.clear();

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("backed-up tab reaches the alignment delivery boundary"),
        ReplayStep::Continue
    );
    assert!(
        observations.0.windows(4).any(|events| {
            matches!(
                events,
                [
                    CommandObservation::Alignment(delimiter),
                    CommandObservation::Input(template_input),
                    CommandObservation::Alignment(template),
                    CommandObservation::Alignment(state_change),
                ] if delimiter.transition == "delimiter"
                    && delimiter.align_state == 0
                    && delimiter.delimiter == Some("tab")
                    && template_input.transition == InputTransition::Push
                    && template_input.reason == InputReason::AlignmentVTemplate
                    && template.transition == "v_template_push"
                    && state_change.transition == "state_change"
                    && state_change.align_state == 1_000_000
            )
        }),
        "unexpected observations: {:?}",
        observations.0
    );
}

#[test]
fn canonical_initex_replay_scans_token_register_assignments_through_command_core() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\toks0={TOKEN LIST}\end");
    let mut observations = ObservationRecorder::default();

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("token-register assignment"),
        ReplayStep::Continue
    );
    assert_eq!(replay_text(universe.tokens(universe.toks(0))), "TOKEN LIST");
    assert!(observations.0.windows(2).any(|pair| {
        matches!(
            &pair[0],
            CommandObservation::Input(input)
                if input.transition == InputTransition::Backup && input.reason == InputReason::Backup
        ) && matches!(
            &pair[1],
            CommandObservation::Recovery(recovery) if recovery.kind == RecoveryKind::Backup
        )
    }));
    assert!(matches!(
        observations.0.as_slice(),
        [..,
            CommandObservation::ScannerStatus(status),
            CommandObservation::TokenList(tokens),
            CommandObservation::Mutation(mutation)]
            if status.to == "normal"
                && tokens.transition == "complete"
                && tokens.purpose == "scan_toks"
                && mutation.target == "register"
                && mutation.key.as_deref() == Some("toks:0")
                && mutation.value == "tokens"
                && !mutation.global
    ));
    assert_eq!(control.step(&mut universe).expect("end"), ReplayStep::End);
}

#[test]
fn canonical_toksdef_projects_its_committed_named_register_meaning() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\toksdef\tokens=256\end");
    let mut observations = ObservationRecorder::default();

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("toksdef"),
        ReplayStep::Continue
    );
    assert_eq!(
        universe.meaning(universe.symbol("tokens").expect("toksdef target")),
        Meaning::ToksRegister(0)
    );
    assert!(matches!(
        observations.0.last(),
        Some(CommandObservation::Mutation(mutation))
            if mutation.target == "meaning"
                && mutation.key.as_deref() == Some("tokens")
                && mutation.value == "assign_toks"
                && !mutation.global
    ));
    assert_eq!(control.step(&mut universe).expect("end"), ReplayStep::End);
}

#[test]
fn canonical_initex_replay_copies_direct_token_register_rhs() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\toks0={zero}\toks20={twenty}\toks1=\toks20\toks2=\toks256\end",
    );
    let mut observations = ObservationRecorder::default();

    for _ in 0..4 {
        assert_eq!(
            control
                .step_with_observer(&mut universe, &mut observations)
                .expect("token-register assignment"),
            ReplayStep::Continue
        );
    }

    assert_eq!(replay_text(universe.tokens(universe.toks(1))), "twenty");
    assert_eq!(replay_text(universe.tokens(universe.toks(2))), "zero");
    assert!(observations.0.windows(2).any(|pair| {
        matches!(
            pair,
            [
                CommandObservation::Scanner(scanner),
                CommandObservation::Mutation(mutation),
            ] if scanner.kind == "integer"
                && scanner.value == "20"
                && mutation.key.as_deref() == Some("toks:1")
        )
    }));
    assert_eq!(control.step(&mut universe).expect("end"), ReplayStep::End);
}

#[test]
fn canonical_initex_replay_scans_setbox_then_hands_vbox_to_executor() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\setbox10=\vbox{}\end");
    let mut observations = ObservationRecorder::default();

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("setbox prefix"),
        ReplayStep::Continue
    );
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("vbox handoff"),
        ReplayStep::Continue
    );
    assert!(matches!(
        universe.group_kinds().next_back(),
        Some(tex_state::GroupKind::VBox)
    ));
    // §645's `scan_spec` consumed `{` during the handoff step above, so the
    // next delivered command is the `}` that packages the empty body.
    assert_eq!(
        control.step(&mut universe).expect("package vbox"),
        ReplayStep::Continue
    );
    assert!(
        universe.box_reg(10).is_some(),
        "vbox is assigned at group exit"
    );

    assert!(observations.0.windows(2).any(|pair| {
        matches!(
            &pair,
            [CommandObservation::Input(input), CommandObservation::Recovery(recovery)]
                if input.transition == InputTransition::Backup
                    && input.reason == InputReason::Backup
                    && recovery.kind == RecoveryKind::Backup
        )
    }));
    assert!(observations.0.iter().any(|event| {
        matches!(event, CommandObservation::Command(command)
            if command.command == "make_box" && command.command_operand == Some(5))
    }));
}

/// TeX82 §1063's `non_math(left_brace): new_save_level(simple_group)` applies
/// inside a box body exactly as it does outside one, and §1069's
/// `simple_group: unsave` closes it -- backing up the `\aftergroup` tokens
/// §282's `unsave` saved and restoring the level's local values. Only the
/// brace delivered while `cur_group` is still the body's own group packages
/// the box (§1068's `handle_right_brace` dispatches purely on `cur_group`).
#[test]
fn canonical_nested_brace_in_box_body_is_a_simple_group() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\def\A{\global\count1=2}\setbox10=\vbox{{\count0=1\aftergroup\A}}\end",
    );

    run_to_end(&mut control, &mut universe);

    assert!(
        universe.box_reg(10).is_some(),
        "the body's own closing brace still packages the vbox"
    );
    assert_eq!(
        universe.count(0),
        0,
        "the nested group's local assignment is restored by §1069's unsave"
    );
    assert_eq!(
        universe.count(1),
        2,
        "§282's unsave backs the nested group's \\aftergroup token up"
    );
}

#[test]
fn canonical_initex_replay_scans_box_register_before_stomach_consumes_it() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\setbox10=\vbox{}\box10\end");

    // `\setbox10=`, `\vbox{` (§645 consumes the brace), `}`.
    for _ in 0..3 {
        assert_eq!(
            control.step(&mut universe).expect("setbox construction"),
            ReplayStep::Continue
        );
    }
    assert!(universe.box_reg(10).is_some(), "setbox completed");

    let mut observations = ObservationRecorder::default();
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("box register scan"),
        ReplayStep::Continue
    );
    assert!(universe.box_reg(10).is_none(), "box consumes its register");

    let make_box = observations
        .0
        .iter()
        .position(|event| {
            matches!(event, CommandObservation::Command(command)
            if command.boundary == CommandDeliveryBoundary::Raw
                && command.command == "make_box"
                && command.command_operand == Some(0))
        })
        .expect("raw make_box(box_code) identity");
    let first_digit = observations
        .0
        .iter()
        .enumerate()
        .skip(make_box + 1)
        .find_map(|(index, event)| {
            matches!(event, CommandObservation::Command(command)
            if command.boundary == CommandDeliveryBoundary::Raw
                && command.spelling == ObservedToken::Character {
                    character: '1', catcode: tex_state::token::Catcode::Other,
                })
            .then_some(index)
        })
        .expect("command-owned scan_int delivers the first register digit raw");
    assert!(
        !observations.0[make_box + 1..first_digit]
            .iter()
            .any(|event| matches!(event, CommandObservation::Input(input)
                if input.transition == InputTransition::Backup)),
        "the register digit is not an executor-created backup replay: {:?}",
        observations.0
    );
    let second_digit = observations
        .0
        .iter()
        .enumerate()
        .skip(first_digit + 1)
        .find_map(|(index, event)| {
            matches!(event, CommandObservation::Command(command)
            if command.boundary == CommandDeliveryBoundary::Raw
                && command.spelling == ObservedToken::Character {
                    character: '0', catcode: tex_state::token::Catcode::Other,
                })
            .then_some(index)
        })
        .expect("second register digit remains raw command input");
    let terminator_backup = observations
        .0
        .iter()
        .enumerate()
        .skip(second_digit + 1)
        .find_map(|(index, event)| {
            matches!(event, CommandObservation::Input(input)
            if input.transition == InputTransition::Backup)
            .then_some(index)
        })
        .expect("scan_int backs up the following box terminator after both digits");
    assert!(
        second_digit < terminator_backup,
        "integer terminator backup follows the completed register operand"
    );
}

#[test]
fn shipout_box_completion_precedes_its_terminator_backup_retirement() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\setbox10=\vbox{}\setbox11=\vbox{}\shipout\vbox{\box10\box11}\end",
    );
    let mut observations = ObservationRecorder::default();

    for _ in 0..32 {
        if matches!(
            control
                .step_with_observer(&mut universe, &mut observations)
                .expect("canonical replay"),
            ReplayStep::End | ReplayStep::EndOfInput
        ) {
            break;
        }
    }

    let shipout = observations
        .0
        .iter()
        .position(|event| {
            matches!(event, CommandObservation::Effect(effect)
                if effect.kind == "shipout" && effect.detail == "dvi\0".to_owned() + "1")
        })
        .expect("completed vbox publishes DVI page one");
    let retirement = observations
        .0
        .iter()
        .enumerate()
        .skip(shipout + 1)
        .find_map(|(index, event)| {
            matches!(event, CommandObservation::Input(input)
                if input.transition == InputTransition::Retire && input.reason == InputReason::Backup)
            .then_some(index)
        })
        .expect("box-register terminator backup retires on the next raw fetch");
    assert!(
        shipout < retirement,
        "TeX82 box_end ships out before scan_int's terminator backup retires: {:?}",
        observations.0
    );
}

#[test]
fn canonical_initex_replay_observes_committed_message_effects() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\message{READY}\end");
    let mut observations = ObservationRecorder::default();

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("message"),
        ReplayStep::Continue
    );
    assert!(matches!(
        observations.0.last(),
        Some(CommandObservation::Effect(effect))
            if effect.kind == "message" && effect.detail == "READY"
    ));
    assert_eq!(control.step(&mut universe).expect("end"), ReplayStep::End);
}

#[test]
fn end_in_outer_horizontal_mode_replays_paragraph_before_retrying_stop() {
    // TeX82 §1095 (`hmode+stop` / `head_for_vmode`) first backs up \end,
    // then backs up inserted \par. The command processor owns both input
    // transitions; applying the delivered paragraph is the executor's typed
    // mode transition before \end is reconsidered in vertical mode.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(&mut control, b"x\\end");
    let mut observations = ObservationRecorder::default();

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("paragraph starts"),
        ReplayStep::Continue
    );
    assert_eq!(control.current_mode(), crate::Mode::Horizontal);
    observations.0.clear();

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("replayed paragraph character"),
        ReplayStep::Continue
    );
    observations.0.clear();

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("stop is deferred to vertical mode"),
        ReplayStep::Continue
    );
    assert!(
        matches!(
            observations.0.as_slice(),
            [
                CommandObservation::Input(previous_retirement),
                CommandObservation::Command(raw),
                CommandObservation::Command(expanded),
                CommandObservation::Input(end_backup),
                CommandObservation::Recovery(end_recovery),
                CommandObservation::Input(par_backup),
                CommandObservation::Recovery(par_recovery),
            ] if previous_retirement.transition == InputTransition::Retire
                && previous_retirement.reason == InputReason::Backup
                && raw.command == "stop"
                && expanded.command == "stop"
                && end_backup.transition == InputTransition::Backup
                && end_recovery.kind == RecoveryKind::Backup
                && par_backup.transition == InputTransition::Backup
                && par_recovery.kind == RecoveryKind::Backup
                && matches!(par_recovery.tokens.as_slice(), [ObservedToken::ControlSequence(name)] if name == "par")
        ),
        "unexpected stop recovery observations: {:?}",
        observations.0
    );

    observations.0.clear();
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("inserted paragraph"),
        ReplayStep::Continue
    );
    assert_eq!(control.current_mode(), crate::Mode::Vertical);
    assert!(matches!(
        observations.0.as_slice(),
        [CommandObservation::Command(raw), CommandObservation::Command(expanded)]
            if raw.command == "par_end" && expanded.command == "par_end"
    ));

    observations.0.clear();
    // TeX82 §1054: the paragraph is on the page, so the retried stop is not
    // "all over" -- it is backed up again while §994's `build_page` ejects
    // the residual page. §1012's `fire_up` reaches §638's `ship_out` through
    // §1025's null-`\output` case, so §640's page commit publishes a shipout
    // effect here even though no `\shipout` command was ever executed.
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("stop retries in vertical mode"),
        ReplayStep::Continue
    );
    assert!(
        matches!(
            observations.0.as_slice(),
            [
                CommandObservation::Input(retirement),
                CommandObservation::Command(raw),
                CommandObservation::Command(expanded),
                CommandObservation::Input(backup_retirement),
                CommandObservation::Input(backup),
                CommandObservation::Recovery(recovery),
                CommandObservation::Effect(shipout),
            ] if retirement.transition == InputTransition::Retire
                && retirement.reason == InputReason::Recovery
                && raw.command == "stop"
                && expanded.command == "stop"
                && backup_retirement.transition == InputTransition::Retire
                && backup_retirement.reason == InputReason::Backup
                && backup.transition == InputTransition::Backup
                && recovery.kind == RecoveryKind::Backup
                && shipout.kind == "shipout"
                && shipout.detail == "dvi\u{0}1"
        ),
        "unexpected residual-page ejection observations: {:?}",
        observations.0
    );

    observations.0.clear();
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("the retry after the ejected page ends the job"),
        ReplayStep::End
    );
    // §1335's `final_cleanup` unwinds the abandoned input stack and the
    // termination effect follows it.
    assert!(
        matches!(
            observations.0.as_slice(),
            [
                CommandObservation::Command(raw),
                CommandObservation::Command(expanded),
                CommandObservation::Input(backup_retire),
                CommandObservation::Input(source_retire),
                CommandObservation::Input(terminal_stop),
                CommandObservation::Effect(terminate),
            ] if raw.command == "stop"
                && expanded.command == "stop"
                && backup_retire.transition == InputTransition::Retire
                && backup_retire.reason == InputReason::Backup
                && source_retire.transition == InputTransition::Retire
                && source_retire.reason == InputReason::Source
                && terminal_stop.transition == InputTransition::Stop
                && terminal_stop.reason == InputReason::Source
                && terminate.kind == "terminate"
        ),
        "unexpected final-cleanup observations: {:?}",
        observations.0
    );
}

#[test]
fn final_stop_retires_its_backup_before_starting_output_input() {
    // TeX82 §46 (`its_all_over`) starts \output only after the §1095
    // redelivery's exhausted \end backup has retired and a new final-stop
    // backup is in place.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\output={O}x\end");
    let mut observations = ObservationRecorder::default();

    for expected in [
        "output assignment",
        "paragraph start",
        "paragraph character",
    ] {
        assert_eq!(
            control
                .step_with_observer(&mut universe, &mut observations)
                .expect(expected),
            ReplayStep::Continue
        );
        observations.0.clear();
    }

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("horizontal stop recovery"),
        ReplayStep::Continue
    );
    observations.0.clear();
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("inserted paragraph"),
        ReplayStep::Continue
    );
    observations.0.clear();

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("output hand-off"),
        ReplayStep::Continue
    );
    assert!(
        matches!(
            observations.0.as_slice(),
            [
                CommandObservation::Input(recovery_retirement),
                CommandObservation::Command(raw),
                CommandObservation::Command(expanded),
                CommandObservation::Input(retirement),
                CommandObservation::Input(backup),
                CommandObservation::Recovery(recovery),
                CommandObservation::Input(output),
            ] if recovery_retirement.transition == InputTransition::Retire
                && recovery_retirement.reason == InputReason::Recovery
                && retirement.transition == InputTransition::Retire
                && retirement.reason == InputReason::Backup
                && raw.command == "stop"
                && expanded.command == "stop"
                && backup.transition == InputTransition::Backup
                && recovery.kind == RecoveryKind::Backup
                && output.transition == InputTransition::Push
                && output.reason == InputReason::OutputRoutine
        ),
        "unexpected output hand-off observations: {:?}",
        observations.0
    );

    for expected in [
        "output opening brace",
        "output paragraph start",
        "output paragraph character",
        "output closing brace",
    ] {
        observations.0.clear();
        assert_eq!(
            control
                .step_with_observer(&mut universe, &mut observations)
                .expect(expected),
            ReplayStep::Continue
        );
    }

    observations.0.clear();
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("final stop retries below the completed output routine"),
        ReplayStep::Continue
    );
    assert!(
        matches!(
            observations.0.as_slice(),
            [
                CommandObservation::Input(output_retirement),
                CommandObservation::Command(raw),
                CommandObservation::Command(expanded),
                CommandObservation::Input(backup_retirement),
                CommandObservation::Input(backup),
                CommandObservation::Recovery(recovery),
                CommandObservation::Input(output),
            ] if output_retirement.transition == InputTransition::Retire
                && output_retirement.reason == InputReason::OutputRoutine
                && raw.command == "stop"
                && expanded.command == "stop"
                && backup_retirement.transition == InputTransition::Retire
                && backup_retirement.reason == InputReason::Backup
                && backup.transition == InputTransition::Backup
                && recovery.kind == RecoveryKind::Backup
                && output.transition == InputTransition::Push
                && output.reason == InputReason::OutputRoutine
        ),
        "completed output routine must restore vertical final-stop retry: {:?}",
        observations.0
    );
}

#[test]
fn output_group_waits_for_nested_box_body_before_closing() {
    // TeX82 §1016 starts the braced \output list inside an already-open
    // output_group. §1077's nested box body must consume its own right brace
    // before §1026 can tear down that enclosing output group. This is the
    // reduced Plain `\line{\vbox to8.5\p@{}}` shape.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\output={\global\output={}\shipout\vbox{\hbox{\vbox to8.5pt{}}}}\topskip=0pt\vsize=1pt\setbox0=\hbox{}\ht0=2pt\copy0\penalty-10000\end",
    );

    run_to_end(&mut control, &mut universe);

    // One page, not two: the routine's own `\shipout` resets `dead_cycles`
    // (§638) and empties the page, so §1054's `its_all_over` is true when
    // `\end` is reconsidered and the job ends without ejecting a second
    // page.
    assert_eq!(universe.world().artifact_commits().len(), 1);
    assert!(universe.box_reg(255).is_none());
}

/// TeX82 §1026 retires the output token list before it runs the shared §1096
/// `end_graf`; therefore a live output paragraph becomes a line on the next
/// page, while the output group is still the scope that `unsave` restores.
#[test]
fn output_routine_end_graf_keeps_non_null_paragraph_and_unsaves_afterward() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_cmr10_font(&mut control, &mut universe);
    register_source(
        &mut control,
        br"\font\f=cmr10 \f \count7=3 \output={\global\output={} \count7=9 \global\count8=1 \noindent A\shipout\box255}\topskip=0pt\vsize=1pt\hsize=100pt\setbox0=\hbox{}\ht0=2pt\copy0\penalty-10000\end",
    );

    run_to_end(&mut control, &mut universe);

    assert_eq!(
        universe.count(7),
        3,
        "§1026 unsaves output-local assignments"
    );
    assert_eq!(
        universe.count(8),
        1,
        "global output assignments survive unsave"
    );
    assert_eq!(control.current_mode(), crate::Mode::Vertical);
    assert_eq!(universe.group_depth(), 0, "the output group has closed");
    assert!(universe.box_reg(255).is_none());
    assert_eq!(
        universe.world().artifact_commits().len(),
        2,
        "the line broken from the non-null output paragraph resumes the page builder"
    );
}

/// The §1096 null-paragraph branch still applies at §1026: it pops the
/// horizontal level without adding a line to the resumed page builder.
#[test]
fn output_routine_end_graf_ignores_null_paragraph() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\output={\global\output={}\shipout\box255\noindent}\topskip=0pt\vsize=1pt\setbox0=\hbox{}\ht0=2pt\copy0\penalty-10000\end",
    );

    run_to_end(&mut control, &mut universe);

    assert_eq!(control.current_mode(), crate::Mode::Vertical);
    assert_eq!(universe.group_depth(), 0);
    assert_eq!(universe.world().artifact_commits().len(), 1);
}

#[test]
fn simple_group_ancestor_does_not_close_nested_output_box() {
    // TeX82 §1068 dispatches `}` from the live `cur_group`.  Plain's `\big`
    // has this shape: its simple group stays open while the nested `\vbox`
    // body closes.  The closer must package the vbox, not unsave that
    // ancestor simple group.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\output={{\global\output={}\shipout\vbox{\hbox{\vbox to8.5pt{}}}}}\topskip=0pt\vsize=1pt\setbox0=\hbox{}\ht0=2pt\copy0\penalty-10000\end",
    );

    run_to_end(&mut control, &mut universe);

    // One page, not two: the routine's own `\shipout` resets `dead_cycles`
    // (§638) and empties the page, so §1054's `its_all_over` is true when
    // `\end` is reconsidered and the job ends without ejecting a second
    // page.
    assert_eq!(universe.world().artifact_commits().len(), 1);
    assert!(universe.box_reg(255).is_none());
}

#[test]
fn hrule_contributes_to_outer_page_before_final_shipout() {
    // TeX82 §1056 puts a vertical-mode \hrule on the contribution list
    // without calling build_page. It must remain there until a later explicit
    // page-builder visit, then reach final shipout rather than staying
    // stranded on the mode nest.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\hrule height1pt width2pt\end");

    assert_eq!(
        control.step(&mut universe).expect("hrule executes"),
        ReplayStep::Continue
    );
    assert!(matches!(
        universe.page_contribution_front(),
        Some(Node::Rule { width, height, depth })
            if *width == Some(Scaled::from_raw(2 * Scaled::UNITY))
                && *height == Some(Scaled::from_raw(Scaled::UNITY))
                && *depth == Some(Scaled::from_raw(0))
    ));
    run_to_end(&mut control, &mut universe);
    assert_eq!(universe.world().artifact_commits().len(), 1);
}

#[test]
fn end_with_an_empty_page_ends_the_job_without_running_output() {
    // TeX82 §1054's `its_all_over`: with the current page and the
    // contribution list both empty and `dead_cycles=0`, `\end` ends the job
    // immediately. `\output` is not consulted at all -- §1025 is reached
    // only through §1012's `fire_up`, never from the stop dispatch -- so a
    // job that typeset nothing produces no page, exactly as TeX82's
    // "No pages of output." reports.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\maxdeadcycles=1\output={X}\end");
    let mut observations = ObservationRecorder::default();

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("maxdeadcycles assignment"),
        ReplayStep::Continue
    );
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("output assignment"),
        ReplayStep::Continue
    );
    observations.0.clear();
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("stop ends the job"),
        ReplayStep::End
    );

    assert!(universe.world().artifact_commits().is_empty());
    assert!(
        !observations.0.iter().any(|observation| matches!(
            observation,
            CommandObservation::Effect(effect) if effect.kind == "shipout"
        )),
        "an empty job must not ship a page: {:?}",
        observations.0
    );
    assert!(observations.0.iter().any(|observation| matches!(
        observation,
        CommandObservation::Effect(effect) if effect.kind == "terminate"
    )));
}

#[test]
fn dead_output_cycles_force_a_shipout_from_fire_up() {
    // TeX82 §1005's `@<Explain that too many dead cycles have occurred...@>`:
    // once `dead_cycles` reaches `\maxdeadcycles`, `fire_up` ships `\box255`
    // itself instead of entering `\output` again. Reaching that escape needs
    // real page material, because §1054 would otherwise have ended the job
    // before any output routine ran.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\maxdeadcycles=1\output={X}\setbox0=\hbox{}\copy0\end",
    );

    run_to_end(&mut control, &mut universe);

    assert_eq!(universe.world().artifact_commits().len(), 1);
    assert!(
        terminal_text(&universe).contains("Output loop---"),
        "the dead-cycle escape must report itself: {}",
        terminal_text(&universe)
    );
}

#[test]
fn off_save_reports_before_replaying_its_inserted_closer() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(&mut control, br"{\endgroup}");
    let mut observations = ObservationRecorder::default();

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("opening group"),
        ReplayStep::Continue
    );
    observations.0.clear();
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("off_save recovery"),
        ReplayStep::Continue
    );
    assert!(matches!(
        observations.0.as_slice(),
        [
            CommandObservation::Command(_),
            CommandObservation::Command(_),
            CommandObservation::Diagnostic(diagnostic),
            CommandObservation::Input(backup),
            CommandObservation::Recovery(recovery),
            CommandObservation::Input(inserted),
            CommandObservation::Recovery(inserted_recovery),
        ] if diagnostic.diagnostic == "off_save_replay"
            && backup.transition == InputTransition::Backup
            && recovery.kind == RecoveryKind::Backup
            && inserted.transition == InputTransition::Recovery
            && inserted_recovery.kind == RecoveryKind::InsertedToken
    ));
    assert_eq!(
        terminal_text(&universe),
        "\n! Missing } inserted.\n",
        "TeX82 §1065 prints the terminal-and-log diagnostic exactly once before the inserted closer replays",
    );

    observations.0.clear();
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("inserted closer"),
        ReplayStep::Continue
    );
    assert!(
        matches!(
            observations.0.as_slice(),
            [CommandObservation::Command(raw), CommandObservation::Command(expanded)]
                if matches!(raw.spelling, ObservedToken::Character { character: '}', catcode: Catcode::EndGroup })
                    && matches!(expanded.spelling, ObservedToken::Character { character: '}', catcode: Catcode::EndGroup })
        ),
        "unexpected inserted-closer observations: {:?}",
        observations.0
    );

    observations.0.clear();
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("bottom-level replay drop"),
        ReplayStep::Continue
    );
    assert!(matches!(
        observations.0.as_slice(),
        [
            CommandObservation::Input(retirement),
            CommandObservation::Command(_),
            CommandObservation::Command(_),
            CommandObservation::Diagnostic(diagnostic),
        ] if retirement.transition == InputTransition::Retire
            && retirement.reason == InputReason::Recovery
            && diagnostic.diagnostic == "off_save_bottom_drop"
    ));
    assert_eq!(
        terminal_text(&universe),
        "\n! Missing } inserted.\n\n! Extra \\endgroup.\n",
        "TeX82 §§1065--1066 print one recovery diagnostic and one later bottom-level drop diagnostic",
    );
}

#[test]
fn canonical_vskip_in_restricted_horizontal_runs_off_save() {
    // TeX82 §1091's `head_for_vmode` restricted branch (`mode<0`): inside an
    // `\hbox`, `\vskip` (and its `\vfil`/`\vfill`/`\vss`/`\vfilneg` siblings)
    // cannot simply retry behind an inserted `\par` the way unrestricted
    // horizontal mode does -- `\par` has no meaning in restricted horizontal
    // mode. Instead `off_save` (§1064) must first close the hbox's own
    // group, which is a `HBox`-kind `Universe` group here, so it takes
    // §1065's "othercases" branch and inserts an ordinary `}`.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\setbox0=\hbox{\vskip1pt}");
    run_to_end(&mut control, &mut universe);

    let text = terminal_text(&universe);
    assert!(text.contains("Missing } inserted"), "{text}");

    // The hbox is packaged empty: `\vskip` never entered its hlist.
    let box_nodes = universe
        .box_reg(0)
        .map(|id| universe.nodes(id))
        .expect("hbox was still assigned despite the interrupted body");
    let Some(tex_state::node_arena::NodeRef::HList(hbox)) = box_nodes.first() else {
        panic!("expected an assigned hbox: {box_nodes:?}");
    };
    assert!(
        universe.nodes(hbox.children).is_empty(),
        "vskip should not have entered the hbox"
    );

    // Once off_save closes the hbox, the recovered `\vskip1pt` replays as
    // ordinary vertical glue -- outer vertical mode contributes straight to
    // the page builder's contribution list rather than `ModeNest`'s own
    // per-mode list, so that (not `control.modes.current_list()`) is where
    // the recovered glue shows up.
    assert!(matches!(
        universe.page_contributions().back(),
        Some(Node::Glue { spec, .. })
            if universe.glue(*spec).width == Scaled::from_raw(Scaled::UNITY)
    ));
}

#[test]
fn canonical_vskip_in_restricted_horizontal_closes_a_semisimple_group_first() {
    // TeX82 §1065's `semi_simple_group` branch of `off_save`: `\begingroup`
    // (`any_mode(begin_group): new_save_level(semi_simple_group)`) can open
    // a semisimple group nested inside an `\hbox` without changing the mode,
    // so `off_save` must close it with the frozen, redefinition-proof
    // `\endgroup` rather than the hbox's own `}` -- and exercises
    // `CommandProcessor::frozen_primitive_token` in the process.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\setbox0=\hbox{\begingroup\vskip1pt}");
    run_to_end(&mut control, &mut universe);

    let text = terminal_text(&universe);
    assert!(text.contains("Missing \\endgroup inserted"), "{text}");
    // Closing the semisimple group still leaves restricted horizontal mode
    // active, so `\vskip` reaches `off_save` a second time against the
    // hbox's own group.
    assert!(text.contains("Missing } inserted"), "{text}");

    let box_nodes = universe
        .box_reg(0)
        .map(|id| universe.nodes(id))
        .expect("hbox was still assigned despite the interrupted body");
    let Some(tex_state::node_arena::NodeRef::HList(hbox)) = box_nodes.first() else {
        panic!("expected an assigned hbox: {box_nodes:?}");
    };
    assert!(
        universe.nodes(hbox.children).is_empty(),
        "vskip should not have entered the hbox"
    );
    assert!(matches!(
        universe.page_contributions().back(),
        Some(Node::Glue { spec, .. })
            if universe.glue(*spec).width == Scaled::from_raw(Scaled::UNITY)
    ));
}

#[test]
fn canonical_initex_replay_scans_and_applies_code_table_assignments() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\catcode`@=11 \lccode`Z=`z \end");

    assert_eq!(
        control.step(&mut universe).expect("catcode assignment"),
        ReplayStep::Continue
    );
    assert_eq!(universe.catcode('@'), tex_state::token::Catcode::Letter);
    assert_eq!(
        control.step(&mut universe).expect("lccode assignment"),
        ReplayStep::Continue
    );
    assert_eq!(universe.lccode('Z'), u32::from('z'));
    assert_eq!(control.step(&mut universe).expect("end"), ReplayStep::End);
}

#[test]
fn canonical_initex_replay_scans_raw_let_operands_and_commits_the_meaning() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\let\alias = \begingroup\end");
    let mut observations = ObservationRecorder::default();

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("let assignment"),
        ReplayStep::Continue
    );
    let alias = universe.symbol("alias").expect("let target is interned");
    assert_eq!(
        universe.meaning(alias),
        Meaning::UnexpandablePrimitive(tex_state::meaning::UnexpandablePrimitive::BeginGroup)
    );
    assert!(matches!(
        observations.0.last(),
        Some(CommandObservation::Mutation(mutation))
            if mutation.target == "meaning"
                && mutation.key.as_deref() == Some("alias")
                && mutation.value == "begin_group"
    ));
    assert_eq!(control.step(&mut universe).expect("end"), ReplayStep::End);
}

#[test]
fn canonical_initex_replay_keeps_macro_target_and_expanded_body_in_command_core() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\def\source{expanded}\long\outer\edef\target{\source}\end",
    );

    assert_eq!(
        control.step(&mut universe).expect("source definition"),
        ReplayStep::Continue
    );
    assert_eq!(
        control.step(&mut universe).expect("expanded definition"),
        ReplayStep::Continue
    );
    let target = universe.symbol("target").expect("macro target is interned");
    let meaning = universe.macro_meaning(target).expect("target is a macro");
    assert!(
        meaning
            .flags()
            .contains(tex_state::meaning::MeaningFlags::LONG)
    );
    assert!(
        meaning
            .flags()
            .contains(tex_state::meaning::MeaningFlags::OUTER)
    );
    assert_eq!(
        replay_text(universe.tokens(meaning.replacement_text())),
        "expanded"
    );
    assert_eq!(control.step(&mut universe).expect("end"), ReplayStep::End);
}

#[test]
fn canonical_case_shift_replays_raw_text_with_categories_and_order_intact() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\uccode`!=`Z \lccode`?=`y
           \uppercase{\def\up{!\relax}}
           \lowercase{\def\down{?\relax}}
           \uppercase{\def\zero{@}}\end",
    );

    for _ in 0..2 {
        assert_eq!(
            control.step(&mut universe).expect("case setup"),
            ReplayStep::Continue
        );
    }

    let mut observations = ObservationRecorder::default();
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("uppercase scans its balanced text"),
        ReplayStep::Continue
    );
    // TeX82 §1288 ends `shift_case` with §323's `back_list`, so the last
    // thing the scanning step commits is a backup-class input push, after
    // the completed `scan_toks` collection.
    assert!(matches!(
        observations.0.iter().rev().nth(1),
        Some(CommandObservation::TokenList(record))
            if record.transition == "complete" && record.purpose == "scan_toks"
    ));
    assert!(matches!(
        observations.0.last(),
        Some(CommandObservation::Input(push))
            if push.transition == InputTransition::Push
                && push.reason == InputReason::Backup
    ));
    assert_eq!(
        control
            .step(&mut universe)
            .expect("uppercase replays definition"),
        ReplayStep::Continue
    );

    observations.0.clear();
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("lowercase follows the retired uppercase replay"),
        ReplayStep::Continue
    );
    assert!(matches!(
        observations.0.as_slice(),
        [CommandObservation::Input(retirement), ..]
            if retirement.transition == InputTransition::Retire
                && retirement.reason == InputReason::Backup
    ));
    assert_eq!(
        control
            .step(&mut universe)
            .expect("lowercase replays definition"),
        ReplayStep::Continue
    );
    assert_eq!(
        control
            .step(&mut universe)
            .expect("zero-code uppercase scans"),
        ReplayStep::Continue
    );
    assert_eq!(
        control
            .step(&mut universe)
            .expect("zero-code definition replays"),
        ReplayStep::Continue
    );
    assert_eq!(
        control
            .step(&mut universe)
            .expect("zero-code replay retires before the next command"),
        ReplayStep::Continue
    );

    let relax = universe
        .symbol("relax")
        .expect("primitive is interned")
        .symbol();
    let up = universe
        .macro_meaning(universe.symbol("up").expect("uppercase macro"))
        .expect("uppercase definition persists");
    assert_eq!(
        universe.tokens(up.replacement_text()),
        &[
            Token::Char {
                ch: 'Z',
                cat: Catcode::Other,
            },
            Token::Cs(relax),
        ]
    );
    let down = universe
        .macro_meaning(universe.symbol("down").expect("lowercase macro"))
        .expect("lowercase definition persists");
    assert_eq!(
        universe.tokens(down.replacement_text()),
        &[
            Token::Char {
                ch: 'y',
                cat: Catcode::Other,
            },
            Token::Cs(relax),
        ]
    );
    while matches!(
        control.step(&mut universe).expect("finish case replay"),
        ReplayStep::Continue
    ) {}
    let zero = universe
        .macro_meaning(universe.symbol("zero").expect("zero-code macro"))
        .unwrap_or_else(|| {
            panic!(
                "zero-code definition persists: {}",
                terminal_text(&universe)
            )
        });
    assert_eq!(
        universe.tokens(zero.replacement_text()),
        &[Token::Char {
            ch: '@',
            cat: Catcode::Other,
        }]
    );
}

#[test]
fn canonical_initex_replays_afterassignment_before_fifo_aftergroup_tokens() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\def\first{\global\count0=1}\def\second{\global\count0=2}\def\assigned{\global\count2=\count1}{\aftergroup\first\aftergroup\second\afterassignment\assigned\count1=7}\end",
    );

    loop {
        if matches!(
            control
                .step(&mut universe)
                .expect("canonical after-token replay"),
            ReplayStep::End
        ) {
            break;
        }
    }
    assert_eq!(
        universe.count(0),
        2,
        "aftergroup tokens replay FIFO after restoration"
    );
    assert_eq!(
        universe.count(2),
        7,
        "afterassignment observes the committed assignment"
    );
    assert_eq!(
        universe.count(1),
        0,
        "the local assignment restores before group exit"
    );
}

#[test]
fn canonical_initex_replay_futurelet_preserves_lookahead_order_after_assignment() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\futurelet\next\first x\end");
    let mut observations = ObservationRecorder::default();

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("futurelet assignment"),
        ReplayStep::Continue
    );
    let next = universe
        .symbol("next")
        .expect("futurelet target is interned");
    assert_eq!(
        universe.meaning(next),
        Meaning::CharToken {
            ch: 'x',
            cat: tex_state::token::Catcode::Letter,
        }
    );
    assert!(matches!(
        observations.0.last(),
        Some(CommandObservation::Mutation(mutation))
            if mutation.target == "meaning" && mutation.key.as_deref() == Some("next")
    ));

    observations.0.clear();
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("first lookahead replay"),
        ReplayStep::Continue
    );
    assert!(matches!(
        observations.0.as_slice(),
        [.., CommandObservation::Command(delivery)]
            if matches!(delivery.spelling, ObservedToken::ControlSequence(ref name) if name == "first")
    ));
    observations.0.clear();
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("second lookahead replay"),
        ReplayStep::Continue
    );
    assert!(observations.0.iter().any(|observation| {
        matches!(
            observation,
            CommandObservation::Command(delivery)
                if matches!(delivery.spelling, ObservedToken::Character { character: 'x', .. })
        )
    }));
    assert_eq!(control.current_mode(), crate::Mode::Horizontal);
    assert_eq!(
        control
            .step(&mut universe)
            .expect("paragraph character replay"),
        ReplayStep::Continue
    );
    assert_eq!(
        control
            .step(&mut universe)
            .expect("end is deferred through paragraph recovery"),
        ReplayStep::Continue
    );
    assert_eq!(
        control.step(&mut universe).expect("inserted paragraph"),
        ReplayStep::Continue
    );
    assert_end_after_ejecting_residual_page(&mut control, &mut universe);
}

#[test]
fn canonical_initex_replay_scans_and_applies_dimension_and_glue_registers() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\dimen0=1.5pt\skip0=2pt plus 3fil minus 4pt\end",
    );
    let mut observations = ObservationRecorder::default();

    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("dimension assignment"),
        ReplayStep::Continue
    );
    assert_eq!(universe.dimen(0).raw(), 98_304);
    assert!(matches!(
        observations.0.as_slice(),
        [..,
            CommandObservation::Scanner(scanner),
            CommandObservation::Mutation(mutation)]
            if scanner.kind == "dimension"
                && scanner.value == "98304"
                && mutation.target == "register"
                && mutation.key.as_deref() == Some("dimen:0")
                && mutation.value == "scaled:98304"
    ));

    observations.0.clear();
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("glue assignment"),
        ReplayStep::Continue
    );
    let glue = universe.glue(universe.skip(0));
    assert_eq!(glue.width.raw(), 131_072);
    assert_eq!(glue.stretch.raw(), 196_608);
    assert_eq!(glue.stretch_order, tex_state::glue::Order::Fil);
    assert_eq!(glue.shrink.raw(), 262_144);
    assert!(matches!(
        observations.0.as_slice(),
        [..,
            CommandObservation::Scanner(scanner),
            CommandObservation::Mutation(mutation)]
            if scanner.kind == "glue"
                && mutation.target == "register"
                && mutation.key.as_deref() == Some("skip:0")
                && mutation.value.starts_with("glue:width=131072;")
    ));
    assert_eq!(control.step(&mut universe).expect("end"), ReplayStep::End);
}

/// Steps once and returns everything the step committed.
fn step_observations(
    control: &mut CommandReplayControl,
    universe: &mut Universe,
    what: &str,
) -> Vec<CommandObservation> {
    let mut observations = ObservationRecorder::default();
    assert_eq!(
        control
            .step_with_observer(universe, &mut observations)
            .unwrap_or_else(|error| panic!("{what}: {error:?}")),
        ReplayStep::Continue
    );
    observations.0
}

fn scanned_any(observations: &[CommandObservation]) -> bool {
    observations
        .iter()
        .any(|observation| matches!(observation, CommandObservation::Scanner(_)))
}

fn scanned(observations: &[CommandObservation], kind: &str, value: &str) -> bool {
    observations.iter().any(|observation| {
        matches!(
            observation,
            CommandObservation::Scanner(scanner)
                if scanner.kind == kind && scanner.value == value
        )
    })
}

/// Asserts the two records TeX82 §1090's `back_input` commits before §1091
/// `new_graf` runs: §325 pushes a one-token `backed_up` level and records the
/// token it holds.
fn assert_backed_up_paragraph_start(observations: &[CommandObservation], what: &str) {
    assert!(
        observations.iter().any(|observation| matches!(
            observation,
            CommandObservation::Input(record)
                if record.transition == InputTransition::Backup
                    && record.reason == InputReason::Backup
        )),
        "{what}: §1090 must push a backed_up input level, got {observations:?}"
    );
    assert!(
        observations.iter().any(|observation| matches!(
            observation,
            CommandObservation::Recovery(record) if record.kind == RecoveryKind::Backup
        )),
        "{what}: §1090's backup must be recorded, got {observations:?}"
    );
    assert!(
        !scanned_any(observations),
        "{what}: §1090 scans no operand before `new_graf`, got {observations:?}"
    );
}

// TeX82 §1090's `vmode+vrule` is one member of the shared
// `begin back_input; new_graf(true); end` case, so the first main-control step
// for a vertical-mode `\vrule` scans no operand at all: it pushes the token
// back and opens the paragraph. The rule specification is scanned only when
// the backed-up `\vrule` is redelivered in horizontal mode.
#[test]
fn canonical_initex_replay_scans_complete_rule_specs_through_command_control() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\vrule width1pt height2pt depth0pt\hrule width3pt height4pt depth1pt\end",
    );

    let start = step_observations(
        &mut control,
        &mut universe,
        "vertical rule starts a paragraph",
    );
    assert_backed_up_paragraph_start(&start, "vmode+vrule");

    let spec = step_observations(&mut control, &mut universe, "redelivered vertical rule");
    assert!(scanned(&spec, "dimension", "65536"), "{spec:?}");
    assert!(
        spec.iter().any(|observation| matches!(
            observation,
            CommandObservation::Command(delivery)
                if matches!(delivery.spelling, ObservedToken::Character { character: 'w', .. })
        )),
        "{spec:?}"
    );

    // §1095 `head_for_vmode` ends the paragraph and retries `\hrule` in
    // vertical mode, so the horizontal rule's own spec is scanned a few steps
    // later rather than immediately.
    let mut horizontal = Vec::new();
    for _ in 0..8 {
        horizontal = step_observations(&mut control, &mut universe, "horizontal rule");
        if scanned(&horizontal, "dimension", "196608") {
            break;
        }
    }
    assert!(
        scanned(&horizontal, "dimension", "196608"),
        "{horizontal:?}"
    );
}

#[test]
fn outer_vertical_rule_waits_for_the_next_page_builder_command() {
    // TeX82 §1056's append_rule contributes the rule and resets prev_depth,
    // but has no build_page tail. The backed-up \par that terminated the rule
    // specification is therefore delivered first; §1096's vertical-par arm
    // owns the subsequent page-builder visit.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\hrule height2pt\par\end");

    control
        .step(&mut universe)
        .expect("vertical rule is contributed");
    assert!(matches!(
        universe.page_contribution_front(),
        Some(Node::Rule {
            height: Some(height),
            ..
        }) if *height == Scaled::from_raw(131_072)
    ));
    assert_eq!(
        universe.current_page_len(),
        0,
        "append_rule itself does not visit the page builder"
    );

    control
        .step(&mut universe)
        .expect("backed-up par visits the page builder");
    assert_eq!(universe.page_contribution_front(), None);
    assert!(
        universe.current_page_len() > 0,
        "the following vertical par owns §1096's build_page call"
    );
}

#[test]
fn outer_vertical_penalty_still_visits_page_builder_after_a_rule() {
    // Negative control: §1056 changes only append_rule. §1103's append_penalty
    // retains its explicit `if mode=vmode then build_page` tail.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\hrule height2pt\penalty0\end");

    control.step(&mut universe).expect("vertical rule");
    assert!(universe.page_contribution_front().is_some());
    control
        .step(&mut universe)
        .expect("vertical penalty visits page builder");
    assert_eq!(universe.page_contribution_front(), None);
}

// The rest of §1090's shared vertical-mode case, each checked the same way:
// the delivering step commits the backup and scans nothing, and the operand is
// scanned only after the backed-up token is redelivered in horizontal mode.
// `\char` is the case the canonical gentle trace caught: its `scan_int` ran in
// vertical mode, so the character number was read before `\everypar`.
#[test]
fn canonical_vertical_char_num_backs_up_before_scanning_its_number() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\char92 \end");

    let start = step_observations(&mut control, &mut universe, "\\char starts a paragraph");
    assert_backed_up_paragraph_start(&start, "vmode+char_num");

    let number = step_observations(&mut control, &mut universe, "redelivered \\char");
    assert!(scanned(&number, "integer", "92"), "{number:?}");
}

#[test]
fn canonical_vertical_hskip_backs_up_before_scanning_its_glue() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\hskip 3pt\end");

    let start = step_observations(&mut control, &mut universe, "\\hskip starts a paragraph");
    assert_backed_up_paragraph_start(&start, "vmode+hskip");

    let glue = step_observations(&mut control, &mut universe, "redelivered \\hskip");
    assert!(scanned(&glue, "dimension", "196608"), "{glue:?}");
}

#[test]
fn canonical_vertical_accent_backs_up_before_scanning_its_number() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\accent23 \end");

    let start = step_observations(&mut control, &mut universe, "\\accent starts a paragraph");
    assert_backed_up_paragraph_start(&start, "vmode+accent");

    let number = step_observations(&mut control, &mut universe, "redelivered \\accent");
    assert!(scanned(&number, "integer", "23"), "{number:?}");
}

/// TeX82 §1123's `make_accent` runs §1270's `do_assignments` between
/// `scan_char_num` and §1124's base-character classification. §1270 executes
/// each assignment *in place*: §404 fetches it and `prefixed_command` runs it,
/// with no `back_input` anywhere in the loop.
///
/// The regression this pins is the extra replay round. Stopping the accent
/// scan on the assignment and backing it up pushed a `backed_up` input level,
/// committed a recovery record, delivered the assignment a second time and
/// retired the level -- five records tex.web never produces -- and, worse,
/// ended the accent scan with no base at all, so the accent was appended
/// alone and the character that follows it was typeset as an unrelated
/// letter (`umber2-johp.264`).
#[test]
fn canonical_accent_runs_do_assignments_in_place_and_keeps_its_base() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_cmr10_font(&mut control, &mut universe);
    register_source(
        &mut control,
        br"\font\f=cmr10 \f\setbox0=\hbox{\accent23 \global\count0=7 A}",
    );

    let mut observations = ObservationRecorder::default();
    loop {
        match control
            .step_with_observer(&mut universe, &mut observations)
            .expect("canonical program executes")
        {
            MainControlStep::End | MainControlStep::EndOfInput => break,
            MainControlStep::Continue => {}
        }
    }

    assert_eq!(
        universe.count(0),
        7,
        "§1270's `prefixed_command` must run the assignment the accent scan stopped on"
    );

    let stored = universe
        .box_reg(0)
        .and_then(|id| universe.nodes(id).first().map(|node| node.to_owned()))
        .expect("setbox0 stores an hbox");
    let Node::HList(stored) = stored else {
        panic!("setbox0 contains an hbox");
    };
    // §1123's `link(tail):=p` after §1125's two `acc_kern`s: the accent (in
    // its own shifted box, because its x-height differs from the base's
    // height), then the base character. A dropped base would leave the accent
    // alone, with no kerns and no `A` attached to it.
    let children: Vec<Node> = universe
        .nodes(stored.children)
        .iter()
        .map(|node| node.to_owned())
        .collect();
    let [
        Node::Kern {
            kind: KernKind::Accent,
            ..
        },
        accent,
        Node::Kern {
            kind: KernKind::Accent,
            ..
        },
        Node::Char { ch: 'A', .. },
    ] = children.as_slice()
    else {
        panic!("§1124 must read its base character from after `do_assignments`: {children:?}")
    };
    let accent_children = match accent {
        Node::Char { ch, .. } => vec![*ch],
        Node::HList(boxed) => universe
            .nodes(boxed.children)
            .iter()
            .filter_map(|node| match node.to_owned() {
                Node::Char { ch, .. } => Some(ch),
                _ => None,
            })
            .collect(),
        other => panic!("the accent is a character or a shifted box holding one: {other:?}"),
    };
    assert_eq!(accent_children, vec![char::from(23u8)]);

    // The rest of the source backs up freely -- §415's font identifier,
    // §404's optional `=`, `scan_left_brace` -- so the proof is specific:
    // §1270 never replays the assignment it stopped on, so neither prefix nor
    // register token can appear in a `backed_up` level's recovery record.
    let replayed_assignment = observations.0.iter().find(|observation| {
        matches!(
            observation,
            CommandObservation::Recovery(record)
                if record.kind == RecoveryKind::Backup
                    && record.tokens.iter().any(|token| matches!(
                        token,
                        ObservedToken::ControlSequence(name)
                            if name == "global" || name == "count"
                    ))
        )
    });
    assert!(
        replayed_assignment.is_none(),
        "§1270 runs each assignment in place: {replayed_assignment:?}"
    );
}

// `\hfil`, `\hfill`, `\hss`, and `\hfilneg` share tex.web's `hskip` command
// code (§1058), and `\ ` is `ex_space` (§265); neither scans an operand, so
// the proof for these is the backup itself.
#[test]
fn canonical_vertical_hfil_and_control_space_back_up_before_new_graf() {
    for source in [&b"\\hfil\\end"[..], &b"\\ \\end"[..]] {
        let mut universe = Universe::new_with_plain_catcodes();
        let mut control = CommandReplayControl::tex82_initex(&mut universe);
        register_source(&mut control, source);

        let start = step_observations(&mut control, &mut universe, "paragraph start");
        assert_backed_up_paragraph_start(&start, &String::from_utf8_lossy(source));
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
fn replay_expands_registered_input_without_executor_source_consumption() {
    let mut universe = Universe::new_with_plain_catcodes();
    crate::install_unexpandable_primitives(&mut universe);
    install_input(&mut universe);
    let mut control = CommandReplayControl::default();
    control.capabilities_mut().register_input(
        "child",
        SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(&b"\\count3=9"[..]),
        ),
    );
    register_source(&mut control, br"\input child\count4=8\end");

    assert_eq!(
        control.step(&mut universe).expect("nested assignment"),
        ReplayStep::Continue
    );
    assert_eq!(universe.count(3), 9);
    assert_eq!(
        control.step(&mut universe).expect("parent assignment"),
        ReplayStep::Continue
    );
    assert_eq!(universe.count(4), 8);
    assert_eq!(control.step(&mut universe).expect("end"), ReplayStep::End);
}

#[test]
fn replay_command_snapshot_restores_typed_scanner_input_deterministically() {
    let mut universe = Universe::new_with_plain_catcodes();
    crate::install_unexpandable_primitives(&mut universe);
    let mut control = CommandReplayControl::default();
    register_source(&mut control, br"\count12=7\end");
    let snapshot = control.command_mut().snapshot();

    assert_eq!(
        control.step(&mut universe).expect("first assignment"),
        ReplayStep::Continue
    );
    assert_eq!(universe.count(12), 7);

    control
        .command_mut()
        .rollback(snapshot)
        .expect("command snapshot restores scanner-owned input");
    let mut replayed_universe = Universe::new_with_plain_catcodes();
    crate::install_unexpandable_primitives(&mut replayed_universe);
    assert_eq!(
        control
            .step(&mut replayed_universe)
            .expect("replayed assignment"),
        ReplayStep::Continue
    );
    assert_eq!(replayed_universe.count(12), 7);
    assert_eq!(
        control.step(&mut replayed_universe).expect("end"),
        ReplayStep::End
    );
}

#[test]
fn replay_dispatches_modes_effects_and_typed_alignment_lifecycle() {
    let mut universe = Universe::new_with_plain_catcodes();
    crate::install_unexpandable_primitives(&mut universe);
    let mut control = CommandReplayControl::default();
    register_source(&mut control, br"a$ $\par\message{ok}\halign&\end");

    assert_eq!(
        control.step(&mut universe).expect("character"),
        ReplayStep::Continue
    );
    assert_eq!(control.current_mode(), crate::Mode::Horizontal);
    assert_eq!(
        control.step(&mut universe).expect("backed-up character"),
        ReplayStep::Continue
    );
    assert_eq!(control.current_mode(), crate::Mode::Horizontal);
    assert_eq!(
        control.step(&mut universe).expect("math start"),
        ReplayStep::Continue
    );
    assert_eq!(control.current_mode(), crate::Mode::Math);
    assert_eq!(
        control.step(&mut universe).expect("math space"),
        ReplayStep::Continue
    );
    assert_eq!(
        control.step(&mut universe).expect("math end"),
        ReplayStep::Continue
    );
    assert_eq!(control.current_mode(), crate::Mode::Horizontal);
    assert_eq!(
        control.step(&mut universe).expect("paragraph"),
        ReplayStep::Continue
    );
    assert_eq!(control.current_mode(), crate::Mode::Vertical);
    assert_eq!(
        control.step(&mut universe).expect("message"),
        ReplayStep::Continue
    );
    assert!(matches!(
        universe.world().effect_records(),
        [tex_state::EffectRecord::StreamWrite { text, .. }] if text == "ok"
    ));
    assert_eq!(
        control.step(&mut universe).expect("alignment"),
        ReplayStep::Continue
    );
    let alignment = control
        .active_alignment()
        .expect("typed alignment identity");
    control
        .apply_alignment_request(AlignmentRequest::Preamble(alignment))
        .expect("preamble lifecycle");
    control
        .apply_alignment_request(AlignmentRequest::BeginCell {
            alignment,
            templates: AlignmentCellTemplates {
                u_template: None,
                v_template: TracedTokenList::synthetic(universe.intern_token_list(&[])),
            },
        })
        .expect("cell lifecycle");
    control
        .apply_alignment_request(AlignmentRequest::InstallCellTemplate(alignment))
        .expect("cell template lifecycle");
    assert_eq!(
        control
            .alignment_step(alignment, &mut universe)
            .expect("command processor intercepts the cell delimiter"),
        ReplayStep::Continue
    );
    assert_eq!(
        control
            .step(&mut universe)
            .expect("expanded end-v finishes the cell through typed command control"),
        ReplayStep::Continue
    );
    control
        .apply_alignment_request(AlignmentRequest::Finish(alignment))
        .expect("alignment lifecycle finishes through command core");
    assert_eq!(control.active_alignment(), None);
    // TeX82 §1051's `privileged`: the alignment left this replay in internal
    // vertical mode, where `\end` reports an illegal case instead of ending
    // the job, so the run finishes by running out of input.
    assert_eq!(
        control
            .step(&mut universe)
            .expect("stop is not privileged in internal vertical mode"),
        ReplayStep::Continue
    );
    assert!(terminal_text(&universe).contains("You can't use `\\end'"));
    assert_eq!(
        control
            .step(&mut universe)
            .expect("input exhausted after end"),
        ReplayStep::EndOfInput
    );
}

#[test]
fn command_owned_endv_finishes_cell_and_publishes_retirement_in_canonical_order() {
    let mut universe = Universe::new_with_plain_catcodes();
    crate::install_unexpandable_primitives(&mut universe);
    let mut control = CommandReplayControl::default();
    register_source(&mut control, br"\halign&\end");

    assert_eq!(
        control.step(&mut universe).expect("alignment"),
        ReplayStep::Continue
    );
    let alignment = control.active_alignment().expect("active alignment");
    for request in [
        AlignmentRequest::Preamble(alignment),
        AlignmentRequest::BeginCell {
            alignment,
            templates: AlignmentCellTemplates {
                u_template: None,
                v_template: TracedTokenList::synthetic(universe.intern_token_list(&[])),
            },
        },
        AlignmentRequest::InstallCellTemplate(alignment),
    ] {
        control
            .apply_alignment_request(request)
            .expect("cell lifecycle setup");
    }
    assert_eq!(
        control.step(&mut universe).expect("intercepted delimiter"),
        ReplayStep::Continue
    );

    let mut observations = ObservationRecorder::default();
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("command-owned end-v"),
        ReplayStep::Continue
    );
    // TeX82 §1131's `do_endv` inspects the input stack and pops nothing, so
    // this step ends at §791 `fin_col`'s `align_state:=1000000`.
    assert!(
        observations.0.windows(3).any(|events| {
            matches!(
                events,
                [
                    CommandObservation::Command(raw),
                    CommandObservation::Command(expanded),
                    CommandObservation::Alignment(state_change),
                ] if raw.command == "end_template"
                    && expanded.command == "endv"
                    && state_change.transition == "state_change"
                    && state_change.align_state == 1_000_000
            )
        }),
        "unexpected observations: {:?}",
        observations.0
    );
    assert!(
        !observations.0.iter().any(|observation| matches!(
            observation,
            CommandObservation::Input(retirement)
                if retirement.transition == InputTransition::Retire
                    && retirement.reason == InputReason::AlignmentVTemplate
        )),
        "do_endv must not retire the v-template: {:?}",
        observations.0
    );

    control
        .apply_alignment_request(AlignmentRequest::Finish(alignment))
        .expect("alignment lifecycle finishes through command core");
    let after_endv = observations.0.len();
    // TeX82 §1051's `privileged`: this replay ends inside the alignment's
    // internal vertical mode, so `\end` reports an illegal case rather than
    // ending the job. Reaching it needs a token, and TeX82 §357's
    // `end_token_list` retires the depleted v-template on the way.
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("stop is not privileged in internal vertical mode"),
        ReplayStep::Continue
    );
    assert!(
        observations.0[after_endv..].windows(2).any(|events| {
            matches!(
                events,
                [
                    CommandObservation::Input(retirement),
                    CommandObservation::Alignment(template_retire),
                ] if retirement.transition == InputTransition::Retire
                    && retirement.reason == InputReason::AlignmentVTemplate
                    && template_retire.transition == "v_template_retire"
                    && template_retire.align_state == 1_000_000
            )
        }),
        "unexpected observations: {:?}",
        &observations.0[after_endv..]
    );
    assert!(terminal_text(&universe).contains("You can't use `\\end'"));
}

#[test]
fn nested_alignment_begin_suspends_the_outer_replay_context() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CommandReplayControl::tex82_initex(&mut universe);
    let outer = AlignmentIdentity::new(1);
    control
        .command
        .apply_alignment_request(AlignmentRequest::Begin(outer))
        .expect("outer alignment begins");
    control.active_alignment = Some(ActiveReplayAlignment {
        identity: outer,
        kind: AlignmentKind::HAlign,
        packing: crate::mode::AlignmentPackSpec::Natural,
        columns: Vec::new(),
        repeat_start: None,
        column: 0,
        preamble_opening_pending: false,
        preamble_start_pending: false,
        cell_opening_pending: false,
        next_cell_opening_pending: false,
        align_peek_pending: false,
        align_peek_after_noalign: false,
        noalign_depth: None,
        captured_rows: Vec::new(),
        tabskips: vec![universe.glue_param(GlueParam::TAB_SKIP)],
        default_tabskip: universe.glue_param(GlueParam::TAB_SKIP),
        row_migrations: Vec::new(),
        cell_span: 1,
        row_open: false,
        cell_open: false,
    });
    control.next_alignment_identity = 2;

    apply_scanned_step(
        ScannedStep::BeginAlignment { vertical: false },
        &mut universe,
        &mut control.modes,
        &mut control.next_alignment_identity,
        &mut control.active_alignment,
        &mut CommandMachine {
            state: &mut control.command,
            runtime: &mut control.runtime,
            capabilities: &mut control.capabilities,
            observations: &mut control.operation_observations,
            initex: control.initex,
        },
        &mut control.boxes,
        &mut control.prepared_dvi_pages,
    )
    .expect("nested alignment begins through typed suspension");

    assert_eq!(control.boxes.suspended_alignments.len(), 1);
    let inner = control
        .active_alignment()
        .expect("inner alignment is active");
    assert_ne!(inner, outer);
    // This focused lifecycle test drives `BeginAlignment` straight to
    // `AlignmentFinish` without replaying a preamble, so it must stand in for
    // the two save levels TeX82 §774's `init_align` would have opened -- §645's
    // `scan_spec(align_group,false)` for the alignment and the explicit
    // `new_save_level(align_group)` for its first entry -- which §800's
    // `fin_align` removes with its two `unsave`s.
    universe.enter_group_with_kind(tex_state::GroupKind::Align);
    universe.enter_group_with_kind(tex_state::GroupKind::Align);
    apply_scanned_step(
        ScannedStep::AlignmentFinish { alignment: inner },
        &mut universe,
        &mut control.modes,
        &mut control.next_alignment_identity,
        &mut control.active_alignment,
        &mut CommandMachine {
            state: &mut control.command,
            runtime: &mut control.runtime,
            capabilities: &mut control.capabilities,
            observations: &mut control.operation_observations,
            initex: control.initex,
        },
        &mut control.boxes,
        &mut control.prepared_dvi_pages,
    )
    .expect("right-brace align_peek finish resumes the outer context");
    assert_eq!(control.active_alignment(), Some(outer));
    assert_eq!(control.boxes.suspended_alignments.len(), 0);
}

#[test]
fn canonical_alignment_captures_completed_cell_material_before_fin_align() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\halign{#\cr\kern1pt\cr\kern2pt\cr}\end");

    for _ in 0..32 {
        control
            .step(&mut universe)
            .expect("canonical alignment structural step");
        let Some(active) = control.active_alignment.as_ref() else {
            continue;
        };
        if active
            .captured_rows
            .first()
            .is_some_and(|row| !row.is_empty())
        {
            assert!(
                !universe.nodes(active.captured_rows[0][0]).is_empty(),
                "the first completed cell is frozen before final alignment packaging"
            );
            assert_eq!(control.current_mode(), Mode::InternalVertical);
            return;
        }
    }
    panic!("canonical alignment did not capture its first completed row");
}

#[test]
fn canonical_alignment_finalizes_rows_into_the_enclosing_list() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\halign{#&#\cr\kern1pt&\kern2pt\cr}");

    run_to_end(&mut control, &mut universe);

    // TeX82 §800's `fin_align` pops the alignment's nest level and returns
    // to whatever mode enclosed it (§812's "Insert the current list into
    // its environment"); a top-level `\halign` therefore leaves outer
    // vertical mode exactly as `new_graf` never ran. The finished row
    // becomes a page contribution rather than a `current_list` node because
    // outer-vertical material is routed through the page builder (TeX82
    // §994's `build_page`), not retained on the enclosing nest level.
    assert_eq!(control.current_mode(), Mode::Vertical);
    let nodes = universe.current_page_nodes();
    assert!(
        nodes.iter().any(|node| matches!(node, Node::HList(_))),
        "nodes: {nodes:?}; terminal: {}",
        terminal_text(&universe)
    );
}

#[test]
fn canonical_alignment_entries_are_save_levels() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    // TeX82 §774 opens two `align_group` save levels -- §645's
    // `scan_spec(align_group,false)` for the alignment and an explicit
    // `new_save_level(align_group)` for its first entry -- §791's `fin_col`
    // replaces the entry level at every `&` and `\cr`, and §800's `fin_align`
    // removes both. A local assignment made in one cell therefore reaches
    // neither the next cell, nor the next row, nor anything after the
    // alignment.
    register_source(
        &mut control,
        br"\halign{#&#\cr\count0=1 \global\count3=\count0 &\global\count4=\count0 \cr\global\count5=\count0 &\cr}\global\count6=\count0 \end",
    );

    run_to_end(&mut control, &mut universe);

    assert_eq!(universe.count(3), 1, "the assigning cell still sees it");
    assert_eq!(universe.count(4), 0, "§791 unsaves at the alignment tab");
    assert_eq!(universe.count(5), 0, "§791 unsaves at the carriage return");
    assert_eq!(universe.count(6), 0, "§800 unsaves both align levels");
    assert_eq!(universe.count(0), 0);
}

#[test]
fn scanner_backed_endv_retires_before_an_omit_template() {
    let mut universe = Universe::new_with_plain_catcodes();
    crate::install_unexpandable_primitives(&mut universe);
    let mut control = CommandReplayControl::default();
    // `scan_rule_spec` reads the alignment delimiter while scanning the
    // omitted cell's rule. Its scalar retry backs up the effective `endv`,
    // reproducing TeX82 §772's exhausted backup above `omit_template`.
    register_source(
        &mut control,
        br"\halign{#&#\cr\omit\vrule width3pt&\relax\cr}\end",
    );
    let mut observations = ObservationRecorder::default();

    for _ in 0..48 {
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("scanner-backed end-v completion");
        if observations.0.windows(6).any(|events| {
            matches!(
                events,
                [
                    CommandObservation::Command(raw),
                    CommandObservation::Command(expanded),
                    CommandObservation::Alignment(state_change),
                    CommandObservation::Input(backup_retirement),
                    CommandObservation::Input(template_retirement),
                    CommandObservation::Alignment(template),
                ] if raw.command == "endv"
                    && expanded.command == "endv"
                    && state_change.transition == "state_change"
                    && backup_retirement.transition == InputTransition::Retire
                    && backup_retirement.reason == InputReason::Backup
                    && template_retirement.transition == InputTransition::Retire
                    && template_retirement.reason == InputReason::AlignmentVTemplate
                    && template.transition == "omit_template_retire"
            )
        }) {
            return;
        }
    }
    panic!(
        "scanner-backed end-v must retire backup then omit-template: {:?}",
        observations.0
    );
}

#[test]
fn paragraph_start_backs_up_the_triggering_macro_parameter_before_replay() {
    let mut universe = Universe::new_with_plain_catcodes();
    crate::install_unexpandable_primitives(&mut universe);
    let mut control = CommandReplayControl::default();
    register_source(&mut control, br"\def\pair#1#2{#2#1}\pair AB\end");

    assert_eq!(
        control.step(&mut universe).expect("definition"),
        ReplayStep::Continue
    );

    let mut observations = ObservationRecorder::default();
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("paragraph start"),
        ReplayStep::Continue
    );
    assert_eq!(control.current_mode(), crate::Mode::Horizontal);
    assert!(observations.0.windows(2).any(|pair| {
        matches!(
            &pair[0],
            CommandObservation::Input(input)
                if input.transition == InputTransition::Backup && input.reason == InputReason::Backup
        ) && matches!(
            &pair[1],
            CommandObservation::Recovery(recovery)
                if recovery.kind == RecoveryKind::Backup
                    && matches!(
                        recovery.tokens.as_slice(),
                        [ObservedToken::Character { character: 'B', .. }]
                    )
        )
    }));

    observations.0.clear();
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("backed-up character replay"),
        ReplayStep::Continue
    );
    observations.0.clear();
    assert_eq!(
        control
            .step_with_observer(&mut universe, &mut observations)
            .expect("following macro parameter"),
        ReplayStep::Continue
    );
    assert!(observations.0.iter().any(|observation| {
        matches!(
            observation,
            CommandObservation::Input(input)
                if input.transition == InputTransition::Retire && input.reason == InputReason::Backup
        )
    }));
    assert!(observations.0.iter().any(|observation| {
        matches!(
            observation,
            CommandObservation::Command(delivery)
                if matches!(
                    delivery.spelling,
                    ObservedToken::Character { character: 'A', .. }
                )
        )
    }));
}

#[test]
fn missing_canonical_input_rolls_back_the_whole_step_and_retries_fresh() {
    let child = SourceRegistration::new(
        RegisteredSourceKind::Generated,
        Arc::<[u8]>::from(&b"\\count3=9"[..]),
    );

    let mut failed_universe = Universe::new_with_plain_catcodes();
    crate::install_unexpandable_primitives(&mut failed_universe);
    install_input(&mut failed_universe);
    let mut failed = CommandReplayControl::default();
    register_source(&mut failed, br"\input child\end");
    let mut failed_observations = ObservationRecorder::default();

    assert!(matches!(
        failed
            .advance_with_observer(&mut failed_universe, &mut failed_observations)
            .expect("missing input suspends"),
        CanonicalStepResult::Suspended(CanonicalResourceNeed::Input { name }) if name == "child"
    ));
    assert_eq!(failed_universe.count(3), 0);
    assert_eq!(failed.current_mode(), Mode::Vertical);
    assert!(
        failed_observations.0.is_empty(),
        "failed delivery leaked observation"
    );

    failed
        .capabilities_mut()
        .register_input("child", child.clone());
    assert_eq!(
        failed
            .advance_with_observer(&mut failed_universe, &mut failed_observations)
            .expect("retry succeeds"),
        CanonicalStepResult::Progress(ReplayStep::Continue)
    );

    let mut fresh_universe = Universe::new_with_plain_catcodes();
    crate::install_unexpandable_primitives(&mut fresh_universe);
    install_input(&mut fresh_universe);
    let mut fresh = CommandReplayControl::default();
    register_source(&mut fresh, br"\input child\end");
    fresh.capabilities_mut().register_input("child", child);
    let mut fresh_observations = ObservationRecorder::default();
    assert_eq!(
        fresh
            .advance_with_observer(&mut fresh_universe, &mut fresh_observations)
            .expect("fresh input succeeds"),
        CanonicalStepResult::Progress(ReplayStep::Continue)
    );

    assert_eq!(failed_universe.count(3), fresh_universe.count(3));
    assert_eq!(failed.current_mode(), fresh.current_mode());
    assert_eq!(failed_observations.0, fresh_observations.0);
}

#[test]
fn macro_retry_rolls_back_command_and_provenance_as_one_timeline() {
    let child = SourceRegistration::new(
        RegisteredSourceKind::Generated,
        Arc::<[u8]>::from(&b"\\global\\advance\\count3 by1"[..]),
    );
    let mut universe = Universe::new_with_plain_catcodes();
    crate::install_unexpandable_primitives(&mut universe);
    install_input(&mut universe);
    let mut control = CommandReplayControl::default();
    register_source(
        &mut control,
        br"\def\outer#1{\inner{#1}}\def\inner#1{\input #1}\outer{child}\outer{child}\end",
    );

    for label in ["outer definition", "inner definition"] {
        assert_eq!(
            control.advance(&mut universe).expect(label),
            CanonicalStepResult::Progress(ReplayStep::Continue)
        );
    }
    let baseline = universe.provenance_stats();
    let mut retained_after_first_retry = None;
    for retry in 0..32 {
        assert!(matches!(
            control.advance(&mut universe).expect("missing nested input"),
            CanonicalStepResult::Suspended(CanonicalResourceNeed::Input { name })
                if name == "child"
        ));
        assert_eq!(
            universe.provenance_stats(),
            baseline,
            "retry {retry} restores every live provenance watermark"
        );
        assert!(
            universe.macro_invocation_origins_for_testing().is_empty(),
            "retry {retry} leaves no invocation record from either nested call"
        );
        let retained = universe.provenance_stats().retained_bytes();
        if let Some(first) = retained_after_first_retry {
            assert_eq!(
                retained, first,
                "rollback reuses bounded arena capacity instead of retaining retry history"
            );
        } else {
            retained_after_first_retry = Some(retained);
        }
    }

    control.capabilities_mut().register_input("child", child);
    assert_eq!(
        control
            .advance(&mut universe)
            .expect("nested retry commits"),
        CanonicalStepResult::Progress(ReplayStep::Continue)
    );
    assert_eq!(
        universe.count(3),
        1,
        "the shared argument range names child"
    );
    let first_commit = universe.macro_invocation_origins_for_testing();
    assert_eq!(first_commit.len(), 4);
    let parents: Vec<_> = first_commit
        .iter()
        .map(|origin| match universe.origin(*origin) {
            OriginRecord::MacroInvocation(invocation) => invocation.parent_invocation(),
            _ => panic!("enumerated invocation origin has an invocation record"),
        })
        .collect();
    assert_eq!(parents[0], tex_state::token::OriginId::UNKNOWN);
    assert!(
        parents
            .iter()
            .all(|parent| *parent == tex_state::token::OriginId::UNKNOWN),
        "the exhausted outer activation retires before the callee is invoked"
    );

    let empty = universe.intern_token_list(&[]);
    let checkpoint_definition =
        universe.intern_macro(MacroMeaning::new(MeaningFlags::EMPTY, empty, empty));
    let checkpoint = control
        .capture_checkpoint(
            crate::EngineBoundary::OuterParagraphEnd,
            &mut universe,
            crate::ExecutionBudgetCounters::default(),
        )
        .expect("quiescent command state serializes into a named checkpoint");
    let added_outer = universe.macro_invocation_origin(
        checkpoint_definition,
        tex_state::token::OriginId::UNKNOWN,
        tex_state::token::OriginId::UNKNOWN,
        tex_state::token::OriginId::UNKNOWN,
    );
    universe.macro_invocation_origin(
        checkpoint_definition,
        tex_state::token::OriginId::UNKNOWN,
        tex_state::token::OriginId::UNKNOWN,
        added_outer,
    );
    let second_commit = universe.macro_invocation_origins_for_testing();
    assert_eq!(second_commit.len(), 6);

    control
        .restore_checkpoint(&checkpoint, &mut universe)
        .expect("named checkpoint restores command and aggregate provenance");
    assert_eq!(universe.count(3), 1);
    assert_eq!(
        universe.macro_invocation_origins_for_testing(),
        first_commit,
        "snapshot restoration preserves committed origin identity"
    );
    for stale in &second_commit[4..] {
        assert!(
            universe.origin_if_live(*stale).is_none(),
            "rolled-back origin identities never alias retained records"
        );
    }

    let replayed_outer = universe.macro_invocation_origin(
        checkpoint_definition,
        tex_state::token::OriginId::UNKNOWN,
        tex_state::token::OriginId::UNKNOWN,
        tex_state::token::OriginId::UNKNOWN,
    );
    universe.macro_invocation_origin(
        checkpoint_definition,
        tex_state::token::OriginId::UNKNOWN,
        tex_state::token::OriginId::UNKNOWN,
        replayed_outer,
    );
    let replayed = universe.macro_invocation_origins_for_testing();
    assert_eq!(replayed.len(), 6);
    assert_ne!(
        &replayed[4..],
        &second_commit[4..],
        "replayed allocations receive fresh non-aliasing diagnostic identities"
    );
    let OriginRecord::MacroInvocation(replayed_inner) = universe.origin(replayed[5]) else {
        panic!("replayed child invocation remains live");
    };
    assert_eq!(replayed_inner.parent_invocation(), replayed[4]);
    assert_eq!(
        universe.macro_invocation_provenance_stats().invocations(),
        6,
        "only committed invocation pairs contribute to aggregate bounds"
    );
}

#[test]
fn canonical_assignments_apply_prefix_globaldefs_registers_and_arithmetic() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"{\count0=9}\globaldefs=1{\count1=4}\globaldefs=-1{\global\count2=5}\globaldefs=0 \count3=7\advance\count3 by 5\multiply\count3 3\divide\count3 by 2 \dimen4=2pt\advance\dimen4 by 1pt \hsize=4pt\advance\hsize by 2pt \skip5=1pt\advance\skip5 by 2pt \muskip6=1mu\advance\muskip6 by 2mu \end",
    );
    loop {
        if matches!(
            control.step(&mut universe).expect("canonical assignment"),
            MainControlStep::End | MainControlStep::EndOfInput
        ) {
            break;
        }
    }
    assert_eq!(universe.count(0), 0, "local count restores");
    assert_eq!(universe.count(1), 4, "positive globaldefs forces global");
    assert_eq!(
        universe.count(2),
        0,
        "negative globaldefs suppresses global prefix"
    );
    assert_eq!(universe.count(3), 18);
    assert_eq!(universe.dimen(4), Scaled::from_raw(3 * Scaled::UNITY));
    assert_eq!(
        universe.dimen_param(DimenParam::H_SIZE),
        Scaled::from_raw(6 * Scaled::UNITY)
    );
    assert_eq!(
        universe.glue(universe.skip(5)).width,
        Scaled::from_raw(3 * Scaled::UNITY)
    );
    assert_eq!(
        universe.glue(universe.muskip(6)).width,
        Scaled::from_raw(3 * Scaled::UNITY)
    );
}

#[test]
fn canonical_box_construction_scans_specs_hooks_and_scopes_targets() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\everyhbox{\global\advance\count6 by1}\everyvbox{\global\advance\count7 by1}\begingroup\setbox0=\hbox to 10pt{}\global\setbox1=\vbox to 12pt{}\global\setbox2=\vtop spread 2pt{}\global\setbox3=\hbox{}\endgroup\end",
    );

    loop {
        match control.advance(&mut universe).expect("fresh hook retry") {
            CanonicalStepResult::Progress(MainControlStep::End | MainControlStep::EndOfInput) => {
                break;
            }
            CanonicalStepResult::Progress(MainControlStep::Continue) => {}
            CanonicalStepResult::Suspended(need) => {
                panic!("registered hook remained suspended: {need:?}")
            }
        }
    }

    assert!(universe.box_reg(0).is_none(), "local setbox restores");
    assert_eq!((universe.count(6), universe.count(7)), (2, 2));
    let vbox = universe
        .box_reg(1)
        .and_then(|id| universe.nodes(id).first().map(|node| node.to_owned()))
        .expect("global vbox survives");
    let Node::VList(vbox) = vbox else {
        panic!("setbox1 contains a vbox");
    };
    assert_eq!(
        vbox.height + vbox.depth,
        Scaled::from_raw(12 * Scaled::UNITY)
    );
    let vtop = universe
        .box_reg(2)
        .and_then(|id| universe.nodes(id).first().map(|node| node.to_owned()))
        .expect("global vtop survives");
    assert!(matches!(vtop, Node::VList(_)), "setbox2 contains a vtop");
    let natural = universe
        .box_reg(3)
        .and_then(|id| universe.nodes(id).first().map(|node| node.to_owned()))
        .expect("global natural hbox survives");
    let Node::HList(natural) = natural else {
        panic!("setbox3 contains an hbox");
    };
    assert_eq!(natural.width, Scaled::from_raw(0));

    let mut packed = Universe::new_with_plain_catcodes();
    let mut packed_control = CanonicalMainControl::tex82_initex(&mut packed);
    register_source(&mut packed_control, br"\setbox0=\hbox to 10pt{}\end");
    run_to_end(&mut packed_control, &mut packed);
    let hbox = packed
        .box_reg(0)
        .and_then(|id| packed.nodes(id).first().map(|node| node.to_owned()))
        .expect("hbox stores");
    let Node::HList(hbox) = hbox else {
        panic!("setbox0 contains an hbox");
    };
    assert_eq!(hbox.width, Scaled::from_raw(10 * Scaled::UNITY));
}

#[test]
fn canonical_box_groups_nest_recover_and_preserve_everybox_provenance() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\setbox0=\hbox{\hbox{}}");
    loop {
        if matches!(
            control
                .step(&mut universe)
                .expect("nested and recovered box program executes"),
            MainControlStep::End | MainControlStep::EndOfInput
        ) {
            break;
        }
    }

    let outer = universe
        .box_reg(0)
        .and_then(|id| universe.nodes(id).first().map(|node| node.to_owned()))
        .expect("outer hbox stores");
    let Node::HList(outer) = outer else {
        panic!("nested result is an hbox");
    };
    assert!(
        matches!(universe.nodes(outer.children).first(), Some(node) if matches!(node.to_owned(), Node::HList(_)))
    );

    let mut provenance = Universe::new_with_plain_catcodes();
    let mut provenance_control = CanonicalMainControl::tex82_initex(&mut provenance);
    register_source(
        &mut provenance_control,
        br"\everyhbox{\relax}\setbox0=\hbox{}",
    );
    let mut hook_observations = ObservationRecorder::default();
    loop {
        if matches!(
            provenance_control
                .step_with_observer(&mut provenance, &mut hook_observations)
                .expect("everyhbox program executes"),
            MainControlStep::End | MainControlStep::EndOfInput
        ) {
            break;
        }
    }
    let hook_origin = hook_observations
        .0
        .iter()
        .find_map(|event| match event {
            CommandObservation::Command(command) if command.command == "relax" => {
                Some(command.provenance.origin)
            }
            _ => None,
        })
        .expect("everyhbox relax is delivered");
    assert_ne!(hook_origin, tex_state::token::OriginId::UNKNOWN);

    let mut recovered = Universe::new_with_plain_catcodes();
    let mut recovery_control = CanonicalMainControl::tex82_initex(&mut recovered);
    register_source(&mut recovery_control, br"\setbox1=\hbox to 1pt\relax}");
    run_to_end(&mut recovery_control, &mut recovered);
    assert!(
        recovered.box_reg(1).is_some(),
        "missing brace recovers as a box group"
    );
}

#[test]
fn canonical_box_lifecycle_unboxing_and_leaders_do_not_reopen_input() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\setbox0=\hbox{A}\setbox1=\hbox{\unhcopy0}\setbox2=\hbox{\unhbox0}\setbox3=\hbox{\leaders\hbox{}\hskip10pt}\setbox4=\hbox{\cleaders\hrule\hskip10pt}\setbox5=\hbox{\xleaders\copy3\hskip10pt}\end",
    );
    run_to_end(&mut control, &mut universe);

    assert!(
        universe.box_reg(0).is_none(),
        "unhbox consumes its register"
    );
    assert!(
        universe.box_reg(1).is_some(),
        "unhcopy retains source children"
    );
    for index in [3, 4, 5] {
        let node = universe
            .box_reg(index)
            .and_then(|id| universe.nodes(id).first().map(|node| node.to_owned()))
            .expect("leader result box");
        let Node::HList(box_node) = node else {
            panic!("leader result is hbox")
        };
        assert!(
            matches!(universe.nodes(box_node.children).first(), Some(node) if matches!(node.to_owned(), Node::Glue { leader: Some(_), .. }))
        );
    }
}

#[test]
fn canonical_insert_builds_typed_ins_node_in_current_list() {
    // TeX82 §1099/§1100: `\insert<class>{...}` packages its body like a
    // natural `\vbox` and appends the resulting `ins_node` to whatever list
    // was open, not a side channel -- so it shows up as a plain child of the
    // enclosing `\vbox` here, carrying its own separate `content` list.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\setbox0=\vbox{\insert3{\hrule height5pt}}\end",
    );
    run_to_end(&mut control, &mut universe);

    let vbox = universe
        .box_reg(0)
        .and_then(|id| universe.nodes(id).first().map(|node| node.to_owned()))
        .expect("outer vbox stores");
    let Node::VList(vbox) = vbox else {
        panic!("setbox0 contains a vbox");
    };
    let children = universe.nodes(vbox.children);
    assert_eq!(children.len(), 1, "the ins_node is the vbox's only child");
    let Node::Ins {
        class,
        size,
        content,
        ..
    } = children.first().expect("checked len").to_owned()
    else {
        panic!("vbox child is an ins_node");
    };
    assert_eq!(class, 3);
    assert_eq!(size, Scaled::from_raw(5 * Scaled::UNITY));
    let inner = universe.nodes(content);
    assert_eq!(inner.len(), 1, "insertion content is the lone rule");
    assert!(matches!(
        inner.first().expect("checked len").to_owned(),
        Node::Rule {
            height: Some(height),
            depth: Some(depth),
            ..
        } if height == Scaled::from_raw(5 * Scaled::UNITY) && depth == Scaled::from_raw(0)
    ));
}

#[test]
fn canonical_insert_recovers_reserved_and_out_of_range_class_numbers() {
    // TeX82 §1099: `scan_eight_bit_int`'s own 0..=255 clamp ("Bad register
    // code") and the additional `\insert255` rejection ("box 255 is
    // special") both recover as class 0 rather than aborting the run.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\setbox0=\vbox{\insert255{}\insert1000{}}\end",
    );
    run_to_end(&mut control, &mut universe);

    let text = terminal_text(&universe);
    assert!(
        text.contains("You can't \\insert255"),
        "reserved class 255 is diagnosed: {text}"
    );
    assert!(
        text.contains("Bad register code (1000)"),
        "out-of-range class is diagnosed: {text}"
    );

    let vbox = universe
        .box_reg(0)
        .and_then(|id| universe.nodes(id).first().map(|node| node.to_owned()))
        .expect("outer vbox stores");
    let Node::VList(vbox) = vbox else {
        panic!("setbox0 contains a vbox");
    };
    let children = universe.nodes(vbox.children);
    assert_eq!(children.len(), 2);
    for child in children {
        let Node::Ins { class, .. } = child.to_owned() else {
            panic!("vbox child is an ins_node");
        };
        assert_eq!(class, 0, "both recoveries fall back to class 0");
    }
}

#[test]
fn canonical_insert_at_outer_vertical_reaches_the_page_builder() {
    // TeX82 §1099: `if nest_ptr=0 then build_page` -- an `\insert` delivered
    // directly in outer vertical mode must hand its ins_node to the page
    // builder immediately, exercising §§980--987's insertion-class
    // accounting (`tex-exec::page_builder`) rather than merely constructing
    // the typed node.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\dimen0=100pt \insert0{\hrule height5pt}");
    run_to_end(&mut control, &mut universe);

    let insertions = universe.page_insertions();
    assert_eq!(insertions.len(), 1);
    assert_eq!(insertions[0].class(), 0);
    assert_eq!(insertions[0].height(), Scaled::from_raw(5 * Scaled::UNITY));
    assert_eq!(
        insertions[0].status(),
        tex_state::page::PageInsertionStatus::Inserting
    );
}

#[test]
fn canonical_vadjust_builds_adjust_node_migrated_out_of_its_enclosing_hbox() {
    // TeX82 §1099/§1100: `\vadjust{...}` shares `\insert`'s exact
    // `begin_insert_or_adjust`/`insert_group` construction with `class`
    // fixed at 255, but closes as an `adjust_node` holding only the packed
    // content (no split parameters). §1100's own hpack (§649) then migrates
    // that `adjust_node` out of the enclosing `\hbox` when *that* box is
    // appended to the vlist -- `extract_box_migrations` unwraps it to its
    // bare content, exactly like tex.web's `Transfer node p to the
    // adjustment list` discarding the `adjust_node` wrapper itself.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\setbox0=\vbox{\hbox{\vadjust{\penalty123}}}\end",
    );
    run_to_end(&mut control, &mut universe);

    let vbox = universe
        .box_reg(0)
        .and_then(|id| universe.nodes(id).first().map(|node| node.to_owned()))
        .expect("outer vbox stores");
    let Node::VList(vbox) = vbox else {
        panic!("setbox0 contains a vbox");
    };
    let children = universe.nodes(vbox.children);
    assert_eq!(
        children.len(),
        2,
        "the migrated penalty follows the now-empty hbox: {children:?}"
    );
    let Node::HList(hbox) = children.iter().next().expect("checked len").to_owned() else {
        panic!("first child is the hbox that housed \\vadjust");
    };
    assert!(
        universe.nodes(hbox.children).is_empty(),
        "the adjust_node's content left the hbox behind"
    );
    assert_eq!(
        children.iter().nth(1).expect("checked len").to_owned(),
        Node::Penalty(123)
    );
}

#[test]
fn canonical_vadjust_splices_across_a_paragraph_line_break() {
    // TeX82 §§1355ish (`post_line_break`'s per-line `hpack` with `adjust_tail`
    // engaged): a `\vadjust` inside a paragraph line must be spliced out of
    // that line's packed hbox and appear as the line's next vlist sibling,
    // not buried inside the line's own hlist.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\setbox0=\vbox{\hsize=1000pt \parindent=0pt Hello\vadjust{\penalty456}world\par}\end",
    );
    run_to_end(&mut control, &mut universe);

    let vbox = universe
        .box_reg(0)
        .and_then(|id| universe.nodes(id).first().map(|node| node.to_owned()))
        .expect("outer vbox stores");
    let Node::VList(vbox) = vbox else {
        panic!("setbox0 contains a vbox");
    };
    let children = universe.nodes(vbox.children);
    let penalty_position = children
        .iter()
        .position(|node| matches!(node.to_owned(), Node::Penalty(456)))
        .expect("the migrated penalty appears as a direct vlist sibling");
    let line_position = children
        .iter()
        .position(|node| matches!(node.to_owned(), Node::HList(_)))
        .expect("the paragraph produced its one line");
    assert!(
        line_position < penalty_position,
        "the migrated material follows the line it came from: {children:?}"
    );
    let Node::HList(line) = children
        .iter()
        .nth(line_position)
        .expect("checked above")
        .to_owned()
    else {
        unreachable!("checked above")
    };
    assert!(
        universe
            .nodes(line.children)
            .iter()
            .all(|node| !matches!(node.to_owned(), Node::Penalty(456))),
        "the penalty left the packed line behind"
    );
}

#[test]
fn canonical_vadjust_is_forbidden_in_vertical_mode() {
    // tex.web's "Forbidden cases" list includes `vmode+vadjust`; unlike
    // `\insert` (`any_mode`), `\vadjust` directly in vertical mode never
    // reaches `scan_box_group_opening` at all.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\vadjust{\kern1pt}\end");
    run_to_end(&mut control, &mut universe);

    let text = terminal_text(&universe);
    assert!(
        text.contains("You can't use `\\vadjust' in vertical mode"),
        "{text}"
    );
}

#[test]
fn canonical_eqno_outside_math_mode_reports_illegal_case() {
    // TeX82 §1144's `@<Forbidden cases@>=non_math(eq_no)`: `\eqno`/`\leqno`
    // outside math mode take `report_illegal_case`, not §1047's
    // `insert_dollar_sign` (unlike the rest of the math-noad family sharing
    // the `eq_no` command code) -- regression test for umber2-johp.88.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\eqno\end");
    run_to_end(&mut control, &mut universe);

    let text = terminal_text(&universe);
    assert!(
        text.contains("You can't use `\\eqno' in vertical mode"),
        "{text}"
    );
}

#[test]
fn canonical_leqno_in_horizontal_mode_reports_illegal_case() {
    // `hmode+eq_no` is likewise a Forbidden case (`non_math(#)==vmode+#,
    // hmode+#`); `\leqno` shares `eq_no`'s command code with chr_code 1.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\setbox0=\hbox{\leqno}");
    run_to_end(&mut control, &mut universe);

    let text = terminal_text(&universe);
    assert!(
        text.contains("You can't use `\\leqno' in restricted horizontal mode"),
        "{text}"
    );
}

#[test]
fn canonical_eqno_inside_display_math_is_unaffected() {
    // `mmode+eq_no` (gated by `privileged`/`cur_group`, TeX82 §1140-1142)
    // must be unaffected by the vmode/hmode Forbidden-case wiring above.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"$$a\eqno b$$\end");
    run_to_end(&mut control, &mut universe);

    let text = terminal_text(&universe);
    assert!(
        !text.contains("You can't use `\\eqno'"),
        "\\eqno is legal in display math mode: {text}"
    );
}

#[test]
fn canonical_mark_builds_mark_node_with_expanded_text() {
    // TeX82 §1101's `make_mark`: `scan_toks(false,true)` -- a fully expanded
    // balanced general text -- becomes a class-0 mark node appended to
    // whatever list is current, with no mode restriction and no `build_page`
    // call (unlike `\insert`/`\penalty`).
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\setbox0=\vbox{\def\who{world}\mark{hello \who}}\end",
    );
    run_to_end(&mut control, &mut universe);

    let vbox = universe
        .box_reg(0)
        .and_then(|id| universe.nodes(id).first().map(|node| node.to_owned()))
        .expect("outer vbox stores");
    let Node::VList(vbox) = vbox else {
        panic!("setbox0 contains a vbox");
    };
    let children = universe.nodes(vbox.children);
    assert_eq!(children.len(), 1, "the mark node is the vbox's only child");
    let Node::Mark { class, tokens } = children.first().expect("checked len").to_owned() else {
        panic!("vbox child is a mark_node");
    };
    assert_eq!(class, 0, "plain \\mark always uses class 0");
    let text: String = universe
        .tokens(tokens)
        .iter()
        .filter_map(|token| match token {
            Token::Char { ch, .. } => Some(*ch),
            _ => None,
        })
        .collect();
    assert_eq!(text, "hello world", "\\who expanded before capture");
}

/// Extracts the shift of each `HList`/`VList` child of `children`, skipping
/// interleaved glue (e.g. `\baselineskip` inserted between vertical-list
/// boxes) so tests can assert on box shifts alone.
fn box_shifts(universe: &Universe, children: tex_state::ids::NodeListId) -> Vec<Scaled> {
    universe
        .nodes(children)
        .into_iter()
        .filter_map(|node| match node.to_owned() {
            Node::HList(box_node) | Node::VList(box_node) => Some(box_node.shift),
            _ => None,
        })
        .collect()
}

#[test]
fn canonical_raise_lower_apply_signed_shift_in_horizontal_mode() {
    // TeX82 §1073: `hmode+vmove` is legal (`\raise`/`\lower`'s own mode
    // family), and `t:=cur_chr; scan_normal_dimen; if t=0 then
    // scan_box(cur_val) else scan_box(-cur_val)` -- `\lower` (chr_code 0)
    // keeps the scanned dimension, `\raise` (chr_code 1) negates it.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\setbox0=\hbox{\lower3pt\hbox{}\raise2pt\hbox{}}",
    );
    run_to_end(&mut control, &mut universe);

    let outer = universe
        .box_reg(0)
        .and_then(|id| universe.nodes(id).first().map(|node| node.to_owned()))
        .expect("outer hbox stores");
    let Node::HList(outer) = outer else {
        panic!("setbox0 contains an hbox");
    };
    assert_eq!(
        box_shifts(&universe, outer.children),
        vec![
            Scaled::from_raw(3 * Scaled::UNITY),
            Scaled::from_raw(-2 * Scaled::UNITY),
        ],
        "\\lower keeps its sign, \\raise negates it"
    );
}

#[test]
fn canonical_moveleft_moveright_apply_signed_shift_in_vertical_mode() {
    // TeX82 §1073: `vmode+hmove` is legal (`\moveleft`/`\moveright`'s own
    // mode family). `\moveright` (chr_code 0) keeps the scanned dimension,
    // `\moveleft` (chr_code 1) negates it -- the opposite pairing from
    // `\raise`/`\lower` even though the sign rule is the same shape.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\setbox0=\vbox{\moveright3pt\hbox{}\moveleft2pt\hbox{}}",
    );
    run_to_end(&mut control, &mut universe);

    let outer = universe
        .box_reg(0)
        .and_then(|id| universe.nodes(id).first().map(|node| node.to_owned()))
        .expect("outer vbox stores");
    let Node::VList(outer) = outer else {
        panic!("setbox0 contains a vbox");
    };
    assert_eq!(
        box_shifts(&universe, outer.children),
        vec![
            Scaled::from_raw(3 * Scaled::UNITY),
            Scaled::from_raw(-2 * Scaled::UNITY),
        ],
        "\\moveright keeps its sign, \\moveleft negates it"
    );
}

#[test]
fn canonical_box_shift_illegal_mode_reports_and_never_scans_a_dimension() {
    // TeX82 §1073's "Forbidden cases" list `hmode+hmove` (`\moveleft`/
    // `\moveright` used outside vertical mode) alongside `vmode+vmove` and
    // `mmode+hmove`. `report_illegal_case` fires immediately and
    // `scan_normal_dimen` is never called, so the following "2pt" is left
    // as perfectly ordinary character tokens -- not consumed as an
    // operand -- and `\hbox{}` after them is a plain, unshifted box. A real
    // font is selected first so those characters actually reach the list
    // instead of being dropped as "Missing character" under `\nullfont`.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_cmr10_font(&mut control, &mut universe);
    register_source(
        &mut control,
        br"\font\f=cmr10 \setbox0=\hbox{\f \moveleft2pt\hbox{}}",
    );
    run_to_end(&mut control, &mut universe);

    let text = terminal_text(&universe);
    assert!(
        text.contains("You can't use `\\moveleft' in restricted horizontal mode"),
        "illegal box-shift mode is diagnosed: {text}"
    );

    let outer = universe
        .box_reg(0)
        .and_then(|id| universe.nodes(id).first().map(|node| node.to_owned()))
        .expect("outer hbox stores");
    let Node::HList(outer) = outer else {
        panic!("setbox0 contains an hbox");
    };
    let children = universe.nodes(outer.children);
    let chars: String = children
        .iter()
        .filter_map(|node| match node.to_owned() {
            Node::Char { ch, .. } => Some(ch),
            _ => None,
        })
        .collect();
    assert_eq!(
        chars, "2pt",
        "the un-scanned dimension became ordinary characters: {children:?}"
    );
    assert_eq!(
        box_shifts(&universe, outer.children),
        vec![Scaled::from_raw(0)],
        "the trailing \\hbox{{}} is an ordinary, unshifted box"
    );
}

#[test]
fn canonical_box_shift_applies_to_box_register_and_last_box() {
    // TeX82 §1084's `scan_box` accepts any `make_box` command, not just
    // `\hbox`/`\vbox`/`\vtop`: `\box`/`\copy` and `\lastbox` resolve to a
    // node immediately rather than opening a group, and the shift must
    // still apply to that immediate result. (A box-shift's own box can
    // never itself be a `\setbox` target -- `scan_box` requires
    // `cur_cmd=make_box`, which `\raise`/`\lower`/`\moveleft`/`\moveright`
    // never are -- so both results are observed as the last thing appended
    // to an enclosing box body instead.)
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\setbox1=\hbox{}\setbox0=\hbox{\raise5pt\box1}\setbox3=\hbox{\hbox{}\lower4pt\lastbox}",
    );
    run_to_end(&mut control, &mut universe);

    let register_shift = universe
        .box_reg(0)
        .and_then(|id| universe.nodes(id).first().map(|node| node.to_owned()))
        .expect("outer hbox stores");
    let Node::HList(register_shift) = register_shift else {
        panic!("setbox0 contains an hbox");
    };
    assert_eq!(
        box_shifts(&universe, register_shift.children),
        vec![Scaled::from_raw(-5 * Scaled::UNITY)],
        "\\raise5pt\\box1 shifts the register's box by -5pt"
    );

    let last_box_shift = universe
        .box_reg(3)
        .and_then(|id| universe.nodes(id).first().map(|node| node.to_owned()))
        .expect("outer hbox stores");
    let Node::HList(last_box_shift) = last_box_shift else {
        panic!("setbox3 contains an hbox");
    };
    // `\lastbox` removes the inner `\hbox{}` from this same body, then the
    // shifted result is re-appended to it -- so exactly one child remains.
    assert_eq!(
        box_shifts(&universe, last_box_shift.children),
        vec![Scaled::from_raw(4 * Scaled::UNITY)],
        "\\lower4pt\\lastbox shifts the removed box by +4pt and reappends it"
    );
}

#[test]
fn canonical_box_shift_missing_box_operand_recovers_and_replays_the_command() {
    // TeX82 §1084's `scan_box` "A <box> was supposed to be here" recovery:
    // a non-`make_box` command is backed up and replayed normally, rather
    // than being consumed as (or silently dropping) the shift's operand.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\setbox0=\hbox{\raise2pt\kern1pt}");
    run_to_end(&mut control, &mut universe);

    let text = terminal_text(&universe);
    assert!(
        text.contains("A <box> was supposed to be here"),
        "the missing box operand is diagnosed: {text}"
    );

    let outer = universe
        .box_reg(0)
        .and_then(|id| universe.nodes(id).first().map(|node| node.to_owned()))
        .expect("outer hbox stores");
    let Node::HList(outer) = outer else {
        panic!("setbox0 contains an hbox");
    };
    let children = universe.nodes(outer.children);
    assert!(
        children
            .iter()
            .any(|node| matches!(node.to_owned(), Node::Kern { amount, .. } if amount == Scaled::from_raw(Scaled::UNITY))),
        "the backed-up \\kern1pt was replayed normally: {children:?}"
    );
}

#[test]
fn canonical_assignments_cover_code_tables_and_reject_macro_prefixes() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br#"\catcode`@=11\lccode`A=`a\uccode`a=`A\sfcode`A=1000\mathcode`x="7131\delcode`(=123\end"#,
    );
    loop {
        if matches!(
            control.step(&mut universe).expect("code table assignment"),
            MainControlStep::End
        ) {
            break;
        }
    }
    assert_eq!(universe.catcode('@'), tex_state::token::Catcode::Letter);
    assert_eq!(universe.lccode('A'), 'a' as u32);
    assert_eq!(universe.uccode('a'), 'A' as u32);
    assert_eq!(universe.sfcode('A'), 1000);
    assert_eq!(universe.mathcode('x'), 0x7131);
    assert_eq!(universe.delcode('('), 123);

    // TeX82 §1213 reports an irrelevant `\long`/`\outer` prefix and still
    // performs the assignment; §1214 leaves `a` unadjusted on purpose.
    let mut invalid_universe = Universe::new_with_plain_catcodes();
    let mut invalid = CanonicalMainControl::tex82_initex(&mut invalid_universe);
    register_source(&mut invalid, br"\long\count0=1\end");
    run_to_end(&mut invalid, &mut invalid_universe);
    assert_eq!(invalid_universe.count(0), 1);
    let reported = terminal_text(&invalid_universe);
    assert!(
        reported.contains("! You can't use `\\long' or `\\outer' with `\\count'."),
        "{reported}"
    );
}

#[test]
fn canonical_prefixed_command_skips_relax_and_preserves_group_scope() {
    let source = include_bytes!("../../../../tests/corpus/tex_exec/prefixed_macro.tex");
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, source);
    run_to_end(&mut control, &mut universe);

    assert_eq!(
        universe.count(0),
        7,
        "the committed TeX82 microfixture proves §§404,1211 retain the prefix across relax and a macro call"
    );
    assert!(
        terminal_text(&universe).contains("P:7"),
        "the canonical replay matches the committed reference observation"
    );
}

#[test]
fn canonical_illegal_prefix_reports_and_replays_the_command_once() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\global\relax\message{replayed}\count0=1\end",
    );
    run_to_end(&mut control, &mut universe);

    assert_eq!(universe.count(0), 1, "execution continues after back_error");
    let output = terminal_text(&universe);
    assert_eq!(
        output.matches("replayed").count(),
        1,
        "§1212 backs the rejected command up exactly once: {output}"
    );
    assert!(
        output.contains("! You can't use a prefix with `\\message'."),
        "§1212 prints the rejected command exactly: {output}"
    );
    assert!(
        output.contains("I'll pretend you didn't say \\long or \\outer or \\global."),
        "§1212 prints its one-line help exactly: {output}"
    );
}

#[test]
fn canonical_relax_inside_prefix_collection_does_not_fire_afterassignment() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\def\mark{\count1=\count0}\afterassignment\mark\global\relax\count0=9\end",
    );
    run_to_end(&mut control, &mut universe);

    assert_eq!(universe.count(0), 9);
    assert_eq!(
        universe.count(1),
        9,
        "§404 filler does not interrupt §1211 or fire afterassignment before the assignment commits"
    );
}

#[test]
fn canonical_vsplit_scans_operands_before_replaying_destructive_repack() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\setbox0=\vbox{\hrule height 10pt\vskip10pt\hrule height 10pt}\setbox1=\vsplit0 to 10pt\end",
    );
    let mut observations = ObservationRecorder::default();
    loop {
        if matches!(
            control
                .step_with_observer(&mut universe, &mut observations)
                .expect("canonical vsplit executes"),
            MainControlStep::End | MainControlStep::EndOfInput
        ) {
            break;
        }
    }

    let split = universe.box_reg(1).expect("split result is assigned");
    let remainder = universe.box_reg(0).expect("remainder is repacked");
    assert_ne!(
        split, remainder,
        "vsplit does not alias its destructive remainder"
    );
}

#[test]
fn canonical_display_diagnostics_keep_show_raw_and_scan_other_operands() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\def\shown{expanded}\show\shown\count0=17\showthe\count0\setbox0=\hbox{}\showbox0\end",
    );
    run_to_end(&mut control, &mut universe);

    let text = terminal_text(&universe);
    assert!(text.contains("> \\shown=macro:->expanded."), "{text}");
    assert!(text.contains("> 17."), "{text}");
    assert!(text.contains("> \\box0="), "{text}");
    assert!(
        !text.contains("Undefined control sequence \\expanded"),
        "raw show must not expand its operand: {text}"
    );
}

#[test]
fn canonical_showthe_and_the_preserve_font_identifier_tokens() {
    let mut universe = Universe::new_with_plain_catcodes();
    let nullfont = universe.intern("nullfont");
    universe.set_font_identifier_symbol(tex_state::font::NULL_FONT, nullfont);
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\showthe\font\showthe\nullfont\showthe\textfont0\message{\the\font}\end",
    );
    run_to_end(&mut control, &mut universe);

    let text = terminal_text(&universe);
    assert_eq!(text.matches("> \\nullfont.").count(), 3, "{text}");
    assert!(text.contains("\\nullfont"), "{text}");
}

#[test]
fn canonical_errmessage_uses_the_world_terminal_and_log_boundary() {
    // TeX82's `issue_message` sends \errmessage through `print_err`/`error`,
    // then main control resumes normally.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\errmessage{canonical diagnostic}\message{recovered}\end",
    );
    run_to_end(&mut control, &mut universe);

    // Every character reaches the World through the §54 selector's sink, so
    // the report is a run of `StreamWrite` effects rather than one.
    assert!(
        universe
            .world()
            .effect_records()
            .iter()
            .any(|effect| matches!(
                effect,
                EffectRecord::StreamWrite {
                    sink: tex_state::PrintSink::TerminalAndLog,
                    ..
                }
            )),
        "diagnostic must be a World output effect"
    );

    let text = terminal_text(&universe);
    assert!(text.contains("! canonical diagnostic."), "{text}");
    assert!(text.contains("recovered"), "{text}");
}

#[test]
fn canonical_delete_last_removes_only_matching_tail_node() {
    // TeX82 §1105's `delete_last`: `\unpenalty` after a kern tail leaves the
    // list untouched (no error, no removal), while `\unkern` correctly
    // matches and removes it.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    // `\hbox`, not `\vbox`: `\kern` in vertical mode is covered separately by
    // `canonical_kern_in_vertical_mode_does_not_start_a_paragraph` below, so
    // this test exercises only `delete_last`'s ordinary (`tail<>head`)
    // branch.
    register_source(
        &mut control,
        br"\setbox0=\hbox{\kern1pt\unpenalty\kern2pt\unkern}",
    );
    run_to_end(&mut control, &mut universe);

    let hbox = universe
        .box_reg(0)
        .and_then(|id| universe.nodes(id).first().map(|node| node.to_owned()))
        .expect("outer hbox stores");
    let Node::HList(hbox) = hbox else {
        panic!("setbox0 contains an hbox");
    };
    let children = universe.nodes(hbox.children).to_vec();
    assert_eq!(
        children,
        vec![Node::Kern {
            amount: Scaled::from_raw(Scaled::UNITY),
            kind: KernKind::Explicit,
        }],
        "\\unpenalty left the kern alone; \\unkern removed the second one: {children:?}"
    );
}

#[test]
fn canonical_kern_in_vertical_mode_does_not_start_a_paragraph() {
    // TeX82 §1057's `any_mode(kern): append_kern` (§1061): `\kern` has no
    // mode-specific dispatch entry at all, unlike `\hskip` (§1090's
    // `head_for_vmode`, genuinely `vmode+hskip`-listed). A bare `\kern` in
    // (internal) vertical mode must append directly to the current list, not
    // silently trigger `new_graf` and swallow the kern into a bogus empty
    // paragraph -- regression test for umber2-johp.85.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\setbox0=\vbox{\kern5pt\hrule height1pt}");
    run_to_end(&mut control, &mut universe);

    let vbox = universe
        .box_reg(0)
        .and_then(|id| universe.nodes(id).first().map(|node| node.to_owned()))
        .expect("outer vbox stores");
    let Node::VList(vbox) = vbox else {
        panic!("setbox0 contains a vbox");
    };
    let children = universe.nodes(vbox.children).to_vec();
    assert_eq!(
        children,
        vec![
            Node::Kern {
                amount: Scaled::from_raw(5 * Scaled::UNITY),
                kind: KernKind::Explicit,
            },
            Node::Rule {
                width: None,
                height: Some(Scaled::from_raw(Scaled::UNITY)),
                depth: Some(Scaled::from_raw(0)),
            },
        ],
        "the kern must land directly on the vlist, not be dropped by a bogus \
         empty paragraph: {children:?}"
    );
}

#[test]
fn canonical_unhbox_in_vertical_mode_starts_a_paragraph() {
    // TeX82 §1090's vmode-paragraph-starting list includes `vmode+un_hbox`
    // (opposite-direction category error from umber2-johp.85's `\kern` bug):
    // unlike `\unvbox`/`\unvcopy` (`vmode+un_vbox`, legitimately *not* in
    // this list, spliced directly onto the vlist), a bare `\unhbox`/
    // `\unhcopy` directly in vertical mode must back up the token and start
    // a paragraph first, so the unboxed hlist's contents become ordinary
    // horizontal-mode material instead of being spliced directly onto the
    // enclosing vlist -- regression test for umber2-johp.87.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\setbox1=\hbox{}\setbox0=\vbox{\unhbox1}");
    run_to_end(&mut control, &mut universe);

    let vbox = universe
        .box_reg(0)
        .and_then(|id| universe.nodes(id).first().map(|node| node.to_owned()))
        .expect("outer vbox stores");
    let Node::VList(vbox) = vbox else {
        panic!("setbox0 contains a vbox");
    };
    let children = universe.nodes(vbox.children).to_vec();
    assert!(
        matches!(children.first(), Some(Node::HList(_))),
        "\\unhbox started a paragraph, indenting the (empty) first line: {children:?}"
    );
}

#[test]
fn canonical_unvbox_in_vertical_mode_does_not_start_a_paragraph() {
    // TeX82 §1090's vmode-paragraph-starting list does *not* include
    // `vmode+un_vbox`: `\unvbox`/`\unvcopy` legitimately append the unboxed
    // vlist directly to the current vertical list. Negative control for
    // `canonical_unhbox_in_vertical_mode_starts_a_paragraph` above.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\setbox1=\vbox{\kern1pt}\setbox0=\vbox{\unvbox1}",
    );
    run_to_end(&mut control, &mut universe);

    let vbox = universe
        .box_reg(0)
        .and_then(|id| universe.nodes(id).first().map(|node| node.to_owned()))
        .expect("outer vbox stores");
    let Node::VList(vbox) = vbox else {
        panic!("setbox0 contains a vbox");
    };
    let children = universe.nodes(vbox.children).to_vec();
    assert_eq!(
        children,
        vec![Node::Kern {
            amount: Scaled::from_raw(Scaled::UNITY),
            kind: KernKind::Explicit,
        }],
        "the unboxed kern must land directly on the vlist, with no bogus \
         paragraph started: {children:?}"
    );
}

#[test]
fn canonical_valign_in_vertical_mode_starts_a_paragraph() {
    // TeX82 §1090's vmode-paragraph-starting list includes `vmode+valign`
    // (unlike `vmode+halign`, which is legal directly in vertical mode via
    // `vmode+halign,hmode+valign:init_align` and is not in this list): a
    // bare `\valign` directly in vertical mode must back up the token and
    // start a paragraph first, then reprocess `\valign` as embedded
    // alignment material inside the resulting paragraph's horizontal list
    // -- regression test for umber2-johp.87.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\setbox0=\vbox{\valign{#\cr\cr}}");
    run_to_end(&mut control, &mut universe);

    assert!(
        !terminal_text(&universe).contains('!'),
        "no diagnostics expected: {}",
        terminal_text(&universe)
    );
    let vbox = universe
        .box_reg(0)
        .and_then(|id| universe.nodes(id).first().map(|node| node.to_owned()))
        .expect("outer vbox stores");
    let Node::VList(vbox) = vbox else {
        panic!("setbox0 contains a vbox");
    };
    let children = universe.nodes(vbox.children).to_vec();
    assert!(
        matches!(children.first(), Some(Node::HList(_))),
        "\\valign started a paragraph, indenting the (empty) first line: {children:?}"
    );
}

#[test]
fn canonical_spacefactor_accepts_both_horizontal_modes_and_boundary_values() {
    // TeX82 §§1210/1243: `set_aux` is delivered in every mode, but
    // `alter_aux` accepts the hmode selector only when `abs(mode)=hmode`.
    for (mode, value) in [
        (Mode::Horizontal, 1),
        (Mode::Horizontal, 32_767),
        (Mode::RestrictedHorizontal, 1),
        (Mode::RestrictedHorizontal, 32_767),
    ] {
        let mut universe = Universe::new_with_plain_catcodes();
        let mut control = CanonicalMainControl::tex82_initex(&mut universe);
        control.modes.push(mode);
        register_source(&mut control, format!("\\spacefactor = {value} ").as_bytes());
        run_to_end(&mut control, &mut universe);

        assert_eq!(control.modes.current_list().space_factor(), value);
        assert!(!terminal_text(&universe).contains('!'));
    }
}

#[test]
fn canonical_spacefactor_out_of_range_values_are_diagnosed_and_leave_state_unchanged() {
    // TeX82 §1243: `if (cur_val<=0)or(cur_val>32767) then int_error(cur_val)
    // else space_factor:=cur_val` -- an out-of-range value is diagnosed and
    // the space factor is left untouched, not clamped.
    for value in [-1, 0, 32_768] {
        let mut universe = Universe::new_with_plain_catcodes();
        let mut control = CanonicalMainControl::tex82_initex(&mut universe);
        control.modes.push(Mode::Horizontal);
        control.modes.current_list_mutation().set_space_factor(1234);
        register_source(&mut control, format!("\\spacefactor={value} ").as_bytes());
        run_to_end(&mut control, &mut universe);

        assert_eq!(control.modes.current_list().space_factor(), 1234);
        let terminal = terminal_text(&universe);
        assert!(terminal.contains(&format!("! Bad space factor ({value}).")));
        assert!(terminal.contains("I allow only values in the range 1..32767 here."));
    }
}

#[test]
fn canonical_spacefactor_illegal_modes_report_before_scanning_and_preserve_next_token() {
    // TeX82 §1243 tests the mode before `scan_optional_equals; scan_int`.
    // The following assignment must therefore execute in all four illegal
    // modes instead of being consumed as a putative integer operand.
    for mode in [
        Mode::Vertical,
        Mode::InternalVertical,
        Mode::Math,
        Mode::DisplayMath,
    ] {
        let mut universe = Universe::new_with_plain_catcodes();
        let mut control = CanonicalMainControl::tex82_initex(&mut universe);
        if mode != Mode::Vertical {
            control.modes.push(mode);
        }
        register_source(&mut control, br"\spacefactor\global\count0=17 ");
        run_to_end(&mut control, &mut universe);

        assert_eq!(universe.count(0), 17, "following token in {mode:?}");
        let terminal = terminal_text(&universe);
        let mode_text = match mode {
            Mode::Vertical => "vertical mode",
            Mode::InternalVertical => "internal vertical mode",
            Mode::Math => "math mode",
            Mode::DisplayMath => "display math mode",
            _ => unreachable!("the table contains only illegal modes"),
        };
        assert!(
            terminal.contains(&format!("You can't use `\\spacefactor' in {mode_text}.")),
            "{terminal}"
        );
    }
}

#[test]
fn canonical_spacefactor_targets_only_the_current_list_and_is_always_global() {
    // TeX82 §§1242/1243 identify auxiliary assignments as always global:
    // braces and `\global` do not create save-stack entries. The target is
    // nevertheless the current horizontal list, so a nested hbox owns an
    // independent value.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    control.modes.push(Mode::Horizontal);
    register_source(
        &mut control,
        br"{\spacefactor 2000}\setbox0=\hbox{\global\spacefactor=3000}\relax",
    );
    run_to_end(&mut control, &mut universe);

    assert_eq!(control.modes.current_list().space_factor(), 2000);
    assert!(!terminal_text(&universe).contains('!'));
}

#[test]
fn canonical_prevgraf_assignment_sets_the_enclosing_vertical_level() {
    // TeX82 §1244's `alter_prev_graf`: `\prevgraf` is `any_mode` and writes
    // the nearest enclosing vertical level's paragraph count, even from
    // horizontal mode -- regression test for umber2-johp.86.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    control.modes.push(Mode::Horizontal);
    register_source(&mut control, br"\prevgraf=3 ");
    run_to_end(&mut control, &mut universe);

    assert_eq!(control.modes.enclosing_vertical_prev_graf(), 3);
}

#[test]
fn canonical_prevgraf_assignment_is_legal_in_every_mode() {
    // TeX82 §1210 dispatches `any_mode(set_prev_graf)` to `prefixed_command`,
    // and §1244's `alter_prev_graf` walks outward to a vertical level instead
    // of rejecting any mode. Positive and zero values therefore work in all
    // six main-control modes.
    for (mode, value) in [
        (Mode::Vertical, 0),
        (Mode::InternalVertical, 1),
        (Mode::Horizontal, 2),
        (Mode::RestrictedHorizontal, 3),
        (Mode::Math, 4),
        (Mode::DisplayMath, 5),
    ] {
        let mut universe = Universe::new_with_plain_catcodes();
        let mut control = CanonicalMainControl::tex82_initex(&mut universe);
        if mode != Mode::Vertical {
            control.modes.push(mode);
        }
        register_source(&mut control, format!("\\prevgraf={value} ").as_bytes());
        run_to_end(&mut control, &mut universe);

        assert_eq!(
            control.modes.enclosing_vertical_prev_graf(),
            value,
            "mode {mode:?}"
        );
        assert!(
            !terminal_text(&universe).contains("You can't use"),
            "mode {mode:?}: {}",
            terminal_text(&universe)
        );
    }
}

#[test]
fn canonical_prevgraf_updates_nearest_internal_vertical_level() {
    // TeX82 §1244 stops at the first enclosing mode whose absolute value is
    // vmode. An hmode nested inside an internal-vmode box must update that
    // box's paragraph count, not the outer page's.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    control.modes.set_enclosing_vertical_prev_graf(11);
    control.modes.push(Mode::InternalVertical);
    control.modes.set_enclosing_vertical_prev_graf(12);
    control.modes.push(Mode::Horizontal);
    register_source(&mut control, br"\prevgraf = 6 ");
    run_to_end(&mut control, &mut universe);

    assert_eq!(control.modes.enclosing_vertical_prev_graf(), 6);
    control.modes.pop().expect("leave horizontal mode");
    control.modes.pop().expect("leave internal vertical mode");
    assert_eq!(control.modes.enclosing_vertical_prev_graf(), 11);
}

#[test]
fn canonical_prevgraf_is_ungrouped_and_prefixes_do_not_change_its_scope() {
    // TeX82 §1242 says these definitions are always global. `prev_graf` is a
    // mode-list field, not an eqtb entry, so braces, `\global`, and
    // `\globaldefs` cannot create save-stack restoration.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"{\prevgraf=2}\global\prevgraf 3 \globaldefs=-1 \prevgraf = 4 ",
    );
    run_to_end(&mut control, &mut universe);

    assert_eq!(control.modes.enclosing_vertical_prev_graf(), 4);
}

#[test]
fn canonical_prevgraf_negative_value_is_diagnosed_and_left_unchanged() {
    // TeX82 §1244: `if cur_val<0 then int_error(cur_val)` -- a negative value
    // is diagnosed and the paragraph count is left untouched.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    control.modes.set_enclosing_vertical_prev_graf(7);
    register_source(&mut control, br"\prevgraf=-1 ");
    run_to_end(&mut control, &mut universe);

    assert_eq!(control.modes.enclosing_vertical_prev_graf(), 7);
    assert_eq!(
        terminal_text(&universe),
        "\n! Bad \\prevgraf (-1).\nI allow only nonnegative values here.\n"
    );
}

#[test]
fn canonical_prevgraf_scanner_preserves_the_following_token_after_negative_value() {
    // TeX82 §1244 calls the ordinary `scan_int`; its error branch changes no
    // input-stack state after scanning. The command immediately following a
    // rejected integer must therefore still execute.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    control.modes.set_enclosing_vertical_prev_graf(7);
    register_source(&mut control, br"\prevgraf=-1\count0=23 ");
    run_to_end(&mut control, &mut universe);

    assert_eq!(control.modes.enclosing_vertical_prev_graf(), 7);
    assert_eq!(universe.count(0), 23);
}

#[test]
fn canonical_delete_last_outer_vertical_apologizes_only_when_last_page_item_is_glue() {
    // TeX82 §1105: `(mode=vmode)and(tail=head)` never structurally removes
    // anything -- `\unpenalty`/`\unkern` always apologize, but `\unskip`
    // apologizes only when the page builder's own `last_glue` memo (§996)
    // shows the most recently placed page item really was glue. This is a
    // regression test for a real bug: the outer-vertical empty-list branch
    // used to ignore that memo entirely and always succeed for `\unskip`.
    let mut universe = Universe::new_with_plain_catcodes();
    let glue_spec = universe.intern_glue(GlueSpec::ZERO);
    universe.update_page_last_from_node(&Node::Glue {
        spec: glue_spec,
        kind: tex_state::node::GlueKind::Normal,
        leader: None,
    });
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\unskip\count0=23\end");
    run_to_end(&mut control, &mut universe);
    assert!(terminal_text(&universe).contains("You can't use `\\unskip' in vertical mode"));
    assert!(terminal_text(&universe).contains("Try `I\\vskip-\\lastskip' instead."));
    assert_eq!(
        universe.count(0),
        23,
        "§1105's error resumes at the following token"
    );

    let mut universe = Universe::new_with_plain_catcodes();
    universe.update_page_last_from_node(&Node::Penalty(0));
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\unskip\end");
    run_to_end(&mut control, &mut universe);
    assert!(
        !terminal_text(&universe).contains("You can't"),
        "\\unskip is silent when the last page item was not glue: {}",
        terminal_text(&universe)
    );
}

#[test]
fn canonical_delete_last_outer_vertical_diagnostics_recover_for_all_three_commands() {
    // TeX82 §1105 selects the second help line by `cur_chr`, calls `error`,
    // and resumes. In particular, this is not a fatal executor boundary.
    for (source, command, help) in [
        (
            br"\unpenalty\count0=17\end".as_slice(),
            "\\unpenalty",
            "Perhaps you can make the output routine do it.",
        ),
        (
            br"\unkern\count0=17\end".as_slice(),
            "\\unkern",
            "Try `I\\kern-\\lastkern' instead.",
        ),
        (
            br"\unskip\count0=17\end".as_slice(),
            "\\unskip",
            "Try `I\\vskip-\\lastskip' instead.",
        ),
    ] {
        let mut universe = Universe::new_with_plain_catcodes();
        if command == "\\unskip" {
            let spec = universe.intern_glue(GlueSpec::ZERO);
            universe.update_page_last_from_node(&Node::Glue {
                spec,
                kind: GlueKind::Normal,
                leader: None,
            });
        }
        let mut control = CanonicalMainControl::tex82_initex(&mut universe);
        register_source(&mut control, source);
        run_to_end(&mut control, &mut universe);

        let terminal = terminal_text(&universe);
        assert!(
            terminal.contains(&format!("You can't use `{command}' in vertical mode")),
            "{terminal}"
        );
        assert!(terminal.contains(help), "{terminal}");
        assert_eq!(universe.count(0), 17);
    }
}

#[test]
fn canonical_delete_last_removes_matching_unswept_page_contribution_tails() {
    // TeX82 §1105 permits outer-vertical removal only while the node is still
    // on the contribution list (`tail<>head`). Once swept, the page builder's
    // memo drives the apology branch covered above.
    for (source, tail) in [
        (br"\unpenalty\end".as_slice(), Node::Penalty(50)),
        (
            br"\unkern\end".as_slice(),
            Node::Kern {
                amount: Scaled::from_raw(Scaled::UNITY),
                kind: KernKind::Explicit,
            },
        ),
    ] {
        let mut universe = Universe::new_with_plain_catcodes();
        universe.append_page_contribution(tail);
        let mut control = CanonicalMainControl::tex82_initex(&mut universe);
        register_source(&mut control, source);
        run_to_end(&mut control, &mut universe);

        assert!(universe.page_contributions().is_empty());
        assert!(!terminal_text(&universe).contains("You can't"));
    }

    let mut universe = Universe::new_with_plain_catcodes();
    let spec = universe.intern_glue(GlueSpec::ZERO);
    universe.append_page_contribution(Node::Glue {
        spec,
        kind: GlueKind::Normal,
        leader: None,
    });
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\unskip\end");
    run_to_end(&mut control, &mut universe);
    assert!(universe.page_contributions().is_empty());
    assert!(!terminal_text(&universe).contains("You can't"));
}

#[test]
fn canonical_delete_last_is_mode_complete_and_preserves_mismatched_tails() {
    // TeX82 §1105's `any_mode(remove_item)` applies unchanged in restricted
    // hmode, internal vmode, inline math, and display math. Matching tails
    // disappear; empty and mismatched tails are silent no-ops.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\setbox0=\hbox{\kern1pt\unpenalty\unkern\unskip}
           \setbox1=\vbox{\penalty7\unkern\unpenalty\unskip}
           \setbox2=\hbox{$\kern2pt\unpenalty\unkern\unskip$}
           \setbox3=\hbox{$$\kern3pt\unpenalty\unkern\unskip$$}
           \count0=41\end",
    );
    run_to_end(&mut control, &mut universe);

    for index in 0..=3 {
        let box_node = universe
            .box_reg(index)
            .and_then(|id| universe.nodes(id).first().map(|node| node.to_owned()))
            .expect("box register stores a box");
        let children = match box_node {
            Node::HList(node) | Node::VList(node) => universe.nodes(node.children).to_vec(),
            node => panic!("register {index} stores {node:?}"),
        };
        assert!(
            !children
                .iter()
                .any(|node| matches!(node, Node::Kern { .. } | Node::Penalty(_))),
            "matching tail survives in register {index}: {children:?}"
        );
    }
    assert_eq!(universe.count(0), 41);
}

#[test]
fn canonical_delete_last_does_not_enter_discretionary_replacement_lists() {
    // TeX82 §1105 advances over a discretionary's `replace_count` nodes while
    // finding the predecessor of the physical tail. If those replacement
    // nodes end at `tail`, it returns without deleting them. Umber stores the
    // replacement as the discretionary node's child list, so the equivalent
    // invariant is that `delete_last` never descends into that child.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\setbox0=\hbox{\discretionary{}{}{\kern4pt}\unkern}\end",
    );
    run_to_end(&mut control, &mut universe);

    let hbox = universe
        .box_reg(0)
        .and_then(|id| universe.nodes(id).first().map(|node| node.to_owned()))
        .expect("hbox exists");
    let Node::HList(hbox) = hbox else {
        panic!("register 0 stores {hbox:?}");
    };
    let children = universe.nodes(hbox.children).to_vec();
    let [Node::Disc { replace, .. }] = children.as_slice() else {
        panic!(
            "hbox must retain its discretionary: {:?}",
            universe.nodes(hbox.children)
        );
    };
    assert!(matches!(
        universe.nodes(*replace).testing_decoded(),
        [Node::Kern {
            amount,
            kind: KernKind::Explicit
        }] if *amount == Scaled::from_raw(4 * Scaled::UNITY)
    ));
}

#[test]
fn canonical_delete_last_rejects_prefix_then_executes_without_consuming_following_token() {
    // TeX82 §§1211-1212 diagnose a prefix on this non-prefixed command,
    // discard the prefix, and execute `remove_item` normally. §1105 scans no
    // operand, so the immediately following assignment remains untouched.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\setbox0=\hbox{\kern1pt\global\unkern\global\count0=29}\end",
    );
    run_to_end(&mut control, &mut universe);

    assert!(terminal_text(&universe).contains("You can't use a prefix with `\\unkern'."));
    assert_eq!(universe.count(0), 29);
    let Node::HList(hbox) = universe
        .box_reg(0)
        .and_then(|id| universe.nodes(id).first().map(|node| node.to_owned()))
        .expect("hbox exists")
    else {
        panic!("register 0 must store an hbox");
    };
    assert!(
        universe.nodes(hbox.children).is_empty(),
        "prefixed \\unkern must still remove the matching tail"
    );
}

#[test]
fn canonical_last_item_reads_the_matching_current_list_tail_and_zero_otherwise() {
    // TeX82 §424's "Fetch an item in the current node, if appropriate":
    // `\lastkern` reads the tail when it really is a kern node, while
    // `\lastpenalty`/`\lastskip` see a type mismatch and fall back to their
    // own level's zero, exactly like an empty list would.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    control.modes.push(Mode::InternalVertical);
    control.modes.current_list_mutation().push(Node::Kern {
        amount: Scaled::from_raw(65536 * 3),
        kind: tex_state::node::KernKind::Explicit,
    });
    register_source(
        &mut control,
        br"\showthe\lastkern\showthe\lastpenalty\showthe\lastskip",
    );
    run_to_end(&mut control, &mut universe);

    let text = terminal_text(&universe);
    assert!(text.contains("> 3.0pt."), "{text}");
    assert!(text.contains("> 0."), "{text}");
    assert!(text.contains("> 0.0pt."), "{text}");
}

#[test]
fn canonical_last_item_outer_vertical_prefers_real_contribution_tail_over_page_memo() {
    // TeX82 §424: while the outer vertical list's contribution tail is
    // still real (not yet swept onto the page by `build_page`), it governs
    // -- the page builder's own memo is consulted only once that list is
    // empty, exactly like `\unskip`'s existing precedent.
    let mut universe = Universe::new_with_plain_catcodes();
    universe.update_page_last_from_node(&Node::Penalty(99));
    universe.append_page_contribution(Node::Penalty(7));
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\showthe\lastpenalty");
    run_to_end(&mut control, &mut universe);

    assert!(
        terminal_text(&universe).contains("> 7."),
        "the real contribution tail (7), not the stale page memo (99): {}",
        terminal_text(&universe)
    );
}

#[test]
fn canonical_last_item_outer_vertical_falls_back_to_page_memo_when_contribution_list_is_empty() {
    // TeX82 §996/§424: once `build_page` has swept the whole contribution
    // list onto the page, `\lastskip` reads the page builder's own
    // `last_glue` memo instead of the (now empty) contribution list.
    let mut universe = Universe::new_with_plain_catcodes();
    let glue_spec = universe.intern_glue(GlueSpec {
        width: Scaled::from_raw(65536 * 5),
        ..GlueSpec::ZERO
    });
    universe.update_page_last_from_node(&Node::Glue {
        spec: glue_spec,
        kind: tex_state::node::GlueKind::Normal,
        leader: None,
    });
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\showthe\lastskip\showthe\lastpenalty");
    run_to_end(&mut control, &mut universe);

    let text = terminal_text(&universe);
    assert!(text.contains("> 5.0pt."), "{text}");
    assert!(text.contains("> 0."), "{text}");
}

#[test]
fn canonical_last_skip_reads_an_explicit_mskip_at_mu_val_level() {
    // TeX82 §424: `if subtype(tail)=mu_glue then cur_val_level:=mu_val` --
    // an explicit `\mskip`-shaped glue node (`GlueKind::MuSkip`) renders in
    // mu units through `\the`, unlike ordinary glue.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    control.modes.push(Mode::Math);
    let glue_spec = universe.intern_glue(GlueSpec {
        width: Scaled::from_raw(65536 * 2),
        ..GlueSpec::ZERO
    });
    control.modes.current_list_mutation().push(Node::Glue {
        spec: glue_spec,
        kind: tex_state::node::GlueKind::MuSkip,
        leader: None,
    });
    register_source(&mut control, br"\showthe\lastskip");
    run_to_end(&mut control, &mut universe);

    assert!(
        terminal_text(&universe).contains("> 2.0mu."),
        "{}",
        terminal_text(&universe)
    );
}

#[test]
fn canonical_italic_correction_appends_kern_even_when_zero() {
    // TeX82 §1113's `append_italic_correction` appends the kern
    // unconditionally when the tail is a character node, even when the
    // correction happens to be exactly zero -- there is no width guard in
    // tex.web. This is a regression test: the ported logic used to skip the
    // append whenever the metric was zero.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_cmr10_font(&mut control, &mut universe);
    register_source(&mut control, br"\font\f=cmr10 \setbox0=\hbox{\f A\/}");
    run_to_end(&mut control, &mut universe);

    let hbox = universe
        .box_reg(0)
        .and_then(|id| universe.nodes(id).first().map(|node| node.to_owned()))
        .expect("outer hbox stores");
    let Node::HList(hbox) = hbox else {
        panic!("setbox0 contains an hbox");
    };
    let children = universe.nodes(hbox.children).to_vec();
    assert!(
        matches!(
            children.last(),
            Some(Node::Kern {
                amount,
                kind: KernKind::Explicit,
            }) if *amount == Scaled::from_raw(0)
        ),
        "a zero-width explicit kern still lands after `A`: {children:?}"
    );
}

#[test]
fn canonical_italic_correction_uses_the_font_metric_amount() {
    // Same TeX82 §1113 path as above, but `f` has a real (nonzero) italic
    // correction in cmr10 -- the classic textbook example.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_cmr10_font(&mut control, &mut universe);
    register_source(&mut control, br"\font\f=cmr10 \setbox0=\hbox{\f f\/}");
    run_to_end(&mut control, &mut universe);

    let hbox = universe
        .box_reg(0)
        .and_then(|id| universe.nodes(id).first().map(|node| node.to_owned()))
        .expect("outer hbox stores");
    let Node::HList(hbox) = hbox else {
        panic!("setbox0 contains an hbox");
    };
    let children = universe.nodes(hbox.children).to_vec();
    assert!(
        matches!(
            children.last(),
            Some(Node::Kern {
                amount,
                kind: KernKind::Explicit,
            }) if amount.raw() > 0
        ),
        "`f`'s nonzero italic correction lands as an explicit kern: {children:?}"
    );
}

#[test]
fn canonical_italic_correction_in_math_mode_appends_a_font_kind_zero_kern() {
    // TeX82 §1112's `mmode+ital_corr: tail_append(new_kern(0))` never
    // overrides `new_kern`'s default `normal` subtype the way hmode's
    // italic-correction kern (or an explicit `\kern`) does, so it must not
    // become a legal kern-then-glue line-break point. `KernKind::Font` is
    // Umber's non-breakpoint kern kind (see `linebreak::mod`'s
    // `KernKind::Explicit | KernKind::Mu` break-legality check).
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\setbox0=\hbox{$\/$}");
    run_to_end(&mut control, &mut universe);

    let hbox = universe
        .box_reg(0)
        .and_then(|id| universe.nodes(id).first().map(|node| node.to_owned()))
        .expect("outer hbox stores");
    let Node::HList(hbox) = hbox else {
        panic!("setbox0 contains an hbox");
    };
    let children = universe.nodes(hbox.children).to_vec();
    assert!(
        children.iter().any(|node| matches!(
            node,
            Node::Kern {
                amount,
                kind: KernKind::Font,
            } if *amount == Scaled::from_raw(0)
        )),
        "math-mode \\/ appends a zero kern with the non-breakpoint `Font` kind: {children:?}"
    );
}

#[test]
fn canonical_italic_correction_in_vertical_mode_reports_illegal_case() {
    // TeX82 §1111's "Forbidden cases": `vmode+ital_corr` never starts a
    // paragraph the way most other hmode-triggering commands do; it is a
    // plain `report_illegal_case` diagnostic, and the following text is
    // therefore never scanned as an operand.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\/\end");
    run_to_end(&mut control, &mut universe);

    let text = terminal_text(&universe);
    assert!(
        text.contains("You can't use `\\/' in vertical mode."),
        "{text}"
    );
}

#[test]
fn canonical_italic_correction_is_illegal_in_internal_vertical_mode_too() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\setbox0=\vbox{\/X}\end");
    run_to_end(&mut control, &mut universe);

    let text = terminal_text(&universe);
    assert!(
        text.contains("You can't use `\\/' in internal vertical mode."),
        "{text}"
    );
    assert!(
        universe.box_reg(0).is_some(),
        "the following token remains available after the operand-free error"
    );
}

#[test]
fn canonical_italic_correction_respects_right_noboundary_before_metric_lookup() {
    // TeX82 §§1038/1113: an ordinary run flush may append right-boundary
    // material, in which case the tail is no longer a character and \/ does
    // nothing. A consumed \noboundary suppresses that material, leaving the
    // character at the tail for §1113's metric lookup.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_boundary_probe_font(&mut control, &mut universe);
    register_source(
        &mut control,
        br"\font\f=boundary-probe
           \setbox0=\hbox{\f A\/}
           \setbox1=\hbox{\f A\noboundary\/}\end",
    );
    run_to_end(&mut control, &mut universe);

    let children = |register| {
        let hbox = universe
            .box_reg(register)
            .and_then(|id| universe.nodes(id).first().map(|node| node.to_owned()))
            .expect("box register stores");
        let Node::HList(hbox) = hbox else {
            panic!("box register contains an hbox");
        };
        universe.nodes(hbox.children).to_vec()
    };
    let with_boundary = children(0);
    let without_boundary = children(1);
    assert!(matches!(
        with_boundary.as_slice(),
        [
            Node::Char { ch: 'A', .. },
            Node::Kern {
                kind: KernKind::Font,
                ..
            }
        ]
    ));
    assert!(matches!(
        without_boundary.as_slice(),
        [
            Node::Char { ch: 'A', .. },
            Node::Kern {
                amount,
                kind: KernKind::Explicit,
            }
        ] if *amount == Scaled::from_raw(0)
    ));
}

#[test]
fn prefix_before_italic_correction_recovers_without_losing_following_input() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_cmr10_font(&mut control, &mut universe);
    register_source(
        &mut control,
        br"\font\f=cmr10\setbox0=\hbox{\f f\global\/X}\end",
    );
    run_to_end(&mut control, &mut universe);

    assert!(
        terminal_text(&universe).contains("You can't use a prefix with `\\/'."),
        "{}",
        terminal_text(&universe)
    );
    let hbox = universe
        .box_reg(0)
        .and_then(|id| universe.nodes(id).first().map(|node| node.to_owned()))
        .expect("box register stores");
    let Node::HList(hbox) = hbox else {
        panic!("box register contains an hbox");
    };
    let children = universe.nodes(hbox.children).to_vec();
    assert!(children.iter().any(|node| matches!(
        node,
        Node::Kern {
            kind: KernKind::Explicit,
            ..
        }
    )));
    assert!(
        children
            .iter()
            .any(|node| matches!(node, Node::Char { ch: 'X', .. }))
    );
}

#[test]
fn canonical_ignorespaces_skips_spaces_before_the_next_command() {
    // TeX82 §1045's `any_mode(ignore_spaces)`: repeated `get_x_token` skips
    // spaces, then the first non-space command is reprocessed in place --
    // it must not itself become an interword-glue-triggering space.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_cmr10_font(&mut control, &mut universe);
    register_source(
        &mut control,
        br"\font\f=cmr10 \setbox0=\hbox{\f \ignorespaces   A}",
    );
    run_to_end(&mut control, &mut universe);

    let hbox = universe
        .box_reg(0)
        .and_then(|id| universe.nodes(id).first().map(|node| node.to_owned()))
        .expect("outer hbox stores");
    let Node::HList(hbox) = hbox else {
        panic!("setbox0 contains an hbox");
    };
    let children = universe.nodes(hbox.children).to_vec();
    assert!(
        matches!(children.as_slice(), [Node::Char { ch: 'A', .. }]),
        "the skipped spaces left no glue behind: {children:?}"
    );
}

#[test]
fn canonical_ignorespaces_reswitches_without_backing_the_next_command_up() {
    // TeX82 §1045's `goto reswitch` dispatches the command §406's
    // `repeat get_x_token until cur_cmd<>spacer` already fetched, from the
    // `reswitch:` label §1030 places *above* `main_control`'s big case. No
    // input level is pushed and the command is delivered exactly once;
    // `back_input` in its place emits a backup push, a recovery record, a
    // duplicate raw/expanded delivery pair, and a backup retirement that the
    // pinned oracle never records (`umber2-johp.196`).
    // `\relax` is §1045's neighbouring `do_nothing` case: it scans nothing, so
    // the only backup this run could produce is the one under test.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\ignorespaces   \relax");
    let mut observations = ObservationRecorder::default();
    loop {
        match control
            .step_with_observer(&mut universe, &mut observations)
            .expect("canonical program executes")
        {
            MainControlStep::End | MainControlStep::EndOfInput => break,
            MainControlStep::Continue => {}
        }
    }

    assert!(
        !observations.0.iter().any(|observation| matches!(
            observation,
            CommandObservation::Input(record) if record.reason == InputReason::Backup
        )),
        "§1045 reswitches in place and pushes no backup level: {:?}",
        observations.0
    );
    let relax_deliveries = observations
        .0
        .iter()
        .filter(|observation| {
            matches!(
                observation,
                CommandObservation::Command(delivery)
                    if matches!(&delivery.spelling, ObservedToken::ControlSequence(name) if name == "relax")
            )
        })
        .count();
    assert_eq!(
        relax_deliveries, 2,
        "the reswitched command is delivered once raw and once expanded: {:?}",
        observations.0
    );
}

#[test]
fn canonical_ignorespaces_is_mode_complete_and_preserves_the_next_command() {
    // TeX82 §1045 declares `any_mode(ignore_spaces)`. The first non-spacer
    // command is dispatched by `reswitch` in the same step in all six Umber
    // mode projections; it is not consumed as an operand or deferred.
    for mode in [
        Mode::Vertical,
        Mode::InternalVertical,
        Mode::Horizontal,
        Mode::RestrictedHorizontal,
        Mode::Math,
        Mode::DisplayMath,
    ] {
        let mut universe = Universe::new_with_plain_catcodes();
        let mut control = CanonicalMainControl::tex82_initex(&mut universe);
        if mode != Mode::Vertical {
            control.modes.push(mode);
        }
        register_source(&mut control, br"\ignorespaces   \global\count0=17 ");
        run_to_end(&mut control, &mut universe);

        assert_eq!(universe.count(0), 17, "following assignment in {mode:?}");
        assert!(
            !terminal_text(&universe).contains("Unimplemented primitive"),
            "{mode:?} reached the shared §1045 route"
        );
    }
}

#[test]
fn canonical_ignorespaces_expands_macros_while_skipping_and_keeps_relax() {
    // §§406/1045 use `get_x_token`: macro-produced spacer commands disappear
    // across replacement-list retirement. `\relax` is the negative control
    // distinguishing §406 from §404; it stops the scan and is dispatched as
    // §1045's neighbouring no-op before the later assignment.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\def\spaces{   }\ignorespaces\spaces\relax\global\count0=23 ",
    );
    run_to_end(&mut control, &mut universe);

    assert_eq!(universe.count(0), 23);
    assert!(!terminal_text(&universe).contains('!'));
}

#[test]
fn canonical_ignorespaces_crosses_nested_source_retirement() {
    // §406's repeated `get_x_token` is oblivious to physical source levels:
    // after a nested `\input` contributes only spacer commands and retires,
    // the first non-spacer in the parent is still the command reswitched.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    install_input(&mut universe);
    control.capabilities_mut().register_input(
        "spaces",
        SourceRegistration::new(
            RegisteredSourceKind::World,
            Arc::<[u8]>::from(b"   ".as_slice()),
        ),
    );
    register_source(
        &mut control,
        br"\ignorespaces\input spaces \global\count0=29 ",
    );
    run_to_end(&mut control, &mut universe);

    assert_eq!(universe.count(0), 29);
}

#[test]
fn canonical_ignorespaces_at_eof_ends_without_recovery() {
    // §406 has no backup or missing-token recovery. If expanded delivery
    // reaches terminal EOF while looking for a non-spacer, main control ends
    // normally with no invented command.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\ignorespaces   ");
    run_to_end(&mut control, &mut universe);

    assert!(!terminal_text(&universe).contains('!'));
}

#[test]
fn prefix_before_ignorespaces_is_rejected_before_ignorespaces_runs() {
    // `\ignorespaces` is `any_mode`, not a prefixed command. TeX82
    // §§1211-1212 discard the erroneous `\global`, back up `\ignorespaces`,
    // and only then let §1045 skip spaces. The following local assignment
    // must therefore remain local and disappear at group exit.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"{\global\ignorespaces   \count0=31 }\relax");
    run_to_end(&mut control, &mut universe);

    assert_eq!(universe.count(0), 0);
    assert!(terminal_text(&universe).contains("You can't use a prefix with `\\ignorespaces'."));
}

#[test]
fn canonical_noboundary_in_vertical_mode_starts_a_paragraph() {
    // TeX82 §1090's `vmode+no_boundary` groups with `vmode+ex_space` and
    // friends: `back_input; new_graf(true)`.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\setbox0=\vbox{\noboundary}");
    run_to_end(&mut control, &mut universe);

    let vbox = universe
        .box_reg(0)
        .and_then(|id| universe.nodes(id).first().map(|node| node.to_owned()))
        .expect("outer vbox stores");
    let Node::VList(vbox) = vbox else {
        panic!("setbox0 contains a vbox");
    };
    let children = universe.nodes(vbox.children).to_vec();
    assert!(
        matches!(children.first(), Some(Node::HList(_))),
        "\\noboundary started a paragraph, indenting the (empty) first line: {children:?}"
    );
}

#[test]
fn canonical_noboundary_in_math_mode_is_a_no_op() {
    // TeX82 §1045's `mmode+no_boundary: do_nothing`.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"$\noboundary$\end");
    run_to_end(&mut control, &mut universe);

    assert!(
        !terminal_text(&universe).contains('!'),
        "no diagnostics expected: {}",
        terminal_text(&universe)
    );
}

#[test]
fn canonical_noboundary_suppresses_left_and_right_boundaries_independently() {
    // TeX82 §§1030 and 1038: the first two boxes exercise the left-boundary
    // kern (`boundary+C`), while the second pair exercise the right boundary
    // kern (`A+boundary`).
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_boundary_probe_font(&mut control, &mut universe);
    register_source(
        &mut control,
        br"\font\f=boundary-probe
           \setbox0=\hbox{\f C}\setbox1=\hbox{\f\noboundary C}
           \setbox2=\hbox{\f A}\setbox3=\hbox{\f A\noboundary}\end",
    );
    run_to_end(&mut control, &mut universe);

    let boundary_kerns = |register| {
        box_children(&universe, register)
            .into_iter()
            .filter(|node| {
                matches!(
                    node,
                    Node::Kern {
                        kind: KernKind::Font,
                        ..
                    }
                )
            })
            .count()
    };
    assert_eq!((boundary_kerns(0), boundary_kerns(1)), (1, 0));
    assert_eq!((boundary_kerns(2), boundary_kerns(3)), (1, 0));
}

#[test]
fn canonical_noboundary_noncharacter_lookahead_has_no_lingering_effect() {
    // §1030 sets cancel_boundary only for letter/other/char_given/char_num.
    // A relax is reswitched in place and the later C therefore retains its
    // ordinary left-boundary kern.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_boundary_probe_font(&mut control, &mut universe);
    register_source(
        &mut control,
        br"\font\f=boundary-probe\setbox0=\hbox{\f\noboundary\relax C}\end",
    );
    run_to_end(&mut control, &mut universe);

    assert!(box_children(&universe, 0).into_iter().any(|node| matches!(
        node,
        Node::Kern {
            kind: KernKind::Font,
            ..
        }
    )));
}

#[test]
fn canonical_noboundary_character_forms_and_expansion_preserve_the_following_command() {
    // §1030 uses get_x_token, then recognizes all four main-loop character
    // entries. The expanded macro, char_given, and char_num cases must each
    // execute once with their original character and suppress the left edge.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_boundary_probe_font(&mut control, &mut universe);
    register_source(
        &mut control,
        br"\font\f=boundary-probe\chardef\c=67 \def\m{C}
           \setbox0=\hbox{\f\noboundary\m}
           \setbox1=\hbox{\f\noboundary\c}
           \setbox2=\hbox{\f\noboundary\char67 }\end",
    );
    run_to_end(&mut control, &mut universe);

    for register in 0..=2 {
        let glyphs = box_children(&universe, register)
            .into_iter()
            .filter_map(|node| match node {
                Node::Char { ch, .. } | Node::Lig { ch, .. } => Some(ch),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(glyphs, vec!['C'], "box {register}");
    }
}

#[test]
fn prefix_before_noboundary_recovers_then_preserves_its_lookahead() {
    // §§1211-1212 reject the prefix, back up \noboundary, and later execute
    // it normally. The following C is neither lost nor delivered twice.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_boundary_probe_font(&mut control, &mut universe);
    register_source(
        &mut control,
        br"\font\f=boundary-probe\setbox0=\hbox{\f\global\noboundary C}\end",
    );
    run_to_end(&mut control, &mut universe);

    assert!(terminal_text(&universe).contains("You can't use a prefix with `\\noboundary'."));
    let glyphs = box_children(&universe, 0)
        .into_iter()
        .filter(|node| matches!(node, Node::Char { .. } | Node::Lig { .. }))
        .count();
    assert_eq!(glyphs, 1);
}

#[test]
fn canonical_nonscript_appends_a_zero_glue_in_math_mode() {
    // TeX82 §1171's `mmode+non_script: tail_append(new_glue(zero_glue));
    // subtype(tail):=cond_math_glue`.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\setbox0=\hbox{$\nonscript\kern1pt$}");
    run_to_end(&mut control, &mut universe);

    let hbox = universe
        .box_reg(0)
        .and_then(|id| universe.nodes(id).first().map(|node| node.to_owned()))
        .expect("outer hbox stores");
    let Node::HList(hbox) = hbox else {
        panic!("setbox0 contains an hbox");
    };
    let children = universe.nodes(hbox.children).to_vec();
    assert!(
        children.iter().any(|node| matches!(
            node,
            Node::Glue {
                kind: tex_state::node::GlueKind::NonScript,
                ..
            }
        )),
        "the nonscript glue reaches the finished hlist: {children:?}"
    );
}

#[test]
fn canonical_nonscript_outside_math_mode_inserts_missing_dollar_sign() {
    // TeX82 §1046's `non_math(non_script)`: `insert_dollar_sign` recovers by
    // opening math mode and reconsidering `\nonscript` there, exactly like
    // the existing `\vskip`-in-math-mode recovery this reuses
    // (`recover_missing_math_shift` is generic over the offending command).
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\setbox0=\hbox{\nonscript\kern1pt$}");
    run_to_end(&mut control, &mut universe);

    let text = terminal_text(&universe);
    assert!(text.contains("Missing $ inserted."), "{text}");
}

// umber2-johp.79: TeX82 §1046's `non_math(...)` table also lists every
// math-noad, math-style, and math-delimiter primitive that
// `scan_canonical_math_request` (or the `\left`/`\right`/`\middle` gate)
// otherwise dispatches only under `Mode::Math`/`DisplayMath`. Each of these
// covers a structurally distinct scan shape (no operand, a scalar integer, a
// paired delimiter boundary, and an `mu` dimension) to demonstrate the shared
// `recover_missing_math_shift` category, not just the single primitive
// `non_math(non_script)` above already happened to cover.

#[test]
fn canonical_displaystyle_outside_math_mode_inserts_missing_dollar_sign() {
    // `\displaystyle` takes no operand at all: recovery must fire before any
    // scan is attempted, purely from the primitive's identity.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\setbox0=\hbox{\displaystyle$}");
    run_to_end(&mut control, &mut universe);

    let text = terminal_text(&universe);
    assert!(text.contains("Missing $ inserted."), "{text}");
}

#[test]
fn canonical_mathchar_outside_math_mode_inserts_missing_dollar_sign() {
    // `\mathchar` scans a 15-bit integer constant (§1046's
    // `non_math(math_char_num)`); the recovered replay must still be able to
    // complete that scan once math mode is open.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br#"\setbox0=\hbox{\mathchar"41 $}"#);
    run_to_end(&mut control, &mut universe);

    let text = terminal_text(&universe);
    assert!(text.contains("Missing $ inserted."), "{text}");
}

#[test]
fn canonical_left_outside_math_mode_inserts_missing_dollar_sign() {
    // `\left`/`\right`/`\middle` have their own early gate for entering
    // `scan_math_delimiter_boundary` once already in math mode (§1046's
    // `non_math(left_right)`), separate from `scan_canonical_math_request`;
    // this proves that gate's recovery is likewise generic outside math mode.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\setbox0=\hbox{\left.\right.$}");
    run_to_end(&mut control, &mut universe);

    let text = terminal_text(&universe);
    assert!(text.contains("Missing $ inserted."), "{text}");
}

#[test]
fn canonical_mkern_outside_math_mode_inserts_missing_dollar_sign() {
    // `\mkern` scans an `mu` dimension (§1046's `non_math(mkern)`), a
    // distinct unit family from the ordinary dimension `\kern` scans.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\setbox0=\hbox{\mkern5mu $}");
    run_to_end(&mut control, &mut universe);

    let text = terminal_text(&universe);
    assert!(text.contains("Missing $ inserted."), "{text}");
}

#[test]
fn canonical_globaldefs_forces_and_suppresses_global_assignments() {
    // TeX82 §1211's `prefixed_command` resolves every assignment's effective
    // global bit from the live `\globaldefs` value before mutating, the same
    // `assignment_global` helper already used by ordinary register/parameter
    // assignments (canonical_main_control.rs) -- regression test for
    // umber2-johp.83: `\def`'s and `\let`'s apply arms used the raw `\global`
    // prefix bit directly and silently ignored a nonzero `\globaldefs`.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"{\globaldefs=1 \def\a{A}\globaldefs=-1 \gdef\b{B}\globaldefs=1 \let\c=\relax}",
    );
    run_to_end(&mut control, &mut universe);

    let a = universe.symbol("a").expect("a");
    let b = universe.symbol("b").expect("b");
    let c = universe.symbol("c").expect("c");

    // The group (and its closing brace) has already fully run by this point.
    // `\a` was forced global by `\globaldefs=1`, so its `\def` survives group
    // exit; `\b`'s `\gdef` was forced back to local by `\globaldefs=-1`, so it
    // does not; `\c`'s `\let` was forced global by `\globaldefs=1` again.
    assert!(matches!(universe.meaning(a), Meaning::Macro { .. }));
    assert_eq!(universe.meaning(b), Meaning::Undefined);
    assert_eq!(universe.meaning(c), Meaning::Relax);
}

#[test]
fn canonical_interaction_mode_primitives_set_the_live_mode() {
    // TeX82 §1264's `new_interaction`: `\batchmode`/`\nonstopmode`/
    // `\scrollmode`/`\errorstopmode` each set `interaction` directly from
    // their own fixed `chr_code`, with no operand scan of their own.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    assert_eq!(
        universe.interaction_mode(),
        tex_state::InteractionMode::ErrorStop
    );

    register_source(&mut control, br"\batchmode");
    run_to_end(&mut control, &mut universe);
    assert_eq!(
        universe.interaction_mode(),
        tex_state::InteractionMode::Batch
    );

    register_source(&mut control, br"\nonstopmode");
    run_to_end(&mut control, &mut universe);
    assert_eq!(
        universe.interaction_mode(),
        tex_state::InteractionMode::Nonstop
    );

    register_source(&mut control, br"\scrollmode");
    run_to_end(&mut control, &mut universe);
    assert_eq!(
        universe.interaction_mode(),
        tex_state::InteractionMode::Scroll
    );

    register_source(&mut control, br"\errorstopmode");
    run_to_end(&mut control, &mut universe);
    assert_eq!(
        universe.interaction_mode(),
        tex_state::InteractionMode::ErrorStop
    );
}

#[test]
fn production_driver_math_choice_inside_alignment_cell_retires_cleanly() {
    // Regression test for umber2-johp.93: a `\mathchoice` (as plain.tex's
    // `\mathstrut`/`\mathpalette` build via `\vphantom`) whose branches
    // conclude with a `\chardef`'d register operand (no scan_int trailing-
    // space lookahead of its own, unlike an ordinary digit sequence) reached
    // inside an alignment cell's own inline math. Each branch is its own
    // executor-owned replay episode (`execute_math_group`); its retirement
    // must be detected by `scan_alignment_delivery_step` exactly like
    // ordinary (non-alignment) `scan_step` already detects an episode's
    // completion, rather than silently cascading past it (via
    // `CommandProcessor::get_x_alignment_delivery`'s former plain `get_next`)
    // and misattributing the alignment cell's own closing `$` to the
    // just-retired branch. Before the fix, this panicked in
    // `tex-exec::align::widths::debug::debug_assert_no_unset_node` ("unset
    // node escaped fin_align") because the misattributed `$` prematurely
    // popped and converted the branch's own math level, throwing off the
    // mode nest so the alignment's row/cell structure never packaged
    // correctly.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\chardef\myreg=0\setbox\myreg=\hbox{}$$\halign{#\cr$\mathchoice{\box\myreg}{\box\myreg}{\box\myreg}{\box\myreg}$\cr}$$\end",
    );
    run_to_end(&mut control, &mut universe);
}

#[test]
fn canonical_math_group_spanning_v_template_does_not_redeliver_its_row_terminator() {
    // plain.tex's `\eqalign` (and `\displaylines`) wraps every cell field in
    // `$\displaystyle{##}$`, and its second-column template additionally
    // nests an empty group before the field: `$\displaystyle{{}##}$`. That
    // outer `{` is a bare `mmode+left_brace` (TeX82 §1153), whose *matching*
    // `}` lives in the column's `v_j` template -- i.e. on the far side of
    // the row's `\cr`. A `scan_toks` collection that starts inside such a
    // cell (crates/tex-command/src/scan_toks.rs) must scan straight
    // through that `\cr`; TeX82 §790's `insert_vj` intercepts it and inserts
    // `v_j` exactly once, never letting the delimiter surface as ordinary
    // scanned text (§343, `car_ret`/`tab_mark` at `align_state=0`). Before
    // the fix in crates/tex-command/src/scan_toks.rs, the collector treated
    // the intercepted delimiter as literal content, capturing it into the
    // group's replay episode; replaying that episode then redelivered the
    // same `\cr` a second time, after `align_state` had already moved past
    // the point where interception is recognized, so it fell through to
    // ordinary primitive dispatch and errored as
    // `ExecError::UnimplementedPrimitive` in Math mode.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"$$\halign{#&$\displaystyle{{}#}$\cr a&b\cr x&y\cr e&f\cr}$$\end",
    );
    run_to_end(&mut control, &mut universe);
    assert_eq!(control.current_mode(), crate::Mode::Vertical);
}

#[test]
fn canonical_display_alignment_discards_a_preceding_formula() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"$$A\over B\begingroup\halign{#\cr a\cr}$$\end",
    );
    run_to_end(&mut control, &mut universe);
    let output = terminal_text(&universe);
    assert!(
        output.contains("Missing \\endgroup inserted"),
        "alignment first restores the enclosing display-math group"
    );
    assert!(
        output.contains("Improper \\halign inside $$'s"),
        "display alignment reports and flushes its preceding formula"
    );
    assert_eq!(control.current_mode(), crate::Mode::Vertical);
}

#[test]
fn canonical_math_shift_closes_nested_math_groups_before_finishing_math() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\noindent${A$\end");
    run_to_end(&mut control, &mut universe);
    assert!(
        terminal_text(&universe).contains("Missing } inserted"),
        "the nested math group is closed before the shift is retried"
    );
    assert_eq!(control.current_mode(), crate::Mode::Vertical);
}

#[test]
fn canonical_interaction_mode_assignment_is_ungrouped() {
    // `interaction` is a plain global Pascal variable outside `eqtb`
    // (tex.web's globals), so `\batchmode` inside a group is never undone at
    // group exit -- unlike an ordinary local assignment.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"{\batchmode}");
    run_to_end(&mut control, &mut universe);
    assert_eq!(
        universe.interaction_mode(),
        tex_state::InteractionMode::Batch
    );
}

#[test]
fn canonical_etex_interaction_mode_is_both_internal_and_assignable() {
    // e-TeX 2.6 etex.ch §3736 adds chr_code 2 to `set_page_int`: the same
    // primitive fetches the live interaction scalar in an integer scan and
    // assigns a scanned replacement when delivered by main control.
    let mut universe = Universe::new_with_plain_catcodes();
    tex_expand::install_expandable_primitives(&mut universe);
    tex_expand::install_etex_expandable_primitives(&mut universe);
    crate::install_unexpandable_primitives(&mut universe);
    crate::install_etex_unexpandable_primitives(&mut universe);
    let mut control = CanonicalMainControl::prepared_initex(tex_command::CommandProfile::ETEX26);
    register_source(
        &mut control,
        br"\count20=\interactionmode \interactionmode=1 \count21=\interactionmode\end",
    );
    run_to_end(&mut control, &mut universe);

    assert_eq!(universe.count(20), 3);
    assert_eq!(universe.count(21), 1);
    assert_eq!(
        universe.interaction_mode(),
        tex_state::InteractionMode::Nonstop
    );
}

#[test]
fn canonical_etex_saved_vertical_discards_do_not_block_format_dump() {
    // e-TeX 2.6 etex.ch [45.999] saves discarded vertical nodes, while
    // TeX82 §1335 releases the page builder's transient `last_glue` before
    // `store_fmt_file`; neither saved-discard list belongs to the format.
    let mut universe = Universe::new_with_plain_catcodes();
    tex_expand::install_expandable_primitives(&mut universe);
    tex_expand::install_etex_expandable_primitives(&mut universe);
    crate::install_unexpandable_primitives(&mut universe);
    crate::install_etex_unexpandable_primitives(&mut universe);
    let mut control = CanonicalMainControl::prepared_initex(tex_command::CommandProfile::ETEX26);
    register_source(
        &mut control,
        br"\bgroup\savingvdiscards=1
           \vfill\penalty1234
           \setbox0=\vbox{\vbox to10pt{}\vskip5pt\penalty-4321}
           \setbox1=\vsplit0 to10pt
           \egroup\dump",
    );
    let mut observations = ObservationRecorder::default();
    loop {
        match control
            .step_with_observer(&mut universe, &mut observations)
            .expect("saved-discard microfixture executes")
        {
            MainControlStep::End | MainControlStep::EndOfInput => break,
            MainControlStep::Continue => {}
        }
    }

    assert!(control.dumped_format());
    assert!(
        universe.page_contributions().is_empty(),
        "live contributions: {:?}",
        universe.page_contributions()
    );
    assert!(
        universe.current_page_nodes().is_empty(),
        "current page: {:?}",
        universe.current_page_nodes()
    );
    assert!(!universe.page_discards().is_empty());
    assert!(!universe.split_discards().is_empty());
    let delivered = observations
        .0
        .iter()
        .filter_map(|observation| match observation {
            CommandObservation::Command(record) => match &record.spelling {
                ObservedToken::ControlSequence(name) => Some(name.as_str()),
                _ => None,
            },
            _ => None,
        })
        .collect::<Vec<_>>();
    for command in ["vfill", "penalty", "vsplit", "dump"] {
        assert!(
            delivered.contains(&command),
            "bounded semantic stream omitted {command}: {delivered:?}"
        );
    }
    universe
        .dump_format()
        .expect("saved vertical discards are not format state");
}

#[test]
fn canonical_the_and_showthe_recover_invalid_trip_operand_as_zero() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\edef\fromthe{\the$}\showthe$\end");
    run_to_end(&mut control, &mut universe);
    let fromthe = universe.intern("fromthe").symbol();
    let meaning = universe.macro_meaning(fromthe).expect("macro is defined");
    assert_eq!(
        replay_text(universe.tokens(meaning.replacement_text())),
        "0"
    );
    let output = terminal_text(&universe);
    assert_eq!(output.matches("after \\the").count(), 2);
    assert!(output.contains("\n> 0.\n"));
}

/// TeX82 §1210 files `prefix` under `any_mode`, and §1211's
/// `while cur_cmd=prefix` loop runs inside `prefixed_command` -- reached from
/// the same `main_control` big case an alignment cell's body runs through.
/// An alignment cell is bounded by §785's `align_peek` and §1130's `endv`,
/// not dispatched by a narrowed main control of its own, so `\global` inside
/// a cell must reach the assignment with its prefix intact
/// (`umber2-johp.208`).
#[test]
fn alignment_cell_body_collects_the_global_prefix() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\halign{#\cr\begingroup\global\count9=5 \endgroup\cr}\end",
    );
    run_to_end(&mut control, &mut universe);
    assert_eq!(universe.count(9), 5);
}

/// The same for a `\noalign` body, which §785 opens as an ordinary
/// `no_align_group` running plain `main_control` between its braces.
#[test]
fn noalign_body_collects_the_global_prefix() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\halign{#\cr\noalign{\begingroup\global\count9=5 \endgroup}a\cr}\end",
    );
    run_to_end(&mut control, &mut universe);
    assert_eq!(universe.count(9), 5);
}

/// §1045's `any_mode(ignore_spaces)` is the other command tex.web consumes
/// above its big case, so it takes the same shared main-control step and must
/// not reach `scan_command` from an alignment cell either.
#[test]
fn alignment_cell_body_handles_ignore_spaces() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(&mut control, br"\halign{#\cr\ignorespaces a\cr}\end");
    run_to_end(&mut control, &mut universe);
}

/// tex.web §1085's `handle_right_brace` runs `end_graf` (§1096) for
/// `vbox_group` and `vtop_group` -- and only for those two -- before
/// `package`, so a paragraph still open at the box's closing brace is
/// line-broken into the box's own vertical list rather than being packaged as
/// if its hlist material were the box body (`umber2-johp.232`).
#[test]
fn vertical_box_group_end_runs_end_graf_before_packaging() {
    for opener in [&b"\\vbox"[..], &b"\\vtop"[..]] {
        let mut universe = Universe::new_with_plain_catcodes();
        let mut control = CanonicalMainControl::tex82_initex(&mut universe);
        register_cmr10_font(&mut control, &mut universe);
        let mut source = b"\\font\\f=cmr10 \\f \\hsize=100pt \\setbox1=".to_vec();
        source.extend_from_slice(opener);
        source.extend_from_slice(b"{\\noindent A}");
        register_source(&mut control, &source);
        run_to_end(&mut control, &mut universe);

        let stored = universe
            .box_reg(1)
            .and_then(|id| universe.nodes(id).first().map(|node| node.to_owned()))
            .expect("setbox1 stores a vertical box");
        let Node::VList(stored) = stored else {
            panic!("setbox1 contains a vertical box");
        };
        let children: Vec<_> = universe
            .nodes(stored.children)
            .iter()
            .map(|node| node.to_owned())
            .collect();
        let [Node::HList(line)] = children.as_slice() else {
            panic!("the finished paragraph contributes exactly one line box, got {children:?}");
        };
        assert_eq!(
            line.width,
            Scaled::from_raw(100 * 65_536),
            "the line box was packaged to \\hsize by the line breaker"
        );
        assert!(
            universe
                .nodes(line.children)
                .iter()
                .any(|node| matches!(node.to_owned(), Node::Char { ch: 'A', .. })),
            "the paragraph's character reached the line box"
        );
    }
}

/// tex.web §1200's `resume_after_display` ends with §443's
/// `@<Scan an optional space@>`. Without it the space following a closing
/// `$$` becomes interword glue, the resumed paragraph is no longer null, and
/// §1096's `if head=tail then pop_nest {null paragraphs are ignored}` never
/// fires -- leaving an extra empty line box and its interline glue in the
/// enclosing vertical list (`umber2-johp.231`).
#[test]
fn display_resumption_scans_tex82_s1200_optional_space() {
    // Both variants must produce the same vertical list: with the optional
    // space consumed, the trailing `\par` sees a null paragraph either way.
    let mut lists = Vec::new();
    for body in [
        &br"\setbox1=\vbox{\hsize=100pt\noindent\hbox{}$$\hbox{}$$\par}"[..],
        &br"\setbox1=\vbox{\hsize=100pt\noindent\hbox{}$$\hbox{}$$ \par}"[..],
    ] {
        let mut universe = Universe::new_with_plain_catcodes();
        let mut control = CanonicalMainControl::tex82_initex(&mut universe);
        register_source(&mut control, body);
        run_to_end(&mut control, &mut universe);

        let stored = universe
            .box_reg(1)
            .and_then(|id| universe.nodes(id).first().map(|node| node.to_owned()))
            .expect("setbox1 stores a vbox");
        let Node::VList(stored) = stored else {
            panic!("setbox1 contains a vbox");
        };
        lists.push(
            universe
                .nodes(stored.children)
                .iter()
                .map(|node| vertical_node_shape(&node.to_owned()))
                .collect::<Vec<_>>(),
        );
    }
    assert_eq!(
        lists[0], lists[1],
        "a space between the closing $$ and \\par is consumed by §1200, so it \
         cannot make the resumed paragraph non-null"
    );
    assert!(
        !lists[1]
            .last()
            .is_some_and(|node| node.starts_with("hlist")),
        "the resumed null paragraph contributes no line box: {:?}",
        lists[1]
    );
}

/// tex.web §1200's `resume_after_display` ends with
/// `if nest_ptr=1 then build_page`, and §1005's `fire_up` inside §994's
/// `build_page` reaches §1025's `begin_token_list(output_routine,output_text)`
/// before `build_page` returns. So the page the closing `$$` overfills enters
/// `\output` inside that same command, ahead of whatever token follows the
/// display -- not one command later (`umber2-johp.237`).
#[test]
fn display_resumption_enters_output_before_the_next_command() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        // A 20pt box fills two thirds of a 30pt page; the display contributes
        // a second one, so the penalty after it is the first breakpoint whose
        // cost is `awful_bad`. `\count2` records whether `\output` had
        // already run when the command right after `$$` executed.
        br"\output={\global\count1=1 \global\setbox9=\box255}\topskip=0pt\vsize=30pt\maxdepth=2pt
           \setbox0=\hbox{}\ht0=20pt \copy0
           \noindent$$\copy0$$\global\count2=\count1 \end",
    );

    run_to_end(&mut control, &mut universe);

    assert_eq!(
        universe.count(1),
        1,
        "the overfull page entered the \\output routine"
    );
    assert_eq!(
        universe.count(2),
        1,
        "\\output ran during the command that closed the display, so the very \
         next command already sees its global assignment"
    );
}

/// Node identity for a vertical list, with arena handles erased: two
/// independent runs allocate different `NodeListId`s for structurally
/// identical material, so only kind and dimensions may be compared.
fn vertical_node_shape(node: &Node) -> String {
    match node {
        Node::HList(box_node) => format!(
            "hlist {} {} {}",
            box_node.width.raw(),
            box_node.height.raw(),
            box_node.depth.raw()
        ),
        Node::VList(box_node) => format!(
            "vlist {} {} {}",
            box_node.width.raw(),
            box_node.height.raw(),
            box_node.depth.raw()
        ),
        Node::Glue { kind, .. } => format!("glue {kind:?}"),
        Node::Penalty(penalty) => format!("penalty {penalty}"),
        other => format!("{other:?}"),
    }
}
fn box_children(universe: &Universe, register: u16) -> Vec<Node> {
    let list = universe.box_reg(register).expect("box register is nonvoid");
    let boxed = universe.nodes(list).first().expect("box node").to_owned();
    let children = match boxed {
        Node::HList(node) | Node::VList(node) => node.children,
        _ => panic!("box node"),
    };
    universe.nodes(children).to_vec()
}

#[test]
fn canonical_text_material_space_factor_and_ligature_matrix() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_cmr10_font(&mut control, &mut universe);
    register_source(&mut control, br"\font\f=cmr10\f\setbox0=\hbox{A a. A}\end");
    run_to_end(&mut control, &mut universe);
    let nodes = box_children(&universe, 0);
    assert!(
        nodes
            .iter()
            .filter(|node| matches!(node, Node::Char { .. } | Node::Lig { .. }))
            .count()
            >= 4
    );
    assert!(
        nodes.iter().any(|node| matches!(node, Node::Glue { .. })),
        "spaces append glue"
    );
}

#[test]
fn canonical_vrule_resets_space_factor_before_zero_sfcode_closer() {
    // TeX82 §1056 sets `space_factor:=1000` after `\vrule` in hmode. The
    // closing parenthesis has sfcode zero, so it preserves that reset. The
    // no-rule box is the negative control: there the same closer preserves
    // the colon's sentence-space factor.
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_cmr10_font(&mut control, &mut universe);
    register_source(
        &mut control,
        br"\font\f=cmr10 \sfcode41=0
           \setbox0=\hbox{\f A\spacefactor=2000\vrule width0pt height0pt depth0pt) X}
           \setbox1=\hbox{\f A\spacefactor=2000) X}\end",
    );
    run_to_end(&mut control, &mut universe);

    let glue_widths = |register| {
        box_children(&universe, register)
            .into_iter()
            .filter_map(|node| match node {
                Node::Glue { spec, .. } => Some(universe.glue(spec).width.raw()),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(glue_widths(0), vec![218_453]);
    assert_eq!(glue_widths(1), vec![291_271]);
}

#[test]
fn canonical_direct_material_rule_glue_kern_and_group_cleanup_matrix() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\setbox0=\hbox{\kern1pt\hskip2pt\vrule width3pt height4pt}\end",
    );
    run_to_end(&mut control, &mut universe);
    let nodes = box_children(&universe, 0);
    assert!(matches!(
        nodes.as_slice(),
        [Node::Kern { .. }, Node::Glue { .. }, Node::Rule { .. }]
    ));
}

#[test]
fn canonical_box_request_lifecycle_mode_and_recovery_matrix() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\global\setbox0=\hbox to10pt{}\global\setbox1=\vtop spread2pt{}\copy0\end",
    );
    run_to_end(&mut control, &mut universe);
    assert!(universe.box_reg(0).is_some(), "copy preserves source");
    assert!(universe.box_reg(1).is_some(), "vtop construction completes");
}

#[test]
fn canonical_paragraph_entry_exit_mode_and_recovery_matrix() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_cmr10_font(&mut control, &mut universe);
    register_source(
        &mut control,
        br"\font\f=cmr10\f\everypar{\global\advance\count0 by1}A\par\vbox{B\par}\end",
    );
    run_to_end(&mut control, &mut universe);
    assert_eq!(
        universe.count(0),
        2,
        "outer and internal vertical paragraphs each run everypar"
    );
}

#[test]
fn canonical_structured_list_material_mode_and_recovery_matrix() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_source(
        &mut control,
        br"\setbox0=\hbox{\penalty7\kern2pt}\global\setbox1=\copy0\unhbox0\end",
    );
    run_to_end(&mut control, &mut universe);
    assert!(
        universe.box_reg(0).is_none(),
        "unhbox destructively voids source"
    );
}

#[test]
fn canonical_discretionary_and_text_accent_boundary_matrix() {
    let mut universe = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    register_cmr10_font(&mut control, &mut universe);
    register_source(
        &mut control,
        br"\font\f=cmr10\f\setbox0=\hbox{a\discretionary{b}{c}{d}e\accent19 a}\end",
    );
    run_to_end(&mut control, &mut universe);
    let nodes = box_children(&universe, 0);
    assert!(
        nodes.iter().any(|node| matches!(node, Node::Disc { .. })),
        "explicit discretionary survives"
    );
    assert!(
        nodes
            .iter()
            .filter(|node| matches!(node, Node::Kern { .. }))
            .count()
            >= 2,
        "accent adds positioning kerns"
    );
}
