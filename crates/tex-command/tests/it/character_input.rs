use std::sync::Arc;

use tex_command::{
    Catcode, CharacterCode, CommandDialect, CommandProfile, CommandState, RegisteredSourceKind,
    SourceControlSequenceKind, SourceRegistration, SourceToken, SourceTokenizationStep,
};

const CANONICAL_INPUT: &[u8] = include_bytes!(
    "../../../../tests/corpus/command/tex82/command-transitions-v1/sources/input-recovery.tex"
);
const CANONICAL_MATRIX: &str =
    include_str!("../../../../tests/tex82-oracle/semantic-event-matrix.txt");

fn fixture_body() -> &'static [u8] {
    let start_marker = b"\\long\\def\\physicaltokens{";
    let start = CANONICAL_INPUT
        .windows(start_marker.len())
        .position(|window| window == start_marker)
        .expect("pinned fixture contains physicaltokens definition")
        + start_marker.len();
    let end_marker = b"}\n\\catcode`\\!=";
    let end = CANONICAL_INPUT[start..]
        .windows(end_marker.len())
        .position(|window| window == end_marker)
        .expect("pinned fixture contains physicaltokens terminator")
        + start
        + 1;
    &CANONICAL_INPUT[start..end]
}

fn fixture_catcode(code: CharacterCode) -> Catcode {
    match code
        .to_byte()
        .expect("canonical fixture is exact-byte input")
    {
        b'\\' => Catcode::Escape,
        b'{' => Catcode::BeginGroup,
        b'}' => Catcode::EndGroup,
        b'^' => Catcode::Superscript,
        b'%' => Catcode::Comment,
        b'!' => Catcode::Ignored,
        b' ' | b'\t' => Catcode::Space,
        b'\r' => Catcode::EndLine,
        b'a'..=b'z' | b'A'..=b'Z' => Catcode::Letter,
        _ => Catcode::Other,
    }
}

fn canonical_token_projection(profile: CommandProfile) -> Vec<(u16, Catcode)> {
    let mut state = CommandState::new(profile);
    let source = state
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(fixture_body()),
        ))
        .expect("canonical exact bytes register");
    state
        .open_registered_source(source)
        .expect("registered fixture source opens");

    let mut tokens = Vec::new();
    loop {
        match state.next_exact_source_step(13, fixture_catcode) {
            SourceTokenizationStep::Token(SourceToken::Character { code, catcode, .. }) => {
                tokens.push((u16::from(code.to_byte().expect("exact byte")), catcode));
            }
            SourceTokenizationStep::Token(SourceToken::ControlSequence {
                kind: SourceControlSequenceKind::Paragraph,
                ..
            }) => tokens.push((256, Catcode::Escape)),
            SourceTokenizationStep::Token(other) => {
                panic!("unexpected control sequence in focused fixture body: {other:?}");
            }
            SourceTokenizationStep::InvalidCharacter(invalid) => {
                panic!("unexpected invalid fixture character: {invalid:?}");
            }
            SourceTokenizationStep::End => return tokens,
        }
    }
}

#[test]
fn exact_profiles_match_the_pinned_tex82_source_token_fixture() {
    assert!(CANONICAL_MATRIX.contains(
        "source|ignored character and trailing-space collapse|input-recovery.tex|get_next M/N/S source-token loop"
    ));
    assert!(CANONICAL_MATRIX.contains(
        "source|comment discard and blank-line par|input-recovery.tex|get_next end_line and new_line cases"
    ));

    let expected = [
        (b'A' as u16, Catcode::Letter),
        (b'i' as u16, Catcode::Letter),
        (b'g' as u16, Catcode::Letter),
        (b'n' as u16, Catcode::Letter),
        (b'o' as u16, Catcode::Letter),
        (b'r' as u16, Catcode::Letter),
        (b'e' as u16, Catcode::Letter),
        (b'd' as u16, Catcode::Letter),
        (b' ' as u16, Catcode::Space),
        (b'B' as u16, Catcode::Letter),
        (b'C' as u16, Catcode::Letter),
        (b' ' as u16, Catcode::Space),
        (256, Catcode::Escape),
        (b'D' as u16, Catcode::Letter),
        (b'}' as u16, Catcode::EndGroup),
        (b' ' as u16, Catcode::Space),
    ];

    for dialect in [
        CommandDialect::Tex82,
        CommandDialect::Etex26,
        CommandDialect::Pdftex14027,
    ] {
        assert_eq!(
            canonical_token_projection(CommandProfile::exact(dialect)),
            expected,
            "{dialect:?} exact input diverged from the pinned TeX82 shared-domain fixture"
        );
    }
}
