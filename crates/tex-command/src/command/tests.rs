use tex_state::env::AssignmentScope;
use tex_state::meaning::{
    ExpandablePrimitive, InternalInteger, Meaning, MeaningFlags, MeaningWord, ResolvedMeaning,
    UnexpandablePrimitive,
};
use tex_state::token::{Catcode, OriginId, Token, TokenWord, TracedTokenWord};

use super::{CurrentCommand, DeliveryStamp};

fn resolved<G>(universe: &mut tex_state::Universe<G>, token: Token) -> CurrentCommand<G> {
    CurrentCommand::resolve(
        TracedTokenWord::pack(token, OriginId::UNKNOWN),
        DeliveryStamp::new(17, 23, 29),
        None,
        false,
        None,
        &universe.command_context().expect("command context"),
    )
}

#[test]
fn delivered_command_keeps_the_resolved_meaning_and_exact_spelling() {
    crate::test_harness::with_universe(|universe| {
        let symbol = universe.intern("defined").expect("intern");
        universe
            .assign_meaning(
                symbol,
                MeaningWord::from_static(Meaning::CharGiven('A')),
                AssignmentScope::Global,
            )
            .expect("first meaning");
        let command = resolved(universe, Token::Cs(symbol.symbol()));
        universe
            .assign_meaning(
                symbol,
                MeaningWord::from_static(Meaning::CharGiven('B')),
                AssignmentScope::Global,
            )
            .expect("replacement meaning");

        assert_eq!(
            command.spelling().semantic_token(),
            Token::Cs(symbol.symbol())
        );
        assert_eq!(command.meaning(), Meaning::CharGiven('A'));
        assert_eq!(command.delivery_stamp(), DeliveryStamp::new(17, 23, 29));
    });
}

#[test]
fn ordinary_character_is_resolved_without_a_state_handle() {
    crate::test_harness::with_universe(|universe| {
        let command = resolved(
            universe,
            Token::Char {
                ch: 'x',
                cat: Catcode::Letter,
            },
        );
        assert_eq!(
            command.meaning(),
            ResolvedMeaning::Static(Meaning::CharToken {
                ch: 'x',
                cat: Catcode::Letter,
            })
        );
        assert_eq!(command.control_sequence(), None);
    });
}

#[test]
fn macro_delivery_carries_a_generation_typed_definition_coordinate() {
    crate::test_harness::with_universe(|universe| {
        let replacement = TokenWord::pack(Token::Char {
            ch: 'M',
            cat: Catcode::Letter,
        });
        let definition = universe
            .allocate_definition(&[], &[replacement])
            .expect("definition");
        let symbol = universe.intern("macro").expect("intern");
        universe
            .assign_meaning(
                symbol,
                MeaningWord::macro_definition(MeaningFlags::LONG, definition.clone()),
                AssignmentScope::Global,
            )
            .expect("macro meaning");

        let command = resolved(universe, Token::Cs(symbol.symbol()));
        let ResolvedMeaning::Macro {
            flags,
            definition: delivered,
        } = command.meaning()
        else {
            panic!("macro meaning")
        };
        assert_eq!(flags, MeaningFlags::LONG);
        assert_eq!(delivered, definition);
        let context = universe.command_context().expect("context");
        assert_eq!(
            context.definition(delivered).replacement_text(),
            &[replacement]
        );
    });
}

#[test]
fn frozen_endwrite_delivery_retains_its_outer_macro_command() {
    crate::test_harness::with_universe(|universe| {
        crate::install_tex82_unexpandable_primitives(universe);
        let endwrite = universe.primitive_token("endwrite").expect("write stopper");
        let command = resolved(universe, endwrite);
        let ResolvedMeaning::Macro { flags, definition } = command.meaning() else {
            panic!("frozen endwrite meaning")
        };
        assert_eq!(flags, MeaningFlags::OUTER);
        assert!(
            universe
                .command_context()
                .expect("context")
                .definition(definition)
                .replacement_text()
                .is_empty()
        );
        assert_eq!(
            crate::observation::canonical_current_command_identity(&command),
            ("outer_call".to_owned(), None)
        );
    });
}

#[test]
fn command_code_partition_classifies_character_internal_unexpandable_and_expandable_ranges() {
    crate::test_harness::with_universe(|universe| {
        let cases = [
            (
                Meaning::CharToken {
                    ch: 'x',
                    cat: Catcode::Letter,
                },
                "character",
                false,
            ),
            (
                Meaning::InternalInteger(InternalInteger::Badness),
                "internal",
                false,
            ),
            (
                Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Def),
                "unexpandable",
                false,
            ),
            (
                Meaning::ExpandablePrimitive(ExpandablePrimitive::ExpandAfter),
                "expandable",
                true,
            ),
        ];

        for (index, (meaning, expected_partition, expected_expandable)) in
            cases.into_iter().enumerate()
        {
            let symbol = universe
                .intern(&format!("partition{index}"))
                .expect("partition name");
            universe
                .assign_meaning(
                    symbol,
                    MeaningWord::from_static(meaning),
                    AssignmentScope::Global,
                )
                .expect("partition meaning");
            let command = resolved(universe, Token::Cs(symbol.symbol()));
            let actual_partition = match command.meaning() {
                ResolvedMeaning::Static(Meaning::CharToken { .. } | Meaning::CharGiven(_)) => {
                    "character"
                }
                ResolvedMeaning::Static(
                    Meaning::InternalInteger(_)
                    | Meaning::CountRegister(_)
                    | Meaning::DimenRegister(_)
                    | Meaning::SkipRegister(_)
                    | Meaning::MuskipRegister(_)
                    | Meaning::ToksRegister(_)
                    | Meaning::IntParam(_)
                    | Meaning::DimenParam(_)
                    | Meaning::GlueParam(_)
                    | Meaning::MuGlueParam(_)
                    | Meaning::TokParam(_)
                    | Meaning::PageDimension(_)
                    | Meaning::PageInteger(_)
                    | Meaning::Font(_),
                ) => "internal",
                ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(_) | Meaning::EndV) => {
                    "unexpandable"
                }
                ResolvedMeaning::Static(Meaning::ExpandablePrimitive(_))
                | ResolvedMeaning::Macro { .. } => "expandable",
                _ => "other",
            };
            assert_eq!(actual_partition, expected_partition, "case {index}");
            assert_eq!(
                crate::processor::expand::is_expandable_command(&command),
                expected_expandable,
                "case {index} expansion boundary"
            );
        }

        assert_eq!(Catcode::Escape as u8, 0);
        assert_eq!(Catcode::Invalid as u8, 15);
        assert_eq!(UnexpandablePrimitive::Def.operand(), 0);
        assert_eq!(ExpandablePrimitive::ExpandAfter.operand(), 0);
    });
}
