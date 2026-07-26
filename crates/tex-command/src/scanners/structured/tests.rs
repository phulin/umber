use std::sync::Arc;

use tex_state::Universe;
use tex_state::macro_store::MacroMeaning;
use tex_state::meaning::{ExpandablePrimitive, Meaning, MeaningFlags, UnexpandablePrimitive};
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use super::*;
use crate::input::{
    ReplayTrace, RetirementBehavior, SharedTokenBuffer, TokenBehavior, TokenPayload,
};
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
    let mut universe = Universe::new();
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
    let mut universe = Universe::new();
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
        .scan_character_definition(false)
        .expect("character definition scans");

    assert_eq!(definition.target, target);
    assert_eq!(definition.value, 65);
}

#[test]
fn register_definition_scanner_owns_target_scope_equals_and_bounded_index() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
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
fn completed_math_field_replays_nested_group_without_exposing_tokens() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
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
    assert_eq!(field.recovery, MathEpisodeRecovery::None);
    let episode = command.push_math_field_episode(field);
    let nested = {
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
        processor
            .scan_math_group_episode()
            .expect("nested group scans")
    };
    assert_eq!(nested.recovery, MathEpisodeRecovery::None);
    let nested_episode = command.push_math_group_episode(nested);
    let token = {
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
        processor
            .get_x_token()
            .expect("delivery succeeds")
            .expect("x arrives")
    };
    assert_eq!(
        token.spelling().semantic_token(),
        Token::Char {
            ch: 'x',
            cat: Catcode::Letter
        }
    );
    assert!(command.replay_episode_is_active(episode));
    assert!(command.replay_episode_is_active(nested_episode));
}

#[test]
fn replay_completion_precedes_parent_delivery() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
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
            Token::Char {
                ch: 'z',
                cat: Catcode::Letter,
            },
        ],
    );
    let field = {
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
        processor.scan_math_field_episode().expect("field scans")
    };
    let episode = command.push_math_field_episode(field);

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
fn math_choice_groups_are_completed_independently() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
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
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Char {
                ch: 'b',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Char {
                ch: 'c',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Char {
                ch: 'd',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
        ],
    );
    let choices = {
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
        processor.scan_math_choice_episodes().expect("choice scans")
    };
    for group in [
        choices.display,
        choices.text,
        choices.script,
        choices.scriptscript,
    ] {
        assert_eq!(group.recovery, MathEpisodeRecovery::None);
        command.push_math_group_episode(group);
    }
    let replayed = {
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
        processor
            .get_x_token()
            .expect("replay remains command owned")
            .expect("last branch replays first")
    };
    assert_eq!(
        replayed.spelling().semantic_token(),
        Token::Char {
            ch: 'd',
            cat: Catcode::Letter
        }
    );
}

#[test]
fn missing_math_group_brace_recovers_without_consuming_rejected_command() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    let mut capabilities = CommandHostCapabilities::default();
    push(
        &mut command,
        [Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        }],
    );
    let group = {
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
        processor
            .scan_math_group_episode()
            .expect("recovery completes")
    };
    assert_eq!(group.recovery, MathEpisodeRecovery::MissingOpeningBrace);
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
    let mut plain_universe = Universe::new();
    let mut observed_universe = Universe::new();
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
    assert_eq!(plain_field.recovery, observed_field.recovery);
    assert_eq!(plain_field.provenance, observed_field.provenance);
    assert_eq!(plain.snapshot(), observed.snapshot());
    assert!(!recorder.0.is_empty());
}

#[test]
fn math_delimiter_and_mu_requests_recover_and_consume_units() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
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
        let delimiter = processor.scan_math_delimiter().expect("delimiter scans");
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
fn math_fraction_delimiters_and_family_recovery_are_command_owned() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
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
                ch: '7',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: ' ',
                cat: Catcode::Space,
            },
            Token::Char {
                ch: '3',
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
                ch: ' ',
                cat: Catcode::Space,
            },
            Token::Char {
                ch: '1',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: '6',
                cat: Catcode::Other,
            },
        ],
    );
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
    assert_eq!(fraction.left_delimiter.expect("left").code, 4096);
    assert_eq!(fraction.right_delimiter.expect("right").code, 4097);
    assert_eq!(
        fraction.thickness,
        Some(tex_state::scaled::Scaled::from_raw(196_608))
    );
    assert_eq!(family.family, 0);
    assert!(family.recovered);
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
    let mut universe = Universe::new();
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
    let mut universe = Universe::new();
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
    let mut universe = Universe::new();
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
fn expanded_balanced_text_uses_canonical_macro_argument_matching() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
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
    let mut universe = Universe::new();
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
fn accent_scanner_returns_completed_operands_and_replays_noncharacter_base() {
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
    let mut universe = Universe::new();
    let mut capabilities = CommandHostCapabilities::default();
    let accent = processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
        .scan_accent()
        .expect("accent operands scan");
    assert_eq!(accent.accent, 18);
    assert_eq!(accent.base.expect("base character").character, b'A');

    let punctuation = processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
        .get_x_token()
        .expect("punctuation delivers")
        .expect("punctuation exists");
    assert!(matches!(
        punctuation.meaning(),
        Meaning::CharToken { ch: '!', .. }
    ));
}

#[test]
fn discretionary_scanner_freezes_all_three_groups_with_provenance() {
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
    let mut universe = Universe::new();
    let mut capabilities = CommandHostCapabilities::default();
    let discretionary = processor(&mut command, &mut runtime, &mut universe, &mut capabilities)
        .scan_discretionary()
        .expect("three groups scan");
    for (group, character) in [
        (discretionary.pre_break, 'a'),
        (discretionary.post_break, 'b'),
        (discretionary.replacement, 'c'),
    ] {
        assert_ne!(group.provenance.primary, OriginId::UNKNOWN);
        assert_eq!(
            universe.tokens(group.tokens.token_list()),
            &[Token::Char {
                ch: character,
                cat: Catcode::Letter,
            }]
        );
    }
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
        let mut universe = Universe::new();
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
                cat: if byte.is_ascii_alphabetic() {
                    Catcode::Letter
                } else {
                    Catcode::Other
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

#[test]
fn alignment_preamble_discards_leading_spaces_from_each_u_template_only() {
    let mut command = CommandState::default();
    let alignment = crate::AlignmentIdentity::new(1);
    command.begin_alignment(alignment);
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
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
            .expect("opening brace validates and backs up");
        processor
            .replay_alignment_preamble_opening()
            .expect("opening brace replays before preamble collection");
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
    let mut universe = Universe::new();
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
            .expect("opening brace validates and backs up");
        processor
            .replay_alignment_preamble_opening()
            .expect("opening brace replays before preamble collection");
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
    let mut universe = Universe::new();
    let mut capabilities = CommandHostCapabilities::default();
    capabilities.register_input(
        "inc",
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
    assert_eq!(input.file_name.name, "inc");
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
    assert_eq!(error, CommandError::MissingInput("x".to_owned()));
}

#[test]
fn pdf_graphics_scanners_freeze_immediate_and_shipout_literal_payloads() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
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
    let mut universe = Universe::new();
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
fn pdf_navigation_applies_halfword_bound_only_to_dest_and_thread_ids() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
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
    let mut universe = Universe::new();
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
    let mut universe = Universe::new();
    let mut capabilities = CommandHostCapabilities::default();
    push(&mut command, text_tokens("3 -1 16 15 "));
    let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);
    // TeX82 §1350: `if cur_val<0 then cur_val:=17 else if cur_val>15 then
    // cur_val:=16`, so `write_stream` is always one of §1342's eighteen
    // slots and `\wlog`'s `\m@ne` is recorded as 17, not -1.
    let scanned: Vec<_> = (0..4)
        .map(|_| {
            processor
                .scan_write_stream()
                .expect("write stream number scans")
        })
        .collect();
    assert_eq!(scanned, vec![3, 17, 16, 15]);
}
