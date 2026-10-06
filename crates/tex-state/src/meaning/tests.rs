use super::{
    ExpandablePrimitive, InternalInteger, Meaning, MeaningFlags, MeaningWord, OPERAND_MASK,
    RawMeaning, ResolvedMeaning, UnexpandablePrimitive,
};
use crate::generation::with_generation;
use crate::ids::FontId;
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
fn scalar_words_keep_their_value_when_opcodes_overlap_coordinate_commands() {
    for opcode in [super::OP_MACRO, super::OP_FONT, u8::MAX] {
        let raw =
            RawMeaning::testing_new_with_flags(opcode, MeaningFlags::from_bits(0xa5), OPERAND_MASK);
        let scalar =
            ((opcode as u64) << super::OPCODE_SHIFT) | (0xa5 << super::FLAGS_SHIFT) | OPERAND_MASK;
        let word = MeaningWord::<()>::scalar(scalar);
        assert_eq!(word.scalar_word(), Some(scalar));
        assert!(!word.is_macro());
        assert!(!word.is_font());
        assert_eq!(
            word.resolve(),
            ResolvedMeaning::Static(Meaning::decode_stored(Meaning::Unknown(raw).encode()))
        );
    }
}

#[test]
fn native_dispatch_preserves_scalar_domain_and_semantic_classification() {
    for opcode in 0..=u8::MAX {
        for flags in [0_u64, 1, 8, 0x80, 0xa5, 0xff] {
            for operand in [
                0,
                1,
                255,
                256,
                265,
                MeaningWord::<()>::EMPTY_MACRO_KEY - super::UNEXPANDABLE_CONTROL_BASE - 2,
                MeaningWord::<()>::EMPTY_MACRO_KEY - super::UNEXPANDABLE_CONTROL_BASE - 1,
                65_006,
                65_007,
                OPERAND_MASK,
                ExpandablePrimitive::EndTemplate as u64,
            ] {
                let scalar = ((opcode as u64) << super::OPCODE_SHIFT)
                    | (flags << super::FLAGS_SHIFT)
                    | operand;
                let native = MeaningWord::<()>::scalar(scalar);
                assert_eq!(native.scalar_word(), Some(scalar));
                assert!(!native.is_macro());
                assert!(!native.is_font());
                assert_eq!(native.is_relax(), opcode == super::OP_RELAX);
                assert_eq!(native.is_undefined(), opcode == super::OP_UNDEFINED);
                assert!(native.execution_key() <= u16::MAX as u64);
                assert_eq!(native.is_outer(), scalar == Meaning::END_TEMPLATE_WORD);
                // Noexpand freezes undefined and expandable control meanings,
                // including unknown primitive ids, while preserving endcsname.
                assert_eq!(
                    native.suppressed_by_noexpand(),
                    opcode == super::OP_UNDEFINED
                        || (opcode == super::OP_EXPANDABLE_PRIMITIVE
                            && ExpandablePrimitive::from_operand(operand)
                                != Some(ExpandablePrimitive::EndCsName))
                );
                let primitive = if opcode == super::OP_EXPANDABLE_PRIMITIVE {
                    ExpandablePrimitive::from_operand(operand)
                } else {
                    None
                };
                assert_eq!(native.expandable_primitive(), primitive);
                if let Some(primitive) = primitive {
                    assert_eq!(
                        native.execution_key(),
                        Meaning::runtime_word_control_code(
                            Meaning::ExpandablePrimitive(primitive).encode()
                        )
                    );
                }
                let primitive = if opcode == super::OP_UNEXPANDABLE_PRIMITIVE {
                    UnexpandablePrimitive::from_operand(operand)
                } else {
                    None
                };
                assert_eq!(native.unexpandable_primitive(), primitive);
                if let Some(primitive) = primitive {
                    assert_eq!(
                        native.execution_key(),
                        Meaning::runtime_word_control_code(
                            Meaning::UnexpandablePrimitive(primitive).encode()
                        )
                    );
                }
                assert_eq!(native.character(), Meaning::runtime_word_character(scalar));
                if let Some((ch, cat)) = native.character() {
                    assert_eq!(
                        native.execution_key(),
                        Meaning::runtime_word_control_code(Meaning::CharToken { ch, cat }.encode())
                    );
                }
            }
        }
    }
}

#[test]
fn macro_meanings_carry_only_generation_typed_definition_ids() {
    with_generation(|mut generation| {
        let definition = generation
            .definitions_mut()
            .allocate(&[], &[])
            .expect("test fixture is valid");
        let definition_publication = definition;
        let definition = definition_publication.reference();
        let flags = MeaningFlags::LONG
            | MeaningFlags::OUTER
            | MeaningFlags::PROTECTED
            | MeaningFlags::FROZEN;
        let word = definition_publication.meaning(flags);
        assert_eq!(word.resolve(), ResolvedMeaning::Macro { flags, definition });
        assert!(word.suppressed_by_noexpand());
    });
}

#[test]
fn pdf_accessibility_operands_are_unique_and_follow_parallel_reservations() {
    // Operand 209 was the former registry placeholder. It never represented
    // a pdfTeX command and must not become a meaning through format decoding.
    assert_eq!(UnexpandablePrimitive::from_operand(209), None);

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

    let mut expandable = Vec::new();
    for operand in 0..=85 {
        let value =
            ExpandablePrimitive::from_operand(operand).expect("contiguous expandable operand");
        assert_eq!(value.operand(), operand);
        assert!(
            !expandable.contains(&value),
            "duplicate expandable operand {operand}"
        );
        expandable.push(value);
    }
    // 238..=246 belong to the accepted form/image branch. They may be absent
    // until that branch merges, but this slice must never claim them.
    for reserved in 238..=246 {
        assert!(expected.iter().all(|(operand, _)| *operand != reserved));
    }
}

#[test]
fn complete_primitive_codecs_are_unique_through_navigation_and_compatibility_registrations() {
    let annotations = [
        (255, UnexpandablePrimitive::PdfAnnot),
        (256, UnexpandablePrimitive::PdfStartLink),
        (257, UnexpandablePrimitive::PdfEndLink),
        (258, UnexpandablePrimitive::PdfRunningLinkOn),
        (259, UnexpandablePrimitive::PdfRunningLinkOff),
        (260, UnexpandablePrimitive::PdfOutline),
        (261, UnexpandablePrimitive::PdfDest),
        (262, UnexpandablePrimitive::PdfThread),
        (263, UnexpandablePrimitive::PdfStartThread),
        (264, UnexpandablePrimitive::PdfEndThread),
    ];
    for (operand, primitive) in annotations {
        assert_eq!(primitive.operand(), operand);
        assert_eq!(
            UnexpandablePrimitive::from_operand(operand),
            Some(primitive)
        );
        round_trip(Meaning::UnexpandablePrimitive(primitive));
    }
    let images = [
        (253, UnexpandablePrimitive::PdfXImage),
        (254, UnexpandablePrimitive::PdfRefXImage),
    ];
    for (operand, primitive) in images {
        assert_eq!(primitive.operand(), operand);
        assert_eq!(
            UnexpandablePrimitive::from_operand(operand),
            Some(primitive)
        );
        round_trip(Meaning::UnexpandablePrimitive(primitive));
    }

    let mut unexpandable = std::collections::HashSet::new();
    for operand in 0..=266 {
        if let Some(primitive) = UnexpandablePrimitive::from_operand(operand) {
            assert_eq!(primitive.operand(), operand);
            assert!(
                unexpandable.insert(primitive),
                "duplicate unexpandable primitive at operand {operand}"
            );
        }
    }
    assert_eq!(UnexpandablePrimitive::QuitVMode.operand(), 265);
    assert_eq!(
        UnexpandablePrimitive::from_operand(265),
        Some(UnexpandablePrimitive::QuitVMode)
    );
    round_trip(Meaning::UnexpandablePrimitive(
        UnexpandablePrimitive::QuitVMode,
    ));
    let internals = [
        (17, InternalInteger::PdfLastAnnot),
        (18, InternalInteger::PdfLastLink),
        (21, InternalInteger::PdfLastXImage),
        (22, InternalInteger::PdfReturnValue),
        (23, InternalInteger::PdfLastXImagePages),
        (24, InternalInteger::PdfLastXImageColorDepth),
    ];
    for (operand, integer) in internals {
        assert_eq!(integer.operand(), operand);
        assert_eq!(InternalInteger::from_operand(operand), Some(integer));
        round_trip(Meaning::InternalInteger(integer));
    }
    let mut internal = std::collections::HashSet::new();
    for reserved in 19..=20 {
        assert_eq!(InternalInteger::from_operand(reserved), None);
    }
    for operand in 0..=24 {
        if let Some(integer) = InternalInteger::from_operand(operand) {
            assert_eq!(integer.operand(), operand);
            assert!(
                internal.insert(integer),
                "duplicate internal integer at operand {operand}"
            );
        }
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

#[test]
fn control_dispatch_keys_distinguish_all_native_families_and_selectors() {
    let mut keys = std::collections::HashSet::new();
    for meaning in [
        Meaning::Undefined,
        Meaning::Relax,
        Meaning::CharGiven('x'),
        Meaning::MathCharGiven(1),
        Meaning::CountRegister(1),
        Meaning::DimenRegister(1),
        Meaning::SkipRegister(1),
        Meaning::MuskipRegister(1),
        Meaning::ToksRegister(1),
        Meaning::IntParam(1),
        Meaning::DimenParam(1),
        Meaning::GlueParam(1),
        Meaning::MuGlueParam(1),
        Meaning::TokParam(1),
        Meaning::PageDimension(PageDimension::Goal),
        Meaning::PageInteger(PageInteger::DeadCycles),
        Meaning::InternalInteger(InternalInteger::from_operand(0).expect("internal integer")),
        Meaning::Font(FontId::new(0)),
        Meaning::EndV,
        Meaning::Unknown(RawMeaning::testing_new(u8::MAX, OPERAND_MASK)),
    ] {
        assert!(
            keys.insert(Meaning::runtime_word_control_code(meaning.encode())),
            "{meaning:?}"
        );
    }
    for cat in 0..16 {
        let word =
            Meaning::char_token_word('x', crate::token::Catcode::from_raw(cat).expect("catcode"));
        assert!(
            keys.insert(Meaning::runtime_word_control_code(word)),
            "catcode {cat}"
        );
    }
    for operand in 0..=u8::MAX {
        if let Some(primitive) = ExpandablePrimitive::from_operand(u64::from(operand)) {
            let word = Meaning::ExpandablePrimitive(primitive).encode();
            assert!(
                keys.insert(Meaning::runtime_word_control_code(word)),
                "{primitive:?}"
            );
        }
    }
    for operand in 0..=u16::MAX {
        if let Some(primitive) = UnexpandablePrimitive::from_operand(u64::from(operand)) {
            let word = Meaning::UnexpandablePrimitive(primitive).encode();
            assert!(
                keys.insert(Meaning::runtime_word_control_code(word)),
                "{primitive:?}"
            );
        }
    }
    for (left, right) in [
        (Meaning::CountRegister(0), Meaning::CountRegister(u16::MAX)),
        (Meaning::MathCharGiven(0), Meaning::MathCharGiven(u16::MAX)),
        (Meaning::CharGiven('x'), Meaning::CharGiven(char::MAX)),
        (
            Meaning::Font(FontId::new(0)),
            Meaning::Font(FontId::new(u32::MAX)),
        ),
    ] {
        assert_eq!(
            Meaning::runtime_word_control_code(left.encode()),
            Meaning::runtime_word_control_code(right.encode()),
            "values belong to their semantic consumers"
        );
    }
}
