//! Font queries, csname collection, and typed selector operands.

use super::*;

#[test]
fn nested_pdf_font_sizes_use_the_shared_font_selector_lane() {
    crate::test_harness::with_universe(|universe| {
        let pdf_font_size = install_static(
            universe,
            "pdffontsize",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::PdfFontSize),
        );
        let nullfont = install_static(
            universe,
            "nullfont",
            Meaning::Font(tex_state::font::NULL_FONT),
        );
        let depth = 16;
        let mut input = Vec::with_capacity(depth + 2);
        input.extend(std::iter::repeat_n(pdf_font_size, depth));
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
        assert!(!output.is_empty());
        assert!(output.ends_with('X'));
    });
}

#[test]
fn nested_pdf_font_queries_use_the_shared_font_selector_lane() {
    crate::test_harness::with_universe(|universe| {
        let pdf_font_name = install_static(
            universe,
            "pdffontname",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::PdfFontName),
        );
        let pdf_font_object_number = install_static(
            universe,
            "pdffontobjnum",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::PdfFontObjectNumber),
        );
        let nullfont = install_static(
            universe,
            "nullfont",
            Meaning::Font(tex_state::font::NULL_FONT),
        );

        let mut command = CommandState::new(CommandProfile::PDFTEX14029);
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(
            &mut command,
            [
                pdf_font_name,
                nullfont,
                Token::Char {
                    ch: '/',
                    cat: Catcode::Other,
                },
                pdf_font_object_number,
                nullfont,
                Token::Char {
                    ch: 'X',
                    cat: Catcode::Letter,
                },
            ],
        );
        let output = collect_expanded_characters(universe, &mut command);
        let slash = output.find('/').expect("font-name separator");
        assert!(slash > 0, "pdffontname must render a nonempty name");
        assert!(output[slash + 1..].ends_with('X'));
    });
}

#[test]
fn ifvoid_uses_the_shared_integer_lane_for_its_register_index() {
    crate::test_harness::with_universe(|universe| {
        let ifvoid = install_static(
            universe,
            "ifvoid",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfVoid),
        );
        let fi = install_static(
            universe,
            "fi",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Fi),
        );
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(
            &mut command,
            [
                ifvoid,
                Token::Char {
                    ch: '0',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: 'A',
                    cat: Catcode::Letter,
                },
                fi,
                Token::Char {
                    ch: 'X',
                    cat: Catcode::Letter,
                },
            ],
        );
        assert_eq!(collect_expanded_characters(universe, &mut command), "AX");
    });
}

#[test]
fn iffontchar_consumes_font_and_character_on_the_shared_integer_lane() {
    crate::test_harness::with_universe(|universe| {
        let iffontchar = install_static(
            universe,
            "iffontchar",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfFontChar),
        );
        let nullfont = install_static(
            universe,
            "nullfont",
            Meaning::Font(tex_state::font::NULL_FONT),
        );
        let else_token = install_static(
            universe,
            "else",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Else),
        );
        let fi = install_static(
            universe,
            "fi",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Fi),
        );
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(
            &mut command,
            [
                iffontchar,
                nullfont,
                Token::Char {
                    ch: '6',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: '5',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: 'A',
                    cat: Catcode::Letter,
                },
                else_token,
                Token::Char {
                    ch: 'B',
                    cat: Catcode::Letter,
                },
                fi,
                Token::Char {
                    ch: 'X',
                    cat: Catcode::Letter,
                },
            ],
        );
        assert_eq!(collect_expanded_characters(universe, &mut command), "BX");
    });
}

#[test]
fn ifcsname_collects_in_the_shared_delivery_lane() {
    crate::test_harness::with_universe(|universe| {
        let ifcsname = install_static(
            universe,
            "ifcsname",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfCsName),
        );
        let endcsname = install_static(
            universe,
            "endcsname",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::EndCsName),
        );
        let _known = install_static(universe, "known", Meaning::Relax);
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(
            &mut command,
            [
                ifcsname,
                Token::Char {
                    ch: 'k',
                    cat: Catcode::Letter,
                },
                Token::Char {
                    ch: 'n',
                    cat: Catcode::Letter,
                },
                Token::Char {
                    ch: 'o',
                    cat: Catcode::Letter,
                },
                Token::Char {
                    ch: 'w',
                    cat: Catcode::Letter,
                },
                Token::Char {
                    ch: 'n',
                    cat: Catcode::Letter,
                },
                endcsname,
                Token::Char {
                    ch: 'T',
                    cat: Catcode::Letter,
                },
                Token::Char {
                    ch: 'F',
                    cat: Catcode::Letter,
                },
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
        let ownership_before = crate::command::command_ownership_counters();
        let result = crate::test_harness::expect_expanded_command(&mut processor);
        let ownership_after = crate::command::command_ownership_counters();
        assert_eq!(
            result.spelling().semantic_token(),
            Token::Char {
                ch: 'T',
                cat: Catcode::Letter,
            }
        );
        assert_eq!(
            ownership_after.rich_materializations - ownership_before.rich_materializations,
            1,
            "only the outward selected branch materializes a rich command",
        );
        drop(processor);
    });
}

#[test]
fn csname_chardef_uses_missing_endcsname_recovery() {
    crate::test_harness::with_universe(|universe| {
        let csname = install_static(
            universe,
            "csname",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::CsName),
        );
        let endcsname = install_static(
            universe,
            "endcsname",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::EndCsName),
        );
        let chardef = universe.intern("csname-chardef").expect("chardef name");
        universe
            .assign_meaning(
                chardef,
                MeaningWord::from_static(Meaning::CharGiven('A')),
                AssignmentScope::Global,
            )
            .expect("chardef meaning");
        let empty = universe.intern("").expect("empty control sequence");
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(
            &mut command,
            [
                csname,
                Token::Cs(chardef.symbol()),
                endcsname,
                Token::Char {
                    ch: 'X',
                    cat: Catcode::Letter,
                },
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

        let result = crate::test_harness::expect_expanded_command(&mut processor);
        assert_eq!(
            result.spelling().semantic_token(),
            Token::Cs(empty.symbol())
        );
        assert_eq!(result.meaning(), Meaning::Relax);
        drop(processor);
        let diagnostics = command.take_semantic_diagnostics();
        assert!(matches!(
            diagnostics.as_slice(),
            [crate::CommandSemanticDiagnostic::Recoverable { message, .. }]
                if message.ends_with("endcsname inserted")
        ));
    });
}

#[test]
fn ifcsname_chardef_uses_missing_endcsname_recovery() {
    crate::test_harness::with_universe(|universe| {
        let ifcsname = install_static(
            universe,
            "ifcsname",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::IfCsName),
        );
        let endcsname = install_static(
            universe,
            "endcsname",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::EndCsName),
        );
        let chardef = universe.intern("ifcsname-chardef").expect("chardef name");
        universe
            .assign_meaning(
                chardef,
                MeaningWord::from_static(Meaning::CharGiven('A')),
                AssignmentScope::Global,
            )
            .expect("chardef meaning");
        let else_symbol = universe.intern("else").expect("else name");
        let fi_symbol = universe.intern("fi").expect("fi name");
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(
            &mut command,
            [
                ifcsname,
                Token::Cs(chardef.symbol()),
                endcsname,
                Token::Char {
                    ch: 'F',
                    cat: Catcode::Letter,
                },
                Token::Cs(else_symbol.symbol()),
                Token::Char {
                    ch: 'T',
                    cat: Catcode::Letter,
                },
                Token::Cs(fi_symbol.symbol()),
            ],
        );
        universe
            .assign_meaning(
                else_symbol,
                MeaningWord::from_static(Meaning::ExpandablePrimitive(ExpandablePrimitive::Else)),
                AssignmentScope::Global,
            )
            .expect("else meaning");
        universe
            .assign_meaning(
                fi_symbol,
                MeaningWord::from_static(Meaning::ExpandablePrimitive(ExpandablePrimitive::Fi)),
                AssignmentScope::Global,
            )
            .expect("fi meaning");
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

        let result = crate::test_harness::expect_expanded_command(&mut processor);
        assert_eq!(
            result.spelling().semantic_token(),
            Token::Char {
                ch: 'T',
                cat: Catcode::Letter,
            }
        );
        drop(processor);
        let diagnostics = command.take_semantic_diagnostics();
        assert!(matches!(
            diagnostics.as_slice(),
            [crate::CommandSemanticDiagnostic::Recoverable { message, .. }]
                if message.ends_with("endcsname inserted")
        ));
    });
}
