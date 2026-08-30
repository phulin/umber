use tex_state::env::AssignmentScope;
use tex_state::meaning::{
    ExpandablePrimitive, InternalInteger, Meaning, MeaningFlags, MeaningWord, ResolvedMeaning,
    UnexpandablePrimitive,
};
use tex_state::token::{Catcode, OriginId, Token, TokenWord, TracedTokenWord};

use super::{CurrentCommand, DeliveryStamp, EmptyCommand, RawCommand, ResolvedCommand};

#[test]
fn command_delivery_layout_stays_compact() {
    assert!(std::mem::size_of::<ResolvedMeaning<()>>() <= 24);
    assert_eq!(std::mem::size_of::<Option<crate::SourceProvenance>>(), 32);
    // The canonical command carries raw identity, resolved meaning, exact
    // provenance, and execution metadata in less than two 64-byte cache lines.
    assert_eq!(std::mem::size_of::<CurrentCommand<()>>(), 112);
    assert!(std::mem::size_of::<crate::DeliveryStatus>() <= 16);
    assert_eq!(
        std::mem::size_of::<EmptyCommand<'_, ()>>(),
        std::mem::size_of::<&mut CurrentCommand<()>>()
    );
    assert_eq!(
        std::mem::size_of::<RawCommand<'_, ()>>(),
        std::mem::size_of::<&mut CurrentCommand<()>>()
    );
    assert_eq!(
        std::mem::size_of::<ResolvedCommand<'_, ()>>(),
        std::mem::size_of::<&mut CurrentCommand<()>>()
    );
}

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
fn raw_resolution_preparation_and_execution_borrow_one_command_address() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CurrentCommand::empty();
        let spelling = TracedTokenWord::pack(
            Token::Char {
                ch: 'x',
                cat: Catcode::Letter,
            },
            OriginId::UNKNOWN,
        );
        let slot = core::ptr::from_ref(&command);
        let raw = command
            .empty_for_raw_delivery()
            .write_raw_delivery(spelling, 17, 23, None, None, false, None, false);
        assert_eq!(core::ptr::from_ref(raw.0), slot);
        let (resolved, meaning_lookup) =
            raw.resolve_in_place(29, &universe.command_context().expect("command context"));
        assert!(!meaning_lookup);
        assert_eq!(core::ptr::from_ref(resolved.as_ref()), slot);

        fn prepare<G>(command: &CurrentCommand<G>) -> *const CurrentCommand<G> {
            core::ptr::from_ref(command)
        }
        fn execute<G>(command: &CurrentCommand<G>) -> *const CurrentCommand<G> {
            core::ptr::from_ref(command)
        }
        assert_eq!(prepare(resolved.as_ref()), slot);
        assert_eq!(execute(resolved.as_ref()), slot);
        assert_eq!(
            resolved.as_ref().delivery_stamp(),
            DeliveryStamp::new(17, 23, 29)
        );
        assert_eq!(
            resolved.as_ref().meaning_ref(),
            &ResolvedMeaning::Static(Meaning::CharToken {
                ch: 'x',
                cat: Catcode::Letter,
            })
        );
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
        universe
            .assign_meaning(
                symbol,
                MeaningWord::from_static(Meaning::Relax),
                AssignmentScope::Global,
            )
            .expect("replace delivered meaning");
        drop(definition);

        let ResolvedMeaning::Macro {
            flags,
            definition: delivered,
        } = command.meaning_ref()
        else {
            panic!("macro meaning")
        };
        assert_eq!(*flags, MeaningFlags::LONG);
        assert_eq!(delivered.replacement_word(0), Some(replacement));
        assert_eq!(
            crate::observation::canonical_current_command_identity(&command),
            ("long_call".to_owned(), None)
        );
    });
}

#[test]
fn resolving_in_place_acquires_and_releases_exactly_one_macro_owner() {
    crate::test_harness::with_universe(|universe| {
        let definition = universe
            .allocate_definition(
                &[],
                &[TokenWord::pack(Token::Char {
                    ch: 'M',
                    cat: Catcode::Letter,
                })],
            )
            .expect("definition");
        let symbol = universe.intern("ownedmacro").expect("intern");
        universe
            .assign_meaning(
                symbol,
                MeaningWord::macro_definition(MeaningFlags::EMPTY, definition.clone()),
                AssignmentScope::Global,
            )
            .expect("macro meaning");
        let baseline = definition.semantic_owner_count();
        let mut command = CurrentCommand::empty();

        let raw = command.empty_for_raw_delivery().write_raw_delivery(
            TracedTokenWord::pack(Token::Cs(symbol.symbol()), OriginId::UNKNOWN),
            3,
            5,
            None,
            None,
            false,
            None,
            false,
        );
        assert_eq!(definition.semantic_owner_count(), baseline);
        let _ = raw.resolve_in_place(7, &universe.command_context().expect("command context"));
        assert_eq!(definition.semantic_owner_count(), baseline + 1);

        let raw = command.empty_for_raw_delivery().write_raw_delivery(
            TracedTokenWord::pack(
                Token::Char {
                    ch: 'x',
                    cat: Catcode::Letter,
                },
                OriginId::UNKNOWN,
            ),
            11,
            13,
            None,
            None,
            false,
            None,
            false,
        );
        // Raw preparation does not manufacture another owner. Resolution is
        // the sole point that replaces the preceding resolved meaning.
        assert_eq!(definition.semantic_owner_count(), baseline + 1);
        let _ = raw.resolve_in_place(17, &universe.command_context().expect("command context"));
        assert_eq!(definition.semantic_owner_count(), baseline);
        assert_eq!(command.delivery_stamp(), DeliveryStamp::new(11, 13, 17));
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
