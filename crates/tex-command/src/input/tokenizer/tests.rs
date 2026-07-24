use std::cell::Cell;
use std::sync::Arc;

use tex_state::token::Catcode;

use super::{SourceControlSequenceKind, SourceToken, SourceTokenizationStep};
use crate::{
    CharacterCode, CommandDialect, CommandProfile, CommandState, RegisteredSourceKind,
    SourceRegistration,
};

fn state(bytes: &[u8]) -> CommandState {
    state_for_profile(bytes, CommandProfile::TEX82)
}

fn state_for_profile(bytes: &[u8], profile: CommandProfile) -> CommandState {
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
        SourceToken::ControlSequence { name, kind, range } => (
            name.into_iter()
                .map(|code| code.to_byte().expect("exact byte"))
                .collect(),
            kind,
            range.start(),
            range.end(),
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
        character(state.next_exact_source_step(13, catcode)),
        (b'A', Catcode::Letter, 0, 1)
    );
    assert_eq!(
        character(state.next_exact_source_step(13, catcode)),
        (b'B', Catcode::Letter, 3, 4)
    );
    assert_eq!(
        character(state.next_exact_source_step(13, catcode)),
        (b' ', Catcode::Space, 4, 4)
    );
    assert_eq!(
        control(state.next_exact_source_step(13, catcode)),
        (
            b"par".to_vec(),
            SourceControlSequenceKind::Paragraph,
            19,
            19
        )
    );
    assert_eq!(
        character(state.next_exact_source_step(13, catcode)),
        (b'C', Catcode::Letter, 21, 22)
    );
    assert_eq!(
        character(state.next_exact_source_step(13, catcode)),
        (b' ', Catcode::Space, 22, 23)
    );
    assert_eq!(
        character(state.next_exact_source_step(13, catcode)),
        (b'D', Catcode::Letter, 23, 24)
    );
    assert_eq!(
        character(state.next_exact_source_step(13, catcode)),
        (b' ', Catcode::Space, 24, 24)
    );
    assert_eq!(
        state.next_exact_source_step(13, catcode),
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
        control(command.next_exact_source_step(13, catcode)),
        (b"foo".to_vec(), SourceControlSequenceKind::Word, 0, 4)
    );
    assert_eq!(
        control(command.next_exact_source_step(13, catcode)),
        (b"!".to_vec(), SourceControlSequenceKind::Symbol, 6, 8)
    );
    assert_eq!(
        control(command.next_exact_source_step(13, catcode)),
        (b"~".to_vec(), SourceControlSequenceKind::Active, 8, 9)
    );
    assert_eq!(
        control(command.next_exact_source_step(13, catcode)),
        (b"abc".to_vec(), SourceControlSequenceKind::Word, 9, 16)
    );

    let mut null_state = state(b"\\");
    assert_eq!(
        control(null_state.next_exact_source_step(-1, classic_catcode)),
        (Vec::new(), SourceControlSequenceKind::Null, 0, 1)
    );
}

#[test]
fn canonical_superscript_forms_use_lowercase_hex_and_complete_ranges() {
    let mut state = state(b"^^41^^7a^^8^^5A");

    assert_eq!(
        character(state.next_exact_source_step(-1, classic_catcode)),
        (b'A', Catcode::Letter, 0, 4)
    );
    assert_eq!(
        character(state.next_exact_source_step(-1, classic_catcode)),
        (b'z', Catcode::Letter, 4, 8)
    );
    assert_eq!(
        character(state.next_exact_source_step(-1, classic_catcode)),
        (b'x', Catcode::Letter, 8, 11)
    );
    assert_eq!(
        character(state.next_exact_source_step(-1, classic_catcode)),
        (b'u', Catcode::Letter, 11, 14)
    );
    assert_eq!(
        character(state.next_exact_source_step(-1, classic_catcode)),
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

    match state.next_exact_source_step(-1, catcode) {
        SourceTokenizationStep::InvalidCharacter(invalid) => {
            assert_eq!(invalid.code().to_byte(), Ok(b'?'));
            assert_eq!((invalid.range().start(), invalid.range().end()), (0, 4));
        }
        other => panic!("expected invalid-character step, found {other:?}"),
    }
    assert_eq!(
        character(state.next_exact_source_step(-1, catcode)),
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
        character(state.next_exact_source_step(-1, catcode)),
        (b'a', Catcode::Other, 0, 1)
    );
    second_is_letter.set(true);
    assert_eq!(
        character(state.next_exact_source_step(-1, catcode)),
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
        let (actual, actual_catcode, _, _) = character(state.next_exact_source_step(-1, catcode));
        assert_eq!(actual, expected);
        assert_eq!(actual_catcode, Catcode::Other);
    }
}

#[test]
fn all_canonical_dialects_share_the_exact_byte_state_machine() {
    for dialect in [
        CommandDialect::Tex82,
        CommandDialect::Etex26,
        CommandDialect::Pdftex14027,
    ] {
        let mut state = state_for_profile(b"^^7a", CommandProfile::exact(dialect));
        assert_eq!(
            character(state.next_exact_source_step(-1, classic_catcode)),
            (b'z', Catcode::Letter, 0, 4)
        );
    }
}

#[test]
fn snapshots_restore_the_lexer_state_and_exact_source_cursor() {
    let mut state = state(b"A\n");
    assert_eq!(
        character(state.next_exact_source_step(13, classic_catcode)),
        (b'A', Catcode::Letter, 0, 1)
    );
    let snapshot = state.snapshot();
    let expected = state.next_exact_source_step(13, classic_catcode);
    assert_eq!(character(expected.clone()), (b' ', Catcode::Space, 1, 1));

    state.rollback(snapshot).expect("same-profile snapshot");
    assert_eq!(state.next_exact_source_step(13, classic_catcode), expected);
}
