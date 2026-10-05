//! TeX82 §§405/407 token identity, backup order and exact raw-token fuel.
use super::*;
use crate::{CommandHostCapabilities, CommandObserver, CommandState};
use tex_state::AssignmentScope;
use tex_state::token::Token;

struct Observer;
impl CommandObserver for Observer {
    fn committed(&mut self, _: crate::CommandObservation) {}
}

fn character(ch: char, cat: Catcode) -> Token {
    Token::Char { ch, cat }
}

fn alias<G>(universe: &mut tex_state::Universe<G>, name: &str, ch: char, cat: Catcode) -> Token {
    let symbol = universe.intern(name).expect("alias name");
    universe
        .assign_meaning(
            symbol,
            MeaningWord::from_static(Meaning::CharToken { ch, cat }),
            AssignmentScope::Global,
        )
        .expect("character alias");
    Token::Cs(symbol.symbol())
}

#[test]
fn keyword_requires_literal_spelling_and_preserves_failed_prefix_levels() {
    for observed in [false, true] {
        crate::test_harness::with_universe(|universe| {
            let space = alias(universe, "blank", ' ', Catcode::Space);
            let equals = character('=', Catcode::Other);
            let letter = alias(universe, "letter", 'e', Catcode::Letter);
            let active = universe.intern_active_character('~').expect("active name");
            universe
                .assign_meaning(
                    active,
                    MeaningWord::from_static(Meaning::CharToken {
                        ch: 'e',
                        cat: Catcode::Letter,
                    }),
                    AssignmentScope::Global,
                )
                .expect("active character alias");
            let active = character('~', Catcode::Active);
            let tail = character('X', Catcode::Other);
            let mut command = CommandState::default();
            crate::test_harness::push(
                &mut command,
                [
                    space,
                    equals,
                    space,
                    character('E', Catcode::BeginGroup),
                    character('M', Catcode::EndGroup),
                    letter,
                    active,
                    character('e', Catcode::Letter),
                    tail,
                    character('Y', Catcode::Other),
                ],
            );
            let mut capabilities = CommandHostCapabilities::default();
            let mut fuel = crate::CommandFuelLedger::default();
            let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
            let mut observer = Observer;
            let mut context = universe.command_context().expect("context");
            let processor = crate::test_harness::processor(
                &mut command,
                &mut context,
                &mut capabilities,
                &mut fuel,
                &mut effects,
            );
            let mut processor = if observed {
                processor.with_observer(&mut observer)
            } else {
                processor
            };
            let alignment_before = processor.command.roots.alignment.align_state;
            assert!(processor.scan_optional_equals().expect("optional equals"));
            assert!(
                processor
                    .scan_keyword("em")
                    .expect("category-independent keyword")
            );
            assert_eq!(
                processor.command.roots.alignment.align_state,
                alignment_before
            );
            for token in [letter, active] {
                assert!(
                    !processor
                        .scan_keyword("e")
                        .expect("CS cannot spell keyword")
                );
                assert_eq!(
                    crate::test_harness::expect_expanded_command(&mut processor)
                        .spelling()
                        .semantic_token(),
                    token,
                    "the declined alias must be backed with its original spelling",
                );
            }
            // §407 backs a matched prefix above the separate offending level.
            assert!(!processor.scan_keyword("em").expect("partial mismatch"));
            assert_eq!(processor.command.roots.input.levels.len(), 3);
            for token in [
                character('e', Catcode::Letter),
                tail,
                character('Y', Catcode::Other),
            ] {
                assert_eq!(
                    crate::test_harness::expect_expanded_command(&mut processor)
                        .spelling()
                        .semantic_token(),
                    token,
                );
            }
        });
    }
}

#[test]
fn small_scalar_syntax_charges_once_and_stops_before_the_next_word() {
    for observed in [false, true] {
        for keyword in [false, true] {
            for limit in 1..=4 {
                crate::test_harness::with_universe(|universe| {
                    let mut command = CommandState::default();
                    crate::test_harness::push(
                        &mut command,
                        if keyword { " BYX" } else { "  =X" }.chars().map(|ch| {
                            character(
                                ch,
                                if ch == ' ' {
                                    Catcode::Space
                                } else {
                                    Catcode::Other
                                },
                            )
                        }),
                    );
                    let mut capabilities = CommandHostCapabilities::default();
                    let mut fuel = crate::CommandFuelLedger::new(limit).expect("bounded fuel");
                    let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
                    let mut observer = Observer;
                    let mut context = universe.command_context().expect("context");
                    let processor = crate::test_harness::processor(
                        &mut command,
                        &mut context,
                        &mut capabilities,
                        &mut fuel,
                        &mut effects,
                    );
                    let mut processor = if observed {
                        processor.with_observer(&mut observer)
                    } else {
                        processor
                    };
                    let result = if keyword {
                        processor.scan_keyword("by")
                    } else {
                        processor.scan_optional_equals()
                    };
                    if limit < 3 {
                        assert!(
                            result.is_err(),
                            "exact fuel cut precedes grammar completion"
                        );
                    } else {
                        assert!(result.expect("complete syntax"));
                    }
                    assert_eq!(
                        processor
                            .command
                            .roots
                            .input
                            .levels
                            .current
                            .as_ref()
                            .expect("actual frame")
                            .frame
                            .position(),
                        limit.min(3) as u32,
                        "completion or a failed charge leaves the following word unread",
                    );
                    drop(processor);
                    assert_eq!(fuel.burned(), limit.min(3));
                });
            }
        }
    }
}
