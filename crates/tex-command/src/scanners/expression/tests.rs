use tex_state::env::AssignmentScope;
use tex_state::meaning::{Meaning, MeaningWord, UnexpandablePrimitive};
use tex_state::token::{Catcode, Token};

use crate::{CommandHostCapabilities, CommandProfile, CommandState, processor::DeliveryStatus};

fn other(ch: char) -> Token {
    Token::Char {
        ch,
        cat: Catcode::Other,
    }
}

#[test]
fn signs_before_parentheses_leave_the_invalid_factor_for_recovery() {
    // etex.ch [53a]'s factor admission tests '(' before TeX82 §§440--445
    // scans signs. The sign does not turn a parenthesis into a number.
    crate::test_harness::with_universe(|universe| {
        for signs in ["-", "+", "--"] {
            let mut command = CommandState::new(CommandProfile::ETEX26);
            crate::test_harness::push(
                &mut command,
                signs.chars().chain("(1+2)X".chars()).map(other),
            );
            let mut capabilities = CommandHostCapabilities::default();
            let mut fuel = crate::CommandFuelLedger::default();
            let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
            let mut context = universe.command_context().expect("command context");
            let mut processor = crate::test_harness::processor(
                &mut command,
                &mut context,
                &mut capabilities,
                &mut fuel,
                &mut effects,
            );
            assert!(matches!(
                processor
                    .scan_expression_primitive(UnexpandablePrimitive::NumExpr)
                    .expect("missing-number recovery"),
                crate::InternalValue::Integer(0)
            ));
            assert_eq!(
                crate::test_harness::expect_expanded_command(&mut processor)
                    .spelling()
                    .semantic_token(),
                other('('),
                "the invalid factor remains available after {signs}"
            );
        }
    });
}

#[test]
fn numexpr_honors_precedence_and_leaves_its_relax_terminator_consumed() {
    crate::test_harness::with_universe(|universe| {
        let numexpr = universe.intern("numexpr").expect("numexpr");
        universe
            .assign_meaning(
                numexpr,
                MeaningWord::from_static(Meaning::UnexpandablePrimitive(
                    UnexpandablePrimitive::NumExpr,
                )),
                AssignmentScope::Global,
            )
            .expect("numexpr meaning");
        let relax = universe.intern("relax").expect("relax");
        universe
            .assign_meaning(
                relax,
                MeaningWord::from_static(Meaning::Relax),
                AssignmentScope::Global,
            )
            .expect("relax meaning");
        let mut command = CommandState::new(CommandProfile::ETEX26);
        crate::test_harness::push(
            &mut command,
            [
                Token::Cs(numexpr.symbol()),
                other('2'),
                other('+'),
                other('3'),
                other('*'),
                other('4'),
                Token::Cs(relax.symbol()),
                other('X'),
            ],
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );
        let scalar_result = match processor.scan_integer() {
            Ok(value) => value,
            Err(error) => panic!("scalar scan failed: {error:?}"),
        };
        let scanned = scalar_result;
        assert_eq!(scanned.value, 14);
        let mut following = None;
        assert_eq!(
            processor
                .get_x_token_into(&mut following)
                .expect("following"),
            DeliveryStatus::Command
        );
        assert_eq!(
            following
                .expect("following token")
                .spelling()
                .semantic_token(),
            other('X')
        );
    });
}

#[cfg(feature = "profiling")]
#[test]
fn warmed_numexpr_destination_path_allocates_zero_heap() {
    crate::test_harness::with_universe(|universe| {
        const SCANS: usize = 4_097;
        let numexpr = universe.intern("numexpr").expect("numexpr");
        universe
            .assign_meaning(
                numexpr,
                MeaningWord::from_static(Meaning::UnexpandablePrimitive(
                    UnexpandablePrimitive::NumExpr,
                )),
                AssignmentScope::Global,
            )
            .expect("numexpr meaning");
        let relax = universe.intern("relax").expect("relax");
        universe
            .assign_meaning(
                relax,
                MeaningWord::from_static(Meaning::Relax),
                AssignmentScope::Global,
            )
            .expect("relax meaning");
        let expression = [
            Token::Cs(numexpr.symbol()),
            other('2'),
            other('+'),
            other('3'),
            other('*'),
            other('4'),
            Token::Cs(relax.symbol()),
        ];
        let mut command = CommandState::new(CommandProfile::ETEX26);
        crate::test_harness::push(
            &mut command,
            (0..SCANS).flat_map(|_| expression.iter().cloned()),
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );
        let scalar_result = match processor.scan_integer() {
            Ok(value) => value,
            Err(error) => panic!("scalar scan failed: {error:?}"),
        };
        assert_eq!(scalar_result.value, 14);

        let owner = tex_state::measurement::HotCoreAllocationOwner::DeliveryAndScan;
        let before = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
        {
            let _scope = tex_state::measurement::hot_core_allocation_scope(owner);
            for _ in 1..SCANS {
                let scalar_result = match processor.scan_integer() {
                    Ok(value) => value,
                    Err(error) => {
                        panic!("scalar scan failed: {error:?}")
                    }
                };
                assert_eq!(scalar_result.value, 14);
            }
        }
        let after = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
        assert_eq!(after.calls - before.calls, 0);
        assert_eq!(after.requested_bytes - before.requested_bytes, 0);
    });
}

#[test]
fn expression_probes_use_character_alias_meanings_and_preserve_following_spelling() {
    // e-TeX scan_expr tests cur_cmd/cur_chr, so aliases of spaces and
    // other-character parentheses/operators participate in its grammar.
    #[derive(Default)]
    struct Observer(Vec<crate::CommandObservation>);
    impl crate::CommandObserver for Observer {
        fn committed(&mut self, observation: crate::CommandObservation) {
            self.0.push(observation);
        }
    }
    for observed in [false, true] {
        crate::test_harness::with_universe(|universe| {
            let mut aliases = Vec::new();
            for (name, ch, cat) in [
                ("space_alias", ' ', Catcode::Space),
                ("left_alias", '(', Catcode::Other),
                ("plus_alias", '+', Catcode::Other),
                ("right_alias", ')', Catcode::Other),
                ("following_alias", 'X', Catcode::Other),
            ] {
                let symbol = universe.intern(name).expect("alias symbol");
                universe
                    .assign_meaning(
                        symbol,
                        MeaningWord::from_static(Meaning::CharToken { ch, cat }),
                        AssignmentScope::Global,
                    )
                    .expect("character alias");
                aliases.push(Token::Cs(symbol.symbol()));
            }
            let [space, left, plus, right, following] = aliases[..] else {
                unreachable!();
            };
            let mut command = CommandState::new(CommandProfile::ETEX26);
            crate::test_harness::push(
                &mut command,
                [
                    space,
                    other('2'),
                    other('*'),
                    left,
                    space,
                    other('1'),
                    space,
                    plus,
                    space,
                    other('2'),
                    right,
                    space,
                    following,
                ],
            );
            let mut capabilities = CommandHostCapabilities::default();
            let mut fuel = crate::CommandFuelLedger::default();
            let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
            let mut context = universe.command_context().expect("context");
            let mut observer = Observer::default();
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
            assert!(matches!(
                processor
                    .scan_expression_primitive(UnexpandablePrimitive::NumExpr)
                    .expect("alias expression"),
                crate::InternalValue::Integer(6)
            ));
            assert_eq!(
                crate::test_harness::expect_expanded_command(&mut processor)
                    .spelling()
                    .semantic_token(),
                following,
                "backup preserves the control-sequence spelling"
            );
        });
    }
}
