use std::sync::Arc;

use tex_state::SourceId;

use super::{LineTerminator, SourceLineState, SourceLocation, SourceProvenance, SourceRange};
use crate::input::source::{
    RegisteredSource, RegisteredSourceKind, SourceCursor, SourceRegistration,
};
use crate::profile::{CharacterCode, CharacterMode, CommandProfile};

fn cursor(mode: CharacterMode, bytes: &[u8]) -> SourceCursor {
    let profile = match mode {
        CharacterMode::EightBitExact => CommandProfile::TEX82,
        CharacterMode::UnicodeExtended => {
            CommandProfile::unicode_extended(crate::CommandDialect::Tex82)
        }
    };
    let source = RegisteredSource::register(
        SourceId::new(5),
        profile,
        SourceRegistration::new(RegisteredSourceKind::Generated, Arc::<[u8]>::from(bytes)),
    )
    .expect("test backing is valid for selected mode");
    SourceCursor::new(source)
}

#[test]
fn compact_source_provenance_preserves_every_coordinate_and_option_niche() {
    let source = SourceId::new(u32::MAX);
    let range = SourceRange::new(source, u64::MAX - 9, u64::MAX - 2);
    let location = SourceLocation::new(source, u64::MAX - 4);
    let provenance = SourceProvenance::from_range_and_location(range, location);

    assert_eq!(provenance.range(), range);
    assert_eq!(provenance.location(), location);
    assert_eq!(core::mem::size_of::<SourceProvenance>(), 32);
    assert_eq!(core::mem::size_of::<Option<SourceProvenance>>(), 32);
}

fn drain(
    line: &mut SourceLineState,
    mode: CharacterMode,
    bytes: &[u8],
) -> Vec<super::SourceCharacter> {
    std::iter::from_fn(|| line.next_character(mode, bytes)).collect()
}

#[test]
fn splits_lf_cr_crlf_and_missing_final_terminators_exactly() {
    let mut cursor = cursor(CharacterMode::EightBitExact, b"lf\ncr\rcrlf\r\nfinal");
    let expected = [
        (0..2, 2..3, LineTerminator::Lf),
        (3..5, 5..6, LineTerminator::Cr),
        (6..10, 10..12, LineTerminator::CrLf),
        (12..17, 17..17, LineTerminator::Missing),
    ];

    for (index, (content, terminator, kind)) in expected.into_iter().enumerate() {
        let line = cursor.load_next_line(-1).expect("physical line");
        assert_eq!(line.physical.number(), index as u64 + 1);
        assert_eq!(line.physical.content_range().start(), content.start);
        assert_eq!(line.physical.content_range().end(), content.end);
        assert_eq!(line.physical.terminator_range().start(), terminator.start);
        assert_eq!(line.physical.terminator_range().end(), terminator.end);
        assert_eq!(line.physical.terminator(), kind);
        cursor.finish_line();
    }
    assert!(cursor.load_next_line(-1).is_none());
}

#[test]
fn a_final_terminator_does_not_create_an_extra_physical_line() {
    for bytes in [&b"a\n"[..], &b"a\r"[..], &b"a\r\n"[..]] {
        let mut cursor = cursor(CharacterMode::EightBitExact, bytes);
        assert!(cursor.load_next_line(13).is_some());
        cursor.finish_line();
        assert!(cursor.load_next_line(13).is_none());
    }
}

#[test]
fn source_line_state_stays_at_the_pre_transition_layout() {
    assert_eq!(std::mem::size_of::<super::SourceLineState>(), 128);
    assert_eq!(std::mem::size_of::<super::SourceLexCursor>(), 24);
}

#[test]
fn empty_and_consecutive_terminators_have_exact_blank_line_behavior() {
    let mut empty = cursor(CharacterMode::EightBitExact, b"");
    assert!(empty.load_next_line(13).is_none());

    let mut cursor = cursor(CharacterMode::EightBitExact, b"\n\r\r\n");
    for expected in [LineTerminator::Lf, LineTerminator::Cr, LineTerminator::CrLf] {
        let line = cursor.load_next_line(13).expect("blank physical line");
        assert!(line.physical.content_range().is_empty());
        assert_eq!(line.physical.terminator(), expected);
        cursor.finish_line();
    }
    assert!(cursor.load_next_line(13).is_none());
}

#[test]
fn exact_byte_line_strips_spaces_and_anchors_synthetic_endline() {
    let mut cursor = cursor(CharacterMode::EightBitExact, b"a  \r\n");
    let bytes = cursor.backing.bytes.clone();
    let line = cursor.load_next_line(13).expect("line");
    let chars = drain(line, CharacterMode::EightBitExact, &bytes);

    assert_eq!(chars.len(), 2);
    assert_eq!(chars[0].code, CharacterCode::from_byte(b'a'));
    assert_eq!((chars[0].range.start(), chars[0].range.end()), (0, 1));
    assert!(!chars[0].synthetic);
    assert_eq!(chars[1].code, CharacterCode::from_byte(13));
    assert_eq!((chars[1].range.start(), chars[1].range.end()), (1, 1));
    assert!(chars[1].synthetic);
    assert_eq!(chars[1].scalar_offset, 1);
}

#[test]
fn unicode_cursor_decodes_scalars_but_preserves_utf8_byte_ranges() {
    let text = "é𐐀  \n";
    let mut cursor = cursor(CharacterMode::UnicodeExtended, text.as_bytes());
    let bytes = cursor.backing.bytes.clone();
    let line = cursor.load_next_line(0x03c0).expect("line");
    let chars = drain(line, CharacterMode::UnicodeExtended, &bytes);

    assert_eq!(chars.len(), 3);
    assert_eq!(chars[0].code, CharacterCode::from('é'));
    assert_eq!((chars[0].range.start(), chars[0].range.end()), (0, 2));
    assert_eq!(chars[0].scalar_offset, 0);
    assert_eq!(chars[1].code, CharacterCode::from('𐐀'));
    assert_eq!((chars[1].range.start(), chars[1].range.end()), (2, 6));
    assert_eq!(chars[1].scalar_offset, 1);
    assert_eq!(chars[2].code, CharacterCode::from('π'));
    assert_eq!((chars[2].range.start(), chars[2].range.end()), (6, 6));
    assert_eq!(chars[2].scalar_offset, 2);
}

#[test]
fn unicode_ranges_cover_every_utf8_width_and_scalar_position() {
    let text = "\0¢€𐀀";
    let mut cursor = cursor(CharacterMode::UnicodeExtended, text.as_bytes());
    let bytes = cursor.backing.bytes.clone();
    let line = cursor.load_next_line(-1).expect("line");
    let chars = drain(line, CharacterMode::UnicodeExtended, &bytes);

    let expected = [
        ('\0', 0, 1, 0),
        ('¢', 1, 3, 1),
        ('€', 3, 6, 2),
        ('𐀀', 6, 10, 3),
    ];
    for (character, (scalar, start, end, scalar_offset)) in chars.into_iter().zip(expected) {
        assert_eq!(character.code(), CharacterCode::from(scalar));
        assert_eq!(
            (character.range().start(), character.range().end()),
            (start, end)
        );
        assert_eq!(character.scalar_offset(), scalar_offset);
        assert!(!character.is_synthetic());
    }
}

#[test]
fn endlinechar_validation_is_profile_specific() {
    let cases = [
        (CharacterMode::EightBitExact, -1, false),
        (CharacterMode::EightBitExact, 255, true),
        (CharacterMode::EightBitExact, 256, false),
        (CharacterMode::UnicodeExtended, 0x10ffff, true),
        (CharacterMode::UnicodeExtended, 0xd800, false),
        (CharacterMode::UnicodeExtended, -1, false),
    ];
    for (mode, endlinechar, expected) in cases {
        let mut cursor = cursor(mode, b"x");
        let line = cursor.load_next_line(endlinechar).expect("final line");
        assert_eq!(line.endline.is_some(), expected);
    }
}
