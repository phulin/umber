use std::sync::Arc;

use tex_command::{FontResource, RegisteredSourceKind, SourceRegistration};
use tex_state::env::banks::{DimenParam, GlueParam, IntParam, TokParam};
use tex_state::font::NULL_FONT;
use tex_state::ids::GlueId;
use tex_state::meaning::MeaningFlags;
use tex_state::{
    BoxDimension, EffectRecord, GroupKind, InputReadState, PrepareMagDiagnostic, StreamSlot,
};

use super::*;
use crate::{CanonicalMainControl, MainControlStep};

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

fn register_cmr10_font(control: &mut CanonicalMainControl, stores: &mut Universe) {
    const CMR10: &[u8] = include_bytes!("../../../tex-fonts/tests/fixtures/cm/cmr10.tfm");
    stores
        .world_mut()
        .set_memory_file("cmr10.tfm", CMR10.to_vec())
        .expect("font fixture installs");
    let metrics = InputReadState::read_input_file(
        &mut stores.input_open_context(),
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

fn routed_text(stores: &Universe, terminal: bool) -> String {
    stores
        .world()
        .effect_records()
        .iter()
        .filter_map(|effect| match effect {
            EffectRecord::StreamWrite { sink, text }
                if matches!(
                    (terminal, sink),
                    (
                        true,
                        tex_state::PrintSink::Terminal | tex_state::PrintSink::TerminalAndLog
                    ) | (
                        false,
                        tex_state::PrintSink::Log | tex_state::PrintSink::TerminalAndLog
                    )
                ) =>
            {
                Some(text.as_str())
            }
            _ => None,
        })
        .collect()
}

fn letter(ch: char) -> Token {
    Token::Char {
        ch,
        cat: Catcode::Letter,
    }
}

fn macro_text(stores: &Universe, name: &str) -> String {
    let meaning = stores
        .macro_meaning(stores.symbol(name).expect("macro target"))
        .expect("macro is defined");
    stores
        .tokens(meaning.replacement_text())
        .iter()
        .filter_map(|token| match token {
            Token::Char { ch, .. } => Some(*ch),
            _ => None,
        })
        .collect()
}

#[test]
fn eqtb_regions_keep_parameter_register_kinds_and_shared_zero_glue_distinct() {
    let mut stores = Universe::new_with_plain_catcodes();
    let symbol = stores.intern("region-meaning").symbol();
    let tokens = stores.intern_token_list(&[letter('T')]);
    let ordinary_glue = stores.intern_glue(GlueSpec {
        width: Scaled::from_raw(3 * Scaled::UNITY),
        stretch: Scaled::from_raw(2 * Scaled::UNITY),
        stretch_order: Order::Fil,
        shrink: Scaled::from_raw(0),
        shrink_order: Order::Normal,
    });
    let parameter_glue = stores.intern_glue(GlueSpec {
        width: Scaled::from_raw(5 * Scaled::UNITY),
        ..GlueSpec::ZERO
    });

    assert_eq!(stores.skip(0), GlueId::ZERO);
    assert_eq!(stores.muskip(0), GlueId::ZERO);
    stores.set_skip(0, ordinary_glue);
    stores.set_count(0, 11);
    stores.set_dimen(0, Scaled::from_raw(13));
    stores.set_toks(0, tokens);
    stores.set_meaning(symbol, Meaning::CharGiven('M'));
    stores.set_int_param(IntParam::TOLERANCE, 17);
    stores.set_dimen_param(DimenParam::H_SIZE, Scaled::from_raw(19));
    stores.set_glue_param(GlueParam::BASELINE_SKIP, parameter_glue);
    stores.set_tok_param(TokParam::EVERY_PAR, tokens);

    assert_eq!(stores.skip(0), ordinary_glue);
    assert_eq!(stores.muskip(0), GlueId::ZERO);
    assert_eq!(stores.count(0), 11);
    assert_eq!(stores.dimen(0), Scaled::from_raw(13));
    assert_eq!(stores.tokens(stores.toks(0)), &[letter('T')]);
    assert_eq!(stores.meaning(symbol), Meaning::CharGiven('M'));
    assert_eq!(stores.int_param(IntParam::TOLERANCE), 17);
    assert_eq!(stores.dimen_param(DimenParam::H_SIZE), Scaled::from_raw(19));
    assert_eq!(stores.glue_param(GlueParam::BASELINE_SKIP), parameter_glue);
    assert_eq!(stores.tok_param(TokParam::EVERY_PAR), tokens);
}

#[test]
fn nested_typed_groups_push_one_boundary_and_restore_outer_metadata() {
    const KINDS: [GroupKind; 16] = [
        GroupKind::Simple,
        GroupKind::HBox,
        GroupKind::AdjustedHBox,
        GroupKind::VBox,
        GroupKind::VTop,
        GroupKind::SemiSimple,
        GroupKind::MathShift,
        GroupKind::Align,
        GroupKind::NoAlign,
        GroupKind::Output,
        GroupKind::Math,
        GroupKind::Disc,
        GroupKind::Insert,
        GroupKind::VCenter,
        GroupKind::MathChoice,
        GroupKind::MathLeft,
    ];

    let mut stores = Universe::new_with_plain_catcodes();
    for (index, kind) in KINDS.into_iter().enumerate() {
        stores.enter_group_with_kind(kind);
        stores.push_aftergroup(letter(char::from(b'A' + index as u8)));
        assert_eq!(stores.group_depth(), index as u32 + 1);
        assert_eq!(stores.innermost_group_kind(), Some(kind));
        assert_eq!(stores.group_kinds().collect::<Vec<_>>(), KINDS[..=index]);
    }

    for (index, kind) in KINDS.into_iter().enumerate().rev() {
        assert_eq!(
            stores
                .leave_group_with_kind(kind)
                .expect("matching typed group leaves"),
            [letter(char::from(b'A' + index as u8))]
        );
        assert_eq!(stores.group_depth(), index as u32);
        assert_eq!(stores.group_kinds().collect::<Vec<_>>(), KINDS[..index]);
    }
    assert_eq!(stores.innermost_group_kind(), None);
}

#[test]
fn save_stack_local_global_redefinition_restores_once_per_level() {
    let mut stores = Universe::new_with_plain_catcodes();
    let symbol = stores.intern("saved-meaning").symbol();
    stores.set_count(0, 10);
    stores.set_count(1, 20);
    stores.set_meaning(symbol, Meaning::Relax);
    let baseline = stores.env_journal_bytes();

    stores.enter_group_with_kind(GroupKind::Simple);
    stores.set_count(0, 11);
    let one_count_save = stores.env_journal_bytes();
    stores.set_count(0, 12);
    assert_eq!(stores.env_journal_bytes(), one_count_save);
    stores.set_count(1, 21);
    stores.set_count(1, 22);
    stores.set_meaning(symbol, Meaning::CharGiven('A'));
    let one_meaning_save = stores.env_journal_bytes();
    stores.set_meaning(symbol, Meaning::CharGiven('B'));
    assert_eq!(stores.env_journal_bytes(), one_meaning_save);

    stores.enter_group_with_kind(GroupKind::SemiSimple);
    stores.set_count(0, 13);
    stores.set_count_global(0, 14);
    stores.set_meaning(symbol, Meaning::CharGiven('C'));
    stores.set_meaning_global(symbol, Meaning::CharGiven('D'));
    assert!(stores.leave_group_with_kind(GroupKind::SemiSimple).is_ok());
    assert_eq!(stores.count(0), 14);
    assert_eq!(stores.meaning(symbol), Meaning::CharGiven('D'));

    assert!(stores.leave_group_with_kind(GroupKind::Simple).is_ok());
    assert_eq!(
        stores.count(0),
        14,
        "inner global suppresses stale outer save"
    );
    assert_eq!(
        stores.count(1),
        20,
        "repeated locals restore the one old value"
    );
    assert_eq!(stores.meaning(symbol), Meaning::CharGiven('D'));
    assert!(stores.env_journal_bytes() >= baseline);
}

#[test]
fn save_stack_aftergroup_tokens_replay_fifo_after_unsave() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\def\digit#1{\global\multiply\count0 by10\global\advance\count0 by#1}\def\inner{\digit3}\def\first{\digit1}\def\second{\digit2}{\aftergroup\first{\aftergroup\inner}\aftergroup\second}\end",
    );

    run_to_end(&mut control, &mut stores);

    assert_eq!(
        stores.count(0),
        312,
        "inner replay precedes outer FIFO replay"
    );
    assert_eq!(stores.group_depth(), 0);
}

#[test]
fn unsave_restores_local_entries_retains_globals_then_replays_tokens() {
    let mut stores = Universe::new_with_plain_catcodes();
    let symbol = stores.intern("unsave-meaning").symbol();
    stores.set_count(0, 10);
    stores.set_meaning(symbol, Meaning::Relax);
    stores.enter_group_with_kind(GroupKind::SemiSimple);
    stores.set_count(0, 11);
    stores.set_meaning(symbol, Meaning::CharGiven('L'));
    stores.push_aftergroup(letter('O'));
    stores.enter_group_with_kind(GroupKind::Simple);
    stores.set_count(0, 12);
    stores.set_count_global(0, 13);
    stores.set_meaning(symbol, Meaning::CharGiven('N'));
    stores.set_meaning_global(symbol, Meaning::CharGiven('G'));
    stores.push_aftergroup(letter('I'));

    assert_eq!(
        stores
            .leave_group_with_kind(GroupKind::Simple)
            .expect("inner unsave"),
        [letter('I')]
    );
    assert_eq!(stores.count(0), 13);
    assert_eq!(stores.meaning(symbol), Meaning::CharGiven('G'));
    assert_eq!(stores.innermost_group_kind(), Some(GroupKind::SemiSimple));
    assert_eq!(
        stores
            .leave_group_with_kind(GroupKind::SemiSimple)
            .expect("outer unsave"),
        [letter('O')]
    );
    assert_eq!(stores.count(0), 13);
    assert_eq!(stores.meaning(symbol), Meaning::CharGiven('G'));
    assert_eq!(stores.group_depth(), 0);
}

#[test]
fn brace_dispatch_opens_normal_groups_and_recovers_each_mismatched_closer() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"{\count0=1}\begingroup\count1=2\endgroup}\endgroup\begingroup}\count2=3\end",
    );

    run_to_end(&mut control, &mut stores);

    assert_eq!(stores.count(0), 0);
    assert_eq!(stores.count(1), 0);
    assert_eq!(stores.count(2), 3);
    assert_eq!(
        stores.group_depth(),
        1,
        "TeX82 §1068 discards the mismatched brace without unsaving"
    );
    let output = terminal_text(&stores);
    assert!(output.contains("Too many }'s"), "{output}");
    assert!(output.contains("Extra \\endgroup"), "{output}");
    assert!(
        output.contains("Extra }, or forgotten \\endgroup"),
        "{output}"
    );
}

#[test]
fn prefix_collection_expands_repeats_validates_and_resolves_globaldefs() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\def\G{\global}{\global\G\count0=7}\globaldefs=1{\count1=11}\globaldefs=-1{\global\count2=13}\globaldefs=0\long\long\outer\gdef\prefixed{}\end",
    );

    run_to_end(&mut control, &mut stores);

    assert_eq!(stores.count(0), 7, "expanded repeated global is idempotent");
    assert_eq!(
        stores.count(1),
        11,
        "positive globaldefs forces global scope"
    );
    assert_eq!(
        stores.count(2),
        0,
        "negative globaldefs suppresses global scope"
    );
    let prefixed = stores
        .macro_meaning(stores.symbol("prefixed").expect("prefixed target"))
        .expect("prefixed definition survives");
    assert!(prefixed.flags().contains(MeaningFlags::LONG));
    assert!(prefixed.flags().contains(MeaningFlags::OUTER));
    assert!(
        reject_all_prefixes(Prefixes {
            global: true,
            flags: MeaningFlags::EMPTY,
        })
        .is_err()
    );
    assert!(
        reject_macro_prefixes(Prefixes {
            global: false,
            flags: MeaningFlags::LONG,
        })
        .is_err()
    );
}

#[test]
fn prefix_collection_skips_spaces_relax_and_macro_calls_without_losing_bits() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\def\G{\global\relax}\def\L{\long\relax}\def\O{\outer\relax}\begingroup\G \relax\G\count0=7\L \relax\O\def\local{L}\global\L\O\def\shared{S}\endgroup\end",
    );

    run_to_end(&mut control, &mut stores);

    assert_eq!(
        stores.count(0),
        7,
        "§404 filler and expanded calls retain the accumulated global prefix"
    );
    assert!(
        stores
            .symbol("local")
            .and_then(|symbol| stores.macro_meaning(symbol))
            .is_none(),
        "the unprefixed long/outer definition remains local"
    );
    let shared = stores
        .macro_meaning(stores.symbol("shared").expect("global definition target"))
        .expect("global prefixed definition survives");
    assert!(shared.flags().contains(MeaningFlags::LONG));
    assert!(shared.flags().contains(MeaningFlags::OUTER));
}

#[test]
fn get_r_token_skips_spaces_and_recovers_non_control_sequence_targets() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(&mut control, br"\def   \valid{V}\def Q{B}\end");

    run_to_end(&mut control, &mut stores);

    assert!(
        stores
            .macro_meaning(stores.symbol("valid").expect("valid target"))
            .is_some()
    );
    assert!(
        stores
            .macro_meaning(stores.symbol("inaccessible").expect("recovery target"))
            .is_some()
    );
    assert!(
        terminal_text(&stores).contains("Missing control sequence inserted"),
        "invalid character target uses inaccessible recovery"
    );
}

#[test]
fn set_font_assignment_obeys_local_global_and_globaldefs_scope() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_cmr10_font(&mut control, &mut stores);
    assert_eq!(stores.current_font(), NULL_FONT);
    register_source(
        &mut control,
        br"\font\f=cmr10 \nullfont{\f}{\global\f}\globaldefs=-1{\global\nullfont}\globaldefs=1{\nullfont}",
    );

    assert_eq!(
        control.step(&mut stores).expect("font definition"),
        MainControlStep::Continue
    );
    let selected = match stores.meaning(stores.symbol("f").expect("font target")) {
        Meaning::Font(font) => font,
        meaning => panic!("font definition installed {meaning:?}"),
    };
    for _ in 0..7 {
        assert_eq!(
            control.step(&mut stores).expect("font selection step"),
            MainControlStep::Continue
        );
    }
    assert_eq!(
        stores.current_font(),
        selected,
        "local font restored; global survived"
    );

    run_to_end(&mut control, &mut stores);
    assert_eq!(
        stores.current_font(),
        NULL_FONT,
        "globaldefs overrides explicit scope"
    );
    assert_eq!(stores.group_depth(), 0);
}

#[test]
fn def_family_selects_expansion_flags_meaning_kind_and_effective_scope() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\def\source{A}{\def\local{L}\edef\localexpanded{\source}\long\outer\gdef\globalraw{\source}\xdef\globalexpanded{\source}}\end",
    );

    run_to_end(&mut control, &mut stores);

    assert_eq!(
        stores.meaning(stores.symbol("local").expect("local target")),
        Meaning::Undefined
    );
    assert_eq!(
        stores.meaning(stores.symbol("localexpanded").expect("local edef target")),
        Meaning::Undefined
    );
    let source = stores.intern("source").symbol();
    let raw = stores
        .macro_meaning(stores.symbol("globalraw").expect("gdef target"))
        .expect("gdef survives");
    assert!(raw.flags().contains(MeaningFlags::LONG));
    assert!(raw.flags().contains(MeaningFlags::OUTER));
    assert_eq!(stores.tokens(raw.replacement_text()), &[Token::Cs(source)]);
    let expanded = stores
        .macro_meaning(stores.symbol("globalexpanded").expect("xdef target"))
        .expect("xdef survives");
    assert_eq!(stores.tokens(expanded.replacement_text()), &[letter('A')]);
}

#[test]
fn let_and_futurelet_copy_meaning_and_preserve_lookahead_order() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\def\first{\global\count0=1}\def\second{\global\multiply\count0 by10\global\advance\count0 by2}\let\alias = \begingroup\futurelet\next\first\second\end",
    );

    run_to_end(&mut control, &mut stores);

    assert_eq!(
        stores.meaning(stores.symbol("alias").expect("let target")),
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::BeginGroup)
    );
    assert_eq!(
        stores.meaning(stores.symbol("next").expect("futurelet target")),
        stores.meaning(stores.symbol("second").expect("lookahead macro"))
    );
    assert_eq!(
        stores.count(0),
        12,
        "first then second lookahead order is preserved"
    );
}

#[test]
fn shorthand_definitions_install_each_command_operand_and_bound_recovery() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\global\chardef\c=65\global\mathchardef\m=4660\global\countdef\n=7\global\dimendef\d=8\global\skipdef\s=9\global\muskipdef\u=10\global\toksdef\t=256\global\chardef\badchar=256\global\countdef\badcount=256\end",
    );

    run_to_end(&mut control, &mut stores);

    let meaning = |name| stores.meaning(stores.symbol(name).expect("shorthand target"));
    assert_eq!(meaning("c"), Meaning::CharGiven('A'));
    assert_eq!(meaning("m"), Meaning::MathCharGiven(4660));
    assert_eq!(meaning("n"), Meaning::CountRegister(7));
    assert_eq!(meaning("d"), Meaning::DimenRegister(8));
    assert_eq!(meaning("s"), Meaning::SkipRegister(9));
    assert_eq!(meaning("u"), Meaning::MuskipRegister(10));
    assert_eq!(meaning("t"), Meaning::ToksRegister(0));
    assert_eq!(meaning("badchar"), Meaning::CharGiven('\0'));
    assert_eq!(meaning("badcount"), Meaning::CountRegister(0));
    let output = terminal_text(&stores);
    assert!(output.contains("Bad character code"), "{output}");
}

#[test]
fn typed_parameter_register_and_arithmetic_assignments_cover_scope_copy_and_bounds() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\count0=7\advance\count0 by5\multiply\count0 by3\divide\count0 by2\dimen0=1pt\advance\dimen0 by2pt\skip0=1pt plus2fil\advance\skip0 by2pt\muskip0=1mu\advance\muskip0 by2mu\toks0={alpha}\toks1=\toks0\tolerance=123\hsize=4pt\baselineskip=5pt\everypar={hook}\catcode`@=11{\count9=41}\globaldefs=1{\dimen9=6pt}\globaldefs=-1{\global\count10=43}\globaldefs=0\end",
    );

    run_to_end(&mut control, &mut stores);

    assert_eq!(stores.count(0), 18);
    assert_eq!(stores.dimen(0), Scaled::from_raw(3 * Scaled::UNITY));
    assert_eq!(
        stores.glue(stores.skip(0)).width,
        Scaled::from_raw(3 * Scaled::UNITY)
    );
    assert_eq!(
        stores.glue(stores.muskip(0)).width,
        Scaled::from_raw(3 * Scaled::UNITY)
    );
    assert_eq!(stores.tokens(stores.toks(1)), stores.tokens(stores.toks(0)));
    assert_eq!(
        stores.tokens(stores.toks(0)),
        &[
            letter('a'),
            letter('l'),
            letter('p'),
            letter('h'),
            letter('a')
        ]
    );
    assert_eq!(stores.int_param(IntParam::TOLERANCE), 123);
    assert_eq!(
        stores.dimen_param(DimenParam::H_SIZE),
        Scaled::from_raw(4 * Scaled::UNITY)
    );
    assert_eq!(
        stores
            .glue(stores.glue_param(GlueParam::BASELINE_SKIP))
            .width,
        Scaled::from_raw(5 * Scaled::UNITY)
    );
    assert_eq!(
        stores.tokens(stores.tok_param(TokParam::EVERY_PAR)),
        &[letter('h'), letter('o'), letter('o'), letter('k')]
    );
    assert_eq!(stores.catcode('@'), Catcode::Letter);
    assert_eq!(stores.count(9), 0, "ordinary local register restores");
    assert_eq!(stores.dimen(9), Scaled::from_raw(6 * Scaled::UNITY));
    assert_eq!(stores.count(10), 0, "negative globaldefs suppresses global");
}

#[test]
fn afterassignment_slot_overwrites_and_fires_once_after_successful_assignment() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\def\first{\global\count0=100}\def\second{\global\advance\count0 by1}\afterassignment\first\afterassignment\second\count1=7\count2=9\end",
    );

    run_to_end(&mut control, &mut stores);

    assert_eq!(stores.count(0), 1, "replacement token fires exactly once");
    assert_eq!(stores.count(1), 7);
    assert_eq!(
        stores.count(2),
        9,
        "later assignment does not refire the slot"
    );
    assert_eq!(stores.take_afterassignment(), None);
}

#[test]
fn prepare_mag_bounds_and_freezes_first_effective_value() {
    let mut stores = Universe::new_with_plain_catcodes();
    stores.set_mag(0);
    assert_eq!(
        stores.prepare_mag(),
        (
            1_000,
            Some(PrepareMagDiagnostic::IllegalMagnification { attempted: 0 })
        )
    );
    assert_eq!(stores.mag(), 1_000);
    assert_eq!(stores.prepared_mag(), Some(1_000));
    stores.set_mag(1_200);
    assert_eq!(
        stores.prepare_mag(),
        (
            1_000,
            Some(PrepareMagDiagnostic::IncompatibleMagnification {
                attempted: 1_200,
                retained: 1_000,
            })
        )
    );
    assert_eq!(stores.mag(), 1_000);

    let mut maximum = Universe::new_with_plain_catcodes();
    maximum.set_mag(32_768);
    assert_eq!(maximum.prepare_mag(), (32_768, None));
    assert_eq!(maximum.prepare_mag(), (32_768, None));
    let mut too_large = Universe::new_with_plain_catcodes();
    too_large.set_mag(32_769);
    assert_eq!(
        too_large.prepare_mag(),
        (
            1_000,
            Some(PrepareMagDiagnostic::IllegalMagnification { attempted: 32_769 })
        )
    );
}

#[test]
fn prepare_mag_accepts_one_and_preserves_exact_errors() {
    fn run(source: &[u8]) -> Universe {
        let mut stores = Universe::new_with_plain_catcodes();
        stores.set_interaction_mode(InteractionMode::Nonstop);
        let mut control = CanonicalMainControl::tex82_initex(&mut stores);
        register_source(&mut control, source);
        run_to_end(&mut control, &mut stores);
        stores
    }

    let legal = run(br"\mag=1\dimen0=1truept\end");
    assert_eq!(legal.mag(), 1);
    assert_eq!(legal.prepared_mag(), Some(1));
    assert_eq!(legal.dimen(0), Scaled::from_raw(1_000 * Scaled::UNITY));
    assert_eq!(routed_text(&legal, true), "");
    assert_eq!(routed_text(&legal, false), "");

    let illegal = run(br"\mag=0\dimen0=1truept\end");
    assert_eq!(illegal.mag(), 1_000);
    assert_eq!(illegal.prepared_mag(), Some(1_000));
    let illegal_terminal = concat!(
        "! Illegal magnification has been changed to 1000 (0).\n",
        "l.1 \\mag=0\\dimen0=1true\n",
        "                       pt\\end\n",
        "(see the transcript file for additional information)",
    );
    assert_eq!(routed_text(&illegal, true), illegal_terminal);
    let illegal_log = concat!(
        "! Illegal magnification has been changed to 1000 (0).\n",
        "l.1 \\mag=0\\dimen0=1true\n",
        "                       pt\\end\n",
        "The magnification ratio must be between 1 and 32768.\n\n",
    );
    assert_eq!(routed_text(&illegal, false), illegal_log);

    let incompatible = run(br"\mag=1200\dimen0=1truept\mag=2000\dimen1=1truept\end");
    assert_eq!(incompatible.mag(), 1_200);
    assert_eq!(incompatible.prepared_mag(), Some(1_200));
    assert_eq!(incompatible.dimen(0), Scaled::from_raw(54_613));
    assert_eq!(incompatible.dimen(1), Scaled::from_raw(54_613));
    let incompatible_terminal = concat!(
        "! Incompatible magnification (2000);\n",
        " the previous value will be retained (1200).\n",
        "l.1 \\mag=1200\\dimen0=1truept\\mag=2000\\dimen1=1true\n",
        "                                                  pt\\end\n",
        "(see the transcript file for additional information)",
    );
    assert_eq!(routed_text(&incompatible, true), incompatible_terminal);
    let incompatible_log = concat!(
        "! Incompatible magnification (2000);\n",
        " the previous value will be retained (1200).\n",
        "l.1 \\mag=1200\\dimen0=1truept\\mag=2000\\dimen1=1true\n",
        "                                                  pt\\end\n",
        "I can handle only one magnification ratio per job. So I've\n",
        "reverted to the magnification you used earlier on this run.\n\n",
    );
    assert_eq!(routed_text(&incompatible, false), incompatible_log);
}

#[test]
fn read_to_definition_scans_stream_keyword_target_and_installs_tokens() {
    let mut stores = Universe::new_with_plain_catcodes();
    stores.set_interaction_mode(InteractionMode::ErrorStop);
    stores
        .world_mut()
        .push_memory_terminal_line("terminal")
        .expect("terminal input registers");
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    control.capabilities_mut().register_input(
        "child.tex",
        SourceRegistration::new(RegisteredSourceKind::World, Arc::<[u8]>::from(&b"file"[..])),
    );
    register_source(
        &mut control,
        br"\openin1=child.tex \read1 to \fileline \closein1 \read1 to \closedline \end",
    );
    run_to_end(&mut control, &mut stores);
    // §483 stores `\endlinechar` in `buffer[limit]` and then tokenizes the
    // line, so the line ends in whatever §348 makes of a category-5
    // character in `mid_line` state -- a space. The raw carriage return
    // reached these lists only while `\read` bypassed the tokenizer
    // (umber2-johp.253).
    assert_eq!(macro_text(&stores, "fileline"), "file ");
    assert_eq!(macro_text(&stores, "closedline"), "terminal ");
    assert!(stores.input_stream_eof(StreamSlot::new(1)));
}

#[test]
fn read_missing_to_and_invalid_target_recover_exactly() {
    let mut stores = Universe::new_with_plain_catcodes();
    stores.set_interaction_mode(InteractionMode::Scroll);
    stores
        .world_mut()
        .push_memory_terminal_line("first")
        .expect("first terminal line registers");
    stores
        .world_mut()
        .push_memory_terminal_line("second")
        .expect("second terminal line registers");
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(&mut control, br"\read1 x\read1 to \line \count0=9\end");

    run_to_end(&mut control, &mut stores);

    assert_eq!(macro_text(&stores, "inaccessible"), "first ");
    assert_eq!(macro_text(&stores, "line"), "second ");
    assert_eq!(
        stores.count(0),
        9,
        "input following the bad target continues"
    );
    let output = terminal_text(&stores);
    assert_eq!(
        output.matches("Missing `to' inserted").count(),
        1,
        "{output}"
    );
    assert_eq!(
        output.matches("Missing control sequence inserted").count(),
        1,
        "{output}"
    );
}

#[test]
fn setbox_request_encodes_scope_and_rejects_disallowed_contexts() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"{\setbox0=\hbox{}\global\setbox1=\hbox{}}\end",
    );
    run_to_end(&mut control, &mut stores);
    assert!(stores.box_reg(0).is_none(), "ordinary local box restores");
    assert!(stores.box_reg(1).is_some(), "explicit global box survives");
}

#[test]
fn auxiliary_assignments_validate_mode_bounds_and_update_only_owned_state() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\prevdepth=2pt\prevgraf=3\spacefactor\prevgraf=-1\xdef\depth{\the\prevdepth}\xdef\graf{\the\prevgraf}\setbox0=\hbox{}\wd0=6pt\ht0=7pt\dp0=8pt\copy0\pagegoal=10pt\deadcycles=4\insertpenalties=5\xdef\goal{\the\pagegoal}\xdef\dead{\the\deadcycles}\xdef\penalties{\the\insertpenalties}\end",
    );
    run_to_end(&mut control, &mut stores);
    assert_eq!(macro_text(&stores, "depth"), "2.0pt");
    assert_eq!(macro_text(&stores, "graf"), "3");
    assert_eq!(macro_text(&stores, "goal"), "10.0pt");
    assert_eq!(macro_text(&stores, "dead"), "4");
    assert_eq!(macro_text(&stores, "penalties"), "5");
    assert_eq!(
        stores.box_dimension(0, BoxDimension::Width),
        Some(Scaled::from_raw(6 * Scaled::UNITY))
    );
    assert_eq!(
        stores.box_dimension(0, BoxDimension::Height),
        Some(Scaled::from_raw(7 * Scaled::UNITY))
    );
    assert_eq!(
        stores.box_dimension(0, BoxDimension::Depth),
        Some(Scaled::from_raw(8 * Scaled::UNITY))
    );
    let output = terminal_text(&stores);
    assert!(output.contains("can't use `\\spacefactor'"), "{output}");
    assert!(output.contains("Bad \\prevgraf"), "{output}");

    let mut horizontal_stores = crate::test_harness::universe_with_plain_catcodes();
    let mut horizontal = CanonicalMainControl::tex82_initex(&mut horizontal_stores);
    register_source(
        &mut horizontal,
        br"x\spacefactor=2000\xdef\sf{\the\spacefactor}\par\end",
    );
    run_to_end(&mut horizontal, &mut horizontal_stores);
    assert_eq!(macro_text(&horizontal_stores, "sf"), "2000");
}

#[test]
fn parshape_assignment_scans_pair_count_and_restores_local_shape() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\parshape=2 1pt 10pt 2pt 20pt\xdef\multiple{\the\parshape}{\parshape=0\xdef\zeroShape{\the\parshape}}{\parshape=-1\xdef\negativeShape{\the\parshape}}\globaldefs=1{\parshape=1 3pt 30pt}\globaldefs=-1{\global\parshape=1 4pt 40pt}\end",
    );
    run_to_end(&mut control, &mut stores);
    assert_eq!(macro_text(&stores, "multiple"), "2");
    assert_eq!(macro_text(&stores, "zeroShape"), "0");
    assert_eq!(macro_text(&stores, "negativeShape"), "0");
    assert_eq!(stores.paragraph_shape_len(), 1);
    assert_eq!(
        stores.paragraph_shape_dimension(1, false),
        Scaled::from_raw(3 * Scaled::UNITY)
    );
    assert_eq!(
        stores.paragraph_shape_dimension(1, true),
        Scaled::from_raw(30 * Scaled::UNITY)
    );
    assert_eq!(
        stores.paragraph_shape_dimension(2, true),
        Scaled::from_raw(30 * Scaled::UNITY)
    );
}

#[test]
fn etex_parshape_enquiries_follow_grouped_shape_through_canonical_scanners() {
    // e-TeX 2.6 etex.ch [3455--3488]: all three `last_item` enquiries read
    // the live, group-scoped parshape and return dimensions inside `\the`.
    let mut stores = Universe::new_with_plain_catcodes();
    tex_command::install_tex82_expandable_primitives(&mut stores);
    tex_command::install_etex_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    crate::install_etex_unexpandable_primitives(&mut stores);
    let format = stores.dump_format().expect("dump extended e-TeX format");
    let mut stores = Universe::from_format(tex_state::World::memory(), &format)
        .expect("restore extended e-TeX format");
    let mut control = CanonicalMainControl::with_profile(tex_command::CommandProfile::ETEX26);
    control
        .set_fuel_limit(20_000)
        .expect("bounded canonical fuel");
    register_source(
        &mut control,
        br"\parshape=2 1pt 2pt 3pt 4pt
           \edef\shapeA{\the\parshapeindent1/\the\parshapelength1/\the\parshapedimen3/\the\parshapedimen4/\the\parshapelength8}
           {\parshape=0\global\edef\shapeEmpty{\the\parshapelength1/\the\parshapedimen1}}
           \edef\shapeRestored{\the\parshapeindent2/\the\parshapelength2}\end",
    );

    run_to_end(&mut control, &mut stores);

    assert_eq!(
        macro_text(&stores, "shapeA"),
        "1.0pt/2.0pt/3.0pt/4.0pt/4.0pt"
    );
    assert_eq!(macro_text(&stores, "shapeEmpty"), "0.0pt/0.0pt");
    assert_eq!(macro_text(&stores, "shapeRestored"), "3.0pt/4.0pt");
}

#[test]
fn font_parameter_assignments_update_global_dimen_hyphenchar_and_skewchar() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_cmr10_font(&mut control, &mut stores);
    register_source(
        &mut control,
        br"\font\f=cmr10 {\fontdimen2\f=3pt\hyphenchar\f=45\skewchar\f=127}\end",
    );
    run_to_end(&mut control, &mut stores);
    let font = match stores.meaning(stores.symbol("f").expect("font target")) {
        Meaning::Font(font) => font,
        meaning => panic!("font definition installed {meaning:?}"),
    };
    assert_eq!(
        stores.font_dimen(font, 2),
        Scaled::from_raw(3 * Scaled::UNITY)
    );
    assert_eq!(stores.font_hyphen_char(font), 45);
    assert_eq!(stores.font_skew_char(font), 127);
}

#[test]
fn interaction_assignment_updates_mode_and_selector_for_log_state() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    // Each mode is read after the assignment step and before the source runs
    // dry: §360's `*` prompt reads a terminal with nothing left, and §71
    // answers that with §93's `fatal_error`, whose `succumb` turns
    // `error_stop_mode` into `scroll_mode` on the way out.
    for (source, expected) in [
        (&br"\batchmode\relax"[..], InteractionMode::Batch),
        (&br"\nonstopmode\relax"[..], InteractionMode::Nonstop),
        (&br"\scrollmode\relax"[..], InteractionMode::Scroll),
        (&br"\errorstopmode\relax"[..], InteractionMode::ErrorStop),
    ] {
        register_source(&mut control, source);
        assert_eq!(
            control.step(&mut stores).expect("interaction mode assigns"),
            MainControlStep::Continue
        );
        assert_eq!(stores.interaction_mode(), expected);
        run_to_end(&mut control, &mut stores);
    }
    register_source(&mut control, br"{\batchmode}");
    run_to_end(&mut control, &mut stores);
    assert_eq!(stores.interaction_mode(), InteractionMode::Batch);
}

#[test]
fn interaction_selector_matrix_with_open_and_closed_log() {
    use tex_state::PrintSink;
    use tex_state::print::{Printer, Selector};

    for (source, mode) in [
        (
            &br"\batchmode\advance\count0 by1"[..],
            InteractionMode::Batch,
        ),
        (
            &br"\nonstopmode\advance\count0 by1"[..],
            InteractionMode::Nonstop,
        ),
        (
            &br"\scrollmode\advance\count0 by1"[..],
            InteractionMode::Scroll,
        ),
        (
            &br"\errorstopmode\advance\count0 by1"[..],
            InteractionMode::ErrorStop,
        ),
    ] {
        for (log_opened, expected_selector, expected_terminal, expected_log) in [
            (
                false,
                if mode == InteractionMode::Batch {
                    Selector::NoPrint
                } else {
                    Selector::TermOnly
                },
                mode != InteractionMode::Batch,
                false,
            ),
            (
                true,
                if mode == InteractionMode::Batch {
                    Selector::LogOnly
                } else {
                    Selector::TermAndLog
                },
                mode != InteractionMode::Batch,
                true,
            ),
        ] {
            let mut stores = Universe::new_with_plain_catcodes();
            stores.set_count(1, 41);
            let before_history = stores.world().error_channel().history();
            let before_errors = stores.world().error_channel().error_count();
            let mut control = CanonicalMainControl::tex82_initex(&mut stores);
            register_source(&mut control, source);

            assert_eq!(
                control.step(&mut stores).expect("interaction mode assigns"),
                MainControlStep::Continue
            );
            assert_eq!(stores.interaction_mode(), mode);
            let selector = Selector::for_interaction_and_log(mode, log_opened);
            assert_eq!(selector, expected_selector);
            Printer::new(&mut stores, selector).print("selector-marker");

            assert_eq!(
                control
                    .step(&mut stores)
                    .expect("following assignment executes"),
                MainControlStep::Continue
            );
            assert_eq!(stores.count(0), 1, "{mode:?}, log_opened={log_opened}");
            assert_eq!(stores.count(1), 41, "unrelated state changed");
            assert_eq!(stores.interaction_mode(), mode);
            assert_eq!(stores.world().error_channel().history(), before_history);
            assert_eq!(stores.world().error_channel().error_count(), before_errors);

            let terminal = stores.world().effect_records().iter().any(|effect| {
                matches!(effect, EffectRecord::StreamWrite { sink: PrintSink::Terminal | PrintSink::TerminalAndLog, text } if text == "selector-marker")
            });
            let log = stores.world().effect_records().iter().any(|effect| {
                matches!(effect, EffectRecord::StreamWrite { sink: PrintSink::Log | PrintSink::TerminalAndLog, text } if text == "selector-marker")
            });
            assert_eq!(terminal, expected_terminal);
            assert_eq!(log, expected_log);
        }
    }
}

#[test]
fn assignment_terminator_skips_space_and_relax_but_retains_first_command() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\def\mark{\global\advance\count1 by1}\count0=7   \relax\relax\mark\end",
    );
    run_to_end(&mut control, &mut stores);
    assert_eq!(stores.count(0), 7);
    assert_eq!(stores.count(1), 1);

    register_source(&mut control, br"\count2=9");
    run_to_end(&mut control, &mut stores);
    assert_eq!(stores.count(2), 9, "EOF also terminates the assignment");
}

#[test]
fn hyphenation_data_distinguishes_exceptions_initex_patterns_and_flush_recovery() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_source(&mut control, br"\hyphenation{for-get}\patterns{a1b}\end");
    run_to_end(&mut control, &mut stores);

    assert_eq!(stores.hyphenation_exception("forget"), Some(&[3][..]));
    assert_eq!(stores.hyphen_positions_for_language(0, "ab", 0, 0), vec![1]);
}

#[test]
fn font_definition_scans_sizes_reuses_identity_and_recovers_illegal_values() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    register_cmr10_font(&mut control, &mut stores);
    register_source(
        &mut control,
        br"\font\a=cmr10 \font\b=cmr10 \font\c=cmr10 at 12pt \font\d=cmr10 scaled 1200 \font\e=cmr10 scaled 0 \font\f=cmr10 at 0pt \end",
    );
    run_to_end(&mut control, &mut stores);

    let font = |name| match stores.meaning(stores.symbol(name).expect("font target")) {
        Meaning::Font(font) => font,
        meaning => panic!("font definition installed {meaning:?}"),
    };
    let default = font("a");
    assert_eq!(font("b"), default, "equal loads reuse font identity");
    let twelve_point = font("c");
    assert_eq!(font("d"), twelve_point, "at and scaled sizes normalize");
    assert_eq!(
        stores.font(twelve_point).size(),
        Scaled::from_raw(12 * Scaled::UNITY)
    );
    assert_eq!(font("e"), default, "illegal scale recovers to default size");
    assert_eq!(
        font("f"),
        default,
        "illegal at size recovers to default size"
    );
}
