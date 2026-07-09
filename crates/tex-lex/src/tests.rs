use super::{
    ConditionFrameSummary, ConditionKind, ConditionLimb, InputFrame, InputFrameSummary, InputStack,
    LexError, Lexer, LexerState, LineEvent, LineReader, MemoryInput, TokenListReplayKind,
    load_next_line,
};
use tex_state::env::banks::IntParam;
use tex_state::token::{Catcode, Token};
use tex_state::{ExpansionState, Universe};

mod input_lines;

#[test]
fn strips_trailing_spaces_and_appends_endlinechar() {
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, 13);
    let mut reader = LineReader::new(MemoryInput::new("abc   \n"));

    assert_eq!(
        reader
            .next_event(&stores)
            .expect("memory input should read"),
        Some(LineEvent::Text("abc\r".to_owned()))
    );
    assert_eq!(
        reader
            .next_event(&stores)
            .expect("memory input should read"),
        None
    );
}

#[test]
fn empty_lines_emit_par_event() {
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, 13);
    let mut reader = LineReader::new(MemoryInput::new("   \n\nx\n"));

    assert_eq!(
        reader
            .next_event(&stores)
            .expect("memory input should read"),
        Some(LineEvent::Par)
    );
    assert_eq!(
        reader
            .next_event(&stores)
            .expect("memory input should read"),
        Some(LineEvent::Par)
    );
    assert_eq!(
        reader
            .next_event(&stores)
            .expect("memory input should read"),
        Some(LineEvent::Text("x\r".to_owned()))
    );
    assert_eq!(
        reader
            .next_event(&stores)
            .expect("memory input should read"),
        None
    );
}

#[test]
fn suppresses_invalid_endlinechar_values() {
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, -1);
    let mut reader = LineReader::new(MemoryInput::new("abc\n"));

    assert_eq!(
        reader
            .next_event(&stores)
            .expect("memory input should read"),
        Some(LineEvent::Text("abc".to_owned()))
    );
}

#[test]
fn letters_spaces_and_endline_state_match_tex_rules() {
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, 13);
    let mut lexer = Lexer::new(MemoryInput::new(" a  b\n\n"));

    assert_eq!(
        collect_tokens(&mut lexer, &mut stores),
        vec![
            char_token('a', Catcode::Letter),
            char_token(' ', Catcode::Space),
            char_token('b', Catcode::Letter),
            char_token(' ', Catcode::Space),
            cs_token(&mut stores, "par"),
        ]
    );
    assert_eq!(lexer.frame().state(), LexerState::NewLine);
}

#[test]
fn inactive_endlinechar_blank_line_does_not_emit_par_token() {
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, -1);
    let mut lexer = Lexer::new(MemoryInput::new("a\n\nb"));

    assert_eq!(
        collect_tokens(&mut lexer, &mut stores),
        vec![
            char_token('a', Catcode::Letter),
            char_token('b', Catcode::Letter),
        ]
    );
}

#[test]
fn scans_control_words_and_control_symbols() {
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, 13);
    let mut lexer = Lexer::new(MemoryInput::new("\\foo   x\\$"));

    assert_eq!(
        collect_tokens(&mut lexer, &mut stores),
        vec![
            cs_token(&mut stores, "foo"),
            char_token('x', Catcode::Letter),
            cs_token(&mut stores, "$"),
            char_token(' ', Catcode::Space),
        ]
    );
}

#[test]
fn control_word_scanning_uses_current_catcodes() {
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, 13);
    stores.set_catcode('@', Catcode::Letter);
    let mut lexer = Lexer::new(MemoryInput::new("\\foo@bar"));

    assert_eq!(
        collect_tokens(&mut lexer, &mut stores),
        vec![cs_token(&mut stores, "foo@bar")]
    );
}

#[test]
fn unread_characters_use_catcodes_current_at_token_read() {
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, 13);
    let mut lexer = Lexer::new(MemoryInput::new("a@b"));

    assert_eq!(
        lexer.next_token(&mut stores).expect("first token"),
        Some(char_token('a', Catcode::Letter))
    );

    // pdfTeX check: after `a\catcode`\@=11 @b`, the unread `@` is
    // tokenized as a letter while the already-read `a` keeps its token.
    stores.set_catcode('@', Catcode::Letter);

    assert_eq!(
        collect_tokens(&mut lexer, &mut stores),
        vec![
            char_token('@', Catcode::Letter),
            char_token('b', Catcode::Letter),
            char_token(' ', Catcode::Space),
        ]
    );
}

#[test]
fn control_word_scan_rechecks_catcodes_after_escape_token_read() {
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, 13);
    stores.set_catcode('@', Catcode::Other);
    let mut lexer = Lexer::new(MemoryInput::new("\\@a"));

    // pdfTeX check: a `\catcode`\@=11` assignment before the following
    // token makes `\@a` scan as the control word `@a`, not control symbol
    // `@` followed by letter `a`.
    stores.set_catcode('@', Catcode::Letter);

    assert_eq!(
        collect_tokens(&mut lexer, &mut stores),
        vec![cs_token(&mut stores, "@a")]
    );
}

#[test]
fn next_physical_line_uses_current_endlinechar() {
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, b'!' as i32);
    let mut lexer = Lexer::new(MemoryInput::new("a\nb\nc"));

    assert_eq!(
        lexer.next_token(&mut stores).expect("first token"),
        Some(char_token('a', Catcode::Letter))
    );
    assert_eq!(
        lexer.next_token(&mut stores).expect("first line ending"),
        Some(char_token('!', Catcode::Other))
    );

    // pdfTeX check: `\endlinechar` is read when a physical line is
    // converted to an input line, so changing it here affects the next
    // unread line but cannot rewrite the line already in progress.
    stores.set_int_param(IntParam::END_LINE_CHAR, b'?' as i32);

    assert_eq!(
        lexer
            .next_token(&mut stores)
            .expect("second line first token"),
        Some(char_token('b', Catcode::Letter))
    );
    assert_eq!(
        lexer.next_token(&mut stores).expect("second line ending"),
        Some(char_token('?', Catcode::Other))
    );

    stores.set_int_param(IntParam::END_LINE_CHAR, -1);

    assert_eq!(
        collect_tokens(&mut lexer, &mut stores),
        vec![char_token('c', Catcode::Letter)]
    );
}

#[test]
fn comments_ignore_rest_of_physical_line() {
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, 13);
    let mut lexer = Lexer::new(MemoryInput::new("a% ignored\nb"));

    assert_eq!(
        collect_tokens(&mut lexer, &mut stores),
        vec![
            char_token('a', Catcode::Letter),
            char_token('b', Catcode::Letter),
            char_token(' ', Catcode::Space),
        ]
    );
}

#[test]
fn comment_line_continuation_starts_next_line_in_new_line_state() {
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, 13);
    let mut lexer = Lexer::new(MemoryInput::new("a%\n   b"));

    assert_eq!(
        collect_tokens(&mut lexer, &mut stores),
        vec![
            char_token('a', Catcode::Letter),
            char_token('b', Catcode::Letter),
            char_token(' ', Catcode::Space),
        ]
    );
}

#[test]
fn inactive_endlinechar_still_starts_next_line_in_new_line_state() {
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, -1);
    let mut lexer = Lexer::new(MemoryInput::new("a\n   b"));

    assert_eq!(
        collect_tokens(&mut lexer, &mut stores),
        vec![
            char_token('a', Catcode::Letter),
            char_token('b', Catcode::Letter)
        ]
    );
}

#[test]
fn ignored_and_invalid_catcodes_follow_tex_rules() {
    let mut stores = Universe::new();
    stores.set_catcode('!', Catcode::Ignored);
    stores.set_catcode('?', Catcode::Invalid);
    let mut lexer = Lexer::new(MemoryInput::new("a!?"));

    assert_eq!(
        lexer.next_token(&mut stores).expect("valid token"),
        Some(char_token('a', Catcode::Letter))
    );
    match lexer.next_token(&mut stores) {
        Err(LexError::InvalidCharacter('?')) => {}
        other => panic!("expected invalid-character error, got {other:?}"),
    }
}

#[test]
fn superscript_notation_is_expanded_before_catcode_lookup() {
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, 13);
    stores.set_catcode('@', Catcode::Letter);
    let mut lexer = Lexer::new(MemoryInput::new("^^40 ^^41 ^^^^00E9"));

    assert_eq!(
        collect_tokens(&mut lexer, &mut stores),
        vec![
            char_token('@', Catcode::Letter),
            char_token(' ', Catcode::Space),
            char_token('A', Catcode::Letter),
            char_token(' ', Catcode::Space),
            char_token('é', Catcode::Other),
            char_token(' ', Catcode::Space),
        ]
    );
}

#[test]
fn every_non_ignored_non_invalid_char_catcode_emits_char_token() {
    let cases = [
        ('{', Catcode::BeginGroup),
        ('}', Catcode::EndGroup),
        ('$', Catcode::MathShift),
        ('&', Catcode::AlignmentTab),
        ('#', Catcode::Parameter),
        ('_', Catcode::Subscript),
        ('~', Catcode::Active),
        ('1', Catcode::Other),
        ('^', Catcode::Superscript),
    ];

    for (ch, cat) in cases {
        let mut stores = Universe::new();
        stores.set_catcode(ch, cat);
        let mut lexer = Lexer::new(MemoryInput::new(ch.to_string()));
        assert_eq!(
            lexer.next_token(&mut stores).expect("valid token"),
            Some(char_token(ch, cat))
        );
    }
}

#[test]
fn token_list_frames_replay_before_sources_and_pop_at_end() {
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, 13);
    let list = stores.intern_token_list(&[
        char_token('x', Catcode::Letter),
        char_token('y', Catcode::Letter),
    ]);
    let mut input = InputStack::new(MemoryInput::new("a"));
    input.push_token_list(list, TokenListReplayKind::MacroBody);

    assert!(matches!(
        input.summary().frames(),
        [
            InputFrameSummary::Source { .. },
            InputFrameSummary::TokenList {
                token_list,
                replay_kind: TokenListReplayKind::MacroBody,
                index: 0,
                macro_arguments
            }
        ] if *token_list == list && macro_arguments.is_empty()
    ));
    assert_eq!(
        input.next_token(&mut stores).expect("token-list replay"),
        Some(char_token('x', Catcode::Letter))
    );
    assert!(matches!(
        input.summary().frames().last(),
        Some(InputFrameSummary::TokenList { index: 1, .. })
    ));
    assert_eq!(
        input.next_token(&mut stores).expect("token-list replay"),
        Some(char_token('y', Catcode::Letter))
    );
    assert_eq!(
        input.next_token(&mut stores).expect("source replay"),
        Some(char_token('a', Catcode::Letter))
    );
}

#[test]
fn source_summaries_track_position_and_eof_pop() {
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, 13);
    let mut input = InputStack::new(MemoryInput::new("ab\nc"));

    assert_eq!(
        input.next_token(&mut stores).expect("source token"),
        Some(char_token('a', Catcode::Letter))
    );
    assert!(matches!(
        input.summary().frames(),
        [InputFrameSummary::Source {
            source_id,
            source,
        }] if source_id.raw() == 0
            && source.buffer_offset() == 0
            && source.line_number() == 1
            && source.column() == 1
            && source.lexer_state() == LexerState::MidLine
    ));

    while input
        .next_token(&mut stores)
        .expect("drain input")
        .is_some()
    {}
    assert!(input.summary().is_empty());
    assert!(input.is_empty());
}

#[test]
fn source_summary_is_resume_complete_inside_current_line() {
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, 13);
    let mut input = InputStack::new(MemoryInput::new("éa"));

    assert_eq!(
        input.next_token(&mut stores).expect("source token"),
        Some(char_token('é', Catcode::Other))
    );
    let summary = input.summary();
    let [InputFrameSummary::Source { source_id, source }] = summary.frames() else {
        panic!("expected one source frame");
    };

    assert_eq!(source_id.raw(), 0);
    assert_eq!(source.normalized_line(), "éa\r");
    assert_eq!(source.line_char_offset(), 1);
    assert_eq!(source.line_byte_offset(), 2);
    assert_eq!(source.column(), 1);
    assert_eq!(source.lexer_state(), LexerState::MidLine);
    assert!(source.pending().is_empty());
    assert!(source.is_resume_complete());
}

#[test]
fn source_summary_captures_pending_synthetic_par() {
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, 13);
    let mut input = InputStack::new(MemoryInput::new("\nnext"));
    let Some(InputFrame::Source(source)) = input.frames.last_mut() else {
        panic!("expected source frame");
    };

    assert!(load_next_line(source, &mut stores).expect("blank line loads"));
    let summary = input.summary();
    let [InputFrameSummary::Source { source, .. }] = summary.frames() else {
        panic!("expected one source frame");
    };

    assert_eq!(source.normalized_line(), "");
    assert_eq!(source.line_char_offset(), 0);
    assert_eq!(source.line_byte_offset(), 0);
    assert_eq!(source.line_number(), 1);
    assert_eq!(source.pending(), &[cs_token(&mut stores, "par")]);
    assert!(source.is_resume_complete());
}

#[test]
fn condition_frames_round_trip_through_input_summary() {
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, 13);
    let mut input = InputStack::new(MemoryInput::new("ab"));
    let condition = ConditionFrameSummary::new_ifcase(false)
        .with_or_limb(2, true)
        .with_skip_nesting(1);

    input.push_condition(condition);

    let first = input.summary();
    let round_tripped = first.clone();
    assert_eq!(round_tripped, first);
    assert!(matches!(
        round_tripped.frames(),
        [
            InputFrameSummary::Source { .. },
            InputFrameSummary::Condition(frame),
        ] if frame.kind() == ConditionKind::IfCase
            && frame.limb() == ConditionLimb::Or
            && frame.current_limb_taken()
            && frame.any_limb_taken()
            && frame.ifcase_or_count() == 2
            && frame.skip_nesting() == 1
    ));

    assert_eq!(
        input
            .next_token(&mut stores)
            .expect("condition frame skips"),
        Some(char_token('a', Catcode::Letter))
    );
    assert!(matches!(
        input.summary().frames(),
        [
            InputFrameSummary::Source { source, .. },
            InputFrameSummary::Condition(frame),
        ] if source.column() == 1 && *frame == condition
    ));
}

#[test]
fn open_condition_survives_checkpoint_rollback_resume_summary() {
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, 13);
    let mut input = InputStack::new(MemoryInput::new("xy"));
    input.push_condition(ConditionFrameSummary::new_if(true));

    assert_eq!(
        input.next_token(&mut stores).expect("source token"),
        Some(char_token('x', Catcode::Letter))
    );
    let checkpoint = stores.snapshot();
    let resume_summary = input.summary();

    let updated = ConditionFrameSummary::new_if(true).with_else_limb(false);
    assert_eq!(
        input.update_current_condition(updated),
        Some(ConditionFrameSummary::new_if(true))
    );
    stores.set_int_param(IntParam::END_LINE_CHAR, b'!' as i32);
    assert_eq!(
        input.next_token(&mut stores).expect("source token"),
        Some(char_token('y', Catcode::Letter))
    );

    stores.rollback(&checkpoint);

    assert_eq!(stores.endlinechar(), 13);
    assert!(matches!(
        resume_summary.frames(),
        [
            InputFrameSummary::Source { source, .. },
            InputFrameSummary::Condition(frame),
        ] if source.column() == 1
            && frame.kind() == ConditionKind::If
            && frame.limb() == ConditionLimb::If
            && frame.current_limb_taken()
            && frame.any_limb_taken()
            && frame.ifcase_or_count() == 0
            && frame.skip_nesting() == 0
    ));
}

#[test]
fn source_summary_restores_mid_world_input_from_recorded_content() {
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, 13);
    stores
        .world_mut()
        .set_memory_file("main.tex", b"m".to_vec())
        .expect("seed main");
    stores
        .world_mut()
        .set_memory_file("inc.tex", b"ab\nc".to_vec())
        .expect("seed include");
    let main = stores.world_mut().read_file("main.tex").expect("read main");
    let inc = stores
        .world_mut()
        .read_file("inc.tex")
        .expect("read include");
    let mut input = InputStack::new(super::WorldInput::from_content(main));
    input.push_source(super::WorldInput::from_content(inc));

    assert_eq!(
        input.next_token(&mut stores).expect("first include token"),
        Some(char_token('a', Catcode::Letter))
    );
    stores.set_input_summary(input.summary());
    let snapshot = stores.snapshot();

    assert_eq!(
        input.next_token(&mut stores).expect("second include token"),
        Some(char_token('b', Catcode::Letter))
    );
    stores
        .world_mut()
        .set_memory_file("inc.tex", b"changed".to_vec())
        .expect("mutate source after snapshot");
    stores.rollback(&snapshot);

    let summary = stores.input_summary().clone();
    let mut restored = InputStack::from_summary(&summary, |source_id, source| {
        let content = stores
            .world()
            .recorded_input_content(source_id.raw() as usize)
            .expect("recorded source content");
        Ok::<_, ()>(super::WorldInput::from_content_after_lines(
            content,
            source.line_number(),
        ))
    })
    .expect("restore input stack");

    assert_eq!(
        restored
            .next_token(&mut stores)
            .expect("restored second include token"),
        Some(char_token('b', Catcode::Letter))
    );
}

fn collect_tokens(lexer: &mut Lexer<MemoryInput>, stores: &mut impl ExpansionState) -> Vec<Token> {
    let mut tokens = Vec::new();
    while let Some(token) = lexer.next_token(stores).expect("lexing should succeed") {
        tokens.push(token);
    }
    tokens
}

fn char_token(ch: char, cat: Catcode) -> Token {
    Token::Char { ch, cat }
}

fn cs_token(stores: &mut impl ExpansionState, name: &str) -> Token {
    Token::Cs(stores.intern(name))
}
