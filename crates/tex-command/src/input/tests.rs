use tex_state::ExpansionState;
use tex_state::print::{ErrorContextLevel, ErrorContextWidths, render_error_context};

use crate::CommandState;
use crate::input::{
    InputLevel, InputState, RegisteredSourceKind, ReplayTrace, RetirementBehavior,
    SharedTokenBuffer, SourceRegistration, TokenBehavior, TokenPayload,
};

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
        InputState::source_context_level(level, true, None, None).expect("source context");
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
        InputState::source_context_level(level, true, None, None).expect("consumed context");
    assert_eq!(
        render_error_context(&[context], ErrorContextWidths::default(), 5),
        "\nl.1 left^^M\n           }--"
    );

    let context = InputState::source_context_level(level, true, Some('\r'), None)
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

    let context =
        InputState::source_context_level(level, true, None, Some('Y')).expect("source context");
    assert_eq!(
        render_error_context(&[context], ErrorContextWidths::default(), 5),
        "\nl.1 left\n\n         right^^M"
    );
}

#[test]
fn retained_v_template_pseudoprints_its_current_endtemplate_token() {
    // TeX82 §§354/390: `get_next` returns `frozen_end_template` when the
    // v-template's stored list is exhausted. Although `loc=null`, §315 shows
    // that synthetic current token after the cursor until `do_endv` finishes.
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
    let InputLevel::Tokens(cursor) = command.input.levels.last_mut().expect("v-template") else {
        panic!("token-list level expected");
    };
    cursor.index = 1;
    assert_eq!(
        command.output_open_context(&universe.command_context()),
        "\n<template> \\A \n              \\endtemplate "
    );
    command
        .retire_exhausted_input(identity)
        .expect("template boundary is retained");

    assert_eq!(
        command.output_open_context(&universe.command_context()),
        "\n<template> \\A \n              \\endtemplate "
    );

    let backup = command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(std::sync::Arc::from([
            tex_state::token::TracedTokenWord::pack(
                universe.frozen_end_template_token(),
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
    assert_eq!(cursor.identity, backup);
    cursor.index = 1;
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
