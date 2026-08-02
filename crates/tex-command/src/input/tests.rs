use tex_state::print::{ErrorContextLevel, ErrorContextWidths, render_error_context};

use crate::CommandState;
use crate::input::{InputLevel, InputState, RegisteredSourceKind, SourceRegistration};

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
