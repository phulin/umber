use super::{
    ExpandablePrimitive, InternalInteger, Meaning, MeaningFlags, OPERAND_MASK, RawMeaning,
    UnexpandablePrimitive,
};
use crate::ids::{FontId, MacroDefinitionId};
use crate::page::{PageDimension, PageInteger};

fn round_trip(meaning: Meaning) {
    assert_eq!(Meaning::decode_stored(meaning.encode()), meaning);
}

#[test]
fn undefined_is_the_all_zero_word() {
    // Fresh zeroed meaning segments decode as Undefined, so this exact
    // encoding is required for fresh-segment correctness.
    assert_eq!(Meaning::Undefined.encode(), 0);
    assert_eq!(Meaning::decode_stored(0), Meaning::Undefined);
}

#[test]
fn meaning_variants_round_trip() {
    round_trip(Meaning::Undefined);
    round_trip(Meaning::Relax);
    round_trip(Meaning::Macro {
        flags: MeaningFlags::LONG
            | MeaningFlags::OUTER
            | MeaningFlags::PROTECTED
            | MeaningFlags::FROZEN,
        definition: MacroDefinitionId::new(0),
    });
    round_trip(Meaning::Macro {
        flags: MeaningFlags::EMPTY,
        definition: MacroDefinitionId::new(u32::MAX),
    });
    round_trip(Meaning::CharGiven('\0'));
    round_trip(Meaning::CharGiven(char::MAX));
    round_trip(Meaning::Font(FontId::new(0)));
    round_trip(Meaning::Font(FontId::new(u32::MAX)));
    round_trip(Meaning::ExpandablePrimitive(
        ExpandablePrimitive::ExpandAfter,
    ));
    round_trip(Meaning::ExpandablePrimitive(ExpandablePrimitive::NoExpand));
    round_trip(Meaning::ExpandablePrimitive(ExpandablePrimitive::CsName));
    round_trip(Meaning::ExpandablePrimitive(ExpandablePrimitive::EndCsName));
    round_trip(Meaning::ExpandablePrimitive(
        ExpandablePrimitive::EndTemplate,
    ));
    round_trip(Meaning::ExpandablePrimitive(ExpandablePrimitive::String));
    round_trip(Meaning::ExpandablePrimitive(ExpandablePrimitive::Number));
    round_trip(Meaning::ExpandablePrimitive(
        ExpandablePrimitive::RomanNumeral,
    ));
    round_trip(Meaning::ExpandablePrimitive(ExpandablePrimitive::Meaning));
    round_trip(Meaning::ExpandablePrimitive(ExpandablePrimitive::The));
    round_trip(Meaning::ExpandablePrimitive(ExpandablePrimitive::Input));
    round_trip(Meaning::ExpandablePrimitive(ExpandablePrimitive::EndInput));
    round_trip(Meaning::ExpandablePrimitive(ExpandablePrimitive::JobName));
    round_trip(Meaning::ExpandablePrimitive(ExpandablePrimitive::FontName));
    round_trip(Meaning::ExpandablePrimitive(ExpandablePrimitive::TopMark));
    round_trip(Meaning::ExpandablePrimitive(ExpandablePrimitive::FirstMark));
    round_trip(Meaning::ExpandablePrimitive(ExpandablePrimitive::BotMark));
    round_trip(Meaning::ExpandablePrimitive(
        ExpandablePrimitive::SplitFirstMark,
    ));
    round_trip(Meaning::ExpandablePrimitive(
        ExpandablePrimitive::SplitBotMark,
    ));
    round_trip(Meaning::ExpandablePrimitive(ExpandablePrimitive::Expanded));
    round_trip(Meaning::ExpandablePrimitive(ExpandablePrimitive::FileSize));
    round_trip(Meaning::ExpandablePrimitive(
        ExpandablePrimitive::CreationDate,
    ));
    round_trip(Meaning::ExpandablePrimitive(
        ExpandablePrimitive::IfInCsName,
    ));
    round_trip(Meaning::UnexpandablePrimitive(
        UnexpandablePrimitive::FutureLet,
    ));
    round_trip(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Mark));
    round_trip(Meaning::UnexpandablePrimitive(
        UnexpandablePrimitive::VAdjust,
    ));
    round_trip(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::HRule));
    round_trip(Meaning::UnexpandablePrimitive(
        UnexpandablePrimitive::LastSkip,
    ));
    round_trip(Meaning::UnexpandablePrimitive(
        UnexpandablePrimitive::PrevGraf,
    ));
    round_trip(Meaning::UnexpandablePrimitive(
        UnexpandablePrimitive::HAlign,
    ));
    round_trip(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::CrCr));
    round_trip(Meaning::PageDimension(PageDimension::Goal));
    round_trip(Meaning::PageDimension(PageDimension::FilllStretch));
    round_trip(Meaning::PageInteger(PageInteger::DeadCycles));
    round_trip(Meaning::PageInteger(PageInteger::InsertPenalties));
    round_trip(Meaning::MuGlueParam(17));
    round_trip(Meaning::Unknown(RawMeaning::testing_new(u8::MAX, 0)));
    round_trip(Meaning::Unknown(RawMeaning::testing_new(
        u8::MAX,
        OPERAND_MASK,
    )));
    round_trip(Meaning::Unknown(RawMeaning::testing_new_with_flags(
        u8::MAX,
        MeaningFlags::from_bits(0xa5),
        OPERAND_MASK,
    )));
}

#[test]
fn pdf_accessibility_operands_are_unique_and_follow_parallel_reservations() {
    let expected = [
        (247, UnexpandablePrimitive::PdfInterwordSpaceOn),
        (248, UnexpandablePrimitive::PdfInterwordSpaceOff),
        (249, UnexpandablePrimitive::PdfFakeSpace),
        (250, UnexpandablePrimitive::PdfSpaceFont),
    ];
    for (operand, primitive) in expected {
        assert_eq!(primitive.operand(), operand);
        assert_eq!(
            UnexpandablePrimitive::from_operand(operand),
            Some(primitive)
        );
        round_trip(Meaning::UnexpandablePrimitive(primitive));
    }

    let mut seen = std::collections::HashSet::new();
    for operand in 0..=250 {
        if let Some(primitive) = UnexpandablePrimitive::from_operand(operand) {
            assert_eq!(primitive.operand(), operand);
            assert!(
                seen.insert(primitive),
                "duplicate primitive at operand {operand}"
            );
        }
    }
    // 238..=246 belong to the accepted form/image branch. They may be absent
    // until that branch merges, but this slice must never claim them.
    for reserved in 238..=246 {
        assert!(expected.iter().all(|(operand, _)| *operand != reserved));
    }
}

#[test]
fn complete_primitive_codecs_are_unique_through_annotation_reservations() {
    let annotations = [
        (255, UnexpandablePrimitive::PdfAnnot),
        (256, UnexpandablePrimitive::PdfStartLink),
        (257, UnexpandablePrimitive::PdfEndLink),
        (258, UnexpandablePrimitive::PdfRunningLinkOn),
        (259, UnexpandablePrimitive::PdfRunningLinkOff),
    ];
    for (operand, primitive) in annotations {
        assert_eq!(primitive.operand(), operand);
        assert_eq!(
            UnexpandablePrimitive::from_operand(operand),
            Some(primitive)
        );
        round_trip(Meaning::UnexpandablePrimitive(primitive));
    }

    let mut unexpandable = std::collections::HashSet::new();
    for operand in 0..=259 {
        if let Some(primitive) = UnexpandablePrimitive::from_operand(operand) {
            assert_eq!(primitive.operand(), operand);
            assert!(
                unexpandable.insert(primitive),
                "duplicate unexpandable primitive at operand {operand}"
            );
        }
    }
    for reserved in 251..=254 {
        assert_eq!(UnexpandablePrimitive::from_operand(reserved), None);
    }

    let internals = [
        (17, InternalInteger::PdfLastAnnot),
        (18, InternalInteger::PdfLastLink),
    ];
    for (operand, integer) in internals {
        assert_eq!(integer.operand(), operand);
        assert_eq!(InternalInteger::from_operand(operand), Some(integer));
        round_trip(Meaning::InternalInteger(integer));
    }
    let mut internal = std::collections::HashSet::new();
    for operand in 0..=18 {
        if let Some(integer) = InternalInteger::from_operand(operand) {
            assert_eq!(integer.operand(), operand);
            assert!(
                internal.insert(integer),
                "duplicate internal integer at operand {operand}"
            );
        }
    }
    for reserved in 14..=16 {
        assert_eq!(InternalInteger::from_operand(reserved), None);
    }
}

#[test]
fn unknown_meaning_exposes_raw_parts_without_public_fields() {
    let flags = MeaningFlags::from_bits(0xa5);
    let word = Meaning::Unknown(RawMeaning::testing_new_with_flags(200, flags, 42)).encode();
    let Meaning::Unknown(raw) = Meaning::decode_stored(word) else {
        panic!("expected unknown meaning");
    };

    assert_eq!(raw.op(), 200);
    assert_eq!(raw.flags(), flags);
    assert_eq!(raw.operand(), 42);
    assert_eq!(Meaning::Unknown(raw).encode(), word);
}

#[test]
fn invalid_known_meaning_preserves_reserved_flags() {
    let flags = MeaningFlags::from_bits(0x80);
    let word = super::pack(super::OP_CHAR_GIVEN, flags, u64::from(u32::MAX));
    let Meaning::Unknown(raw) = Meaning::decode_stored(word) else {
        panic!("expected invalid character meaning to remain opaque");
    };

    assert_eq!(raw.flags(), flags);
    assert_eq!(Meaning::Unknown(raw).encode(), word);
}
