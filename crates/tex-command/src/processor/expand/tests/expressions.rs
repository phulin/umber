//! Integer and dimension expression scanning and evaluation.

use super::*;

#[test]
fn the_direct_internal_meanings_use_the_hot_value_projection() {
    crate::test_harness::with_universe(|universe| {
        let the = install_static(
            universe,
            "the",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::The),
        );
        let count_alias = install_static(universe, "countalias", Meaning::CountRegister(0));
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(
            &mut command,
            [
                the,
                count_alias,
                Token::Char {
                    ch: 'X',
                    cat: Catcode::Letter,
                },
            ],
        );
        assert_eq!(collect_expanded_characters(universe, &mut command), "0X");
    });
}

#[test]
fn the_integer_expression_lane_preserves_operator_precedence() {
    crate::test_harness::with_universe(|universe| {
        let the = install_static(
            universe,
            "the",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::The),
        );
        let numexpr = install_static(
            universe,
            "numexpr",
            Meaning::UnexpandablePrimitive(tex_state::meaning::UnexpandablePrimitive::NumExpr),
        );
        let relax = install_static(universe, "relax", Meaning::Relax);
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(
            &mut command,
            [
                the,
                numexpr,
                Token::Char {
                    ch: '1',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: '+',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: '2',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: '*',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: '3',
                    cat: Catcode::Other,
                },
                relax,
                Token::Char {
                    ch: 'X',
                    cat: Catcode::Letter,
                },
            ],
        );
        assert_eq!(collect_expanded_characters(universe, &mut command), "7X");
    });
}

#[test]
fn the_integer_and_dimension_expressions_accept_parenthesized_factors() {
    crate::test_harness::with_universe(|universe| {
        let the = install_static(
            universe,
            "the",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::The),
        );
        let numexpr = install_static(
            universe,
            "numexpr",
            Meaning::UnexpandablePrimitive(tex_state::meaning::UnexpandablePrimitive::NumExpr),
        );
        let dimexpr = install_static(
            universe,
            "dimexpr",
            Meaning::UnexpandablePrimitive(tex_state::meaning::UnexpandablePrimitive::DimExpr),
        );
        let relax = install_static(universe, "relax", Meaning::Relax);
        let time = install_static(universe, "time", Meaning::IntParam(IntParam::TIME.raw()));
        universe
            .assign_int_param(IntParam::TIME, 23, AssignmentScope::Global)
            .expect("time parameter assignment");
        let grouped_definition = universe
            .allocate_definition(
                &[],
                &[
                    TokenWord::pack(other('(')),
                    TokenWord::pack(other('1')),
                    TokenWord::pack(other('+')),
                    TokenWord::pack(other('2')),
                    TokenWord::pack(other(')')),
                ],
            )
            .expect("grouped expression definition");
        let grouped = universe
            .intern("grouped")
            .expect("grouped expression macro");
        universe
            .assign_meaning(
                grouped,
                MeaningWord::macro_definition(MeaningFlags::EMPTY, grouped_definition),
                AssignmentScope::Global,
            )
            .expect("grouped expression meaning");
        let integer_input = [
            the,
            numexpr,
            other('('),
            other('1'),
            other('+'),
            other('('),
            other('2'),
            other('*'),
            other('3'),
            other(')'),
            other(')'),
            relax,
            letter('X'),
        ];
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(&mut command, integer_input);
        assert_eq!(collect_expanded_characters(universe, &mut command), "7X");

        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(
            &mut command,
            [
                the,
                numexpr,
                Token::Cs(grouped.symbol()),
                relax,
                letter('X'),
            ],
        );
        assert_eq!(collect_expanded_characters(universe, &mut command), "3X");

        // etex.ch [53a], scan_expr checks for '(' before scan_int's
        // sign scan. A signed subexpression must use subtraction, e.g.
        // 0-(1+2), rather than the noncanonical -(1+2).
        let integer_prefix_input = [
            the,
            numexpr,
            other('1'),
            other('+'),
            other('('),
            other('2'),
            other('*'),
            other('3'),
            other(')'),
            relax,
            letter('X'),
        ];
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(&mut command, integer_prefix_input);
        assert_eq!(collect_expanded_characters(universe, &mut command), "7X");

        let dimension_input = [
            the,
            dimexpr,
            other('('),
            other('1'),
            other('p'),
            other('t'),
            other('+'),
            other('('),
            other('2'),
            other('p'),
            other('t'),
            other(')'),
            other(')'),
            relax,
            letter('X'),
        ];
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(&mut command, dimension_input);
        assert_eq!(
            collect_expanded_characters(universe, &mut command),
            "3.0ptX"
        );

        let dimension_prefix_input = [
            the,
            dimexpr,
            other('1'),
            other('p'),
            other('t'),
            other('*'),
            other('('),
            other('2'),
            other(')'),
            relax,
            letter('X'),
        ];
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(&mut command, dimension_prefix_input);
        assert_eq!(
            collect_expanded_characters(universe, &mut command),
            "2.0ptX"
        );

        let internal_input = [
            the,
            numexpr,
            other('('),
            time,
            other('+'),
            other('1'),
            other(')'),
            relax,
            letter('X'),
        ];
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(&mut command, internal_input);
        assert_eq!(collect_expanded_characters(universe, &mut command), "24X");

        let unmatched_input = [
            the,
            numexpr,
            other('('),
            other('1'),
            other('+'),
            other('2'),
            relax,
            letter('X'),
        ];
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(&mut command, unmatched_input);
        assert_eq!(collect_expanded_characters(universe, &mut command), "3X");

        let invalid_input = [
            the,
            numexpr,
            other('('),
            other('1'),
            other(','),
            other('2'),
            letter('X'),
        ];
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(&mut command, invalid_input);
        assert_eq!(collect_expanded_characters(universe, &mut command), "1,2X");
    });
}

#[test]
fn number_integer_expression_uses_the_shared_expression_lane() {
    crate::test_harness::with_universe(|universe| {
        let number = install_static(
            universe,
            "number",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Number),
        );
        let numexpr = install_static(
            universe,
            "numexpr",
            Meaning::UnexpandablePrimitive(tex_state::meaning::UnexpandablePrimitive::NumExpr),
        );
        let relax = install_static(universe, "relax", Meaning::Relax);
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(
            &mut command,
            [
                number,
                numexpr,
                Token::Char {
                    ch: '1',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: '+',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: '2',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: '*',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: '3',
                    cat: Catcode::Other,
                },
                relax,
                Token::Char {
                    ch: 'X',
                    cat: Catcode::Letter,
                },
            ],
        );
        assert_eq!(collect_expanded_characters(universe, &mut command), "7X");
    });
}

#[test]
fn integer_expression_factors_admit_typed_internal_values() {
    crate::test_harness::with_universe(|universe| {
        let the = install_static(
            universe,
            "the",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::The),
        );
        let numexpr = install_static(
            universe,
            "numexpr",
            Meaning::UnexpandablePrimitive(tex_state::meaning::UnexpandablePrimitive::NumExpr),
        );
        let count = install_static(
            universe,
            "count",
            Meaning::UnexpandablePrimitive(tex_state::meaning::UnexpandablePrimitive::Count),
        );
        let time = install_static(universe, "time", Meaning::IntParam(IntParam::TIME.raw()));
        let year = install_static(universe, "year", Meaning::IntParam(IntParam::YEAR.raw()));
        let relax = install_static(universe, "relax", Meaning::Relax);
        universe
            .assign_int_param(IntParam::TIME, 23, AssignmentScope::Global)
            .expect("time parameter assignment");
        universe
            .assign_int_param(IntParam::YEAR, 2026, AssignmentScope::Global)
            .expect("year parameter assignment");
        universe
            .assign_count(0, 17, AssignmentScope::Global)
            .expect("count register assignment");
        let space = Token::Char {
            ch: ' ',
            cat: Catcode::Space,
        };

        let cases = [
            (vec![the, numexpr, time, relax, letter('X')], "23X"),
            (
                vec![
                    the,
                    numexpr,
                    count,
                    other('0'),
                    other('+'),
                    other('1'),
                    relax,
                    letter('X'),
                ],
                "18X",
            ),
            (vec![the, numexpr, year, relax, letter('X')], "2026X"),
            (
                vec![
                    the,
                    numexpr,
                    time,
                    space,
                    other('+'),
                    space,
                    other('1'),
                    relax,
                    letter('X'),
                ],
                "24X",
            ),
            (
                vec![
                    the,
                    numexpr,
                    other('1'),
                    space,
                    other('2'),
                    other('+'),
                    other('3'),
                    letter('X'),
                ],
                "12+3X",
            ),
            (
                vec![
                    the,
                    numexpr,
                    other('-'),
                    other('-'),
                    time,
                    other('*'),
                    other('2'),
                    other('+'),
                    other('-'),
                    count,
                    other('0'),
                    relax,
                    letter('X'),
                ],
                "29X",
            ),
            (vec![the, numexpr, letter('X')], "0X"),
        ];
        for (input, expected) in cases {
            let mut command = CommandState::default();
            let _operation = command.begin_attempt_operation();
            crate::test_harness::push(&mut command, input);
            assert_eq!(
                collect_expanded_characters(universe, &mut command),
                expected
            );
        }
    });
}

#[test]
fn number_dimension_expression_uses_the_shared_dimension_lane() {
    crate::test_harness::with_universe(|universe| {
        let number = install_static(
            universe,
            "number",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Number),
        );
        let dimexpr = install_static(
            universe,
            "dimexpr",
            Meaning::UnexpandablePrimitive(tex_state::meaning::UnexpandablePrimitive::DimExpr),
        );
        let relax = install_static(universe, "relax", Meaning::Relax);
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(
            &mut command,
            [
                number,
                dimexpr,
                Token::Char {
                    ch: '1',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: 'p',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: 't',
                    cat: Catcode::Other,
                },
                relax,
                Token::Char {
                    ch: 'X',
                    cat: Catcode::Letter,
                },
            ],
        );
        assert_eq!(
            collect_expanded_characters(universe, &mut command),
            "65536X"
        );
    });
}

#[test]
fn the_dimension_expression_lane_preserves_fixed_point_addition() {
    crate::test_harness::with_universe(|universe| {
        let the = install_static(
            universe,
            "the",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::The),
        );
        let dimexpr = install_static(
            universe,
            "dimexpr",
            Meaning::UnexpandablePrimitive(tex_state::meaning::UnexpandablePrimitive::DimExpr),
        );
        let relax = install_static(universe, "relax", Meaning::Relax);
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(
            &mut command,
            [
                the,
                dimexpr,
                Token::Char {
                    ch: '1',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: 'p',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: 't',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: '+',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: '2',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: 'p',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: 't',
                    cat: Catcode::Other,
                },
                relax,
                Token::Char {
                    ch: 'X',
                    cat: Catcode::Letter,
                },
            ],
        );
        assert_eq!(
            collect_expanded_characters(universe, &mut command),
            "3.0ptX"
        );
    });
}

#[test]
fn nested_fontname_operands_use_the_shared_control_lane() {
    crate::test_harness::with_universe(|universe| {
        let fontname = install_static(
            universe,
            "fontname",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::FontName),
        );
        let nullfont = install_static(
            universe,
            "nullfont",
            Meaning::Font(tex_state::font::NULL_FONT),
        );
        // Only the innermost selector can be valid: each enclosing
        // `\fontname` sees the first rendered character of its child and
        // therefore exercises TeX's missing-identifier recovery. Keep the
        // chain below the engine's fatal-error threshold while still
        // traversing the compact control lane repeatedly.
        let depth = 16;
        let mut input = Vec::with_capacity(depth + 2);
        input.extend(std::iter::repeat_n(fontname, depth));
        input.extend([
            nullfont,
            Token::Char {
                ch: 'X',
                cat: Catcode::Letter,
            },
        ]);
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(&mut command, input);
        let output = collect_expanded_characters(universe, &mut command);
        assert!(output.starts_with("nullfont"));
        assert!(output.ends_with('X'));
    });
}
