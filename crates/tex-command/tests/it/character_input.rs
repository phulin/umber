use std::sync::Arc;

use tex_command::{
    Catcode, CharacterCode, CommandDialect, CommandHostCapabilities, CommandHostContext,
    CommandProcessor, CommandProfile, CommandRuntime, CommandState, RegisteredSourceKind,
    SourceControlSequenceKind, SourceRegistration, SourceToken, SourceTokenizationStep,
};
use tex_state::Universe;
use tex_state::token::Token;

const CANONICAL_INPUT: &[u8] = include_bytes!(
    "../../../../tests/corpus/command/tex82/command-transitions-v1/sources/input-recovery.tex"
);
const CANONICAL_MATRIX: &str =
    include_str!("../../../../tests/tex82-oracle/semantic-event-matrix.txt");
const ETEX_CANONICAL_MATRIX: &str =
    include_str!("../../../../tests/etex26-oracle/semantic-event-matrix.txt");
const PDFTEX_CANONICAL_MATRIX: &str =
    include_str!("../../../../tests/pdftex14027-oracle/semantic-event-matrix.txt");

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
    assert_shared_input_matrix_coverage();

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

#[derive(Debug, Eq, PartialEq)]
enum RawProjection {
    Character(char, Catcode),
    ControlSequence,
}

fn processor_projection(profile: CommandProfile, use_get_token: bool) -> Vec<RawProjection> {
    let mut command = CommandState::new(profile);
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(fixture_body()),
        ))
        .expect("canonical exact bytes register");
    command
        .open_registered_source(source)
        .expect("registered fixture source opens");
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    // The focused source makes `!` ignored before entering this body. Feed
    // the committed body with that already-observed aggregate catcode state;
    // the processor must query it itself for every delivered character.
    universe.set_catcode('!', Catcode::Ignored);
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = CommandProcessor::new(
        &mut command,
        &mut runtime,
        universe.command_context(),
        CommandHostContext::new(&mut capabilities),
    );

    let mut projection = Vec::new();
    loop {
        let command = if use_get_token {
            processor.get_token()
        } else {
            processor.get_next()
        }
        .expect("canonical input delivery succeeds");
        let Some(command) = command else {
            return projection;
        };
        projection.push(match command.spelling().semantic_token() {
            Token::Char { ch, cat } => RawProjection::Character(ch, cat),
            Token::Cs(_) => RawProjection::ControlSequence,
            unexpected => panic!("focused source delivered unexpected raw token {unexpected:?}"),
        });
    }
}

#[test]
fn raw_delivery_matches_the_pinned_fixture_through_the_single_processor_loop() {
    assert_shared_input_matrix_coverage();
    let expected = vec![
        RawProjection::Character('A', Catcode::Letter),
        RawProjection::Character('i', Catcode::Letter),
        RawProjection::Character('g', Catcode::Letter),
        RawProjection::Character('n', Catcode::Letter),
        RawProjection::Character('o', Catcode::Letter),
        RawProjection::Character('r', Catcode::Letter),
        RawProjection::Character('e', Catcode::Letter),
        RawProjection::Character('d', Catcode::Letter),
        RawProjection::Character(' ', Catcode::Space),
        RawProjection::Character('B', Catcode::Letter),
        RawProjection::Character('C', Catcode::Letter),
        RawProjection::Character(' ', Catcode::Space),
        RawProjection::ControlSequence,
        RawProjection::Character('D', Catcode::Letter),
        RawProjection::Character('}', Catcode::EndGroup),
        RawProjection::Character(' ', Catcode::Space),
    ];

    for dialect in [
        CommandDialect::Tex82,
        CommandDialect::Etex26,
        CommandDialect::Pdftex14027,
    ] {
        let profile = CommandProfile::exact(dialect);
        assert_eq!(
            processor_projection(profile, false),
            expected,
            "{dialect:?} get_next diverged from the pinned TeX82 shared-domain fixture"
        );
        assert_eq!(
            processor_projection(profile, true),
            expected,
            "{dialect:?} get_token diverged from the shared raw-delivery loop"
        );
    }
}

fn assert_shared_input_matrix_coverage() {
    for matrix in [
        CANONICAL_MATRIX,
        ETEX_CANONICAL_MATRIX,
        PDFTEX_CANONICAL_MATRIX,
    ] {
        assert!(matrix.contains("command|raw delivery|transitions.tex|get_next return"));
        assert!(matrix.contains("input|source retirement|transitions-child.tex"));
        assert!(matrix.contains("input|terminal stop|transitions-child.tex"));
    }
    assert!(CANONICAL_MATRIX.contains(
        "alignment|literal brace increment|alignment-delivery.tex|get_next left-brace accounting"
    ));
    for matrix in [ETEX_CANONICAL_MATRIX, PDFTEX_CANONICAL_MATRIX] {
        assert!(matrix.contains(
            "alignment|align_state change|transitions.tex|get_next literal brace and alignment state commits"
        ));
    }
    assert!(CANONICAL_MATRIX.contains(
        "source|ignored character and trailing-space collapse|input-recovery.tex|get_next M/N/S source-token loop"
    ));
    assert!(CANONICAL_MATRIX.contains(
        "source|comment discard and blank-line par|input-recovery.tex|get_next end_line and new_line cases"
    ));
    assert!(CANONICAL_MATRIX.contains(
        "command|get_token string operand|input-recovery.tex|string_code scan_toks operand fetch"
    ));
}
