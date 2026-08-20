use tex_state::print::{ErrorContextLevel, ErrorContextWidths, render_error_context};

use crate::CommandState;
use crate::input::levels::TransientReplayReason;
use crate::input::{
    InputLevel, InputState, RegisteredSourceKind, ReplayTrace, RetirementBehavior,
    SharedTokenBuffer, SourceRegistration, TokenBehavior, TokenPayload,
};

#[test]
fn retired_root_context_reaches_the_retained_startup_terminal_line() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    command.set_terminal_context_line("missing-end.tex");

    assert_eq!(
        command.output_open_context(&universe.command_context()),
        "\n<*> missing-end.tex\n                   "
    );
}

#[test]
fn macro_context_reads_the_admitted_chunk_after_definition_retirement() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let name = universe.intern("retired");
    let replacement = universe.intern_token_list_ref(&[tex_state::token::Token::Char {
        ch: 'x',
        cat: tex_state::token::Catcode::Letter,
    }]);
    let root = universe.intern_macro(tex_state::macro_store::MacroMeaning::new(
        tex_state::meaning::MeaningFlags::EMPTY,
        tex_state::ids::TokenListId::EMPTY,
        replacement.id(),
    ));
    let definition = root.id();
    let first = universe.begin_direct_operation();
    universe.set_meaning(
        name,
        tex_state::meaning::Meaning::Macro {
            flags: tex_state::meaning::MeaningFlags::EMPTY,
            definition,
        },
    );
    universe.commit_direct_operation(first);

    let definition_view = universe.macro_definition(definition);
    let admitted = command
        .parameters
        .admit_macro(definition, definition_view.meaning());
    let arguments = command.parameters.store_arguments(
        tex_state::token::RootedTracedTokenBuffer::default(),
        [None; 9],
    );
    command.push_macro_activation(
        name.symbol(),
        definition,
        arguments,
        tex_state::token::OriginId::UNKNOWN,
        admitted,
        definition_view.replacement_len(),
    );

    let second = universe.begin_direct_operation();
    universe.set_meaning(name, tex_state::meaning::Meaning::Relax);
    let _ = root;
    universe.commit_direct_operation(second);

    let context = command.output_open_context(&universe.command_context());
    assert_eq!(context, "\n\\retired ->\n           x");
}

#[test]
fn cropped_pseudoprint_preserves_the_location_label() {
    let widths = ErrorContextWidths::new(79, 35).expect("TeX82 context widths");
    let output = render_error_context(
        &[ErrorContextLevel::new(
            "l.26 ",
            r#"  \nonstopmode\lccode256-0\mathchardef\a="8000"#,
            r"\def\a{ SCALED 3~2769}",
        )],
        widths,
        5,
    );

    let mut lines = output.lines().skip(1);
    assert_eq!(lines.next(), Some("l.26 ...de256-0\\mathchardef\\a=\"8000"));
    let second = lines.next().expect("second context line");
    assert_eq!(second.chars().take(35).collect::<String>(), " ".repeat(35));
    assert_eq!(second.trim_start(), r"\def\a{ SCALED 3~2769}");
}

#[test]
fn source_context_pseudoprints_synthetic_endline_on_the_live_cursor_side() {
    // TeX82 §§313/362: `buffer[limit]` participates in pseudoprint even
    // though immutable source backing does not physically contain it.
    let mut command = CommandState::default();
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            b"left}--".as_slice(),
        ))
        .expect("source registers");
    command
        .open_registered_source(source)
        .expect("source opens");
    command
        .prepare_started_input(13)
        .expect("opening line is acquired");
    let InputLevel::Source(level) = command.input.levels.last_mut().expect("source level") else {
        panic!("source level expected");
    };
    let line = level.cursor.line.as_mut().expect("loaded line");
    line.byte_cursor = 4;

    let context =
        InputState::source_context_level(level, true, None, None, ErrorContextWidths::default())
            .expect("source context");
    assert_eq!(
        render_error_context(&[context], ErrorContextWidths::default(), 5),
        "\nl.1 left\n        }--^^M"
    );

    level
        .cursor
        .line
        .as_mut()
        .expect("loaded line")
        .endline_delivered = true;
    let context =
        InputState::source_context_level(level, true, None, None, ErrorContextWidths::default())
            .expect("consumed context");
    assert_eq!(
        render_error_context(&[context], ErrorContextWidths::default(), 5),
        "\nl.1 left^^M\n           }--"
    );

    let context = InputState::source_context_level(
        level,
        true,
        Some('\r'),
        None,
        ErrorContextWidths::default(),
    )
    .expect("matching live sentinel context");
    assert_eq!(
        render_error_context(&[context], ErrorContextWidths::default(), 5),
        "\nl.1 left\n        }--"
    );
}

#[test]
fn source_context_prints_physical_characters_through_live_newlinechar() {
    // TeX82 §§59/313: source pseudoprint calls `print(buffer[k])`; it does
    // not copy the physical line directly to the diagnostic sink.
    let mut command = CommandState::default();
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            b"leftYright".as_slice(),
        ))
        .expect("source registers");
    command
        .open_registered_source(source)
        .expect("source opens");
    command
        .prepare_started_input(13)
        .expect("opening line is acquired");
    let InputLevel::Source(level) = command.input.levels.last_mut().expect("source level") else {
        panic!("source level expected");
    };
    level.cursor.line.as_mut().expect("loaded line").byte_cursor = 5;

    let context = InputState::source_context_level(
        level,
        true,
        None,
        Some('Y'),
        ErrorContextWidths::default(),
    )
    .expect("source context");
    assert_eq!(
        render_error_context(&[context], ErrorContextWidths::default(), 5),
        "\nl.1 left\n\n         right^^M"
    );
}

#[test]
fn source_context_projects_recursive_superscript_reductions_from_live_buffer() {
    // TeX82 §§355/316: `qq1qM` is reduced in place first to `qqM`, then to
    // carriage return when `q` has superscript catcode. Error pseudoprint
    // observes that mutable buffer as `^^M`, not the immutable source bytes.
    let mut command = CommandState::default();
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            b"leftqq1qMright".as_slice(),
        ))
        .expect("source registers");
    command
        .open_registered_source(source)
        .expect("source opens");
    let mut queries = crate::input::CatcodeQueries(|code: crate::profile::CharacterCode| {
        if code.to_byte() == Ok(b'q') {
            tex_state::token::Catcode::Superscript
        } else if code.to_byte() == Ok(b'\r') {
            tex_state::token::Catcode::Active
        } else {
            tex_state::token::Catcode::Other
        }
    });
    for _ in 0..5 {
        let _ = command.next_exact_source_step(13, &mut queries);
    }

    let InputLevel::Source(level) = command.input.levels.last().expect("source level") else {
        panic!("source level expected");
    };
    let context =
        InputState::source_context_level(level, true, None, None, ErrorContextWidths::default())
            .expect("source context");
    assert_eq!(
        render_error_context(&[context], ErrorContextWidths::default(), 5),
        "\nl.1 left^^M\n           right^^M"
    );
}

#[test]
fn control_word_lookahead_does_not_cross_the_pseudoprint_cursor() {
    // TeX82 §§355--356 back `loc` up before the first nonletter. The command
    // core's source cursor projection keeps that still-unread spelling on the
    // second pseudoprint line, where the live newline character applies.
    let mut command = CommandState::default();
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            b"\\foo^^Ytail".as_slice(),
        ))
        .expect("source registers");
    command
        .open_registered_source(source)
        .expect("source opens");
    let mut queries =
        crate::input::CatcodeQueries(|code: crate::profile::CharacterCode| match code.to_byte() {
            Ok(b'\\') => tex_state::token::Catcode::Escape,
            Ok(b'^') => tex_state::token::Catcode::Superscript,
            Ok(b'a'..=b'z') => tex_state::token::Catcode::Letter,
            _ => tex_state::token::Catcode::Other,
        });
    let _ = command.next_exact_source_step(13, &mut queries);

    let InputLevel::Source(level) = command.input.levels.last().expect("source level") else {
        panic!("source level expected");
    };
    let context = InputState::source_context_level(
        level,
        true,
        None,
        Some('Y'),
        ErrorContextWidths::default(),
    )
    .expect("source context");
    assert_eq!(
        render_error_context(&[context], ErrorContextWidths::default(), 5),
        "\nl.1 \\foo\n        ^^\ntail^^M"
    );
}

#[test]
fn retained_v_template_pseudoprints_its_current_endtemplate_token() {
    // TeX82 §§315/354/375/780: the stored `frozen_end_template` is current
    // after raw delivery, then read once §375 replaces it with `frozen_endv`.
    // Umber's structural sentinel must cross the pseudoprint cursor at that
    // same retained-template lifecycle transition.
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let relax = universe.intern("A").symbol();
    let identity = command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(std::sync::Arc::<
            [tex_state::token::TracedTokenWord],
        >::from(vec![
            tex_state::token::TracedTokenWord::pack(
                tex_state::token::Token::Cs(relax),
                tex_state::token::OriginId::UNKNOWN,
            ),
        ]))),
        TokenBehavior::VTemplate,
        RetirementBehavior::RetainExhaustedVTemplate,
        ReplayTrace::VTemplate,
    );
    assert_eq!(
        command.output_open_context(&universe.command_context()),
        "\n<template> \n           \\A \\endtemplate "
    );
    let InputLevel::Tokens(cursor) = command.input.levels.last_mut().expect("v-template") else {
        panic!("token-list level expected");
    };
    cursor.frame.advance().expect("template cursor advances");
    assert_eq!(
        command.output_open_context(&universe.command_context()),
        "\n<template> \\A \n              \\endtemplate "
    );
    command
        .retire_exhausted_input(identity)
        .expect("template boundary is retained");

    assert_eq!(
        command.output_open_context(&universe.command_context()),
        "\n<template> \\A \\endtemplate \n                           "
    );

    let backup = command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(std::sync::Arc::from([
            tex_state::token::TracedTokenWord::pack(
                universe.command_context().frozen_end_template_token(),
                tex_state::token::OriginId::UNKNOWN,
            ),
        ]))),
        TokenBehavior::BackedUp(crate::input::BackupTreatment::Ordinary),
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let InputLevel::Tokens(cursor) = command.input.levels.last_mut().expect("backup") else {
        panic!("token-list level expected");
    };
    assert_eq!(cursor.identity(), backup);
    cursor.frame.advance().expect("backup cursor advances");
    assert_eq!(
        command.output_open_context(&universe.command_context()),
        "\n<recently read> \\endtemplate \n                             \n<template> \\A \\endtemplate \n                           "
    );
}

#[test]
fn token_context_pseudoprints_nul_in_control_sequence_names() {
    // TeX82 §§59/262/315 route token-list context through `print`, so a
    // control sequence's non-printable name bytes never reach the log raw.
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    universe.set_int_param(tex_state::env::banks::IntParam::NEWLINE_CHAR, -1);
    let symbol = universe.intern("a\0\0a").symbol();
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(std::sync::Arc::from([
            tex_state::token::TracedTokenWord::pack(
                tex_state::token::Token::Cs(symbol),
                tex_state::token::OriginId::UNKNOWN,
            ),
        ]))),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );

    let context = command.output_open_context(&universe.command_context());
    assert_eq!(
        context,
        "\n<to be read again> \n                   \\a^^@^^@a "
    );
    assert!(!context.contains('\0'));
}

#[test]
fn noexpand_backup_context_projects_frozen_marker() {
    // TeX82 §358 physically prefixes the operand with
    // `frozen_dont_expand`; §§258/315 therefore pseudoprint that marker even
    // though Umber represents its one-delivery effect structurally.
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let expandafter = universe.intern("expandafter").symbol();
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(std::sync::Arc::from([
            tex_state::token::TracedTokenWord::pack(
                tex_state::token::Token::Cs(expandafter),
                tex_state::token::OriginId::UNKNOWN,
            ),
        ]))),
        TokenBehavior::BackedUp(crate::input::BackupTreatment::SuppressExpandableControlSequence),
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    assert_eq!(
        command.output_open_context(&universe.command_context()),
        "\n<to be read again> \n                   \\notexpanded: \\expandafter "
    );
}

#[test]
fn large_unread_token_context_has_bounded_eager_projection() {
    const TOKEN_COUNT: usize = 65_536;
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let token = tex_state::token::TracedTokenWord::pack(
        tex_state::token::Token::Char {
            ch: 'x',
            cat: tex_state::token::Catcode::Other,
        },
        tex_state::token::OriginId::UNKNOWN,
    );
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(std::sync::Arc::from(
            vec![token; TOKEN_COUNT],
        ))),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::Transient(TransientReplayReason::ExpandedTokenList),
    );

    super::reset_token_context_projection_stats();
    let context = command.output_open_context(&universe.command_context());
    let stats = super::token_context_projection_stats();

    assert_eq!(
        context,
        format!("\n<token list> \n{}{}...", " ".repeat(13), "x".repeat(63))
    );
    assert_eq!(stats.visited_tokens, 67);
    assert_eq!(stats.peak_temporary_string_capacity, 66);
    assert!(
        stats.visited_tokens <= ErrorContextWidths::default().error_line()
            && stats.peak_temporary_string_capacity <= 256,
        "bounded unread projection visited {} of {TOKEN_COUNT} tokens and retained a peak temporary String capacity of {} bytes",
        stats.visited_tokens,
        stats.peak_temporary_string_capacity,
    );
}

#[test]
fn short_multibyte_control_sequence_context_is_byte_exact() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    universe.set_int_param(tex_state::env::banks::IntParam::NEWLINE_CHAR, -1);
    let symbol = universe.intern("β\0").symbol();
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(std::sync::Arc::from([
            tex_state::token::TracedTokenWord::pack(
                tex_state::token::Token::Cs(symbol),
                tex_state::token::OriginId::UNKNOWN,
            ),
            tex_state::token::TracedTokenWord::pack(
                tex_state::token::Token::Char {
                    ch: 'λ',
                    cat: tex_state::token::Catcode::Other,
                },
                tex_state::token::OriginId::UNKNOWN,
            ),
        ]))),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::Transient(TransientReplayReason::ExpandedTokenList),
    );

    super::reset_token_context_projection_stats();
    assert_eq!(
        command.output_open_context(&universe.command_context()),
        "\n<token list> \n             \\β^^@ λ"
    );
    assert_eq!(super::token_context_projection_stats().visited_tokens, 2);
}

#[test]
fn large_file_source_context_is_bounded_and_preserves_coordinates() {
    let mut command = CommandState::default();
    let mut bytes = b"left".to_vec();
    bytes.extend(std::iter::repeat_n(b'x', 65_536));
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            bytes,
        ))
        .expect("source registers");
    command
        .open_registered_source(source)
        .expect("source opens");
    command
        .prepare_started_input(13)
        .expect("opening line is acquired");
    let InputLevel::Source(level) = command.input.levels.last_mut().expect("source level") else {
        panic!("source level expected");
    };
    level.cursor.line.as_mut().expect("loaded line").byte_cursor = 4;

    let context =
        InputState::source_context_level(level, true, None, None, ErrorContextWidths::default())
            .expect("source context");
    assert_eq!(
        render_error_context(&[context], ErrorContextWidths::default(), 5),
        format!("\nl.1 left\n{}{}...", " ".repeat(8), "x".repeat(68))
    );
}

#[test]
fn context_projection_is_snapshot_neutral_and_rollback_exact() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let words = ['a', 'b'].map(|ch| {
        tex_state::token::TracedTokenWord::pack(
            tex_state::token::Token::Char {
                ch,
                cat: tex_state::token::Catcode::Other,
            },
            tex_state::token::OriginId::UNKNOWN,
        )
    });
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(std::sync::Arc::from(words))),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::Transient(TransientReplayReason::ExpandedTokenList),
    );
    let snapshot = command.snapshot();
    let before = command.output_open_context(&universe.command_context());
    let InputLevel::Tokens(cursor) = command.input.levels.last_mut().expect("token level") else {
        panic!("token level expected");
    };
    cursor.frame.advance().expect("token cursor advances");
    let advanced = command.output_open_context(&universe.command_context());
    assert_ne!(advanced, before);
    command.rollback(snapshot).expect("snapshot rolls back");
    assert_eq!(
        command.output_open_context(&universe.command_context()),
        before
    );
}

#[test]
fn silent_state_operations_do_not_project_token_context() {
    let command = CommandState::default();
    super::reset_token_context_projection_stats();
    let snapshot = command.snapshot();
    drop(snapshot);
    assert_eq!(
        super::token_context_projection_stats(),
        super::TokenContextProjectionStats::default()
    );
}
