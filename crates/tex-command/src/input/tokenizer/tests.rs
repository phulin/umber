use std::cell::Cell;
use std::sync::Arc;

use tex_state::token::{Catcode, Token, TokenWord};

use super::{SourceControlSequenceKind, SourceToken, SourceTokenizationStep};
use crate::{
    CatcodeQueries, CharacterCode, CommandDialect, CommandProfile, CommandState,
    RegisteredSourceKind, SourceRegistration,
};

fn state(bytes: &[u8]) -> CommandState<()> {
    state_for_profile(bytes, CommandProfile::TEX82)
}

fn state_for_profile(bytes: &[u8], profile: CommandProfile) -> CommandState<()> {
    let mut state = CommandState::new(profile);
    let source = state
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(bytes),
        ))
        .expect("register exact-byte source");
    state
        .open_registered_source(source)
        .expect("open registered source");
    state
}

fn unicode_state(text: &str) -> CommandState<()> {
    state_for_profile(
        text.as_bytes(),
        CommandProfile::unicode_extended(CommandDialect::Tex82),
    )
}

fn classic_catcode(code: CharacterCode) -> Catcode {
    match code.to_byte().expect("exact byte") {
        b'\\' => Catcode::Escape,
        b'{' => Catcode::BeginGroup,
        b'}' => Catcode::EndGroup,
        b'^' => Catcode::Superscript,
        b'%' => Catcode::Comment,
        b' ' | b'\t' => Catcode::Space,
        b'\r' => Catcode::EndLine,
        b'a'..=b'z' | b'A'..=b'Z' => Catcode::Letter,
        _ => Catcode::Other,
    }
}

fn token(step: SourceTokenizationStep) -> SourceToken {
    match step {
        SourceTokenizationStep::Token(token) => token,
        other => panic!("expected token, found {other:?}"),
    }
}

fn character(step: SourceTokenizationStep) -> (u8, Catcode, u64, u64) {
    match token(step) {
        SourceToken::Character {
            code,
            catcode,
            range,
            ..
        } => (
            code.to_byte().expect("exact byte"),
            catcode,
            range.start(),
            range.end(),
        ),
        other => panic!("expected character token, found {other:?}"),
    }
}

fn control(step: SourceTokenizationStep) -> (Vec<u8>, SourceControlSequenceKind, u64, u64) {
    match token(step) {
        SourceToken::ControlSequence {
            name, kind, range, ..
        } => (
            name.iter()
                .copied()
                .map(|code| code.to_byte().expect("exact byte"))
                .collect(),
            kind,
            range.start(),
            range.end(),
        ),
        other => panic!("expected control-sequence token, found {other:?}"),
    }
}

fn unicode_catcode(code: CharacterCode) -> Catcode {
    match code.to_char().expect("Unicode scalar") {
        '\\' => Catcode::Escape,
        '{' => Catcode::BeginGroup,
        '}' => Catcode::EndGroup,
        '^' => Catcode::Superscript,
        '%' => Catcode::Comment,
        ' ' | '\t' => Catcode::Space,
        '\r' => Catcode::EndLine,
        'a'..='z' | 'A'..='Z' | 'λ' | 'é' => Catcode::Letter,
        _ => Catcode::Other,
    }
}

fn unicode_character(step: SourceTokenizationStep) -> (char, Catcode, u64, u64, u64, u64) {
    match token(step) {
        SourceToken::Character {
            code,
            catcode,
            range,
            scalar_range,
        } => (
            code.to_char().expect("Unicode scalar"),
            catcode,
            range.start(),
            range.end(),
            scalar_range.start(),
            scalar_range.end(),
        ),
        other => panic!("expected character token, found {other:?}"),
    }
}

fn unicode_control(
    step: SourceTokenizationStep,
) -> (Vec<char>, SourceControlSequenceKind, u64, u64, u64, u64) {
    match token(step) {
        SourceToken::ControlSequence {
            name,
            kind,
            range,
            scalar_range,
            ..
        } => (
            name.iter()
                .copied()
                .map(|code| code.to_char().expect("Unicode scalar"))
                .collect(),
            kind,
            range.start(),
            range.end(),
            scalar_range.start(),
            scalar_range.end(),
        ),
        other => panic!("expected control-sequence token, found {other:?}"),
    }
}

#[test]
fn m_n_s_states_cover_ignored_comments_spaces_endlines_and_blank_lines() {
    let mut state = state(b"A!!B  \n% discarded\n\n C\tD\n");
    let catcode = |code: CharacterCode| {
        if code.to_byte() == Ok(b'!') {
            Catcode::Ignored
        } else {
            classic_catcode(code)
        }
    };

    assert_eq!(
        character(state.next_exact_source_step(13, &mut CatcodeQueries(catcode))),
        (b'A', Catcode::Letter, 0, 1)
    );
    assert_eq!(
        character(state.next_exact_source_step(13, &mut CatcodeQueries(catcode))),
        (b'B', Catcode::Letter, 3, 4)
    );
    assert_eq!(
        character(state.next_exact_source_step(13, &mut CatcodeQueries(catcode))),
        (b' ', Catcode::Space, 4, 4)
    );
    assert_eq!(
        control(state.next_exact_source_step(13, &mut CatcodeQueries(catcode))),
        (
            b"par".to_vec(),
            SourceControlSequenceKind::Paragraph,
            19,
            19
        )
    );
    assert_eq!(
        character(state.next_exact_source_step(13, &mut CatcodeQueries(catcode))),
        (b'C', Catcode::Letter, 21, 22)
    );
    assert_eq!(
        character(state.next_exact_source_step(13, &mut CatcodeQueries(catcode))),
        (b' ', Catcode::Space, 22, 23)
    );
    assert_eq!(
        character(state.next_exact_source_step(13, &mut CatcodeQueries(catcode))),
        (b'D', Catcode::Letter, 23, 24)
    );
    assert_eq!(
        character(state.next_exact_source_step(13, &mut CatcodeQueries(catcode))),
        (b' ', Catcode::Space, 24, 24)
    );
    assert_eq!(
        state.next_exact_source_step(13, &mut CatcodeQueries(catcode)),
        SourceTokenizationStep::End
    );
}

/// tex.web §351 locates the generated `\par` at `buffer[limit]`, which §362
/// fills with `\endlinechar` at the *normalized* content end. A line whose
/// only content is trailing spaces normalizes to empty, so the `\par` belongs
/// to the first stripped space, never to the physical terminator bytes.
#[test]
fn blank_line_par_is_anchored_at_the_normalized_line_end() {
    let mut state = state(b" \nA\n");

    assert_eq!(
        control(state.next_exact_source_step(13, &mut CatcodeQueries(classic_catcode))),
        (b"par".to_vec(), SourceControlSequenceKind::Paragraph, 0, 0)
    );
    assert_eq!(
        character(state.next_exact_source_step(13, &mut CatcodeQueries(classic_catcode))),
        (b'A', Catcode::Letter, 2, 3)
    );
    assert_eq!(
        character(state.next_exact_source_step(13, &mut CatcodeQueries(classic_catcode))),
        (b' ', Catcode::Space, 3, 3)
    );
    assert_eq!(
        state.next_exact_source_step(13, &mut CatcodeQueries(classic_catcode)),
        SourceTokenizationStep::End
    );
}

/// TeX82 §1126 receives an arbitrary category-5 source character as a raw
/// `car_ret`; it is not the synthetic character that terminates a physical
/// input line.
#[test]
fn explicit_car_ret_character_is_delivered_without_finishing_its_line() {
    let catcode = |code: CharacterCode| match code.to_byte().expect("exact byte") {
        b'X' => Catcode::EndLine,
        other => classic_catcode(CharacterCode::from_byte(other)),
    };
    let mut state = state(b"aXb\nXb\nc\n");

    assert_eq!(
        character(state.next_exact_source_step(13, &mut CatcodeQueries(catcode))),
        (b'a', Catcode::Letter, 0, 1)
    );
    assert_eq!(
        character(state.next_exact_source_step(13, &mut CatcodeQueries(catcode))),
        (b'X', Catcode::EndLine, 1, 2)
    );
    assert_eq!(
        character(state.next_exact_source_step(13, &mut CatcodeQueries(catcode))),
        (b'b', Catcode::Letter, 2, 3)
    );
    assert_eq!(
        character(state.next_exact_source_step(13, &mut CatcodeQueries(catcode))),
        (b' ', Catcode::Space, 3, 3)
    );
    assert_eq!(
        character(state.next_exact_source_step(13, &mut CatcodeQueries(catcode))),
        (b'X', Catcode::EndLine, 4, 5)
    );
    assert_eq!(
        character(state.next_exact_source_step(13, &mut CatcodeQueries(catcode))),
        (b'b', Catcode::Letter, 5, 6)
    );
    assert_eq!(
        character(state.next_exact_source_step(13, &mut CatcodeQueries(catcode))),
        (b' ', Catcode::Space, 6, 6)
    );
    assert_eq!(
        character(state.next_exact_source_step(13, &mut CatcodeQueries(catcode))),
        (b'c', Catcode::Letter, 7, 8)
    );
    assert_eq!(
        character(state.next_exact_source_step(13, &mut CatcodeQueries(catcode))),
        (b' ', Catcode::Space, 8, 8)
    );
    assert_eq!(
        state.next_exact_source_step(13, &mut CatcodeQueries(catcode)),
        SourceTokenizationStep::End
    );
}

/// With `\endlinechar` inactive, a category-5 source character remains an
/// ordinary source-backed `car_ret` token rather than acquiring a synthetic
/// line-end anchor.
#[test]
fn inactive_endlinechar_does_not_change_explicit_car_ret_origin() {
    let catcode = |code: CharacterCode| match code.to_byte().expect("exact byte") {
        b'X' => Catcode::EndLine,
        other => classic_catcode(CharacterCode::from_byte(other)),
    };
    let mut state = state(b"aXb\nc\n");

    assert_eq!(
        character(state.next_exact_source_step(-1, &mut CatcodeQueries(catcode))),
        (b'a', Catcode::Letter, 0, 1)
    );
    assert_eq!(
        character(state.next_exact_source_step(-1, &mut CatcodeQueries(catcode))),
        (b'X', Catcode::EndLine, 1, 2)
    );
    assert_eq!(
        character(state.next_exact_source_step(-1, &mut CatcodeQueries(catcode))),
        (b'b', Catcode::Letter, 2, 3)
    );
    assert_eq!(
        character(state.next_exact_source_step(-1, &mut CatcodeQueries(catcode))),
        (b'c', Catcode::Letter, 4, 5)
    );
    assert_eq!(
        state.next_exact_source_step(-1, &mut CatcodeQueries(catcode)),
        SourceTokenizationStep::End
    );
}

#[test]
fn constructs_words_symbols_active_characters_and_null_names_with_exact_ranges() {
    let mut command = state(b"\\foo  \\!~\\^^61bc\n");
    let catcode = |code: CharacterCode| {
        if code.to_byte() == Ok(b'~') {
            Catcode::Active
        } else {
            classic_catcode(code)
        }
    };

    assert_eq!(
        control(command.next_exact_source_step(13, &mut CatcodeQueries(catcode))),
        (b"foo".to_vec(), SourceControlSequenceKind::Word, 0, 4)
    );
    assert_eq!(
        control(command.next_exact_source_step(13, &mut CatcodeQueries(catcode))),
        (b"!".to_vec(), SourceControlSequenceKind::Symbol, 6, 8)
    );
    assert_eq!(
        control(command.next_exact_source_step(13, &mut CatcodeQueries(catcode))),
        (b"~".to_vec(), SourceControlSequenceKind::Active, 8, 9)
    );
    assert_eq!(
        control(command.next_exact_source_step(13, &mut CatcodeQueries(catcode))),
        (b"abc".to_vec(), SourceControlSequenceKind::Word, 9, 16)
    );

    let mut null_state = state(b"\\");
    assert_eq!(
        control(null_state.next_exact_source_step(-1, &mut CatcodeQueries(classic_catcode))),
        (Vec::new(), SourceControlSequenceKind::Null, 0, 1)
    );
}

#[test]
fn rejected_control_word_superscript_probe_does_not_publish_or_retain_an_edit() {
    let mut command = state(br"\a^^3f");
    assert_eq!(
        control(command.next_exact_source_step(13, &mut CatcodeQueries(classic_catcode))),
        (b"a".to_vec(), SourceControlSequenceKind::Word, 0, 2)
    );
    let line = match command.input.levels.last() {
        Some(crate::input::InputLevel::Source(source)) => source
            .slot
            .cursor
            .line
            .as_ref()
            .expect("control-word line remains loaded"),
        _ => panic!("source remains live"),
    };
    assert_eq!(line.reduced_spelling_storage_len(), 0);
    assert_eq!(line.active_reduced_spellings().len(), 0);
    assert_eq!(
        character(command.next_exact_source_step(13, &mut CatcodeQueries(classic_catcode))),
        (b'?', Catcode::Other, 2, 6)
    );
}

#[test]
fn canonical_superscript_forms_use_lowercase_hex_and_complete_ranges() {
    let mut state = state(b"^^41^^7a^^8^^5A");

    let first = token(state.next_exact_source_step(-1, &mut CatcodeQueries(classic_catcode)));
    assert_eq!(
        first.provenance().location().byte(),
        3,
        "TeX82 observes the post-reduction cursor, while the raw span keeps the complete ^^41 spelling"
    );
    let SourceToken::Character {
        code,
        catcode,
        range,
        ..
    } = first
    else {
        panic!("expected character token");
    };
    assert_eq!(
        (
            code.to_byte().expect("exact byte"),
            catcode,
            range.start(),
            range.end()
        ),
        (b'A', Catcode::Letter, 0, 4),
    );
    assert_eq!(
        character(state.next_exact_source_step(-1, &mut CatcodeQueries(classic_catcode))),
        (b'z', Catcode::Letter, 4, 8)
    );
    assert_eq!(
        character(state.next_exact_source_step(-1, &mut CatcodeQueries(classic_catcode))),
        (b'x', Catcode::Letter, 8, 11)
    );
    assert_eq!(
        character(state.next_exact_source_step(-1, &mut CatcodeQueries(classic_catcode))),
        (b'u', Catcode::Letter, 11, 14)
    );
    assert_eq!(
        character(state.next_exact_source_step(-1, &mut CatcodeQueries(classic_catcode))),
        (b'A', Catcode::Letter, 14, 15)
    );
}

#[test]
fn invalid_reduced_character_is_consumed_once_before_delivery_restarts() {
    let mut state = state(b"^^3fB");
    let catcode = |code: CharacterCode| {
        if code.to_byte() == Ok(b'?') {
            Catcode::Invalid
        } else {
            classic_catcode(code)
        }
    };

    match state.next_exact_source_step(-1, &mut CatcodeQueries(catcode)) {
        SourceTokenizationStep::InvalidCharacter(invalid) => {
            assert_eq!(invalid.code().to_byte(), Ok(b'?'));
            assert_eq!((invalid.range().start(), invalid.range().end()), (0, 4));
        }
        other => panic!("expected invalid-character step, found {other:?}"),
    }
    assert_eq!(
        character(state.next_exact_source_step(-1, &mut CatcodeQueries(catcode))),
        (b'B', Catcode::Letter, 4, 5)
    );
}

#[test]
fn catcode_changes_are_observed_at_the_next_token_boundary() {
    let mut state = state(b"ab");
    let second_is_letter = Cell::new(false);
    let catcode = |code: CharacterCode| match code.to_byte().expect("byte") {
        b'a' => Catcode::Other,
        b'b' if second_is_letter.get() => Catcode::Letter,
        _ => Catcode::Other,
    };

    assert_eq!(
        character(state.next_exact_source_step(-1, &mut CatcodeQueries(catcode))),
        (b'a', Catcode::Other, 0, 1)
    );
    second_is_letter.set(true);
    assert_eq!(
        character(state.next_exact_source_step(-1, &mut CatcodeQueries(catcode))),
        (b'b', Catcode::Letter, 1, 2)
    );
}

#[test]
fn every_exact_byte_value_survives_source_token_construction() {
    for expected in u8::MIN..=u8::MAX {
        let (spelling, superscript) = match expected {
            b'\n' => (b"^^0aX".to_vec(), true),
            b'\r' => (b"^^0dX".to_vec(), true),
            _ => (vec![expected, b'X'], false),
        };
        let mut state = state(&spelling);
        let catcode = |code: CharacterCode| {
            if superscript && code.to_byte() == Ok(b'^') {
                Catcode::Superscript
            } else {
                Catcode::Other
            }
        };
        let (actual, actual_catcode, _, _) =
            character(state.next_exact_source_step(-1, &mut CatcodeQueries(catcode)));
        assert_eq!(actual, expected);
        assert_eq!(actual_catcode, Catcode::Other);
    }
}

#[test]
fn all_canonical_dialects_share_the_exact_byte_state_machine() {
    for dialect in [
        CommandDialect::Tex82,
        CommandDialect::Etex26,
        CommandDialect::Pdftex14029,
    ] {
        let mut state = state_for_profile(b"^^7a", CommandProfile::exact(dialect));
        assert_eq!(
            character(state.next_exact_source_step(-1, &mut CatcodeQueries(classic_catcode))),
            (b'z', Catcode::Letter, 0, 4)
        );
    }
}

#[test]
fn ordinary_token_catcodes_are_observed_without_reclassification() {
    let ordinary = [
        Catcode::BeginGroup,
        Catcode::EndGroup,
        Catcode::MathShift,
        Catcode::AlignmentTab,
        Catcode::Parameter,
        Catcode::Superscript,
        Catcode::Subscript,
        Catcode::Letter,
        Catcode::Other,
    ];

    for expected in ordinary {
        let mut state = state(b"x");
        assert_eq!(
            character(state.next_exact_source_step(-1, &mut CatcodeQueries(|_| expected))),
            (b'x', expected, 0, 1)
        );
    }
}

#[test]
fn unicode_words_symbols_active_and_combining_scalars_keep_both_ranges() {
    let mut state = unicode_state("\\λé  \\🦀🙂\u{301}");
    let catcode = |code: CharacterCode| match code.to_char().expect("Unicode scalar") {
        '🙂' => Catcode::Active,
        other => unicode_catcode(CharacterCode::from(other)),
    };

    assert_eq!(
        unicode_control(state.next_unicode_source_step(-1, &mut CatcodeQueries(catcode))),
        (vec!['λ', 'é'], SourceControlSequenceKind::Word, 0, 5, 0, 3)
    );
    assert_eq!(
        unicode_control(state.next_unicode_source_step(-1, &mut CatcodeQueries(catcode))),
        (vec!['🦀'], SourceControlSequenceKind::Symbol, 7, 12, 5, 7)
    );
    assert_eq!(
        unicode_control(state.next_unicode_source_step(-1, &mut CatcodeQueries(catcode))),
        (vec!['🙂'], SourceControlSequenceKind::Active, 12, 16, 7, 8)
    );
    assert_eq!(
        unicode_character(state.next_unicode_source_step(-1, &mut CatcodeQueries(catcode))),
        ('\u{301}', Catcode::Other, 16, 18, 8, 9)
    );
}

#[test]
fn unicode_m_n_s_states_use_unicode_domain_space_par_and_endline() {
    let mut state = unicode_state("é  \n\n λ\tπ\n");

    assert_eq!(
        unicode_character(state.next_unicode_source_step(13, &mut CatcodeQueries(unicode_catcode))),
        ('é', Catcode::Letter, 0, 2, 0, 1)
    );
    assert_eq!(
        unicode_character(state.next_unicode_source_step(13, &mut CatcodeQueries(unicode_catcode))),
        (' ', Catcode::Space, 2, 2, 1, 2)
    );
    assert_eq!(
        unicode_control(state.next_unicode_source_step(13, &mut CatcodeQueries(unicode_catcode))),
        (
            vec!['p', 'a', 'r'],
            SourceControlSequenceKind::Paragraph,
            5,
            5,
            0,
            1
        )
    );
    assert_eq!(
        unicode_character(state.next_unicode_source_step(13, &mut CatcodeQueries(unicode_catcode))),
        ('λ', Catcode::Letter, 7, 9, 1, 2)
    );
    assert_eq!(
        unicode_character(state.next_unicode_source_step(13, &mut CatcodeQueries(unicode_catcode))),
        (' ', Catcode::Space, 9, 10, 2, 3)
    );
    assert_eq!(
        unicode_character(state.next_unicode_source_step(13, &mut CatcodeQueries(unicode_catcode))),
        ('π', Catcode::Other, 10, 12, 3, 4)
    );
}

#[test]
fn unicode_catcode_mutation_is_observable_and_defaults_remain_external() {
    let mut state = unicode_state("🦀🙂");
    let active = Cell::new(false);
    let catcode = |code: CharacterCode| {
        assert!(code.is_unicode_scalar());
        match code.to_char().expect("Unicode scalar") {
            '🙂' if active.get() => Catcode::Active,
            _ => Catcode::Other,
        }
    };

    assert_eq!(
        unicode_character(state.next_unicode_source_step(-1, &mut CatcodeQueries(catcode))),
        ('🦀', Catcode::Other, 0, 4, 0, 1)
    );
    active.set(true);
    assert_eq!(
        unicode_control(state.next_unicode_source_step(-1, &mut CatcodeQueries(catcode))),
        (vec!['🙂'], SourceControlSequenceKind::Active, 4, 8, 1, 2)
    );
}

#[test]
fn unicode_superscript_policy_accepts_unicode_forms_and_exact_ranges() {
    let mut state = unicode_state("^^^^00E9 ^^4A ^^é");

    assert_eq!(
        unicode_character(state.next_unicode_source_step(-1, &mut CatcodeQueries(unicode_catcode))),
        ('é', Catcode::Letter, 0, 8, 0, 8)
    );
    assert_eq!(
        unicode_character(state.next_unicode_source_step(-1, &mut CatcodeQueries(unicode_catcode))),
        (' ', Catcode::Space, 8, 9, 8, 9)
    );
    assert_eq!(
        unicode_character(state.next_unicode_source_step(-1, &mut CatcodeQueries(unicode_catcode))),
        ('J', Catcode::Letter, 9, 13, 9, 13)
    );
    assert_eq!(
        unicode_character(state.next_unicode_source_step(-1, &mut CatcodeQueries(unicode_catcode))),
        (' ', Catcode::Space, 13, 14, 13, 14)
    );
    assert_eq!(
        unicode_character(state.next_unicode_source_step(-1, &mut CatcodeQueries(unicode_catcode))),
        ('©', Catcode::Other, 14, 18, 14, 17)
    );
}

#[test]
fn unicode_superscript_introducer_is_selected_only_by_live_catcode() {
    let mut state = unicode_state("⁁⁁4a ⁁⁁⁁⁁00E9");
    let catcode = |code: CharacterCode| {
        if code.to_char() == Ok('⁁') {
            Catcode::Superscript
        } else {
            unicode_catcode(code)
        }
    };

    assert_eq!(
        unicode_character(state.next_unicode_source_step(-1, &mut CatcodeQueries(catcode))),
        ('J', Catcode::Letter, 0, 8, 0, 4)
    );
    assert_eq!(
        unicode_character(state.next_unicode_source_step(-1, &mut CatcodeQueries(catcode))),
        (' ', Catcode::Space, 8, 9, 4, 5)
    );
    assert_eq!(
        unicode_character(state.next_unicode_source_step(-1, &mut CatcodeQueries(catcode))),
        ('é', Catcode::Letter, 9, 25, 5, 13)
    );
}

#[test]
fn unicode_categories_never_supply_implicit_catcodes() {
    let mut state = unicode_state("\u{2003}λ");
    let observed = Cell::new(0);
    let catcode = |code: CharacterCode| {
        observed.set(observed.get() + 1);
        match code.to_char().expect("Unicode scalar") {
            '\u{2003}' => Catcode::Other,
            'λ' => Catcode::Active,
            _ => Catcode::Other,
        }
    };

    assert_eq!(
        unicode_character(state.next_unicode_source_step(-1, &mut CatcodeQueries(catcode))),
        ('\u{2003}', Catcode::Other, 0, 3, 0, 1)
    );
    assert_eq!(
        unicode_control(state.next_unicode_source_step(-1, &mut CatcodeQueries(catcode))),
        (vec!['λ'], SourceControlSequenceKind::Active, 3, 5, 1, 2)
    );
    assert!(observed.get() >= 2);
}

#[test]
fn unicode_invalid_reduction_reports_semantic_code_and_scalar_spelling() {
    let mut state = unicode_state("^^^^263Aλ");
    let catcode = |code: CharacterCode| {
        if code.to_char() == Ok('☺') {
            Catcode::Invalid
        } else {
            unicode_catcode(code)
        }
    };

    match state.next_unicode_source_step(-1, &mut CatcodeQueries(catcode)) {
        SourceTokenizationStep::InvalidCharacter(invalid) => {
            assert_eq!(invalid.code().to_char(), Ok('☺'));
            assert_eq!((invalid.range().start(), invalid.range().end()), (0, 8));
            assert_eq!(
                (invalid.scalar_range().start(), invalid.scalar_range().end()),
                (0, 8)
            );
        }
        other => panic!("expected invalid-character step, found {other:?}"),
    }
    assert_eq!(
        unicode_character(state.next_unicode_source_step(-1, &mut CatcodeQueries(catcode))),
        ('λ', Catcode::Letter, 8, 10, 8, 9)
    );
}

#[test]
#[should_panic(expected = "exact-byte tokenization requires an exact-byte command profile")]
fn unicode_profile_cannot_enter_exact_byte_tokenizer() {
    let mut state = unicode_state("x");
    let _ = state.next_exact_source_step(-1, &mut CatcodeQueries(unicode_catcode));
}

#[test]
#[should_panic(expected = "Unicode tokenization requires a UnicodeExtended command profile")]
fn exact_profile_cannot_enter_unicode_tokenizer() {
    let mut state = state(b"x");
    let _ = state.next_unicode_source_step(-1, &mut CatcodeQueries(|_| Catcode::Other));
}

/// Live queries whose §363 replacement is a fixed script of typed lines.
struct ScriptedPausing {
    typed: Vec<&'static str>,
    displayed: Vec<String>,
}

impl crate::SourceStepQueries for ScriptedPausing {
    fn catcode(&mut self, code: CharacterCode) -> Catcode {
        classic_catcode(code)
    }

    fn firm_up_the_line(&mut self, line: &str) -> Option<SourceRegistration> {
        self.displayed.push(line.to_owned());
        let typed = self.typed.remove(0);
        (!typed.is_empty()).then(|| {
            SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(typed.as_bytes()),
            )
        })
    }
}

#[test]
fn pausing_replaces_the_firmed_line_before_tokenization() {
    // TeX82 §363: the line is displayed as `limit` bounds it -- trailing
    // blanks already stripped -- and the typed line is what gets tokenized.
    let mut state = state(b"AB  \nCD\n");
    let mut queries = ScriptedPausing {
        typed: vec!["XY", ""],
        displayed: Vec::new(),
    };

    assert_eq!(
        character(state.next_exact_source_step(-1, &mut queries)),
        (b'X', Catcode::Letter, 0, 1)
    );
    assert_eq!(
        character(state.next_exact_source_step(-1, &mut queries)),
        (b'Y', Catcode::Letter, 1, 2)
    );
    // §363 runs at every refill, not only the first: the second line is
    // offered too, and a bare carriage return leaves the file's line alone.
    assert_eq!(
        character(state.next_exact_source_step(-1, &mut queries)),
        (b'C', Catcode::Letter, 5, 6)
    );
    assert_eq!(
        character(state.next_exact_source_step(-1, &mut queries)),
        (b'D', Catcode::Letter, 6, 7)
    );
    assert_eq!(queries.displayed, vec!["AB".to_owned(), "CD".to_owned()]);
}

#[test]
fn zero_pausing_leaves_every_line_exactly_as_the_file_supplied_it() {
    // §363's replacement is the whole of `\pausing`; with it declined the
    // tokenizer must be byte-identical to a run that never offered one.
    let mut state = state(b"AB\n");
    let mut queries = CatcodeQueries(classic_catcode);

    assert_eq!(
        character(state.next_exact_source_step(-1, &mut queries)),
        (b'A', Catcode::Letter, 0, 1)
    );
    assert_eq!(
        character(state.next_exact_source_step(-1, &mut queries)),
        (b'B', Catcode::Letter, 1, 2)
    );
}

#[test]
fn control_sequence_names_use_inline_storage_through_the_measured_bound() {
    let source = format!(
        "\\{} ",
        "a".repeat(super::CONTROL_SEQUENCE_NAME_INLINE_CAPACITY)
    );
    let mut state = state(source.as_bytes());
    let SourceToken::ControlSequence { name, .. } =
        token(state.next_exact_source_step(-1, &mut CatcodeQueries(classic_catcode)))
    else {
        panic!("expected control-sequence token");
    };

    assert_eq!(name.len(), super::CONTROL_SEQUENCE_NAME_INLINE_CAPACITY);
    assert!(!name.is_spilled());
}

#[test]
fn control_sequence_names_spill_without_a_length_limit() {
    let length = super::CONTROL_SEQUENCE_NAME_INLINE_CAPACITY * 8;
    let source = format!("\\{} ", "a".repeat(length));
    let mut state = state(source.as_bytes());
    let SourceToken::ControlSequence { name, .. } =
        token(state.next_exact_source_step(-1, &mut CatcodeQueries(classic_catcode)))
    else {
        panic!("expected control-sequence token");
    };

    assert_eq!(name.len(), length);
    assert!(name.is_spilled());
    assert!(name.iter().all(|code| code.to_byte() == Ok(b'a')));
}

#[test]
fn special_control_sequence_names_remain_inline() {
    let cases = [
        ("~", 13, SourceControlSequenceKind::Active, 1),
        ("\n", 13, SourceControlSequenceKind::Paragraph, 3),
        ("\\", -1, SourceControlSequenceKind::Null, 0),
    ];

    for (source, endlinechar, expected_kind, expected_len) in cases {
        let mut state = state(source.as_bytes());
        let mut queries = CatcodeQueries(|code: CharacterCode| {
            if code.to_byte() == Ok(b'~') {
                Catcode::Active
            } else {
                classic_catcode(code)
            }
        });
        let SourceToken::ControlSequence { name, kind, .. } =
            token(state.next_exact_source_step(endlinechar, &mut queries))
        else {
            panic!("expected control-sequence token");
        };

        assert_eq!(kind, expected_kind);
        assert_eq!(name.len(), expected_len);
        assert!(!name.is_spilled());
    }
}

#[test]
fn production_source_step_does_not_carry_owned_control_sequence_names() {
    assert!(
        std::mem::size_of::<super::CompactSourceToken>() <= 48,
        "packed token identity plus direct provenance stays compact"
    );
    assert!(
        std::mem::size_of::<super::CompactSourceTokenizationStep>()
            < std::mem::size_of::<SourceTokenizationStep>(),
        "production delivery must not inherit the owned tokenizer-name width"
    );
}

#[test]
fn speculative_line_probe_is_exactly_the_copy_small_lexer_cursor() {
    assert_eq!(std::mem::size_of::<super::LineProbe>(), 24);
    assert_eq!(
        std::mem::size_of::<super::LineProbe>(),
        std::mem::size_of::<crate::input::lines::SourceLexCursor>()
    );
}

#[derive(Default)]
struct CompactNameProbe {
    borrowed_words: Vec<String>,
    owned_words: Vec<String>,
}

impl super::SourceStepQueries for CompactNameProbe {
    fn catcode(&mut self, code: CharacterCode) -> Catcode {
        classic_catcode(code)
    }
}

impl super::CompactSourceStepQueries for CompactNameProbe {
    fn compact_source_token(&mut self, token: &SourceToken) -> TokenWord {
        if let SourceToken::ControlSequence {
            name,
            kind: SourceControlSequenceKind::Word,
            ..
        } = token
        {
            name.with_text(|text| self.owned_words.push(text.to_owned()));
        }
        TokenWord::pack(Token::undefined_control_sequence())
    }

    fn compact_control_word(&mut self, name: &str) -> TokenWord {
        self.borrowed_words.push(name.to_owned());
        TokenWord::pack(Token::undefined_control_sequence())
    }
}

#[test]
fn compact_control_words_borrow_raw_text_and_own_superscript_fallbacks() {
    let mut state = state(br"\a \alpha \^^61lpha");
    let mut queries = CompactNameProbe::default();

    let _ = state.next_compact_exact_source_step(-1, &mut queries);
    let _ = state.next_compact_exact_source_step(-1, &mut queries);
    let _ = state.next_compact_exact_source_step(-1, &mut queries);

    assert_eq!(queries.borrowed_words, ["alpha"]);
    assert_eq!(queries.owned_words, ["a", "alpha"]);
}
