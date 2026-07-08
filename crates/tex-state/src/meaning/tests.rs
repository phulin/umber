use super::{
    ExpandablePrimitive, Meaning, MeaningFlags, OPERAND_MASK, RawMeaning, UnexpandablePrimitive,
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
    round_trip(Meaning::PageDimension(PageDimension::Goal));
    round_trip(Meaning::PageDimension(PageDimension::FilllStretch));
    round_trip(Meaning::PageInteger(PageInteger::DeadCycles));
    round_trip(Meaning::PageInteger(PageInteger::InsertPenalties));
    round_trip(Meaning::Unknown(RawMeaning::testing_new(u8::MAX, 0)));
    round_trip(Meaning::Unknown(RawMeaning::testing_new(
        u8::MAX,
        OPERAND_MASK,
    )));
}

#[test]
fn unknown_meaning_exposes_raw_parts_without_public_fields() {
    let word = Meaning::Unknown(RawMeaning::testing_new(200, 42)).encode();
    let Meaning::Unknown(raw) = Meaning::decode_stored(word) else {
        panic!("expected unknown meaning");
    };

    assert_eq!(raw.op(), 200);
    assert_eq!(raw.operand(), 42);
    assert_eq!(Meaning::Unknown(raw).encode(), word);
}
