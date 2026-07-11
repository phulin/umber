use crate as tex_fonts;
use tex_arith::Scaled;
use tex_fonts::{CharacterTag, FontParameterKind, LigKernAction, ParseError, TfmFont, TfmTable};
use tex_fonts::{LigKernChar, LigKernCommand};

const CMR10: &[u8] = include_bytes!("../../tests/fixtures/cm/cmr10.tfm");
const CMMI10: &[u8] = include_bytes!("../../tests/fixtures/cm/cmmi10.tfm");
const CMSY10: &[u8] = include_bytes!("../../tests/fixtures/cm/cmsy10.tfm");
const CMEX10: &[u8] = include_bytes!("../../tests/fixtures/cm/cmex10.tfm");
const CMTT10: &[u8] = include_bytes!("../../tests/fixtures/cm/cmtt10.tfm");
const BOUNDARY_CHAR: &[u8] = include_bytes!("../../tests/fixtures/edge/boundary-char.tfm");
const LONG_JUMP: &[u8] = include_bytes!("../../tests/fixtures/edge/ptmr8g-longjump.tfm");

#[test]
fn parses_required_computer_modern_corpus() {
    let corpus = [
        ("cmr10", CMR10, 7usize, false),
        ("cmmi10", CMMI10, 7usize, false),
        ("cmsy10", CMSY10, 22usize, false),
        ("cmex10", CMEX10, 13usize, true),
        ("cmtt10", CMTT10, 7usize, false),
    ];

    for (name, bytes, expected_params, has_extensible) in corpus {
        let font = parse(bytes);
        assert_eq!(font.header.design_size.raw(), 10 * Scaled::UNITY, "{name}");
        assert_eq!(font.font_size.raw(), 10 * Scaled::UNITY, "{name}");
        assert_eq!(font.parameters.values.len(), expected_params, "{name}");
        assert_eq!(
            font.parameters.get(1).map(|p| p.kind),
            Some(FontParameterKind::SlantRatio)
        );
        assert_eq!(
            font.parameters.get(2).map(|p| p.kind),
            Some(FontParameterKind::Dimension)
        );
        assert_eq!(
            !font.extensible_recipes.is_empty(),
            has_extensible,
            "{name} extensible recipe presence"
        );
        assert!(font.characters.iter().flatten().count() > 50, "{name}");
        assert_eq!(font.widths[0].raw(), 0, "{name}");
        assert_eq!(font.heights[0].raw(), 0, "{name}");
        assert_eq!(font.depths[0].raw(), 0, "{name}");
        assert_eq!(font.italic_corrections[0].raw(), 0, "{name}");
    }

    let cmr = parse(CMR10);
    let f = char_metric(&cmr, b'f');
    assert!(matches!(f.tag, CharacterTag::LigKern { .. }));
    assert!(
        cmr.lig_kern_program
            .iter()
            .any(|step| matches!(step.action, Some(LigKernAction::Ligature(_))))
    );
    assert!(
        cmr.lig_kern_program
            .iter()
            .any(|step| matches!(step.action, Some(LigKernAction::Kern(_))))
    );

    let cmex = parse(CMEX10);
    assert!(
        cmex.characters
            .iter()
            .flatten()
            .any(|ch| matches!(ch.tag, CharacterTag::Extensible(_)))
    );
    assert!(!cmex.parameters.math_parameters().is_empty());

    let cmsy = parse(CMSY10);
    assert_eq!(cmsy.parameters.math_parameters().len(), 15);
}

#[test]
fn parses_real_boundary_char_and_long_jump_encodings() {
    let boundary = parse(BOUNDARY_CHAR);
    assert_eq!(boundary.right_boundary_char, Some(b' '));
    assert_eq!(boundary.header.seven_bit_safe, Some(true));

    let long_jump = parse(LONG_JUMP);
    assert!(
        long_jump
            .lig_kern_program
            .iter()
            .any(|step| step.skip_byte > 128 && step.restart_index.is_some())
    );
    assert!(long_jump.characters.iter().flatten().any(|ch| {
        matches!(
            ch.tag,
            CharacterTag::LigKern {
                program_index,
                start_index
            } if start_index > u16::from(program_index)
        )
    }));
}

#[test]
fn kernel_metrics_api_exposes_chars_lig_kerns_boundaries_and_recipes() {
    let cmr = parse(CMR10);
    let metrics = cmr.font_metrics();
    let f = metrics.character(b'f').expect("f metric");
    assert_eq!(f.width.raw(), char_metric(&cmr, b'f').width.raw());
    assert!(metrics.char_exists(b'A'));
    assert!(!metrics.char_exists(255));
    assert!(matches!(
        metrics.lig_kern_command(LigKernChar::Char(b'f'), LigKernChar::Char(b'i')),
        Some(LigKernCommand::Ligature(ligature)) if ligature.replacement == 0o14
            && ligature.delete_current
            && ligature.delete_next
            && ligature.pass_over == 0
    ));
    assert!(matches!(
        metrics.lig_kern_command(LigKernChar::Char(b'T'), LigKernChar::Char(b'o')),
        Some(LigKernCommand::Kern(amount)) if amount.raw() < 0
    ));

    let cmex = parse(CMEX10);
    let cmex_metrics = cmex.font_metrics();
    let extensible = cmex
        .characters
        .iter()
        .flatten()
        .find(|character| matches!(character.tag, CharacterTag::Extensible(_)))
        .expect("cmex extensible character");
    let CharacterTag::Extensible(recipe_index) = extensible.tag else {
        unreachable!("find restricts tag");
    };
    let recipe = cmex_metrics
        .extensible_recipe(extensible.code)
        .expect("extensible recipe");
    assert_eq!(
        recipe.repeated,
        cmex.extensible_recipes[usize::from(recipe_index)].repeated
    );

    let boundary = parse(&tfm_with_sections(Sections {
        bc: b'A',
        ec: b'B',
        char_info: vec![[1, 0, 1, 1], [1, 0, 0, 0]],
        widths: vec![[0, 0, 0, 0], [0, 8, 0, 0]],
        heights: vec![[0, 0, 0, 0]],
        depths: vec![[0, 0, 0, 0]],
        italics: vec![[0, 0, 0, 0]],
        lig_kerns: vec![[255, b' ', 0, 0], [128, b' ', 0, b'B'], [255, 0, 0, 1]],
        kerns: Vec::new(),
        extensibles: Vec::new(),
        params: Vec::new(),
    }));
    let boundary_metrics = boundary.font_metrics();
    assert!(matches!(
        boundary_metrics.lig_kern_command(LigKernChar::Char(b'A'), LigKernChar::Boundary),
        Some(LigKernCommand::Ligature(ligature)) if ligature.replacement == b'B'
    ));
    assert!(matches!(
        boundary_metrics.lig_kern_command(LigKernChar::Boundary, LigKernChar::Boundary),
        Some(LigKernCommand::Ligature(ligature)) if ligature.replacement == b'B'
    ));
}

#[test]
fn malformed_files_return_specific_errors() {
    assert!(matches!(
        TfmFont::parse(&[0, 1, 2]),
        Err(ParseError::TooShort { actual_bytes: 3 })
    ));

    let mut bytes = minimal_tfm();
    bytes.push(0);
    assert!(matches!(
        TfmFont::parse(&bytes),
        Err(ParseError::LengthNotMultipleOfFour { .. })
    ));

    let mut bytes = minimal_tfm();
    set_u16(&mut bytes, 0, 99);
    assert!(matches!(
        TfmFont::parse(&bytes),
        Err(ParseError::DeclaredLengthMismatch {
            declared_words: 99,
            ..
        })
    ));

    let mut bytes = minimal_tfm();
    set_u16(&mut bytes, 8, 3);
    assert!(matches!(
        TfmFont::parse(&bytes),
        Err(ParseError::SectionLengthMismatch { .. })
    ));

    let mut bytes = minimal_tfm();
    let char_word = word_offset(6 + 2);
    bytes[char_word] = 9;
    assert!(matches!(
        TfmFont::parse(&bytes),
        Err(ParseError::CharMetricIndexOutOfBounds {
            table: TfmTable::Width,
            index: 9,
            ..
        })
    ));
}

#[test]
fn malformed_lig_kern_and_recipe_indexes_are_rejected() {
    let bad_restart = tfm_with_lig_kern([255, b'A', 0, 9], 0);
    assert!(matches!(
        TfmFont::parse(&bad_restart),
        Err(ParseError::LigKernRestartOutOfBounds {
            index: 0,
            target: 9,
            ..
        })
    ));

    let bad_kern = tfm_with_lig_kern([128, b'A', 128, 1], 1);
    assert!(matches!(
        TfmFont::parse(&bad_kern),
        Err(ParseError::KernIndexOutOfBounds {
            instruction: 0,
            index: 1,
            ..
        })
    ));

    let bad_next_larger = tfm_with_char_info([1, 0, 2, b'B'], 0, 0, 0);
    assert!(matches!(
        TfmFont::parse(&bad_next_larger),
        Err(ParseError::NextLargerCharacterOutOfBounds {
            code: b'A',
            next: b'B',
            ..
        })
    ));

    let bad_extensible = tfm_with_char_info([1, 0, 3, 0], 0, 0, 1);
    assert!(matches!(
        TfmFont::parse(&bad_extensible),
        Err(ParseError::ExtensibleRecipeCharacterOutOfBounds {
            recipe: 0,
            field: "repeated",
            code: b'B',
            ..
        })
    ));
}

#[test]
fn size_fields_are_fifteen_bit_and_trailing_words_are_ignored() {
    for (offset, field) in [
        (0, "lf"),
        (2, "lh"),
        (4, "bc"),
        (6, "ec"),
        (8, "nw"),
        (10, "nh"),
        (12, "nd"),
        (14, "ni"),
        (16, "nl"),
        (18, "nk"),
        (20, "ne"),
        (22, "np"),
    ] {
        let mut bytes = minimal_tfm();
        set_u16(&mut bytes, offset, 0x8000);
        assert_eq!(
            TfmFont::parse(&bytes),
            Err(ParseError::SizeFieldOutOfRange {
                field,
                value: 0x8000,
            })
        );
    }

    let mut with_trailing_words = minimal_tfm();
    with_trailing_words.extend_from_slice(&[0xde, 0xad, 0xbe, 0xef]);
    let font = parse(&with_trailing_words);
    assert!(font.character(b'A').is_some());
}

#[test]
fn missing_width_characters_may_carry_structurally_valid_tags() {
    let cases = [
        ("lig/kern", [0, 0, 1, 0], vec![[255, 0, 0, 0]], Vec::new()),
        ("next larger", [0, 0, 2, b'B'], Vec::new(), Vec::new()),
        (
            "extensible",
            [0, 0, 3, 0],
            Vec::new(),
            vec![[0, 0, 0, b'B']],
        ),
    ];

    for (name, missing_info, lig_kerns, extensibles) in cases {
        let font = parse(&tfm_with_sections(Sections {
            bc: b'A',
            ec: b'B',
            char_info: vec![missing_info, [1, 0, 0, 0]],
            widths: vec![[0, 0, 0, 0], [0, 8, 0, 0]],
            heights: vec![[0, 0, 0, 0]],
            depths: vec![[0, 0, 0, 0]],
            italics: vec![[0, 0, 0, 0]],
            lig_kerns,
            kerns: Vec::new(),
            extensibles,
            params: Vec::new(),
        }));
        assert!(font.character(b'A').is_none(), "{name}");
        assert!(font.character(b'B').is_some(), "{name}");
    }
}

#[test]
fn next_larger_uses_declared_range_not_character_existence() {
    let font = parse(&tfm_with_sections(Sections {
        bc: b'A',
        ec: b'B',
        char_info: vec![[1, 0, 2, b'B'], [0, 0, 0, 0]],
        widths: vec![[0, 0, 0, 0], [0, 8, 0, 0]],
        heights: vec![[0, 0, 0, 0]],
        depths: vec![[0, 0, 0, 0]],
        italics: vec![[0, 0, 0, 0]],
        lig_kerns: Vec::new(),
        kerns: Vec::new(),
        extensibles: Vec::new(),
        params: Vec::new(),
    }));
    assert!(matches!(
        char_metric(&font, b'A').tag,
        CharacterTag::NextLarger(b'B')
    ));
    assert!(font.character(b'B').is_none());

    let cycle = tfm_with_sections(Sections {
        bc: b'A',
        ec: b'B',
        char_info: vec![[1, 0, 2, b'B'], [0, 0, 2, b'A']],
        widths: vec![[0, 0, 0, 0], [0, 8, 0, 0]],
        heights: vec![[0, 0, 0, 0]],
        depths: vec![[0, 0, 0, 0]],
        italics: vec![[0, 0, 0, 0]],
        lig_kerns: Vec::new(),
        kerns: Vec::new(),
        extensibles: Vec::new(),
        params: Vec::new(),
    });
    assert!(matches!(
        TfmFont::parse(&cycle),
        Err(ParseError::NextLargerCycle { code: b'A' })
    ));
}

#[test]
fn lig_kern_character_operands_require_in_range_existing_characters() {
    let make = |step| {
        tfm_with_sections(Sections {
            bc: b'A',
            ec: b'B',
            char_info: vec![[1, 0, 1, 0], [1, 0, 0, 0]],
            widths: vec![[0, 0, 0, 0], [0, 8, 0, 0]],
            heights: vec![[0, 0, 0, 0]],
            depths: vec![[0, 0, 0, 0]],
            italics: vec![[0, 0, 0, 0]],
            lig_kerns: vec![step],
            kerns: vec![[0, 0, 0, 0]],
            extensibles: Vec::new(),
            params: Vec::new(),
        })
    };
    for (field, bytes) in [
        ("match", make([128, b'C', 128, 0])),
        ("replacement", make([128, b'B', 0, b'C'])),
    ] {
        assert!(matches!(
            TfmFont::parse(&bytes),
            Err(ParseError::LigKernCharacterOutOfBounds { field: actual, .. }) if actual == field
        ));
    }

    let missing = tfm_with_sections(Sections {
        bc: b'A',
        ec: b'B',
        char_info: vec![[1, 0, 1, 0], [0, 0, 0, 0]],
        widths: vec![[0, 0, 0, 0], [0, 8, 0, 0]],
        heights: vec![[0, 0, 0, 0]],
        depths: vec![[0, 0, 0, 0]],
        italics: vec![[0, 0, 0, 0]],
        lig_kerns: vec![[128, b'B', 128, 0]],
        kerns: vec![[0, 0, 0, 0]],
        extensibles: Vec::new(),
        params: Vec::new(),
    });
    assert!(matches!(
        TfmFont::parse(&missing),
        Err(ParseError::LigKernCharacterMissing {
            field: "match",
            code: b'B',
            ..
        })
    ));
}

#[test]
fn extensible_recipe_pieces_require_in_range_existing_characters() {
    let in_range_missing = tfm_with_sections(Sections {
        bc: b'A',
        ec: b'B',
        char_info: vec![[1, 0, 3, 0], [0, 0, 0, 0]],
        widths: vec![[0, 0, 0, 0], [0, 8, 0, 0]],
        heights: vec![[0, 0, 0, 0]],
        depths: vec![[0, 0, 0, 0]],
        italics: vec![[0, 0, 0, 0]],
        lig_kerns: Vec::new(),
        kerns: Vec::new(),
        extensibles: vec![[0, 0, 0, b'B']],
        params: Vec::new(),
    });
    assert!(matches!(
        TfmFont::parse(&in_range_missing),
        Err(ParseError::ExtensibleRecipeCharacterMissing {
            recipe: 0,
            field: "repeated",
            code: b'B'
        })
    ));
}

#[test]
fn slant_parameter_is_unscaled_signed_fix_word_ratio() {
    let bytes = tfm_with_sections(Sections {
        bc: b'A',
        ec: b'A',
        char_info: vec![[1, 0, 0, 0]],
        widths: vec![[0, 0, 0, 0], [0, 8, 0, 0]],
        heights: vec![[0, 0, 0, 0]],
        depths: vec![[0, 0, 0, 0]],
        italics: vec![[0, 0, 0, 0]],
        lig_kerns: Vec::new(),
        kerns: Vec::new(),
        extensibles: Vec::new(),
        params: vec![[1, 0, 0, 0]],
    });
    let font = parse(&bytes);
    assert_eq!(
        font.parameters.slant().map(Scaled::raw),
        Some(0x0100_0000 / 16)
    );

    let negative = parse(&tfm_with_sections(Sections {
        bc: b'A',
        ec: b'A',
        char_info: vec![[1, 0, 0, 0]],
        widths: vec![[0, 0, 0, 0], [0, 8, 0, 0]],
        heights: vec![[0, 0, 0, 0]],
        depths: vec![[0, 0, 0, 0]],
        italics: vec![[0, 0, 0, 0]],
        lig_kerns: Vec::new(),
        kerns: Vec::new(),
        extensibles: Vec::new(),
        params: vec![[0xff, 0xff, 0xff, 0xff]],
    }));
    assert_eq!(negative.parameters.slant().map(Scaled::raw), Some(-1));
}

#[test]
fn short_parameter_tables_are_zero_padded_through_fontdimen_seven() {
    for np in 0..7 {
        let font = parse(&tfm_with_sections(Sections {
            bc: b'A',
            ec: b'A',
            char_info: vec![[1, 0, 0, 0]],
            widths: vec![[0, 0, 0, 0], [0, 8, 0, 0]],
            heights: vec![[0, 0, 0, 0]],
            depths: vec![[0, 0, 0, 0]],
            italics: vec![[0, 0, 0, 0]],
            lig_kerns: Vec::new(),
            kerns: Vec::new(),
            extensibles: Vec::new(),
            params: vec![[0, 0x10, 0, 0]; np],
        }));

        assert_eq!(font.parameters.values.len(), 7, "np={np}");
        for number in 1..=7 {
            let parameter = font.parameters.get(number).expect("padded parameter");
            assert_eq!(parameter.number, number, "np={np}");
            assert_eq!(
                parameter.value.raw() == 0,
                usize::from(number) > np,
                "np={np}, number={number}"
            );
        }
        assert_eq!(
            font.parameters.get(1).map(|parameter| parameter.kind),
            Some(FontParameterKind::SlantRatio)
        );
        assert!(
            font.parameters.values[1..]
                .iter()
                .all(|parameter| parameter.kind == FontParameterKind::Dimension)
        );
    }
}

#[test]
fn empty_font_bounds_are_accepted_and_normalized() {
    let font = parse(&tfm_with_sections(Sections {
        bc: 1,
        ec: 0,
        char_info: Vec::new(),
        widths: vec![[0, 0, 0, 0]],
        heights: vec![[0, 0, 0, 0]],
        depths: vec![[0, 0, 0, 0]],
        italics: vec![[0, 0, 0, 0]],
        lig_kerns: Vec::new(),
        kerns: Vec::new(),
        extensibles: Vec::new(),
        params: Vec::new(),
    }));
    assert_eq!(font.bounds.bc, 1);
    assert_eq!(font.bounds.ec, 0);
    assert_eq!(font.characters.iter().flatten().count(), 0);
}

fn parse(bytes: &[u8]) -> TfmFont {
    match TfmFont::parse(bytes) {
        Ok(font) => font,
        Err(error) => panic!("TFM should parse: {error}"),
    }
}

fn char_metric(font: &TfmFont, code: u8) -> &tex_fonts::tfm::Character {
    match font.character(code) {
        Some(character) => character,
        None => panic!("missing character {code}"),
    }
}

fn minimal_tfm() -> Vec<u8> {
    tfm_with_sections(Sections {
        bc: b'A',
        ec: b'A',
        char_info: vec![[1, 0, 0, 0]],
        widths: vec![[0, 0, 0, 0], [0, 8, 0, 0]],
        heights: vec![[0, 0, 0, 0]],
        depths: vec![[0, 0, 0, 0]],
        italics: vec![[0, 0, 0, 0]],
        lig_kerns: Vec::new(),
        kerns: Vec::new(),
        extensibles: Vec::new(),
        params: Vec::new(),
    })
}

fn tfm_with_lig_kern(step: [u8; 4], kern_count: usize) -> Vec<u8> {
    let mut kerns = Vec::new();
    for _ in 0..kern_count {
        kerns.push([0, 0, 0, 0]);
    }
    tfm_with_sections(Sections {
        bc: b'A',
        ec: b'A',
        char_info: vec![[1, 0, 1, 0]],
        widths: vec![[0, 0, 0, 0], [0, 8, 0, 0]],
        heights: vec![[0, 0, 0, 0]],
        depths: vec![[0, 0, 0, 0]],
        italics: vec![[0, 0, 0, 0]],
        lig_kerns: vec![step],
        kerns,
        extensibles: Vec::new(),
        params: Vec::new(),
    })
}

fn tfm_with_char_info(
    char_info: [u8; 4],
    lig_count: usize,
    kern_count: usize,
    ext_count: usize,
) -> Vec<u8> {
    let lig_kerns = vec![[128, b'A', 0, b'A']; lig_count];
    let kerns = vec![[0, 0, 0, 0]; kern_count];
    let extensibles = vec![[0, 0, 0, b'B']; ext_count];
    tfm_with_sections(Sections {
        bc: b'A',
        ec: b'A',
        char_info: vec![char_info],
        widths: vec![[0, 0, 0, 0], [0, 8, 0, 0]],
        heights: vec![[0, 0, 0, 0]],
        depths: vec![[0, 0, 0, 0]],
        italics: vec![[0, 0, 0, 0]],
        lig_kerns,
        kerns,
        extensibles,
        params: Vec::new(),
    })
}

struct Sections {
    bc: u8,
    ec: u8,
    char_info: Vec<[u8; 4]>,
    widths: Vec<[u8; 4]>,
    heights: Vec<[u8; 4]>,
    depths: Vec<[u8; 4]>,
    italics: Vec<[u8; 4]>,
    lig_kerns: Vec<[u8; 4]>,
    kerns: Vec<[u8; 4]>,
    extensibles: Vec<[u8; 4]>,
    params: Vec<[u8; 4]>,
}

fn tfm_with_sections(sections: Sections) -> Vec<u8> {
    let lh = 2usize;
    let lf = 6
        + lh
        + sections.char_info.len()
        + sections.widths.len()
        + sections.heights.len()
        + sections.depths.len()
        + sections.italics.len()
        + sections.lig_kerns.len()
        + sections.kerns.len()
        + sections.extensibles.len()
        + sections.params.len();

    let mut bytes = Vec::new();
    push_u16(&mut bytes, lf as u16);
    push_u16(&mut bytes, lh as u16);
    push_u16(&mut bytes, u16::from(sections.bc));
    push_u16(&mut bytes, u16::from(sections.ec));
    push_u16(&mut bytes, sections.widths.len() as u16);
    push_u16(&mut bytes, sections.heights.len() as u16);
    push_u16(&mut bytes, sections.depths.len() as u16);
    push_u16(&mut bytes, sections.italics.len() as u16);
    push_u16(&mut bytes, sections.lig_kerns.len() as u16);
    push_u16(&mut bytes, sections.kerns.len() as u16);
    push_u16(&mut bytes, sections.extensibles.len() as u16);
    push_u16(&mut bytes, sections.params.len() as u16);

    push_words(&mut bytes, &[[0, 0, 0, 0], [0, 0xa0, 0, 0]]);
    push_words(&mut bytes, &sections.char_info);
    push_words(&mut bytes, &sections.widths);
    push_words(&mut bytes, &sections.heights);
    push_words(&mut bytes, &sections.depths);
    push_words(&mut bytes, &sections.italics);
    push_words(&mut bytes, &sections.lig_kerns);
    push_words(&mut bytes, &sections.kerns);
    push_words(&mut bytes, &sections.extensibles);
    push_words(&mut bytes, &sections.params);
    bytes
}

fn push_words(bytes: &mut Vec<u8>, words: &[[u8; 4]]) {
    for word in words {
        bytes.extend_from_slice(word);
    }
}

fn push_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_be_bytes());
}

fn set_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
}

fn word_offset(word: usize) -> usize {
    word * 4
}
