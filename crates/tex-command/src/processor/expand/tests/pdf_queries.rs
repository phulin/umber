//! PDF integer, string, image, mark, and comparison queries.

use super::*;

#[test]
fn nested_pdf_integer_queries_return_through_the_shared_number_lane() {
    crate::test_harness::with_universe(|universe| {
        let pdf_xform_name = install_static(
            universe,
            "pdfxformname",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::PdfXFormName),
        );
        let pdf_page_ref = install_static(
            universe,
            "pdfpageref",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::PdfPageRef),
        );
        let mut command = CommandState::new(CommandProfile::PDFTEX14029);
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(
            &mut command,
            [
                pdf_xform_name,
                Token::Char {
                    ch: '1',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: '/',
                    cat: Catcode::Other,
                },
                pdf_page_ref,
                Token::Char {
                    ch: '1',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: 'X',
                    cat: Catcode::Letter,
                },
            ],
        );
        assert_eq!(collect_expanded_characters(universe, &mut command), "0/0X");
    });
}

#[test]
fn pdf_ximage_bbox_uses_the_shared_two_integer_lane() {
    crate::test_harness::with_universe(|universe| {
        let pdf_ximage_bbox = install_static(
            universe,
            "pdfximagebbox",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::PdfXImageBBox),
        );
        let source = tex_state::PdfExternalImageSource {
            identity: tex_state::ContentHash::from_bytes(b"iterative pdfximagebbox image"),
            metadata: tex_state::PdfExternalImageMetadata::Raster(
                tex_state::PdfRasterImageMetadata::placeholder(),
            ),
            natural_width: tex_state::scaled::Scaled::from_raw(1),
            natural_height: tex_state::scaled::Scaled::from_raw(1),
            bytes: tex_state::SharedBytes::from(Vec::new()),
        };
        let image = universe
            .command_context()
            .expect("command context")
            .allocate_pdf_external_image(
                source,
                tex_state::PdfExternalImageDimensions {
                    width: tex_state::scaled::Scaled::from_raw(1),
                    height: tex_state::scaled::Scaled::from_raw(1),
                    depth: tex_state::scaled::Scaled::from_raw(0),
                },
                0,
                Vec::new(),
            )
            .expect("external image allocation");
        assert_eq!(image.id().raw(), 1);

        let number = install_static(
            universe,
            "number",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Number),
        );

        let mut command = CommandState::new(CommandProfile::PDFTEX14029);
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(
            &mut command,
            [
                pdf_ximage_bbox,
                Token::Char {
                    ch: '1',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: ' ',
                    cat: Catcode::Space,
                },
                number,
                Token::Char {
                    ch: '1',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: 'X',
                    cat: Catcode::Letter,
                },
            ],
        );
        assert_eq!(
            collect_expanded_characters(universe, &mut command),
            "0.0ptX"
        );
    });
}

#[test]
fn nested_pdf_string_projections_use_the_shared_collector_lane() {
    crate::test_harness::with_universe(|universe| {
        let escape_string = install_static(
            universe,
            "pdfescapestring",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::PdfEscapeString),
        );
        let escape_hex = install_static(
            universe,
            "pdfescapehex",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::PdfEscapeHex),
        );
        let unescape_hex = install_static(
            universe,
            "pdfunescapehex",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::PdfUnescapeHex),
        );
        let mut command = CommandState::new(CommandProfile::PDFTEX14029);
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(
            &mut command,
            [
                escape_string,
                Token::Char {
                    ch: '{',
                    cat: Catcode::BeginGroup,
                },
                Token::Char {
                    ch: 'a',
                    cat: Catcode::Letter,
                },
                Token::Char {
                    ch: '(',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: 'b',
                    cat: Catcode::Letter,
                },
                Token::Char {
                    ch: ')',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: '}',
                    cat: Catcode::EndGroup,
                },
                Token::Char {
                    ch: '/',
                    cat: Catcode::Other,
                },
                escape_hex,
                Token::Char {
                    ch: '{',
                    cat: Catcode::BeginGroup,
                },
                Token::Char {
                    ch: 'A',
                    cat: Catcode::Letter,
                },
                Token::Char {
                    ch: 'b',
                    cat: Catcode::Letter,
                },
                Token::Char {
                    ch: '}',
                    cat: Catcode::EndGroup,
                },
                Token::Char {
                    ch: '/',
                    cat: Catcode::Other,
                },
                unescape_hex,
                Token::Char {
                    ch: '{',
                    cat: Catcode::BeginGroup,
                },
                Token::Char {
                    ch: '4',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: '1',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: '4',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: '2',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: '}',
                    cat: Catcode::EndGroup,
                },
                Token::Char {
                    ch: 'X',
                    cat: Catcode::Letter,
                },
            ],
        );
        assert_eq!(
            collect_expanded_characters(universe, &mut command),
            "a\\(b\\)/4162/ABX"
        );
    });
}

#[test]
fn numbered_marks_use_the_shared_integer_lane() {
    crate::test_harness::with_universe(|universe| {
        let top_marks = install_static(
            universe,
            "topmarks",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::TopMarks),
        );
        let number = install_static(
            universe,
            "number",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Number),
        );
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(
            &mut command,
            [
                top_marks,
                number,
                Token::Char {
                    ch: '0',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: 'X',
                    cat: Catcode::Letter,
                },
            ],
        );
        assert_eq!(collect_expanded_characters(universe, &mut command), "X");
    });
}

#[test]
fn string_compare_uses_two_shared_collector_phases() {
    crate::test_harness::with_universe(|universe| {
        let string_compare = install_static(
            universe,
            "stringcompare",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::StringCompare),
        );
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(
            &mut command,
            [
                string_compare,
                Token::Char {
                    ch: '{',
                    cat: Catcode::BeginGroup,
                },
                Token::Char {
                    ch: 'a',
                    cat: Catcode::Letter,
                },
                Token::Char {
                    ch: '}',
                    cat: Catcode::EndGroup,
                },
                Token::Char {
                    ch: '{',
                    cat: Catcode::BeginGroup,
                },
                Token::Char {
                    ch: 'b',
                    cat: Catcode::Letter,
                },
                Token::Char {
                    ch: '}',
                    cat: Catcode::EndGroup,
                },
                Token::Char {
                    ch: 'X',
                    cat: Catcode::Letter,
                },
            ],
        );
        assert_eq!(collect_expanded_characters(universe, &mut command), "-1X");
    });
}

#[test]
fn pdf_last_match_uses_the_shared_number_lane_at_end_of_input() {
    crate::test_harness::with_universe(|universe| {
        let pdf_last_match = install_static(
            universe,
            "pdflastmatch",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::PdfLastMatch),
        );
        let mut command = CommandState::new(CommandProfile::PDFTEX14029);
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(
            &mut command,
            [
                pdf_last_match,
                Token::Char {
                    ch: '1',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: 'X',
                    cat: Catcode::Letter,
                },
            ],
        );
        assert_eq!(collect_expanded_characters(universe, &mut command), "-1->X");
    });
}
