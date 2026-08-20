use std::sync::Arc;

use tex_state::Universe;
use tex_state::macro_store::MacroMeaning;
use tex_state::meaning::UnexpandablePrimitive;

use super::*;
use crate::test_harness::{Recorder, diagnostic_text, processor, push, traced};
use crate::{
    CommandDeliveryBoundary, CommandHostCapabilities, CommandHostContext, CommandObservation,
    CommandState, InputTransition, ObservedToken, RegisteredSourceKind, SourceRegistration,
};

#[test]
fn scan_toks_modes_parse_into_semantic_configurations() {
    let owner = tex_state::interner::Symbol::testing_new(41);
    let primary = tex_state::token::OriginId::UNKNOWN;

    assert_eq!(
        ScanToksConfig::parse(ScanToksMode::GeneralAfterOpening {
            expanded: true,
            primary,
            owner: Some(owner),
        }),
        ScanToksConfig {
            grammar: ScanToksGrammar::General,
            opening: ScanToksOpening::Prevalidated { primary },
            expansion: ScanToksExpansion::Expanded,
            owner: ScanToksOwner::Absorbed(Some(owner)),
            purpose: ScanToksPurpose::ExpandedBalanced,
            status_visibility: ScannerStatusVisibility::Observed,
        }
    );
    assert_eq!(
        ScanToksConfig::parse(ScanToksMode::GeneralText {
            purpose: "detokenize",
        }),
        ScanToksConfig {
            grammar: ScanToksGrammar::General,
            opening: ScanToksOpening::Required,
            expansion: ScanToksExpansion::Unexpanded,
            owner: ScanToksOwner::Absorbed(None),
            purpose: ScanToksPurpose::GeneralText("detokenize"),
            status_visibility: ScannerStatusVisibility::Hidden,
        }
    );
    assert_eq!(
        ScanToksConfig::parse(ScanToksMode::MacroDefinitionFor {
            expanded: false,
            target: owner,
        }),
        ScanToksConfig {
            grammar: ScanToksGrammar::MacroDefinition,
            opening: ScanToksOpening::AfterParameterText,
            expansion: ScanToksExpansion::Unexpanded,
            owner: ScanToksOwner::Definition(Some(owner)),
            purpose: ScanToksPurpose::MacroReplacement,
            status_visibility: ScannerStatusVisibility::Observed,
        }
    );
}

#[test]
fn scratch_pool_warmth_preserves_scan_semantics_and_publications() {
    fn run(
        warm_scratch_pool: bool,
    ) -> (
        Vec<Token>,
        Vec<Token>,
        crate::CommandStateSnapshot,
        crate::CommandSummary,
        Vec<CommandObservation>,
        String,
    ) {
        let mut command = CommandState::default();
        push(
            &mut command,
            vec![
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
                    ch: 'a',
                    cat: Catcode::Letter,
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
        if warm_scratch_pool {
            let mut first = crate::state::traced_token_scratch();
            first.extend_unowned([traced(Token::Param(1)); 32]);
            let mut second = crate::state::traced_token_scratch();
            second.extend_unowned([traced(Token::Param(1)); 16]);
        }
        let mut universe = crate::test_harness::universe_with_plain_catcodes();
        let mut capabilities = CommandHostCapabilities::default();
        let mut recorder = Recorder::default();
        let (parameters, replacement) = {
            let mut processor = processor(&mut command, &mut universe, &mut capabilities)
                .with_observer(&mut recorder);
            let scanned = processor
                .scan_toks(ScanToksMode::MacroDefinition { expanded: false })
                .expect("macro definition scans");
            (
                processor
                    .state
                    .tokens(scanned.parameter_text.token_list())
                    .to_vec(),
                processor
                    .state
                    .tokens(scanned.replacement_text.token_list())
                    .to_vec(),
            )
        };
        let summary = command
            .publish_summary()
            .expect("completed scan is quiescent");
        (
            parameters,
            replacement,
            command.snapshot(),
            summary,
            recorder.0,
            diagnostic_text(&universe),
        )
    }

    assert_eq!(run(false), run(true));
}

#[test]
fn origin_list_budget_fallback_preserves_section_478_splice_semantics() {
    // TeX82 §478 and e-TeX 2.6 change [27.465] splice the complete semantic
    // token list. Provenance storage is diagnostic and deliberately degrades
    // a saturated origin-list arena to EMPTY, so reconstruction must use the
    // same indexed UNKNOWN fallback as ordinary stored input rather than zip
    // the nonempty token list with an empty origin projection.
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let tokens = [
        Token::Char {
            ch: '1',
            cat: Catcode::Other,
        },
        Token::Char {
            ch: '3',
            cat: Catcode::Other,
        },
        Token::Char {
            ch: '9',
            cat: Catcode::Other,
        },
        Token::Char {
            ch: '0',
            cat: Catcode::Other,
        },
    ];
    let token_list = universe.intern_token_list_ref(&tokens);
    let list = TracedTokenList::synthetic(token_list);
    let mut capabilities = CommandHostCapabilities::default();
    let processor = processor(&mut command, &mut universe, &mut capabilities);

    let words = processor.rooted_words(list);

    assert_eq!(
        words
            .iter()
            .map(|word| word.word().semantic_token())
            .collect::<Vec<_>>(),
        tokens
    );
    assert!(
        words
            .iter()
            .all(|word| word.word().origin() == OriginId::UNKNOWN)
    );
}

#[test]
fn general_scan_toks_continues_after_section_403_inserted_left_brace() {
    let mut command = CommandState::default();
    push(
        &mut command,
        vec![
            Token::Char {
                ch: 'x',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
        ],
    );
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        let scanned = processor
            .scan_toks(ScanToksMode::General { expanded: false })
            .expect("§403 recovery supplies the required opening brace");
        assert_eq!(
            processor
                .state
                .tokens(scanned.replacement_text.token_list()),
            &[Token::Char {
                ch: 'x',
                cat: Catcode::Letter,
            }]
        );
        assert_eq!(
            processor.command.alignment.align_state,
            crate::processor::TOP_LEVEL_ALIGN_STATE
        );
    }
    assert!(diagnostic_text(&universe).starts_with("! Missing { inserted."));
}

#[test]
fn macro_definition_right_brace_reports_missing_left_brace_and_finishes() {
    let mut command = CommandState::default();
    push(
        &mut command,
        vec![
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
            Token::Char {
                ch: 'x',
                cat: Catcode::Letter,
            },
        ],
    );
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        let scanned = processor
            .scan_toks(ScanToksMode::MacroDefinition { expanded: false })
            .expect("TeX82 §§475--476 finish the recovered empty definition");
        assert!(
            processor
                .state
                .tokens(scanned.parameter_text.token_list())
                .is_empty()
        );
        assert!(
            processor
                .state
                .tokens(scanned.replacement_text.token_list())
                .is_empty()
        );
        assert_eq!(
            processor.command.alignment.align_state,
            crate::processor::TOP_LEVEL_ALIGN_STATE
        );
        assert_eq!(
            processor
                .get_token()
                .expect("following token delivers")
                .expect("following token remains unread")
                .spelling()
                .semantic_token(),
            Token::Char {
                ch: 'x',
                cat: Catcode::Letter,
            }
        );
    }
    let diagnostic = diagnostic_text(&universe);
    assert!(diagnostic.starts_with("! Missing { inserted."));
    assert!(diagnostic.contains("Where was the left brace?"));
}

#[test]
fn general_after_opening_replays_a_begin_group_alias_by_meaning() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let open = universe.intern("open").symbol();
    universe.set_meaning(
        open,
        Meaning::CharToken {
            ch: '{',
            cat: Catcode::BeginGroup,
        },
    );
    push(
        &mut command,
        vec![
            Token::Cs(open),
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
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);
    let opening = processor
        .get_x_token()
        .expect("expanded delivery succeeds")
        .expect("opening alias is present");
    assert!(matches!(
        opening.meaning(),
        Meaning::CharToken {
            cat: Catcode::BeginGroup,
            ..
        }
    ));
    let primary = opening.origin();
    processor
        .back_input(opening)
        .expect("opening alias is backed up for absorbing replay");

    let scanned = processor
        .scan_toks(ScanToksMode::GeneralAfterOpening {
            expanded: false,
            primary,
            owner: None,
        })
        .expect("the backed-up semantic begin-group starts collection");
    assert_eq!(
        processor
            .state
            .tokens(scanned.replacement_text.token_list()),
        &[Token::Char {
            ch: 'a',
            cat: Catcode::Letter,
        }]
    );
}

#[test]
fn eof_recovery_restores_defining_status_before_macro_replacement_completes() {
    let mut command = CommandState::default();
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(&b"{DEF"[..]),
        ))
        .expect("source registers");
    command
        .open_registered_source(source)
        .expect("source opens");
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    let mut processor = CommandProcessor::new(
        &mut command,
        universe.command_context(),
        CommandHostContext::new(&mut capabilities),
    )
    .with_observer(&mut recorder);

    processor
        .scan_toks(ScanToksMode::MacroDefinition { expanded: false })
        .expect("EOF recovery closes the replacement text");
    let diagnostics = processor.take_semantic_diagnostics();
    let [
        crate::CommandSemanticDiagnostic::Recoverable {
            runaway: Some(runaway),
            ..
        },
    ] = diagnostics.as_slice()
    else {
        panic!("expected runaway-definition diagnostic")
    };
    assert_eq!(runaway.partial, "->DEF ");

    let close = recorder
        .0
        .iter()
        .position(|event| {
            matches!(event, CommandObservation::Command(command)
            if matches!(command.spelling, ObservedToken::Character {
                character: '}',
                catcode: Catcode::EndGroup,
            }))
        })
        .expect("inserted right brace is delivered");
    let restored = recorder
        .0
        .iter()
        .position(|event| {
            matches!(event, CommandObservation::ScannerStatus(status)
                if status.from == "defining" && status.to == "normal")
        })
        .expect("defining status restores after the inserted right brace");
    assert!(close < restored);
}

fn install_expandable(
    universe: &mut Universe,
    name: &str,
    primitive: ExpandablePrimitive,
) -> tex_state::interner::Symbol {
    let symbol = universe.intern(name).symbol();
    universe.set_meaning(symbol, Meaning::ExpandablePrimitive(primitive));
    symbol
}

fn etex_boundary_tokens(universe: &mut Universe) -> Vec<Token> {
    let control_word = universe.intern("word").symbol();
    let control_space = universe.intern(" ").symbol();
    let active = universe.intern_active_character('~').symbol();
    vec![
        Token::Cs(control_word),
        Token::Cs(control_space),
        Token::Cs(active),
        Token::Char {
            ch: '{',
            cat: Catcode::BeginGroup,
        },
        Token::Char {
            ch: '}',
            cat: Catcode::EndGroup,
        },
        Token::Char {
            ch: '$',
            cat: Catcode::MathShift,
        },
        Token::Char {
            ch: '&',
            cat: Catcode::AlignmentTab,
        },
        Token::Char {
            ch: '\r',
            cat: Catcode::EndLine,
        },
        Token::Char {
            ch: '#',
            cat: Catcode::Parameter,
        },
        Token::Char {
            ch: '^',
            cat: Catcode::Superscript,
        },
        Token::Char {
            ch: '_',
            cat: Catcode::Subscript,
        },
        Token::Char {
            ch: ' ',
            cat: Catcode::Space,
        },
        Token::Char {
            ch: 'L',
            cat: Catcode::Letter,
        },
        Token::Char {
            ch: 'o',
            cat: Catcode::Other,
        },
    ]
}

#[test]
fn etex_unexpanded_preserves_empty_and_full_boundary_token_lists() {
    // e-TeX 2.6 etex.ch [17.3623--3660, 27.465]: scan_general_text uses
    // unexpanded get_token collection and the_toks attaches the resulting
    // list directly. This includes empty text, control space, active control
    // sequences, parameters, and every category that can exist as a delivered
    // character token; ignored/comment/invalid source characters are covered
    // separately because TeX never makes tokens for them.
    for full_matrix in [false, true] {
        let mut command = CommandState::new(crate::CommandProfile::ETEX26);
        let mut universe = crate::test_harness::universe_with_plain_catcodes();
        let unexpanded =
            install_expandable(&mut universe, "unexpanded", ExpandablePrimitive::Unexpanded);
        let expected = if full_matrix {
            etex_boundary_tokens(&mut universe)
        } else {
            Vec::new()
        };
        let mut input = vec![Token::Char {
            ch: '{',
            cat: Catcode::BeginGroup,
        }];
        input.push(Token::Cs(unexpanded));
        input.push(Token::Char {
            ch: '{',
            cat: Catcode::BeginGroup,
        });
        input.extend(expected.iter().copied());
        input.extend([
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
        ]);
        push(&mut command, input);
        let mut capabilities = CommandHostCapabilities::default();
        let scanned = processor(&mut command, &mut universe, &mut capabilities)
            .scan_toks(ScanToksMode::General { expanded: true })
            .expect("expanded collector accepts the unexpanded boundary list");

        assert_eq!(
            universe
                .tokens(scanned.replacement_text.token_list())
                .tokens(),
            expected,
            "unexpanded must preserve exact token identity and category"
        );
        assert_eq!(command.scanner.status(), &ScannerStatus::Normal);
        assert_eq!(
            command.alignment.align_state,
            crate::processor::TOP_LEVEL_ALIGN_STATE
        );
    }
}

#[test]
fn etex_detokenize_projects_full_boundary_matrix_with_exact_spacing() {
    // e-TeX 2.6 etex.ch [17.3623--3660, 53a]: token_show doubles parameter
    // characters and supplies control-word separators; str_toks then makes
    // exactly category-10 spaces and category-12 other characters.
    let mut command = CommandState::new(crate::CommandProfile::ETEX26);
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let detokenize =
        install_expandable(&mut universe, "detokenize", ExpandablePrimitive::Detokenize);
    let body = etex_boundary_tokens(&mut universe);
    let mut input = vec![
        Token::Char {
            ch: '{',
            cat: Catcode::BeginGroup,
        },
        Token::Cs(detokenize),
        Token::Char {
            ch: '{',
            cat: Catcode::BeginGroup,
        },
    ];
    input.extend(body);
    input.extend([
        Token::Char {
            ch: '}',
            cat: Catcode::EndGroup,
        },
        Token::Char {
            ch: '}',
            cat: Catcode::EndGroup,
        },
    ]);
    push(&mut command, input);
    let mut capabilities = CommandHostCapabilities::default();
    let scanned = processor(&mut command, &mut universe, &mut capabilities)
        .scan_toks(ScanToksMode::General { expanded: true })
        .expect("detokenize boundary matrix completes");
    let output = universe.tokens(scanned.replacement_text.token_list());
    let text = output
        .iter()
        .map(|token| match token {
            Token::Char { ch, .. } => *ch,
            _ => panic!("detokenize returned a non-character token"),
        })
        .collect::<String>();

    assert_eq!(text, "\\word \\ ~{}$&\r##^_ Lo");
    assert!(output.iter().all(|token| matches!(
        token,
        Token::Char {
            ch: ' ',
            cat: Catcode::Space
        } | Token::Char {
            cat: Catcode::Other,
            ..
        }
    )));
    assert_eq!(command.scanner.status(), &ScannerStatus::Normal);
    assert_eq!(
        command.alignment.align_state,
        crate::processor::TOP_LEVEL_ALIGN_STATE
    );
}

#[test]
fn etex_unexpanded_and_detokenize_discard_nontoken_categories() {
    // e-TeX 2.6 etex.ch [17.3623--3660, 27.465, 53a] inherits TeX82
    // §§346--348 source tokenization: ignored and invalid characters make no
    // token, while a comment discards the rest of its physical line. The next
    // line starts in new_line state rather than contributing a space.
    let mut command = CommandState::new(crate::CommandProfile::ETEX26);
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(&b"{\\unexpanded{A?B%gone\nC!D}\\detokenize{E?F%gone\nG!H}}"[..]),
        ))
        .expect("boundary source registers");
    command
        .open_registered_source(source)
        .expect("boundary source opens");
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    universe.set_catcode('?', Catcode::Ignored);
    universe.set_catcode('!', Catcode::Invalid);
    install_expandable(&mut universe, "unexpanded", ExpandablePrimitive::Unexpanded);
    install_expandable(&mut universe, "detokenize", ExpandablePrimitive::Detokenize);
    let mut capabilities = CommandHostCapabilities::default();
    let scanned = processor(&mut command, &mut universe, &mut capabilities)
        .scan_toks(ScanToksMode::General { expanded: true })
        .expect("comment boundary source scans");

    assert_eq!(
        universe.tokens(scanned.replacement_text.token_list()),
        &[
            Token::Char {
                ch: 'A',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: 'B',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: 'C',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: 'D',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: 'E',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: 'F',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: 'G',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: 'H',
                cat: Catcode::Other,
            },
        ]
    );
    assert_eq!(command.scanner.status(), &ScannerStatus::Normal);
    assert_eq!(
        command.alignment.align_state,
        crate::processor::TOP_LEVEL_ALIGN_STATE
    );
}

#[test]
fn direct_the_toks_splice_is_unexpanded_and_does_not_balance_the_collector() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let the = install_expandable(&mut universe, "the", ExpandablePrimitive::The);
    let macro_symbol = universe.intern("storedmacro").symbol();
    let register = universe.intern("stored").symbol();
    universe.set_meaning(register, Meaning::ToksRegister(3));
    let stored = universe.intern_token_list(&[
        Token::Char {
            ch: '{',
            cat: Catcode::BeginGroup,
        },
        Token::Cs(macro_symbol),
    ]);
    universe.set_toks(3, stored);
    push(
        &mut command,
        vec![
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Cs(the),
            Token::Cs(register),
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
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);

    let scanned = processor
        .scan_toks(ScanToksMode::General { expanded: true })
        .expect("scan succeeds");
    assert_eq!(
        processor
            .state
            .tokens(scanned.replacement_text.token_list()),
        &[
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Cs(macro_symbol)
        ]
    );
    assert_eq!(
        processor
            .get_next()
            .expect("trailing token delivers")
            .expect("trailing token exists")
            .spelling()
            .semantic_token(),
        Token::Char {
            ch: 'z',
            cat: Catcode::Letter,
        }
    );
}

#[test]
fn unexpanded_expands_scan_general_text_opener_before_copying_raw_body() {
    // e-TeX 2.6 etex.ch [27.465] implements `\unexpanded` through
    // `scan_general_text`. Its opener uses §403's expanded fetch, so the
    // e-TRIP idiom `\unexpanded\expandafter{...}` reaches the brace before
    // switching to raw balanced-text collection.
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let unexpanded =
        install_expandable(&mut universe, "unexpanded", ExpandablePrimitive::Unexpanded);
    let expandafter = install_expandable(
        &mut universe,
        "expandafter",
        ExpandablePrimitive::ExpandAfter,
    );
    push(
        &mut command,
        vec![
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Cs(unexpanded),
            Token::Cs(expandafter),
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Char {
                ch: 'X',
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
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);

    let scanned = processor
        .scan_toks(ScanToksMode::General { expanded: true })
        .expect("expanded opener reaches the raw balanced text");
    assert_eq!(
        processor
            .state
            .tokens(scanned.replacement_text.token_list()),
        &[Token::Char {
            ch: 'X',
            cat: Catcode::Letter,
        }]
    );
}

#[test]
fn unexpanded_observes_the_completed_raw_balanced_text_before_its_direct_splice() {
    // e-TeX 2.6 etex.ch [17.3623--3699, 27.465] makes `scan_general_text`
    // construct the raw balanced list before `the_toks` returns it to the
    // enclosing expanded `scan_toks` collector.
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let unexpanded =
        install_expandable(&mut universe, "unexpanded", ExpandablePrimitive::Unexpanded);
    let raw = universe.intern("raw").symbol();
    push(
        &mut command,
        vec![
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Cs(unexpanded),
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Cs(raw),
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
    let mut recorder = Recorder::default();
    CommandProcessor::new(
        &mut command,
        universe.command_context(),
        CommandHostContext::new(&mut capabilities),
    )
    .with_observer(&mut recorder)
    .scan_toks(ScanToksMode::General { expanded: true })
    .expect("expanded token-list scan completes");

    let unexpanded = recorder
        .0
        .iter()
        .position(|event| {
            matches!(
                event,
                CommandObservation::TokenList(record)
                    if record.transition == "complete"
                        && record.purpose == "unexpanded"
                        && record.tokens == [ObservedToken::ControlSequence("raw".into())]
            )
        })
        .expect("raw balanced text completion is observed");
    let enclosing = recorder
        .0
        .iter()
        .position(|event| {
            matches!(
                event,
                CommandObservation::TokenList(record)
                    if record.transition == "complete"
                        && record.purpose == "expanded_scan_toks"
            )
        })
        .expect("enclosing expanded scan completion is observed");
    let splice = recorder
        .0
        .iter()
        .position(|event| {
            matches!(
                event,
                CommandObservation::TokenList(record)
                    if record.transition == "splice"
                        && record.purpose == "the_toks"
                        && record.tokens == [ObservedToken::ControlSequence("raw".into())]
            )
        })
        .expect("raw balanced text is observed at the_toks attachment");
    assert!(unexpanded < splice && splice < enclosing);
    assert_eq!(
        recorder
            .0
            .iter()
            .filter(|event| matches!(event, CommandObservation::ScannerStatus(_)))
            .count(),
        2,
        "only the enclosing TeX82 scan_toks entry and exit are observed"
    );
}

#[test]
fn unexpanded_general_text_recovers_runaway_input_and_restores_outer_scanner_state() {
    // e-TeX 2.6 etex.ch §53a saves scanner_status, warning_index, and
    // def_ref, installs its own absorbing collection, and restores all three
    // after raw get_token collection and §23 runaway recovery.
    let mut command = CommandState::default();
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(&b"{runaway"[..]),
        ))
        .expect("source registers");
    command
        .open_registered_source(source)
        .expect("source opens");
    let outer = ScannerStatus::Defining(DefinitionContext {
        target: None,
        builder: TokenBuilderId(91),
        warning: ScannerWarning(37),
    });
    command.begin_scanner_status(outer.clone());
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    let (scanned, diagnostics) = {
        let mut processor =
            processor(&mut command, &mut universe, &mut capabilities).with_observer(&mut recorder);
        let scanned = processor
            .scan_toks(ScanToksMode::GeneralText {
                purpose: "unexpanded",
            })
            .expect("runaway general text recovers with an inserted right brace");
        (scanned, processor.take_semantic_diagnostics())
    };

    let [
        crate::CommandSemanticDiagnostic::Recoverable {
            runaway: Some(runaway),
            message,
            help,
            ..
        },
    ] = diagnostics.as_slice()
    else {
        panic!("expected one runaway diagnostic: {diagnostics:?}");
    };
    assert_eq!(runaway.heading, "Runaway text?");
    assert_eq!(runaway.partial, "runaway ");
    assert_eq!(message, "File ended while scanning text of ");
    assert_eq!(
        *help,
        [
            "I suspect you have forgotten a `}', causing me",
            "to read past where you wanted me to stop.",
            "I'll try to recover; but if the error is serious,",
            "you'd better type `E' or `X' now and fix your file.",
        ]
    );

    assert_eq!(command.scanner.status(), &outer);
    assert_eq!(command.scanner.warning(), Some(ScannerWarning(37)));
    assert_eq!(
        universe.tokens(scanned.replacement_text.token_list()),
        &[
            Token::Char {
                ch: 'r',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: 'u',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: 'n',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: 'a',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: 'w',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: 'a',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: 'y',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: ' ',
                cat: Catcode::Space,
            },
        ]
    );
    assert!(recorder.0.iter().any(|event| matches!(
        event,
        CommandObservation::Diagnostic(record)
            if record.diagnostic == "outer_validity_eof"
    )));
    assert!(
        !recorder
            .0
            .iter()
            .any(|event| matches!(event, CommandObservation::ScannerStatus(_))),
        "the recursive general-text scope is not a TeX82 scan_toks observation"
    );
}

#[test]
fn general_text_failure_restores_the_complete_outer_scanner_state() {
    let mut command = CommandState::default();
    push(
        &mut command,
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
        ],
    );
    let outer = ScannerStatus::Defining(DefinitionContext {
        target: None,
        builder: TokenBuilderId(83),
        warning: ScannerWarning(29),
    });
    command.begin_scanner_status(outer.clone());
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    let mut fuel = crate::CommandFuelLedger::new(1).expect("finite test fuel");
    let result = processor(&mut command, &mut universe, &mut capabilities)
        .with_fuel(fuel.fuel_mut())
        .scan_toks(ScanToksMode::GeneralText {
            purpose: "unexpanded",
        });

    assert!(result.is_err(), "the deliberately exhausted scan fails");
    assert_eq!(command.scanner.status(), &outer);
    assert_eq!(command.scanner.warning(), Some(ScannerWarning(29)));
    assert!(fuel.burned() <= 1);
}

#[test]
fn direct_the_count_scans_the_eight_bit_index_before_its_terminator_backup() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let the = install_expandable(&mut universe, "the", ExpandablePrimitive::The);
    let count = universe.intern("count").symbol();
    universe.set_meaning(
        count,
        Meaning::UnexpandablePrimitive(tex_state::meaning::UnexpandablePrimitive::Count),
    );
    universe.set_count(21, -83);
    push(
        &mut command,
        vec![
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Cs(the),
            Token::Cs(count),
            Token::Char {
                ch: '2',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: '1',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: ',',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
        ],
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    let mut processor =
        processor(&mut command, &mut universe, &mut capabilities).with_observer(&mut recorder);

    let scanned = processor
        .scan_toks(ScanToksMode::General { expanded: true })
        .expect("expanded collection succeeds");
    assert_eq!(
        processor
            .state
            .tokens(scanned.replacement_text.token_list()),
        &[
            Token::Char {
                ch: '-',
                cat: Catcode::Other
            },
            Token::Char {
                ch: '8',
                cat: Catcode::Other
            },
            Token::Char {
                ch: '3',
                cat: Catcode::Other
            },
            Token::Char {
                ch: ',',
                cat: Catcode::Other
            },
        ]
    );
    let two = recorder
        .0
        .iter()
        .position(|observation| {
            matches!(
                observation,
                CommandObservation::Command(record)
                    if matches!(record.spelling, ObservedToken::Character { character: '2', .. })
            )
        })
        .expect("index digit is delivered");
    let backup = recorder
        .0
        .iter()
        .position(|observation| {
            matches!(
                observation,
                CommandObservation::Input(record) if record.transition == InputTransition::Backup
            )
        })
        .expect("terminator is backed up");
    assert!(two < backup);
}

#[test]
fn completed_direct_splice_scan_rolls_back_to_the_exact_input_state() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let the = install_expandable(&mut universe, "the", ExpandablePrimitive::The);
    let register = universe.intern("stored").symbol();
    universe.set_meaning(register, Meaning::ToksRegister(3));
    let stored = universe.intern_token_list(&[
        Token::Char {
            ch: '{',
            cat: Catcode::BeginGroup,
        },
        Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        },
    ]);
    universe.set_toks(3, stored);
    push(
        &mut command,
        vec![
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Cs(the),
            Token::Cs(register),
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
        ],
    );
    let expected = command.clone();
    let snapshot = command.snapshot();
    let mut capabilities = CommandHostCapabilities::default();

    let first = {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        let scanned = processor
            .scan_toks(ScanToksMode::General { expanded: true })
            .expect("direct splice scan succeeds");
        processor
            .state
            .tokens(scanned.replacement_text.token_list())
            .to_vec()
    };
    command.rollback(snapshot).expect("rollback succeeds");
    assert_eq!(command, expected);

    let replayed = {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        let scanned = processor
            .scan_toks(ScanToksMode::General { expanded: true })
            .expect("rolled-back direct splice scan succeeds");
        processor
            .state
            .tokens(scanned.replacement_text.token_list())
            .to_vec()
    };
    assert_eq!(replayed, first);
}

#[test]
fn empty_direct_splice_is_unobserved_across_rollback_and_retry() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let the = install_expandable(&mut universe, "the", ExpandablePrimitive::The);
    let register = universe.intern("empty").symbol();
    universe.set_meaning(register, Meaning::ToksRegister(3));
    let empty = universe.intern_token_list(&[]);
    universe.set_toks(3, empty);
    push(
        &mut command,
        vec![
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Cs(the),
            Token::Cs(register),
            Token::Char {
                ch: 'x',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
        ],
    );
    let expected = command.clone();
    let mut snapshot = Some(command.snapshot());
    let mut capabilities = CommandHostCapabilities::default();

    for attempt in 0..2 {
        let mut recorder = Recorder::default();
        let scanned = CommandProcessor::new(
            &mut command,
            universe.command_context(),
            CommandHostContext::new(&mut capabilities),
        )
        .with_observer(&mut recorder)
        .scan_toks(ScanToksMode::General { expanded: true })
        .expect("empty direct splice scan succeeds");
        assert_eq!(
            universe.tokens(scanned.replacement_text.token_list()),
            &[Token::Char {
                ch: 'x',
                cat: Catcode::Letter,
            }],
            "empty §478 result changes no collected tokens"
        );
        assert!(
            !recorder.0.iter().any(|event| matches!(
                event,
                CommandObservation::TokenList(record)
                    if record.transition == "splice" && record.purpose == "the_toks"
            )),
            "empty §478 result publishes no splice observation"
        );
        if attempt == 0 {
            command
                .rollback(snapshot.take().expect("first attempt owns snapshot"))
                .expect("rollback succeeds");
            assert_eq!(command, expected);
        }
    }
}

#[test]
fn macro_definition_converts_parameters_and_preserves_doubled_hashes() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    push(
        &mut command,
        vec![
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
                ch: '#',
                cat: Catcode::Parameter,
            },
            Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
            },
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
        ],
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);

    let scanned = processor
        .scan_toks(ScanToksMode::MacroDefinition { expanded: false })
        .expect("definition scans");
    assert_eq!(
        processor.state.tokens(scanned.parameter_text.token_list()),
        &[Token::Param(1)]
    );
    assert_eq!(
        processor
            .state
            .tokens(scanned.replacement_text.token_list()),
        &[
            Token::Param(1),
            Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
            },
        ]
    );
}

/// TeX82 §477 gates the body's parameter-character rule on `macro_def`
/// alone, never on whether the parameter text declared a parameter, so a
/// parameterless definition still collapses `##` to one token. plain.tex's
/// `\m@ketabbox` (`\ialign\bgroup&\t@bbox##\t@bb@x\crcr`) is the canonical
/// witness.
#[test]
fn parameterless_macro_definition_still_collapses_doubled_hashes() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    push(
        &mut command,
        vec![
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
            },
            Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
            },
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
        ],
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);

    let scanned = processor
        .scan_toks(ScanToksMode::MacroDefinition { expanded: false })
        .expect("definition scans");
    assert!(
        processor
            .state
            .tokens(scanned.parameter_text.token_list())
            .is_empty()
    );
    assert_eq!(
        processor
            .state
            .tokens(scanned.replacement_text.token_list()),
        &[Token::Char {
            ch: '#',
            cat: Catcode::Parameter,
        }]
    );
}

/// The same rule is `macro_def`-gated: a general text scan (`\message`,
/// `\toks`, e-TeX `\unexpanded`) stores both parameter characters.
#[test]
fn general_text_keeps_both_parameter_characters() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    push(
        &mut command,
        vec![
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
            },
            Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
            },
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
        ],
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);

    let scanned = processor
        .scan_toks(ScanToksMode::General { expanded: false })
        .expect("general text scans");
    assert_eq!(
        processor
            .state
            .tokens(scanned.replacement_text.token_list()),
        &[
            Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
            },
            Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
            },
        ]
    );
}

#[test]
fn macro_definition_hash_brace_reuses_the_left_brace_after_the_body() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    push(
        &mut command,
        vec![
            Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
            },
            Token::Char {
                ch: '1',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
            },
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Char {
                ch: '[',
                cat: Catcode::Other,
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
                ch: ']',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
        ],
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);

    let scanned = processor
        .scan_toks(ScanToksMode::MacroDefinition { expanded: false })
        .expect("definition scans");
    assert_eq!(
        processor.state.tokens(scanned.parameter_text.token_list()),
        &[
            Token::Param(1),
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
        ]
    );
    assert_eq!(
        processor
            .state
            .tokens(scanned.replacement_text.token_list()),
        &[
            Token::Char {
                ch: '[',
                cat: Catcode::Other,
            },
            Token::Param(1),
            Token::Char {
                ch: ']',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
        ]
    );
}

#[test]
fn expanded_collection_expands_a_macro_one_step_at_a_time() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let macro_symbol = universe.intern("m").symbol();
    let empty = universe.intern_token_list(&[]);
    let replacement = universe.intern_token_list(&[Token::Char {
        ch: 'x',
        cat: Catcode::Letter,
    }]);
    let definition =
        universe.intern_macro(MacroMeaning::new(MeaningFlags::EMPTY, empty, replacement));
    universe.set_meaning(
        macro_symbol,
        Meaning::Macro {
            flags: MeaningFlags::EMPTY,
            definition: definition.id(),
        },
    );
    push(
        &mut command,
        vec![
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Cs(macro_symbol),
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
        ],
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);
    let scanned = processor
        .scan_toks(ScanToksMode::General { expanded: true })
        .expect("expanded scan succeeds");
    assert_eq!(
        processor
            .state
            .tokens(scanned.replacement_text.token_list()),
        &[Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        }]
    );
}

#[test]
fn scan_toks_all_parameter_number_success_and_diagnostic_boundaries() {
    for count in 0_u8..=9 {
        let mut tokens = Vec::new();
        for number in 1..=count {
            tokens.push(Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
            });
            tokens.push(Token::Char {
                ch: char::from(b'0' + number),
                cat: Catcode::Other,
            });
        }
        tokens.extend([
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
        ]);
        let mut command = CommandState::default();
        push(&mut command, tokens);
        let mut universe = crate::test_harness::universe_with_plain_catcodes();
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        let scanned = processor
            .scan_toks(ScanToksMode::MacroDefinition { expanded: false })
            .expect("parameter matrix scans");
        let expected = (1..=count).map(Token::Param).collect::<Vec<_>>();
        assert_eq!(
            processor
                .state
                .tokens(scanned.parameter_text.token_list())
                .tokens(),
            expected,
            "parameter count {count}"
        );
        assert!(!scanned.malformed_parameter);
    }

    for (tokens, expected_parameters) in [
        (
            vec![
                Token::Char {
                    ch: '#',
                    cat: Catcode::Parameter,
                },
                Token::Char {
                    ch: '2',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: '{',
                    cat: Catcode::BeginGroup,
                },
                Token::Char {
                    ch: '}',
                    cat: Catcode::EndGroup,
                },
            ],
            vec![
                Token::Param(1),
                Token::Char {
                    ch: '2',
                    cat: Catcode::Other,
                },
            ],
        ),
        (
            {
                let mut tokens = Vec::new();
                for number in 1_u8..=9 {
                    tokens.push(Token::Char {
                        ch: '#',
                        cat: Catcode::Parameter,
                    });
                    tokens.push(Token::Char {
                        ch: char::from(b'0' + number),
                        cat: Catcode::Other,
                    });
                }
                tokens.extend([
                    Token::Char {
                        ch: '#',
                        cat: Catcode::Parameter,
                    },
                    Token::Char {
                        ch: '0',
                        cat: Catcode::Other,
                    },
                    Token::Char {
                        ch: '{',
                        cat: Catcode::BeginGroup,
                    },
                    Token::Char {
                        ch: '}',
                        cat: Catcode::EndGroup,
                    },
                ]);
                tokens
            },
            { (1_u8..=9).map(Token::Param).collect::<Vec<_>>() },
        ),
    ] {
        let mut command = CommandState::default();
        push(&mut command, tokens);
        let mut universe = crate::test_harness::universe_with_plain_catcodes();
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        let scanned = processor
            .scan_toks(ScanToksMode::MacroDefinition { expanded: false })
            .expect("malformed parameter text recovers");
        assert!(scanned.malformed_parameter);
        assert_eq!(
            processor
                .state
                .tokens(scanned.parameter_text.token_list())
                .tokens(),
            expected_parameters
        );
    }

    let mut command = CommandState::default();
    push(
        &mut command,
        vec![
            Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
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
        ],
    );
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);
    let scanned = processor
        .scan_toks(ScanToksMode::MacroDefinition { expanded: false })
        .expect("hash-brace definition scans");
    assert_eq!(
        processor.state.tokens(scanned.parameter_text.token_list()),
        &[Token::Char {
            ch: '{',
            cat: Catcode::BeginGroup,
        }]
    );
    assert_eq!(
        processor
            .state
            .tokens(scanned.replacement_text.token_list()),
        &[
            Token::Char {
                ch: 'x',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
        ]
    );
}

#[test]
fn parameter_text_retains_non_hash_match_character_spelling() {
    let mut command = CommandState::default();
    let other = |ch| Token::Char {
        ch,
        cat: Catcode::Other,
    };
    push(
        &mut command,
        vec![
            Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
            },
            other('1'),
            Token::Char {
                ch: 'U',
                cat: Catcode::Parameter,
            },
            other('2'),
            other('x'),
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
        ],
    );
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);

    let scanned = processor
        .scan_toks(ScanToksMode::MacroDefinition { expanded: false })
        .expect("parameter text scans");
    assert_eq!(
        processor.state.tokens(scanned.parameter_text.token_list()),
        &[
            Token::Param(1),
            Token::Char {
                ch: 'U',
                cat: Catcode::Parameter,
            },
            Token::Param(2),
            other('x'),
        ]
    );
}

#[test]
fn scan_toks_raw_expanded_nested_brace_illegal_hash_and_missing_brace_matrix() {
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let macro_symbol = universe.intern("m").symbol();
    let empty = universe.intern_token_list(&[]);
    let replacement = universe.intern_token_list(&[Token::Char {
        ch: 'x',
        cat: Catcode::Letter,
    }]);
    universe.set_macro_meaning(
        macro_symbol,
        MacroMeaning::new(MeaningFlags::EMPTY, empty, replacement),
    );
    for (expanded, expected) in [
        (false, vec![Token::Cs(macro_symbol)]),
        (
            true,
            vec![Token::Char {
                ch: 'x',
                cat: Catcode::Letter,
            }],
        ),
    ] {
        let mut command = CommandState::default();
        push(
            &mut command,
            vec![
                Token::Char {
                    ch: '{',
                    cat: Catcode::BeginGroup,
                },
                Token::Cs(macro_symbol),
                Token::Char {
                    ch: '}',
                    cat: Catcode::EndGroup,
                },
            ],
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        let scanned = processor
            .scan_toks(ScanToksMode::General { expanded })
            .expect("raw/expanded collection scans");
        assert_eq!(
            processor
                .state
                .tokens(scanned.replacement_text.token_list())
                .tokens(),
            expected
        );
    }

    let mut command = CommandState::default();
    push(
        &mut command,
        vec![
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Char {
                ch: 'n',
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
    let mut nested_processor = processor(&mut command, &mut universe, &mut capabilities);
    let nested = nested_processor
        .scan_toks(ScanToksMode::General { expanded: false })
        .expect("nested raw collection scans");
    assert_eq!(
        nested_processor
            .state
            .tokens(nested.replacement_text.token_list()),
        &[
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Char {
                ch: 'n',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
        ]
    );

    let mut command = CommandState::default();
    push(
        &mut command,
        vec![
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
                ch: '#',
                cat: Catcode::Parameter,
            },
            Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
            },
            Token::Char {
                ch: '2',
                cat: Catcode::Other,
            },
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
        ],
    );
    let mut hashes_processor = processor(&mut command, &mut universe, &mut capabilities);
    let hashes = hashes_processor
        .scan_toks(ScanToksMode::MacroDefinition { expanded: false })
        .expect("hash recovery scans");
    assert_eq!(
        hashes_processor
            .state
            .tokens(hashes.replacement_text.token_list()),
        &[
            Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
            },
            Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
            },
            Token::Char {
                ch: '2',
                cat: Catcode::Other,
            },
        ]
    );

    let mut command = CommandState::default();
    push(
        &mut command,
        vec![Token::Char {
            ch: 'z',
            cat: Catcode::Letter,
        }],
    );
    let mut missing_processor = processor(&mut command, &mut universe, &mut capabilities);
    let recovered = missing_processor
        .scan_toks(ScanToksMode::General { expanded: false })
        .expect("§403 and runaway recovery complete the token list");
    assert_eq!(
        missing_processor
            .state
            .tokens(recovered.replacement_text.token_list()),
        &[Token::Char {
            ch: 'z',
            cat: Catcode::Letter,
        }],
        "the backed-up offender is the first token of the inserted group"
    );

    let the = install_expandable(&mut universe, "the-matrix", ExpandablePrimitive::The);
    let register = universe.intern("matrix-toks").symbol();
    let stored = universe.intern_token_list(&[Token::Cs(macro_symbol)]);
    universe.set_toks(5, stored);
    universe.set_meaning(register, Meaning::ToksRegister(5));
    let mut command = CommandState::default();
    push(
        &mut command,
        vec![
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Cs(the),
            Token::Cs(register),
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
        ],
    );
    let mut direct_processor = processor(&mut command, &mut universe, &mut capabilities);
    let direct = direct_processor
        .scan_toks(ScanToksMode::General { expanded: true })
        .expect("direct the splice scans");
    assert_eq!(
        direct_processor
            .state
            .tokens(direct.replacement_text.token_list()),
        &[Token::Cs(macro_symbol)],
        "direct the output is not recursively expanded"
    );
}

#[test]
fn scan_toks_all_scanner_status_outer_and_eof_recovery() {
    for (mode, active, purpose) in [
        (
            ScanToksMode::MacroDefinition { expanded: false },
            "defining",
            "macro_replacement",
        ),
        (
            ScanToksMode::General { expanded: false },
            "absorbing",
            "scan_toks",
        ),
    ] {
        let mut command = CommandState::default();
        push(
            &mut command,
            vec![
                Token::Char {
                    ch: '{',
                    cat: Catcode::BeginGroup,
                },
                Token::Char {
                    ch: '}',
                    cat: Catcode::EndGroup,
                },
            ],
        );
        let mut universe = crate::test_harness::universe_with_plain_catcodes();
        let mut capabilities = CommandHostCapabilities::default();
        let mut recorder = Recorder::default();
        CommandProcessor::new(
            &mut command,
            universe.command_context(),
            CommandHostContext::new(&mut capabilities),
        )
        .with_observer(&mut recorder)
        .scan_toks(mode)
        .expect("status-scoped scan completes");
        assert!(recorder.0.iter().any(|event| matches!(
            event,
            CommandObservation::ScannerStatus(status)
                if status.from == "normal" && status.to == active
        )));
        assert!(recorder.0.iter().any(|event| matches!(
            event,
            CommandObservation::ScannerStatus(status)
                if status.from == active && status.to == "normal"
        )));
        assert!(recorder.0.iter().any(|event| matches!(
            event,
            CommandObservation::TokenList(record) if record.purpose == purpose
        )));
    }

    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    let outer = universe.intern("outer-matrix").symbol();
    let empty = universe.intern_token_list(&[]);
    universe.set_macro_meaning(outer, MacroMeaning::new(MeaningFlags::OUTER, empty, empty));
    push(
        &mut command,
        vec![
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Cs(outer),
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
        ],
    );
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);
    let recovered = processor
        .scan_toks(ScanToksMode::MacroDefinition { expanded: false })
        .expect("outer validity inserts a right brace");
    assert_eq!(
        processor
            .state
            .tokens(recovered.replacement_text.token_list()),
        &[],
        "check_outer_validity's temporary recovery space is not collected"
    );
    assert_eq!(
        processor
            .get_token()
            .expect("outer token delivers")
            .expect("outer token remains")
            .control_sequence(),
        Some(outer)
    );

    let mut command = CommandState::default();
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(&b"{EOF"[..]),
        ))
        .expect("source registers");
    command
        .open_registered_source(source)
        .expect("source opens");
    let mut recorder = Recorder::default();
    let scanned = CommandProcessor::new(
        &mut command,
        universe.command_context(),
        CommandHostContext::new(&mut capabilities),
    )
    .with_observer(&mut recorder)
    .scan_toks(ScanToksMode::General { expanded: false })
    .expect("EOF recovery inserts a right brace");
    assert_eq!(
        universe.tokens(scanned.replacement_text.token_list()),
        &[
            Token::Char {
                ch: 'E',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: 'O',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: 'F',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: ' ',
                cat: Catcode::Space,
            },
        ]
    );
    assert!(recorder.0.iter().any(|event| matches!(
        event,
        CommandObservation::ScannerStatus(status)
            if status.from == "absorbing" && status.to == "normal"
    )));
}

#[test]
fn expanded_scan_toks_resumes_after_outer_token_aborts_macro_argument() {
    // TeX82 §394 returns from a macro call when §23 changes `long_state` to
    // `outer_call` and inserts frozen `\par`. The enclosing §380
    // get_x_token loop must resume; an expanded scan_toks collector is one
    // such loop and must not surface the internal matcher abort.
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    universe.install_primitive_meaning(
        "par",
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Par),
    );
    let caller = universe.intern("caller").symbol();
    let parameter = universe.intern_token_list(&[Token::Param(1)]);
    let empty = universe.intern_token_list(&[]);
    universe.set_macro_meaning(
        caller,
        MacroMeaning::new(MeaningFlags::EMPTY, parameter, empty),
    );
    let outer = universe.intern("outer").symbol();
    universe.set_macro_meaning(outer, MacroMeaning::new(MeaningFlags::OUTER, empty, empty));
    push(
        &mut command,
        vec![
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Cs(caller),
            Token::Cs(outer),
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
        ],
    );

    let mut processor = processor(&mut command, &mut universe, &mut capabilities);
    let recovered = processor
        .scan_toks(ScanToksMode::General { expanded: true })
        .expect("§394 outer recovery resumes expanded token collection");

    assert_eq!(
        processor
            .state
            .tokens(recovered.replacement_text.token_list()),
        &[]
    );
    assert_eq!(
        processor
            .get_token()
            .expect("backed outer token delivers")
            .expect("outer token remains")
            .control_sequence(),
        Some(outer)
    );
}

#[test]
fn expanded_scan_toks_outer_abort_reinstates_saved_collector_status() {
    // TeX82 §§23, 394, and 400: nested outer-token recovery can leave
    // `scanner_status := normal` as the abort unwinds, but scan_toks still
    // owns the saved absorbing episode that must govern backed input.
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);
    let collector = ScannerStatus::Absorbing(AbsorbingContext {
        owner: None,
        builder: TokenBuilderId(17),
        warning: ScannerWarning(17),
    });
    let episode =
        processor.begin_scanner_episode(collector.clone(), ScannerStatusVisibility::Observed);
    processor.command.scanner.clear_for_recovery();

    processor.resume_scanner_episode_after_recovery(&episode);

    assert_eq!(processor.command.scanner.status(), &collector);
    processor.finish_scanner_episode(episode);
    assert_eq!(processor.command.scanner.status(), &ScannerStatus::Normal);
}

#[test]
fn expanded_collection_observes_protected_macro_suppression_before_delivery() {
    // e-TeX 2.6 change section [27.465] changes a protected macro to
    // `relax/no_expand_flag` inside expanded `scan_toks`. The reference
    // instrumentation records the retained spelling at that transition.
    let mut command = CommandState::new(crate::CommandProfile::ETEX26);
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let protected = universe.intern("protected-macro").symbol();
    let empty = universe.intern_token_list(&[]);
    universe.set_macro_meaning(
        protected,
        MacroMeaning::new(MeaningFlags::PROTECTED, empty, empty),
    );
    push(
        &mut command,
        vec![
            Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            },
            Token::Cs(protected),
            Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            },
        ],
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    let scanned = CommandProcessor::new(
        &mut command,
        universe.command_context(),
        CommandHostContext::new(&mut capabilities),
    )
    .with_observer(&mut recorder)
    .scan_toks(ScanToksMode::General { expanded: true })
    .expect("protected macro remains in expanded collection");

    assert_eq!(
        universe.tokens(scanned.replacement_text.token_list()),
        &[Token::Cs(protected)]
    );
    let suppression = recorder
        .0
        .iter()
        .position(|event| {
            matches!(
                event,
                CommandObservation::TokenList(record)
                    if record.transition == "splice"
                        && record.purpose == "protected_expansion_suppression"
                        && record.tokens
                            == [ObservedToken::ControlSequence("protected-macro".into())]
            )
        })
        .expect("protected suppression splice is observed");
    let delivery = recorder
        .0
        .iter()
        .position(|event| {
            matches!(
                event,
                CommandObservation::Command(record)
                    if record.boundary == CommandDeliveryBoundary::Expanded
                        && record.spelling
                        == ObservedToken::ControlSequence("protected-macro".into())
                        && record.command == "relax"
                        && record.command_operand == Some(257)
            )
        })
        .expect("terminal expanded delivery is observed");
    assert!(suppression < delivery);
}

#[test]
fn tex82_expansion_macros_observes_raw_expanded_and_direct_splice_scan_toks() {
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let macro_symbol = universe.intern("observed-macro").symbol();
    let empty = universe.intern_token_list(&[]);
    let replacement = universe.intern_token_list(&[Token::Char {
        ch: 'x',
        cat: Catcode::Letter,
    }]);
    universe.set_macro_meaning(
        macro_symbol,
        MacroMeaning::new(MeaningFlags::EMPTY, empty, replacement),
    );

    let raw_events = {
        let mut command = CommandState::default();
        push(
            &mut command,
            vec![
                Token::Char {
                    ch: '{',
                    cat: Catcode::BeginGroup,
                },
                Token::Cs(macro_symbol),
                Token::Char {
                    ch: '}',
                    cat: Catcode::EndGroup,
                },
            ],
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut recorder = Recorder::default();
        let scanned = CommandProcessor::new(
            &mut command,
            universe.command_context(),
            CommandHostContext::new(&mut capabilities),
        )
        .with_observer(&mut recorder)
        .scan_toks(ScanToksMode::MacroDefinition { expanded: false })
        .expect("ordinary definition scans");
        assert_eq!(
            universe.tokens(scanned.replacement_text.token_list()),
            &[Token::Cs(macro_symbol)]
        );
        recorder.0
    };
    let raw_enter = raw_events
        .iter()
        .position(|event| {
            matches!(event, CommandObservation::ScannerStatus(status)
            if status.from == "normal" && status.to == "defining")
        })
        .expect("definition status begins");
    let raw_restore = raw_events
        .iter()
        .position(|event| {
            matches!(event, CommandObservation::ScannerStatus(status)
            if status.from == "defining" && status.to == "normal")
        })
        .expect("definition status restores");
    let raw_complete = raw_events
        .iter()
        .position(|event| {
            matches!(event, CommandObservation::TokenList(record)
            if record.transition == "complete"
                && record.purpose == "macro_replacement"
                && record.tokens == [ObservedToken::ControlSequence("observed-macro".into())])
        })
        .expect("ordinary definition result is observed");
    assert!(raw_enter < raw_restore && raw_restore < raw_complete);

    let expanded_events = {
        let mut command = CommandState::default();
        push(
            &mut command,
            vec![
                Token::Char {
                    ch: '{',
                    cat: Catcode::BeginGroup,
                },
                Token::Cs(macro_symbol),
                Token::Char {
                    ch: '}',
                    cat: Catcode::EndGroup,
                },
            ],
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut recorder = Recorder::default();
        CommandProcessor::new(
            &mut command,
            universe.command_context(),
            CommandHostContext::new(&mut capabilities),
        )
        .with_observer(&mut recorder)
        .scan_toks(ScanToksMode::General { expanded: true })
        .expect("expanded definition scans");
        recorder.0
    };
    assert!(expanded_events.iter().any(|event| matches!(
        event,
        CommandObservation::TokenList(record)
            if record.transition == "complete"
                && record.purpose == "expanded_scan_toks"
                && record.tokens == [ObservedToken::Character {
                    character: 'x',
                    catcode: Catcode::Letter,
                }]
    )));

    let direct_events = {
        let mut command = CommandState::default();
        let the = install_expandable(&mut universe, "the-observed", ExpandablePrimitive::The);
        let register = universe.intern("observed-register").symbol();
        let stored = universe.intern_token_list(&[Token::Cs(macro_symbol)]);
        universe.set_toks(5, stored);
        universe.set_meaning(register, Meaning::ToksRegister(5));
        push(
            &mut command,
            vec![
                Token::Char {
                    ch: '{',
                    cat: Catcode::BeginGroup,
                },
                Token::Cs(the),
                Token::Cs(register),
                Token::Char {
                    ch: '}',
                    cat: Catcode::EndGroup,
                },
            ],
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut recorder = Recorder::default();
        let scanned = CommandProcessor::new(
            &mut command,
            universe.command_context(),
            CommandHostContext::new(&mut capabilities),
        )
        .with_observer(&mut recorder)
        .scan_toks(ScanToksMode::General { expanded: true })
        .expect("direct the splice scans");
        assert_eq!(
            universe.tokens(scanned.replacement_text.token_list()),
            &[Token::Cs(macro_symbol)],
            "the_toks output is copied without recursive expansion"
        );
        recorder.0
    };
    let splice = direct_events
        .iter()
        .position(|event| {
            matches!(event, CommandObservation::TokenList(record)
            if record.transition == "splice"
                && record.purpose == "the_toks"
                && record.tokens == [ObservedToken::ControlSequence("observed-macro".into())])
        })
        .expect("direct token-list splice is observed");
    assert!(
        !direct_events.iter().any(|event| {
            matches!(event, CommandObservation::Command(record)
            if record.boundary == crate::CommandDeliveryBoundary::Expanded
                && record.command == "the"
                && record.spelling
                    == ObservedToken::ControlSequence("the-observed".into()))
        }),
        "the_toks consumes its expandable opener before any expanded delivery"
    );
    let raw_operand = direct_events
        .iter()
        .position(|event| {
            matches!(event, CommandObservation::Command(record)
                if record.boundary == crate::CommandDeliveryBoundary::Raw
                    && record.spelling
                        == ObservedToken::ControlSequence("observed-register".into()))
        })
        .expect("the operand has a raw delivery");
    assert!(
        raw_operand < splice,
        "the operand scan precedes its resulting splice"
    );
    let restore = direct_events
        .iter()
        .position(|event| {
            matches!(event, CommandObservation::ScannerStatus(status)
            if status.from == "absorbing" && status.to == "normal")
        })
        .expect("absorbing status restores");
    let complete = direct_events
        .iter()
        .position(|event| {
            matches!(event, CommandObservation::TokenList(record)
            if record.transition == "complete"
                && record.purpose == "expanded_scan_toks"
                && record.tokens == [ObservedToken::ControlSequence("observed-macro".into())])
        })
        .expect("completed direct-splice result is observed");
    assert!(splice < restore && restore < complete);
}

/// TeX82 §403 opens with §404's "Get the next non-blank non-relax
/// non-call token", so a `\relax` before a mandatory `{` is skipped
/// rather than treated as the missing brace.
///
/// §403 states the rule in prose too: "\TeX\ allows \relax to appear
/// before the left_brace". Skipping only spaces made every mandatory
/// group that a `\relax` guards -- the plain-TeX idiom for stopping an
/// unwanted lookahead -- take §403's `back_error` recovery instead
/// (umber2-johp.209).
#[test]
fn a_mandatory_left_brace_scan_skips_relax_as_well_as_spaces() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe();
    let relax = universe.intern("relax").symbol();
    universe.set_meaning(relax, Meaning::Relax);
    push(
        &mut command,
        vec![
            Token::Char {
                ch: ' ',
                cat: Catcode::Space,
            },
            Token::Cs(relax),
            Token::Char {
                ch: ' ',
                cat: Catcode::Space,
            },
            Token::Cs(relax),
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
        ],
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);
    let scanned = processor
        .scan_toks(ScanToksMode::General { expanded: false })
        .expect("§404 skips the guarding `\\relax`");
    assert_eq!(
        processor
            .state
            .tokens(scanned.replacement_text.token_list()),
        &[Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        }]
    );
}

fn read_stream(universe: &mut Universe, bytes: &[u8]) -> tex_state::world::StreamSlot {
    read_stream_at(universe, 1, bytes)
}

fn read_stream_at(
    universe: &mut Universe,
    stream: u8,
    bytes: &[u8],
) -> tex_state::world::StreamSlot {
    let path = format!("stream-{stream}.tex");
    universe
        .world_mut()
        .set_memory_file(&path, bytes.to_vec())
        .expect("memory world accepts a seeded file");
    let slot = tex_state::world::StreamSlot::new(stream);
    universe
        .world_mut()
        .open_in(slot, path)
        .expect("stream opens");
    slot
}

fn read_text(processor: &CommandProcessor<'_>, list: &TracedTokenList) -> String {
    processor
        .state
        .tokens(list.token_list())
        .iter()
        .map(|token| match token {
            Token::Char { ch, .. } => *ch,
            _ => '\u{0}',
        })
        .collect()
}

#[test]
fn readline_exact_bytes_nested_in_scantokens_replay_after_rollback() {
    // e-TeX 2.6 etex.ch §53a and §53c retain TeX's eight-bit character
    // domain: `\readline` assigns catcode 12 without requiring the byte to be
    // a Unicode-domain scalar. Its one-line pseudo-file must then retire back
    // to the enclosing `\scantokens` pseudo-file.
    let mut command = CommandState::new(crate::CommandProfile::ETEX26);
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    universe.set_int_param(tex_state::env::banks::IntParam::END_LINE_CHAR, -1);
    let empty = TracedTokenList::synthetic(universe.intern_token_list_ref(&[]));
    let scantokens = command
        .open_scantokens(
            SourceRegistration::new(RegisteredSourceKind::Generated, b"q\n".to_vec()),
            Some(empty),
            18,
        )
        .expect("scantokens pseudo-file opens");
    let expected = command.clone();
    let mut snapshot = Some(command.snapshot());
    let mut first = None;
    let mut capabilities = CommandHostCapabilities::default();

    for _attempt in 0..2 {
        let mut fuel = crate::CommandFuelLedger::new(16).expect("finite test fuel");
        let collected = {
            let mut processor = processor(&mut command, &mut universe, &mut capabilities)
                .with_fuel(fuel.fuel_mut());
            let line = processor
                .command
                .begin_read_line()
                .expect("readline pseudo-file opens");
            processor
                .command
                .finish_read_line(
                    line,
                    crate::input::SourceNameClass::ReadStream(1),
                    vec![0xff],
                )
                .expect("readline bytes install");
            let mut tokens = tex_state::token::RootedTracedTokenBuffer::default();
            processor
                .collect_read_line_verbatim(line, &mut tokens)
                .expect("exact-byte readline collects");
            assert_eq!(
                processor.command.top_input_level_identity(),
                Some(scantokens),
                "readline retirement resumes the enclosing scantokens source"
            );
            tokens
                .into_iter()
                .map(|word| word.word().semantic_token())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            collected,
            [Token::Char {
                ch: '\u{ff}',
                cat: Catcode::Other,
            }]
        );
        assert!(fuel.burned() <= 16);
        if let Some(previous) = &first {
            assert_eq!(&collected, previous, "rollback replays the exact byte");
        } else {
            first = Some(collected);
            command
                .rollback(snapshot.take().expect("first attempt owns snapshot"))
                .expect("rollback succeeds");
            assert_eq!(command, expected);
        }
    }
}

#[test]
fn read_toks_collects_balanced_multiline_input_and_appends_one_eof_line() {
    // TeX82 §482: `repeat <input and store one line> until
    // align_state=1000000`, so an unmatched `{` continues onto the next line.
    // §486 closes the stream at end of file and appends one empty line, which
    // §483 tokenizes into the `\par` an active `\endlinechar` produces.
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let slot = read_stream(&mut universe, b"{one\ntwo}\n");
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);
    let target = processor.state.intern_control_sequence("line");

    let list = processor
        .read_toks(1, target, false)
        .expect("read collects");

    // The trailing space is line two's own `\endlinechar`, which
    // §483 stores in `buffer[limit]` before tokenizing the line.
    assert_eq!(read_text(&processor, &list), "{one two} ");
    // §482 restores `align_state`, so the collection leaves no alignment
    // state behind for the caller.
    assert_eq!(
        processor.command.alignment.align_state,
        crate::processor::alignment::TOP_LEVEL_ALIGN_STATE
    );
    // §486 appends its empty line only when `input_ln` actually fails, so
    // a `\read` that balanced on the file's last line leaves the stream
    // open for the next one.
    assert!(!processor.state.read_stream_at_eof(slot));

    let second = processor
        .read_toks(1, target, false)
        .expect("read collects the appended empty line");
    // §486: the stream closes and one empty line is appended. §483 still
    // tokenizes it, and an empty line in `state=new_line` is §351's `\par`.
    let par = processor.state.intern_control_sequence("par");
    assert_eq!(
        processor.state.tokens(second.token_list()),
        [Token::Cs(par)]
    );
    assert!(processor.state.read_stream_at_eof(slot));
}

#[test]
fn read_toks_outer_recovery_pseudoprints_end_match_and_partial_body() {
    // TeX82 §§482/306/336: `read_toks` seeds `def_ref` with `end_match_token`,
    // and `runaway` pseudoprints that live list when an outer command ends
    // the read. The sentinel therefore renders as `->` before the body. An
    // outer command delivered from `name=1..17` is not backed up, so the live
    // context remains the read-stream line rather than a token-list replay.
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    read_stream(&mut universe, b"x\\stop");
    let empty = tex_state::ids::TokenListId::EMPTY;
    let stop = universe.intern("stop").symbol();
    universe.set_macro_meaning(stop, MacroMeaning::new(MeaningFlags::OUTER, empty, empty));
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);
    let target = processor.state.intern_control_sequence("line");

    processor
        .read_toks(1, target, false)
        .expect("outer recovery completes the read");

    let diagnostics = processor.take_semantic_diagnostics();
    assert!(
        matches!(
            diagnostics.first(),
            Some(crate::CommandSemanticDiagnostic::Recoverable {
                runaway: Some(crate::state::RunawayPrelude { partial, .. }),
                ..
            }) if partial == "->x"
        ),
        "{diagnostics:?}"
    );
    let context = match diagnostics.first() {
        Some(crate::CommandSemanticDiagnostic::Recoverable { context, .. }) => context,
        other => panic!("expected outer-recovery diagnostic, got {other:?}"),
    };
    assert!(context.contains("<read 1> x\\stop"), "{context:?}");
    assert!(!context.contains("<to be read again>"), "{context:?}");
}

#[test]
fn read_toks_covers_stream_boundaries_and_empty_first_line() {
    // TeX82 §§480, 482-485: 0 and 15 are the inclusive open-stream
    // boundaries, while 16 and every negative number clamp to the permanently
    // closed terminal slot. An empty physical line is input, not EOF: its
    // endline becomes §351's `\par`, and the next read consumes only `a`.
    for stream in [0_i32, 15, 16, -1] {
        let mut command = CommandState::default();
        let outer = ScannerStatus::Absorbing(AbsorbingContext {
            owner: None,
            builder: TokenBuilderId(59),
            warning: ScannerWarning(41),
        });
        command.begin_scanner_status(outer.clone());
        command.alignment.align_state = 73;
        let mut universe = crate::test_harness::universe_with_plain_catcodes();
        universe.set_interaction_mode(tex_state::InteractionMode::ErrorStop);
        let slot = u8::try_from(stream)
            .ok()
            .filter(|stream| *stream < tex_state::world::STREAM_SLOT_COUNT as u8)
            .map(|stream| read_stream_at(&mut universe, stream, b"\na\n"));
        if slot.is_none() {
            for line in ["", "a"] {
                universe
                    .world_mut()
                    .push_memory_terminal_line(line)
                    .expect("terminal input registers");
            }
        }
        let mut capabilities = CommandHostCapabilities::default();
        {
            let mut processor = processor(&mut command, &mut universe, &mut capabilities);
            let target = processor.state.intern_control_sequence("line");
            let par = processor.state.intern_control_sequence("par");

            let empty = processor
                .read_toks(stream, target, false)
                .expect("empty first line is a successful read");
            assert_eq!(processor.state.tokens(empty.token_list()), [Token::Cs(par)]);
            assert_eq!(
                processor.command.scanner.status(),
                &outer,
                "stream {stream}"
            );
            assert_eq!(
                processor.command.scanner.warning(),
                Some(ScannerWarning(41))
            );
            assert_eq!(
                processor.command.alignment.align_state, 73,
                "stream {stream}"
            );

            let a = processor
                .read_toks(stream, target, false)
                .expect("second line is independently readable");
            assert_eq!(read_text(&processor, &a), "a ", "stream {stream}");
            assert_eq!(
                processor.command.scanner.status(),
                &outer,
                "stream {stream}"
            );
            assert_eq!(
                processor.command.scanner.warning(),
                Some(ScannerWarning(41))
            );
            assert_eq!(
                processor.command.alignment.align_state, 73,
                "stream {stream}"
            );

            if let Some(slot) = slot {
                assert!(!processor.state.read_stream_at_eof(slot));
            }
        }

        if slot.is_none() {
            for slot in 0..tex_state::world::STREAM_SLOT_COUNT as u8 {
                assert!(
                    universe
                        .world()
                        .stream_bufs()
                        .read_stream_target(tex_state::world::StreamSlot::new(slot))
                        .is_none(),
                    "clamped stream {stream} must not initialize slot {slot}"
                );
            }
        }

        let expected_terminal = match stream {
            // The prompt precedes the line echoed by the memory terminal.
            16 => "\n\\line=\n\n\\line=a\n",
            // §484's negative-stream policy uses an empty prompt, but the
            // two accepted terminal lines are still echoed exactly once.
            -1 => "\na\n",
            _ => "",
        };
        assert_eq!(
            diagnostic_text(&universe),
            expected_terminal,
            "stream {stream}"
        );
    }
}

#[test]
fn read_terminal_in_nonstop_mode_reports_canonical_fatal() {
    // TeX82 §484: a closed stream selects terminal input, but interaction at
    // or below nonstop mode cannot prompt and calls `fatal_error` instead.
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    universe.set_interaction_mode(tex_state::InteractionMode::Nonstop);
    let mut capabilities = CommandHostCapabilities::default();
    {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        let target = processor.state.intern_control_sequence("line");

        let error = processor
            .read_toks(1, target, false)
            .expect_err("nonstop terminal read is fatal");
        assert_eq!(
            error,
            CommandError::Fatal(crate::FatalError::emergency_stop(
                "job aborted, file error in nonstop mode",
            ))
        );
        assert_eq!(
            processor.command.alignment.align_state,
            crate::processor::alignment::TOP_LEVEL_ALIGN_STATE
        );
        assert!(matches!(
            processor.command.scanner.status(),
            crate::processor::status::ScannerStatus::Normal
        ));
    }
    assert_eq!(diagnostic_text(&universe), "");
}

#[test]
fn read_unbalanced_eof_reports_file_ended_within_read() {
    // TeX82 §§306/486: EOF during an unbalanced `\read` pseudoprints the
    // live `def_ref` before the exact error, resets only the temporary brace
    // state, and keeps the supplied tokens. The report precedes tokenizing
    // the appended empty line, so its partial excludes that line and its
    // context still owns the read-stream frame.
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let slot = read_stream(&mut universe, b"{open");
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);
    let target = processor.state.intern_control_sequence("line");

    let list = processor
        .read_toks(1, target, false)
        .expect("§486 recovers the unbalanced read");
    assert_eq!(read_text(&processor, &list), "{open \0");
    let par = processor.state.intern_control_sequence("par");
    assert_eq!(
        processor.state.tokens(list.token_list()).last(),
        Some(&Token::Cs(par)),
        "§486's appended empty line is still tokenized"
    );
    assert!(processor.state.read_stream_at_eof(slot));
    assert_eq!(
        processor.command.alignment.align_state,
        crate::processor::alignment::TOP_LEVEL_ALIGN_STATE
    );
    assert!(matches!(
        processor.command.scanner.status(),
        crate::processor::status::ScannerStatus::Normal
    ));
    let diagnostics = processor.take_semantic_diagnostics();
    let [
        crate::CommandSemanticDiagnostic::Recoverable {
            identity,
            runaway: Some(runaway),
            message,
            help,
            context,
        },
    ] = diagnostics.as_slice()
    else {
        panic!("expected one §486 runaway diagnostic: {diagnostics:?}");
    };
    assert_eq!(*identity, FILE_ENDED_WITHIN_READ_DIAGNOSTIC);
    assert_eq!(runaway.heading, "Runaway definition?");
    assert_eq!(runaway.partial, "->{open ");
    assert_eq!(message, "File ended within \\read");
    assert_eq!(*help, &["This \\read has unbalanced braces."]);
    assert!(context.contains("<read 1>"), "{context:?}");
    assert!(!context.contains("\\par"), "{context:?}");
}

#[test]
fn read_and_readline_retire_with_the_open_stream_name() {
    // TeX82 §483 assigns `name=m+1` before acquiring the line. e-TeX's
    // `\readline` branch changes token construction only, so both controls
    // retain the open stream's source name. Closed-stream terminal fallback
    // remains covered separately below.
    let mut command = CommandState::new(crate::CommandProfile::ETEX26);
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    universe.set_int_param(tex_state::env::banks::IntParam::END_LINE_CHAR, -1);
    let slot = read_stream(&mut universe, b"ordinary\nverbatim\n");
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    let mut fuel = crate::CommandFuelLedger::new(64).expect("finite test fuel");
    {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities)
            .with_observer(&mut recorder)
            .with_fuel(fuel.fuel_mut());
        let target = processor.state.intern_control_sequence("line");
        let ordinary = processor
            .read_toks(1, target, false)
            .expect("ordinary read collects");
        assert_eq!(read_text(&processor, &ordinary), "ordinary");
        let verbatim = processor
            .read_toks(1, target, true)
            .expect("verbatim readline collects");
        assert_eq!(read_text(&processor, &verbatim), "verbatim");
    }

    let retirements = recorder
        .0
        .iter()
        .filter_map(|event| match event {
            CommandObservation::Input(record)
                if record.transition == InputTransition::Retire
                    && record.reason == crate::InputReason::Source =>
            {
                record.source_name
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        retirements,
        [
            crate::SourceNameClass::ReadStream(slot.raw()),
            crate::SourceNameClass::ReadStream(slot.raw()),
        ],
        "`\\read` and `\\readline` share §483's open-stream source name"
    );
    let stops = recorder
        .0
        .iter()
        .filter_map(|event| match event {
            CommandObservation::Input(record)
                if record.transition == InputTransition::Stop
                    && record.reason == crate::InputReason::Source =>
            {
                record.source_name
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        stops,
        [],
        "§360's zero command is not a terminal input-stack stop"
    );
    assert!(fuel.burned() <= 64);
    assert!(!universe.command_context().read_stream_at_eof(slot));
}

#[test]
fn read_toks_reads_the_terminal_for_a_closed_or_out_of_range_stream() {
    // TeX82 §482: `if (n<0)or(n>15) then m:=16 else m:=n`. Stream 16 is never
    // open, so §483's `read_open[m]=closed` selects §484's terminal branch
    // for every out-of-range number, and for an in-range stream nobody
    // opened. §484 prompts once and then sets `n` negative, so a second line
    // is read with `prompt_input("")`.
    for stream in [-1_i32, 99, 3] {
        let mut command = CommandState::default();
        let mut universe = Universe::new_with_plain_catcodes();
        universe.set_interaction_mode(tex_state::InteractionMode::ErrorStop);
        for line in ["{first", "second}"] {
            universe
                .world_mut()
                .push_memory_terminal_line(line)
                .expect("terminal input registers");
        }
        let mut capabilities = CommandHostCapabilities::default();
        let mut recorder = Recorder::default();
        {
            let mut processor = processor(&mut command, &mut universe, &mut capabilities)
                .with_observer(&mut recorder);
            let target = processor.state.intern_control_sequence("line");
            let list = processor
                .read_toks(stream, target, false)
                .expect("terminal read collects");
            assert_eq!(read_text(&processor, &list), "{first second} ", "{stream}");
        }

        let defining = recorder
            .0
            .iter()
            .position(|event| {
                matches!(
                    event,
                    CommandObservation::ScannerStatus(record)
                        if record.from == "normal" && record.to == "defining"
                )
            })
            .expect("§482 enters defining status");
        let push = recorder
            .0
            .iter()
            .position(|event| {
                matches!(
                    event,
                    CommandObservation::Input(record)
                        if record.transition == InputTransition::Push
                            && record.reason == crate::InputReason::Source
                            && record.source_name == Some(crate::SourceNameClass::Terminal)
                )
            })
            .expect("§483 begins a terminal source level");
        let retire = recorder
            .0
            .iter()
            .rposition(|event| {
                matches!(
                    event,
                    CommandObservation::Input(record)
                        if record.transition == InputTransition::Retire
                            && record.reason == crate::InputReason::Source
                            && record.source_name == Some(crate::SourceNameClass::Terminal)
                )
            })
            .expect("§483 ends the terminal source level");
        assert!(
            !recorder.0.iter().any(|event| matches!(
                event,
                CommandObservation::Input(record)
                    if record.transition == InputTransition::Stop
                        && record.reason == crate::InputReason::Source
            )),
            "§360's zero command is a delivery result, not an input-stack stop"
        );
        let normal = recorder
            .0
            .iter()
            .rposition(|event| {
                matches!(
                    event,
                    CommandObservation::ScannerStatus(record)
                        if record.from == "defining" && record.to == "normal"
                )
            })
            .expect("§482 restores normal status");
        assert!(
            defining < push && push < retire && retire < normal,
            "§§482-484 retire the read pseudo-file before restoring scanner status"
        );
    }
}

#[test]
fn read_toks_prompts_only_for_a_nonnegative_stream() {
    // TeX82 §484: `if n<0 then prompt_input("") else begin wake_up_terminal;
    // print_ln; sprint_cs(r); prompt_input("="); n:=-1; end`. The test is on
    // §1225's *scanned* `n`, not on §482's clamped `m`: 99 and 3 both reach
    // the terminal, 99 because §482 clamped it to the never-open stream 16 and
    // 3 because nobody opened it, yet both still announce themselves as
    // `\line=` because the announcement tests `n`. Only a negative stream --
    // which is equally clamped to 16 -- reads silently. §484 then sets
    // `n:=-1`, so the second line of a multi-line read is never prompted again.
    for (stream, expected) in [(-1_i32, ""), (99, "\n\\line="), (3, "\n\\line=")] {
        let mut command = CommandState::default();
        let mut universe = Universe::new_with_plain_catcodes();
        universe.set_interaction_mode(tex_state::InteractionMode::ErrorStop);
        for line in ["{first", "second}"] {
            universe
                .world_mut()
                .push_memory_terminal_line(line)
                .expect("terminal input registers");
        }
        let mut capabilities = CommandHostCapabilities::default();
        {
            let mut processor = processor(&mut command, &mut universe, &mut capabilities);
            let target = processor.state.intern_control_sequence("line");
            processor
                .read_toks(stream, target, false)
                .expect("terminal read collects");
        }
        // §484's prompt is still in the rollback-capable live effect suffix
        // at this point, not the committed memory backend.
        let mut terminal = String::new();
        for record in universe.world().effect_records() {
            if let tex_state::world::EffectRecord::StreamWrite { sink, text } = record
                && matches!(
                    sink,
                    tex_state::PrintSink::Terminal | tex_state::PrintSink::TerminalAndLog
                )
            {
                terminal.push_str(text);
            }
        }
        assert_eq!(terminal, expected, "§484's prompt for \\read{stream}");
    }
}

#[test]
fn read_toks_disables_alignment_delimiters_and_restores_scanner_state() {
    // §482: `s:=align_state; align_state:=1000000` for the collection's whole
    // duration, so an alignment tab in the line is stored as an ordinary
    // token instead of ending a cell, and `align_state` and `scanner_status`
    // are both returned to what the caller had.
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let slot = read_stream(&mut universe, b"a&b\n");
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);
    let target = processor.state.intern_control_sequence("line");

    let list = processor
        .read_toks(1, target, false)
        .expect("read collects");

    assert_eq!(read_text(&processor, &list), "a&b ");
    assert_eq!(
        processor.command.alignment.align_state,
        crate::processor::alignment::TOP_LEVEL_ALIGN_STATE
    );
    assert!(matches!(
        processor.command.scanner.status(),
        crate::processor::status::ScannerStatus::Normal
    ));
    assert!(!processor.state.read_stream_at_eof(slot));
}
