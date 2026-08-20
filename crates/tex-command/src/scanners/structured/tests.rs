use std::sync::Arc;

use tex_state::macro_store::MacroMeaning;
use tex_state::meaning::{ExpandablePrimitive, Meaning, MeaningFlags, UnexpandablePrimitive};
use tex_state::token::{Catcode, OriginId, Token};

use super::*;
use crate::input::{ReplayTrace, TokenBehavior};
use crate::observation::{MutationTarget, ObservationValue, RecoveryKind};
use crate::test_harness::{
    ProcessorScenario, Recorder, ScannerRig, diagnostic_text, plain_text_tokens as text_tokens,
    processor, push, traced,
};
use crate::{
    CommandHostCapabilities, CommandHostContext, CommandObservation, CommandReplayDelivery,
    CommandState, RegisteredSourceKind, SourceRegistration,
};

#[test]
fn futurelet_target_control_sequence_is_not_skipped_when_it_means_space() {
    // TeX82 §1215's `get_r_token` accepts a control-sequence token based on
    // `cur_cs`, independent of its current command meaning. LaTeX's
    // space-skipping `\@ifnextchar` path relies on this when `\@let@token`
    // was just made equivalent to a space and is immediately reused as the
    // target of another `\futurelet`.
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let target = universe.intern("target").symbol();
    let first = universe.intern("first").symbol();
    universe.set_meaning_global(
        target,
        Meaning::CharToken {
            ch: ' ',
            cat: Catcode::Space,
        },
    );
    universe.set_meaning_global(first, Meaning::Relax);
    push(
        &mut command,
        [
            Token::Cs(target),
            Token::Cs(first),
            Token::Char {
                ch: '[',
                cat: Catcode::Other,
            },
        ],
    );
    let mut capabilities = CommandHostCapabilities::default();

    let assignment = processor(&mut command, &mut universe, &mut capabilities)
        .scan_let_assignment(true)
        .expect("futurelet assignment scans");

    assert_eq!(assignment.target, target);
    assert_eq!(
        assignment.meaning,
        Meaning::CharToken {
            ch: '[',
            cat: Catcode::Other
        }
    );
}

#[test]
fn setbox_forbidden_path_does_not_fetch_or_back_up_the_box_command() {
    let scan = |allowed| {
        let mut rig = ScannerRig::plain();
        rig.scenario.push(text_tokens("0=x"));
        let (assignment, next, context) = {
            let mut processor = rig.processor();
            let assignment = processor
                .scan_setbox_assignment(allowed)
                .expect("setbox operand scans");
            let context = processor.error_context();
            let next = processor
                .get_x_token()
                .expect("following command delivery succeeds")
                .expect("following command remains input");
            (assignment, next, context)
        };
        (assignment, next, context, rig.recorder.0)
    };

    let (forbidden, forbidden_next, forbidden_context, forbidden_observations) = scan(false);
    let ScannedSetBoxPath::Forbidden { error_context } = forbidden.path else {
        panic!("set_box_allowed=false must bypass scan_box");
    };
    assert_eq!(error_context, forbidden_context);
    assert!(matches!(
        forbidden_next.meaning(),
        Meaning::CharToken {
            ch: 'x',
            cat: Catcode::Letter
        }
    ));
    let forbidden_backups = forbidden_observations
        .iter()
        .filter(|observation| matches!(
            observation,
            CommandObservation::Input(record) if record.transition == crate::InputTransition::Backup
        ))
        .count();

    let (allowed, allowed_next, allowed_context, allowed_observations) = scan(true);
    assert_eq!(
        allowed.path,
        ScannedSetBoxPath::Payload(ScannedBoxShiftPayload::Missing)
    );
    assert!(matches!(
        allowed_next.meaning(),
        Meaning::CharToken {
            ch: 'x',
            cat: Catcode::Letter
        }
    ));
    let allowed_backups = allowed_observations
        .iter()
        .filter(|observation| matches!(
            observation,
            CommandObservation::Input(record) if record.transition == crate::InputTransition::Backup
        ))
        .count();
    assert_eq!(allowed_backups, forbidden_backups + 1);
    assert!(forbidden_context.contains("<recently read> ="));
    assert!(forbidden_context.contains("<to be read again> 0="));
    assert!(!allowed_context.contains("<recently read> ="));
    assert!(allowed_context.contains("<to be read again>"));
}

#[test]
fn math_scalar_requests_are_completed_before_replay() {
    let mut scenario = ProcessorScenario::plain();
    scenario.push([
        Token::Char {
            ch: '4',
            cat: Catcode::Other,
        },
        Token::Char {
            ch: '0',
            cat: Catcode::Other,
        },
        Token::Char {
            ch: '9',
            cat: Catcode::Other,
        },
        Token::Char {
            ch: '6',
            cat: Catcode::Other,
        },
        Token::Char {
            ch: ' ',
            cat: Catcode::Space,
        },
        Token::Char {
            ch: '0',
            cat: Catcode::Other,
        },
        Token::Char {
            ch: 'p',
            cat: Catcode::Letter,
        },
        Token::Char {
            ch: 't',
            cat: Catcode::Letter,
        },
    ]);
    let (character, fraction) = {
        let mut processor = scenario.processor();
        let character = processor
            .scan_math_character()
            .expect("math character scans");
        let fraction = processor
            .scan_math_fraction(MathFractionKind::Atop, false)
            .expect("atop request scans");
        (character, fraction)
    };
    assert_eq!(character.code, 4096);
    assert!(!character.recovered);
    assert_eq!(
        fraction.thickness,
        Some(tex_state::scaled::Scaled::from_raw(0))
    );
}

#[test]
fn character_definition_scanner_owns_target_optional_equals_and_integer() {
    let mut scenario = ProcessorScenario::plain();
    let target = scenario.universe.intern("definedchar").symbol();
    scenario.push([
        Token::Char {
            ch: ' ',
            cat: Catcode::Space,
        },
        Token::Cs(target),
        Token::Char {
            ch: '=',
            cat: Catcode::Other,
        },
        Token::Char {
            ch: '6',
            cat: Catcode::Other,
        },
        Token::Char {
            ch: '5',
            cat: Catcode::Other,
        },
    ]);

    let definition = scenario
        .processor()
        .scan_character_definition(RestrictedIntegerClass::CharacterCode, false)
        .expect("character definition scans");

    assert_eq!(definition.target, target);
    assert_eq!(definition.value, 65);
    assert_eq!(definition.scanned, 65);
    assert!(!definition.recovered);
}

#[test]
fn frozen_control_target_becomes_inaccessible_without_consuming_following_input() {
    let mut scenario = ProcessorScenario::plain();
    scenario.push([
        Token::frozen_relax(),
        Token::Char {
            ch: '6',
            cat: Catcode::Other,
        },
        Token::Char {
            ch: '5',
            cat: Catcode::Other,
        },
    ]);

    let definition = scenario
        .processor()
        .scan_character_definition(RestrictedIntegerClass::CharacterCode, false)
        .expect("frozen target recovers");

    assert_eq!(scenario.universe.resolve(definition.target), "inaccessible");
    assert_eq!(definition.value, 65, "the following operand remains owned");
    assert!(
        scenario
            .diagnostic_text()
            .contains("Missing control sequence inserted")
    );
}

#[test]
fn inaccessible_recovery_is_inserted_above_the_ordinary_backup() {
    // TeX82 §§1215, 314: `get_r_token` backs up the rejected character,
    // then `ins_error` makes the synthesized control sequence an `inserted`
    // level before `show_context` runs.
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    universe.set_int_param(tex_state::env::banks::IntParam::new(54), 10);
    let mut capabilities = CommandHostCapabilities::default();
    push(&mut command, text_tokens("x=65"));

    let definition = processor(&mut command, &mut universe, &mut capabilities)
        .scan_character_definition(RestrictedIntegerClass::CharacterCode, false)
        .expect("character definition recovers");

    assert_eq!(universe.resolve(definition.target), "inaccessible");
    let diagnostics = diagnostic_text(&universe);
    let expected_context = "\n<inserted text> \n                \\inaccessible \
\n<to be read again> \n                   x\
\n<to be read again> x\n                    =65";
    assert!(diagnostics.contains(expected_context), "{diagnostics:?}");
}

#[test]
fn missing_macro_target_reports_once_and_leaves_following_command() {
    // TeX82 §1215's `get_r_token` performs the whole `ins_error; restart`
    // episode. Its rejected `{` starts the definition; no executor recovery
    // remains, and the command after the balanced replacement stays unread.
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    push(&mut command, text_tokens("{}?"));

    let (definition, next) = {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        let definition = processor
            .scan_macro_definition(false)
            .expect("missing macro target recovers");
        let next = processor
            .get_x_token()
            .expect("following delivery succeeds")
            .expect("following command remains");
        (definition, next)
    };

    assert_eq!(universe.resolve(definition.target), "inaccessible");
    assert!(definition.parameter_text.is_empty());
    assert!(definition.replacement_text.is_empty());
    assert!(matches!(
        next.meaning(),
        Meaning::CharToken {
            ch: '?',
            cat: Catcode::Other
        }
    ));
    assert_eq!(
        diagnostic_text(&universe)
            .matches("Missing control sequence inserted")
            .count(),
        1
    );
}

#[test]
fn ordinary_error_backup_remains_to_be_read_again() {
    // TeX82 §§325, 314: a normal `back_input` used by number recovery is not
    // retyped as §327's inserted input.
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    let target = universe.intern("ordinary").symbol();
    push(
        &mut command,
        [
            Token::Cs(target),
            Token::Char {
                ch: 'x',
                cat: Catcode::Letter,
            },
        ],
    );

    processor(&mut command, &mut universe, &mut capabilities)
        .scan_character_definition(RestrictedIntegerClass::CharacterCode, false)
        .expect("missing number recovers");

    let diagnostics = diagnostic_text(&universe);
    assert!(
        diagnostics.contains(
            "! Missing number, treated as zero.\n<to be read again> \n                   x"
        ),
        "{diagnostics:?}"
    );
    assert!(!diagnostics.contains("<inserted text>"), "{diagnostics:?}");
}

/// TeX82 §1224 spells `\chardef`'s value scan as §434's `scan_char_num` and
/// `\mathchardef`'s as §436's `scan_fifteen_bit_int`, so an out-of-range
/// operand is already `cur_val=0` when the assignment reads it.
#[test]
fn character_definition_scanner_recovers_out_of_range_operands_to_zero() {
    for (class, digits, scanned) in [
        (RestrictedIntegerClass::CharacterCode, "256", 256),
        (RestrictedIntegerClass::FifteenBit, "32768", 32_768),
    ] {
        let mut command = CommandState::default();
        let mut universe = crate::test_harness::universe_with_plain_catcodes();
        let mut capabilities = CommandHostCapabilities::default();
        let target = universe.intern("definedbadchar").symbol();
        let mut tokens = vec![
            Token::Char {
                ch: ' ',
                cat: Catcode::Space,
            },
            Token::Cs(target),
            Token::Char {
                ch: '=',
                cat: Catcode::Other,
            },
        ];
        tokens.extend(digits.chars().map(|ch| Token::Char {
            ch,
            cat: Catcode::Other,
        }));
        push(&mut command, tokens);

        let definition = processor(&mut command, &mut universe, &mut capabilities)
            .scan_character_definition(class, false)
            .expect("character definition scans");

        assert_eq!(definition.target, target);
        assert_eq!(definition.value, 0);
        assert_eq!(definition.scanned, scanned);
        assert!(definition.recovered);
    }
}

#[test]
fn register_definition_scanner_owns_target_scope_equals_and_bounded_index() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    let target = universe.intern("definedregister").symbol();
    push(
        &mut command,
        [
            Token::Char {
                ch: ' ',
                cat: Catcode::Space,
            },
            Token::Cs(target),
            Token::Char {
                ch: '=',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: '2',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: '5',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: '6',
                cat: Catcode::Other,
            },
        ],
    );

    let definition = processor(&mut command, &mut universe, &mut capabilities)
        .scan_register_definition(true)
        .expect("register definition scans");

    assert_eq!(definition.target, target);
    assert_eq!(definition.index, 0);
    assert_eq!(universe.meaning(target), Meaning::Relax);
}

#[test]
fn font_definition_scanner_defines_the_null_font_before_scanning_operands() {
    // TeX82 §1257's `new_font` runs `define(u,set_font,null_font)` on the
    // `get_r_token` target before `scan_optional_equals` and `scan_file_name`,
    // so the identifier already denotes the null font while those operands are
    // delivered, and the observed mutation precedes them.
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    let target = universe.intern("tenrm").symbol();
    let mut tokens = vec![Token::Cs(target)];
    tokens.extend(text_tokens("=cmr10 "));
    push(&mut command, tokens);

    let mut recorder = Recorder::default();
    let request = {
        let mut processor =
            processor(&mut command, &mut universe, &mut capabilities).with_observer(&mut recorder);
        processor
            .scan_font_definition(false)
            .expect("font definition scans")
    };

    assert_eq!(request.target, target);
    assert_eq!(request.name, "cmr10");
    assert_eq!(
        universe.meaning(target),
        Meaning::Font(tex_state::font::NULL_FONT),
    );

    let mutation = recorder
        .0
        .iter()
        .position(|observation| {
            matches!(
                observation,
                CommandObservation::Mutation(record)
                    if record.target == MutationTarget::Meaning
                        && record.value == ObservationValue::Name("set_font".into())
                        && record.key == ObservationValue::Name("tenrm".into())
                        && !record.global
            )
        })
        .expect("the provisional null-font definition is observed");
    let equals = recorder
        .0
        .iter()
        .position(|observation| {
            matches!(
                observation,
                CommandObservation::Command(record) if record.command_operand == Some(i64::from('=' as u32))
            )
        })
        .expect("the optional equals sign is delivered");
    assert!(mutation < equals);
}

#[test]
fn generated_font_scanner_binds_null_before_source_and_scans_letterspace_tail() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    let target = universe.intern("spaced").symbol();
    universe.set_meaning_global(target, Meaning::Relax);
    let mut tokens = vec![Token::Cs(target)];
    tokens.extend(text_tokens("="));
    tokens.push(Token::Cs(target));
    tokens.extend(text_tokens(" 1200 nolig "));
    push(&mut command, tokens);

    let definition = processor(&mut command, &mut universe, &mut capabilities)
        .scan_generated_font_definition(GeneratedFontKind::Letterspace, false)
        .expect("letterspace definition scans");

    assert_eq!(definition.target, target);
    assert_eq!(definition.source, tex_state::font::NULL_FONT);
    assert_eq!(definition.amount, 1000);
    assert!(definition.no_ligatures);
    assert_eq!(
        universe.meaning(target),
        Meaning::Font(tex_state::font::NULL_FONT)
    );
}

#[test]
fn font_size_recovery_carries_the_backed_up_error_context() {
    // TeX82 §§82, 1258: `scan_int` leaves its lookahead under §325
    // `back_input`; the deferred stomach-side `int_error` must display that
    // exact command-owned stack state.
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    let target = universe.intern("oversized").symbol();
    let mut tokens = vec![Token::Cs(target)];
    tokens.extend(text_tokens("=cmr10 scaled 32769="));
    push(&mut command, tokens);

    let request = processor(&mut command, &mut universe, &mut capabilities)
        .scan_font_definition(false)
        .expect("font definition scans");
    let FontSizeRecovery::IllegalMagnification { value, context } =
        request.size_recovery.expect("illegal scale recovers")
    else {
        panic!("expected illegal magnification recovery");
    };
    assert_eq!(value, 32_769);
    assert_eq!(context, "\n<to be read again> \n                   =");
}

#[test]
fn show_context_labels_an_exhausted_backup_as_recently_read() {
    // TeX82 §530: a backed-up token list with `loc=null` names the token just
    // consumed, while a nonempty backup remains `<to be read again>`.
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    let font = universe.intern("font").symbol();
    push(&mut command, [Token::Cs(font)]);

    processor(&mut command, &mut universe, &mut capabilities)
        .scan_show()
        .expect("show operand scans");

    assert_eq!(
        command.output_open_context(&universe.command_context()),
        "\n<recently read> \\font \n                      "
    );
}

#[test]
fn show_prints_control_character_meaning_with_caret_notation() {
    // TeX82 §§49/59/298: `print_cmd_chr` prints a character through its
    // one-character string. The generated `^^Y` spelling is not rescanned
    // through the live `\newlinechar`, even when that parameter is `Y`.
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    universe.set_int_param(IntParam::NEWLINE_CHAR, i32::from(b'Y'));
    let mut capabilities = CommandHostCapabilities::default();
    push(
        &mut command,
        [Token::Char {
            ch: '\u{19}',
            cat: Catcode::Other,
        }],
    );

    let shown = processor(&mut command, &mut universe, &mut capabilities)
        .scan_show()
        .expect("show operand scans");
    assert_eq!(shown.content, "> the character ^^Y");
}

#[test]
fn show_moves_singular_mark_contents_to_the_next_line() {
    // TeX82 §296 calls `print_ln` between a singular mark's colon and its
    // token list. An empty list still owns the colon and line break.
    for (contents, expected) in [
        ("", "> \\botmark=\\botmark:\n"),
        ("0.", "> \\botmark=\\botmark:\n0."),
    ] {
        let mut command = CommandState::default();
        let mut universe = crate::test_harness::universe_with_plain_catcodes();
        let mut capabilities = CommandHostCapabilities::default();
        let botmark = universe.intern("botmark").symbol();
        universe.set_meaning(
            botmark,
            Meaning::ExpandablePrimitive(ExpandablePrimitive::BotMark),
        );
        let tokens = universe.intern_token_list(&text_tokens(contents));
        universe.set_page_mark(tex_state::page::PageMark::Bot, tokens);
        push(&mut command, [Token::Cs(botmark)]);

        let shown = processor(&mut command, &mut universe, &mut capabilities)
            .scan_show()
            .expect("show operand scans");
        assert_eq!(shown.content, expected);
    }
}

#[test]
fn show_of_end_template_alias_retains_outer_identity_and_empty_body_line() {
    // TeX82 §§296, 298, 780: `end_template` is an intrinsically outer
    // call-class command. Its empty token list still puts the completion
    // diagnostic on the line after the colon.
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    let alias = universe.intern("endt").symbol();
    universe.set_meaning(
        alias,
        Meaning::ExpandablePrimitive(ExpandablePrimitive::EndTemplate),
    );
    push(&mut command, [Token::Cs(alias)]);

    let shown = processor(&mut command, &mut universe, &mut capabilities)
        .scan_show()
        .expect("show operand scans");
    assert_eq!(shown.content, "> \\endt=\\outer endtemplate:\n");
}

#[test]
fn show_does_not_scan_or_render_etex_mark_class_contents() {
    // e-TeX change [20.296] leaves plural mark enquiries at print_cmd_chr:
    // `\show\botmarks` neither scans a class number nor appends class zero.
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    let botmarks = universe.intern("botmarks").symbol();
    universe.set_meaning(
        botmarks,
        Meaning::ExpandablePrimitive(ExpandablePrimitive::BotMarks),
    );
    let tokens = universe.intern_token_list(&text_tokens("hidden"));
    universe.set_page_mark_class(tex_state::page::PageMark::Bot, 0, tokens);
    push(&mut command, [Token::Cs(botmarks)]);

    let shown = processor(&mut command, &mut universe, &mut capabilities)
        .scan_show()
        .expect("show operand scans");
    assert_eq!(shown.content, "> \\botmarks=\\botmarks");
}

#[test]
fn math_field_brace_opens_group_without_absorbing_its_body() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    push(
        &mut command,
        [
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Char {
                ch: 'x',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
        ],
    );
    let field = {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        processor.scan_math_field_episode().expect("field scans")
    };
    // TeX82 §1153 consumes only the mandatory brace; the body stays live
    // input for `push_math`'s `math_group`, so no replay level is opened and
    // the very next delivery is the body's own first token.
    assert_eq!(field.body, MathFieldBody::OpenGroup);
    let mut delivered = Vec::new();
    for _ in 0..4 {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        delivered.push(
            processor
                .get_x_token()
                .expect("delivery succeeds")
                .expect("body token arrives")
                .spelling()
                .semantic_token(),
        );
    }
    assert_eq!(
        delivered,
        vec![
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Char {
                ch: 'x',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
        ]
    );
}

#[test]
fn replay_completion_precedes_parent_delivery() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    push(
        &mut command,
        [Token::Char {
            ch: 'z',
            cat: Catcode::Letter,
        }],
    );
    // A `\discretionary` part is the vehicle: it is command-owned material
    // TeX82 §1117 reads as its own list. No math scan opens an episode --
    // §1151's scalar field is classified in place, and §1153's braced field
    // and §1172's `\mathchoice` branches are live input.
    let part = universe.finish_traced_token_list(&[traced(Token::Char {
        ch: 'a',
        cat: Catcode::Letter,
    })]);
    let episode = command.push_discretionary_episode(&universe.command_context(), part);

    let first = {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        processor
            .get_x_token_with_replay_completion()
            .expect("episode token delivers")
            .expect("episode token exists")
    };
    assert!(matches!(
        first,
        CommandReplayDelivery::Command(ref command)
            if command.spelling().semantic_token()
                == Token::Char { ch: 'a', cat: Catcode::Letter }
    ));

    let completed = {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        processor
            .get_x_token_with_replay_completion()
            .expect("completion delivers")
            .expect("episode completion exists")
    };
    assert!(matches!(
        completed,
        CommandReplayDelivery::Completed(completed) if completed == episode
    ));

    let parent = {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        processor
            .get_x_token_with_replay_completion()
            .expect("parent delivery succeeds")
            .expect("parent command exists")
    };
    assert!(matches!(
        parent,
        CommandReplayDelivery::Command(ref command)
            if command.spelling().semantic_token()
                == Token::Char { ch: 'z', cat: Catcode::Letter }
    ));
}

#[test]
fn output_replay_completion_follows_final_macro_replacement() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    let value = universe.intern("value").symbol();
    let parameters = universe.intern_token_list(&[]);
    let replacement = universe.intern_token_list(&text_tokens("TWO"));
    let definition = universe.intern_macro(MacroMeaning::new(
        MeaningFlags::EMPTY,
        parameters,
        replacement,
    ));
    universe.set_meaning(
        value,
        Meaning::Macro {
            flags: MeaningFlags::EMPTY,
            definition: definition.id(),
        },
    );
    push(&mut command, text_tokens("PARENT"));
    let replay = universe.finish_traced_token_list(
        &text_tokens("DEFERRED-")
            .into_iter()
            .chain([Token::Cs(value)])
            .map(traced)
            .collect::<Vec<_>>(),
    );

    let expanded = {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        processor
            .expand_output_replay(replay)
            .expect("output replay expands")
    };
    assert_eq!(
        universe.tokens(expanded.token_list()).tokens(),
        text_tokens("DEFERRED-TWO")
    );
    let parent = {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        processor
            .get_x_token()
            .expect("parent delivery succeeds")
            .expect("parent remains input")
    };
    assert_eq!(
        parent.spelling().semantic_token(),
        Token::Char {
            ch: 'P',
            cat: Catcode::Letter,
        }
    );
}

#[test]
fn math_choice_group_consumes_only_its_opening_brace() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    push(
        &mut command,
        [
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Char {
                ch: 'a',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
        ],
    );
    // TeX82 §1172/§1174 scan the mandatory brace and nothing else: the
    // branch body is live input the stomach reads through main control, so
    // no episode is opened and the first body token is delivered next.
    let recovered = {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        processor
            .scan_math_choice_group()
            .expect("branch brace scans")
    };
    assert!(!recovered);
    let body = {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        processor
            .get_x_token()
            .expect("body stays live")
            .expect("body token exists")
    };
    assert_eq!(
        body.spelling().semantic_token(),
        Token::Char {
            ch: 'a',
            cat: Catcode::Letter
        }
    );
}

#[test]
fn missing_math_choice_brace_recovers_without_consuming_rejected_command() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    push(
        &mut command,
        [Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        }],
    );
    let recovered = {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        processor
            .scan_math_choice_group()
            .expect("recovery completes")
    };
    // TeX82 §403 backs the rejected command up and proceeds as though a `{`
    // had been read, so the group still opens and `x` becomes the first
    // token of the branch body.
    assert!(recovered);
    let replayed = {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        processor
            .get_x_token()
            .expect("replay remains command owned")
            .expect("rejected token replays")
    };
    assert_eq!(
        replayed.spelling().semantic_token(),
        Token::Char {
            ch: 'x',
            cat: Catcode::Letter
        }
    );
}

/// TeX82 §1151's scalar cases -- `letter`, `other_char`, `char_given`,
/// `char_num`, `math_char_num`, `math_given`, `delim_num` -- each end by
/// assigning one math code `c`. None of them pushes an input level, backs a
/// token up, or re-reads the command that selected the case, so the very
/// next delivery is the token that follows the field (`umber2-johp.265`).
#[test]
fn math_field_scalar_case_resolves_without_replaying_its_command() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    push(
        &mut command,
        [
            Token::Char {
                ch: 'x',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: 'z',
                cat: Catcode::Letter,
            },
        ],
    );
    let mut recorder = Recorder::default();
    let field = {
        let mut processor =
            processor(&mut command, &mut universe, &mut capabilities).with_observer(&mut recorder);
        processor.scan_math_field_episode().expect("field scans")
    };

    assert_eq!(
        field.body,
        MathFieldBody::Character(universe.mathcode('x') as u16)
    );
    assert!(
        !recorder
            .0
            .iter()
            .any(|observation| matches!(observation, CommandObservation::Input(_))),
        "§1151 opens and retires no input level for a scalar field"
    );
    let next = {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        processor
            .get_x_token()
            .expect("delivery succeeds")
            .expect("the following token arrives")
            .spelling()
            .semantic_token()
    };
    assert_eq!(
        next,
        Token::Char {
            ch: 'z',
            cat: Catcode::Letter
        }
    );
}

/// §1151's `char_num`, `math_char_num`, and `delim_num` cases scan their own
/// operand and reduce to the same math code: `\char` re-enters the table as
/// `char_given`, `\mathchar` takes §436's fifteen-bit value directly, and
/// `\delimiter` takes §437's twenty-seven-bit value `div @'10000`.
#[test]
fn math_field_operand_cases_reduce_to_one_math_code() {
    for (primitive, text, expected) in [
        (UnexpandablePrimitive::Char, "`x ", u32::from('x')),
        (UnexpandablePrimitive::MathChar, "\"3161 ", 0x3161),
        (UnexpandablePrimitive::Delimiter, "\"1161361 ", 0x1161),
    ] {
        let mut command = CommandState::default();
        let mut universe = crate::test_harness::universe_with_plain_catcodes();
        let mut capabilities = CommandHostCapabilities::default();
        let symbol = universe.intern("p").symbol();
        universe.set_meaning(symbol, Meaning::UnexpandablePrimitive(primitive));
        let mut tokens = vec![Token::Cs(symbol)];
        tokens.extend(text_tokens(text));
        push(&mut command, tokens);
        let field = {
            let mut processor = processor(&mut command, &mut universe, &mut capabilities);
            processor.scan_math_field_episode().expect("field scans")
        };
        let expected = if primitive == UnexpandablePrimitive::Char {
            universe.mathcode(char::from_u32(expected).expect("a character code"))
        } else {
            expected
        };
        assert_eq!(field.body, MathFieldBody::Character(expected as u16));
    }
}

/// TeX82 §1151's `math_given` case copies the complete fifteen-bit value into
/// `c`, but its scalar result remains a `math_char` for every class nibble.
/// Classification happens on the already-delivered meaning: no replay input
/// level or recovery delivery is introduced for non-Ord values.
#[test]
fn math_given_field_preserves_every_non_ord_code_without_input_events() {
    for class in 1_u16..=7 {
        let code = (class << 12) | 0x13a;
        let mut command = CommandState::default();
        let mut universe = crate::test_harness::universe_with_plain_catcodes();
        let mut capabilities = CommandHostCapabilities::default();
        let symbol = universe.intern("field").symbol();
        universe.set_meaning(symbol, Meaning::MathCharGiven(code));
        push(&mut command, [Token::Cs(symbol)]);
        let mut recorder = Recorder::default();

        let field = {
            let mut processor = processor(&mut command, &mut universe, &mut capabilities)
                .with_observer(&mut recorder);
            processor.scan_math_field_episode().expect("field scans")
        };

        assert_eq!(field.body, MathFieldBody::Character(code));
        assert!(
            !recorder
                .0
                .iter()
                .any(|observation| matches!(observation, CommandObservation::Input(_))),
            "class {class} opens and retires no replay input level"
        );
    }
}

/// §1151's `othercases` is the whole rest of the vocabulary, not just a left
/// brace: it is §1153's `back_input; scan_left_brace`, and §403's recovery
/// reaches §1153 with `cur_cmd = left_brace`. The `math_group` therefore
/// opens either way, and the rejected command -- already backed up by §403 --
/// becomes the first token of the live subformula body.
#[test]
fn math_field_rejects_a_non_field_command_into_an_open_group() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    let symbol = universe.intern("hbox").symbol();
    universe.set_meaning(
        symbol,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::HBox),
    );
    push(&mut command, [Token::Cs(symbol)]);
    let field = {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        processor.scan_math_field_episode().expect("field scans")
    };

    assert_eq!(field.body, MathFieldBody::OpenGroup);
    assert_eq!(
        command.alignment.align_state,
        crate::processor::TOP_LEVEL_ALIGN_STATE + 1
    );
    let next = {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        processor
            .get_x_token()
            .expect("delivery succeeds")
            .expect("the rejected command opens the body")
            .spelling()
            .semantic_token()
    };
    assert_eq!(next, Token::Cs(symbol));
}

#[test]
fn math_episode_observation_does_not_change_frozen_command_state() {
    let mut plain = CommandState::default();
    let mut observed = CommandState::default();
    let tokens = [
        Token::Char {
            ch: '{',
            cat: Catcode::BeginGroup,
        },
        Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        },
        Token::Char {
            ch: '}',
            cat: Catcode::EndGroup,
        },
    ];
    push(&mut plain, tokens);
    push(&mut observed, tokens);
    let mut plain_universe = crate::test_harness::universe_with_plain_catcodes();
    let mut observed_universe = crate::test_harness::universe_with_plain_catcodes();
    let mut plain_capabilities = CommandHostCapabilities::default();
    let mut observed_capabilities = CommandHostCapabilities::default();
    let plain_field = {
        let mut processor = processor(&mut plain, &mut plain_universe, &mut plain_capabilities);
        processor
            .scan_math_field_episode()
            .expect("plain field scans")
    };
    let mut recorder = Recorder::default();
    let observed_field = {
        let mut processor = processor(
            &mut observed,
            &mut observed_universe,
            &mut observed_capabilities,
        )
        .with_observer(&mut recorder);
        processor
            .scan_math_field_episode()
            .expect("observed field scans")
    };
    assert_eq!(plain_field.body, observed_field.body);
    assert_eq!(plain_field.provenance, observed_field.provenance);
    assert_eq!(plain.snapshot(), observed.snapshot());
    assert!(!recorder.0.is_empty());
}

#[test]
fn math_delimiter_and_mu_requests_recover_and_consume_units() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    push(
        &mut command,
        [
            Token::Char {
                ch: '9',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: '9',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: '9',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: '9',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: '9',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: '9',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: '9',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: '9',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: '9',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: ' ',
                cat: Catcode::Space,
            },
            Token::Char {
                ch: '2',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: 'm',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: 'u',
                cat: Catcode::Letter,
            },
        ],
    );
    let (delimiter, material) = {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        let delimiter = processor.scan_delimiter_number().expect("delimiter scans");
        let material = processor
            .scan_math_mu_material(false)
            .expect("mu kern scans");
        (delimiter, material)
    };
    assert_eq!(delimiter.code, 0);
    assert!(delimiter.recovered);
    assert_eq!(
        material,
        ScannedMathMuMaterial::Kern(tex_state::scaled::Scaled::from_raw(131_072))
    );
}

#[test]
fn generalized_fraction_delimiters_read_delimiter_codes_not_integers() {
    // TeX82 §1182's `\abovewithdelims` family runs
    // `scan_delimiter(...,false)` twice, so each delimiter is §1160's
    // classified token -- here a letter/other_char whose `\delcode` is the
    // value -- and never a bare `scan_twenty_seven_bit_int`. Scanning them as
    // integers made `\abovewithdelims()3pt` read `()` as a vacuous number and
    // consumed the fraction's own operands as digits.
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    universe.set_delcode('(', 0x02_8300);
    universe.set_delcode(')', 0x02_9301);
    push(&mut command, text_tokens("()3pt 16"));
    let (fraction, family) = {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        let fraction = processor
            .scan_math_fraction(MathFractionKind::Above, true)
            .expect("fraction scans");
        let family = processor
            .scan_math_family(MathFamilySize::Text)
            .expect("family scans");
        (fraction, family)
    };
    assert_eq!(fraction.left_delimiter.expect("left").code, 0x02_8300);
    assert_eq!(fraction.right_delimiter.expect("right").code, 0x02_9301);
    assert_eq!(
        fraction.thickness,
        Some(tex_state::scaled::Scaled::from_raw(196_608))
    );
    assert_eq!(family.family, 0);
    assert!(family.recovered);
}

#[test]
fn a_non_radical_delimiter_consumes_delimiter_in_place_and_backs_up_nothing_else() {
    // TeX82 §1160's `delim_num` case runs `scan_twenty_seven_bit_int` on the
    // command §404 already delivered, so `\left\delimiter"4266308` reads one
    // delimiter and installs no backup level. Treating the whole `r=false`
    // position as `scan_twenty_seven_bit_int` made `\delimiter` the first
    // token of a `scan_int`, which §444's `vacuous` case backed up and
    // redelivered.
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    let delimiter = universe.intern("delimiter").symbol();
    universe.set_meaning(
        delimiter,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Delimiter),
    );
    let mut tokens = vec![Token::Cs(delimiter)];
    tokens.extend(text_tokens("\"4266308 x"));
    push(&mut command, tokens);
    let mut recorder = Recorder::default();
    let (boundary, next) = {
        let mut processor =
            processor(&mut command, &mut universe, &mut capabilities).with_observer(&mut recorder);
        let boundary = processor
            .scan_math_delimiter_boundary(MathDelimiterBoundaryKind::Left)
            .expect("boundary scans");
        let next = processor
            .get_x_token()
            .expect("next token")
            .expect("present");
        (boundary, next)
    };
    assert_eq!(boundary.kind, MathDelimiterBoundaryKind::Left);
    assert_eq!(boundary.delimiter.code, 0x0426_6308);
    assert!(!boundary.delimiter.recovered);
    assert!(matches!(
        next.meaning(),
        Meaning::CharToken {
            ch: 'x',
            cat: Catcode::Letter
        }
    ));
    assert!(
        !recorder.0.iter().any(|observation| matches!(
            observation,
            CommandObservation::Recovery(record) if record.kind == RecoveryKind::Backup
        )),
        "§1160 consumes the delivered \\delimiter in place: {:?}",
        recorder.0
    );
}

#[test]
fn a_non_radical_delimiter_backs_up_a_token_with_no_delimiter_code() {
    // TeX82 §1160's `othercases cur_val:=-1` and §1161's `back_error`: the
    // rejected token returns to the input and the delimiter becomes null.
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    push(&mut command, text_tokens("  x"));
    let (boundary, next) = {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        let boundary = processor
            .scan_math_delimiter_boundary(MathDelimiterBoundaryKind::Right)
            .expect("boundary scans");
        let next = processor
            .get_x_token()
            .expect("next token")
            .expect("present");
        (boundary, next)
    };
    assert_eq!(boundary.delimiter.code, 0);
    assert!(boundary.delimiter.recovered);
    assert!(matches!(
        next.meaning(),
        Meaning::CharToken {
            ch: 'x',
            cat: Catcode::Letter
        }
    ));
}

#[test]
fn balanced_text_and_macro_definition_freeze_typed_lists_with_provenance() {
    let mut command = CommandState::default();
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(b"{xy}".as_slice()),
        ))
        .expect("source registers");
    command
        .open_registered_source(source)
        .expect("source opens");
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let target = universe.intern("defined").symbol();
    let mut capabilities = CommandHostCapabilities::default();
    let snapshot = command.snapshot();
    let balanced = {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        processor
            .scan_balanced_text(false)
            .expect("balanced text scans")
    };
    let provenance = balanced.provenance;
    assert_eq!(
        universe.tokens(balanced.tokens.token_list()),
        &[
            Token::Char {
                ch: 'x',
                cat: Catcode::Letter
            },
            Token::Char {
                ch: 'y',
                cat: Catcode::Letter
            }
        ]
    );
    command
        .rollback(snapshot)
        .expect("balanced scan rolls back exactly");
    let replayed = {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        processor
            .scan_balanced_text(false)
            .expect("balanced replay scans")
    };
    assert_eq!(replayed.provenance, provenance);

    push(
        &mut command,
        [
            Token::Cs(target),
            Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
            },
            Token::Char {
                ch: '1',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
            },
            Token::Char {
                ch: '1',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
        ],
    );
    let definition = {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        processor
            .scan_macro_definition(false)
            .expect("definition scans")
    };
    assert_eq!(definition.target, target);
    assert_eq!(
        definition
            .parameter_text
            .words()
            .iter()
            .map(|word| word.semantic_token())
            .collect::<Vec<_>>(),
        &[Token::Param(1)]
    );
    assert_eq!(
        definition
            .replacement_text
            .words()
            .iter()
            .map(|word| word.semantic_token())
            .collect::<Vec<_>>(),
        &[Token::Param(1)]
    );
}

#[test]
fn expanded_macro_definition_splices_the_spacefactor_from_the_host() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let target = universe.intern("captured_space_factor").symbol();
    let the = universe.intern("the").symbol();
    let space_factor = universe.intern("spacefactor").symbol();
    universe.set_meaning(the, Meaning::ExpandablePrimitive(ExpandablePrimitive::The));
    universe.set_meaning(
        space_factor,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::SpaceFactor),
    );
    let mut capabilities = CommandHostCapabilities::default();
    capabilities.set_space_factor(Some(1000));
    let mut recorder = Recorder::default();
    push(
        &mut command,
        [
            Token::Cs(target),
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Cs(the),
            Token::Cs(space_factor),
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
        ],
    );

    let definition = processor(&mut command, &mut universe, &mut capabilities)
        .with_observer(&mut recorder)
        .scan_macro_definition(true)
        .expect("expanded definition scans the current space factor");

    assert_eq!(definition.target, target);
    assert_eq!(
        definition
            .replacement_text
            .words()
            .iter()
            .map(|word| word.semantic_token())
            .collect::<Vec<_>>(),
        &[
            Token::Char {
                ch: '1',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: '0',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: '0',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: '0',
                cat: Catcode::Other,
            },
        ]
    );
    assert!(recorder.0.iter().any(|observation| {
        matches!(
            observation,
            CommandObservation::TokenList(record)
                if record.transition == "splice"
                    && record.purpose == "the_toks"
                    && record.tokens
                        == [
                            crate::ObservedToken::Character {
                                character: '1',
                                catcode: Catcode::Other,
                            },
                            crate::ObservedToken::Character {
                                character: '0',
                                catcode: Catcode::Other,
                            },
                            crate::ObservedToken::Character {
                                character: '0',
                                catcode: Catcode::Other,
                            },
                            crate::ObservedToken::Character {
                                character: '0',
                                catcode: Catcode::Other,
                            },
                        ]
        )
    }));
}

#[test]
fn balanced_text_enters_absorbing_before_its_opening_brace() {
    let mut command = CommandState::default();
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(b"{x}".as_slice()),
        ))
        .expect("source registers");
    command
        .open_registered_source(source)
        .expect("source opens");
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();

    processor(&mut command, &mut universe, &mut capabilities)
        .with_observer(&mut recorder)
        .scan_balanced_text(true)
        .expect("balanced text scans");

    assert!(matches!(
        recorder.0.as_slice(),
        [
            CommandObservation::ScannerStatus(status),
            CommandObservation::Command(opening),
            ..
        ] if status.from == "normal"
            && status.to == "absorbing"
            && matches!(opening.spelling, crate::ObservedToken::Character {
                character: '{', catcode: Catcode::BeginGroup
            })
    ));
}

#[test]
fn special_shipout_probe_is_owned_only_by_the_pdftex_profile() {
    fn scan(profile: crate::CommandProfile, source: &[u8]) -> (bool, Vec<CommandObservation>) {
        let mut command = CommandState::new(profile);
        let source = command
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(source),
            ))
            .expect("source registers");
        command
            .open_registered_source(source)
            .expect("source opens");
        let mut universe = crate::test_harness::universe_with_plain_catcodes();
        let mut capabilities = CommandHostCapabilities::default();
        let mut recorder = Recorder::default();
        let (deferred, _) = processor(&mut command, &mut universe, &mut capabilities)
            .with_observer(&mut recorder)
            .scan_special()
            .expect("special scans");
        (deferred, recorder.0)
    }

    // TeX82 §473 enters `scan_toks` directly; e-TeX does not acquire
    // pdfTeX 1.40.29 §1534's optional `shipout` syntax.
    let (deferred, events) = scan(crate::CommandProfile::ETEX26, b"{x}");
    assert!(!deferred);
    assert!(matches!(
        events.as_slice(),
        [
            CommandObservation::ScannerStatus(status),
            CommandObservation::Command(opening),
            ..
        ] if status.from == "normal"
            && status.to == "absorbing"
            && matches!(opening.spelling, crate::ObservedToken::Character {
                character: '{', catcode: Catcode::BeginGroup
            })
    ));

    let (deferred, _) = scan(crate::CommandProfile::PDFTEX14029, b"shipout{x}");
    assert!(deferred);
}

#[test]
fn discretionary_delivers_each_opening_brace_before_body_collection() {
    // TeX82 §1117 consumes only the opening brace before returning to live
    // main control. The body and later parts must remain untouched.
    let mut command = CommandState::default();
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(b"{a}{b}{c}".as_slice()),
        ))
        .expect("source registers");
    command
        .open_registered_source(source)
        .expect("source opens");
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();

    processor(&mut command, &mut universe, &mut capabilities)
        .with_observer(&mut recorder)
        .scan_discretionary_opening()
        .expect("opening scans");

    assert_eq!(
        recorder
            .0
            .iter()
            .filter(|event| matches!(
                event,
                CommandObservation::Command(command)
                    if matches!(
                        command.spelling,
                        crate::ObservedToken::Character {
                            character: '{',
                            catcode: Catcode::BeginGroup
                        }
                    )
            ))
            .count(),
        2
    );
    assert!(!recorder.0.iter().any(|event| matches!(
        event,
        CommandObservation::ScannerStatus(status) if status.to == "absorbing"
    )));
    let next = processor(&mut command, &mut universe, &mut capabilities)
        .get_x_token()
        .expect("body remains live")
        .expect("body command exists");
    assert_eq!(
        next.spelling().semantic_token(),
        Token::Char {
            ch: 'a',
            cat: Catcode::Letter
        }
    );
}

#[test]
fn expanded_balanced_text_uses_canonical_macro_argument_matching() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let macro_name = universe.intern("arg").symbol();
    let parameters = universe.intern_token_list(&[Token::Param(1)]);
    let replacement = universe.intern_token_list(&[Token::Param(1)]);
    let definition = universe.intern_macro(MacroMeaning::new(
        MeaningFlags::EMPTY,
        parameters,
        replacement,
    ));
    universe.set_meaning(
        macro_name,
        Meaning::Macro {
            flags: MeaningFlags::EMPTY,
            definition: definition.id(),
        },
    );
    push(
        &mut command,
        [
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Cs(macro_name),
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Char {
                ch: 'q',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
        ],
    );
    let mut capabilities = CommandHostCapabilities::default();
    let scanned = {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        processor
            .scan_balanced_text(true)
            .expect("macro argument expands")
    };
    assert_eq!(
        universe.tokens(scanned.tokens.token_list()),
        &[Token::Char {
            ch: 'q',
            cat: Catcode::Letter
        }]
    );
}

#[test]
fn rule_spec_scans_expanded_keywords_and_dimensions() {
    let mut command = CommandState::default();
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(b"width1pt height2pt depth0pt!".as_slice()),
        ))
        .expect("source registers");
    command
        .open_registered_source(source)
        .expect("source opens");
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    let spec = processor(&mut command, &mut universe, &mut capabilities)
        .scan_rule_spec(UnexpandablePrimitive::VRule)
        .expect("rule spec scans");

    assert_eq!(spec.width.map(Scaled::raw), Some(Scaled::UNITY));
    assert_eq!(spec.height.map(Scaled::raw), Some(2 * Scaled::UNITY));
    assert_eq!(spec.depth.map(Scaled::raw), Some(0));
    let terminator = processor(&mut command, &mut universe, &mut capabilities)
        .get_x_token()
        .expect("terminator delivers")
        .expect("terminator exists");
    assert!(matches!(
        terminator.meaning(),
        Meaning::CharToken { ch: '!', .. }
    ));
}

/// TeX.web §463 starts rules with orientation-specific running dimensions
/// and the canonical 0.4pt thickness. Every subsequently scanned dimension
/// assignment replaces the preceding value for that keyword.
#[test]
fn rule_spec_defaults_and_last_keyword_are_observable() {
    let default_rule = Scaled::from_raw(26_214);
    let explicit = ScannedRuleSpec {
        width: Some(Scaled::from_raw(Scaled::UNITY)),
        height: Some(Scaled::from_raw(2 * Scaled::UNITY)),
        depth: Some(Scaled::from_raw(3 * Scaled::UNITY)),
    };
    let repeated = ScannedRuleSpec {
        width: Some(Scaled::from_raw(4 * Scaled::UNITY)),
        height: Some(Scaled::from_raw(5 * Scaled::UNITY)),
        depth: Some(Scaled::from_raw(6 * Scaled::UNITY)),
    };
    let mut cases = vec![
        (
            "bare vrule",
            UnexpandablePrimitive::VRule,
            "!",
            ScannedRuleSpec {
                width: Some(default_rule),
                height: None,
                depth: None,
            },
        ),
        (
            "bare hrule",
            UnexpandablePrimitive::HRule,
            "!",
            ScannedRuleSpec {
                width: None,
                height: Some(default_rule),
                depth: Some(Scaled::from_raw(0)),
            },
        ),
        (
            "repeated keywords on vrule",
            UnexpandablePrimitive::VRule,
            "width1pt height2pt depth3pt width4pt height5pt depth6pt!",
            repeated,
        ),
        (
            "repeated keywords on hrule",
            UnexpandablePrimitive::HRule,
            "width1pt height2pt depth3pt width4pt height5pt depth6pt!",
            repeated,
        ),
    ];
    for (name, source_text) in [
        ("width-height-depth", "width1pt height2pt depth3pt!"),
        ("width-depth-height", "width1pt depth3pt height2pt!"),
        ("height-width-depth", "height2pt width1pt depth3pt!"),
        ("height-depth-width", "height2pt depth3pt width1pt!"),
        ("depth-width-height", "depth3pt width1pt height2pt!"),
        ("depth-height-width", "depth3pt height2pt width1pt!"),
    ] {
        cases.push((name, UnexpandablePrimitive::VRule, source_text, explicit));
        cases.push((name, UnexpandablePrimitive::HRule, source_text, explicit));
    }

    for (name, primitive, source_text, expected) in cases {
        let mut command = CommandState::default();
        push(&mut command, text_tokens(source_text));
        let mut universe = crate::test_harness::universe_with_plain_catcodes();
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);

        assert_eq!(
            processor
                .scan_rule_spec(primitive)
                .expect("rule specification scans"),
            expected,
            "{name}",
        );
        assert!(matches!(
            processor
                .get_x_token()
                .expect("terminator delivers")
                .expect("terminator exists")
                .meaning(),
            Meaning::CharToken { ch: '!', .. }
        ));
        assert!(
            processor
                .get_x_token()
                .expect("input exhausts after the terminator")
                .is_none(),
            "{name} left a keyword or dimension token behind",
        );
    }
}

#[test]
fn accent_scanner_separates_the_accent_code_from_the_base_lookahead() {
    let mut command = CommandState::default();
    push(
        &mut command,
        [
            Token::Char {
                ch: '1',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: '8',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: ' ',
                cat: Catcode::Space,
            },
            Token::Char {
                ch: 'A',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: '!',
                cat: Catcode::Other,
            },
        ],
    );
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    let accent = processor(&mut command, &mut universe, &mut capabilities)
        .scan_accent()
        .expect("accent operands scan");
    assert_eq!(accent.accent, 18);
    let base = processor(&mut command, &mut universe, &mut capabilities)
        .scan_accent_base()
        .expect("base lookahead scans");
    assert!(matches!(
        base,
        ScannedAccentBase::Character {
            character: b'A',
            ..
        }
    ));

    let punctuation = processor(&mut command, &mut universe, &mut capabilities)
        .get_x_token()
        .expect("punctuation delivers")
        .expect("punctuation exists");
    assert!(matches!(
        punctuation.meaning(),
        Meaning::CharToken { ch: '!', .. }
    ));
}

/// TeX82 §1123 reaches §1124 through §1270's `do_assignments`, whose
/// `prefixed_command` executes the command §404 stopped on *in place*. The
/// base lookahead therefore hands a prefixed command back still delivered
/// rather than replaying it: a `back_input` here would push a backup level,
/// emit a recovery record and deliver the command a second time, none of
/// which tex.web does (`umber2-johp.264`).
#[test]
fn accent_base_lookahead_hands_a_prefixed_command_back_unreplayed() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let target = universe.intern("advance").symbol();
    universe.set_meaning(
        target,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Advance),
    );
    push(
        &mut command,
        [
            Token::Char {
                ch: ' ',
                cat: Catcode::Space,
            },
            Token::Cs(target),
        ],
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    let base = processor(&mut command, &mut universe, &mut capabilities)
        .with_observer(&mut recorder)
        .scan_accent_base()
        .expect("base lookahead scans");

    let ScannedAccentBase::Assignment(handed_back) = base else {
        panic!("a prefixed command is handed back, not classified as a base");
    };
    assert_eq!(
        handed_back.meaning(),
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Advance)
    );
    assert!(
        !recorder.0.iter().any(|observation| matches!(
            observation,
            CommandObservation::Recovery(_) | CommandObservation::Input(_)
        )),
        "handing the command back must push no input level and record no recovery: {:?}",
        recorder.0
    );
}

#[test]
fn assignment_loop_skips_relaxations_and_stops_before_following_token() {
    // TeX82 §§1270--1271's `do_assignments` repeatedly uses the same
    // non-blank, non-relax, non-call fetch. It stops with the first command
    // above `max_non_prefixed_command` still delivered for
    // `prefixed_command`; after that assignment, the following token is the
    // next base lookup rather than part of the assignment.
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let assignment = universe.intern("advance").symbol();
    universe.set_meaning(
        assignment,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Advance),
    );
    let relaxation = universe.intern("relax").symbol();
    universe.set_meaning(relaxation, Meaning::Relax);
    push(
        &mut command,
        [
            Token::Char {
                ch: ' ',
                cat: Catcode::Space,
            },
            Token::Cs(relaxation),
            Token::Cs(assignment),
            Token::Char {
                ch: 'x',
                cat: Catcode::Letter,
            },
        ],
    );
    let mut capabilities = CommandHostCapabilities::default();

    let first = processor(&mut command, &mut universe, &mut capabilities)
        .scan_accent_base()
        .expect("assignment-loop command scans");
    assert!(matches!(first, ScannedAccentBase::Assignment(command)
        if command.meaning() == Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Advance)));

    let second = processor(&mut command, &mut universe, &mut capabilities)
        .scan_accent_base()
        .expect("following token remains available");
    assert!(matches!(
        second,
        ScannedAccentBase::Character {
            character: b'x',
            ..
        }
    ));
}

/// The other half of §1124: a command that is neither a base character nor a
/// prefixed command takes tex.web's `else back_input`, and that replay stays
/// inside the delivery episode that fetched it.
#[test]
fn accent_base_lookahead_replays_a_command_that_is_neither_base_nor_assignment() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let target = universe.intern("hbox").symbol();
    universe.set_meaning(
        target,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::HBox),
    );
    push(&mut command, [Token::Cs(target)]);
    let mut capabilities = CommandHostCapabilities::default();
    let base = processor(&mut command, &mut universe, &mut capabilities)
        .scan_accent_base()
        .expect("base lookahead scans");
    assert!(matches!(base, ScannedAccentBase::Missing));

    let replayed = processor(&mut command, &mut universe, &mut capabilities)
        .get_x_token()
        .expect("replayed command delivers")
        .expect("replayed command exists");
    assert_eq!(
        replayed.meaning(),
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::HBox)
    );
}

#[test]
fn rule_spec_starts_v_template_when_scalar_lookahead_hits_cell_delimiters() {
    for (name, primitive, expected) in [
        ("tab", None, crate::AlignmentCellDelimiter::Tab),
        (
            "span",
            Some(UnexpandablePrimitive::Span),
            crate::AlignmentCellDelimiter::Span,
        ),
        (
            "cr",
            Some(UnexpandablePrimitive::Cr),
            crate::AlignmentCellDelimiter::Row,
        ),
    ] {
        let mut command = CommandState::default();
        let alignment = crate::AlignmentIdentity::new(1);
        let mut universe = crate::test_harness::universe_with_plain_catcodes();
        let mut capabilities = CommandHostCapabilities::default();
        let delimiter = if let Some(primitive) = primitive {
            let symbol = universe.intern(name).symbol();
            universe.set_meaning(symbol, Meaning::UnexpandablePrimitive(primitive));
            Token::Cs(symbol)
        } else {
            Token::Char {
                ch: '&',
                cat: Catcode::AlignmentTab,
            }
        };
        let v_template =
            tex_state::input::TracedTokenList::synthetic(universe.intern_token_list_ref(&[
                Token::Char {
                    ch: 'v',
                    cat: Catcode::Letter,
                },
            ]));
        command.begin_alignment(alignment);
        command
            .begin_alignment_cell(
                alignment,
                crate::AlignmentCellTemplates {
                    u_template: None,
                    v_template,
                },
            )
            .expect("cell begins");
        command
            .install_alignment_cell_template(&universe.command_context(), alignment)
            .expect("omit-style cell has no u-template input");
        let mut tokens = b"width1pt height2pt depth0pt"
            .iter()
            .map(|byte| Token::Char {
                ch: char::from(*byte),
                // A space is category 10: §407's `scan_keyword` skips a
                // leading `spacer`, so spelling the separators as
                // `other_char` would exercise a token no tokenizer produces.
                cat: match byte {
                    b if b.is_ascii_alphabetic() => Catcode::Letter,
                    b' ' => Catcode::Space,
                    _ => Catcode::Other,
                },
            })
            .collect::<Vec<_>>();
        tokens.push(delimiter);
        push(&mut command, tokens);

        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        let spec = processor
            .scan_rule_spec(UnexpandablePrimitive::VRule)
            .unwrap_or_else(|error| panic!("{name} rule scan succeeds: {error}"));
        assert_eq!(spec.depth.map(Scaled::raw), Some(0));
        let v = processor
            .get_x_token()
            .unwrap_or_else(|error| panic!("{name} v-template delivery succeeds: {error}"))
            .expect("v-template token is live");
        assert!(matches!(v.meaning(), Meaning::CharToken { ch: 'v', .. }));
        let endv = processor
            .get_x_token()
            .unwrap_or_else(|error| panic!("{name} end-template delivery succeeds: {error}"))
            .expect("retained v-template emits endv");
        assert!(matches!(endv.meaning(), Meaning::EndV));
        let finished = processor
            .command
            .finish_alignment_cell(alignment)
            .expect("only exhausted v-template completes the cell");
        assert_eq!(finished.delimiter, expected, "{name} delimiter is retained");
    }
}

/// TeX82 §774 `init_align` runs `scan_spec(align_group,false)`, so an
/// alignment takes §645's `to`/`spread` clause exactly as `\hbox` does.
/// Reaching §403's mandatory left brace without scanning the clause first
/// rejects the `t` of `to` as a missing brace.
#[test]
fn alignment_preamble_opening_scans_the_scan_spec_clause() {
    for (body, expected) in [
        (r"{#\cr", ScannedPackingSpec::Natural),
        (
            r"to 12pt{#\cr",
            ScannedPackingSpec::Exactly(Scaled::from_raw(12 * Scaled::UNITY)),
        ),
        (
            r"spread 3pt{#\cr",
            ScannedPackingSpec::Spread(Scaled::from_raw(3 * Scaled::UNITY)),
        ),
    ] {
        let mut command = CommandState::default();
        let alignment = crate::AlignmentIdentity::new(1);
        command.begin_alignment(alignment);
        let source = command
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(body.as_bytes().to_vec()),
            ))
            .expect("source registers");
        command
            .open_registered_source(source)
            .expect("source opens");
        let mut universe = crate::test_harness::universe_with_plain_catcodes();
        let cr = universe.intern("cr").symbol();
        universe.set_meaning(
            cr,
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Cr),
        );
        let mut capabilities = CommandHostCapabilities::default();
        {
            let mut processor = processor(&mut command, &mut universe, &mut capabilities);
            let packing = processor
                .scan_alignment_preamble_opening()
                .unwrap_or_else(|_| panic!("scan_spec accepts `{body}`"));
            assert_eq!(packing, expected, "`{body}` packing specification");
            processor
                .begin_alignment_preamble_scan(None)
                .unwrap_or_else(|_| panic!("`{body}` preamble scans"));
        }
        let preamble = command
            .take_completed_alignment_preamble(alignment)
            .unwrap_or_else(|_| panic!("`{body}` freezes a preamble"));
        assert_eq!(preamble.columns.len(), 1, "`{body}` column count");
    }
}

#[test]
fn alignment_preamble_discards_leading_spaces_from_each_u_template_only() {
    let mut command = CommandState::default();
    let alignment = crate::AlignmentIdentity::new(1);
    command.begin_alignment(alignment);
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let hfil = universe.intern("hfil").symbol();
    let cr = universe.intern("cr").symbol();
    universe.set_meaning(
        hfil,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::HFil),
    );
    universe.set_meaning(
        cr,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Cr),
    );
    push(
        &mut command,
        [
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Char {
                ch: ' ',
                cat: Catcode::Space,
            },
            Token::Cs(hfil),
            Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
            },
            Token::Char {
                ch: ' ',
                cat: Catcode::Space,
            },
            Token::Cs(hfil),
            Token::Char {
                ch: '&',
                cat: Catcode::AlignmentTab,
            },
            Token::Char {
                ch: ' ',
                cat: Catcode::Space,
            },
            Token::Cs(hfil),
            Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
            },
            Token::Cs(cr),
        ],
    );
    let mut capabilities = CommandHostCapabilities::default();
    {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        processor
            .scan_alignment_preamble_opening()
            .expect("scan_spec consumes the opening brace");
        processor
            .begin_alignment_preamble_scan(None)
            .expect("preamble scans");
    }

    let preamble = command
        .take_completed_alignment_preamble(alignment)
        .expect("frozen preamble is available");
    assert_eq!(preamble.columns.len(), 2);
    for column in &preamble.columns {
        let template = column
            .u_template
            .as_ref()
            .expect("u-template remains nonempty");
        assert_eq!(universe.tokens(template.token_list()), &[Token::Cs(hfil)]);
    }
    assert_eq!(
        universe.tokens(preamble.columns[0].v_template.token_list()),
        &[
            Token::Char {
                ch: ' ',
                cat: Catcode::Space,
            },
            Token::Cs(hfil),
        ]
    );
}

#[test]
fn alignment_preamble_tabskip_assignment_preserves_the_prior_boundary() {
    let mut command = CommandState::default();
    let alignment = crate::AlignmentIdentity::new(1);
    command.begin_alignment(alignment);
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let initial = universe.intern_glue(tex_state::glue::GlueSpec {
        width: tex_state::scaled::Scaled::from_raw(tex_state::scaled::Scaled::UNITY),
        ..tex_state::glue::GlueSpec::ZERO
    });
    universe.set_glue_param(tex_state::env::banks::GlueParam::TAB_SKIP, &initial);
    let tabskip = universe.intern("tabskip").symbol();
    let cr = universe.intern("cr").symbol();
    universe.set_meaning(
        tabskip,
        Meaning::GlueParam(tex_state::env::banks::GlueParam::TAB_SKIP.raw()),
    );
    universe.set_meaning(
        cr,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Cr),
    );
    push(
        &mut command,
        [
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
            },
            Token::Cs(tabskip),
            Token::Char {
                ch: '=',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: '2',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: 'p',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: 't',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: '&',
                cat: Catcode::AlignmentTab,
            },
            Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
            },
            Token::Cs(cr),
        ],
    );
    let mut capabilities = CommandHostCapabilities::default();
    {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        processor
            .scan_alignment_preamble_opening()
            .expect("scan_spec consumes the opening brace");
        processor
            .begin_alignment_preamble_scan(None)
            .expect("preamble scans");
    }

    let preamble = command
        .take_completed_alignment_preamble(alignment)
        .expect("frozen preamble is available");
    assert_eq!(preamble.tabskips.len(), 3);
    assert_eq!(
        preamble.tabskips[0],
        universe.glue(initial),
        "the glue preceding the first template was already frozen"
    );
    assert_eq!(
        preamble.tabskips[1].width.raw(),
        2 * tex_state::scaled::Scaled::UNITY
    );
}

#[test]
fn alignment_preamble_classifies_parameter_and_tab_aliases_by_resolved_command() {
    let mut command = CommandState::default();
    let alignment = crate::AlignmentIdentity::new(1);
    command.begin_alignment(alignment);
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let sharp = universe.intern("sharp").symbol();
    let tab = universe.intern("tab").symbol();
    let cr = universe.intern("cr").symbol();
    universe.set_meaning(
        sharp,
        Meaning::CharToken {
            ch: '#',
            cat: Catcode::Parameter,
        },
    );
    universe.set_meaning(
        tab,
        Meaning::CharToken {
            ch: '&',
            cat: Catcode::AlignmentTab,
        },
    );
    universe.set_meaning(
        cr,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Cr),
    );
    push(
        &mut command,
        [
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Cs(sharp),
            Token::Cs(tab),
            Token::Cs(sharp),
            Token::Cs(sharp),
            Token::Cs(cr),
        ],
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    {
        let mut processor =
            processor(&mut command, &mut universe, &mut capabilities).with_observer(&mut recorder);
        processor
            .scan_alignment_preamble_opening()
            .expect("scan_spec consumes the opening brace");
        processor
            .begin_alignment_preamble_scan(None)
            .expect("aliased command codes form the preamble");
    }

    let preamble = command
        .take_completed_alignment_preamble(alignment)
        .expect("frozen preamble is available");
    assert_eq!(preamble.columns.len(), 2);
    assert!(preamble.columns.iter().all(|column| {
        column
            .u_template
            .as_ref()
            .is_some_and(|template| universe.tokens(template.token_list()).is_empty())
            && universe.tokens(column.v_template.token_list()).is_empty()
    }));
    assert_eq!(
        recorder
            .0
            .iter()
            .filter(|observation| matches!(
                observation,
                CommandObservation::Alignment(record) if record.transition == "extra_parameter"
            ))
            .count(),
        1,
        "TeX82 §24 and 760 discard an extra resolved mac_param command"
    );
    assert!(!recorder.0.iter().any(|observation| matches!(
        observation,
        CommandObservation::Alignment(record) if record.transition == "missing_parameter"
    )));
}

#[test]
fn alignment_preamble_does_not_treat_an_ordinary_character_alias_as_a_parameter() {
    let mut command = CommandState::default();
    let alignment = crate::AlignmentIdentity::new(1);
    command.begin_alignment(alignment);
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let ordinary = universe.intern("ordinary").symbol();
    let cr = universe.intern("cr").symbol();
    universe.set_meaning(
        ordinary,
        Meaning::CharToken {
            ch: 'x',
            cat: Catcode::Other,
        },
    );
    universe.set_meaning(
        cr,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Cr),
    );
    push(
        &mut command,
        [
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Cs(ordinary),
            Token::Cs(cr),
        ],
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    {
        let mut processor =
            processor(&mut command, &mut universe, &mut capabilities).with_observer(&mut recorder);
        processor
            .scan_alignment_preamble_opening()
            .expect("scan_spec consumes the opening brace");
        processor
            .begin_alignment_preamble_scan(None)
            .expect("missing parameter recovers");
    }

    assert!(recorder.0.iter().any(|observation| matches!(
        observation,
        CommandObservation::Alignment(record) if record.transition == "missing_parameter"
    )));
}

#[test]
fn alignment_preamble_missing_parameter_before_tab_replays_the_delimiter_into_v_template() {
    assert_missing_preamble_parameter(
        [
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Char {
                ch: 'l',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: '&',
                cat: Catcode::AlignmentTab,
            },
            Token::Char {
                ch: 'r',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
            },
        ],
        2,
    );
}

#[test]
fn alignment_preamble_missing_parameter_before_cr_replays_the_delimiter_into_v_template() {
    assert_missing_preamble_parameter(
        [
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Char {
                ch: 'l',
                cat: Catcode::Letter,
            },
        ],
        1,
    );
}

fn assert_missing_preamble_parameter(
    prefix: impl IntoIterator<Item = Token>,
    expected_columns: usize,
) {
    let mut command = CommandState::default();
    let alignment = crate::AlignmentIdentity::new(1);
    command.begin_alignment(alignment);
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let cr = universe.intern("cr").symbol();
    universe.set_meaning(
        cr,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Cr),
    );
    let mut tokens = prefix.into_iter().collect::<Vec<_>>();
    tokens.push(Token::Cs(cr));
    push(&mut command, tokens);
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    {
        let mut processor =
            processor(&mut command, &mut universe, &mut capabilities).with_observer(&mut recorder);
        processor
            .scan_alignment_preamble_opening()
            .expect("scan_spec consumes the opening brace");
        processor
            .begin_alignment_preamble_scan(None)
            .expect("missing parameter recovers through the v-template");
    }
    let preamble = command
        .take_completed_alignment_preamble(alignment)
        .expect("frozen preamble is available");
    assert_eq!(preamble.columns.len(), expected_columns);
    let recovery = recorder
        .0
        .iter()
        .position(|observation| {
            matches!(
                observation,
                CommandObservation::Alignment(record) if record.transition == "missing_parameter"
            )
        })
        .expect("TeX82 missing-parameter recovery is observed");
    let backup = recorder
        .0
        .iter()
        .enumerate()
        .skip(recovery + 1)
        .find_map(|(index, observation)| {
            matches!(
            observation,
            CommandObservation::Input(record) if record.transition == crate::InputTransition::Backup
        ).then_some(index)
        })
        .expect("back_error pushes the delimiter back into command input");
    assert!(
        recovery < backup,
        "recovery is selected before back_error input backup"
    );
}

#[test]
fn filename_registered_input_recovery_and_rollback_stay_command_owned() {
    let mut command = CommandState::default();
    push(
        &mut command,
        [
            Token::Char {
                ch: ' ',
                cat: Catcode::Space,
            },
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Char {
                ch: 'i',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: 'n',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: 'c',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
        ],
    );
    let snapshot = command.snapshot();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    capabilities.register_input(
        "inc.tex",
        SourceRegistration::new(
            RegisteredSourceKind::World,
            Arc::<[u8]>::from(b"z".as_slice()),
        ),
    );
    let input = {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        processor
            .open_registered_input()
            .expect("registered input opens")
    };
    assert_eq!(input.file_name.packed(), "inc.tex");
    command
        .rollback(snapshot)
        .expect("input opening rolls back");

    push(
        &mut command,
        [Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        }],
    );
    let error = {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        processor
            .open_registered_input()
            .expect_err("unregistered input is structured recovery")
    };
    assert_eq!(
        error,
        CommandError::MissingInput {
            name: "x.tex".to_owned(),
            original_name: "x".to_owned(),
        }
    );
}

#[test]
fn start_input_retries_the_default_area_and_retires_failed_attempt() {
    let mut command = CommandState::default();
    push(&mut command, text_tokens("nested "));
    let before = command.snapshot();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    capabilities.register_input(
        "TeXinputs:nested.tex",
        SourceRegistration::new(RegisteredSourceKind::World, Arc::<[u8]>::from(&b"x"[..])),
    );
    let opened = {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        processor
            .open_registered_input()
            .expect("default-area retry")
    };
    assert_eq!(opened.file_name.packed(), "TeXinputs:nested.tex");

    command
        .rollback(before)
        .expect("successful retry rolls back");
    push(&mut command, text_tokens("missing "));
    let failed = command.snapshot();
    let error = {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        processor
            .open_registered_input()
            .expect_err("both attempts fail")
    };
    assert_eq!(
        error,
        CommandError::MissingInput {
            name: "missing.tex".into(),
            original_name: "missing".into(),
        }
    );
    command
        .rollback(failed)
        .expect("failed attempts leave no source level");
}

#[test]
fn start_input_retains_parent_relative_authored_spelling() {
    let mut command = CommandState::default();
    push(&mut command, text_tokens("../secret "));
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();

    let error = processor(&mut command, &mut universe, &mut capabilities)
        .open_registered_input()
        .expect_err("parent-relative request remains host-owned");
    assert_eq!(
        error,
        CommandError::MissingInput {
            name: "../secret.tex".into(),
            original_name: "../secret".into(),
        }
    );
}

#[test]
fn start_input_normalizes_empty_and_nonempty_first_lines() {
    for (bytes, expected) in [
        (
            &b""[..],
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Par),
        ),
        (
            &b"z"[..],
            Meaning::CharToken {
                ch: 'z',
                cat: Catcode::Letter,
            },
        ),
    ] {
        let mut command = CommandState::default();
        push(&mut command, text_tokens("case "));
        let mut universe = crate::test_harness::universe_with_plain_catcodes();
        universe.set_int_param(IntParam::END_LINE_CHAR, 13);
        let par = universe.intern("par").symbol();
        universe.set_meaning(
            par,
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Par),
        );
        let mut capabilities = CommandHostCapabilities::default();
        capabilities.register_input(
            "case.tex",
            SourceRegistration::new(RegisteredSourceKind::World, Arc::<[u8]>::from(bytes)),
        );
        let actual = {
            let mut processor = processor(&mut command, &mut universe, &mut capabilities);
            processor.open_registered_input().expect("input opens");
            processor
                .get_x_token()
                .expect("first-line delivery")
                .expect("opening line has a token")
                .meaning()
        };
        assert_eq!(actual, expected);
    }
}

#[test]
fn start_input_honors_inactive_endlinechar_on_the_opening_line() {
    let mut command = CommandState::default();
    push(&mut command, text_tokens("case "));
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    universe.set_int_param(IntParam::END_LINE_CHAR, -1);
    let mut capabilities = CommandHostCapabilities::default();
    capabilities.register_input(
        "case.tex",
        SourceRegistration::new(RegisteredSourceKind::World, Arc::<[u8]>::from(&b"z"[..])),
    );
    let (first, end) = {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        processor.open_registered_input().expect("input opens");
        (
            processor
                .get_x_token()
                .expect("character")
                .expect("character is present")
                .meaning(),
            processor.get_x_token().expect("source retirement"),
        )
    };
    assert!(matches!(first, Meaning::CharToken { ch: 'z', .. }));
    assert!(end.is_none());
}

#[test]
fn start_input_nests_and_initializes_the_job_name_only_once() {
    let mut command = CommandState::default();
    push(&mut command, text_tokens("outer "));
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    capabilities.register_input(
        "outer.tex",
        SourceRegistration::new(
            RegisteredSourceKind::World,
            Arc::<[u8]>::from(&b"inner p"[..]),
        ),
    );
    capabilities.register_input(
        "inner.tex",
        SourceRegistration::new(RegisteredSourceKind::World, Arc::<[u8]>::from(&b"c"[..])),
    );
    let (child, parent) = {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        processor.open_registered_input().expect("outer opens");
        processor.open_registered_input().expect("inner opens");
        let child = processor
            .get_x_token()
            .expect("child delivery")
            .expect("child is present")
            .meaning();
        let _child_end = processor.get_x_token().expect("child endline");
        let parent = processor
            .get_x_token()
            .expect("parent resumes")
            .expect("parent is present")
            .meaning();
        (child, parent)
    };
    assert!(matches!(child, Meaning::CharToken { ch: 'c', .. }));
    assert!(matches!(parent, Meaning::CharToken { ch: 'p', .. }));
    assert_eq!(capabilities.job_name(), "outer");
}

#[test]
fn immediate_pdf_object_dvi_result_precedes_every_operand_scan() {
    // pdftex.web §§1535, 1542, and 1621: `\immediate` performs its expanded
    // command lookahead, but the recursive `\pdfobj` case checks output mode
    // before recognizing any keyword or scanning any operand.
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    let pdfobj = universe.intern("pdfobj").symbol();
    universe.set_meaning(
        pdfobj,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfObject),
    );
    push(
        &mut command,
        [
            vec![Token::Cs(pdfobj)],
            text_tokens(" useobjnum 37 stream attr{x} file{y}"),
        ]
        .concat(),
    );
    let snapshot = command.snapshot();

    let result = {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        processor
            .scan_immediate_extension(false)
            .expect("DVI result needs no operand scan")
    };
    assert_eq!(
        result,
        ImmediateExtension::PdfExtensionInDviMode(UnexpandablePrimitive::PdfObject)
    );
    let next = {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        processor
            .get_x_token()
            .expect("operand input remains valid")
            .expect("space after pdfobj remains unconsumed")
            .meaning()
    };
    assert!(matches!(
        next,
        Meaning::CharToken {
            cat: Catcode::Space,
            ..
        }
    ));

    command
        .rollback(snapshot)
        .expect("scanner attempt rolls back");
    let request = {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        processor
            .scan_immediate_extension(true)
            .expect("PDF retry scans the preserved request")
    };
    assert!(matches!(
        request,
        ImmediateExtension::PdfObject(PdfObjectRequest::Define {
            use_object: Some(37),
            stream: true,
            stream_attr: Some(_),
            file: true,
            ..
        })
    ));
}

#[test]
fn immediate_pdf_form_dvi_result_precedes_every_operand_scan() {
    // pdftex.web §§1548 and 1623: `\immediate` performs its expanded command
    // lookahead, then the recursive `\pdfxform` case checks output mode before
    // allocating a form or scanning attr/resources/the box register.
    let mut command = CommandState::new(crate::CommandProfile::PDFTEX14029);
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    let pdfxform = universe.intern("pdfxform").symbol();
    universe.set_meaning(
        pdfxform,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfXForm),
    );
    push(
        &mut command,
        [
            vec![Token::Cs(pdfxform)],
            text_tokens(" attr{x} resources{y} 37"),
        ]
        .concat(),
    );
    let snapshot = command.snapshot();

    let result = {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        processor
            .scan_immediate_extension(false)
            .expect("DVI result needs no form operand scan")
    };
    assert_eq!(
        result,
        ImmediateExtension::PdfExtensionInDviMode(UnexpandablePrimitive::PdfXForm)
    );
    let next = {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        processor
            .get_x_token()
            .expect("operand input remains valid")
            .expect("space after pdfxform remains unconsumed")
            .meaning()
    };
    assert!(matches!(
        next,
        Meaning::CharToken {
            cat: Catcode::Space,
            ..
        }
    ));

    command
        .rollback(snapshot)
        .expect("scanner attempt rolls back");
    let request = {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        processor
            .scan_immediate_extension(true)
            .expect("PDF retry scans the preserved form request")
    };
    assert!(
        matches!(
            &request,
            ImmediateExtension::PdfForm(PdfFormRequest::Create {
                attr: Some(_),
                resources: Some(_),
                box_register: 37,
            })
        ),
        "{request:?}"
    );
}

#[test]
fn immediate_pdf_image_dvi_result_precedes_every_operand_scan() {
    // pdftex.web §§1551 and 1621: `\immediate` performs its expanded command
    // lookahead, then the recursive `\pdfximage` case checks output mode
    // before image allocation or any dimension, attr, page, box, or file
    // scan.
    let mut command = CommandState::new(crate::CommandProfile::PDFTEX14029);
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    let pdfximage = universe.intern("pdfximage").symbol();
    universe.set_meaning(
        pdfximage,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfXImage),
    );
    push(
        &mut command,
        [
            vec![Token::Cs(pdfximage)],
            text_tokens(" width 10pt height 20pt depth 3pt attr{x} page 2 mediabox {image.pdf}"),
        ]
        .concat(),
    );
    let snapshot = command.snapshot();

    let result = {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        processor
            .scan_immediate_extension(false)
            .expect("DVI result needs no image operand scan")
    };
    assert_eq!(
        result,
        ImmediateExtension::PdfExtensionInDviMode(UnexpandablePrimitive::PdfXImage)
    );
    let next = {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        processor
            .get_x_token()
            .expect("operand input remains valid")
            .expect("space after pdfximage remains unconsumed")
            .meaning()
    };
    assert!(matches!(
        next,
        Meaning::CharToken {
            cat: Catcode::Space,
            ..
        }
    ));

    command
        .rollback(snapshot)
        .expect("scanner attempt rolls back");
    let request = {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        processor
            .scan_immediate_extension(true)
            .expect("PDF retry scans the preserved image request")
    };
    let ImmediateExtension::PdfImage(request) = request else {
        panic!("expected an immediate image request, got {request:?}");
    };
    assert_eq!(request.name, "image.pdf");
    assert_eq!(request.width, Some(Scaled::from_raw(10 * Scaled::UNITY)));
    assert_eq!(request.height, Some(Scaled::from_raw(20 * Scaled::UNITY)));
    assert_eq!(request.depth, Some(Scaled::from_raw(3 * Scaled::UNITY)));
    assert_eq!(request.page, PdfImagePageSelection::Number(2));
    assert_eq!(request.color_space_object, 0);
    assert_eq!(request.page_box, PdfImagePageBox::Media);
    assert!(request.page_box_explicit);
    assert!(request.attr.is_some());
}

#[test]
fn pdf_image_scans_named_page_colorspace_and_general_text_in_source_order() {
    let mut command = CommandState::new(crate::CommandProfile::PDFTEX14029);
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    push(
        &mut command,
        text_tokens(
            "attr{/Intent /RelativeColorimetric} named{chapter.one} colorspace -7 trimbox {image.pdf}!",
        ),
    );

    let request = {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        processor
            .scan_pdf_image_request()
            .expect("scan named-page image request")
    };

    assert_eq!(
        request.page,
        PdfImagePageSelection::Named(b"chapter.one".to_vec())
    );
    assert_eq!(request.color_space_object, -7);
    assert_eq!(request.page_box, PdfImagePageBox::Trim);
    assert!(request.page_box_explicit);
    assert_eq!(request.name, "image.pdf");
    assert!(request.attr.is_some());
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);
    assert_eq!(
        processor
            .get_x_token()
            .expect("read following token")
            .expect("following token exists")
            .spelling()
            .semantic_token(),
        Token::Char {
            ch: '!',
            cat: Catcode::Other,
        }
    );
}

#[test]
fn pdf_image_filename_uses_canonical_quoted_unquoted_and_grouped_boundaries() {
    for (source, expected, following) in [
        (r#""figure name.png" !"#, "figure name.png", '!'),
        ("figure.png !", "figure.png", '!'),
        ("{figure.png}!", "figure.png", '!'),
    ] {
        let mut command = CommandState::new(crate::CommandProfile::PDFTEX14029);
        let mut universe = crate::test_harness::universe_with_plain_catcodes();
        let mut capabilities = CommandHostCapabilities::default();
        push(&mut command, text_tokens(source));

        let request = {
            let mut processor = processor(&mut command, &mut universe, &mut capabilities);
            processor
                .scan_pdf_image_request()
                .expect("pdfximage filename scans")
        };
        assert_eq!(request.name, expected, "source {source:?}");

        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        assert_eq!(
            processor
                .get_x_token()
                .expect("following token delivery succeeds")
                .expect("following token remains")
                .spelling()
                .semantic_token(),
            Token::Char {
                ch: following,
                cat: Catcode::Other,
            },
            "source {source:?}"
        );
    }
}

#[test]
fn pdf_graphics_scanners_freeze_immediate_and_shipout_literal_payloads() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    push(&mut command, text_tokens("direct{q}shipout page{Q}"));
    let (immediate, deferred) = {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        (
            processor
                .scan_pdf_graphics_request(UnexpandablePrimitive::PdfLiteral)
                .expect("immediate literal scans")
                .expect("literal request"),
            processor
                .scan_pdf_graphics_request(UnexpandablePrimitive::PdfLiteral)
                .expect("shipout literal scans")
                .expect("literal request"),
        )
    };
    assert!(matches!(
        immediate,
        PdfGraphicsRequest::Literal {
            mode: tex_state::node::PdfLiteralMode::Direct,
            deferred: false,
            ..
        }
    ));
    assert!(matches!(
        deferred,
        PdfGraphicsRequest::Literal {
            mode: tex_state::node::PdfLiteralMode::Page,
            deferred: true,
            ..
        }
    ));
}

#[test]
fn pdf_colorstack_scanner_keeps_setter_text_and_missing_action_typed() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    push(&mut command, text_tokens("2 set{g}3"));
    let (set, missing) = {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        (
            processor
                .scan_pdf_graphics_request(UnexpandablePrimitive::PdfColorStack)
                .expect("setter scans")
                .expect("color request"),
            processor
                .scan_pdf_graphics_request(UnexpandablePrimitive::PdfColorStack)
                .expect("missing action scans")
                .expect("color request"),
        )
    };
    assert!(matches!(
        set,
        PdfGraphicsRequest::ColorStack {
            id: 2,
            action: Some(PdfColorStackActionRequest::Set(_)),
        }
    ));
    assert!(matches!(
        missing,
        PdfGraphicsRequest::ColorStack {
            id: 3,
            action: None
        }
    ));
}

#[test]
fn pdf_snapping_scanners_preserve_glue_and_clamp_compensation() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    push(
        &mut command,
        text_tokens(" 3pt plus 2fil minus 1pt -7 1007"),
    );
    let (reference, snap, low, high) = {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        (
            processor
                .scan_pdf_graphics_request(UnexpandablePrimitive::PdfSnapRefPoint)
                .expect("reference scans")
                .expect("reference request"),
            processor
                .scan_pdf_graphics_request(UnexpandablePrimitive::PdfSnapY)
                .expect("glue scans")
                .expect("snap request"),
            processor
                .scan_pdf_graphics_request(UnexpandablePrimitive::PdfSnapYComp)
                .expect("low compensation scans")
                .expect("compensation request"),
            processor
                .scan_pdf_graphics_request(UnexpandablePrimitive::PdfSnapYComp)
                .expect("high compensation scans")
                .expect("compensation request"),
        )
    };
    assert_eq!(reference, PdfGraphicsRequest::SnapReferencePoint);
    assert!(matches!(
        snap,
        PdfGraphicsRequest::SnapY { glue }
            if glue.width == Scaled::from_raw(3 * 65_536)
                && glue.stretch == Scaled::from_raw(2 * 65_536)
                && glue.stretch_order == tex_state::glue::Order::Fil
                && glue.shrink == Scaled::from_raw(65_536)
    ));
    assert_eq!(low, PdfGraphicsRequest::SnapYComp { ratio: 0 });
    assert_eq!(high, PdfGraphicsRequest::SnapYComp { ratio: 1000 });
}

#[test]
fn pdf_navigation_applies_halfword_bound_only_to_dest_and_thread_ids() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    push(
        &mut command,
        text_tokens("struct 1073741824 num 1 fit goto page 1073741824{Fit}num 1073741824 fit"),
    );
    let (destination, action, too_large) = {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        let destination = processor
            .scan_pdf_navigation_request(UnexpandablePrimitive::PdfDest)
            .expect("large structure object remains a scan_int value");
        let action = processor
            .scan_pdf_navigation_request(UnexpandablePrimitive::PdfStartLink)
            .expect("large page number remains a scan_int value");
        let too_large = processor
            .scan_pdf_navigation_request(UnexpandablePrimitive::PdfDest)
            .expect_err("destination identifiers retain max_halfword bound");
        (destination, action, too_large)
    };
    assert!(matches!(
        destination,
        PdfNavigationRequest::Destination(PdfDestinationRequest {
            structure: Some(1_073_741_824),
            ..
        })
    ));
    assert!(matches!(
        action,
        PdfNavigationRequest::StartLink(PdfStartLinkRequest {
            action: tex_state::PdfActionSpec::GoTo(tex_state::PdfActionDestination {
                target: tex_state::PdfActionTarget::Page {
                    number: 1_073_741_824,
                    ..
                },
                ..
            }),
            ..
        })
    ));
    assert_eq!(
        too_large,
        CommandError::PdfNavigation("pdfTeX error (ext1): number too big")
    );
}

#[test]
fn malformed_pdf_navigation_keeps_the_nonoperand_token() {
    // pdftex.web §§1561, 1562, 1565, and 1566 diagnose a missing action or
    // identifier after keyword probes. `scan_keyword` must leave the first
    // nonoperand token available to the surrounding recovery path.
    for (primitive, expected) in [
        (
            UnexpandablePrimitive::PdfStartLink,
            "pdfTeX error (ext1): action type missing",
        ),
        (
            UnexpandablePrimitive::PdfOutline,
            "pdfTeX error (ext1): action type missing",
        ),
        (
            UnexpandablePrimitive::PdfDest,
            "pdfTeX error (ext1): identifier type missing",
        ),
        (
            UnexpandablePrimitive::PdfThread,
            "pdfTeX error (ext4): thread identifier type missing",
        ),
        (
            UnexpandablePrimitive::PdfStartThread,
            "pdfTeX error (ext4): thread identifier type missing",
        ),
    ] {
        let mut command = CommandState::default();
        let mut universe = crate::test_harness::universe_with_plain_catcodes();
        let mut capabilities = CommandHostCapabilities::default();
        push(&mut command, text_tokens("Z"));
        let (error, following) = {
            let mut processor = processor(&mut command, &mut universe, &mut capabilities);
            let error = processor
                .scan_pdf_navigation_request(primitive)
                .expect_err("malformed request is rejected");
            let following = processor
                .get_x_token()
                .expect("following token delivery succeeds")
                .expect("keyword mismatch preserves its token")
                .meaning();
            (error, following)
        };
        assert_eq!(error, CommandError::PdfNavigation(expected));
        assert!(matches!(following, Meaning::CharToken { ch: 'Z', .. }));
    }
}

#[test]
fn pdf_catalog_scanner_consumes_the_complete_open_action_suffix() {
    // pdftex.web §1571 scans the expanded catalog fragment and the complete
    // optional action before DVI-mode execution decides whether to publish it.
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    push(
        &mut command,
        text_tokens("{} openaction goto file{other.pdf} page 2 {/Fit} newwindow!"),
    );

    let request = {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        processor
            .scan_pdf_document_fragment_request(UnexpandablePrimitive::PdfCatalog)
            .expect("scan catalog open action")
    };
    assert_eq!(request.kind, tex_state::PdfDocumentFragmentKind::Catalog);
    let tex_state::PdfActionSpec::GoTo(destination) =
        request.open_action.expect("open action is present")
    else {
        panic!("expected GoTo action");
    };
    assert!(destination.file.is_some());
    assert_eq!(destination.window, tex_state::PdfActionWindow::New);
    assert!(matches!(
        destination.target,
        tex_state::PdfActionTarget::Page { number: 2, .. }
    ));

    let mut processor = processor(&mut command, &mut universe, &mut capabilities);
    assert_eq!(
        processor
            .get_x_token()
            .expect("read following token")
            .expect("following token exists")
            .spelling()
            .semantic_token(),
        Token::Char {
            ch: '!',
            cat: Catcode::Other,
        }
    );
}

#[test]
fn shift_case_rewrites_characters_and_backs_the_shifted_list_up() {
    let mut command = CommandState::default();
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(b"{ab@}".as_slice()),
        ))
        .expect("source registers");
    command
        .open_registered_source(source)
        .expect("source opens");
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    universe.set_catcode('b', Catcode::Active);
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    {
        let mut processor =
            processor(&mut command, &mut universe, &mut capabilities).with_observer(&mut recorder);
        processor.shift_case(true).expect("shift_case completes");
    }

    // TeX82 §1288 rewrites character *and* active-character tokens through
    // `\uccode` without changing their category, leaves a zero-code entry
    // alone, and hands the result to `back_list`.
    let Some(crate::input::InputLevel::Tokens(cursor)) = command.input.levels.last() else {
        panic!("shift_case pushes a token level");
    };
    let crate::input::TokenPayload::Packed(buffer) = &cursor.payload else {
        panic!("`back_list` owns one packed temporary chunk");
    };
    let shifted: Vec<_> = (0..3)
        .map(|index| {
            buffer
                .word(index)
                .expect("the shifted list retains every token")
                .semantic_token()
        })
        .collect();
    assert_eq!(
        shifted,
        vec![
            Token::Char {
                ch: 'A',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: 'B',
                cat: Catcode::Active,
            },
            Token::Char {
                ch: '@',
                cat: Catcode::Other,
            },
        ]
    );
    assert!(buffer.get(3).is_none());
    let shifted_origins = (0..3)
        .map(|index| {
            buffer
                .get(index)
                .expect("the shifted list retains every origin")
                .0
                .origin()
        })
        .collect::<Vec<_>>();
    assert!(
        shifted_origins
            .iter()
            .all(|origin| *origin != OriginId::UNKNOWN),
        "case shifting preserves source provenance",
    );
    assert!(
        shifted_origins.windows(2).all(|pair| pair[0] < pair[1]),
        "case shifting preserves token-to-origin order",
    );
    assert_eq!(
        cursor.behavior,
        TokenBehavior::BackedUp(crate::input::BackupTreatment::Ordinary)
    );
    assert_eq!(cursor.trace, ReplayTrace::BackedUp);

    // §323's `back_list` is a plain `begin_token_list`: exactly one observed
    // input push, classified as backup, and no §325 recovery record.
    let pushes: Vec<_> = recorder
        .0
        .iter()
        .filter_map(|observation| match observation {
            CommandObservation::Input(record)
                if record.reason == crate::InputReason::Backup
                    && record.level == cursor.identity().0 =>
            {
                Some(record.transition)
            }
            _ => None,
        })
        .collect();
    assert_eq!(pushes, vec![crate::InputTransition::Push]);
    assert!(
        !recorder
            .0
            .iter()
            .any(|observation| matches!(observation, CommandObservation::Recovery(_))),
        "back_list reports no inserted-token recovery",
    );
}

#[test]
fn write_stream_scan_normalizes_out_of_range_stream_numbers() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    push(&mut command, text_tokens("-1 0 15 16 999999 "));
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);
    // TeX82 §1350: `if cur_val<0 then cur_val:=17 else if cur_val>15 then
    // cur_val:=16`, so `write_stream` is always one of §1342's eighteen
    // slots and `\wlog`'s `\m@ne` is recorded as 17, not -1.
    let scanned: Vec<_> = (0..5)
        .map(|_| {
            processor
                .scan_write_stream()
                .expect("write stream number scans")
        })
        .collect();
    assert_eq!(
        scanned,
        vec![
            WriteStreamSelector::Negative,
            WriteStreamSelector::Stream(0),
            WriteStreamSelector::Stream(15),
            WriteStreamSelector::AboveRange,
            WriteStreamSelector::AboveRange,
        ]
    );
}

#[test]
fn unbalanced_write_captures_context_before_recovery_retires_its_levels() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    universe.set_int_param(tex_state::env::banks::IntParam::new(54), 10);
    let mut capabilities = CommandHostCapabilities::default();
    let empty = universe.intern_token_list(&[]);
    let endwrite = universe.intern_macro(MacroMeaning::new(MeaningFlags::OUTER, empty, empty));
    universe.register_primitive_meaning(
        "endwrite",
        Meaning::Macro {
            flags: MeaningFlags::OUTER,
            definition: endwrite.id(),
        },
    );
    push(&mut command, text_tokens("x"));
    let words = [
        traced(Token::Char {
            ch: 'a',
            cat: Catcode::Letter,
        }),
        traced(Token::Char {
            ch: '}',
            cat: Catcode::EndGroup,
        }),
        traced(Token::Char {
            ch: 'b',
            cat: Catcode::Letter,
        }),
    ];
    let tokens = universe.finish_traced_token_list(&words);
    let expanded = processor(&mut command, &mut universe, &mut capabilities)
        .expand_write_text(tokens)
        .expect("write text expands");

    assert!(expanded.unbalanced);
    let context = expanded
        .error_context
        .expect("TeX82 §1372 captures context before consuming to endwrite");
    let write = context
        .find("<write> ")
        .unwrap_or_else(|| panic!("write list remains visible: {context:?}"));
    let inserted = context[write..]
        .find("<inserted text> ")
        .map(|offset| write + offset)
        .unwrap_or_else(|| panic!("artificial stopper remains visible: {context:?}"));
    let backed_up = context[inserted..]
        .find("<to be read again> ")
        .map(|offset| inserted + offset)
        .unwrap_or_else(|| panic!("enclosing backed-up level remains visible: {context:?}"));
    assert!(write < inserted && inserted < backed_up, "{context}");
}

#[test]
fn write_heavy_expansion_has_an_exact_monotonic_work_vector() {
    const INVOCATIONS: u64 = 64;

    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    let empty = universe.intern_token_list(&[]);
    let endwrite = universe.intern_macro(MacroMeaning::new(MeaningFlags::OUTER, empty, empty));
    universe.register_primitive_meaning(
        "endwrite",
        Meaning::Macro {
            flags: MeaningFlags::OUTER,
            definition: endwrite.id(),
        },
    );
    let macro_symbol = universe.intern("writework").symbol();
    let replacement = universe.intern_token_list(&[Token::Char {
        ch: 'x',
        cat: Catcode::Letter,
    }]);
    let parameters = universe.intern_token_list(&[]);
    let definition = universe.intern_macro(MacroMeaning::new(
        MeaningFlags::EMPTY,
        parameters,
        replacement,
    ));
    universe.set_meaning(
        macro_symbol,
        Meaning::Macro {
            flags: MeaningFlags::EMPTY,
            definition: definition.id(),
        },
    );
    let words = (0..INVOCATIONS)
        .map(|_| traced(Token::Cs(macro_symbol)))
        .collect::<Vec<_>>();
    let tokens = universe.finish_traced_token_list(&words);
    let mut fuel = crate::CommandFuelLedger::new(10_000).expect("bounded test fuel");
    let expanded = processor(&mut command, &mut universe, &mut capabilities)
        .with_fuel(fuel.fuel_mut())
        .expand_write_text(tokens)
        .expect("write text expands");

    assert!(!expanded.unbalanced);
    assert_eq!(
        universe.tokens(expanded.tokens.token_ref().id()).len(),
        INVOCATIONS as usize
    );
    assert_eq!(
        fuel.work(),
        crate::CommandWorkCounters {
            fuel_charges: INVOCATIONS * 2 + 3,
            token_frame_steps: INVOCATIONS * 2 + 3,
            expanded_deliveries: 1,
            meaning_lookups: INVOCATIONS,
            scanner_tokens: INVOCATIONS * 2 + 2,
            write_expansions: INVOCATIONS,
        }
    );
}

#[test]
fn restricted_integer_consumers_observe_recovered_zero_before_commit() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    let target = universe.intern("bad-register").symbol();
    push(
        &mut command,
        [
            vec![
                Token::Cs(target),
                Token::Char {
                    ch: '=',
                    cat: Catcode::Other,
                },
            ],
            text_tokens("256"),
        ]
        .concat(),
    );
    let register = processor(&mut command, &mut universe, &mut capabilities)
        .scan_register_definition(false)
        .expect("register definition scans");
    assert_eq!(register.index, 0);

    let mut command = CommandState::default();
    let target = universe.intern("bad-character").symbol();
    push(
        &mut command,
        [
            vec![
                Token::Cs(target),
                Token::Char {
                    ch: '=',
                    cat: Catcode::Other,
                },
            ],
            text_tokens("256"),
        ]
        .concat(),
    );
    let character = processor(&mut command, &mut universe, &mut capabilities)
        .scan_character_definition(RestrictedIntegerClass::CharacterCode, false)
        .expect("character definition scans");
    assert_eq!(
        (character.value, character.scanned, character.recovered),
        (0, 256, true)
    );

    let mut command = CommandState::default();
    push(&mut command, text_tokens("16"));
    let mut recorder = Recorder::default();
    let family = CommandProcessor::new(
        &mut command,
        universe.command_context(),
        CommandHostContext::new(&mut capabilities),
    )
    .with_observer(&mut recorder)
    .scan_math_family(MathFamilySize::Text)
    .expect("math family scans");
    assert_eq!((family.family, family.recovered), (0, true));
    assert!(recorder.0.iter().any(|event| matches!(
        event,
        CommandObservation::Scanner(record) if record.kind == "integer" && record.value == ObservationValue::Integer(16)
    )));

    let mut command = CommandState::default();
    push(&mut command, text_tokens("134217728"));
    let delimiter = processor(&mut command, &mut universe, &mut capabilities)
        .scan_delimiter_number()
        .expect("delimiter number scans");
    assert_eq!((delimiter.code, delimiter.recovered), (0, true));
}

#[test]
fn register_definition_uses_the_profile_register_bound() {
    fn scan(profile: crate::CommandProfile, index: &str) -> (u16, String) {
        let mut command = CommandState::new(profile);
        let mut universe = crate::test_harness::universe_with_plain_catcodes();
        let mut capabilities = CommandHostCapabilities::default();
        let target = universe.intern("alias").symbol();
        push(
            &mut command,
            [vec![Token::Cs(target)], text_tokens(&format!("={index}"))].concat(),
        );
        let definition = processor(&mut command, &mut universe, &mut capabilities)
            .scan_register_definition(false)
            .expect("register definition scans");
        (definition.index, diagnostic_text(&universe))
    }

    // TeX82 §1224 retains its eight-bit bound, but e-TeX 2.6 etex.ch
    // [49.1224] replaces that operand scan with `scan_register_num`.
    let (tex82, tex82_diagnostic) = scan(crate::CommandProfile::TEX82, "2002");
    assert_eq!(tex82, 0);
    assert!(tex82_diagnostic.contains("Bad register code (2002)"));

    for profile in [
        crate::CommandProfile::ETEX26,
        crate::CommandProfile::PDFTEX14029,
    ] {
        let (extended, diagnostic) = scan(profile, "2002");
        assert_eq!(extended, 2002);
        assert!(diagnostic.is_empty(), "{diagnostic}");
    }
}

/// TeX82 §435's `scan_four_bit_int` is §§1272--1275's `in_stream` selector
/// scan, and only that: `\\openin`/`\\closein` see `cur_val=0` after an
/// invalid selector, while `int_error` retains the original value and the
/// ordinary integer observation precedes the request.
///
/// §1225's `read_to_cs` deliberately does *not* share it -- it scans a plain
/// `scan_int` and lets §482's `if (n<0)or(n>15) then m:=16` send an
/// out-of-range stream to the terminal with no diagnostic at all, which
/// `read_stream_selector_is_unrestricted` covers.
#[test]
fn restricted_input_stream_consumers_recover_out_of_range_to_zero() {
    use tex_state::meaning::UnexpandablePrimitive as P;

    for primitive in [P::OpenIn, P::CloseIn] {
        for (source, expected, recovered) in [
            ("-1", 0, true),
            ("15", 15, false),
            ("16", 0, true),
            ("1000000", 0, true),
        ] {
            let mut command = CommandState::default();
            let mut universe = crate::test_harness::universe_with_plain_catcodes();
            // §484 reads `\read`'s replacement from the terminal only when
            // `interaction>nonstop_mode`, so this one case needs a mode above
            // the harness default.
            universe.set_interaction_mode(tex_state::InteractionMode::Scroll);
            let mut capabilities = CommandHostCapabilities::default();
            universe.intern("readtarget");
            let suffix = match primitive {
                P::OpenIn => "=fixture ",
                P::CloseIn => "",
                _ => unreachable!("only the in_stream primitives are scanned here"),
            };
            let input = format!("{source} {suffix}");
            let input_source = command
                .register_source(SourceRegistration::new(
                    RegisteredSourceKind::Generated,
                    Arc::<[u8]>::from(input.into_bytes()),
                ))
                .expect("operand source registers");
            command
                .open_registered_source(input_source)
                .expect("operand source opens");
            let mut recorder = Recorder::default();
            let request = CommandProcessor::new(
                &mut command,
                universe.command_context(),
                CommandHostContext::new(&mut capabilities),
            )
            .with_observer(&mut recorder)
            .scan_input_stream_request(primitive, false)
            .unwrap_or_else(|error| panic!("{primitive:?} selector {source} scans: {error}"));

            let (stream, scanned, did_recover) = match request {
                InputStreamRequest::Open {
                    stream,
                    scanned,
                    recovered,
                    ..
                }
                | InputStreamRequest::Close {
                    stream,
                    scanned,
                    recovered,
                } => (stream, scanned, recovered),
                InputStreamRequest::Read { .. } => {
                    unreachable!("only the in_stream primitives are scanned here")
                }
            };
            assert_eq!(
                (stream, scanned, did_recover),
                (
                    expected,
                    source.parse::<i32>().expect("decimal case"),
                    recovered
                ),
                "{primitive:?} selector {source}",
            );

            let integer_events: Vec<_> = recorder
                .0
                .iter()
                .filter_map(|event| match event {
                    CommandObservation::Scanner(record) if record.kind == "integer" => {
                        match record.value {
                            ObservationValue::Integer(value) => Some(value),
                            _ => None,
                        }
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(
                integer_events.first().copied(),
                Some(i64::from(source.parse::<i32>().expect("decimal case"))),
                "{primitive:?} observes the raw integer before request commit",
            );
        }
    }
}

/// TeX82 §1225's `read_to_cs` scans a plain `scan_int`, so an out-of-range
/// `\\read` stream reaches the request unchanged and §482's
/// `if (n<0)or(n>15) then m:=16` reads it from the terminal. Routing it
/// through §435 instead would report a `Bad number` the reference engine never
/// prints.
#[test]
fn read_stream_selector_is_unrestricted() {
    use tex_state::meaning::UnexpandablePrimitive as P;

    for primitive in [P::Read, P::ReadLine] {
        for source in ["-1", "15", "16", "1000000"] {
            let mut command = CommandState::default();
            let mut universe = crate::test_harness::universe_with_plain_catcodes();
            // §484 reads `\read`'s replacement from the terminal only when
            // `interaction>nonstop_mode`, so this one case needs a mode above
            // the harness default.
            universe.set_interaction_mode(tex_state::InteractionMode::Scroll);
            let mut capabilities = CommandHostCapabilities::default();
            universe.intern("readtarget");
            if primitive == P::ReadLine {
                universe.set_int_param(tex_state::env::banks::IntParam::END_LINE_CHAR, -1);
            }
            universe
                .world_mut()
                .push_memory_terminal_line(if primitive == P::ReadLine { "" } else { "body" })
                .expect("terminal line queues");
            let input = format!("{source} to \\readtarget ");
            let input_source = command
                .register_source(SourceRegistration::new(
                    RegisteredSourceKind::Generated,
                    Arc::<[u8]>::from(input.into_bytes()),
                ))
                .expect("operand source registers");
            command
                .open_registered_source(input_source)
                .expect("operand source opens");
            let request = CommandProcessor::new(
                &mut command,
                universe.command_context(),
                CommandHostContext::new(&mut capabilities),
            )
            .scan_input_stream_request(primitive, false)
            .unwrap_or_else(|error| panic!("{primitive:?} selector {source} scans: {error}"));

            let InputStreamRequest::Read { stream, .. } = request else {
                unreachable!("\\read scans a Read request")
            };
            assert_eq!(
                stream,
                source.parse::<i32>().expect("decimal case"),
                "{primitive:?} selector {source} reaches §482 unrecovered",
            );
        }
    }
}

#[test]
fn delimiter_direct_numeric_min_max_overflow_and_radical_policy_matrix() {
    use tex_state::meaning::UnexpandablePrimitive as P;

    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    universe.set_delcode('(', 0x0123_4567);
    let mut command = CommandState::default();
    let mut capabilities = CommandHostCapabilities::default();
    push(
        &mut command,
        vec![Token::Char {
            ch: '(',
            cat: Catcode::Other,
        }],
    );
    let delimiter = processor(&mut command, &mut universe, &mut capabilities)
        .scan_delimiter(false)
        .expect("direct delimiter scans");
    assert_eq!((delimiter.code, delimiter.recovered), (0x0123_4567, false));

    let delimiter_primitive = universe.intern("delimiter").symbol();
    universe.set_meaning(
        delimiter_primitive,
        Meaning::UnexpandablePrimitive(P::Delimiter),
    );
    for (source, expected, recovered) in [
        ("0", 0_u32, false),
        ("134217727", 134_217_727, false),
        ("134217728", 0, true),
    ] {
        let mut command = CommandState::default();
        push(
            &mut command,
            [vec![Token::Cs(delimiter_primitive)], text_tokens(source)].concat(),
        );
        let delimiter = processor(&mut command, &mut universe, &mut capabilities)
            .scan_delimiter(false)
            .expect("numeric delimiter scans");
        assert_eq!((delimiter.code, delimiter.recovered), (expected, recovered));
    }

    let mut command = CommandState::default();
    push(&mut command, text_tokens("-1x"));
    let (radical, following) = {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        let radical = processor
            .scan_delimiter(true)
            .expect("radical delimiter recovers");
        let following = processor
            .get_x_token()
            .expect("following token delivers")
            .expect("following token remains")
            .meaning();
        (radical, following)
    };
    assert_eq!((radical.code, radical.recovered), (0, true));
    assert!(matches!(following, Meaning::CharToken { ch: 'x', .. }));

    let invalid = Token::Char {
        ch: '{',
        cat: Catcode::BeginGroup,
    };
    let mut command = CommandState::default();
    push(&mut command, vec![invalid]);
    let (delimiter, following) = {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        let delimiter = processor
            .scan_delimiter(false)
            .expect("invalid delimiter recovers");
        let following = processor
            .get_x_token()
            .expect("invalid token delivers")
            .expect("invalid token remains")
            .meaning();
        (delimiter, following)
    };
    assert_eq!((delimiter.code, delimiter.recovered), (0, true));
    assert!(matches!(
        following,
        Meaning::CharToken {
            cat: Catcode::BeginGroup,
            ..
        }
    ));
}
