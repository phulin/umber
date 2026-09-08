use super::{RecordingObserver, install_static, letter, other};
use crate::processor::expansion_depth::EXPANSION_DEPTH_LIMIT;
use crate::{CommandError, CommandFuelLedger, CommandHostCapabilities, CommandState, FatalError};
use tex_state::meaning::{ExpandablePrimitive, Meaning, UnexpandablePrimitive};
use tex_state::token::Token;

#[derive(Clone, Copy, Debug)]
enum Family {
    TokenLists,
    Uniform,
    RegisterIndex,
    IntegerExpression,
    DimensionExpression,
}

impl Family {
    fn calls_per_level(self) -> u32 {
        match self {
            Self::IntegerExpression | Self::DimensionExpression => 2,
            _ => 1,
        }
    }

    fn output(self) -> &'static str {
        match self {
            Self::TokenLists => "X",
            Self::DimensionExpression => "0.0ptX",
            _ => "0X",
        }
    }
}

fn fixture<G>(universe: &mut tex_state::Universe<G>, family: Family, depth: usize) -> Vec<Token> {
    let the = install_static(
        universe,
        "the",
        Meaning::ExpandablePrimitive(ExpandablePrimitive::The),
    );
    let relax = install_static(universe, "relax", Meaning::Relax);
    let mut input = Vec::new();
    match family {
        Family::TokenLists => {
            let toks = install_static(universe, "toks", Meaning::ToksRegister(0));
            input.extend(std::iter::repeat_n(the, depth));
            input.extend(std::iter::repeat_n(toks, depth));
        }
        Family::Uniform => {
            let uniform = install_static(
                universe,
                "pdfuniformdeviate",
                Meaning::ExpandablePrimitive(ExpandablePrimitive::PdfUniformDeviate),
            );
            input.extend(std::iter::repeat_n(uniform, depth));
            input.push(other('1'));
        }
        Family::RegisterIndex => {
            let count = install_static(
                universe,
                "count",
                Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Count),
            );
            input.extend((0..depth).flat_map(|_| [the, count]));
            input.push(other('0'));
        }
        Family::IntegerExpression | Family::DimensionExpression => {
            let dimension = matches!(family, Family::DimensionExpression);
            let expression = install_static(
                universe,
                "expression",
                Meaning::UnexpandablePrimitive(if dimension {
                    UnexpandablePrimitive::DimExpr
                } else {
                    UnexpandablePrimitive::NumExpr
                }),
            );
            input.extend((0..depth).flat_map(|_| [the, expression]));
            input.push(other('0'));
            if dimension {
                input.extend([other('p'), other('t')]);
            }
            input.extend(std::iter::repeat_n(relax, depth));
        }
    }
    input.push(letter('X'));
    input
}

fn check(family: Family, depth: usize, parent_depth: u32, should_overflow: bool, observed: bool) {
    crate::test_harness::with_universe(|universe| {
        let input = fixture(universe, family, depth);
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(&mut command, input);
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = CommandFuelLedger::default();
        let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut observer = RecordingObserver::default();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut effects,
        );
        if observed {
            processor = processor.with_observer(&mut observer);
        }
        // Model an already-active caller without requiring thousands of native
        // frames in the routine tier. Actual nested scans consume the remaining
        // capacity, including both expand and scan_expr for expression families.
        processor.expansion_depth = parent_depth;
        let mut output = String::new();
        let mut failure = None;
        loop {
            match processor.get_x_token() {
                Ok(Some(command)) => match command.spelling().semantic_token() {
                    Token::Char { ch, .. } => output.push(ch),
                    token => panic!("unexpected terminal spelling {token:?}"),
                },
                Ok(None) => break,
                Err(error) => {
                    failure = Some(error);
                    break;
                }
            }
            assert_eq!(
                processor.expansion_depth, parent_depth,
                "completed scan leaked depth"
            );
        }
        assert_eq!(
            processor.expansion_depth, parent_depth,
            "unwind leaked depth"
        );
        assert_eq!(processor.command.transient.active_expansion_depth, 0);
        if should_overflow {
            assert!(
                matches!(
                    failure,
                    Some(CommandError::Fatal(FatalError::CapacityExceeded {
                        resource: "expansion depth",
                        amount: 10_000,
                    }))
                ),
                "{family:?} at depth {depth}: {failure:?}"
            );
            assert!(
                output.is_empty(),
                "no outer conversion may finish before its child"
            );
        } else {
            assert!(
                failure.is_none(),
                "{family:?} at depth {depth}: {failure:?}"
            );
            assert_eq!(output, family.output());
        }
    });
}

fn check_family(family: Family) {
    // tex.ch §366 and etex.ch [53a]: entry reaching 10000 is rejected;
    // completed siblings release capacity. These are semantic depth tests,
    // not tests of a retired internal continuation-lane implementation.
    for observed in [false, true] {
        for depth in [1, 16, 64] {
            let calls = u32::try_from(depth).expect("test depth") * family.calls_per_level();
            check(family, depth, 0, false, observed);
            check(
                family,
                depth,
                EXPANSION_DEPTH_LIMIT - calls - 1,
                false,
                observed,
            );
            check(family, depth, EXPANSION_DEPTH_LIMIT - calls, true, observed);
        }
    }
}

#[test]
fn nested_the_token_lists_obey_pdftex_expansion_capacity() {
    check_family(Family::TokenLists);
}

#[test]
fn nested_uniform_scans_obey_pdftex_expansion_capacity() {
    check_family(Family::Uniform);
}

#[test]
fn nested_register_indices_obey_pdftex_expansion_capacity() {
    check_family(Family::RegisterIndex);
}

#[test]
fn nested_integer_expressions_share_pdftex_expansion_capacity() {
    check_family(Family::IntegerExpression);
}

#[test]
fn nested_dimension_expressions_share_pdftex_expansion_capacity() {
    check_family(Family::DimensionExpression);
}

#[test]
fn sequential_macro_delivery_does_not_consume_recursive_expansion_capacity() {
    crate::test_harness::with_universe(|universe| {
        let definition = universe
            .allocate_definition(&[], &[tex_state::token::TokenWord::pack(letter('A'))])
            .expect("macro definition");
        let symbol = universe.intern("lettermacro").expect("macro symbol");
        universe
            .assign_meaning(
                symbol,
                tex_state::meaning::MeaningWord::macro_definition(
                    tex_state::meaning::MeaningFlags::EMPTY,
                    definition,
                ),
                tex_state::env::AssignmentScope::Global,
            )
            .expect("macro meaning");
        let mut command = CommandState::default();
        crate::test_harness::push(
            &mut command,
            std::iter::repeat_n(Token::Cs(symbol.symbol()), 10_001),
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = CommandFuelLedger::default();
        let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut effects,
        );
        processor.expansion_depth = EXPANSION_DEPTH_LIMIT - 1;
        for _ in 0..10_001 {
            assert_eq!(
                processor
                    .get_x_token()
                    .expect("iterative macro")
                    .expect("macro character")
                    .spelling()
                    .semantic_token(),
                letter('A')
            );
            assert_eq!(processor.expansion_depth, EXPANSION_DEPTH_LIMIT - 1);
        }
        assert!(processor.get_x_token().expect("end").is_none());
    });
}

#[test]
fn expression_parentheses_do_not_enter_recursive_expansion_capacity() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        crate::test_harness::push(
            &mut command,
            std::iter::repeat_n(other('('), 128)
                .chain([other('7')])
                .chain(std::iter::repeat_n(other(')'), 128)),
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = CommandFuelLedger::default();
        let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut effects,
        );
        processor.expansion_depth = EXPANSION_DEPTH_LIMIT - 2;
        assert_eq!(
            processor
                .scan_expression_primitive(UnexpandablePrimitive::NumExpr)
                .expect("parenthesized expression"),
            crate::InternalValue::Integer(7)
        );
        assert_eq!(processor.expansion_depth, EXPANSION_DEPTH_LIMIT - 2);
    });
}

#[test]
fn failed_nested_operand_scan_restores_shared_expansion_capacity() {
    crate::test_harness::with_universe(|universe| {
        for budget in 1..16 {
            let input = fixture(universe, Family::IntegerExpression, 4);
            let mut command = CommandState::default();
            crate::test_harness::push(&mut command, input);
            let mut capabilities = CommandHostCapabilities::default();
            let mut fuel = CommandFuelLedger::new(budget).expect("bounded fuel");
            let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
            let mut context = universe.command_context().expect("command context");
            let mut processor = crate::test_harness::processor(
                &mut command,
                &mut context,
                &mut capabilities,
                &mut fuel,
                &mut effects,
            );
            let mut destination = None;
            assert!(matches!(
                processor.get_x_token_into(&mut destination),
                Err(CommandError::FuelExhausted { .. })
            ));
            assert!(destination.is_none());
            assert_eq!(processor.expansion_depth, 0);
            assert_eq!(processor.command.transient.active_expansion_depth, 0);
            assert_eq!(processor.command.scratch.expression_stack_len(), 0);
        }
    });
}

#[test]
#[ignore = "explicit native-stack scaling tier; see command_delivery_kernel.md"]
fn full_default_expansion_capacity_on_a_sufficient_native_stack() {
    // Web2C documents that a small native stack can overflow before expand_depth.
    // Reserve enough virtual stack for this explicit default-capacity audit;
    // production semantics and the configured capacity are not changed.
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(|| {
            for family in [
                Family::TokenLists,
                Family::Uniform,
                Family::RegisterIndex,
                Family::IntegerExpression,
                Family::DimensionExpression,
            ] {
                check(family, 1_024, 0, false, false);
                let boundary = EXPANSION_DEPTH_LIMIT / family.calls_per_level();
                check(family, boundary as usize, 0, true, false);
            }
        })
        .expect("scaling thread")
        .join()
        .expect("scaling audit");
}
