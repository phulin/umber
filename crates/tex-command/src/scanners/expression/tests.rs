use tex_state::Universe;
use tex_state::glue::{GlueSpec, Order};
use tex_state::meaning::{Meaning, UnexpandablePrimitive as P};
use tex_state::scaled::Scaled;
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};
use tex_state::{EffectRecord, PrintSink};

use super::*;
use crate::input::{
    ReplayTrace, RetirementBehavior, SharedTokenBuffer, TokenBehavior, TokenPayload,
};
use crate::{
    CommandHostCapabilities, CommandHostContext, CommandObservation, CommandObserver,
    CommandProfile, CommandRuntime, CommandState,
};

#[derive(Default)]
struct Recorder(Vec<CommandObservation>);

impl CommandObserver for Recorder {
    fn committed(&mut self, observation: CommandObservation) {
        self.0.push(observation);
    }
}

fn push(command: &mut CommandState, tokens: Vec<Token>) {
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(
            tokens
                .into_iter()
                .map(|token| TracedTokenWord::pack(token, OriginId::UNKNOWN))
                .collect::<Vec<_>>(),
        )),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
}

fn char_token(ch: char) -> Token {
    Token::Char {
        ch,
        cat: if ch.is_ascii_alphabetic() {
            Catcode::Letter
        } else if ch == ' ' {
            Catcode::Space
        } else {
            Catcode::Other
        },
    }
}

fn chars(source: &str) -> Vec<Token> {
    source.chars().map(char_token).collect()
}

fn primitive(universe: &mut Universe, name: &str, primitive: P) -> Token {
    let symbol = universe.intern(name).symbol();
    universe.set_meaning(symbol, Meaning::UnexpandablePrimitive(primitive));
    Token::Cs(symbol)
}

fn relax(universe: &mut Universe) -> Token {
    let symbol = universe.intern("relax").symbol();
    universe.set_meaning(symbol, Meaning::Relax);
    Token::Cs(symbol)
}

fn scanner_kinds(recorder: &Recorder) -> Vec<&'static str> {
    recorder
        .0
        .iter()
        .filter_map(|observation| match observation {
            CommandObservation::Scanner(record) => Some(record.kind),
            _ => None,
        })
        .collect()
}

fn diagnostic_text(universe: &Universe) -> String {
    universe
        .world()
        .effect_records()
        .iter()
        .filter_map(|effect| match effect {
            EffectRecord::StreamWrite {
                sink: PrintSink::Terminal | PrintSink::TerminalAndLog | PrintSink::Log,
                text,
            } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

#[test]
fn numexpr_honors_precedence_parentheses_relax_and_following_token() {
    let mut universe = crate::test_harness::universe();
    let numexpr = primitive(&mut universe, "numexpr", P::NumExpr);
    let relax = relax(&mut universe);
    let mut command = CommandState::new(CommandProfile::ETEX26);
    let mut tokens = vec![numexpr];
    tokens.extend(chars("2+3*4"));
    tokens.extend([relax, char_token('X')]);
    push(&mut command, tokens);
    let mut runtime = CommandRuntime::default();
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = CommandProcessor::new(
        &mut command,
        &mut runtime,
        universe.command_context(),
        CommandHostContext::new(&mut capabilities),
    );

    assert_eq!(
        processor.scan_integer().expect("expression scans").value,
        14
    );
    assert!(matches!(
        processor
            .get_x_token()
            .expect("following token delivers")
            .expect("following token exists")
            .meaning(),
        Meaning::CharToken { ch: 'X', .. }
    ));

    let numexpr = primitive(&mut universe, "numexpr-2", P::NumExpr);
    let mut command = CommandState::new(CommandProfile::ETEX26);
    let mut tokens = vec![numexpr];
    tokens.extend(chars("(2+3)*4"));
    push(&mut command, tokens);
    let mut runtime = CommandRuntime::default();
    let mut capabilities = CommandHostCapabilities::default();
    assert_eq!(
        CommandProcessor::new(
            &mut command,
            &mut runtime,
            universe.command_context(),
            CommandHostContext::new(&mut capabilities),
        )
        .scan_integer()
        .expect("parenthesized expression scans")
        .value,
        20
    );
}

#[test]
fn expression_operators_require_other_character_tokens() {
    let mut universe = crate::test_harness::universe();
    let numexpr = primitive(&mut universe, "numexpr", P::NumExpr);
    let mut command = CommandState::new(CommandProfile::ETEX26);
    push(
        &mut command,
        vec![
            numexpr,
            char_token('2'),
            Token::Char {
                ch: '+',
                cat: Catcode::Letter,
            },
        ],
    );
    let mut runtime = CommandRuntime::default();
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = CommandProcessor::new(
        &mut command,
        &mut runtime,
        universe.command_context(),
        CommandHostContext::new(&mut capabilities),
    );

    assert_eq!(processor.scan_integer().expect("expression scans").value, 2);
    assert!(matches!(
        processor
            .get_x_token()
            .expect("operator token delivers")
            .expect("operator token exists")
            .meaning(),
        Meaning::CharToken {
            ch: '+',
            cat: Catcode::Letter,
        }
    ));
}

#[test]
fn dimexpr_uses_etex_rounding_for_division_and_combined_scaling() {
    for (source, expected) in [("5sp/2", 3), ("-5sp/2", -3), ("1pt*10/3", 218_453)] {
        let mut universe = crate::test_harness::universe();
        let dimexpr = primitive(&mut universe, "dimexpr", P::DimExpr);
        let mut command = CommandState::new(CommandProfile::ETEX26);
        let mut tokens = vec![dimexpr];
        tokens.extend(chars(source));
        push(&mut command, tokens);
        let mut runtime = CommandRuntime::default();
        let mut capabilities = CommandHostCapabilities::default();
        let value = CommandProcessor::new(
            &mut command,
            &mut runtime,
            universe.command_context(),
            CommandHostContext::new(&mut capabilities),
        )
        .scan_dimension()
        .expect("dimension expression scans")
        .value;
        assert_eq!(value.raw(), expected, "source: {source}");
    }
}

#[test]
fn glueexpr_and_muexpr_scale_every_component_and_keep_orders() {
    for (primitive_kind, source, scan_mu, expected) in [
        (
            P::GlueExpr,
            "(1pt plus 2fil+3pt plus 4fil)*3/2",
            false,
            GlueSpec {
                width: Scaled::from_raw(6 * Scaled::UNITY),
                stretch: Scaled::from_raw(9 * Scaled::UNITY),
                stretch_order: Order::Fil,
                shrink: Scaled::from_raw(0),
                shrink_order: Order::Normal,
            },
        ),
        (
            P::MuExpr,
            "(2mu plus 3fil minus 1mu)*3/2",
            true,
            GlueSpec {
                width: Scaled::from_raw(3 * Scaled::UNITY),
                stretch: Scaled::from_raw(4 * Scaled::UNITY + Scaled::UNITY / 2),
                stretch_order: Order::Fil,
                shrink: Scaled::from_raw(Scaled::UNITY + Scaled::UNITY / 2),
                shrink_order: Order::Normal,
            },
        ),
    ] {
        let mut universe = crate::test_harness::universe();
        let expression = primitive(&mut universe, "glue-expression", primitive_kind);
        let mut command = CommandState::new(CommandProfile::ETEX26);
        let mut tokens = vec![expression];
        tokens.extend(chars(source));
        push(&mut command, tokens);
        let mut runtime = CommandRuntime::default();
        let mut capabilities = CommandHostCapabilities::default();
        let value = CommandProcessor::new(
            &mut command,
            &mut runtime,
            universe.command_context(),
            CommandHostContext::new(&mut capabilities),
        )
        .scan_glue(scan_mu)
        .expect("glue expression scans")
        .value;
        assert_eq!(value, expected, "source: {source}");
    }
}

#[test]
fn glueexpr_normalizes_zero_orders_only_at_etex_mutation_points() {
    for (source, expected_order) in [
        ("0pt plus 0fil", Order::Fil),
        ("0pt plus 0fil+0pt", Order::Normal),
    ] {
        let mut universe = crate::test_harness::universe();
        let glueexpr = primitive(&mut universe, "glueexpr", P::GlueExpr);
        let mut command = CommandState::new(CommandProfile::ETEX26);
        let mut tokens = vec![glueexpr];
        tokens.extend(chars(source));
        push(&mut command, tokens);
        let mut runtime = CommandRuntime::default();
        let mut capabilities = CommandHostCapabilities::default();
        let value = CommandProcessor::new(
            &mut command,
            &mut runtime,
            universe.command_context(),
            CommandHostContext::new(&mut capabilities),
        )
        .scan_glue(false)
        .expect("glue expression scans")
        .value;
        assert_eq!(value.stretch, Scaled::from_raw(0));
        assert_eq!(value.stretch_order, expected_order, "source: {source}");
    }
}

#[test]
fn glue_conversions_preserve_components_orders_and_destination_level() {
    for (primitive_kind, source, scan_mu, expected) in [
        (
            P::GlueToMu,
            "2pt plus 3fill minus 4fil",
            true,
            GlueSpec {
                width: Scaled::from_raw(2 * Scaled::UNITY),
                stretch: Scaled::from_raw(3 * Scaled::UNITY),
                stretch_order: Order::Fill,
                shrink: Scaled::from_raw(4 * Scaled::UNITY),
                shrink_order: Order::Fil,
            },
        ),
        (
            P::MuToGlue,
            "5mu plus 6fil minus 7mu",
            false,
            GlueSpec {
                width: Scaled::from_raw(5 * Scaled::UNITY),
                stretch: Scaled::from_raw(6 * Scaled::UNITY),
                stretch_order: Order::Fil,
                shrink: Scaled::from_raw(7 * Scaled::UNITY),
                shrink_order: Order::Normal,
            },
        ),
    ] {
        let mut universe = crate::test_harness::universe();
        let conversion = primitive(&mut universe, "glue-conversion", primitive_kind);
        let mut command = CommandState::new(CommandProfile::ETEX26);
        let mut tokens = vec![conversion];
        tokens.extend(chars(source));
        push(&mut command, tokens);
        let mut runtime = CommandRuntime::default();
        let mut capabilities = CommandHostCapabilities::default();
        let value = CommandProcessor::new(
            &mut command,
            &mut runtime,
            universe.command_context(),
            CommandHostContext::new(&mut capabilities),
        )
        .scan_glue(scan_mu)
        .expect("glue conversion scans")
        .value;
        assert_eq!(value, expected, "source: {source}");
    }
}

#[test]
fn overflow_zeros_the_whole_expression_once_and_consumes_relax() {
    let mut universe = crate::test_harness::universe();
    let numexpr = primitive(&mut universe, "numexpr", P::NumExpr);
    let relax = relax(&mut universe);
    let mut command = CommandState::new(CommandProfile::ETEX26);
    let mut tokens = vec![numexpr];
    tokens.extend(chars("2147483647+1"));
    tokens.extend([relax, char_token('X')]);
    push(&mut command, tokens);
    let mut runtime = CommandRuntime::default();
    let mut capabilities = CommandHostCapabilities::default();
    {
        let mut processor = CommandProcessor::new(
            &mut command,
            &mut runtime,
            universe.command_context(),
            CommandHostContext::new(&mut capabilities),
        );
        assert_eq!(
            processor.scan_integer().expect("expression recovers").value,
            0
        );
        assert!(matches!(
            processor
                .get_x_token()
                .expect("following token delivers")
                .expect("following token exists")
                .meaning(),
            Meaning::CharToken { ch: 'X', .. }
        ));
    }
    assert_eq!(
        diagnostic_text(&universe)
            .matches("Arithmetic overflow")
            .count(),
        1
    );
}

#[test]
fn missing_parenthesis_is_inserted_without_consuming_the_terminator() {
    let mut universe = crate::test_harness::universe();
    let numexpr = primitive(&mut universe, "numexpr", P::NumExpr);
    let mut command = CommandState::new(CommandProfile::ETEX26);
    let mut tokens = vec![numexpr];
    tokens.extend(chars("(1+2X"));
    push(&mut command, tokens);
    let mut runtime = CommandRuntime::default();
    let mut capabilities = CommandHostCapabilities::default();
    {
        let mut processor = CommandProcessor::new(
            &mut command,
            &mut runtime,
            universe.command_context(),
            CommandHostContext::new(&mut capabilities),
        );
        assert_eq!(
            processor.scan_integer().expect("expression recovers").value,
            3
        );
        assert!(matches!(
            processor
                .get_x_token()
                .expect("terminator delivers")
                .expect("terminator exists")
                .meaning(),
            Meaning::CharToken { ch: 'X', .. }
        ));
    }
    assert_eq!(
        diagnostic_text(&universe)
            .matches("Missing ) inserted for expression")
            .count(),
        1
    );
}

#[test]
fn expression_observations_and_checkpoint_retry_are_deterministic() {
    let mut universe = crate::test_harness::universe();
    let numexpr = primitive(&mut universe, "numexpr", P::NumExpr);
    let mut command = CommandState::new(CommandProfile::ETEX26);
    let mut tokens = vec![numexpr];
    tokens.extend(chars("2+3X"));
    push(&mut command, tokens);
    let snapshot = command.snapshot();
    let mut runtime = CommandRuntime::default();
    let mut capabilities = CommandHostCapabilities::default();
    let mut attempts = Vec::new();

    for _ in 0..2 {
        let mut recorder = Recorder::default();
        {
            let mut processor = CommandProcessor::new(
                &mut command,
                &mut runtime,
                universe.command_context(),
                CommandHostContext::new(&mut capabilities),
            )
            .with_observer(&mut recorder);
            assert_eq!(processor.scan_integer().expect("expression scans").value, 5);
            assert!(matches!(
                processor
                    .get_x_token()
                    .expect("terminator delivers")
                    .expect("terminator exists")
                    .meaning(),
                Meaning::CharToken { ch: 'X', .. }
            ));
        }
        assert_eq!(
            scanner_kinds(&recorder),
            vec!["integer", "integer", "expression_integer", "integer"]
        );
        attempts.push(recorder.0);
        command
            .rollback(snapshot.clone())
            .expect("checkpoint retry rolls back");
    }

    assert_eq!(attempts[0], attempts[1]);
}

#[test]
fn expression_primitives_return_before_the_generic_internal_observation() {
    for (primitive_kind, source, expected_expression, expected_outer) in [
        (P::NumExpr, "2", "expression_integer", "integer"),
        (P::DimExpr, "3pt", "expression_dimension", "dimension"),
        (P::GlueExpr, "4pt plus 3fil", "expression_glue", "glue"),
        (P::MuExpr, "5mu minus 1mu", "expression_muglue", "glue"),
    ] {
        let mut universe = crate::test_harness::universe();
        let expression = primitive(&mut universe, "expression", primitive_kind);
        let mut command = CommandState::new(CommandProfile::ETEX26);
        let mut tokens = vec![expression];
        tokens.extend(chars(source));
        push(&mut command, tokens);
        let mut runtime = CommandRuntime::default();
        let mut capabilities = CommandHostCapabilities::default();
        let mut recorder = Recorder::default();
        let mut processor = CommandProcessor::new(
            &mut command,
            &mut runtime,
            universe.command_context(),
            CommandHostContext::new(&mut capabilities),
        )
        .with_observer(&mut recorder);

        match primitive_kind {
            P::NumExpr => {
                processor.scan_integer().expect("integer expression scans");
            }
            P::DimExpr => {
                processor
                    .scan_dimension()
                    .expect("dimension expression scans");
            }
            P::GlueExpr => {
                processor.scan_glue(false).expect("glue expression scans");
            }
            P::MuExpr => {
                processor.scan_glue(true).expect("mu expression scans");
            }
            _ => unreachable!("case table contains only expression primitives"),
        }

        let kinds = scanner_kinds(&recorder);
        assert!(
            kinds.contains(&expected_expression),
            "primitive: {primitive_kind:?}"
        );
        assert_eq!(
            kinds.last(),
            Some(&expected_outer),
            "primitive: {primitive_kind:?}"
        );
        assert!(
            !kinds.contains(&"internal"),
            "primitive: {primitive_kind:?}"
        );
    }
}

#[test]
fn rounded_fraction_handles_signs_ties_zero_divisors_and_bounds() {
    assert_eq!(rounded_fraction(5, 1, 2, INTEGER_LIMIT), Some(3));
    assert_eq!(rounded_fraction(-5, 1, 2, INTEGER_LIMIT), Some(-3));
    assert_eq!(rounded_fraction(5, -1, 2, INTEGER_LIMIT), Some(-3));
    assert_eq!(rounded_fraction(1, 1, 0, INTEGER_LIMIT), None);
    assert_eq!(rounded_fraction(INTEGER_LIMIT, 2, 1, INTEGER_LIMIT), None);
}
