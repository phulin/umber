use std::sync::Arc;

use tex_state::Universe;
use tex_state::macro_store::MacroMeaning;
use tex_state::meaning::{ExpandablePrimitive, Meaning, MeaningFlags, UnexpandablePrimitive};
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use super::*;
use crate::input::{
    ReplayTrace, RetirementBehavior, SharedTokenBuffer, TokenBehavior, TokenPayload,
};
use crate::observation::RecoveryKind;
use crate::{
    CommandHostCapabilities, CommandHostContext, CommandObservation, CommandObserver,
    CommandReplayDelivery, CommandRuntime, CommandState, RegisteredSourceKind, SourceRegistration,
};

#[derive(Default)]
struct Recorder(Vec<CommandObservation>);

impl CommandObserver for Recorder {
    fn committed(&mut self, observation: CommandObservation) {
        self.0.push(observation);
    }
}

fn traced(token: Token) -> TracedTokenWord {
    TracedTokenWord::pack(token, OriginId::UNKNOWN)
}

fn push(command: &mut CommandState, tokens: impl IntoIterator<Item = Token>) {
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(
            tokens.into_iter().map(traced).collect::<Vec<_>>(),
        )),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
}

fn text_tokens(text: &str) -> Vec<Token> {
    text.chars()
        .map(|ch| Token::Char {
            ch,
            cat: match ch {
                '{' => Catcode::BeginGroup,
                '}' => Catcode::EndGroup,
                ' ' => Catcode::Space,
                'a'..='z' | 'A'..='Z' => Catcode::Letter,
                _ => Catcode::Other,
            },
        })
        .collect()
}

#[test]
fn math_scalar_requests_are_completed_before_replay() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    push(
        &mut command,
        [
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
        ],
    );
    let (character, fraction) = {
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
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
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    let target = universe.intern("definedchar").symbol();
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
                ch: '6',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: '5',
                cat: Catcode::Other,
            },
        ],
    );

    let definition = processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
        .scan_character_definition(RestrictedIntegerClass::CharacterCode, false)
        .expect("character definition scans");

    assert_eq!(definition.target, target);
    assert_eq!(definition.value, 65);
    assert_eq!(definition.scanned, 65);
    assert!(!definition.recovered);
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
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new_with_plain_catcodes();
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

        let definition = processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
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
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
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

    let definition = processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
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
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    let target = universe.intern("tenrm").symbol();
    let mut tokens = vec![Token::Cs(target)];
    tokens.extend(text_tokens("=cmr10 "));
    push(&mut command, tokens);

    let mut recorder = Recorder::default();
    let request = {
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
            .with_observer(&mut recorder);
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
                    if record.target == "meaning"
                        && record.value == "set_font"
                        && record.key.as_deref() == Some("tenrm")
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
fn font_size_recovery_carries_the_backed_up_error_context() {
    // TeX82 §§82, 1258: `scan_int` leaves its lookahead under §325
    // `back_input`; the deferred stomach-side `int_error` must display that
    // exact command-owned stack state.
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    let target = universe.intern("oversized").symbol();
    let mut tokens = vec![Token::Cs(target)];
    tokens.extend(text_tokens("=cmr10 scaled 32769="));
    push(&mut command, tokens);

    let request = processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
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
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    let font = universe.intern("font").symbol();
    push(&mut command, [Token::Cs(font)]);

    processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
        .scan_show()
        .expect("show operand scans");

    assert_eq!(
        command.output_open_context(&universe.command_context()),
        "\n<recently read> \\font \n                      "
    );
}

#[test]
fn math_field_brace_opens_group_without_absorbing_its_body() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
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
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
        processor.scan_math_field_episode().expect("field scans")
    };
    // TeX82 §1153 consumes only the mandatory brace; the body stays live
    // input for `push_math`'s `math_group`, so no replay level is opened and
    // the very next delivery is the body's own first token.
    assert_eq!(field.body, MathFieldBody::OpenGroup);
    let mut delivered = Vec::new();
    for _ in 0..4 {
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
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
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
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
    let episode = command.push_discretionary_episode(part);

    let first = {
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
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
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
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
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
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
fn math_choice_group_consumes_only_its_opening_brace() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
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
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
        processor
            .scan_math_choice_group()
            .expect("branch brace scans")
    };
    assert!(!recovered);
    let body = {
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
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
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    push(
        &mut command,
        [Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        }],
    );
    let recovered = {
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
        processor
            .scan_math_choice_group()
            .expect("recovery completes")
    };
    // TeX82 §403 backs the rejected command up and proceeds as though a `{`
    // had been read, so the group still opens and `x` becomes the first
    // token of the branch body.
    assert!(recovered);
    let replayed = {
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
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
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
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
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
            .with_observer(&mut recorder);
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
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
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
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new_with_plain_catcodes();
        let mut capabilities = CommandHostCapabilities::default();
        let symbol = universe.intern("p").symbol();
        universe.set_meaning(symbol, Meaning::UnexpandablePrimitive(primitive));
        let mut tokens = vec![Token::Cs(symbol)];
        tokens.extend(text_tokens(text));
        push(&mut command, tokens);
        let field = {
            let mut processor =
                processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
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
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new_with_plain_catcodes();
        let mut capabilities = CommandHostCapabilities::default();
        let symbol = universe.intern("field").symbol();
        universe.set_meaning(symbol, Meaning::MathCharGiven(code));
        push(&mut command, [Token::Cs(symbol)]);
        let mut recorder = Recorder::default();

        let field = {
            let mut processor =
                processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
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
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    let symbol = universe.intern("hbox").symbol();
    universe.set_meaning(
        symbol,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::HBox),
    );
    push(&mut command, [Token::Cs(symbol)]);
    let field = {
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
        processor.scan_math_field_episode().expect("field scans")
    };

    assert_eq!(field.body, MathFieldBody::OpenGroup);
    assert_eq!(
        command.alignment.align_state,
        crate::processor::TOP_LEVEL_ALIGN_STATE + 1
    );
    let next = {
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
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
    let mut plain_runtime = CommandRuntime::default();
    let mut observed_runtime = CommandRuntime::default();
    let mut plain_universe = Universe::new_with_plain_catcodes();
    let mut observed_universe = Universe::new_with_plain_catcodes();
    let mut plain_capabilities = CommandHostCapabilities::default();
    let mut observed_capabilities = CommandHostCapabilities::default();
    let plain_field = {
        let mut processor = processor(
            &mut plain,
            &mut plain_runtime,
            &mut plain_universe,
            &mut plain_capabilities,
        );
        processor
            .scan_math_field_episode()
            .expect("plain field scans")
    };
    let mut recorder = Recorder::default();
    let observed_field = {
        let mut processor = processor(
            &mut observed,
            &mut observed_runtime,
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
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
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
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
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
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    universe.set_delcode('(', 0x02_8300);
    universe.set_delcode(')', 0x02_9301);
    push(&mut command, text_tokens("()3pt 16"));
    let (fraction, family) = {
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
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
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
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
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
            .with_observer(&mut recorder);
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
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    push(&mut command, text_tokens("  x"));
    let (boundary, next) = {
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
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

fn processor<'a>(
    command: &'a mut CommandState,
    runtime: &'a mut CommandRuntime,
    universe: &'a mut Universe,
    capabilities: &'a mut CommandHostCapabilities,
) -> CommandProcessor<'a> {
    CommandProcessor::new(
        command,
        runtime,
        universe.command_context(),
        CommandHostContext::new(capabilities),
    )
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
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let target = universe.intern("defined").symbol();
    let mut capabilities = CommandHostCapabilities::default();
    let snapshot = command.snapshot();
    let balanced = {
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
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
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
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
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
        processor
            .scan_macro_definition(false)
            .expect("definition scans")
    };
    assert_eq!(definition.target, target);
    assert_eq!(
        universe.tokens(definition.parameter_text.token_list()),
        &[Token::Param(1)]
    );
    assert_eq!(
        universe.tokens(definition.replacement_text.token_list()),
        &[Token::Param(1)]
    );
}

#[test]
fn expanded_macro_definition_splices_the_spacefactor_from_the_host() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
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

    let definition = processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
        .with_observer(&mut recorder)
        .scan_macro_definition(true)
        .expect("expanded definition scans the current space factor");

    assert_eq!(definition.target, target);
    assert_eq!(
        universe.tokens(definition.replacement_text.token_list()),
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
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();

    processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
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
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();

    processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
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
    let next = processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
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
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
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
            definition,
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
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
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
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    let spec = processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
        .scan_rule_spec(UnexpandablePrimitive::VRule)
        .expect("rule spec scans");

    assert_eq!(spec.width.map(Scaled::raw), Some(Scaled::UNITY));
    assert_eq!(spec.height.map(Scaled::raw), Some(2 * Scaled::UNITY));
    assert_eq!(spec.depth.map(Scaled::raw), Some(0));
    let terminator = processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
        .get_x_token()
        .expect("terminator delivers")
        .expect("terminator exists");
    assert!(matches!(
        terminator.meaning(),
        Meaning::CharToken { ch: '!', .. }
    ));
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
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    let accent = processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
        .scan_accent()
        .expect("accent operands scan");
    assert_eq!(accent.accent, 18);
    let base = processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
        .scan_accent_base()
        .expect("base lookahead scans");
    assert!(matches!(
        base,
        ScannedAccentBase::Character {
            character: b'A',
            ..
        }
    ));

    let punctuation = processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
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
    let mut universe = Universe::new_with_plain_catcodes();
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
    let mut runtime = CommandRuntime::default();
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    let base = processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
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

/// The other half of §1124: a command that is neither a base character nor a
/// prefixed command takes tex.web's `else back_input`, and that replay stays
/// inside the delivery episode that fetched it.
#[test]
fn accent_base_lookahead_replays_a_command_that_is_neither_base_nor_assignment() {
    let mut command = CommandState::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let target = universe.intern("hbox").symbol();
    universe.set_meaning(
        target,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::HBox),
    );
    push(&mut command, [Token::Cs(target)]);
    let mut runtime = CommandRuntime::default();
    let mut capabilities = CommandHostCapabilities::default();
    let base = processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
        .scan_accent_base()
        .expect("base lookahead scans");
    assert!(matches!(base, ScannedAccentBase::Missing));

    let replayed = processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
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
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new_with_plain_catcodes();
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
            tex_state::input::TracedTokenList::synthetic(universe.intern_token_list(&[
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
            .install_alignment_cell_template(alignment)
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

        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
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
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new_with_plain_catcodes();
        let cr = universe.intern("cr").symbol();
        universe.set_meaning(
            cr,
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Cr),
        );
        let mut capabilities = CommandHostCapabilities::default();
        {
            let mut processor =
                processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
            let packing = processor
                .scan_alignment_preamble_opening()
                .unwrap_or_else(|_| panic!("scan_spec accepts `{body}`"));
            assert_eq!(packing, expected, "`{body}` packing specification");
            processor
                .begin_alignment_preamble_scan()
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
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
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
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
        processor
            .scan_alignment_preamble_opening()
            .expect("scan_spec consumes the opening brace");
        processor
            .begin_alignment_preamble_scan()
            .expect("preamble scans");
    }

    let preamble = command
        .take_completed_alignment_preamble(alignment)
        .expect("frozen preamble is available");
    assert_eq!(preamble.columns.len(), 2);
    for column in &preamble.columns {
        let template = column.u_template.expect("u-template remains nonempty");
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
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let initial = universe.intern_glue(tex_state::glue::GlueSpec {
        width: tex_state::scaled::Scaled::from_raw(tex_state::scaled::Scaled::UNITY),
        ..tex_state::glue::GlueSpec::ZERO
    });
    universe.set_glue_param(tex_state::env::banks::GlueParam::TAB_SKIP, initial);
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
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
        processor
            .scan_alignment_preamble_opening()
            .expect("scan_spec consumes the opening brace");
        processor
            .begin_alignment_preamble_scan()
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
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
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
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
            .with_observer(&mut recorder);
        processor
            .scan_alignment_preamble_opening()
            .expect("scan_spec consumes the opening brace");
        processor
            .begin_alignment_preamble_scan()
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
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    capabilities.register_input(
        "inc.tex",
        SourceRegistration::new(
            RegisteredSourceKind::World,
            Arc::<[u8]>::from(b"z".as_slice()),
        ),
    );
    let input = {
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
        processor
            .open_registered_input()
            .expect("registered input opens")
    };
    assert_eq!(input.file_name.packed(), "inc.tex");
    assert_eq!(input.file_name.termination, FileNameTermination::Group);
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
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
        processor
            .open_registered_input()
            .expect_err("unregistered input is structured recovery")
    };
    assert_eq!(error, CommandError::MissingInput("x.tex".to_owned()));
}

#[test]
fn start_input_retries_the_default_area_and_retires_failed_attempt() {
    let mut command = CommandState::default();
    push(&mut command, text_tokens("nested "));
    let before = command.snapshot();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    capabilities.register_input(
        "TeXinputs:nested.tex",
        SourceRegistration::new(RegisteredSourceKind::World, Arc::<[u8]>::from(&b"x"[..])),
    );
    let opened = {
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
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
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
        processor
            .open_registered_input()
            .expect_err("both attempts fail")
    };
    assert_eq!(error, CommandError::MissingInput("missing.tex".into()));
    command
        .rollback(failed)
        .expect("failed attempts leave no source level");
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
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new_with_plain_catcodes();
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
            let mut processor =
                processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
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
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    universe.set_int_param(IntParam::END_LINE_CHAR, -1);
    let mut capabilities = CommandHostCapabilities::default();
    capabilities.register_input(
        "case.tex",
        SourceRegistration::new(RegisteredSourceKind::World, Arc::<[u8]>::from(&b"z"[..])),
    );
    let (first, end) = {
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
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
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
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
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
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
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
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
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
        processor
            .scan_immediate_extension(false)
            .expect("DVI result needs no operand scan")
    };
    assert_eq!(
        result,
        ImmediateExtension::PdfExtensionInDviMode(UnexpandablePrimitive::PdfObject)
    );
    let next = {
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
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
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
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
    let mut command = CommandState::new(crate::CommandProfile::PDFTEX14027);
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
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
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
        processor
            .scan_immediate_extension(false)
            .expect("DVI result needs no form operand scan")
    };
    assert_eq!(
        result,
        ImmediateExtension::PdfExtensionInDviMode(UnexpandablePrimitive::PdfXForm)
    );
    let next = {
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
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
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
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
    let mut command = CommandState::new(crate::CommandProfile::PDFTEX14027);
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
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
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
        processor
            .scan_immediate_extension(false)
            .expect("DVI result needs no image operand scan")
    };
    assert_eq!(
        result,
        ImmediateExtension::PdfExtensionInDviMode(UnexpandablePrimitive::PdfXImage)
    );
    let next = {
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
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
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
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
    let mut command = CommandState::new(crate::CommandProfile::PDFTEX14027);
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    push(
        &mut command,
        text_tokens(
            "attr{/Intent /RelativeColorimetric} named{chapter.one} colorspace -7 trimbox {image.pdf}!",
        ),
    );

    let request = {
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
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
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
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
fn pdf_graphics_scanners_freeze_immediate_and_shipout_literal_payloads() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    push(&mut command, text_tokens("direct{q}shipout page{Q}"));
    let (immediate, deferred) = {
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
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
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    push(&mut command, text_tokens("2 set{g}3"));
    let (set, missing) = {
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
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
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    push(
        &mut command,
        text_tokens(" 3pt plus 2fil minus 1pt -7 1007"),
    );
    let (reference, snap, low, high) = {
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
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
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    push(
        &mut command,
        text_tokens("struct 1073741824 num 1 fit goto page 1073741824{Fit}num 1073741824 fit"),
    );
    let (destination, action, too_large) = {
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
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
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    universe.set_catcode('b', Catcode::Active);
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    {
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
            .with_observer(&mut recorder);
        processor.shift_case(true).expect("shift_case completes");
    }

    // TeX82 §1288 rewrites character *and* active-character tokens through
    // `\uccode` without changing their category, leaves a zero-code entry
    // alone, and hands the result to `back_list`.
    let Some(crate::input::InputLevel::Tokens(cursor)) = command.input.levels.last() else {
        panic!("shift_case pushes a token level");
    };
    let crate::input::TokenPayload::Transient(buffer) = &cursor.payload else {
        panic!("`back_list` owns a temporary list, not immutable storage");
    };
    let shifted: Vec<_> = (0..3)
        .map(|index| {
            buffer
                .get(index)
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
                    && record.level == cursor.identity.0 =>
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
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    push(&mut command, text_tokens("-1 0 15 16 999999 "));
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
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
fn restricted_integer_consumers_observe_recovered_zero_before_commit() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new_with_plain_catcodes();
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
    let register = processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
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
    let character = processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
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
        &mut runtime,
        universe.command_context(),
        CommandHostContext::new(&mut capabilities),
    )
    .with_observer(&mut recorder)
    .scan_math_family(MathFamilySize::Text)
    .expect("math family scans");
    assert_eq!((family.family, family.recovered), (0, true));
    assert!(recorder.0.iter().any(|event| matches!(
        event,
        CommandObservation::Scanner(record) if record.kind == "integer" && record.value == "16"
    )));

    let mut command = CommandState::default();
    push(&mut command, text_tokens("134217728"));
    let delimiter = processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
        .scan_delimiter_number()
        .expect("delimiter number scans");
    assert_eq!((delimiter.code, delimiter.recovered), (0, true));
}

/// TeX82 §435 supplies the one selector scan used by §1225's
/// `read_to_cs` and §§1272-1275's `in_stream` command. Every consumer sees
/// `cur_val=0` after an invalid selector, while `int_error` retains the
/// original value and the ordinary integer observation precedes the request.
#[test]
fn restricted_input_stream_consumers_recover_out_of_range_to_zero() {
    use tex_state::meaning::UnexpandablePrimitive as P;

    for primitive in [P::OpenIn, P::CloseIn, P::Read, P::ReadLine] {
        for (source, expected, recovered) in [
            ("-1", 0, true),
            ("15", 15, false),
            ("16", 0, true),
            ("1000000", 0, true),
        ] {
            let mut command = CommandState::default();
            let mut runtime = CommandRuntime::default();
            let mut universe = Universe::new_with_plain_catcodes();
            let mut capabilities = CommandHostCapabilities::default();
            universe.intern("readtarget");
            let suffix = match primitive {
                P::OpenIn => "=fixture ".to_owned(),
                P::CloseIn => String::new(),
                P::Read | P::ReadLine => {
                    if primitive == P::ReadLine {
                        universe.set_int_param(tex_state::env::banks::IntParam::END_LINE_CHAR, -1);
                    }
                    universe
                        .world_mut()
                        .push_memory_terminal_line(if primitive == P::ReadLine {
                            ""
                        } else {
                            "body"
                        })
                        .expect("terminal line queues");
                    " to \\readtarget ".to_owned()
                }
                _ => unreachable!(),
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
                &mut runtime,
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
                }
                | InputStreamRequest::Read {
                    stream,
                    scanned,
                    recovered,
                    ..
                } => (stream, scanned, recovered),
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
                        Some(record.value.as_str())
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(
                integer_events.first().copied(),
                Some(source),
                "{primitive:?} observes the raw integer before request commit",
            );
        }
    }
}

#[test]
fn delimiter_direct_numeric_min_max_overflow_and_radical_policy_matrix() {
    use tex_state::meaning::UnexpandablePrimitive as P;

    let mut universe = Universe::new_with_plain_catcodes();
    universe.set_delcode('(', 0x0123_4567);
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut capabilities = CommandHostCapabilities::default();
    push(
        &mut command,
        vec![Token::Char {
            ch: '(',
            cat: Catcode::Other,
        }],
    );
    let delimiter = processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
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
        let delimiter = processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
            .scan_delimiter(false)
            .expect("numeric delimiter scans");
        assert_eq!((delimiter.code, delimiter.recovered), (expected, recovered));
    }

    let mut command = CommandState::default();
    push(&mut command, text_tokens("-1x"));
    let (radical, following) = {
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
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
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
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
